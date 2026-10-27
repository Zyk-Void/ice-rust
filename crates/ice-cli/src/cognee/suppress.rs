//! Local recall suppression for superseded and expired Cognee facts.
//!
//! Matching uses only the exact SHA-256 hash of normalized fact text (without
//! a `[category]` header). Paraphrased facts returned by the graph are not
//! recognized, and the remote copy is never deleted or rewritten. The bounded
//! loader keeps the 2,000 newest records for one dataset.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config::{create_private_dir, write_private_file_atomic};

pub const MAX_SUPPRESSIONS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuppressionKind {
    Superseded { by: String },
    Expires { at_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suppression {
    pub id: String,
    pub hash: String,
    pub kind: SuppressionKind,
    pub dataset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub recorded_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Superseded,
    Expired,
}

#[derive(Debug, Default)]
pub struct SuppressionSet {
    dataset: Option<String>,
    records: HashMap<String, Vec<Suppression>>,
}

impl SuppressionSet {
    /// Loads the 2,000 newest records for one dataset.
    pub fn load(directory: &Path, dataset: &str) -> Self {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Self::default();
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| {
            std::cmp::Reverse(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned(),
            )
        });
        let mut records: HashMap<String, Vec<Suppression>> = HashMap::new();
        let mut loaded = 0;
        for path in paths {
            if loaded >= MAX_SUPPRESSIONS {
                break;
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<Suppression>(&text) else {
                continue;
            };
            if record.dataset == dataset && valid_hash(&record.hash) {
                records.entry(record.hash.clone()).or_default().push(record);
                loaded += 1;
            }
        }
        Self {
            dataset: Some(dataset.to_owned()),
            records,
        }
    }

    pub fn insert(&mut self, suppression: Suppression) {
        if self
            .dataset
            .as_deref()
            .is_some_and(|dataset| dataset != suppression.dataset)
        {
            return;
        }
        let records = self.records.entry(suppression.hash.clone()).or_default();
        if !records.contains(&suppression) {
            records.push(suppression);
        }
        let count = self.records.values().map(Vec::len).sum::<usize>();
        if count > MAX_SUPPRESSIONS {
            let oldest =
                self.records
                    .iter()
                    .flat_map(|(hash, records)| {
                        records.iter().enumerate().map(move |(index, record)| {
                            (hash.clone(), index, record.recorded_at_ms)
                        })
                    })
                    .min_by_key(|(_, _, recorded_at_ms)| *recorded_at_ms);
            if let Some((hash, index, _)) = oldest {
                if let Some(records) = self.records.get_mut(&hash) {
                    records.remove(index);
                    if records.is_empty() {
                        self.records.remove(&hash);
                    }
                }
            }
        }
    }

    pub fn is_suppressed(&self, text: &str, now_ms: u64, branch: Option<&str>) -> Option<Reason> {
        let hash = content_hash(text);
        self.records.get(&hash)?.iter().find_map(|record| {
            if match record.branch.as_deref() {
                Some(record_branch) => branch == Some(record_branch),
                None => true,
            } {
                match record.kind {
                    SuppressionKind::Superseded { .. } => Some(Reason::Superseded),
                    SuppressionKind::Expires { at_ms } if now_ms >= at_ms => Some(Reason::Expired),
                    SuppressionKind::Expires { .. } => None,
                }
            } else {
                None
            }
        })
    }
}

pub fn content_hash(text: &str) -> String {
    let normalized = normalize_fact(text);
    let mut digest = Sha256::new();
    digest.update(normalized.as_bytes());
    format!("{:x}", digest.finalize())
}

pub fn normalize_fact(text: &str) -> String {
    let trimmed = text.trim();
    let without_header = trimmed
        .strip_prefix('[')
        .and_then(|value| value.find("] ").map(|index| &value[index + 2..]))
        .unwrap_or(trimmed);
    without_header
        .trim()
        .strip_prefix("- ")
        .unwrap_or(without_header.trim())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn record(directory: &Path, suppression: &Suppression) -> std::io::Result<PathBuf> {
    if !valid_hash(&suppression.hash)
        || suppression.id.is_empty()
        || !suppression
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid suppression record",
        ));
    }
    create_private_dir(directory)?;
    let path = directory.join(format!(
        "{:020}-{}.json",
        suppression.recorded_at_ms, suppression.id
    ));
    let text = format!(
        "{}\n",
        serde_json::to_string(suppression).map_err(std::io::Error::other)?
    );
    write_private_file_atomic(&path, &text)?;
    Ok(path)
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suppression(text: &str, kind: SuppressionKind, branch: Option<&str>) -> Suppression {
        Suppression {
            id: uuid::Uuid::new_v4().to_string(),
            hash: content_hash(text),
            kind,
            dataset: "test".into(),
            branch: branch.map(str::to_owned),
            recorded_at_ms: 1,
        }
    }

    #[test]
    fn normalizes_category_header_and_line_format() {
        assert_eq!(
            content_hash("[decision] - Use   SQLite"),
            content_hash("use SQLite")
        );
    }

    #[test]
    fn expiry_and_branch_scope_are_restrictive() {
        let mut set = SuppressionSet::default();
        set.insert(suppression(
            "fact",
            SuppressionKind::Superseded { by: "new".into() },
            Some("feature"),
        ));
        set.insert(suppression(
            "other fact",
            SuppressionKind::Expires { at_ms: 100 },
            None,
        ));
        assert_eq!(set.is_suppressed("fact", 200, None), None);
        assert_eq!(set.is_suppressed("fact", 200, Some("main")), None);
        assert_eq!(
            set.is_suppressed("fact", 200, Some("feature")),
            Some(Reason::Superseded)
        );
        assert_eq!(set.is_suppressed("other fact", 99, None), None);
        assert_eq!(
            set.is_suppressed("other fact", 100, None),
            Some(Reason::Expired)
        );
    }

    #[test]
    fn loader_keeps_newest_records_and_caps_at_two_thousand() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..MAX_SUPPRESSIONS + 8 {
            let mut suppression = suppression(
                &format!("fact-{index}"),
                SuppressionKind::Superseded {
                    by: format!("id-{index}"),
                },
                None,
            );
            suppression.id = format!("{index:08}");
            suppression.recorded_at_ms = index as u64;
            record(directory.path(), &suppression).unwrap();
        }
        let loaded = SuppressionSet::load(directory.path(), "test");
        assert_eq!(
            loaded.records.values().map(Vec::len).sum::<usize>(),
            MAX_SUPPRESSIONS
        );
        assert!(loaded.is_suppressed("fact-2007", 0, None).is_some());
        assert!(loaded.is_suppressed("fact-7", 0, None).is_none());
    }

    #[test]
    fn immutable_records_load_for_dataset_and_are_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let first = suppression(
            "first",
            SuppressionKind::Superseded { by: "a".into() },
            None,
        );
        record(directory.path(), &first).unwrap();
        let loaded = SuppressionSet::load(directory.path(), "test");
        assert_eq!(
            loaded.is_suppressed("first", 1, None),
            Some(Reason::Superseded)
        );
        assert_eq!(
            SuppressionSet::load(directory.path(), "elsewhere")
                .records
                .len(),
            0
        );
        let second = Suppression {
            id: uuid::Uuid::new_v4().to_string(),
            kind: SuppressionKind::Superseded { by: "b".into() },
            ..first.clone()
        };
        record(directory.path(), &second).unwrap();
        let same_fact = SuppressionSet::load(directory.path(), "test");
        assert_eq!(same_fact.records.values().map(Vec::len).sum::<usize>(), 2);

        let concurrent_dir = directory.path().join("concurrent");
        let mut threads = Vec::new();
        for index in 0..8 {
            let path = concurrent_dir.clone();
            threads.push(std::thread::spawn(move || {
                let suppression = Suppression {
                    id: uuid::Uuid::new_v4().to_string(),
                    hash: content_hash(&format!("parallel-{index}")),
                    kind: SuppressionKind::Superseded {
                        by: format!("candidate-{index}"),
                    },
                    dataset: "test".into(),
                    branch: None,
                    recorded_at_ms: index,
                };
                record(&path, &suppression).unwrap();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let files = std::fs::read_dir(&concurrent_dir).unwrap().count();
        assert_eq!(files, 8);
        let concurrent = SuppressionSet::load(&concurrent_dir, "test");
        assert_eq!(concurrent.records.values().map(Vec::len).sum::<usize>(), 8);
    }
}
