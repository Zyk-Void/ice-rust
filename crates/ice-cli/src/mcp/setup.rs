//! Data behind `/mcp setup`: detected import sources, name conflicts across
//! layers, and quick-add presets. Ports pi-mcp-adapter's setup summary
//! (`config.ts` `getMcpDiscoverySummary`, `getConfigConflicts`) and its
//! `KNOWN_SERVER_PRESETS`.
use super::*;

pub use import::Detected;

#[derive(Clone, Debug, PartialEq)]
pub enum PresetEntry {
    Stdio {
        command: &'static str,
        args: &'static [&'static str],
    },
    /// A remote server; `oauth` presets sign in with `/mcp-auth` (step 7.5).
    Http { url: &'static str, oauth: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    /// Also the server name written to `mcp.json`.
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub entry: PresetEntry,
}

/// pi-mcp-adapter 2.32.1 `KNOWN_SERVER_PRESETS`, in its order.
pub const PRESETS: [Preset; 6] = [
    Preset {
        id: "deepwiki",
        name: "DeepWiki",
        summary: "Ask questions about public GitHub repositories.",
        entry: PresetEntry::Http {
            url: "https://mcp.deepwiki.com/mcp",
            oauth: false,
        },
    },
    Preset {
        id: "context7",
        name: "Context7",
        summary: "Look up current library documentation and examples.",
        entry: PresetEntry::Http {
            url: "https://mcp.context7.com/mcp",
            oauth: false,
        },
    },
    Preset {
        id: "parallel-search",
        name: "Parallel Search",
        summary: "Search the web and fetch pages without an API key.",
        entry: PresetEntry::Http {
            url: "https://search.parallel.ai/mcp",
            oauth: false,
        },
    },
    Preset {
        id: "notion",
        name: "Notion",
        summary: "Search and work with your Notion workspace.",
        entry: PresetEntry::Http {
            url: "https://mcp.notion.com/mcp",
            oauth: true,
        },
    },
    Preset {
        id: "github",
        name: "GitHub",
        summary: "Work with GitHub through your Copilot account.",
        entry: PresetEntry::Http {
            url: "https://api.githubcopilot.com/mcp",
            oauth: true,
        },
    },
    Preset {
        id: "chrome-devtools",
        name: "Chrome DevTools",
        summary: "Inspect and automate a local Chrome browser.",
        entry: PresetEntry::Stdio {
            command: "npx",
            args: &["-y", "chrome-devtools-mcp@1.6.0"],
        },
    },
];

impl Preset {
    /// The `mcp.json` server entry this preset writes.
    pub fn entry_json(&self) -> Value {
        match &self.entry {
            PresetEntry::Stdio { command, args } => json!({"command":command,"args":args}),
            PresetEntry::Http { url, oauth: false } => json!({"url":url}),
            PresetEntry::Http { url, oauth: true } => json!({"url":url,"auth":"oauth"}),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportSource {
    pub detected: Detected,
    /// Listed in `imports` of the global or the trusted project `mcp.json`.
    pub enabled: bool,
}

/// A server name defined by more than one source; `winner` provides it.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub server: String,
    pub winner: PathBuf,
    pub shadowed: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetupView {
    pub imports: Vec<ImportSource>,
    pub conflicts: Vec<Conflict>,
    /// Imports that were skipped, as `--list-mcp` reports them.
    pub notes: Vec<String>,
    /// Server names already configured, so presets can show as added.
    pub configured: Vec<String>,
}

pub(super) fn view(
    home: Option<&Path>,
    agent_dir: &Path,
    cwd: &Path,
    trusted: bool,
) -> Result<SetupView, String> {
    let layered = load_layered_from(home, agent_dir, cwd, trusted)?;
    let enabled: HashSet<String> = read_configs(agent_dir, cwd, trusted)?
        .into_iter()
        .flat_map(|(_, config)| config.imports)
        .collect();
    let imports = import::detect(home, cwd, trusted)
        .into_iter()
        .map(|detected| ImportSource {
            enabled: enabled.contains(detected.kind),
            detected,
        })
        .collect();
    let mut shadowed = BTreeMap::<String, Vec<PathBuf>>::new();
    for (name, path) in layered.shadowed {
        shadowed.entry(name).or_default().push(path);
    }
    let conflicts = shadowed
        .into_iter()
        .filter_map(|(server, shadowed)| {
            Some(Conflict {
                winner: layered.servers.get(&server)?.1.clone(),
                server,
                shadowed,
            })
        })
        .collect();
    Ok(SetupView {
        imports,
        conflicts,
        notes: layered.notes,
        configured: layered.servers.into_keys().collect(),
    })
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
