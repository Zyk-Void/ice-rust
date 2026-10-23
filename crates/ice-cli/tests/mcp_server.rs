//! Offline MCP CLI integration tests.

#[tokio::test]
async fn manager_reconnect_reloads_approval_policy_without_duplicates() {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    use ice_cli::mcp::{McpManager, ServerStatus};
    use ice_harness::{agent_harness::AgentHarness, types::AgentHarnessOptions};
    use std::sync::{Arc, Mutex};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.json");
    let write = |approval: bool| {
        std::fs::write(
            &path,
            json!({"mcpServers":{"fake":{
                "command":mcp_fixture::server_executable(),
                "env":{"ICERS_MCP_TEST_CHILD":"1"}, "approveTools":approval
            }}})
            .to_string(),
        )
        .unwrap();
    };
    write(true);
    let provider = FauxProvider::new(FauxScript::new());
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        ..Default::default()
    })
    .await
    .unwrap();
    let approvals = Arc::new(Mutex::new(Default::default()));
    let manager = McpManager::new(
        &harness,
        &ice_cli::args::Args::default(),
        dir.path(),
        dir.path().into(),
        false,
        approvals.clone(),
    );
    manager.reconnect(None).await.unwrap();
    assert!(matches!(
        manager.snapshot()[0].status,
        ServerStatus::Connected { .. }
    ));
    assert!(approvals.lock().unwrap().contains("fake_echo"));
    write(false);
    manager.reconnect(None).await.unwrap();
    assert!(approvals.lock().unwrap().is_empty());
    assert_eq!(manager.tools("fake").len(), 2);
    assert_eq!(harness.get_tools().await.unwrap().len(), 2);
    std::fs::write(&path, "{\"mcpServers\":{}}").unwrap();
    manager.reconnect(None).await.unwrap();
    assert!(manager.snapshot().is_empty());
    assert!(harness.get_tools().await.unwrap().is_empty());
    assert!(harness
        .lane("main")
        .get_active_tools()
        .await
        .unwrap()
        .is_empty());
}
use serde_json::json;
#[path = "support/mcp_fixture.rs"]
mod mcp_fixture;

#[test]
fn list_mcp_cli_trust_disabled_and_failure() {
    let dir = tempfile::tempdir().unwrap();
    let agent = dir.path().join("agent");
    let project = dir.path().join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(project.join(".icers")).unwrap();
    let fake = mcp_fixture::server_executable();
    std::fs::write(
        agent.join("mcp.json"),
        json!({"mcpServers":{
            "fake":{"command":fake,"env":{"ICERS_MCP_TEST_CHILD":"1"},"approveTools":["echo"]},
            "disabled":{"disabled":true},
            "failed":{"command":"icers-nonexistent-mcp-test-command"}
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        project.join(".icers/mcp.json"),
        json!({"mcpServers":{"fake":{"disabled":true}}}).to_string(),
    )
    .unwrap();
    let run = |trust: &str| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_icers"))
            .args(["--list-mcp", trust])
            .current_dir(&project)
            .env("ICERS_CODING_AGENT_DIR", &agent)
            .env("ICERS_OFFLINE", "1")
            .output()
            .unwrap();
        output
    };
    let output = run("--no-approve");
    assert!(
        !output.status.success(),
        "failed discovery must fail inspection"
    );
    let untrusted = String::from_utf8(output.stdout).unwrap();
    assert!(untrusted.contains("fake: ready"), "{untrusted}");
    assert!(untrusted.contains("fake_echo (approval required)"));
    assert!(untrusted.contains("fake_other"));
    assert!(untrusted.contains("disabled: disabled"));
    assert!(untrusted.contains("failed: failed"));
    let output = run("--approve");
    assert!(!output.status.success());
    let trusted = String::from_utf8(output.stdout).unwrap();
    assert!(trusted.contains("fake: disabled"), "{trusted}");

    std::fs::write(
        agent.join("mcp.json"),
        json!({"mcpServers":{
            "fake":{"command":fake,"env":{"ICERS_MCP_TEST_CHILD":"1"}},
            "disabled":{"disabled":true}
        }})
        .to_string(),
    )
    .unwrap();
    for trust in ["--no-approve", "--approve"] {
        let output = run(trust);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = String::from_utf8(output.stdout).unwrap();
        assert!(report.contains("disabled: disabled"), "{report}");
        let expected = if trust == "--approve" {
            "disabled"
        } else {
            "ready"
        };
        assert!(report.contains(&format!("fake: {expected}")), "{report}");
    }
}
