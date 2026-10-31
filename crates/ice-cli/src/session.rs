//! Harness construction + session-storage wiring. Mirrors the Rust-side
//! equivalent of the TS `packages/coding-agent/src/core/sdk.ts`
//! (`createAgentSession`) — build the env, tools, durable session storage, and
//! `AgentHarnessOptions`, then `AgentHarness::create`.
//!
//! v1 scope cuts vs the TS SDK (tracked in `docs/m6-cli-open-questions.md`):
//! - **Skill / prompt-template / context-file discovery IS wired**
//!   (`--no-skills`/`-ns`, `--no-prompt-templates`/`-np`, `--no-context-files`/
//!   `-nc` each suppress one channel; project `.icers/<sub>` + legacy
//!   `.icers/<sub>` + global
//!   `agent_dir()<sub>` discovery with project-wins dedupe via
//!   [`crate::resource_dirs`]; SYSTEM.md/APPEND_SYSTEM.md project-wins
//!   precedence). **Extension `resources_discover` (B5b) feeds the SAME loaders:
//!   a plugin's discovered skill/prompt paths merge with the static dirs and
//!   re-run through `load_skills`/`load_prompt_templates` (individual `.md` files
//!   load too — `load_skills` accepts both dirs and files). Package themes are
//!   parsed by the TUI when selected via settings or `--theme`.**
//!   Project resources load by default without a prompt. `--no-approve` or a
//!   stored negative `trust.json` decision disables them explicitly.
//! - **No `ModelRuntime`/multi-provider registry.** The resolver supports the
//!   built-in Anthropic/OpenAI-compatible providers and `models.json`, but
//!   runtime catalog mutation remains outside this layer.
//! - **Built-in tools**: `read`, `bash`, `edit`, `write`, and the read-only
//!   `docs` lookup tool. The former ice-only `grep`, `find`, `ls`, and
//!   `powershell` tools remain library modules but are not registered by the
//!   CLI.
//! - **Session restore (`-c`/`-r`/`--session`)** is *partially* supported: a
//!   fresh session is always created. The harness's `create` rejects sessions
//!   that already have records unless `allow_existing_session` is enabled.
//!   The interactive `-c`/`-r`/`--session` paths enable that mode and replay
//!   the existing branch before appending new messages. See [`SessionSelection`].

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use ice_ai::Provider;
use ice_harness::agent_harness::AgentHarness;
use ice_harness::context_files::{format_project_context, load_project_context_files};
use ice_harness::session::memory::{InMemorySessionStorage, SystemClock};
use ice_harness::session::session::DefaultIdGenerator;
use ice_harness::session::types::SessionMetadata;
use ice_harness::session::Session;
use ice_harness::system_prompt::compose_system_prompt;
use ice_harness::types::{
    AgentHarnessOptions, AgentHarnessResources, AgentHarnessStreamOptions, DrivingMode,
    HarnessTool, HarnessToolExecution, ToolReplay,
};
#[cfg(windows)]
use ice_tools::create_powershell_tool;
use ice_tools::{
    create_bash_tool, create_edit_tool, create_read_tool, create_write_tool, ExecutionToolContext,
    MutationQueueRegistry, OsExecutionEnv,
};

use crate::args::Args;
use crate::docs_tool::create_docs_tool;
use crate::provider::ResolvedModel;
use crate::resource_dirs::{
    discover_append_system_prompt_file, discover_system_prompt_file, extension_dirs,
    global_extension_dirs, global_prompt_template_dirs, global_skill_dirs,
    load_prompt_templates_with_precedence, load_skills_with_precedence, project_extension_dirs,
    project_prompt_template_dirs, project_skill_dirs, prompt_template_dirs, skill_dirs,
};
use ice_extensions::{
    emit_resources_discover, ExtensionEmitter, ExtensionSession, NullDiagnostics,
    PluginDiagnostics, PluginToolAdapter, TeeEmitter,
};

/// The coding tools registered by the CLI by default.
pub const BUILTIN_TOOL_NAMES: &[&str] = &["read", "bash", "edit", "write", "docs"];

/// Await extension observers before exposing the same lifecycle event publicly.
/// Streaming tool updates still use the emitter's non-blocking `try_emit` path.
/// Extension ABI v1 observers cannot replace the message payload; this ordering
/// does not pretend to implement an extension transformation channel.
fn compose_agent_emitter(
    public: Arc<dyn ice_agent::AgentEmitter>,
    extension: Option<Arc<dyn ice_agent::AgentEmitter>>,
) -> Arc<dyn ice_agent::AgentEmitter> {
    match extension {
        Some(extension) => Arc::new(TeeEmitter::new(vec![extension, public])),
        None => public,
    }
}

/// The default coding system prompt. A condensed port of the TS
/// `packages/coding-agent/src/core/system-prompt.ts` base prompt.
pub fn default_system_prompt(cwd: &str, prior_reasoning_replayed: bool) -> String {
    // The working-state rule must match what actually comes back on the next
    // request. With `compat.requiresThinkingAsText` (or the native thinking
    // channels of anthropic/responses) the model *does* see its own reasoning
    // again, and claiming otherwise makes it re-derive and restate the plan
    // every turn — the re-plan symptom in `docs/llm-repetition-forensics.md`.
    let working_state_rule = if prior_reasoning_replayed {
        "- Keep a SHORT working plan visible (a few bullets, or the todo tool) and update it as you go.
  Your own reasoning is carried back to you on the next turn, so do not re-derive it or restate
  the whole plan: continue from the last unfinished step, and when a step lands, say which one.
  Re-listing the same plan without acting on it is a bug, not progress."
    } else {
        "- Keep your working state in your VISIBLE replies, not only in reasoning. Reasoning is not
  carried into your next turn: only the text you write and the tool output you produce come
  back. Before each batch of tool calls, write one short line naming the task you are on and
  what remains. When you finish a step, say which one is done. If you keep the plan only in
  your head you will re-derive it from scratch every turn."
    };

    format!(
        "You are an expert coding assistant operating inside icers, a coding agent harness. \
You help users by reading files, executing commands, editing code, and writing new files.

Available tools:
- read  — Read file contents
- bash  — Execute Bash commands using Bash/POSIX syntax, including on Windows
- edit  — Find/replace edits to existing files
- write — Create or overwrite files
- docs  — Look up icers usage, extension, package, and compatibility documentation

Guidelines:
- Reply in the SAME language the user is writing in. Detect the language of the user's
  latest message (and, when it is too short to be sure, the working language of the
  conversation so far) and use it for your prose replies. Mirror it unless the user
  explicitly asks for a different language; only fall back to English when the
  user's language cannot be determined.
- Be concise in your responses
- Show file paths clearly when working with files
- Prefer the smallest change that solves the problem
- The bash tool runs Bash, even on Windows. Use ls, find, rg, and POSIX pipelines;
  do not pass PowerShell cmdlets such as Get-ChildItem, Select-Object, or Select-String.
  If a powershell tool is available, use it for PowerShell commands.
{working_state_rule}
- Track multi-step work with the todo tool instead of re-listing the plan in prose: add the
  steps once, then mark them done as you go. Re-stating the same plan without acting on it is
  a bug, not progress.
- Read only what you need. If a file or search result was already shown earlier in this
  conversation, use it instead of fetching it again.
- When unsure about icers commands, extensions, Pi package compatibility, or .icers configuration, consult the project documentation before guessing

Current working directory: {cwd}"
    )
}

/// How the user asked to select a session. v1 honors `NoSession` (ephemeral
/// `InMemorySessionStorage`), `New` (a fresh JSONL file), and — new this pass —
/// `Latest` / `ById`, which **restore** an existing JSONL session on launch
/// (`--continue`/`-c`, `--resume`/`-r`, `--session <id|path>`). The restored
/// transcript renders into the TUI on startup and the run continues appending
/// to the same file.
#[derive(Debug, Clone)]
pub enum SessionSelection {
    /// `--no-session`: ephemeral, in-memory, nothing persisted.
    Ephemeral,
    /// Fresh durable JSONL session under `--session-dir` (or the default dir).
    New { dir: PathBuf, name: Option<String> },
    /// `-c` / `-r`: restore the most recent session in the default dir.
    Latest { dir: PathBuf },
    /// `--session <id|path>`: restore the session whose id matches, or whose
    /// file name contains the id.
    ById { id: String, dir: PathBuf },
    /// `--session-id <id>`: use the EXACT session id, creating it if missing.
    ByExactId { id: String, dir: PathBuf },
    /// `--fork <path|id>`: fork the given session into a new one and start in
    /// the fork.
    Fork { source: String, dir: PathBuf },
}

/// Decide the session selection from parsed args + the resolved cwd.
pub fn select_session(args: &Args, cwd: &Path) -> SessionSelection {
    if args.no_session {
        return SessionSelection::Ephemeral;
    }
    // `--session-dir` wins; then the saved `sessionDir` (upstream's key, same
    // format as the flag); then the built-in default. Keep this directory on
    // restore selections too, not only on fresh sessions.
    let dir = configured_session_dir(args).unwrap_or_else(|| default_session_dir(cwd));
    if args.continue_session || args.resume {
        return SessionSelection::Latest { dir };
    }
    if let Some(s) = &args.fork {
        return SessionSelection::Fork {
            source: s.clone(),
            dir,
        };
    }
    if let Some(s) = &args.session_id {
        return SessionSelection::ByExactId { id: s.clone(), dir };
    }
    if let Some(s) = &args.session {
        return SessionSelection::ById { id: s.clone(), dir };
    }
    SessionSelection::New {
        dir,
        name: args.name.clone(),
    }
}

/// `--session-dir`, else the saved `sessionDir` setting (upstream's key).
fn configured_session_dir(args: &Args) -> Option<PathBuf> {
    args.session_dir.clone().or_else(|| {
        crate::settings::load_settings()
            .ok()
            .and_then(|settings| settings.session_dir)
    })
}

static SESSION_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Record the user-configured session directory for the whole process so every
/// listing/open path (`-r` picker, `/session`, hot-switch, fork) agrees with
/// the startup selection. Called once from the app entry point.
pub fn configure_session_dir(args: &Args) {
    if let Ok(mut dir) = SESSION_DIR.lock() {
        *dir = configured_session_dir(args);
    }
}

/// The session directory in effect: the configured one, else the default.
pub fn session_dir(cwd: &Path) -> PathBuf {
    SESSION_DIR
        .lock()
        .ok()
        .and_then(|dir| dir.clone())
        .unwrap_or_else(|| default_session_dir(cwd))
}

/// The default session directory is `<cwd>/.icers/sessions`.
pub fn default_session_dir(cwd: &Path) -> PathBuf {
    cwd.join(".icers").join("sessions")
}

/// Extensions are loaded exactly once before model resolution. Their action
/// host remains unbound until the harness and authenticated catalog exist.
pub struct PreparedExtensions {
    pub extension_session: ExtensionSession,
    runtime: tokio::runtime::Handle,
    tool_context: ice_extensions::ToolCallContext,
    action_host: Arc<crate::extensions_actions::HarnessActionHost>,
    harness_cell: Arc<std::sync::OnceLock<ice_harness::agent_harness::WeakAgentHarness>>,
    reload_mailbox: ice_extensions::ReloadMailbox,
    ui_dialog_mailbox: ice_extensions::UiDialogMailbox,
    ext_status_mailbox: ice_extensions::ExtensionStatusMailbox,
    editor_text_mailbox: ice_extensions::EditorTextMailbox,
    action_bridge: Arc<ice_extensions::ActionBridge>,
}

pub fn prepare_extensions(
    args: &Args,
    cwd: &Path,
    project_trusted: bool,
) -> Result<PreparedExtensions, BuildError> {
    let cwd_str = cwd.to_string_lossy().to_string();
    let tool_context = ice_extensions::ToolCallContext::new(cwd_str.clone());
    if !project_trusted && args.verbose {
        eprintln!(
            "warning: current-project settings, resources, and discovered extensions are explicitly disabled (use --approve or /trust yes to re-enable)"
        );
    }
    // Pi packages are explicitly opt-in because discovery can start Node and
    // execute package code. An explicit project opt-out limits discovery to
    // global settings.

    // ---- B5a: build the action bridge BEFORE extension load ----
    // Extensions load before `AgentHarness::create` (extensions provide tools the
    // harness is built with), but a plugin stores the `ActionBridge`'s raw
    // `user_data` pointer during `register` and it must remain valid + the host
    // must be ready for the whole session. So:
    //  1. Capture the current tokio `Handle` (the async main-thread runtime) —
    //     the bridge spawns dispatch from any thread via `Handle::spawn`.
    //  2. Build an *empty* `HarnessActionHost` (its harness cell is unset; no
    //     plugin can call a runtime action before the harness runs).
    //  3. Wrap it as `Arc<dyn RuntimeActionHost>` + `ActionBridge`, thread
    //     `Some(bridge)` into `load_extensions` so every plugin's `user_data`
    //     points at this bridge.
    //  4. After `AgentHarness::create` succeeds, call `set_harness(&cell, …)` to
    //     fill the host cell the bridge recovers on the first action call.
    let runtime = tokio::runtime::Handle::try_current().map_err(|e| {
        BuildError::HarnessCreate(format!("no tokio runtime for action bridge: {e}"))
    })?;
    let catalog = Vec::new();
    let (action_host, harness_cell) = crate::extensions_actions::HarnessActionHost::new_empty(
        catalog.clone(),
        cwd.to_path_buf(),
        runtime.clone(),
        args.unknown_flags.clone(),
        tool_context.clone(),
    );
    let action_host = Arc::new(action_host);
    let host_arc: Arc<dyn ice_extensions::RuntimeActionHost> = action_host.clone();
    // `runtime` is reused below (B5c: `PluggableProvider` needs a captured
    // `Handle` to `spawn_blocking` the sync `ProviderRequestFn`), so clone here.
    //
    // B5d: build the initial bridge WITH a reload callback backed by a session-
    // long `ReloadMailbox` (cloned into `ReloadContext` + handed to the TUI). A
    // plugin's `runtime_action(Reload)` then signals the TUI's main loop instead
    // of hitting the "not configured" fallback. The same mailbox is reused on
    // `/reload` (the fresh bridge carries `ctx.mailbox`), so the bridge always
    // points at the one TUI-installed sender across reloads.
    let reload_mailbox = ice_extensions::ReloadMailbox::new();
    // Session-scoped UI-dialog mailbox (runtime action 17 / `ask_user`). The
    // initial bridge carries it and the TUI attaches itself once it starts
    // polling; a headless run leaves it detached so `ask_user` fails loudly
    // instead of fabricating a user answer. The SAME mailbox is reused across
    // `/reload` so in-flight prompts + TUI attachment survive a plugin swap.
    let ui_dialog_mailbox = ice_extensions::UiDialogMailbox::new();
    // Session-scoped extension status registry (runtime action 18 / `SetStatus`).
    // The bridge lets plugins write; the TUI polls the revision and repaints its
    // footer. Reused across `/reload` so a plugin's status survives the swap.
    let ext_status_mailbox = ice_extensions::ExtensionStatusMailbox::new();
    // Session-scoped editor-text queue (runtime action 19 / `SetEditorText`).
    // The bridge lets a plugin drop a draft into the prompt editor; the TUI
    // drains it each tick. Reused across `/reload` so a pending transcription
    // survives a plugin swap.
    let editor_text_mailbox = ice_extensions::EditorTextMailbox::new();
    let action_bridge = ice_extensions::ActionBridge::with_reload_and_ui(
        runtime.clone(),
        host_arc,
        ice_extensions::reload_callback_from_mailbox(reload_mailbox.clone()),
        ui_dialog_mailbox.clone(),
        ext_status_mailbox.clone(),
        editor_text_mailbox.clone(),
    );

    let extension_session = if args.no_extensions {
        ExtensionSession::none()
    } else {
        load_extensions(args, cwd, project_trusted, Some(Arc::clone(&action_bridge)))
    };
    if args.verbose {
        if let Some(s) = extension_session.summary() {
            eprintln!("extensions: {s}");
        }
        report_deferred_renderers(&extension_session);
    }
    Ok(PreparedExtensions {
        runtime,
        tool_context,
        action_host,
        harness_cell,
        reload_mailbox,
        ui_dialog_mailbox,
        ext_status_mailbox,
        editor_text_mailbox,
        action_bridge,
        extension_session,
    })
}

/// Build the `AgentHarness` from the resolved model + parsed args + cwd.
///
/// This is the v1 equivalent of TS `createAgentSession`. It:
/// 1. Builds the `OsExecutionEnv` rooted at `cwd`.
/// 2. Constructs the built-in tools (optionally filtered by `--tools`/
///    `--exclude-tools`/`--no-tools`/`--no-builtin-tools`).
/// 3. Resolves the session storage (ephemeral vs fresh JSONL vs restore-error).
/// 4. Assembles `AgentHarnessOptions` and calls `AgentHarness::create`.
///
/// Returns the harness plus a `broadcast::Receiver<AgentEvent>` carrying the
/// live `AgentEvent` stream from every run (backed by a `BroadcastEmitter`
/// installed on the harness). Interactive mode drains this to render streaming
/// responses; the non-interactive modes simply drop it.
/// Returns the harness, the live `AgentEvent` broadcast receiver, and a
/// [`ReloadContext`] the interactive TUI holds to drive `/reload` (and a
/// plugin's `runtime_action(Reload)` via the mailbox). Non-interactive modes
/// drop the context (no `/reload` surface in print/json mode).
pub async fn build(
    resolved: &ResolvedModel,
    args: &Args,
    cwd: &Path,
    project_trusted: bool,
) -> Result<
    (
        AgentHarness,
        tokio::sync::broadcast::Receiver<ice_agent::AgentEvent>,
        ReloadContext,
    ),
    BuildError,
> {
    let prepared = prepare_extensions(args, cwd, project_trusted)?;
    build_prepared(resolved, args, cwd, project_trusted, prepared).await
}

pub async fn build_prepared(
    resolved: &ResolvedModel,
    args: &Args,
    cwd: &Path,
    project_trusted: bool,
    prepared: PreparedExtensions,
) -> Result<
    (
        AgentHarness,
        tokio::sync::broadcast::Receiver<ice_agent::AgentEvent>,
        ReloadContext,
    ),
    BuildError,
> {
    let cwd_str = cwd.to_string_lossy().to_string();
    // Plugin tools get the session's identity injected into every call's
    // arguments (see `ice_extensions::ToolCallContext`). It is created here,
    // before the tools are built, and completed with the session id below once
    // the session exists — the tool set has to exist before the harness does.
    let PreparedExtensions {
        runtime,
        tool_context,
        action_host,
        harness_cell,
        reload_mailbox,
        ui_dialog_mailbox,
        ext_status_mailbox,
        editor_text_mailbox,
        action_bridge,
        extension_session,
    } = prepared;
    let catalog = crate::provider::authenticated_catalog(resolved);
    action_host.set_catalog(catalog.clone());

    // ---- Built-in extensions (Cognee memory, Blackhole compaction) ----
    let builtins = match crate::config::agent_dir() {
        Ok(dir) => crate::builtin_extensions::BuiltinExtensions::for_agent_dir(dir),
        Err(_) => crate::builtin_extensions::BuiltinExtensions::disabled(),
    };

    // ---- Execution env + tools ----
    let env = Arc::new(OsExecutionEnv::with_cwd(cwd.to_path_buf()));
    let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = env.clone();
    let _registry = Arc::new(MutationQueueRegistry::new());
    // `env_dyn` is shared between the tool context (moved in) and the resource
    // loaders below (borrowed); clone one branch so both hold a reference.
    let ctx = ExecutionToolContext::new(env_dyn.clone(), Some(mut_env));

    let shells = crate::background_shells::SessionShells::new(
        &crate::settings::load_settings().unwrap_or_default(),
    );
    let mut tools = build_tools(&ctx, args, Some(&shells));

    // ---- Extensions (Part B2) ----
    // Load cdylib plugins from the resolved extension dirs, merge their tools
    // into the built-in set (extension overrides same-named built-in; first-
    // extension-wins across plugins; explicit `--tools`/`--exclude-tools` still
    // apply to the merged set), and keep the loaded `Library` handles alive for
    // the harness lifetime via the returned session guard. `--no-extensions`
    // skips discovery entirely (no dirs scanned, no plugins loaded).
    merge_extension_tools(&mut tools, &extension_session, args, &tool_context);
    let active = filter_active_tool_names(active_tool_names(&tools, args), args);

    // ---- Session storage ----
    let selection = select_session(args, cwd);
    let session = build_session(&selection, &cwd_str).await?;
    // Plugin tools now know which session they serve: `todo` and friends key
    // their per-session state off it. A session without an id (ephemeral) leaves
    // the field unset and a plugin is expected to fall back to project scope.
    if let Ok(metadata) = session.get_metadata().await {
        publish_session_identity(&tool_context, &metadata.id);
    }
    // `--name`/`-n` sets the session display name durably (a `name` fact in the
    // log). It applies to whichever session this launch works in — a fresh one
    // or a restored one — and an empty value clears a previous name. This is the
    // only place the flag is applied: `SessionSelection::New` carries it, but
    // builds that do not create a session (restore/fork) must still honor it.
    if let Some(requested) = args.name.as_deref() {
        let name = requested.trim();
        let name = if name.is_empty() { None } else { Some(name) };
        if let Err(error) = session.set_name(name).await {
            if args.verbose {
                eprintln!("warning: could not set session name: {error}");
            }
        }
    }
    // ---- System prompt base (precedence: --system-prompt > SYSTEM.md > default) ----
    // Mirrors pi `discoverSystemPromptFile` (`resource-loader.ts:1022-1034`):
    // an explicit `--system-prompt` flag wins; otherwise a discovered
    // `<cwd>/.icers/SYSTEM.md` wins, then
    // `<agent_dir>/SYSTEM.md`.
    // (global); otherwise the built-in default. **Project-wins** — the same
    // direction as skills/prompts precedence.
    // Whether this model sees its own prior reasoning again decides how the
    // working-state rule is worded (see `default_system_prompt`).
    let prior_reasoning_replayed = ice_ai::model::prior_reasoning_is_replayed(&resolved.model);
    let mut context_paths = Vec::new();
    let base_prompt = match args.system_prompt.as_deref() {
        Some(explicit) => explicit.to_string(),
        None if project_trusted => match discover_system_prompt_file(cwd) {
            Some(path) => read_prompt_source(&path, &mut context_paths)
                .unwrap_or_else(|| default_system_prompt(&cwd_str, prior_reasoning_replayed)),
            None => default_system_prompt(&cwd_str, prior_reasoning_replayed),
        },
        None => default_system_prompt(&cwd_str, prior_reasoning_replayed),
    };

    // ---- Append-text sources (precedence: --append-system-prompt > APPEND_SYSTEM.md) ----
    // Mirrors pi `appendSystemPrompt` (`resource-loader.ts:525-542`). Explicit
    // `--append-system-prompt` flags are joined together; when none are given, a
    // discovered `APPEND_SYSTEM.md` (project-wins over global) provides the
    // append text. `--append-system-prompt` takes a value that may be a literal
    // string OR a readable file path (mirrors TS `resolvePromptInput`).
    let mut append_texts: Vec<String> = Vec::new();
    for extra in &args.append_system_prompt {
        let text = read_append_target(extra, &mut context_paths).unwrap_or_else(|| extra.clone());
        append_texts.push(text);
    }
    if args.append_system_prompt.is_empty() {
        if let Some(path) = project_trusted
            .then(|| discover_append_system_prompt_file(cwd))
            .flatten()
        {
            if let Some(text) = read_prompt_source(&path, &mut context_paths) {
                append_texts.push(text);
            }
        }
    }
    let append_join = if append_texts.is_empty() {
        None
    } else {
        Some(append_texts.join("\n\n"))
    };

    // ---- Resource discovery (skills + prompt-templates + context-files) ----
    // The env is OS-backed, rooted at cwd. Each `--no-*` flag suppresses its
    // channel independently (pi parity). Skills/prompts load project→global,
    // explicit/plugin paths, then static packages; dedupe first-wins-by-name
    // keeps project and user resources ahead of packages. Context files walk
    // global→ancestor(cwd→root), deepest-last (pi parity).
    //
    // **Trust gate (v1 divergence):** pi gates project config discovery on
    // `isProjectTrusted()` (global resources are unconditional). icers v1 has no
    // trust prompt — project resources are discovered unconditionally (a copied
    // `.icers/` dir drops in and works). Full trust gating is deferred.
    let agent_dir = crate::config::agent_dir().ok();

    // ---- B5b: extension resources_discover ----
    // If any plugin registered a `resources_discover` handler, fan the event out
    // (reason "startup") and collect skill/prompt/theme paths. These plugin-
    // contributed paths merge WITH the static Part-A dirs (project
    // `.icers/skills` +
    // `agent_dir/skills`, etc.) and the loaders re-run over the union — the
    // coherence point: a plugin's discovered skills land through the SAME loaders
    // as static skills. Static dirs load FIRST so project skills keep winning name
    // collisions (a plugin must not shadow a project skill of the same name —
    // mirrors pi `extendResources` running AFTER the default load's first-wins
    // map). `load_skills` now accepts both dirs and individual `.md` files, so a
    // plugin returning bare `SKILL.md` paths loads them (the gap this closes).
    // Theme paths are available to the TUI through the package resource list;
    // skill/prompt loaders are the only resources needed by the harness here.
    // A `--no-*` flag suppresses its channel for BOTH static and discovered paths.
    let discovered = extension_session
        .snapshot_arc()
        .map(|snap| emit_resources_discover(&cwd_str, "startup", &snap))
        .unwrap_or_default();

    let mut live_skill_result = ice_harness::skills::LoadSkillsResult::default();
    let mut skills: Vec<ice_harness::types::Skill> = Vec::new();
    let mut skill_diags: Vec<ice_harness::skills::SkillDiagnostic> = Vec::new();
    let mut resource_diags: Vec<ice_harness::diagnostics::ResourceDiagnostic> = Vec::new();
    if !args.no_skills {
        let mut dirs = if args.dev_local_only || args.project_only {
            project_skill_dirs(cwd)
        } else if project_trusted {
            skill_dirs(cwd)
        } else {
            global_skill_dirs()
        };
        dirs.extend(args.skill.iter().cloned());
        dirs.extend(discovered.skill_paths.iter().map(PathBuf::from));
        dirs.extend(builtins.skill_paths());
        let result = load_skills_with_precedence(&env_dyn, &dirs).await;
        live_skill_result = result.clone();
        skills = result.skills;
        skill_diags = result.diagnostics;
        resource_diags.extend(result.resource_diagnostics);
    }

    let mut prompt_templates: Vec<ice_harness::types::PromptTemplate> = Vec::new();
    let mut prompt_diags: Vec<ice_harness::prompt_templates::PromptTemplateDiagnostic> = Vec::new();
    if !args.no_prompt_templates {
        let mut dirs = if args.dev_local_only || args.project_only {
            project_prompt_template_dirs(cwd)
        } else if project_trusted {
            prompt_template_dirs(cwd)
        } else {
            global_prompt_template_dirs()
        };
        dirs.extend(args.prompt_template.iter().cloned());
        dirs.extend(discovered.prompt_paths.iter().map(PathBuf::from));
        let result = load_prompt_templates_with_precedence(&env_dyn, &dirs).await;
        prompt_templates = result.prompt_templates;
        prompt_diags = result.diagnostics;
        resource_diags.extend(result.resource_diagnostics);
    }

    let context_block = if args.no_context_files || !project_trusted {
        String::new()
    } else {
        // `load_project_context_files` walks the global agentDir first then
        // ancestor-walks cwd→root (deepest last). It needs a real agent_dir; if
        // none is resolvable, pass the cwd dir so only the ancestor-walk runs
        // (the global step returns None anyway).
        let agent_dir_path = agent_dir.clone().unwrap_or_else(|| cwd.to_path_buf());
        let files = load_project_context_files(&env_dyn, cwd, &agent_dir_path).await;
        context_paths.extend(
            files
                .iter()
                .map(|file| file.path.to_string_lossy().into_owned()),
        );
        format_project_context(&files)
    };

    // Surface resource-discovery diagnostics as startup warnings (verbose-only).
    if args.verbose {
        for d in &skill_diags {
            eprintln!(
                "warning: skill {} ({}): {}",
                d.path,
                d.code.as_str(),
                d.message
            );
        }
        for d in &prompt_diags {
            eprintln!(
                "warning: prompt template {} ({}): {}",
                d.path,
                d.code.as_str(),
                d.message
            );
        }
        // Structured collisions: make the winner/loser explicit.
        for d in &resource_diags {
            if let ice_harness::diagnostics::ResourceDiagnostic::Collision(c) = d {
                eprintln!(
                    "warning: {} name \"{}\" collision: keeping {} (shadowed {})",
                    c.resource_type.as_str(),
                    c.name,
                    c.winner_path,
                    c.loser_path
                );
            }
        }
    }

    // ---- Compose the full system prompt ----
    // Order mirrors pi `buildSystemPrompt` (`system-prompt.ts:28-72`):
    // base → append → context → skills. The skills listing is the harness's own
    // section: `AgentHarness::compose_prompt` appends `<available_skills>` (gated
    // on the `read` tool + `disable_model_invocation`, applied inside
    // `format_skills_for_system_prompt`). So we pass None for skills here (the
    // harness adds the listing itself) and fold only base+append+context into
    // the prompt we hand the harness.
    let system_prompt = compose_system_prompt(
        Some(&base_prompt),
        &[], // skills: harness appends the listing itself
        if context_block.is_empty() {
            None
        } else {
            Some(&context_block)
        },
        append_join.as_deref(),
    );

    // ---- Debug: dump the resolved system-prompt sections (verification) ----
    // A verification affordance for Part-A resource discovery: prints the
    // composed sections + resource counts to stderr so a smoke can confirm
    // `<available_skills>` + `<project_context>` + appended text reached the
    // prompt without parsing a provider round-trip. The harness composes the
    // final prompt (base → append → context → skills); here we print the
    // pre-harness sections (the harness adds the skills listing itself, gated
    // on `read` + `disable_model_invocation`).
    if args.debug_system_prompt {
        eprintln!("=== --debug-system-prompt ===");
        let base_src = if args.system_prompt.is_some() {
            "--system-prompt"
        } else if discover_system_prompt_file(cwd).is_some() {
            "SYSTEM.md"
        } else {
            "default"
        };
        eprintln!("[base source: {base_src}]");
        eprintln!("--- base ---\n{base_prompt}");
        if let Some(append) = append_join.as_deref() {
            eprintln!("--- append ---\n{append}");
        } else {
            eprintln!("--- append: (none) ---");
        }
        if context_block.is_empty() {
            eprintln!("--- context: (none) ---");
        } else {
            eprintln!("--- context ---{context_block}");
        }
        let visible_skills = skills
            .iter()
            .filter(|s| s.disable_model_invocation != Some(true))
            .count();
        eprintln!(
            "--- skills: {} loaded ({} model-visible, {} hidden) ---",
            skills.len(),
            visible_skills,
            skills.len() - visible_skills
        );
        for s in &skills {
            let hidden = if s.disable_model_invocation == Some(true) {
                " [hidden]"
            } else {
                ""
            };
            eprintln!("    {}{hidden} — {}", s.name, s.description);
        }
        eprintln!("--- prompt templates: {} ---", prompt_templates.len());
        for t in &prompt_templates {
            eprintln!("    /{}", t.name);
        }
        // B5b: surface plugin-contributed discovery paths so a smoke can confirm
        // the resources_discover round-trip fed the loaders; package themes are
        // selected by the TUI rather than injected into the harness prompt.
        eprintln!(
            "--- discovered via resources_discover: {} skill(s), {} prompt(s), {} theme(s) ---",
            discovered.skill_paths.len(),
            discovered.prompt_paths.len(),
            discovered.theme_paths.len(),
        );
        for p in &discovered.skill_paths {
            eprintln!("    skill: {p}");
        }
        for p in &discovered.prompt_paths {
            eprintln!("    prompt: {p}");
        }
        eprintln!(
            "--- final composed base+append+context (skills listing added by harness) ---\n{system_prompt}"
        );
        eprintln!("=== end --debug-system-prompt ===");
    }

    // ---- Options ----
    // Install a BroadcastEmitter so the caller (the interactive TUI) can drain
    // AgentEvents live as a run unfolds. The corresponding broadcast::Receiver
    // is returned alongside the harness; non-interactive modes simply drop it.
    let (broadcast, event_rx) = ice_agent::events::BroadcastEmitter::new(256);
    let broadcast_emitter: Arc<dyn ice_agent::AgentEmitter> = Arc::new(broadcast);
    // The broadcast half stays live for the whole session (the TUI's drain task
    // holds the receiver); reload re-wraps it in a fresh `TeeEmitter`, so keep
    // a clone for the `ReloadContext` before the tee match consumes the original.
    let broadcast_for_context: Arc<dyn ice_agent::AgentEmitter> = Arc::clone(&broadcast_emitter);

    // ---- Extensions emitter (Part B3a) ----
    // If extensions loaded + registered any `on()` handlers, wrap the
    // broadcast emitter in a `TeeEmitter` so every `AgentEvent` flows to BOTH
    // the TUI (via the broadcast receiver above) AND the plugin handlers (via
    // the `ExtensionEmitter`, which translates each `AgentEvent` →
    // `StablePluginEvent` and fans out to the handlers registered for its tag).
    // With no extensions the tee degrades to the bare broadcast emitter (a
    // one-child passthrough), so the TUI path is unchanged.
    let emitter: Arc<dyn ice_agent::AgentEmitter> = match extension_session.snapshot_arc() {
        Some(snapshot) => {
            let ext = ExtensionEmitter::new(snapshot, extension_session.keepalive());
            compose_agent_emitter(broadcast_emitter, Some(Arc::new(ext)))
        }
        None => broadcast_emitter,
    };
    let emitter = compose_agent_emitter(emitter, builtins.agent_emitter());

    // Saved user defaults. Best-effort: a missing or malformed file must not stop
    // a session from starting, and every accessor falls back to the harness
    // default when its key is absent.
    let saved_settings = crate::settings::load_settings().unwrap_or_default();

    // `httpProxy` applies to icers's own HTTP clients. Setting the environment is how
    // they are configured (`pi-ai/src/http.rs`), and upstream does the same thing
    // with this key — an explicit environment variable still wins, so a shell-level
    // proxy override is not silently replaced by the setting.
    if let Some(proxy) = &saved_settings.http_proxy {
        for key in ["HTTP_PROXY", "HTTPS_PROXY"] {
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, proxy);
            }
        }
    }

    // Share the live session cell with both /reload and the approval hook.
    // The hook is installed only when the current snapshot has a tool-call
    // handler; a later reload rebuilds the hook state together with the
    // extension session.
    let extension_session_cell = Arc::new(Mutex::new(extension_session.clone()));
    // Only install the approval hook when a plugin actually registered a
    // tool-call handler. An extension session may contain flags/resources but
    // no handler; dispatching through that empty session needlessly crosses
    // the plugin bridge and can leave a headless RPC tool call waiting.
    let before_tool_call = extension_session
        .snapshot()
        .filter(|snapshot| {
            !snapshot
                .handlers_for(ice_plugin_sdk::EventTag::ToolCall)
                .is_empty()
        })
        .map(|_| {
            extension_tool_call_hook(
                Arc::clone(&extension_session_cell),
                ui_dialog_mailbox.clone(),
            )
        });

    let mcp_approvals = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let command_hooks = crate::hooks::Hooks::load(
        cwd,
        session
            .get_metadata()
            .await
            .map(|metadata| metadata.id)
            .unwrap_or_default(),
        &saved_settings.hooks,
        project_trusted,
    );
    let startup_context = command_hooks.start().await;
    let hooks_enabled = !command_hooks.is_empty();
    let before_tool_call = if hooks_enabled {
        Some(command_hooks.before(before_tool_call))
    } else {
        before_tool_call
    };
    let before_tool_call = Some(crate::mcp::approval_hook(
        before_tool_call,
        mcp_approvals.clone(),
        ui_dialog_mailbox.clone(),
    ));
    let after_tool_call = hooks_enabled.then(|| command_hooks.after(None));

    let options = AgentHarnessOptions {
        model: resolved.model.clone(),
        thinking_level: resolved.thinking_level,
        active_tool_names: active,
        tools,
        system_prompt: Some(system_prompt),
        resources: AgentHarnessResources {
            context_files: context_paths,
            skills: if skills.is_empty() {
                None
            } else {
                Some(skills)
            },
            prompt_templates: if prompt_templates.is_empty() {
                None
            } else {
                Some(prompt_templates)
            },
        },
        // A restored session (--continue/--resume/--session) already has
        // records — let the harness load it and keep appending.
        allow_existing_session: matches!(
            selection,
            SessionSelection::Latest { .. }
                | SessionSelection::ById { .. }
                | SessionSelection::ByExactId { .. }
                | SessionSelection::Fork { .. }
        ),
        stream_options: AgentHarnessStreamOptions {
            timeout: args.timeout,
            cache_retention: crate::args::cache_retention_from_env(),
            ..Default::default()
        },
        // Settings-backed harness options: upstream lets users tune retry,
        // compaction and queue drain modes from settings.json. Global only,
        // matching how `defaultTools` and the other settings-backed options in
        // this file are read.
        retry: saved_settings.retry_policy(),
        compaction: saved_settings.compaction_settings(),
        steering_mode: saved_settings.steering_mode(),
        follow_up_mode: saved_settings.follow_up_mode(),
        tool_execution: HarnessToolExecution::default(),
        drive: DrivingMode::default(),
        session,
        // B5c: inject the resolved gateway provider PLUS one `Arc<dyn Provider>`
        // per registered extension provider (`PluggableProvider` wraps a plugin's
        // sync `ProviderRequestFn`). The harness's `build_stream_fn` resolves a
        // provider lazily per call by `models.iter().find(|p| p.id() == model.provider)`,
        // so a catalog model whose `provider` matches an extension provider's id
        // routes to it. Extension providers land AFTER the gateway so the gateway
        // stays first-match for its own ids (first-wins on a `.find`).
        models: build_models_with_extensions(resolved, &extension_session, runtime.clone()),
        to_provider_messages: None,
        entry_projectors: Default::default(),
        agent_emitter: Some(emitter),
        // Command hooks compose with plugin approval, never replacing its veto.
        before_tool_call,
        after_tool_call,
        transform_context: hooks_enabled.then(|| command_hooks.transform()),
        entry_transforms: Vec::new(),
        // Extension provider hooks (B4): plugins subscribing to the
        // BeforeProviderRequest / BeforeProviderHeaders / AfterProviderResponse
        // events observe every provider call (observer semantics — the handler
        // ABI has no patch channel in v1). A session without provider-hook
        // subscribers runs hook-free.
        provider_hooks: ice_extensions::ExtensionProviderHooks::from_session(&extension_session)
            .map(|h| Arc::new(h) as Arc<dyn ice_ai::ProviderHooks>),
    };

    let harness = match AgentHarness::create(options).await {
        Ok(h) => {
            // CLI sessions opt into the coding product policy; embedded generic
            // harnesses keep their reserve-based behavior unless explicitly set.
            h.set_coding_compaction_policy(Some(saved_settings.coding_agent_compaction_policy()))
                .await
                .map_err(|error| BuildError::HarnessCreate(error.to_string()))?;
            if let Some(reserve_tokens) = saved_settings.branch_summary_reserve_tokens() {
                h.set_branch_summary_reserve_tokens(reserve_tokens)
                    .await
                    .map_err(|error| BuildError::HarnessCreate(error.to_string()))?;
            }
            // Fill the extension action host now that the harness exists
            // (plugin runtime_action calls can then reach it).
            crate::extensions_actions::HarnessActionHost::set_harness(
                &harness_cell,
                Arc::new(h.clone()),
            );
            h
        }
        Err(e) => return Err(BuildError::HarnessCreate(e.to_string())),
    };

    if !startup_context.is_empty() {
        let prompt = harness
            .get_system_prompt()
            .await
            .map_err(|error| BuildError::HarnessCreate(error.to_string()))?
            .unwrap_or_default();
        harness
            .set_system_prompt(Some(format!("{prompt}\n{}", startup_context.join("\n"))))
            .await
            .map_err(|error| BuildError::HarnessCreate(error.to_string()))?;
    }

    // ---- Durable operation recovery (runtime layer) ----
    // A previous process can die between `operation_started` and
    // `operation_finished`, leaving the lane with an orphaned operation. Sweep
    // every lane, surface what was found, and emit `run_start`/`run_end`
    // (aborted) pairs so a TUI that already attached sees the settle. This is
    // read-only: nothing is mutated here, so a failed sweep cannot corrupt the
    // log. A clean session (the common case) reports no findings.
    {
        let runtime = ice_harness::runtime::SessionRuntime::new(harness.session().clone());
        match ice_harness::runtime::recover_all_lanes(&runtime).await {
            Ok(reports) => {
                for (lane, report) in reports {
                    if report.is_clean() {
                        continue;
                    }
                    if report.is_corrupt() {
                        eprintln!(
                            "warning: session lane '{lane}' is corrupt ({} open operations); \
                             refusing to auto-repair — inspect the session file",
                            report.open_operations.len()
                        );
                        continue;
                    }
                    for decision in runtime.reconcile(&report) {
                        if let ice_harness::runtime::RecoveryDecision::Abort { run_id } = decision {
                            if args.verbose {
                                eprintln!(
                                    "recovered interrupted run {run_id} on lane '{lane}' (aborted)"
                                );
                            }
                        }
                    }
                    runtime.emit_recovery_events(harness.events(), &report);
                }
            }
            Err(e) => {
                if args.verbose {
                    eprintln!("warning: session recovery sweep failed: {e}");
                }
            }
        }
    }

    let mcp_tools = crate::mcp::McpManager::new(
        &harness,
        args,
        cwd,
        crate::config::agent_dir().map_err(|e| BuildError::SessionDir(e.to_string()))?,
        project_trusted,
        mcp_approvals.clone(),
    )
    .with_approval_ui(ui_dialog_mailbox.clone());
    use std::io::IsTerminal;
    let providers = Arc::new(std::sync::RwLock::new(resolved.providers.clone()));
    // Same decision as `app.rs`, including the forced TUI.
    let mcp_tools = if crate::args::force_tui()
        || crate::args::resolve_mode(
            args,
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
        ) == crate::args::RunMode::Interactive
    {
        mcp_tools
            .with_elicitation_ui(ui_dialog_mailbox.clone())
            .with_sampling(providers.clone())
    } else {
        mcp_tools
    };
    let ready = crate::mcp::start(mcp_tools.clone());
    harness.set_lifecycle_hooks(Arc::new(crate::mcp::McpReadyGate {
        inner: Some(command_hooks),
        ready,
    }));

    harness.set_extensions(builtins.harness_extensions());
    let mut capability_service = None;
    if !args.no_tools && tool_name_allowed("capability_search", args) {
        let service = crate::capability_service::CapabilityService::new(
            crate::capability_catalog::CapabilityCatalog::from_live(
                &live_skill_result,
                &crate::resource_dirs::skill_source_dirs(cwd, true, true, builtins.skill_paths()),
                &mcp_tools.snapshot(),
            ),
        );
        let mut tools = harness.get_tools().await.unwrap_or_default();
        tools.push(service.tool());
        let active = filter_active_tool_names(active_tool_names(&tools, args), args);
        let _ = harness.set_tools(tools, Some(active)).await;
        if let Some(cognee) = &builtins.cognee {
            cognee.bind_capability_service(&service);
        }
        capability_service = Some(service.clone());
        let mut extensions = harness.extensions();
        extensions.push(service as Arc<dyn ice_harness::extension::HarnessExtension>);
        harness.set_extensions(extensions);
    }
    builtins
        .bind(
            &harness,
            ext_status_mailbox.clone(),
            cwd.to_path_buf(),
            tool_name_allowed(crate::cognee::runtime::SEARCH_TOOL_NAME, args),
        )
        .await;
    if let Some(service) = &capability_service {
        let tools = harness.get_tools().await.unwrap_or_default();
        let active_names: std::collections::HashSet<_> =
            active_tool_names(&tools, args).into_iter().collect();
        let active_tools: Vec<_> = tools
            .into_iter()
            .filter(|tool| active_names.contains(&tool.tool.schema().name))
            .collect();
        let catalog = crate::capability_catalog::CapabilityCatalog::from_live(
            &live_skill_result,
            &crate::resource_dirs::skill_source_dirs(cwd, true, true, builtins.skill_paths()),
            &mcp_tools.snapshot(),
        )
        .with_native_tools(&active_tools);
        service.refresh(catalog);
    }

    // ---- B5d: assemble the ReloadContext the TUI holds ----
    // Every field is cheap to clone (Arc / Vec / args Clone). The cells own the
    // live session + bridge so `/reload` can swap them; the harness itself is
    // NOT held here (the TUI already owns a `&AgentHarness` / clone at the call
    // site — passing it into `reload_extension_resources` keeps this structfree
    // of a harness back-reference so it can be `Clone` into the reload callback).
    // Only a TUI (local, or remote through `--server`) may be woken by a
    // background notice; headless clients run just the turns they ask for.
    let wake_idle = args.mode == crate::args::Mode::Text && !args.print;
    shells.connect(&harness, wake_idle);
    let reload_context = ReloadContext {
        skill_registry: Arc::new(std::sync::RwLock::new(live_skill_result)),
        capability_service: capability_service.clone(),
        mcp_tools,
        shells,
        extension_session: extension_session_cell,
        action_bridge: Arc::new(Mutex::new(Some(Arc::clone(&action_bridge)))),
        tool_context,
        catalog: Arc::new(std::sync::RwLock::new(catalog)),
        providers,
        runtime: runtime.clone(),
        cwd: cwd.to_path_buf(),
        project_trusted,
        args: args.clone(),
        resolved_model: resolved.model.clone(),
        broadcast: broadcast_for_context,
        mailbox: reload_mailbox,
        ui_dialog: ui_dialog_mailbox,
        ext_status: ext_status_mailbox,
        editor_text: editor_text_mailbox,
        dev_extension: None,
        builtins,
    };

    Ok((harness, event_rx, reload_context))
}

/// Resolve the current plugin session for each call rather than keeping a
/// startup snapshot. A clone retains its libraries while approval is pending.
fn extension_tool_call_hook(
    sessions: ExtensionSessionCell,
    ui_dialog: ice_extensions::UiDialogMailbox,
) -> ice_agent::BeforeToolCall {
    Arc::new(move |ctx: ice_agent::BeforeToolCallContext<'_>, signal| {
        let session = sessions
            .lock()
            .map(|session| session.clone())
            .map_err(|_| ());
        let ui_dialog = ui_dialog.clone();
        let tool_call_id = ctx.tool_call.id.clone();
        let tool_name = ctx.tool_call.name.clone();
        let params = ctx.args.clone();
        Box::pin(async move {
            let reason = match session {
                Ok(session) => {
                    ice_extensions::dispatch_tool_call_event(
                        &session,
                        &tool_call_id,
                        &tool_name,
                        &params,
                        &signal,
                        &ui_dialog,
                    )
                    .await
                }
                Err(_) => Some("extension approval session is unavailable".to_string()),
            };
            reason.map(|reason| ice_agent::BeforeToolCallResult {
                block: true,
                reason: Some(reason),
                terminate: false,
                args: None,
            })
        })
    })
}

// ===========================================================================
// B5d — `/reload`: re-run extension + resource discovery into a LIVE harness
// ===========================================================================
//
// `/reload` (interactive TUI command, or a plugin's `runtime_action(Reload)`)
// re-runs everything `build` did around resources/extensions WITHOUT rebuilding
// the `AgentHarness` itself (rebuilding would tear down the session/lane/event
// wiring + the broadcast drain task the TUI owns). Instead it:
//
//  1. Builds a fresh `ExtensionSession` (re-load the cdylibs) over the same
//     dir set, with a FRESH `ActionBridge` (the old one is `invalidate`d so
//     in-flight plugin→host calls on the old bridge fail fast).
//  2. Fans `resources_discover(_, "reload")` over the fresh snapshot.
//  3. Re-runs the Part-A loaders (skills/prompts/context/SYSTEM.md/
//     APPEND_SYSTEM.md) with the discovered paths merged in — same precedence
//     + `--no-*` gates as startup.
//  4. Rebuilds the harness's live state via the B5d setters
//     (`set_system_prompt`/`set_resources`/`set_agent_emitter`/`set_models`/
//     `set_provider_hooks`/`set_tools`) so the NEXT run observes the reloaded
//     config (in-flight runs finish on the old `ConfigSnapshot`).
//  5. Swaps the cells (`ExtensionSession`, `ActionBridge`, harness action
//     host's harness cell stays — the harness is the same object) and drops
//     the old session + bridge (their keepalives unmap the old cdylibs; the
//     new session's keepalive holds the fresh mappings).
//
// The reload is a `ice-cli` concern (NOT a harness op): `ice-extensions`
// carries only the `ActionBridge` staleness flag + a `ReloadMailbox` `()` signal
// (no pi-cli `TuiMessage` type — leaf DAG preserved). The TUI owns the mailbox
// receiver + the actual reload routine; a plugin's
// `runtime_action(Reload)` signals the mailbox and returns `Ok(null)`
// immediately so the calling plugin's cdylib is NOT unmapped while its
// `runtime_action` frame is still on the stack (the self-unmapping race a
// synchronous plugin-initiated reload would have).
//
// `reload_extension_resources` is the shared routine both `/reload` (TUI) and
// a plugin's `runtime_action(Reload)` (via the mailbox) drive. It is `pub` so
// the TUI's main-loop handler + the mailbox-driven path call the same code.

/// The cell that holds the live `ExtensionSession` across a `/reload`. Cloned
/// into every site that needs the current session (the TUI, the reload
/// callback). On reload the old session is `replace`d out (its `active` flag
/// flipped) and the fresh one stored atomically. In-flight callbacks retain
/// the old keepalive until they return; callers never observe an empty
/// placeholder during the swap.
pub type ExtensionSessionCell = Arc<Mutex<ExtensionSession>>;

/// The cell that holds the live `ActionBridge` across a `/reload`. A plugin
/// stores the bridge's raw `user_data` pointer during `register`; on reload the
/// old bridge is `invalidate`d (in-flight calls fail fast) and the fresh one
/// `store`d. The fresh session's plugins are handed the fresh bridge pointer.
pub type ActionBridgeCell = Arc<Mutex<Option<Arc<ice_extensions::ActionBridge>>>>;

/// Everything `/reload` needs to rebuild extension + resource state into a live
/// harness. Built once in [`build`] (alongside the harness) and held by the TUI
/// (cloned into the reload callback the bridge carries + the `/reload` command
/// handler). The harness itself is NOT held here — the TUI already owns a
/// `&AgentHarness` / a clone; passing it at the call site keeps this struct
/// free of a harness back-reference (so it can be `Clone` and moved into the
/// reload callback without borrowing the harness).
#[derive(Clone)]
pub struct ReloadContext {
    /// Session-shared local capability registry used by the model tool and task hints.
    pub capability_service: Option<Arc<crate::capability_service::CapabilityService>>,
    /// Last skill loader result, including shadowed definitions.
    pub skill_registry: Arc<std::sync::RwLock<ice_harness::skills::LoadSkillsResult>>,
    /// Session-local MCP tools retained across extension reloads.
    pub mcp_tools: crate::mcp::McpManager,
    /// The session's background shells and watches, kept across reloads so
    /// running shells stay reachable from the rebuilt tools.
    pub shells: Arc<crate::background_shells::SessionShells>,
    /// The live extension-session cell (swapped on reload).
    pub extension_session: ExtensionSessionCell,
    /// The session's plugin-tool context (`cwd` + session id), reused — not
    /// rebuilt — on reload so the rebuilt adapters keep serving the SAME
    /// session. A reload that made a fresh context would leave every plugin
    /// tool without a session id until the next launch.
    pub tool_context: ice_extensions::ToolCallContext,
    /// The live action-bridge cell (swapped + old invalidated on reload).
    pub action_bridge: ActionBridgeCell,
    /// The extension status registry the TUI renders in its footer (`SetStatus`).
    /// Lives in the context (not the cell) so it survives a bridge swap.
    pub ext_status: ice_extensions::ExtensionStatusMailbox,
    /// The extension editor-text queue the TUI drains into the prompt editor
    /// (`SetEditorText`, runtime action 19). Lives in the context (not the cell)
    /// so it survives a bridge swap — exactly like `ext_status`. A voice plugin
    /// writes a transcription here as a *draft*; the TUI applies it, starts the
    /// optional auto-send countdown, and submits through the normal path so the
    /// user bubble renders as usual.
    pub editor_text: ice_extensions::EditorTextMailbox,
    /// Shared authenticated model catalog for runtime actions and the TUI.
    /// Provider-package reloads replace it together with the runtime bindings.
    pub catalog: Arc<std::sync::RwLock<Vec<ice_ai::Model>>>,
    /// Current configured provider runtimes. Replaced after package reload so
    /// subsequent reloads cannot recover stale plugin snapshots.
    pub providers: Arc<std::sync::RwLock<Vec<Arc<dyn Provider>>>>,
    /// The ambient runtime handle (captured in `build`) — `PluggableProvider`
    /// + the fresh `ActionBridge` need a captured `Handle` to spawn from any
    /// thread.
    pub runtime: tokio::runtime::Handle,
    /// The cwd (for static resource-dir resolution + context-file walk).
    pub cwd: PathBuf,
    /// The project-trust decision captured before provider and session setup.
    /// Startup consumers reuse this value so extension code cannot change the
    /// effective policy by mutating the process cwd while it is loading.
    pub project_trusted: bool,
    /// The parsed args (cloned) — `--no-*`/`--tools`/`--exclude-tools`/
    /// `--extensions-dir`/`--no-extensions`/`--system-prompt`/etc all apply on
    /// reload exactly as at startup (a reload re-reads the same flags; it does
    /// not pick up argv changes mid-session, which is the right contract — pi's
    /// `/reload` re-runs discovery with the same config).
    pub args: Args,
    /// The resolved model + thinking level (the harness's active model stays
    /// unless `set_model` changed it; reload does not touch the model).
    pub resolved_model: ice_ai::Model,
    /// The broadcast emitter the harness was built with. Reload rebuilds the
    /// `TeeEmitter` over the fresh `ExtensionEmitter` (the old tee's extension
    /// child is dropped, unsubscribing from the old registry). The broadcast
    /// half stays live the whole session (the TUI's drain task holds the
    /// receiver), so we keep a handle to re-wrap.
    pub broadcast: Arc<dyn ice_agent::AgentEmitter>,
    /// The session-long reload mailbox (B5d). Build creates one, installs it on
    /// the initial `ActionBridge` via [`reload_callback_from_mailbox`], and hands
    /// a clone to the TUI. The TUI installs its `TuiMessage` sender so a plugin's
    /// `runtime_action(Reload)` signals the main loop — the reload routine reuses
    /// THIS mailbox (not a fresh default) when building the fresh bridge, so the
    /// bridge always carries the mailbox the TUI installed across reloads.
    pub mailbox: ice_extensions::ReloadMailbox,
    /// Session-long UI-dialog mailbox (runtime action 17). The initial bridge
    /// carries it; the TUI attaches itself and polls `take_pending` from its key
    /// loop, and `/reload` reuses THIS mailbox when it builds the fresh bridge.
    /// A detached mailbox makes `ask_user` fail with "requires an interactive
    /// UI" (headless contract).
    pub ui_dialog: ice_extensions::UiDialogMailbox,
    /// Active `icers dev` extension builder. `/reload` rebuilds it before
    /// swapping plugin sessions; its watcher signals `mailbox` after a
    /// successful background build.
    pub dev_extension: Option<Arc<crate::dev_extension::DevExtension>>,
    /// Built-in Cognee/Blackhole extensions installed into the harness.
    pub builtins: crate::builtin_extensions::BuiltinExtensions,
}

impl ReloadContext {
    /// Build a read-only inventory from the last skill load and MCP discovery.
    pub fn capability_catalog(&self) -> crate::capability_catalog::CapabilityCatalog {
        let project = self.args.dev_local_only || self.args.project_only || self.project_trusted;
        let global = !self.args.dev_local_only && !self.args.project_only;
        let dirs = crate::resource_dirs::skill_source_dirs(
            &self.cwd,
            project,
            global,
            self.builtins.skill_paths(),
        );
        crate::capability_catalog::CapabilityCatalog::from_live(
            &self.skill_registry.read().unwrap(),
            &dirs,
            &self.mcp_tools.snapshot(),
        )
    }
}

/// Re-read the local model configuration and replace only one provider's live
/// catalog and request bindings. The active lane and extension hooks are left
/// untouched. If its active model was removed, reject the reload rather than
/// leaving the lane pointing at a model that can no longer be routed.
///
/// Discovered descriptors are merged in memory; local
/// definitions take precedence. Returns the number of published models.
pub async fn reload_provider_catalog(
    ctx: &ReloadContext,
    harness: &AgentHarness,
    provider_id: &str,
    discovered: Option<Vec<ice_ai::Model>>,
) -> Result<usize, String> {
    let active = harness
        .lane("main")
        .get_model()
        .await
        .map_err(|e| e.to_string())?;
    let mut args = ctx.args.clone();
    args.provider = Some(provider_id.to_owned());
    args.model = None;
    if provider_id != ctx.resolved_model.provider {
        args.api_key = None;
        args.base_url = None;
    }
    let discovered = discovered.unwrap_or_default();
    if discovered
        .iter()
        .any(|model| model.provider != provider_id || model.headers.is_some())
    {
        return Err(
            "discovered models must belong to the selected provider and contain no headers".into(),
        );
    }
    let resolved = {
        let session = ctx.extension_session.lock().unwrap();
        crate::provider::resolve_with_extensions_and_models(
            &args,
            &ctx.cwd,
            ctx.project_trusted,
            &session,
            &discovered,
        )
        .map_err(|e| e.to_string())?
    };
    let selected = resolved.model.provider.as_str();
    if !selected.eq_ignore_ascii_case(provider_id) {
        return Err(format!(
            "resolved provider '{selected}' does not match requested provider '{provider_id}'"
        ));
    }
    let replacement: Vec<_> = resolved
        .providers
        .iter()
        .filter(|provider| provider.id() == selected)
        .cloned()
        .collect();
    let selected_models: Vec<_> = crate::provider::authenticated_catalog(&resolved)
        .into_iter()
        .filter(|model| model.provider == selected)
        .collect();
    if replacement.is_empty() || selected_models.is_empty() {
        return Err(format!(
            "no authenticated models for provider '{provider_id}'"
        ));
    }
    if active.provider == selected
        && !selected_models
            .iter()
            .any(|model| model.id == active.id && model.api == active.api)
    {
        return Err(format!(
            "active model '{}/{}' is absent from the refreshed catalog",
            active.provider, active.id
        ));
    }
    let mut providers = ctx
        .providers
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    providers.retain(|provider| provider.id() != selected);
    providers.extend(replacement);
    let mut catalog = ctx
        .catalog
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    catalog.retain(|model| model.provider != selected);
    let count = selected_models.len();
    catalog.extend(selected_models);
    let models = {
        let session = ctx.extension_session.lock().unwrap();
        build_models_with_extensions_for_reload(&providers, &session, ctx.runtime.clone())
    };
    harness
        .set_models(models)
        .await
        .map_err(|e| e.to_string())?;
    *ctx.providers.write().unwrap_or_else(|e| e.into_inner()) = providers;
    *ctx.catalog.write().unwrap_or_else(|e| e.into_inner()) = catalog.clone();
    if let Some(bridge) = ctx.action_bridge.lock().unwrap().as_ref() {
        bridge.clone_host().replace_model_catalog(catalog);
    }
    Ok(count)
}

/// Dispatch a no-payload lifecycle event (e.g. `SessionStart`, `SessionShutdown`,
/// the ice-specific `BeforeTuiStart`) to every extension handler subscribed to
/// that tag, with **veto** + **timeout** semantics (P1).
///
/// Resolves the **current** extension session from the reload cell so a
/// `/reload`-swapped session is honored. Each handler runs on a blocking thread
/// with a per-event timeout ([`ice_extensions::lifecycle_timeout_for`]); a
/// handler returning [`ice_plugin_sdk::EVENT_HANDLER_ABORT`] vetoes the phase
/// and this returns `Some(reason)`. No-op (`None`) when no plugins loaded, no
/// handler subscribes, or the registry is stale.
///
/// The caller decides what a veto means: a `BeforeTuiStart` veto aborts startup
/// (never enter the TUI); `SessionStart`/`SessionShutdown` vetoes are advisory
/// (the session is already up or already closing).
pub async fn dispatch_session_event_async(
    reload_context: &ReloadContext,
    tag: ice_plugin_sdk::EventTag,
) -> Option<String> {
    // Take the snapshot Arc + keepalive, drop the session lock, then dispatch —
    // we must not hold the session cell's mutex across the await (a handler
    // could re-enter a reload path that locks it). Holding the keepalive across
    // the await keeps the cdylibs (owning the handler fn pointers) mapped even
    // if a concurrent `/reload` swaps the session cell out.
    let (snapshot, keepalive) = {
        let session = reload_context.extension_session.lock().unwrap();
        (session.snapshot_arc(), session.keepalive())
    };
    let veto = match snapshot {
        Some(snapshot) => ice_extensions::dispatch_lifecycle_event(&snapshot, tag, keepalive).await,
        None => None,
    };
    match tag {
        ice_plugin_sdk::EventTag::SessionStart => reload_context.builtins.session_start().await,
        ice_plugin_sdk::EventTag::SessionShutdown => {
            reload_context.builtins.session_shutdown().await
        }
        _ => {}
    }
    veto
}

/// Join plugin teardown before unloading modules. A detached timed-out handler
/// could still execute code in the old DLL, so it cannot safely permit a swap.
async fn shutdown_extensions_for_reload(
    snapshot: Option<Arc<ice_extensions::RegistrySnapshot>>,
    keepalive: Arc<ice_extensions::PluginKeepalive>,
) -> Result<(), String> {
    if let Some(snapshot) = snapshot {
        tokio::task::spawn_blocking(move || {
            let _keepalive = keepalive;
            ice_extensions::dispatch_empty_event(
                &snapshot,
                ice_plugin_sdk::EventTag::SessionShutdown,
            );
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The outcome of a reload: a human-readable status line for the transcript
/// (counts of what reloaded), and whether any load diagnostics appeared.
pub struct ReloadOutcome {
    /// One-line summary for the transcript note (e.g. "Reloaded 2 plugin(s),
    /// 5 skill(s), 1 prompt(s).").
    pub summary: String,
    /// True iff at least one extension load warning fired (ABI mismatch / skip).
    pub had_warnings: bool,
    /// Formatted diagnostic lines (package / skill / prompt-template warnings)
    /// that the caller should surface in its UI. Kept out of `eprintln!`
    /// because the TUI owns the terminal: direct stderr writes during a run
    /// would corrupt the alternate-screen input row.
    pub details: Vec<String>,
    /// True iff the reload itself failed hard (e.g. the extension build or
    /// settings reload errored) and the previous resources were kept. Soft
    /// diagnostics (skill shadowing, name validation) do not set this.
    pub had_errors: bool,
}

struct PreparedReloadInputs {
    extension_dirs: Vec<PathBuf>,
    skill_base_dirs: Vec<PathBuf>,
    prompt_base_dirs: Vec<PathBuf>,
}

/// Append the resource sources that are specific to a reload after the
/// conventional project/global directories. Keep this order aligned with the
/// initial build: explicit CLI paths must remain available after `/reload`,
/// while discovered and package resources retain their lower precedence.
fn append_reload_resource_paths(
    mut paths: Vec<PathBuf>,
    explicit: &[PathBuf],
    discovered: &[String],
    js_paths: &[PathBuf],
    package_paths: &[PathBuf],
) -> Vec<PathBuf> {
    paths.extend(explicit.iter().cloned());
    paths.extend(discovered.iter().map(PathBuf::from));
    paths.extend(js_paths.iter().cloned());
    paths.extend(package_paths.iter().cloned());
    paths
}

/// Re-run extension + resource discovery and push the rebuilt state into the
/// live `harness` via the B5d setters. The old `ExtensionSession` +
/// `ActionBridge` are invalidated + swapped in [`ReloadContext`]'s cells. This
/// is the single routine both `/reload` (TUI) and a plugin's
/// `runtime_action(Reload)` drive (the latter via the mailbox signal).
///
/// Returns a [`ReloadOutcome`] for the transcript. Best-effort: a failure in
/// one channel (e.g. a plugin that fails to reload) does not abort the others —
/// the reload completes with whatever loaded, mirroring pi's per-plugin
/// skip-on-error. A hard failure (e.g. the harness is closed) surfaces as an
/// error summary.
pub async fn reload_extension_resources(
    harness: &AgentHarness,
    ctx: &ReloadContext,
) -> ReloadOutcome {
    reload_extension_resources_inner(harness, ctx, || {}).await
}

async fn reload_extension_resources_inner<F>(
    harness: &AgentHarness,
    ctx: &ReloadContext,
    after_prepare: F,
) -> ReloadOutcome
where
    F: FnOnce() + Send,
{
    // Serialize publication with MCP reconnect/startup so neither loses tools.
    let _mcp_registry = ctx.mcp_tools.lock_registry().await;
    // Resolve the arguments once for this reload. `icers dev` may append its
    // freshly staged extension directory; every subsequent loader and policy
    // decision must observe that same effective set rather than falling back
    // to the pre-dev snapshot held in `ctx.args`.
    let mut effective_args = ctx.args.clone();
    if let Some(dev) = &ctx.dev_extension {
        if let Err(error) = dev
            .rebuild()
            .and_then(|_| dev.apply_to_args(&mut effective_args))
        {
            return ReloadOutcome {
                summary: format!(
                    "Extension build failed for {}: {error}. Keeping the currently loaded version.",
                    dev.package_name()
                ),
                had_warnings: true,
                details: Vec::new(),
                had_errors: true,
            };
        }
    }
    let cwd_str = ctx.cwd.to_string_lossy().to_string();
    let project_trusted = resolve_project_trust(&effective_args, &ctx.cwd);
    let prepared = match prepare_reload_inputs(&effective_args, &ctx.cwd, project_trusted) {
        Ok(prepared) => prepared,
        Err(error) => {
            return ReloadOutcome {
                summary: format!(
                    "Settings reload failed: {error}. Keeping the currently loaded resources."
                ),
                had_warnings: true,
                details: Vec::new(),
                had_errors: true,
            };
        }
    };
    after_prepare();
    let PreparedReloadInputs {
        extension_dirs,
        skill_base_dirs,
        prompt_base_dirs,
    } = prepared;
    let mut warnings = false;

    // Shutdown callbacks must finish before a replacement can unload old DLLs.
    // Ordinary lifecycle dispatch can time out or stop on a veto; unload cleanup
    // instead joins every handler while keeping its module mapped.
    let (shutdown_snapshot, shutdown_keepalive) = {
        let session = ctx.extension_session.lock().unwrap();
        (session.snapshot_arc(), session.keepalive())
    };
    if let Err(reason) = shutdown_extensions_for_reload(shutdown_snapshot, shutdown_keepalive).await
    {
        return ReloadOutcome {
            summary: "Reload failed during extension shutdown".into(),
            had_warnings: true,
            details: vec![reason],
            had_errors: true,
        };
    }

    // ---- 1. Build a fresh ActionBridge + ExtensionSession ----
    // The fresh bridge carries the SAME `HarnessActionHost` (the host's harness
    // cell already points at this harness; the host impl is reusable across
    // reloads — only the bridge's staleness flag + reload callback differ). We
    // re-use the host by reading it off the OLD bridge (it's the same
    // `Arc<dyn RuntimeActionHost>`).
    let old_bridge = ctx.action_bridge.lock().unwrap().clone();
    let host: Arc<dyn ice_extensions::RuntimeActionHost> = match &old_bridge {
        Some(b) => b.clone_host(),
        None => {
            // No prior bridge (no extensions ever loaded). Build a fresh host so
            // a reload that newly discovers plugins can still drive actions.
            let (action_host, _cell) = crate::extensions_actions::HarnessActionHost::new_empty(
                ctx.catalog
                    .read()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone(),
                ctx.cwd.clone(),
                ctx.runtime.clone(),
                ctx.args.unknown_flags.clone(),
                ctx.tool_context.clone(),
            );
            crate::extensions_actions::HarnessActionHost::set_harness(
                &_cell,
                Arc::new(harness.clone()),
            );
            Arc::new(action_host)
        }
    };

    let reload_cb = ice_extensions::reload_callback_from_mailbox(ctx.mailbox.clone());
    let fresh_bridge = ice_extensions::ActionBridge::with_reload_and_ui(
        ctx.runtime.clone(),
        host,
        reload_cb,
        ctx.ui_dialog.clone(),
        ctx.ext_status.clone(),
        ctx.editor_text.clone(),
    );

    let extension_session = if effective_args.no_extensions {
        ice_extensions::ExtensionSession::none()
    } else {
        load_extensions_from_dirs(
            &effective_args,
            &extension_dirs,
            Some(Arc::clone(&fresh_bridge)),
        )
    };
    if extension_session.is_empty() && !effective_args.no_extensions {
        // The fresh session may be empty if no cdylibs are present — not a
        // warning per se, but note it.
    }
    if effective_args.verbose {
        if let Some(s) = extension_session.summary() {
            eprintln!("reload: {s}");
        }
        report_deferred_renderers(&extension_session);
    }

    let has_package_providers = |session: &ExtensionSession| {
        session.snapshot().is_some_and(|snapshot| {
            snapshot
                .providers()
                .iter()
                .any(|provider| snapshot.provider_manifest(&provider.provider_id).is_some())
        })
    };
    let packaged = has_package_providers(&extension_session)
        || has_package_providers(&ctx.extension_session.lock().unwrap());
    // Re-fetch the signed-in ChatGPT account's model list so newly granted
    // models appear and unavailable ones disappear. A failure keeps the cached
    // list and is reported with the reload outcome.
    let (_codex_models_changed, mut codex_models_error) =
        if effective_args.offline || crate::args::offline_env_enabled() {
            (false, None)
        } else {
            match crate::codex_models::refresh(
                crate::provider::OPENAI_CODEX_BASE_URL,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            {
                Ok(changed) => (changed, None),
                Err(error) => (false, Some(error)),
            }
        };
    // Reauth can change credentials without changing the cached Codex catalog.
    // Re-resolve on every reload rather than keeping stale execution bindings.
    let refreshed = {
        let mut args = effective_args.clone();
        if let Ok(current) = harness.lane("main").get_model().await {
            let was_package = ctx
                .extension_session
                .lock()
                .unwrap()
                .snapshot()
                .is_some_and(|snapshot| snapshot.provider_manifest(&current.provider).is_some());
            let has_replacement = extension_session.snapshot().is_some_and(|snapshot| {
                snapshot
                    .providers()
                    .iter()
                    .any(|provider| provider.provider_id == current.provider)
            });
            if was_package && !has_replacement {
                return ReloadOutcome {
                    summary: "Reload failed while resolving provider packages".into(),
                    had_warnings: true,
                    details: vec![format!(
                        "active provider package '{}' is no longer loaded",
                        current.provider
                    )],
                    had_errors: true,
                };
            }
            args.provider = Some(current.provider.clone());
            args.model = Some(current.id);
            if current.provider != ctx.resolved_model.provider {
                args.api_key = None;
                args.base_url = None;
            }
        }
        match crate::provider::resolve_with_extensions(
            &args,
            &ctx.cwd,
            ctx.project_trusted,
            &extension_session,
        ) {
            Ok(resolved) => Some(resolved),
            // The refreshed list may no longer include the active model; that
            // must not fail a reload that has no provider packages to apply.
            Err(error) if !packaged => {
                codex_models_error = Some(error.to_string());
                None
            }
            Err(error) => {
                return ReloadOutcome {
                    summary: "Reload failed while resolving provider packages".into(),
                    had_warnings: true,
                    details: vec![error.to_string()],
                    had_errors: true,
                };
            }
        }
    };

    // ---- 2. Invalidate the old session + bridge BEFORE the swap ----
    // The old registry's `active` flag flips false so any in-flight
    // `emit_resources_discover`/event dispatch on the old snapshot no-ops; the
    // old bridge's flag flips false so in-flight `runtime_action` calls parked
    // on the old `user_data` hit the staleness guard. Invalidate under the
    // session lock before exposing the replacement to approval callbacks.
    //
    // Swap directly to the fresh session under the same lock that approvals
    // use. A temporary empty session would let a concurrent call bypass its gate.
    // In-flight approvals retain their old session until their callback exits;
    // its staleness flag prevents them from authorizing after this swap.
    {
        let mut session_guard = ctx.extension_session.lock().unwrap();
        let old_session = std::mem::replace(&mut *session_guard, extension_session.clone());
        if let Some(old_snap) = old_session.snapshot_arc() {
            // `invalidate` is on the registry, but the snapshot shares the flag —
            // flipping the snapshot's flag invalidates the registry too (same Arc).
            // `RegistrySnapshot` exposes `active_flag()` for this.
            old_snap.active_flag().store(false, Ordering::SeqCst);
        }
        // `old_session` drops here; in-flight adapters/callbacks keep their
        // own library references until they return.
    }
    if let Some(old_b) = old_bridge {
        old_b.invalidate();
    }

    // The session was swapped atomically above; expose its matching bridge.
    *ctx.action_bridge.lock().unwrap() = Some(Arc::clone(&fresh_bridge));

    // ---- 3. resources_discover ("reload") over the fresh snapshot ----
    let discovered = extension_session
        .snapshot_arc()
        .map(|snap| ice_extensions::emit_resources_discover(&cwd_str, "reload", &snap))
        .unwrap_or_default();

    // ---- 4. Re-run the Part-A loaders (same precedence + --no-* gates) ----
    let env = Arc::new(ice_tools::OsExecutionEnv::with_cwd(ctx.cwd.clone()));
    let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env.clone();

    let mut live_skill_result = ice_harness::skills::LoadSkillsResult::default();
    let mut skills: Vec<ice_harness::types::Skill> = Vec::new();
    let mut skill_diags: Vec<ice_harness::skills::SkillDiagnostic> = Vec::new();
    let mut resource_diags: Vec<ice_harness::diagnostics::ResourceDiagnostic> = Vec::new();
    if !effective_args.no_skills {
        // JS discovery is backed by the session-long lazy Node host. Until
        // that host is swapped as part of a future full JS reload, preserve
        // the paths it contributed at startup across `/reload`.
        let js_paths: &[PathBuf] = &[];
        let package_paths: &[PathBuf] = &[];
        let mut dirs = append_reload_resource_paths(
            skill_base_dirs,
            &effective_args.skill,
            &discovered.skill_paths,
            js_paths,
            &package_paths,
        );
        dirs.extend(ctx.builtins.skill_paths());
        let result = load_skills_with_precedence(&env_dyn, &dirs).await;
        live_skill_result = result.clone();
        skills = result.skills;
        skill_diags = result.diagnostics;
        resource_diags.extend(result.resource_diagnostics);
    }

    *ctx.skill_registry.write().unwrap() = live_skill_result;
    if let Some(service) = &ctx.capability_service {
        service.refresh(crate::capability_catalog::CapabilityCatalog::from_live(
            &ctx.skill_registry.read().unwrap(),
            &crate::resource_dirs::skill_source_dirs(
                &ctx.cwd,
                true,
                true,
                ctx.builtins.skill_paths(),
            ),
            &ctx.mcp_tools.snapshot(),
        ));
    }
    let mut prompt_templates: Vec<ice_harness::types::PromptTemplate> = Vec::new();
    let mut prompt_diags: Vec<ice_harness::prompt_templates::PromptTemplateDiagnostic> = Vec::new();
    if !effective_args.no_prompt_templates {
        let js_paths: &[PathBuf] = &[];
        let package_paths: &[PathBuf] = &[];
        let dirs = append_reload_resource_paths(
            prompt_base_dirs,
            &effective_args.prompt_template,
            &discovered.prompt_paths,
            js_paths,
            &package_paths,
        );
        let result = load_prompt_templates_with_precedence(&env_dyn, &dirs).await;
        prompt_templates = result.prompt_templates;
        prompt_diags = result.diagnostics;
        resource_diags.extend(result.resource_diagnostics);
    }

    let context_files = if effective_args.no_context_files || !project_trusted {
        Vec::new()
    } else {
        let agent_dir = crate::config::agent_dir().ok();
        let agent_dir_path = agent_dir.unwrap_or_else(|| ctx.cwd.clone());
        load_project_context_files(&env_dyn, &ctx.cwd, &agent_dir_path).await
    };
    let context_block = format_project_context(&context_files);

    let mut details: Vec<String> = Vec::new();
    if !skill_diags.is_empty() || !prompt_diags.is_empty() || !resource_diags.is_empty() {
        warnings = true;
        // Collect formatted diagnostics for the caller to surface in its UI
        // instead of writing to stderr directly: the TUI owns the terminal,
        // and a raw `eprintln!` during a run would scribble over the
        // alternate-screen input row.
        for d in &skill_diags {
            details.push(format!(
                "warning: skill {} ({}): {}",
                d.path,
                d.code.as_str(),
                d.message
            ));
        }
        for d in &prompt_diags {
            details.push(format!(
                "warning: prompt template {} ({}): {}",
                d.path,
                d.code.as_str(),
                d.message
            ));
        }
        // Structured collisions: winner (kept) · loser (shadowed).
        for d in &resource_diags {
            if let ice_harness::diagnostics::ResourceDiagnostic::Collision(c) = d {
                details.push(format!(
                    "warning: {} name \"{}\" collision: keeping {} (shadowed {})",
                    c.resource_type.as_str(),
                    c.name,
                    c.winner_path,
                    c.loser_path
                ));
            }
        }
    }

    // ---- Re-compose the system prompt (same precedence as build) ----
    // Same working-state wording rule as `build`; a mid-session `/model` switch
    // re-composes the prompt on the next reload, which is the same staleness the
    // rest of this path already has (provider resolution is a startup concern).
    let prior_reasoning_replayed = ice_ai::model::prior_reasoning_is_replayed(&ctx.resolved_model);
    let mut context_paths = Vec::new();
    let base_prompt = match effective_args.system_prompt.as_deref() {
        Some(explicit) => explicit.to_string(),
        None if project_trusted => match discover_system_prompt_file(&ctx.cwd) {
            Some(path) => read_prompt_source(&path, &mut context_paths)
                .unwrap_or_else(|| default_system_prompt(&cwd_str, prior_reasoning_replayed)),
            None => default_system_prompt(&cwd_str, prior_reasoning_replayed),
        },
        None => default_system_prompt(&cwd_str, prior_reasoning_replayed),
    };
    let mut append_texts: Vec<String> = Vec::new();
    for extra in &effective_args.append_system_prompt {
        let text = read_append_target(extra, &mut context_paths).unwrap_or_else(|| extra.clone());
        append_texts.push(text);
    }
    if effective_args.append_system_prompt.is_empty() {
        if let Some(path) = project_trusted
            .then(|| discover_append_system_prompt_file(&ctx.cwd))
            .flatten()
        {
            if let Some(text) = read_prompt_source(&path, &mut context_paths) {
                append_texts.push(text);
            }
        }
    }
    context_paths.extend(
        context_files
            .iter()
            .map(|file| file.path.to_string_lossy().into_owned()),
    );
    let append_join = if append_texts.is_empty() {
        None
    } else {
        Some(append_texts.join("\n\n"))
    };
    let system_prompt = compose_system_prompt(
        Some(&base_prompt),
        &[],
        if context_block.is_empty() {
            None
        } else {
            Some(&context_block)
        },
        append_join.as_deref(),
    );

    // ---- Rebuild the emitter (TeeEmitter over fresh ExtensionEmitter) ----
    let emitter: Arc<dyn ice_agent::AgentEmitter> = match extension_session.snapshot_arc() {
        Some(snapshot) => {
            let ext = ExtensionEmitter::new(snapshot, extension_session.keepalive());
            compose_agent_emitter(ctx.broadcast.clone(), Some(Arc::new(ext)))
        }
        None => ctx.broadcast.clone(),
    };
    let emitter = compose_agent_emitter(emitter, ctx.builtins.agent_emitter());

    // ---- 5. Push the rebuilt state into the live harness via the B5d setters ----
    let resources = AgentHarnessResources {
        context_files: context_paths,
        skills: if skills.is_empty() {
            None
        } else {
            Some(skills.clone())
        },
        prompt_templates: if prompt_templates.is_empty() {
            None
        } else {
            Some(prompt_templates.clone())
        },
    };
    let _ = harness.set_system_prompt(Some(system_prompt)).await;
    let _ = harness.set_resources(resources).await;
    let _ = harness.set_agent_emitter(Some(emitter)).await;
    let _ = harness
        .set_models(build_models_with_extensions_for_reload(
            &refreshed
                .as_ref()
                .map(|resolved| resolved.providers.clone())
                .unwrap_or_else(|| {
                    ctx.providers
                        .read()
                        .unwrap_or_else(|error| error.into_inner())
                        .clone()
                }),
            &extension_session,
            ctx.runtime.clone(),
        ))
        .await;
    let _ = harness
        .set_provider_hooks(
            ice_extensions::ExtensionProviderHooks::from_session(&extension_session)
                .map(|h| Arc::new(h) as Arc<dyn ice_ai::ProviderHooks>),
        )
        .await;

    let mut model_refresh_error: Option<String> = None;
    if let Some(resolved) = &refreshed {
        *ctx.providers
            .write()
            .unwrap_or_else(|error| error.into_inner()) = resolved.providers.clone();
        let catalog = crate::provider::authenticated_catalog(resolved);
        fresh_bridge
            .clone_host()
            .replace_model_catalog(catalog.clone());
        *ctx.catalog
            .write()
            .unwrap_or_else(|error| error.into_inner()) = catalog;
        // Refresh the public descriptor. Credentials remain in the newly bound
        // provider instances installed above and never enter the lane model.
        if let Err(error) = harness.lane("main").set_model(resolved.model.clone()).await {
            model_refresh_error = Some(error.to_string());
        }
    }

    // Re-merge extension tools (a reloaded plugin may have added/removed a
    // tool). The built-in set is rebuilt from scratch + extension tools merged
    // on top, mirroring `build`.
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = env.clone();
    let tool_ctx = ice_tools::ExecutionToolContext::new(env_dyn.clone(), Some(mut_env));
    let mut tools = build_tools(&tool_ctx, &effective_args, Some(&ctx.shells));
    merge_extension_tools(
        &mut tools,
        &extension_session,
        &effective_args,
        &ctx.tool_context,
    );
    if let Some(service) = &ctx.capability_service {
        let active_names: std::collections::HashSet<_> = active_tool_names(&tools, &effective_args)
            .into_iter()
            .collect();
        let active_tools: Vec<_> = tools
            .iter()
            .filter(|tool| active_names.contains(&tool.tool.schema().name))
            .cloned()
            .collect();
        let catalog = ctx.capability_catalog().with_native_tools(&active_tools);
        service.refresh(catalog);
        if !effective_args.no_tools && tool_name_allowed("capability_search", &effective_args) {
            tools.push(service.tool());
        }
    }
    tools.extend(
        ctx.mcp_tools
            .registered_tools()
            .iter()
            .filter(|mcp| {
                !tools
                    .iter()
                    .any(|tool| tool.tool.schema().name == mcp.tool.schema().name)
            })
            .cloned()
            .collect::<Vec<_>>(),
    );
    let mut active = active_tool_names(&tools, &effective_args);
    active = filter_active_tool_names(active, &effective_args);
    let _ = harness.set_tools(tools, Some(active)).await;
    // The rebuilt set drops built-in extension tools; register them again.
    ctx.builtins
        .register_tools(tool_name_allowed(
            crate::cognee::runtime::SEARCH_TOOL_NAME,
            &effective_args,
        ))
        .await;

    if let Some(error) = codex_models_error {
        warnings = true;
        details.push(format!(
            "Could not refresh the ChatGPT Codex model list; keeping the cached list: {error}"
        ));
    }
    if let Some(error) = model_refresh_error {
        warnings = true;
        details.push(format!(
            "Could not apply the refreshed model after reload; keeping the previous model: {error}"
        ));
    }
    let summary = format!(
        "Reloaded {} plugin(s), {} skill(s), {} prompt(s).",
        extension_session.loaded_paths().len(),
        skills.len(),
        prompt_templates.len(),
    );
    ReloadOutcome {
        summary,
        had_warnings: warnings,
        details,
        // Reload completed; any diagnostics here are soft (shadowing / name
        // validation / skipped optional resources), not hard failures.
        had_errors: false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReloadSettingsFields {
    skill_dirs: Option<Vec<String>>,
    prompt_dirs: Option<Vec<String>>,
    extension_dirs: Option<Vec<String>>,
}

impl From<crate::settings::Settings> for ReloadSettingsFields {
    fn from(settings: crate::settings::Settings) -> Self {
        Self {
            skill_dirs: settings.skill_dirs,
            prompt_dirs: settings.prompt_dirs,
            extension_dirs: settings.extension_dirs,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReloadSettingsSnapshot {
    global: ReloadSettingsFields,
    project: Option<ReloadSettingsFields>,
}

fn reload_reads_settings(args: &Args) -> bool {
    !args.dev_local_only && (!args.no_extensions || !args.no_skills || !args.no_prompt_templates)
}

/// Strictly read the settings fields consumed while preparing a reload. The
/// snapshot intentionally excludes UI/model fields that `/reload` does not
/// use, so an unrelated settings save does not invalidate the operation.
fn load_reload_settings_snapshot(
    args: &Args,
    cwd: &Path,
    project_trusted: bool,
) -> Result<Option<ReloadSettingsSnapshot>, String> {
    if !reload_reads_settings(args) {
        return Ok(None);
    }
    let global = crate::settings::load_settings()
        .map_err(|error| format!("could not load global settings: {error}"))?
        .into();
    let project = if project_trusted {
        crate::settings::load_active_project_settings(cwd)
            .map_err(|error| format!("could not load project settings: {error}"))?
            .map(|(_, settings)| settings.into())
    } else {
        None
    };
    Ok(Some(ReloadSettingsSnapshot { global, project }))
}

/// Validate every settings document that this reload will consume before
/// replacing any live extension, bridge, or harness resource.
#[cfg(test)]
fn validate_settings_for_reload(
    args: &Args,
    cwd: &Path,
    project_trusted: bool,
) -> Result<(), String> {
    load_reload_settings_snapshot(args, cwd, project_trusted).map(|_| ())
}

fn prepare_reload_inputs(
    args: &Args,
    cwd: &Path,
    project_trusted: bool,
) -> Result<PreparedReloadInputs, String> {
    prepare_reload_inputs_inner(args, cwd, project_trusted, || {})
}

fn prepare_reload_inputs_inner<F>(
    args: &Args,
    cwd: &Path,
    project_trusted: bool,
    before_verify: F,
) -> Result<PreparedReloadInputs, String>
where
    F: FnOnce(),
{
    let settings_before = load_reload_settings_snapshot(args, cwd, project_trusted)?;

    let mut extension_dirs = if args.no_extensions || args.dev_local_only {
        Vec::new()
    } else if args.project_only {
        project_extension_dirs(cwd)
    } else if project_trusted {
        extension_dirs(cwd)
    } else {
        global_extension_dirs()
    };
    if !args.no_extensions {
        extension_dirs.extend(args.extensions_dir.iter().cloned());
    }

    let skill_base_dirs = if args.no_skills {
        Vec::new()
    } else if args.dev_local_only || args.project_only {
        project_skill_dirs(cwd)
    } else if project_trusted {
        skill_dirs(cwd)
    } else {
        global_skill_dirs()
    };

    let prompt_base_dirs = if args.no_prompt_templates {
        Vec::new()
    } else if args.dev_local_only || args.project_only {
        project_prompt_template_dirs(cwd)
    } else if project_trusted {
        prompt_template_dirs(cwd)
    } else {
        global_prompt_template_dirs()
    };

    before_verify();
    let settings_after = load_reload_settings_snapshot(args, cwd, project_trusted)?;
    if settings_before != settings_after {
        return Err(
            "settings changed while reload inputs were being prepared; retry /reload".to_string(),
        );
    }

    Ok(PreparedReloadInputs {
        extension_dirs,
        skill_base_dirs,
        prompt_base_dirs,
    })
}

/// `build_models_with_extensions` for the reload path: the resolved provider
/// runtimes (NOT `resolved` — the reload context carries the `Arc<dyn Provider>`
/// list directly, since provider/auth did not change) first, then one
/// `PluggableProvider` per registered extension provider in the fresh session.
fn build_models_with_extensions_for_reload(
    providers: &[Arc<dyn Provider>],
    extension_session: &ExtensionSession,
    runtime: tokio::runtime::Handle,
) -> Vec<Arc<dyn Provider>> {
    let mut models: Vec<Arc<dyn Provider>> = providers.to_vec();
    let pluggable = ice_extensions::PluggableProvider::from_session(extension_session, runtime);
    for provider in pluggable {
        // Declared packages must enter through credential-bound factories.
        // Falling back to a raw adapter would reintroduce packages rejected by
        // auth filtering and would discard their configured model snapshot.
        if extension_session
            .snapshot()
            .is_some_and(|snapshot| snapshot.provider_manifest(provider.id()).is_some())
        {
            continue;
        }
        if !models.iter().any(|known| known.id() == provider.id()) {
            models.push(provider);
        }
    }
    models
}

/// Diagnostic for registered TUI renderers. All three renderer kinds are
/// consumed by the interactive TUI's JSON component adapter; this line remains
/// useful under `--verbose` for extension authors.
fn report_deferred_renderers(session: &ExtensionSession) {
    let Some(snap) = session.snapshot_arc() else {
        return;
    };
    let all = snap.renderers();
    let markdown = all
        .iter()
        .filter(|r| r.kind == ice_extensions::RegisteredRendererKind::Markdown)
        .count();
    let message = all
        .iter()
        .filter(|r| r.kind == ice_extensions::RegisteredRendererKind::Message)
        .count();
    let entry = all
        .iter()
        .filter(|r| r.kind == ice_extensions::RegisteredRendererKind::Entry)
        .count();
    if markdown + message + entry == 0 {
        return;
    }
    eprintln!(
        "renderers: {} markdown-transform, {} message-render, {} entry-render (active)",
        markdown, message, entry
    );
}

/// A harness-build error.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("Could not create the session directory: {0}")]
    SessionDir(String),
    #[error("Could not open the saved session: {0}")]
    SessionOpen(String),
    #[error(
        "No session found for {requested} in {dir}. Start a fresh session instead (drop --continue/--resume/--session)."
    )]
    SessionNotFound { requested: String, dir: String },
    #[error("Could not build the harness: {0}")]
    HarnessCreate(String),
}

/// Keep configured factories (including package-specific auth and catalogs)
/// first, then append legacy plugin registrations that have no runtime yet.
fn build_models_with_extensions(
    resolved: &ResolvedModel,
    extension_session: &ExtensionSession,
    runtime: tokio::runtime::Handle,
) -> Vec<Arc<dyn Provider>> {
    build_models_with_extensions_for_reload(&resolved.providers, extension_session, runtime)
}

/// Resolve project resource loading without prompting. Explicit CLI overrides
/// win, then a stored decision; projects with no decision load by default.
pub(crate) fn resolve_project_trust(args: &Args, cwd: &Path) -> bool {
    if let Some(override_value) = args.trust_override {
        return override_value;
    }
    crate::config::project_trust_decision(cwd)
        .ok()
        .flatten()
        .unwrap_or(true)
}

/// Resolve the extension dirs to scan and load the cdylib plugins, returning
/// the loaded session guard (keeps the `Library` handles alive for the harness
/// lifetime). Scan order: configured project paths, project `.icers/extensions`,
/// configured/global conventional paths, then any
/// `--extensions-dir` flags (scanned after the defaults — `args.rs`).
/// Diagnostics are a no-op sink for now; load skips/ABI mismatches surface via
/// the `--verbose` summary.
pub(crate) fn load_extensions(
    args: &Args,
    cwd: &Path,
    project_trusted: bool,
    action_bridge: Option<Arc<ice_extensions::ActionBridge>>,
) -> ExtensionSession {
    if args.no_extensions {
        return ExtensionSession::none();
    }
    let mut dirs = if args.dev_local_only {
        Vec::new()
    } else if args.project_only {
        project_extension_dirs(cwd)
    } else if project_trusted {
        extension_dirs(cwd)
    } else {
        global_extension_dirs()
    };
    dirs.extend(args.extensions_dir.iter().cloned());
    load_extensions_from_dirs(args, &dirs, action_bridge)
}

fn load_extensions_from_dirs(
    args: &Args,
    dirs: &[PathBuf],
    action_bridge: Option<Arc<ice_extensions::ActionBridge>>,
) -> ExtensionSession {
    let diagnostics: Arc<dyn PluginDiagnostics> = Arc::new(NullDiagnostics);
    // B5a: the action bridge is cloned into every loaded plugin's vtable
    // `user_data` so post-register `runtime_action` calls recover the harness
    // host from any thread. The call site already gates `load_extensions` behind
    // `!no_extensions` and threads `Some(bridge)`; `None` is only passed by the
    // `--no-extensions` branch (which calls `ExtensionSession::none()` directly)
    // and tests. Explicit `--extension`/`-e` files load after the dirs.
    ice_extensions::load_session_mixed(dirs, &args.extension, diagnostics, action_bridge)
}

/// Merge the loaded extension tools into the built-in set. An extension tool
/// overrides a same-named built-in; first-extension-wins across plugins is
/// already guaranteed by the registry (`register_tool` keeps the prior). The
/// explicit `--tools` allowlist / `--exclude-tools` denylist apply to the
/// merged set (the built-ins were already filtered in [`build_tools`]).
fn merge_extension_tools(
    tools: &mut Vec<HarnessTool>,
    session: &ExtensionSession,
    args: &Args,
    tool_context: &ice_extensions::ToolCallContext,
) {
    let Some(snapshot) = session.snapshot() else {
        return;
    };
    for et in snapshot.tools() {
        let name = &et.tool.name;
        if !tool_name_allowed(name, args) {
            continue;
        }
        let adapter = PluginToolAdapter::new(et.tool.clone(), et.handle(), session.keepalive())
            .with_context(tool_context.clone());
        let harness_tool = HarnessTool::new(Arc::new(adapter));
        match tools.iter_mut().find(|t| t.tool.schema().name == *name) {
            Some(slot) => *slot = harness_tool,
            None => tools.push(harness_tool),
        }
    }
}

/// Build the tool list per `--tools`/`--exclude-tools`/`--no-tools`/
/// `--no-builtin-tools`. Mirrors the TS `tools`/`excludeTools`/`noTools`
/// resolution in `createAgentSession`.
/// Default bash timeout: 120s for a foreground call when the model doesn't
/// pass one (prevents a forgotten `timeout` from hanging the run forever — the
/// "stuck" report). With background shells the call then moves to the
/// background instead of being stopped; background shells are not subject to
/// it. `ICERS_BASH_TIMEOUT` overrides; a model-supplied timeout always wins.
pub fn bash_options(
    shells: Option<&crate::background_shells::SessionShells>,
) -> ice_tools::tools::bash::BashToolOptions {
    use ice_tools::tools::bash::BashToolOptions;
    let default = std::env::var("ICERS_BASH_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(120.0);
    // Saved user defaults, matching how `defaultTools` is read in `build_tools`.
    // `shellCommandPrefix` is prepended to every command (upstream's key for
    // alias/`shopt` setup). Native's `shellPath` is *not* wired: neither
    // `BashToolOptions` nor `ShellCaptureOptions` carries a shell binary today, so
    // honouring it means plumbing one through pi-tools first.
    let settings = crate::settings::load_settings().unwrap_or_default();
    BashToolOptions {
        command_prefix: settings.shell_command_prefix.clone(),
        default_timeout: Some(default),
        background: shells.map(|s| s.registry()),
        auto_background_secs: shells.and_then(|s| s.auto_background_secs()),
        steer_probe: shells.map(|s| s.steer_probe()),
        ..Default::default()
    }
}

fn build_tools(
    ctx: &ExecutionToolContext,
    args: &Args,
    shells: Option<&crate::background_shells::SessionShells>,
) -> Vec<HarnessTool> {
    if args.no_tools {
        return Vec::new();
    }
    // Keep the coding tools aligned with Pi and expose icers's read-only docs
    // lookup as a default assistant capability.
    //
    // Replay policy (native `ToolDeclaration.replay`): only tools that are
    // genuinely free of side effects may be re-run when a crash leaves their
    // outcome unknown — everything else must stay `Never`, because replaying
    // `bash`/`write`/`edit` would duplicate a side effect. `HarnessTool::new`
    // already defaults to `Never`, so this is opt-in per read-only tool.
    let mut all: Vec<(&'static str, HarnessTool)> = vec![
        (
            "read",
            HarnessTool::new(create_read_tool(
                ctx,
                Some(ice_tools::tools::read::ReadToolOptions {
                    image_processor: Some(Arc::new(
                        ice_tools::image_processing::InlineImageProcessor,
                    )),
                    ..Default::default()
                }),
            ))
            .with_replay(ToolReplay::Safe),
        ),
        (
            "bash",
            HarnessTool::new(create_bash_tool(ctx, Some(bash_options(shells)))),
        ),
        ("edit", HarnessTool::new(create_edit_tool(ctx))),
        ("write", HarnessTool::new(create_write_tool(ctx))),
        (
            "docs",
            HarnessTool::new(create_docs_tool()).with_replay(ToolReplay::Safe),
        ),
    ];

    // Add PowerShell tool on Windows
    #[cfg(target_os = "windows")]
    all.push(("powershell", HarnessTool::new(create_powershell_tool(ctx))));

    // Apply default_tools from settings (if no CLI override)
    if args.tools.is_none() {
        if let Ok(settings) = crate::settings::load_settings() {
            if let Some(default_tools) = &settings.default_tools {
                all.retain(|(name, _)| default_tools.iter().any(|t| t == name));
            }
        }
    }

    // `--no-builtin-tools` disables the built-in set but would keep
    // extension/custom tools — v1 has none, so it's equivalent to `--no-tools`
    // here. We honor it by clearing the built-ins.
    if args.no_builtin_tools {
        all.clear();
    }

    // Allowlist (`--tools`): keep only named built-ins.
    if let Some(allow) = &args.tools {
        all.retain(|(name, _)| allow.iter().any(|a| a == name));
    }
    // The background-shell controls come with bash (they are not in the
    // seven-name built-in set, as in ice); the denylist still applies.
    if let Some(shells) = shells {
        if all.iter().any(|(name, _)| *name == "bash") {
            all.extend(shells.tools());
        }
    }
    // Denylist (`--exclude-tools`): drop named tools.
    if let Some(deny) = &args.exclude_tools {
        all.retain(|(name, _)| !deny.iter().any(|d| d == name));
    }

    all.into_iter().map(|(_, t)| t).collect()
}

/// Resolve the active tool names from the constructed tools when no explicit
/// `--tools` allowlist was given. Mirrors the TS default: all registered tools
/// active.
fn active_tool_names(tools: &[HarnessTool], args: &Args) -> Vec<String> {
    filter_active_tool_names(
        tools.iter().map(|tool| tool.tool.schema().name.clone()),
        args,
    )
}

/// Whether a tool name survives the command-line tool policy. Keep this check
/// centralized so extension-provided tools honor the same policy as built-ins.
pub(crate) fn tool_name_allowed(name: &str, args: &Args) -> bool {
    if args.no_tools {
        return false;
    }
    if args
        .tools
        .as_ref()
        .is_some_and(|allow| !allow.iter().any(|value| value == name))
    {
        return false;
    }
    if args
        .exclude_tools
        .as_ref()
        .is_some_and(|deny| deny.iter().any(|value| value == name))
    {
        return false;
    }
    true
}

pub(crate) fn filter_active_tool_names<I>(names: I, args: &Args) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    names
        .into_iter()
        .filter(|name| tool_name_allowed(name, args))
        .collect()
}

/// Build the `Session` facade for the chosen selection.
async fn build_session(selection: &SessionSelection, cwd: &str) -> Result<Session, BuildError> {
    match selection {
        SessionSelection::Ephemeral => Ok(ephemeral_session()),
        SessionSelection::New { dir, .. } => {
            // Ensure the sessions directory exists, then create a fresh JSONL
            // session file inside it.
            std::fs::create_dir_all(dir)
                .map_err(|e| BuildError::SessionDir(format!("{}: {e}", dir.display())))?;
            let session = create_jsonl_session(dir, cwd)
                .await
                .map_err(|e| BuildError::SessionDir(format!("{}: {e}", dir.display())))?;
            Ok(session)
        }
        SessionSelection::Latest { .. }
        | SessionSelection::ById { .. }
        | SessionSelection::ByExactId { .. } => restore_session(selection, cwd).await,
        SessionSelection::Fork { source, dir } => fork_session_at_launch(source, dir, cwd).await,
    }
}

/// Open an existing JSONL session for `Latest` / `ById`. Mirrors the TS
/// `SessionManager.resume`/`open` flow: list the session dir (newest-first),
/// match the request, then open the matched file and wrap it in a `Session`
/// facade. The restored transcript renders into the TUI at startup and the
/// harness continues appending to the same file.
async fn restore_session(selection: &SessionSelection, cwd: &str) -> Result<Session, BuildError> {
    // `list_typed` is newest-first; `Latest` takes the head, `ById` matches
    // the id exactly or by file-name containment (so `--session 01a02…` or a
    // partial id works, mirroring the TS id/path matching).
    match selection {
        SessionSelection::Latest { dir } => {
            let metas = list_session_metadata_in_dir(cwd, dir).await?;
            let Some(meta) = first_resumable_session(&metas) else {
                return Err(BuildError::SessionNotFound {
                    requested: "the most recent session".to_string(),
                    dir: session_dir(Path::new(cwd)).display().to_string(),
                });
            };
            open_session(meta, cwd).await
        }
        SessionSelection::ById { id, dir } => open_session_by_id_in_dir(id, cwd, dir)
            .await
            .map_err(|e| match e {
                OpenError::NotFound { requested } => BuildError::SessionNotFound {
                    requested,
                    dir: session_dir(Path::new(cwd)).display().to_string(),
                },
                OpenError::Other(msg) => BuildError::SessionOpen(msg),
            }),
        SessionSelection::ByExactId { id, dir } => {
            // Exact id match only (pi `--session-id`): restore when the
            // session exists, else create a fresh one under the selected dir.
            let metas = list_session_metadata_in_dir(cwd, dir).await?;
            if let Some(meta) = metas.iter().find(|m| m.id == *id) {
                return open_session(meta, cwd).await;
            }
            std::fs::create_dir_all(dir)
                .map_err(|e| BuildError::SessionDir(format!("{}: {e}", dir.display())))?;
            create_jsonl_session_with_id(dir, cwd, Some(id.clone()))
                .await
                .map_err(|e| BuildError::SessionDir(format!("{}: {e}", dir.display())))
        }
        _ => unreachable!("restore_session only called for Latest/ById/ByExactId"),
    }
}

/// `--fork <path|id>`: open the source session, fork it into a new JSONL
/// session (records the parent id), and start in the fork.
async fn fork_session_at_launch(
    source: &str,
    dir: &Path,
    cwd: &str,
) -> Result<Session, BuildError> {
    use ice_harness::session::jsonl::{
        JsonlSessionCreateOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
    };
    use ice_harness::session::types::{ForkOptions, SessionStorage};
    use ice_tools::FileSystem;

    let dir = dir.to_path_buf();
    std::fs::create_dir_all(&dir)
        .map_err(|e| BuildError::SessionDir(format!("{}: {e}", dir.display())))?;
    let env = Arc::new(OsExecutionEnv::with_cwd(PathBuf::from(cwd)));
    let fs: Arc<dyn FileSystem> = env.clone();
    let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
        fs: fs.clone(),
        sessions_root: dir.to_string_lossy().into_owned(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(DefaultIdGenerator::new()),
    });
    let metas = repo
        .list_typed(&ice_harness::session::jsonl::JsonlSessionListOptions::default())
        .await
        .map_err(|e| BuildError::SessionDir(format!("list sessions: {e}")))?;
    let source_meta = metas
        .iter()
        .find(|m| m.id == *source || m.path.contains(source) || source.contains(&m.id))
        .ok_or_else(|| BuildError::SessionNotFound {
            requested: format!("--fork {source}"),
            dir: dir.display().to_string(),
        })?;
    let fork_storage = repo
        .fork_typed(
            source_meta,
            &JsonlSessionCreateOptions {
                id: None,
                parent_session_id: Some(source_meta.id.clone()),
                cwd: cwd.to_string(),
                metadata: None,
            },
            &ForkOptions::default(),
        )
        .await
        .map_err(|e| BuildError::SessionDir(format!("fork {}: {e}", source_meta.path)))?;
    let storage_arc: Arc<dyn SessionStorage> = Arc::new(fork_storage);
    Ok(Session::new(storage_arc, None))
}

/// Errors from [`open_session_by_id`], split so the CLI can map them to
/// [`BuildError`] while the TUI can surface a friendlier note.
#[derive(Debug)]
pub enum OpenError {
    /// No session matched the request.
    NotFound { requested: String },
    /// The match existed but could not be opened/parsed.
    Other(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NotFound { requested } => write!(f, "no session matches {requested}"),
            // `Other` carries the bare reason (see `open_session_by_id`); the
            // prefix lives here so the TUI and the CLI's
            // `BuildError::SessionOpen` wrapper each render it exactly once
            // (wrapping the already-prefixed `BuildError` string doubled it).
            OpenError::Other(msg) => write!(f, "Could not open the saved session: {msg}"),
        }
    }
}

/// List the JSONL session metadata under the default session dir, newest
/// first. Shared by startup restore and the TUI `/session` hot-switch.
pub async fn list_session_metadata(
    cwd: &str,
) -> Result<Vec<ice_harness::session::jsonl::JsonlSessionMetadata>, BuildError> {
    let dir = session_dir(Path::new(cwd));
    list_session_metadata_in_dir(cwd, &dir).await
}

async fn list_session_metadata_in_dir(
    cwd: &str,
    dir: &Path,
) -> Result<Vec<ice_harness::session::jsonl::JsonlSessionMetadata>, BuildError> {
    Ok(discover_session_metadata_in_dir(cwd, dir).await?.sessions)
}

pub(crate) async fn discover_session_metadata(
    cwd: &str,
) -> Result<ice_harness::session::jsonl::repo::JsonlSessionDiscovery, BuildError> {
    discover_session_metadata_in_dir(cwd, &session_dir(Path::new(cwd))).await
}

pub(crate) async fn discover_session_metadata_in_dir(
    cwd: &str,
    dir: &Path,
) -> Result<ice_harness::session::jsonl::repo::JsonlSessionDiscovery, BuildError> {
    use ice_harness::session::jsonl::{
        JsonlSessionListOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
    };
    use ice_tools::FileSystem;

    let env = Arc::new(OsExecutionEnv::with_cwd(PathBuf::from(cwd)));
    let fs: Arc<dyn FileSystem> = env.clone();
    let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
        fs: fs.clone(),
        sessions_root: dir.to_string_lossy().into_owned(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(DefaultIdGenerator::new()),
    });
    repo.list_with_diagnostics(&JsonlSessionListOptions::default())
        .await
        .map_err(|e| BuildError::SessionDir(format!("list sessions: {e}")))
}

/// A cheap, display-oriented summary of one JSONL session file.
///
/// The session pickers (`-r` and `/session`) need enough context to tell saved
/// sessions apart. The header-only [`JsonlSessionMetadata`] carries no name and
/// no transcript, so a list built from it renders as a wall of identical
/// `2026-09-25T15-05-15-847Z_…` file names — including the many header-only
/// sessions an abandoned launch leaves behind. This reads just the head (and a
/// small tail, for a late `--name`/`/name` fact) of the file to recover:
///
/// - the session display name, when one was recorded;
/// - the first user prompt, single-lined and truncated, for a readable label;
/// - whether the file is header-only (nothing to resume).
///
/// It intentionally does **not** parse the whole log: sessions can be tens of
/// megabytes, and the picker runs over every session at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionSummary {
    /// The session display name (`--name` / `/name`), if one was recorded.
    pub name: Option<String>,
    /// The first user prompt, single-lined and trimmed to a label-sized length.
    pub preview: Option<String>,
    /// True when the file holds only its header (a session with no content).
    pub empty: bool,
    /// On-disk size in bytes (a rough content-volume hint for the picker).
    pub bytes: u64,
}

/// Bytes read from the head of a session file to recover a name + first prompt.
const SUMMARY_HEAD_BYTES: u64 = 256 * 1024;
/// Bytes read from the tail of a session file to recover a late name fact.
const SUMMARY_TAIL_BYTES: u64 = 8 * 1024;
/// Longest preview retained (characters, not bytes).
const SUMMARY_PREVIEW_CHARS: usize = 80;

/// Summarize one session file for the pickers. Best-effort: an unreadable or
/// malformed file yields a summary with `empty: false` and no label, so it is
/// still selectable rather than silently dropped.
pub fn summarize_session_file(path: &Path) -> SessionSummary {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut file) = std::fs::File::open(path) else {
        return SessionSummary::default();
    };
    let Ok(size) = file.metadata().map(|m| m.len()) else {
        return SessionSummary::default();
    };
    if size == 0 {
        return SessionSummary {
            name: None,
            preview: None,
            empty: true,
            bytes: 0,
        };
    }

    // Head: the header line (for the empty check) + a name fact + the first user
    // prompt. The whole window is scanned; it is already in memory.
    let mut head = vec![0u8; SUMMARY_HEAD_BYTES.min(size) as usize];
    let head_len = file.read(&mut head).unwrap_or(0);
    head.truncate(head_len);
    let head = String::from_utf8_lossy(&head);
    let mut lines: Vec<&str> = head.lines().collect();
    let header_len = lines.first().map(|line| line.len()).unwrap_or(0);
    // A header-only file is exactly the header plus an optional trailing `\n`.
    let empty = (size as usize) <= header_len + 1;
    // When the window stopped short of the file end, its final line is partial:
    // drop it so a half-read message can never be mistaken for a complete one.
    if (head_len as u64) < size && !lines.is_empty() {
        lines.pop();
    }

    let mut summary = SessionSummary {
        name: None,
        preview: None,
        empty,
        bytes: size,
    };
    for line in lines {
        apply_summary_line(line, &mut summary);
    }

    // Tail: a name set late in the session lives after the head window. Read the
    // last window and scan only complete lines so a torn/partial first line is
    // ignored. Only do this when the file is larger than the head window.
    if size > SUMMARY_HEAD_BYTES
        && file
            .seek(SeekFrom::End(-(SUMMARY_TAIL_BYTES as i64)))
            .is_ok()
    {
        let mut tail = Vec::new();
        if file.read_to_end(&mut tail).is_ok() {
            let tail = String::from_utf8_lossy(&tail);
            let mut tail_lines = tail.lines();
            // Drop the possibly-partial first line of the window.
            tail_lines.next();
            for line in tail_lines {
                if let Some(name) = name_from_line(line) {
                    summary.name = name;
                }
            }
        }
    }
    summary
}

/// Extract the name from a `{"kind":"fact","fact":"name","name":…}` line.
/// Returns `Some(None)` for an explicit `name: null` (the fact was cleared).
fn name_from_line(line: &str) -> Option<Option<String>> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("kind").and_then(|v| v.as_str()) != Some("fact") {
        return None;
    }
    if value.get("fact").and_then(|v| v.as_str()) != Some("name") {
        return None;
    }
    Some(match value.get("name") {
        Some(serde_json::Value::String(name)) => Some(name.clone()),
        _ => None,
    })
}

/// Fold one JSONL line into `summary`: record a name fact, or capture the first
/// user prompt as the preview.
fn apply_summary_line(line: &str, summary: &mut SessionSummary) {
    if let Some(name) = name_from_line(line) {
        summary.name = name;
        return;
    }
    if summary.preview.is_some() {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    if value.get("kind").and_then(|v| v.as_str()) != Some("entry") {
        return;
    }
    if value.get("type").and_then(|v| v.as_str()) != Some("message") {
        return;
    }
    let Some(message) = value.get("message") else {
        return;
    };
    if message.get("role").and_then(|v| v.as_str()) != Some("user") {
        return;
    }
    let text = match message.get("content") {
        Some(serde_json::Value::String(text)) => Some(text.as_str()),
        Some(serde_json::Value::Array(blocks)) => blocks.iter().find_map(|block| {
            (block.get("type").and_then(|v| v.as_str()) == Some("text"))
                .then(|| block.get("text").and_then(|v| v.as_str()))
                .flatten()
        }),
        _ => None,
    };
    let Some(text) = text else { return };
    let collapsed = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.is_empty() {
        return;
    }
    summary.preview = Some(collapsed.chars().take(SUMMARY_PREVIEW_CHARS).collect());
}

/// The human label for a session row: its recorded name, else the first user
/// prompt, else a marker that says the file has no transcript to show. Shared
/// by the startup picker and the in-TUI `/session` selector.
pub fn session_display_label(summary: &SessionSummary) -> String {
    if let Some(name) = summary.name.as_deref().map(str::trim) {
        if !name.is_empty() {
            return name.to_string();
        }
    }
    match summary.preview.as_deref().map(str::trim) {
        Some(preview) if !preview.is_empty() => preview.to_string(),
        _ => "(no messages)".to_string(),
    }
}

/// Compact byte count (1.2MB / 34KB / 512B), shared by the startup picker and
/// the in-TUI `/session` selector.
pub fn format_session_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}

/// The first 8 characters of a session id, enough to disambiguate two rows that
/// share a timestamp without shouting the whole UUID.
pub fn short_session_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

/// Synchronously list every JSONL session file under `dir`, newest-first by
/// modification time.
///
/// The repo's [`list_session_metadata`] is async and scans one cwd-encoded
/// subdirectory level; the in-TUI `/session` selector runs on the blocking key
/// thread and cannot await it, so this walks the same layout directly. Both
/// `.jsonl` files directly under `dir` and one level of subdirectories are
/// accepted.
pub fn list_session_files_sync(dir: &Path) -> Vec<PathBuf> {
    discover_session_files_sync(dir).0
}

/// Header validation for the blocking in-TUI picker, with the same diagnostics
/// policy as repository discovery. Reads one line, never the whole transcript.
pub(crate) fn discover_session_files_sync(dir: &Path) -> (Vec<PathBuf>, Vec<String>) {
    use std::io::BufRead;

    let mut skipped = Vec::new();
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    let mut visit = |path: PathBuf| {
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            return;
        }
        let header = std::fs::File::open(&path)
            .and_then(|file| {
                std::io::BufReader::new(file)
                    .lines()
                    .next()
                    .unwrap_or_else(|| {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "empty header",
                        ))
                    })
            })
            .map_err(|error| format!("unreadable header ({error})"))
            .and_then(|line| {
                ice_harness::session::jsonl::parse_header(&line)
                    .map_err(|error| format!("malformed header ({error})"))
            });
        if let Err(error) = header {
            skipped.push(format!("{path:?}: {error}"));
            return;
        }
        let modified = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        files.push((path, modified));
    };
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(inner) = std::fs::read_dir(&path) {
                    for child in inner.flatten() {
                        visit(child.path());
                    }
                }
            } else {
                visit(path);
            }
        }
    }
    // Newest first; the path breaks mtime ties so the order is deterministic.
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    skipped.sort();
    (files.into_iter().map(|(path, _)| path).collect(), skipped)
}

pub(crate) fn session_discovery_warning(skipped: &[String]) -> Option<String> {
    if skipped.is_empty() {
        None
    } else {
        Some(format!(
            "Skipped {} session file(s) with unreadable or malformed headers:\n{}",
            skipped.len(),
            skipped.join("\n")
        ))
    }
}

#[cfg(test)]
#[tokio::test]
async fn discovery_keeps_good_sessions_and_reports_malformed_headers() {
    use ice_harness::session::jsonl::{encode_header, JsonlV4Header};

    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("--project--");
    std::fs::create_dir(&directory).unwrap();
    let good = directory.join("good.jsonl");
    let bad = directory.join("malformed.jsonl");
    let header = JsonlV4Header::new("good".into(), 0, "/project".into(), None, None, None);
    std::fs::write(&good, encode_header(&header)).unwrap();
    std::fs::write(&bad, "not a session header\n").unwrap();

    assert_eq!(list_session_files_sync(root.path()), vec![good.clone()]);
    let (files, skipped) = discover_session_files_sync(root.path());
    assert_eq!(files, vec![good]);
    let warning = session_discovery_warning(&skipped).unwrap();
    assert!(warning.contains("Skipped 1 session file(s)"), "{warning}");
    assert!(warning.contains("malformed.jsonl"), "{warning}");
    let discovery = discover_session_metadata_in_dir("/project", root.path())
        .await
        .unwrap();
    assert_eq!(discovery.sessions.len(), 1);
    let warning = session_discovery_warning(&discovery.skipped).unwrap();
    assert!(warning.contains("Skipped 1 session file(s)"), "{warning}");
    assert!(warning.contains("malformed.jsonl"), "{warning}");
    assert_eq!(
        list_session_metadata_in_dir("/project", root.path())
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Pick the session `--continue`/`-c` should open: the newest session that has
/// content (a header-only session resumes to an empty transcript). Falls back to
/// the newest overall when every session is empty, and `None` only when the
/// list is empty.
fn first_resumable_session<'a>(
    metas: &'a [ice_harness::session::jsonl::JsonlSessionMetadata],
) -> Option<&'a ice_harness::session::jsonl::JsonlSessionMetadata> {
    metas
        .iter()
        .find(|meta| !summarize_session_file(Path::new(&meta.path)).empty)
        .or_else(|| metas.first())
}

/// Open a session whose id matches exactly or by file-name containment
/// (so `--session 01a02…` / a partial id / a full file name all work). The
/// TUI `/session` hot-switch calls this with the selector's item value.
pub async fn open_session_by_id(id: &str, cwd: &str) -> Result<Session, OpenError> {
    let requested = Path::new(id);
    if requested.is_absolute() || requested.components().count() > 1 {
        let target = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            Path::new(cwd).join(requested)
        };
        let target = target.canonicalize().map_err(|_| OpenError::NotFound {
            requested: format!("session {id}"),
        })?;
        // An exact file can sit directly in a custom directory. Repository
        // listing treats its root as a parent of cwd-encoded directories, so
        // scanning that parent would miss the selected file.
        use ice_harness::session::jsonl::{metadata_from_header, parse_header};
        use std::io::BufRead;
        let file =
            std::fs::File::open(&target).map_err(|error| OpenError::Other(error.to_string()))?;
        let modified_at = file
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or_default();
        let mut header_line = String::new();
        std::io::BufReader::new(std::io::Read::take(file, 64 * 1024))
            .read_line(&mut header_line)
            .map_err(|error| OpenError::Other(error.to_string()))?;
        let header = parse_header(header_line.trim_end())
            .map_err(|error| OpenError::Other(error.to_string()))?;
        let path = target
            .to_str()
            .ok_or_else(|| OpenError::Other("The session path is not valid UTF-8.".into()))?;
        let meta = metadata_from_header(&header, path, modified_at);
        return open_session(&meta, cwd)
            .await
            .map_err(|error| OpenError::Other(error.to_string()));
    }
    let dir = session_dir(Path::new(cwd));
    open_session_by_id_in_dir(id, cwd, &dir).await
}

async fn open_session_by_id_in_dir(id: &str, cwd: &str, dir: &Path) -> Result<Session, OpenError> {
    let metas = list_session_metadata_in_dir(cwd, dir)
        .await
        .map_err(|e| OpenError::Other(e.to_string()))?;
    let Some(meta) = metas
        .iter()
        .find(|m| m.id == id || m.path.contains(id) || id.contains(&m.id))
    else {
        return Err(OpenError::NotFound {
            requested: format!("session {id}"),
        });
    };
    open_session(meta, cwd).await.map_err(|e| {
        OpenError::Other(match e {
            // `open_session` renders as "Could not open the saved session:
            // <reason>" via `BuildError`. Carry the bare reason so it is not
            // prefixed twice (once here, once by the CLI/TUI).
            BuildError::SessionOpen(reason) => reason,
            other => other.to_string(),
        })
    })
}

/// Publish the active session's id to plugins, on both channels that carry it.
///
/// Called on **every** activation of a session — the startup build AND every
/// in-process swap (`switch_to_session`, `fork_session`, and the plugin
/// `NewSession`/`Fork`/`SwitchSession` runtime actions all route through
/// [`set_active_session`]). Two things must stay in step:
///
/// - `ToolCallContext` — read per plugin **tool** call, which is why a stale
///   value there made a plugin's `todo`-style state follow the wrong session.
/// - `ICERS_SESSION_ID` — read per **turn** by the provider hooks, which is the
///   only channel an observer gets for a pure-chat run. Process-global, which is
///   exactly the scope of "the session this process is serving".
///
/// The write overwrites unconditionally, deliberately. An inherited value is not
/// an override: `icers` spawning `icers` would otherwise hand the child the parent's
/// session id, and the child — which is *serving its own* session — would report
/// it forever. (Children no longer inherit the variable at all; see
/// `ice_tools::HOST_PRIVATE_ENV`. This overwrite is the second line of defence,
/// for any other way a value can arrive pre-set.)
///
/// An embedder that genuinely wants to pin the id can revert to the old
/// behaviour by setting the variable *after* this runs; there is no supported
/// way to pin it before, because a session that does not match its own id is
/// never what the caller wants.
fn publish_session_identity(tool_context: &ice_extensions::ToolCallContext, id: &str) {
    std::env::set_var(ice_plugin_sdk::SESSION_ID_ENV, id);
    tool_context.set_session_id(id);
}

/// Swap the session the harness serves, republishing the identity to plugins.
///
/// The single funnel for an in-process session change. Prefer this over calling
/// `AgentHarness::set_session` directly: the harness holds no reference to the
/// plugin channels, so a bare `set_session` would leave every plugin reporting
/// the session it had before the swap.
///
/// `tool_context` is optional because not every caller has one (the harness's own
/// tests, a headless embedder); the env channel is always published, since the
/// provider hooks depend on it.
pub(crate) async fn set_active_session(
    harness: &AgentHarness,
    session: Session,
    tool_context: Option<&ice_extensions::ToolCallContext>,
) -> Result<(), String> {
    let metadata = session.get_metadata().await.ok();
    harness
        .set_session(session)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(metadata) = metadata {
        std::env::set_var(ice_plugin_sdk::SESSION_ID_ENV, &metadata.id);
        if let Some(context) = tool_context {
            context.set_session_id(metadata.id);
        }
    }
    Ok(())
}

/// Start a fresh durable conversation after the previous operation settles.
///
/// The old backing remains active if creation or activation fails. The harness
/// keeps its model/tool configuration; only conversation state and queues change.
/// Both the TUI and extension new-session action use this transition.
pub(crate) async fn start_new_session(
    harness: &AgentHarness,
    cwd: &Path,
    tool_context: Option<&ice_extensions::ToolCallContext>,
) -> Result<String, String> {
    let lane = harness.lane("main");
    match lane.abort().await {
        Ok(_) | Err(ice_harness::result::HarnessError::NoActiveRun { .. }) => {}
        Err(error) => return Err(error.to_string()),
    }
    lane.wait_for_idle()
        .await
        .map_err(|error| error.to_string())?;
    let directory = session_dir(cwd);
    let session = create_jsonl_session(&directory, &cwd.to_string_lossy()).await?;
    let id = session
        .get_metadata()
        .await
        .map_err(|error| error.to_string())?
        .id;
    set_active_session(harness, session, tool_context).await?;
    Ok(id)
}

/// Fork the harness's current session into a new JSONL session (new id, parent
/// set to the source) and wrap it in a `Session`. Mirrors the TUI's
/// `fork_session` flow (`interactive_tui.rs`) — hoisted here so both the TUI
/// and the plugin `runtime_action(Fork)` host share one implementation.
/// Returns the new `Session` (NOT yet swapped onto the harness — the caller
/// does `harness.set_session(...)`).
pub(crate) async fn fork_session_storage(
    harness: &AgentHarness,
    cwd: &str,
) -> Result<Session, String> {
    fork_session_storage_with_options(
        harness,
        cwd,
        &ice_harness::session::types::ForkOptions::default(),
    )
    .await
}

/// Fork the exact active JSONL file into its current directory, preserving
/// custom locations and the normal exclusive lock for the new session.
pub(crate) async fn fork_session_storage_with_options(
    harness: &AgentHarness,
    cwd: &str,
    options: &ice_harness::session::types::ForkOptions,
) -> Result<Session, String> {
    use ice_harness::session::jsonl::{JsonlSessionRepo, JsonlSessionRepoOptions};
    use ice_tools::FileSystem;

    if harness
        .snapshot()
        .await
        .map_err(|error| error.to_string())?
        .active
        .is_some()
    {
        return Err("wait for the current run to finish before forking".into());
    }
    let active = harness.session();
    let source_path = active
        .storage()
        .persistence_path()
        .ok_or("current session is not saved to a JSONL file")?;
    let dir = Path::new(&source_path)
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or("current session file has no directory")?;
    let env = Arc::new(OsExecutionEnv::with_cwd(PathBuf::from(cwd)));
    let fs: Arc<dyn FileSystem> = env.clone();
    let source = active.storage();
    let source_id = source.metadata().id;
    let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
        fs,
        sessions_root: dir.to_string_lossy().into_owned(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(DefaultIdGenerator::new()),
    });
    let fork_storage = repo
        .fork_from_storage_in_directory(
            source.as_ref(),
            &dir.to_string_lossy(),
            &ice_harness::session::jsonl::JsonlSessionCreateOptions {
                id: None,
                parent_session_id: Some(source_id),
                cwd: cwd.to_string(),
                metadata: None,
            },
            options,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(Session::new(Arc::new(fork_storage), None))
}

/// Wrap an opened [`JsonlSessionStorage`] in the `Session` facade (shared by
/// startup restore + TUI hot-switch).
async fn open_session(
    meta: &ice_harness::session::jsonl::JsonlSessionMetadata,
    cwd: &str,
) -> Result<Session, BuildError> {
    use ice_harness::session::jsonl::{JsonlSessionRepo, JsonlSessionRepoOptions};
    use ice_harness::session::types::SessionStorage;
    use ice_tools::FileSystem;

    let dir = session_dir(Path::new(cwd));
    let env = Arc::new(OsExecutionEnv::with_cwd(PathBuf::from(cwd)));
    let fs: Arc<dyn FileSystem> = env.clone();
    let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
        fs: fs.clone(),
        sessions_root: dir.to_string_lossy().into_owned(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(DefaultIdGenerator::new()),
    });
    let storage = repo
        .open_by_jsonl_metadata(meta)
        .await
        .map_err(|e| BuildError::SessionOpen(format!("open {}: {e}", meta.path)))?;
    // A session records the cwd it was created in. When that directory has
    // since been removed, continuing silently in it is confusing; warn the
    // user that we fell back to the process cwd (pi `getMissingSessionCwdIssue`).
    if let Some(issue) = crate::session_cwd::get_missing_session_cwd_issue(
        Some(Path::new(&meta.path)),
        Path::new(&meta.cwd),
        Path::new(cwd),
    ) {
        eprintln!(
            "warning: {}",
            crate::session_cwd::format_missing_session_cwd_error(&issue)
        );
    }
    let storage_arc: Arc<dyn SessionStorage> = Arc::new(storage);
    Ok(Session::new(storage_arc, None))
}

/// A fresh ephemeral in-memory session (no persistence). Used for `--no-session`.
fn ephemeral_session() -> Session {
    let storage = Arc::new(InMemorySessionStorage::new(
        SessionMetadata {
            id: "ephemeral".into(),
            created_at: 0,
            parent_session_id: None,
        },
        Arc::new(SystemClock),
        Arc::new(DefaultIdGenerator::new()),
    ));
    Session::new(storage, None)
}

/// Create a fresh JSONL session file under `dir` and wrap it in a `Session`.
///
/// Uses the `JsonlSessionRepo` over an `OsExecutionEnv`-backed `FileSystem`
/// rooted at the cwd, so paths resolve consistently with the tools. Mirrors the
/// TS `SessionManager.create` flow (header write + `JsonlSessionStorage` open).
/// Create a fresh JSONL session file under `dir` and wrap it in a `Session`.
///
/// Uses the `JsonlSessionRepo` over an `OsExecutionEnv`-backed `FileSystem`
/// rooted at the cwd, so paths resolve consistently with the tools. Mirrors the
/// TS `SessionManager.create` flow (header write + `JsonlSessionStorage` open).
pub(crate) async fn create_jsonl_session(dir: &Path, cwd: &str) -> Result<Session, String> {
    create_jsonl_session_with_id(dir, cwd, None).await
}

/// `create_jsonl_session` with an explicit id (the `--session-id` fixed-id
/// contract: the file is named with the given id so later `--session-id`
/// launches restore the same session).
pub(crate) async fn create_jsonl_session_with_id(
    dir: &Path,
    cwd: &str,
    id: Option<String>,
) -> Result<Session, String> {
    create_jsonl_session_at(dir, cwd, id, false).await
}

/// Create beside an active JSONL file, preserving its exact directory instead
/// of treating that directory as a root for another cwd-encoded subdirectory.
pub(crate) async fn create_jsonl_session_in_directory(
    dir: &Path,
    cwd: &str,
) -> Result<Session, String> {
    create_jsonl_session_at(dir, cwd, None, true).await
}

async fn create_jsonl_session_at(
    dir: &Path,
    cwd: &str,
    id: Option<String>,
    exact_directory: bool,
) -> Result<Session, String> {
    use ice_harness::session::jsonl::{
        JsonlSessionCreateOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
    };
    use ice_tools::FileSystem;

    // A dedicated OS env for session-file I/O, rooted at the cwd so the repo's
    // relative-path resolution matches the tool env.
    let env = Arc::new(OsExecutionEnv::with_cwd(PathBuf::from(cwd)));
    let fs: Arc<dyn FileSystem> = env.clone();

    let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
        fs: fs.clone(),
        sessions_root: dir.to_string_lossy().into_owned(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(DefaultIdGenerator::new()),
    });

    let opts = JsonlSessionCreateOptions {
        id, // fresh uuidv7 when None (--session-id passes the fixed id)
        parent_session_id: None,
        cwd: cwd.to_string(),
        metadata: None,
    };
    let storage = if exact_directory {
        repo.create_in_directory(&dir.to_string_lossy(), &opts)
            .await
    } else {
        repo.create_typed(&opts).await
    }
    .map_err(|e| format!("create session: {e}"))?;
    // `JsonlSessionStorage` implements `SessionStorage`; wrap in the facade.
    let storage_arc: Arc<dyn ice_harness::session::types::SessionStorage> = Arc::new(storage);
    Ok(Session::new(storage_arc, None))
}

/// Read an `--append-system-prompt` target: if it's a readable file path, return
/// its contents; otherwise return `None` and let the caller use the literal.
fn read_append_target(target: &str, sources: &mut Vec<String>) -> Option<String> {
    let path = Path::new(target);
    if path.is_file() {
        read_prompt_source(path, sources)
    } else {
        None
    }
}

/// Record provenance at the successful read, rather than rediscovering files
/// during rendering (which could report a shadowed or unreadable file).
fn read_prompt_source(path: &Path, sources: &mut Vec<String>) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let source = absolute.to_string_lossy().into_owned();
    if !sources.contains(&source) {
        sources.push(source);
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_catalog_reload_is_scoped_and_transactional() {
        const CHILD: &str = "ICERS_PROVIDER_CATALOG_RELOAD_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let dir = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "session::tests::provider_catalog_reload_is_scoped_and_transactional",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env(crate::config::CONFIG_DIR_ENV, dir.path())
                .env("ICERS_OFFLINE", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
            let agent = crate::config::agent_dir().unwrap();
            std::fs::create_dir_all(&agent).unwrap();
            let cwd = agent.join("project");
            std::fs::create_dir_all(&cwd).unwrap();
            let path = crate::config::models_path().unwrap();
            let write_config = |second: &str| std::fs::write(&path, format!(r#"{{"providers":{{
                "first":{{"api":"openai-completions","apiKey":"first-key","baseUrl":"https://first.example/v1","models":[{{"id":"one"}}]}},
                "second":{{"api":"openai-completions","apiKey":"second-key","baseUrl":"https://second.example/v1","models":[{{"id":"{second}"}}]}}
            }}}}"#)).unwrap();
            write_config("old");
            std::fs::write(agent.join("settings.json"), r#"{"defaultProvider":"first","defaultModel":"one"}"#).unwrap();
            let args = Args { provider: Some("first".into()), model: Some("one".into()), no_extensions: true, ..Args::default() };
            let resolved = crate::provider::resolve_with_extensions(&args, &cwd, true, &ExtensionSession::none()).unwrap();
            let (harness, _events, ctx) = build(&resolved, &args, &cwd, true).await.unwrap();
            let before = ctx.providers.read().unwrap().iter().find(|p| p.id() == "first").unwrap().clone();
            let mut discovered = ctx.catalog.read().unwrap().iter().find(|m| m.provider == "second").unwrap().clone();
            discovered.id = "remote".into();
            write_config("new");
            let count = reload_provider_catalog(&ctx, &harness, "second", Some(vec![discovered])).await.unwrap();
            assert_eq!(count, 2);
            assert_eq!(harness.lane("main").get_model().await.unwrap().id, "one");
            let providers = ctx.providers.read().unwrap().clone();
            assert!(Arc::ptr_eq(&before, providers.iter().find(|p| p.id() == "first").unwrap()));
            let second = providers.iter().find(|p| p.id() == "second").unwrap();
            assert!(second.models().iter().any(|m| m.id == "new"));
            assert!(second.models().iter().any(|m| m.id == "remote"));
            let catalog = ctx.catalog.read().unwrap().clone();
            let remote = catalog.iter().find(|m| m.provider == "second" && m.id == "remote").unwrap().clone();
            harness.lane("main").set_model(remote).await.unwrap();
            std::fs::write(&path, "{invalid").unwrap();
            assert!(reload_provider_catalog(&ctx, &harness, "second", None).await.is_err());
            assert_eq!(harness.lane("main").get_model().await.unwrap().id, "remote");
            assert!(Arc::ptr_eq(second, ctx.providers.read().unwrap().iter().find(|p| p.id() == "second").unwrap()));
            assert_eq!(catalog.len(), ctx.catalog.read().unwrap().len());
        });
    }

    #[test]
    fn branch_summary_settings_reach_created_harness() {
        const CHILD: &str = "ICERS_BRANCH_SUMMARY_SETTINGS_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let dir = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "session::tests::branch_summary_settings_reach_created_harness",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env(crate::config::CONFIG_DIR_ENV, dir.path())
                .env("ICERS_OFFLINE", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use ice_ai::providers::faux::{FauxProvider, FauxScript};
                let agent = crate::config::agent_dir().unwrap();
                let cwd = agent.join("project");
                std::fs::create_dir_all(&cwd).unwrap();
                // Resolve only to initialize the binding; all harness providers
                // and the selected model are replaced by the in-memory fixture.
                let mut resolved = crate::provider::resolve(
                    Some("anthropic"),
                    Some(crate::provider::DEFAULT_MODEL_ID),
                    None,
                    Some("local-test-placeholder"),
                    None,
                )
                .unwrap();
                let provider = FauxProvider::new(FauxScript::new());
                resolved.model = provider.default_model().clone();
                resolved.provider = provider.clone();
                resolved.providers = vec![provider];
                let args = Args {
                    no_session: true,
                    no_extensions: true,
                    no_skills: true,
                    no_prompt_templates: true,
                    no_tools: true,
                    ..Default::default()
                };
                for (configured, expected) in [(Some(2048), 2048), (Some(0), 1), (None, 16384)] {
                    let settings = match configured {
                        Some(value) => {
                            serde_json::json!({"branchSummary": {"reserveTokens": value}})
                        }
                        None => serde_json::json!({}),
                    };
                    std::fs::write(
                        agent.join("settings.json"),
                        serde_json::to_string(&settings).unwrap(),
                    )
                    .unwrap();
                    let (harness, _events, _context) =
                        build(&resolved, &args, &cwd, false).await.unwrap();
                    assert_eq!(
                        harness.branch_summary_reserve_tokens().await.unwrap(),
                        expected,
                        "configured reserve {configured:?} must reach the created harness"
                    );
                }
            });
    }

    #[tokio::test]
    async fn session_absolute_path_opens_the_selected_project_with_duplicate_ids() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("project-a");
        let b = temp.path().join("project-b");
        let mut selected_path = PathBuf::new();
        for (cwd, label) in [
            (&a, "first project message"),
            (&b, "second project message"),
        ] {
            let directory = cwd.join("saved-sessions");
            std::fs::create_dir_all(&directory).unwrap();
            let session = create_jsonl_session_with_id(
                &directory,
                &cwd.to_string_lossy(),
                Some("same-session-id".into()),
            )
            .await
            .unwrap();
            session
                .append_message(ice_agent::AgentMessage::User(
                    ice_ai::types::UserMessage::new(label, 0),
                ))
                .await
                .unwrap();
            if cwd == &a {
                selected_path = PathBuf::from(session.storage().persistence_path().unwrap());
            }
        }
        // The same ID in the execution cwd must not shadow the selected file.
        let selected = open_session_by_id(&selected_path.to_string_lossy(), &b.to_string_lossy())
            .await
            .unwrap();
        let entries = selected
            .view("main")
            .find_entries(&Default::default())
            .await
            .unwrap();
        let serialized = serde_json::to_value(&entries).unwrap();
        assert_eq!(serialized[0]["message"]["content"], "first project message");
        assert!(matches!(
            open_session_by_id(
                &a.join("missing.jsonl").to_string_lossy(),
                &b.to_string_lossy()
            )
            .await,
            Err(OpenError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn extension_lifecycle_observers_finish_before_publication() {
        struct OrderedEmitter {
            name: &'static str,
            order: Arc<Mutex<Vec<&'static str>>>,
        }
        impl ice_agent::AgentEmitter for OrderedEmitter {
            fn emit(&self, _: ice_agent::AgentEvent) -> futures::future::BoxFuture<'static, ()> {
                let name = self.name;
                let order = Arc::clone(&self.order);
                Box::pin(async move {
                    tokio::task::yield_now().await;
                    order.lock().unwrap().push(name);
                })
            }
        }
        let order = Arc::new(Mutex::new(Vec::new()));
        let public = Arc::new(OrderedEmitter {
            name: "public",
            order: Arc::clone(&order),
        });
        let extension = Arc::new(OrderedEmitter {
            name: "extension",
            order: Arc::clone(&order),
        });
        compose_agent_emitter(public, Some(extension))
            .emit(ice_agent::AgentEvent::AgentStart)
            .await;
        assert_eq!(*order.lock().unwrap(), ["extension", "public"]);
    }

    #[tokio::test]
    async fn unauthenticated_packages_are_not_reintroduced_as_legacy_adapters() {
        extern "C" fn request(
            _: ice_plugin_sdk::StbStringRef,
            _: *mut ice_plugin_sdk::StbString,
            _: *mut std::ffi::c_void,
        ) -> i32 {
            1
        }
        let mut registry = ice_extensions::ExtensionRegistry::new();
        for id in ["package", "legacy"] {
            registry.register_provider(ice_extensions::RegisteredProvider {
                provider_id: id.into(),
                base_url: "http://localhost".into(),
                api_style: "openai-completions".into(),
                request_fn: request,
                stream_fn: None,
                plugin_free_string: ice_plugin_sdk::callbacks::free_string,
                user_data: std::ptr::null_mut(),
            });
        }
        assert!(registry.apply_declaration_json(&serde_json::json!({"providers":[{
            "id":"package", "requiresAuth":true,
            "models":[ice_ai::Model::new("model", "Model", ice_ai::Api::OpenaiCompletions, "package", "http://localhost")]
        }]}).to_string()));
        let session = ExtensionSession::from_parts(
            Arc::new(registry.snapshot()),
            ice_extensions::PluginKeepalive::empty(),
            vec![],
            None,
        );
        let providers = build_models_with_extensions_for_reload(
            &[],
            &session,
            tokio::runtime::Handle::current(),
        );
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.id())
                .collect::<Vec<_>>(),
            ["legacy"]
        );
    }
    use crate::args::Args;

    #[test]
    fn coding_compaction_settings_reach_live_harness() {
        const CHILD: &str = "ICERS_CODING_COMPACTION_SETTINGS_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let dir = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "session::tests::coding_compaction_settings_reach_live_harness",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env(crate::config::CONFIG_DIR_ENV, dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let agent = crate::config::agent_dir().unwrap();
                let cwd = agent.join("project");
                std::fs::create_dir_all(&cwd).unwrap();
                std::fs::write(
                    agent.join("settings.json"),
                    serde_json::json!({
                        "compaction": {
                            "enabled": true, "thresholdPercent": 71, "reserveTokens": 123,
                            "keepRecentTokens": 456, "midRunCompaction": "pause"
                        }
                    })
                    .to_string(),
                )
                .unwrap();
                let resolved = crate::provider::resolve(
                    Some("anthropic"),
                    Some(crate::provider::DEFAULT_MODEL_ID),
                    None,
                    Some("test-key"),
                    None,
                )
                .unwrap();
                let args = Args {
                    no_session: true,
                    no_extensions: true,
                    no_skills: true,
                    no_prompt_templates: true,
                    trust_override: Some(true),
                    ..Default::default()
                };
                let (harness, _, _) = build(&resolved, &args, &cwd, true).await.unwrap();
                let policy = harness
                    .get_coding_compaction_policy()
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(policy.threshold_percent, 71.0);
                assert_eq!(policy.reserve_tokens, Some(123));
                assert_eq!(policy.keep_recent_tokens, 456);
                assert_eq!(
                    policy.mid_run_compaction,
                    ice_harness::compaction::settings::MidRunCompactionMode::Pause
                );
                let context = harness.context_snapshot().await.unwrap();
                assert_eq!(context.compaction.reserve_tokens, 123);
                assert_eq!(context.compaction.keep_recent_tokens, 456);
                assert_eq!(
                    context.compact_at_tokens,
                    Some((context.model.context_window as f64 * 0.71).floor() as i64)
                );
            });
    }

    #[test]
    fn resume_keeps_selected_model_when_historical_provider_is_unavailable() {
        const CHILD: &str = "ICERS_RESUME_MODEL_PRECEDENCE_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let directory = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "session::tests::resume_keeps_selected_model_when_historical_provider_is_unavailable",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env(crate::config::CONFIG_DIR_ENV, directory.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use ice_harness::agent_harness::AgentLane;
                use ice_harness::session::types::{Entry, ProvisionedEntry, ProvisionedKind};

                let agent = crate::config::agent_dir().unwrap();
                let cwd = agent.join("project");
                let sessions = cwd.join("custom-sessions");
                std::fs::create_dir_all(&sessions).unwrap();
                let settings_path = agent.join("settings.json");
                let settings = br#"{"defaultProvider":"unavailable-default","defaultModel":"expensive-default","defaultThinkingLevel":"high","compaction":{"enabled":false}}"#;
                std::fs::write(&settings_path, settings).unwrap();

                // Resolve without making a request, then install only the faux
                // runtime. No historical/default provider can be contacted.
                let mut resolved = crate::provider::resolve(
                    Some("anthropic"),
                    Some(crate::provider::DEFAULT_MODEL_ID),
                    None,
                    Some("test-key"),
                    None,
                ).unwrap();
                let faux = ice_ai::providers::faux::FauxProvider::new(
                    ice_ai::providers::faux::FauxScript::new(),
                );
                resolved.model = faux.default_model().clone();
                resolved.thinking_level = ice_ai::types::ThinkingLevel::Off;
                resolved.provider = faux.clone();
                resolved.providers = vec![faux.clone()];

                for explicit_id in [false, true] {
                    let saved = create_jsonl_session(&sessions, &cwd.to_string_lossy()).await.unwrap();
                    let id = saved.get_metadata().await.unwrap().id;
                    saved.append_message(ice_agent::AgentMessage::User(
                        ice_ai::types::UserMessage::new("SAVED_HISTORY", 1),
                    )).await.unwrap();
                    for (entry_id, kind) in [
                        ("historical-model", ProvisionedKind::ModelChange {
                            provider: "unavailable-historical-provider".into(),
                            model_id: "expensive-historical-model".into(),
                        }),
                        ("historical-thinking", ProvisionedKind::ThinkingLevel {
                            thinking_level: "high".into(),
                        }),
                    ] {
                        saved.append_entry(ProvisionedEntry { id: entry_id.into(), kind }, "main")
                            .await.unwrap();
                    }
                    let metadata = list_session_metadata_in_dir(&cwd.to_string_lossy(), &sessions)
                        .await.unwrap().into_iter().find(|meta| meta.id == id).unwrap();
                    drop(saved);
                    let original = std::fs::read(&metadata.path).unwrap();
                    let args = Args {
                        continue_session: !explicit_id,
                        session: explicit_id.then_some(id.clone()),
                        session_dir: Some(sessions.clone()),
                        no_tools: true,
                        no_extensions: true,
                        no_skills: true,
                        no_prompt_templates: true,
                        no_context_files: true,
                        system_prompt: Some("offline resume regression".into()),
                        ..Default::default()
                    };
                    let (harness, events, reload) = build(&resolved, &args, &cwd, false).await.unwrap();
                    assert_eq!(harness.session().get_metadata().await.unwrap().id, id);
                    assert_eq!(AgentLane::get_model(&harness).await.unwrap(), resolved.model);
                    assert_eq!(AgentLane::get_thinking_level(&harness).await.unwrap(), resolved.thinking_level);
                    let snapshot = harness.context_snapshot().await.unwrap();
                    assert_eq!(snapshot.context.model, Some((
                        "unavailable-historical-provider".into(), "expensive-historical-model".into(),
                    )));
                    assert_eq!(snapshot.context.thinking_level, "high");
                    assert_eq!(std::fs::read(&metadata.path).unwrap(), original);
                    let calls_before = faux.state().call_count.load(std::sync::atomic::Ordering::SeqCst);
                    faux.script().set_responses(vec![ice_ai::providers::faux::FauxStep::text("startup resumed answer")]);
                    let result = harness.prompt_text("CONTINUE_AT_STARTUP", Vec::new()).await.unwrap();
                    assert!(matches!(result.outcome, ice_harness::agent_harness::HarnessRunOutcome::Completed { .. }));
                    assert_eq!(faux.state().call_count.load(std::sync::atomic::Ordering::SeqCst), calls_before + 1);
                    // Restore an unavailable last historical choice before the
                    // hot-switch check; a new assistant must not mask it.
                    harness.session().append_entry(ProvisionedEntry {
                        id: "historical-model-before-switch".into(),
                        kind: ProvisionedKind::ModelChange {
                            provider: "unavailable-historical-provider".into(),
                            model_id: "expensive-historical-model".into(),
                        },
                    }, "main").await.unwrap();

                    // TUI activation follows the same live-choice policy.
                    // Release this harness's writer before reopening the file.
                    drop(snapshot);
                    drop(reload);
                    drop(events);
                    drop(harness);
                    let live = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
                        model: resolved.model.clone(),
                        thinking_level: resolved.thinking_level,
                        models: vec![faux.clone()],
                        ..Default::default()
                    }).await.unwrap();
                    let restored = open_session(&metadata, &cwd.to_string_lossy()).await.unwrap();
                    set_active_session(&live, restored, None).await.unwrap();
                    assert_eq!(AgentLane::get_model(&live).await.unwrap(), resolved.model);
                    assert_eq!(AgentLane::get_thinking_level(&live).await.unwrap(), resolved.thinking_level);
                    assert_eq!(live.context_snapshot().await.unwrap().context.model, Some((
                        "unavailable-historical-provider".into(), "expensive-historical-model".into(),
                    )));
                    let calls_before = faux.state().call_count.load(std::sync::atomic::Ordering::SeqCst);
                    faux.script().set_responses(vec![ice_ai::providers::faux::FauxStep::text("resumed answer")]);
                    let result = live.prompt_text("CONTINUE_SAVED_WORK", Vec::new()).await.unwrap();
                    assert!(matches!(result.outcome, ice_harness::agent_harness::HarnessRunOutcome::Completed { .. }));
                    assert_eq!(faux.state().call_count.load(std::sync::atomic::Ordering::SeqCst), calls_before + 1);
                    let entries = live.session().find_entries(&Default::default()).await.unwrap();
                    assert!(entries.iter().any(|entry| matches!(entry, Entry::ModelChange(change)
                        if change.provider == "unavailable-historical-provider")));
                    assert!(entries.iter().any(|entry| matches!(entry, Entry::Message(message)
                        if matches!(&message.message, ice_agent::AgentMessage::Assistant(assistant)
                            if assistant.provider == resolved.model.provider && assistant.model == resolved.model.id))));
                    assert!(std::fs::read(&metadata.path).unwrap().starts_with(&original));
                    assert_eq!(std::fs::read(&settings_path).unwrap(), settings);
                }
            });
    }

    #[test]
    fn loaded_context_paths_track_precedence_and_reload() {
        const CHILD: &str = "ICERS_CONTEXT_PATHS_REGRESSION_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let dir = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "session::tests::loaded_context_paths_track_precedence_and_reload",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env(crate::config::CONFIG_DIR_ENV, dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let agent = crate::config::agent_dir().unwrap();
                let cwd = agent.join("project");
                let project_config = cwd.join(".icers");
                std::fs::create_dir_all(&project_config).unwrap();
                for (path, content) in [
                    (agent.join("SYSTEM.md"), "GLOBAL_BASE"),
                    (agent.join("APPEND_SYSTEM.md"), "GLOBAL_APPEND"),
                    (agent.join("AGENTS.md"), "GLOBAL_INSTRUCTIONS"),
                    (project_config.join("SYSTEM.md"), "PROJECT_BASE"),
                    (project_config.join("APPEND_SYSTEM.md"), "PROJECT_APPEND"),
                    (cwd.join("AGENTS.md"), "PROJECT_INSTRUCTIONS"),
                    (cwd.join("CLAUDE.md"), "SHADOWED_CLAUDE"),
                ] {
                    std::fs::write(path, content).unwrap();
                }
                let resolved = crate::provider::resolve(
                    Some("anthropic"),
                    Some(crate::provider::DEFAULT_MODEL_ID),
                    None,
                    Some("test-key"),
                    None,
                )
                .unwrap();
                let args = Args {
                    no_session: true,
                    no_extensions: true,
                    no_skills: true,
                    no_prompt_templates: true,
                    trust_override: Some(true),
                    ..Default::default()
                };
                let (harness, _events, mut context) =
                    build(&resolved, &args, &cwd, true).await.unwrap();
                let resources = harness.get_resources().await.unwrap();
                let path = |p: PathBuf| p.to_string_lossy().into_owned();
                assert_eq!(
                    &resources.context_files[..2],
                    &[
                        path(project_config.join("SYSTEM.md")),
                        path(project_config.join("APPEND_SYSTEM.md")),
                    ]
                );
                assert!(resources
                    .context_files
                    .contains(&path(agent.join("AGENTS.md"))));
                assert!(resources
                    .context_files
                    .contains(&path(cwd.join("AGENTS.md"))));
                assert!(!resources
                    .context_files
                    .contains(&path(cwd.join("CLAUDE.md"))));
                assert!(!resources
                    .context_files
                    .contains(&path(agent.join("SYSTEM.md"))));
                let prompt = harness.get_system_prompt().await.unwrap().unwrap();
                assert!(prompt.contains("PROJECT_BASE") && prompt.contains("PROJECT_APPEND"));
                assert!(!prompt.contains("GLOBAL_BASE") && !prompt.contains("SHADOWED_CLAUDE"));

                // Removing the selected project files makes reload fall back to
                // globals; removed instruction paths must disappear as well.
                std::fs::remove_file(project_config.join("SYSTEM.md")).unwrap();
                std::fs::remove_file(project_config.join("APPEND_SYSTEM.md")).unwrap();
                std::fs::remove_file(cwd.join("AGENTS.md")).unwrap();
                assert!(
                    !reload_extension_resources(&harness, &context)
                        .await
                        .had_errors
                );
                let resources = harness.get_resources().await.unwrap();
                assert_eq!(
                    &resources.context_files[..2],
                    &[
                        path(agent.join("SYSTEM.md")),
                        path(agent.join("APPEND_SYSTEM.md")),
                    ]
                );
                assert!(resources
                    .context_files
                    .contains(&path(cwd.join("CLAUDE.md"))));
                assert!(!resources
                    .context_files
                    .contains(&path(project_config.join("SYSTEM.md"))));

                // Explicit append files are listed; literal text and overridden
                // discovered base/append files are not invented as file sources.
                let explicit = agent.join("custom-append.md");
                std::fs::write(&explicit, "EXPLICIT_APPEND").unwrap();
                context.args.system_prompt = Some("EXPLICIT_BASE".into());
                context.args.append_system_prompt =
                    vec![path(explicit.clone()), "literal append".into()];
                context.args.no_context_files = true;
                assert!(
                    !reload_extension_resources(&harness, &context)
                        .await
                        .had_errors
                );
                assert_eq!(
                    harness.get_resources().await.unwrap().context_files,
                    vec![path(explicit)]
                );

                // The reload must honor the startup opt-out from project context.
                context.args.system_prompt = None;
                context.args.append_system_prompt.clear();
                context.args.no_context_files = false;
                context.args.trust_override = Some(false);
                assert!(
                    !reload_extension_resources(&harness, &context)
                        .await
                        .had_errors
                );
                assert!(harness
                    .get_resources()
                    .await
                    .unwrap()
                    .context_files
                    .is_empty());
                let prompt = harness.get_system_prompt().await.unwrap().unwrap();
                assert!(
                    !prompt.contains("SHADOWED_CLAUDE") && !prompt.contains("GLOBAL_INSTRUCTIONS")
                );
            });
    }

    #[test]
    fn prompt_source_read_failures_and_literals_do_not_create_file_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("SYSTEM.md");
        let mut sources = Vec::new();
        assert!(read_prompt_source(&file, &mut sources).is_none());
        std::fs::write(&file, [0xff, 0xfe]).unwrap();
        assert!(read_prompt_source(&file, &mut sources).is_none());
        assert!(read_append_target("literal instructions", &mut sources).is_none());
        assert!(sources.is_empty());
        std::fs::write(&file, "loaded instructions").unwrap();
        assert_eq!(
            read_prompt_source(&file, &mut sources).as_deref(),
            Some("loaded instructions")
        );
        assert!(read_prompt_source(&file, &mut sources).is_some());
        assert_eq!(sources, vec![file.to_string_lossy().into_owned()]);
    }

    extern "C" fn deny_test_tool_call(
        _event: ice_plugin_sdk::StablePluginEvent,
        _data: *mut std::ffi::c_void,
    ) -> i32 {
        ice_plugin_sdk::EVENT_HANDLER_ABORT
    }

    async fn invoke_test_tool_hook(
        hook: &ice_agent::BeforeToolCall,
    ) -> Option<ice_agent::BeforeToolCallResult> {
        let message = ice_ai::providers::faux::faux_assistant_message(
            "approval",
            ice_ai::types::StopReason::ToolUse,
        );
        let ice_ai::types::Content::ToolCall(call) =
            ice_ai::types::Content::tool_call("call", "read", serde_json::json!({"path":"x"}))
        else {
            unreachable!()
        };
        hook(
            ice_agent::BeforeToolCallContext {
                assistant_message: &message,
                tool_call: &call,
                args: &call.arguments,
                context: &ice_agent::AgentContext::default(),
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }

    #[tokio::test]
    async fn tool_call_hook_uses_reloaded_session_and_newly_discovered_gate() {
        let sessions = Arc::new(Mutex::new(ExtensionSession::none()));
        let hook =
            extension_tool_call_hook(sessions.clone(), ice_extensions::UiDialogMailbox::new());
        assert!(invoke_test_tool_hook(&hook).await.is_none());
        for name in ["first-gate", "reloaded-gate"] {
            let mut registry = ice_extensions::ExtensionRegistry::new();
            registry.register_event_handler(
                name.into(),
                ice_plugin_sdk::EventTag::ToolCall,
                deny_test_tool_call,
                std::ptr::null_mut(),
            );
            let fresh = ExtensionSession::from_parts(
                Arc::new(registry.snapshot()),
                ice_extensions::PluginKeepalive::empty(),
                Vec::new(),
                None,
            );
            {
                let mut current = sessions.lock().unwrap();
                if let Some(snapshot) = current.snapshot() {
                    snapshot.active_flag().store(false, Ordering::SeqCst);
                }
                *current = fresh;
            }
            let result = invoke_test_tool_hook(&hook).await.unwrap();
            assert!(result.block);
            assert!(result.reason.unwrap().contains(name));
        }
        *sessions.lock().unwrap() = ExtensionSession::none();
        assert!(invoke_test_tool_hook(&hook).await.is_none());
    }

    /// A unique scratch file for one summary test.
    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ice-summary-{}-{nanos}-{name}.jsonl",
            std::process::id()
        ));
        path
    }

    #[test]
    fn summary_marks_a_header_only_session_empty() {
        let path = scratch("empty");
        std::fs::write(
            &path,
            "{\"kind\":\"header\",\"version\":4,\"id\":\"abc\",\"createdAt\":1,\"cwd\":\"x\"}\n",
        )
        .unwrap();
        let summary = summarize_session_file(&path);
        assert!(summary.empty, "a header-only file has nothing to resume");
        assert!(summary.preview.is_none());
        assert_eq!(summary.bytes, std::fs::metadata(&path).unwrap().len());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn summary_reads_the_first_user_prompt_and_name() {
        let path = scratch("content");
        let body = concat!(
            "{\"kind\":\"header\",\"version\":4,\"id\":\"abc\",\"createdAt\":1,\"cwd\":\"x\"}\n",
            "{\"kind\":\"fact\",\"seq\":1,\"fact\":\"name\",\"name\":\"fix resume picker\"}\n",
            "{\"kind\":\"record\",\"type\":\"operation_started\",\"id\":\"r1\"}\n",
            "{\"kind\":\"entry\",\"lane\":\"main\",\"type\":\"message\",\"id\":\"e1\",\"message\":{\"kind\":\"user\",\"role\":\"user\",\"content\":\"read the icers -r picker\"}}\n",
            "{\"kind\":\"entry\",\"lane\":\"main\",\"type\":\"message\",\"id\":\"e2\",\"message\":{\"kind\":\"assistant\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"second\"}]}}\n",
        );
        std::fs::write(&path, body).unwrap();
        let summary = summarize_session_file(&path);
        assert!(!summary.empty);
        assert_eq!(summary.name.as_deref(), Some("fix resume picker"));
        assert_eq!(summary.preview.as_deref(), Some("read the icers -r picker"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn summary_extracts_a_block_style_user_prompt() {
        let path = scratch("blocks");
        let body = concat!(
            "{\"kind\":\"header\",\"version\":4,\"id\":\"abc\",\"createdAt\":1,\"cwd\":\"x\"}\n",
            "{\"kind\":\"entry\",\"type\":\"message\",\"id\":\"e1\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"line one\\nline two\"}]}}\n",
        );
        std::fs::write(&path, body).unwrap();
        let summary = summarize_session_file(&path);
        assert_eq!(summary.preview.as_deref(), Some("line one line two"));
        assert_eq!(session_display_label(&summary), "line one line two");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn summary_truncates_unicode_preview_by_characters() {
        let path = scratch("unicode-preview");
        let prompt = "界😀".repeat(60);
        let message = serde_json::json!({
            "kind": "entry",
            "type": "message",
            "id": "unicode",
            "message": {"role": "user", "content": prompt},
        });
        let body = format!(
            "{{\"kind\":\"header\",\"version\":4,\"id\":\"abc\",\"createdAt\":1,\"cwd\":\"x\"}}\n{}\n",
            serde_json::to_string(&message).unwrap()
        );
        std::fs::write(&path, body).unwrap();
        let summary = summarize_session_file(&path);
        let preview = summary.preview.expect("Unicode prompt preview");
        assert_eq!(preview.chars().count(), SUMMARY_PREVIEW_CHARS);
        assert_eq!(
            preview,
            prompt
                .chars()
                .take(SUMMARY_PREVIEW_CHARS)
                .collect::<String>()
        );
        std::fs::remove_file(&path).ok();
    }

    fn meta_for(path: &Path, id: &str) -> ice_harness::session::jsonl::JsonlSessionMetadata {
        ice_harness::session::jsonl::JsonlSessionMetadata {
            id: id.to_string(),
            created_at: 0,
            cwd: ".".to_string(),
            path: path.to_string_lossy().into_owned(),
            modified_at: 0,
            source_format: ice_harness::session::jsonl::JsonlSourceFormat::V4,
            parent_session_id: None,
            legacy_parent_session_path: None,
            metadata: None,
        }
    }

    #[test]
    fn continue_skips_newer_empty_sessions() {
        let empty = scratch("newest-empty");
        std::fs::write(
            &empty,
            "{\"kind\":\"header\",\"version\":4,\"id\":\"empty\",\"createdAt\":1,\"cwd\":\"x\"}\n",
        )
        .unwrap();
        let content = scratch("older-content");
        std::fs::write(
            &content,
            concat!(
                "{\"kind\":\"header\",\"version\":4,\"id\":\"old\",\"createdAt\":1,\"cwd\":\"x\"}\n",
                "{\"kind\":\"entry\",\"type\":\"message\",\"id\":\"e1\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
            ),
        )
        .unwrap();

        let metas = vec![meta_for(&empty, "empty"), meta_for(&content, "old")];
        let picked = first_resumable_session(&metas).expect("a resumable session");
        assert_eq!(picked.id, "old");

        // When every session is empty, fall back to the newest rather than
        // erroring.
        let all_empty = vec![meta_for(&empty, "empty")];
        assert_eq!(first_resumable_session(&all_empty).unwrap().id, "empty");

        std::fs::remove_file(&empty).ok();
        std::fs::remove_file(&content).ok();
    }

    #[test]
    fn list_session_files_sync_walks_the_cwd_subdirectory() {
        let root = std::env::temp_dir().join(format!("ice-summary-{}-dir", std::process::id()));
        let nested = root.join("--proj--");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("2026-01-01_a.jsonl");
        std::fs::write(
            &file,
            r#"{"kind":"header","version":4,"id":"a","createdAt":0,"cwd":"/proj"}"#,
        )
        .unwrap();
        let found = list_session_files_sync(&root);
        assert!(found.iter().any(|p| p == &file));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_session_files_sync_breaks_mtime_ties_by_path_descending() {
        let root = std::env::temp_dir().join(format!("ice-tie-{}-dir", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let paths: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|name| {
                let path = root.join(format!("{name}.jsonl"));
                std::fs::write(
                    &path,
                    format!(r#"{{"kind":"header","version":4,"id":"{name}","createdAt":0,"cwd":"/proj"}}"#),
                )
                .unwrap();
                let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                file.set_modified(when).unwrap();
                path
            })
            .collect();
        let found = list_session_files_sync(&root);
        assert_eq!(
            found,
            vec![paths[2].clone(), paths[1].clone(), paths[0].clone()]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn default_prompt_mentions_cwd_and_tools() {
        let p = default_system_prompt("/tmp/proj", false);
        assert!(p.contains("/tmp/proj"));
        assert!(p.contains("read"));
        assert!(p.contains("bash"));
        assert!(p.contains("edit"));
        assert!(p.contains("write"));
        assert!(p.contains("docs"));
        assert!(!p.contains("- grep"));
        assert!(!p.contains("- find"));
        assert!(!p.contains("- ls"));
        assert!(!p.contains("- powershell"));
        assert!(p.contains("Bash/POSIX syntax, including on Windows"));
        assert!(p.contains("do not pass PowerShell cmdlets"));
    }

    #[test]
    fn default_prompt_instructs_language_matching() {
        // The model must detect the user's language and answer in it, so a
        // Chinese (or any non-English) user gets a same-language reply without
        // having to ask every turn.
        let p = default_system_prompt("/tmp/proj", false);
        let lower = p.to_lowercase();
        assert!(
            lower.contains("same language") && lower.contains("detect"),
            "the prompt must tell the model to detect and mirror the user's language: {p}"
        );
    }

    #[test]
    fn default_prompt_keeps_working_state_in_the_visible_channel() {
        // Regression guard for the re-plan loop: on openai-compatible providers
        // the assistant's `thinking` blocks are dropped when the next request is
        // built (`providers/openai_completions.rs`: `Content::text_only`), so a
        // plan that lives only in reasoning is re-derived every turn. One
        // observed session paid for that 112 times in a single run, and twice
        // hallucinated the user's input ("The user is greeting me with hello",
        // with no such message). See `docs/llm-repetition-forensics.md` §八.
        let p = default_system_prompt("/tmp/proj", false);
        let lower = p.to_lowercase();
        // The rule must state the mechanism, not just "be clear".
        assert!(
            lower.contains("reasoning is not") && lower.contains("carried into your next turn"),
            "the prompt must explain that reasoning does not survive the turn: {p}"
        );
        // …and the remedy: keep state visible, use the todo tool.
        assert!(
            lower.contains("visible") && lower.contains("todo tool"),
            "the prompt must point at visible text and the todo tool: {p}"
        );
        // …plus the anti-repetition rule itself.
        assert!(
            lower.contains("re-stating the same plan"),
            "the prompt must call out plan re-statement as a bug: {p}"
        );
        // And the guidance must not instruct re-reading what is already shown.
        assert!(
            lower.contains("already shown earlier"),
            "the prompt must discourage re-fetching known content: {p}"
        );
    }

    #[test]
    fn default_prompt_does_not_claim_reasoning_is_lost_when_it_is_replayed() {
        // `compat.requiresThinkingAsText` (and the native thinking channels of
        // anthropic-messages / openai-responses) DO carry the model's reasoning
        // back on the next request. Telling such a model "reasoning is not
        // carried into your next turn" is false, and the instruction to restate
        // the plan every turn is exactly the busy-work the re-plan analysis
        // measured. The wording must flip with the mechanism.
        let p = default_system_prompt("/tmp/proj", true);
        let lower = p.to_lowercase();
        assert!(
            lower.contains("carried back"),
            "a replaying model must be told its reasoning survives: {p}"
        );
        assert!(
            !lower.contains("reasoning is not"),
            "the replayed variant must not claim reasoning is dropped: {p}"
        );
        assert!(
            lower.contains("do not re-derive"),
            "the replayed variant must forbid re-deriving the plan: {p}"
        );
        assert!(
            lower.contains("do not re-derive") && lower.contains("re-listing the same plan"),
            "the anti-repetition rule must survive the flip: {p}"
        );
        assert!(
            lower.contains("already shown earlier"),
            "the re-read rule must survive the flip: {p}"
        );
    }

    /// Only side-effect-free tools may be replayed after a crash.
    ///
    /// Regression guard for an inverted default: `ToolReplay` used to default to
    /// `Safe` and the CLI then marked *every* tool `Safe`, so the first time
    /// recovery acted on the flag it would have re-run `bash`/`write`/`edit` and
    /// duplicated their side effects. upstream defaults to `"never"`
    /// (`drive/tools.ts: tool.replay ?? "never"`).
    #[test]
    fn only_read_only_tools_are_replayable() {
        let env = Arc::new(OsExecutionEnv::with_cwd(PathBuf::from(".")));
        let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env.clone();
        let mut_env: Arc<dyn ice_tools::MutatingEnv> = env.clone();
        let context = ExecutionToolContext::new(env_dyn, Some(mut_env));
        let tools = build_tools(&context, &Args::default(), None);

        let replay_of = |name: &str| {
            tools
                .iter()
                .find(|tool| tool.tool.schema().name == name)
                .map(|tool| tool.replay)
        };

        for read_only in ["read", "docs"] {
            assert_eq!(
                replay_of(read_only),
                Some(ToolReplay::Safe),
                "{read_only} is side-effect free, so recovery may re-run it"
            );
        }
        for mutating in ["bash", "edit", "write", "powershell"] {
            if let Some(replay) = replay_of(mutating) {
                assert_eq!(
                    replay,
                    ToolReplay::Never,
                    "{mutating} must never be replayed: re-running it would duplicate its effect"
                );
            }
        }
    }

    #[test]
    fn default_tools_include_docs_lookup() {
        let env = Arc::new(OsExecutionEnv::with_cwd(PathBuf::from(".")));
        let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env.clone();
        let mut_env: Arc<dyn ice_tools::MutatingEnv> = env.clone();
        let context = ExecutionToolContext::new(env_dyn, Some(mut_env));
        let names: Vec<String> = build_tools(&context, &Args::default(), None)
            .iter()
            .map(|tool| tool.tool.schema().name.clone())
            .collect();

        // `docs` is a default capability; the legacy ice-only lookup tools stay
        // unregistered. Asserted by membership (not exact equality) because
        // `powershell` is Windows-only and `settings.json#defaultTools` may
        // narrow the set in a developer's environment.
        assert!(names.contains(&"docs".to_string()));
        for legacy in ["grep", "find", "ls"] {
            assert!(
                !names.contains(&legacy.to_string()),
                "legacy tool {legacy} should not be registered by default"
            );
        }
    }

    #[test]
    fn reload_settings_preflight_respects_trust() {
        struct RestoreConfigDir(Option<std::ffi::OsString>);

        impl Drop for RestoreConfigDir {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var(crate::config::CONFIG_DIR_ENV, value),
                    None => std::env::remove_var(crate::config::CONFIG_DIR_ENV),
                }
            }
        }

        let _guard = crate::config::test_support::env_lock().lock().unwrap();
        let _restore = RestoreConfigDir(std::env::var_os(crate::config::CONFIG_DIR_ENV));
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let cwd = tmp.path().join("project");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(cwd.join(".icers")).unwrap();
        std::fs::write(agent.join("settings.json"), "{}").unwrap();
        std::fs::write(cwd.join(".icers/settings.json"), "{ malformed").unwrap();
        std::env::set_var(crate::config::CONFIG_DIR_ENV, &agent);
        let args = Args::default();

        let trusted = validate_settings_for_reload(&args, &cwd, true);
        let untrusted = validate_settings_for_reload(&args, &cwd, false);
        assert!(trusted
            .unwrap_err()
            .contains("could not load project settings"));
        assert!(untrusted.is_ok());

        std::fs::write(agent.join("settings.json"), "{ malformed").unwrap();
        assert!(validate_settings_for_reload(&args, &cwd, false)
            .unwrap_err()
            .contains("could not load global settings"));

        let local_only = Args {
            dev_local_only: true,
            ..Args::default()
        };
        assert!(validate_settings_for_reload(&local_only, &cwd, true).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reload_preserves_live_resources_when_settings_break() {
        struct RestoreConfigDir(Option<std::ffi::OsString>);

        impl Drop for RestoreConfigDir {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var(crate::config::CONFIG_DIR_ENV, value),
                    None => std::env::remove_var(crate::config::CONFIG_DIR_ENV),
                }
            }
        }

        let _guard = crate::config::test_support::env_lock().lock().unwrap();
        let _restore = RestoreConfigDir(std::env::var_os(crate::config::CONFIG_DIR_ENV));
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let cwd = tmp.path().join("project");
        let skill_dir = agent.join("skills");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: keep-me\ndescription: Reload sentinel\n---\nKeep this skill loaded.",
        )
        .unwrap();
        std::fs::write(
            agent.join("settings.json"),
            serde_json::json!({"skillDirs": [skill_dir]}).to_string(),
        )
        .unwrap();
        std::env::set_var(crate::config::CONFIG_DIR_ENV, &agent);

        let resolved = crate::provider::resolve(
            Some("anthropic"),
            Some(crate::provider::DEFAULT_MODEL_ID),
            None,
            Some("test-key"),
            None,
        )
        .unwrap();
        let args = Args {
            trust_override: Some(false),
            no_session: true,
            no_extensions: true,
            no_prompt_templates: true,
            no_context_files: true,
            system_prompt: Some("stable system prompt".into()),
            ..Args::default()
        };
        let (harness, _events, context) = build(&resolved, &args, &cwd, false).await.unwrap();
        let before_resources = harness.get_resources().await.unwrap();
        assert!(before_resources
            .skills
            .as_ref()
            .unwrap()
            .iter()
            .any(|skill| skill.name == "keep-me"));
        let before_prompt = harness.get_system_prompt().await.unwrap();
        let before_tools: Vec<String> = harness
            .get_tools()
            .await
            .unwrap()
            .iter()
            .map(|tool| tool.tool.schema().name.clone())
            .collect();
        let before_bridge = context
            .action_bridge
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();

        std::fs::write(agent.join("settings.json"), "{ malformed").unwrap();
        let outcome = reload_extension_resources(&harness, &context).await;

        assert!(outcome.had_warnings);
        assert!(outcome.summary.contains("Settings reload failed"));
        assert!(outcome.summary.contains("could not load global settings"));
        assert_eq!(harness.get_resources().await.unwrap(), before_resources);
        assert_eq!(harness.get_system_prompt().await.unwrap(), before_prompt);
        let after_tools: Vec<String> = harness
            .get_tools()
            .await
            .unwrap()
            .iter()
            .map(|tool| tool.tool.schema().name.clone())
            .collect();
        assert_eq!(after_tools, before_tools);
        let after_bridge = context
            .action_bridge
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        assert!(Arc::ptr_eq(&after_bridge, &before_bridge));
    }

    #[test]
    fn reload_preparation_rejects_settings_changed_during_derivation() {
        struct RestoreConfigDir(Option<std::ffi::OsString>);

        impl Drop for RestoreConfigDir {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var(crate::config::CONFIG_DIR_ENV, value),
                    None => std::env::remove_var(crate::config::CONFIG_DIR_ENV),
                }
            }
        }

        let _guard = crate::config::test_support::env_lock().lock().unwrap();
        let _restore = RestoreConfigDir(std::env::var_os(crate::config::CONFIG_DIR_ENV));
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let cwd = tmp.path().join("project");
        let first_skill_dir = tmp.path().join("first-skills");
        let second_skill_dir = tmp.path().join("second-skills");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(
            agent.join("settings.json"),
            serde_json::json!({"skillDirs": [first_skill_dir]}).to_string(),
        )
        .unwrap();
        std::env::set_var(crate::config::CONFIG_DIR_ENV, &agent);
        let args = Args {
            trust_override: Some(false),
            no_extensions: true,
            no_prompt_templates: true,
            ..Args::default()
        };

        let result = prepare_reload_inputs_inner(&args, &cwd, false, || {
            std::fs::write(
                agent.join("settings.json"),
                serde_json::json!({"skillDirs": [second_skill_dir]}).to_string(),
            )
            .unwrap();
        });

        let error = result
            .err()
            .expect("settings mutation must fail preparation");
        assert!(error.contains("settings changed while reload inputs were being prepared"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reload_uses_frozen_settings_inputs_after_preparation() {
        struct RestoreConfigDir(Option<std::ffi::OsString>);

        impl Drop for RestoreConfigDir {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var(crate::config::CONFIG_DIR_ENV, value),
                    None => std::env::remove_var(crate::config::CONFIG_DIR_ENV),
                }
            }
        }

        let _guard = crate::config::test_support::env_lock().lock().unwrap();
        let _restore = RestoreConfigDir(std::env::var_os(crate::config::CONFIG_DIR_ENV));
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let cwd = tmp.path().join("project");
        let skill_dir = agent.join("skills");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: frozen-skill\ndescription: Reload sentinel\n---\nFrozen input.",
        )
        .unwrap();
        std::fs::write(
            agent.join("settings.json"),
            serde_json::json!({"skillDirs": [skill_dir]}).to_string(),
        )
        .unwrap();
        std::env::set_var(crate::config::CONFIG_DIR_ENV, &agent);

        let resolved = crate::provider::resolve(
            Some("anthropic"),
            Some(crate::provider::DEFAULT_MODEL_ID),
            None,
            Some("test-key"),
            None,
        )
        .unwrap();
        let args = Args {
            trust_override: Some(false),
            no_session: true,
            no_extensions: true,
            no_prompt_templates: true,
            no_context_files: true,
            system_prompt: Some("stable system prompt".into()),
            ..Args::default()
        };
        let (harness, _events, context) = build(&resolved, &args, &cwd, false).await.unwrap();

        let outcome = reload_extension_resources_inner(&harness, &context, || {
            std::fs::write(agent.join("settings.json"), "{ malformed").unwrap();
        })
        .await;

        assert!(!outcome.summary.contains("Settings reload failed"));
        let resources = harness.get_resources().await.unwrap();
        let skills = resources.skills.as_ref().unwrap();
        assert!(outcome
            .summary
            .contains(&format!("{} skill(s)", skills.len())));
        assert!(skills.iter().any(|skill| skill.name == "frozen-skill"));
    }

    #[tokio::test]
    async fn reload_resource_paths_include_explicit_skill_and_prompt_files() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_path = tmp.path().join("explicit-skill.md");
        std::fs::write(
            &skill_path,
            "---\nname: explicit-skill\ndescription: Explicit skill\n---\nSkill body",
        )
        .unwrap();
        let prompt_path = tmp.path().join("explicit-prompt.md");
        std::fs::write(
            &prompt_path,
            "---\ndescription: Explicit prompt\n---\nPrompt body",
        )
        .unwrap();

        let args = Args {
            skill: vec![skill_path.clone()],
            prompt_template: vec![prompt_path.clone()],
            ..Args::default()
        };
        let skill_paths = append_reload_resource_paths(Vec::new(), &args.skill, &[], &[], &[]);
        let prompt_paths =
            append_reload_resource_paths(Vec::new(), &args.prompt_template, &[], &[], &[]);
        let env = Arc::new(OsExecutionEnv::with_cwd(tmp.path().to_path_buf()));
        let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env;

        let skills = load_skills_with_precedence(&env_dyn, &skill_paths).await;
        assert_eq!(skills.skills.len(), 1, "{:?}", skills.diagnostics);
        assert_eq!(skills.skills[0].name, "explicit-skill");

        let prompts = load_prompt_templates_with_precedence(&env_dyn, &prompt_paths).await;
        assert_eq!(
            prompts.prompt_templates.len(),
            1,
            "{:?}",
            prompts.diagnostics
        );
        assert_eq!(prompts.prompt_templates[0].name, "explicit-prompt");
    }

    #[test]
    fn tool_policy_applies_to_rust_and_js_active_names() {
        let names = vec![
            "read".to_string(),
            "ask_user_question".to_string(),
            "write".to_string(),
        ];

        let args = Args {
            tools: Some(vec!["read".into(), "ask_user_question".into()]),
            ..Args::default()
        };
        assert_eq!(
            filter_active_tool_names(names.clone(), &args),
            vec!["read", "ask_user_question"]
        );

        let args = Args {
            exclude_tools: Some(vec!["ask_user_question".into()]),
            ..Args::default()
        };
        assert_eq!(
            filter_active_tool_names(names.clone(), &args),
            vec!["read", "write"]
        );

        let args = Args {
            no_tools: true,
            ..Args::default()
        };
        assert!(filter_active_tool_names(names, &args).is_empty());
    }

    #[test]
    fn select_ephemeral_when_no_session() {
        let args = Args {
            no_session: true,
            ..Args::default()
        };
        let cwd = Path::new("/tmp");
        assert!(matches!(
            select_session(&args, cwd),
            SessionSelection::Ephemeral
        ));
    }

    #[test]
    fn project_resources_load_by_default_and_allow_explicit_opt_out() {
        let defaults = Args::default();
        assert!(resolve_project_trust(
            &defaults,
            Path::new("C:/definitely-not-a-project")
        ));

        let denied = Args {
            trust_override: Some(false),
            ..Args::default()
        };
        assert!(!resolve_project_trust(
            &denied,
            Path::new("C:/definitely-not-a-project")
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_preserves_the_supplied_startup_project_snapshot() {
        struct RestoreConfigDir(Option<std::ffi::OsString>);

        impl Drop for RestoreConfigDir {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var(crate::config::CONFIG_DIR_ENV, value),
                    None => std::env::remove_var(crate::config::CONFIG_DIR_ENV),
                }
            }
        }

        let _guard = crate::config::test_support::env_lock().lock().unwrap();
        let _restore = RestoreConfigDir(std::env::var_os(crate::config::CONFIG_DIR_ENV));
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let cwd = tmp.path().join("project");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::env::set_var(crate::config::CONFIG_DIR_ENV, &agent);

        let resolved = crate::provider::resolve(
            Some("anthropic"),
            Some(crate::provider::DEFAULT_MODEL_ID),
            None,
            Some("test-key"),
            None,
        )
        .unwrap();
        let args = Args {
            trust_override: Some(false),
            no_session: true,
            no_tools: true,
            no_extensions: true,
            no_skills: true,
            no_prompt_templates: true,
            no_context_files: true,
            system_prompt: Some("test prompt".into()),
            ..Args::default()
        };

        let (_, _, context) = build(&resolved, &args, &cwd, true).await.unwrap();

        assert_eq!(context.cwd, cwd);
        assert!(context.project_trusted);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_applies_the_name_flag_to_the_session() {
        struct RestoreConfigDir(Option<std::ffi::OsString>);

        impl Drop for RestoreConfigDir {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var(crate::config::CONFIG_DIR_ENV, value),
                    None => std::env::remove_var(crate::config::CONFIG_DIR_ENV),
                }
            }
        }

        let _guard = crate::config::test_support::env_lock().lock().unwrap();
        let _restore = RestoreConfigDir(std::env::var_os(crate::config::CONFIG_DIR_ENV));
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let cwd = tmp.path().join("project");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::env::set_var(crate::config::CONFIG_DIR_ENV, &agent);

        let resolved = crate::provider::resolve(
            Some("anthropic"),
            Some(crate::provider::DEFAULT_MODEL_ID),
            None,
            Some("test-key"),
            None,
        )
        .unwrap();
        let args = Args {
            trust_override: Some(false),
            no_tools: true,
            no_extensions: true,
            no_skills: true,
            no_prompt_templates: true,
            no_context_files: true,
            system_prompt: Some("test prompt".into()),
            name: Some("smoke name".into()),
            session_dir: Some(cwd.join("sessions")),
            ..Args::default()
        };

        let (harness, _, _) = build(&resolved, &args, &cwd, true).await.unwrap();
        assert_eq!(
            harness.get_name().await.unwrap().as_deref(),
            Some("smoke name")
        );
    }

    #[test]
    fn select_latest_for_continue_and_resume() {
        let args = Args {
            continue_session: true,
            ..Args::default()
        };
        let cwd = Path::new("/tmp");
        assert!(matches!(
            select_session(&args, cwd),
            SessionSelection::Latest { .. }
        ));

        let args = Args {
            resume: true,
            ..Args::default()
        };
        assert!(matches!(
            select_session(&args, cwd),
            SessionSelection::Latest { .. }
        ));
    }

    #[test]
    fn select_by_id_for_session_flag() {
        let args = Args {
            session: Some("01a02ece".into()),
            ..Args::default()
        };
        let cwd = Path::new("/tmp");
        assert!(matches!(
            select_session(&args, cwd),
            SessionSelection::ById { id, .. } if id == "01a02ece"
        ));
    }

    #[test]
    fn select_new_with_custom_dir() {
        let args = Args {
            session_dir: Some(PathBuf::from("/tmp/sess")),
            ..Args::default()
        };
        let cwd = Path::new("/tmp");
        match select_session(&args, cwd) {
            SessionSelection::New { dir, .. } => assert_eq!(dir, PathBuf::from("/tmp/sess")),
            other => panic!("expected New, got {other:?}"),
        }
    }

    #[test]
    fn select_new_default_dir() {
        let args = Args::default();
        let cwd = Path::new("/proj");
        match select_session(&args, cwd) {
            SessionSelection::New { dir, .. } => {
                assert_eq!(dir, Path::new("/proj/.icers/sessions"));
            }
            other => panic!("expected New, got {other:?}"),
        }
    }

    #[test]
    fn default_session_dir_only_uses_ice() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        std::fs::create_dir_all(cwd.join(".icers/sessions")).unwrap();
        assert_eq!(default_session_dir(cwd), cwd.join(".icers/sessions"));
        std::fs::create_dir_all(cwd.join(".icers/sessions")).unwrap();
        assert_eq!(default_session_dir(cwd), cwd.join(".icers/sessions"));
    }

    #[test]
    fn new_session_replaces_conversation_without_modifying_old_history() {
        const CHILD: &str = "ICERS_SESSION_RESET_TEST_CWD";
        let Some(cwd) = std::env::var_os(CHILD) else {
            let directory = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "session::tests::new_session_replaces_conversation_without_modifying_old_history", "--nocapture"])
                .env(CHILD, directory.path())
                .env(crate::config::CONFIG_DIR_ENV, directory.path().join("config"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let harness = AgentHarness::create(Default::default()).await.unwrap();
                let old = Session::new(harness.session().storage(), None);
                let old_id = old.get_metadata().await.unwrap().id;
                old.append_message(ice_agent::AgentMessage::User(
                    ice_ai::types::UserMessage::new("old history", 1),
                ))
                .await
                .unwrap();
                harness
                    .lane("main")
                    .follow_up(ice_agent::AgentMessage::User(
                        ice_ai::types::UserMessage::new("old queued message", 2),
                    ))
                    .await
                    .unwrap();
                let context = ice_extensions::ToolCallContext::new(
                    PathBuf::from(&cwd).to_string_lossy().into_owned(),
                );
                let id = start_new_session(&harness, Path::new(&cwd), Some(&context))
                    .await
                    .unwrap();
                assert_ne!(id, old_id);
                assert_eq!(harness.session().get_metadata().await.unwrap().id, id);
                assert!(harness
                    .session()
                    .find_entries(&Default::default())
                    .await
                    .unwrap()
                    .is_empty());
                assert_eq!(
                    old.find_entries(&Default::default()).await.unwrap().len(),
                    1
                );
                harness
                    .session()
                    .append_message(ice_agent::AgentMessage::User(
                        ice_ai::types::UserMessage::new("new history", 3),
                    ))
                    .await
                    .unwrap();
                assert_eq!(
                    harness.session().get_stats().await.unwrap().message_count,
                    1
                );
                assert_eq!(old.get_stats().await.unwrap().message_count, 1);
                assert_ne!(
                    harness.session().get_leaf_id().await.unwrap(),
                    old.get_leaf_id().await.unwrap()
                );
                assert!(
                    list_session_metadata(&PathBuf::from(&cwd).to_string_lossy())
                        .await
                        .unwrap()
                        .iter()
                        .any(|metadata| metadata.id == id)
                );
            });
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ephemeral_session_builds_roundtrips() {
        // Sanity: the ephemeral path produces a usable Session facade (the
        // harness build itself needs a provider; tested via the integration
        // path in tests/build.rs instead).
        let s = ephemeral_session();
        let leaf = s.get_leaf_id().await;
        assert!(leaf.is_ok());
    }

    // NOTE: `build_tools`/`active_tool_names` integration is exercised by the
    // `tests/build.rs` harness-build test (needs a provider + multi-thread rt).

    /// The exclusion policy, checked against the constants rather than copies.
    ///
    /// `ice-tools` builds its exclusion from `ice-plugin-sdk`'s declarations, so
    /// within this repo a rename is a compile error — this test is not the link
    /// between those two crates any more. What it still guards is the part the
    /// compiler cannot see:
    ///
    /// - **The wire values.** `ice-langfuse` lives in a different repository and
    ///   agrees on the *strings*, not the constant paths. Rewriting a constant's
    ///   value would compile here and silently stop matching there, so the exact
    ///   values are pinned below. Changing one is a deliberate, breaking change to
    ///   a cross-repo contract, and it should fail here first.
    /// - **Coverage.** Every name the SDK declares must actually be excluded.
    /// - **Narrowness.** The user's own configuration must still reach the shell.
    #[test]
    fn host_private_env_matches_the_declared_contract() {
        // 1. The wire values. Pinned literally on purpose: see the doc comment.
        assert_eq!(ice_plugin_sdk::SESSION_ID_ENV, "ICERS_SESSION_ID");
        assert_eq!(
            ice_plugin_sdk::SUBAGENT_PARENT_ENV,
            [
                "LANGFUSE_PI_PARENT_TRACE_ID",
                "LANGFUSE_PI_PARENT_SPAN_ID",
                "LANGFUSE_PI_PARENT_SESSION_ID",
                "LANGFUSE_PI_PARENT_DEPTH",
            ],
            "these strings are what ice-langfuse (a different repo) matches; a \
             change here is a breaking cross-repo change, not a refactor"
        );

        // 2. Coverage — derived from the declarations, so a newly added family
        //    member is checked without anyone remembering to extend this test.
        for name in ice_plugin_sdk::SUBAGENT_PARENT_ENV
            .iter()
            .chain(std::iter::once(&ice_plugin_sdk::SESSION_ID_ENV))
        {
            assert!(
                ice_tools::is_host_private_env(name),
                "`{name}` must not reach a spawned shell, but ice-tools excludes only {:?}",
                ice_tools::HOST_PRIVATE_ENV
            );
        }

        // 3. Narrowness — these name how the *user* configured the run and are
        //    exactly what a command inside the shell should still see.
        for visible in [
            crate::args::ICERS_OFFLINE_ENV,
            crate::config::CONFIG_DIR_ENV,
            "PATH",
        ] {
            assert!(
                !ice_tools::is_host_private_env(visible),
                "{visible} describes the user's configuration and must still reach the shell"
            );
        }
    }
    #[tokio::test]
    async fn reload_shutdown_joins_all_handlers_even_after_veto() {
        use ice_plugin_sdk::{EventTag, StablePluginEvent};
        use std::sync::atomic::AtomicUsize;
        static CONTEXTS: ice_plugin_sdk::handles::ToolHandles<Arc<AtomicUsize>> =
            ice_plugin_sdk::handles::ToolHandles::new();
        struct ContextOwner(usize);
        impl Drop for ContextOwner {
            fn drop(&mut self) {
                drop(CONTEXTS.remove(self.0 as ice_plugin_sdk::StepHandle));
            }
        }
        extern "C" fn stop(_: StablePluginEvent, data: *mut std::ffi::c_void) -> i32 {
            let context = CONTEXTS.get(data).expect("live test context");
            let calls = context.lock().clone();
            std::thread::sleep(std::time::Duration::from_millis(20));
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                ice_plugin_sdk::EVENT_HANDLER_ABORT
            } else {
                0
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let context = CONTEXTS.insert(calls.clone());
        let _owner = ContextOwner(context as usize);
        let mut registry = ice_extensions::ExtensionRegistry::new();
        for name in ["first", "second"] {
            registry.register_event_handler(name.into(), EventTag::SessionShutdown, stop, context);
        }
        super::shutdown_extensions_for_reload(
            Some(Arc::new(registry.snapshot())),
            ice_extensions::PluginKeepalive::empty(),
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
