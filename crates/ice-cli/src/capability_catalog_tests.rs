use super::*;
use crate::mcp::ToolInfo;
use std::sync::Arc;
use tempfile::TempDir;

fn write_skill(root: &Path, folder: &str, name: &str, extra: &str) -> PathBuf {
    let dir = root.join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("SKILL.md");
    std::fs::write(
        &file,
        format!("---\nname: {name}\ndescription: useful search\n{extra}---\nbody\n"),
    )
    .unwrap();
    file
}

async fn load(root: &Path, dirs: &[PathBuf]) -> LoadSkillsResult {
    let env: Arc<dyn ice_tools::ExecutionEnv> =
        Arc::new(ice_tools::OsExecutionEnv::with_cwd(root.to_path_buf()));
    crate::resource_dirs::load_skills_with_precedence(&env, dirs).await
}

fn roots(project: PathBuf, global: PathBuf) -> SkillSourceDirs {
    SkillSourceDirs {
        project: vec![project],
        global: vec![global],
        ..Default::default()
    }
}

#[tokio::test]
async fn missing_directory_is_empty() {
    let tmp = TempDir::new().unwrap();
    let result = load(tmp.path(), &[tmp.path().join("missing")]).await;
    assert!(
        CapabilityCatalog::from_live(&result, &SkillSourceDirs::default(), &[])
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn malformed_frontmatter_is_omitted() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("bad");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "---\nname: [broken\n---\nbody").unwrap();
    let result = load(tmp.path(), &[tmp.path().to_path_buf()]).await;
    assert!(!result.diagnostics.is_empty());
    assert!(
        CapabilityCatalog::from_live(&result, &SkillSourceDirs::default(), &[])
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn collision_retains_winner_and_loser() {
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("project");
    let global = tmp.path().join("global");
    write_skill(&project, "same", "same", "");
    write_skill(&global, "same", "same", "");
    let result = load(tmp.path(), &[project.clone(), global.clone()]).await;
    let catalog = CapabilityCatalog::from_live(&result, &roots(project, global), &[]);
    assert_eq!(catalog.entries.len(), 2);
    assert_eq!(catalog.entries[0].id, "skill:project:same");
    assert_eq!(catalog.entries[1].id, "skill:global:same");
    assert!(catalog.entries[0].available);
    assert!(!catalog.entries[1].available);
}

#[tokio::test]
async fn same_source_collision_has_unique_stable_ids() {
    let tmp = TempDir::new().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    write_skill(&a, "same", "same", "");
    write_skill(&b, "same", "same", "");
    let result = load(tmp.path(), &[a, b]).await;
    let dirs = SkillSourceDirs {
        project: vec![tmp.path().to_path_buf()],
        ..Default::default()
    };
    let catalog = CapabilityCatalog::from_live(&result, &dirs, &[]);
    assert_eq!(catalog.entries.len(), 2);
    assert_ne!(catalog.entries[0].id, catalog.entries[1].id);
    assert!(catalog
        .entries
        .iter()
        .all(|e| e.id.starts_with("skill:project:same:")));
    assert_eq!(catalog, CapabilityCatalog::from_live(&result, &dirs, &[]));
}

#[tokio::test]
async fn hidden_skill_is_present_but_not_shortlisted() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "hidden",
        "hidden",
        "disable-model-invocation: true\n",
    );
    let result = load(tmp.path(), &[tmp.path().to_path_buf()]).await;
    let catalog = CapabilityCatalog::from_live(&result, &SkillSourceDirs::default(), &[]);
    assert_eq!(catalog.entries.len(), 1);
    assert!(!catalog.entries[0].model_invocable);
    assert!(catalog.shortlist("hidden", 8).is_empty());
}

fn server(
    name: &str,
    status: ServerStatus,
    disabled: bool,
    catalog: Option<Vec<ToolInfo>>,
) -> ServerView {
    ServerView {
        name: name.into(),
        source: PathBuf::new(),
        status,
        disabled,
        catalog,
    }
}
fn tool() -> ToolInfo {
    ToolInfo {
        name: "find".into(),
        description: "search".into(),
        tokens: 0,
        direct: false,
        schema_hash: None,
    }
}

#[test]
fn disabled_server_tool_is_unavailable() {
    let servers = [server(
        "disabled",
        ServerStatus::Connected { tools: vec![] },
        true,
        Some(vec![tool()]),
    )];
    let catalog = CapabilityCatalog::from_live(
        &LoadSkillsResult::default(),
        &SkillSourceDirs::default(),
        &servers,
    );
    assert!(!catalog.entries[0].available);
    assert!(catalog.shortlist("find", 8).is_empty());
}

#[test]
fn mcp_schema_only_change_updates_capability_fingerprint() {
    let mut first = tool();
    first.direct = true;
    first.schema_hash = Some("schema-one".into());
    let mut second = first.clone();
    second.schema_hash = Some("schema-two".into());
    let make_catalog = |tool| {
        CapabilityCatalog::from_live(
            &LoadSkillsResult::default(),
            &SkillSourceDirs::default(),
            &[server(
                "remote",
                ServerStatus::Connected {
                    tools: vec!["remote_find".into()],
                },
                false,
                Some(vec![tool]),
            )],
        )
    };
    let old = make_catalog(first);
    let updated = make_catalog(second);
    assert_eq!(old.entries[0].name, updated.entries[0].name);
    assert_eq!(old.entries[0].description, updated.entries[0].description);
    assert_ne!(old.entries[0].content_hash, updated.entries[0].content_hash);
    assert!(old.entries[0].model_invocable && updated.entries[0].model_invocable);
}

#[test]
fn disconnected_and_unknown_catalog_are_unavailable() {
    let servers = [
        server(
            "failed",
            ServerStatus::Failed("offline".into()),
            false,
            Some(vec![tool()]),
        ),
        server(
            "unknown",
            ServerStatus::Failed("offline".into()),
            false,
            None,
        ),
    ];
    let catalog = CapabilityCatalog::from_live(
        &LoadSkillsResult::default(),
        &SkillSourceDirs::default(),
        &servers,
    );
    assert_eq!(catalog.entries.len(), 1);
    assert!(!catalog.entries[0].available);
    assert!(catalog.shortlist("find", 8).is_empty());
}

#[test]
fn descriptions_cap_unicode_safely() {
    let description = format!("{}😀suffix", "é".repeat(239));
    let entry = descriptor(
        "id".into(),
        CapabilityKind::Skill,
        "name",
        &description,
        "project",
        true,
        true,
    );
    assert_eq!(entry.description.chars().count(), 240);
    assert!(entry.description.ends_with('😀'));
}

#[test]
fn ranking_prioritizes_exact_name_and_breaks_ties_by_id() {
    let skills: Vec<_> = ["z", "a", "search"]
        .into_iter()
        .map(|name| {
            (
                "project",
                Skill {
                    name: name.into(),
                    description: "search".into(),
                    content: String::new(),
                    file_path: name.into(),
                    disable_model_invocation: None,
                },
            )
        })
        .collect();
    let catalog = CapabilityCatalog::build(&skills, &[]);
    let ids: Vec<_> = catalog
        .shortlist("search", 8)
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["skill:project:search", "skill:project:a", "skill:project:z"]
    );
}

#[test]
fn shortlist_obeys_limits() {
    let skills: Vec<_> = (0..10)
        .map(|i| {
            (
                "project",
                Skill {
                    name: format!("item{i}"),
                    description: "search".into(),
                    content: String::new(),
                    file_path: format!("{i}"),
                    disable_model_invocation: None,
                },
            )
        })
        .collect();
    let catalog = CapabilityCatalog::build(&skills, &[]);
    assert!(catalog.shortlist("search", 0).is_empty());
    assert_eq!(catalog.shortlist("search", 100).len(), 8);
}

#[tokio::test]
async fn refresh_tracks_added_and_removed_skill() {
    let tmp = TempDir::new().unwrap();
    let dirs = SkillSourceDirs {
        project: vec![tmp.path().to_path_buf()],
        ..Default::default()
    };
    let paths = [tmp.path().to_path_buf()];
    let mut catalog = CapabilityCatalog::from_live(&load(tmp.path(), &paths).await, &dirs, &[]);
    let file = write_skill(tmp.path(), "added", "added", "");
    catalog.refresh_live(&load(tmp.path(), &paths).await, &dirs, &[]);
    assert_eq!(catalog.entries.len(), 1);
    std::fs::remove_file(file).unwrap();
    catalog.refresh_live(&load(tmp.path(), &paths).await, &dirs, &[]);
    assert!(catalog.entries.is_empty());
}

#[tokio::test]
async fn rebuilt_ids_and_hashes_are_stable() {
    let tmp = TempDir::new().unwrap();
    write_skill(tmp.path(), "stable", "stable", "");
    let paths = [tmp.path().to_path_buf()];
    let dirs = SkillSourceDirs {
        project: paths.to_vec(),
        ..Default::default()
    };
    let first = CapabilityCatalog::from_live(&load(tmp.path(), &paths).await, &dirs, &[]);
    let second = CapabilityCatalog::from_live(&load(tmp.path(), &paths).await, &dirs, &[]);
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first, second);
}

#[test]
fn classifies_all_sources_and_prefers_specific_roots() {
    let tmp = TempDir::new().unwrap();
    let dirs = SkillSourceDirs {
        project: vec![tmp.path().join("project")],
        global: vec![tmp.path().join("global")],
        package: vec![tmp.path().join("project/package")],
        builtin: vec![tmp.path().join("builtin")],
        settings: vec![tmp.path().join("project/custom")],
    };
    for (path, expected) in [
        ("project/plain", "project"),
        ("global/plain", "global"),
        ("project/package/plain", "package"),
        ("builtin/plain", "builtin"),
        ("project/custom/plain", "settings"),
        ("other/plain", "other"),
    ] {
        assert_eq!(dirs.classify(&tmp.path().join(path)), expected);
    }
}
