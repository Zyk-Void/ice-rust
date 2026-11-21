use super::*;
use ice_ai::providers::faux::{FauxProvider, FauxScript, FauxStep};
use ice_harness::agent_harness::AgentLane;
use ice_harness::types::{AgentHarnessOptions, HarnessLifecycleHooks};
use std::sync::atomic::AtomicUsize;
use tokio::sync::broadcast;

#[derive(Default)]
struct Inputs(AtomicUsize);

#[async_trait::async_trait]
impl HarnessLifecycleHooks for Inputs {
    async fn input(
        &self,
        message: ice_agent::AgentMessage,
    ) -> ice_harness::result::HarnessResult<ice_agent::AgentMessage> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(message)
    }
}

struct Fixture {
    runtime: Arc<AcpRuntime>,
    harness: AgentHarness,
    provider: Arc<FauxProvider>,
    shells: Arc<SessionShells>,
    events: broadcast::Receiver<AgentEvent>,
    inputs: Arc<Inputs>,
}

async fn fixture() -> Fixture {
    let script = FauxScript::new();
    script.set_responses(vec![
        FauxStep::text("wake reply"),
        FauxStep::text("client reply"),
    ]);
    let provider = FauxProvider::new(script);
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider.clone()],
        ..Default::default()
    })
    .await
    .unwrap();
    let inputs = Arc::new(Inputs::default());
    harness.set_lifecycle_hooks(inputs.clone());
    let (emitter, events) = ice_agent::events::BroadcastEmitter::new(256);
    harness
        .set_agent_emitter(Some(Arc::new(emitter)))
        .await
        .unwrap();
    let shells = SessionShells::new(&Default::default());
    let runtime = AcpRuntime::new(&shells, &harness);
    Fixture {
        runtime,
        harness,
        provider,
        shells,
        events,
        inputs,
    }
}

fn notice(text: &str) -> ice_agent::AgentMessage {
    ice_harness::messages::create_custom_message(
        "test_background_complete",
        ice_ai::types::UserContent::Text(text.to_string()),
        true,
        None,
        1,
    )
}

fn waking() -> SendMessageOptions {
    SendMessageOptions {
        trigger_turn: Some(true),
        deliver_as: MessageDelivery::Steer,
    }
}

fn start_wake(f: &Fixture) -> tokio::task::JoinHandle<()> {
    let runtime = f.runtime.clone();
    let harness = f.harness.clone();
    tokio::spawn(async move {
        let epoch = runtime.epoch();
        runtime
            .deliver(harness, notice("shell completed"), waking(), epoch)
            .await;
    })
}

async fn wake_settlement(f: &mut Fixture) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = f.events.recv().await.unwrap();
            if let Some(frame) = f.runtime.ordered_settlement(&event) {
                return frame;
            }
        }
    })
    .await
    .expect("wake reaches its ordered settlement")
}

fn calls(f: &Fixture) -> usize {
    f.provider.state().call_count.load(Ordering::SeqCst)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_admission_waits_for_wake_usage_acknowledgement() {
    let mut f = fixture().await;
    let wake = start_wake(&f);
    let settlement = wake_settlement(&mut f).await;
    assert_eq!(settlement["origin"], "wake");
    assert_eq!(settlement["outcome"], "completed");
    assert!(f.runtime.operation().try_lock_owned().is_err());
    assert_eq!(calls(&f), 1);
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 1);

    let cancel = CancellationToken::new();
    let client = async {
        let _operation = f.runtime.reserve(&cancel).await?;
        f.harness
            .prompt_text("client prompt", vec![])
            .await
            .map_err(|e| e.to_string())
    };
    tokio::pin!(client);
    assert!(futures::poll!(&mut client).is_pending());
    // State reads used for usage stay available while admission is held.
    assert!(f.harness.context_snapshot().await.is_ok());
    assert_eq!(calls(&f), 1);
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 1);
    f.runtime.acknowledge("unrelated-wake");
    assert!(f.runtime.operation().try_lock_owned().is_err());

    f.runtime
        .acknowledge(settlement["wakeId"].as_str().unwrap());
    assert!(matches!(
        client.await.unwrap().outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Completed { .. }
    ));
    wake.await.unwrap();
    assert_eq!(calls(&f), 2);
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 2);
    f.runtime.shutdown(&f.harness).await;
    f.shells.dispose().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_queued_client_never_reaches_hooks_or_provider() {
    let mut f = fixture().await;
    let wake = start_wake(&f);
    let settlement = wake_settlement(&mut f).await;
    let cancel = CancellationToken::new();
    let client = async {
        let _operation = f.runtime.reserve(&cancel).await?;
        f.harness
            .prompt_text("must never be admitted", vec![])
            .await
            .map_err(|e| e.to_string())
    };
    tokio::pin!(client);
    assert!(futures::poll!(&mut client).is_pending());
    cancel.cancel();
    assert_eq!(client.await.unwrap_err(), "Operation aborted");
    assert_eq!(calls(&f), 1);
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 1);

    // Stop must not release a finished wake before its usage acknowledgement.
    f.runtime.cancel_wake();
    assert!(f.runtime.operation().try_lock_owned().is_err());
    f.runtime
        .acknowledge(settlement["wakeId"].as_str().unwrap());
    wake.await.unwrap();
    let next_cancel = CancellationToken::new();
    let _next = f.runtime.reserve(&next_cancel).await.unwrap();
    assert!(matches!(
        f.harness
            .prompt_text("next client", vec![])
            .await
            .unwrap()
            .outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Completed { .. }
    ));
    assert_eq!(calls(&f), 2);
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 2);
    f.runtime.shutdown(&f.harness).await;
    f.shells.dispose().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_queued_before_stop_is_retained_for_the_next_client_turn() {
    let f = fixture().await;
    let operation = f.runtime.operation().lock_owned().await;
    let epoch = f.runtime.epoch();
    let delivery = f.runtime.deliver(
        f.harness.clone(),
        notice("stale completion"),
        waking(),
        epoch,
    );
    tokio::pin!(delivery);
    assert!(futures::poll!(&mut delivery).is_pending());
    f.runtime.cancel_wake();
    drop(operation);
    delivery.await;
    assert_eq!(calls(&f), 0, "Stop prevents an already queued wake");
    assert!(f.runtime.runs.lock().unwrap().is_empty());
    assert!(f.runtime.acknowledgements.lock().unwrap().is_empty());

    f.harness
        .prompt_text("next user prompt", vec![])
        .await
        .unwrap();
    assert_eq!(calls(&f), 1);
    let context = f.harness.context_snapshot().await.unwrap();
    let serialized = serde_json::to_string(&context.context.messages).unwrap();
    assert!(serialized.contains("stale completion"), "{serialized}");
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 2);
    f.runtime.shutdown(&f.harness).await;
    f.shells.dispose().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_headless_delivery_does_not_start_a_wake() {
    let f = fixture().await;
    super::super::deliver(
        f.harness.clone(),
        notice("plain RPC completion"),
        SendMessageOptions {
            trigger_turn: None,
            deliver_as: MessageDelivery::Steer,
        },
    )
    .await;
    assert_eq!(calls(&f), 0);
    assert!(f.runtime.runs.lock().unwrap().is_empty());
    let messages = f.harness.context_snapshot().await.unwrap().context.messages;
    assert!(serde_json::to_string(&messages)
        .unwrap()
        .contains("plain RPC completion"));
    f.runtime.shutdown(&f.harness).await;
    f.shells.dispose().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_captured_during_stop_stays_suppressed_after_stop_clears() {
    let f = fixture().await;
    f.runtime.stopping.store(true, Ordering::Release);
    f.runtime.cancel_wake();
    // Capture after Stop's final generation bump but before it clears stopping.
    let epoch = f.runtime.epoch();
    f.runtime.stopping.store(false, Ordering::Release);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        f.runtime.deliver(
            f.harness.clone(),
            notice("completion during Stop"),
            waking(),
            epoch,
        ),
    )
    .await
    .expect("a Stop-window notice is recorded without waiting for wake acknowledgement");
    assert_eq!(calls(&f), 0);
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 1);
    assert!(f.runtime.runs.lock().unwrap().is_empty());
    assert!(f.runtime.acknowledgements.lock().unwrap().is_empty());

    f.harness
        .prompt_text("next user prompt", vec![])
        .await
        .unwrap();
    assert_eq!(calls(&f), 1);
    let context = f.harness.context_snapshot().await.unwrap();
    let serialized = serde_json::to_string(&context.context.messages).unwrap();
    assert!(
        serialized.contains("completion during Stop"),
        "{serialized}"
    );
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 2);
    f.runtime.shutdown(&f.harness).await;
    f.shells.dispose().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wake_acknowledgement_releases_synchronous_operation_admission() {
    let mut f = fixture().await;
    let runtime = f.runtime.clone();
    let wake = runtime.deliver(
        f.harness.clone(),
        notice("shell completed"),
        waking(),
        runtime.epoch(),
    );
    tokio::pin!(wake);
    let settlement = tokio::select! {
        settlement = wake_settlement(&mut f) => settlement,
        _ = &mut wake => panic!("wake released admission before its acknowledgement"),
    };
    assert!(f.runtime.operation().try_lock_owned().is_err());
    let ack = f
        .runtime
        .acknowledge_wake(settlement["wakeId"].as_str().unwrap());
    tokio::pin!(ack);
    // The wake is unpolled here, so only a release-aware ACK stays pending.
    assert!(futures::poll!(&mut ack).is_pending());
    tokio::join!(wake, ack);
    let operation = f
        .runtime
        .operation()
        .try_lock_owned()
        .expect("a synchronous fork/config command can enter immediately after the ACK");
    drop(operation);
    assert_eq!(calls(&f), 1);
    assert_eq!(f.inputs.0.load(Ordering::SeqCst), 1);
    f.runtime.shutdown(&f.harness).await;
    f.shells.dispose().await;
}

#[derive(Default)]
struct GatedInput {
    calls: AtomicUsize,
    entered: CancellationToken,
    release: CancellationToken,
}

#[async_trait::async_trait]
impl HarnessLifecycleHooks for GatedInput {
    async fn input(
        &self,
        message: ice_agent::AgentMessage,
    ) -> ice_harness::result::HarnessResult<ice_agent::AgentMessage> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.entered.cancel();
            self.release.cancelled().await;
        }
        Ok(message)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_before_wake_run_start_retains_notice_without_repeating_input_hooks() {
    let f = fixture().await;
    let input = Arc::new(GatedInput::default());
    f.harness.set_lifecycle_hooks(input.clone());
    let wake = start_wake(&f);
    tokio::time::timeout(std::time::Duration::from_secs(5), input.entered.cancelled())
        .await
        .expect("the completion reaches its first input hook");
    assert_eq!(calls(&f), 0);
    assert_eq!(input.calls.load(Ordering::SeqCst), 1);
    assert!(f.runtime.runs.lock().unwrap().is_empty());

    f.runtime.stop_background(&f.harness).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), wake)
        .await
        .expect("Stop drops the pending hook and retains the completion")
        .unwrap();
    assert_eq!(
        calls(&f),
        0,
        "Stop does not start an automatic provider call"
    );
    assert_eq!(input.calls.load(Ordering::SeqCst), 1);
    assert!(f.runtime.admitting.lock().unwrap().is_none());
    assert!(f.runtime.acknowledgements.lock().unwrap().is_empty());
    assert!(f.runtime.operation().try_lock_owned().is_ok());

    input.release.cancel();
    f.harness
        .prompt_text("next explicit user prompt", vec![])
        .await
        .unwrap();
    assert_eq!(calls(&f), 1);
    assert_eq!(input.calls.load(Ordering::SeqCst), 2);
    let context = f.harness.context_snapshot().await.unwrap();
    let serialized = serde_json::to_string(&context.context.messages).unwrap();
    assert_eq!(
        serialized.matches("shell completed").count(),
        1,
        "{serialized}"
    );
    f.runtime.shutdown(&f.harness).await;
    f.shells.dispose().await;
}
