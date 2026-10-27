use super::*;
#[path = "../../tests/support/mcp_fixture.rs"]
mod mcp_fixture;

fn entry(server: &str, remote: &str, description: &str) -> Entry {
    Entry {
        name: format!("{server}_{remote}"),
        remote: remote.into(),
        description: description.into(),
        schema: json!({"type":"object","properties":{"q":{"type":"string"}}}),
        resource_uri: None,
    }
}
fn server(name: &str, status: ServerStatus, entries: Vec<Entry>, config: Value) -> Server {
    Server {
        name: name.into(),
        status,
        cached: false,
        entries,
        instructions: Some("Use search first.".into()),
        config: serde_json::from_value(config).unwrap(),
    }
}
fn connected() -> ServerStatus {
    ServerStatus::Connected { tools: vec![] }
}
fn sample() -> Vec<Server> {
    vec![
        server(
            "github",
            connected(),
            vec![
                entry("github", "search_code", "Search code in repositories"),
                entry("github", "create-issue", "Open a new issue"),
                entry("github", "list_issues", "List issues"),
            ],
            json!({"command":"gh","approveTools":["create-issue"],"searchKeywords":{"search_code":["grep"],"*":["gh"]}}),
        ),
        server(
            "notes",
            connected(),
            vec![entry("notes", "search", "Search notes")],
            json!({"command":"n"}),
        ),
        server(
            "off",
            ServerStatus::Disabled,
            vec![entry("off", "search_all", "Search everything")],
            json!({"command":"o","disabled":true}),
        ),
    ]
}
fn body(result: &AgentToolResult) -> &str {
    match &result.content[0] {
        TextContentOrImage::Text(text) => &text.text,
        _ => panic!("expected text"),
    }
}

#[test]
fn search_ranks_pages_and_uses_keywords_but_skips_disabled_servers() {
    let servers = sample();
    let result = search(&servers, "search", None, 12, 0);
    let tools: Vec<String> = serde_json::from_value(result.details["tools"].clone()).unwrap();
    // An exact remote-name hit outranks a name prefix; disabled servers never show.
    assert_eq!(tools, ["notes_search", "github_search_code"]);
    assert!(!body(&result).contains("off_"));

    let grep = search(&servers, "grep", None, 12, 0);
    assert_eq!(grep.details["tools"], json!(["github_search_code"]));
    let gh = search(&servers, "gh", None, 2, 0);
    assert_eq!(gh.details["total"], 3);
    assert_eq!(gh.details["nextOffset"], 2);
    assert!(body(&gh).contains("offset: 2"));
    assert_eq!(
        search(&servers, "gh", None, 2, 2).details["nextOffset"],
        Value::Null
    );
    assert!(body(&search(&servers, "zzz", None, 12, 0)).contains("No MCP tools match"));
    assert!(body(&search(&servers, "search", Some("notes"), 12, 0)).contains("1 of 1"));
}

#[test]
fn names_resolve_fuzzily_and_report_ambiguity_and_suggestions() {
    let servers = sample();
    assert!(
        matches!(resolve(&servers, "github_create_issue", None), Found::One(_, e) if e.remote == "create-issue")
    );
    assert!(
        matches!(resolve(&servers, "LIST-ISSUES", None), Found::One(_, e) if e.name == "github_list_issues")
    );
    // `search` is notes' remote name and a prefix of nothing else exactly.
    assert!(matches!(
        resolve(&servers, "search", Some("notes")),
        Found::One(..)
    ));
    let ambiguous = vec![
        server(
            "a",
            connected(),
            vec![entry("a", "run", "")],
            json!({"command":"a"}),
        ),
        server(
            "b",
            connected(),
            vec![entry("b", "run", "")],
            json!({"command":"b"}),
        ),
    ];
    let result = describe(&ambiguous, "run");
    assert_eq!(result.details["error"], "ambiguous");
    let missing = describe(&servers, "github_search_cod");
    assert_eq!(missing.details["error"], "tool_not_found");
    assert!(
        body(&missing).contains("github_search_code"),
        "{}",
        body(&missing)
    );
}

#[test]
fn status_list_describe_and_instructions_render_clean_text() {
    let mut servers = sample();
    servers[1].name = "no\u{1b}tes".into();
    let status = status(&servers);
    assert!(body(&status).starts_with("MCP: 2/2 servers, 4 tools (1 disabled)"));
    assert!(body(&status).contains("⊘ off (disabled)"));
    assert!(!body(&status).contains('\u{1b}'));
    let listed = list(&servers, "github");
    assert!(body(&listed).contains("github_create-issue (requires approval) — Open a new issue"));
    assert!(body(&listed).contains("Instructions: Use search first."));
    assert_eq!(list(&servers, "nope").details["error"], "server_not_found");
    let described = describe(&servers, "github_search_code");
    assert!(body(&described).contains("Server: github"));
    assert!(body(&described).contains("\"q\""));
    assert_eq!(body(&instructions(&servers, "github")), "Use search first.");
}

#[test]
fn globs_match_like_pma_keywords() {
    assert!(glob("*", "anything"));
    assert!(glob("search_*", "search_code"));
    assert!(glob("*_code", "search_code"));
    assert!(glob("s*r*e", "search_code"));
    assert!(!glob("search_*", "list_issues"));
    assert!(glob("exact", "exact") && !glob("exact", "exactly"));
}

async fn proxy_manager(dir: &Path, server: Value) -> (AgentHarness, McpManager) {
    std::fs::write(
        dir.join("mcp.json"),
        serde_json::to_vec(&json!({"mcpServers":{"fake":server}})).unwrap(),
    )
    .unwrap();
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let manager = McpManager::new(
        &harness,
        &Default::default(),
        dir,
        dir.into(),
        false,
        Default::default(),
    );
    manager.reconnect(None).await.unwrap();
    (harness, manager)
}

async fn run(harness: &AgentHarness, params: Value) -> Result<AgentToolResult, AgentError> {
    let proxy = harness
        .get_tools()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.tool.schema().name == NAME)
        .expect("mcp proxy registered");
    proxy
        .tool
        .execute(
            "proxy-call",
            params,
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
}

#[tokio::test]
async fn proxy_surfaces_remote_errors_and_timeouts() {
    let dir = tempfile::tempdir().unwrap();
    let command = mcp_fixture::server_executable();
    let (harness, _manager) = proxy_manager(
        dir.path(),
        json!({"command":command,"env":{"ICERS_MCP_TEST_CHILD":"1"},"directTools":false,"requestTimeoutMs":50}),
    )
    .await;
    for (mode, expected) in [
        ("cognee_404", "HTTP 404"),
        ("json_rpc_error", "fixture error"),
        ("timeout", "timed out"),
    ] {
        let error = run(&harness, json!({"tool":"echo","args":{"mode":mode}}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{mode}: {error}");
    }
}

#[tokio::test]
async fn the_proxy_calls_tools_that_are_not_registered_directly() {
    let dir = tempfile::tempdir().unwrap();
    let command = mcp_fixture::server_executable();
    let (harness, manager) = proxy_manager(
        dir.path(),
        json!({"command":command,"env":{"ICERS_MCP_TEST_CHILD":"1"},"directTools":false}),
    )
    .await;
    let names: Vec<String> = harness
        .get_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.tool.schema().name.clone())
        .collect();
    assert!(names.contains(&NAME.to_string()));
    assert!(!names.iter().any(|n| n.starts_with("fake_")));

    let result = run(&harness, json!({"tool":"echo","args":"{\"mode\":\"pid\"}"}))
        .await
        .unwrap();
    assert!(body(&result).parse::<u32>().is_ok(), "{}", body(&result));
    let status = run(&harness, json!({})).await.unwrap();
    assert!(
        body(&status).contains("✓ fake (2 tools)"),
        "{}",
        body(&status)
    );
    assert!(run(&harness, json!({"tool":"echo","args":[1]}))
        .await
        .is_err());

    // Removing every server removes the proxy too.
    std::fs::write(dir.path().join("mcp.json"), "{}").unwrap();
    manager.reconnect(None).await.unwrap();
    assert!(!harness
        .get_tools()
        .await
        .unwrap()
        .iter()
        .any(|t| t.tool.schema().name == NAME));
}

#[tokio::test]
async fn proxied_calls_need_approval_like_direct_ones() {
    let dir = tempfile::tempdir().unwrap();
    let command = mcp_fixture::server_executable();
    let (harness, _manager) = proxy_manager(
        dir.path(),
        json!({"command":command,"env":{"ICERS_MCP_TEST_CHILD":"1"},"directTools":false,"approveTools":["echo"]}),
    )
    .await;
    // No approval UI is installed, so the call is refused rather than run.
    let error = run(&harness, json!({"tool":"fake_echo"}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("approval denied"), "{error}");
    let other = run(&harness, json!({"tool":"other"})).await.unwrap();
    assert!(body(&other).contains("hello"), "{}", body(&other));
}
