//! Locations and rows for the cross-project session picker.
//! Registrations are independent atomic files, so concurrent projects do not
//! overwrite one another's index. Session JSONL files keep their existing layout.
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(crate) struct SessionRow {
    pub path: PathBuf,
    pub cwd: PathBuf,
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub modified: SystemTime,
}

pub(crate) fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Remember a visited session directory without scanning unrelated projects.
pub(crate) fn register_directory(agent_dir: &Path, directory: &Path) -> Result<(), String> {
    if !directory.is_dir() {
        return Ok(());
    }
    let directory = canonical(directory);
    let bytes = serde_json::to_vec(&directory).map_err(|error| error.to_string())?;
    let key = format!("{:x}", Sha256::digest(&bytes));
    let index = agent_dir.join("session-directories");
    std::fs::create_dir_all(&index).map_err(|error| error.to_string())?;
    crate::config::atomic_write(&index.join(format!("{key}.json")), &bytes)
        .map_err(|error| error.to_string())
}

pub(crate) fn known_directories(agent_dir: &Path, current: &Path) -> Vec<PathBuf> {
    let mut directories = vec![canonical(current), canonical(&agent_dir.join("sessions"))];
    if let Ok(files) = std::fs::read_dir(agent_dir.join("session-directories")) {
        for entry in files.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let parsed = std::fs::File::open(&path)
                .ok()
                .and_then(|file| serde_json::from_reader::<_, PathBuf>(file.take(64 * 1024)).ok());
            if let Some(directory) = parsed.filter(|directory| directory.is_absolute()) {
                directories.push(canonical(&directory));
            }
        }
    }
    let mut seen = HashSet::new();
    directories.retain(|directory| seen.insert(directory.clone()));
    directories
}

pub(crate) fn display_text(text: &str) -> String {
    ice_tui::strip_ansi(text)
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn load_rows(directories: &[PathBuf]) -> Vec<SessionRow> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for directory in directories {
        for path in crate::session::list_session_files_sync(directory) {
            let path = canonical(&path);
            if !seen.insert(path.clone()) || !path.is_file() {
                continue;
            }
            let Some(row) = read_row(&path) else { continue };
            rows.push(row);
        }
    }
    rows.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.path.cmp(&right.path))
    });
    rows
}

fn read_row(path: &Path) -> Option<SessionRow> {
    let file = std::fs::File::open(path).ok()?;
    let mut header = String::new();
    BufReader::new(file.take(64 * 1024))
        .read_line(&mut header)
        .ok()?;
    let header: serde_json::Value = serde_json::from_str(&header).ok()?;
    let id = header.get("id")?.as_str()?.to_string();
    let cwd = canonical(Path::new(header.get("cwd")?.as_str()?));
    let summary = crate::session::summarize_session_file(path);
    if summary.empty {
        return None;
    }
    Some(SessionRow {
        path: path.to_path_buf(),
        cwd,
        id,
        label: display_text(&crate::session::session_display_label(&summary)),
        bytes: summary.bytes,
        modified: std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(directory: &Path, name: &str, cwd: &Path) -> PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join(name);
        let header = serde_json::json!({"kind":"session","id":"same-id","cwd":cwd});
        let entry = serde_json::json!({"kind":"entry","type":"message","message":{"role":"user","content":"A saved conversation"}});
        std::fs::write(&path, format!("{header}\n{entry}\n")).unwrap();
        path
    }

    #[test]
    fn all_projects_keep_equal_ids_distinct_and_deduplicate_directories() {
        let temp = tempfile::tempdir().unwrap();
        let agent = temp.path().join("agent");
        let first = temp.path().join("project-a");
        let second = temp.path().join("project-b");
        let a = write_session(&first.join(".icers/sessions"), "a.jsonl", &first);
        let b = write_session(&second.join(".icers/sessions"), "b.jsonl", &second);
        register_directory(&agent, a.parent().unwrap()).unwrap();
        register_directory(&agent, b.parent().unwrap()).unwrap();
        register_directory(&agent, a.parent().unwrap()).unwrap();
        let directories = known_directories(&agent, a.parent().unwrap());
        let rows = load_rows(&directories);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.path == canonical(&a)));
        assert!(rows.iter().any(|row| row.path == canonical(&b)));
        assert!(rows.iter().all(|row| row.id == "same-id"));
    }

    #[test]
    fn stale_and_malformed_index_entries_do_not_hide_current_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let current = temp.path().join("project/.icers/sessions");
        write_session(&current, "session.jsonl", temp.path());
        let index = temp.path().join("agent/session-directories");
        std::fs::create_dir_all(&index).unwrap();
        std::fs::write(index.join("broken.json"), "{").unwrap();
        std::fs::write(index.join("relative.json"), "\"relative/path\"").unwrap();
        let rows = load_rows(&known_directories(&temp.path().join("agent"), &current));
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn labels_remove_terminal_control_sequences() {
        assert_eq!(
            display_text("\u{1b}[31mname\u{1b}[0m\nnext\u{7}"),
            "name next"
        );
    }
}
