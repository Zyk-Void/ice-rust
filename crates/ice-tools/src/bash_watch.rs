//! Mirrors `packages/coding-agent/src/core/tools/bash-watch.ts` — bounded
//! watches on running background shells and the `bash_watch` tool.
//!
//! A watch fires an event (delivered to the session, which decides how to
//! wake the model) when a literal phrase appears in new output, when a shell
//! has been quiet for a while, on a fixed interval, or — as an automatic
//! default — when a quiet shell's last line looks like an interactive
//! prompt. Each watch has a lifetime fire budget; it is removed when the
//! budget is spent or its shell stops running. Explicit and automatic
//! watches share these semantics; only explicit ones count toward the limit.
//!
//! Lengths ice measures in UTF-16 code units are measured in Unicode scalar
//! values here.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, TextContentOrImage, ToolResultPartial};
use ice_ai::types::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::bash_background::{BackgroundShellStatus, BashBackgroundRegistry, ListenerId};

/// Custom message type for fire delivery. Mirrors `BASH_WATCH_FIRED_CUSTOM_TYPE`.
pub const BASH_WATCH_FIRED_CUSTOM_TYPE: &str = "bash_watch_fired";
/// Explicit watches allowed at once. Mirrors `MAX_BASH_WATCHES`.
pub const MAX_BASH_WATCHES: usize = 16;
/// Largest lifetime fire budget. Mirrors `MAX_WATCH_FIRES`.
pub const MAX_WATCH_FIRES: u32 = 10;
/// Shortest explicit silence window. Mirrors `MIN_SILENCE_MS`.
pub const MIN_SILENCE: Duration = Duration::from_millis(5000);
/// Shortest explicit interval. Mirrors `MIN_INTERVAL_MS`.
pub const MIN_INTERVAL: Duration = Duration::from_millis(30_000);
/// Longest literal pattern (characters).
const MAX_PATTERN_CHARS: usize = 200;
/// Characters of the matching line quoted in a pattern fire.
const PATTERN_REASON_CHARS: usize = 300;
/// Characters of the prompt line quoted in a prompt fire.
const PROMPT_REASON_CHARS: usize = 200;
/// Fire tails: last 20 lines, then last 2000 characters.
const TAIL_LINES: usize = 20;
const TAIL_CHARS: usize = 2000;

fn escape_pattern_for_reason(pattern: &str) -> String {
    pattern.chars().flat_map(char::escape_default).collect()
}

/// What a watch reacts to. `Prompt` is internal (automatic defaults only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BashWatchKind {
    Pattern,
    Silence,
    Interval,
    #[schemars(skip)]
    Prompt,
}

impl BashWatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pattern => "pattern",
            Self::Silence => "silence",
            Self::Interval => "interval",
            Self::Prompt => "prompt",
        }
    }
}

/// When a disarmed watch arms again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BashWatchRearm {
    Manual,
    OnActivity,
}

/// Automatic watch defaults for new shells (0 disables). Mirrors
/// `BashWatchDefaults`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BashWatchDefaults {
    pub silence_alert: Duration,
    pub prompt_alert: Duration,
}

/// A watch's public state. Mirrors `BashWatchSnapshot`.
#[derive(Debug, Clone, Serialize)]
pub struct BashWatchSnapshot {
    pub id: String,
    pub target: String,
    pub kind: BashWatchKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    pub max_fires: u32,
    pub fires: u32,
    pub armed: bool,
    pub rearm: BashWatchRearm,
    pub auto: bool,
}

/// One watch alert. Mirrors `BashWatchFire`.
#[derive(Debug, Clone, Serialize)]
pub struct BashWatchFire {
    pub watch_id: String,
    pub target: String,
    pub kind: BashWatchKind,
    pub reason: String,
    pub tail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    pub auto: bool,
    /// `<watchId>#<sequence>`, unique per fire.
    pub fire_id: String,
}

struct Watch {
    id: String,
    target: String,
    kind: BashWatchKind,
    pattern: Option<String>,
    period: Option<Duration>,
    max_fires: u32,
    fires: u32,
    sequence: u64,
    armed: bool,
    rearm: BashWatchRearm,
    auto: bool,
    carry: String,
    timer: Option<tokio::task::JoinHandle<()>>,
    /// Bumped on every (re)schedule so a stale timer does nothing.
    generation: u64,
}

impl Watch {
    fn snapshot(&self) -> BashWatchSnapshot {
        BashWatchSnapshot {
            id: self.id.clone(),
            target: self.target.clone(),
            kind: self.kind,
            pattern: self.pattern.clone(),
            silence_ms: matches!(self.kind, BashWatchKind::Silence | BashWatchKind::Prompt)
                .then(|| self.period.map(|p| p.as_millis() as u64))
                .flatten(),
            interval_ms: (self.kind == BashWatchKind::Interval)
                .then(|| self.period.map(|p| p.as_millis() as u64))
                .flatten(),
            max_fires: self.max_fires,
            fires: self.fires,
            armed: self.armed,
            rearm: self.rearm,
            auto: self.auto,
        }
    }

    fn cancel_timer(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
        self.generation += 1;
    }
}

/// A requested watch, validated by [`BashWatchRegistry::create`].
#[derive(Debug, Clone)]
pub struct BashWatchSpec {
    pub target: String,
    pub kind: BashWatchKind,
    pub pattern: Option<String>,
    pub period: Option<Duration>,
    pub max_fires: u32,
    pub rearm: BashWatchRearm,
}

type FireListener = Arc<dyn Fn(&BashWatchFire) + Send + Sync>;
type ChangeListener = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Inner {
    watches: Vec<Watch>,
    next_id: u64,
    fire_listeners: Vec<(u64, FireListener)>,
    change_listeners: Vec<(u64, ChangeListener)>,
    next_listener: u64,
    disposed: bool,
}

/// The per-session watch registry. Mirrors `BashWatchRegistry`.
pub struct BashWatchRegistry {
    inner: Mutex<Inner>,
    shells: Arc<BashBackgroundRegistry>,
    defaults: Option<Arc<dyn Fn() -> BashWatchDefaults + Send + Sync>>,
    shell_listeners: Mutex<Vec<ListenerId>>,
}

impl BashWatchRegistry {
    /// Create a registry subscribed to `shells`. `defaults` supplies the
    /// automatic prompt/silence watches installed on every new shell.
    pub fn new(
        shells: Arc<BashBackgroundRegistry>,
        defaults: Option<Arc<dyn Fn() -> BashWatchDefaults + Send + Sync>>,
    ) -> Arc<Self> {
        let registry = Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            shells: shells.clone(),
            defaults,
            shell_listeners: Mutex::new(Vec::new()),
        });
        let weak = Arc::downgrade(&registry);
        let on_start = {
            let weak = weak.clone();
            shells.on_start(move |id| {
                if let Some(r) = weak.upgrade() {
                    r.add_defaults(id);
                }
            })
        };
        let on_output = {
            let weak = weak.clone();
            shells.on_output(move |id, text| {
                if let Some(r) = weak.upgrade() {
                    r.handle_output(id, text);
                }
            })
        };
        let on_change = shells.on_change(move || {
            if let Some(r) = weak.upgrade() {
                r.handle_shell_change();
            }
        });
        *registry
            .shell_listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = vec![on_start, on_output, on_change];
        registry
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn on_fire(&self, f: impl Fn(&BashWatchFire) + Send + Sync + 'static) -> u64 {
        let mut inner = self.lock();
        inner.next_listener += 1;
        let id = inner.next_listener;
        inner.fire_listeners.push((id, Arc::new(f)));
        id
    }

    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) -> u64 {
        let mut inner = self.lock();
        inner.next_listener += 1;
        let id = inner.next_listener;
        inner.change_listeners.push((id, Arc::new(f)));
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        let mut inner = self.lock();
        inner.fire_listeners.retain(|(l, _)| *l != id);
        inner.change_listeners.retain(|(l, _)| *l != id);
    }

    fn notify_change(&self) {
        let listeners: Vec<ChangeListener> = self
            .lock()
            .change_listeners
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for listener in listeners {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| listener()));
        }
    }

    fn deliver(&self, fires: Vec<BashWatchFire>) {
        if fires.is_empty() {
            return;
        }
        let listeners: Vec<FireListener> = self
            .lock()
            .fire_listeners
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for fire in &fires {
            for listener in &listeners {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| listener(fire)));
            }
        }
        self.notify_change();
    }

    /// Watches in creation order, optionally only those on `shell`.
    pub fn list(&self, shell: Option<&str>) -> Vec<BashWatchSnapshot> {
        self.lock()
            .watches
            .iter()
            .filter(|w| shell.is_none_or(|s| w.target == s))
            .map(Watch::snapshot)
            .collect()
    }

    /// Add an explicit watch on a running shell.
    pub fn create(self: &Arc<Self>, spec: BashWatchSpec) -> Result<BashWatchSnapshot, String> {
        let Some(shell) = self.shells.get(&spec.target) else {
            return Err(self.shells.unknown_shell_error(&spec.target));
        };
        if shell.status != BackgroundShellStatus::Running {
            return Err(format!(
                "{} is {}; watches only apply to running shells.",
                spec.target,
                shell.status.as_str()
            ));
        }
        self.insert(spec, false)
    }

    fn insert(
        self: &Arc<Self>,
        spec: BashWatchSpec,
        auto: bool,
    ) -> Result<BashWatchSnapshot, String> {
        let snapshot = {
            let mut inner = self.lock();
            if !auto && inner.watches.iter().filter(|watch| !watch.auto).count() >= MAX_BASH_WATCHES
            {
                return Err(format!(
                    "{MAX_BASH_WATCHES} watches already exist. Cancel ones you no longer need with bash_watch action \"cancel\" first."
                ));
            }
            inner.next_id += 1;
            let watch = Watch {
                id: format!("watch-{}", inner.next_id),
                target: spec.target,
                kind: spec.kind,
                pattern: spec.pattern,
                period: spec.period,
                max_fires: spec.max_fires,
                fires: 0,
                sequence: 0,
                armed: true,
                rearm: spec.rearm,
                auto,
                carry: String::new(),
                timer: None,
                generation: 0,
            };
            let snapshot = watch.snapshot();
            inner.watches.push(watch);
            snapshot
        };
        self.schedule(&snapshot.id);
        self.notify_change();
        Ok(snapshot)
    }

    /// Remove a watch. Mirrors `cancel`.
    pub fn cancel(&self, watch_id: &str) -> Result<(), String> {
        {
            let mut inner = self.lock();
            let Some(index) = inner.watches.iter().position(|w| w.id == watch_id) else {
                return Err(format!("Unknown watch \"{watch_id}\"."));
            };
            let mut watch = inner.watches.remove(index);
            watch.cancel_timer();
        }
        self.notify_change();
        Ok(())
    }

    /// Arm a watch again without resetting its lifetime fire count. Mirrors
    /// `rearm`; a silence watch rearmed after a long quiet can fire at once.
    pub fn rearm(self: &Arc<Self>, watch_id: &str) -> Result<BashWatchSnapshot, String> {
        let snapshot = {
            let mut inner = self.lock();
            let Some(watch) = inner.watches.iter_mut().find(|w| w.id == watch_id) else {
                return Err(format!(
                    "Unknown watch \"{watch_id}\". A watch that used all its fires is removed."
                ));
            };
            watch.armed = true;
            watch.carry.clear();
            watch.snapshot()
        };
        self.schedule(watch_id);
        self.notify_change();
        Ok(snapshot)
    }

    /// Install the automatic prompt, then silence, watches on a new shell.
    fn add_defaults(self: &Arc<Self>, shell_id: &str) {
        let Some(defaults) = self.defaults.as_ref().map(|f| f()) else {
            return;
        };
        for (kind, period) in [
            (BashWatchKind::Prompt, defaults.prompt_alert),
            (BashWatchKind::Silence, defaults.silence_alert),
        ] {
            if period.is_zero() {
                continue;
            }
            let _ = self.insert(
                BashWatchSpec {
                    target: shell_id.to_string(),
                    kind,
                    pattern: None,
                    period: Some(period),
                    max_fires: 3,
                    rearm: BashWatchRearm::OnActivity,
                },
                true,
            );
        }
    }

    fn handle_shell_change(&self) {
        let removed = {
            let mut inner = self.lock();
            let before = inner.watches.len();
            let shells = &self.shells;
            inner.watches.retain_mut(|w| {
                let running = shells
                    .get(&w.target)
                    .is_some_and(|s| s.status == BackgroundShellStatus::Running);
                if !running {
                    w.cancel_timer();
                }
                running
            });
            before != inner.watches.len()
        };
        if removed {
            self.notify_change();
        }
    }

    fn handle_output(self: &Arc<Self>, shell_id: &str, text: &str) {
        let mut fires = Vec::new();
        let mut reschedule = Vec::new();
        {
            let mut inner = self.lock();
            let mut finished = Vec::new();
            for watch in inner.watches.iter_mut().filter(|w| w.target == shell_id) {
                match watch.kind {
                    BashWatchKind::Pattern => {
                        if !watch.armed {
                            // on_activity: this chunk rearms; it is not matched.
                            if watch.rearm == BashWatchRearm::OnActivity {
                                watch.armed = true;
                                watch.carry.clear();
                            }
                            continue;
                        }
                        let pattern = watch.pattern.clone().unwrap_or_default();
                        let haystack = format!("{}{text}", watch.carry);
                        if haystack.contains(&pattern) {
                            watch.carry.clear();
                            watch.armed = false;
                            let output = self
                                .shells
                                .get(shell_id)
                                .map(|s| s.output)
                                .unwrap_or_default();
                            let line =
                                matching_line(&output, &pattern).unwrap_or_else(|| pattern.clone());
                            let reason = format!(
                                "Output matched \"{}\": {}",
                                escape_pattern_for_reason(&pattern),
                                clip(line.trim(), PATTERN_REASON_CHARS)
                            );
                            if let Some(fire) = record_fire(watch, reason) {
                                fires.push(fire);
                            }
                            if watch.fires >= watch.max_fires {
                                finished.push(watch.id.clone());
                            }
                        } else {
                            let keep = pattern.chars().count().saturating_sub(1);
                            watch.carry = last_chars(&haystack, keep);
                        }
                    }
                    BashWatchKind::Silence | BashWatchKind::Prompt => {
                        if !watch.armed {
                            if watch.rearm == BashWatchRearm::OnActivity {
                                watch.armed = true;
                                reschedule.push(watch.id.clone());
                            }
                        } else {
                            reschedule.push(watch.id.clone());
                        }
                    }
                    // Output does not reset an interval's cadence.
                    BashWatchKind::Interval => {}
                }
            }
            // Removal happens before listener delivery.
            for id in &finished {
                if let Some(index) = inner.watches.iter().position(|w| &w.id == id) {
                    let mut watch = inner.watches.remove(index);
                    watch.cancel_timer();
                }
            }
        }
        let fires = self.complete_fires(fires);
        for id in reschedule {
            self.schedule(&id);
        }
        self.deliver(fires);
    }

    /// Fill in the shell tail and log path for pending fires.
    fn complete_fires(&self, fires: Vec<(String, BashWatchFire)>) -> Vec<BashWatchFire> {
        fires
            .into_iter()
            .map(|(target, mut fire)| {
                if let Some(shell) = self.shells.get(&target) {
                    fire.tail = fire_tail(&shell.output);
                    fire.log_path = shell.log_path;
                }
                fire
            })
            .collect()
    }

    /// (Re)start the timer for a time-based watch. Mirrors `schedule`.
    fn schedule(self: &Arc<Self>, watch_id: &str) {
        let mut inner = self.lock();
        let Some(watch) = inner.watches.iter_mut().find(|w| w.id == watch_id) else {
            return;
        };
        watch.cancel_timer();
        if !watch.armed {
            return;
        }
        let Some(period) = watch.period else {
            return;
        };
        let generation = watch.generation;
        let id = watch.id.clone();
        let weak: Weak<Self> = Arc::downgrade(self);
        let delay = match watch.kind {
            BashWatchKind::Interval => period,
            BashWatchKind::Silence | BashWatchKind::Prompt => {
                let quiet = self
                    .shells
                    .get(&watch.target)
                    .map(|s| s.since_output)
                    .unwrap_or_default();
                period.saturating_sub(quiet)
            }
            BashWatchKind::Pattern => return,
        };
        watch.timer = Some(tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Some(registry) = weak.upgrade() {
                registry.timer_fired(&id, generation);
            }
        }));
    }

    fn timer_fired(self: &Arc<Self>, watch_id: &str, generation: u64) {
        let mut fires = Vec::new();
        let mut reschedule = false;
        {
            let mut inner = self.lock();
            if inner.disposed {
                return;
            }
            let Some(index) = inner.watches.iter().position(|w| w.id == watch_id) else {
                return;
            };
            let shell = {
                let watch = &inner.watches[index];
                if watch.generation != generation || !watch.armed {
                    return;
                }
                self.shells.get(&watch.target)
            };
            let Some(shell) = shell.filter(|s| s.status == BackgroundShellStatus::Running) else {
                return;
            };
            let watch = &mut inner.watches[index];
            watch.timer = None;
            match watch.kind {
                BashWatchKind::Interval => {
                    let reason = format!(
                        "Interval check: {} has been running for {}.",
                        watch.target,
                        crate::bash_background::format_runtime(shell.runtime)
                    );
                    if let Some(fire) = record_fire(watch, reason) {
                        fires.push(fire);
                    }
                    reschedule = true;
                }
                BashWatchKind::Silence => {
                    let reason = format!(
                        "No output from {} for {}s.",
                        watch.target,
                        shell.since_output.as_secs_f64().round() as u64
                    );
                    watch.armed = false;
                    if let Some(fire) = record_fire(watch, reason) {
                        fires.push(fire);
                    }
                }
                BashWatchKind::Prompt => match last_nonempty_line(&shell.output) {
                    Some(line) if looks_like_prompt(&line) => {
                        let reason = format!(
                            "{} looks like it is waiting for input: \"{}\". If it should not be interactive, stop it with bash_kill and re-run it non-interactively.",
                            watch.target,
                            clip(line.trim(), PROMPT_REASON_CHARS)
                        );
                        watch.armed = false;
                        if let Some(fire) = record_fire(watch, reason) {
                            fires.push(fire);
                        }
                    }
                    // Not a prompt: check again a full period later, staying
                    // armed and spending no budget.
                    _ => reschedule = true,
                },
                BashWatchKind::Pattern => {}
            }
            if watch.fires >= watch.max_fires {
                let mut watch = inner.watches.remove(index);
                watch.cancel_timer();
                reschedule = false;
            }
        }
        let fires = self.complete_fires(fires);
        if reschedule {
            let mut inner = self.lock();
            if let Some(watch) = inner.watches.iter_mut().find(|w| w.id == watch_id) {
                // A prompt re-check waits a full period from now.
                if watch.kind == BashWatchKind::Prompt {
                    let period = watch.period.unwrap_or_default();
                    let generation = {
                        watch.generation += 1;
                        watch.generation
                    };
                    let id = watch.id.clone();
                    let weak = Arc::downgrade(self);
                    watch.timer = Some(tokio::spawn(async move {
                        tokio::time::sleep(period).await;
                        if let Some(registry) = weak.upgrade() {
                            registry.timer_fired(&id, generation);
                        }
                    }));
                    drop(inner);
                } else {
                    drop(inner);
                    self.schedule(watch_id);
                }
            }
        }
        self.deliver(fires);
    }

    /// Stop all timers, drop watches and listeners, unsubscribe from shells.
    pub fn dispose(&self) {
        {
            let mut inner = self.lock();
            inner.disposed = true;
            for watch in &mut inner.watches {
                watch.cancel_timer();
            }
            inner.watches.clear();
            inner.fire_listeners.clear();
            inner.change_listeners.clear();
        }
        for id in self
            .shell_listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            self.shells.unsubscribe(id);
        }
    }
}

/// Count a fire and build its event (tail filled in later). Returns the
/// target with the fire.
fn record_fire(watch: &mut Watch, reason: String) -> Option<(String, BashWatchFire)> {
    watch.fires += 1;
    watch.sequence += 1;
    Some((
        watch.target.clone(),
        BashWatchFire {
            watch_id: watch.id.clone(),
            target: watch.target.clone(),
            kind: watch.kind,
            reason,
            tail: String::new(),
            log_path: None,
            auto: watch.auto,
            fire_id: format!("{}#{}", watch.id, watch.sequence),
        },
    ))
}

/// The retained line holding the last occurrence of `pattern`.
fn matching_line(output: &str, pattern: &str) -> Option<String> {
    let at = output.rfind(pattern)?;
    let start = output[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = output[at..].find('\n').map_or(output.len(), |i| at + i);
    Some(output[start..end].to_string())
}

fn last_nonempty_line(output: &str) -> Option<String> {
    output
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::to_string)
}

/// ice's interactive-prompt heuristic (case-insensitive): y/n or yes/no
/// choices, "press enter/return/any key", continue?/proceed?/overwrite?,
/// password or passphrase prompts ending in a colon, or a trailing "?".
pub fn looks_like_prompt(line: &str) -> bool {
    static PROMPT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = PROMPT.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(\by/n\b|\byes/no\b|[\[(]\s*y(es)?\s*/\s*n(o)?\s*[\])]|press (enter|return|any key)|\b(continue|proceed|overwrite)\s*\?|\b(password|passphrase)\b[^:]*:\s*$|\?\s*$)",
        )
        .expect("valid prompt heuristic")
    });
    re.is_match(line.trim_end())
}

/// Event tail: trailing whitespace removed, last 20 lines, last 2000 chars.
fn fire_tail(output: &str) -> String {
    let trimmed = output.trim_end();
    let lines: Vec<&str> = trimmed.split('\n').collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    last_chars(&lines[start..].join("\n"), TAIL_CHARS)
}

fn last_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// One-line description used by `bash_watch list`.
fn describe(w: &BashWatchSnapshot) -> String {
    let what = match w.kind {
        BashWatchKind::Pattern => format!("pattern \"{}\"", w.pattern.as_deref().unwrap_or("")),
        BashWatchKind::Silence => format!("silence {}s", w.silence_ms.unwrap_or(0) / 1000),
        BashWatchKind::Prompt => format!("prompt {}s", w.silence_ms.unwrap_or(0) / 1000),
        BashWatchKind::Interval => format!("interval {}s", w.interval_ms.unwrap_or(0) / 1000),
    };
    let state = if w.armed { "armed" } else { "waiting to rearm" };
    let rearm = match w.rearm {
        BashWatchRearm::Manual => "manual rearm",
        BashWatchRearm::OnActivity => "rearms on activity",
    };
    let default = if w.auto { ", default" } else { "" };
    format!(
        "{} on {}: {what}, fired {}/{}, {state}, {rearm}{default}",
        w.id, w.target, w.fires, w.max_fires
    )
}

/// Input for `bash_watch`. Mirrors `BashWatchInput`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BashWatchInput {
    /// create, list, cancel or rearm.
    pub action: WatchAction,
    /// Background shell id (create; optional filter for list).
    #[serde(default)]
    pub shell: Option<String>,
    /// Watch id (cancel, rearm).
    #[serde(default)]
    pub watch_id: Option<String>,
    /// pattern (a literal phrase), silence (no output for a while) or
    /// interval (periodic).
    #[serde(default)]
    pub kind: Option<BashWatchKind>,
    /// Literal, case-sensitive text to watch for (kind pattern).
    #[serde(default)]
    pub pattern: Option<String>,
    /// Quiet seconds before alerting (kind silence, at least 5).
    #[serde(default)]
    pub silence_seconds: Option<f64>,
    /// Seconds between alerts (kind interval, at least 30).
    #[serde(default)]
    pub interval_seconds: Option<f64>,
    /// Lifetime alerts before the watch is removed (1-10; default 1 for
    /// pattern, 3 otherwise).
    #[serde(default)]
    pub max_fires: Option<f64>,
    /// manual, or on_activity (rearm on the next output; default for
    /// silence).
    #[serde(default)]
    pub rearm: Option<BashWatchRearm>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WatchAction {
    Create,
    List,
    Cancel,
    Rearm,
}

/// The `bash_watch` tool. Mirrors `createBashWatchTool`.
pub fn create_bash_watch_tool(watches: Arc<BashWatchRegistry>) -> Arc<dyn AgentTool> {
    Arc::new(BashWatchTool {
        schema: Tool {
            name: "bash_watch".to_string(),
            description: "Watch a running background shell and get alerted without polling: when a literal phrase appears in its output (pattern), when it has been quiet for a while (silence), or periodically (interval). Actions: create, list, cancel, rearm. Watches end when their shell stops or their alerts are used up.".to_string(),
            parameters: ice_ai::types::Schema::new(
                serde_json::to_value(schemars::schema_for!(BashWatchInput)).unwrap_or_default(),
            ),
            constrained_sampling: None,
        },
        watches,
    })
}

struct BashWatchTool {
    schema: Tool,
    watches: Arc<BashWatchRegistry>,
}

fn watch_result(text: String, watches: Vec<BashWatchSnapshot>) -> AgentToolResult {
    AgentToolResult {
        content: vec![TextContentOrImage::text(text)],
        details: serde_json::json!({ "watches": watches }),
        ..Default::default()
    }
}

fn seconds(value: Option<f64>, name: &str, min: Duration) -> Result<Duration, AgentError> {
    let Some(secs) = value else {
        return Err(AgentError::Tool(format!(
            "{name} is required for this kind."
        )));
    };
    let ms = secs * 1000.0;
    if !ms.is_finite() || ms < min.as_millis() as f64 {
        return Err(AgentError::Tool(format!(
            "{name} must be at least {}.",
            min.as_secs()
        )));
    }
    Ok(Duration::from_millis(ms as u64))
}

#[async_trait]
impl AgentTool for BashWatchTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "bash_watch"
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        _signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: BashWatchInput = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("bash_watch input invalid: {e}")))?;
        match input.action {
            WatchAction::List => {
                let watches = self.watches.list(input.shell.as_deref());
                let text = if watches.is_empty() {
                    "No watches.".to_string()
                } else {
                    watches.iter().map(describe).collect::<Vec<_>>().join("\n")
                };
                Ok(watch_result(text, watches))
            }
            WatchAction::Cancel => {
                let id = input
                    .watch_id
                    .ok_or_else(|| AgentError::Tool("watch_id is required.".into()))?;
                self.watches.cancel(&id).map_err(AgentError::Tool)?;
                Ok(watch_result(format!("Cancelled {id}."), Vec::new()))
            }
            WatchAction::Rearm => {
                let id = input
                    .watch_id
                    .ok_or_else(|| AgentError::Tool("watch_id is required.".into()))?;
                let snapshot = self.watches.rearm(&id).map_err(AgentError::Tool)?;
                Ok(watch_result(
                    format!("Rearmed {}.", describe(&snapshot)),
                    vec![snapshot],
                ))
            }
            WatchAction::Create => {
                let shell = input
                    .shell
                    .ok_or_else(|| AgentError::Tool("shell is required.".into()))?;
                let kind = input
                    .kind
                    .filter(|k| *k != BashWatchKind::Prompt)
                    .ok_or_else(|| {
                        AgentError::Tool("kind must be pattern, silence or interval.".into())
                    })?;
                let max_fires = match input.max_fires {
                    None => {
                        if kind == BashWatchKind::Pattern {
                            1
                        } else {
                            3
                        }
                    }
                    Some(n) if n.fract() == 0.0 && (1.0..=MAX_WATCH_FIRES as f64).contains(&n) => {
                        n as u32
                    }
                    Some(_) => {
                        return Err(AgentError::Tool(format!(
                            "max_fires must be an integer from 1 to {MAX_WATCH_FIRES}."
                        )))
                    }
                };
                // Only fields relevant to the kind are kept.
                let (pattern, period) = match kind {
                    BashWatchKind::Pattern => {
                        let pattern = input.pattern.filter(|p| !p.is_empty()).ok_or_else(|| {
                            AgentError::Tool("pattern must be a non-empty string.".into())
                        })?;
                        if pattern.chars().count() > MAX_PATTERN_CHARS {
                            return Err(AgentError::Tool(format!(
                                "pattern is at most {MAX_PATTERN_CHARS} characters."
                            )));
                        }
                        (Some(pattern), None)
                    }
                    BashWatchKind::Silence => (
                        None,
                        Some(seconds(
                            input.silence_seconds,
                            "silence_seconds",
                            MIN_SILENCE,
                        )?),
                    ),
                    BashWatchKind::Interval => (
                        None,
                        Some(seconds(
                            input.interval_seconds,
                            "interval_seconds",
                            MIN_INTERVAL,
                        )?),
                    ),
                    BashWatchKind::Prompt => unreachable!("rejected above"),
                };
                let rearm = input.rearm.unwrap_or(if kind == BashWatchKind::Silence {
                    BashWatchRearm::OnActivity
                } else {
                    BashWatchRearm::Manual
                });
                let snapshot = self
                    .watches
                    .create(BashWatchSpec {
                        target: shell,
                        kind,
                        pattern,
                        period,
                        max_fires,
                        rearm,
                    })
                    .map_err(AgentError::Tool)?;
                Ok(watch_result(
                    format!("Created {}.", describe(&snapshot)),
                    vec![snapshot],
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_reason_escapes_line_and_terminal_controls() {
        assert_eq!(
            escape_pattern_for_reason("line\nbreak\u{1b}[31m"),
            "line\\nbreak\\u{1b}[31m"
        );
    }

    #[test]
    fn concurrent_watch_inserts_never_exceed_the_explicit_limit() {
        let shells = BashBackgroundRegistry::new();
        let registry = BashWatchRegistry::new(shells, None);
        let spec = || BashWatchSpec {
            target: "b1j6p9uvw".into(),
            kind: BashWatchKind::Pattern,
            pattern: Some("ready".into()),
            period: None,
            max_fires: 1,
            rearm: BashWatchRearm::Manual,
        };
        for _ in 0..MAX_BASH_WATCHES - 1 {
            registry.insert(spec(), false).unwrap();
        }

        let barrier = Arc::new(std::sync::Barrier::new(3));
        let results: Vec<_> = (0..2)
            .map(|_| {
                let registry = registry.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    registry.insert(spec(), false).is_ok()
                })
            })
            .collect();
        barrier.wait();
        let successful = results
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|success| *success)
            .count();
        assert_eq!(successful, 1);
        assert_eq!(registry.list(None).len(), MAX_BASH_WATCHES);
    }

    #[test]
    fn prompt_heuristic() {
        for yes in [
            "Continue? [Y/n]",
            "Overwrite file (y/N) ",
            "Proceed?",
            "Password:",
            "Enter passphrase for key '/x':",
            "Press ENTER to continue",
            "Really delete yes/no",
            "What is your name?",
        ] {
            assert!(looks_like_prompt(yes), "{yes}");
        }
        for no in ["Compiling foo v1.0", "done.", "Password reset complete"] {
            assert!(!looks_like_prompt(no), "{no}");
        }
    }

    fn pattern_watch(registry: &Arc<BashWatchRegistry>, rearm: BashWatchRearm) -> String {
        registry
            .insert(
                BashWatchSpec {
                    target: "bash-x".into(),
                    kind: BashWatchKind::Pattern,
                    pattern: Some("ready".into()),
                    period: None,
                    max_fires: 3,
                    rearm,
                },
                false,
            )
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn pattern_split_across_chunks_fires_and_on_activity_skips_one_chunk() {
        let registry = BashWatchRegistry::new(BashBackgroundRegistry::new(), None);
        let fired = Arc::new(Mutex::new(Vec::new()));
        let sink = fired.clone();
        registry.on_fire(move |f| sink.lock().unwrap().push(f.fire_id.clone()));
        let id = pattern_watch(&registry, BashWatchRearm::OnActivity);
        registry.handle_output("bash-x", "...rea");
        assert!(fired.lock().unwrap().is_empty());
        registry.handle_output("bash-x", "dy\n");
        assert_eq!(*fired.lock().unwrap(), vec![format!("{id}#1")]);
        // Disarmed: the next chunk only rearms (it is not matched) ...
        registry.handle_output("bash-x", "ready\n");
        assert_eq!(fired.lock().unwrap().len(), 1);
        // ... and a following chunk can fire again.
        registry.handle_output("bash-x", "ready\n");
        assert_eq!(fired.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn manual_pattern_ignores_output_until_rearmed() {
        let registry = BashWatchRegistry::new(BashBackgroundRegistry::new(), None);
        let fired = Arc::new(Mutex::new(0usize));
        let sink = fired.clone();
        registry.on_fire(move |_| *sink.lock().unwrap() += 1);
        let id = pattern_watch(&registry, BashWatchRearm::Manual);
        registry.handle_output("bash-x", "ready");
        registry.handle_output("bash-x", "ready");
        registry.handle_output("bash-x", "ready");
        assert_eq!(*fired.lock().unwrap(), 1);
        registry.rearm(&id).unwrap();
        registry.handle_output("bash-x", "ready");
        assert_eq!(*fired.lock().unwrap(), 2);
    }

    #[test]
    fn tail_and_matching_line() {
        let out: String = (1..=30).map(|i| format!("line {i}\n")).collect();
        let tail = fire_tail(&format!("{out}\n\n"));
        assert!(tail.starts_with("line 11\n"));
        assert!(tail.ends_with("line 30"));
        assert_eq!(
            matching_line("a\nready on :3000\nb", "ready").as_deref(),
            Some("ready on :3000")
        );
    }
}
