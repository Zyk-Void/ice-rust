use super::*;
use futures::task::AtomicWaker;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};
use tokio::io::{AsyncWrite, DuplexStream};

#[derive(Default)]
struct AckGate {
    blocked: AtomicBool,
    attempted: CancellationToken,
    waker: AtomicWaker,
}

impl AckGate {
    fn release(&self) {
        self.blocked.store(false, Ordering::Release);
        self.waker.wake();
    }
}

struct GatedInput {
    inner: DuplexStream,
    ack: Arc<AckGate>,
}

impl AsyncWrite for GatedInput {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if bytes
            .windows(b"client_settled_ack".len())
            .any(|part| part == b"client_settled_ack")
            && self.ack.blocked.load(Ordering::Acquire)
        {
            self.ack.waker.register(cx.waker());
            self.ack.attempted.cancel();
            if self.ack.blocked.load(Ordering::Acquire) {
                return Poll::Pending;
            }
        }
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct Fixture {
    session: Arc<Session>,
    first_started: CancellationToken,
    first_finish: CancellationToken,
    ack: Arc<AckGate>,
    prompts: Arc<AtomicUsize>,
}

async fn fixture() -> Fixture {
    fixture_with_id(uuid::Uuid::new_v4().to_string()).await
}

async fn fixture_with_id(session_id: String) -> Fixture {
    let first_started = CancellationToken::new();
    let first_finish = CancellationToken::new();
    let ack = Arc::new(AckGate::default());
    ack.blocked.store(true, Ordering::Release);
    let prompts = Arc::new(AtomicUsize::new(0));
    let spawner: Spawner = {
        let first_started = first_started.clone();
        let first_finish = first_finish.clone();
        let ack = ack.clone();
        let prompts = prompts.clone();
        Arc::new(move |_, _, _| {
            let (input, child_input) = tokio::io::duplex(8192);
            let (mut child_output, output) = tokio::io::duplex(8192);
            let started = first_started.clone();
            let finish = first_finish.clone();
            let prompts = prompts.clone();
            tokio::spawn(async move {
                child_output
                    .write_all(b"{\"type\":\"ready\"}\n")
                    .await
                    .unwrap();
                let mut lines = tokio::io::BufReader::new(child_input).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let frame: Value = serde_json::from_str(&line).unwrap();
                    match frame["type"].as_str() {
                        Some("prompt") => {
                            let index = prompts.fetch_add(1, Ordering::SeqCst) + 1;
                            let accepted = json!({"type":"response","id":frame["id"],"success":true,"data":{"accepted":true}});
                            child_output
                                .write_all(format!("{accepted}\n").as_bytes())
                                .await
                                .unwrap();
                            if index == 1 {
                                started.cancel();
                                finish.cancelled().await;
                            }
                            let settled = json!({"type":"agent_settled","origin":"client","outcome":"completed","settlementId":format!("receipt-{index}")});
                            child_output
                                .write_all(format!("{settled}\n").as_bytes())
                                .await
                                .unwrap();
                        }
                        Some("get_state") => {
                            let state = json!({"type":"response","id":frame["id"],"success":true,"data":{"context":{"tokens":0,"window":0}}});
                            child_output
                                .write_all(format!("{state}\n").as_bytes())
                                .await
                                .unwrap();
                        }
                        Some("stop") => break,
                        _ => {}
                    }
                }
            });
            Ok(ChildIo {
                stdin: Box::new(GatedInput {
                    inner: input,
                    ack: ack.clone(),
                }),
                stdout: Box::new(tokio::io::BufReader::new(output)),
                process: None,
            })
        })
    };
    let session = Session::start(
        session_id,
        PathBuf::from("/test"),
        None,
        &spawner,
        Arc::new(Writer::new(Box::new(tokio::io::sink()))),
        false,
        false,
    )
    .await
    .unwrap();
    Fixture {
        session,
        first_started,
        first_finish,
        ack,
        prompts,
    }
}

#[tokio::test]
async fn next_prompt_waits_for_post_response_ack_but_active_prompt_is_rejected() {
    let f = fixture().await;
    let first_session = f.session.clone();
    let first = tokio::spawn(async move {
        let _response = first_session.response_guard().await.unwrap();
        let stop = first_session
            .prompt("first".to_string(), vec![])
            .await
            .unwrap();
        first_session
            .writer
            .respond(json!(1), json!({"stopReason":stop}))
            .await;
        first_session.after_prompt_response().await;
        stop
    });
    tokio::time::timeout(Duration::from_secs(5), f.first_started.cancelled())
        .await
        .unwrap();
    assert!(
        f.session.response_guard().await.is_err(),
        "an active turn is still rejected"
    );
    f.first_finish.cancel();
    tokio::time::timeout(Duration::from_secs(5), f.ack.attempted.cancelled())
        .await
        .unwrap();
    assert!(!f.session.prompt_running.load(Ordering::Acquire));

    let next = async {
        let _response = f.session.response_guard().await?;
        let stop = f.session.prompt("second".to_string(), vec![]).await?;
        f.session
            .writer
            .respond(json!(2), json!({"stopReason":stop}))
            .await;
        f.session.after_prompt_response().await;
        Ok::<_, String>(stop)
    };
    tokio::pin!(next);
    assert!(
        futures::poll!(&mut next).is_pending(),
        "the ACK tail queues the next prompt"
    );
    assert_eq!(f.prompts.load(Ordering::SeqCst), 1);
    f.ack.release();
    assert_eq!(first.await.unwrap(), "end_turn");
    assert_eq!(next.await.unwrap(), "end_turn");
    assert_eq!(f.prompts.load(Ordering::SeqCst), 2);
    f.session.shutdown().await;
}

#[tokio::test]
async fn response_gate_waiter_is_released_when_the_session_closes() {
    let f = fixture().await;
    let held = f.session.response_gate.clone().lock_owned().await;
    let waiting = f.session.response_guard();
    tokio::pin!(waiting);
    assert!(futures::poll!(&mut waiting).is_pending());
    f.session.closed.cancel();
    assert_eq!(waiting.await.unwrap_err(), "icers session is closed");
    drop(held);
    f.session.shutdown().await;
}

#[tokio::test]
async fn dispatch_pins_prompt_and_ack_to_the_instance_that_owns_the_response_gate() {
    let id = uuid::Uuid::new_v4().to_string();
    let old = fixture_with_id(id.clone()).await;
    let replacement = fixture_with_id(id.clone()).await;
    let server = super::super::Server {
        writer: old.session.writer.clone(),
        spawner: Arc::new(|_, _, _| panic!("both session children are already running")),
        elicitation_form: AtomicBool::new(false),
        sessions: Mutex::new(HashMap::from([(id.clone(), old.session.clone())])),
        loading: tokio::sync::Mutex::new(()),
        session_dir: None,
        compaction: AtomicBool::new(false),
    };
    let held = old.session.response_gate.clone().lock_owned().await;
    let dispatch = server.dispatch(json!({
        "jsonrpc": "2.0", "id": 1, "method": "session/prompt",
        "params": {"sessionId": id, "prompt": [{"type": "text", "text": "captured instance"}]},
    }));
    tokio::pin!(dispatch);
    assert!(futures::poll!(&mut dispatch).is_pending());
    server
        .sessions
        .lock()
        .unwrap()
        .insert(id, replacement.session.clone());
    drop(held);
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = old.first_started.cancelled() => {}
            _ = replacement.first_started.cancelled() => {
                panic!("prompt was routed to the replacement without its response guard");
            }
            _ = &mut dispatch => panic!("dispatch ended before starting the captured child"),
        }
    })
    .await
    .expect("the captured child receives the prompt");
    old.first_finish.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            _ = old.ack.attempted.cancelled() => {}
            _ = replacement.ack.attempted.cancelled() => {
                panic!("the replacement child received the old prompt acknowledgement");
            }
            _ = &mut dispatch => panic!("dispatch ended before acknowledging the captured child"),
        }
    })
    .await
    .expect("the same captured child receives the acknowledgement");
    assert_eq!(old.prompts.load(Ordering::SeqCst), 1);
    assert_eq!(replacement.prompts.load(Ordering::SeqCst), 0);
    assert!(!replacement.ack.attempted.is_cancelled());
    old.ack.release();
    tokio::time::timeout(Duration::from_secs(5), &mut dispatch)
        .await
        .expect("dispatch completes after the captured acknowledgement flushes");
    old.session.shutdown().await;
    replacement.session.shutdown().await;
}
