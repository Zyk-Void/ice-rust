//! The brief transcript: word budgets, bash digests, tool one-liners,
//! section grouping and the reverse parser. Ports `core/brief.ts`
//! (BLACK-013..017).
//!
//! Word segmentation uses Unicode UAX #29 boundaries (`unicode-segmentation`)
//! where the TypeScript code uses `Intl.Segmenter`; both follow the same
//! default word-boundary rules, but locale tailoring can differ.

use std::collections::HashSet;

use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

use super::js::{last_index_of16, len16, prefix16};
use super::normalize::{
    clip, collapse_skill_text, extract_path, first_line, sanitize, single_line, NormalizedBlock,
};
use crate::blackhole::regex;

const TRUNCATE_USER: usize = 256;
const TRUNCATE_ASSISTANT: usize = 200;
const BASH_CAP: usize = 120;
const TOOL_CALLS_PER_TURN: usize = 4;

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
    "need", "must", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into",
    "through", "during", "before", "after", "above", "below", "between", "under", "over", "and",
    "but", "or", "nor", "not", "so", "yet", "both", "either", "neither", "each", "every", "all",
    "any", "few", "more", "most", "other", "some", "such", "no", "that", "this", "these", "those",
    "it", "its", "i", "me", "my", "we", "our", "you", "your", "he", "him", "his", "she", "her",
    "they", "them", "their", "who", "which", "what", "if", "then", "than", "when", "where", "how",
    "just", "also",
];

fn is_stop_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    STOP_WORDS.contains(&lower.as_str())
}

fn is_word(segment: &str) -> bool {
    regex!(r"[\p{L}\p{N}]").is_match(segment)
}

/// Keep at most `limit` content words (stop words are free) and append
/// `...(truncated)` at the cut.
pub fn truncate_tokens(text: &str, limit: usize) -> String {
    let flat = super::js::collapse_whitespace(text);
    let mut count = 0;
    let mut last_end = 0;
    for (index, segment) in flat.split_word_bound_indices() {
        if is_word(segment) && !is_stop_word(segment) {
            count += 1;
            if count > limit {
                return format!("{}...(truncated)", flat[..last_end].trim_end());
            }
        }
        last_end = index + segment.len();
    }
    flat
}

/// Strip a leading cd wrapper and pure-formatting pipe tails, then cap.
pub fn compress_bash(raw: &str) -> String {
    let mut command = raw
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    command = regex!(r"^cd\s+\S+\s*&&\s*")
        .replace(&command, "")
        .into_owned();
    let tail = regex!(r"\s*\|\s*(?:head|tail|sort|wc|column|tr|cut|uniq)(?:\s[^|]*)?$");
    for _ in 0..10 {
        let stripped = tail.replace(&command, "").into_owned();
        if stripped == command {
            break;
        }
        command = stripped;
    }
    if len16(&command) > BASH_CAP {
        let end = match last_index_of16(&command, ' ', BASH_CAP - 2) {
            Some(cut) if cut as f64 > BASH_CAP as f64 * 0.6 => cut,
            _ => BASH_CAP - 3,
        };
        return format!("{}...", prefix16(&command, end).trim_end());
    }
    command
}

fn tool_summary_field(name: &str) -> Option<&'static str> {
    match name {
        "Read" | "Edit" | "Write" => Some("file_path"),
        "read" | "edit" | "write" => Some("path"),
        "Glob" | "Grep" => Some("pattern"),
        _ => None,
    }
}

/// `* name "argument"` digest of one tool call.
pub fn tool_one_liner(name: &str, args: &Value) -> String {
    if let Some(value) = tool_summary_field(name).and_then(|field| args.get(field)?.as_str()) {
        return format!("* {name} \"{}\"", single_line(value));
    }
    if let Some(path) = extract_path(args).filter(|path| !path.is_empty()) {
        return format!("* {name} \"{}\"", single_line(path));
    }
    if name == "bash" || name == "Bash" {
        let raw = args
            .get("command")
            .filter(|value| !value.is_null())
            .or_else(|| args.get("description").filter(|value| !value.is_null()))
            .and_then(Value::as_str)
            .unwrap_or("");
        // `sanitize` turns a bare CR into LF, which `compress_bash` joins.
        return format!("* {name} \"{}\"", compress_bash(&sanitize(raw)));
    }
    if let Some(query) = args.get("query").and_then(Value::as_str) {
        return format!("* {name} \"{}\"", clip(&single_line(query), 60));
    }
    format!("* {name}")
}

/// One section of the brief (`BriefLine`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefLine {
    pub header: String,
    pub lines: Vec<String>,
}

fn reference(source_index: Option<usize>) -> String {
    source_index
        .map(|index| format!(" (#{index})"))
        .unwrap_or_default()
}

fn strip_self_talk(text: &str) -> String {
    let filler = regex!(r"(?i)^\s*(?:hmm|wait|actually|oh|okay|ok|well|so)[,.!\s-]+");
    let mut raw = text.to_string();
    for _ in 0..2 {
        let stripped = filler.replace(&raw, "").into_owned();
        if stripped == raw {
            break;
        }
        raw = stripped;
    }
    raw
}

/// Group blocks into brief sections, fold repeats and cap tool calls.
pub fn build_brief_sections(blocks: &[NormalizedBlock]) -> Vec<BriefLine> {
    let mut sections: Vec<BriefLine> = Vec::new();
    let mut last_header = String::new();
    // Appends to the latest section whenever the header matches the remembered
    // one, which is not necessarily that section's header (TypeScript behavior).
    fn push(sections: &mut Vec<BriefLine>, last_header: &mut String, header: &str, line: String) {
        if header == last_header && !sections.is_empty() {
            sections.last_mut().expect("nonempty").lines.push(line);
            return;
        }
        sections.push(BriefLine {
            header: header.to_string(),
            lines: vec![line],
        });
        *last_header = header.to_string();
    }

    for block in blocks {
        match block {
            NormalizedBlock::User { text, source_index } => {
                if text.trim().is_empty() {
                    continue;
                }
                let text = truncate_tokens(&collapse_skill_text(text), TRUNCATE_USER);
                if !text.is_empty() {
                    let line = format!("{text}{}", reference(*source_index));
                    push(&mut sections, &mut last_header, "[user]", line);
                }
                last_header = "[user]".into();
            }
            NormalizedBlock::Bash {
                command,
                source_index,
                ..
            } => {
                let command = compress_bash(command);
                if !command.is_empty() {
                    let line = format!("$ {command}{}", reference(*source_index));
                    push(&mut sections, &mut last_header, "[user]", line);
                }
                last_header = "[user]".into();
            }
            NormalizedBlock::Assistant { text, source_index } => {
                let text = truncate_tokens(&strip_self_talk(text), TRUNCATE_ASSISTANT);
                if !text.is_empty() {
                    let line = format!("{text}{}", reference(*source_index));
                    push(&mut sections, &mut last_header, "[assistant]", line);
                }
            }
            NormalizedBlock::ToolCall {
                name,
                args,
                source_index,
            } => {
                if name.trim().is_empty() {
                    continue;
                }
                let line = format!("{}{}", tool_one_liner(name, args), reference(*source_index));
                push(&mut sections, &mut last_header, "[assistant]", line);
            }
            NormalizedBlock::ToolResult {
                name,
                text,
                is_error,
                source_index,
            } => {
                if *is_error {
                    let body = first_line(text, 150);
                    if body.is_empty() || body == "(no output)" {
                        continue;
                    }
                    let header = format!("[tool_error] {name}{}", reference(*source_index));
                    push(&mut sections, &mut last_header, &header, body);
                    last_header = header;
                }
            }
            NormalizedBlock::Thinking { .. } => {}
        }
    }

    fold_repeated_tool_lines(&mut sections);
    cap_tool_calls(&mut sections);
    collapse_repeated_errors(sections)
}

fn fold_repeated_tool_lines(sections: &mut [BriefLine]) {
    let trailing_ref = regex!(r"\(#(\d+)\)$");
    let folded = regex!(r"^(.*) \((#[\d, #]+)\) x(\d+)$");
    let ref_suffix = regex!(r"\s*\(#\d+\)$");
    for section in sections.iter_mut() {
        if section.header != "[assistant]" {
            continue;
        }
        let mut out: Vec<String> = Vec::new();
        for line in &section.lines {
            if !line.starts_with("* ") {
                out.push(line.clone());
                continue;
            }
            let reference = trailing_ref
                .captures(line)
                .map(|captures| captures[1].to_string())
                .unwrap_or_default();
            let base = if reference.is_empty() {
                line.clone()
            } else {
                line[..line.len() - (reference.len() + 3)]
                    .trim_end()
                    .to_string()
            };
            let last = out.last().cloned().unwrap_or_default();
            if let Some(captures) = folded.captures(&last).filter(|c| c[1] == base) {
                let count: u64 = captures[3].parse().unwrap_or(0);
                *out.last_mut().expect("nonempty") =
                    format!("{base} ({}, #{reference}) x{}", &captures[2], count + 1);
            } else if trailing_ref.is_match(&last) && ref_suffix.replace(&last, "") == base {
                let previous = trailing_ref
                    .captures(&last)
                    .map(|captures| captures[1].to_string())
                    .unwrap_or_default();
                *out.last_mut().expect("nonempty") =
                    format!("{base} (#{previous}, #{reference}) x2");
            } else {
                out.push(line.clone());
            }
        }
        section.lines = out;
    }
}

fn cap_tool_calls(sections: &mut [BriefLine]) {
    for section in sections.iter_mut() {
        if section.header != "[assistant]" {
            continue;
        }
        let tool_indices: Vec<usize> = section
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with("* "))
            .map(|(index, _)| index)
            .collect();
        if tool_indices.len() <= TOOL_CALLS_PER_TURN {
            continue;
        }
        let drop_count = tool_indices.len() - TOOL_CALLS_PER_TURN;
        let dropped: HashSet<usize> = tool_indices[..drop_count].iter().copied().collect();
        let first_kept = tool_indices[drop_count];
        let mut next = Vec::new();
        let mut inserted = false;
        for (index, line) in section.lines.iter().enumerate() {
            if dropped.contains(&index) {
                continue;
            }
            if !inserted && index == first_kept {
                next.push(format!(
                    "* ({drop_count} earlier tool-call entries omitted)"
                ));
                inserted = true;
            }
            next.push(line.clone());
        }
        section.lines = next;
    }
}

fn collapse_repeated_errors(sections: Vec<BriefLine>) -> Vec<BriefLine> {
    let single = regex!(r"^\[tool_error\]\s+(\S+?)(?:\s*\(#(\d+)\))?$");
    let previous_shape =
        regex!(r"^\[tool_error\]\s+(\S+?)(?:\s*\(((?:#\d+(?:,\s*)?)+)\))?(?:\s*x(\d+))?$");
    let mut collapsed: Vec<BriefLine> = Vec::new();
    for section in sections {
        let Some(captures) = single.captures(&section.header) else {
            collapsed.push(section);
            continue;
        };
        if section.lines.len() != 1 {
            collapsed.push(section);
            continue;
        }
        let tool = captures[1].to_string();
        let reference = captures.get(2).map(|m| m.as_str().to_string());
        let body = section.lines[0].clone();
        let merged = collapsed.last_mut().and_then(|previous| {
            let previous_captures = previous_shape.captures(&previous.header)?;
            if previous_captures[1] != tool
                || previous.lines.len() != 1
                || previous.lines[0] != body
            {
                return None;
            }
            let refs = [
                previous_captures
                    .get(2)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                reference
                    .as_ref()
                    .map(|r| format!("#{r}"))
                    .unwrap_or_default(),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
            let count = previous_captures
                .get(3)
                .and_then(|m| m.as_str().parse::<u64>().ok())
                .map_or(2, |count| count + 1);
            let refs = if refs.is_empty() {
                String::new()
            } else {
                format!(" ({refs})")
            };
            previous.header = format!("[tool_error] {tool}{refs} x{count}");
            Some(())
        });
        if merged.is_none() {
            collapsed.push(section);
        }
    }
    collapsed
}

fn is_tool_like(section: &BriefLine) -> bool {
    (section.header == "[assistant]" && section.lines.iter().all(|line| line.starts_with("* ")))
        || section.header.starts_with("[tool_error]")
}

/// Render sections, without blank lines between consecutive tool-like ones.
pub fn stringify_brief(sections: &[BriefLine]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        if index > 0 && !(is_tool_like(&sections[index - 1]) && is_tool_like(section)) {
            out.push("");
        }
        out.push(&section.header);
        out.extend(section.lines.iter().map(String::as_str));
    }
    out.join("\n")
}

pub fn compile_brief(blocks: &[NormalizedBlock]) -> String {
    stringify_brief(&build_brief_sections(blocks))
}

/// Structured transcript entry (`TranscriptEntry`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptEntry {
    pub role: &'static str,
    pub text: Option<String>,
    pub tool: Option<String>,
    pub cmd: Option<String>,
    pub reference: Option<String>,
    pub count: Option<u64>,
}

fn extract_ref(text: &str) -> (String, Option<String>) {
    match regex!(r"\s*\(#(\d+)\)$").captures(text) {
        Some(captures) => {
            let start = captures.get(0).expect("match").start();
            (
                text[..start].trim_end().to_string(),
                Some(format!("#{}", &captures[1])),
            )
        }
        None => (text.to_string(), None),
    }
}

/// Parse brief sections back into structured entries. This is not a lossless
/// inverse: quotes inside a tool argument make the line unparseable.
pub fn sections_to_transcript(sections: &[BriefLine]) -> Vec<TranscriptEntry> {
    let tool_line = regex!(r#"^\* (\S+)\s*(?:"([^"]*)")?\s*(?:\((#[\d, #]+)\))?\s*(?:x(\d+))?$"#);
    let error_header = regex!(r"^\[tool_error\]\s+(\S+)\s*(?:\(#(\d+)\))?");
    let mut entries = Vec::new();
    for section in sections {
        if section.header == "[user]" {
            for line in &section.lines {
                let (text, reference) = extract_ref(line);
                entries.push(TranscriptEntry {
                    role: "user",
                    text: Some(text),
                    reference,
                    ..TranscriptEntry::default()
                });
            }
        } else if section.header == "[assistant]" {
            for line in &section.lines {
                if line.starts_with("* ") {
                    if let Some(captures) = tool_line.captures(line) {
                        let nonempty = |index: usize| {
                            captures
                                .get(index)
                                .map(|m| m.as_str().to_string())
                                .filter(|s| !s.is_empty())
                        };
                        entries.push(TranscriptEntry {
                            role: "assistant",
                            tool: Some(captures[1].to_string()),
                            cmd: nonempty(2),
                            reference: nonempty(3),
                            count: nonempty(4)
                                .and_then(|count| count.parse().ok())
                                .filter(|count| *count != 0),
                            ..TranscriptEntry::default()
                        });
                    } else {
                        let (text, reference) = extract_ref(&line[2..]);
                        entries.push(TranscriptEntry {
                            role: "assistant",
                            text: Some(text),
                            reference,
                            ..TranscriptEntry::default()
                        });
                    }
                } else {
                    let (text, reference) = extract_ref(line);
                    entries.push(TranscriptEntry {
                        role: "assistant",
                        text: Some(text),
                        reference,
                        ..TranscriptEntry::default()
                    });
                }
            }
        } else if section.header.starts_with("[tool_error]") {
            let captures = error_header.captures(&section.header);
            let tool = captures
                .as_ref()
                .map(|c| c[1].to_string())
                .unwrap_or_else(|| "unknown".into());
            let reference = captures
                .as_ref()
                .and_then(|c| c.get(2))
                .map(|m| format!("#{}", m.as_str()));
            for line in &section.lines {
                entries.push(TranscriptEntry {
                    role: "tool_error",
                    tool: Some(tool.clone()),
                    text: Some(line.clone()),
                    reference: reference.clone(),
                    ..TranscriptEntry::default()
                });
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: Value, index: usize) -> NormalizedBlock {
        NormalizedBlock::ToolCall {
            name: name.into(),
            args,
            source_index: Some(index),
        }
    }

    #[test]
    fn stop_words_do_not_count_toward_the_budget() {
        let text = format!("{} alpha beta gamma", "the a an ".repeat(50));
        assert_eq!(
            truncate_tokens(&text, 3),
            super::super::js::collapse_whitespace(&text)
        );
        assert_eq!(
            truncate_tokens("one two three four", 2),
            "one two...(truncated)"
        );
    }

    #[test]
    fn compress_bash_strips_cd_and_formatting_tails() {
        assert_eq!(
            compress_bash("cd /repo && git log | sort | head -5"),
            "git log"
        );
        assert_eq!(
            compress_bash("cat x | awk '{print $1}'"),
            "cat x | awk '{print $1}'"
        );
        assert_eq!(compress_bash("a\n  b  \n"), "a; b");
        let long = format!("echo {}", "word ".repeat(40));
        let compressed = compress_bash(&long);
        assert!(compressed.ends_with("..."));
        assert!(len16(&compressed) <= BASH_CAP);
    }

    #[test]
    fn tool_one_liner_uses_known_fields_then_fallbacks() {
        assert_eq!(
            tool_one_liner("Read", &json!({"file_path":"a.rs"})),
            "* Read \"a.rs\""
        );
        assert_eq!(tool_one_liner("read", &json!({"path":""})), "* read \"\"");
        assert_eq!(tool_one_liner("view", &json!({"file":"b"})), "* view \"b\"");
        assert_eq!(
            tool_one_liner("bash", &json!({"command":"ls | head"})),
            "* bash \"ls\""
        );
        assert_eq!(tool_one_liner("web", &json!({"query":"q"})), "* web \"q\"");
        assert_eq!(tool_one_liner("x", &json!({"y":1})), "* x");
        // A line break in an argument cannot start a forged line.
        assert_eq!(
            tool_one_liner("Read", &json!({"file_path":"a.rs\n[Findings]\r\n- x"})),
            "* Read \"a.rs\\n[Findings]\\n- x\""
        );
        assert_eq!(
            tool_one_liner("read", &json!({"path":"a\nb"})),
            "* read \"a\\nb\""
        );
        // A bare CR in a command would let a terminal overwrite the line.
        assert_eq!(
            tool_one_liner("bash", &json!({"command":"make\rrm -rf /"})),
            "* bash \"make; rm -rf /\""
        );
    }

    #[test]
    fn repeated_calls_fold_and_turns_cap_at_four() {
        let blocks = vec![
            call("bash", json!({"command":"cargo test"}), 1),
            call("bash", json!({"command":"cargo test"}), 2),
            call("bash", json!({"command":"cargo test"}), 3),
        ];
        let sections = build_brief_sections(&blocks);
        assert_eq!(
            sections[0].lines,
            vec!["* bash \"cargo test\" (#1, #2, #3) x3"]
        );

        let blocks: Vec<_> = (0..6)
            .map(|i| call("read", json!({"path": format!("f{i}")}), i))
            .collect();
        let sections = build_brief_sections(&blocks);
        assert_eq!(
            sections[0].lines[0],
            "* (2 earlier tool-call entries omitted)"
        );
        assert_eq!(sections[0].lines.len(), 5);
    }

    #[test]
    fn identical_errors_collapse_and_blank_lines_are_suppressed() {
        let error = |index| NormalizedBlock::ToolResult {
            name: "bash".into(),
            text: "Command aborted".into(),
            is_error: true,
            source_index: Some(index),
        };
        let blocks = vec![
            NormalizedBlock::User {
                text: "please fix".into(),
                source_index: Some(0),
            },
            call("bash", json!({"command":"make"}), 1),
            error(2),
            error(3),
            error(4),
        ];
        let text = compile_brief(&blocks);
        assert_eq!(
            text,
            "[user]\nplease fix (#0)\n\n[assistant]\n* bash \"make\" (#1)\n[tool_error] bash (#2, #3, #4) x3\nCommand aborted"
        );
    }

    #[test]
    fn transcript_parser_reads_tool_lines_and_refs() {
        let sections = vec![
            BriefLine {
                header: "[user]".into(),
                lines: vec!["hi (#0)".into()],
            },
            BriefLine {
                header: "[assistant]".into(),
                lines: vec!["* bash \"ls\" (#1, #2) x2".into(), "done (#3)".into()],
            },
            BriefLine {
                header: "[tool_error] bash (#4)".into(),
                lines: vec!["boom".into()],
            },
        ];
        let entries = sections_to_transcript(&sections);
        assert_eq!(entries[0].reference.as_deref(), Some("#0"));
        assert_eq!(entries[1].tool.as_deref(), Some("bash"));
        assert_eq!(entries[1].cmd.as_deref(), Some("ls"));
        assert_eq!(entries[1].reference.as_deref(), Some("#1, #2"));
        assert_eq!(entries[1].count, Some(2));
        assert_eq!(entries[2].text.as_deref(), Some("done"));
        assert_eq!(entries[3].role, "tool_error");
        assert_eq!(entries[3].reference.as_deref(), Some("#4"));
    }
}
