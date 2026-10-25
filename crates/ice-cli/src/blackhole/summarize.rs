//! Compile a fresh checkpoint and merge it with the previous one. Ports
//! `core/summarize.ts` (BLACK-027..030).

use ice_ai::types::Message;
use indexmap::IndexSet;

use super::normalize::{filter_noise, normalize, FileOps};
use super::sections::{
    build_sections, cap_brief, format_summary, wrap_long_lines, BuildSectionsInput,
};
use crate::blackhole::regex;

const HEADER_NAMES: [&str; 8] = [
    "User Requests",
    "Findings",
    "Last Actions",
    "Session Goal",
    "Files And Changes",
    "Commits",
    "Outstanding Context",
    "User Preferences",
];

const SEPARATOR: &str = "\n\n---\n\n";
const SCOPE_CHANGE_MARKER: &str = "[Scope change]";

/// `CompileInput`.
#[derive(Debug, Clone, Default)]
pub struct CompileInput {
    pub messages: Vec<Message>,
    pub previous_summary: Option<String>,
    pub file_ops: Option<FileOps>,
    pub user_requests: Option<Vec<String>>,
}

/// The first `[header]` occurrence anywhere, up to the next known header at a
/// line start or the brief separator.
fn section_of(text: &str, header: &str) -> String {
    let tag = format!("[{header}]");
    let Some(start) = text.find(&tag) else {
        return String::new();
    };
    let after = &text[start..];
    let mut candidates: Vec<usize> = HEADER_NAMES
        .iter()
        .filter(|name| **name != header)
        .filter_map(|name| {
            let pattern = format!(r"(?:^|\n)\[{}\]", regex::escape(name));
            let found = regex::Regex::new(&pattern).ok()?.find(after)?;
            Some(found.start() + usize::from(found.as_str().starts_with('\n')))
        })
        .collect();
    if let Some(separator) = after.find(SEPARATOR).filter(|index| *index > 0) {
        candidates.push(separator);
    }
    candidates.sort_unstable();
    match candidates.first() {
        Some(end) if *end > 0 => after[..*end].trim().to_string(),
        _ => after.trim().to_string(),
    }
}

fn brief_of(text: &str) -> String {
    match text.find(SEPARATOR) {
        Some(index) => text[index + SEPARATOR.len()..].trim().to_string(),
        None => String::new(),
    }
}

/// Items of a section with wrapped continuation lines rejoined.
fn section_items(section: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for line in section.split('\n').skip(1) {
        if let Some(item) = line.strip_prefix("- ") {
            items.push(item.trim().to_string());
        } else if !items.is_empty() && !line.trim().is_empty() {
            let last = items.last_mut().expect("nonempty");
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    items
}

fn format_items(header: &str, items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let body = items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("[{header}]\n{body}")
}

fn normalize_goal(item: &str) -> String {
    regex!(r"\s+\(#\d+\)$")
        .replace(item, "")
        .trim()
        .to_lowercase()
}

fn split_goal(items: Vec<String>) -> (Vec<String>, Vec<String>) {
    match items.iter().position(|item| item == SCOPE_CHANGE_MARKER) {
        Some(marker) => (items[..marker].to_vec(), items[marker + 1..].to_vec()),
        None => (items, Vec::new()),
    }
}

/// Keep the original goal plus only the newest scope change.
fn merge_session_goal(prev: &str, fresh: &str) -> String {
    let (previous_base, previous_scope) = split_goal(section_items(prev));
    let (current_base, current_scope) = split_goal(section_items(fresh));
    let previous_keys: std::collections::HashSet<String> = previous_base
        .iter()
        .map(|item| normalize_goal(item))
        .collect();
    let new_base: Vec<String> = current_base
        .iter()
        .filter(|item| !previous_keys.contains(&normalize_goal(item)))
        .cloned()
        .collect();
    let latest = if !current_scope.is_empty() {
        current_scope
    } else if !new_base.is_empty() {
        new_base
    } else if !previous_scope.is_empty() {
        previous_scope
    } else {
        current_base
    };
    let base: Vec<String> = previous_base.into_iter().take(5).collect();
    let base_keys: std::collections::HashSet<String> =
        base.iter().map(|item| normalize_goal(item)).collect();
    let scope: Vec<String> = latest
        .into_iter()
        .filter(|item| !base_keys.contains(&normalize_goal(item)))
        .take(3)
        .collect();
    if scope.is_empty() {
        format_items("Session Goal", &base)
    } else {
        let mut items = base;
        items.push(SCOPE_CHANGE_MARKER.into());
        items.extend(scope);
        format_items("Session Goal", &items)
    }
}

/// Union visible file lists by category; modified paths leave read/created.
fn merge_file_lines(prev: &str, fresh: &str) -> String {
    let categories = ["Modified", "Created", "Read"];
    let mut merged: [IndexSet<String>; 3] = Default::default();
    let more = regex!(r"\s*\(\+\d+ more\)\s*$");
    for text in [prev, fresh] {
        for line in text.split('\n') {
            for (index, category) in categories.iter().enumerate() {
                let prefix = format!("- {category}: ");
                let Some(rest) = line.strip_prefix(&prefix) else {
                    continue;
                };
                let rest = more.replace(rest, "");
                for path in rest.split(',') {
                    let path = path.trim();
                    if !path.is_empty() {
                        merged[index].insert(path.to_string());
                    }
                }
            }
        }
    }
    let modified = merged[0].clone();
    for path in &modified {
        merged[1].shift_remove(path);
        merged[2].shift_remove(path);
    }
    let lines: Vec<String> = categories
        .iter()
        .zip(merged.iter())
        .filter(|(_, set)| !set.is_empty())
        .map(|(category, set)| format!("- {category}: {}", super::sections::cap_list(set, 10)))
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!("[Files And Changes]\n{}", lines.join("\n"))
}

fn merge_header_section(header: &str, prev: &str, fresh: &str) -> String {
    if header == "Outstanding Context" || header == "Last Actions" {
        return fresh.to_string();
    }
    if prev.is_empty() {
        return fresh.to_string();
    }
    if fresh.is_empty() {
        return prev.to_string();
    }
    if header == "User Requests" || header == "Findings" {
        let mut items = section_items(prev);
        items.extend(section_items(fresh));
        let start = items.len().saturating_sub(6);
        return format_items(header, &items[start..]);
    }
    if header == "Session Goal" {
        return merge_session_goal(prev, fresh);
    }
    if header == "Files And Changes" {
        return merge_file_lines(prev, fresh);
    }
    let is_clean = |line: &&str| {
        line.starts_with("- ") && !line.contains("<skill") && !line.contains("</skill")
    };
    let combined: IndexSet<&str> = prev
        .split('\n')
        .filter(is_clean)
        .chain(fresh.split('\n').filter(is_clean))
        .collect();
    let cap = if header == "Commits" { 8 } else { 15 };
    let lines: Vec<&str> = combined.into_iter().collect();
    let capped = &lines[lines.len().saturating_sub(cap)..];
    if capped.is_empty() {
        return String::new();
    }
    format!("[{header}]\n{}", capped.join("\n"))
}

fn merge_previous(prev: &str, fresh: &str) -> String {
    let headers: Vec<String> = HEADER_NAMES
        .iter()
        .map(|header| {
            merge_header_section(
                header,
                &section_of(prev, header),
                &section_of(fresh, header),
            )
        })
        .filter(|section| !section.is_empty())
        .collect();
    let prev_brief = brief_of(prev);
    let fresh_brief = brief_of(fresh);
    let merged_brief = match (prev_brief.is_empty(), fresh_brief.is_empty()) {
        (true, _) => fresh_brief,
        (_, true) => prev_brief,
        _ => format!("{prev_brief}\n\n{fresh_brief}"),
    };
    let mut parts = Vec::new();
    if !headers.is_empty() {
        parts.push(headers.join("\n\n"));
    }
    if !merged_brief.is_empty() {
        parts.push(cap_brief(&merged_brief));
    }
    parts.join(SEPARATOR)
}

/// Drop a line-start `[Relevant Memory]` section and everything after it.
pub fn strip_trailing_volatile_sections(text: &str) -> String {
    regex!(r"(?:^|\n)\[Relevant Memory\](?s:.)*$")
        .replace(text, "")
        .trim_end()
        .to_string()
}

/// Drop paragraphs containing the compacted-conversation recall note.
pub fn strip_recall_notes(text: &str) -> String {
    regex!(r"\n\n+")
        .split(text)
        .filter(|paragraph| {
            !paragraph.contains("The conversation before this point has been compacted")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Remove legacy observational-memory content and its recall footer.
pub fn strip_om_content(text: &str) -> String {
    let reflections = regex!(r"(?m)^## Reflections").find(text).map(|m| m.start());
    let observations = regex!(r"(?m)^## Observations")
        .find(text)
        .map(|m| m.start());
    let basic_footer =
        text.find("Use `recall` with an id to retrieve original context, or `#N:path` drill-down");
    let strip_from = if reflections.is_some() || observations.is_some() {
        let preamble = text.find("These are condensed memories from earlier in this session.");
        let min_section = reflections
            .into_iter()
            .chain(observations)
            .min()
            .expect("one section header");
        match preamble {
            Some(index) if index < min_section => Some(index),
            _ => Some(min_section),
        }
    } else {
        basic_footer
    };
    let Some(strip_from) = strip_from else {
        return text.to_string();
    };
    let before = text[..strip_from].trim_end();
    match before.strip_suffix("---") {
        Some(rest) => rest.trim_end().to_string(),
        None => before.to_string(),
    }
}

/// Compile fresh deterministic content and merge it with the previous summary.
pub fn compile(input: &CompileInput) -> String {
    let blocks = filter_noise(normalize(&input.messages));
    let data = build_sections(BuildSectionsInput {
        blocks: &blocks,
        file_ops: input.file_ops.as_ref(),
        user_requests: input.user_requests.as_deref(),
    });
    let fresh = format_summary(&data);
    let previous = input.previous_summary.as_deref().map(|summary| {
        strip_recall_notes(&strip_om_content(&strip_trailing_volatile_sections(
            summary,
        )))
    });
    let merged = match previous.filter(|prev| !prev.is_empty()) {
        Some(prev) => merge_previous(&prev, &fresh),
        None => fresh,
    };
    if merged.is_empty() {
        return String::new();
    }
    wrap_long_lines(&strip_recall_notes(&merged))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_of_stops_at_the_next_header_or_separator() {
        let text = "[Findings]\n- a\n- b\n\n[Commits]\n- c\n\n---\n\n[user]\nhi";
        assert_eq!(section_of(text, "Findings"), "[Findings]\n- a\n- b");
        assert_eq!(section_of(text, "Commits"), "[Commits]\n- c");
        assert_eq!(brief_of(text), "[user]\nhi");
        assert_eq!(
            section_items("[X]\n- one\n  two\n- three"),
            vec!["one two", "three"]
        );
    }

    #[test]
    fn rolling_and_volatile_sections_merge_by_policy() {
        let prev = "[User Requests]\n- r1\n- r2\n\n[Last Actions]\n- old\n\n[Commits]\n- a\n- b";
        let fresh = "[User Requests]\n- r3\n\n[Commits]\n- b\n- c";
        let merged = merge_previous(prev, fresh);
        assert_eq!(
            merged,
            "[User Requests]\n- r1\n- r2\n- r3\n\n[Commits]\n- a\n- b\n- c"
        );
    }

    #[test]
    fn session_goal_keeps_base_and_newest_scope() {
        let prev = "[Session Goal]\n- Build parser (#0)\n- [Scope change]\n- Old scope (#3)";
        let fresh = "[Session Goal]\n- Build parser (#1)\n- [Scope change]\n- New scope (#9)";
        assert_eq!(
            merge_session_goal(prev, fresh),
            "[Session Goal]\n- Build parser (#0)\n- [Scope change]\n- New scope (#9)"
        );
        // A fresh base goal absent from the stored base is new scope.
        let fresh = "[Session Goal]\n- Translate the docs";
        assert_eq!(
            merge_session_goal(prev, fresh),
            "[Session Goal]\n- Build parser (#0)\n- [Scope change]\n- Translate the docs"
        );
        // Repeated base-only context keeps the previous scope.
        let fresh = "[Session Goal]\n- build parser";
        assert_eq!(
            merge_session_goal(prev, fresh),
            "[Session Goal]\n- Build parser (#0)\n- [Scope change]\n- Old scope (#3)"
        );
    }

    #[test]
    fn file_lines_union_and_reconcile() {
        let prev = "[Files And Changes]\n- Modified: a.rs\n- Read: a.rs, b.rs (+3 more)";
        let fresh = "[Files And Changes]\n- Created: a.rs, c.rs\n- Read: d.rs";
        assert_eq!(
            merge_file_lines(prev, fresh),
            "[Files And Changes]\n- Modified: a.rs\n- Created: c.rs\n- Read: b.rs, d.rs"
        );
    }

    #[test]
    fn cleanup_strips_memory_om_and_recall_notes() {
        assert_eq!(
            strip_trailing_volatile_sections("[Findings]\n- a\n[Relevant Memory]\n- m"),
            "[Findings]\n- a"
        );
        assert_eq!(
            strip_om_content("keep\n---\nThese are condensed memories from earlier in this session.\n## Observations\nx"),
            "keep"
        );
        assert_eq!(
            strip_recall_notes(
                "a\n\nThe conversation before this point has been compacted. Use it.\n\n\nb"
            ),
            "a\n\nb"
        );
    }

    #[test]
    fn compile_merges_with_cleaned_previous_summary() {
        use ice_ai::types::UserMessage;
        let input = CompileInput {
            messages: vec![Message::User(UserMessage::new(
                "Implement the cache layer for the API".to_string(),
                0,
            ))],
            previous_summary: Some(
                "[User Requests]\n- earlier\n\n[Relevant Memory]\n- stale".into(),
            ),
            file_ops: None,
            user_requests: Some(vec!["Implement the cache layer for the API".into()]),
        };
        let summary = compile(&input);
        assert!(summary
            .starts_with("[User Requests]\n- earlier\n- Implement the cache layer for the API"));
        assert!(!summary.contains("stale"));
        assert!(summary.contains("[Session Goal]\n- Implement the cache layer for the API"));
    }
}
