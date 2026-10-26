use super::*;
use crate::mcp::elicitation::tests::next;
use futures::StreamExt;
#[path = "../../tests/support/mcp_fixture.rs"]
mod mcp_fixture;

async fn ui_session(dir: &Path) -> (AgentHarness, McpManager, UiDialogMailbox) {
    std::fs::write(
        dir.join("mcp.json"),
        serde_json::to_vec(&json!({"mcpServers":{"fake":{
            "command": mcp_fixture::server_executable(),
            "env": {"ICERS_MCP_TEST_CHILD":"1","ICERS_MCP_TEST_UI":"1"},
        }}}))
        .unwrap(),
    )
    .unwrap();
    let ui = UiDialogMailbox::new();
    ui.attach();
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let manager = McpManager::new(
        &harness,
        &Default::default(),
        dir,
        dir.into(),
        false,
        Default::default(),
    )
    .with_approval_ui(ui.clone())
    .with_elicitation_ui(ui.clone());
    (harness, manager, ui)
}

async fn call(harness: &AgentHarness, name: &str) {
    let tool = harness
        .get_tools()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.tool.schema().name == name)
        .unwrap();
    tool.tool
        .execute(
            "ui",
            json!({"mode":"show"}),
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
}

async fn post(url: &str, body: Value) -> Value {
    reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn ui_tools_open_a_guarded_window_that_bridges_the_app() {
    std::env::set_var("ICERS_MCP_UI_VIEWER", "none");
    let dir = tempfile::tempdir().unwrap();
    let (harness, manager, ui) = ui_session(dir.path()).await;
    let mut notices = manager.elicitation_notices();
    let mut prompts = manager.ui_prompts().unwrap();
    manager.reconnect(None).await.unwrap();

    // The app-only tool is neither registered nor offered through the proxy.
    let names: Vec<String> = harness
        .get_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.tool.schema().name.clone())
        .collect();
    assert!(names.contains(&"fake_show".to_string()));
    assert!(!names.contains(&"fake_refresh".to_string()));
    assert!(manager
        .proxy_view()
        .iter()
        .flat_map(|s| &s.entries)
        .all(|e| e.remote != "refresh"));

    call(&harness, "fake_show").await;
    let notice = tokio::time::timeout(Duration::from_secs(5), notices.recv())
        .await
        .unwrap()
        .unwrap();
    let url = notice.split("ready at ").nth(1).unwrap().to_owned();
    let (page, token) = url.split_once("?t=").unwrap();

    let host = reqwest::get(&url).await.unwrap();
    assert_eq!(host.status(), 200);
    assert!(host
        .text()
        .await
        .unwrap()
        .contains("sandbox=\"allow-scripts allow-forms\""));
    assert_eq!(reqwest::get(page).await.unwrap().status(), 403);
    let rebound = reqwest::Client::new()
        .get(&url)
        .header("host", "evil.test")
        .send()
        .await
        .unwrap();
    assert_eq!(rebound.status(), 403);

    let app = reqwest::get(format!("{page}/app?t={token}")).await.unwrap();
    let policy = app.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        policy.contains("connect-src https://api.fake.test"),
        "{policy}"
    );
    assert!(
        policy.contains("https://cdn.fake.test") && !policy.contains("bad"),
        "{policy}"
    );
    assert_eq!(app.text().await.unwrap(), "<p>fake app</p>");

    // The event stream replays the call's input and result.
    let events = reqwest::get(format!("{page}/events?t={token}"))
        .await
        .unwrap();
    let mut stream = events.bytes_stream();
    let mut seen = String::new();
    while !seen.contains("event: tool-result") {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(seen.contains("event: tool-input"), "{seen}");

    // The app may call its server's tools after the user consents, once.
    let rpc = format!("{page}/rpc?t={token}");
    let first = tokio::spawn({
        let rpc = rpc.clone();
        async move {
            post(
                &rpc,
                json!({"method":"tools/call","params":{"name":"refresh","arguments":{}}}),
            )
            .await
        }
    });
    let dialog = next(&ui).await;
    assert!(dialog.ui["summary"].as_str().unwrap().contains("fake app"));
    ui.respond(&dialog.request_id, json!({"values":["Allow"]}))
        .unwrap();
    assert_eq!(
        first.await.unwrap()["result"]["content"][0]["text"],
        "hello"
    );
    let again = post(
        &rpc,
        json!({"method":"tools/call","params":{"name":"refresh","arguments":{}}}),
    )
    .await;
    assert_eq!(again["result"]["content"][0]["text"], "hello");
    let refused = post(&rpc, json!({"method":"sampling/createMessage","params":{}})).await;
    assert_eq!(refused["error"]["code"], -32601);

    // A prompt from the app wakes the agent and is queued for ui-messages.
    post(
        &format!("{page}/message?t={token}"),
        json!({"method":"ui/message","params":{"role":"user","content":[{"type":"text","text":"Plot it"}]}}),
    )
    .await;
    let prompt = tokio::time::timeout(Duration::from_secs(5), prompts.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(prompt, "[fake app] Plot it");
    let queued = manager.take_ui_messages();
    assert_eq!(queued[0]["prompt"], "Plot it");
    assert!(manager.take_ui_messages().is_empty());

    // Calling the tool again updates the open window instead of opening another.
    call(&harness, "fake_show").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let sessions = manager
        .0
        .ui
        .sessions
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].log.lock().unwrap().len(), 4);
    assert!(notices.try_recv().is_err());

    post(
        &format!("{page}/complete?t={token}"),
        json!({"reason":"done"}),
    )
    .await;
    assert_eq!(reqwest::get(&url).await.unwrap().status(), 404);
}

#[test]
fn ui_meta_and_csp_parse_conservatively() {
    let tool = json!({"_meta":{"ui":{"resourceUri":"ui://a/b","visibility":["app"]}}});
    assert_eq!(
        meta(&tool),
        Some(UiMeta {
            resource_uri: "ui://a/b".into(),
            app_only: true
        })
    );
    assert!(
        !meta(&json!({"_meta":{"ui":{"resourceUri":"ui://a/b","visibility":["model","app"]}}}))
            .unwrap()
            .app_only
    );
    assert_eq!(
        meta(&json!({"_meta":{"ui":{"resourceUri":"https://a/b"}}})),
        None
    );
    assert_eq!(meta(&json!({"name":"plain"})), None);
    let policy = csp(&json!({"connectDomains":["https://ok.test","x' 'unsafe-eval"]}));
    assert!(policy.contains("connect-src https://ok.test;"), "{policy}");
    assert!(!policy.contains("unsafe-eval"));
}
