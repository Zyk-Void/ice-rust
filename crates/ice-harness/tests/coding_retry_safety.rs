//! Coding-session retry safety regressions using only the faux provider.

use std::sync::{Arc, Mutex};

use ice_agent::{AgentEvent, CollectorEmitter};
use ice_ai::providers::faux::{
    faux_assistant_message, FauxBlock, FauxProvider, FauxScript, FauxStep,
};
use ice_ai::types::StopReason;
use ice_ai::Provider;
use ice_harness::agent_harness::{AgentHarness, AgentLane, HarnessRunOutcome};
use ice_harness::session::memory::{InMemorySessionStorage, SystemClock};
use ice_harness::session::types::SessionMetadata;
use ice_harness::session::{DefaultIdGenerator, Session};
use ice_harness::types::{AgentHarnessOptions, HarnessTool, RetryPolicy};
use ice_tools::{create_write_tool, ExecutionToolContext, InMemoryExecutionEnv};

async fn harness_with_retry(
    session_id: &str,
    script: FauxScript,
) -> (AgentHarness, Arc<FauxProvider>, Arc<Mutex<Vec<AgentEvent>>>) {
    let provider = FauxProvider::new(script);
    let env = Arc::new(InMemoryExecutionEnv::new());
    let write = create_write_tool(&ExecutionToolContext::new(env.clone(), Some(env)));
    let tools = vec![HarnessTool::new(write)];
    let (collector, events) = CollectorEmitter::new();
    let session = Session::new(
        Arc::new(InMemorySessionStorage::new(
            SessionMetadata {
                id: session_id.to_string(),
                created_at: 0,
                parent_session_id: None,
            },
            Arc::new(SystemClock),
            Arc::new(DefaultIdGenerator::new()),
        )),
        None,
    );
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        thinking_level: Default::default(),
        active_tool_names: vec!["write".to_string()],
        tools,
        system_prompt: None,
        resources: Default::default(),
        stream_options: Default::default(),
        retry: RetryPolicy {
            enabled: true,
            max_retries: 2,
            base_delay_ms: 0,
            max_agent_delay_ms: 0,
        },
        compaction: Default::default(),
        steering_mode: Default::default(),
        follow_up_mode: Default::default(),
        tool_execution: Default::default(),
        drive: Default::default(),
        session,
        models: vec![provider.clone() as Arc<dyn Provider>],
        to_provider_messages: None,
        entry_projectors: Default::default(),
        agent_emitter: Some(Arc::new(collector)),
        before_tool_call: None,
        after_tool_call: None,
        transform_context: None,
        entry_transforms: Vec::new(),
        provider_hooks: None,
        allow_existing_session: false,
    })
    .await
    .expect("create in-memory harness");

    (harness, provider, events)
}

fn call_count(provider: &FauxProvider) -> usize {
    provider
        .state()
        .call_count
        .load(std::sync::atomic::Ordering::SeqCst)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transient_failure_with_partial_tool_call_is_not_retried_or_executed() {
    let mut failed = faux_assistant_message(
        vec![FauxBlock::tool_call(
            "write",
            serde_json::json!({"path":"partial.txt", "content":"unsafe"}),
        )],
        StopReason::Error,
    );
    failed.error_message = Some("503 service unavailable".into());
    let script = FauxScript::new();
    script.set_responses(vec![
        FauxStep::message(failed),
        FauxStep::text("must not be requested"),
    ]);
    let (harness, provider, events) = harness_with_retry("partial-tool-call", script).await;

    let result = harness
        .prompt_text("write safely", vec![])
        .await
        .expect("failed response is represented as a run outcome");

    assert!(matches!(result.outcome, HarnessRunOutcome::Failed { .. }));
    assert_eq!(
        call_count(&provider),
        1,
        "the failed generation is not retried"
    );
    let events = events.lock().unwrap();
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::RetryScheduled { .. })));
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::ToolExecutionStart { .. })));
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::RetryRefused { error } if error.contains("503")
        )),
        "the user is told why the transient failure was not retried"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn context_overflow_is_not_retried_even_when_error_looks_transient() {
    let mut failed = faux_assistant_message("", StopReason::Error);
    // 503 is independently retryable; the explicit overflow signal must win.
    failed.error_message = Some("503 prompt-is-too-long".into());
    let script = FauxScript::new();
    script.set_responses(vec![
        FauxStep::message(failed),
        FauxStep::text("must not be requested"),
    ]);
    let (harness, provider, events) = harness_with_retry("context-overflow", script).await;

    let result = harness
        .prompt_text("large request", vec![])
        .await
        .expect("overflow response is represented as a run outcome");

    assert!(matches!(result.outcome, HarnessRunOutcome::Failed { .. }));
    assert_eq!(
        call_count(&provider),
        1,
        "overflow is terminal for ordinary retry"
    );
    let events = events.lock().unwrap();
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::RetryScheduled { .. })));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::RetryRefused { .. })),
        "overflow is not a tool-effect refusal"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completed_tool_turn_does_not_suppress_later_empty_failure_retry() {
    let mut failed = faux_assistant_message("", StopReason::Error);
    failed.error_message = Some("503 service unavailable".into());
    let script = FauxScript::new();
    script.set_responses(vec![
        FauxStep::tool_call(
            "write",
            serde_json::json!({"path":"once.txt", "content":"once"}),
        ),
        FauxStep::message(failed),
        FauxStep::text("finished after retry"),
    ]);
    let (harness, provider, events) = harness_with_retry("effect-reset", script).await;

    let result = harness
        .prompt_text("write once, then answer", vec![])
        .await
        .expect("later empty failure is safely retried");

    assert!(matches!(
        result.outcome,
        HarnessRunOutcome::Completed { .. }
    ));
    assert_eq!(
        call_count(&provider),
        3,
        "one tool turn, failed generation, retry"
    );
    let events = events.lock().unwrap();
    let executions = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::ToolExecutionStart { .. }))
        .count();
    assert_eq!(executions, 1, "the committed tool effect is not replayed");
    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::RetryScheduled { attempt: 1, .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transient_empty_failures_stop_at_the_configured_retry_bound() {
    let transient = || {
        let mut failed = faux_assistant_message("", StopReason::Error);
        failed.error_message = Some("503 service unavailable".into());
        FauxStep::message(failed)
    };
    let script = FauxScript::new();
    script.set_responses(vec![
        transient(),
        transient(),
        transient(),
        FauxStep::text("unexpected"),
    ]);
    let (harness, provider, events) = harness_with_retry("bounded-retries", script).await;

    let result = harness
        .prompt_text("bounded retry request", vec![])
        .await
        .expect("exhausted retry returns a run outcome");
    assert!(matches!(result.outcome, HarnessRunOutcome::Failed { .. }));
    assert_eq!(call_count(&provider), 3, "initial call plus two retries");
    let attempts: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentEvent::RetryScheduled { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, [1, 2]);
}
