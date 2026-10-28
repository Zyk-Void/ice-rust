// Offline chronological replay driver, included inside runtime::tests to exercise
// the production runtime and the existing scripted transport.
use sha2::{Digest, Sha256};
use std::time::Instant;

#[derive(serde::Deserialize)]
struct ReplayCorpus {
    version: String,
    cases: Vec<ReplayCase>,
}
#[derive(serde::Deserialize)]
struct ReplayCase {
    id: String,
    cohort: String,
    task: String,
    request_ms: u64,
    prompt: String,
    fact_id: String,
    fact_text: String,
    available_at: u64,
    expected_required: Vec<String>,
    expected_forbidden: Vec<String>,
    failure: Option<String>,
}

fn replay_contains_fact(text: &str, case: &ReplayCase) -> bool {
    text.contains(&case.fact_id) || text.contains(&case.fact_text)
}

fn nearest_rank(values: &mut [u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[((percentile * values.len()).div_ceil(100))
        .saturating_sub(1)
        .min(values.len() - 1)]
}

#[tokio::test]
#[ignore = "opt-in offline replay; set ICERS_MEMORY_REPLAY_REPORT to write JSON"]
async fn chronological_offline_replay_report() {
    let Some(report_path) = std::env::var_os("ICERS_MEMORY_REPLAY_REPORT") else {
        return;
    };
    let corpus_bytes =
        include_bytes!("../../../../docs/cognee-jev-memory/evidence/offline-replay-cases.json");
    let corpus: ReplayCorpus = serde_json::from_slice(corpus_bytes).unwrap();
    assert!(corpus.cases.len() >= 48);
    let fixture_hash = format!("{:x}", Sha256::digest(corpus_bytes));
    // D is intentionally not run: this driver has no typed helper transport and
    // must not present C's results as a JEV control-flow exercise.
    let arms = [
        "A_memory_off",
        "B_legacy",
        "C_local_selective",
        "E_on_demand",
    ];
    let mut arm_reports = serde_json::Map::new();
    let mut all_requests = 0usize;
    let mut all_latencies = Vec::new();
    let mut traces = Vec::new();
    for arm in arms {
        let mut requested = 0usize;
        let mut skipped = 0usize;
        let mut retrieved = 0usize;
        let mut injected = 0usize;
        let mut persisted = 0usize;
        let mut unknown = 0usize;
        let mut failures = 0usize;
        let mut unnecessary = 0usize;
        let mut required_miss = 0usize;
        let mut forbidden = 0usize;
        let mut tool_results = 0usize;
        let mut context_tokens = 0usize;
        let mut future_evidence = 0usize;
        let mut latencies = Vec::new();
        let mut per_cohort = serde_json::Map::new();
        for case in &corpus.cases {
            let available = case.available_at < case.request_ms;
            let eligible_response = case.failure.is_none() && available;
            let candidate = serde_json::json!({"text":case.fact_text,"metadata":{"source_id":case.fact_id,"dataset":"offline-replay","created_at":case.available_at,"available_at":case.available_at,"scope":"project"}});
            let transport = Arc::new(FakeTransport::default());
            if eligible_response {
                transport.push(FakeResponse::json(
                    200,
                    serde_json::json!({"results":[candidate]}),
                ));
            } else if case.failure.as_deref() == Some("timeout") {
                transport.push(FakeResponse::Ok {
                    status: 200,
                    headers: vec![],
                    chunks: vec![],
                    delay_ms: 1100,
                });
            } else if case.failure.is_some() {
                transport.push(FakeResponse::Fail);
            }
            let policy = match arm {
                "A_memory_off" => None,
                "B_legacy" => Some("legacy"),
                "C_local_selective" => Some("event"),
                _ => Some("on_demand"),
            };
            let config=policy.map(|read|serde_json::json!({"enabled":true,"baseUrl":"http://cognee.test/","dataset":"offline-replay","readPolicy":read,"writePolicy":"manual","autoRecall":true,"recallBudgetMs":1000,"helperEnabled":false}));
            let dir = tempfile::tempdir().unwrap();
            let request_ms = case.request_ms;
            let rt = runtime_with_clock(
                dir.path(),
                &transport,
                config,
                false,
                Arc::new(move || request_ms),
            );
            let started = Instant::now();
            let mut packet = None;
            let mut tool_evidence = None;
            if arm != "A_memory_off" {
                // Legacy and event request boundary are real production calls. On-demand
                // simulates an explicit user tool request only for explicit history cases.
                if arm == "E_on_demand" && case.cohort == "historical_decision" {
                    let query = &case.prompt;
                    let (text, details) = rt.search_for_tool(query, CancellationToken::new()).await;
                    if details["count"].as_u64().unwrap_or(0) > 0 && details.get("error").is_none()
                    {
                        tool_evidence = Some(text);
                    }
                } else {
                    packet = rt
                        .request_context(&request_event(&case.prompt, &[], &case.task, 0))
                        .await;
                    if let Some(ref context) = packet {
                        rt.prompt_context_persisted(context, true).await;
                    }
                }
            }
            let elapsed = started.elapsed().as_micros() as u64;
            latencies.push(elapsed);
            all_latencies.push(elapsed);
            let sent = transport.sent().len();
            requested += sent;
            all_requests += sent;
            if sent == 0 {
                skipped += 1;
            }
            if eligible_response && sent > 0 {
                retrieved += 1;
            }
            let injected_fact = packet
                .as_ref()
                .is_some_and(|p| replay_contains_fact(&p.text, case));
            let tool_fact = tool_evidence
                .as_ref()
                .is_some_and(|text| replay_contains_fact(text, case));
            let delivered = injected_fact || tool_fact;
            injected += usize::from(injected_fact);
            // The harness persistence callback only acknowledges context delivery;
            // it does not establish remote persistence or downstream use.
            persisted += usize::from(injected_fact);
            tool_results += usize::from(tool_fact);
            context_tokens += packet
                .as_ref()
                .map_or(0, |p| super::super::text::estimate_tokens(&p.text));
            future_evidence += usize::from(delivered && !available);
            let miss = case.expected_required.iter().any(|id| id == &case.fact_id) && !delivered;
            required_miss += usize::from(miss);
            if miss && available && case.failure.is_none() {
                unknown += 1;
            }
            failures += usize::from(case.failure.is_some() && sent > 0);
            unnecessary += usize::from(sent > 0 && case.expected_required.is_empty());
            forbidden += usize::from(
                case.expected_forbidden
                    .iter()
                    .any(|id| delivered && id == &case.fact_id),
            );
            let cohort = per_cohort
                .entry(case.cohort.clone())
                .or_insert_with(|| serde_json::json!({"cases":0,"requests":0,"required_misses":0}));
            cohort["cases"] = serde_json::json!(cohort["cases"].as_u64().unwrap_or(0) + 1);
            cohort["requests"] =
                serde_json::json!(cohort["requests"].as_u64().unwrap_or(0) + sent as u64);
            cohort["required_misses"] = serde_json::json!(
                cohort["required_misses"].as_u64().unwrap_or(0) + u64::from(miss)
            );
            if arm == "C_local_selective" && case.cohort == "historical_decision" && delivered {
                traces.push(serde_json::json!({"case":case.id,"prior_fact":case.fact_id,"available_at":case.available_at,"request_at":case.request_ms,"active_context_absent":true,"expected_fact_prepared":true,"persist_acknowledged":false,"delivered_to_model":null,"downstream_observation":false,"constraint_reused":null,"benefit":null}));
            }
        }
        let count = latencies.len();
        let p50 = nearest_rank(&mut latencies, 50);
        let p95 = nearest_rank(&mut latencies, 95);
        let max = latencies.iter().copied().max().unwrap_or(0);
        let min = latencies.iter().copied().min().unwrap_or(0);
        arm_reports.insert(arm.into(),serde_json::json!({"cases":count,"failure_denominator":count,"by_cohort":per_cohort,"requested":requested,"skipped":skipped,"cached":0,"retrieved":retrieved,"packet_prepared":injected,"harness_persist_callback":persisted,"cognee_persist_acknowledged":0,"tool_delivered":tool_results,"delivered_to_model":null,"downstream_observed_use":null,"benefit":null,"unknown":unknown,"failed_requests":failures,"wrong_scope":"not_measured_no_source_scope_fixture_in_selector","stale":"not_measured_no_expiry_fixture_in_selector","contradictory":"not_measured_no_supersession_fixture_in_selector","novelty_miss":"not_measured_no_active_context_fixture_in_selector","false_positive_admission":"not_measured_no_action_observation","future_evidence_delivered":future_evidence,"packet_context_tokens_estimated":context_tokens,"required_critical_miss":required_miss,"forbidden_evidence":forbidden,"unnecessary_requests":unnecessary,"latency_us":{"n":count,"includes_failures":true,"warmup_excluded":false,"min":min,"p50_nearest_rank":p50,"p95_nearest_rank":p95,"max":max},"local_driver_latency":"includes runtime/fake transport; not remote service latency"}));
    }
    assert_eq!(
        arm_reports["C_local_selective"]["by_cohort"]["self_contained"]["requests"],
        0
    );
    assert_eq!(arm_reports["C_local_selective"]["forbidden_evidence"], 0);
    assert!(arm_reports["C_local_selective"]["cognee_persist_acknowledged"].as_u64() == Some(0));
    assert_eq!(arm_reports["E_on_demand"]["harness_persist_callback"], 0);
    assert!(
        arm_reports["E_on_demand"]["tool_delivered"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        arm_reports["E_on_demand"]["by_cohort"]["historical_decision"]["required_misses"],
        0
    );
    assert!(arm_reports
        .values()
        .all(|arm| arm["future_evidence_delivered"] == 0));
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let fast_rt = runtime(
        dir.path(),
        &transport,
        Some(
            serde_json::json!({"enabled":true,"readPolicy":"event","writePolicy":"manual","helperEnabled":false}),
        ),
        false,
    );
    let mut fast_latencies = Vec::with_capacity(1000);
    for i in 0..1010 {
        let prompt = format!("ordinary self contained request {i}");
        let start = Instant::now();
        assert!(fast_rt
            .request_context(&request_event(&prompt, &[], "fastpath", i))
            .await
            .is_none());
        if i >= 10 {
            fast_latencies.push(start.elapsed().as_micros() as u64);
        }
    }
    assert!(
        transport.sent().is_empty(),
        "fast path must never call fake transport"
    );
    let fast_n = fast_latencies.len();
    let fast_p50 = nearest_rank(&mut fast_latencies, 50);
    let fast_p95 = nearest_rank(&mut fast_latencies, 95);
    let fast_max = fast_latencies.iter().copied().max().unwrap_or(0);
    let report = serde_json::json!({"schema_version":2,"corpus_version":corpus.version,"fixture_sha256":fixture_hash,"case_count":corpus.cases.len(),"arms":arm_reports,"arm_D":{"status":"not_executed","eligible":false,"reason":"no typed fake helper judgment is wired into this chronological driver","quality":"not_evaluated","results":null},"runtime":{"code_commit":std::process::Command::new("git").args(["rev-parse","HEAD"]).output().ok().map(|o|String::from_utf8_lossy(&o.stdout).trim().to_string()),"worktree_dirty":std::process::Command::new("git").args(["status","--porcelain"]).output().ok().is_some_and(|o|!o.stdout.is_empty()),"hardware":"supplied local metadata: not provided","date_utc":chrono_now_ms(),"main_model_correctness":"unverified","TTFT":"unverified","end_to_end_benefit":"unverified","JEV_model_quality":"unknown/off","JEV_backend_searchability_lag":"unknown/off","mock_JEV_quality_claim":false},"fastpath_benchmark":{"admissions":1010,"measured_n":fast_n,"warmup_excluded":10,"cache":"cold runtime; no filesystem directory scans or backend requests during admission","window_tokens":128000,"remaining_tokens":800,"prompt_bytes":"synthetic 35-38 bytes","fake_transport_requests":transport.sent().len(),"latency_us":{"n":fast_n,"p50_nearest_rank":fast_p50,"p95_nearest_rank":fast_p95,"max":fast_max}},"paired_C_vs_B":{"unnecessary_request_count":{"C":arm_reports["C_local_selective"]["unnecessary_requests"],"B":arm_reports["B_legacy"]["unnecessary_requests"]},"denominator":corpus.cases.len(),"comparison":"synthetic policy replay only; no answer/action benefit claim"},"positive_traces":traces,"failure_inclusive":{"total_requests":all_requests,"local_latency_samples_including_failures":all_latencies.len()},"generated_at_ms":chrono_now_ms()});
    std::fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}

#[tokio::test]
async fn isolated_mode_disable_and_rollback_preserve_uncertain_records() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(
            serde_json::json!({"enabled":true,"readPolicy":"event","writePolicy":"manual","helperEnabled":false,"dataset":"offline-closeout"}),
        ),
        false,
    );
    rt.ensure_session_id("offline-closeout-session");

    let uncertain = super::super::queue::create_pending_remember(
        "synthetic durable uncertainty must survive rollback",
        "offline-closeout",
        "user_context",
        NOW,
    );
    let mut uncertain = uncertain;
    uncertain.state = super::super::queue::PendingState::Uncertain;
    super::super::queue::enqueue_pending_remember(&rt.pending_dir(), &uncertain, 64).unwrap();

    let canary = rt
        .config()
        .with_mode(super::super::config::MemoryMode::Canary)
        .unwrap();
    assert_eq!(canary.read_policy, super::super::config::ReadPolicy::Event);
    assert_eq!(
        canary.write_policy,
        super::super::config::WritePolicy::Manual
    );
    assert!(rt.persist_config(canary).await);
    let disabled = rt
        .config()
        .with_mode(super::super::config::MemoryMode::Manual)
        .unwrap();
    assert!(rt.persist_config(disabled).await);
    let mut off = rt.config();
    off.enabled = false;
    off.helper_enabled = false;
    assert!(rt.persist_config(off).await);
    assert!(rt
        .request_context(&request_event(
            "What earlier constraint applies to this task?",
            &[],
            "offline",
            0
        ))
        .await
        .is_none());
    assert!(
        transport.sent().is_empty(),
        "disabled/offline fixture must send no helper or Cognee requests"
    );

    let rollback = rt
        .config()
        .with_mode(super::super::config::MemoryMode::Legacy)
        .unwrap();
    assert!(rt.persist_config(rollback).await);
    rt.wait_for_background().await;
    let records = super::super::queue::read_pending_remember(&rt.pending_dir());
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].state,
        super::super::queue::PendingState::Uncertain
    );
    assert!(
        transport.sent().is_empty(),
        "rollback must not drain uncertain writes implicitly"
    );
}

#[tokio::test]
async fn frozen_selector_fixture_exercises_production_provenance_filters() {
    #[derive(serde::Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }
    #[derive(serde::Deserialize)]
    struct Case {
        id: String,
        now_ms: u64,
        dataset: String,
        branch: Option<String>,
        revision: Option<String>,
        worktree_id: Option<String>,
        active_text: Vec<String>,
        fact_text: String,
        metadata: serde_json::Value,
        expected: String,
    }
    let bytes =
        include_bytes!("../../../../docs/cognee-jev-memory/evidence/closeout-selector-cases.json");
    let fixture: Fixture = serde_json::from_slice(bytes).unwrap();
    let mut measured = serde_json::Map::new();
    for case in fixture.cases {
        if case.expected == "fixture_not_queried" {
            measured.insert(case.id, serde_json::json!({"status":"not_exercised","reason":"query admission is outside the evidence selector"}));
            continue;
        }
        let result = super::super::client::RecallResult {
            text: case.fact_text.clone(),
            score: Some(0.5),
            metadata: Some(case.metadata),
        };
        let mut seen = super::super::evidence::SeenEvidence::default();
        let active = case
            .active_text
            .into_iter()
            .map(|text| super::super::text::RecallLine {
                text,
                score: None,
                provenance: None,
            })
            .collect::<Vec<_>>();
        seen.remember_lines(&active);
        let context = super::super::evidence::FilterContext {
            now_ms: case.now_ms,
            dataset: case.dataset,
            legacy_dataset: None,
            branch: case.branch,
            revision: case.revision,
            worktree_id: case.worktree_id,
            suppressions: None,
        };
        let selected =
            super::super::evidence::select_new_with_context(&[result], &mut seen, &context);
        let observed = if !selected.lines.is_empty() {
            "keep"
        } else if selected.rejected.wrong_scope > 0 {
            "wrong_scope"
        } else if selected.rejected.wrong_revision > 0 {
            "wrong_revision"
        } else if selected.rejected.superseded > 0 {
            "superseded"
        } else if selected.rejected.expired > 0 {
            "expired"
        } else if selected.rejected.duplicate > 0 {
            "active_context_duplicate"
        } else {
            "other_rejection"
        };
        measured.insert(case.id, serde_json::json!({"expected":case.expected,"observed":observed,"match":case.expected == observed,"kept":selected.kept,"rejected":selected.rejected}));
    }
    for (case_id, entry) in &measured {
        assert!(
            entry["match"] == true || entry["status"] == "not_exercised",
            "frozen selector label mismatch for {case_id}: {entry}"
        );
    }
    assert_eq!(measured["wrong-scope"]["observed"], "wrong_scope");
    assert_eq!(measured["stale-revision"]["observed"], "wrong_revision");
    assert_eq!(measured["superseded"]["observed"], "superseded");
    assert_eq!(measured["expired-before-boundary"]["observed"], "expired");
}

#[tokio::test]
async fn chronological_replay_regressions_temporal_boundary_zero_calls_and_denominators() {
    let bytes =
        include_bytes!("../../../../docs/cognee-jev-memory/evidence/offline-replay-cases.json");
    let corpus: ReplayCorpus = serde_json::from_slice(bytes).unwrap();
    assert!(corpus.cases.len() >= 48);
    assert!(corpus.cases.iter().any(|c| c.available_at >= c.request_ms));
    assert!(
        corpus
            .cases
            .iter()
            .filter(|c| c.cohort == "self_contained")
            .count()
            >= 20
    );
    assert!(
        corpus
            .cases
            .iter()
            .filter(|c| c.cohort == "historical_decision")
            .count()
            >= 12
    );
    let mut values = vec![100, 1, 2, 3];
    assert_eq!(nearest_rank(&mut values, 95), 100);
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(
            serde_json::json!({"enabled":true,"readPolicy":"event","writePolicy":"manual","helperEnabled":false}),
        ),
        false,
    );
    for i in 0..1000 {
        let prompt = format!("ordinary self contained request {i}");
        assert!(rt
            .request_context(&request_event(&prompt, &[], "ordinary", i))
            .await
            .is_none());
    }
    assert!(
        transport.sent().is_empty(),
        "ordinary admissions must make zero backend requests"
    );
}
