//! Pure, bounded capability hints based on the current catalog.
//!
//! Suggestions are advisory only: this module never auto-executes a capability,
//! makes a model call, reads skill bodies, or consults memory. Ambiguous-set JEV
//! ranking is future work; callers can decide whether and how to provide it.

use std::collections::{HashMap, HashSet};

use crate::capability_catalog::{CapabilityCatalog, CapabilityDescriptor, CapabilityKind};
use crate::skill_coverage::SkillContextState;

/// The event that caused the local capability lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskEvent {
    TaskStart,
    CapabilityMiss,
    InventoryQuestion,
}

/// Inputs to one local suggestion decision.
pub struct SuggestContext<'a> {
    pub query: &'a str,
    /// Coverage keyed by stable capability descriptor ID.
    pub coverage: &'a HashMap<String, SkillContextState>,
    /// IDs returned by earlier suggestions in the same task.
    pub already_suggested: &'a HashSet<String>,
    pub task_event: TaskEvent,
}

/// A small, advisory capability result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Suggestion {
    None,
    Skill {
        id: String,
        name: String,
        reason: String,
        load_hint: String,
    },
    Tools {
        handles: Vec<ToolHandle>,
        reason: String,
    },
    Ambiguous {
        candidates: Vec<String>,
    },
    /// Entries are catalog names with an optional one-line description.
    Inventory {
        skills: Vec<String>,
        tools: Vec<String>,
    },
}

/// Stable ID and local MCP routing information for a tool suggestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolHandle {
    pub id: String,
    pub server: String,
    pub name: String,
}

/// Bounded in-memory suppression state for suggestions already shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SuggestState {
    suggested: HashSet<String>,
}

impl SuggestState {
    /// Record capability IDs included in a suggestion.
    pub fn record(&mut self, suggestion: &Suggestion) {
        match suggestion {
            Suggestion::Skill { id, .. } => {
                self.suggested.insert(id.clone());
            }
            Suggestion::Tools { handles, .. } => {
                self.suggested
                    .extend(handles.iter().map(|handle| handle.id.clone()));
            }
            Suggestion::Ambiguous { candidates } => {
                self.suggested.extend(candidates.iter().cloned());
            }
            Suggestion::None | Suggestion::Inventory { .. } => {}
        }
    }

    /// IDs suggested so far, suitable for [`SuggestContext::already_suggested`].
    pub fn suggested(&self) -> &HashSet<String> {
        &self.suggested
    }
}

/// Suggest a relevant capability from the current catalog without side effects.
pub fn suggest_capabilities(
    catalog: &CapabilityCatalog,
    context: &SuggestContext<'_>,
) -> Suggestion {
    if context.task_event == TaskEvent::InventoryQuestion || is_inventory_question(context.query) {
        return inventory(catalog);
    }

    let mut candidates: Vec<_> = catalog
        .shortlist(context.query, 4)
        .into_iter()
        .filter(|entry| {
            !context.already_suggested.contains(&entry.id)
                && !(entry.kind == CapabilityKind::Skill
                    && context.coverage.get(&entry.id) == Some(&SkillContextState::Loaded))
        })
        .collect();
    candidates = prefer_native(&candidates);

    let Some(top) = candidates.first() else {
        return Suggestion::None;
    };

    let name_matches = whole_word_contains(context.query, &top.name);
    if name_matches || candidates.len() == 1 {
        let reason = if name_matches {
            "name matches the request"
        } else {
            "only relevant capability"
        }
        .to_owned();
        if top.kind == CapabilityKind::Skill {
            return Suggestion::Skill {
                id: top.id.clone(),
                name: top.name.clone(),
                load_hint: format!("${}", top.name),
                reason,
            };
        }
        return Suggestion::Tools {
            handles: tool_handles(&candidates),
            reason,
        };
    }

    Suggestion::Ambiguous {
        candidates: candidates
            .iter()
            .take(4)
            .map(|entry| entry.id.clone())
            .collect(),
    }
}

/// Prefer the native Cognee search tool over MCP memory routes when both occur.
pub fn prefer_native<'a>(candidates: &[&'a CapabilityDescriptor]) -> Vec<&'a CapabilityDescriptor> {
    let native_search: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|entry| {
            entry.kind == CapabilityKind::McpTool
                && entry.name == "cognee_search"
                && !entry.source.starts_with("mcp:")
        })
        .take(1)
        .collect();
    let has_mcp_memory = candidates.iter().any(|entry| {
        entry.kind == CapabilityKind::McpTool
            && entry.source.starts_with("mcp:")
            && (entry.name.to_lowercase().contains("cognee")
                || entry.name.to_lowercase().contains("memory"))
    });
    if has_mcp_memory && !native_search.is_empty() {
        native_search
    } else {
        candidates.to_vec()
    }
}

/// Whether a query asks for an inventory rather than a task capability.
pub fn is_inventory_question(query: &str) -> bool {
    const PHRASES: &[&str] = &[
        "what skills",
        "which skills",
        "list skills",
        "available skills",
        "what tools",
        "which tools",
        "list tools",
        "available tools",
    ];
    let query = query.to_lowercase();
    PHRASES.iter().any(|phrase| query.contains(phrase))
}

/// Render a compact advisory hint, never exceeding 280 UTF-16 code units.
pub fn render_suggestion(suggestion: &Suggestion) -> Option<String> {
    let text = match suggestion {
        Suggestion::None => return None,
        Suggestion::Skill {
            name, load_hint, ..
        } => format!(
            "Capability hint (advisory): skill \"{}\" may help ({}).",
            name, load_hint
        ),
        Suggestion::Tools { handles, .. } => {
            let handles = handles
                .iter()
                .take(2)
                .map(|handle| format!("{}/{}", handle.server, handle.name))
                .collect::<Vec<_>>()
                .join(", ");
            format!("Capability hint (advisory): tool {} may help.", handles)
        }
        Suggestion::Ambiguous { candidates } => format!(
            "Capability hint (advisory): possible capabilities: {}.",
            candidates
                .iter()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Suggestion::Inventory { skills, tools } => {
            let skills = skills
                .iter()
                .take(12)
                .map(|entry| inventory_entry(entry))
                .collect::<Vec<_>>()
                .join(", ");
            let tools = tools
                .iter()
                .take(12)
                .map(|entry| inventory_entry(entry))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Capability inventory (advisory): skills [{}]; tools [{}].",
                skills, tools
            )
        }
    };
    Some(truncate_utf16(&text, 280))
}

fn inventory(catalog: &CapabilityCatalog) -> Suggestion {
    let mut skills = Vec::new();
    let mut tools = Vec::new();
    for entry in &catalog.entries {
        if !entry.available || !entry.model_invocable {
            continue;
        }
        let target = match entry.kind {
            CapabilityKind::Skill => &mut skills,
            CapabilityKind::McpTool => &mut tools,
        };
        if target.len() < 12 {
            let description = truncate_utf16(entry.description.trim(), 80);
            let line = if description.is_empty() {
                entry.name.clone()
            } else {
                format!("{} — {}", entry.name, description)
            };
            target.push(line);
        }
    }
    Suggestion::Inventory { skills, tools }
}

fn inventory_entry(entry: &str) -> String {
    let Some((name, description)) = entry.split_once(" — ") else {
        return entry.to_owned();
    };
    format!("{} — {}", name, truncate_utf16(description, 80))
}

fn tool_handles(candidates: &[&CapabilityDescriptor]) -> Vec<ToolHandle> {
    candidates
        .iter()
        .filter(|entry| entry.kind == CapabilityKind::McpTool)
        .take(2)
        .filter_map(|entry| {
            let server = entry.source.strip_prefix("mcp:").unwrap_or(&entry.source);
            Some(ToolHandle {
                id: entry.id.clone(),
                server: server.to_owned(),
                name: entry.name.clone(),
            })
        })
        .collect()
}

fn whole_word_contains(haystack: &str, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    let needle: Vec<_> = needle
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    if needle.is_empty() {
        return false;
    }
    let haystack = haystack.to_lowercase();
    let words: Vec<_> = haystack
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    words.windows(needle.len()).any(|window| window == needle)
}

/// Truncate at a Unicode scalar boundary while keeping within UTF-16 units.
fn truncate_utf16(value: &str, max_units: usize) -> String {
    if value.encode_utf16().count() <= max_units {
        return value.to_owned();
    }
    if max_units == 0 {
        return String::new();
    }
    let budget = max_units - 1;
    let mut output = String::new();
    let mut used = 0;
    for character in value.chars() {
        let units = character.len_utf16();
        if used + units > budget {
            break;
        }
        output.push(character);
        used += units;
    }
    output.push('…');
    output
}

#[cfg(test)]
#[path = "capability_suggest_tests.rs"]
mod tests;
