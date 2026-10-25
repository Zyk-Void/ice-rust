//! Message normalization and the shared text/argument helpers.
//!
//! Ports `types.ts`, `core/normalize.ts`, `core/sanitize.ts`,
//! `core/filter-noise.ts`, `core/content.ts`, `core/tool-args.ts` and
//! `core/skill-collapse.ts` (BLACK-007..012).

use ice_ai::types::{Content, Message, UserContent};
use serde_json::Value;

use super::js::{last_index_of16, len16, prefix16, slice16};
use crate::blackhole::regex;

/// Host file-operation seeds (`FileOps`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileOps {
    pub read_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub created_files: Vec<String>,
}

/// One normalized compiler input (`NormalizedBlock`).
#[derive(Debug, Clone, PartialEq)]
pub enum NormalizedBlock {
    User {
        text: String,
        source_index: Option<usize>,
    },
    Assistant {
        text: String,
        source_index: Option<usize>,
    },
    ToolCall {
        name: String,
        args: Value,
        source_index: Option<usize>,
    },
    ToolResult {
        name: String,
        text: String,
        is_error: bool,
        source_index: Option<usize>,
    },
    Bash {
        command: String,
        output: String,
        exit_code: Option<f64>,
        source_index: Option<usize>,
    },
    Thinking {
        text: String,
        redacted: bool,
        source_index: Option<usize>,
    },
}

impl NormalizedBlock {
    pub fn source_index(&self) -> Option<usize> {
        match self {
            Self::User { source_index, .. }
            | Self::Assistant { source_index, .. }
            | Self::ToolCall { source_index, .. }
            | Self::ToolResult { source_index, .. }
            | Self::Bash { source_index, .. }
            | Self::Thinking { source_index, .. } => *source_index,
        }
    }
}

// ── sanitize ──

/// Normalize newlines and strip CSI escapes and C0 controls other than TAB/LF.
pub fn sanitize(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = regex!(r"\x1b\[[0-9;]*[A-Za-z]").replace_all(&text, "");
    regex!(r"[\x00-\x08\x0b\x0c\x0e-\x1f]")
        .replace_all(&text, "")
        .into_owned()
}

// ── content helpers ──

/// Text parts of a content array, ignoring images, thinking and tool calls.
pub fn text_parts(content: &[Content]) -> Vec<&str> {
    content
        .iter()
        .filter_map(|block| match block {
            Content::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

pub fn text_of(content: &[Content]) -> String {
    text_parts(content).join("\n")
}

pub fn user_text_of(content: &UserContent) -> String {
    match content {
        UserContent::Text(text) => text.clone(),
        UserContent::Blocks(blocks) => text_of(blocks),
    }
}

/// Clip to `max` UTF-16 units, preferring a late word boundary.
pub fn clip(text: &str, max: usize) -> String {
    if len16(text) <= max {
        return text.to_string();
    }
    let cut = last_index_of16(text, ' ', max);
    let end = match cut {
        Some(cut) if cut as f64 > max as f64 * 0.6 => cut,
        _ => max,
    };
    // `prefix16` already avoids splitting a surrogate pair.
    prefix16(text, end).to_string()
}

/// Clip to the last sentence boundary when it lies in the second half of the
/// window; otherwise fall back to [`clip`].
pub fn clip_sentence(text: &str, max: usize) -> String {
    if len16(text) <= max {
        return text.to_string();
    }
    let window = prefix16(text, max);
    if let Some(last) = regex!(r"[.!?](?:\s|$)").find_iter(window).last() {
        let end = len16(&window[..last.start()]) + 1;
        if end as f64 >= max as f64 * 0.5 {
            return prefix16(text, end).to_string();
        }
    }
    clip(text, max)
}

pub fn non_empty_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// `clip(text.split("\n")[0], max)`.
pub fn first_line(text: &str, max: usize) -> String {
    clip(text.split('\n').next().unwrap_or(""), max)
}

/// First case-insensitive occurrence of `term` with `radius` units of context.
pub fn snippet(text: &str, term: &str, radius: usize) -> Option<String> {
    let lower_text = text.to_lowercase();
    let lower_term = term.to_lowercase();
    let byte = lower_text.find(&lower_term)?;
    let index = len16(&lower_text[..byte]);
    let total = len16(text);
    let start = index.saturating_sub(radius);
    let end = (index + len16(term) + radius).min(total);
    let prefix = if start > 0 { "..." } else { "" };
    let suffix = if end < total { "..." } else { "" };
    Some(format!("{prefix}{}{suffix}", slice16(text, start, end)))
}

// ── tool arguments ──

/// Canonical list of path-like argument keys across tools.
pub const PATH_KEYS: [&str; 4] = ["path", "file_path", "filePath", "file"];

/// First string value among [`PATH_KEYS`], even when empty.
pub fn extract_path(args: &Value) -> Option<&str> {
    let object = args.as_object()?;
    PATH_KEYS
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

pub fn summarize_tool_args(args: &Value) -> String {
    let Some(object) = args.as_object() else {
        return String::new();
    };
    if let Some(path) = extract_path(args).filter(|path| !path.is_empty()) {
        return format!("path={path}");
    }
    if let Some(command) = object.get("command").and_then(Value::as_str) {
        return format!("command={command}");
    }
    if let Some(query) = object.get("query").and_then(Value::as_str) {
        return format!("query={query}");
    }
    object.keys().cloned().collect::<Vec<_>>().join(", ")
}

fn nonempty_str<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// A call is content-bearing when it names a path and carries content,
/// a nonempty edits array of objects, or old/new text without `edits`.
pub fn is_content_bearing(args: &Value) -> bool {
    let Some(object) = args.as_object() else {
        return false;
    };
    if !PATH_KEYS
        .iter()
        .any(|key| object.get(*key).is_some_and(Value::is_string))
    {
        return false;
    }
    if nonempty_str(object, "content").is_some() {
        return true;
    }
    if let Some(edits) = object.get("edits").and_then(Value::as_array) {
        if !edits.is_empty() && edits.iter().all(Value::is_object) {
            return true;
        }
    }
    let no_edits = !object.contains_key("edits");
    no_edits
        && (nonempty_str(object, "oldText").is_some() || nonempty_str(object, "newText").is_some())
}

/// Bounded text recovered from content-bearing tool calls. The cap counts
/// UTF-16 units despite the TypeScript `maxBytesPerCall` name.
pub fn tool_call_args_text(content: &[Content], max_per_call: usize) -> String {
    let mut parts = Vec::new();
    let half = max_per_call / 2;
    for block in content {
        let Content::ToolCall(call) = block else {
            continue;
        };
        if !is_content_bearing(&call.arguments) {
            continue;
        }
        let object = call
            .arguments
            .as_object()
            .expect("content-bearing args are an object");
        let mut extracted = String::new();
        if let Some(text) = object.get("content").and_then(Value::as_str) {
            extracted.push_str(prefix16(text, max_per_call));
            extracted.push('\n');
        }
        let edits = object.get("edits").and_then(Value::as_array);
        if let Some(edits) = edits {
            for edit in edits {
                if len16(&extracted) >= max_per_call {
                    break;
                }
                let Some(edit) = edit.as_object() else {
                    continue;
                };
                if let Some(old) = edit.get("oldText").and_then(Value::as_str) {
                    extracted.push_str(prefix16(old, half));
                    extracted.push('\n');
                }
                if len16(&extracted) >= max_per_call {
                    break;
                }
                if let Some(new) = edit.get("newText").and_then(Value::as_str) {
                    extracted.push_str(prefix16(new, half));
                    extracted.push('\n');
                }
            }
        }
        if edits.is_none() {
            if let Some(old) = object.get("oldText").and_then(Value::as_str) {
                extracted.push_str(prefix16(old, max_per_call));
                extracted.push('\n');
            }
            if let Some(new) = object.get("newText").and_then(Value::as_str) {
                extracted.push_str(prefix16(new, max_per_call));
                extracted.push('\n');
            }
        }
        if !extracted.is_empty() {
            parts.push(prefix16(&extracted, max_per_call).to_string());
        }
    }
    parts.join("\n")
}

// ── skill collapse ──

/// Collapse skill blocks in a line list: one `[skill: name]` per name, drop
/// every line inside a block.
pub fn collapse_skill_lines(lines: &[String]) -> Vec<String> {
    let open = regex!(r#"^-?\s*<skill\s+name="([^"]+)""#);
    let close = regex!(r"^-?\s*</skill>");
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut inside = false;
    for line in lines {
        if let Some(captures) = open.captures(line) {
            inside = true;
            let name = captures[1].to_string();
            if seen.insert(name.clone()) {
                result.push(format!("[skill: {name}]"));
            }
            continue;
        }
        if inside {
            if close.is_match(line) {
                inside = false;
            }
            continue;
        }
        result.push(line.clone());
    }
    result
}

/// Replace each `<skill name="X" ...>...</skill>` block (or an unterminated
/// one through the end of input) with `[skill: X]`.
pub fn collapse_skill_text(text: &str) -> String {
    regex!(r#"<skill\s+name="([^"]+)"[^>]*>(?s:.)*?(?:</skill>|$)"#)
        .replace_all(text, |captures: &regex::Captures<'_>| {
            format!("[skill: {}]", &captures[1])
        })
        .into_owned()
}

// ── normalize ──

fn normalize_one(message: &Message, index: usize) -> Vec<NormalizedBlock> {
    let source_index = Some(index);
    match message {
        Message::User(user) => {
            let mut blocks = Vec::new();
            let text = sanitize(&user_text_of(&user.content));
            if !text.is_empty() {
                blocks.push(NormalizedBlock::User { text, source_index });
            }
            if let UserContent::Blocks(parts) = &user.content {
                for part in parts {
                    if let Content::Image(image) = part {
                        blocks.push(NormalizedBlock::User {
                            text: format!("[image: {}]", image.mime_type),
                            source_index,
                        });
                    }
                }
            }
            if blocks.is_empty() {
                blocks.push(NormalizedBlock::User {
                    text: String::new(),
                    source_index,
                });
            }
            blocks
        }
        Message::ToolResult(result) => vec![NormalizedBlock::ToolResult {
            name: result.tool_name.clone(),
            text: sanitize(&text_of(&result.content)),
            is_error: result.is_error,
            source_index,
        }],
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .filter_map(|part| match part {
                Content::Text(text) => Some(NormalizedBlock::Assistant {
                    text: sanitize(&text.text),
                    source_index,
                }),
                Content::Thinking(thinking) => Some(NormalizedBlock::Thinking {
                    text: sanitize(&thinking.thinking),
                    redacted: thinking.redacted,
                    source_index,
                }),
                Content::ToolCall(call) => Some(NormalizedBlock::ToolCall {
                    name: call.name.clone(),
                    args: call.arguments.clone(),
                    source_index,
                }),
                Content::Image(_) => None,
            })
            .collect(),
    }
}

/// Convert LLM messages into normalized blocks; `source_index` is the
/// zero-based position of the input message.
pub fn normalize(messages: &[Message]) -> Vec<NormalizedBlock> {
    messages
        .iter()
        .enumerate()
        .flat_map(|(index, message)| normalize_one(message, index))
        .collect()
}

// ── noise filter ──

const NOISE_TOOLS: [&str; 7] = [
    "TodoWrite",
    "TodoRead",
    "ToolSearch",
    "WebSearch",
    "AskUser",
    "ExitSpecMode",
    "GenerateDroid",
];

const NOISE_STRINGS: [&str; 3] = [
    "Continue from where you left off.",
    "No response requested.",
    "IMPORTANT: TodoWrite was not called yet.",
];

fn clean_or_none(text: &str) -> Option<String> {
    // One alternative per wrapper tag stands in for the TypeScript backreference.
    let wrapper = regex!(
        r"<system-reminder[^>]*>(?s:.)*?</system-reminder>|<ide_opened_file[^>]*>(?s:.)*?</ide_opened_file>|<command-message[^>]*>(?s:.)*?</command-message>|<context-window-usage[^>]*>(?s:.)*?</context-window-usage>"
    );
    let cleaned = wrapper.replace_all(text, "").trim().to_string();
    if cleaned.is_empty() {
        return None;
    }
    if NOISE_STRINGS.iter().any(|noise| cleaned.contains(noise)) {
        let remaining = NOISE_STRINGS.iter().fold(cleaned, |value, noise| {
            value.replace(noise, "").trim().to_string()
        });
        return (!remaining.is_empty()).then_some(remaining);
    }
    Some(cleaned)
}

/// Drop thinking, known noisy tools and wrapper-only user text.
pub fn filter_noise(blocks: Vec<NormalizedBlock>) -> Vec<NormalizedBlock> {
    let mut out = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block {
            NormalizedBlock::Thinking { .. } => continue,
            NormalizedBlock::ToolCall { ref name, .. }
            | NormalizedBlock::ToolResult { ref name, .. }
                if NOISE_TOOLS.contains(&name.as_str()) =>
            {
                continue
            }
            NormalizedBlock::User { text, source_index } => {
                if let Some(text) = clean_or_none(&text) {
                    out.push(NormalizedBlock::User { text, source_index });
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::types::{
        Api, AssistantMessage, ImageContent, ImageContentType, TextContent, TextContentType,
        ThinkingContent, ToolCall, ToolCallType, ToolResultMessage, UserMessage,
    };
    use serde_json::json;

    pub(crate) fn assistant(content: Vec<Content>) -> Message {
        let mut message = AssistantMessage::empty(Api::AnthropicMessages, "p", "m", 0);
        message.content = content;
        Message::Assistant(Box::new(message))
    }

    pub(crate) fn text_block(text: &str) -> Content {
        Content::Text(TextContent {
            kind: TextContentType,
            text: text.into(),
            text_signature: None,
        })
    }

    pub(crate) fn call(name: &str, args: Value) -> Content {
        Content::ToolCall(ToolCall {
            kind: ToolCallType,
            id: format!("call-{name}"),
            name: name.into(),
            arguments: args,
            thought_signature: None,
            namespace: None,
        })
    }

    pub(crate) fn tool_result(name: &str, text: &str, is_error: bool) -> Message {
        Message::ToolResult(Box::new(ToolResultMessage {
            role: Default::default(),
            tool_call_id: format!("call-{name}"),
            tool_name: name.into(),
            content: vec![text_block(text)],
            details: None,
            usage: None,
            added_tool_names: Vec::new(),
            is_error,
            timestamp: 0,
        }))
    }

    #[test]
    fn sanitize_strips_csi_and_controls_but_keeps_tab_and_newline() {
        assert_eq!(
            sanitize("a\r\nb\rc\x1b[31mred\x1b[0m\x07\td"),
            "a\nb\ncred\td"
        );
    }

    #[test]
    fn normalize_maps_roles_and_source_indices() {
        let image = Message::User(UserMessage::new(
            UserContent::Blocks(vec![
                text_block("look"),
                Content::Image(ImageContent {
                    kind: ImageContentType,
                    data: String::new(),
                    mime_type: "image/png".into(),
                }),
            ]),
            0,
        ));
        let thinking = Content::Thinking(ThinkingContent {
            kind: Default::default(),
            thinking: "hmm".into(),
            thinking_signature: None,
            redacted: true,
        });
        let blocks = normalize(&[
            image,
            assistant(vec![
                thinking,
                text_block("ok"),
                call("read", json!({"path":"a"})),
            ]),
            tool_result("read", "data", false),
            Message::User(UserMessage::new(UserContent::Blocks(vec![]), 0)),
        ]);
        assert_eq!(
            blocks[0],
            NormalizedBlock::User {
                text: "look".into(),
                source_index: Some(0)
            }
        );
        assert_eq!(
            blocks[1],
            NormalizedBlock::User {
                text: "[image: image/png]".into(),
                source_index: Some(0)
            }
        );
        assert!(matches!(
            blocks[2],
            NormalizedBlock::Thinking { redacted: true, .. }
        ));
        assert!(matches!(
            blocks[4],
            NormalizedBlock::ToolCall {
                source_index: Some(1),
                ..
            }
        ));
        assert!(matches!(
            blocks[5],
            NormalizedBlock::ToolResult {
                source_index: Some(2),
                ..
            }
        ));
        assert_eq!(
            blocks[6],
            NormalizedBlock::User {
                text: String::new(),
                source_index: Some(3)
            }
        );
    }

    #[test]
    fn filter_noise_drops_thinking_noise_tools_and_wrappers() {
        let blocks = vec![
            NormalizedBlock::Thinking {
                text: "x".into(),
                redacted: false,
                source_index: None,
            },
            NormalizedBlock::ToolCall {
                name: "TodoWrite".into(),
                args: json!({}),
                source_index: None,
            },
            NormalizedBlock::User {
                text: "<system-reminder>hi</system-reminder>".into(),
                source_index: Some(1),
            },
            NormalizedBlock::User {
                text: "Fix it. Continue from where you left off.".into(),
                source_index: Some(2),
            },
            NormalizedBlock::User {
                text: "No response requested.".into(),
                source_index: Some(3),
            },
        ];
        let out = filter_noise(blocks);
        assert_eq!(
            out,
            vec![NormalizedBlock::User {
                text: "Fix it.".into(),
                source_index: Some(2)
            }]
        );
    }

    #[test]
    fn clip_prefers_late_spaces_and_clip_sentence_prefers_sentences() {
        let text = "word ".repeat(60);
        let clipped = clip(&text, 200);
        assert!(len16(&clipped) <= 200);
        assert!(!clipped.ends_with(' '));
        assert_eq!(clip("abcdefghij", 5), "abcde");
        let sentence = format!("{}. {}", "a".repeat(120), "b".repeat(200));
        assert_eq!(
            clip_sentence(&sentence, 200),
            format!("{}.", "a".repeat(120))
        );
    }

    #[test]
    fn snippet_adds_ellipses_at_clipped_edges() {
        let text = format!("{}needle{}", "x".repeat(100), "y".repeat(100));
        let found = snippet(&text, "NEEDLE", 10).unwrap();
        assert_eq!(
            found,
            format!("...{}needle{}...", "x".repeat(10), "y".repeat(10))
        );
        assert_eq!(snippet("abc", "z", 5), None);
    }

    #[test]
    fn tool_args_helpers_follow_alias_priority() {
        assert_eq!(extract_path(&json!({"file":"b","path":"a"})), Some("a"));
        assert_eq!(extract_path(&json!({"path":""})), Some(""));
        assert_eq!(summarize_tool_args(&json!({"path":"","x":2})), "path, x");
        assert_eq!(summarize_tool_args(&json!({"command":"ls"})), "command=ls");
        assert!(is_content_bearing(&json!({"path":"a","content":"x"})));
        assert!(!is_content_bearing(&json!({"path":"a","edits":[]})));
        assert!(is_content_bearing(&json!({"path":"a","edits":[{}]})));
        assert!(!is_content_bearing(
            &json!({"path":"a","oldText":"x","edits":null})
        ));
        assert!(is_content_bearing(&json!({"path":"a","newText":"x"})));
    }

    #[test]
    fn tool_call_args_text_caps_each_call() {
        let content = vec![
            call("write", json!({"path":"a","content":"x".repeat(50)})),
            call(
                "edit",
                json!({"path":"b","edits":[{"oldText":"old","newText":"new"}]}),
            ),
            call("read", json!({"path":"c"})),
        ];
        let text = tool_call_args_text(&content, 20);
        assert_eq!(text, format!("{}\nold\nnew\n", "x".repeat(20)));
    }

    #[test]
    fn skill_collapse_variants() {
        let lines: Vec<String> = [
            "<skill name=\"a\" x>",
            "body",
            "</skill>",
            "keep",
            "- <skill name=\"a\">",
            "</skill>",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(collapse_skill_lines(&lines), vec!["[skill: a]", "keep"]);
        assert_eq!(
            collapse_skill_text("x <skill name=\"s\">body</skill> y <skill name=\"s\">tail"),
            "x [skill: s] y [skill: s]"
        );
    }
}
