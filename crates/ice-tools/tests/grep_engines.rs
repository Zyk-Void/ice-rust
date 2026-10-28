use std::sync::Arc;

use ice_agent::types::TextContentOrImage;
use ice_tools::{ExecutionToolContext, OsExecutionEnv};
use tokio_util::sync::CancellationToken;

fn tool(dir: &std::path::Path) -> Arc<dyn ice_agent::agent_tool::AgentTool> {
    let env: Arc<dyn ice_tools::ExecutionEnv> =
        Arc::new(OsExecutionEnv::with_cwd(dir.to_path_buf()));
    ice_tools::create_grep_tool(&ExecutionToolContext::new(env, None), None)
}

async fn run(
    tool: Arc<dyn ice_agent::agent_tool::AgentTool>,
    params: serde_json::Value,
) -> Result<ice_agent::types::AgentToolResult, ice_agent::error::AgentError> {
    tool.execute("test", params, CancellationToken::new(), Arc::new(|_| ()))
        .await
}

fn text(result: &ice_agent::types::AgentToolResult) -> &str {
    match &result.content[0] {
        TextContentOrImage::Text(value) => &value.text,
        _ => panic!("expected text"),
    }
}

#[tokio::test]
async fn text_mode_searches_real_files_with_ripgrep() {
    if std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("source.rs"), "before\nneedle\nafter\n").unwrap();
    let result = run(tool(dir.path()), serde_json::json!({"pattern":"needle"}))
        .await
        .unwrap();
    assert_eq!(result.details["engine"], "rg");
    assert!(
        text(&result).contains("source.rs:2: needle"),
        "{}",
        text(&result)
    );
}

#[tokio::test]
async fn ripgrep_limit_counts_matches_not_context_lines() {
    if std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("source.rs"),
        "before\nneedle one\nafter\nother\nneedle two\n",
    )
    .unwrap();
    let result = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"needle", "context":1, "limit":1}),
    )
    .await
    .unwrap();
    assert!(text(&result).contains("needle one"), "{}", text(&result));
    assert!(text(&result).contains("after"), "{}", text(&result));
    assert!(!text(&result).contains("needle two"), "{}", text(&result));
    assert_eq!(result.details["truncated"], true);
}

#[tokio::test]
async fn invalid_modes_and_missing_language_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let err = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"x", "mode":"ast"}),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("requires language"), "{err}");
    let err = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"x", "mode":"unknown"}),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("grep input invalid"), "{err}");
    let err = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"x", "mode":"ast", "language":"rust", "literal":true}),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("not supported in ast mode"),
        "{err}"
    );
}

#[tokio::test]
async fn virtual_environment_keeps_text_fallback_and_rejects_external_modes() {
    let env = Arc::new(ice_tools::InMemoryExecutionEnv::with_cwd("/work".into()));
    env.seed_file("/work/a.txt", b"needle\n".to_vec()).await;
    let dyn_env: Arc<dyn ice_tools::ExecutionEnv> = env;
    let tool = ice_tools::create_grep_tool(&ExecutionToolContext::new(dyn_env, None), None);
    let result = run(tool.clone(), serde_json::json!({"pattern":"needle"}))
        .await
        .unwrap();
    assert!(text(&result).contains("a.txt:1: needle"));
    let err = run(
        tool,
        serde_json::json!({"pattern":"needle", "mode":"archive"}),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("ugrep is unavailable"), "{err}");
}

#[tokio::test]
async fn ast_mode_searches_syntax_when_installed() {
    if std::process::Command::new("ast-grep")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("sample.rs"),
        "fn main() { example(42); }\n// example(12)\n",
    )
    .unwrap();
    let result = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"example($ARG)","mode":"ast","language":"rust"}),
    )
    .await
    .unwrap();
    assert_eq!(result.details["engine"], "ast-grep");
    assert!(
        text(&result).contains("sample.rs:1: example(42)"),
        "{}",
        text(&result)
    );
    assert!(!text(&result).contains("example(12)"));
}

#[tokio::test]
async fn archive_mode_searches_tar_without_extracting() {
    if std::process::Command::new("ugrep")
        .arg("--version")
        .output()
        .is_err()
        || std::process::Command::new("tar")
            .arg("--version")
            .output()
            .is_err()
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("inside.txt"), "archive needle\n").unwrap();
    let archive = dir.path().join("sample.tar");
    assert!(std::process::Command::new("tar")
        .args([
            "-cf",
            archive.to_str().unwrap(),
            "-C",
            source.path().to_str().unwrap(),
            "inside.txt"
        ])
        .status()
        .unwrap()
        .success());
    let result = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"archive needle", "mode":"archive", "path":"sample.tar"}),
    )
    .await
    .unwrap();
    assert_eq!(result.details["engine"], "ugrep");
    assert!(
        text(&result).contains("archive needle"),
        "{}",
        text(&result)
    );
}

#[tokio::test]
async fn missing_ripgrep_falls_back_and_missing_ugrep_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("source.txt"), "needle\n").unwrap();
    let mut variables = std::collections::HashMap::new();
    variables.insert(
        "PATH".to_string(),
        dir.path().to_string_lossy().into_owned(),
    );
    let env: Arc<dyn ice_tools::ExecutionEnv> =
        Arc::new(OsExecutionEnv::with_cwd(dir.path().to_path_buf()).with_shell_env(variables));
    let tool = ice_tools::create_grep_tool(&ExecutionToolContext::new(env, None), None);
    let result = run(tool.clone(), serde_json::json!({"pattern":"needle"}))
        .await
        .unwrap();
    assert!(
        text(&result).contains("source.txt:1: needle"),
        "{}",
        text(&result)
    );
    let error = run(
        tool,
        serde_json::json!({"pattern":"needle", "mode":"archive"}),
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("ugrep is unavailable"),
        "{error}"
    );
}

#[tokio::test]
async fn already_cancelled_search_does_not_start() {
    let dir = tempfile::tempdir().unwrap();
    let signal = CancellationToken::new();
    signal.cancel();
    let err = tool(dir.path())
        .execute(
            "test",
            serde_json::json!({"pattern":"x"}),
            signal,
            Arc::new(|_| ()),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("abort"), "{err}");
}

fn has_ripgrep() -> bool {
    std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_ok()
}

#[tokio::test]
async fn ripgrep_searches_several_paths_with_asymmetric_context() {
    if !has_ripgrep() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.rs"), "a0\nneedle a\na2\na3\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "b0\nneedle b\nb2\n").unwrap();
    std::fs::write(dir.path().join("skipped.rs"), "needle c\n").unwrap();
    let result = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"needle", "path":["src", "b.rs"], "before":0, "after":2}),
    )
    .await
    .unwrap();
    let out = text(&result);
    assert!(out.contains("src/a.rs:2: needle a"), "{out}");
    assert!(out.contains("src/a.rs-4- a3"), "{out}");
    assert!(!out.contains("a0"), "{out}");
    assert!(out.contains("b.rs:2: needle b"), "{out}");
    assert!(!out.contains("needle c"), "{out}");
}

#[tokio::test]
async fn ripgrep_context_line_that_looks_like_a_match_is_not_counted() {
    if !has_ripgrep() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("log.txt"),
        "needle one\n12:30:45 tick\nneedle two\n",
    )
    .unwrap();
    let result = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"needle", "after":1, "limit":1}),
    )
    .await
    .unwrap();
    let out = text(&result);
    assert!(out.contains("log.txt-2- 12:30:45 tick"), "{out}");
    assert!(!out.contains("needle two"), "{out}");
    assert!(out.contains("1 matches limit reached"), "{out}");
}

#[cfg(unix)]
#[tokio::test]
async fn ripgrep_keeps_results_when_a_path_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    if !has_ripgrep() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ok.txt"), "needle\n").unwrap();
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root can read anything, so there is nothing to skip.
    let readable = std::fs::read_dir(&locked).is_ok();
    let result = run(tool(dir.path()), serde_json::json!({"pattern":"needle"})).await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return;
    }
    let result = result.unwrap();
    let out = text(&result);
    assert!(out.contains("ok.txt:1: needle"), "{out}");
    assert!(out.contains("Skipped 1 unreadable path(s)"), "{out}");
    assert!(!out.contains(&dir.path().display().to_string()), "{out}");
    assert_eq!(result.details["skippedPaths"], 1);
}

/// A FIFO given as a path never ends, so only stopping rg at the limit lets
/// the search finish before the 30s timeout.
#[cfg(unix)]
#[tokio::test]
async fn ripgrep_stops_once_the_limit_is_reached() {
    if !has_ripgrep() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "needle 1\nneedle 2\n").unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(dir.path().join("pipe"))
        .status();
    if !status.is_ok_and(|status| status.success()) {
        return;
    }
    let started = std::time::Instant::now();
    let result = run(
        tool(dir.path()),
        serde_json::json!({"pattern":"needle", "path":["pipe", "a.txt"], "limit":1}),
    )
    .await
    .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    let out = text(&result);
    assert!(out.contains("a.txt:1: needle 1"), "{out}");
    assert!(out.contains("1 matches limit reached"), "{out}");
}
