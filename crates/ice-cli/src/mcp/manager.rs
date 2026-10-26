use super::*;

const LIFECYCLE_TICK: Duration = Duration::from_secs(15);
const DEFAULT_IDLE_MINUTES: f64 = 10.0;
const RETRY_FIRST: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub enum ServerStatus {
    Disabled,
    Connecting,
    Connected {
        tools: Vec<String>,
    },
    /// A lazy server that is not running; `tools` were registered from the
    /// metadata cache and start the server on first use.
    Idle {
        tools: Vec<String>,
    },
    Failed(String),
}
impl ServerStatus {
    /// Tool names this state registered, which a restart must unregister.
    fn registered(&self) -> &[String] {
        match self {
            ServerStatus::Connected { tools } | ServerStatus::Idle { tools } => tools,
            _ => &[],
        }
    }
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
pub(super) struct ServerState {
    config: ServerConfig,
    source: PathBuf,
    status: ServerStatus,
    pub(super) client: Option<Arc<Client>>,
    catalog: Option<Vec<ToolInfo>>,
    pub(super) discovered: Vec<McpTool>,
    prompts: Vec<Prompt>,
}
pub(super) struct Inner {
    harness: ice_harness::agent_harness::WeakAgentHarness,
    args: crate::args::Args,
    cwd: PathBuf,
    agent_dir: PathBuf,
    trusted: bool,
    approvals: Arc<Mutex<HashSet<String>>>,
    tools: Mutex<Vec<HarnessTool>>,
    pub(super) servers: Mutex<BTreeMap<String, ServerState>>,
    /// MCP UI: loopback server, open app windows, queued app messages, consent.
    pub(super) ui: ui::Host,
    /// Keep-alive restarts: when the next attempt is due, and the current backoff.
    retries: Mutex<HashMap<String, (std::time::Instant, Duration)>>,
    /// Set once the lifecycle ticker runs.
    ticking: AtomicBool,
    /// OAuth sign-ins in progress: state, the pending exchange, and its listener's stop.
    #[allow(clippy::type_complexity)]
    signins: Mutex<
        HashMap<
            String,
            (
                String,
                Arc<Mutex<Option<oauth::Pending>>>,
                CancellationToken,
            ),
        >,
    >,
    /// Catalogs with a refresh queued; coalesces list_changed bursts to one re-list.
    refreshing: Mutex<HashSet<(String, String)>>,
    operation: tokio::sync::Mutex<()>,
    /// Output spill files for the whole session; survives reconnects.
    spill: Arc<output_guard::SpillStore>,
    prompt_changes: watch::Sender<u64>,
    pub(super) elicitation_ui: Option<UiDialogMailbox>,
    /// Asks approval for proxied calls; the same mailbox the approval hook uses.
    pub(super) approval_ui: Option<UiDialogMailbox>,
    pub(super) elicitation_notices: tokio::sync::broadcast::Sender<String>,
    /// Set with the dialog host: sampling needs both a UI and callable providers.
    sampling_providers: Option<sampling::Providers>,
}
#[derive(Clone)]
pub struct McpManager(pub(super) Arc<Inner>);
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
            retries: Mutex::new(HashMap::new()),
            ticking: AtomicBool::new(false),
            signins: Mutex::new(HashMap::new()),
            ui: ui::Host::default(),
            operation: tokio::sync::Mutex::new(()),
            spill: Default::default(),
            prompt_changes: watch::channel(0).0,
            elicitation_ui: None,
            approval_ui: None,
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
    /// Install before startup clones the manager. Without it, proxied calls to
    /// tools that require approval are refused, like direct ones without a UI.
    pub fn with_approval_ui(mut self, ui: UiDialogMailbox) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("MCP manager already shared")
            .approval_ui = Some(ui);
        self
    }
    /// Install only for a TUI host, before startup clones the manager.
    pub fn with_elicitation_ui(mut self, ui: UiDialogMailbox) -> Self {
        let inner = Arc::get_mut(&mut self.0).expect("MCP manager already shared");
        inner.elicitation_ui = Some(ui);
        // A TUI also submits prompts that MCP apps send.
        inner.ui.prompts = Some(tokio::sync::broadcast::channel(16).0);
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
        let find = || {
            self.0
                .servers
                .lock()
                .unwrap()
                .iter()
                .find_map(|(server, state)| {
                    let prompt = state
                        .prompts
                        .iter()
                        .find(|prompt| prompt.command_name(server) == command)?;
                    Some((server.clone(), prompt.clone(), state.client.clone()))
                })
        };
        let (server, prompt, client) = find().ok_or("MCP prompt is no longer available")?;
        let client = match client {
            Some(client) => client,
            // An idle lazy server lists cached prompts; start it for this one.
            None => {
                self.connect_lazy(&server).await?;
                find()
                    .and_then(|(_, _, client)| client)
                    .ok_or("MCP prompt is no longer available")?
            }
        };
        let text = prompts::get_prompt(&client, &server, &prompt, input).await?;
        // The text becomes a user message, so it gets the same bound as tool output.
        Ok(self.output_guard()?.bound_text(text))
    }
    /// The session's output guard with the current `settings.outputGuard`.
    pub(super) fn output_guard(&self) -> Result<OutputGuard, String> {
        let settings = load_settings(&self.0.agent_dir, &self.0.cwd, self.0.trusted)?;
        Ok(OutputGuard {
            store: self.0.spill.clone(),
            limits: output_guard::resolve(settings.get("outputGuard")),
        })
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
            Some(status) => status.registered().to_vec(),
            None => Vec::new(),
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
    /// What `/mcp setup` shows: detected import sources, conflicts and notes.
    pub fn setup_view(&self) -> Result<setup::SetupView, String> {
        setup::view(
            dirs::home_dir().as_deref(),
            &self.0.agent_dir,
            &self.0.cwd,
            self.0.trusted,
        )
    }
    /// Add or remove `kind` in the global `imports` list, then reload every server.
    pub async fn set_import(&self, kind: &str, enabled: bool) -> Result<(), String> {
        if !import::KINDS.contains(&kind) {
            return Err(format!("Unknown import kind: {kind}"));
        }
        let _guard = self.0.operation.lock().await;
        let path = self.config_path(false)?;
        if !enabled {
            for (other, config) in read_configs(&self.0.agent_dir, &self.0.cwd, self.0.trusted)? {
                if other != path && config.imports.iter().any(|k| k == kind) {
                    return Err(format!(
                        "{kind} is imported by {}; remove it there",
                        other.display()
                    ));
                }
            }
        }
        edit_config_root(&path, |root| {
            let imports = root
                .entry("imports")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or("imports must be an array")?;
            imports.retain(|k| k != kind);
            if enabled {
                imports.push(json!(kind));
            }
            Ok(())
        })?;
        self.reconnect_locked(None).await
    }
    /// Write a preset to the global `mcp.json` under its id and start it.
    pub async fn add_preset(&self, id: &str) -> Result<(), String> {
        let preset = setup::PRESETS
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("Unknown MCP preset: {id}"))?;
        self.add_entry(preset.id, preset.entry_json()).await
    }
    /// Begin an OAuth sign-in and return the URL to open. A loopback callback
    /// finishes it in the background; `auth_complete` accepts a pasted one.
    /// Either way the server is reconnected once tokens are stored.
    pub async fn auth_start(&self, server: &str) -> Result<String, String> {
        let (config, _) = self
            .configured()?
            .remove(server)
            .ok_or_else(|| format!("Unknown MCP server: {server}"))?;
        let url = match (&config.url, config.auth) {
            (Some(url), Some(Auth::Oauth)) => http::interpolate(url)?,
            _ => return Err(format!("{server} is not configured with auth \"oauth\"")),
        };
        let mut pending = oauth::start(&url, &config.oauth).await?;
        let authorize_url = pending.authorize_url.clone();
        let listener = pending.listener.take();
        let state = pending.state().to_owned();
        let cancel = CancellationToken::new();
        let slot = Arc::new(Mutex::new(Some(pending)));
        // A new sign-in replaces (and stops) an earlier one for the same server.
        if let Some((_, _, old)) = self.0.signins.lock().unwrap().insert(
            server.to_owned(),
            (state.clone(), slot.clone(), cancel.clone()),
        ) {
            old.cancel();
        }
        if let Some(listener) = listener {
            let manager = Arc::downgrade(&self.0);
            let server = server.to_owned();
            tokio::spawn(async move {
                let code = oauth::Pending::wait(listener, state, cancel).await;
                let Some(manager) = manager.upgrade().map(McpManager) else {
                    return;
                };
                let outcome = match code {
                    Ok(code) => manager.finish_signin(&server, &slot, &code).await,
                    // Replaced or completed by paste: nothing to report.
                    Err(_) if slot.lock().unwrap().is_none() => return,
                    Err(error) => Err(error),
                };
                manager.report_signin(&server, outcome);
            });
        }
        Ok(authorize_url)
    }
    /// Finish a sign-in with a pasted callback URL, query string, or code.
    pub async fn auth_complete(&self, server: &str, input: &str) -> Result<(), String> {
        let (slot, cancel) = {
            let signins = self.0.signins.lock().unwrap();
            let (_, slot, cancel) = signins.get(server).ok_or_else(|| {
                format!("No sign-in in progress for {server}; run /mcp-auth {server}")
            })?;
            (slot.clone(), cancel.clone())
        };
        let code = slot
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("This sign-in already finished")?
            .parse(input)?;
        let outcome = self.finish_signin(server, &slot, &code).await;
        cancel.cancel();
        outcome
    }
    async fn finish_signin(
        &self,
        server: &str,
        slot: &Mutex<Option<oauth::Pending>>,
        code: &str,
    ) -> Result<(), String> {
        let pending = slot
            .lock()
            .unwrap()
            .take()
            .ok_or("This sign-in already finished")?;
        self.0.signins.lock().unwrap().remove(server);
        pending.finish(&self.0.agent_dir, server, code).await?;
        self.reconnect(Some(server)).await
    }
    fn report_signin(&self, server: &str, outcome: Result<(), String>) {
        let notice = match outcome {
            Ok(()) => format!("Signed in to MCP server {server}."),
            Err(error) => format!("MCP sign-in to {server} failed: {error}"),
        };
        let _ = self.0.elicitation_notices.send(notice);
    }
    /// Forget stored OAuth tokens for `server` and reconnect it (signed out).
    pub async fn logout(&self, server: &str) -> Result<bool, String> {
        if let Some((_, _, cancel)) = self.0.signins.lock().unwrap().remove(server) {
            cancel.cancel();
        }
        let removed = oauth::remove(&self.0.agent_dir, server)?;
        self.reconnect(Some(server)).await?;
        Ok(removed)
    }
    /// Reload every server, or restart one. Restarting a named server connects
    /// it even when it is lazy; a full reload leaves lazy servers idle.
    pub async fn reconnect(&self, name: Option<&str>) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        self.reconnect_with(name, name.is_some()).await
    }
    async fn reconnect_locked(&self, name: Option<&str>) -> Result<(), String> {
        self.reconnect_with(name, false).await
    }
    async fn reconnect_with(&self, name: Option<&str>, force: bool) -> Result<(), String> {
        if let Some(name) = name {
            if !self.configured()?.contains_key(name)
                && !self.0.servers.lock().unwrap().contains_key(name)
            {
                return Err(format!("Unknown MCP server: {name}"));
            }
        }
        self.reconnect_servers(name.map(|n| HashSet::from([n.to_owned()])), force)
            .await
    }
    /// Start a lazy server for a call; a no-op when it is already running.
    pub(super) async fn connect_lazy(&self, server: &str) -> Result<(), String> {
        let _guard = self.0.operation.lock().await;
        let idle = matches!(
            self.0
                .servers
                .lock()
                .unwrap()
                .get(server)
                .map(|s| &s.status),
            Some(ServerStatus::Idle { .. })
        );
        if idle {
            self.reconnect_servers(Some(HashSet::from([server.to_owned()])), true)
                .await?;
        }
        match self
            .0
            .servers
            .lock()
            .unwrap()
            .get(server)
            .map(|s| &s.status)
        {
            Some(ServerStatus::Connected { .. }) => Ok(()),
            Some(ServerStatus::Failed(error)) => Err(error.clone()),
            Some(ServerStatus::Disabled) => Err(format!("MCP server {server} is disabled")),
            _ => Err(format!("MCP server {server} is not connected")),
        }
    }
    /// Restart `only` (or every known server) concurrently from current configuration.
    /// Lazy servers with a cached catalog stay idle unless `force` is set.
    async fn reconnect_servers(
        &self,
        only: Option<HashSet<String>>,
        force: bool,
    ) -> Result<(), String> {
        self.start_ticker();
        let mut configured = self.configured()?;
        let configured_names: HashSet<String> = configured.keys().cloned().collect();
        let guard = self.output_guard()?;
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
            let names = old
                .as_ref()
                .map(|s| s.status.registered().to_vec())
                .unwrap_or_default();
            let old_catalog = old.as_ref().and_then(|s| s.catalog.clone());
            if let Some(client) = old.and_then(|s| s.client) {
                client.shut_down("MCP server disconnected");
            }
            // Stop and unregister before anything fallible, so an error cannot leave
            // tools of a server the manager no longer tracks.
            self.unregister(&harness, &names).await?;
            self.sync_instructions().await?;
            self.0
                .prompt_changes
                .send_modify(|version| *version = version.wrapping_add(1));
            let Some((config, source)) = configured.remove(&name) else {
                continue;
            };
            let disabled = config.disabled;
            // Keep the catalog visible while the server is disabled or failing,
            // falling back to the metadata cache before its first connection.
            let catalog = old_catalog
                .or_else(|| {
                    cache::load(&self.0.agent_dir, &name, &config)
                        .map(|cached| cached_catalog(&cached))
                })
                .map(|catalog| {
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
            if config.lifecycle.lazy() && !force {
                if let Some(cached) = cache::load(&self.0.agent_dir, &name, &config) {
                    // Registered from the cache; the first call starts the server.
                    let link = ToolLink::Lazy {
                        manager: Arc::downgrade(&self.0),
                        server: name.clone(),
                    };
                    let tools = McpTool::from_cache(&name, &cached, &config, &link, &guard);
                    let (names, catalog) = self
                        .register(&harness, &config.direct_tools, tools.clone())
                        .await?;
                    if let Some(state) = self.0.servers.lock().unwrap().get_mut(&name) {
                        state.status = ServerStatus::Idle { tools: names };
                        state.catalog = Some(catalog);
                        state.discovered = tools;
                        state.prompts = cached.prompts;
                    }
                    continue;
                }
            }
            let cwd = self.0.cwd.clone();
            let handlers = self.handlers(&name);
            let guard = guard.clone();
            let agent_dir = self.0.agent_dir.clone();
            discoveries.spawn(async move {
                let result = async {
                    let token = match (&config.url, config.auth) {
                        (Some(url), Some(Auth::Oauth)) => {
                            let url = http::interpolate(url)?;
                            Some(
                                oauth::access_token(&agent_dir, &name, &url, &config.oauth)
                                    .await?
                                    .ok_or_else(|| {
                                        format!("needs sign-in: run /mcp-auth {name}")
                                    })?,
                            )
                        }
                        _ => None,
                    };
                    let client = Client::start(&name, &config, &cwd, handlers, token.as_deref())?;
                    // One budget covers discovery and prompts (see `crate::mcp::start`).
                    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
                    let tools =
                        tokio::time::timeout_at(deadline, client.discover(&name, &config, &guard))
                            .await
                            .map_err(|_| "MCP discovery timed out".to_string())??;
                    // Prompts are optional; their failure must not cost the server its tools.
                    let prompts = tokio::time::timeout_at(deadline, client.list_prompts())
                        .await
                        .map_err(|_| "MCP prompts/list timed out".to_string())
                        .and_then(|prompts| prompts)
                        .unwrap_or_else(|error| {
                            tracing::warn!(server = %name, %error, "MCP prompts unavailable");
                            Vec::new()
                        });
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
                Err(error) => {
                    state.status = ServerStatus::Failed(error);
                    continue;
                }
            }
            drop(servers);
            self.save_cache(&name);
        }
        if only.is_none() {
            cache::retain(&self.0.agent_dir, &configured_names);
        }
        self.sync_proxy(&harness).await?;
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
        // A failed re-list drops the announced catalog rather than keep entries the
        // server may have removed; `/mcp reconnect` or the next change restores it.
        if catalog == "prompts" {
            let listed = tokio::time::timeout(START_TIMEOUT, client.list_prompts())
                .await
                .map_err(|_| "MCP prompts/list timed out".to_string())
                .and_then(|listed| listed);
            if let Some(state) = self.0.servers.lock().unwrap().get_mut(server) {
                state.prompts = listed.as_ref().cloned().unwrap_or_default();
            }
            self.0
                .prompt_changes
                .send_modify(|version| *version = version.wrapping_add(1));
            if listed.is_ok() {
                self.save_cache(server);
            }
            return listed.map(drop);
        }
        let guard = self.output_guard()?;
        let resources = catalog == "resources";
        let listed = tokio::time::timeout(START_TIMEOUT, async {
            if resources {
                client.list_resources(server, &config, &guard).await
            } else {
                client.list_tools(server, &config, &guard).await
            }
        })
        .await
        .map_err(|_| format!("MCP {catalog}/list timed out"))
        .and_then(|listed| listed);
        discovered.retain(|tool| tool.resource_uri.is_some() != resources);
        if let Ok(updated) = &listed {
            discovered.extend(updated.iter().cloned());
        }
        discovered.sort_by_key(|tool| tool.resource_uri.is_some());
        let harness = self.0.harness.upgrade().ok_or("Session ended")?;
        self.unregister(&harness, &old).await?;
        let (tools, catalog_view) = self
            .register(&harness, &config.direct_tools, discovered.clone())
            .await?;
        if let Some(state) = self.0.servers.lock().unwrap().get_mut(server) {
            state.discovered = discovered;
            state.status = ServerStatus::Connected { tools };
            state.catalog = Some(catalog_view);
        }
        if listed.is_ok() {
            self.save_cache(server);
        }
        listed.map(drop)
    }
    /// Run `check_lifecycle` periodically for as long as the manager lives.
    fn start_ticker(&self) {
        if self.0.ticking.swap(true, Ordering::SeqCst) {
            return;
        }
        let manager = Arc::downgrade(&self.0);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(LIFECYCLE_TICK).await;
                let Some(inner) = manager.upgrade() else {
                    break;
                };
                McpManager(inner).check_lifecycle().await;
            }
        });
    }
    /// Stop `lazy` servers unused for their idle timeout, and restart stopped
    /// `keep-alive` servers with backoff (30 s doubling to 5 min).
    pub(super) async fn check_lifecycle(&self) {
        let default_idle = load_settings(&self.0.agent_dir, &self.0.cwd, self.0.trusted)
            .ok()
            .and_then(|settings| settings.get("idleTimeout").and_then(Value::as_f64));
        let now = std::time::Instant::now();
        let (idle, revive): (Vec<_>, Vec<_>) = {
            let servers = self.0.servers.lock().unwrap();
            let idle = servers
                .iter()
                .filter(|(_, state)| state.config.lifecycle == Lifecycle::Lazy)
                .filter_map(|(name, state)| {
                    let client = state.client.as_ref()?;
                    let minutes = state
                        .config
                        .idle_timeout
                        .or(default_idle)
                        .filter(|m| m.is_finite() && *m > 0.0)
                        .unwrap_or(DEFAULT_IDLE_MINUTES);
                    (matches!(state.status, ServerStatus::Connected { .. })
                        && !client.busy()
                        && client.idle_for() >= Duration::from_secs_f64(minutes * 60.0))
                    .then(|| name.clone())
                })
                .collect();
            let revive = servers
                .iter()
                .filter(|(_, state)| state.config.lifecycle.keep_alive())
                .filter(|(_, state)| match (&state.status, &state.client) {
                    (ServerStatus::Connected { .. }, Some(client)) => {
                        client.closed.load(Ordering::SeqCst)
                    }
                    (ServerStatus::Failed(_), _) => true,
                    _ => false,
                })
                .map(|(name, _)| name.clone())
                .filter(|name| {
                    self.0
                        .retries
                        .lock()
                        .unwrap()
                        .get(name)
                        .is_none_or(|(due, _)| *due <= now)
                })
                .collect();
            (idle, revive)
        };
        for name in idle {
            let _guard = self.0.operation.lock().await;
            if let Err(error) = self
                .reconnect_servers(Some(HashSet::from([name.clone()])), false)
                .await
            {
                tracing::warn!(server = %name, %error, "MCP idle stop failed");
            }
        }
        for name in revive {
            let _guard = self.0.operation.lock().await;
            let result = self
                .reconnect_servers(Some(HashSet::from([name.clone()])), true)
                .await;
            let connected = matches!(
                self.0.servers.lock().unwrap().get(&name).map(|s| &s.status),
                Some(ServerStatus::Connected { .. })
            );
            let mut retries = self.0.retries.lock().unwrap();
            if connected {
                retries.remove(&name);
            } else {
                let backoff = retries
                    .get(&name)
                    .map_or(RETRY_FIRST, |(_, backoff)| (*backoff * 2).min(RETRY_MAX));
                retries.insert(name.clone(), (std::time::Instant::now() + backoff, backoff));
                if let Err(error) = result {
                    tracing::warn!(server = %name, %error, "MCP keep-alive restart failed");
                }
            }
        }
    }
    /// Register the `mcp` proxy tool while any server is configured.
    async fn sync_proxy(&self, harness: &AgentHarness) -> Result<(), String> {
        let configured = !self.0.servers.lock().unwrap().is_empty();
        let script_mode = load_settings(&self.0.agent_dir, &self.0.cwd, self.0.trusted)
            .map_or(true, |settings| {
                settings.get("scriptMode") != Some(&Value::Bool(false))
            });
        let wanted =
            |name: &str| configured && crate::session::tool_name_allowed(name, &self.0.args);
        let gateways: [(&str, bool); 2] = [
            (proxy::NAME, wanted(proxy::NAME)),
            (script::NAME, script_mode && wanted(script::NAME)),
        ];
        let mut tools = harness.get_tools().await.map_err(|e| e.to_string())?;
        let mut active = harness
            .lane("main")
            .get_active_tools()
            .await
            .map_err(|e| e.to_string())?;
        let mut changed = false;
        for (name, wanted) in gateways {
            let present = tools.iter().any(|t| t.tool.schema().name == name);
            if wanted == present {
                continue;
            }
            changed = true;
            if wanted {
                let manager = Arc::downgrade(&self.0);
                tools.push(if name == proxy::NAME {
                    HarnessTool::new(Arc::new(proxy::ProxyTool::new(manager)))
                } else {
                    HarnessTool::new(Arc::new(script::ScriptTool::new(manager)))
                });
                active.push(name.into());
            } else {
                tools.retain(|t| t.tool.schema().name != name);
                active.retain(|n| n != name);
            }
        }
        if !changed {
            return Ok(());
        }
        harness
            .set_tools(tools, Some(active))
            .await
            .map_err(|e| e.to_string())
    }
    /// Every server as the proxy sees it: live catalogs, else the metadata cache.
    pub(super) fn proxy_view(&self) -> Vec<proxy::Server> {
        let servers = self.0.servers.lock().unwrap();
        servers
            .iter()
            .map(|(name, state)| {
                let live = state.client.is_some()
                    && matches!(state.status, ServerStatus::Connected { .. });
                let (entries, instructions, cached) = if live {
                    (
                        state
                            .discovered
                            .iter()
                            .filter(|tool| !tool.ui.as_ref().is_some_and(|ui| ui.app_only))
                            .map(proxy::Entry::from_tool)
                            .collect(),
                        state
                            .client
                            .as_ref()
                            .and_then(|c| c.instructions().map(str::to_owned)),
                        false,
                    )
                } else if matches!(state.status, ServerStatus::Idle { .. }) {
                    // Idle tools are registered from the cache and start the server.
                    (
                        state
                            .discovered
                            .iter()
                            .filter(|tool| !tool.ui.as_ref().is_some_and(|ui| ui.app_only))
                            .map(proxy::Entry::from_tool)
                            .collect(),
                        cache::load(&self.0.agent_dir, name, &state.config)
                            .and_then(|cached| cached.instructions),
                        true,
                    )
                } else {
                    match cache::load(&self.0.agent_dir, name, &state.config) {
                        Some(cached) => (
                            proxy::Entry::from_cache(name, &cached, &state.config),
                            cached.instructions,
                            true,
                        ),
                        None => (Vec::new(), None, false),
                    }
                };
                proxy::Server {
                    name: name.clone(),
                    status: state.status.clone(),
                    cached,
                    entries,
                    instructions,
                    config: state.config.clone(),
                }
            })
            .collect()
    }
    /// The live tool behind a proxied name, with the mailbox for its approval.
    pub(super) fn live_tool(
        &self,
        server: &str,
        name: &str,
    ) -> Option<(McpTool, bool, Option<UiDialogMailbox>)> {
        let servers = self.0.servers.lock().unwrap();
        let state = servers.get(server)?;
        let tool = state
            .discovered
            .iter()
            .find(|tool| tool.schema.name == name)?
            .clone();
        let approval = tool.approval;
        Some((tool, approval, self.0.approval_ui.clone()))
    }
    /// Record a connected server's catalog for offline listing.
    fn save_cache(&self, server: &str) {
        let entry = {
            let servers = self.0.servers.lock().unwrap();
            let Some(state) = servers.get(server) else {
                return;
            };
            let Some(client) = &state.client else {
                return;
            };
            let (resources, tools): (Vec<_>, Vec<_>) = state
                .discovered
                .iter()
                .partition(|tool| tool.resource_uri.is_some());
            cache::CachedServer {
                fingerprint: cache::fingerprint(&state.config),
                tools: tools.into_iter().map(|tool| tool.raw.clone()).collect(),
                resources: resources.into_iter().map(|tool| tool.raw.clone()).collect(),
                prompts: state.prompts.clone(),
                instructions: client.instructions().map(str::to_owned),
            }
        };
        cache::save(&self.0.agent_dir, server, entry);
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
            // Not direct: reachable through the `mcp` proxy only. App-only UI
            // tools are never offered to the model at all.
            if !direct || tool.ui.as_ref().is_some_and(|ui| ui.app_only) {
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
            let tool = if tool.ui.is_some() {
                HarnessTool::new(Arc::new(ui::UiTool {
                    tool,
                    manager: Arc::downgrade(&self.0),
                }))
            } else {
                HarnessTool::new(Arc::new(tool))
            };
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
            self.reconnect_servers(Some(written), false).await?;
        }
        result
    }
    pub async fn add(&self, name: &str, command: &str, args: &[String]) -> Result<(), String> {
        if command.is_empty() {
            return Err("Missing MCP command".into());
        }
        self.add_entry(name, json!({"command":command,"args":args}))
            .await
    }
    /// Add a new server entry to the global `mcp.json`; never overwrites one.
    async fn add_entry(&self, name: &str, entry: Value) -> Result<(), String> {
        validate_name(name)?;
        let _guard = self.0.operation.lock().await;
        edit_config(&self.config_path(false)?, |servers| {
            if servers.contains_key(name) {
                return Err(format!("MCP server already exists: {name}"));
            }
            servers.insert(name.into(), entry);
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
    edit_config_root(path, |object| {
        edit(
            object
                .entry("mcpServers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or("mcpServers must be an object")?,
        )
    })
}
/// Edit the whole `mcp.json` object, for top-level keys such as `imports`.
fn edit_config_root(
    path: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), String>,
) -> Result<(), String> {
    let mut value: Value = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"mcpServers":{}}),
        Err(e) => return Err(e.to_string()),
    };
    edit(
        value
            .as_object_mut()
            .ok_or("MCP config must be an object")?,
    )?;
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

/// Panel entries for a cached `tools/list`; `direct` is set by the caller.
fn cached_catalog(cached: &cache::CachedServer) -> Vec<ToolInfo> {
    cached
        .tools
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let description = item.get("description").and_then(Value::as_str);
            Some(ToolInfo {
                name: name.into(),
                description: description.unwrap_or("MCP tool").into(),
                tokens: tool_tokens(name, description, item.get("inputSchema")?),
                direct: true,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
