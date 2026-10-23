//! Offline MCP CLI integration tests.
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
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let untrusted = run("--no-approve");
    assert!(untrusted.contains("fake: ready"), "{untrusted}");
    assert!(untrusted.contains("fake_echo (approval required)"));
    assert!(untrusted.contains("fake_other"));
    assert!(untrusted.contains("disabled: disabled"));
    assert!(untrusted.contains("failed: failed"));
    let trusted = run("--approve");
    assert!(trusted.contains("fake: disabled"), "{trusted}");
}
