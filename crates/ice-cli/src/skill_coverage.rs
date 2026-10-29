//! Pure skill-context coverage derived only from the supplied message branch.
//!
//! `Loaded` means matching full instruction text is present in that branch; it
//! does not claim the model attended to or used the instructions. Advertised
//! metadata alone, summaries, partial reads, and messages removed by compaction
//! do not count. Explicit invocation blocks are recognized only in user text and
//! must match the host's skill name, file path, relative-reference line, and
//! block format. Read results use the built-in tool's unnumbered text output and
//! structured truncation details. Skill-file frontmatter is removed and line
//! endings normalized like the skill loader before hashing; no line-number or
//! display-prefix stripping is performed.

use std::collections::{HashMap, HashSet};

use ice_agent::AgentMessage;
use ice_ai::types::{Content, UserContent};
use sha2::{Digest, Sha256};

/// The skill's full instruction content expected in the current branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedSkillRef {
    /// Stable capability descriptor ID; coverage is returned under this key.
    pub id: String,
    pub name: String,
    pub file_path: String,
    /// SHA-256 of the current full skill instruction content.
    pub content_hash: String,
}

/// Whether full instructions for a discovered skill are present in this branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillContextState {
    /// Only advertised metadata is known, or no matching full load is present.
    Advertised,
    /// A matching full instruction body is present in the supplied messages.
    Loaded,
    /// A full load for this path is present, but its content hash is outdated.
    Stale,
}

/// Derive skill instruction coverage from the given branch messages.
///
/// Explicit invocations are checked only in user-role text and must use the
/// exact format emitted by `format_skill_invocation`. A successful `read` tool
/// result counts only when its matching assistant call requested the exact file
/// path without `offset` or `limit`, and the result reports no truncation. The
/// built-in read tool returns plain text without line-number prefixes; skill
/// file frontmatter is normalized and removed before hashing, like the loader.
///
/// The output is keyed by `LoadedSkillRef::id`, allowing capability descriptors
/// to look up their state without conflating same-name skills from different
/// sources. Partial reads are ignored; an earlier full load with a stale hash
/// can still establish `Stale`. If both current and outdated full bodies occur,
/// current instructions are considered loaded.
pub fn skill_coverage(
    messages: &[AgentMessage],
    skills: &[LoadedSkillRef],
) -> HashMap<String, SkillContextState> {
    let mut current = HashSet::new();
    let mut stale = HashSet::new();
    let read_calls = read_calls(messages);

    for message in messages {
        match message {
            AgentMessage::User(user) => {
                for text in user_texts(&user.content) {
                    for skill in skills {
                        for body in invocation_bodies(text, skill) {
                            if hash_content(body) == skill.content_hash {
                                current.insert(skill.id.as_str());
                            } else {
                                stale.insert(skill.id.as_str());
                            }
                        }
                    }
                }
            }
            AgentMessage::ToolResult(result) if result.tool_name == "read" && !result.is_error => {
                let Some(read_call) = read_calls.get(&result.tool_call_id) else {
                    continue;
                };
                if !read_call.full {
                    continue;
                }
                let Some(text) = read_result_text(result) else {
                    continue;
                };
                for skill in skills {
                    if read_call.path == skill.file_path {
                        if hash_content(&skill_file_content(text)) == skill.content_hash {
                            current.insert(skill.id.as_str());
                        } else {
                            stale.insert(skill.id.as_str());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    skills
        .iter()
        .map(|skill| {
            let state = if current.contains(skill.id.as_str()) {
                SkillContextState::Loaded
            } else if stale.contains(skill.id.as_str()) {
                SkillContextState::Stale
            } else {
                SkillContextState::Advertised
            };
            (skill.id.clone(), state)
        })
        .collect()
}

fn user_texts(content: &UserContent) -> Vec<&str> {
    match content {
        UserContent::Text(text) => vec![text],
        UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                Content::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect(),
    }
}

fn invocation_bodies<'a>(text: &'a str, skill: &LoadedSkillRef) -> Vec<&'a str> {
    let dirname = dirname_env_path(&skill.file_path);
    let opening = format!(
        "<skill name=\"{}\" location=\"{}\">\nReferences are relative to {}.\n\n",
        skill.name, skill.file_path, dirname
    );
    let mut remaining = text;
    let mut bodies = Vec::new();
    while let Some(opening_start) = remaining.find(&opening) {
        let start = opening_start + opening.len();
        let Some(body_end) = remaining[start..].find("\n</skill>") else {
            break;
        };
        let body_end = start + body_end;
        bodies.push(&remaining[start..body_end]);
        remaining = &remaining[body_end + "\n</skill>".len()..];
    }
    bodies
}

fn dirname_env_path(path: &str) -> &str {
    let normalized = path.trim_end_matches(['/', '\\']);
    let separator = normalized.rfind(['/', '\\']);
    match separator {
        Some(0) => "/",
        Some(index) if index == 2 && normalized.as_bytes().get(1) == Some(&b':') => {
            &normalized[..3]
        }
        Some(index) => &normalized[..index],
        None => "/",
    }
}

fn skill_file_content(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    if !normalized.starts_with("---") {
        return normalized;
    }
    let Some(end_rel) = normalized[3..].find("\n---") else {
        return normalized;
    };
    let end_index = end_rel + 3;
    normalized
        .get(end_index + 4..)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn hash_content(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

#[derive(Clone, Debug)]
struct ReadCall {
    path: String,
    full: bool,
}

fn read_calls(messages: &[AgentMessage]) -> HashMap<String, ReadCall> {
    let mut calls = HashMap::new();
    for message in messages {
        let AgentMessage::Assistant(assistant) = message else {
            continue;
        };
        for content in &assistant.content {
            let Content::ToolCall(call) = content else {
                continue;
            };
            if call.name != "read" {
                continue;
            }
            let Some(arguments) = call.arguments.as_object() else {
                continue;
            };
            let Some(path) = arguments.get("path").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let no_range = ["offset", "limit"]
                .iter()
                .all(|key| arguments.get(*key).is_none_or(serde_json::Value::is_null));
            calls.insert(
                call.id.clone(),
                ReadCall {
                    path: path.to_string(),
                    full: no_range,
                },
            );
        }
    }
    calls
}

fn read_result_text(result: &ice_ai::types::ToolResultMessage) -> Option<&str> {
    if result.content.len() != 1 {
        return None;
    }
    if let Some(details) = result.details.as_ref() {
        if let Some(truncation) = details.get("truncation") {
            if truncation
                .get("truncated")
                .and_then(serde_json::Value::as_bool)
                != Some(false)
            {
                return None;
            }
        }
    }
    match result.content.first()? {
        Content::Text(text) => Some(&text.text),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::types::{Api, AssistantMessage, ToolResultMessage, ToolResultRole, UserMessage};

    const PATH: &str = "/repo/.icers/skills/review/SKILL.md";
    const BODY: &str = "Check the implementation carefully.";

    fn skill(id: &str, name: &str, path: &str, body: &str) -> LoadedSkillRef {
        LoadedSkillRef {
            id: id.into(),
            name: name.into(),
            file_path: path.into(),
            content_hash: hash_content(body),
        }
    }

    fn invocation(skill: &LoadedSkillRef, body: &str) -> String {
        format_skill_invocation(skill, body, None)
    }

    fn format_skill_invocation(skill: &LoadedSkillRef, body: &str, extra: Option<&str>) -> String {
        ice_harness::skills::format_skill_invocation(
            &ice_harness::types::Skill {
                name: skill.name.clone(),
                description: "Test skill".into(),
                content: body.into(),
                file_path: skill.file_path.clone(),
                disable_model_invocation: None,
            },
            extra,
        )
    }

    fn user(text: impl Into<String>) -> AgentMessage {
        AgentMessage::User(UserMessage::new(text.into(), 1))
    }

    fn read_call(path: &str, offset: Option<u64>, limit: Option<u64>) -> AgentMessage {
        let mut arguments = serde_json::json!({"path": path});
        if let Some(offset) = offset {
            arguments["offset"] = serde_json::json!(offset);
        }
        if let Some(limit) = limit {
            arguments["limit"] = serde_json::json!(limit);
        }
        let mut assistant = AssistantMessage::empty(Api::OpenaiCompletions, "test", "test", 2);
        assistant
            .content
            .push(Content::tool_call("read-call", "read", arguments));
        AgentMessage::Assistant(Box::new(assistant))
    }

    fn read_result(text: &str, is_error: bool, details: Option<serde_json::Value>) -> AgentMessage {
        AgentMessage::ToolResult(Box::new(ToolResultMessage {
            role: ToolResultRole,
            tool_call_id: "read-call".into(),
            tool_name: "read".into(),
            content: vec![Content::text(text)],
            details,
            usage: None,
            added_tool_names: Vec::new(),
            is_error,
            timestamp: 3,
        }))
    }

    fn state(messages: &[AgentMessage], skill: &LoadedSkillRef) -> SkillContextState {
        skill_coverage(messages, std::slice::from_ref(skill))[&skill.id]
    }

    #[test]
    fn explicit_invocation_block_is_loaded() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [user(invocation(&skill, BODY))];
        assert_eq!(state(&messages, &skill), SkillContextState::Loaded);
    }

    #[test]
    fn explicit_invocation_allows_additional_user_instructions() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [user(
            invocation(&skill, BODY) + "\n\nPlease focus on tests.",
        )];
        assert_eq!(state(&messages, &skill), SkillContextState::Loaded);
    }

    #[test]
    fn full_read_of_the_exact_skill_file_is_loaded() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let file = format!("---\nname: review\ndescription: Review\n---\n{BODY}");
        let messages = [read_call(PATH, None, None), read_result(&file, false, None)];
        assert_eq!(state(&messages, &skill), SkillContextState::Loaded);
    }

    #[test]
    fn failed_read_result_does_not_load_skill() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [read_call(PATH, None, None), read_result(BODY, true, None)];
        assert_eq!(state(&messages, &skill), SkillContextState::Advertised);
    }

    #[test]
    fn read_with_offset_is_ignored() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [
            read_call(PATH, Some(1), None),
            read_result(BODY, false, None),
        ];
        assert_eq!(state(&messages, &skill), SkillContextState::Advertised);
    }

    #[test]
    fn read_with_limit_is_ignored() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [
            read_call(PATH, None, Some(100)),
            read_result(BODY, false, None),
        ];
        assert_eq!(state(&messages, &skill), SkillContextState::Advertised);
    }

    #[test]
    fn tool_reported_truncation_is_ignored() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let details = Some(serde_json::json!({"truncation": {"truncated": true}}));
        let messages = [
            read_call(PATH, None, None),
            read_result(BODY, false, details),
        ];
        assert_eq!(state(&messages, &skill), SkillContextState::Advertised);
    }

    #[test]
    fn changed_skill_invocation_is_stale() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [user(invocation(&skill, "Old instructions."))];
        assert_eq!(state(&messages, &skill), SkillContextState::Stale);
    }

    #[test]
    fn changed_skill_read_is_stale() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [
            read_call(PATH, None, None),
            read_result("Old instructions.", false, None),
        ];
        assert_eq!(state(&messages, &skill), SkillContextState::Stale);
    }

    #[test]
    fn no_matching_message_is_advertised() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        assert_eq!(state(&[], &skill), SkillContextState::Advertised);
    }

    #[test]
    fn same_name_skills_from_distinct_paths_are_independent() {
        let first = skill("skill:project:review", "review", PATH, BODY);
        let second = skill(
            "skill:package:review",
            "review",
            "/repo/.icers/packages/review/SKILL.md",
            "Other instructions.",
        );
        let messages = [user(invocation(&first, BODY))];
        let coverage = skill_coverage(&messages, &[first.clone(), second.clone()]);
        assert_eq!(coverage[&first.id], SkillContextState::Loaded);
        assert_eq!(coverage[&second.id], SkillContextState::Advertised);
    }

    #[test]
    fn summary_mention_after_compaction_is_advertised() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [user("Summary: the review skill was used earlier.")];
        assert_eq!(state(&messages, &skill), SkillContextState::Advertised);
    }

    #[test]
    fn restored_messages_reproduce_live_coverage() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let live = vec![user(invocation(&skill, BODY))];
        let restored: Vec<AgentMessage> = live
            .iter()
            .map(|message| serde_json::from_value(serde_json::to_value(message).unwrap()).unwrap())
            .collect();
        assert_eq!(
            skill_coverage(&live, std::slice::from_ref(&skill)),
            skill_coverage(&restored, std::slice::from_ref(&skill))
        );
    }

    #[test]
    fn assistant_skill_lookalike_does_not_load() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [
            AgentMessage::Assistant(Box::new(AssistantMessage::empty(
                Api::OpenaiCompletions,
                "test",
                "test",
                2,
            ))),
            user("ordinary user message"),
        ];
        let mut messages = messages.to_vec();
        let AgentMessage::Assistant(assistant) = &mut messages[0] else {
            unreachable!();
        };
        assistant
            .content
            .push(Content::text(invocation(&skill, BODY)));
        assert_eq!(state(&messages, &skill), SkillContextState::Advertised);
    }

    #[test]
    fn tool_result_skill_lookalike_does_not_load() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let messages = [
            read_call("/other/file.txt", None, None),
            read_result(&invocation(&skill, BODY), false, None),
        ];
        assert_eq!(state(&messages, &skill), SkillContextState::Advertised);
    }

    #[test]
    fn user_lookalike_with_different_location_is_ignored() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let lookalike = format!(
            "<skill name=\"review\" location=\"/elsewhere/SKILL.md\">\nReferences are relative to /elsewhere.\n\n{BODY}\n</skill>"
        );
        assert_eq!(
            state(&[user(lookalike)], &skill),
            SkillContextState::Advertised
        );
    }

    #[test]
    fn user_code_block_with_different_location_is_ignored() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        let lookalike = format!(
            "```xml\n<skill name=\"review\" location=\"/elsewhere/SKILL.md\">\nReferences are relative to /elsewhere.\n\n{BODY}\n</skill>\n```"
        );
        assert_eq!(
            state(&[user(lookalike)], &skill),
            SkillContextState::Advertised
        );
    }

    #[test]
    fn user_lookalike_with_matching_path_and_old_hash_is_stale() {
        let skill = skill("skill:project:review", "review", PATH, BODY);
        assert_eq!(
            state(&[user(invocation(&skill, "Older body."))], &skill),
            SkillContextState::Stale
        );
    }

    #[test]
    fn hidden_skill_can_be_loaded_by_explicit_invocation() {
        let skill_ref = skill("skill:project:hidden", "hidden", PATH, BODY);
        let messages = [user(invocation(&skill_ref, BODY))];
        assert_eq!(state(&messages, &skill_ref), SkillContextState::Loaded);

        let hidden = ice_harness::types::Skill {
            name: skill_ref.name.clone(),
            description: "Hidden instructions".into(),
            content: BODY.into(),
            file_path: PATH.into(),
            disable_model_invocation: Some(true),
        };
        let catalog =
            crate::capability_catalog::CapabilityCatalog::build(&[("project", hidden)], &[]);
        assert!(catalog.shortlist("hidden instructions", 8).is_empty());
        assert_eq!(
            catalog.with_context_state(&skill_coverage(&messages, &[skill_ref]))[0].1,
            SkillContextState::Loaded
        );
    }

    #[test]
    fn context_accessor_returns_only_skill_descriptors() {
        let catalog = crate::capability_catalog::CapabilityCatalog {
            entries: vec![
                crate::capability_catalog::CapabilityDescriptor {
                    id: "skill:project:review".into(),
                    kind: crate::capability_catalog::CapabilityKind::Skill,
                    name: "review".into(),
                    description: "Review code".into(),
                    source: "project".into(),
                    content_hash: "descriptor-hash".into(),
                    available: true,
                    model_invocable: true,
                },
                crate::capability_catalog::CapabilityDescriptor {
                    id: "mcp:server:search".into(),
                    kind: crate::capability_catalog::CapabilityKind::McpTool,
                    name: "search".into(),
                    description: "Search".into(),
                    source: "mcp:server".into(),
                    content_hash: "descriptor-hash".into(),
                    available: true,
                    model_invocable: true,
                },
            ],
        };
        let coverage = HashMap::from([("skill:project:review".into(), SkillContextState::Stale)]);
        let states = catalog.with_context_state(&coverage);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].0.id, "skill:project:review");
        assert_eq!(states[0].1, SkillContextState::Stale);
    }

    #[test]
    fn catalog_defaults_missing_skill_coverage_to_advertised() {
        let catalog = crate::capability_catalog::CapabilityCatalog {
            entries: vec![crate::capability_catalog::CapabilityDescriptor {
                id: "skill:project:review".into(),
                kind: crate::capability_catalog::CapabilityKind::Skill,
                name: "review".into(),
                description: "Review code".into(),
                source: "project".into(),
                content_hash: "descriptor-hash".into(),
                available: true,
                model_invocable: true,
            }],
        };
        assert_eq!(
            catalog.with_context_state(&HashMap::new())[0].1,
            SkillContextState::Advertised
        );
    }
}
