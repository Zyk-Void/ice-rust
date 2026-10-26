//! Per-tool presentation rules for the boxed tool panel. Mirrors the
//! `renderCall`/`renderResult` functions of ice's built-in file tools
//! (`core/tools/read.ts`, `write.ts`, `grep.ts`, `find.ts`, `ls.ts`).
//!
//! - read: the title names the line range and labels skill and agent
//!   instruction files; a successful result's file text stays collapsed
//!   (its notices stay visible), errors show up to 10 lines.
//! - write: the arriving `content` argument is previewed (10 lines
//!   collapsed, all expanded) with syntax highlighting by extension; a
//!   successful result is hidden because the preview already shows it.
//! - grep previews 15 result lines, find and ls 20.
//!
//! Differences from ice, on purpose: ice's package-documentation label is
//! not ported (icers serves its docs through a separate tool), paths are not
//! rewritten relative to cwd (the panel does not know it), and the whole
//! preview is re-highlighted on each change rather than incrementally (the
//! panel's render cache already skips unchanged revisions).

use crate::syntax_highlight::Highlighter;
use crate::theme::ThemeColors;

/// Collapsed write preview. Mirrors ice's 10-line write preview.
pub(crate) const WRITE_PREVIEW_LINES: usize = 10;
/// Collapsed rows of a failed read. Mirrors ice's 10-line error preview.
pub(crate) const READ_ERROR_PREVIEW_LINES: usize = 10;

/// Collapsed result rows for `tool`, or `fallback` for tools without a rule.
pub(crate) fn result_preview_lines(tool: &str, fallback: usize) -> usize {
    match tool {
        "grep" => 15,
        "find" | "ls" => 20,
        _ => fallback,
    }
}

/// The read title: the path with its requested line range, labelled when it
/// is a skill or an agent instruction file. Mirrors read's `renderCall`.
pub(crate) fn read_title(path: &str, offset: Option<f64>, limit: Option<f64>) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if name == "SKILL.md" {
        let parent = path[..path.len() - name.len()].trim_end_matches(['/', '\\']);
        let skill = parent.rsplit(['/', '\\']).next().unwrap_or("");
        if !skill.is_empty() {
            return format!("[skill] {skill}");
        }
    }
    let label = match name {
        "AGENTS.md" | "AGENTS.MD" | "CLAUDE.md" | "CLAUDE.MD" => "[resource] ",
        _ => "",
    };
    let whole = |n: Option<f64>| n.filter(|n| n.is_finite() && *n >= 1.0).map(|n| n as u64);
    let range = match (whole(offset), whole(limit)) {
        (None, None) => String::new(),
        (start, Some(limit)) => {
            let start = start.unwrap_or(1);
            start
                .checked_add(limit.saturating_sub(1))
                .map(|end| format!(":{start}-{end}"))
                .unwrap_or_default()
        }
        (Some(start), None) => format!(":{start}"),
    };
    format!("{label}{path}{range}")
}

/// Rows of a successful read result that stay visible while collapsed: the
/// tool's bracketed notices (truncation, continuation, image warnings).
pub(crate) fn read_notice_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| {
            let line = line.trim();
            line.starts_with('[') && line.ends_with(']')
        })
        .cloned()
        .collect()
}

/// A string field from tool-call arguments that may still be streaming: an
/// unterminated value returns what has arrived. Handles JSON escapes.
pub(crate) fn partial_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut rest = &json[json.find(&needle)? + needle.len()..];
    rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let mut chars = rest.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{8}'),
                Some('f') => out.push('\u{c}'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(decoded) => out.push(decoded),
                        // A surrogate pair or a code still arriving.
                        None if hex.len() == 4 => out.push('\u{FFFD}'),
                        None => return Some(out),
                    }
                }
                Some(other) => out.push(other),
                None => return Some(out),
            },
            c => out.push(c),
        }
    }
    Some(out)
}

/// The highlighted preview of a write call's `content`, or nothing before
/// any content arrives. `hint` renders the "N more lines" row.
pub(crate) fn write_preview_lines(
    colors: &ThemeColors,
    args: &str,
    expanded: bool,
    hint: impl Fn(usize) -> String,
) -> Vec<String> {
    let Some(content) = partial_string_field(args, "content") else {
        return Vec::new();
    };
    let path = partial_string_field(args, "path").unwrap_or_default();
    let language = path
        .rsplit(['/', '\\'])
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .unwrap_or("");
    let mut source: Vec<&str> = content.split('\n').collect();
    while source.last().is_some_and(|line| line.trim().is_empty()) {
        source.pop();
    }
    let shown = if expanded {
        source.len()
    } else {
        source.len().min(WRITE_PREVIEW_LINES)
    };
    let mut highlighter = Highlighter::new(language);
    let mut rows: Vec<String> = source[..shown]
        .iter()
        .map(|line| {
            let clean: String = line
                .replace('\t', "   ")
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            highlighter.highlight(&clean, colors)
        })
        .collect();
    if source.len() > shown {
        rows.push(hint(source.len() - shown));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_titles_show_ranges_and_resource_labels() {
        assert_eq!(read_title("src/a.rs", None, None), "src/a.rs");
        assert_eq!(
            read_title("src/a.rs", Some(10.0), Some(20.0)),
            "src/a.rs:10-29"
        );
        assert_eq!(read_title("src/a.rs", Some(5.0), None), "src/a.rs:5");
        assert_eq!(read_title("src/a.rs", None, Some(3.0)), "src/a.rs:1-3");
        assert_eq!(
            read_title("/home/u/.icers/skills/release/SKILL.md", None, None),
            "[skill] release"
        );
        assert_eq!(read_title("AGENTS.md", None, None), "[resource] AGENTS.md");
        assert_eq!(
            read_title("repo/CLAUDE.MD", Some(1.0), Some(2.0)),
            "[resource] repo/CLAUDE.MD:1-2"
        );
        assert_eq!(
            read_title("src/a.rs", Some(u64::MAX as f64), Some(2.0)),
            "src/a.rs"
        );
    }

    #[test]
    fn partial_fields_decode_escapes_and_tolerate_streaming() {
        let complete = r#"{"path":"a.rs","content":"fn main() {\n\t\"hi\" é\n}"}"#;
        assert_eq!(
            partial_string_field(complete, "content").as_deref(),
            Some("fn main() {\n\t\"hi\" é\n}")
        );
        let streaming = r#"{"path": "a.rs", "content": "line 1\nline 2\nli"#;
        assert_eq!(
            partial_string_field(streaming, "content").as_deref(),
            Some("line 1\nline 2\nli")
        );
        assert_eq!(partial_string_field(r#"{"path":"a.rs"}"#, "content"), None);
        assert_eq!(partial_string_field(r#"{"content":null}"#, "content"), None);
        assert_eq!(
            partial_string_field(r#"{"content":"x\"#, "content").as_deref(),
            Some("x")
        );
    }

    #[test]
    fn write_preview_collapses_to_ten_lines() {
        let colors = crate::theme::Theme::default().colors;
        let body: Vec<String> = (1..=14).map(|n| format!("line {n}")).collect();
        let args = format!(
            r#"{{"path":"notes.txt","content":"{}\n"}}"#,
            body.join("\\n")
        );
        let hint = |n: usize| format!("+{n}");
        let collapsed = write_preview_lines(&colors, &args, false, hint);
        assert_eq!(collapsed.len(), 11);
        assert_eq!(collapsed.last().map(String::as_str), Some("+4"));
        let expanded = write_preview_lines(&colors, &args, true, hint);
        assert_eq!(expanded.len(), 14);
        assert!(crate::ansi::strip_ansi(&expanded[13]).contains("line 14"));
    }

    #[test]
    fn search_tools_get_their_own_preview_lengths() {
        assert_eq!(result_preview_lines("grep", 12), 15);
        assert_eq!(result_preview_lines("find", 12), 20);
        assert_eq!(result_preview_lines("ls", 12), 20);
        assert_eq!(result_preview_lines("bash", 5), 5);
    }
}
