use super::*;
use crate::interactive_tui::mcp_panel::{self, McpPanel, McpPanelView, PanelAction};
use crate::interactive_tui::mcp_setup_panel::{self, SetupAction, SetupPanel, SetupPanelView};
use crate::mcp::{McpManager, ServerStatus};

const USAGE: &str = "/mcp [status|tools [server]|prompts [server]|reconnect [server]|enable <server>|disable <server>|add <name> <command> [args...]|remove <name>|edit [project]|setup]";

struct McpPromptCommand {
    name: String,
    description: String,
}
impl SlashCommand for McpPromptCommand {
    fn name(&self) -> &str {
        &self.name
    }
    fn description_owned(&self) -> String {
        self.description.clone()
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let ctx = ctx.clone();
        let storage = ctx.session.harness().session().storage();
        let command = self.name.clone();
        let args = args.to_owned();
        tokio::spawn(async move {
            let result = ctx
                .reload_context
                .mcp_tools
                .get_prompt(&command, &args)
                .await;
            if ctx
                .state
                .event_drain
                .session_changing
                .load(std::sync::atomic::Ordering::Acquire)
                || !Arc::ptr_eq(&storage, &ctx.session.harness().session().storage())
            {
                return;
            }
            match result {
                Ok(prompt) => submit_user_prompt(&ctx, &prompt),
                Err(error) => add_error_message(&ctx.chat, &error),
            }
            ctx.tui.request_render(false);
        });
    }
}
pub(in crate::interactive_tui) fn register_prompts(
    registry: &mut CommandRegistry,
    manager: &McpManager,
) {
    for (server, prompt) in manager.prompts() {
        let name = prompt.command_name(&server);
        // Names can collide (a server named `a__b` vs prompt `b__c` on `a`, or a
        // built-in); the first registration wins, matching `get_prompt`'s lookup.
        if registry.find(&name).is_some() {
            tracing::warn!(%server, prompt = %prompt.name, command = %name, "MCP prompt command name already taken; skipped");
        } else {
            registry.register(Arc::new(McpPromptCommand {
                name,
                description: if prompt.description.is_empty() {
                    "Run MCP prompt".into()
                } else {
                    prompt.description
                },
            }));
        }
    }
}
fn prompts_report(manager: &McpManager, selected: Option<&str>) -> String {
    let lines: Vec<_> = manager
        .prompts()
        .into_iter()
        .filter(|(server, _)| selected.map_or(true, |selected| selected == server))
        .map(|(server, prompt)| {
            format!("{} — {}", prompt.command_name(&server), prompt.description)
        })
        .collect();
    if lines.is_empty() {
        "No MCP prompts available.".into()
    } else {
        lines.join("\n")
    }
}
pub(super) struct McpCommand;
impl SlashCommand for McpCommand {
    fn name(&self) -> &'static str {
        "/mcp"
    }
    fn description(&self) -> &'static str {
        "Manage stdio MCP servers"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let _ = ctx.tx.send(TuiMessage::Mcp(args.into()));
    }
}
pub(in crate::interactive_tui) fn tools_report(
    manager: &McpManager,
    selected: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    for server in manager
        .snapshot()
        .into_iter()
        .filter(|s| selected.map_or(true, |name| s.name == name))
    {
        lines.push(format!("{}: {}", server.name, status(&server.status)));
        if selected.is_some() {
            if let Some(instructions) = manager.instructions(&server.name) {
                let instructions: String = instructions
                    .chars()
                    .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
                    .collect();
                lines.push(format!("Instructions:\n{instructions}"));
            }
        }
        for (name, approval) in manager.tools(&server.name) {
            lines.push(format!(
                "  {name}{}",
                if approval { " (approval required)" } else { "" }
            ));
        }
    }
    if lines.is_empty() {
        "No MCP servers configured.".into()
    } else {
        lines.join("\n")
    }
}
fn status(status: &ServerStatus) -> String {
    match status {
        ServerStatus::Disabled => "disabled".into(),
        ServerStatus::Connecting => "connecting".into(),
        ServerStatus::Connected { tools } => format!("connected · {} tools", tools.len()),
        ServerStatus::Idle { tools } => format!("idle (starts on use) · {} tools", tools.len()),
        ServerStatus::Failed(error) => format!("failed: {error}"),
    }
}
pub(in crate::interactive_tui) async fn apply(
    manager: &McpManager,
    args: &str,
) -> Result<String, String> {
    let parsed =
        shlex::split(args).ok_or("Invalid MCP command arguments: unmatched quote or escape")?;
    let words: Vec<_> = parsed.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["tools"] => return Ok(tools_report(manager, None)),
        ["tools", server] => return Ok(tools_report(manager, Some(server))),
        ["prompts"] => return Ok(prompts_report(manager, None)),
        ["prompts", server] => return Ok(prompts_report(manager, Some(server))),
        ["reconnect"] => manager.reconnect(None).await?,
        ["reconnect", name] => manager.reconnect(Some(name)).await?,
        ["enable", name] => manager.set_disabled(name, false).await?,
        ["disable", name] => manager.set_disabled(name, true).await?,
        ["remove", name] => manager.remove(name).await?,
        ["add", name, command, rest @ ..] => {
            manager
                .add(
                    name,
                    command,
                    &rest.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
                )
                .await?
        }
        _ => return Err(USAGE.into()),
    }
    Ok(tools_report(manager, None))
}
/// Open the server panel as a centered modal overlay over the chat.
pub(in crate::interactive_tui) fn panel(ctx: &CommandContext) {
    let manager = ctx.reload_context.mcp_tools.clone();
    let snapshot = manager.snapshot();
    // With nothing configured, start where pi-mcp-adapter does: setup.
    if snapshot.is_empty() {
        setup_panel(ctx);
        return;
    }
    let view = Arc::new(McpPanelView(Mutex::new(McpPanel::new(&snapshot))));
    let runtime = tokio::runtime::Handle::current();
    let context = ctx.clone();
    let panel = view.clone();
    let router = SelectorView::Custom(Arc::new(move |key| {
        let action = panel.0.lock().unwrap().handle_key(key);
        let close = || {
            close_selector(
                &context.state,
                &context.editor_container,
                &context.editor,
                &context.tui,
            )
        };
        match action {
            PanelAction::None => {}
            PanelAction::Close => close(),
            PanelAction::Save(changes) => {
                close();
                if !changes.is_empty() {
                    let _ = context.tx.send(TuiMessage::McpSave(changes));
                }
            }
            // Reconnect off the key thread; the panel stays open and refreshes.
            PanelAction::Reconnect(name) => {
                let (manager, panel, tui) = (manager.clone(), panel.clone(), context.tui.clone());
                runtime.spawn(async move {
                    let result = manager.reconnect(Some(&name)).await;
                    let mut panel = panel.0.lock().unwrap();
                    panel.refresh(&manager.snapshot());
                    if let Err(error) = result {
                        panel.set_notice(format!("Reconnect failed: {error}"));
                    }
                    drop(panel);
                    tui.request_render(false);
                });
            }
            PanelAction::CopyError(error) => {
                let notice = if run::copy_to_clipboard(&error) {
                    "Copied error to clipboard"
                } else {
                    "Could not copy error to clipboard"
                };
                panel.0.lock().unwrap().set_notice(notice.into());
            }
        }
        // The key loop does not repaint after routing Esc to a selector.
        context.tui.request_render(false);
    }));
    open_overlay_selector(
        &ctx.state,
        &ctx.editor,
        &ctx.tui,
        router,
        view,
        ice_tui::OverlayOptions {
            anchor: ice_tui::OverlayAnchor::Center,
            width: Some(ice_tui::tui::SizeValue::Absolute(mcp_panel::WIDTH)),
            ..Default::default()
        },
        SelectorKind::Tools,
    );
}

/// Open `/mcp setup`: import sources, presets, conflicts and skipped imports.
pub(in crate::interactive_tui) fn setup_panel(ctx: &CommandContext) {
    let manager = ctx.reload_context.mcp_tools.clone();
    let view = match manager.setup_view() {
        Ok(view) => view,
        Err(error) => {
            add_error_message(&ctx.chat, &error);
            ctx.tui.request_render(false);
            return;
        }
    };
    let view = Arc::new(SetupPanelView(Mutex::new(SetupPanel::new(
        view,
        dirs::home_dir(),
    ))));
    let runtime = tokio::runtime::Handle::current();
    let context = ctx.clone();
    let panel = view.clone();
    let router = SelectorView::Custom(Arc::new(move |key| {
        let action = panel.0.lock().unwrap().handle_key(key);
        match action {
            SetupAction::None => {}
            SetupAction::Close => close_selector(
                &context.state,
                &context.editor_container,
                &context.editor,
                &context.tui,
            ),
            // Writes and restarts run off the key thread; the panel stays open.
            action => {
                let (manager, panel, tui) = (manager.clone(), panel.clone(), context.tui.clone());
                runtime.spawn(async move {
                    let result = match action {
                        SetupAction::SetImport(kind, enabled) => {
                            manager.set_import(kind, enabled).await.map(|()| {
                                if enabled {
                                    format!("Importing {kind}.")
                                } else {
                                    format!("Stopped importing {kind}.")
                                }
                            })
                        }
                        SetupAction::AddPreset(id) => manager
                            .add_preset(id)
                            .await
                            .map(|()| format!("Added {id}.")),
                        SetupAction::None | SetupAction::Close => return,
                    };
                    panel.0.lock().unwrap().finish(manager.setup_view(), result);
                    tui.request_render(false);
                });
            }
        }
        context.tui.request_render(false);
    }));
    open_overlay_selector(
        &ctx.state,
        &ctx.editor,
        &ctx.tui,
        router,
        view,
        ice_tui::OverlayOptions {
            anchor: ice_tui::OverlayAnchor::Center,
            width: Some(ice_tui::tui::SizeValue::Absolute(mcp_setup_panel::WIDTH)),
            ..Default::default()
        },
        SelectorKind::Tools,
    );
}

#[cfg(test)]
#[path = "mcp_prompt_tests.rs"]
mod prompt_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use ice_tui::AutocompleteProvider;

    #[tokio::test]
    async fn mcp_parsing_and_live_name_completion() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        let dir = tempfile::tempdir().unwrap();
        let provider = FauxProvider::new(FauxScript::new());
        let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider],
            ..Default::default()
        })
        .await
        .unwrap();
        let manager = McpManager::new(
            &harness,
            &crate::args::Args::default(),
            dir.path(),
            dir.path().into(),
            false,
            Arc::new(Mutex::new(Default::default())),
        );
        for input in [
            "wat",
            "enable",
            "disable a b",
            "remove",
            "add name",
            "reconnect a b",
        ] {
            assert_eq!(apply(&manager, input).await.unwrap_err(), USAGE);
        }
        assert!(
            apply(&manager, "add demo icers-missing-mcp-fixture --stdio")
                .await
                .unwrap()
                .contains("demo: failed:")
        );
        apply(
            &manager,
            r#"add quoted 'icers missing fixture' --label "two words" '' escaped\ value "C:\Program Files\mcp""#,
        )
        .await
        .unwrap();
        let config_path = dir.path().join("mcp.json");
        let config = std::fs::read_to_string(&config_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(
            value["mcpServers"]["quoted"]["command"],
            "icers missing fixture"
        );
        assert_eq!(
            value["mcpServers"]["quoted"]["args"],
            serde_json::json!([
                "--label",
                "two words",
                "",
                "escaped value",
                r"C:\Program Files\mcp"
            ])
        );
        for input in [
            r#"add broken command "unterminated"#,
            "remove 'demo",
            "add broken command \\",
        ] {
            assert!(apply(&manager, input).await.is_err());
            assert_eq!(std::fs::read_to_string(&config_path).unwrap(), config);
        }
        apply(&manager, "remove quoted").await.unwrap();
        let provider = autocomplete::McpAutocomplete {
            manager: Some(manager.clone()),
        };
        let input = "/mcp dis";
        assert_eq!(
            provider.get_suggestions(input, input.len()).unwrap().items[0].text,
            "disable"
        );
        let input = "/mcp pro";
        assert_eq!(
            provider.get_suggestions(input, input.len()).unwrap().items[0].text,
            "prompts"
        );
        for command in ["reconnect", "enable", "disable", "remove", "prompts"] {
            let input = format!("/mcp {command} de");
            let suggestions = provider.get_suggestions(&input, input.len()).unwrap();
            assert_eq!(suggestions.items[0].text, "demo");
            assert_eq!(suggestions.start, input.len() - 2);
        }
        assert!(apply(&manager, "disable demo")
            .await
            .unwrap()
            .contains("demo: disabled"));
        assert!(apply(&manager, "enable demo")
            .await
            .unwrap()
            .contains("demo: failed:"));
        assert!(apply(&manager, "reconnect demo")
            .await
            .unwrap()
            .contains("demo: failed:"));
        assert!(apply(&manager, "tools").await.unwrap().contains("demo"));
        assert!(apply(&manager, "remove demo")
            .await
            .unwrap()
            .contains("No MCP"));
        assert!(manager.config_path(true).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path().join("mcp.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
