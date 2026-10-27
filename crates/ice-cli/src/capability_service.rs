//! Session-local capability inventory and event hint service.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, ToolExecutionMode, ToolResultPartial};
use ice_ai::types::Tool;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::capability_catalog::{CapabilityCatalog, CapabilityDescriptor, CapabilityKind};
use crate::capability_suggest::{
    render_suggestion, suggest_capabilities, SuggestContext, Suggestion, TaskEvent,
};
use crate::skill_coverage::{skill_coverage, LoadedSkillRef, SkillContextState};

static NEXT_CORRELATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct PromptContext {
    pub custom_type: &'static str,
    pub correlation_id: String,
    pub text: String,
    task_id: String,
    branch_generation: u64,
    registry_revision: u64,
    suggested_ids: Vec<String>,
}

#[derive(Default)]
struct State {
    catalog: CapabilityCatalog,
    registry_revision: u64,
    coverage_revision: u64,
    coverage: HashMap<String, SkillContextState>,
    delivered: HashMap<(String, u64), HashSet<String>>,
    active_owner: HashMap<String, (String, u64)>,
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
            state.delivered.clear();
        }
    }
    pub fn snapshot(&self) -> CapabilityCatalog {
        self.state.lock().unwrap().catalog.clone()
    }
    pub fn registry_revision(&self) -> u64 {
        self.state.lock().unwrap().registry_revision
    }
    pub fn search(&self, query: &str, limit: usize, offset: usize) -> Value {
        let state = self.state.lock().unwrap();
        let inventory = query.trim().is_empty() || is_inventory(query);
        let entries: Vec<_> = if inventory {
            state.catalog.entries.iter().collect()
        } else {
            // Keep a bounded but useful match set and paginate that same set;
            // never report a total larger than the results subsequent pages can reach.
            state.catalog.shortlist(query, 64)
        };
        let total = entries.len();
        let page: Vec<_> = entries
            .into_iter()
            .skip(offset.min(64))
            .take(limit.clamp(1, 12))
            .map(|entry| {
                let coverage = state
                    .coverage
                    .get(&entry.id)
                    .copied()
                    .unwrap_or(SkillContextState::Advertised);
                descriptor_json(entry, coverage)
            })
            .collect();
        json!({"items": page, "offset": offset.min(64), "limit": limit.clamp(1,12), "total": total, "truncated": offset.saturating_add(limit.clamp(1,12)) < total})
    }
    pub fn update_coverage(&self, messages: &[ice_agent::AgentMessage], revision: u64) {
        let mut state = self.state.lock().unwrap();
        if state.coverage_revision == revision {
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
    }
    pub fn prepare_hint(
        &self,
        session_id: &str,
        task_id: &str,
        branch_generation: u64,
        query: &str,
        messages: &[ice_agent::AgentMessage],
        task_event: TaskEvent,
        max_chars: usize,
    ) -> Option<PromptContext> {
        if query.len() > 2048 || max_chars < 16 || task_event == TaskEvent::InventoryQuestion {
            return None;
        }
        let (catalog, revision, coverage, delivered) = {
            let mut state = self.state.lock().unwrap();
            let coverage_revision = branch_generation;
            if state.coverage_revision != coverage_revision {
                let skills: Vec<_> = state
                    .catalog
                    .entries
                    .iter()
                    .filter(|e| e.kind == CapabilityKind::Skill)
                    .map(|e| LoadedSkillRef {
                        id: e.id.clone(),
                        name: e.name.clone(),
                        file_path: skill_path(e),
                        content_hash: e.content_hash.clone(),
                    })
                    .collect();
                state.coverage = skill_coverage(messages, &skills);
                state.coverage_revision = coverage_revision;
            }
            let owner = format!("{task_id}:{branch_generation}");
            if !state.active_owner.contains_key(session_id) && state.active_owner.len() >= 128 {
                if let Some(old) = state.active_owner.keys().next().cloned() {
                    state.active_owner.remove(&old);
                }
            }
            state
                .active_owner
                .insert(session_id.to_owned(), (owner.clone(), branch_generation));
            let delivered = state
                .delivered
                .get(&(format!("{session_id}:{owner}"), state.registry_revision))
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
        let text = bounded(
            render_suggestion(&suggestion).unwrap_or_default(),
            max_chars,
        );
        if text.trim().is_empty() {
            return None;
        }
        let correlation_id = format!(
            "cap-{:016x}",
            NEXT_CORRELATION_ID.fetch_add(1, Ordering::Relaxed)
        );
        Some(PromptContext {
            custom_type: "ice-capability-hint",
            correlation_id,
            text,
            task_id: format!("{session_id}:{task_id}"),
            branch_generation,
            registry_revision: revision,
            suggested_ids: ids,
        })
    }
    pub fn acknowledge_hint(&self, context: &PromptContext, persisted: bool) {
        if !persisted {
            return;
        }
        let mut state = self.state.lock().unwrap();
        if context.registry_revision != state.registry_revision {
            return;
        }
        // The current owner includes branch generation; acknowledgments from a
        // superseded task/compaction must not suppress a new hint.
        let Some((active_task, active_generation)) = state
            .active_owner
            .get(context.task_id.split(':').next().unwrap_or_default())
        else {
            return;
        };
        let expected_task = context.task_id.rsplit(':').next().unwrap_or_default();
        if active_task != &format!("{expected_task}:{}", context.branch_generation)
            || *active_generation != context.branch_generation
        {
            return;
        }
        let owner = format!("{}:{}", context.task_id, context.branch_generation);
        let key = (owner, context.registry_revision);
        if !state.delivered.contains_key(&key) && state.delivered.len() >= 128 {
            if let Some(old) = state.delivered.keys().next().cloned() {
                state.delivered.remove(&old);
            }
        }
        let entries = state.delivered.entry(key).or_default();
        for id in &context.suggested_ids {
            entries.insert(id.clone());
        }
    }
    pub fn tool(self: &Arc<Self>) -> ice_harness::types::HarnessTool {
        ice_harness::types::HarnessTool::new(Arc::new(CapabilitySearchTool {
            service: Arc::downgrade(self),
            schema: tool_schema(),
        }))
    }
    pub fn doctor_cognee_contract(&self) -> Value {
        let catalog = self.snapshot();
        let mut tools: Vec<_> = catalog
            .entries
            .iter()
            .filter(|e| {
                e.kind != CapabilityKind::Skill
                    && (e.name.contains("cognee") || e.source.contains("cognee"))
            })
            .map(|e| {
                json!({"name":e.name,"id":e.id,"schema_fingerprint":e.content_hash,"status":if e.available {"last_known_available_unprobed"} else {"offline_or_disabled"},"methods":[]})
            })
            .collect();
        // Native Cognee search is registered by the built-in extension rather
        // than MCP. Report its identity locally, never as probed.
        if !tools
            .iter()
            .any(|tool| tool["name"] == crate::cognee::runtime::SEARCH_TOOL_NAME)
        {
            tools.push(json!({
                "name": crate::cognee::runtime::SEARCH_TOOL_NAME,
                "id": format!("native:{}", crate::cognee::runtime::SEARCH_TOOL_NAME),
                "schema_fingerprint": null,
                "status": "unprobed",
                "methods": []
            }));
        }
        json!({"tools":tools,"probed":false})
    }
}
fn skill_path(entry: &CapabilityDescriptor) -> String {
    entry.source.clone()
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
            .any(|component| component == std::path::Component::ParentDir)
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
fn bounded(s: String, max: usize) -> String {
    s.chars().take(max).collect()
}
fn tool_schema() -> Tool {
    Tool { name:"capability_search".into(), description:"Search the current local skill/tool catalog. This read-only tool never invokes a capability.".into(), parameters:ice_ai::types::Schema::new(json!({"type":"object","properties":{"query":{"type":"string","maxLength":2048},"limit":{"type":"integer","minimum":1,"maximum":12},"offset":{"type":"integer","minimum":0,"maximum":64}},"additionalProperties":false})), constrained_sampling:None }
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
        "Capability Search"
    }
    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Parallel
    }
    async fn execute(
        &self,
        _: &str,
        params: Value,
        _: CancellationToken,
        _: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let query = params.get("query").and_then(Value::as_str).unwrap_or("");
        if query.len() > 2048 {
            return Err(AgentError::Validation("query exceeds 2048 bytes".into()));
        }
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| AgentError::Tool("capability catalog unavailable".into()))?;
        let output = service.search(query, limit, offset);
        Ok(AgentToolResult::text(output.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_catalog::CapabilityCatalog;
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
    fn refresh_hashes_change_and_failed_or_stale_ack_does_not_suppress() {
        let service = CapabilityService::new(catalog(&["alpha"]));
        let messages = Vec::new();
        let hint =
            service.prepare_hint("s", "t1", 1, "alpha", &messages, TaskEvent::TaskStart, 280);
        let hint = hint.expect("first event should suggest local skill");
        service.acknowledge_hint(&hint, false);
        assert!(service
            .prepare_hint("s", "t1", 1, "alpha", &messages, TaskEvent::TaskStart, 280)
            .is_some());

        let stale = service
            .prepare_hint("s", "t1", 1, "alpha", &messages, TaskEvent::TaskStart, 280)
            .unwrap();
        let _current =
            service.prepare_hint("s", "t2", 2, "alpha", &messages, TaskEvent::TaskStart, 280);
        service.acknowledge_hint(&stale, true);
        assert!(service
            .prepare_hint("s", "t2", 2, "alpha", &messages, TaskEvent::TaskStart, 280)
            .is_some());

        let old_hash = service.snapshot().entries[0].content_hash.clone();
        service.refresh(catalog(&["alpha-updated"]));
        assert_ne!(old_hash, service.snapshot().entries[0].content_hash);
    }
}
