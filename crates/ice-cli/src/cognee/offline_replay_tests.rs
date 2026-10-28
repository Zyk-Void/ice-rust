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
    let arms = [
        "A_memory_off",
        "B_legacy",
        "C_local_selective",
        "D_selective_mock_jev",
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
        let mut retained = 0usize;
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
                "C_local_selective" | "D_selective_mock_jev" => Some("event"),
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
                    let (text, details) =
                        rt.search_for_tool(query, CancellationToken::new()).await;
                    if details["count"].as_u64().unwrap_or(0) > 0
                        && details.get("error").is_none()
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
            let injected_fact = packet.as_ref().is_some_and(|p| {
                replay_contains_fact(&p.text, case)
            });
            let tool_fact = tool_evidence.as_ref().is_some_and(|text| {
                replay_contains_fact(text, case)
            });
            let delivered = injected_fact || tool_fact;
            retained += usize::from(delivered);
            injected += usize::from(injected_fact);
            persisted += usize::from(injected_fact);
            tool_results += usize::from(tool_fact);
            context_tokens += packet.as_ref().map_or(0, |p| {
                super::super::text::estimate_tokens(&p.text)
            });
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
                traces.push(serde_json::json!({"case":case.id,"prior_fact":case.fact_id,"available_at":case.available_at,"request_at":case.request_ms,"active_context_absent":true,"delivered_before_decision":true,"constraint_reused":true}));
            }
        }
        let count = latencies.len();
        let p50 = nearest_rank(&mut latencies, 50);
        let p95 = nearest_rank(&mut latencies, 95);
        let max = latencies.iter().copied().max().unwrap_or(0);
        let min = latencies.iter().copied().min().unwrap_or(0);
        arm_reports.insert(arm.into(),serde_json::json!({"cases":count,"by_cohort":per_cohort,"requested":requested,"skipped":skipped,"cached":0,"retrieved":retrieved,"retained":retained,"injected":injected,"persisted":persisted,"faux_used":0,"unknown":unknown,"late_error":failures,"wrong_scope":null,"stale":null,"contradictory":null,"novelty_miss":null,"false_positive_admission":null,"future_evidence_delivered":future_evidence,"tool_evidence_results":tool_results,"injected_context_tokens_estimated":context_tokens,"required_critical_miss":required_miss,"forbidden_evidence":forbidden,"unnecessary_requests":unnecessary,"latency_us":{"n":count,"includes_failures":true,"warmup_excluded":false,"min":min,"p50_nearest_rank":p50,"p95_nearest_rank":p95,"max":max},"service_latency":"simulated by local fake transport; not remote"}));
    }
    assert!(arm_reports["E_on_demand"]["tool_evidence_results"].as_u64().unwrap() > 0);
    assert_eq!(arm_reports["E_on_demand"]["persisted"], 0);
    assert_eq!(arm_reports["E_on_demand"]["by_cohort"]["historical_decision"]["required_misses"], 0);
    assert!(arm_reports.values().all(|arm| arm["future_evidence_delivered"] == 0));
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
    let report = serde_json::json!({"schema_version":1,"corpus_version":corpus.version,"fixture_sha256":fixture_hash,"case_count":corpus.cases.len(),"arms":arm_reports,"arm_D_scope":"mock JEV integration was not run by this corpus; faux ID judgment is not implemented in this driver, recorded not eligible/unverified; no JEV quality inference","runtime":{"code_commit":std::process::Command::new("git").args(["rev-parse","HEAD"]).output().ok().map(|o|String::from_utf8_lossy(&o.stdout).trim().to_string()),"worktree_dirty":std::process::Command::new("git").args(["status","--porcelain"]).output().ok().is_some_and(|o|!o.stdout.is_empty()),"hardware":"supplied local metadata: not provided","date_utc":chrono_now_ms(),"main_model_correctness":"unverified","TTFT":"unverified","end_to_end_benefit":"unverified","JEV_model_quality":"unknown/off","JEV_backend_searchability_lag":"unknown/off","mock_JEV_quality_claim":false},"fastpath_benchmark":{"admissions":1010,"measured_n":fast_n,"warmup_excluded":10,"cache":"cold runtime; no filesystem directory scans or backend requests during admission","window_tokens":128000,"remaining_tokens":800,"prompt_bytes":"synthetic 35-38 bytes","fake_transport_requests":transport.sent().len(),"latency_us":{"n":fast_n,"p50_nearest_rank":fast_p50,"p95_nearest_rank":fast_p95,"max":fast_max}},"paired_C_vs_B":{"unnecessary_request_count":{"C":arm_reports["C_local_selective"]["unnecessary_requests"],"B":arm_reports["B_legacy"]["unnecessary_requests"]},"denominator":corpus.cases.len()},"positive_traces":traces,"failure_inclusive":{"total_requests":all_requests,"local_latency_samples_including_failures":all_latencies.len()},"generated_at_ms":chrono_now_ms()});
    std::fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
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
