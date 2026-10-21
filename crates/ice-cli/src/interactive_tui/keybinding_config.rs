//! Load effective keybindings without rewriting user configuration.
//! TS reference: core/keybindings.ts and keybindings-migration.test.ts.

use super::*;

type RawBindings = serde_json::Map<String, serde_json::Value>;

/// Legacy names are resolved in memory. A namespaced key in the same source wins.
fn canonical_name(name: &str) -> String {
    let app = match name {
        "interrupt" => Some("app.interrupt"),
        "clear" => Some("app.clear"),
        "exit" => Some("app.exit"),
        "suspend" => Some("app.suspend"),
        "cycleThinkingLevel" => Some("app.thinking.cycle"),
        "cycleModelForward" => Some("app.model.cycleForward"),
        "cycleModelBackward" => Some("app.model.cycleBackward"),
        "selectModel" => Some("app.model.select"),
        "expandTools" => Some("app.tools.expand"),
        "toggleThinking" => Some("app.thinking.toggle"),
        "externalEditor" => Some("app.editor.external"),
        "followUp" => Some("app.message.followUp"),
        "sendNow" => Some("app.message.sendNow"),
        "dequeue" => Some("app.message.dequeue"),
        "pasteImage" => Some("app.clipboard.pasteImage"),
        "newSession" => Some("app.session.new"),
        "tree" => Some("app.session.tree"),
        "fork" => Some("app.session.fork"),
        "resume" => Some("app.session.resume"),
        "renameSession" => Some("app.session.rename"),
        "deleteSession" => Some("app.session.delete"),
        "toggleSessionPath" => Some("app.session.togglePath"),
        "toggleSessionSort" => Some("app.session.toggleSort"),
        "toggleSessionNamedFilter" => Some("app.session.toggleNamedFilter"),
        "treeFoldOrUp" => Some("app.tree.foldOrUp"),
        "treeUnfoldOrDown" => Some("app.tree.unfoldOrDown"),
        "treeEditLabel" => Some("app.tree.editLabel"),
        "treeToggleLabelTimestamp" => Some("app.tree.toggleLabelTimestamp"),
        "deleteSessionNoninvasive" => Some("app.session.deleteNoninvasive"),
        _ => None,
    };
    if let Some(name) = app {
        return name.into();
    }
    if matches!(name, "newLine" | "submit" | "tab" | "copy") {
        return format!("tui.input.{name}");
    }
    if let Some(action) = name.strip_prefix("select") {
        if let Some(first) = action.chars().next() {
            return format!(
                "tui.select.{}{}",
                first.to_ascii_lowercase(),
                &action[first.len_utf8()..]
            );
        }
    }
    if name.contains('.') {
        name.into()
    } else {
        format!("tui.editor.{name}")
    }
}

fn apply_bindings(bindings: &mut ice_tui::Keybindings, raw: &RawBindings, allow_null: bool) {
    let known: HashMap<_, _> = bindings
        .definitions()
        .map(|(id, _)| (id.to_string(), id))
        .collect();
    for (name, value) in raw {
        let canonical = canonical_name(name);
        if canonical != *name && raw.contains_key(&canonical) {
            continue;
        }
        let Some(&id) = known.get(&canonical) else {
            continue;
        };
        let values: Option<Vec<&str>> = match value {
            serde_json::Value::String(value) => Some(vec![value.as_str()]),
            serde_json::Value::Array(values) => values.iter().map(|value| value.as_str()).collect(),
            serde_json::Value::Null if allow_null => Some(Vec::new()),
            _ => None,
        };
        let Some(values) = values else { continue };
        // Reject the whole malformed entry; never silently turn invalid keys
        // into an empty binding or a partial array.
        let combos: Option<Vec<_>> = values
            .iter()
            .map(|value| parse_configured_key(value))
            .collect();
        if let Some(combos) = combos {
            bindings.set(id, combos);
        }
    }
}

pub(super) fn load_keybindings(
    settings: Option<&HashMap<String, serde_json::Value>>,
    path: Option<&std::path::Path>,
) -> ice_tui::Keybindings {
    let mut bindings = ice_tui::Keybindings::new();
    if let Some(settings) = settings {
        let raw = settings
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        apply_bindings(&mut bindings, &raw, true);
    }
    if let Some(raw) = path
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<RawBindings>(&bytes).ok())
    {
        apply_bindings(&mut bindings, &raw, false);
    }
    bindings
}

pub(super) fn effective_hotkey_rows(
    bindings: &ice_tui::Keybindings,
) -> Vec<(String, String, String)> {
    let mut rows: Vec<_> = bindings
        .definitions()
        .map(|(id, definition)| {
            let keys = bindings.get_keys(id);
            let display = if keys.is_empty() {
                "(unbound)".into()
            } else {
                keys.iter()
                    .map(|key| key.display())
                    .collect::<Vec<_>>()
                    .join(" / ")
            };
            (
                id.to_string(),
                display,
                definition.description.unwrap_or(id).to_string(),
            )
        })
        .collect();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(
        raw: &str,
        settings: Option<&HashMap<String, serde_json::Value>>,
    ) -> ice_tui::Keybindings {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keybindings.json");
        std::fs::write(&path, raw).unwrap();
        let bindings = load_keybindings(settings, Some(&path));
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            raw,
            "loading must not rewrite the user's file"
        );
        bindings
    }

    #[test]
    fn keybindings_file_parser_normalizes_terminal_keys_and_rejects_invalid_modifiers() {
        let mut bindings = ice_tui::Keybindings::new();
        bindings.set(
            "tui.select.confirm",
            vec![parse_configured_key("shift+tab").unwrap()],
        );
        assert!(bindings.matches(
            &KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            "tui.select.confirm"
        ));
        assert!(bindings.matches(
            &KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
            "tui.select.confirm"
        ));
        assert_eq!(
            bindings.get_keys("tui.select.confirm")[0].display(),
            "Shift+Tab"
        );
        assert_eq!(
            parse_configured_key("ctrl++").unwrap().code,
            KeyCode::Char('+')
        );
        assert_eq!(parse_configured_key("f24").unwrap().code, KeyCode::F(24));
        assert!(parse_configured_key("typo+f6").is_none());
        assert!(parse_configured_key("ctrl+").is_none());
        assert!(parse_configured_key("f25").is_none());
    }

    #[test]
    fn keybindings_file_migrates_legacy_and_namespaced_wins() {
        // TS: loads old names before rewrite; keeps namespaced values.
        let bindings = load(
            r#"{
            "cursorUp": ["up", "ctrl+p"],
            "expandTools": "ctrl+x",
            "app.tools.expand": "ctrl+y",
            "selectConfirm": "alt+enter"
        }"#,
            None,
        );
        assert!(bindings.matches(
            &KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            "tui.editor.cursorUp"
        ));
        assert!(bindings.matches(
            &KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            "tui.editor.cursorUp"
        ));
        assert!(bindings.matches(
            &KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
            "app.tools.expand"
        ));
        assert!(!bindings.matches(
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            "app.tools.expand"
        ));
        assert!(bindings.matches(
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
            "tui.select.confirm"
        ));
    }

    #[test]
    fn keybindings_file_overrides_settings_and_empty_array_unbinds() {
        let settings = serde_json::from_value(serde_json::json!({
            "app.model.select": "alt+m",
            "app.thinking.toggle": "alt+t"
        }))
        .unwrap();
        let bindings = load(
            r#"{"app.model.select":[],"app.thinking.toggle":["f6","ctrl+t"]}"#,
            Some(&settings),
        );
        assert!(bindings.get_keys("app.model.select").is_empty());
        assert!(bindings.matches(
            &KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE),
            "app.thinking.toggle"
        ));
        assert!(!bindings.matches(
            &KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT),
            "app.thinking.toggle"
        ));
    }

    #[test]
    fn keybindings_file_invalid_data_falls_back_without_partial_unbinding() {
        let settings =
            serde_json::from_value(serde_json::json!({"app.model.select":"alt+m"})).unwrap();
        for raw in [
            "{",
            "[]",
            "null",
            r#"{"app.model.select":["f6",false]}"#,
            r#"{"app.model.select":["f6","ctrl+not-a-key"]}"#,
            r#"{"app.model.select":null}"#,
        ] {
            let bindings = load(raw, Some(&settings));
            assert_eq!(
                bindings.get_keys("app.model.select"),
                vec![ice_tui::KeyCombo::new(
                    KeyCode::Char('m'),
                    KeyModifiers::ALT
                )],
                "{raw}"
            );
        }
        let defaults = load_keybindings(
            None,
            Some(std::path::Path::new(
                "/nonexistent/icers-test/keybindings.json",
            )),
        );
        assert!(!defaults.get_keys("app.model.select").is_empty());
    }

    #[test]
    fn keybindings_file_hotkeys_uses_effective_keys_and_marks_unbound() {
        let bindings = load(r#"{"app.model.select":"alt+m","tui.editor.undo":[]}"#, None);
        let rows = effective_hotkey_rows(&bindings);
        assert_eq!(
            rows.iter()
                .find(|row| row.0 == "app.model.select")
                .unwrap()
                .1,
            "Alt+M"
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.0 == "tui.editor.undo")
                .unwrap()
                .1,
            "(unbound)"
        );
        assert!(rows.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn keybindings_file_editor_uses_the_loaded_override() {
        let bindings = load(r#"{"tui.editor.cursorLeft":"alt+h"}"#, None);
        let editor = Editor::new(
            EditorOptions::default(),
            EditorStyle::default(),
            Arc::new(bindings),
        );
        editor.set_text("ab");
        editor.set_cursor(0, 2);
        assert!(!editor.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
        editor.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::ALT));
        editor.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
        assert_eq!(editor.get_text(), "aXb");
    }

    #[test]
    fn keybindings_file_editor_submit_newline_and_kill_use_effective_keys() {
        let bindings = load(
            r#"{"tui.input.submit":"alt+enter","tui.input.newLine":"ctrl+n","tui.editor.deleteWordBackward":"alt+w"}"#,
            None,
        );
        let editor = Editor::new(
            EditorOptions::default(),
            EditorStyle::default(),
            Arc::new(bindings),
        );
        let submitted = Arc::new(Mutex::new(Vec::new()));
        let submitted_cb = submitted.clone();
        editor.on_submit(Arc::new(move |text| {
            submitted_cb.lock().unwrap().push(text.to_string())
        }));
        editor.set_text("one two");
        editor.set_cursor(0, 7);
        assert!(!editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert!(!editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)));
        assert!(submitted.lock().unwrap().is_empty());
        editor.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT));
        assert_eq!(editor.get_text(), "one ");
        editor.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(editor.get_text(), "one \n");
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        assert_eq!(*submitted.lock().unwrap(), vec!["one \n".to_string()]);
    }

    #[test]
    fn keybindings_file_selector_uses_override_and_releases_default() {
        let bindings = load(
            r#"{"tui.select.down":"alt+j","tui.select.confirm":"alt+enter"}"#,
            None,
        );
        let list = Arc::new(SelectList::new(
            vec![SelectItem::new("a", "A"), SelectItem::new("b", "B")],
            5,
        ));
        let picker = SearchableSelectList::new(None, None, list.clone());
        let picked = Arc::new(Mutex::new(None));
        let picked_cb = picked.clone();
        list.on_select(Arc::new(move |item| {
            *picked_cb.lock().unwrap() = Some(item.value.clone())
        }));
        picker
            .handle_key_with_bindings(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &bindings);
        assert_eq!(list.get_selected_item().unwrap().value, "a");
        picker.handle_key_with_bindings(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT),
            &bindings,
        );
        picker
            .handle_key_with_bindings(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &bindings);
        assert!(picked.lock().unwrap().is_none());
        picker
            .handle_key_with_bindings(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), &bindings);
        assert_eq!(*picked.lock().unwrap(), Some("b".into()));
    }
}
