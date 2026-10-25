//! Deterministic sections and the checkpoint layout. Ports
//! `core/build-sections.ts`, `extract/{files,commits,goals,preferences}.ts`,
//! `sections.ts` and `core/format.ts` (BLACK-018..026).

use std::collections::HashSet;

use indexmap::IndexSet;

use super::brief::{compile_brief, tool_one_liner};
use super::js::{len16, prefix16};
use super::normalize::{
    clip, clip_sentence, collapse_skill_lines, extract_path, first_line, non_empty_lines, FileOps,
    NormalizedBlock,
};
use crate::blackhole::regex;

/// Deterministic section data (`SectionData`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionData {
    pub user_requests: Vec<String>,
    pub findings: Vec<String>,
    pub recent_actions: Vec<String>,
    pub session_goal: Vec<String>,
    pub outstanding_context: Vec<String>,
    pub files_and_changes: Vec<String>,
    pub commits: Vec<String>,
    pub user_preferences: Vec<String>,
    pub brief_transcript: String,
}

/// `BuildSectionsInput`.
#[derive(Debug, Clone, Default)]
pub struct BuildSectionsInput<'a> {
    pub blocks: &'a [NormalizedBlock],
    pub file_ops: Option<&'a FileOps>,
    /// Genuine user requests; `None` produces an empty request section.
    pub user_requests: Option<&'a [String]>,
}

fn ellipsize(text: &str, max: usize) -> String {
    if len16(text) > max {
        format!("{}…", prefix16(text, max - 1))
    } else {
        text.to_string()
    }
}

fn last_n<T: Clone>(items: &[T], count: usize) -> Vec<T> {
    items[items.len().saturating_sub(count)..].to_vec()
}

fn format_user_requests(requests: &[String]) -> Vec<String> {
    let cleaned: Vec<String> = requests
        .iter()
        .map(|request| ellipsize(&super::js::collapse_whitespace(request), 400))
        .filter(|request| !request.is_empty())
        .collect();
    last_n(&cleaned, 6)
}

fn extract_findings(blocks: &[NormalizedBlock]) -> Vec<String> {
    let tool_indices: HashSet<usize> = blocks
        .iter()
        .filter_map(|block| match block {
            NormalizedBlock::ToolCall { source_index, .. } => *source_index,
            _ => None,
        })
        .collect();
    let mut seen = HashSet::new();
    let mut findings = Vec::new();
    for block in blocks {
        let NormalizedBlock::Assistant { text, source_index } = block else {
            continue;
        };
        if source_index.is_some_and(|index| tool_indices.contains(&index)) {
            continue;
        }
        let item = ellipsize(&super::js::collapse_whitespace(text), 500);
        if len16(&item) < 40 {
            continue;
        }
        if seen.insert(item.clone()) {
            findings.push(item);
        }
    }
    last_n(&findings, 6)
}

fn extract_recent_actions(blocks: &[NormalizedBlock]) -> Vec<String> {
    let mut actions: Vec<(String, bool)> = Vec::new();
    for block in blocks {
        match block {
            NormalizedBlock::ToolCall { name, args, .. } if !name.trim().is_empty() => {
                let line = tool_one_liner(name, args);
                actions.push((line.trim_start_matches("* ").to_string(), false));
            }
            NormalizedBlock::ToolResult {
                name,
                is_error: true,
                ..
            } => {
                if let Some(action) = actions
                    .iter_mut()
                    .rev()
                    .find(|(line, _)| line.split_whitespace().next() == Some(name.as_str()))
                {
                    action.1 = true;
                }
            }
            _ => {}
        }
    }
    last_n(&actions, 10)
        .into_iter()
        .map(|(line, error)| {
            if error {
                format!("{line} [error]")
            } else {
                line
            }
        })
        .collect()
}

fn extract_outstanding_context(blocks: &[NormalizedBlock]) -> Vec<String> {
    let blocker = regex!(
        r"(?i)(?-u:\b)(fail(ed|s|ure|ing)?|broken|cannot|can't|won't work|does not work|doesn't work|still (broken|failing|wrong)|blocked|blocker|not (fixed|resolved|working)|crash(es|ed|ing)?)(?-u:\b)"
    );
    let mut items: Vec<String> = Vec::new();
    for block in &blocks[blocks.len().saturating_sub(20)..] {
        match block {
            NormalizedBlock::ToolResult {
                name,
                text,
                is_error: true,
                ..
            } => items.push(format!("[{name}] {}", first_line(text, 150))),
            NormalizedBlock::Assistant { text, .. } | NormalizedBlock::User { text, .. } => {
                let is_user = matches!(block, NormalizedBlock::User { .. });
                for line in non_empty_lines(text) {
                    if !blocker.is_match(&line) || len16(&line) < 15 {
                        continue;
                    }
                    if regex!(r"^\s*[-*+>]\s").is_match(&line) || regex!(r"^\s*\(").is_match(&line)
                    {
                        continue;
                    }
                    if !regex!(r#"^\s*["'`*_]?[A-Z`]"#).is_match(&line) {
                        continue;
                    }
                    let clipped = if is_user {
                        format!("[user] {}", clip_sentence(&line, 150))
                    } else {
                        clip_sentence(&line, 150)
                    };
                    if !items.contains(&clipped) {
                        items.push(clipped);
                    }
                    break;
                }
            }
            _ => {}
        }
    }
    items.truncate(5);
    items
}

// ── files ──

const FILE_READ_TOOLS: [&str; 4] = ["Read", "read", "read_file", "View"];
const FILE_WRITE_TOOLS: [&str; 7] = [
    "Edit",
    "Write",
    "edit",
    "write",
    "edit_file",
    "write_file",
    "MultiEdit",
];

/// Ordered file activity (`FileActivity`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileActivity {
    pub read: IndexSet<String>,
    pub modified: IndexSet<String>,
    pub created: IndexSet<String>,
}

/// Seed from host file operations, then add raw paths from observed calls.
pub fn extract_files(blocks: &[NormalizedBlock], file_ops: Option<&FileOps>) -> FileActivity {
    let mut activity = FileActivity::default();
    if let Some(ops) = file_ops {
        activity.read.extend(ops.read_files.iter().cloned());
        activity.modified.extend(ops.modified_files.iter().cloned());
        activity.created.extend(ops.created_files.iter().cloned());
    }
    for block in blocks {
        let NormalizedBlock::ToolCall { name, args, .. } = block else {
            continue;
        };
        let Some(path) = extract_path(args).filter(|path| !path.is_empty()) else {
            continue;
        };
        if FILE_READ_TOOLS.contains(&name.as_str()) {
            activity.read.insert(path.to_string());
        }
        if FILE_WRITE_TOOLS.contains(&name.as_str()) {
            activity.modified.insert(path.to_string());
        }
    }
    activity
}

/// `a, b (+N more)` with the first `limit` entries.
pub fn cap_list(set: &IndexSet<String>, limit: usize) -> String {
    let items: Vec<&str> = set.iter().map(String::as_str).collect();
    if items.len() <= limit {
        items.join(", ")
    } else {
        format!(
            "{} (+{} more)",
            items[..limit].join(", "),
            items.len() - limit
        )
    }
}

fn format_file_activity(blocks: &[NormalizedBlock], file_ops: Option<&FileOps>) -> Vec<String> {
    let mut activity = extract_files(blocks, file_ops);
    for path in activity.modified.clone() {
        activity.created.shift_remove(&path);
    }
    let mut lines = Vec::new();
    if !activity.modified.is_empty() {
        lines.push(format!("Modified: {}", cap_list(&activity.modified, 10)));
    }
    if !activity.created.is_empty() {
        lines.push(format!("Created: {}", cap_list(&activity.created, 10)));
    }
    if !activity.read.is_empty() {
        lines.push(format!("Read: {}", cap_list(&activity.read, 10)));
    }
    lines
}

// ── commits ──

/// An apparent commit recovered from the conversation (`CommitInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub hash: Option<String>,
    pub message: String,
}

fn extract_hash_from_output(text: &str) -> Option<String> {
    if let Some(captures) = regex!(r"\[\S+\s+([0-9a-f]{7,12})\]").captures(text) {
        return Some(captures[1].to_string());
    }
    if let Some(captures) =
        regex!(r"(?-u:\b)([0-9a-f]{7,12})\.\.([0-9a-f]{7,12})(?-u:\b)").captures(text)
    {
        return Some(captures[2].to_string());
    }
    regex!(r"(?-u:\b)([0-9a-f]{8,12})(?-u:\b)")
        .captures(text)
        .map(|captures| captures[1].to_string())
}

fn try_extract_message(command: &str) -> Option<String> {
    if !regex!(r"(?-u:\b)git\s+commit(?-u:\b)").is_match(command) {
        return None;
    }
    let captures = regex!(
        r#"git\s+commit[^\n]*?-m\s+(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'|\$?'((?:[^'\\]|\\.)*)')"#
    )
    .captures(command)?;
    let raw = captures
        .get(1)
        .or_else(|| captures.get(2))
        .or_else(|| captures.get(3))
        .map_or("", |m| m.as_str());
    let cleaned = raw.replace("\\\"", "\"").replace("\\'", "'");
    let cleaned = cleaned.trim();
    let first = regex!(r"\\n|\n")
        .split(cleaned)
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    (!first.is_empty()).then_some(first)
}

/// Commits from bash calls, local bash messages and `Ran \`cmd\`` wrappers.
pub fn extract_commits(blocks: &[NormalizedBlock]) -> Vec<CommitInfo> {
    let mut commits: Vec<CommitInfo> = Vec::new();
    let mut add = |hash: Option<String>, message: String| {
        if !commits
            .iter()
            .any(|commit| commit.hash == hash && commit.message == message)
        {
            commits.push(CommitInfo { hash, message });
        }
    };
    for (index, block) in blocks.iter().enumerate() {
        match block {
            NormalizedBlock::ToolCall { name, args, .. } if name == "bash" => {
                let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let Some(message) = try_extract_message(command) else {
                    continue;
                };
                let mut hash = None;
                for next in blocks
                    .iter()
                    .take((index + 3).min(blocks.len()))
                    .skip(index + 1)
                {
                    if let NormalizedBlock::ToolResult { text, .. } = next {
                        hash = extract_hash_from_output(text);
                        if hash.is_some() {
                            break;
                        }
                    }
                }
                add(hash, message);
            }
            NormalizedBlock::Bash {
                command, output, ..
            } => {
                if let Some(message) = try_extract_message(command) {
                    add(extract_hash_from_output(output), message);
                }
            }
            NormalizedBlock::User { text, .. } => {
                let Some(ran) = regex!(r"Ran\s+`((?:[^`\\]|\\.)*)`").captures(text) else {
                    continue;
                };
                let Some(message) = try_extract_message(&ran[1]) else {
                    continue;
                };
                let hash = regex!(r"```\n((?s:.)*?)```")
                    .captures(text)
                    .and_then(|code| extract_hash_from_output(&code[1]));
                add(hash, message);
            }
            _ => {}
        }
    }
    commits
}

pub fn format_commits(commits: &[CommitInfo], limit: usize) -> Vec<String> {
    last_n(commits, limit)
        .into_iter()
        .map(|commit| match commit.hash {
            Some(hash) => format!("{hash}: {}", commit.message),
            None => commit.message,
        })
        .collect()
}

// ── goals ──

const MAX_GOAL_CHARS: usize = 200;
const FIRST_MSG_CLIP: usize = 80;
const LEADING_CHARS: usize = 200;

fn is_substantive_goal(text: &str) -> bool {
    let trimmed = text.trim();
    let length = len16(trimmed);
    if length <= 5 || length > MAX_GOAL_CHARS {
        return false;
    }
    let noise_short =
        regex!(r"(?i)^(ok|yes|no|sure|yeah|yep|go|hi|hey|thx|thanks|ok(?-u:\b).*|y|n|k)\s*[.!?]*$");
    let non_goal = regex!(
        r"^\s*[\[│├└─╭╰]|```|^\s*(=[A-Z]+\(|function |const |let |var |import |export |class )|^(https?:|file:|/[A-Za-z])|\\n|^\s*For each(?-u:\b)|(?-u:\b)in full(?-u:\b)[^\n]*(?-u:\b)(comments|issue|issues|PRs?|linked)(?-u:\b)"
    );
    !noise_short.is_match(trimmed) && !non_goal.is_match(trimmed)
}

fn strip_leading_bullet(line: &str) -> String {
    regex!(r"^\s*(?:[-*+]|\d+\.)\s+")
        .replace(line, "")
        .trim()
        .to_string()
}

fn index_suffix(source_index: Option<usize>) -> String {
    source_index
        .map(|index| format!(" (#{index})"))
        .unwrap_or_default()
}

/// The initial objective plus only the latest scope change (`extractGoals`).
pub fn extract_goals(blocks: &[NormalizedBlock]) -> Vec<String> {
    let scope_change = regex!(
        r"(?i)(?-u:\b)(instead|actually|change of plan|forget that|new task|switch to|now I want|pivot|let'?s do|stop .* and)(?-u:\b)"
    );
    let task = regex!(
        r"(?i)(?-u:\b)(fix|implement|add|create|build|refactor|debug|investigate|update|remove|delete|migrate|deploy|test|write|set up)(?-u:\b)"
    );
    let template = regex!(
        r"(?i)^\s*(For each(?-u:\b)|Do NOT implement(?-u:\b)|Analyze and propose(?-u:\b)|If Task/context(?-u:\b)|Output:\s*$)"
    );
    let mut goals: Vec<String> = Vec::new();
    let mut latest_scope: Option<Vec<String>> = None;
    let mut latest_scope_index: Option<usize> = None;

    for block in blocks {
        let NormalizedBlock::User { text, source_index } = block else {
            continue;
        };
        let raw_lines = non_empty_lines(text);
        let truncated = match raw_lines.iter().position(|line| template.is_match(line)) {
            Some(index) => raw_lines[..index].to_vec(),
            None => raw_lines,
        };
        let lines: Vec<String> = collapse_skill_lines(&truncated)
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    clip(&line, MAX_GOAL_CHARS)
                } else {
                    line
                }
            })
            .filter(|line| is_substantive_goal(line))
            .map(|line| strip_leading_bullet(&line))
            .filter(|line| len16(line) > 5)
            .collect();
        if lines.is_empty() {
            continue;
        }
        let leading = prefix16(text, LEADING_CHARS);
        if goals.is_empty() && latest_scope.is_none() {
            if scope_change.is_match(leading) {
                latest_scope = Some(
                    lines
                        .iter()
                        .take(3)
                        .map(|line| clip(line, MAX_GOAL_CHARS))
                        .collect(),
                );
                latest_scope_index = *source_index;
                continue;
            }
            goals.extend(lines.iter().take(6).map(|line| {
                format!(
                    "{}{}",
                    clip(line, FIRST_MSG_CLIP),
                    index_suffix(*source_index)
                )
            }));
            continue;
        }
        if scope_change.is_match(leading) {
            latest_scope = Some(
                lines
                    .iter()
                    .take(3)
                    .map(|line| clip(line, MAX_GOAL_CHARS))
                    .collect(),
            );
            latest_scope_index = *source_index;
        } else if task.is_match(leading) && len16(&lines[0]) > 15 {
            latest_scope = Some(
                lines
                    .iter()
                    .take(2)
                    .map(|line| clip(line, MAX_GOAL_CHARS))
                    .collect(),
            );
            latest_scope_index = *source_index;
        }
    }

    if let Some(scope) = latest_scope.filter(|scope| !scope.is_empty()) {
        let mut items = vec!["[Scope change]".to_string()];
        items.extend(
            scope
                .into_iter()
                .map(|line| format!("{line}{}", index_suffix(latest_scope_index))),
        );
        if goals.len() + items.len() > 8 {
            goals.truncate(8usize.saturating_sub(items.len()));
        }
        goals.extend(items);
    }
    goals.truncate(8);
    if goals.last().map(String::as_str) == Some("[Scope change]") {
        goals.pop();
    }
    goals
}

// ── preferences ──

fn is_preference(line: &str) -> bool {
    let patterns = [
        regex!(r"(?i)(?-u:\b)prefer(?:s|red|ring)?\s+[A-Za-z0-9_]"),
        regex!(r"(?i)(?-u:\b)don'?t want(?-u:\b)"),
        regex!(
            r"(?i)(?-u:\b)always (?:use|do|run|prefer|keep|make|format|write|add|set|put|prefix|start|include|append)(?-u:\b)"
        ),
        regex!(
            r"(?i)(?-u:\b)never (?:use|do|run|push|commit|write|ignore|add|set|put|remove|delete|include|deploy)(?-u:\b)"
        ),
        regex!(r"(?i)(?-u:\b)please (?:use|avoid|keep|make|don'?t|do not|format|write)(?-u:\b)"),
        regex!(r"(?i)(?-u:\b)(?:style|format|language|naming)\s*[:=]\s*\S"),
    ];
    patterns.iter().any(|pattern| pattern.is_match(line))
}

/// At most one explicit preference per user block; the first 10 overall.
pub fn extract_preferences(blocks: &[NormalizedBlock]) -> Vec<String> {
    let mut prefs = Vec::new();
    let mut seen = HashSet::new();
    for block in blocks {
        let NormalizedBlock::User { text, .. } = block else {
            continue;
        };
        for line in non_empty_lines(text) {
            let trimmed = line.trim();
            let length = len16(trimmed);
            if !(5..=200).contains(&length) {
                continue;
            }
            if trimmed.ends_with('?') || trimmed.contains("?...") {
                continue;
            }
            if !is_preference(trimmed) {
                continue;
            }
            let clipped = clip(trimmed, 200);
            if !seen.insert(clipped.to_lowercase()) {
                continue;
            }
            prefs.push(clipped);
            break;
        }
    }
    prefs.truncate(10);
    prefs
}

pub fn dedup_preferences_against_goals(prefs: Vec<String>, goals: &[String]) -> Vec<String> {
    let normalize = |text: &str| text.trim().to_lowercase();
    let goal_set: HashSet<String> = goals.iter().map(|goal| normalize(goal)).collect();
    prefs
        .into_iter()
        .filter(|pref| !goal_set.contains(&normalize(pref)))
        .collect()
}

// ── build ──

pub fn build_sections(input: BuildSectionsInput<'_>) -> SectionData {
    let blocks = input.blocks;
    let user_requests = input
        .user_requests
        .map(format_user_requests)
        .unwrap_or_default();
    let goal_blocks: Vec<NormalizedBlock>;
    let goal_source: &[NormalizedBlock] = match input.user_requests {
        Some(requests) => {
            goal_blocks = requests
                .iter()
                .map(|text| NormalizedBlock::User {
                    text: text.clone(),
                    source_index: None,
                })
                .collect();
            &goal_blocks
        }
        None => blocks,
    };
    let session_goal = extract_goals(goal_source);
    let user_preferences =
        dedup_preferences_against_goals(extract_preferences(blocks), &session_goal);
    SectionData {
        user_requests,
        findings: extract_findings(blocks),
        recent_actions: extract_recent_actions(blocks),
        session_goal,
        outstanding_context: extract_outstanding_context(blocks),
        files_and_changes: format_file_activity(blocks, input.file_ops),
        commits: format_commits(&extract_commits(blocks), 8),
        user_preferences,
        brief_transcript: compile_brief(blocks),
    }
}

// ── format ──

const BRIEF_MAX_LINES: usize = 120;
const TUI_SAFE_LINE_CHARS: usize = 120;

fn section(title: &str, items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let body = items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("[{title}]\n{body}")
}

fn wrap_line_with_continuation(line: &str, max_chars: usize) -> Vec<String> {
    let indent = regex!(r"^\s*(?:[-*]\s+|\d+\.\s+)?")
        .find(line)
        .map_or("", |m| m.as_str());
    let continuation = if indent.is_empty() {
        String::new()
    } else {
        " ".repeat(len16(indent).min(8))
    };
    let safe_max = max_chars.saturating_sub(continuation.len());
    let wrapped = ice_tui::utils::wrap_text_with_ansi(line, safe_max);
    if wrapped.len() <= 1 || continuation.is_empty() {
        return wrapped;
    }
    let mut out = vec![wrapped[0].clone()];
    out.extend(
        wrapped[1..]
            .iter()
            .map(|row| format!("{continuation}{row}")),
    );
    out
}

/// Wrap every line to 120 terminal columns, keeping list continuations aligned.
pub fn wrap_long_lines(text: &str) -> String {
    text.split('\n')
        .flat_map(|line| wrap_line_with_continuation(line, TUI_SAFE_LINE_CHARS))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keep the last 120 lines, starting at a bracketed header when possible.
pub fn cap_brief(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() <= BRIEF_MAX_LINES {
        return text.to_string();
    }
    let kept = &lines[lines.len() - BRIEF_MAX_LINES..];
    let header = regex!(r"^\[.+\]");
    let mut first_header = kept.iter().position(|line| header.is_match(line));
    if first_header.is_none() {
        let anchor = regex!(r"^\[[^\]]+\]");
        if let Some(index) = kept.iter().position(|line| anchor.is_match(line)) {
            if index > 0 {
                first_header = Some(index);
            }
        }
    }
    let clean = match first_header {
        Some(index) if index > 0 => &kept[index..],
        _ => kept,
    };
    let omitted = lines.len() - clean.len();
    format!(
        "...({omitted} earlier lines omitted)\n\n{}",
        clean.join("\n")
    )
}

pub fn format_summary(data: &SectionData) -> String {
    let header_parts: Vec<String> = [
        section("User Requests", &data.user_requests),
        section("Findings", &data.findings),
        section("Last Actions", &data.recent_actions),
        section("Session Goal", &data.session_goal),
        section("Files And Changes", &data.files_and_changes),
        section("Commits", &data.commits),
        section("Outstanding Context", &data.outstanding_context),
        section("User Preferences", &data.user_preferences),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect();
    let mut parts = Vec::new();
    if !header_parts.is_empty() {
        parts.push(header_parts.join("\n\n"));
    }
    if !data.brief_transcript.is_empty() {
        parts.push(cap_brief(&data.brief_transcript));
    }
    if parts.is_empty() {
        return String::new();
    }
    wrap_long_lines(&parts.join("\n\n---\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str, index: usize) -> NormalizedBlock {
        NormalizedBlock::User {
            text: text.into(),
            source_index: Some(index),
        }
    }

    fn assistant(text: &str, index: usize) -> NormalizedBlock {
        NormalizedBlock::Assistant {
            text: text.into(),
            source_index: Some(index),
        }
    }

    fn call(name: &str, args: serde_json::Value, index: usize) -> NormalizedBlock {
        NormalizedBlock::ToolCall {
            name: name.into(),
            args,
            source_index: Some(index),
        }
    }

    #[test]
    fn regex_ascii_word_boundaries_compile() {
        assert!(regex!(r"(?-u:\b)git(?-u:\b)").is_match("run git now"));
    }

    #[test]
    fn user_requests_keep_the_last_six_and_clip_long_items() {
        let requests: Vec<String> = (0..8).map(|i| format!("request  {i}")).collect();
        let out = format_user_requests(&requests);
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], "request 2");
        let long = format_user_requests(&["x".repeat(500)]);
        assert_eq!(len16(&long[0]), 400);
        assert!(long[0].ends_with('…'));
    }

    #[test]
    fn findings_skip_short_and_tool_call_prose() {
        let long = "The parser fails because the tokenizer drops the final byte.";
        let blocks = vec![
            assistant("short", 0),
            assistant(long, 1),
            assistant(
                "This sentence is long enough to count but shares a tool message.",
                2,
            ),
            call("read", json!({"path":"a"}), 2),
            assistant(long, 3),
        ];
        assert_eq!(extract_findings(&blocks), vec![long.to_string()]);
    }

    #[test]
    fn recent_actions_mark_the_latest_matching_call() {
        let blocks = vec![
            call("bash", json!({"command":"make"}), 0),
            call("read", json!({"path":"a"}), 0),
            NormalizedBlock::ToolResult {
                name: "bash".into(),
                text: "boom".into(),
                is_error: true,
                source_index: Some(1),
            },
        ];
        assert_eq!(
            extract_recent_actions(&blocks),
            vec!["bash \"make\" [error]", "read \"a\""]
        );
    }

    #[test]
    fn outstanding_context_keeps_sentence_like_blockers() {
        let blocks = vec![
            assistant(
                "- the build fails sometimes\nThe build still failing on CI. More text",
                0,
            ),
            user("It does not work at all on Windows", 1),
            NormalizedBlock::ToolResult {
                name: "bash".into(),
                text: "error: x\nmore".into(),
                is_error: true,
                source_index: Some(2),
            },
        ];
        assert_eq!(
            extract_outstanding_context(&blocks),
            vec![
                "The build still failing on CI. More text",
                "[user] It does not work at all on Windows",
                "[bash] error: x"
            ]
        );
    }

    #[test]
    fn file_activity_orders_categories_and_dedups_created() {
        let ops = FileOps {
            read_files: vec!["r.rs".into()],
            modified_files: vec![],
            created_files: vec!["new.rs".into(), "w.rs".into()],
        };
        let blocks = vec![
            call("write", json!({"path":"w.rs"}), 0),
            call("Read", json!({"file_path":"x.rs"}), 0),
        ];
        assert_eq!(
            format_file_activity(&blocks, Some(&ops)),
            vec!["Modified: w.rs", "Created: new.rs", "Read: r.rs, x.rs"]
        );
    }

    #[test]
    fn commits_come_from_calls_bash_and_wrappers() {
        let blocks = vec![
            call(
                "bash",
                json!({"command":"git commit -m \"feat: add x\\nbody\""}),
                0,
            ),
            NormalizedBlock::ToolResult {
                name: "bash".into(),
                text: "[main abc1234] feat: add x".into(),
                is_error: false,
                source_index: Some(1),
            },
            user(
                "Ran `git commit -m 'fix: y'`\n```\n[dev 1234567890] fix\n```",
                2,
            ),
            NormalizedBlock::Bash {
                command: "git commit -m \"chore: z\"".into(),
                output: "a1b2c3d..e4f5a6b7c".into(),
                exit_code: Some(0.0),
                source_index: Some(3),
            },
        ];
        assert_eq!(
            format_commits(&extract_commits(&blocks), 8),
            vec![
                "abc1234: feat: add x",
                "1234567890: fix: y",
                "e4f5a6b7c: chore: z"
            ]
        );
    }

    #[test]
    fn goals_keep_original_and_latest_scope_change() {
        let blocks = vec![
            user("Implement the parser for config files", 0),
            user("ok", 1),
            user("Actually switch to the YAML loader instead", 2),
        ];
        assert_eq!(
            extract_goals(&blocks),
            vec![
                "Implement the parser for config files (#0)",
                "[Scope change]",
                "Actually switch to the YAML loader instead (#2)"
            ]
        );
        let blocks = vec![user("Fix bug\nFor each issue:\nRead the issue in full", 0)];
        assert_eq!(extract_goals(&blocks), vec!["Fix bug (#0)"]);
    }

    #[test]
    fn preferences_take_one_per_block_and_skip_questions() {
        let blocks = vec![
            user("Please use tabs here\nAlways run cargo fmt", 0),
            user("do you prefer spaces?", 1),
            user("never push to main", 2),
        ];
        assert_eq!(
            extract_preferences(&blocks),
            vec!["Please use tabs here", "never push to main"]
        );
        assert_eq!(
            dedup_preferences_against_goals(vec!["A".into(), "b".into()], &[" a ".into()]),
            vec!["b"]
        );
    }

    #[test]
    fn cap_brief_keeps_tail_from_a_header() {
        let mut lines: Vec<String> = (0..130).map(|i| format!("line {i}")).collect();
        lines[20] = "[user]".into();
        let capped = cap_brief(&lines.join("\n"));
        assert!(capped.starts_with("...(20 earlier lines omitted)\n\n[user]"));
        lines[15] = "[assistant]".into();
        let capped = cap_brief(&lines.join("\n"));
        assert!(capped.starts_with("...(15 earlier lines omitted)\n\n[assistant]"));
        let plain: Vec<String> = (0..130).map(|i| format!("line {i}")).collect();
        assert!(
            cap_brief(&plain.join("\n")).starts_with("...(10 earlier lines omitted)\n\nline 10")
        );
    }

    #[test]
    fn format_summary_orders_sections_and_wraps() {
        let data = SectionData {
            recent_actions: vec!["read \"a\"".into()],
            user_requests: vec!["do it".into()],
            brief_transcript: "[user]\ndo it (#0)".into(),
            files_and_changes: vec![format!("Read: {}", "path/".repeat(40))],
            ..SectionData::default()
        };
        let text = format_summary(&data);
        assert!(text.starts_with("[User Requests]\n- do it\n\n[Last Actions]\n- read \"a\""));
        assert!(text.contains("\n\n---\n\n[user]\ndo it (#0)"));
        assert!(text.lines().all(|line| ice_tui::visible_width(line) <= 120));
    }
}
