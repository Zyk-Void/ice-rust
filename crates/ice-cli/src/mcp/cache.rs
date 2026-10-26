//! Last known catalog per server in `<agent dir>/mcp-cache.json` (mode 600), so
//! `/mcp` and the proxy can show servers that are not connected. Ports
//! pi-mcp-adapter `metadata-cache.ts`. An entry is only trusted while its
//! fingerprint matches the server's current launch configuration.
use super::*;

use sha2::{Digest, Sha256};

const FILE: &str = "mcp-cache.json";
const VERSION: u64 = 1;

/// One server's catalog as the server sent it, so it can be rebuilt later.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct CachedServer {
    pub fingerprint: String,
    /// Raw `tools/list` entries.
    #[serde(default)]
    pub tools: Vec<Value>,
    /// Raw `resources/list` entries.
    #[serde(default)]
    pub resources: Vec<Value>,
    #[serde(default)]
    pub prompts: Vec<Prompt>,
    #[serde(default)]
    pub instructions: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    version: u64,
    #[serde(default)]
    servers: BTreeMap<String, CachedServer>,
}

/// Hash of what decides which process (or endpoint) answers, not of
/// presentation settings such as `directTools` or `approveTools`.
pub(crate) fn fingerprint(config: &ServerConfig) -> String {
    let launch = json!({
        "command": config.command,
        "args": config.args,
        "env": config.env,
        "cwd": config.cwd,
    });
    let digest = Sha256::digest(launch.to_string().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read(agent_dir: &Path) -> CacheFile {
    std::fs::read_to_string(agent_dir.join(FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<CacheFile>(&text).ok())
        .filter(|file| file.version == VERSION)
        .unwrap_or_default()
}

/// The cached catalog for `name`, if it was saved for this launch configuration.
pub(crate) fn load(agent_dir: &Path, name: &str, config: &ServerConfig) -> Option<CachedServer> {
    let entry = read(agent_dir).servers.remove(name)?;
    (entry.fingerprint == fingerprint(config)).then_some(entry)
}

/// Replace one server's entry. Best effort: a cache that cannot be written only
/// costs the offline catalog, so failures are logged, never returned.
pub(crate) fn save(agent_dir: &Path, name: &str, entry: CachedServer) {
    let mut file = read(agent_dir);
    file.version = VERSION;
    file.servers.insert(name.to_owned(), entry);
    if let Err(error) = write(agent_dir, &file) {
        tracing::warn!(server = %name, %error, "MCP metadata cache not written");
    }
}

/// Drop entries for servers that are no longer configured.
pub(crate) fn retain(agent_dir: &Path, names: &HashSet<String>) {
    let mut file = read(agent_dir);
    let before = file.servers.len();
    file.servers.retain(|name, _| names.contains(name));
    if file.servers.len() != before {
        file.version = VERSION;
        if let Err(error) = write(agent_dir, &file) {
            tracing::warn!(%error, "MCP metadata cache not pruned");
        }
    }
}

fn write(agent_dir: &Path, file: &CacheFile) -> Result<(), String> {
    std::fs::create_dir_all(agent_dir).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(agent_dir).map_err(|e| e.to_string())?;
    // NamedTempFile is created 600 on Unix; the cache can hold server instructions.
    serde_json::to_writer(&mut temp, file).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(agent_dir.join(FILE))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(command: &str) -> ServerConfig {
        serde_json::from_value(json!({"command":command,"directTools":["a"]})).unwrap()
    }

    #[test]
    fn entries_round_trip_and_follow_the_launch_config() {
        let dir = tempfile::tempdir().unwrap();
        let entry = CachedServer {
            fingerprint: fingerprint(&config("srv")),
            tools: vec![json!({"name":"a","inputSchema":{"type":"object"}})],
            instructions: Some("Use a.".into()),
            ..Default::default()
        };
        save(dir.path(), "s", entry.clone());
        assert_eq!(load(dir.path(), "s", &config("srv")), Some(entry));
        // Presentation settings do not invalidate; a new command does.
        let mut shown = config("srv");
        shown.direct_tools = DirectTools::All(false);
        assert!(load(dir.path(), "s", &shown).is_some());
        assert_eq!(load(dir.path(), "s", &config("other")), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        retain(dir.path(), &HashSet::new());
        assert_eq!(load(dir.path(), "s", &config("srv")), None);
    }

    #[test]
    fn a_corrupt_or_foreign_file_is_ignored_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "{not json").unwrap();
        assert_eq!(load(dir.path(), "s", &config("srv")), None);
        save(
            dir.path(),
            "s",
            CachedServer {
                fingerprint: fingerprint(&config("srv")),
                ..Default::default()
            },
        );
        assert!(load(dir.path(), "s", &config("srv")).is_some());
        std::fs::write(dir.path().join(FILE), r#"{"version":99,"servers":{}}"#).unwrap();
        assert_eq!(load(dir.path(), "s", &config("srv")), None);
    }
}
