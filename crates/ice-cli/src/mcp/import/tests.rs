use super::*;

struct Env {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}
impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
            project: tempfile::tempdir().unwrap(),
        }
    }
    fn write(root: &Path, relative: &str, value: Value) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, value.to_string()).unwrap();
        path
    }
    fn home(&self, relative: &str, value: Value) -> PathBuf {
        Self::write(self.home.path(), relative, value)
    }
    fn project(&self, relative: &str, value: Value) -> PathBuf {
        Self::write(self.project.path(), relative, value)
    }
    fn load(&self, kinds: &[&str], trusted: bool) -> Imported {
        let kinds: Vec<String> = kinds.iter().map(|kind| kind.to_string()).collect();
        load(&kinds, Some(self.home.path()), self.project.path(), trusted)
    }
}
fn command(imported: &Imported, name: &str) -> (String, Vec<String>) {
    let config = &imported.servers[name].0;
    (config.command.clone(), config.args.clone())
}

#[test]
fn json_kinds_import_stdio_servers_with_args_env_and_cwd() {
    let env = Env::new();
    let stdio = json!({"a": {"command":"npx","args":["-y","pkg"],"env":{"K":"V"},"cwd":"/work"}});
    let cursor = env.home(".cursor/mcp.json", json!({"mcpServers": stdio}));
    env.home(
        ".config/Claude/claude_desktop_config.json",
        json!({"mcpServers": {"desktop": {"command":"d"}}}),
    );
    env.home(
        ".codeium/windsurf/mcp_config.json",
        json!({"mcpServers": {"wind": {"command":"w"}}}),
    );
    let imported = env.load(&["cursor", "claude-desktop", "windsurf"], false);
    let (config, source) = &imported.servers["a"];
    assert_eq!(
        (config.command.as_str(), config.args.as_slice()),
        ("npx", ["-y".to_string(), "pkg".to_string()].as_slice())
    );
    assert_eq!(config.env["K"], "V");
    assert_eq!(config.cwd.as_deref(), Some(Path::new("/work")));
    assert_eq!(source, &cursor);
    assert_eq!(command(&imported, "desktop").0, "d");
    assert_eq!(command(&imported, "wind").0, "w");
    assert!(imported.notes.is_empty(), "{:?}", imported.notes);
}

#[test]
fn claude_code_reads_user_servers_and_project_sections_only_when_trusted() {
    let env = Env::new();
    let cwd = env.project.path().to_str().unwrap().to_string();
    env.home(
        ".claude.json",
        json!({
            "mcpServers": {"user": {"command":"u"}, "both": {"command":"top"}},
            "projects": {
                (cwd.clone()): {"mcpServers": {"local": {"command":"l"}, "both": {"command":"scoped"}}},
                "/elsewhere": {"mcpServers": {"other": {"command":"o"}}},
            }
        }),
    );
    let untrusted = env.load(&["claude-code"], false);
    assert_eq!(
        untrusted.servers.keys().collect::<Vec<_>>(),
        ["both", "user"]
    );
    let trusted = env.load(&["claude-code"], true);
    assert_eq!(
        trusted.servers.keys().collect::<Vec<_>>(),
        ["both", "local", "user"]
    );
    // The project-scoped entry is more specific, so it wins.
    assert_eq!(command(&trusted, "both").0, "scoped");
}

#[test]
fn project_scoped_sources_need_trust() {
    let env = Env::new();
    env.project(
        ".vscode/mcp.json",
        json!({"servers": {"code": {"command":"c"}}}),
    );
    env.project(
        "opencode.json",
        json!({"mcp": {"oc": {"type":"local","command":["o","--flag"]}}}),
    );
    let kinds = ["vscode", "opencode"];
    assert!(env.load(&kinds, false).servers.is_empty());
    let trusted = env.load(&kinds, true);
    assert_eq!(command(&trusted, "code").0, "c");
    assert_eq!(command(&trusted, "oc"), ("o".into(), vec!["--flag".into()]));
}

#[test]
fn opencode_project_overrides_user_and_maps_local_entries() {
    let env = Env::new();
    env.home(
        ".config/opencode/opencode.json",
        json!({"mcp": {
            "shared": {"type":"local","command":["user"]},
            "off": {"type":"local","command":["x"],"enabled":false},
            "env": {"type":"local","command":["e"],"environment":{"A":"B"}},
        }}),
    );
    env.project(
        "opencode.json",
        json!({"mcp": {"shared": {"type":"local","command":["project"]}}}),
    );
    let imported = env.load(&["opencode"], true);
    assert_eq!(command(&imported, "shared").0, "project");
    // `enabled: false` imports as a disabled server, visible in /mcp.
    assert!(imported.servers["off"].0.disabled);
    assert!(!imported.servers["env"].0.disabled);
    assert_eq!(imported.servers["env"].0.env["A"], "B");
}

#[test]
fn remote_entries_import_their_url_and_headers() {
    let env = Env::new();
    env.home(
        ".cursor/mcp.json",
        json!({"mcpServers": {
            "web": {"url":"https://example.com/mcp","headers":{"x-key":"${KEY}"}},
            "typed": {"type":"sse","command":"ignored"},
            "ok": {"command":"fine"},
        }}),
    );
    env.home(
        ".windsurf/mcp.json",
        json!({"mcpServers": {"surf": {"serverUrl":"https://surf.example/mcp","disabled":true}}}),
    );
    env.project(
        "opencode.json",
        json!({"mcp": {"remote": {"type":"remote","url":"https://example.com","enabled":false}}}),
    );
    let imported = env.load(&["cursor", "windsurf", "opencode"], true);
    assert_eq!(
        imported.servers.keys().collect::<Vec<_>>(),
        ["ok", "remote", "surf", "web"]
    );
    let web = &imported.servers["web"].0;
    assert_eq!(web.url.as_deref(), Some("https://example.com/mcp"));
    assert_eq!(web.headers["x-key"], "${KEY}");
    assert!(web.command.is_empty());
    assert!(imported.servers["remote"].0.disabled);
    let surf = &imported.servers["surf"].0;
    assert_eq!(
        entry_json(surf),
        json!({"url":"https://surf.example/mcp","disabled":true})
    );
    // A remote type without a URL is reported, not dropped silently.
    assert!(
        imported
            .notes
            .iter()
            .any(|note| note.contains("typed") && note.contains("no url")),
        "{:?}",
        imported.notes
    );
}

#[test]
fn bad_input_becomes_notes_and_never_an_error() {
    let env = Env::new();
    std::fs::create_dir_all(env.home.path().join(".cursor")).unwrap();
    std::fs::write(env.home.path().join(".cursor/mcp.json"), "{ not json").unwrap();
    env.home(
        ".windsurf/mcp.json",
        json!({"mcpServers": {
            "bad name": {"command":"x"},
            "empty": {"command":""},
            "args": {"command":"x","args":[1]},
            "fine": {"command":"y"},
        }}),
    );
    let imported = env.load(&["cursor", "windsurf", "codex", "ice", "nope"], false);
    assert_eq!(imported.servers.keys().collect::<Vec<_>>(), ["fine"]);
    let notes = imported.notes.join("\n");
    for expected in [
        "import:cursor",
        "invalid server name",
        "empty: no command",
        "args:",
        "import:codex: unsupported",
        "import:ice: unknown import kind",
        "import:nope: unknown import kind",
    ] {
        assert!(notes.contains(expected), "{expected}\n{notes}");
    }
}

#[test]
fn earlier_kinds_win_and_repeated_kinds_are_read_once() {
    let env = Env::new();
    env.home(
        ".cursor/mcp.json",
        json!({"mcpServers": {"same": {"command":"cursor"}}}),
    );
    env.home(
        ".windsurf/mcp.json",
        json!({"mcpServers": {"same": {"command":"windsurf"}}}),
    );
    assert_eq!(
        command(
            &env.load(&["windsurf", "cursor", "windsurf"], false),
            "same"
        )
        .0,
        "windsurf"
    );
    assert_eq!(
        command(&env.load(&["cursor", "windsurf"], false), "same").0,
        "cursor"
    );
}

#[test]
fn no_home_directory_skips_user_sources_without_failing() {
    let project = tempfile::tempdir().unwrap();
    let kinds = vec!["cursor".to_string(), "claude-code".to_string()];
    let imported = load(&kinds, None, project.path(), true);
    assert!(imported.servers.is_empty() && imported.notes.is_empty());
}

#[test]
fn entry_json_round_trips_through_the_config_schema() {
    let config: ServerConfig = serde_json::from_value(
        json!({"command":"c","args":["a"],"env":{"K":"V"},"cwd":"/w","disabled":true}),
    )
    .unwrap();
    let copy: ServerConfig = serde_json::from_value(entry_json(&config)).unwrap();
    assert_eq!(
        (copy.command, copy.args, copy.env, copy.cwd),
        (config.command, config.args, config.env, config.cwd)
    );
    assert!(copy.disabled);
}

#[test]
fn layers_override_imports_and_project_wins_over_global() {
    let env = Env::new();
    let agent = tempfile::tempdir().unwrap();
    env.home(
        ".cursor/mcp.json",
        json!({"mcpServers": {
            "imported": {"command":"i"},
            "shadowed": {"command":"import"},
            "layered": {"command":"import"},
        }}),
    );
    let global = Env::write(
        agent.path(),
        "mcp.json",
        json!({"imports":["cursor"],"mcpServers":{
            "shadowed": {"command":"global"},
            "layered": {"command":"global"},
        }}),
    );
    let project = env.project(
        ".icers/mcp.json",
        json!({"mcpServers": {"layered": {"command":"project"}}}),
    );
    let layered = load_layered_from(
        Some(env.home.path()),
        agent.path(),
        env.project.path(),
        true,
    )
    .unwrap();
    let source = |name: &str| (&layered.servers[name].0.command, &layered.servers[name].1);
    assert_eq!(source("imported").0, "i");
    assert!(source("imported").1.starts_with(env.home.path()));
    assert_eq!(source("shadowed"), (&"global".to_string(), &global));
    assert_eq!(source("layered"), (&"project".to_string(), &project));

    // Without trust the project layer and its project-scoped imports are ignored.
    let untrusted = load_layered_from(
        Some(env.home.path()),
        agent.path(),
        env.project.path(),
        false,
    )
    .unwrap();
    assert_eq!(untrusted.servers["layered"].0.command, "global");
}

#[tokio::test]
async fn editing_an_imported_server_copies_it_and_leaves_the_foreign_file_alone() {
    use ice_harness::agent_harness::AgentHarness;
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    // A project-scoped import keeps the test independent of the real home.
    let foreign = Env::write(
        project.path(),
        ".vscode/mcp.json",
        json!({"servers": {"imp": {"command":"echo","args":["hi"],"env":{"K":"V"}}}}),
    );
    let before = std::fs::read_to_string(&foreign).unwrap();
    Env::write(agent.path(), "mcp.json", json!({"imports":["vscode"]}));
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let manager = McpManager::new(
        &harness,
        &Default::default(),
        project.path(),
        agent.path().into(),
        true,
        Default::default(),
    );
    manager.set_disabled("imp", true).await.unwrap();

    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), before);
    let global: Value =
        serde_json::from_str(&std::fs::read_to_string(agent.path().join("mcp.json")).unwrap())
            .unwrap();
    assert_eq!(
        global["mcpServers"]["imp"],
        json!({"command":"echo","args":["hi"],"env":{"K":"V"},"disabled":true})
    );
    assert_eq!(global["imports"], json!(["vscode"]));
    // The user's copy now provides the server.
    let view = manager.snapshot();
    assert!(view.iter().any(|s| s.name == "imp" && s.disabled));
    assert_eq!(
        load_layered(agent.path(), project.path(), true)
            .unwrap()
            .servers["imp"]
            .1,
        agent.path().join("mcp.json")
    );
}

#[tokio::test]
async fn panel_save_on_an_imported_server_copies_it_and_keeps_its_disabled_flag() {
    use crate::mcp::{DirectTools, PanelChange};
    use ice_harness::agent_harness::AgentHarness;
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let foreign = Env::write(
        project.path(),
        ".vscode/mcp.json",
        json!({"servers": {"imp": {"command":"echo","disabled":true}}}),
    );
    let before = std::fs::read_to_string(&foreign).unwrap();
    Env::write(agent.path(), "mcp.json", json!({"imports":["vscode"]}));
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let manager = McpManager::new(
        &harness,
        &Default::default(),
        project.path(),
        agent.path().into(),
        true,
        Default::default(),
    );
    manager
        .save_panel(&[PanelChange {
            server: "imp".into(),
            disabled: None,
            direct_tools: Some(DirectTools::Tools(vec!["one".into()])),
        }])
        .await
        .unwrap();

    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), before);
    let global: Value =
        serde_json::from_str(&std::fs::read_to_string(agent.path().join("mcp.json")).unwrap())
            .unwrap();
    assert_eq!(
        global["mcpServers"]["imp"],
        json!({"command":"echo","args":[],"disabled":true,"directTools":["one"]})
    );
}

#[test]
fn a_broken_newer_entry_masks_the_one_it_replaced() {
    let env = Env::new();
    let older = env.home(
        ".codeium/windsurf/mcp_config.json",
        json!({"mcpServers": {"s": {"command":"old"}, "kept": {"command":"k"}}}),
    );
    env.home(
        ".windsurf/mcp.json",
        json!({"mcpServers": {"s": {"args":["no command"]}}}),
    );
    let imported = env.load(&["windsurf"], false);
    assert!(!imported.servers.contains_key("s"));
    assert_eq!(command(&imported, "kept").0, "k");
    assert_eq!(imported.shadowed, [("s".to_string(), older)]);
    assert_eq!(
        imported.notes,
        ["import:windsurf: s: no command".to_string()]
    );
}
