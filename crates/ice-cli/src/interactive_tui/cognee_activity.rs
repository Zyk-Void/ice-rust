//! Cognee activity panel: a few rows above the prompt that appear only when
//! Cognee actually did something (recall, saved prompt/answer/trace, improve)
//! and hide again once the activity is stale. Fed by the live, already-redacted
//! observation stream, so it shows exactly what `observations.jsonl` records.

use super::*;
use crate::cognee::observer::{subscribe, Observation, Subscription};
use ice_tui::utils::truncate_to_width;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MAX_ROWS: usize = 5;
/// Rows stay up this long after the last event, then the panel hides.
const STALE_AFTER: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RowState {
    Pending,
    Done,
    Failed,
}

#[derive(Clone, Debug)]
struct Row {
    id: String,
    label: &'static str,
    state: RowState,
    detail: String,
}

#[derive(Default)]
struct Model {
    rows: Vec<Row>,
    last_event: Option<Instant>,
    visible: bool,
}

/// One line of plain text: no ANSI, no control characters, no newlines.
fn flat(text: &str) -> String {
    ice_tui::strip_ansi(text)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Saved entries arrive as JSON text; show what they are (prompt, answer or
/// tool trace) instead of the raw object. Falls back to the flattened text
/// when the preview is not valid JSON (it is capped, so it can be cut short).
fn readable(operation: &str, raw: &str) -> String {
    if matches!(operation, "remember_entry" | "trace") {
        if let Ok(serde_json::Value::Object(entry)) = serde_json::from_str(raw) {
            let text = |key: &str| entry.get(key).and_then(|v| v.as_str()).map(flat);
            if let Some(tool) = text("origin_function") {
                return tool;
            }
            if let Some(answer) = text("answer").filter(|t| !t.is_empty()) {
                return format!("answer · {answer}");
            }
            if let Some(question) = text("question").filter(|t| !t.is_empty()) {
                return format!("prompt · {question}");
            }
        }
    }
    flat(raw)
}

fn label(operation: &str) -> Option<&'static str> {
    match operation {
        "recall" => Some("recall"),
        "remember" | "remember_entry" => Some("saved"),
        "trace" => Some("trace"),
        "improve" => Some("improve"),
        _ => None,
    }
}

impl Model {
    fn position(&self, id: &str) -> Option<usize> {
        self.rows.iter().position(|row| row.id == id)
    }

    fn upsert(&mut self, row: Row) {
        match self.position(&row.id) {
            Some(index) => self.rows[index] = row,
            None => {
                self.rows.push(row);
                if self.rows.len() > MAX_ROWS {
                    self.rows.remove(0);
                }
            }
        }
    }

    /// Returns true when the visible rows changed.
    fn apply(&mut self, obs: &Observation, now: Instant) -> bool {
        let Some(label) = label(&obs.operation) else {
            return false;
        };
        let id = obs.request_id.clone().unwrap_or_else(|| obs.id.clone());
        let preview = obs
            .preview
            .as_deref()
            .map(|raw| readable(&obs.operation, raw))
            .unwrap_or_default();
        let meta_flag = |key: &str| {
            obs.meta
                .as_ref()
                .and_then(|meta| meta.get(key))
                .is_some_and(|value| value.as_bool() == Some(true))
        };
        let checkpoint = meta_flag("checkpoint");
        match (obs.operation.as_str(), obs.phase.as_str()) {
            // A turn-scoped recall starts a new turn: drop the previous one.
            ("recall", "started") => {
                if !checkpoint {
                    self.rows.clear();
                }
                self.upsert(Row {
                    id,
                    label,
                    state: RowState::Pending,
                    detail: format!("“{preview}”"),
                });
            }
            ("recall", "succeeded") => {
                // Nothing was asked of Cognee: don't show a row for it.
                if meta_flag("skipped") || meta_flag("duplicate") || meta_flag("disabled") {
                    self.rows.retain(|row| row.id != id);
                } else {
                    let hits = obs
                        .meta
                        .as_ref()
                        .and_then(|meta| meta.get("hits"))
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let detail = if hits == 0 {
                        "nothing relevant found".to_string()
                    } else {
                        format!("{hits} recalled · {preview}")
                    };
                    self.upsert(Row {
                        id,
                        label,
                        state: RowState::Done,
                        detail,
                    });
                }
            }
            (_, "started") | (_, "queued") => {
                self.upsert(Row {
                    id,
                    label,
                    state: RowState::Pending,
                    detail: preview,
                });
            }
            (_, "succeeded") => {
                let detail = match self.position(&id) {
                    Some(index) => self.rows[index].detail.clone(),
                    None => preview,
                };
                self.upsert(Row {
                    id,
                    label,
                    state: RowState::Done,
                    detail,
                });
            }
            (_, "failed") => {
                let error = obs.error.as_deref().map(flat).unwrap_or_default();
                self.upsert(Row {
                    id,
                    label,
                    state: RowState::Failed,
                    detail: format!("failed: {error}"),
                });
            }
            _ => return false,
        }
        self.last_event = Some(now);
        self.visible = !self.rows.is_empty();
        true
    }

    /// Hide once stale. Returns true when the visibility flipped.
    fn expire(&mut self, now: Instant) -> bool {
        let stale = self
            .last_event
            .is_some_and(|at| now.duration_since(at) > STALE_AFTER);
        if self.visible && stale {
            self.visible = false;
            return true;
        }
        false
    }
}

pub(super) struct CogneeActivity {
    model: Arc<Mutex<Model>>,
    dirty: Arc<AtomicBool>,
    _subscription: Subscription,
}

impl CogneeActivity {
    pub(super) fn new() -> Self {
        let model = Arc::new(Mutex::new(Model::default()));
        let dirty = Arc::new(AtomicBool::new(false));
        let (model_for_feed, dirty_for_feed) = (model.clone(), dirty.clone());
        let subscription = subscribe(move |obs| {
            if let Ok(mut model) = model_for_feed.lock() {
                if model.apply(obs, Instant::now()) {
                    dirty_for_feed.store(true, Ordering::SeqCst);
                }
            }
        });
        Self {
            model,
            dirty,
            _subscription: subscription,
        }
    }

    /// Tick hook: true when the panel needs a repaint.
    pub(super) fn sync(&self) -> bool {
        let expired = self
            .model
            .lock()
            .map(|mut model| model.expire(Instant::now()))
            .unwrap_or(false);
        self.dirty.swap(false, Ordering::SeqCst) || expired
    }
}

impl Component for CogneeActivity {
    fn render(&self, width: usize) -> Vec<String> {
        let Ok(model) = self.model.lock() else {
            return vec![];
        };
        if !model.visible || model.rows.is_empty() {
            return vec![];
        }
        let c = current_theme().colors;
        let mut lines = vec![truncate_to_width(
            &format!("  {}", c.muted.fg("cognee memory")),
            width,
            "",
        )];
        for row in &model.rows {
            let icon = match row.state {
                RowState::Pending => c.muted.fg("…"),
                RowState::Done => c.success.fg("✓"),
                RowState::Failed => c.error.fg("✗"),
            };
            let detail = if row.state == RowState::Failed {
                c.error.fg(&row.detail)
            } else {
                c.dim.fg(&row.detail)
            };
            lines.push(truncate_to_width(
                &format!(
                    "  {icon} {} {detail}",
                    c.accent.fg(&format!("{:<7}", row.label))
                ),
                width,
                "…",
            ));
        }
        lines
    }
    fn invalidate(&self) {}
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

    fn obs(op: &str, phase: &str, id: &str, preview: Option<&str>, meta: Value) -> Observation {
        Observation {
            id: format!("o-{id}-{phase}"),
            operation: op.into(),
            phase: phase.into(),
            request_id: Some(id.into()),
            preview: preview.map(String::from),
            meta: meta.as_object().cloned().or_else(|| Some(Map::new())),
            ..Observation::default()
        }
    }

    #[test]
    fn recall_shows_prompt_then_hits_and_next_turn_clears() {
        let mut model = Model::default();
        let now = Instant::now();
        assert!(model.apply(
            &obs("recall", "started", "r1", Some("fix the\nbuild"), json!({})),
            now
        ));
        assert_eq!(model.rows[0].detail, "“fix the build”");
        assert!(model.apply(
            &obs(
                "recall",
                "succeeded",
                "r1",
                Some("use cargo\ncheck"),
                json!({"hits": 2})
            ),
            now
        ));
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].state, RowState::Done);
        assert_eq!(model.rows[0].detail, "2 recalled · use cargo check");
        model.apply(
            &obs("recall", "started", "r2", Some("next"), json!({})),
            now,
        );
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].id, "r2");
    }

    #[test]
    fn skipped_recall_leaves_no_row_and_unknown_ops_are_ignored() {
        let mut model = Model::default();
        let now = Instant::now();
        model.apply(&obs("recall", "started", "r1", Some("hi"), json!({})), now);
        model.apply(
            &obs("recall", "succeeded", "r1", None, json!({"skipped": true})),
            now,
        );
        assert!(model.rows.is_empty());
        assert!(!model.visible);
        assert!(!model.apply(&obs("session", "succeeded", "s", None, json!({})), now));
    }

    #[test]
    fn failures_and_staleness() {
        let mut model = Model::default();
        let now = Instant::now();
        let mut failed = obs("remember", "failed", "m1", None, json!({}));
        failed.error = Some("timeout".into());
        model.apply(&failed, now);
        assert_eq!(model.rows[0].state, RowState::Failed);
        assert_eq!(model.rows[0].detail, "failed: timeout");
        assert!(!model.expire(now));
        assert!(model.expire(now + STALE_AFTER + Duration::from_secs(1)));
        assert!(!model.visible);
    }

    #[test]
    fn saved_entries_show_what_they_are() {
        let mut model = Model::default();
        let now = Instant::now();
        let qa = r#"{"type":"qa","question":"fix it","answer":"done\nok","context":""}"#;
        model.apply(
            &obs(
                "remember_entry",
                "started",
                "a",
                Some(qa),
                json!({"kind": "answer"}),
            ),
            now,
        );
        let prompt = r#"{"type":"qa","question":"fix it","answer":"","context":""}"#;
        model.apply(
            &obs("remember_entry", "started", "b", Some(prompt), json!({})),
            now,
        );
        let trace = r#"{"type":"trace","origin_function":"bash","status":"ok"}"#;
        model.apply(&obs("trace", "started", "c", Some(trace), json!({})), now);
        let cut = r#"{"type":"qa","question":"cut off"#;
        model.apply(
            &obs("remember_entry", "started", "d", Some(cut), json!({})),
            now,
        );
        let details: Vec<_> = model.rows.iter().map(|r| r.detail.as_str()).collect();
        assert_eq!(
            details,
            [
                "answer · done ok",
                "prompt · fix it",
                "bash",
                r#"{"type":"qa","question":"cut off"#
            ]
        );
    }

    #[test]
    fn rows_are_bounded() {
        let mut model = Model::default();
        let now = Instant::now();
        for i in 0..8 {
            model.apply(
                &obs("trace", "started", &format!("t{i}"), Some("x"), json!({})),
                now,
            );
        }
        assert_eq!(model.rows.len(), MAX_ROWS);
        assert_eq!(model.rows[0].id, "t3");
    }
}
