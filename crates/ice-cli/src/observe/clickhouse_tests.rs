//! ClickHouse mirror and analytics-source tests against a fake HTTP endpoint
//! on loopback. `live_clickhouse_round_trip` runs the real SQL when
//! `ICERS_TEST_CLICKHOUSE_URL` points at a disposable server.

use super::*;
use crate::observe::analytics::{self, OverviewQuery};
use crate::observe::recorder::{
    ObservePersistence, ObserveRecorder, ObserveRecorderOptions, ObserveSessionInfo,
};
use crate::observe::server::{start_observe_server, ObserveServerOptions};
use ice_agent::{AgentEvent, AgentMessage};
use ice_ai::types::UserMessage;
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone)]
struct Captured {
    params: BTreeMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
}

impl Captured {
    /// The statement: the `query` parameter for inserts, else the body.
    fn statement(&self) -> &str {
        self.params
            .get("query")
            .map(String::as_str)
            .unwrap_or(&self.body)
    }
}

type Responder = Arc<dyn Fn(&Captured) -> (u16, String) + Send + Sync>;

/// A one-request-per-connection HTTP endpoint that records every request.
struct FakeClickHouse {
    url: String,
    requests: Arc<Mutex<Vec<Captured>>>,
}

impl FakeClickHouse {
    async fn start(respond: Responder) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let log = Arc::clone(&log);
                let respond = Arc::clone(&respond);
                tokio::spawn(async move {
                    let Some(captured) = read_request(&mut stream).await else {
                        return;
                    };
                    let (status, body) = respond(&captured);
                    log.lock().unwrap().push(captured);
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { url, requests }
    }

    fn requests(&self) -> Vec<Captured> {
        self.requests.lock().unwrap().clone()
    }

    fn config(&self) -> ClickHouseConfig {
        ClickHouseConfig {
            url: self.url.clone(),
            database: "obs".into(),
            user: Some("agent".into()),
            password: Some("hunter2".into()),
        }
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Captured> {
    let mut buf = Vec::new();
    let head_end = loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let target = lines.next()?.split(' ').nth(1)?.to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let length: usize = headers.get("content-length")?.parse().ok()?;
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let url = reqwest::Url::parse(&format!("http://x{target}")).ok()?;
    Some(Captured {
        params: url.query_pairs().into_owned().collect(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn user(text: &str) -> AgentEvent {
    AgentEvent::MessageStart {
        message: AgentMessage::User(UserMessage::new(text.to_string(), 0)),
    }
}

fn recorder(store: &Arc<ObserveRunStore>, session: &str) -> ObserveRecorder {
    ObserveRecorder::new(ObserveRecorderOptions {
        session_id: session.into(),
        info: ObserveSessionInfo::default(),
        persistence: Some(Arc::clone(store) as Arc<dyn ObservePersistence>),
    })
}

/// `(run_id, sequence)` of every event row the fake received.
fn inserted_events(requests: &[Captured]) -> Vec<(String, i64)> {
    requests
        .iter()
        .filter(|r| r.statement() == "INSERT INTO obs.observe_events FORMAT JSONEachRow")
        .flat_map(|r| r.body.lines().map(str::to_string).collect::<Vec<_>>())
        .map(|line| {
            let row: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert!(
                row["payload"].as_str().unwrap().starts_with('{'),
                "payload is the stored JSON text"
            );
            (
                row["run_id"].as_str().unwrap().to_string(),
                row["sequence"].as_i64().unwrap(),
            )
        })
        .collect()
}

fn inserted_runs(requests: &[Captured]) -> Vec<serde_json::Value> {
    requests
        .iter()
        .filter(|r| r.statement() == "INSERT INTO obs.observe_runs FORMAT JSONEachRow")
        .flat_map(|r| {
            r.body
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect::<Vec<serde_json::Value>>()
        })
        .collect()
}

async fn wait_for(fake: &FakeClickHouse, done: impl Fn(&[Captured]) -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !done(&fake.requests()) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out: {:#?}",
            fake.requests()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[test]
fn config_comes_from_the_environment_and_is_validated() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    };
    assert!(ClickHouseConfig::from_lookup(env(&[])).is_none());
    assert!(ClickHouseConfig::from_lookup(env(&[(ENV_URL, "  ")])).is_none());
    let config = ClickHouseConfig::from_lookup(env(&[
        (ENV_URL, "http://127.0.0.1:8123/"),
        (ENV_PASSWORD, "pw"),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(config.url, "http://127.0.0.1:8123");
    assert_eq!(config.database, DEFAULT_DATABASE);
    assert_eq!(config.password.as_deref(), Some("pw"));
    assert!(
        !format!("{config:?}").contains("pw\""),
        "Debug hides the password"
    );

    for (url, database, expected) in [
        ("ftp://h", None, "http or https"),
        ("http://u:p@h:8123", None, "must not contain credentials"),
        ("http://h:8123/?user=x", None, "query"),
        ("not a url", None, ENV_URL),
        ("http://h", Some("x; DROP"), "plain identifier"),
        ("http://h", Some("1abc"), "plain identifier"),
    ] {
        let error = ClickHouseConfig::from_lookup(|key| match key {
            ENV_URL => Some(url.to_string()),
            ENV_DATABASE => database.map(str::to_string),
            _ => None,
        })
        .unwrap()
        .unwrap_err();
        assert!(error.contains(expected), "{url} {database:?}: {error}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mirror_backfills_what_clickhouse_lacks_then_tails_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    // A finished run from another process; ClickHouse already has its
    // first event.
    let other = recorder(&store, "other");
    other.handle_agent_event(&user("earlier"));
    let other_id = other.run_id();
    other.stop();
    let live = recorder(&store, "live");
    live.handle_agent_event(&user("now"));
    let live_id = live.run_id();

    let known = format!("{{\"run_id\":\"{other_id}\",\"sequence\":1}}\n");
    let fake = FakeClickHouse::start(Arc::new(move |request| {
        if request.statement().contains("max(sequence)") {
            (200, known.clone())
        } else {
            (200, String::new())
        }
    }))
    .await;
    let clickhouse = Arc::new(ClickHouse::new(fake.config()).unwrap());
    let mirror =
        ClickHouseMirror::spawn(Arc::clone(&clickhouse), Arc::clone(&store), live_id.clone());
    wait_for(&fake, |requests| {
        inserted_events(requests)
            .iter()
            .any(|(run, _)| *run == live_id)
    })
    .await;

    let requests = fake.requests();
    let ddl: Vec<&str> = requests
        .iter()
        .map(|r| r.statement())
        .filter(|s| s.starts_with("CREATE"))
        .collect();
    assert_eq!(ddl.len(), 3, "{ddl:?}");
    assert!(ddl[0].starts_with("CREATE DATABASE IF NOT EXISTS obs"));
    for request in &requests {
        // Every statement names `obs.<table>`; a `database` parameter
        // would fail before `CREATE DATABASE` runs.
        assert!(!request.params.contains_key("database"));
        assert_eq!(
            request.headers.get("x-clickhouse-user").map(String::as_str),
            Some("agent")
        );
        assert_eq!(
            request.headers.get("x-clickhouse-key").map(String::as_str),
            Some("hunter2")
        );
    }
    let events = inserted_events(&requests);
    let other_rows: Vec<i64> = events
        .iter()
        .filter(|(run, _)| *run == other_id)
        .map(|(_, s)| *s)
        .collect();
    assert_eq!(other_rows, [2], "only what ClickHouse lacks is sent");
    let local_live = store.get_latest_sequence(&live_id).unwrap();
    let live_rows: Vec<i64> = events
        .iter()
        .filter(|(run, _)| *run == live_id)
        .map(|(_, s)| *s)
        .collect();
    assert_eq!(live_rows, (1..=local_live).collect::<Vec<_>>());

    // A new event and the finish are sent by the final flush.
    live.handle_agent_event(&user("later"));
    live.stop();
    mirror.finish().await;
    let requests = fake.requests();
    let live_rows: Vec<i64> = inserted_events(&requests)
        .into_iter()
        .filter(|(run, _)| *run == live_id)
        .map(|(_, s)| s)
        .collect();
    let latest = store.get_latest_sequence(&live_id).unwrap();
    assert_eq!(
        live_rows,
        (1..=latest).collect::<Vec<_>>(),
        "no gaps or repeats"
    );
    let finished = inserted_runs(&requests)
        .into_iter()
        .rfind(|row| row["run_id"] == live_id.as_str())
        .unwrap();
    assert_eq!(finished["status"], "finished");
    assert!(finished["version"].as_u64().unwrap() >= FINISHED_VERSION_BASE);
    let status = clickhouse.status();
    assert_eq!(status.connected, Some(true));
    assert_eq!(
        status.events_sent as usize,
        inserted_events(&requests).len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_clickhouse_is_reported_without_blocking_stop() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let live = recorder(&store, "live");
    let fake = FakeClickHouse::start(Arc::new(|_| {
        (
            500,
            "Code: 516. DB::Exception: Authentication failed\nstack".into(),
        )
    }))
    .await;
    let clickhouse = Arc::new(ClickHouse::new(fake.config()).unwrap());
    let mirror =
        ClickHouseMirror::spawn(Arc::clone(&clickhouse), Arc::clone(&store), live.run_id());
    wait_for(&fake, |requests| !requests.is_empty()).await;
    live.stop();
    tokio::time::timeout(Duration::from_secs(10), mirror.finish())
        .await
        .expect("finish is bounded");
    let status = clickhouse.status();
    assert_eq!(status.connected, Some(false));
    assert_eq!(
        status.last_error.as_deref(),
        Some("ClickHouse HTTP 500: Code: 516. DB::Exception: Authentication failed")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_abandons_a_hung_sync() {
    // A server that accepts connections and never answers: without
    // cancellation, stop would wait out the 10 s request timeout first.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hold = tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            open.push(stream);
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let live = recorder(&store, "live");
    let clickhouse = Arc::new(
        ClickHouse::new(ClickHouseConfig {
            url,
            database: "obs".into(),
            user: None,
            password: None,
        })
        .unwrap(),
    );
    let mirror =
        ClickHouseMirror::spawn(Arc::clone(&clickhouse), Arc::clone(&store), live.run_id());
    tokio::time::sleep(Duration::from_millis(200)).await;
    live.stop();
    let started = std::time::Instant::now();
    mirror.finish().await;
    let waited = started.elapsed();
    assert!(
        waited < FINAL_FLUSH + Duration::from_secs(1),
        "stop took {waited:?}"
    );
    assert_eq!(
        clickhouse.status().last_error.as_deref(),
        Some("final ClickHouse sync timed out")
    );
    hold.abort();
}

const RUN_ROW: &str = r#"{"run_id":"r1","session_id":"s1","main_thread_id":"m1","session_name":null,"model":"p/m","thinking_level":null,"started_at_ms":1000,"ended_at_ms":5000,"status":"finished","version":1}"#;

fn analytics_fake() -> Responder {
    Arc::new(|request| {
        let sql = request.statement();
        let body = if sql.contains("argMax(ts_ms") {
            [
                r#"{"run_id":"r1","item_id":1,"ts":1500,"kind":"tool","tool_name":"edit","state":"done","duration_ms":40,"thread_kind":"main","role":"","path":"a.rs","input":0,"output":0,"cache_read":0,"cache_write":0,"cost":0,"provider":"","model":"","stop_reason":""}"#,
                r#"{"run_id":"r1","item_id":2,"ts":1600,"kind":"message","tool_name":"","state":"","duration_ms":null,"thread_kind":"main","role":"assistant","path":"","input":9,"output":3,"cache_read":90,"cache_write":0,"cost":0.25,"provider":"p","model":"m","stop_reason":"stop"}"#,
                r#"{"run_id":"r1","item_id":3,"ts":1700,"kind":"brand-new-kind","tool_name":"","state":"","duration_ms":null,"thread_kind":"main","role":"","path":"","input":0,"output":0,"cache_read":0,"cache_write":0,"cost":0,"provider":"","model":"","stop_reason":""}"#,
            ]
            .join("\n")
        } else if sql.contains("max(ts_ms)") {
            r#"{"run_id":"r1","last_ts":4000}"#.to_string()
        } else if sql.contains("observe_runs FINAL") {
            RUN_ROW.to_string()
        } else if sql.contains("observe_events FINAL") {
            let payload = serde_json::json!({
                "type": "activity",
                "item": {
                    "threadKey": "m1", "threadId": "m1", "mainThreadId": "m1",
                    "threadKind": "main", "threadName": "main", "threadColor": "blue",
                    "runId": "r1", "id": 1, "ts": 1500, "kind": "message", "text": "hi"
                }
            });
            serde_json::json!({ "sequence": 1, "payload": payload.to_string() }).to_string()
        } else {
            String::new()
        };
        (200, body)
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_remote_replay_is_rejected_instead_of_truncated() {
    let responder = analytics_fake();
    let fake = FakeClickHouse::start(Arc::new(move |request| {
        if request.statement().contains("SELECT sequence, payload") {
            assert!(request
                .statement()
                .contains(&format!("LIMIT {}", MAX_STATE_EVENTS + 1)));
            return (
                200,
                "{\"sequence\":1,\"payload\":\"{}\"}\n".repeat((MAX_STATE_EVENTS + 1) as usize),
            );
        }
        responder(request)
    }))
    .await;
    let clickhouse = Arc::new(ClickHouse::new(fake.config()).unwrap());
    let error = tokio::task::spawn_blocking(move || clickhouse.run_state("r1"))
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.contains("exceeds the replay limit"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn clickhouse_facts_feed_the_shared_analytics() {
    let fake = FakeClickHouse::start(analytics_fake()).await;
    let clickhouse = Arc::new(ClickHouse::new(fake.config()).unwrap());
    let source = Arc::clone(&clickhouse);
    let (overview, list, state) = tokio::task::spawn_blocking(move || {
        let query = OverviewQuery {
            since: 0,
            until: 10_000,
            bucket_ms: None,
            tz_offset_min: 0,
        };
        (
            analytics::overview(source.as_ref(), &query).unwrap(),
            analytics::run_list(source.as_ref(), 0, 10_000).unwrap(),
            source.run_state("r1").unwrap().unwrap(),
        )
    })
    .await
    .unwrap();
    assert_eq!(overview.source, "clickhouse");
    assert_eq!(overview.totals.tool_calls, 1);
    assert_eq!(overview.totals.turns, 1);
    assert_eq!(overview.totals.tokens.cache_read, 90);
    assert_eq!(overview.models[0].model, "p/m");
    assert_eq!(overview.files[0].path, "a.rs");
    assert_eq!(overview.kinds.len(), 2, "unknown kinds are skipped");
    let run = &list.runs[0];
    assert_eq!(run.summary.status, ObserveRunStatus::Finished);
    assert_eq!(
        (run.last_activity, run.duration_ms),
        (Some(4000), Some(4000))
    );
    assert_eq!(state.header.run_id, "r1");
    assert_eq!(state.items[0].text.as_deref(), Some("hi"));

    let items_query = fake
        .requests()
        .into_iter()
        .find(|r| r.statement().contains("argMax(ts_ms"))
        .unwrap();
    assert!(items_query.statement().contains("FROM obs.observe_events"));
    assert_eq!(
        items_query.params.get("param_since").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        items_query.params.get("param_until").map(String::as_str),
        Some("10000")
    );
    assert_eq!(
        items_query
            .params
            .get("output_format_json_quote_64bit_integers")
            .map(String::as_str),
        Some("0")
    );
}

async fn http_get(port: u16, path: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer token-abc\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").unwrap().1.to_string();
    (status, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_prefers_fresh_sqlite_and_allows_explicit_clickhouse() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let live = recorder(&store, "live");
    live.handle_agent_event(&user("hello"));
    let fake = FakeClickHouse::start(Arc::new(|_| (503, "down".into()))).await;
    let server = start_observe_server(ObserveServerOptions {
        port: 0,
        recorder: live.clone(),
        store: Some(Arc::clone(&store)),
        clickhouse: Some(Arc::new(ClickHouse::new(fake.config()).unwrap())),
        token: Some("token-abc".into()),
    })
    .await
    .unwrap();

    let (status, body) = http_get(server.port, "/api/meta").await;
    assert_eq!(status, 200);
    let meta: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(meta["sources"], serde_json::json!(["sqlite", "clickhouse"]));
    assert_eq!(meta["defaultSource"], "sqlite");
    assert_eq!(meta["clickhouse"]["database"], "obs");
    assert!(!body.contains("hunter2"), "meta never exposes the password");

    let (status, body) = http_get(server.port, "/api/runs?since=0").await;
    assert_eq!(status, 200, "{body}");
    let list: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(list["source"], "sqlite");
    assert!(list.get("sourceError").is_none());
    assert!(
        fake.requests().is_empty(),
        "local analytics never waits for the mirror"
    );
    assert_eq!(list["runs"].as_array().unwrap().len(), 1);

    let (status, body) = http_get(server.port, "/api/overview?source=clickhouse").await;
    assert_eq!(status, 502);
    assert!(body.contains("ClickHouse HTTP 503"), "{body}");
    server.close().await;
    live.stop();
}

/// Round trip through a real server: schema, mirror, facts and replay.
/// Run with a disposable ClickHouse, e.g.
/// `ICERS_TEST_CLICKHOUSE_URL=http://127.0.0.1:8123 cargo test -p ice-cli live_clickhouse -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ICERS_TEST_CLICKHOUSE_URL"]
async fn live_clickhouse_round_trip() {
    let Ok(url) = std::env::var("ICERS_TEST_CLICKHOUSE_URL") else {
        return;
    };
    let database = format!("icers_test_{}", now_ms());
    let config = ClickHouseConfig {
        url,
        database: database.clone(),
        user: std::env::var("ICERS_TEST_CLICKHOUSE_USER").ok(),
        password: std::env::var("ICERS_TEST_CLICKHOUSE_PASSWORD").ok(),
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let live = recorder(&store, "live");
    live.handle_agent_event(&user("hello"));
    live.handle_agent_event(&AgentEvent::ToolExecutionStart {
        tool_call_id: "c1".into(),
        tool_name: "write".into(),
        args: serde_json::json!({ "path": "x.txt", "content": "x" }),
    });
    let run_id = live.run_id();
    let clickhouse = Arc::new(ClickHouse::new(config).unwrap());
    let mirror =
        ClickHouseMirror::spawn(Arc::clone(&clickhouse), Arc::clone(&store), run_id.clone());
    live.stop();
    mirror.finish().await;
    assert_eq!(
        clickhouse.status().connected,
        Some(true),
        "{:?}",
        clickhouse.status()
    );

    let source = Arc::clone(&clickhouse);
    let id = run_id.clone();
    let (list, state, items) = tokio::task::spawn_blocking(move || {
        (
            analytics::run_list(source.as_ref(), 0, i64::MAX).unwrap(),
            source.run_state(&id).unwrap().unwrap(),
            source.items_between(0, i64::MAX).unwrap(),
        )
    })
    .await
    .unwrap();
    assert_eq!(list.runs.len(), 1);
    assert_eq!(list.runs[0].summary.status, ObserveRunStatus::Finished);
    assert_eq!(list.runs[0].files, 1);
    assert_eq!(list.runs[0].prompt.as_deref(), Some("hello"));
    assert_eq!(
        state.items.len(),
        store.get_state(&run_id, None).unwrap().unwrap().items.len()
    );
    assert!(items.iter().any(|i| i.path.as_deref() == Some("x.txt")));
    let _ = clickhouse
        .post(None, &[], format!("DROP DATABASE IF EXISTS {database}"))
        .await;
}
