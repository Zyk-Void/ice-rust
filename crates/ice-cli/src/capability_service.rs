//! Session-local capability inventory and event hint service.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, ToolResultPartial};
use ice_ai::types::Tool;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::capability_catalog::{CapabilityCatalog, CapabilityDescriptor, CapabilityKind};
use crate::capability_suggest::{
    render_suggestion, suggest_capabilities, SuggestContext, Suggestion, TaskEvent as HintEvent,
};
use crate::cognee::task::{TaskCursor, TaskEvent, TaskScope};
use crate::skill_coverage::{skill_coverage, LoadedSkillRef, SkillContextState};

static NEXT_CORRELATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct PromptContext {
    pub custom_type: &'static str,
    pub correlation_id: String,
    pub text: String,
    session_id: String,
    task_id: String,
    branch_generation: u64,
    registry_revision: u64,
    suggested_ids: Vec<String>,
}

#[derive(Clone)]
struct ActiveRequest {
    run_id: String,
    branch_generation: u64,
    signal: CancellationToken,
}

#[derive(Default)]
struct State {
    catalog: CapabilityCatalog,
    registry_revision: u64,
    coverage_revision: u64,
    coverage_fingerprint: String,
    coverage: HashMap<String, SkillContextState>,
    delivered: HashMap<(String, u64), HashSet<String>>,
    active_owner: HashMap<String, (String, u64)>,
    cursors: HashMap<(String, String), TaskCursor>,
    active_requests: HashMap<(String, String), ActiveRequest>,
    pending_misses: HashMap<(String, String, String, u64), String>,
    pending: HashMap<String, PromptContext>,
}

/// Bounded immutable-at-read catalog with task-event-local hint bookkeeping.
#[derive(Default)]
pub struct CapabilityService {
    state: Mutex<State>,
}
impl CapabilityService {
    pub fn new(catalog: CapabilityCatalog) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                catalog,
                registry_revision: 1,
                ..State::default()
            }),
        })
    }
    pub fn refresh(&self, catalog: CapabilityCatalog) {
        let mut state = self.state.lock().unwrap();
        if state.catalog != catalog {
            state.catalog = catalog;
            state.registry_revision = state.registry_revision.wrapping_add(1);
            state.coverage.clear();
            state.coverage_fingerprint.clear();
            state.delivered.clear();
            state.pending.clear();
        }
    }
    pub fn snapshot(&self) -> CapabilityCatalog {
        self.state.lock().unwrap().catalog.clone()
    }
    pub fn registry_revision(&self) -> u64 {
        self.state.lock().unwrap().registry_revision
    }

    pub fn search(&self, query: &str, limit: usize, offset: usize) -> Value {
        let limit = limit.clamp(1, 12);
        if query.len() > 2048 {
            return json!({"error":"query exceeds 2048 bytes","items":[],"offset":offset,"limit":limit,"total":0,"truncated":false,"next_offset":null});
        }
        let state = self.state.lock().unwrap();
        let inventory = query.trim().is_empty() || is_inventory(query);
        let entries: Vec<_> = if inventory {
            let skills_only =
                query.to_lowercase().contains("skill") && !query.to_lowercase().contains("tool");
            let tools_only =
                query.to_lowercase().contains("tool") && !query.to_lowercase().contains("skill");
            state
                .catalog
                .entries
                .iter()
                .filter(|entry| {
                    entry.available
                        && entry.model_invocable
                        && (!skills_only || entry.kind == CapabilityKind::Skill)
                        && (!tools_only || entry.kind != CapabilityKind::Skill)
                })
                .collect()
        } else {
            state.catalog.shortlist(query, 8)
        };
        let total = entries.len();
        let catalog_truncated = state.catalog.entries.len() >= CapabilityCatalog::MAX_ENTRIES;
        let offset = offset.min(total);
        let page: Vec<_> = entries
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|entry| {
                let coverage = state
                    .coverage
                    .get(&entry.id)
                    .copied()
                    .unwrap_or(SkillContextState::Advertised);
                descriptor_json(entry, coverage)
            })
            .collect();
        let next = offset.saturating_add(limit);
        let truncated = next < total;
        json!({"items":page,"offset":offset,"limit":limit,"total":total,"truncated":truncated,"catalog_truncated":catalog_truncated,"next_offset":if truncated {Some(next)} else {None}})
    }

    fn update_coverage_locked(
        state: &mut State,
        messages: &[ice_agent::AgentMessage],
        revision: u64,
    ) {
        let mut hasher = Sha256::new();
        hasher.update(revision.to_be_bytes());
        for message in messages {
            hasher.update(format!("{message:?}").as_bytes());
        }
        let fingerprint = format!("{:x}", hasher.finalize());
        if state.coverage_fingerprint == fingerprint {
            return;
        }
        let skills: Vec<_> = state
            .catalog
            .entries
            .iter()
            .filter(|e| e.kind == CapabilityKind::Skill)
            .map(|e| LoadedSkillRef {
                id: e.id.clone(),
                name: e.name.clone(),
                file_path: e.source.clone(),
                content_hash: e.content_hash.clone(),
            })
            .collect();
        state.coverage = skill_coverage(messages, &skills);
        state.coverage_revision = revision;
        state.coverage_fingerprint = fingerprint;
    }
    pub fn update_coverage(&self, messages: &[ice_agent::AgentMessage], revision: u64) {
        Self::update_coverage_locked(&mut self.state.lock().unwrap(), messages, revision);
    }

    pub fn prepare_hint(
        &self,
        session_id: &str,
        task_id: &str,
        branch_generation: u64,
        query: &str,
        messages: &[ice_agent::AgentMessage],
        task_event: HintEvent,
        max_chars: usize,
    ) -> Option<PromptContext> {
        if query.len() > 2048 || max_chars < 16 || task_event == HintEvent::InventoryQuestion {
            return None;
        }
        let (catalog, revision, coverage, delivered) = {
            let mut state = self.state.lock().unwrap();
            Self::update_coverage_locked(&mut state, messages, branch_generation);
            if !state.active_owner.contains_key(session_id) && state.active_owner.len() >= 128 {
                if let Some(old) = state.active_owner.keys().min().cloned() {
                    state.active_owner.remove(&old);
                }
            }
            state.active_owner.insert(
                session_id.to_owned(),
                (task_id.to_owned(), branch_generation),
            );
            let owner = format!("{session_id}\0{task_id}\0{branch_generation}");
            let delivered = state
                .delivered
                .get(&(owner, state.registry_revision))
                .cloned()
                .unwrap_or_default();
            (
                state.catalog.clone(),
                state.registry_revision,
                state.coverage.clone(),
                delivered,
            )
        };
        let suggestion = suggest_capabilities(
            &catalog,
            &SuggestContext {
                query,
                coverage: &coverage,
                already_suggested: &delivered,
                task_event,
            },
        );
        let ids = match &suggestion {
            Suggestion::Skill { id, .. } => vec![id.clone()],
            Suggestion::Tools { handles, .. } => handles.iter().map(|h| h.id.clone()).collect(),
            Suggestion::Ambiguous { candidates } => candidates.clone(),
            _ => return None,
        };
        let text = render_suggestion(&suggestion)?;
        if text.chars().count() > max_chars {
            return None;
        }
        let correlation_id = format!(
            "cap-{:016x}",
            NEXT_CORRELATION_ID.fetch_add(1, Ordering::Relaxed)
        );
        let packet = PromptContext {
            custom_type: "ice-capability-hint",
            correlation_id: correlation_id.clone(),
            text,
            session_id: session_id.to_owned(),
            task_id: task_id.to_owned(),
            branch_generation,
            registry_revision: revision,
            suggested_ids: ids,
        };
        let mut state = self.state.lock().unwrap();
        if state.pending.len() >= 128 {
            if let Some(key) = state.pending.keys().min().cloned() {
                state.pending.remove(&key);
            }
        }
        state.pending.insert(correlation_id, packet.clone());
        Some(packet)
    }
    pub fn acknowledge_hint(&self, context: &PromptContext, persisted: bool) {
        if !persisted {
            return;
        }
        let mut state = self.state.lock().unwrap();
        if context.registry_revision != state.registry_revision {
            return;
        }
        let Some((task, generation)) = state.active_owner.get(&context.session_id) else {
            return;
        };
        if task != &context.task_id || *generation != context.branch_generation {
            return;
        }
        let owner = format!(
            "{}\0{}\0{}",
            context.session_id, context.task_id, context.branch_generation
        );
        let key = (owner, context.registry_revision);
        if !state.delivered.contains_key(&key) && state.delivered.len() >= 128 {
            if let Some(old) = state.delivered.keys().min().cloned() {
                state.delivered.remove(&old);
            }
        }
        let entries = state.delivered.entry(key).or_default();
        for id in &context.suggested_ids {
            if entries.len() >= 64 && !entries.contains(id) {
                if let Some(old) = entries.iter().min().cloned() {
                    entries.remove(&old);
                }
            }
            entries.insert(id.clone());
        }
    }
    fn observe_event(
        &self,
        session: &str,
        lane: &str,
        branch_generation: u64,
        original_prompt: Option<&str>,
        compaction: Option<i64>,
        resumed: bool,
        capability_miss: bool,
    ) -> (String, Option<TaskEvent>) {
        let mut state = self.state.lock().unwrap();
        let key = (session.to_owned(), lane.to_owned());
        if !state.cursors.contains_key(&key) && state.cursors.len() >= 256 {
            if let Some(oldest) = state.cursors.keys().min().cloned() {
                state.cursors.remove(&oldest);
            }
        }
        let cursor = state.cursors.entry(key).or_default();
        cursor.observe(
            TaskScope {
                session_id: session.to_owned(),
                lane_id: lane.to_owned(),
                branch_generation,
                repository_key: String::new(),
            },
            original_prompt,
            compaction,
            resumed,
        );
        if capability_miss {
            let query = original_prompt.unwrap_or_default();
            cursor.signal(TaskEvent::CapabilityMiss, query);
        }
        let event = cursor.take_event();
        (cursor.id(), event)
    }

    fn record_capability_miss(
        &self,
        session: &str,
        lane: &str,
        run_id: &str,
        branch_generation: u64,
        query: &str,
        signal: &CancellationToken,
    ) -> bool {
        if signal.is_cancelled()
            || query.trim().is_empty()
            || query.len() > 2048
            || !crate::cognee::delivery::has_query_anchor(query)
        {
            return false;
        }
        let mut state = self.state.lock().unwrap();
        let key = (session.to_owned(), lane.to_owned());
        let Some(active) = state.active_requests.get(&key) else {
            return false;
        };
        if active.run_id != run_id
            || active.branch_generation != branch_generation
            || active.signal.is_cancelled()
            || signal.is_cancelled()
        {
            return false;
        }
        if state.pending_misses.len() >= 128 {
            if let Some(oldest) = state.pending_misses.keys().min().cloned() {
                state.pending_misses.remove(&oldest);
            }
        }
        state.pending_misses.insert(
            (
                session.to_owned(),
                lane.to_owned(),
                run_id.to_owned(),
                branch_generation,
            ),
            query.to_owned(),
        );
        true
    }

    pub fn tool(self: &Arc<Self>) -> ice_harness::types::HarnessTool {
        ice_harness::types::HarnessTool::new(Arc::new(CapabilitySearchTool {
            service: Arc::downgrade(self),
            schema: tool_schema(),
        }))
    }
    pub fn doctor_cognee_contract(&self) -> Value {
        let catalog = self.snapshot();
        let tools: Vec<_> = catalog.entries.iter().filter(|e| e.kind != CapabilityKind::Skill && (e.name.contains("cognee") || e.source.contains("cognee"))).map(|e| {
            json!({"name":e.name,"id":e.id,"schema_fingerprint":e.content_hash,"status":if e.available {"last_known_available_unprobed"} else {"offline_or_disabled"},"methods":[]})
        }).collect();
        json!({"tools":tools,"probed":false})
    }
}

#[async_trait]
impl ice_harness::extension::HarnessExtension for CapabilityService {
    async fn request_context(
        &self,
        event: &ice_harness::extension::RequestContextEvent<'_>,
    ) -> Option<ice_harness::extension::PromptContext> {
        if event.signal.is_cancelled() || event.remaining_context_tokens < 48 {
            return None;
        }
        let original_prompt = event.original_prompt.unwrap_or(event.prompt);
        {
            let mut state = self.state.lock().unwrap();
            let key = (event.session_id.to_owned(), event.lane_id.to_owned());
            if !state.active_requests.contains_key(&key) && state.active_requests.len() >= 256 {
                if let Some(oldest) = state.active_requests.keys().min().cloned() {
                    state.active_requests.remove(&oldest);
                }
            }
            state.active_requests.insert(
                key,
                ActiveRequest {
                    run_id: event.run_id.to_owned(),
                    branch_generation: event.branch_generation,
                    signal: event.signal.clone(),
                },
            );
        }
        let (task_id, mut task_event) = self.observe_event(
            event.session_id,
            event.lane_id,
            event.branch_generation,
            Some(original_prompt),
            event.latest_compaction_at,
            false,
            false,
        );
        let pending_miss = self.state.lock().unwrap().pending_misses.remove(&(
            event.session_id.to_owned(),
            event.lane_id.to_owned(),
            event.run_id.to_owned(),
            event.branch_generation,
        ));
        let hint_query = pending_miss.as_deref().unwrap_or(original_prompt);
        if let Some(query) = pending_miss.as_deref() {
            if let Some(cursor) = self
                .state
                .lock()
                .unwrap()
                .cursors
                .get_mut(&(event.session_id.to_owned(), event.lane_id.to_owned()))
            {
                if cursor.signal(TaskEvent::CapabilityMiss, &query) {
                    task_event = cursor.take_event().or(task_event);
                }
            }
        }
        let Some(task_event) = task_event else {
            return None;
        };
        let hint_event = match task_event {
            TaskEvent::Started | TaskEvent::Resumed | TaskEvent::Changed | TaskEvent::Compacted => {
                HintEvent::TaskStart
            }
            TaskEvent::CapabilityMiss | TaskEvent::HistoryGap => HintEvent::CapabilityMiss,
        };
        let chars = event.remaining_context_tokens.saturating_mul(4).min(280);
        let packet = self.prepare_hint(
            event.session_id,
            &task_id,
            event.branch_generation,
            hint_query,
            event.messages,
            hint_event,
            chars,
        )?;
        Some(ice_harness::extension::PromptContext {
            custom_type: packet.custom_type.into(),
            text: packet.text,
            correlation_id: Some(packet.correlation_id),
        })
    }
    async fn prompt_context_persisted(
        &self,
        context: &ice_harness::extension::PromptContext,
        persisted: bool,
    ) {
        if context.custom_type != "ice-capability-hint" {
            return;
        }
        let mut state = self.state.lock().unwrap();
        let Some(packet) = state
            .pending
            .remove(context.correlation_id.as_deref().unwrap_or_default())
        else {
            return;
        };
        drop(state);
        self.acknowledge_hint(&packet, persisted);
    }
}

fn descriptor_json(e: &CapabilityDescriptor, coverage: SkillContextState) -> Value {
    json!({"id":e.id,"kind":format!("{:?}",e.kind),"name":e.name,"description":e.description,"source":e.source,"content_hash":e.content_hash,"configured":true,"enabled":e.model_invocable,"readiness":if e.available {"last_known_available_unprobed"} else {"unavailable_or_shadowed"},"available":e.available,"coverage":format!("{:?}",coverage),"load_hint":if e.kind==CapabilityKind::Skill && valid_skill_path(&e.source) {Some(format!("Read the validated local skill resource at {}", e.source))} else {None}})
}
fn valid_skill_path(path: &str) -> bool {
    let path = std::path::Path::new(path);
    path.is_absolute()
        && path.file_name().is_some_and(|name| name == "SKILL.md")
        && !path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
}
fn is_inventory(q: &str) -> bool {
    let q = q.to_lowercase();
    [
        "list skills",
        "list tools",
        "inventory",
        "what skills",
        "what tools",
        "capabilities",
    ]
    .iter()
    .any(|x| q.contains(x))
}
fn tool_schema() -> Tool {
    Tool { name:"capability_search".into(), description:"Search the current local skill/tool catalog. This read-only tool never invokes a capability.".into(), parameters:ice_ai::types::Schema::new(json!({"type":"object","properties":{"query":{"type":"string","maxLength":2048},"limit":{"type":"integer","minimum":1,"maximum":12},"offset":{"type":"integer","minimum":0},"context_event":{"type":"string","enum":["capability_miss"]}},"additionalProperties":false})), constrained_sampling:None }
}
struct CapabilitySearchTool {
    service: std::sync::Weak<CapabilityService>,
    schema: Tool,
}
#[async_trait]
impl AgentTool for CapabilitySearchTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "Capability search"
    }
    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        signal: CancellationToken,
        _: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let query = params.get("query").and_then(Value::as_str).unwrap_or("");
        if query.len() > 2048 {
            return Err(AgentError::Validation("query exceeds 2048 bytes".into()));
        }
        if signal.is_cancelled() {
            return Err(AgentError::Tool("capability search cancelled".into()));
        }
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let context_event = params.get("context_event").and_then(Value::as_str);
        if context_event.is_some_and(|value| value != "capability_miss") {
            return Err(AgentError::Validation("unsupported context_event".into()));
        }
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| AgentError::Tool("capability catalog unavailable".into()))?;
        if signal.is_cancelled() {
            return Err(AgentError::Tool("capability search cancelled".into()));
        }
        let mut result = service.search(query, limit, offset);
        if context_event == Some("capability_miss") {
            let scope = {
                let state = service.state.lock().unwrap();
                let mut live = state
                    .active_requests
                    .iter()
                    .filter(|(_, active)| !active.signal.is_cancelled() && !signal.is_cancelled());
                let first = live.next();
                if live.next().is_none() {
                    first.map(|((session, lane), active)| {
                        (
                            session.clone(),
                            lane.clone(),
                            active.run_id.clone(),
                            active.branch_generation,
                        )
                    })
                } else {
                    None
                }
            };
            if let Some((session, lane, run, generation)) = scope {
                let recorded = service
                    .record_capability_miss(&session, &lane, &run, generation, query, &signal);
                result["context_event"] = json!({
                    "type":"capability_miss",
                    "outcome": if recorded { "event_recorded" } else { "not_recorded" }
                });
            } else {
                result["context_event"] =
                    json!({"type":"capability_miss","outcome":"not_recorded"});
            }
        }
        Ok(AgentToolResult::text(result.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_catalog::CapabilityCatalog;
    use ice_ai::types::{Api, AssistantMessage, Content, ToolResultMessage, ToolResultRole};
    use ice_harness::extension::{HarnessExtension, RequestContextEvent};
    use ice_harness::types::Skill;

    fn catalog(names: &[&str]) -> CapabilityCatalog {
        let skills: Vec<_> = names
            .iter()
            .map(|name| {
                (
                    "project",
                    Skill {
                        name: (*name).into(),
                        description: format!("{name} feature"),
                        content: format!("instructions for {name}"),
                        file_path: format!("/tmp/{name}/SKILL.md"),
                        disable_model_invocation: None,
                    },
                )
            })
            .collect();
        CapabilityCatalog::build(&skills, &[])
    }
    #[test]
    fn search_pages_inventory_without_claiming_unreachable_results() {
        let service = CapabilityService::new(catalog(&[
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
        ]));
        let first = service.search("", 4, 0);
        let second = service.search("", 4, 4);
        assert_eq!(first["total"], 8);
        assert_eq!(first["items"].as_array().unwrap().len(), 4);
        assert_eq!(second["items"].as_array().unwrap().len(), 4);
        assert_eq!(first["truncated"], true);
        assert_eq!(second["truncated"], false);
        assert!(first["items"][0]["load_hint"]
            .as_str()
            .unwrap()
            .contains("SKILL.md"));
    }
    #[tokio::test]
    async fn registered_tool_reads_the_refreshed_service_snapshot() {
        let service = CapabilityService::new(catalog(&["alpha"]));
        let tool = service.tool();
        let before = service.registry_revision();
        service.refresh(catalog(&["bravo"]));
        assert!(service.registry_revision() > before);
        let result = tool
            .tool
            .execute(
                "call-1",
                json!({"query":"bravo"}),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap();
        let text = result
            .content
            .iter()
            .filter_map(|block| match block {
                ice_agent::types::TextContentOrImage::Text(text) => Some(text.text.as_str()),
                ice_agent::types::TextContentOrImage::Image(_) => None,
            })
            .collect::<String>();
        assert!(text.contains("bravo"));
        assert!(!text.contains("alpha"));
    }
    #[test]
    fn inventory_pages_beyond_64_and_filters_disabled_and_scopes_kind() {
        let names: Vec<String> = (0..90).map(|i| format!("skill{i:02}")).collect();
        let refs: Vec<_> = names.iter().map(String::as_str).collect();
        let service = CapabilityService::new(catalog(&refs));
        let page = service.search("list skills", 12, 72);
        assert_eq!(page["offset"], 72);
        assert_eq!(page["items"].as_array().unwrap().len(), 12);
        assert_eq!(page["total"], 90);
        assert_eq!(page["next_offset"], 84);
        assert!(page["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["kind"] == "Skill"));
    }

    fn request_event<'a>(
        prompt: &'a str,
        messages: &'a [ice_agent::AgentMessage],
        compaction: Option<i64>,
        signal: CancellationToken,
    ) -> RequestContextEvent<'a> {
        RequestContextEvent {
            prompt,
            original_prompt: Some(prompt),
            messages,
            system_prompt: "system",
            session_id: "session",
            run_id: "run",
            lane_id: "main",
            branch_generation: 0,
            request_index: 0,
            context_window: 128_000,
            remaining_context_tokens: 800,
            latest_compaction_at: compaction,
            signal,
        }
    }

    #[tokio::test]
    async fn request_hook_releases_loaded_skill_after_compaction_same_generation() {
        let service = CapabilityService::new(catalog(&["alpha"]));
        let skill = &service.snapshot().entries[0];
        let mut assistant = AssistantMessage::empty(Api::OpenaiCompletions, "test", "test", 1);
        assistant.content.push(Content::tool_call(
            "read-call",
            "read",
            json!({"path": skill.source}),
        ));
        let body = "instructions for alpha";
        let messages = vec![
            ice_agent::AgentMessage::Assistant(Box::new(assistant)),
            ice_agent::AgentMessage::ToolResult(Box::new(ToolResultMessage {
                role: ToolResultRole,
                tool_call_id: "read-call".into(),
                tool_name: "read".into(),
                content: vec![Content::text(body)],
                details: None,
                usage: None,
                added_tool_names: Vec::new(),
                is_error: false,
                timestamp: 2,
            })),
        ];
        assert!(
            service
                .request_context(&request_event(
                    "alpha feature",
                    &messages,
                    None,
                    CancellationToken::new(),
                ))
                .await
                .is_none(),
            "full body in branch suppresses suggestion"
        );
        assert_eq!(service.search("", 1, 0)["items"][0]["coverage"], "Loaded");
        let compacted = Vec::new();
        let hint = service
            .request_context(&request_event(
                "alpha feature",
                &compacted,
                Some(42),
                CancellationToken::new(),
            ))
            .await
            .expect("removed loaded skill is eligible after compaction");
        assert_eq!(hint.custom_type, "ice-capability-hint");
        assert_eq!(
            service.search("", 1, 0)["items"][0]["coverage"],
            "Advertised"
        );
    }

    #[tokio::test]
    async fn capability_miss_requires_live_scope_honors_cancellation_and_consumes_at_next_boundary()
    {
        let service = CapabilityService::new(catalog(&["alpha"]));
        let tool = service.tool();
        let call = |signal| {
            tool.tool.execute(
                "call",
                json!({"query":"Need alpha project decision", "context_event":"capability_miss"}),
                signal,
                Arc::new(|_| {}),
            )
        };
        let unscoped = call(CancellationToken::new()).await.unwrap();
        let unscoped_text = unscoped
            .content
            .iter()
            .filter_map(|c| match c {
                ice_agent::types::TextContentOrImage::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(
            serde_json::from_str::<Value>(&unscoped_text).unwrap()["context_event"]["outcome"],
            "not_recorded"
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(call(cancelled).await.is_err());

        let messages = Vec::new();
        service
            .request_context(&request_event(
                "Need alpha project decision",
                &messages,
                None,
                CancellationToken::new(),
            ))
            .await;
        let response = call(CancellationToken::new()).await.unwrap();
        let response_text = response
            .content
            .iter()
            .filter_map(|c| match c {
                ice_agent::types::TextContentOrImage::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(
            serde_json::from_str::<Value>(&response_text).unwrap()["context_event"]["outcome"],
            "event_recorded"
        );
        assert!(
            service
                .request_context(&request_event(
                    "alpha feature",
                    &messages,
                    None,
                    CancellationToken::new(),
                ))
                .await
                .is_some(),
            "queued miss consumed at the next existing request boundary"
        );
        for _ in 0..10 {
            assert!(
                service
                    .request_context(&request_event(
                        "Need alpha project decision",
                        &messages,
                        None,
                        CancellationToken::new(),
                    ))
                    .await
                    .is_none(),
                "follow-up request must not repeat the suggestion"
            );
        }
    }

    #[test]
    fn first_generation_zero_and_same_generation_context_changes_refresh_coverage() {
        let service = CapabilityService::new(catalog(&["alpha"]));
        let empty = Vec::new();
        assert_eq!(
            service.search("", 1, 0)["items"][0]["coverage"],
            "Advertised"
        );
        service.update_coverage(&empty, 0);
        assert_eq!(
            service.search("", 1, 0)["items"][0]["coverage"],
            "Advertised"
        );
        let changed = empty.clone();
        service.update_coverage(&changed, 0);
        assert_eq!(
            service.search("", 1, 0)["items"][0]["coverage"],
            "Advertised"
        );
    }

    #[test]
    fn tiny_hint_budget_omits_whole_packet() {
        let service = CapabilityService::new(catalog(&["alpha"]));
        assert!(service
            .prepare_hint("s", "t", 0, "alpha", &[], HintEvent::TaskStart, 16)
            .is_none());
        assert!(service
            .prepare_hint("s", "t", 0, "alpha", &[], HintEvent::TaskStart, 280)
            .is_some());
    }

    #[test]
    fn refresh_hashes_change_and_failed_or_stale_ack_does_not_suppress() {
        let service = CapabilityService::new(catalog(&["alpha"]));
        let messages = Vec::new();
        let hint = service
            .prepare_hint("s", "t1", 1, "alpha", &messages, HintEvent::TaskStart, 280)
            .expect("local skill hint");
        service.acknowledge_hint(&hint, false);
        assert!(service
            .prepare_hint("s", "t1", 1, "alpha", &messages, HintEvent::TaskStart, 280)
            .is_some());
        let stale = service
            .prepare_hint("s", "t1", 1, "alpha", &messages, HintEvent::TaskStart, 280)
            .unwrap();
        let _current =
            service.prepare_hint("s", "t2", 2, "alpha", &messages, HintEvent::TaskStart, 280);
        service.acknowledge_hint(&stale, true);
        assert!(service
            .prepare_hint("s", "t2", 2, "alpha", &messages, HintEvent::TaskStart, 280)
            .is_some());
        let old_hash = service.snapshot().entries[0].content_hash.clone();
        service.refresh(catalog(&["alpha-updated"]));
        assert_ne!(old_hash, service.snapshot().entries[0].content_hash);
    }
}
