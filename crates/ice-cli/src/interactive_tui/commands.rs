//! Built-in slash commands and extension command registration.

use super::*;

mod context_usage;
pub(super) mod mcp;
use mcp::McpCommand;

// ---- Built-in command implementations ----

pub(super) struct HelpCommand;
impl SlashCommand for HelpCommand {
    fn name(&self) -> &'static str {
        "/help"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["/?"]
    }
    fn description(&self) -> &'static str {
        "Show available commands"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        add_help_message(&ctx.chat);
        ctx.tui.request_render(false);
    }
}

/// `/new` (ice `handleClearCommand`) starts a new session. `/clear` is an
/// alias: a "clear" that kept the agent's history would leave the model
/// reading a conversation the screen claims is gone.
pub(super) struct NewSessionCommand;
impl SlashCommand for NewSessionCommand {
    fn name(&self) -> &'static str {
        "/new"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["/clear"]
    }
    fn alias_visible(&self) -> &'static [&'static str] {
        &["/clear"]
    }
    fn description(&self) -> &'static str {
        "Start a new session"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        let _ = ctx.tx.send(TuiMessage::NewSession);
    }
}

pub(super) struct ExitCommand;
impl SlashCommand for ExitCommand {
    fn name(&self) -> &'static str {
        "/exit"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["/quit", "/q"]
    }
    // `/quit` is surfaced (matches pi's BUILTIN list); `/q` stays a hidden alias.
    fn alias_visible(&self) -> &'static [&'static str] {
        &["/quit"]
    }
    fn description(&self) -> &'static str {
        "Exit the application"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        ctx.state.cancel_js_preparation();
        let _ = ctx.tx.send(TuiMessage::Exit);
    }
}

pub(super) struct VersionCommand;
impl SlashCommand for VersionCommand {
    fn name(&self) -> &'static str {
        "/version"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["/v"]
    }
    fn description(&self) -> &'static str {
        "Show version information"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        add_version_message(&ctx.chat);
        ctx.tui.request_render(false);
    }
}

pub(super) struct ChangelogCommand;
impl SlashCommand for ChangelogCommand {
    fn name(&self) -> &'static str {
        "/changelog"
    }
    fn description(&self) -> &'static str {
        "Show latest release changes (/changelog all for history)"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        match args.trim() {
            "" => add_changelog_message(&ctx.chat),
            "all" => add_changelog_message_with_scope(&ctx.chat, true),
            _ => {
                ctx.chat
                    .add_child(Arc::new(Text::new("Usage: /changelog [all]", 1, 0)));
            }
        }
        ctx.tui.request_render(false);
    }
}

pub(super) struct HotkeysCommand;
impl SlashCommand for HotkeysCommand {
    fn name(&self) -> &'static str {
        "/hotkeys"
    }
    fn description(&self) -> &'static str {
        "Show keyboard shortcuts"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        add_hotkeys_message(&ctx.chat);
        ctx.tui.request_render(false);
    }
}

pub(super) struct ModelCommand;
impl SlashCommand for ModelCommand {
    fn name(&self) -> &'static str {
        "/model"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["/m"]
    }
    fn description(&self) -> &'static str {
        "Choose a model (selector)"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let term = args.trim();
        if !term.is_empty() {
            // /model <name> — direct switch by id (pi handleModelCommand).
            //
            // upstream only switches on an EXACT match; a term that matches
            // nothing falls through to the selector with the query prefilled
            // instead of erroring out (`showModelSelector(searchTerm)`).
            if let Some(model) = find_model_selector_match(
                &ctx.model_catalog
                    .read()
                    .unwrap_or_else(|error| error.into_inner()),
                term,
            ) {
                spawn_set_model(
                    &ctx.state,
                    &ctx.lane,
                    model,
                    &ctx.chat,
                    &ctx.tui,
                    ModelSwitchMode::Session,
                );
                return;
            }
        }
        let current_id = ctx.state.current_model_id();
        open_model_selector(
            &ctx.state,
            &ctx.editor_container,
            &ctx.editor,
            &ctx.tui,
            &ctx.model_catalog
                .read()
                .unwrap_or_else(|error| error.into_inner()),
            &ctx.lane,
            &current_id,
            &ctx.chat,
            (!term.is_empty()).then_some(term),
        );
    }
}

pub(super) struct ProviderCommand;
impl SlashCommand for ProviderCommand {
    fn name(&self) -> &'static str {
        "/provider"
    }
    fn description(&self) -> &'static str {
        "Switch provider now, or add/remove provider configuration"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let term = args.trim();
        if term == "add" || term == "new" {
            begin_provider_form(&ctx.state, &ctx.editor_container, &ctx.editor, &ctx.tui);
            return;
        }
        if let Some(id) = term.strip_prefix("remove ") {
            match remove_provider(id) {
                Ok(note) => add_note_message(&ctx.chat, &note),
                Err(error) => add_error_message(&ctx.chat, &error),
            }
            ctx.tui.request_render(false);
            return;
        }
        if !term.is_empty() {
            let _ = ctx.tx.send(TuiMessage::SwitchProvider(term.to_string()));
            return;
        }
        let _ = ctx.tx.send(TuiMessage::OpenProviderSelector);
    }
}

pub(super) struct ThinkingCommand;
impl SlashCommand for ThinkingCommand {
    fn name(&self) -> &'static str {
        "/thinking"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["/think"]
    }
    fn description(&self) -> &'static str {
        "Set thinking level (selector)"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let level_name = args.trim();
        if !level_name.is_empty() {
            // /thinking <level> — direct set (pi supports the param form).
            let Some(level) = thinking_level_from_name(level_name) else {
                add_error_message(
                    &ctx.chat,
                    &format!(
                        "Unknown thinking level \"{level_name}\". Valid: {}",
                        crate::args::VALID_THINKING_LEVELS.join(", ")
                    ),
                );
                ctx.tui.request_render(false);
                return;
            };
            let lane = ctx.lane.clone();
            let state = ctx.state.clone();
            let chat = ctx.chat.clone();
            let tui = ctx.tui.clone();
            tokio::spawn(async move {
                apply_thinking_selection(&lane, &state, &chat, level, false).await;
                tui.request_render(false);
            });
            ctx.tui.request_render(false);
            return;
        }
        let current_id = ctx.state.current_model_id();
        open_thinking_selector(
            &ctx.state,
            &ctx.editor_container,
            &ctx.editor,
            &ctx.tui,
            &ctx.lane,
            &ctx.model_catalog
                .read()
                .unwrap_or_else(|error| error.into_inner()),
            &current_id,
            &ctx.chat,
        );
    }
}

pub(super) struct ToolsCommand;
impl SlashCommand for ToolsCommand {
    fn name(&self) -> &'static str {
        "/tools"
    }
    fn description(&self) -> &'static str {
        "Toggle tools on/off"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        open_tools_selector(
            &ctx.state,
            &ctx.editor_container,
            &ctx.editor,
            &ctx.tui,
            &ctx.lane,
            &ctx.chat,
        );
    }
}

pub(super) struct ImagesCommand;
impl SlashCommand for ImagesCommand {
    fn name(&self) -> &'static str {
        "/images"
    }
    fn description(&self) -> &'static str {
        "Toggle inline images"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        open_images_selector(
            &ctx.state,
            &ctx.editor_container,
            &ctx.editor,
            &ctx.tui,
            &ctx.chat,
        );
    }
}

pub(super) struct SessionCommand;
impl SlashCommand for SessionCommand {
    fn name(&self) -> &'static str {
        "/session"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["/resume"]
    }
    fn alias_visible(&self) -> &'static [&'static str] {
        &["/resume"]
    }
    fn description(&self) -> &'static str {
        "List saved sessions"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        open_session_selector(
            &ctx.state,
            &ctx.editor_container,
            &ctx.editor,
            &ctx.tui,
            &ctx.cwd,
            &ctx.tx,
            Some(ctx.session.harness().session().storage()),
        );
    }
}

pub(super) struct ThemeCommand;
impl SlashCommand for ThemeCommand {
    fn name(&self) -> &'static str {
        "/theme"
    }
    fn description(&self) -> &'static str {
        "Choose a theme (selector)"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let name = args.trim().to_ascii_lowercase();
        if !name.is_empty() {
            // /theme <name> — direct apply + persist (matches /settings Theme).
            let preset = match name.as_str() {
                "light" => ThemePreset::Light,
                "monochrome" => ThemePreset::Monochrome,
                "dark" => ThemePreset::Dark,
                _ => {
                    add_error_message(
                        &ctx.chat,
                        &format!("Unknown theme \"{name}\". Valid: dark, light, monochrome."),
                    );
                    ctx.tui.request_render(false);
                    return;
                }
            };
            apply_theme_preset(preset);
            let mut settings = crate::settings::load_settings().unwrap_or_default();
            settings.theme = Some(name.clone());
            let _ = crate::settings::save_settings(&settings);
            add_note_message(&ctx.chat, &format!("Theme set to {name} (saved)."));
            ctx.tui.request_render(false);
            ctx.tui.render_now(true);
            return;
        }
        open_theme_selector(&ctx.state, &ctx.editor_container, &ctx.editor, &ctx.tui);
    }
}

pub(super) struct CompactCommand;
impl SlashCommand for CompactCommand {
    fn name(&self) -> &'static str {
        "/compact"
    }
    fn description(&self) -> &'static str {
        "Compact the conversation"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let instructions = match args.trim() {
            // Blackhole's manual mode only compacts on its own marker.
            "" => ctx
                .reload_context
                .builtins
                .blackhole
                .as_ref()
                .and_then(|engine| engine.manual_compaction_instruction())
                .map(str::to_string),
            text => Some(text.to_string()),
        };
        let _ = ctx.tx.send(TuiMessage::Compact(instructions));
    }
}

pub(super) struct CopyCommand;
impl SlashCommand for CopyCommand {
    fn name(&self) -> &'static str {
        "/copy"
    }
    fn description(&self) -> &'static str {
        "Copy last reply to clipboard"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        let _ = ctx.tx.send(TuiMessage::Copy);
    }
}

pub(super) struct ExportCommand;
impl SlashCommand for ExportCommand {
    fn name(&self) -> &'static str {
        "/export"
    }
    fn description(&self) -> &'static str {
        "Export session (supports: md, html, jsonl, native)"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let format = args.trim().to_lowercase();
        if format.is_empty() || format == "md" || format == "markdown" {
            let _ = ctx.tx.send(TuiMessage::ExportSession);
        } else if format == "html" {
            let _ = ctx.tx.send(TuiMessage::ExportSessionWithFormat(
                crate::export::ExportFormat::Html,
            ));
        } else if format == "jsonl" {
            let _ = ctx.tx.send(TuiMessage::ExportSessionWithFormat(
                crate::export::ExportFormat::Jsonl,
            ));
        } else if format == "native" {
            let _ = ctx.tx.send(TuiMessage::ExportNativeSession);
        } else {
            // Invalid format, show error
            add_error_message(
                &ctx.chat,
                &format!(
                    "Unknown export format: {}. Supported: md, html, jsonl, native",
                    args.trim()
                ),
            );
            ctx.tui.request_render(false);
        }
    }
}

/// `/unqueue [n]` — take one queued steering/follow-up message back into the
/// editor (1-based, in dock order; the newest when omitted). Unlike Alt+Q this
/// leaves the other queued messages in place.
pub(super) struct UnqueueCommand;
impl SlashCommand for UnqueueCommand {
    fn name(&self) -> &'static str {
        "/unqueue"
    }
    fn description(&self) -> &'static str {
        "Move one queued message back into the editor (/unqueue [n])"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let position = match args.trim() {
            "" => None,
            text => match text.parse::<usize>() {
                Ok(n) if n >= 1 => Some(n),
                _ => {
                    add_error_message(&ctx.chat, "Usage: /unqueue [n] (n starts at 1)");
                    ctx.tui.request_render(false);
                    return;
                }
            },
        };
        tokio::spawn(unqueue_message(
            ctx.session.harness().clone(),
            ctx.editor.clone(),
            ctx.state.clone(),
            ctx.tui.clone(),
            position,
        ));
    }
}

pub(super) struct ForkCommand;
impl SlashCommand for ForkCommand {
    fn name(&self) -> &'static str {
        "/fork"
    }
    fn description(&self) -> &'static str {
        "Fork from an earlier message into a new session"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        let _ = ctx.tx.send(TuiMessage::OpenForkSelector);
    }
}

/// `/clone` is the upstream spelling for duplicating the current session.
/// Reuse the same durable fork path as `/fork`; both create a child session
/// and rebind the live harness to it.
pub(super) struct CloneCommand;
impl SlashCommand for CloneCommand {
    fn name(&self) -> &'static str {
        "/clone"
    }
    fn description(&self) -> &'static str {
        "Duplicate the current session"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        let _ = ctx.tx.send(TuiMessage::ForkSession);
    }
}

pub(super) struct TreeCommand;
impl SlashCommand for TreeCommand {
    fn name(&self) -> &'static str {
        "/tree"
    }
    fn description(&self) -> &'static str {
        "Navigate the current session tree"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        let _ = ctx.tx.send(TuiMessage::OpenTree);
    }
}

pub(super) struct LoginCommand;
impl SlashCommand for LoginCommand {
    fn name(&self) -> &'static str {
        "/login"
    }
    fn description(&self) -> &'static str {
        "Log in to a provider (selector, API key, or OAuth)"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let args = args.trim();
        if args.is_empty() {
            open_login_provider_selector(
                &ctx.state,
                &ctx.editor_container,
                &ctx.editor,
                &ctx.tui,
                &ctx.state.extension_session,
                &ctx.tx,
            );
            return;
        }

        let manifests = current_provider_manifests(&ctx.state.extension_session);
        let oauth_providers = current_oauth_providers(&ctx.state.extension_session);
        // Provider ids are taken from the catalog so case-insensitive input is
        // persisted under the canonical id. A single argument preserves the
        // historical `/login <key>` shortcut for Anthropic.
        let mut parts = args.splitn(2, char::is_whitespace);
        let first = parts.next().unwrap_or_default();
        let login_items = login_provider_items_with_oauth(&manifests, &oauth_providers);
        let (provider, key) = match parts.next().map(str::trim).filter(|key| !key.is_empty()) {
            Some(key) => match login_items
                .iter()
                .find(|item| item.value.eq_ignore_ascii_case(first))
            {
                Some(item)
                    if oauth_providers
                        .iter()
                        .any(|oauth| oauth.id.eq_ignore_ascii_case(&item.value)) =>
                {
                    add_error_message(
                        &ctx.chat,
                        &format!(
                            "Provider '{first}' uses OAuth. Use /login to start its interactive login."
                        ),
                    );
                    ctx.tui.request_render(false);
                    return;
                }
                Some(item) => (item.value.clone(), key.to_string()),
                None => {
                    add_error_message(
                        &ctx.chat,
                        &format!(
                            "Unknown provider '{first}'. Use /login to choose a provider, or /login <api-key> for the Anthropic shortcut."
                        ),
                    );
                    ctx.tui.request_render(false);
                    return;
                }
            },
            None => {
                if login_items
                    .iter()
                    .any(|item| item.value.eq_ignore_ascii_case(first))
                {
                    add_error_message(
                        &ctx.chat,
                        &format!("Missing API key for provider '{first}'."),
                    );
                    ctx.tui.request_render(false);
                    return;
                }
                ("anthropic".to_string(), args.to_string())
            }
        };
        if ctx
            .tx
            .send(TuiMessage::AuthStoreMutation(AuthStoreMutationRequest {
                provider: provider.clone(),
                operation: AuthStoreMutationOperation::Upsert(crate::config::Credential::ApiKey {
                    key: Some(key.to_string()),
                    env: None,
                }),
                purpose: AuthStoreMutationPurpose::ApiKeyCommand,
            }))
            .is_err()
        {
            add_error_message(&ctx.chat, "Could not queue the credential save.");
        }
        ctx.tui.request_render(false);
    }
}

pub(super) struct LogoutCommand;
impl SlashCommand for LogoutCommand {
    fn name(&self) -> &'static str {
        "/logout"
    }
    fn description(&self) -> &'static str {
        "Remove saved provider credentials"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let provider = args.trim();
        if provider.is_empty() {
            open_logout_provider_selector(
                &ctx.state,
                &ctx.editor_container,
                &ctx.editor,
                &ctx.tui,
                &ctx.tx,
            );
            return;
        }
        let provider_id = crate::config::read_auth()
            .unwrap_or_default()
            .keys()
            .find(|id| id.eq_ignore_ascii_case(provider))
            .cloned()
            .unwrap_or_else(|| provider.to_string());
        let _ = ctx.tx.send(TuiMessage::LogoutProvider(provider_id));
        ctx.tui.request_render(false);
    }
}

pub(super) fn set_project_trust_for_command(
    cwd: &std::path::Path,
    value: Option<bool>,
) -> Result<(), crate::config::ConfigError> {
    crate::config::set_project_trust(cwd, value)
}

/// `/trust [yes|no|clear]`. A bare `/trust` reports saved and effective trust:
/// it used to mean "yes", so picking the command from autocomplete and
/// pressing Enter silently trusted the project (ice opens a selector and
/// changes nothing until a choice is made). Returns the note to show, or the
/// error.
pub(super) fn run_trust_command(
    cwd: &std::path::Path,
    args: &str,
    project_trusted: bool,
) -> Result<String, String> {
    const USAGE: &str = "Change it with /trust yes, /trust no or /trust clear.";
    let effective = if project_trusted {
        "trusted"
    } else {
        "untrusted"
    };
    let value = match args.trim().to_ascii_lowercase().as_str() {
        "" => {
            let saved = crate::config::project_trust_decision(cwd)
                .map_err(|error| format!("Could not read trust decision: {error}"))?;
            let state = match saved {
                Some(true) => "trusted (saved)",
                Some(false) => "untrusted (saved)",
                None => "no saved decision",
            };
            return Ok(format!(
                "Project trust for {}: {state}. Current session: {effective}. {USAGE} Restart icers after changing the saved decision.",
                cwd.display()
            ));
        }
        "yes" | "y" | "true" => Some(true),
        "no" | "n" | "false" => Some(false),
        "clear" | "reset" | "none" => None,
        _ => return Ok(format!("Usage: /trust [yes|no|clear]. {USAGE}")),
    };
    set_project_trust_for_command(cwd, value)
        .map_err(|error| format!("Could not save trust decision: {error}"))?;
    let label = match value {
        Some(true) => "marked trusted",
        Some(false) => "marked untrusted",
        None => "trust decision cleared",
    };
    Ok(format!(
        "Current project {label}. Current session: {effective}. Restart icers to apply it to project resources."
    ))
}

pub(super) struct TrustCommand;
impl SlashCommand for TrustCommand {
    fn name(&self) -> &'static str {
        "/trust"
    }
    fn description(&self) -> &'static str {
        "Show or set trust for the current project"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        match run_trust_command(&ctx.cwd, args, ctx.reload_context.project_trusted) {
            Ok(note) => add_note_message(&ctx.chat, &note),
            Err(error) => add_error_message(&ctx.chat, &error),
        }
        ctx.tui.request_render(false);
    }
}

pub(super) struct NameCommand;
impl SlashCommand for NameCommand {
    fn name(&self) -> &'static str {
        "/name"
    }
    fn description(&self) -> &'static str {
        "Set session display name"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let name = args.trim();
        if name.is_empty() {
            add_note_message(
                &ctx.chat,
                "Usage: /name <display name> — sets the current session's name.",
            );
            ctx.tui.request_render(false);
            return;
        }
        let _ = ctx.tx.send(TuiMessage::SetSessionName(name.to_string()));
    }
}

pub(super) struct ImportCommand;
impl SlashCommand for ImportCommand {
    fn name(&self) -> &'static str {
        "/import"
    }
    fn description(&self) -> &'static str {
        "Import a session file (path)"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let path = args.trim();
        if path.is_empty() {
            add_note_message(
                &ctx.chat,
                "Usage: /import <path-to-session.jsonl> — copies the file into the session dir and switches to it.",
            );
            ctx.tui.request_render(false);
            return;
        }
        let _ = ctx.tx.send(TuiMessage::ImportSession(path.to_string()));
    }
}

pub(super) struct SettingsCommand;
impl SlashCommand for SettingsCommand {
    fn name(&self) -> &'static str {
        "/settings"
    }
    fn description(&self) -> &'static str {
        "Open settings menu"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        let current_id = ctx.state.current_model_id();
        open_settings_selector(
            &ctx.state,
            &ctx.editor_container,
            &ctx.editor,
            &ctx.tui,
            &ctx.lane,
            &ctx.model_catalog
                .read()
                .unwrap_or_else(|error| error.into_inner()),
            &current_id,
            &ctx.chat,
            &ctx.reload_context.builtins,
        );
    }
}

/// `/blackhole` — inspect or change Blackhole compaction.
pub(super) struct BlackholeCommand;
impl SlashCommand for BlackholeCommand {
    fn name(&self) -> &'static str {
        "/blackhole"
    }
    fn description(&self) -> &'static str {
        "Configure optional Blackhole compaction"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let Some(dir) = ctx.reload_context.builtins.agent_dir.as_ref() else {
            add_note_message(
                &ctx.chat,
                "Blackhole is unavailable without an agent directory.",
            );
            ctx.tui.request_render(false);
            return;
        };
        let (level, text) = crate::blackhole::command::run_command(
            dir,
            &crate::blackhole::config::process_env,
            args,
        );
        add_notice_message(&ctx.chat, level, &text);
        ctx.tui.request_render(false);
    }
}

pub(super) struct ScopedModelsCommand;
impl SlashCommand for ScopedModelsCommand {
    fn name(&self) -> &'static str {
        "/scoped-models"
    }
    fn description(&self) -> &'static str {
        "Choose models for Ctrl+P cycling"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        open_scoped_models_selector(
            &ctx.state,
            &ctx.editor_container,
            &ctx.editor,
            &ctx.tui,
            &ctx.model_catalog
                .read()
                .unwrap_or_else(|error| error.into_inner()),
            &ctx.chat,
        );
    }
}

/// Shown instead of sharing until the user re-runs `/share confirm`.
const SHARE_CONFIRMATION_NOTE: &str = "/share sends the active main branch transcript (including compaction summaries) to an external service (a gh gist, else the clipboard). Abandoned branches are not included. Review the transcript for secrets or sensitive text first; sharing does not redact anything. Run `/share confirm` to proceed.";

pub(super) struct ShareCommand;
impl SlashCommand for ShareCommand {
    fn name(&self) -> &'static str {
        "/share"
    }
    fn description(&self) -> &'static str {
        "Share session (gist via gh, or clipboard); `/share confirm` to proceed"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        if args.trim() == "confirm" {
            let _ = ctx.tx.send(TuiMessage::ShareSession);
        } else {
            add_note_message(&ctx.chat, SHARE_CONFIRMATION_NOTE);
            ctx.tui.request_render(false);
        }
    }
}

/// `/context` — read-only active-context estimates, category grid and largest entries.
pub(super) struct ContextCommand;
impl SlashCommand for ContextCommand {
    fn name(&self) -> &'static str {
        "/context"
    }
    fn description(&self) -> &'static str {
        "Show context usage by category"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        context_usage::show(ctx);
    }
}

/// `/usage` — token/cost/cache totals read through the product-layer
/// `AgentSession`. This is the same numbers the footer shows, but expanded and
/// with the cache-hit breakdown that the one-line footer cannot fit.
pub(super) struct UsageCommand;
impl SlashCommand for UsageCommand {
    fn name(&self) -> &'static str {
        "/usage"
    }
    fn description(&self) -> &'static str {
        "Show ChatGPT plan limits and session token, cost, and cache totals"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        let _ = ctx.tx.send(TuiMessage::ShowUsage);
        ctx.tui.request_render(false);
    }
}

/// `/reload` — re-run extension + resource discovery into the LIVE harness
/// (B5d): reload the cdylib plugins, invalidate the old `ActionBridge` +
/// registry snapshot, rebuild skills/prompts/context/SYSTEM.md/APPEND_SYSTEM.md
/// + the `TeeEmitter`, and push the rebuilt state via the B5d harness setters.
/// The command itself runs on the blocking submit thread, so it can't drive
/// the async `reload_extension_resources` routine directly — it signals the main
/// loop via `TuiMessage::ReloadExtensions`, which awaits it on the async runtime.
/// (A plugin's `runtime_action(Reload)` signals the same loop via the
/// `ReloadMailbox` the TUI installs — the B5d async-reload design avoids the
/// self-unmapping race a synchronous plugin-initiated reload would have.)
pub(super) struct ReloadCommand;
impl SlashCommand for ReloadCommand {
    fn name(&self) -> &'static str {
        "/reload"
    }
    fn description(&self) -> &'static str {
        "Reload extensions, skills, prompts, provider models"
    }
    fn execute(&self, ctx: &CommandContext, _args: &str) {
        // Signal the main loop. It owns the `&AgentHarness` borrow the
        // `reload_extension_resources` routine needs (the blocking submit thread
        // only has the context's `Arc<ReloadContext>` + the `Arc<dyn AgentLane>`).
        add_note_message(&ctx.chat, "Reloading extensions + resources…");
        ctx.tui.request_render(false);
        let _ = ctx.tx.send(TuiMessage::ReloadExtensions);
    }
}

/// Build the full command registry: active built-ins first (so they win on a
/// fuzzy autocomplete tie), then the v1-out-of-scope stubs. Prompt-template
/// commands are merged in separately by the autocomplete builder (they dispatch
/// via template expansion, not this registry).
pub(super) fn build_builtin_registry() -> CommandRegistry {
    let mut r = CommandRegistry::new();
    r.register(Arc::new(HelpCommand));
    r.register(Arc::new(NewSessionCommand));
    r.register(Arc::new(ExitCommand));
    r.register(Arc::new(VersionCommand));
    r.register(Arc::new(ChangelogCommand));
    r.register(Arc::new(ModelCommand));
    r.register(Arc::new(ProviderCommand));
    r.register(Arc::new(ThinkingCommand));
    r.register(Arc::new(ToolsCommand));
    r.register(Arc::new(McpCommand));
    r.register(Arc::new(ImagesCommand));
    r.register(Arc::new(SessionCommand));
    r.register(Arc::new(ThemeCommand));
    r.register(Arc::new(CompactCommand));
    r.register(Arc::new(BlackholeCommand));
    r.register(Arc::new(CopyCommand));
    r.register(Arc::new(HotkeysCommand));
    r.register(Arc::new(ContextCommand));
    r.register(Arc::new(UsageCommand));
    // Recognized but inert in v1 (one struct backs them all). The TS builtins
    // out of v1 scope; each carries a description so autocomplete surfaces its
    // existence even though running it reports "not supported".
    r.register(Arc::new(NameCommand));
    r.register(Arc::new(SettingsCommand));
    r.register(Arc::new(ScopedModelsCommand));
    r.register(Arc::new(ExportCommand));
    r.register(Arc::new(ImportCommand));
    r.register(Arc::new(ShareCommand));
    r.register(Arc::new(UnqueueCommand));
    r.register(Arc::new(ForkCommand));
    r.register(Arc::new(CloneCommand));
    r.register(Arc::new(TreeCommand));
    r.register(Arc::new(TrustCommand));
    r.register(Arc::new(LoginCommand));
    r.register(Arc::new(LogoutCommand));
    r.register(Arc::new(ReloadCommand));
    r
}

pub(super) fn register_extension_commands(
    registry: &mut CommandRegistry,
    session: crate::session::ExtensionSessionCell,
) {
    let commands = session
        .lock()
        .ok()
        .and_then(|s| s.snapshot_arc())
        .map(|snap| snap.commands().to_vec())
        .unwrap_or_default();
    for command in commands {
        let name = if command.name.starts_with('/') {
            command.name.clone()
        } else {
            format!("/{}", command.name)
        };
        if registry.find(&name).is_some() {
            continue;
        }
        registry.register(Arc::new(ExtensionCommand {
            name,
            description: command.description,
            session: session.clone(),
        }));
    }
}
