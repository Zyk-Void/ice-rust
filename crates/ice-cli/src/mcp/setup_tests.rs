use super::*;

fn write(root: &Path, relative: &str, value: Value) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, value.to_string()).unwrap();
    path
}

#[test]
fn view_lists_detected_sources_conflicts_and_notes() {
    let home = tempfile::tempdir().unwrap();
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let cursor = write(
        home.path(),
        ".cursor/mcp.json",
        json!({"mcpServers": {
            "shared": {"command":"cursor"},
            "only": {"command":"c"},
            "remote": {"url":"https://example.invalid/mcp"},
        }}),
    );
    write(home.path(), ".codex/config.toml", json!({}));
    let windsurf = write(
        home.path(),
        ".windsurf/mcp.json",
        json!({"mcpServers": {"only": {"command":"w"}}}),
    );
    // Project-scoped sources are not even detected without trust.
    write(project.path(), ".vscode/mcp.json", json!({"servers": {}}));
    let global = write(
        agent.path(),
        "mcp.json",
        json!({"imports":["cursor","windsurf"],"mcpServers":{"shared":{"command":"mine"}}}),
    );

    let view = view(Some(home.path()), agent.path(), project.path(), false).unwrap();
    let sources: Vec<_> = view
        .imports
        .iter()
        .map(|s| (s.detected.kind, s.enabled, s.detected.servers.clone()))
        .collect();
    assert_eq!(
        sources,
        [
            (
                "cursor",
                true,
                vec!["only".to_string(), "shared".to_string()]
            ),
            ("codex", false, vec![]),
            ("windsurf", true, vec!["only".to_string()]),
        ]
    );
    assert_eq!(view.imports[0].detected.files, [cursor.clone()]);
    assert_eq!(
        view.conflicts,
        [
            Conflict {
                server: "only".into(),
                winner: cursor.clone(),
                shadowed: vec![windsurf],
            },
            Conflict {
                server: "shared".into(),
                winner: global,
                shadowed: vec![cursor],
            },
        ]
    );
    assert_eq!(
        view.notes,
        ["import:cursor: remote: unsupported (http)".to_string()]
    );
    assert_eq!(view.configured, ["only", "shared"]);

    let trusted = view_from(home.path(), agent.path(), project.path());
    assert!(trusted.imports.iter().any(|s| s.detected.kind == "vscode"));
}

fn view_from(home: &Path, agent: &Path, project: &Path) -> SetupView {
    view(Some(home), agent, project, true).unwrap()
}

/// The manager holds the harness weakly, so the caller keeps it alive.
async fn manager(agent: &Path, project: &Path) -> (AgentHarness, McpManager) {
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let manager = McpManager::new(
        &harness,
        &Default::default(),
        project,
        agent.into(),
        true,
        Default::default(),
    );
    (harness, manager)
}

#[tokio::test]
async fn set_import_edits_global_imports_and_reloads_servers() {
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    // A disabled, project-scoped import keeps the test offline and off the real home.
    write(
        project.path(),
        ".vscode/mcp.json",
        json!({"servers": {"imp": {"command":"echo","disabled":true}}}),
    );
    let global = write(
        agent.path(),
        "mcp.json",
        json!({"imports":["opencode-typo"],"settings":{"keep":1}}),
    );
    let (_harness, manager) = manager(agent.path(), project.path()).await;
    let read =
        || -> Value { serde_json::from_str(&std::fs::read_to_string(&global).unwrap()).unwrap() };

    manager.set_import("vscode", true).await.unwrap();
    assert_eq!(read()["imports"], json!(["opencode-typo", "vscode"]));
    assert_eq!(read()["settings"], json!({"keep":1}));
    assert!(manager.snapshot().iter().any(|s| s.name == "imp"));
    // Enabling twice does not duplicate the kind.
    manager.set_import("vscode", true).await.unwrap();
    assert_eq!(read()["imports"], json!(["opencode-typo", "vscode"]));

    manager.set_import("vscode", false).await.unwrap();
    assert_eq!(read()["imports"], json!(["opencode-typo"]));
    assert!(manager.snapshot().iter().all(|s| s.name != "imp"));

    assert!(manager.set_import("ice", true).await.is_err());
    write(
        project.path(),
        ".icers/mcp.json",
        json!({"imports":["vscode"]}),
    );
    let error = manager.set_import("vscode", false).await.unwrap_err();
    assert!(error.contains(".icers"), "{error}");
}

#[tokio::test]
async fn presets_add_stdio_entries_and_refuse_http_and_duplicates() {
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let (_harness, manager) = manager(agent.path(), project.path()).await;
    let error = manager.add_preset("context7").await.unwrap_err();
    assert!(error.contains("HTTP"), "{error}");
    assert!(manager.add_preset("missing").await.is_err());
    // An existing entry is never overwritten (and nothing is spawned).
    write(
        agent.path(),
        "mcp.json",
        json!({"mcpServers":{"chrome-devtools":{"command":"mine","disabled":true}}}),
    );
    let error = manager.add_preset("chrome-devtools").await.unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert!(PRESETS
        .iter()
        .any(|p| matches!(p.entry, PresetEntry::Stdio { .. })));
}
