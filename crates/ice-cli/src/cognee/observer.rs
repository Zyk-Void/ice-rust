//! Cognee memory-operation observations. Ports `ice-cognee-observer.ts`
//! (COG-059, COG-060).
//!
//! Observations go to in-process listeners first, then to
//! `<storage>/observations.jsonl`, which rotates to a single `.1` file past
//! 2 MiB. Logging failures are swallowed: observation never affects the
//! agent loop.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::text::redact_memory_text;

pub const OBSERVATION_FILE: &str = "observations.jsonl";
pub const MAX_EVENTS: usize = 500;
pub const MAX_PREVIEW_CHARS: usize = 900;
/// Bound on the in-process `detail` text.
pub const MAX_DETAIL_CHARS: usize = 64_000;
pub const MAX_OBSERVATION_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// `CogneeObservation`. `operation` is one of session, recall,
/// remember_entry, remember, trace, improve or queue; `phase` is started,
/// queued, succeeded or failed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub id: String,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub operation: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
    /// The whole redacted preview, for in-process listeners (the TUI shows it
    /// in full). Never written to the log, which keeps the capped `preview`.
    #[serde(skip)]
    pub detail: Option<String>,
}

type Listener = Arc<dyn Fn(&Observation) + Send + Sync>;

struct Listeners {
    next_id: u64,
    entries: Vec<(u64, Listener)>,
}

fn listeners() -> &'static Mutex<Listeners> {
    static LISTENERS: OnceLock<Mutex<Listeners>> = OnceLock::new();
    LISTENERS.get_or_init(|| {
        Mutex::new(Listeners {
            next_id: 0,
            entries: Vec::new(),
        })
    })
}

/// Serializes appends so two writers never race the size-check rename.
fn append_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Unsubscribes on drop.
pub struct Subscription(u64);

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Ok(mut listeners) = listeners().lock() {
            listeners.entries.retain(|(id, _)| *id != self.0);
        }
    }
}

/// Live feed of redacted observations; no replay on subscribe.
pub fn subscribe(listener: impl Fn(&Observation) + Send + Sync + 'static) -> Subscription {
    let mut listeners = listeners().lock().expect("observation listeners");
    let id = listeners.next_id;
    listeners.next_id += 1;
    listeners.entries.push((id, Arc::new(listener)));
    Subscription(id)
}

/// `appendCogneeObservation`: redact the preview, notify, then append.
pub fn append_observation(storage_dir: &Path, observation: Observation) {
    let safe = publish_observation(observation);
    persist_observation(storage_dir, &safe);
}

/// Redact the preview and notify listeners on the calling thread, so they see
/// observations in emission order. Returns the redacted observation to persist.
pub fn publish_observation(observation: Observation) -> Observation {
    let mut safe = observation;
    safe.preview = safe
        .preview
        .map(|preview| redact_memory_text(&preview, MAX_PREVIEW_CHARS));
    let current: Vec<Listener> = listeners()
        .lock()
        .map(|listeners| listeners.entries.iter().map(|(_, l)| l.clone()).collect())
        .unwrap_or_default();
    for listener in current {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| listener(&safe)));
    }
    safe
}

/// Append an already-published observation to the log.
pub fn persist_observation(storage_dir: &Path, safe: &Observation) {
    let _guard = append_lock().lock();
    let _ = write_observation(storage_dir, safe);
}

fn write_observation(storage_dir: &Path, safe: &Observation) -> std::io::Result<()> {
    super::config::create_private_dir(storage_dir)?;
    let path = storage_dir.join(OBSERVATION_FILE);
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let line = format!("{}\n", serde_json::to_string(safe)?);
    use std::io::Write;
    options.open(&path)?.write_all(line.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    if std::fs::metadata(&path)?.len() > MAX_OBSERVATION_FILE_BYTES {
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        std::fs::rename(&path, rotated)?;
    }
    Ok(())
}

/// `readCogneeObservations`: the last `MAX_EVENTS` lines across the current
/// and previous file, keeping objects with string id/at/operation/phase.
pub fn read_observations(storage_dir: &Path) -> Vec<Observation> {
    let read_lines = |path: &Path| -> Vec<String> {
        std::fs::read_to_string(path)
            .map(|text| {
                text.split('\n')
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let path = storage_dir.join(OBSERVATION_FILE);
    let mut lines = read_lines(&path);
    if lines.len() < MAX_EVENTS {
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        let mut previous = read_lines(Path::new(&rotated));
        previous.append(&mut lines);
        lines = previous;
    }
    let start = lines.len().saturating_sub(MAX_EVENTS);
    lines[start..]
        .iter()
        .filter_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            let object = value.as_object()?;
            let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_string);
            let mut observation: Observation =
                serde_json::from_value(value.clone()).unwrap_or_default();
            // Only the four required fields are validated.
            observation.id = text("id")?;
            observation.at = text("at")?;
            observation.operation = text("operation")?;
            observation.phase = text("phase")?;
            Some(observation)
        })
        .collect()
}

/// The configured endpoint without credentials, query or fragment.
pub fn sanitize_endpoint(base_url: &str) -> String {
    match reqwest::Url::parse(base_url) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "configured".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, preview: Option<&str>) -> Observation {
        Observation {
            id: id.into(),
            at: "2026-01-01T00:00:00.000Z".into(),
            operation: "recall".into(),
            phase: "started".into(),
            preview: preview.map(str::to_string),
            ..Observation::default()
        }
    }

    #[test]
    fn full_detail_text_is_never_logged() {
        let dir = tempfile::tempdir().unwrap();
        let mut observation = event("obs-detail", Some("short"));
        observation.detail = Some("FULL_DETAIL_SENTINEL".into());
        append_observation(dir.path(), observation);
        let logged = std::fs::read_to_string(dir.path().join(OBSERVATION_FILE)).unwrap();
        assert!(logged.contains("short"), "{logged}");
        assert!(!logged.contains("FULL_DETAIL_SENTINEL"), "{logged}");
    }

    #[test]
    fn appends_redact_notify_and_replay() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let storage = dir.path().to_path_buf();
        let subscription = subscribe(move |observation| {
            if observation.id.starts_with("obs-test") {
                sink.lock().unwrap().push(observation.preview.clone());
            }
        });
        append_observation(&storage, event("obs-test-1", Some("token=abc")));
        drop(subscription);
        append_observation(&storage, event("obs-test-2", None));
        assert_eq!(
            *seen.lock().unwrap(),
            vec![Some("token=[REDACTED]".to_string())]
        );
        std::fs::OpenOptions::new()
            .append(true)
            .open(storage.join(OBSERVATION_FILE))
            .and_then(|mut file| {
                use std::io::Write;
                file.write_all(b"{\"id\":1}\nnot json\n")
            })
            .unwrap();
        let events = read_observations(&storage);
        assert_eq!(
            events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["obs-test-1", "obs-test-2"]
        );
        let line = std::fs::read_to_string(storage.join(OBSERVATION_FILE)).unwrap();
        assert!(line.starts_with(
            "{\"id\":\"obs-test-1\",\"at\":\"2026-01-01T00:00:00.000Z\",\"operation\":\"recall\",\"phase\":\"started\",\"preview\":\"token=[REDACTED]\"}\n"
        ));
    }

    #[test]
    fn rotation_keeps_one_generation_and_replay_fills_from_it() {
        let dir = tempfile::tempdir().unwrap();
        // Metadata is not capped, so a few large events fill the file.
        let mut meta = Map::new();
        meta.insert("blob".into(), Value::String("x".repeat(64 * 1024)));
        let mut count = 0;
        while !dir.path().join("observations.jsonl.1").exists() {
            let mut observation = event(&format!("e{count}"), None);
            observation.meta = Some(meta.clone());
            append_observation(dir.path(), observation);
            count += 1;
        }
        for index in 0..3 {
            append_observation(dir.path(), event(&format!("after{index}"), None));
        }
        let events = read_observations(dir.path());
        assert_eq!(events.len(), count + 3);
        assert_eq!(events[0].id, "e0");
        assert_eq!(events.last().unwrap().id, "after2");
        assert!(
            std::fs::metadata(dir.path().join("observations.jsonl.1"))
                .unwrap()
                .len()
                > MAX_OBSERVATION_FILE_BYTES
        );
    }

    #[test]
    fn endpoints_lose_credentials() {
        assert_eq!(
            sanitize_endpoint("http://user:pw@host:1/p?q=1#h"),
            "http://host:1/p"
        );
        assert_eq!(sanitize_endpoint("nope"), "configured");
    }
}
