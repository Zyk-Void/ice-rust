//! Blackhole configuration: schema, tolerant reads, environment overrides
//! and writes. Ports `src/core/unified-config.ts` (BLACK-002..004).
//!
//! The file is `<agent dir>/ice-blackhole/ice-blackhole-config.json`. The
//! TypeScript `ICE_BLACKHOLE_*` variables are read under the icers names
//! `ICERS_BLACKHOLE_*`; the agent dir already honors `ICERS_CODING_AGENT_DIR`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::js::{js_number, number_to_string};

pub const CONFIG_DIR: &str = "ice-blackhole";
pub const CONFIG_FILE: &str = "ice-blackhole-config.json";

pub const ENV_COMPACTION: &str = "ICERS_BLACKHOLE_COMPACTION";
pub const ENV_COMPACTION_ENGINE: &str = "ICERS_BLACKHOLE_COMPACTION_ENGINE";
pub const ENV_MID_RUN_COMPACTION: &str = "ICERS_BLACKHOLE_MID_RUN_COMPACTION";
pub const ENV_COMPACT_AFTER_PERCENT: &str = "ICERS_BLACKHOLE_COMPACT_AFTER_PERCENT";

pub const DEFAULT_COMPACT_AFTER_PERCENT: f64 = 85.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionMode {
    Auto,
    Manual,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionEngine {
    Blackhole,
    IceDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidRunCompaction {
    Resume,
    Pause,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailBehavior {
    IceDefault,
    Minimal,
}

impl CompactionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::Off => "off",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "manual" => Some(Self::Manual),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

impl CompactionEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blackhole => "blackhole",
            Self::IceDefault => "ice-default",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "blackhole" => Some(Self::Blackhole),
            "ice-default" => Some(Self::IceDefault),
            _ => None,
        }
    }
}

impl MidRunCompaction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Pause => "pause",
            Self::Off => "off",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "resume" => Some(Self::Resume),
            "pause" => Some(Self::Pause),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

impl TailBehavior {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IceDefault => "ice-default",
            Self::Minimal => "minimal",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ice-default" => Some(Self::IceDefault),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }
}

/// The effective Blackhole configuration (`UnifiedConfig`).
#[derive(Debug, Clone, PartialEq)]
pub struct BlackholeConfig {
    pub compaction: CompactionMode,
    pub compaction_engine: CompactionEngine,
    pub mid_run_compaction: MidRunCompaction,
    pub tail_behavior: TailBehavior,
    pub compact_after_percent: f64,
    pub memory: bool,
    /// Cap large pre-compaction tool results in model context; the session keeps full output.
    pub trim_kept_tool_results: bool,
    /// Add a short model-written `[State Of Work]` section to each summary.
    pub refine: bool,
    /// `provider/modelId` for the refine pass; the active model when unset.
    pub refine_model: Option<String>,
}

impl Default for BlackholeConfig {
    fn default() -> Self {
        Self {
            compaction: CompactionMode::Auto,
            compaction_engine: CompactionEngine::Blackhole,
            mid_run_compaction: MidRunCompaction::Off,
            tail_behavior: TailBehavior::Minimal,
            compact_after_percent: DEFAULT_COMPACT_AFTER_PERCENT,
            memory: false,
            trim_kept_tool_results: true,
            refine: false,
            refine_model: None,
        }
    }
}

/// A partial update passed to [`save_config`]. Values are not validated
/// before they are written; the next load normalizes them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigPatch {
    pub compaction: Option<CompactionMode>,
    pub compaction_engine: Option<CompactionEngine>,
    pub mid_run_compaction: Option<MidRunCompaction>,
    pub tail_behavior: Option<TailBehavior>,
    pub compact_after_percent: Option<f64>,
    pub memory: Option<bool>,
    pub trim_kept_tool_results: Option<bool>,
    pub refine: Option<bool>,
}

/// Process environment lookups, injectable for tests.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// `<agent dir>/ice-blackhole/ice-blackhole-config.json`.
pub fn config_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join(CONFIG_DIR).join(CONFIG_FILE)
}

/// The config path in the effective icers agent dir.
pub fn default_config_path() -> Option<PathBuf> {
    crate::config::agent_dir().ok().map(|dir| config_path(&dir))
}

fn number_value(value: &Value) -> Option<f64> {
    value.as_f64()
}

/// Parse a raw file value. Every non-null object (arrays included) counts as a
/// record; anything else is treated as empty.
fn parse_raw(raw: &Value) -> (BlackholeConfig, Option<f64>) {
    let mut config = BlackholeConfig::default();
    let empty = Map::new();
    let record = match raw {
        Value::Object(map) => map,
        _ => &empty,
    };
    let text = |key: &str| record.get(key).and_then(Value::as_str);
    if let Some(mode) = text("compaction").and_then(CompactionMode::parse) {
        config.compaction = mode;
    }
    if let Some(engine) = text("compactionEngine").and_then(CompactionEngine::parse) {
        config.compaction_engine = engine;
    }
    if let Some(mode) = text("midRunCompaction").and_then(MidRunCompaction::parse) {
        config.mid_run_compaction = mode;
    }
    if let Some(tail) = text("tailBehavior").and_then(TailBehavior::parse) {
        config.tail_behavior = tail;
    }
    config.memory = record.get("memory") == Some(&Value::Bool(true));
    config.refine = record.get("refine") == Some(&Value::Bool(true));
    config.trim_kept_tool_results = record.get("trimKeptToolResults") != Some(&Value::Bool(false));
    config.refine_model = text("refineModel")
        .filter(|model| model.contains('/'))
        .map(str::to_string);
    let percent = record.get("compactAfterPercent").and_then(number_value);
    (config, percent)
}

fn normalize_percent(value: Option<f64>) -> f64 {
    match value {
        Some(percent) if percent.is_finite() && (1.0..=99.0).contains(&percent) => percent,
        _ => DEFAULT_COMPACT_AFTER_PERCENT,
    }
}

/// `loadConfig` against an explicit agent dir and environment.
pub fn load_config_with(
    agent_dir: &Path,
    env: EnvLookup<'_>,
    warn: &mut dyn FnMut(String),
) -> BlackholeConfig {
    let path = config_path(agent_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(error) => {
                warn(format!(
                    "Blackhole config {} is not valid JSON ({error}); using defaults",
                    path.display()
                ));
                Value::Null
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(error) => {
            warn(format!(
                "Could not read Blackhole config {} ({error}); using defaults",
                path.display()
            ));
            Value::Null
        }
    };
    resolve_config(&raw, env, warn)
}

/// Normalize a raw file value and overlay the environment.
fn resolve_config(
    raw: &Value,
    env: EnvLookup<'_>,
    warn: &mut dyn FnMut(String),
) -> BlackholeConfig {
    let (mut config, mut percent) = parse_raw(raw);

    let lowered = |name: &str| env(name).map(|value| value.trim().to_lowercase());
    if let Some(mode) = lowered(ENV_COMPACTION).and_then(|value| CompactionMode::parse(&value)) {
        config.compaction = mode;
    }
    if let Some(engine) =
        lowered(ENV_COMPACTION_ENGINE).and_then(|value| CompactionEngine::parse(&value))
    {
        config.compaction_engine = engine;
    }
    if let Some(mode) =
        lowered(ENV_MID_RUN_COMPACTION).and_then(|value| MidRunCompaction::parse(&value))
    {
        config.mid_run_compaction = mode;
    }
    if let Some(raw_percent) = env(ENV_COMPACT_AFTER_PERCENT) {
        let parsed = js_number(&raw_percent);
        if parsed.is_finite() {
            percent = Some(parsed);
        } else {
            warn(format!(
                "{ENV_COMPACT_AFTER_PERCENT}={raw_percent} is not a finite number; using the configured percentage"
            ));
        }
    }
    config.compact_after_percent = normalize_percent(percent);
    config
}

/// `loadConfig` in the effective icers agent dir with the process environment.
pub fn load_config(warn: &mut dyn FnMut(String)) -> BlackholeConfig {
    match crate::config::agent_dir() {
        Ok(dir) => load_config_with(&dir, &process_env, warn),
        // Without an agent dir only the environment can change the defaults.
        Err(_) => resolve_config(&Value::Null, &process_env, warn),
    }
}

/// `resolveCompactAfterTokens`: `floor(window * percent / 100)` for a finite
/// positive window. No minimum is applied, so a tiny window can yield 0.
pub fn resolve_compact_after_tokens(
    config: &BlackholeConfig,
    context_window: Option<f64>,
    warn: &mut dyn FnMut(String),
) -> Option<i64> {
    match context_window {
        Some(window) if window.is_finite() && window > 0.0 => {
            Some((window * config.compact_after_percent / 100.0).floor() as i64)
        }
        _ => {
            warn("Blackhole compaction threshold needs the model context window; skipping".into());
            None
        }
    }
}

fn percent_json(value: f64) -> Value {
    if !value.is_finite() {
        return Value::Null;
    }
    if value.fract() == 0.0 && value.abs() < 9.007_199_254_740_992e15 {
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

fn config_json(config: &BlackholeConfig) -> Value {
    let mut map = Map::new();
    map.insert("compaction".into(), config.compaction.as_str().into());
    map.insert(
        "compactionEngine".into(),
        config.compaction_engine.as_str().into(),
    );
    map.insert(
        "midRunCompaction".into(),
        config.mid_run_compaction.as_str().into(),
    );
    map.insert("tailBehavior".into(), config.tail_behavior.as_str().into());
    map.insert(
        "compactAfterPercent".into(),
        percent_json(config.compact_after_percent),
    );
    map.insert("memory".into(), config.memory.into());
    map.insert(
        "trimKeptToolResults".into(),
        config.trim_kept_tool_results.into(),
    );
    map.insert("refine".into(), config.refine.into());
    if let Some(model) = &config.refine_model {
        map.insert("refineModel".into(), model.clone().into());
    }
    Value::Object(map)
}

/// `saveConfig`: load (with environment overlays), shallow-merge the patch,
/// write the private file, then reload. There is no lock or atomic rename,
/// matching the TypeScript writer; unknown raw keys are dropped.
pub fn save_config_with(
    agent_dir: &Path,
    env: EnvLookup<'_>,
    patch: &ConfigPatch,
) -> std::io::Result<BlackholeConfig> {
    let mut next = load_config_with(agent_dir, env, &mut |_| {});
    if let Some(value) = patch.compaction {
        next.compaction = value;
    }
    if let Some(value) = patch.compaction_engine {
        next.compaction_engine = value;
    }
    if let Some(value) = patch.mid_run_compaction {
        next.mid_run_compaction = value;
    }
    if let Some(value) = patch.tail_behavior {
        next.tail_behavior = value;
    }
    if let Some(value) = patch.compact_after_percent {
        next.compact_after_percent = value;
    }
    if let Some(value) = patch.memory {
        next.memory = value;
    }
    if let Some(value) = patch.trim_kept_tool_results {
        next.trim_kept_tool_results = value;
    }
    if let Some(value) = patch.refine {
        next.refine = value;
    }
    let path = config_path(agent_dir);
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&config_json(&next)).map_err(std::io::Error::other)?
    );
    write_private(&path, &text)?;
    Ok(load_config_with(agent_dir, env, &mut |_| {}))
}

/// `saveConfig` in the effective icers agent dir.
pub fn save_config(patch: &ConfigPatch) -> std::io::Result<BlackholeConfig> {
    let dir = crate::config::agent_dir().map_err(std::io::Error::other)?;
    save_config_with(&dir, &process_env, patch)
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(text.as_bytes())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, text)
    }
}

/// The percentage as `/blackhole status` prints it (`85`, `12.5`).
pub fn percent_label(config: &BlackholeConfig) -> String {
    number_to_string(config.compact_after_percent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn write(dir: &Path, text: &str) {
        let path = config_path(dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn missing_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_config_with(dir.path(), &env_of(&[]), &mut |_| {});
        assert_eq!(config, BlackholeConfig::default());
    }

    #[test]
    fn invalid_json_warns_and_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "{not json");
        let mut warnings = Vec::new();
        let config = load_config_with(dir.path(), &env_of(&[]), &mut |w| warnings.push(w));
        assert_eq!(config, BlackholeConfig::default());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn tolerant_reads_accept_only_exact_members() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{"compaction":"manual","compactionEngine":"bogus","midRunCompaction":"pause",
               "tailBehavior":"ice-default","compactAfterPercent":42.5,"memory":"true",
               "refine":true,"trimKeptToolResults":0,"refineModel":"no-slash","compactAfterTokens":9}"#,
        );
        let config = load_config_with(dir.path(), &env_of(&[]), &mut |_| {});
        assert_eq!(config.compaction, CompactionMode::Manual);
        assert_eq!(config.compaction_engine, CompactionEngine::Blackhole);
        assert_eq!(config.mid_run_compaction, MidRunCompaction::Pause);
        assert_eq!(config.tail_behavior, TailBehavior::IceDefault);
        assert_eq!(config.compact_after_percent, 42.5);
        assert!(!config.memory);
        assert!(config.refine);
        // Only literal false disables trimming.
        assert!(config.trim_kept_tool_results);
        assert_eq!(config.refine_model, None);
    }

    #[test]
    fn out_of_range_percent_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), r#"{"compactAfterPercent":100}"#);
        let config = load_config_with(dir.path(), &env_of(&[]), &mut |_| {});
        assert_eq!(config.compact_after_percent, 85.0);
    }

    #[test]
    fn environment_overrides_file_values() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{"compaction":"auto","compactAfterPercent":50,"midRunCompaction":"off"}"#,
        );
        let env = env_of(&[
            (ENV_COMPACTION, " OFF "),
            (ENV_COMPACTION_ENGINE, "nope"),
            (ENV_MID_RUN_COMPACTION, "Resume"),
            (ENV_COMPACT_AFTER_PERCENT, "70"),
        ]);
        let config = load_config_with(dir.path(), &env, &mut |_| {});
        assert_eq!(config.compaction, CompactionMode::Off);
        assert_eq!(config.compaction_engine, CompactionEngine::Blackhole);
        assert_eq!(config.mid_run_compaction, MidRunCompaction::Resume);
        assert_eq!(config.compact_after_percent, 70.0);
    }

    #[test]
    fn nonfinite_env_percent_keeps_file_and_empty_env_selects_default() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), r#"{"compactAfterPercent":50}"#);
        let mut warnings = Vec::new();
        let config = load_config_with(
            dir.path(),
            &env_of(&[(ENV_COMPACT_AFTER_PERCENT, "abc")]),
            &mut |w| warnings.push(w),
        );
        assert_eq!(config.compact_after_percent, 50.0);
        assert_eq!(warnings.len(), 1);
        // Number("") is 0, which is out of range and selects 85, not the file value.
        let config = load_config_with(
            dir.path(),
            &env_of(&[(ENV_COMPACT_AFTER_PERCENT, "")]),
            &mut |_| {},
        );
        assert_eq!(config.compact_after_percent, 85.0);
    }

    #[test]
    fn threshold_floors_without_a_minimum() {
        let config = BlackholeConfig::default();
        assert_eq!(
            resolve_compact_after_tokens(&config, Some(200_000.0), &mut |_| {}),
            Some(170_000)
        );
        assert_eq!(
            resolve_compact_after_tokens(&config, Some(1.0), &mut |_| {}),
            Some(0)
        );
        let mut warned = false;
        assert_eq!(
            resolve_compact_after_tokens(&config, None, &mut |_| warned = true),
            None
        );
        assert!(warned);
    }

    #[test]
    fn save_merges_patch_writes_private_file_and_drops_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), r#"{"legacy":1,"refineModel":"a/b"}"#);
        let saved = save_config_with(
            dir.path(),
            &env_of(&[]),
            &ConfigPatch {
                compact_after_percent: Some(60.0),
                ..ConfigPatch::default()
            },
        )
        .unwrap();
        assert_eq!(saved.compact_after_percent, 60.0);
        let text = std::fs::read_to_string(config_path(dir.path())).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\"compactAfterPercent\": 60"));
        assert!(text.contains("\"refineModel\": \"a/b\""));
        assert!(!text.contains("legacy"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config_path(dir.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn save_persists_environment_effective_values() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_of(&[(ENV_MID_RUN_COMPACTION, "pause")]);
        save_config_with(
            dir.path(),
            &env,
            &ConfigPatch {
                refine: Some(true),
                ..ConfigPatch::default()
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(config_path(dir.path())).unwrap();
        assert!(text.contains("\"midRunCompaction\": \"pause\""));
    }
}
