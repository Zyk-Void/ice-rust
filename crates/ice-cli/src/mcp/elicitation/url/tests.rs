use super::*;
use crate::mcp::elicitation::tests::next;

fn setup(
    fail: bool,
) -> (
    Arc<Elicitation>,
    Arc<Mutex<Vec<String>>>,
    tokio::sync::broadcast::Receiver<String>,
) {
    let ui = UiDialogMailbox::new();
    ui.attach();
    let (notices, receiver) = tokio::sync::broadcast::channel(8);
    let opened = Arc::new(Mutex::new(Vec::new()));
    let calls = opened.clone();
    let mut url = UrlElicitation::new(notices);
    url.browser = Arc::new(move |url| {
        calls.lock().unwrap().push(url.into());
        if fail {
            Err("launcher failed".into())
        } else {
            Ok(())
        }
    });
    (
        Arc::new(Elicitation {
            server: "fake".into(),
            ui,
            url,
        }),
        opened,
        receiver,
    )
}
fn request() -> Value {
    json!({"mode":"url","message":"Sign in","url":"https://example.test/login?token=opaque","elicitationId":"login"})
}
#[tokio::test]
async fn consent_launches_once_and_only_accepted_ids_complete() {
    let (state, opened, mut notices) = setup(false);
    state
        .url
        .complete("fake", &json!({"elicitationId":"login"}));
    assert!(notices.try_recv().is_err());
    let child = state.clone();
    let task = tokio::spawn(async move {
        child
            .handle(request(), &CancellationToken::new())
            .await
            .unwrap()
    });
    let dialog = next(&state.ui).await;
    assert!(opened.lock().unwrap().is_empty());
    let question = dialog.ui["question"].as_str().unwrap();
    assert!(question.contains("Host: example.test"));
    assert!(question.contains("Full URL: https://example.test/login?token=opaque"));
    assert!(state
        .handle(request(), &CancellationToken::new())
        .await
        .is_err());
    state
        .url
        .complete("fake", &json!({"elicitationId":"login"}));
    assert!(notices.try_recv().is_err());
    state
        .ui
        .respond(&dialog.request_id, json!({"values":["Open"]}))
        .unwrap();
    assert_eq!(task.await.unwrap(), json!({"action":"accept"}));
    assert_eq!(
        opened.lock().unwrap().as_slice(),
        ["https://example.test/login?token=opaque"]
    );
    state
        .url
        .complete("fake", &json!({"elicitationId":"unknown"}));
    assert!(notices.try_recv().is_err());
    state
        .url
        .complete("fake", &json!({"elicitationId":"login"}));
    assert!(notices.try_recv().unwrap().contains("retry the tool"));
    state
        .url
        .complete("fake", &json!({"elicitationId":"login"}));
    assert!(notices.try_recv().is_err());
}
#[tokio::test]
async fn decline_cancel_drop_and_launcher_failure_release_ids() {
    for choice in ["Decline", "Cancel", "Open", "drop", "abort"] {
        let (state, opened, mut notices) = setup(choice == "Open");
        let token = CancellationToken::new();
        let signal = token.clone();
        let child = state.clone();
        let task = tokio::spawn(async move { child.handle(request(), &signal).await.unwrap() });
        let dialog = next(&state.ui).await;
        if choice == "drop" {
            task.abort();
            assert!(task.await.is_err());
        } else {
            if choice == "abort" {
                token.cancel();
            } else {
                state
                    .ui
                    .respond(&dialog.request_id, json!({"values":[choice]}))
                    .unwrap();
            }
            assert_eq!(
                task.await.unwrap()["action"],
                if choice == "Decline" {
                    "decline"
                } else {
                    "cancel"
                }
            );
        }
        assert!(state.url.ids.lock().unwrap().is_empty());
        assert_eq!(state.ui.pending_len(), 0);
        assert_eq!(opened.lock().unwrap().len(), usize::from(choice == "Open"));
        if choice == "Open" {
            assert!(notices.try_recv().unwrap().contains("Could not open"));
        }
        state
            .url
            .complete("fake", &json!({"elicitationId":"login"}));
        assert!(notices.try_recv().is_err());
    }
}
#[tokio::test]
async fn rejects_unsafe_urls_and_bounds_tracking_before_dialogs() {
    let (state, opened, _) = setup(false);
    for raw in [
        "file:///tmp/a",
        "javascript:alert(1)",
        "https://user:pass@example.test",
        "https://example.test/\n",
        " https://example.test",
        "not a URL",
    ] {
        let mut params = request();
        params["url"] = json!(raw);
        assert!(state
            .handle(params, &CancellationToken::new())
            .await
            .is_err());
    }
    for id in [
        json!(null),
        json!(""),
        json!("bad\n"),
        json!("x".repeat(257)),
    ] {
        let mut params = request();
        params["elicitationId"] = id;
        assert!(state
            .handle(params, &CancellationToken::new())
            .await
            .is_err());
    }
    for n in 0..128 {
        state.url.ids.lock().unwrap().insert(n.to_string(), true);
    }
    assert!(state
        .handle(request(), &CancellationToken::new())
        .await
        .is_err());
    assert_eq!(state.ui.pending_len(), 0);
    assert!(opened.lock().unwrap().is_empty());
}
#[tokio::test]
async fn completion_notification_routes_through_transport() {
    let dir = tempfile::tempdir().unwrap();
    let config: ServerConfig = serde_json::from_value(json!({"command":crate::mcp::elicitation::tests::mcp_fixture::server_executable(),"env":{"ICERS_MCP_TEST_CHILD":"1"}})).unwrap();
    let (notices, mut receiver) = tokio::sync::broadcast::channel(8);
    let ui = UiDialogMailbox::new();
    let mut handlers = register(Handlers::default(), "fake", ui, notices.clone());
    // Seed accepted state without launching a real browser.
    let url = UrlElicitation::new(notices);
    url.ids.lock().unwrap().insert("login".into(), true);
    handlers.elicitation_complete = Some(Arc::new(move |params| url.complete("fake", &params)));
    let client = Client::spawn("fake", &config, dir.path(), handlers).unwrap();
    client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    client
        .request(
            "tools/call",
            json!({"name":"echo","arguments":{"mode":"elicit_complete","elicitationId":"login"}}),
            CancellationToken::new(),
            CALL_TIMEOUT,
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap()
            .contains("retry the tool")
    );
}
