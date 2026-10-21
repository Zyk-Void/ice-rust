//! Background session-content search. The cache is bounded; large logs are still
//! searched in full but are not retained between queries.
use crate::session_catalog::SessionRow;
use regex::{Regex, RegexBuilder};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

const CACHE_BYTES: usize = 16 * 1024 * 1024;

enum Query {
    All,
    Regex(Regex),
    Terms(Vec<(String, bool)>),
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl Query {
    fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Self::All);
        }
        if let Some(pattern) = text.strip_prefix("re:") {
            return RegexBuilder::new(pattern.trim())
                .case_insensitive(true)
                .size_limit(1024 * 1024)
                .dfa_size_limit(1024 * 1024)
                .build()
                .map(Self::Regex)
                .map_err(|error| format!("Invalid regular expression: {error}"));
        }
        let mut rest = text;
        let mut terms = Vec::new();
        while !rest.trim_start().is_empty() {
            rest = rest.trim_start();
            if let Some(quoted) = rest.strip_prefix('"') {
                if let Some(end) = quoted.find('"') {
                    let phrase = normalized(&quoted[..end]);
                    if !phrase.is_empty() {
                        terms.push((phrase, true));
                    }
                    rest = &quoted[end + 1..];
                    continue;
                }
                // A quote being typed does not disable the ordinary text search.
                rest = quoted;
            }
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            if end != 0 {
                terms.push((rest[..end].to_lowercase(), false));
            }
            rest = &rest[end..];
        }
        Ok(Self::Terms(terms))
    }

    fn matches(&self, text: &str) -> bool {
        match self {
            Self::All => true,
            Self::Regex(regex) => regex.is_match(text),
            Self::Terms(terms) => {
                let text = normalized(text);
                terms.iter().all(|(term, phrase)| {
                    if *phrase || text.contains(term) {
                        text.contains(term)
                    } else {
                        let mut wanted = term.chars();
                        let mut next = wanted.next();
                        for ch in text.chars() {
                            if next == Some(ch) {
                                next = wanted.next();
                                if next.is_none() {
                                    return true;
                                }
                            }
                        }
                        next.is_none()
                    }
                })
            }
        }
    }
}

struct CachedText {
    path: PathBuf,
    modified: SystemTime,
    bytes: u64,
    text: Arc<str>,
}

#[derive(Default)]
pub(crate) struct ContentCache {
    entries: VecDeque<CachedText>,
    bytes: usize,
}

impl ContentCache {
    fn get(&mut self, path: &Path, cancelled: &dyn Fn() -> bool) -> std::io::Result<Arc<str>> {
        let metadata = std::fs::metadata(path)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let bytes = metadata.len();
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.path == path && entry.modified == modified && entry.bytes == bytes
        }) {
            let entry = self.entries.remove(index).unwrap();
            let text = entry.text.clone();
            self.entries.push_back(entry);
            return Ok(text);
        }
        if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
            self.bytes -= self.entries.remove(index).unwrap().text.len();
        }
        let text: Arc<str> = read_message_text(path, cancelled)?.into();
        if text.len() <= CACHE_BYTES {
            while self.bytes + text.len() > CACHE_BYTES {
                self.bytes -= self.entries.pop_front().unwrap().text.len();
            }
            self.bytes += text.len();
            self.entries.push_back(CachedText {
                path: path.to_path_buf(),
                modified,
                bytes,
                text: text.clone(),
            });
        }
        Ok(text)
    }
}

fn read_message_text(path: &Path, cancelled: &dyn Fn() -> bool) -> std::io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut text = String::new();
    loop {
        if cancelled() {
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        }
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        // Both the older flat entries and current JSONL entry records are accepted.
        if value.get("type").and_then(|value| value.as_str()) != Some("message")
            || !matches!(
                value.get("kind").and_then(|value| value.as_str()),
                None | Some("entry")
            )
        {
            continue;
        }
        let content = &value["message"]["content"];
        match content {
            serde_json::Value::String(value) => {
                text.push_str(value);
                text.push('\n');
            }
            serde_json::Value::Array(blocks) => {
                for block in blocks {
                    if block["type"] == "text" {
                        if let Some(value) = block["text"].as_str() {
                            text.push_str(value);
                            text.push('\n');
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(text)
}

pub(crate) struct SearchResult {
    pub indices: Vec<usize>,
    pub unreadable: usize,
}

pub(crate) fn search_rows(
    rows: &[SessionRow],
    cwd: &Path,
    all: bool,
    text: &str,
    cache: &mut ContentCache,
    cancelled: &dyn Fn() -> bool,
) -> Result<SearchResult, String> {
    let query = Query::parse(text)?;
    let mut result = SearchResult {
        indices: Vec::new(),
        unreadable: 0,
    };
    for (index, row) in rows.iter().enumerate() {
        if cancelled() {
            break;
        }
        if !all && row.cwd != cwd {
            continue;
        }
        let mut document = format!("{}\n{}\n{}", row.label, row.id, row.cwd.display());
        if matches!(query, Query::All) {
            result.indices.push(index);
            continue;
        }
        match cache.get(&row.path, cancelled) {
            Ok(content) => {
                document.push('\n');
                document.push_str(&content);
            }
            Err(_) if cancelled() => break,
            Err(_) => result.unreadable += 1,
        }
        if query.matches(&document) {
            result.indices.push(index);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(path: &Path) -> SessionRow {
        SessionRow {
            path: path.into(),
            cwd: "/project".into(),
            id: "id".into(),
            label: "Ordinary name".into(),
            bytes: 0,
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn query_handles_phrases_regex_unicode_fuzzy_tokens_and_invalid_patterns() {
        let document = "release notes\nFixed the café\nrenderer with 東京 support";
        for query in [
            r#""café renderer""#,
            r#"re:\bFIXED\b"#,
            "rls nts",
            "東京 café",
            r#""release notes" support"#,
        ] {
            assert!(Query::parse(query).unwrap().matches(document), "{query}");
        }
        assert!(!Query::parse(r#""notes release""#)
            .unwrap()
            .matches(document));
        assert!(!Query::parse("missing 東京").unwrap().matches(document));
        assert!(Query::parse("re:[").is_err());
        assert!(Query::parse("\"release").unwrap().matches(document));
    }

    #[test]
    fn searches_late_messages_and_refreshes_warm_cache_after_append() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({"kind":"session","id":"id","cwd":"/project"})
        )
        .unwrap();
        for _ in 0..5000 {
            writeln!(file, "{}", serde_json::json!({"kind":"entry","type":"message","message":{"role":"user","content":"ordinary conversation padding"}})).unwrap();
        }
        writeln!(file, "{}", serde_json::json!({"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Rare needle\nin a late answer"},{"type":"image","data":"excluded-image-content"}]}})).unwrap();
        file.flush().unwrap();
        let rows = [fixture(&path)];
        let mut cache = ContentCache::default();
        let search = |query: &str, cache: &mut ContentCache| {
            search_rows(&rows, Path::new("/project"), false, query, cache, &|| false).unwrap()
        };
        assert_eq!(search(r#""needle in a late""#, &mut cache).indices, vec![0]);
        assert_eq!(cache.entries.len(), 1);
        assert!(search("re:excluded-image-content", &mut cache)
            .indices
            .is_empty());
        assert!(search("re:brand-new-answer", &mut cache).indices.is_empty());
        writeln!(file, "{}", serde_json::json!({"kind":"entry","type":"message","message":{"role":"toolResult","content":[{"type":"text","text":"brand-new-answer"}]}})).unwrap();
        file.flush().unwrap();
        assert_eq!(search("re:brand-new-answer", &mut cache).indices, vec![0]);
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn cancellation_and_unreadable_files_do_not_install_false_results() {
        let mut cache = ContentCache::default();
        let rows = [fixture(Path::new("/missing-fixture/session.jsonl"))];
        assert!(search_rows(
            &rows,
            Path::new("/project"),
            true,
            "needle",
            &mut cache,
            &|| true
        )
        .unwrap()
        .indices
        .is_empty());
        let result = search_rows(
            &rows,
            Path::new("/project"),
            true,
            "needle",
            &mut cache,
            &|| false,
        )
        .unwrap();
        assert_eq!(result.unreadable, 1);
        assert!(result.indices.is_empty());
    }
}
