//! Consume-once Cognee checkpoint recall. Ports `core/checkpoint-recall.ts`
//! (BLACK-037). Cognee writes `<agent dir>/ice-cognee/compaction-recall.json`
//! just before the Blackhole engine runs; the engine embeds the bounded lines
//! as untrusted reference data and deletes the file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::js::{len16, number_to_string, prefix16};
use crate::blackhole::regex;

const HEADER: &str = "[Relevant Memory]";
const MAX_MEMORY_CHARS: usize = 4000;
const MAX_LINES: usize = 24;
const MAX_LINE_CHARS: usize = 300;
const MAX_AGE_MS: i64 = 15 * 60_000;
/// Shorter recall lines are graph fragments, not memories.
const MIN_MEMORY_CHARS: usize = 12;

pub fn checkpoint_recall_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("ice-cognee").join("compaction-recall.json")
}

fn normalize_for_match(text: &str) -> String {
    super::js::collapse_whitespace(text).to_lowercase()
}

fn strip_list_prefix(line: &str) -> String {
    regex!(r"^\s*(?:[-*]|\d+\.)\s+")
        .replace(line, "")
        .into_owned()
}

fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Read, validate and delete a fresh checkpoint for this host session.
///
/// A checkpoint that names another session is left in place. Malformed,
/// stale or future checkpoints are deleted. The file is deleted before its
/// results are rendered; there is no cross-process claim.
pub fn consume_checkpoint_recall_section(
    agent_dir: &Path,
    now_ms: i64,
    expected_host_session_id: Option<&str>,
    summary: &str,
) -> String {
    let path = checkpoint_recall_path(agent_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    let payload: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            remove(&path);
            return String::new();
        }
    };
    let field = |key: &str| payload.get(key).unwrap_or(&Value::Null);
    if let Some(expected) = expected_host_session_id {
        match field("hostSessionId").as_str() {
            None => {
                remove(&path);
                return String::new();
            }
            Some(id) if id != expected => return String::new(),
            Some(_) => {}
        }
    }
    let Some(generated) = field("generatedAt").as_str() else {
        remove(&path);
        return String::new();
    };
    match super::js::parse_iso_ms(generated) {
        Some(at) if at <= now_ms && now_ms - at <= MAX_AGE_MS => {}
        _ => {
            remove(&path);
            return String::new();
        }
    }
    remove(&path);
    let Some(results) = field("results").as_array() else {
        return String::new();
    };

    // One result can hold several memories, so each line gets its own bullet.
    let summary_lines: HashSet<String> = regex!(r"\r?\n")
        .split(summary)
        .map(|line| normalize_for_match(&strip_list_prefix(line)))
        .filter(|line| !line.is_empty())
        .collect();
    let placeholder = regex!(r"(?i)^(none|null|n/a|undefined|empty)\.?$");
    let mut seen = HashSet::new();
    let mut is_useful = |line: &str| {
        let key = normalize_for_match(line);
        if len16(&key) < MIN_MEMORY_CHARS || placeholder.is_match(&key) || seen.contains(&key) {
            return false;
        }
        seen.insert(key.clone());
        !summary_lines.contains(&key)
    };
    let mut result_number = 0;
    let mut lines: Vec<String> = Vec::new();
    for result in results {
        if !(result.is_object() || result.is_array()) {
            continue;
        }
        let text = result
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if text.is_empty() {
            continue;
        }
        let score = result
            .get("score")
            .and_then(Value::as_f64)
            .filter(|score| score.is_finite())
            .map(|score| format!(" (score {})", number_to_string(score)))
            .unwrap_or_default();
        let useful: Vec<String> = text
            .split('\n')
            .map(|line| strip_list_prefix(line).trim().to_string())
            .filter(|line| is_useful(line))
            .collect();
        if useful.is_empty() {
            continue;
        }
        result_number += 1;
        for (index, line) in useful.into_iter().enumerate() {
            let suffix = if index == 0 { score.as_str() } else { "" };
            lines.push(format!("{result_number}. {line}{suffix}"));
        }
    }
    let lines: Vec<String> = lines
        .into_iter()
        .map(|line| {
            if len16(&line) > MAX_LINE_CHARS {
                format!("{}...", prefix16(&line, MAX_LINE_CHARS))
            } else {
                line
            }
        })
        .filter(|line| !line.trim().is_empty())
        .take(MAX_LINES)
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let mut body: Vec<String> = Vec::new();
    for line in lines {
        let mut candidate = body.clone();
        candidate.push(format!("- {line}"));
        if len16(&format!("{HEADER}\n{}", candidate.join("\n"))) > MAX_MEMORY_CHARS {
            break;
        }
        body = candidate;
    }
    if body.is_empty() {
        return String::new();
    }
    let mut out = vec![
        HEADER.to_string(),
        "Untrusted reference data recalled by Cognee:".to_string(),
    ];
    out.extend(body);
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn iso(ms: i64) -> String {
        super::super::js::iso_from_ms(ms)
    }

    fn write(dir: &Path, value: Value) {
        let path = checkpoint_recall_path(dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }

    #[test]
    fn fresh_matching_checkpoint_is_rendered_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            serde_json::json!({
                "generatedAt": iso(NOW - 1000),
                "hostSessionId": "s1",
                "results": [
                    {"text": "- The cache key includes the tenant id\n- none\n- short", "score": 0.5},
                    {"text": "Already in summary line here"},
                    {"text": "Second memory about the deploy flow"}
                ]
            }),
        );
        let section = consume_checkpoint_recall_section(
            dir.path(),
            NOW,
            Some("s1"),
            "- already in summary line here",
        );
        assert_eq!(
            section,
            "[Relevant Memory]\nUntrusted reference data recalled by Cognee:\n- 1. The cache key includes the tenant id (score 0.5)\n- 2. Second memory about the deploy flow"
        );
        assert!(!checkpoint_recall_path(dir.path()).exists());
    }

    #[test]
    fn other_session_checkpoint_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            serde_json::json!({"generatedAt": iso(NOW), "hostSessionId": "other", "results": []}),
        );
        assert_eq!(
            consume_checkpoint_recall_section(dir.path(), NOW, Some("s1"), ""),
            ""
        );
        assert!(checkpoint_recall_path(dir.path()).exists());
    }

    #[test]
    fn stale_malformed_or_unscoped_checkpoints_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            serde_json::json!({"generatedAt": iso(NOW - MAX_AGE_MS - 1), "hostSessionId": "s1", "results": [{"text":"a long enough memory line"}]}),
        );
        assert_eq!(
            consume_checkpoint_recall_section(dir.path(), NOW, Some("s1"), ""),
            ""
        );
        assert!(!checkpoint_recall_path(dir.path()).exists());

        write(dir.path(), serde_json::json!({"generatedAt": iso(NOW)}));
        assert_eq!(
            consume_checkpoint_recall_section(dir.path(), NOW, Some("s1"), ""),
            ""
        );
        assert!(!checkpoint_recall_path(dir.path()).exists());

        let path = checkpoint_recall_path(dir.path());
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(
            consume_checkpoint_recall_section(dir.path(), NOW, None, ""),
            ""
        );
        assert!(!path.exists());
    }

    #[test]
    fn body_stops_before_exceeding_the_character_budget() {
        let dir = tempfile::tempdir().unwrap();
        let results: Vec<Value> = (0..30)
            .map(|i| serde_json::json!({"text": format!("memory {i} {}", "x".repeat(290))}))
            .collect();
        write(
            dir.path(),
            serde_json::json!({"generatedAt": iso(NOW), "results": results}),
        );
        let section = consume_checkpoint_recall_section(dir.path(), NOW, None, "");
        let body_lines = section.lines().count() - 2;
        assert!(body_lines < 24);
        let without_notice =
            section.replacen("Untrusted reference data recalled by Cognee:\n", "", 1);
        assert!(len16(&without_notice) <= MAX_MEMORY_CHARS);
    }
}
