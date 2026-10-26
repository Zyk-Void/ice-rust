//! Read-only import of stdio MCP servers from other tools' config files.
//! Imported servers are the lowest layer: `mcp.json` entries override them by
//! name. Files are only read, never written (see `entry_json` for copy-on-edit).
//! `ice` is deliberately not a kind: AGENTS.md §2.1 forbids reading `~/.ice`.
use super::*;

/// Servers keep the file that provided them, so `/mcp` shows where each came from.
#[derive(Default)]
pub(super) struct Imported {
    pub servers: BTreeMap<String, (ServerConfig, PathBuf)>,
    /// Entries or files that could not be imported, one line each.
    pub notes: Vec<String>,
}

struct Source {
    path: PathBuf,
    /// Project-scoped sources are read only for a trusted project.
    project: bool,
}

/// Lowest to highest precedence within the kind.
fn sources(kind: &str, home: Option<&Path>, cwd: &Path) -> Option<Vec<Source>> {
    let user = |parts: &[&str]| {
        home.map(|home| Source {
            path: parts
                .iter()
                .fold(home.to_path_buf(), |p, part| p.join(part)),
            project: false,
        })
    };
    let project = |parts: &[&str]| Source {
        path: parts.iter().fold(cwd.to_path_buf(), |p, part| p.join(part)),
        project: true,
    };
    let found: Vec<Option<Source>> = match kind {
        "cursor" => vec![user(&[".cursor", "mcp.json"])],
        "claude-code" => vec![
            user(&[".claude", "claude_desktop_config.json"]),
            // Also holds per-project sections, which `servers_of` gates on trust.
            user(&[".claude.json"]),
            user(&[".claude", "mcp.json"]),
        ],
        "claude-desktop" => vec![
            user(&[
                "Library",
                "Application Support",
                "Claude",
                "claude_desktop_config.json",
            ]),
            user(&[".config", "Claude", "claude_desktop_config.json"]),
        ],
        "windsurf" => vec![
            user(&[".codeium", "windsurf", "mcp_config.json"]),
            user(&[".windsurf", "mcp.json"]),
        ],
        "vscode" => vec![Some(project(&[".vscode", "mcp.json"]))],
        "opencode" => vec![
            user(&[".config", "opencode", "opencode.json"]),
            Some(project(&["opencode.json"])),
        ],
        _ => return None,
    };
    Some(found.into_iter().flatten().collect())
}

/// Entries of the kind's server table, with project-scoped ones last.
fn servers_of<'a>(
    kind: &str,
    root: &'a Value,
    cwd: &Path,
    trusted: bool,
) -> Vec<(&'a String, &'a Value)> {
    let table = |value: &'a Value| value.as_object().into_iter().flatten().collect::<Vec<_>>();
    let keys: &[&str] = match kind {
        "opencode" => &["mcp"],
        "vscode" => &["mcpServers", "mcp-servers", "servers"],
        "cursor" | "windsurf" => &["mcpServers", "mcp-servers"],
        _ => &["mcpServers"],
    };
    let mut found = keys
        .iter()
        .find_map(|key| root.get(key))
        .map(table)
        .unwrap_or_default();
    if kind == "claude-code" && trusted {
        if let Some(section) = cwd
            .to_str()
            .and_then(|cwd| root.get("projects")?.get(cwd)?.get("mcpServers"))
        {
            found.extend(table(section));
        }
    }
    found
}

const HTTP_TYPES: [&str; 4] = ["http", "sse", "streamable-http", "remote"];

fn convert(kind: &str, entry: &Value) -> Result<ServerConfig, String> {
    let object = entry.as_object().ok_or("not an object")?;
    if object.contains_key("url")
        || object
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|ty| HTTP_TYPES.contains(&ty))
    {
        return Err("unsupported (http)".into());
    }
    let mut normalized = serde_json::Map::new();
    if kind == "opencode" {
        let command = object
            .get("command")
            .and_then(Value::as_array)
            .filter(|command| !command.is_empty())
            .ok_or("no command")?;
        normalized.insert("command".into(), command[0].clone());
        normalized.insert("args".into(), Value::Array(command[1..].to_vec()));
        if let Some(env) = object.get("environment") {
            normalized.insert("env".into(), env.clone());
        }
        if object.get("enabled") == Some(&Value::Bool(false)) {
            normalized.insert("disabled".into(), Value::Bool(true));
        }
    } else {
        for key in ["command", "args", "env", "disabled"] {
            if let Some(value) = object.get(key) {
                normalized.insert(key.into(), value.clone());
            }
        }
    }
    if let Some(cwd) = object.get("cwd") {
        normalized.insert("cwd".into(), cwd.clone());
    }
    let config: ServerConfig =
        serde_json::from_value(Value::Object(normalized)).map_err(|e| e.to_string())?;
    if config.command.is_empty() {
        return Err("no command".into());
    }
    Ok(config)
}

/// Kinds earlier in `kinds` win name collisions. Failures become notes, never errors,
/// so one unreadable foreign file cannot take down the servers you configured.
pub(super) fn load(kinds: &[String], home: Option<&Path>, cwd: &Path, trusted: bool) -> Imported {
    let mut out = Imported::default();
    let mut seen = HashSet::new();
    for kind in kinds.iter().filter(|kind| seen.insert(kind.as_str())) {
        if kind == "codex" {
            out.notes
                .push("import:codex: unsupported (config.toml needs a TOML parser)".into());
            continue;
        }
        let Some(sources) = sources(kind, home, cwd) else {
            out.notes
                .push(format!("import:{kind}: unknown import kind"));
            continue;
        };
        let mut kind_servers = BTreeMap::new();
        for source in sources.iter().filter(|source| trusted || !source.project) {
            let text = match std::fs::read_to_string(&source.path) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    out.notes
                        .push(format!("import:{kind}: {}: {e}", source.path.display()));
                    continue;
                }
            };
            let root: Value = match serde_json::from_str(&text) {
                Ok(root) => root,
                Err(e) => {
                    out.notes
                        .push(format!("import:{kind}: {}: {e}", source.path.display()));
                    continue;
                }
            };
            for (name, entry) in servers_of(kind, &root, cwd, trusted) {
                if !valid_name(name) {
                    out.notes
                        .push(format!("import:{kind}: {name:?}: invalid server name"));
                    continue;
                }
                match convert(kind, entry) {
                    Ok(config) => {
                        kind_servers.insert(name.clone(), (config, source.path.clone()));
                    }
                    Err(reason) => out.notes.push(format!("import:{kind}: {name}: {reason}")),
                }
            }
        }
        for (name, server) in kind_servers {
            out.servers.entry(name).or_insert(server);
        }
    }
    out
}

/// The `mcp.json` form of an imported server, written when the user first edits it.
pub(super) fn entry_json(config: &ServerConfig) -> Value {
    let mut entry = json!({"command": config.command, "args": config.args});
    if !config.env.is_empty() {
        entry["env"] = json!(config.env);
    }
    if let Some(cwd) = &config.cwd {
        entry["cwd"] = json!(cwd);
    }
    // An edit that does not touch `disabled` must not re-enable the copy.
    if config.disabled {
        entry["disabled"] = json!(true);
    }
    entry
}

#[cfg(test)]
mod tests;
