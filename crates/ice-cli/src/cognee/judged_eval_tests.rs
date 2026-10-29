//! Explicit, ignored owner-label evaluation. Never runs in offline test gates.
use super::config::{CogneeConfig, ReadPolicy};
use super::helper::{Helper, ReadGateOutcome, ReadNeed};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use tokio_util::sync::CancellationToken;

fn private_json(path: &Path, value: &Value) {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).expect("private output must not exist");
    serde_json::to_writer_pretty(&mut file, value).unwrap();
    file.write_all(b"\n").unwrap();
}

fn percentile(values: &mut [u64], quantile: f64) -> u64 {
    values.sort_unstable();
    if values.is_empty() {
        return 0;
    }
    values[((values.len() as f64 * quantile).ceil() as usize).saturating_sub(1)]
}

#[tokio::test]
#[ignore = "real TypeSafe read gate; requires explicit owner labels, binding and hard cap"]
async fn judged_owner_label_eval() {
    assert_eq!(
        std::env::var("ICERS_JUDGED_EVAL_ALLOW_REMOTE").as_deref(),
        Ok("1")
    );
    let cap: usize = std::env::var("ICERS_JUDGED_EVAL_REQUEST_CAP")
        .expect("explicit hard request cap required")
        .parse()
        .unwrap();
    assert!((1..=200).contains(&cap));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("target/judged-eval");
    assert!(!dir.is_symlink());
    let sample_bytes = std::fs::read(dir.join("sample.json")).unwrap();
    let samples: Vec<Value> = serde_json::from_slice(&sample_bytes).unwrap();
    let labels: Value =
        serde_json::from_slice(&std::fs::read(dir.join("owner-labels.json")).unwrap()).unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(
        labels["sample_sha256"],
        format!("{:x}", Sha256::digest(&sample_bytes))
    );
    let selected: Vec<_> = samples
        .iter()
        .filter(|row| labels["labels"][row["id"].as_str().unwrap()].is_string())
        .collect();
    assert!(
        !selected.is_empty() && selected.len() <= cap,
        "raise cap explicitly; never silently truncate labeled denominators"
    );
    for row in &selected {
        let id = row["id"].as_str().unwrap();
        assert!(
            id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid sample identifier"
        );
        assert!(
            !dir.join(format!("outcome-{id}.json")).exists(),
            "existing outcome: refuse to retry before making any call"
        );
        assert!(["none", "session", "memory", "unsure"]
            .contains(&labels["labels"][id].as_str().unwrap()));
    }
    assert!(!dir.join("results.json").exists() && !dir.join("aggregates.json").exists());
    let config = CogneeConfig {
        read_policy: ReadPolicy::Judged,
        helper_provider: Some(
            std::env::var("ICERS_JUDGED_EVAL_PROVIDER").expect("explicit provider required"),
        ),
        helper_model: Some(
            std::env::var("ICERS_JUDGED_EVAL_MODEL").expect("explicit model required"),
        ),
        ..CogneeConfig::default()
    };
    let helper = Helper::bind_read_gate(&config)
        .expect("binding failed")
        .expect("binding missing");
    let mut rows = Vec::new();
    let mut matrix = BTreeMap::<String, BTreeMap<String, usize>>::new();
    let mut latencies = Vec::new();
    let (mut failures, mut unsure, mut skipped, mut missed, mut unneeded, mut owner_memory) =
        (0, 0, 0, 0, 0, 0);
    for row in selected {
        let id = row["id"].as_str().unwrap();
        let owner = labels["labels"][id].as_str().unwrap();
        assert!(["none", "session", "memory", "unsure"].contains(&owner));
        let started = std::time::Instant::now();
        // Exactly one real helper call. The shared DecisionClient performs no retries.
        let outcome = helper
            .judge_read_gate(
                row["prompt"].as_str().unwrap(),
                row["previous_assistant"].as_str(),
                CancellationToken::new(),
            )
            .await;
        let latency = started.elapsed().as_millis() as u64;
        latencies.push(latency);
        let (prediction, error, incomplete) = match outcome {
            ReadGateOutcome::Judged { needs, incomplete } => {
                if needs != ReadNeed::Memory {
                    skipped += 1;
                }
                (needs.as_str(), None, Some(incomplete))
            }
            ReadGateOutcome::Abstained(reason) => {
                failures += 1;
                ("failure", Some(format!("{reason:?}")), None)
            }
        };
        unsure += usize::from(owner == "unsure");
        owner_memory += usize::from(owner == "memory");
        missed += usize::from(owner == "memory" && matches!(prediction, "none" | "session"));
        unneeded += usize::from(matches!(owner, "none" | "session") && prediction == "memory");
        *matrix
            .entry(owner.into())
            .or_default()
            .entry(prediction.into())
            .or_default() += 1;
        rows.push(json!({"id":id,"owner":owner,"prediction":prediction,"error":error,"incomplete":incomplete,"latency_ms":latency}));
        // Save each completed outcome before the next call, so interrupted runs
        // retain failures and consume no hidden retries on a resumed invocation.
        private_json(
            &dir.join(format!("outcome-{id}.json")),
            rows.last().unwrap(),
        );
    }
    let n = rows.len();
    let assumption = 3000u64;
    // Only an explicitly supplied private log path is read; never discover live state.
    let mut recall_ms = Vec::new();
    if let Ok(path) = std::env::var("ICERS_JUDGED_EVAL_OBSERVATIONS") {
        let path = std::fs::canonicalize(path).unwrap();
        assert!(
            path.starts_with(std::fs::canonicalize(&dir).unwrap()),
            "copy approved observations into the private evaluation directory first"
        );
        for line in std::fs::read_to_string(path).unwrap().lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                if value["operation"] == "recall" && value["phase"] == "succeeded" {
                    if let Some(ms) = value["meta"]["remoteWaitMs"].as_u64() {
                        recall_ms.push(ms);
                    }
                }
            }
        }
    }
    let recall_median = if recall_ms.is_empty() {
        assumption
    } else {
        percentile(&mut recall_ms, 0.5)
    };
    let total_latency: u64 = latencies.iter().sum();
    let aggregates = json!({"total":n,"failures":failures,"unsure":unsure,"confusion_matrix":matrix,
        "skip_rate":{"numerator":skipped,"denominator":n},
        "missed_memory_rate":{"numerator":missed,"denominator":owner_memory},
        "unneeded_recall_rate":{"numerator":unneeded,"denominator":n},
        "jev_p50_ms":percentile(&mut latencies,0.5),"jev_p95_ms":percentile(&mut latencies,0.95),
        "recall_median_ms":recall_median,"recall_latency_basis":if recall_ms.is_empty(){"assumed 3 seconds"}else{"approved private observability log median"},
        "estimated_time_saved_ms": skipped as i64 * recall_median as i64 - total_latency as i64,
        "estimate_limit":"conservative gate-cost subtraction; production calls overlap, backend cancellation may not stop server work"});
    private_json(
        &dir.join("results.json"),
        &json!({"outcomes":rows,"aggregates":aggregates}),
    );
    private_json(&dir.join("aggregates.json"), &aggregates);
    // Publication is a separate explicit owner action: only aggregates.json may
    // be copied to evidence after review. No session text is emitted to stdout.
    println!("{aggregates}");
}
