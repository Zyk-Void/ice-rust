use super::*;

#[derive(Clone, Debug)]
pub enum ServerStatus {
    Disabled,
    Connecting,
    Connected { tools: Vec<String> },
    Failed(String),
}
/// One discovered tool. `name` is the remote name used by `directTools`.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub tokens: usize,
    pub direct: bool,
}
#[derive(Clone, Debug)]
pub struct ServerView {
    pub name: String,
    pub source: PathBuf,
    pub status: ServerStatus,
    pub disabled: bool,
    /// Last discovered catalog; `None` until the server has connected once.
    pub catalog: Option<Vec<ToolInfo>>,
}
/// A staged `/mcp` panel edit for one server; `None` fields stay unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelChange {
    pub server: String,
    pub disabled: Option<bool>,
    pub direct_tools: Option<DirectTools>,
}
struct ServerState {
    config: ServerConfig,
    source: PathBuf,
    status: ServerStatus,
    client: Option<Arc<Client>>,
    catalog: Option<Vec<ToolInfo>>,
}
struct Inner {
    harness: ice_harness::agent_harness::WeakAgentHarness,
    args: crate::args::Args,
    cwd: PathBuf,
    agent_dir: PathBuf,
    trusted: bool,
    approvals: Arc<Mutex<HashSet<String>>>,
    tools: Mutex<Vec<HarnessTool>>,
    servers: Mutex<BTreeMap<String, ServerState>>,
    /// Servers with a tool refresh queued; coalesces list_changed bursts to one re-list.
    refreshing: Mutex<HashSet<String>>,
    operation: tokio::sync::Mutex<()>,
}
#[derive(Clone)]
pub struct McpManager(Arc<Inner>);
impl McpManager {
    pub fn new(
        harness: &AgentHarness,
        args: &crate::args::Args,
        cwd: &Path,
        agent_dir: PathBuf,
        trusted: bool,
        approvals: Arc<Mutex<HashSet<String>>>,
    ) -> Self {
        Self(Arc::new(Inner {
            harness: harness.downgrade(),
            args: args.clone(),
            cwd: cwd.into(),
            agent_dir,
            trusted,
            approvals,
            tools: Mutex::new(Vec::new()),
            servers: Mutex::new(BTreeMap::new()),
            refreshing: Mutex::new(HashSet::new()),
            operation: tokio::sync::Mutex::new(()),
        }))
    }
    pub async fn lock_registry(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.operation.lock().await
    }
    pub fn registered_tools(&self) -> Vec<HarnessTool> {
        self.0.tools.lock().unwrap().clone()
    }
    pub fn snapshot(&self) -> Vec<ServerView> {
        self.0
            .servers
            .lock()
            .unwrap()
            .iter()
            .map(|(name, s)| ServerView {
                name: name.clone(),
                source: s.source.clone(),
                status: s.status.clone(),
                disabled: s.config.disabled,
                catalog: s.catalog.clone(),
            })
            .collect()
    }
    pub fn tools(&self, server: &str) -> Vec<(String, bool)> {
        let names = match self
            .0
            .servers
            .lock()
            .unwrap()
            .get(server)
            .map(|s| s.status.clone())
        {
            Some(ServerStatus::Connected { tools }) => tools,
            _ => Vec::new(),
        };
        let approvals = self.0.approvals.lock().unwrap();
        names
            .into_iter()
            .map(|name| {
                let approval = approvals.contains(&name);
                (name, approval)
            })
            .collect()
    }
    /// Instructions from the server's `initialize` result, while it is connected.
    pub fn instructions(&self, server: &str) -> Option<String> {
        let servers = self.0.servers.lock().unwrap();
        Some(
            servers
                .get(server)?
                .client
                .as_ref()?
                .instructions()?
                .to_owned(),
        )
    }
    pub fn config_path(&self, project: bool) -> Result<PathBuf, String> {
        if project && !self.0.trusted {
            return Err("Project MCP configuration requires project trust".into());
        }
        Ok(if project {
            self.0.cwd.join(".icers/mcp.json")
        } else {
            self.0.agent_dir.join("mcp.json")
        })
    }
    fn configured(&self) -> Result<BTreeMap<String, (ServerConfig, PathBuf)>, String> {
        let servers = load_config(&self.0.agent_dir, &self.0.cwd, self.0.trusted)?;
        let project = self.0.cwd.join(".icers/mcp.json");
        let overrides: Config = if self.0.trusted && project.exists() {
            serde_json::from_str(&std::fs::read_to_string(&project).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        } else {
            Config::default()
        };
        Ok(servers
            .into_iter()
            .map(|(name, config)| {
                let source = if overrides.servers.contains_key(&name) {
                    project.clone()
                } else {
                    self.0.agent_dir.join("mcp.json")
                };
                (name, (config, source))
            })
            .collect())
    }
    pub async fn reconnect(&self, name: Option<&str>) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        self.reconnect_locked(name).await
    }
    async fn reconnect_locked(&self, name: Option<&str>) -> Result<(), String> {
        if let Some(name) = name {
            if !self.configured()?.contains_key(name)
                && !self.0.servers.lock().unwrap().contains_key(name)
            {
                return Err(format!("Unknown MCP server: {name}"));
            }
        }
        self.reconnect_servers(name.map(|n| HashSet::from([n.to_owned()])))
            .await
    }
    /// Restart `only` (or every known server) concurrently from current configuration.
    async fn reconnect_servers(&self, only: Option<HashSet<String>>) -> Result<(), String> {
        let mut configured = self.configured()?;
        let mut targets: HashSet<String> = configured
            .keys()
            .cloned()
            .chain(self.0.servers.lock().unwrap().keys().cloned())
            .filter(|n| only.as_ref().map_or(true, |only| only.contains(n)))
            .collect();
        let harness = self.0.harness.upgrade().ok_or("Session ended")?;
        let mut discoveries = tokio::task::JoinSet::new();
        for name in targets.drain() {
            let old = self.0.servers.lock().unwrap().remove(&name);
            let names = match &old {
                Some(ServerState {
                    status: ServerStatus::Connected { tools },
                    ..
                }) => tools.clone(),
                _ => Vec::new(),
            };
            let old_catalog = old.as_ref().and_then(|s| s.catalog.clone());
            if let Some(client) = old.and_then(|s| s.client) {
                client.closed.store(true, Ordering::SeqCst);
                client._process.tree.kill();
                fail_pending(&client.pending, "MCP server disconnected");
            }
            self.unregister(&harness, &names).await?;
            let Some((config, source)) = configured.remove(&name) else {
                continue;
            };
            let disabled = config.disabled;
            // Keep the catalog visible while the server is disabled or failing.
            let catalog = old_catalog.map(|catalog| {
                catalog
                    .into_iter()
                    .map(|tool| ToolInfo {
                        direct: config.direct_tools.allows(&tool.name),
                        ..tool
                    })
                    .collect()
            });
            self.0.servers.lock().unwrap().insert(
                name.clone(),
                ServerState {
                    config: config.clone(),
                    source,
                    status: if disabled {
                        ServerStatus::Disabled
                    } else {
                        ServerStatus::Connecting
                    },
                    client: None,
                    catalog,
                },
            );
            if disabled {
                continue;
            }
            let cwd = self.0.cwd.clone();
            let handlers = self.handlers(&name);
            discoveries.spawn(async move {
                let result = async {
                    let client = Client::spawn(&name, &config, &cwd, handlers)?;
                    let tools =
                        tokio::time::timeout(START_TIMEOUT, client.discover(&name, &config))
                            .await
                            .map_err(|_| "MCP discovery timed out".to_string())??;
                    Ok::<_, String>((client, tools))
                }
                .await;
                (name, result)
            });
        }
        while let Some(result) = discoveries.join_next().await {
            let (name, discovered) = result.map_err(|e| e.to_string())?;
            let result = async {
                let (client, discovered) = discovered?;
                let direct_tools = self.0.servers.lock().unwrap()[&name]
                    .config
                    .direct_tools
                    .clone();
                let (names, catalog) = self.register(&harness, &direct_tools, discovered).await?;
                watch_shutdown(harness.downgrade(), Arc::downgrade(&client));
                Ok::<_, String>((client, names, catalog))
            }
            .await;
            let mut servers = self.0.servers.lock().unwrap();
            let state = servers.get_mut(&name).unwrap();
            match result {
                Ok((client, tools, catalog)) => {
                    state.client = Some(client);
                    state.status = ServerStatus::Connected { tools };
                    state.catalog = Some(catalog);
                }
                Err(error) => state.status = ServerStatus::Failed(error),
            }
        }
        Ok(())
    }
    /// The server keeps its process; only its tool catalog is re-listed and swapped.
    async fn refresh_tools(&self, server: &str) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        // Cleared before listing so a change announced mid-list queues one more pass.
        self.0.refreshing.lock().unwrap().remove(server);
        let (client, config, old) = match self.0.servers.lock().unwrap().get(server) {
            Some(ServerState {
                client: Some(client),
                config,
                status: ServerStatus::Connected { tools },
                ..
            }) if client.declares("tools") => (client.clone(), config.clone(), tools.clone()),
            _ => return Ok(()),
        };
        let discovered = tokio::time::timeout(START_TIMEOUT, client.list_tools(server, &config))
            .await
            .map_err(|_| "MCP tools/list timed out".to_string())??;
        let harness = self.0.harness.upgrade().ok_or("Session ended")?;
        self.unregister(&harness, &old).await?;
        let (tools, catalog) = self
            .register(&harness, &config.direct_tools, discovered)
            .await?;
        if let Some(state) = self.0.servers.lock().unwrap().get_mut(server) {
            state.status = ServerStatus::Connected { tools };
            state.catalog = Some(catalog);
        }
        Ok(())
    }
    fn handlers(&self, server: &str) -> Handlers {
        let manager = Arc::downgrade(&self.0);
        let server = server.to_owned();
        Handlers {
            list_changed: Some(Arc::new(move |catalog| {
                // Prompts and resources have no catalog yet; they are not discovered.
                if catalog != "tools" {
                    return;
                }
                let Some(inner) = manager.upgrade() else {
                    return;
                };
                if !inner.refreshing.lock().unwrap().insert(server.clone()) {
                    return;
                }
                let server = server.clone();
                tokio::spawn(async move {
                    if let Err(error) = McpManager(inner).refresh_tools(&server).await {
                        tracing::warn!(%server, %error, "MCP tool refresh failed");
                    }
                });
            })),
            ..Default::default()
        }
    }
    async fn unregister(&self, harness: &AgentHarness, names: &[String]) -> Result<(), String> {
        let mut tools = harness.get_tools().await.map_err(|e| e.to_string())?;
        let mut active = harness
            .lane("main")
            .get_active_tools()
            .await
            .map_err(|e| e.to_string())?;
        tools.retain(|t| !names.contains(&t.tool.schema().name));
        active.retain(|n| !names.contains(n));
        self.0
            .tools
            .lock()
            .unwrap()
            .retain(|t| !names.contains(&t.tool.schema().name));
        self.0
            .approvals
            .lock()
            .unwrap()
            .retain(|n| !names.contains(n));
        harness
            .set_tools(tools, Some(active))
            .await
            .map_err(|e| e.to_string())
    }
    /// Returns the names actually registered and the full catalog. Tools that are not
    /// direct, disallowed, or colliding are skipped but still listed in the catalog.
    async fn register(
        &self,
        harness: &AgentHarness,
        direct_tools: &DirectTools,
        discovered: Vec<McpTool>,
    ) -> Result<(Vec<String>, Vec<ToolInfo>), String> {
        let mut tools = harness.get_tools().await.map_err(|e| e.to_string())?;
        let mut active = harness
            .lane("main")
            .get_active_tools()
            .await
            .map_err(|e| e.to_string())?;
        let mut names = Vec::new();
        let mut catalog = Vec::new();
        for tool in discovered {
            let direct = direct_tools.allows(&tool.remote_name);
            catalog.push(ToolInfo {
                name: tool.remote_name.clone(),
                description: tool.schema.description.clone(),
                tokens: tool.tokens,
                direct,
            });
            let name = tool.schema.name.clone();
            // Without a proxy tool, a tool that is not direct is not registered.
            if !direct {
                continue;
            }
            if !crate::session::tool_name_allowed(&name, &self.0.args)
                || tools.iter().any(|t| t.tool.schema().name == name)
            {
                continue;
            }
            if tool.approval {
                self.0.approvals.lock().unwrap().insert(name.clone());
            }
            names.push(name.clone());
            active.push(name);
            let tool = HarnessTool::new(Arc::new(tool));
            self.0.tools.lock().unwrap().push(tool.clone());
            tools.push(tool);
        }
        harness
            .set_tools(tools, Some(active))
            .await
            .map_err(|e| e.to_string())?;
        Ok((names, catalog))
    }
    pub async fn set_disabled(&self, name: &str, disabled: bool) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        let path = self
            .configured()?
            .remove(name)
            .ok_or_else(|| format!("Unknown MCP server: {name}"))?
            .1;
        edit_config(&path, |servers| {
            servers
                .get_mut(name)
                .and_then(Value::as_object_mut)
                .ok_or("Invalid server entry")?
                .insert("disabled".into(), json!(disabled));
            Ok(())
        })?;
        self.reconnect_locked(Some(name)).await
    }
    /// Persist staged panel edits to each server's source file, then restart
    /// the changed servers. Servers whose file was written are restarted even
    /// when a later file fails, so live state matches disk.
    pub async fn save_panel(&self, changes: &[PanelChange]) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        let configured = self.configured()?;
        let mut by_file = BTreeMap::<PathBuf, Vec<&PanelChange>>::new();
        for change in changes {
            let (_, path) = configured
                .get(&change.server)
                .ok_or_else(|| format!("Unknown MCP server: {}", change.server))?;
            by_file.entry(path.clone()).or_default().push(change);
        }
        let mut written = HashSet::new();
        let mut result = Ok(());
        for (path, changes) in by_file {
            result = edit_config(&path, |servers| {
                for change in &changes {
                    let entry = servers
                        .get_mut(&change.server)
                        .and_then(Value::as_object_mut)
                        .ok_or("Invalid server entry")?;
                    if let Some(disabled) = change.disabled {
                        entry.insert("disabled".into(), json!(disabled));
                    }
                    if let Some(direct) = &change.direct_tools {
                        entry.insert("directTools".into(), direct.to_json());
                    }
                }
                Ok(())
            });
            if result.is_err() {
                break;
            }
            written.extend(changes.iter().map(|c| c.server.clone()));
        }
        if !written.is_empty() {
            self.reconnect_servers(Some(written)).await?;
        }
        result
    }
    pub async fn add(&self, name: &str, command: &str, args: &[String]) -> Result<(), String> {
        validate_name(name)?;
        if command.is_empty() {
            return Err("Missing MCP command".into());
        }
        let _guard = self.0.operation.lock().await;
        edit_config(&self.config_path(false)?, |servers| {
            if servers.contains_key(name) {
                return Err(format!("MCP server already exists: {name}"));
            }
            servers.insert(name.into(), json!({"command":command,"args":args}));
            Ok(())
        })?;
        self.reconnect_locked(Some(name)).await
    }
    pub async fn remove(&self, name: &str) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        edit_config(&self.config_path(false)?, |servers| {
            servers
                .remove(name)
                .ok_or_else(|| format!("No global MCP server: {name}"))?;
            Ok(())
        })?;
        self.reconnect_locked(Some(name)).await
    }
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Err(format!("invalid MCP server name: {name:?}"))
    } else {
        Ok(())
    }
}
pub fn ensure_config(path: &Path) -> Result<(), String> {
    if !path.exists() {
        edit_config(path, |_| Ok(()))?;
    }
    Ok(())
}
fn edit_config(
    path: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), String>,
) -> Result<(), String> {
    let mut value: Value = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"mcpServers":{}}),
        Err(e) => return Err(e.to_string()),
    };
    let object = value
        .as_object_mut()
        .ok_or("MCP config must be an object")?;
    let servers = object
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("mcpServers must be an object")?;
    edit(servers)?;
    // Validate before writing, preserving unrecognized keys in the original value.
    let _: Config = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Replace atomically: a failed write must not truncate the live configuration.
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))
        .map_err(|e| e.to_string())?;
    if let Ok(metadata) = std::fs::metadata(path) {
        file.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    writeln!(
        file,
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
