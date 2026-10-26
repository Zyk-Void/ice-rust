//! Read-only Cognee browse calls for inspecting stored knowledge. Ports
//! `CogneeBrowseClient` from `ice-cognee-client.ts` (COG-021..030).
//!
//! Each call projects the server payload to the fields a viewer needs and
//! drops the rest (server filesystem paths, full chunk text). Projection is
//! not a credential redactor: all returned text is untrusted.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use super::client::{
    assert_ok, build_url, condense_graph_context, headers, is_uncognified_dataset, read_json,
    read_text, request, Body, CogneeError, CogneeErrorKind, CogneeTransport, HttpRequest, Method,
};
use crate::blackhole::js::{len16, prefix16};
use crate::blackhole::regex;

const MAX_DATASETS_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_DATA_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_STATUS_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_SEARCH_RESPONSE_BYTES: usize = 1024 * 1024;
pub const DEFAULT_DATA_ITEM_BYTES: usize = 256 * 1024;
const BROWSE_TIMEOUT_MS: u64 = 15_000;
const SEARCH_TIMEOUT_MS: u64 = 60_000;
const MAX_GRAPH_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_GRAPH_NODES: usize = 5_000;
pub const MAX_GRAPH_EDGES: usize = 25_000;
const MAX_GRAPH_NODE_TEXT: usize = 600;
const MAX_SESSIONS_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SESSION_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SESSION_TEXT: usize = 4_000;

/// Search types that return stored context without an LLM completion.
pub const BROWSE_SEARCH_TYPES: [&str; 4] =
    ["CHUNKS", "CHUNKS_LEXICAL", "SUMMARIES", "GRAPH_COMPLETION"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dataset {
    pub id: String,
    pub name: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataItem {
    pub id: String,
    pub name: String,
    pub extension: Option<String>,
    pub mime_type: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub dataset: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub r#type: String,
    pub text: Option<String>,
    pub sets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub total_nodes: usize,
    pub total_edges: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    pub dataset_id: Option<String>,
    pub status: Option<String>,
    pub started_at: Option<String>,
    pub last_activity_at: Option<String>,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub time: Option<String>,
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTrace {
    pub time: Option<String>,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionDetail {
    pub summary: SessionSummary,
    pub label: Option<String>,
    pub entries: Vec<SessionEntry>,
    pub traces: Vec<SessionTrace>,
    pub entry_count: f64,
    pub trace_count: f64,
}

fn malformed(action: &str) -> CogneeError {
    CogneeError::new(
        CogneeErrorKind::Malformed,
        format!("Cognee {action} returned an unexpected shape"),
    )
}

fn assert_uuid<'a>(value: &'a str, label: &str) -> Result<&'a str, CogneeError> {
    let uuid = regex!(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$");
    if uuid.is_match(value) {
        Ok(value)
    } else {
        Err(CogneeError::new(
            CogneeErrorKind::InvalidRequest,
            format!("Invalid Cognee {label} id"),
        ))
    }
}

/// A nonempty string, untrimmed.
fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn records<'a>(
    payload: &'a Value,
    action: &str,
) -> Result<Vec<&'a Map<String, Value>>, CogneeError> {
    payload
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_object).collect())
        .ok_or_else(|| malformed(action))
}

fn cap_text(value: Option<&Value>, max_chars: usize) -> String {
    let text = match value {
        Some(Value::String(text)) => text.clone(),
        None | Some(Value::Null) => String::new(),
        Some(other) => other.to_string(),
    };
    if len16(&text) > max_chars {
        format!("{}…", prefix16(&text, max_chars - 1))
    } else {
        text
    }
}

fn session_summary(value: &Map<String, Value>) -> Option<SessionSummary> {
    Some(SessionSummary {
        session_id: optional_string(value.get("session_id"))?,
        dataset_id: optional_string(value.get("dataset_id")),
        status: optional_string(value.get("effective_status"))
            .or_else(|| optional_string(value.get("status"))),
        started_at: optional_string(value.get("started_at")),
        last_activity_at: optional_string(value.get("last_activity_at")),
        ended_at: optional_string(value.get("ended_at")),
    })
}

fn session_traces(value: Option<&Value>) -> Vec<SessionTrace> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_object)
        .map(|trace| {
            let name = optional_string(trace.get("origin_function"))
                .or_else(|| optional_string(trace.get("method_name")))
                .unwrap_or_else(|| "trace".into());
            let mut rest = trace.clone();
            rest.shift_remove("origin_function");
            let time = rest.shift_remove("time");
            SessionTrace {
                time: optional_string(time.as_ref()),
                name,
                text: cap_text(Some(&Value::Object(rest)), MAX_SESSION_TEXT),
            }
        })
        .collect()
}

fn graph_node_text(properties: &Map<String, Value>) -> Option<String> {
    let text = optional_string(properties.get("description"))
        .or_else(|| optional_string(properties.get("text")))?;
    Some(if len16(&text) > MAX_GRAPH_NODE_TEXT {
        format!("{}…", prefix16(&text, MAX_GRAPH_NODE_TEXT - 1))
    } else {
        text
    })
}

/// Keep identity, label, type, a capped description and node sets. An
/// oversized graph keeps its best-connected nodes (stable by degree).
pub fn normalize_graph(payload: &Value) -> Result<Graph, CogneeError> {
    if !(payload.is_object() || payload.is_array()) {
        return Err(malformed("graph"));
    }
    let empty = json!([]);
    let field = |name: &str| match payload.get(name) {
        None | Some(Value::Null) => &empty,
        Some(value) => value,
    };
    let empty_properties = Map::new();
    let nodes: Vec<GraphNode> = records(field("nodes"), "graph")?
        .into_iter()
        .filter_map(|value| {
            let id = optional_string(value.get("id"))?;
            let properties = value
                .get("properties")
                .and_then(Value::as_object)
                .unwrap_or(&empty_properties);
            let sets = properties
                .get("belongs_to_set")
                .and_then(Value::as_array)
                .map(|sets| {
                    sets.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            Some(GraphNode {
                label: optional_string(value.get("label"))
                    .or_else(|| optional_string(properties.get("name")))
                    .unwrap_or_else(|| id.clone()),
                r#type: optional_string(value.get("type")).unwrap_or_else(|| "Node".into()),
                text: graph_node_text(properties),
                sets,
                id,
            })
        })
        .collect();
    let node_ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let edges: Vec<GraphEdge> = records(field("edges"), "graph")?
        .into_iter()
        .filter_map(|value| {
            let source = optional_string(value.get("source"))?;
            let target = optional_string(value.get("target"))?;
            if !node_ids.contains(source.as_str()) || !node_ids.contains(target.as_str()) {
                return None;
            }
            Some(GraphEdge {
                source,
                target,
                label: optional_string(value.get("label")).unwrap_or_default(),
            })
        })
        .collect();
    let total_nodes = nodes.len();
    let total_edges = edges.len();
    if total_nodes <= MAX_GRAPH_NODES && total_edges <= MAX_GRAPH_EDGES {
        return Ok(Graph {
            nodes,
            edges,
            total_nodes,
            total_edges,
            truncated: false,
        });
    }
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for edge in &edges {
        *degree.entry(edge.source.as_str()).or_default() += 1;
        *degree.entry(edge.target.as_str()).or_default() += 1;
    }
    let mut kept = nodes.clone();
    kept.sort_by_key(|node| std::cmp::Reverse(degree.get(node.id.as_str()).copied().unwrap_or(0)));
    kept.truncate(MAX_GRAPH_NODES);
    let kept_ids: HashSet<&str> = kept.iter().map(|node| node.id.as_str()).collect();
    let kept_edges = edges
        .iter()
        .filter(|edge| {
            kept_ids.contains(edge.source.as_str()) && kept_ids.contains(edge.target.as_str())
        })
        .take(MAX_GRAPH_EDGES)
        .cloned()
        .collect();
    Ok(Graph {
        nodes: kept,
        edges: kept_edges,
        total_nodes,
        total_edges,
        truncated: true,
    })
}

/// Read-only Cognee calls; it never adds, cognifies or deletes.
#[derive(Clone)]
pub struct BrowseClient {
    base_url: String,
    api_key: Option<String>,
    transport: Arc<dyn CogneeTransport>,
}

impl BrowseClient {
    pub fn new(
        base_url: &str,
        api_key: Option<String>,
        transport: Arc<dyn CogneeTransport>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            transport,
        }
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<String, CogneeError> {
        let mut url = reqwest::Url::parse(&build_url(&self.base_url, path)?).map_err(|_| {
            CogneeError::new(CogneeErrorKind::InvalidRequest, "Invalid Cognee base URL")
        })?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url.to_string())
    }

    fn get(&self, url: String) -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            url,
            headers: headers(self.api_key.as_deref(), None),
            body: Body::Empty,
        }
    }

    async fn get_json(
        &self,
        path: &str,
        action: &'static str,
        max_bytes: usize,
        signal: Option<&CancellationToken>,
        query: &[(&str, &str)],
    ) -> Result<Value, CogneeError> {
        let http = self.get(self.url(path, query)?);
        request(
            self.transport.as_ref(),
            http,
            signal,
            BROWSE_TIMEOUT_MS,
            |mut response| async move {
                assert_ok(&response, action)?;
                read_json(&mut response, max_bytes).await
            },
        )
        .await
    }

    pub async fn list_datasets(
        &self,
        signal: Option<&CancellationToken>,
    ) -> Result<Vec<Dataset>, CogneeError> {
        let payload = self
            .get_json(
                "/api/v1/datasets",
                "datasets",
                MAX_DATASETS_RESPONSE_BYTES,
                signal,
                &[],
            )
            .await?;
        Ok(records(&payload, "datasets")?
            .into_iter()
            .filter_map(|value| {
                Some(Dataset {
                    id: optional_string(value.get("id"))?,
                    name: optional_string(value.get("name"))?,
                    created_at: optional_string(value.get("createdAt")),
                    updated_at: optional_string(value.get("updatedAt")),
                })
            })
            .collect())
    }

    /// Document metadata; `rawDataLocation` (a server path) is dropped.
    pub async fn list_dataset_data(
        &self,
        dataset_id: &str,
        signal: Option<&CancellationToken>,
    ) -> Result<Vec<DataItem>, CogneeError> {
        let path = format!(
            "/api/v1/datasets/{}/data",
            assert_uuid(dataset_id, "dataset")?
        );
        let payload = self
            .get_json(&path, "dataset data", MAX_DATA_RESPONSE_BYTES, signal, &[])
            .await?;
        Ok(records(&payload, "dataset data")?
            .into_iter()
            .filter_map(|value| {
                Some(DataItem {
                    id: optional_string(value.get("id"))?,
                    name: optional_string(value.get("name"))?,
                    extension: optional_string(value.get("extension")),
                    mime_type: optional_string(value.get("mimeType")),
                    created_at: optional_string(value.get("createdAt")),
                    updated_at: optional_string(value.get("updatedAt")),
                })
            })
            .collect())
    }

    /// Processing status per dataset id; an empty list makes no request.
    pub async fn dataset_status(
        &self,
        dataset_ids: &[String],
        signal: Option<&CancellationToken>,
    ) -> Result<Vec<(String, String)>, CogneeError> {
        if dataset_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = dataset_ids
            .iter()
            .map(|id| assert_uuid(id, "dataset").map(|id| ("dataset", id)))
            .collect::<Result<Vec<_>, _>>()?;
        let payload = self
            .get_json(
                "/api/v1/datasets/status",
                "dataset status",
                MAX_STATUS_RESPONSE_BYTES,
                signal,
                &query,
            )
            .await?;
        let object = payload
            .as_object()
            .ok_or_else(|| malformed("dataset status"))?;
        Ok(object
            .iter()
            .filter_map(|(key, value)| value.as_str().map(|status| (key.clone(), status.into())))
            .collect())
    }

    /// A document's raw text, truncated (not rejected) past `max_bytes`.
    pub async fn read_data_item(
        &self,
        dataset_id: &str,
        data_id: &str,
        max_bytes: Option<usize>,
        signal: Option<&CancellationToken>,
    ) -> Result<(String, bool), CogneeError> {
        let path = format!(
            "/api/v1/datasets/{}/data/{}/raw",
            assert_uuid(dataset_id, "dataset")?,
            assert_uuid(data_id, "data")?
        );
        let http = self.get(build_url(&self.base_url, &path)?);
        let max_bytes = max_bytes.unwrap_or(DEFAULT_DATA_ITEM_BYTES);
        request(
            self.transport.as_ref(),
            http,
            signal,
            BROWSE_TIMEOUT_MS,
            |mut response| async move {
                assert_ok(&response, "data item")?;
                read_text(&mut response, max_bytes, true).await
            },
        )
        .await
    }

    /// Context-only search of one dataset; never runs an LLM completion.
    pub async fn search(
        &self,
        query: &str,
        search_type: &str,
        dataset: &str,
        top_k: Option<u32>,
        signal: Option<&CancellationToken>,
    ) -> Result<Vec<SearchResult>, CogneeError> {
        if !BROWSE_SEARCH_TYPES.contains(&search_type) {
            return Err(CogneeError::new(
                CogneeErrorKind::InvalidRequest,
                "Unsupported Cognee browse search type",
            ));
        }
        let http = HttpRequest {
            method: Method::Post,
            url: build_url(&self.base_url, "/api/v1/search")?,
            headers: headers(self.api_key.as_deref(), Some("application/json")),
            body: Body::Json(
                json!({
                    "searchType": search_type,
                    "datasets": [dataset],
                    "query": query,
                    "topK": top_k.unwrap_or(10),
                    "onlyContext": true,
                })
                .to_string(),
            ),
        };
        let payload = request(
            self.transport.as_ref(),
            http,
            signal,
            SEARCH_TIMEOUT_MS,
            |mut response| async move {
                if is_uncognified_dataset(&mut response).await {
                    return Ok(json!([]));
                }
                assert_ok(&response, "search")?;
                read_json(&mut response, MAX_SEARCH_RESPONSE_BYTES).await
            },
        )
        .await?;
        Ok(records(&payload, "search")?
            .into_iter()
            .filter_map(|value| {
                let raw = match value.get("search_result") {
                    Some(Value::String(text)) => text.clone(),
                    None => String::new(),
                    Some(other) => serde_json::to_string_pretty(other).unwrap_or_default(),
                };
                let text = condense_graph_context(&raw).trim().to_string();
                if text.is_empty() {
                    return None;
                }
                Some(SearchResult {
                    dataset: optional_string(value.get("dataset_name"))
                        .unwrap_or_else(|| dataset.into()),
                    text,
                })
            })
            .collect())
    }

    /// The dataset's knowledge graph, trimmed for display.
    pub async fn dataset_graph(
        &self,
        dataset_id: &str,
        signal: Option<&CancellationToken>,
    ) -> Result<Graph, CogneeError> {
        let path = format!(
            "/api/v1/datasets/{}/graph",
            assert_uuid(dataset_id, "dataset")?
        );
        let http = self.get(build_url(&self.base_url, &path)?);
        let payload = request(
            self.transport.as_ref(),
            http,
            signal,
            SEARCH_TIMEOUT_MS,
            |mut response| async move {
                assert_ok(&response, "dataset graph")?;
                read_json(&mut response, MAX_GRAPH_RESPONSE_BYTES).await
            },
        )
        .await?;
        normalize_graph(&payload)
    }

    /// Agent sessions with session-cache memory, in server order.
    pub async fn list_sessions(
        &self,
        limit: Option<u32>,
        signal: Option<&CancellationToken>,
    ) -> Result<Vec<SessionSummary>, CogneeError> {
        let limit = limit.unwrap_or(500).to_string();
        let payload = self
            .get_json(
                "/api/v1/sessions",
                "sessions",
                MAX_SESSIONS_RESPONSE_BYTES,
                signal,
                &[("range", "all"), ("limit", &limit)],
            )
            .await?;
        let empty = json!([]);
        let sessions = match payload.get("sessions") {
            None | Some(Value::Null) => &empty,
            Some(sessions) => sessions,
        };
        Ok(records(sessions, "sessions")?
            .into_iter()
            .filter_map(session_summary)
            .collect())
    }

    /// A session's cached QA entries and trace steps.
    pub async fn get_session(
        &self,
        session_id: &str,
        signal: Option<&CancellationToken>,
    ) -> Result<SessionDetail, CogneeError> {
        if !regex!(r"^[A-Za-z0-9._:-]{1,200}$").is_match(session_id) {
            return Err(CogneeError::new(
                CogneeErrorKind::InvalidRequest,
                "Invalid Cognee session id",
            ));
        }
        // `encodeURIComponent` over the allowed set only changes the colon.
        let encoded = session_id.replace(':', "%3A");
        let payload = self
            .get_json(
                &format!("/api/v1/sessions/{encoded}"),
                "session",
                MAX_SESSION_RESPONSE_BYTES,
                signal,
                &[],
            )
            .await?;
        let value = payload.as_object().ok_or_else(|| malformed("session"))?;
        let summary = session_summary(value).unwrap_or_else(|| SessionSummary {
            session_id: session_id.into(),
            ..SessionSummary::default()
        });
        let entries: Vec<SessionEntry> = value
            .get("qas")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|qa| SessionEntry {
                        time: optional_string(qa.get("time")),
                        question: cap_text(qa.get("question"), MAX_SESSION_TEXT),
                        answer: cap_text(qa.get("answer"), MAX_SESSION_TEXT),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let traces = session_traces(value.get("traces"));
        Ok(SessionDetail {
            summary,
            label: optional_string(value.get("label")),
            entry_count: value
                .get("msg_count")
                .and_then(Value::as_f64)
                .unwrap_or(entries.len() as f64),
            trace_count: value
                .get("tool_calls")
                .and_then(Value::as_f64)
                .unwrap_or(traces.len() as f64),
            entries,
            traces,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::client::tests::{FakeResponse, FakeTransport};
    use super::*;

    const DATASET: &str = "0F8FAD5B-D9CB-469F-A165-70867728950E";

    fn browse(transport: Arc<FakeTransport>) -> BrowseClient {
        BrowseClient::new("http://cognee.test", Some("key".into()), transport)
    }

    #[tokio::test]
    async fn datasets_and_documents_project_known_fields() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!([{"id": "d1", "name": "ice", "createdAt": "t", "extra": 1}, {"id": "", "name": "x"}, null]),
        ));
        transport.push(FakeResponse::json(
            200,
            json!([{"id": "a", "name": "doc", "mimeType": "text/plain", "rawDataLocation": "/srv/x"}]),
        ));
        let client = browse(transport.clone());
        let datasets = client.list_datasets(None).await.unwrap();
        assert_eq!(datasets.len(), 1);
        assert_eq!(datasets[0].created_at.as_deref(), Some("t"));
        let items = client.list_dataset_data(DATASET, None).await.unwrap();
        assert_eq!(items[0].mime_type.as_deref(), Some("text/plain"));
        assert_eq!(
            transport.sent()[1].url,
            format!("http://cognee.test/api/v1/datasets/{DATASET}/data")
        );
        let error = client.list_dataset_data("../x", None).await.unwrap_err();
        assert_eq!(error.kind, CogneeErrorKind::InvalidRequest);
        assert_eq!(transport.sent().len(), 2);
    }

    #[tokio::test]
    async fn status_repeats_the_query_and_skips_empty_input() {
        let transport = Arc::new(FakeTransport::default());
        let client = browse(transport.clone());
        assert!(client.dataset_status(&[], None).await.unwrap().is_empty());
        assert!(transport.sent().is_empty());
        transport.push(FakeResponse::json(
            200,
            json!({DATASET: "DATASET_PROCESSING_COMPLETED", "x": 1}),
        ));
        let status = client
            .dataset_status(&[DATASET.into(), DATASET.into()], None)
            .await
            .unwrap();
        assert_eq!(
            status,
            vec![(
                DATASET.to_string(),
                "DATASET_PROCESSING_COMPLETED".to_string()
            )]
        );
        assert_eq!(
            transport.sent()[0].url,
            format!(
                "http://cognee.test/api/v1/datasets/status?dataset={DATASET}&dataset={DATASET}"
            )
        );
    }

    #[tokio::test]
    async fn raw_preview_truncates_and_search_is_context_only() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![("content-length".into(), "999999".into())],
            chunks: vec![b"hello world".to_vec()],
            delay_ms: 0,
        });
        transport.push(FakeResponse::json(
            200,
            json!([{"search_result": " found ", "dataset_name": "other"}, {"search_result": {"a": 1}}, {}]),
        ));
        let client = browse(transport.clone());
        assert_eq!(
            client
                .read_data_item(DATASET, DATASET, Some(5), None)
                .await
                .unwrap(),
            ("hello".to_string(), true)
        );
        let results = client
            .search("q", "CHUNKS", "ice", None, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0],
            SearchResult {
                dataset: "other".into(),
                text: "found".into()
            }
        );
        assert_eq!(results[1].text, "{\n  \"a\": 1\n}");
        let Body::Json(body) = &transport.sent()[1].body else {
            panic!()
        };
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap(),
            json!({"searchType":"CHUNKS","datasets":["ice"],"query":"q","topK":10,"onlyContext":true})
        );
    }

    #[tokio::test]
    async fn search_rejects_types_outside_the_context_only_allowlist() {
        let transport = Arc::new(FakeTransport::default());
        let client = browse(transport.clone());
        let error = client
            .search("q", "GRAPH_COMPLETION_COT", "ice", None, None)
            .await
            .unwrap_err();
        assert_eq!(error.kind, CogneeErrorKind::InvalidRequest);
        assert!(transport.sent().is_empty());
    }

    #[test]
    fn graph_projection_drops_dangling_edges_and_caps_by_degree() {
        let graph = normalize_graph(&json!({
            "nodes": [
                {"id": "a", "properties": {"name": "A", "description": "x".repeat(700), "belongs_to_set": ["agent_actions", 3], "file": "/srv"}},
                {"id": "b", "type": "Entity"},
                {"label": "no id"}
            ],
            "edges": [{"source": "a", "target": "b", "label": "rel"}, {"source": "a", "target": "zzz"}]
        }))
        .unwrap();
        assert_eq!(graph.total_nodes, 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.nodes[0].label, "A");
        assert_eq!(len16(graph.nodes[0].text.as_deref().unwrap()), 600);
        assert_eq!(graph.nodes[0].sets, vec!["agent_actions".to_string()]);
        assert_eq!(graph.nodes[1].r#type, "Entity");
        assert!(!graph.truncated);

        let mut nodes: Vec<Value> = (0..MAX_GRAPH_NODES + 2)
            .map(|index| json!({"id": format!("n{index}")}))
            .collect();
        nodes.push(json!({"id": "hub"}));
        let edges: Vec<Value> = (0..3)
            .map(|index| json!({"source": "hub", "target": format!("n{index}")}))
            .collect();
        let graph = normalize_graph(&json!({"nodes": nodes, "edges": edges})).unwrap();
        assert!(graph.truncated);
        assert_eq!(graph.nodes.len(), MAX_GRAPH_NODES);
        assert_eq!(graph.nodes[0].id, "hub");
        assert_eq!(graph.edges.len(), 3);
        assert!(normalize_graph(&json!("x")).is_err());
    }

    #[tokio::test]
    async fn sessions_list_and_detail_project_qa_and_traces() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"sessions": [{"session_id": "ice_1", "status": "running", "effective_status": "abandoned"}, {"x": 1}]}),
        ));
        transport.push(FakeResponse::json(
            200,
            json!({
                "label": "L",
                "qas": [{"time": "t", "question": "q", "answer": {"a": 1}}],
                "traces": [{"origin_function": "bash", "time": "t2", "status": "ok"}, {"method_name": "m"}],
                "msg_count": 7
            }),
        ));
        let client = browse(transport.clone());
        let sessions = client.list_sessions(None, None).await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status.as_deref(), Some("abandoned"));
        assert_eq!(
            transport.sent()[0].url,
            "http://cognee.test/api/v1/sessions?range=all&limit=500"
        );
        let detail = client.get_session("ice_1:x", None).await.unwrap();
        assert_eq!(detail.summary.session_id, "ice_1:x");
        assert_eq!(detail.entries[0].answer, "{\"a\":1}");
        assert_eq!(detail.traces[0].name, "bash");
        assert_eq!(detail.traces[0].text, "{\"status\":\"ok\"}");
        assert_eq!(detail.traces[1].text, "{\"method_name\":\"m\"}");
        assert_eq!(detail.entry_count, 7.0);
        assert_eq!(detail.trace_count, 2.0);
        assert_eq!(
            transport.sent()[1].url,
            "http://cognee.test/api/v1/sessions/ice_1%3Ax"
        );
        assert_eq!(
            client.get_session("bad/id", None).await.unwrap_err().kind,
            CogneeErrorKind::InvalidRequest
        );
    }
}
