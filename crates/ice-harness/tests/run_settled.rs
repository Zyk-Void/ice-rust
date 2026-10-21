//! Harness-only finalization barrier and authoritative cancellation regressions.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ice_agent::{AgentEvent, AgentMessage, AgentTool};
use ice_ai::providers::faux::{FauxProvider, FauxScript};
use ice_ai::types::{Schema, StopReason, Tool, UserContent};
use ice_harness::agent_harness::{AgentHarness, AgentLane, HarnessRunOutcome};
use ice_harness::events::{RunEndEvent, RunEndOutcome};
use ice_harness::messages::create_custom_message;
use ice_harness::session::types::{LaneRecord, OperationOutcome, RecordQuery};
use ice_harness::types::{AgentHarnessOptions, HarnessTool};

fn collect_events(receiver: &mut tokio::sync::broadcast::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    events
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settled_barrier_follows_run_end_messages_and_precedes_next_run_start() {
    let provider = FauxProvider::new(FauxScript::new().with_text("A done").with_text("B done"));
    let (emitter, mut receiver) = ice_agent::events::BroadcastEmitter::new(256);
    let harness = Arc::new(
        AgentHarness::create(AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider],
            agent_emitter: Some(Arc::new(emitter)),
            ..Default::default()
        })
        .await
        .unwrap(),
    );
    let final_message = create_custom_message(
        "run-end-context",
        UserContent::Text("A final listener message".into()),
        true,
        None,
        0,
    );
    let final_context = final_message.clone();
    let weak = Arc::downgrade(&harness);
    let fired = Arc::new(AtomicBool::new(false));
    let next_ready = Arc::new(tokio::sync::Notify::new());
    let ready = next_ready.clone();
    let _listener = harness.events().on::<RunEndEvent, _>(move |_| {
        if fired.swap(true, Ordering::SeqCst) {
            return;
        }
        // Wake a caller waiting to start B while A still owns finalization.
        ready.notify_one();
        let harness = weak.upgrade().unwrap();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(harness.append_message(final_context.clone()))
        })
        .unwrap();
    });
    let next_harness = harness.clone();
    let next = tokio::spawn(async move {
        next_ready.notified().await;
        next_harness.lane("main").wait_for_idle().await.unwrap();
        next_harness.prompt_text("B", vec![]).await.unwrap()
    });
    let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let a = harness.prompt_text("A", vec![]).await.unwrap();
        (a, next.await.unwrap())
    })
    .await
    .unwrap();
    let events = collect_events(&mut receiver);
    let starts: Vec<usize> = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| matches!(event, AgentEvent::AgentStart).then_some(index))
        .collect();
    assert_eq!(starts.len(), 2);
    let final_message_index = events.iter().position(|event| {
        matches!(event, AgentEvent::MessageEnd { message } if message == &final_message)
    }).expect("RunEnd listener message must be broadcast");
    let settled_a: Vec<usize> = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            matches!(event, AgentEvent::RunSettled { run_id } if run_id == &a.run_id)
                .then_some(index)
        })
        .collect();
    assert_eq!(settled_a.len(), 1);
    assert!(final_message_index < settled_a[0]);
    assert!(
        settled_a[0] < starts[1],
        "B must not start before A's barrier"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(event, AgentEvent::RunSettled { run_id } if run_id == &b.run_id)
            })
            .count(),
        1
    );
    assert_eq!(events[settled_a[0]].type_tag(), "run_settled");
}

struct WaitForAbort {
    schema: Tool,
    started: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl AgentTool for WaitForAbort {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "wait-for-abort"
    }
    async fn execute(
        &self,
        _: &str,
        _: serde_json::Value,
        signal: tokio_util::sync::CancellationToken,
        _: Arc<dyn Fn(ice_agent::ToolResultPartial) + Send + Sync>,
    ) -> Result<ice_agent::AgentToolResult, ice_agent::AgentError> {
        self.started.notify_one();
        signal.cancelled().await;
        Err(ice_agent::AgentError::Abort)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_lane_abort_during_tool_is_aborted_while_retaining_tooluse_message() {
    let started = Arc::new(tokio::sync::Notify::new());
    let provider = FauxProvider::new(
        FauxScript::new().with_tool_call("wait-for-abort", serde_json::json!({})),
    );
    let (emitter, mut receiver) = ice_agent::events::BroadcastEmitter::new(256);
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        tools: vec![HarnessTool::new(Arc::new(WaitForAbort {
            schema: Tool {
                name: "wait-for-abort".into(),
                description: "wait until cancelled".into(),
                parameters: Schema::new(serde_json::json!({"type":"object", "properties":{}})),
                constrained_sampling: None,
            },
            started: started.clone(),
        }))],
        active_tool_names: vec!["wait-for-abort".into()],
        agent_emitter: Some(Arc::new(emitter)),
        ..Default::default()
    })
    .await
    .unwrap();
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let recorded = outcomes.clone();
    let _listener = harness.events().on::<RunEndEvent, _>(move |event| {
        recorded.lock().unwrap().push(event.outcome);
    });
    let lane = harness.lane("main");
    let cancel = async {
        started.notified().await;
        lane.abort().await.unwrap();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(lane.prompt_text("run tool", vec![]), cancel)
    })
    .await
    .unwrap();
    let result = result.unwrap();
    let HarnessRunOutcome::Aborted { final_message, .. } = result.outcome else {
        panic!(
            "cancelled operation must not claim completion: {:?}",
            result.outcome
        )
    };
    assert_eq!(final_message.stop_reason, StopReason::ToolUse);
    assert_eq!(*outcomes.lock().unwrap(), vec![RunEndOutcome::Aborted]);
    let records = harness
        .session()
        .find_records(&RecordQuery::default())
        .await
        .unwrap();
    assert!(records.iter().any(|record| matches!(record,
        LaneRecord::OperationFinished(record) if record.run_id == result.run_id
            && record.outcome == OperationOutcome::Aborted
    )));
    let events = collect_events(&mut receiver);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event,
                AgentEvent::RunSettled { run_id } if run_id == &result.run_id
            ))
            .count(),
        1
    );
    assert!(events.iter().any(|event| matches!(event,
        AgentEvent::MessageEnd { message: AgentMessage::Assistant(message) }
            if message.stop_reason == StopReason::ToolUse
    )));
}
