//! Cognee configuration, the shared environment file, credentials, project
//! datasets, session ids and private file writes. Ports `ice-cognee.ts`
//! (config/connection helpers) and `ice-cognee-env.ts` (COG-001..006,
//! COG-042).
//!
//! icers reads the TypeScript `ICE_COGNEE_*` variables under the names
//! `ICERS_COGNEE_*`. The generic `COGNEE_*` variables and the Cognee-owned
//! `~/.cognee/.env` and `~/.cognee-plugin/api_key.json` are read as in ICE.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::blackhole::js::js_number;
use crate::blackhole::regex;

pub const PROJECT_DATASET_SENTINEL: &str = "$project";

pub const ENV_ENABLED: &str = "ICERS_COGNEE_ENABLED";
pub const ENV_RECALL: &str = "ICERS_COGNEE_RECALL";
pub const ENV_REMEMBER: &str = "ICERS_COGNEE_REMEMBER";
pub const ENV_CAPTURE: &str = "ICERS_COGNEE_CAPTURE";
pub const ENV_CAPTURE_TOOLS: &str = "ICERS_COGNEE_CAPTURE_TOOLS";
pub const ENV_IMPROVE: &str = "ICERS_COGNEE_IMPROVE";
pub const ENV_BASE_URL: &str = "ICERS_COGNEE_BASE_URL";
pub const ENV_DATASET: &str = "ICERS_COGNEE_DATASET";
pub const ENV_COMPACTION_SUMMARY: &str = "ICERS_COGNEE_COMPACTION_SUMMARY";
pub const ENV_CHECKPOINT_RECALL: &str = "ICERS_COGNEE_CHECKPOINT_RECALL";
const ICERS_PREFIX: &str = "ICERS_COGNEE_";

/// A snapshot of the variables Cognee reads.
pub type CogneeEnv = HashMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoRememberMode {
    Off,
    Compaction,
}

impl AutoRememberMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Compaction => "compaction",
        }
    }
}

/// How Cognee cooperates with Blackhole/native compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionSummaryMode {
    Defer,
    Own,
    Auto,
}

impl CompactionSummaryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Defer => "defer",
            Self::Own => "own",
            Self::Auto => "auto",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "defer" => Some(Self::Defer),
            "own" => Some(Self::Own),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// `IceCogneeConfig`.
#[derive(Debug, Clone, PartialEq)]
pub struct CogneeConfig {
    pub enabled: bool,
    pub auto_recall: bool,
    pub auto_remember: AutoRememberMode,
    /// Continuous session capture (prompts/answers/traces into the cache).
    pub capture_session: bool,
    /// Store tool traces when session capture is on.
    pub capture_tools: bool,
    /// Promote the session cache into the permanent graph.
    pub auto_improve: bool,
    pub compaction_summary_mode: CompactionSummaryMode,
    /// Bounded recall written for Blackhole before its checkpoint.
    pub checkpoint_recall: bool,
    pub top_k: u32,
    pub base_url: String,
    pub dataset: String,
    pub recall_budget_ms: u64,
    pub recall_max_chars: usize,
    pub remember_max_chars: usize,
    pub capture_max_chars: usize,
    pub queue_limit: usize,
    pub max_response_chars: usize,
}

impl Default for CogneeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_recall: true,
            auto_remember: AutoRememberMode::Compaction,
            capture_session: true,
            capture_tools: true,
            auto_improve: true,
            compaction_summary_mode: CompactionSummaryMode::Auto,
            checkpoint_recall: false,
            top_k: 5,
            base_url: "http://127.0.0.1:8211".into(),
            dataset: "ice".into(),
            recall_budget_ms: 10_000,
            recall_max_chars: 6000,
            remember_max_chars: 12_000,
            capture_max_chars: 8000,
            queue_limit: 64,
            max_response_chars: 12_000,
        }
    }
}

/// A raw input: an environment string or a stored JSON value.
enum Raw<'a> {
    Env(&'a str),
    Json(&'a Value),
}

fn pick<'a>(
    env: &'a CogneeEnv,
    name: &str,
    stored: &'a Map<String, Value>,
    key: &str,
) -> Option<Raw<'a>> {
    match env.get(name) {
        Some(value) => Some(Raw::Env(value)),
        None => stored.get(key).map(Raw::Json),
    }
}

impl<'a> Raw<'a> {
    /// The text of an environment value or a stored JSON string.
    fn text(&self) -> Option<&'a str> {
        match self {
            Raw::Env(text) => Some(text),
            Raw::Json(Value::String(text)) => Some(text.as_str()),
            Raw::Json(_) => None,
        }
    }
}

fn parse_boolean(value: Option<Raw<'_>>, name: &str) -> Result<Option<bool>, String> {
    let error = || format!("{name} must be true, false, 1, or 0");
    match value {
        None => Ok(None),
        Some(Raw::Json(Value::Bool(value))) => Ok(Some(*value)),
        Some(raw) => match raw.text() {
            Some("1" | "true") => Ok(Some(true)),
            Some("0" | "false") => Ok(Some(false)),
            _ => Err(error()),
        },
    }
}

/// `Number(value)` for a stored JSON value.
fn json_number(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) => js_number(text),
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        Value::Null => 0.0,
        Value::Array(items) if items.is_empty() => 0.0,
        Value::Array(items) if items.len() == 1 => match &items[0] {
            Value::Array(_) | Value::Object(_) => f64::NAN,
            other => json_number(other),
        },
        _ => f64::NAN,
    }
}

fn parse_positive_integer(
    value: Option<&Value>,
    name: &str,
    minimum: f64,
    maximum: f64,
) -> Result<Option<f64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = json_number(value);
    if parsed.fract() != 0.0 || !parsed.is_finite() || parsed < minimum || parsed > maximum {
        return Err(format!(
            "{name} must be an integer from {minimum} to {maximum}"
        ));
    }
    Ok(Some(parsed))
}

fn parse_url(value: Option<Raw<'_>>, name: &str) -> Result<Option<String>, String> {
    let text = match value {
        None => return Ok(None),
        Some(raw) => match raw.text() {
            Some(text) => text,
            None => return Err(format!("{name} must be a nonempty URL")),
        },
    };
    if text.trim().is_empty() {
        return Err(format!("{name} must be a nonempty URL"));
    }
    match reqwest::Url::parse(text.trim()) {
        Ok(url)
            if (url.scheme() == "http" || url.scheme() == "https")
                && url.username().is_empty()
                && url.password().is_none() => {}
        _ => {
            return Err(format!(
                "{name} must be an HTTP or HTTPS URL without embedded credentials"
            ));
        }
    }
    let trimmed = text.trim();
    Ok(Some(
        trimmed.strip_suffix('/').unwrap_or(trimmed).to_string(),
    ))
}

fn parse_dataset(value: Option<Raw<'_>>, name: &str) -> Result<Option<String>, String> {
    let error =
        || format!("{name} must contain 1-128 letters, numbers, dots, underscores, or hyphens");
    let text = match value {
        None => return Ok(None),
        Some(raw) => match raw.text() {
            Some(text) => text.trim(),
            None => return Err(error()),
        },
    };
    if text == PROJECT_DATASET_SENTINEL {
        return Ok(Some(text.into()));
    }
    if !regex!(r"^[A-Za-z0-9._-]{1,128}$").is_match(text) {
        return Err(error());
    }
    Ok(Some(text.into()))
}

fn raw_string(value: &Option<Raw<'_>>) -> Option<String> {
    match value {
        Some(Raw::Env(text)) => Some(text.to_string()),
        Some(Raw::Json(Value::String(text))) => Some(text.clone()),
        _ => None,
    }
}

/// `resolveIceCogneeConfig`: environment over stored values over defaults.
/// Every non-null object (arrays included) counts as stored config. Invalid
/// supplied values are errors, not silent defaults.
pub fn resolve_config(stored: Option<&Value>, env: &CogneeEnv) -> Result<CogneeConfig, String> {
    let empty = Map::new();
    let source = match stored {
        Some(Value::Object(map)) => map,
        _ => &empty,
    };
    let mut config = CogneeConfig::default();
    if let Some(value) = parse_boolean(pick(env, ENV_ENABLED, source, "enabled"), ENV_ENABLED)? {
        config.enabled = value;
    }
    if let Some(value) = parse_boolean(pick(env, ENV_RECALL, source, "autoRecall"), ENV_RECALL)? {
        config.auto_recall = value;
    }
    let remember = pick(env, ENV_REMEMBER, source, "autoRemember");
    if remember.is_some() {
        config.auto_remember = match raw_string(&remember).as_deref() {
            Some("off") => AutoRememberMode::Off,
            Some("compaction") => AutoRememberMode::Compaction,
            _ => return Err(format!("{ENV_REMEMBER} must be off or compaction")),
        };
    }
    let base = env
        .get(ENV_BASE_URL)
        .or_else(|| env.get("COGNEE_BASE_URL"))
        .map(|text| Raw::Env(text))
        .or_else(|| source.get("baseUrl").map(Raw::Json));
    if let Some(url) = parse_url(base, ENV_BASE_URL)? {
        config.base_url = url;
    }
    if let Some(dataset) = parse_dataset(pick(env, ENV_DATASET, source, "dataset"), ENV_DATASET)? {
        config.dataset = dataset;
    }
    if let Some(value) = parse_boolean(
        pick(env, ENV_CAPTURE, source, "captureSession"),
        ENV_CAPTURE,
    )? {
        config.capture_session = value;
    }
    if let Some(value) = parse_boolean(
        pick(env, ENV_CAPTURE_TOOLS, source, "captureTools"),
        ENV_CAPTURE_TOOLS,
    )? {
        config.capture_tools = value;
    }
    if let Some(value) = parse_boolean(pick(env, ENV_IMPROVE, source, "autoImprove"), ENV_IMPROVE)?
    {
        config.auto_improve = value;
    }
    let mode = pick(env, ENV_COMPACTION_SUMMARY, source, "compactionSummaryMode");
    if mode.is_some() {
        config.compaction_summary_mode = raw_string(&mode)
            .as_deref()
            .and_then(CompactionSummaryMode::parse)
            .ok_or_else(|| format!("{ENV_COMPACTION_SUMMARY} must be defer, own, or auto"))?;
    }
    if let Some(value) = parse_boolean(
        pick(env, ENV_CHECKPOINT_RECALL, source, "checkpointRecall"),
        ENV_CHECKPOINT_RECALL,
    )? {
        config.checkpoint_recall = value;
    }
    if let Some(value) = parse_positive_integer(source.get("topK"), "topK", 1.0, 10.0)? {
        config.top_k = value as u32;
    }
    if let Some(value) = parse_positive_integer(
        source.get("recallBudgetMs"),
        "recallBudgetMs",
        100.0,
        120_000.0,
    )? {
        config.recall_budget_ms = value as u64;
    }
    if let Some(value) = parse_positive_integer(
        source.get("recallMaxChars"),
        "recallMaxChars",
        256.0,
        100_000.0,
    )? {
        config.recall_max_chars = value as usize;
        config.max_response_chars = config.recall_max_chars.max(12_000);
    }
    if let Some(value) = parse_positive_integer(
        source.get("rememberMaxChars"),
        "rememberMaxChars",
        256.0,
        200_000.0,
    )? {
        config.remember_max_chars = value as usize;
    }
    if let Some(value) = parse_positive_integer(
        source.get("captureMaxChars"),
        "captureMaxChars",
        256.0,
        100_000.0,
    )? {
        config.capture_max_chars = value as usize;
    }
    if let Some(value) =
        parse_positive_integer(source.get("queueLimit"), "queueLimit", 1.0, 10_000.0)?
    {
        config.queue_limit = value as usize;
    }
    Ok(config)
}

// ── environment file ──

/// Parse `KEY=VALUE` lines (optional `export ` prefix); last value wins.
/// Only `COGNEE_*` and `ICERS_COGNEE_*` keys are kept.
pub fn parse_env_file(text: &str) -> CogneeEnv {
    let mut out = CogneeEnv::new();
    for raw in text.split('\n') {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let body = match line.strip_prefix("export ") {
            Some(rest) => rest.trim(),
            None => line,
        };
        let Some(eq) = body.find('=').filter(|eq| *eq > 0) else {
            continue;
        };
        let key = body[..eq].trim();
        let mut value = body[eq + 1..].trim();
        if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            value = &value[1..value.len() - 1];
        }
        if key.starts_with(ICERS_PREFIX) || key.starts_with("COGNEE_") {
            out.insert(key.to_string(), value.to_string());
        }
    }
    out
}

/// The process variables Cognee reads.
pub fn process_env() -> CogneeEnv {
    std::env::vars()
        .filter(|(key, _)| {
            key.starts_with("COGNEE_")
                || key.starts_with(ICERS_PREFIX)
                || key == "ICERS_CODING_AGENT_DIR"
        })
        .collect()
}

pub fn cognee_env_path(home: &Path) -> PathBuf {
    home.join(".cognee").join(".env")
}

/// Merge `~/.cognee/.env` under the process environment (process wins) and
/// alias `COGNEE_LOCAL_API_URL` to `COGNEE_BASE_URL` when no base URL is set.
pub fn merged_env(process: &CogneeEnv, env_file: Option<&Path>) -> CogneeEnv {
    let mut merged = env_file
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_env_file(&text))
        .unwrap_or_default();
    merged.extend(process.iter().map(|(k, v)| (k.clone(), v.clone())));
    let truthy = |key: &str| merged.get(key).is_some_and(|value| !value.is_empty());
    if !truthy("COGNEE_BASE_URL") && !truthy(ENV_BASE_URL) && truthy("COGNEE_LOCAL_API_URL") {
        let local = merged["COGNEE_LOCAL_API_URL"].clone();
        merged.insert("COGNEE_BASE_URL".into(), local);
    }
    merged
}

// ── credentials ──

/// Where the API key came from; never the key itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Env,
    ApiKeyJson,
    None,
}

impl KeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::ApiKeyJson => "api_key.json",
            Self::None => "none",
        }
    }
}

fn origin(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .map(|url| url.origin().ascii_serialization())
}

/// A cached key whose `base_url`, when present, has the same origin.
pub fn read_cached_api_key(text: Option<&str>, expected_base_url: Option<&str>) -> Option<String> {
    let parsed: Value = serde_json::from_str(text?).ok()?;
    let key = parsed.get("api_key")?.as_str()?.trim();
    if key.is_empty() {
        return None;
    }
    if let (Some(expected), Some(cached)) = (
        expected_base_url,
        parsed.get("base_url").and_then(Value::as_str),
    ) {
        if origin(cached)? != origin(expected)? {
            return None;
        }
    }
    Some(key.to_string())
}

/// `CogneeConnection`.
#[derive(Debug, Clone)]
pub struct CogneeConnection {
    pub environment: CogneeEnv,
    pub config: CogneeConfig,
    /// The stored config failed validation, so defaults apply.
    pub invalid_config: bool,
    pub api_key: Option<String>,
    pub key_source: KeySource,
}

/// Paths and environment a connection is resolved from.
#[derive(Debug, Clone)]
pub struct ConnectionInputs {
    pub config_path: PathBuf,
    pub storage_dir: PathBuf,
    pub home_dir: Option<PathBuf>,
    /// Explicit environment; `None` merges the process with `~/.cognee/.env`.
    pub env: Option<CogneeEnv>,
    pub process_env: CogneeEnv,
    pub api_key: Option<String>,
}

/// `loadCogneeConnection`. Key order: explicit key, process
/// `COGNEE_API_KEY`, the icers then plugin `api_key.json` for this origin,
/// then the merged environment (including `~/.cognee/.env`).
pub fn load_connection(inputs: &ConnectionInputs) -> CogneeConnection {
    let environment = inputs.env.clone().unwrap_or_else(|| {
        let file = inputs.home_dir.as_deref().map(cognee_env_path);
        merged_env(&inputs.process_env, file.as_deref())
    });
    let (config, invalid_config) = match load_config(&inputs.config_path, &environment) {
        Ok(config) => (config, false),
        Err(_) => (CogneeConfig::default(), true),
    };
    let connection = |api_key, key_source| CogneeConnection {
        environment: environment.clone(),
        config: config.clone(),
        invalid_config,
        api_key,
        key_source,
    };
    if let Some(key) = inputs.api_key.clone().filter(|key| !key.is_empty()) {
        return connection(Some(key), KeySource::Env);
    }
    if let Some(key) = inputs
        .process_env
        .get("COGNEE_API_KEY")
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
    {
        return connection(Some(key.to_string()), KeySource::Env);
    }
    let read = |path: PathBuf| std::fs::read_to_string(path).ok();
    let ice_cached = read(inputs.storage_dir.join("api_key.json"));
    let plugin_cached = inputs
        .home_dir
        .as_ref()
        .and_then(|home| read(home.join(".cognee-plugin").join("api_key.json")));
    let cached = read_cached_api_key(ice_cached.as_deref(), Some(&config.base_url))
        .or_else(|| read_cached_api_key(plugin_cached.as_deref(), Some(&config.base_url)));
    if let Some(key) = cached {
        return connection(Some(key), KeySource::ApiKeyJson);
    }
    if let Some(key) = environment
        .get("COGNEE_API_KEY")
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
    {
        return connection(Some(key.to_string()), KeySource::Env);
    }
    connection(None, KeySource::None)
}

// ── dataset and session identity ──

/// Walk up to 24 directories for any `.git` entry; the lexical cwd otherwise.
pub fn find_git_root(cwd: &Path) -> PathBuf {
    let start = std::path::absolute(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut dir = start.clone();
    for _ in 0..24 {
        if dir.join(".git").exists() {
            return dir;
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => break,
        }
    }
    start
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `ice-<basename>-<8 hex of the lexical root>`.
pub fn resolve_project_dataset(cwd: &Path) -> String {
    let root = find_git_root(cwd);
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = regex!(r"[^A-Za-z0-9._-]+").replace_all(&name, "-");
    let base = base.trim_matches('-');
    let base: String = base.chars().take(40).collect();
    let base = if base.is_empty() {
        "repo".to_string()
    } else {
        base
    };
    let hash = sha256_hex(&root.to_string_lossy());
    format!("ice-{base}-{}", &hash[..8])
}

/// Resolve `$project` against `cwd` (the process cwd when blank).
pub fn resolve_runtime_dataset(dataset: &str, cwd: Option<&Path>) -> String {
    if dataset != PROJECT_DATASET_SENTINEL {
        return dataset.to_string();
    }
    match cwd.filter(|cwd| !cwd.as_os_str().is_empty()) {
        Some(cwd) => resolve_project_dataset(cwd),
        None => resolve_project_dataset(&std::env::current_dir().unwrap_or_default()),
    }
}

/// `ice_<hostSessionId>`; a sanitized or truncated id gains a short hash of
/// the original so distinct host ids never collide.
pub fn cognee_session_id(host_session_id: &str) -> String {
    let trimmed = host_session_id.trim();
    let sanitized = regex!(r"[^A-Za-z0-9._-]+").replace_all(trimmed, "_");
    let mut cleaned: String = sanitized.chars().take(120).collect();
    if cleaned != trimmed {
        let prefix: String = sanitized.chars().take(111).collect();
        cleaned = format!("{prefix}_{}", &sha256_hex(trimmed)[..8]);
    }
    if cleaned.starts_with("ice_") {
        return cleaned;
    }
    if cleaned.is_empty() {
        "ice_session".into()
    } else {
        format!("ice_{cleaned}")
    }
}

// ── config file ──

/// `loadIceCogneeConfig`: unreadable or malformed files count as absent;
/// validation errors propagate.
pub fn load_config(path: &Path, env: &CogneeEnv) -> Result<CogneeConfig, String> {
    let stored = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    resolve_config(stored.as_ref(), env)
}

/// The persisted (noncredential) fields in TypeScript key order.
pub fn config_json(config: &CogneeConfig) -> Value {
    serde_json::json!({
        "enabled": config.enabled,
        "autoRecall": config.auto_recall,
        "autoRemember": config.auto_remember.as_str(),
        "captureSession": config.capture_session,
        "captureTools": config.capture_tools,
        "autoImprove": config.auto_improve,
        "compactionSummaryMode": config.compaction_summary_mode.as_str(),
        "checkpointRecall": config.checkpoint_recall,
        "baseUrl": config.base_url,
        "dataset": config.dataset,
        "topK": config.top_k,
        "recallBudgetMs": config.recall_budget_ms,
        "recallMaxChars": config.recall_max_chars,
        "rememberMaxChars": config.remember_max_chars,
        "captureMaxChars": config.capture_max_chars,
        "queueLimit": config.queue_limit,
    })
}

/// `saveIceCogneeConfig`: private parent dir, atomic private file.
pub fn save_config(path: &Path, config: &CogneeConfig) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&config_json(config)).map_err(std::io::Error::other)?
    );
    write_private_file_atomic(path, &text)
}

/// Create `path` (recursively) and set it to mode 0700.
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Write a sibling `<target>.<uuid>.tmp` with mode 0600, then rename it over
/// the target. On failure the temporary file is removed. There is no fsync or
/// cross-file transaction, matching `writePrivateFileAtomic`.
pub fn write_private_file_atomic(target: &Path, content: &str) -> std::io::Result<()> {
    let mut name = target.as_os_str().to_owned();
    name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let temporary = PathBuf::from(name);
    let result = (|| {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(content.as_bytes())?;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        std::fs::write(&temporary, content)?;
        std::fs::rename(&temporary, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env(pairs: &[(&str, &str)]) -> CogneeEnv {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_match_ice() {
        let config = resolve_config(None, &env(&[])).unwrap();
        assert_eq!(config, CogneeConfig::default());
        assert!(config.enabled);
        assert_eq!(config.base_url, "http://127.0.0.1:8211");
        assert_eq!(config.compaction_summary_mode, CompactionSummaryMode::Auto);
        assert!(!config.checkpoint_recall);
    }

    #[test]
    fn environment_overrides_stored_values_and_validates() {
        let stored = json!({"enabled": false, "dataset": "stored", "baseUrl": "http://a/",
            "topK": 3, "recallMaxChars": 20000, "maxResponseChars": 1, "apiKey": "secret"});
        let config = resolve_config(
            Some(&stored),
            &env(&[
                (ENV_ENABLED, "1"),
                (ENV_DATASET, " $project "),
                ("COGNEE_BASE_URL", "https://b.test/x/"),
            ]),
        )
        .unwrap();
        assert!(config.enabled);
        assert_eq!(config.dataset, "$project");
        assert_eq!(config.base_url, "https://b.test/x");
        assert_eq!(config.top_k, 3);
        assert_eq!(config.recall_max_chars, 20_000);
        assert_eq!(config.max_response_chars, 20_000);
        assert!(resolve_config(None, &env(&[(ENV_ENABLED, "yes")])).is_err());
        assert!(resolve_config(Some(&json!({"topK": 11})), &env(&[])).is_err());
        assert!(resolve_config(Some(&json!({"topK": 2.5})), &env(&[])).is_err());
        assert!(resolve_config(Some(&json!({"topK": "4"})), &env(&[])).is_ok());
        assert!(resolve_config(None, &env(&[(ENV_BASE_URL, "ftp://x")])).is_err());
        assert!(resolve_config(None, &env(&[(ENV_DATASET, "bad name")])).is_err());
        assert!(resolve_config(None, &env(&[(ENV_REMEMBER, "always")])).is_err());
        assert!(resolve_config(None, &env(&[(ENV_COMPACTION_SUMMARY, "own")])).is_ok());
        // The TypeScript ICE_* names are never read.
        let config = resolve_config(None, &env(&[("ICE_COGNEE_ENABLED", "0")])).unwrap();
        assert!(config.enabled);
    }

    #[test]
    fn base_url_rejects_embedded_credentials() {
        assert!(resolve_config(
            None,
            &env(&[(ENV_BASE_URL, "http://user:password@127.0.0.1:8211")])
        )
        .is_err());
        assert!(resolve_config(
            None,
            &env(&[(ENV_BASE_URL, "https://:password@example.test")])
        )
        .is_err());
    }

    #[test]
    fn env_file_parser_and_merge() {
        let parsed = parse_env_file(
            "# c\nexport COGNEE_API_KEY=\"k1\"\nCOGNEE_LOCAL_API_URL='http://l:1'\nICE_COGNEE_ENABLED=0\nICERS_COGNEE_RECALL=0\nOTHER=1\nCOGNEE_API_KEY=k2\n=x\n",
        );
        assert_eq!(parsed.get("COGNEE_API_KEY").map(String::as_str), Some("k2"));
        assert!(!parsed.contains_key("ICE_COGNEE_ENABLED"));
        assert!(!parsed.contains_key("OTHER"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "COGNEE_LOCAL_API_URL=http://l:1\nCOGNEE_API_KEY=file\n",
        )
        .unwrap();
        let merged = merged_env(&env(&[("COGNEE_API_KEY", "process")]), Some(&path));
        assert_eq!(merged["COGNEE_API_KEY"], "process");
        assert_eq!(merged["COGNEE_BASE_URL"], "http://l:1");
    }

    #[test]
    fn credentials_follow_precedence_and_origin_binding() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("store");
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".cognee-plugin")).unwrap();
        std::fs::create_dir_all(&storage).unwrap();
        let inputs = |process: CogneeEnv| ConnectionInputs {
            config_path: storage.join("config.json"),
            storage_dir: storage.clone(),
            home_dir: Some(home.clone()),
            env: None,
            process_env: process,
            api_key: None,
        };
        assert_eq!(
            load_connection(&inputs(env(&[]))).key_source,
            KeySource::None
        );
        std::fs::write(
            home.join(".cognee-plugin/api_key.json"),
            r#"{"api_key":" plugin ","base_url":"http://127.0.0.1:8211/other"}"#,
        )
        .unwrap();
        let connection = load_connection(&inputs(env(&[])));
        assert_eq!(connection.api_key.as_deref(), Some("plugin"));
        assert_eq!(connection.key_source, KeySource::ApiKeyJson);
        std::fs::write(
            storage.join("api_key.json"),
            r#"{"api_key":"icers","base_url":"https://elsewhere.test"}"#,
        )
        .unwrap();
        // A cache for another origin is ignored.
        assert_eq!(
            load_connection(&inputs(env(&[]))).api_key.as_deref(),
            Some("plugin")
        );
        let connection = load_connection(&inputs(env(&[("COGNEE_API_KEY", " p ")])));
        assert_eq!(connection.api_key.as_deref(), Some("p"));
        assert_eq!(connection.key_source, KeySource::Env);
    }

    #[test]
    fn invalid_stored_config_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"topK": 99}"#).unwrap();
        let connection = load_connection(&ConnectionInputs {
            config_path: dir.path().join("config.json"),
            storage_dir: dir.path().into(),
            home_dir: None,
            env: Some(env(&[])),
            process_env: env(&[]),
            api_key: None,
        });
        assert!(connection.invalid_config);
        assert_eq!(connection.config, CogneeConfig::default());
    }

    #[test]
    fn project_dataset_hashes_the_lexical_root() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("My Repo!");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        let dataset = resolve_project_dataset(&repo.join("a/b"));
        assert!(dataset.starts_with("ice-My-Repo-"), "{dataset}");
        assert_eq!(dataset.len(), "ice-My-Repo-".len() + 8);
        assert_eq!(resolve_runtime_dataset("ice", None), "ice");
    }

    #[test]
    fn session_ids_are_stable_or_hashed() {
        assert_eq!(cognee_session_id("019a1b2c-3d4e"), "ice_019a1b2c-3d4e");
        assert_eq!(cognee_session_id("ice_x"), "ice_x");
        assert_eq!(cognee_session_id("  "), "ice_session");
        let sanitized = cognee_session_id("a b");
        assert!(sanitized.starts_with("ice_a_b_"));
        assert_ne!(sanitized, cognee_session_id("a/b"));
        assert!(cognee_session_id(&"x".repeat(200)).len() <= 4 + 120);
    }

    #[test]
    fn save_writes_private_fields_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.json");
        save_config(&path, &CogneeConfig::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(!text.contains("maxResponseChars"));
        assert!(text.starts_with("{\n  \"enabled\": true,"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
