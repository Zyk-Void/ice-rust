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
    discovered: Vec<McpTool>,
    prompts: Vec<Prompt>,
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
    /// Catalogs with a refresh queued; coalesces list_changed bursts to one re-list.
    refreshing: Mutex<HashSet<(String, String)>>,
    operation: tokio::sync::Mutex<()>,
    /// Output spill files for the whole session; survives reconnects.
    spill: Arc<output_guard::SpillStore>,
    prompt_changes: watch::Sender<u64>,
    elicitation_ui: Option<UiDialogMailbox>,
    elicitation_notices: tokio::sync::broadcast::Sender<String>,
    /// Set with the dialog host: sampling needs both a UI and callable providers.
    sampling_providers: Option<sampling::Providers>,
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
            spill: Default::default(),
            prompt_changes: watch::channel(0).0,
            elicitation_ui: None,
            elicitation_notices: tokio::sync::broadcast::channel(32).0,
            sampling_providers: None,
        }))
    }
    /// Install only for a TUI host, after `with_elicitation_ui`. `providers` is
    /// the live list `/reload` refreshes, so sampling follows credential changes.
    pub fn with_sampling(mut self, providers: sampling::Providers) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("MCP manager already shared")
            .sampling_providers = Some(providers);
        self
    }
    /// Install only for a TUI host, before startup clones the manager.
    pub fn with_elicitation_ui(mut self, ui: UiDialogMailbox) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("MCP manager already shared")
            .elicitation_ui = Some(ui);
        self
    }
    pub async fn lock_registry(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.operation.lock().await
    }
    pub fn elicitation_notices(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.0.elicitation_notices.subscribe()
    }
    pub fn prompt_changes(&self) -> watch::Receiver<u64> {
        self.0.prompt_changes.subscribe()
    }
    pub fn prompts(&self) -> Vec<(String, Prompt)> {
        self.0
            .servers
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(server, state)| {
                state
                    .prompts
                    .iter()
                    .cloned()
                    .map(|prompt| (server.clone(), prompt))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    pub async fn get_prompt(&self, command: &str, input: &str) -> Result<String, String> {
        let (server, prompt, client) = self
            .0
            .servers
            .lock()
            .unwrap()
            .iter()
            .find_map(|(server, state)| {
                state
                    .prompts
                    .iter()
                    .find(|prompt| prompt.command_name(server) == command)
                    .and_then(|prompt| {
                        state
                            .client
                            .clone()
                            .map(|client| (server.clone(), prompt.clone(), client))
                    })
            })
            .ok_or("MCP prompt is no longer available")?;
        prompts::get_prompt(&client, &server, &prompt, input).await
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
        let client = servers.get(server)?.client.as_ref()?;
        if client.closed.load(Ordering::SeqCst) {
            return None;
        }
        Some(client.instructions()?.to_owned())
    }
    async fn sync_instructions(&self) -> Result<(), String> {
        let instructions = self
            .0
            .servers
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(name, state)| {
                let client = state.client.as_ref()?;
                if client.closed.load(Ordering::SeqCst) {
                    return None;
                }
                Some((name.clone(), client.instructions()?.to_owned()))
            })
            .collect();
        self.0
            .harness
            .upgrade()
            .ok_or("Session ended")?
            .set_mcp_instructions(instructions)
            .await
            .map_err(|error| error.to_string())
    }
    fn watch_instructions(&self, client: &Arc<Client>) {
        let manager = Arc::downgrade(&self.0);
        let client = Arc::downgrade(client);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let Some(client) = client.upgrade() else {
                    break;
                };
                if client.closed.load(Ordering::SeqCst) {
                    if let Some(inner) = manager.upgrade() {
                        let manager = McpManager(inner);
                        let _guard = manager.0.operation.lock().await;
                        let _ = manager.sync_instructions().await;
                    }
                    break;
                }
                if manager.strong_count() == 0 {
                    break;
                }
            }
        });
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
    /// Each server with the file providing it: `mcp.json`, the project file, or an
    /// imported tool's config.
    fn configured(&self) -> Result<BTreeMap<String, (ServerConfig, PathBuf)>, String> {
        Ok(load_layered(&self.0.agent_dir, &self.0.cwd, self.0.trusted)?.servers)
    }
    /// Imports that were skipped (unsupported transport, unreadable file, unknown kind).
    pub fn import_notes(&self) -> Vec<String> {
        load_layered(&self.0.agent_dir, &self.0.cwd, self.0.trusted)
            .map(|layered| layered.notes)
            .unwrap_or_default()
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
        let settings = load_settings(&self.0.agent_dir, &self.0.cwd, self.0.trusted)?;
        let guard = OutputGuard {
            store: self.0.spill.clone(),
            limits: output_guard::resolve(settings.get("outputGuard")),
        };
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
            self.sync_instructions().await?;
            self.0
                .prompt_changes
                .send_modify(|version| *version = version.wrapping_add(1));
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
                    discovered: Vec::new(),
                    prompts: Vec::new(),
                },
            );
            if disabled {
                continue;
            }
            let cwd = self.0.cwd.clone();
            let handlers = self.handlers(&name);
            let guard = guard.clone();
            discoveries.spawn(async move {
                let result = async {
                    let client = Client::spawn(&name, &config, &cwd, handlers)?;
                    let tools = tokio::time::timeout(
                        START_TIMEOUT,
                        client.discover(&name, &config, &guard),
                    )
                    .await
                    .map_err(|_| "MCP discovery timed out".to_string())??;
                    let prompts = tokio::time::timeout(START_TIMEOUT, client.list_prompts())
                        .await
                        .map_err(|_| "MCP prompts/list timed out".to_string())??;
                    Ok::<_, String>((client, tools, prompts))
                }
                .await;
                (name, result)
            });
        }
        while let Some(result) = discoveries.join_next().await {
            let (name, discovered) = result.map_err(|e| e.to_string())?;
            let result = async {
                let (client, discovered, prompts) = discovered?;
                let direct_tools = self.0.servers.lock().unwrap()[&name]
                    .config
                    .direct_tools
                    .clone();
                let (names, catalog) = self
                    .register(&harness, &direct_tools, discovered.clone())
                    .await?;
                self.watch_instructions(&client);
                watch_shutdown(
                    harness.downgrade(),
                    Arc::downgrade(&client),
                    self.0.spill.clone(),
                );
                Ok::<_, String>((client, names, catalog, discovered, prompts))
            }
            .await;
            let mut servers = self.0.servers.lock().unwrap();
            let state = servers.get_mut(&name).unwrap();
            match result {
                Ok((client, tools, catalog, discovered, prompts)) => {
                    state.prompts = prompts;
                    state.discovered = discovered;
                    state.client = Some(client);
                    state.status = ServerStatus::Connected { tools };
                    state.catalog = Some(catalog);
                }
                Err(error) => state.status = ServerStatus::Failed(error),
            }
        }
        self.sync_instructions().await?;
        self.0
            .prompt_changes
            .send_modify(|version| *version = version.wrapping_add(1));
        Ok(())
    }
    /// Re-list only the changed catalog, then register real tools before resources.
    async fn refresh_catalog(&self, server: &str, catalog: &str) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        // Cleared before listing so a change announced mid-list queues one more pass.
        self.0
            .refreshing
            .lock()
            .unwrap()
            .remove(&(server.into(), catalog.into()));
        let (client, config, old, mut discovered) = match self.0.servers.lock().unwrap().get(server)
        {
            Some(ServerState {
                client: Some(client),
                config,
                status: ServerStatus::Connected { tools },
                discovered,
                ..
            }) if client.declares(catalog) => (
                client.clone(),
                config.clone(),
                tools.clone(),
                discovered.clone(),
            ),
            _ => return Ok(()),
        };
        if catalog == "prompts" {
            let prompts = tokio::time::timeout(START_TIMEOUT, client.list_prompts())
                .await
                .map_err(|_| "MCP prompts/list timed out".to_string())??;
            if let Some(state) = self.0.servers.lock().unwrap().get_mut(server) {
                state.prompts = prompts;
            }
            self.0
                .prompt_changes
                .send_modify(|version| *version = version.wrapping_add(1));
            return Ok(());
        }
        let settings = load_settings(&self.0.agent_dir, &self.0.cwd, self.0.trusted)?;
        let guard = OutputGuard {
            store: self.0.spill.clone(),
            limits: output_guard::resolve(settings.get("outputGuard")),
        };
        let resources = catalog == "resources";
        let updated = tokio::time::timeout(START_TIMEOUT, async {
            if resources {
                client.list_resources(server, &config, &guard).await
            } else {
                client.list_tools(server, &config, &guard).await
            }
        })
        .await
        .map_err(|_| format!("MCP {catalog}/list timed out"))??;
        discovered.retain(|tool| tool.resource_uri.is_some() != resources);
        discovered.extend(updated);
        discovered.sort_by_key(|tool| tool.resource_uri.is_some());
        let harness = self.0.harness.upgrade().ok_or("Session ended")?;
        self.unregister(&harness, &old).await?;
        let (tools, listed) = self
            .register(&harness, &config.direct_tools, discovered.clone())
            .await?;
        if let Some(state) = self.0.servers.lock().unwrap().get_mut(server) {
            state.discovered = discovered;
            state.status = ServerStatus::Connected { tools };
            state.catalog = Some(listed);
        }
        Ok(())
    }
    pub(super) fn handlers(&self, server: &str) -> Handlers {
        let manager = Arc::downgrade(&self.0);
        let elicitation_server = server;
        let server = server.to_owned();
        let mut handlers = Handlers {
            list_changed: Some(Arc::new(move |catalog| {
                if !matches!(catalog, "tools" | "resources" | "prompts") {
                    return;
                }
                let Some(inner) = manager.upgrade() else {
                    return;
                };
                if !inner
                    .refreshing
                    .lock()
                    .unwrap()
                    .insert((server.clone(), catalog.into()))
                {
                    return;
                }
                let server = server.clone();
                let catalog = catalog.to_owned();
                tokio::spawn(async move {
                    if let Err(error) = McpManager(inner).refresh_catalog(&server, &catalog).await {
                        tracing::warn!(%server, %catalog, %error, "MCP catalog refresh failed");
                    }
                });
            })),
            ..Default::default()
        };
        // Unreadable settings disable server-initiated requests rather than enable them.
        let settings = load_settings(&self.0.agent_dir, &self.0.cwd, self.0.trusted).ok();
        let flag = |key: &str, default: bool| {
            settings.as_ref().is_some_and(|settings| {
                settings
                    .get(key)
                    .and_then(Value::as_bool)
                    .unwrap_or(default)
            })
        };
        let Some(ui) = &self.0.elicitation_ui else {
            return handlers;
        };
        if flag("elicitation", true) {
            handlers = elicitation::register(
                handlers,
                elicitation_server,
                ui.clone(),
                self.0.elicitation_notices.clone(),
            );
        }
        if let Some(providers) = self
            .0
            .sampling_providers
            .as_ref()
            .filter(|_| flag("sampling", true))
        {
            let harness = self.0.harness.clone();
            handlers = sampling::register(
                handlers,
                elicitation_server,
                ui.clone(),
                providers.clone(),
                Arc::new(move || {
                    let harness = harness.upgrade();
                    Box::pin(async move { harness?.lane("main").get_model().await.ok() })
                }),
                flag("samplingAutoApprove", false),
            );
        }
        handlers
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
            // Resource tools are gated by `exposeResources`, not `directTools`.
            let resource = tool.resource_uri.is_some();
            let direct = resource || direct_tools.allows(&tool.remote_name);
            if !resource {
                catalog.push(ToolInfo {
                    name: tool.remote_name.clone(),
                    description: tool.schema.description.clone(),
                    tokens: tool.tokens,
                    direct,
                });
            }
            let name = tool.schema.name.clone();
            // Without a proxy tool, a tool that is not direct is not registered.
            if !direct {
                continue;
            }
            if !crate::session::tool_name_allowed(&name, &self.0.args) {
                continue;
            }
            if tools.iter().any(|t| t.tool.schema().name == name) {
                tracing::warn!(%name, resource = tool.resource_uri.is_some(), "Skipping colliding MCP tool");
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
        let (config, source) = self
            .configured()?
            .remove(name)
            .ok_or_else(|| format!("Unknown MCP server: {name}"))?;
        edit_config(&self.edit_path(&source)?, |servers| {
            servers
                .entry(name)
                .or_insert_with(|| import::entry_json(&config))
                .as_object_mut()
                .ok_or("Invalid server entry")?
                .insert("disabled".into(), json!(disabled));
            Ok(())
        })?;
        self.reconnect_locked(Some(name)).await
    }
    /// The file an edit to a server from `source` is written to. An imported
    /// server lives in another tool's file, which is never written: the edit
    /// lands in our global config, seeded with the imported entry.
    fn edit_path(&self, source: &Path) -> Result<PathBuf, String> {
        let project = self.0.cwd.join(".icers/mcp.json");
        if source == project {
            Ok(project)
        } else {
            self.config_path(false)
        }
    }
    /// Persist staged panel edits to each server's config file, then restart
    /// the changed servers. Servers whose file was written are restarted even
    /// when a later file fails, so live state matches disk.
    pub async fn save_panel(&self, changes: &[PanelChange]) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        let configured = self.configured()?;
        let mut by_file = BTreeMap::<PathBuf, Vec<(&PanelChange, &ServerConfig)>>::new();
        for change in changes {
            let (config, source) = configured
                .get(&change.server)
                .ok_or_else(|| format!("Unknown MCP server: {}", change.server))?;
            by_file
                .entry(self.edit_path(source)?)
                .or_default()
                .push((change, config));
        }
        let mut written = HashSet::new();
        let mut result = Ok(());
        for (path, changes) in by_file {
            result = edit_config(&path, |servers| {
                for (change, config) in &changes {
                    let entry = servers
                        .entry(change.server.as_str())
                        .or_insert_with(|| import::entry_json(config))
                        .as_object_mut()
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
            written.extend(changes.iter().map(|(c, _)| c.server.clone()));
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
    if valid_name(name) {
        Ok(())
    } else {
        Err(format!("invalid MCP server name: {name:?}"))
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
