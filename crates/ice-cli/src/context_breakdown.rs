//! Pure context-occupancy projection for a caller-supplied live context snapshot.
//!
//! Estimates use `ceil(UTF-16 code units / 4)`, matching JavaScript string-length
//! accounting; each image contributes 4,800 code units without inspecting base64
//! data. These are composition estimates, not tokenizer output. Provider-measured
//! tokens are a separate optional input and can disagree with the category sum.
//!
//! Pass `context` after session compaction projection, before the default
//! `AgentMessage`→provider-message conversion, to preserve summary, shell and
//! extension roles. The helper cannot discover the final hook-transformed system
//! prompt, file contents, skill listing, or context window itself: callers supply
//! those snapshots explicitly. `base_system_prompt` is the composed prompt before
//! caller/extension additions and includes the separately supplied context-file
//! and skill text. `system_prompt` is the final composed prompt snapshot. The base
//! prompt category subtracts the separately estimated files and skills (floored
//! at zero); prompt additions are `max(0, final − base)`.
//!
//! Tool definitions use the concrete provider [`Tool`] schema; callers may supply
//! explicit schema-name provenance from [`SourceInfo`] through `tool_sources`.
//! Missing provenance is `other`; this module does not invent a registry. Tool
//! definitions group by source label, while tool results remain a distinct
//! category attributed to their actual `tool_name`. The current harness conversion
//! drops malformed or unrecognized custom roles; those are omitted here too.
//! Generic `custom` messages are extension messages keyed by `customType`; known
//! shell and summary roles retain their own categories.

use crate::source_info::SourceInfo;
use ice_agent::message::AgentMessage;
use ice_ai::types::{Content, Tool, UserContent};
use ice_harness::messages::{
    bash_execution_data, branch_summary_data, compaction_summary_data, custom_data,
    BASH_EXECUTION_ROLE, BRANCH_SUMMARY_ROLE, COMPACTION_SUMMARY_ROLE, CUSTOM_ROLE,
};
use ice_harness::types::CompactionSettings;
use serde::{Deserialize, Serialize};

/// Fixed category order in the returned breakdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextCategoryKey {
    SystemPrompt,
    ContextFiles,
    Skills,
    Tools,
    PromptAdditions,
    Conversation,
    ToolResults,
    ExtensionMessages,
    Summaries,
    Shell,
}

impl ContextCategoryKey {
    fn label(self) -> &'static str {
        match self {
            Self::SystemPrompt => "System prompt",
            Self::ContextFiles => "Context files",
            Self::Skills => "Skills",
            Self::Tools => "Tools",
            Self::PromptAdditions => "Prompt additions",
            Self::Conversation => "Conversation",
            Self::ToolResults => "Tool results",
            Self::ExtensionMessages => "Extension messages",
            Self::Summaries => "Summaries",
            Self::Shell => "Shell",
        }
    }
}

/// A positive, estimated category component, in estimated tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownPart {
    pub label: String,
    pub tokens: u64,
}

/// One category and its six largest positive, label-aggregated components.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCategory {
    pub key: ContextCategoryKey,
    pub label: String,
    pub tokens: u64,
    pub parts: Vec<ContextBreakdownPart>,
}

/// A largest individual context item. The returned list contains at most five.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownItem {
    pub label: String,
    pub tokens: u64,
}

/// The complete estimate, with provider-measured usage kept distinct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdown {
    pub context_window: u64,
    pub estimated_tokens: u64,
    /// Caller-supplied current-context measurement, or `None` when unknown.
    pub measured_tokens: Option<u64>,
    /// The measurement when present, otherwise the estimated composition sum.
    pub used_tokens: u64,
    pub free_tokens: u64,
    /// Enabled compaction threshold (`context_window - reserve_tokens`), omitted
    /// from serialized output when compaction is disabled or unspecified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_at_tokens: Option<i64>,
    pub categories: Vec<ContextCategory>,
    pub largest_items: Vec<ContextBreakdownItem>,
}

/// Text and its display label for a caller-supplied resource snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBreakdownText<'a> {
    pub label: &'a str,
    /// The actual model-visible text, not its path or discovery metadata.
    pub text: &'a str,
}

/// Input snapshot for [`compute_context_breakdown`].
///
/// `context` must be the active, compaction-projected context. Measured tokens
/// are trusted as a snapshot; callers should pass `None` after compaction until
/// a valid post-compaction assistant usage measurement exists. The helper does
/// not read session state or run context/provider transforms. A session facade
/// should return no breakdown when there is no model or the model window is zero.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContextBreakdownInput<'a> {
    pub context: &'a [AgentMessage],
    pub context_window: u64,
    pub system_prompt: Option<&'a str>,
    pub base_system_prompt: Option<&'a str>,
    pub context_files: &'a [ContextBreakdownText<'a>],
    pub skills: &'a [ContextBreakdownText<'a>],
    /// Active provider schemas; `None` means tool selection is unknown, while
    /// `Some(&[])` explicitly means no tools are active.
    pub tools: Option<&'a [Tool]>,
    /// Explicit `(schema name, provenance)` pairs; no registry lookup is done.
    /// Unmatched tool definitions receive the `other` source label.
    pub tool_sources: &'a [(&'a str, &'a SourceInfo)],
    pub measured_tokens: Option<u64>,
    pub compaction: Option<&'a CompactionSettings>,
}

/// Return the SESSION037 display label for explicit tool provenance.
///
/// Precedence is built-in, SDK, the pi-mcp-adapter marker, npm/git source
/// prefixes, inline marker contents, then a path-derived extension label. Paths
/// are normalized to `/` separators before handling, including Windows paths
/// inspected on Unix. An `index` entrypoint below `src`, `dist`, or `lib` uses
/// the directory immediately above that build/source directory.
pub fn tool_source_label(source_info: Option<&SourceInfo>) -> String {
    let Some(source_info) = source_info else {
        return "other".to_string();
    };

    let source = source_info.source.trim();
    if source.eq_ignore_ascii_case("builtin") || source.to_ascii_lowercase().starts_with("builtin:")
    {
        return "built-in".to_string();
    }
    if source.eq_ignore_ascii_case("sdk") || source.to_ascii_lowercase().starts_with("sdk:") {
        return "sdk".to_string();
    }

    let source_lower = source.to_ascii_lowercase();
    let path = source_info.path.to_string_lossy();
    if source_lower.contains("pi-mcp-adapter")
        || path.to_ascii_lowercase().contains("pi-mcp-adapter")
    {
        return "mcp".to_string();
    }

    for prefix in ["npm:", "git:"] {
        if let Some(label) = source.strip_prefix(prefix) {
            if !label.is_empty() {
                return label.to_string();
            }
        }
    }

    if let Some(label) = inline_marker_contents(source).or_else(|| inline_marker_contents(&path)) {
        if !label.is_empty() {
            return label;
        }
    }
    path_derived_tool_label(&path)
}

fn inline_marker_contents(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let marker = lower.find("inline:")?;
    let contents = value[marker + "inline:".len()..]
        .trim()
        .trim_matches(|character| {
            matches!(character, '<' | '>' | '(' | ')' | '[' | ']' | '\'' | '"')
        });
    (!contents.is_empty()).then(|| contents.to_string())
}

fn path_derived_tool_label(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let components: Vec<&str> = normalized
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect();
    let Some(filename) = components.last().copied() else {
        return "other".to_string();
    };
    let stem = filename
        .rsplit_once('.')
        .map(|(stem, _)| if stem.is_empty() { filename } else { stem })
        .unwrap_or(filename);
    if !stem.eq_ignore_ascii_case("index") {
        return stem.to_string();
    }

    if let Some(build_dir) = components[..components.len().saturating_sub(1)]
        .iter()
        .rposition(|component| {
            component.eq_ignore_ascii_case("src")
                || component.eq_ignore_ascii_case("dist")
                || component.eq_ignore_ascii_case("lib")
        })
    {
        if build_dir > 0 {
            return components[build_dir - 1].to_string();
        }
    }
    stem.to_string()
}

/// Estimate tokens for `text` using UTF-16 code units and the `/ 4` heuristic.
pub fn estimate_text_tokens(text: &str) -> u64 {
    estimate_units_tokens(text.encode_utf16().count() as u64)
}

/// Compute the ten-category projection without I/O or session mutation.
pub fn compute_context_breakdown(input: &ContextBreakdownInput<'_>) -> ContextBreakdown {
    let mut system_prompt = CategoryBuilder::new(ContextCategoryKey::SystemPrompt);
    let mut context_files = CategoryBuilder::new(ContextCategoryKey::ContextFiles);
    let mut skills = CategoryBuilder::new(ContextCategoryKey::Skills);
    let mut tools = CategoryBuilder::new(ContextCategoryKey::Tools);
    let mut prompt_additions = CategoryBuilder::new(ContextCategoryKey::PromptAdditions);
    let mut conversation = CategoryBuilder::new(ContextCategoryKey::Conversation);
    let mut tool_results = CategoryBuilder::new(ContextCategoryKey::ToolResults);
    let mut extension_messages = CategoryBuilder::new(ContextCategoryKey::ExtensionMessages);
    let mut summaries = CategoryBuilder::new(ContextCategoryKey::Summaries);
    let mut shell = CategoryBuilder::new(ContextCategoryKey::Shell);
    let mut largest_items = Vec::new();

    let file_tokens = add_text_items(&mut context_files, input.context_files);
    let read_is_available = input
        .tools
        .map(|active| active.iter().any(|tool| tool.name == "read"))
        .unwrap_or(true);
    let skill_tokens = if read_is_available {
        add_text_items(&mut skills, input.skills)
    } else {
        0
    };
    let base_prompt = input
        .base_system_prompt
        .or(input.system_prompt)
        .unwrap_or_default();
    let current_prompt = input
        .system_prompt
        .or(input.base_system_prompt)
        .unwrap_or_default();
    let base_tokens = estimate_text_tokens(base_prompt);
    let current_tokens = estimate_text_tokens(current_prompt);
    system_prompt.add(
        "Base system prompt",
        base_tokens.saturating_sub(file_tokens.saturating_add(skill_tokens)),
    );
    prompt_additions.add(
        "Prompt additions",
        current_tokens.saturating_sub(base_tokens),
    );

    if let Some(definitions) = input.tools {
        for tool in definitions {
            let schema = serde_json::to_string(tool.parameters.as_value())
                .unwrap_or_else(|_| "[unserializable]".to_string());
            let units = utf16_units(&tool.name)
                .saturating_add(utf16_units(&tool.description))
                .saturating_add(utf16_units(&schema));
            let source_info = input
                .tool_sources
                .iter()
                .find(|(name, _)| *name == tool.name)
                .map(|(_, info)| *info);
            tools.add(
                &tool_source_label(source_info),
                estimate_units_tokens(units),
            );
        }
    }

    for (index, message) in input.context.iter().enumerate() {
        let position = index + 1;
        match message {
            AgentMessage::User(user) => {
                let units = match &user.content {
                    UserContent::Text(text) => utf16_units(text),
                    UserContent::Blocks(blocks) => content_units(blocks),
                };
                let tokens = estimate_units_tokens(units);
                conversation.add("User messages", tokens);
                push_largest(
                    &mut largest_items,
                    format!("User message #{position}"),
                    tokens,
                );
            }
            AgentMessage::Assistant(assistant) => {
                let mut text = 0u64;
                let mut thinking = 0u64;
                let mut calls = 0u64;
                let mut images = 0u64;
                for block in &assistant.content {
                    match block {
                        Content::Text(block) => {
                            text = text.saturating_add(utf16_units(&block.text))
                        }
                        Content::Thinking(block) => {
                            thinking = thinking.saturating_add(utf16_units(&block.thinking))
                        }
                        Content::ToolCall(call) => {
                            let serialized = serde_json::to_string(call)
                                .unwrap_or_else(|_| "[unserializable]".to_string());
                            calls = calls.saturating_add(utf16_units(&serialized));
                        }
                        Content::Image(_) => images = images.saturating_add(IMAGE_CODE_UNITS),
                    }
                }
                let units = text
                    .saturating_add(thinking)
                    .saturating_add(calls)
                    .saturating_add(images);
                let tokens = estimate_units_tokens(units);
                conversation.add("Assistant text", estimate_units_tokens(text));
                conversation.add("Thinking", estimate_units_tokens(thinking));
                conversation.add("Tool calls", estimate_units_tokens(calls));
                conversation.add("Images", estimate_units_tokens(images));
                push_largest(
                    &mut largest_items,
                    format!("Assistant message #{position}"),
                    tokens,
                );
            }
            AgentMessage::ToolResult(result) => {
                let tokens = estimate_units_tokens(content_units(&result.content));
                tool_results.add(&result.tool_name, tokens);
                push_largest(
                    &mut largest_items,
                    format!("Tool result {} #{position}", result.tool_name),
                    tokens,
                );
            }
            AgentMessage::Custom(custom) => match custom.role.as_str() {
                BASH_EXECUTION_ROLE => {
                    if let Some(data) = bash_execution_data(message) {
                        if data.exclude_from_context != Some(true) {
                            let units = utf16_units(&data.command)
                                .saturating_add(utf16_units(&data.output));
                            let tokens = estimate_units_tokens(units);
                            let label = format!("Shell: {}", truncate_utf16(&data.command, 40));
                            shell.add(&label, tokens);
                            push_largest(&mut largest_items, label, tokens);
                        }
                    }
                }
                BRANCH_SUMMARY_ROLE => {
                    if let Some(data) = branch_summary_data(message) {
                        let tokens = estimate_text_tokens(&data.summary);
                        summaries.add("Branch summary", tokens);
                        push_largest(
                            &mut largest_items,
                            format!("Branch summary #{position}"),
                            tokens,
                        );
                    }
                }
                COMPACTION_SUMMARY_ROLE => {
                    if let Some(data) = compaction_summary_data(message) {
                        let tokens = estimate_text_tokens(&data.summary);
                        summaries.add("Compaction summary", tokens);
                        push_largest(
                            &mut largest_items,
                            format!("Compaction summary #{position}"),
                            tokens,
                        );
                    }
                }
                CUSTOM_ROLE => {
                    let label = custom_data(message)
                        .map(|data| data.custom_type)
                        .unwrap_or_else(|| "custom".to_string());
                    let tokens = estimate_units_tokens(content_units(&custom.content));
                    extension_messages.add(&label, tokens);
                    push_largest(
                        &mut largest_items,
                        format!("Extension {label} #{position}"),
                        tokens,
                    );
                }
                // Unknown custom roles and malformed built-ins are omitted by
                // the harness's default provider conversion as well.
                _ => {}
            },
        }
    }

    let categories = vec![
        system_prompt.finish(),
        context_files.finish(),
        skills.finish(),
        tools.finish(),
        prompt_additions.finish(),
        conversation.finish(),
        tool_results.finish(),
        extension_messages.finish(),
        summaries.finish(),
        shell.finish(),
    ];
    let estimated_tokens = categories
        .iter()
        .fold(0u64, |sum, category| sum.saturating_add(category.tokens));
    largest_items.sort_by(|left, right| right.tokens.cmp(&left.tokens));
    largest_items.truncate(MAX_LARGEST_ITEMS);
    let used_tokens = input.measured_tokens.unwrap_or(estimated_tokens);
    ContextBreakdown {
        context_window: input.context_window,
        estimated_tokens,
        measured_tokens: input.measured_tokens,
        used_tokens,
        free_tokens: input.context_window.saturating_sub(used_tokens),
        compact_at_tokens: input.compaction.and_then(|settings| {
            settings.enabled.then(|| {
                (i128::from(input.context_window) - i128::from(settings.reserve_tokens))
                    .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
            })
        }),
        categories,
        largest_items,
    }
}

const IMAGE_CODE_UNITS: u64 = 4_800;
const MAX_CATEGORY_PARTS: usize = 6;
const MAX_LARGEST_ITEMS: usize = 5;

#[derive(Debug)]
struct CategoryBuilder {
    key: ContextCategoryKey,
    parts: Vec<ContextBreakdownPart>,
}

impl CategoryBuilder {
    fn new(key: ContextCategoryKey) -> Self {
        Self {
            key,
            parts: Vec::new(),
        }
    }

    fn add(&mut self, label: &str, tokens: u64) {
        if tokens == 0 {
            return;
        }
        if let Some(part) = self.parts.iter_mut().find(|part| part.label == label) {
            part.tokens = part.tokens.saturating_add(tokens);
        } else {
            self.parts.push(ContextBreakdownPart {
                label: label.to_string(),
                tokens,
            });
        }
    }

    fn finish(mut self) -> ContextCategory {
        let tokens = self
            .parts
            .iter()
            .fold(0u64, |sum, part| sum.saturating_add(part.tokens));
        self.parts
            .sort_by(|left, right| right.tokens.cmp(&left.tokens));
        self.parts.truncate(MAX_CATEGORY_PARTS);
        ContextCategory {
            key: self.key,
            label: self.key.label().to_string(),
            tokens,
            parts: self.parts,
        }
    }
}

fn add_text_items(category: &mut CategoryBuilder, items: &[ContextBreakdownText<'_>]) -> u64 {
    for item in items {
        category.add(item.label, estimate_text_tokens(item.text));
    }
    category
        .parts
        .iter()
        .fold(0u64, |sum, part| sum.saturating_add(part.tokens))
}

fn content_units(content: &[Content]) -> u64 {
    content.iter().fold(0u64, |units, block| {
        units.saturating_add(match block {
            Content::Text(text) => utf16_units(&text.text),
            Content::Image(_) => IMAGE_CODE_UNITS,
            Content::Thinking(_) | Content::ToolCall(_) => 0,
        })
    })
}

fn utf16_units(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}

fn estimate_units_tokens(units: u64) -> u64 {
    units / 4 + u64::from(units % 4 != 0)
}

fn push_largest(items: &mut Vec<ContextBreakdownItem>, label: String, tokens: u64) {
    if tokens > 0 {
        items.push(ContextBreakdownItem { label, tokens });
    }
}

/// Truncate on a UTF-8 boundary at at most `limit` UTF-16 code units.
fn truncate_utf16(text: &str, limit: usize) -> String {
    let mut end = 0;
    let mut units = 0;
    for (index, character) in text.char_indices() {
        let character_units = character.len_utf16();
        if units + character_units > limit {
            break;
        }
        units += character_units;
        end = index + character.len_utf8();
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::types::{
        Api, ImageContent, ImageContentType, Schema, TextContent, TextContentType, ToolCall,
        ToolCallType, ToolResultMessage, ToolResultRole, UserMessage,
    };
    use ice_harness::session::context::build_session_context;
    use ice_harness::session::types::{CompactionEntry, Entry, EntryBase};

    fn category(breakdown: &ContextBreakdown, key: ContextCategoryKey) -> &ContextCategory {
        breakdown
            .categories
            .iter()
            .find(|item| item.key == key)
            .unwrap()
    }

    fn tokens_from_utf16(text: &str) -> u64 {
        let units = text.encode_utf16().count() as u64;
        units / 4 + u64::from(units % 4 != 0)
    }

    #[test]
    fn tool_source_labels_follow_precedence_and_normalize_package_paths() {
        let builtin =
            SourceInfo::synthetic(r"C:\extensions\pi-mcp-adapter\dist\index.js", "builtin");
        let sdk = SourceInfo::synthetic("/sdk/tools/read.js", "sdk");
        let mcp = SourceInfo::synthetic("/pkg/tool.js", "npm:vendor/pi-mcp-adapter");
        let npm = SourceInfo::synthetic("/ignored/path.js", "npm:example-tool");
        let git = SourceInfo::synthetic("/ignored/path.js", "git:https://host/repo.git");
        let inline = SourceInfo::synthetic("/ignored/path.js", "inline:<embedded-tool>");
        let windows_index = SourceInfo::synthetic(
            r"C:\Users\agent\extensions\my-extension\dist\index.js",
            "local",
        );
        let filename_stem = SourceInfo::synthetic("/extensions/other-tool/tool.entry.ts", "local");

        assert_eq!(tool_source_label(None), "other");
        assert_eq!(tool_source_label(Some(&builtin)), "built-in");
        assert_eq!(tool_source_label(Some(&sdk)), "sdk");
        assert_eq!(tool_source_label(Some(&mcp)), "mcp");
        assert_eq!(tool_source_label(Some(&npm)), "example-tool");
        assert_eq!(tool_source_label(Some(&git)), "https://host/repo.git");
        assert_eq!(tool_source_label(Some(&inline)), "embedded-tool");
        assert_eq!(tool_source_label(Some(&windows_index)), "my-extension");
        assert_eq!(tool_source_label(Some(&filename_stem)), "tool.entry");
    }

    #[test]
    fn estimates_unicode_images_thinking_and_serialized_tool_arguments() {
        let user = AgentMessage::User(UserMessage::new(
            UserContent::Blocks(vec![
                Content::Text(TextContent {
                    kind: TextContentType,
                    text: "é😀".into(),
                    text_signature: None,
                }),
                Content::Image(ImageContent {
                    kind: ImageContentType,
                    data: "base64-is-not-counted".into(),
                    mime_type: "image/png".into(),
                }),
            ]),
            1,
        ));
        let mut assistant = ice_ai::types::AssistantMessage::empty(Api::Faux, "faux", "model", 2);
        assistant.content = vec![
            Content::Thinking(ice_ai::types::ThinkingContent {
                kind: Default::default(),
                thinking: "🧠".into(),
                thinking_signature: None,
                redacted: false,
            }),
            Content::ToolCall(ToolCall {
                kind: ToolCallType,
                id: "c".into(),
                name: "lookup".into(),
                arguments: serde_json::json!({"q": "😀"}),
                thought_signature: None,
                namespace: None,
            }),
        ];
        let call_json = r#"{"type":"toolCall","id":"c","name":"lookup","arguments":{"q":"😀"}}"#;
        let tool = Tool {
            name: "tool".into(),
            description: "😀".into(),
            parameters: Schema::new(serde_json::json!({"x": "é😀"})),
            constrained_sampling: None,
        };
        let schema_json = r#"{"x":"é😀"}"#;
        let context = vec![user, AgentMessage::Assistant(Box::new(assistant))];
        let result = compute_context_breakdown(&ContextBreakdownInput {
            context: &context,
            context_window: 100,
            tools: Some(std::slice::from_ref(&tool)),
            ..Default::default()
        });
        assert_eq!(estimate_text_tokens("é😀"), 1); // 3 UTF-16 units, not 2 chars.
        assert_eq!(
            category(&result, ContextCategoryKey::Conversation)
                .parts
                .iter()
                .find(|part| part.label == "User messages")
                .unwrap()
                .tokens,
            1201
        ); // 4,803 units incl. image.
        assert_eq!(
            category(&result, ContextCategoryKey::Conversation)
                .parts
                .iter()
                .find(|part| part.label == "Thinking")
                .unwrap()
                .tokens,
            1
        );
        assert_eq!(
            category(&result, ContextCategoryKey::Conversation)
                .parts
                .iter()
                .find(|part| part.label == "Tool calls")
                .unwrap()
                .tokens,
            tokens_from_utf16(call_json)
        );
        assert_eq!(
            category(&result, ContextCategoryKey::Tools).parts[0].label,
            "other"
        );
        assert_eq!(
            category(&result, ContextCategoryKey::Tools).parts[0].tokens,
            tokens_from_utf16(&format!("tool😀{schema_json}"))
        );
    }

    #[test]
    fn tool_definitions_and_results_have_separate_attribution() {
        let tools = [
            Tool {
                name: "read".into(),
                description: "schema".into(),
                parameters: Schema::empty_object(),
                constrained_sampling: None,
            },
            Tool {
                name: "search".into(),
                description: "schema".into(),
                parameters: Schema::empty_object(),
                constrained_sampling: None,
            },
            Tool {
                name: "docs".into(),
                description: "schema".into(),
                parameters: Schema::empty_object(),
                constrained_sampling: None,
            },
        ];
        let messages = vec![
            AgentMessage::ToolResult(Box::new(ToolResultMessage {
                role: ToolResultRole,
                tool_call_id: "r".into(),
                tool_name: "read".into(),
                content: vec![Content::text("1234567")],
                details: None,
                usage: None,
                added_tool_names: Vec::new(),
                is_error: false,
                timestamp: 1,
            })),
            AgentMessage::ToolResult(Box::new(ToolResultMessage {
                role: ToolResultRole,
                tool_call_id: "s".into(),
                tool_name: "search".into(),
                content: vec![Content::Image(ImageContent {
                    kind: ImageContentType,
                    data: String::new(),
                    mime_type: "image/png".into(),
                })],
                details: None,
                usage: None,
                added_tool_names: Vec::new(),
                is_error: false,
                timestamp: 2,
            })),
        ];
        let builtin_source = SourceInfo::synthetic("/tools/read.rs", "builtin");
        let package_source = SourceInfo::synthetic("/pkg/src/index.js", "npm:search-package");
        let tool_sources = [
            ("read", &builtin_source),
            ("search", &package_source),
            ("docs", &builtin_source),
        ];
        let result = compute_context_breakdown(&ContextBreakdownInput {
            context: &messages,
            tools: Some(&tools),
            tool_sources: &tool_sources,
            ..Default::default()
        });
        let definitions = &category(&result, ContextCategoryKey::Tools).parts;
        assert_eq!(definitions.len(), 2);
        assert_eq!(
            definitions
                .iter()
                .find(|part| part.label == "built-in")
                .unwrap()
                .tokens,
            6
        );
        assert!(definitions
            .iter()
            .any(|part| part.label == "search-package"));
        assert!(!definitions
            .iter()
            .any(|part| part.label == "read" || part.label == "search"));
        let results = &category(&result, ContextCategoryKey::ToolResults).parts;
        assert_eq!(
            results
                .iter()
                .find(|part| part.label == "read")
                .unwrap()
                .tokens,
            2
        );
        assert_eq!(
            results
                .iter()
                .find(|part| part.label == "search")
                .unwrap()
                .tokens,
            1200
        );
    }

    #[test]
    fn uses_measured_context_separately_and_only_exposes_enabled_compaction_threshold() {
        let prompt = "base prompt";
        let addition = "extended system prompt";
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 20,
            keep_recent_tokens: 0,
        };
        let result = compute_context_breakdown(&ContextBreakdownInput {
            context_window: 100,
            base_system_prompt: Some(prompt),
            system_prompt: Some(addition),
            measured_tokens: Some(3),
            compaction: Some(&settings),
            ..Default::default()
        });
        assert!(result.estimated_tokens > 3);
        assert_eq!(result.measured_tokens, Some(3));
        assert_eq!(result.used_tokens, 3);
        assert_eq!(result.free_tokens, 97);
        assert_eq!(result.compact_at_tokens, Some(80));
        let disabled = CompactionSettings {
            enabled: false,
            ..settings
        };
        let disabled = compute_context_breakdown(&ContextBreakdownInput {
            compaction: Some(&disabled),
            ..Default::default()
        });
        assert_eq!(disabled.compact_at_tokens, None);
        assert!(serde_json::to_value(disabled)
            .unwrap()
            .get("compactAtTokens")
            .is_none());
    }

    #[test]
    fn category_totals_include_hidden_parts_and_skills_follow_read_availability() {
        let parts = [
            ContextBreakdownText {
                label: "one",
                text: "1234",
            },
            ContextBreakdownText {
                label: "two",
                text: "12345678",
            },
            ContextBreakdownText {
                label: "three",
                text: "123456789012",
            },
            ContextBreakdownText {
                label: "four",
                text: "1234567890123456",
            },
            ContextBreakdownText {
                label: "five",
                text: "12345678901234567890",
            },
            ContextBreakdownText {
                label: "six",
                text: "123456789012345678901234",
            },
            ContextBreakdownText {
                label: "seven",
                text: "1234567890123456789012345678",
            },
        ];
        let skill = [ContextBreakdownText {
            label: "skill",
            text: "visible skill text",
        }];
        let read = Tool {
            name: "read".into(),
            description: "read files".into(),
            parameters: Schema::empty_object(),
            constrained_sampling: None,
        };
        let all_parts = compute_context_breakdown(&ContextBreakdownInput {
            context_files: &parts,
            skills: &skill,
            tools: None,
            ..Default::default()
        });
        let files = category(&all_parts, ContextCategoryKey::ContextFiles);
        assert_eq!(files.parts.len(), 6);
        assert_eq!(files.tokens, 28); // Includes the seventh, undisplayed part.
        assert_eq!(
            category(&all_parts, ContextCategoryKey::Skills).tokens,
            tokens_from_utf16("visible skill text")
        );

        let no_tools = compute_context_breakdown(&ContextBreakdownInput {
            skills: &skill,
            tools: Some(&[]),
            ..Default::default()
        });
        assert_eq!(category(&no_tools, ContextCategoryKey::Skills).tokens, 0);
        let read_tools = compute_context_breakdown(&ContextBreakdownInput {
            skills: &skill,
            tools: Some(std::slice::from_ref(&read)),
            ..Default::default()
        });
        assert_eq!(
            category(&read_tools, ContextCategoryKey::Skills).tokens,
            tokens_from_utf16("visible skill text")
        );
    }

    #[test]
    fn context_breakdown_uses_compaction_projected_messages() {
        let entries = vec![
            Entry::Message(ice_harness::session::types::MessageEntry {
                base: EntryBase {
                    entry_type: "message".into(),
                    id: "old".into(),
                    seq: 1,
                    parent_id: None,
                    timestamp: 1,
                },
                message: AgentMessage::User(UserMessage::new("discarded history", 1)),
                terminate: None,
            }),
            Entry::Compaction(CompactionEntry {
                base: EntryBase {
                    entry_type: "compaction".into(),
                    id: "compact".into(),
                    seq: 2,
                    parent_id: Some("old".into()),
                    timestamp: 2,
                },
                summary: "retained summary".into(),
                retained_tail: vec![AgentMessage::User(UserMessage::new("tail", 3))],
                tokens_before: 10,
                details: None,
                usage: None,
            }),
        ];
        let context = build_session_context(&entries, &Default::default());
        let result = compute_context_breakdown(&ContextBreakdownInput {
            context: &context.messages,
            ..Default::default()
        });
        let summary = category(&result, ContextCategoryKey::Summaries);
        assert_eq!(summary.parts[0].label, "Compaction summary");
        assert_eq!(summary.tokens, tokens_from_utf16("retained summary"));
        assert!(!category(&result, ContextCategoryKey::Conversation)
            .parts
            .iter()
            .any(|part| part.label == "User messages" && part.tokens > tokens_from_utf16("tail")));
        assert!(context
            .messages
            .iter()
            .any(|message| compaction_summary_data(message).is_some()));
    }
}
