//! Loopback observation dashboard. Mirrors TS `observe-server.ts`
//! (`startObserveServer`, `loadObserveToken`; OBS-014..OBS-016): a read-only
//! HTTP + Server-Sent Events service on `127.0.0.1`, behind a per-user token.
//!
//! It accepts no agent commands. Routes: `/` (dashboard) and its assets,
//! `/state`, `/runs`, `/replay[/<run id>|/events]`, `/events` (SSE) and the
//! icers-only analytics under `/api/`. The Cognee `/knowledge` routes are not
//! ported yet.
//!
//! Rust differences, deliberately:
//!
//! - icers identity: default port [`OBSERVE_DEFAULT_PORT`] (4659, not TS 4649)
//!   and cookie [`OBSERVE_COOKIE`], so a TS ice dashboard on the same host is
//!   never shared or authenticated against.
//! - Only `GET`/`HEAD` reach the routes (TS ignores the method); only `/` and
//!   `/index.html` serve the page (TS matches any `/index*` prefix).
//! - Foreign runs in the combined view are folded from their full history,
//!   not the first 250 events, so late subscribers see the latest items.
//! - Each SSE client has its own bounded queue; a client that falls behind
//!   is sent a fresh snapshot instead of buffering without limit.
//! - A port already in use is reported as `shared` without checking who owns
//!   it when a persistent token is available; the local recorder keeps
//!   recording into the shared store.
//! - The dashboard is an Angular bundle built from `web/observe-ui`
//!   (`scripts/build-observe-ui.sh`) and served with a strict CSP.
//! - `/api/meta`, `/api/overview` and `/api/runs` add cross-run analytics,
//!   read from ClickHouse when it is configured and from SQLite otherwise.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::analytics::{self, AnalyticsSource, OverviewQuery, DAY_MS};
use super::clickhouse::ClickHouse;
use super::recorder::{now_ms, ObserveRecorder};
use super::store::{ObserveRunStore, OBSERVE_RETENTION_MS};
use super::types::*;

pub const OBSERVE_DEFAULT_PORT: u16 = 4659;
pub const OBSERVE_COOKIE: &str = "icers_observe_token";
/// Mirrors TS `COMBINED_RUN_LIMIT`.
pub const COMBINED_RUN_LIMIT: usize = 50;
/// Mirrors TS `LIVE_RUN_ACTIVITY_MS`.
pub const LIVE_RUN_ACTIVITY_MS: i64 = 600_000;
/// Mirrors TS `COMBINED_ITEM_LIMIT`.
pub const COMBINED_ITEM_LIMIT: usize = 1_000;
/// Mirrors TS `SHARED_POLL_MS`.
pub const SHARED_POLL_MS: u64 = 500;
/// Mirrors the TS foreign replay page size.
pub const FOREIGN_PAGE_LIMIT: f64 = 500.0;
const HEARTBEAT: Duration = Duration::from_secs(15);
const MAX_REQUEST_HEAD: usize = 16 * 1024;

/// Default analytics windows.
const OVERVIEW_WINDOW_MS: i64 = 7 * DAY_MS;
const RUN_LIST_WINDOW_MS: i64 = 30 * DAY_MS;

/// The dashboard bundle, built from `web/observe-ui` by
/// `scripts/build-observe-ui.sh`.
static DASHBOARD_HTML: &str = include_str!("ui/index.html");
static UI_ASSETS: &[(&str, &str, &[u8])] = &[
    (
        "/main.js",
        "text/javascript; charset=utf-8",
        include_bytes!("ui/main.js"),
    ),
    (
        "/styles.css",
        "text/css; charset=utf-8",
        include_bytes!("ui/styles.css"),
    ),
    // License notices of the bundled npm packages.
    (
        "/licenses.txt",
        "text/plain; charset=utf-8",
        include_bytes!("ui/licenses.txt"),
    ),
];
/// The page renders untrusted tool output; allow nothing but this origin.
/// Inline styles are needed by Angular component styles and charts.
const DASHBOARD_CSP: &str =
    "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'self'; \
     form-action 'none'; frame-ancestors 'none'";

/// Read or create the persistent dashboard token. Mirrors TS
/// `loadObserveToken`: an existing nonempty token is reused; otherwise 32
/// random bytes (hex) are written with `create_new` and mode 0600 under a
/// 0700 directory. A concurrent writer that won is read back.
pub fn load_observe_token(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(existing) if !existing.trim().is_empty() => {
            set_private_token_permissions(path)?;
            return Ok(existing.trim().to_string());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let token = random_token()?;
    if let Some(parent) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent).map_err(|e| e.to_string())?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(format!("{token}\n").as_bytes())
                .map_err(|e| e.to_string())?;
            Ok(token)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let existing = existing.trim();
            if existing.is_empty() {
                Err(format!("{} exists but is empty", path.display()))
            } else {
                set_private_token_permissions(path)?;
                Ok(existing.to_string())
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

fn set_private_token_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() {
            return Err("observe token path must be a regular file".into());
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Constant-time token comparison. Mirrors TS `tokensMatch`.
pub fn tokens_match(expected: &str, provided: Option<&str>) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    let (a, b) = (expected.as_bytes(), provided.as_bytes());
    if a.is_empty() || a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Loopback Host check. Mirrors TS `ALLOWED_HOST`.
pub fn host_allowed(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let lower = host.to_ascii_lowercase();
    let name = ["localhost", "127.0.0.1", "[::1]"]
        .into_iter()
        .find(|name| lower.starts_with(name));
    match name {
        Some(name) => {
            let rest = &lower[name.len()..];
            rest.is_empty()
                || rest.strip_prefix(':').is_some_and(|port| {
                    !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
                })
        }
        None => false,
    }
}

/// Options for [`start_observe_server`].
pub struct ObserveServerOptions {
    /// `0` binds an ephemeral port.
    pub port: u16,
    pub recorder: ObserveRecorder,
    pub store: Option<Arc<ObserveRunStore>>,
    /// Analytics backend preferred over `store` when configured.
    pub clickhouse: Option<Arc<ClickHouse>>,
    /// `None` generates a per-process token.
    pub token: Option<String>,
}

/// A started (or shared) dashboard. Mirrors TS `ObserveServer`.
pub struct ObserveServer {
    pub url: String,
    pub dashboard_url: String,
    pub port: u16,
    /// Another process already serves this port.
    pub shared: bool,
    cancel: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ObserveServer {
    /// Stop accepting and end every client stream.
    pub async fn close(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

struct ServerState {
    recorder: ObserveRecorder,
    store: Option<Arc<ObserveRunStore>>,
    clickhouse: Option<Arc<ClickHouse>>,
    token: String,
    cancel: CancellationToken,
}

/// Start the dashboard. Mirrors TS `startObserveServer`.
pub async fn start_observe_server(options: ObserveServerOptions) -> Result<ObserveServer, String> {
    let persistent_token = options.token.is_some();
    let token = match options.token {
        Some(token) => token,
        None => random_token()?,
    };
    let requested = options.port;
    let listener = match TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], requested))).await {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && !persistent_token => {
            return Err(
                "cannot reuse an occupied observe port without a persistent observe token".into(),
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            let url = format!("http://127.0.0.1:{requested}");
            return Ok(ObserveServer {
                dashboard_url: format!("{url}/?token={token}"),
                url,
                port: requested,
                shared: true,
                cancel: CancellationToken::new(),
                task: None,
            });
        }
        Err(e) => return Err(e.to_string()),
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let cancel = CancellationToken::new();
    let state = Arc::new(ServerState {
        recorder: options.recorder,
        store: options.store,
        clickhouse: options.clickhouse,
        token: token.clone(),
        cancel: cancel.clone(),
    });
    let accept_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = accept_cancel.cancelled() => break,
                accepted = listener.accept() => {
                    if let Ok((stream, _)) = accepted {
                        connections.spawn(handle_connection(stream, Arc::clone(&state)));
                    }
                }
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
            }
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
    });
    let url = format!("http://127.0.0.1:{port}");
    Ok(ObserveServer {
        dashboard_url: format!("{url}/?token={token}"),
        url,
        port,
        shared: false,
        cancel,
        task: Some(task),
    })
}

/// A parsed request head.
#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: HashMap<String, String>,
}

fn decode_component(raw: &str) -> Option<String> {
    let spaced = raw.replace('+', " ");
    percent_encoding::percent_decode_str(&spaced)
        .decode_utf8()
        .ok()
        .map(|s| s.into_owned())
}

/// Parse an HTTP/1.x request head. Returns `None` for a malformed head.
pub fn parse_request(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut parts = lines.next()?.split(' ');
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    let version = parts.next()?;
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") || parts.next().is_some() {
        return None;
    }
    let (path, query_str) = target.split_once('?').unwrap_or((target, ""));
    let mut query = BTreeMap::new();
    for pair in query_str.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if let (Some(k), Some(v)) = (decode_component(k), decode_component(v)) {
            query.entry(k).or_insert(v);
        }
    }
    let mut headers = HashMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (name, value) = line.split_once(':')?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    Some(Request {
        method,
        path: path.to_string(),
        query,
        headers,
    })
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    fn bearer(&self) -> Option<&str> {
        let value = self.header("authorization")?;
        let (scheme, rest) = value.split_once(char::is_whitespace)?;
        scheme
            .eq_ignore_ascii_case("bearer")
            .then(|| rest.trim())
            .filter(|t| !t.is_empty())
    }

    fn cookie(&self, name: &str) -> Option<&str> {
        self.header("cookie")?.split(';').find_map(|part| {
            let (k, v) = part.trim().split_once('=')?;
            (k == name).then_some(v)
        })
    }

    fn query_number(&self, key: &str) -> Option<f64> {
        let raw = self.query.get(key)?.trim();
        if raw.is_empty() {
            return None;
        }
        raw.parse::<f64>().ok().filter(|n| n.is_finite())
    }
}

/// A complete (non-streaming) response.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        502 => "Bad Gateway",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}

fn text(status: u16, body: &str) -> Response {
    Response {
        status,
        content_type: "text/plain; charset=utf-8",
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn json(status: u16, value: &impl serde::Serialize) -> Response {
    Response {
        status,
        content_type: "application/json; charset=utf-8",
        headers: Vec::new(),
        body: serde_json::to_vec(value).unwrap_or_default(),
    }
}

fn json_error(status: u16, message: &str) -> Response {
    json(status, &serde_json::json!({ "error": message }))
}

/// Outcome of routing one request.
enum Routed {
    Done(Response),
    Events,
}

async fn handle_connection(mut stream: TcpStream, state: Arc<ServerState>) {
    let mut buf = Vec::with_capacity(1024);
    let head = loop {
        let mut chunk = [0u8; 2048];
        let read = tokio::select! {
            _ = state.cancel.cancelled() => return,
            read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk)) => read,
        };
        match read {
            Ok(Ok(0)) | Err(_) | Ok(Err(_)) => return,
            Ok(Ok(n)) => buf.extend_from_slice(&chunk[..n]),
        }
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break String::from_utf8_lossy(&buf[..end]).into_owned();
        }
        if buf.len() > MAX_REQUEST_HEAD {
            let _ = write_response(&mut stream, &text(400, "bad request"), false).await;
            return;
        }
    };
    let Some(request) = parse_request(&head) else {
        let _ = write_response(&mut stream, &text(400, "bad request"), false).await;
        return;
    };
    let head_only = request.method == "HEAD";
    // Routes read SQLite synchronously; keep them off the async workers.
    let route_state = Arc::clone(&state);
    let routed = match tokio::task::spawn_blocking(move || route(&route_state, &request)).await {
        Ok(routed) => routed,
        Err(_) => Routed::Done(text(500, "internal error")),
    };
    match routed {
        Routed::Done(response) => {
            let _ = write_response(&mut stream, &response, head_only).await;
        }
        Routed::Events => stream_events(stream, state).await,
    }
}

async fn write_response(
    stream: &mut TcpStream,
    response: &Response,
    head_only: bool,
) -> std::io::Result<()> {
    let mut out = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        response.status,
        status_text(response.status),
        response.content_type,
        response.body.len()
    );
    for (name, value) in &response.headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    stream.write_all(out.as_bytes()).await?;
    if !head_only {
        stream.write_all(&response.body).await?;
    }
    stream.shutdown().await
}

fn route(state: &ServerState, request: &Request) -> Routed {
    if !host_allowed(request.header("host")) {
        return Routed::Done(text(403, "forbidden"));
    }
    let query_token = request.query.get("token").map(String::as_str);
    let via_query = tokens_match(&state.token, query_token);
    let authed = via_query
        || tokens_match(&state.token, request.bearer())
        || tokens_match(&state.token, request.cookie(OBSERVE_COOKIE));
    if !authed {
        return Routed::Done(text(
            401,
            "unauthorized: open the dashboard URL printed by `icers --observe`",
        ));
    }
    if request.method != "GET" && request.method != "HEAD" {
        return Routed::Done(text(405, "method not allowed"));
    }
    let path = request.path.as_str();
    Routed::Done(match path {
        "/" | "/index.html" => {
            let mut response = Response {
                status: 200,
                content_type: "text/html; charset=utf-8",
                headers: vec![
                    ("Referrer-Policy", "no-referrer".into()),
                    ("Content-Security-Policy", DASHBOARD_CSP.into()),
                ],
                body: DASHBOARD_HTML.as_bytes().to_vec(),
            };
            if via_query {
                response.headers.push((
                    "Set-Cookie",
                    format!(
                        "{OBSERVE_COOKIE}={}; HttpOnly; SameSite=Strict; Path=/",
                        state.token
                    ),
                ));
            }
            response
        }
        "/state" => state_route(state, request),
        "/runs" => match &state.store {
            Some(store) => match store.list_runs(request.query_number("limit")) {
                Ok(runs) => json(200, &serde_json::json!({ "runs": runs })),
                Err(e) => json_error(500, &e),
            },
            None => text(404, "not found"),
        },
        "/events" => return Routed::Events,
        "/api/meta" => json(200, &meta(state)),
        "/api/overview" => overview_route(state, request),
        "/api/runs" => run_list_route(state, request),
        _ if path == "/replay" || path.starts_with("/replay/") => replay_route(state, request),
        _ => match UI_ASSETS.iter().find(|(asset, _, _)| *asset == path) {
            Some((_, content_type, body)) => Response {
                status: 200,
                content_type,
                headers: Vec::new(),
                body: body.to_vec(),
            },
            None => text(404, "not found"),
        },
    })
}

fn sources(state: &ServerState) -> Vec<&dyn AnalyticsSource> {
    let mut sources: Vec<&dyn AnalyticsSource> = Vec::new();
    // The mirror is eventually consistent; prefer freshly persisted local
    // facts unless the caller explicitly selects shared ClickHouse history.
    if let Some(store) = &state.store {
        sources.push(store.as_ref());
    }
    if let Some(clickhouse) = &state.clickhouse {
        sources.push(clickhouse.as_ref());
    }
    sources
}

fn meta(state: &ServerState) -> serde_json::Value {
    let names: Vec<&str> = sources(state).iter().map(|s| s.name()).collect();
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "liveRunId": state.recorder.run_id(),
        "retentionMs": OBSERVE_RETENTION_MS,
        "sources": names,
        "defaultSource": names.first(),
        "clickhouse": state.clickhouse.as_ref().map(|c| c.status()),
    })
}

/// Run `query` on the requested source, or on each configured source in
/// preference order until one answers. A fallback names the failed source
/// in `sourceError`.
fn with_source<T: serde::Serialize>(
    state: &ServerState,
    request: &Request,
    query: impl Fn(&dyn AnalyticsSource) -> Result<T, String>,
) -> Response {
    let mut candidates = sources(state);
    if let Some(wanted) = request.query.get("source").filter(|s| !s.is_empty()) {
        candidates.retain(|s| s.name() == wanted);
        if candidates.is_empty() {
            return json_error(404, &format!("source {wanted} is not configured"));
        }
    }
    let mut failure: Option<String> = None;
    for source in candidates {
        match query(source) {
            Ok(value) => {
                let mut value = serde_json::to_value(value).unwrap_or_default();
                if let (Some(error), Some(object)) = (failure, value.as_object_mut()) {
                    object.insert("sourceError".into(), error.into());
                }
                return json(200, &value);
            }
            Err(error) => failure = Some(format!("{}: {error}", source.name())),
        }
    }
    match failure {
        Some(error) => json_error(502, &error),
        None => json_error(404, "no history store"),
    }
}

/// `since`/`until` in epoch ms, restricted to years 1970 through 9999 so
/// bucket alignment and previous-period arithmetic cannot overflow.
fn window(request: &Request, default_span: i64) -> Result<(i64, i64), &'static str> {
    let timestamp = |key: &str, default: i64| -> Result<i64, &'static str> {
        if !request.query.contains_key(key) {
            return Ok(default);
        }
        let value = request.query_number(key).ok_or("invalid time window")?;
        if !(0.0..=analytics::MAX_TIMESTAMP as f64).contains(&value) {
            return Err("invalid time window");
        }
        Ok(value as i64)
    };
    let until = timestamp("until", now_ms())?;
    let since = timestamp("since", until.saturating_sub(default_span).max(0))?;
    if since >= until {
        return Err("since must be earlier than until");
    }
    Ok((since, until))
}

fn overview_route(state: &ServerState, request: &Request) -> Response {
    let (since, until) = match window(request, OVERVIEW_WINDOW_MS) {
        Ok(window) => window,
        Err(error) => return json_error(400, error),
    };
    let query = OverviewQuery {
        since,
        until,
        bucket_ms: request.query_number("bucket").map(|n| n as i64),
        tz_offset_min: request.query_number("tz").map(|n| n as i64).unwrap_or(0),
    };
    with_source(state, request, |source| analytics::overview(source, &query))
}

fn run_list_route(state: &ServerState, request: &Request) -> Response {
    let (since, until) = match window(request, RUN_LIST_WINDOW_MS) {
        Ok(window) => window,
        Err(error) => return json_error(400, error),
    };
    with_source(state, request, |source| {
        analytics::run_list(source, since, until)
    })
}

fn state_route(state: &ServerState, request: &Request) -> Response {
    if request.query.get("view").map(String::as_str) == Some("all") {
        return json(200, &combined_state(state));
    }
    let Some(run_id) = request.query.get("runId").filter(|s| !s.is_empty()) else {
        return json(200, &state.recorder.get_state());
    };
    if state.store.is_none() && state.clickhouse.is_none() {
        return json(200, &state.recorder.get_state());
    }
    // Local history first; ClickHouse can hold runs past the retention.
    let mut failure = None;
    for source in sources(state) {
        match source.run_state(run_id) {
            Ok(Some(run_state)) => return json(200, &run_state),
            Ok(None) => {}
            Err(e) => failure = Some(e),
        }
    }
    match failure {
        Some(e) => json_error(500, &e),
        None => json_error(404, "run not found"),
    }
}

fn replay_route(state: &ServerState, request: &Request) -> Response {
    let Some(store) = &state.store else {
        return text(404, "not found");
    };
    let suffix = request
        .path
        .strip_prefix("/replay")
        .unwrap_or("")
        .trim_start_matches('/');
    let path_id = if suffix.is_empty() || suffix == "events" {
        None
    } else {
        match percent_encoding::percent_decode_str(suffix).decode_utf8() {
            Ok(id) => Some(id.into_owned()),
            Err(_) => return json_error(400, "malformed run id"),
        }
    };
    let run = if let Some(id) = path_id.or_else(|| {
        request
            .query
            .get("runId")
            .filter(|s| !s.is_empty())
            .cloned()
    }) {
        store.get_run(&id)
    } else if let Some(session) = request.query.get("sessionId").filter(|s| !s.is_empty()) {
        store.get_latest_run_for_session(session)
    } else {
        return json_error(404, "run not found");
    };
    match run {
        Ok(Some(run)) => match store.get_events(
            &run.run_id,
            request.query_number("after"),
            request.query_number("limit"),
        ) {
            Ok(events) => json(200, &serde_json::json!({ "run": run, "events": events })),
            Err(e) => json_error(500, &e),
        },
        Ok(None) => json_error(404, "run not found"),
        Err(e) => json_error(500, &e),
    }
}

fn current_summary(header: &ObserveHeader) -> ObserveRunSummary {
    ObserveRunSummary {
        run_id: header.run_id.clone(),
        session_id: header.session_id.clone(),
        main_thread_id: header.main_thread_id.clone(),
        session_name: header.session_name.clone(),
        model: header.model.clone(),
        thinking_level: header.thinking_level.clone(),
        started_at: header.started_at,
        ended_at: None,
        status: ObserveRunStatus::Running,
    }
}

/// Recent live runs plus the current one, newest first. Mirrors TS
/// `combinedRuns`.
fn combined_runs(state: &ServerState) -> Vec<ObserveRunSummary> {
    let header = state.recorder.get_header();
    let mut runs: Vec<ObserveRunSummary> = state
        .store
        .as_ref()
        .and_then(|store| {
            store
                .list_live_runs(
                    now_ms() - LIVE_RUN_ACTIVITY_MS,
                    Some(COMBINED_RUN_LIMIT as f64),
                )
                .ok()
        })
        .unwrap_or_default();
    runs.retain(|run| run.run_id != header.run_id);
    runs.push(current_summary(&header));
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    runs.truncate(COMBINED_RUN_LIMIT);
    runs
}

/// Merge the current recorder state with every recent live run. Mirrors TS
/// `combinedState`; the top-level header is the current recorder's.
pub(crate) fn combined_state_from(
    recorder: &ObserveRecorder,
    store: Option<&ObserveRunStore>,
    runs: Vec<ObserveRunSummary>,
) -> CombinedObserveState {
    let current = recorder.get_state();
    let live_run_id = current.header.run_id.clone();
    let mut items = Vec::new();
    let mut lanes = Vec::new();
    let mut threads: indexmap::IndexMap<String, ObserveThread> = indexmap::IndexMap::new();
    for run in &runs {
        let run_state = if run.run_id == live_run_id {
            Some(current.clone())
        } else {
            store.and_then(|s| s.get_state(&run.run_id, None).ok().flatten())
        };
        let Some(run_state) = run_state else {
            continue;
        };
        for mut item in run_state.items {
            if item.run_id.is_empty() {
                item.run_id = run.run_id.clone();
            }
            items.push(item);
        }
        for mut lane in run_state.lanes {
            lane.run_id.get_or_insert_with(|| run.run_id.clone());
            lanes.push(lane);
        }
        for mut thread in run_state.threads {
            thread.run_id.get_or_insert_with(|| run.run_id.clone());
            let key = format!("{}:{}", run.run_id, thread.identity.thread_key);
            threads.entry(key).or_insert(thread);
        }
    }
    items.sort_by(|a, b| (a.ts, &a.run_id, a.id).cmp(&(b.ts, &b.run_id, b.id)));
    if items.len() > COMBINED_ITEM_LIMIT {
        items.drain(..items.len() - COMBINED_ITEM_LIMIT);
    }
    CombinedObserveState {
        state: ObserveState {
            header: current.header,
            items,
            lanes,
            threads: threads.into_values().collect(),
        },
        view: "all".into(),
        live_run_id,
        runs,
    }
}

fn combined_state(state: &ServerState) -> CombinedObserveState {
    combined_state_from(
        &state.recorder,
        state.store.as_deref(),
        combined_runs(state),
    )
}

/// SSE frame. Mirrors TS `writeSse`.
pub fn sse_frame(event: &str, data: &impl serde::Serialize) -> String {
    format!(
        "event: {event}\ndata: {}\n\n",
        serde_json::to_string(data).unwrap_or_else(|_| "null".into())
    )
}

fn live_frame(event: &ObserveRecorderEvent) -> String {
    match event {
        ObserveRecorderEvent::Activity { item } => sse_frame("activity", item),
        ObserveRecorderEvent::ActivityUpdate { item } => sse_frame("activity_update", item),
        ObserveRecorderEvent::Lane { lane } => sse_frame("lane", lane),
        ObserveRecorderEvent::Thread { thread } => sse_frame("thread", thread),
        ObserveRecorderEvent::Header { header } => sse_frame("header", header),
    }
}

fn runs_signature(runs: &[ObserveRunSummary]) -> String {
    runs.iter()
        .map(|r| format!("{}:{}:{:?}", r.run_id, r.status.as_str(), r.ended_at))
        .collect::<Vec<_>>()
        .join("|")
}

/// Foreign-run tail state for one SSE client. Mirrors the TS shared poller.
struct ForeignCursors {
    cursors: HashMap<String, i64>,
    signature: String,
}

impl ForeignCursors {
    fn new(state: &ServerState, runs: &[ObserveRunSummary]) -> Self {
        let live = state.recorder.run_id();
        let mut cursors = HashMap::new();
        if let Some(store) = &state.store {
            for run in runs.iter().filter(|r| r.run_id != live) {
                let latest = store.get_latest_sequence(&run.run_id).unwrap_or(0);
                cursors.insert(run.run_id.clone(), latest);
            }
        }
        Self {
            cursors,
            signature: runs_signature(runs),
        }
    }

    /// One poll tick: frames for new runs, run-list changes and new foreign
    /// events. Mirrors TS `pollForeignRuns`.
    fn poll(&mut self, state: &ServerState) -> Vec<String> {
        let Some(store) = &state.store else {
            return Vec::new();
        };
        let mut frames = Vec::new();
        let runs = combined_runs(state);
        let live = state.recorder.run_id();
        let mut discovered = false;
        for run in runs.iter().filter(|r| r.run_id != live) {
            if !self.cursors.contains_key(&run.run_id) {
                let latest = store.get_latest_sequence(&run.run_id).unwrap_or(0);
                self.cursors.insert(run.run_id.clone(), latest);
                discovered = true;
            }
        }
        if discovered {
            frames.push(sse_frame(
                "snapshot",
                &combined_state_from(&state.recorder, Some(store), runs.clone()),
            ));
        }
        let signature = runs_signature(&runs);
        if signature != self.signature {
            self.signature = signature;
            frames.push(sse_frame("runs", &runs));
        }
        for run in runs.iter().filter(|r| r.run_id != live) {
            let cursor = self.cursors.get(&run.run_id).copied().unwrap_or(0);
            let Ok(events) =
                store.get_events(&run.run_id, Some(cursor as f64), Some(FOREIGN_PAGE_LIMIT))
            else {
                continue;
            };
            for event in events {
                self.cursors.insert(run.run_id.clone(), event.sequence);
                let frame = match event.payload {
                    ObserveRecorderEvent::Activity { mut item } => {
                        if item.run_id.is_empty() {
                            item.run_id = run.run_id.clone();
                        }
                        sse_frame("activity", &item)
                    }
                    ObserveRecorderEvent::ActivityUpdate { mut item } => {
                        if item.run_id.is_empty() {
                            item.run_id = run.run_id.clone();
                        }
                        sse_frame("activity_update", &item)
                    }
                    ObserveRecorderEvent::Thread { mut thread } => {
                        thread.run_id.get_or_insert_with(|| run.run_id.clone());
                        sse_frame("thread", &thread)
                    }
                    ObserveRecorderEvent::Lane { lane } => sse_frame("lane", &lane),
                    // A foreign header never replaces the current one.
                    ObserveRecorderEvent::Header { .. } => continue,
                };
                frames.push(frame);
            }
        }
        frames
    }
}

async fn stream_events(mut stream: TcpStream, state: Arc<ServerState>) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n";
    let mut live = state.recorder.subscribe();
    let runs = combined_runs(&state);
    let mut foreign = ForeignCursors::new(&state, &runs);
    let snapshot = sse_frame(
        "snapshot",
        &combined_state_from(&state.recorder, state.store.as_deref(), runs),
    );
    if stream.write_all(head.as_bytes()).await.is_err()
        || stream.write_all(snapshot.as_bytes()).await.is_err()
    {
        return;
    }
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.tick().await;
    let mut poll = tokio::time::interval(Duration::from_millis(SHARED_POLL_MS));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let frames: Vec<String> = tokio::select! {
            _ = state.cancel.cancelled() => break,
            received = live.recv() => match received {
                Ok(event) => vec![live_frame(&event)],
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let runs = combined_runs(&state);
                    vec![sse_frame(
                        "snapshot",
                        &combined_state_from(&state.recorder, state.store.as_deref(), runs),
                    )]
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = heartbeat.tick() => vec![": ping\n\n".to_string()],
            _ = poll.tick() => {
                let state = Arc::clone(&state);
                let mut cursors = std::mem::replace(
                    &mut foreign,
                    ForeignCursors { cursors: HashMap::new(), signature: String::new() },
                );
                match tokio::task::spawn_blocking(move || {
                    let frames = cursors.poll(&state);
                    (cursors, frames)
                })
                .await
                {
                    Ok((cursors, frames)) => {
                        foreign = cursors;
                        frames
                    }
                    Err(_) => break,
                }
            }
        };
        for frame in frames {
            if stream.write_all(frame.as_bytes()).await.is_err() {
                return;
            }
        }
    }
    let _ = stream.shutdown().await;
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
