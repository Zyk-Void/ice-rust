/// Live JEV smoke through the real `Helper::bind` and `judge` path. Ignored by
/// default; it needs `ICERS_LIVE_JEV_MODEL` and an isolated
/// `ICERS_CODING_AGENT_DIR` whose `models.json` binds a `typesafe` decision
/// provider. Makes at most `CASES.len()` + 1 requests, never retried. The
/// expected labels are the author's synthetic smoke labels, not held-out data,
/// so outcomes are printed rather than asserted.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "makes paid requests to a live decision API"]
async fn live_jev_helper_smoke() {
    const CASES: [(&str, &str); 6] = [
        ("fmt", "Always run cargo fmt before committing in this repository."),
        ("greeting", "The user said hello."),
        ("sqlite", "We decided to use SQLite instead of Postgres for the local cache."),
        ("concise", "The user prefers concise answers without emoji."),
        ("diskfull", "The build failed once because the disk was full; freeing space fixed it."),
        ("time", "What time is it right now?"),
    ];
    let model = std::env::var("ICERS_LIVE_JEV_MODEL").expect("ICERS_LIVE_JEV_MODEL");
    let config = CogneeConfig {
        write_policy: crate::cognee::config::WritePolicy::Selective,
        helper_provider: Some("typesafe".into()),
        helper_model: Some(model),
        ..CogneeConfig::default()
    };
    let helper = Helper::bind(&config)
        .expect("binding")
        .expect("helper should bind");
    let single = |id: &str, text: &str| {
        CandidateBatch::new(
            vec![Candidate {
                id: id.into(),
                text: text.into(),
                kind_hint: None,
            }],
            Some("synthetic coding-agent session".into()),
        )
        .unwrap()
    };
    let mut requests = 0;
    for (id, text) in CASES {
        let started = std::time::Instant::now();
        let outcome = helper.judge(&single(id, text), CancellationToken::new()).await;
        requests += 1;
        eprintln!("JEV {id} {}ms {outcome:?}", started.elapsed().as_millis());
    }
    let all = CandidateBatch::new(
        CASES
            .iter()
            .take(4)
            .map(|(id, text)| Candidate {
                id: (*id).into(),
                text: (*text).into(),
                kind_hint: None,
            })
            .collect(),
        Some("synthetic coding-agent session".into()),
    )
    .unwrap();
    let started = std::time::Instant::now();
    let outcome = helper.judge(&all, CancellationToken::new()).await;
    requests += 1;
    eprintln!("JEV batch4 {}ms {outcome:?}", started.elapsed().as_millis());
    eprintln!("JEV requests={requests}");
}
