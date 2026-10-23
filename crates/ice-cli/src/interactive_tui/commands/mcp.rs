use super::*;
use crate::mcp::{McpManager, ServerStatus};

const USAGE: &str = "/mcp [status|tools|reconnect [server]|enable <server>|disable <server>|add <name> <command> [args...]|remove <name>|edit [project]]";
pub(super) struct McpCommand;
impl SlashCommand for McpCommand {
    fn name(&self) -> &'static str {
        "/mcp"
    }
    fn description(&self) -> &'static str {
        "Manage stdio MCP servers"
    }
    fn execute(&self, ctx: &CommandContext, args: &str) {
        let _ = ctx.tx.send(TuiMessage::Mcp(args.into(), false));
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
/// `selected` keeps the cursor on the server an action just targeted when reopening.
pub(in crate::interactive_tui) fn panel(ctx: &CommandContext, selected: Option<&str>) {
    let manager = ctx.reload_context.mcp_tools.clone();
    let snapshot = manager.snapshot();
    if snapshot.is_empty() {
        add_note_message(
            &ctx.chat,
            &format!("No MCP servers configured. Use /mcp add or /mcp edit.\n{USAGE}"),
        );
        ctx.tui.request_render(false);
        return;
    }
    let index = selected
        .and_then(|name| snapshot.iter().position(|s| s.name == name))
        .unwrap_or(0);
    let items = snapshot
        .into_iter()
        .map(|s| {
            SelectItem::new(&s.name, &format!("{} — {}", s.name, status(&s.status)))
                .with_description(&s.source.display().to_string())
        })
        .collect();
    let list = Arc::new(SelectList::new(items, 10));
    list.set_selected_index(index);
    let context = ctx.clone();
    let selected_manager = manager.clone();
    list.on_select(Arc::new(move |item| {
        add_note_message(
            &context.chat,
            &tools_report(&selected_manager, Some(&item.value)),
        );
        context.tui.request_render(false);
    }));
    let context = ctx.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(
            &context.state,
            &context.editor_container,
            &context.editor,
            &context.tui,
        )
    }));
    let view = Arc::new(Container::new());
    view.add_child(list.clone());
    view.add_child(Arc::new(Text::new(
        "enter tools · ctrl+r reconnect · ctrl+d enable/disable · e edit · esc close",
        0,
        0,
    )));
    let context = ctx.clone();
    let key_list = list.clone();
    let router = SelectorView::Custom(Arc::new(move |key| {
        let action = match (key.code, key.modifiers.contains(KeyModifiers::CONTROL)) {
            (KeyCode::Char('r'), true) => key_list
                .get_selected_item()
                .map(|i| format!("reconnect {}", i.value)),
            (KeyCode::Char('d'), true) => key_list.get_selected_item().map(|i| {
                let disabled = manager
                    .snapshot()
                    .iter()
                    .any(|s| s.name == i.value && s.disabled);
                format!(
                    "{} {}",
                    if disabled { "enable" } else { "disable" },
                    i.value
                )
            }),
            (KeyCode::Char('e'), false) => Some(
                if key_list
                    .get_selected_item()
                    .and_then(|i| manager.snapshot().into_iter().find(|s| s.name == i.value))
                    .map_or(false, |s| {
                        s.source == context.reload_context.cwd.join(".icers/mcp.json")
                    })
                {
                    "edit project"
                } else {
                    "edit"
                }
                .into(),
            ),
            _ => None,
        };
        if let Some(action) = action {
            let _ = context.tx.send(TuiMessage::Mcp(action, true));
        } else if let Some(key) = ice_tui::get_keybindings().selection_key(&key) {
            key_list.handle_key(key);
        }
    }));
    open_selector_with_view(
        &ctx.state,
        &ctx.editor_container,
        &ctx.editor,
        &ctx.tui,
        router,
        view,
        SelectorKind::Tools,
    );
}

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
        for command in ["reconnect", "enable", "disable", "remove"] {
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
