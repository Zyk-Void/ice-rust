//! Session-only model scope with an explicit apply/save/cancel boundary.
//! Ports scoped-models-selector.ts; unlike its immediate onChange callback,
//! goal.md requires Escape to discard the working selection.

use super::*;

type Scope = Option<Vec<String>>;
type ApplyScope = Arc<dyn Fn(Scope, bool) -> Result<(), String> + Send + Sync>;

fn full_id(model: &ice_ai::Model) -> String {
    format!("{}/{}", model.provider, model.id)
}

/// Preserve saved ordering, resolve legacy bare ids, and retain unavailable ids.
fn canonical_scope(catalog: &[ice_ai::Model], scope: Scope) -> Scope {
    scope.filter(|ids| !ids.is_empty()).map(|ids| {
        let mut resolved = Vec::new();
        for id in ids {
            let matches: Vec<_> = catalog
                .iter()
                .filter(|model| {
                    id.eq_ignore_ascii_case(&full_id(model)) || id.eq_ignore_ascii_case(&model.id)
                })
                .map(full_id)
                .collect();
            for item in if matches.is_empty() {
                vec![id]
            } else {
                matches
            } {
                if !resolved.contains(&item) {
                    resolved.push(item);
                }
            }
        }
        resolved
    })
}

pub(super) struct ScopedModelsSelector {
    catalog: Vec<ice_ai::Model>,
    // Keep unavailable opening choices addressable even after disabling them.
    unavailable: Vec<String>,
    bindings: ice_tui::Keybindings,
    working: Mutex<Scope>,
    input: Input,
    list: Mutex<Arc<SelectList>>,
    apply: ApplyScope,
    cancel: Arc<dyn Fn() + Send + Sync>,
    error: Mutex<Option<String>>,
}

impl ScopedModelsSelector {
    fn new(
        catalog: Vec<ice_ai::Model>,
        scope: Scope,
        bindings: ice_tui::Keybindings,
        apply: ApplyScope,
        cancel: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let working = canonical_scope(&catalog, scope);
        let unavailable = working
            .iter()
            .flatten()
            .filter(|id| !catalog.iter().any(|model| full_id(model) == **id))
            .cloned()
            .collect();
        let selector = Self {
            catalog,
            unavailable,
            bindings,
            working: Mutex::new(working),
            input: Input::with_placeholder("Search models or providers…"),
            list: Mutex::new(Arc::new(SelectList::new(Vec::new(), 8))),
            apply,
            cancel,
            error: Mutex::new(None),
        };
        selector.input.set_focused(true);
        selector.refresh(None);
        selector
    }

    fn all_ids(&self) -> Vec<String> {
        self.catalog
            .iter()
            .map(full_id)
            .chain(self.unavailable.iter().cloned())
            .collect()
    }

    fn items(&self) -> Vec<SelectItem> {
        let working = self.working.lock().unwrap().clone();
        let mut ids = working.clone().unwrap_or_default();
        for id in self.all_ids() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let items: Vec<_> = ids
            .iter()
            .map(|id| {
                let model = self.catalog.iter().find(|model| full_id(model) == *id);
                let enabled = working.as_ref().map_or(true, |scope| scope.contains(id));
                let label = format!("{} {}", if enabled { "✓" } else { "·" }, id);
                let description = model.map_or("unavailable", |model| model.name.as_str());
                SelectItem::new(id, &label)
                    .with_description(description)
                    .with_search_text(&format!("{id} {description}"))
            })
            .collect();
        ice_tui::fuzzy_filter(&items, &self.input.get_value(), |item| item.search_key())
    }

    fn refresh(&self, selected: Option<&str>) {
        let items = self.items();
        let index = selected.and_then(|id| items.iter().position(|item| item.value == id));
        let list = Arc::new(SelectList::new(items, 8));
        if let Some(index) = index {
            list.set_selected_index(index);
        }
        *self.list.lock().unwrap() = list;
    }

    fn selected(&self) -> Option<String> {
        self.list
            .lock()
            .unwrap()
            .get_selected_item()
            .map(|item| item.value)
    }

    fn toggle(&self, id: &str) {
        let mut working = self.working.lock().unwrap();
        match working.as_mut() {
            None => *working = Some(vec![id.to_string()]),
            Some(ids) => {
                if let Some(index) = ids.iter().position(|item| item == id) {
                    ids.remove(index);
                } else {
                    ids.push(id.to_string());
                }
            }
        }
    }

    fn set_enabled(&self, targets: &[String], enabled: bool) {
        let mut working = self.working.lock().unwrap();
        if enabled {
            if let Some(ids) = working.as_mut() {
                for id in targets {
                    if !ids.contains(id) {
                        ids.push(id.clone());
                    }
                }
            }
        } else {
            let ids = working.get_or_insert_with(|| self.all_ids());
            ids.retain(|id| !targets.contains(id));
        }
    }

    fn reorder(&self, id: &str, delta: isize) {
        let mut working = self.working.lock().unwrap();
        let Some(ids) = working.as_mut() else { return };
        let Some(index) = ids.iter().position(|item| item == id) else {
            return;
        };
        let next = index as isize + delta;
        if next >= 0 && (next as usize) < ids.len() {
            ids.swap(index, next as usize);
        }
    }

    pub(super) fn handle_key(&self, key: KeyEvent) {
        let bindings = &self.bindings;
        let matches = |id| keybinding_matches(&bindings, &key, id);
        if key.code == KeyCode::Char('c')
            && key.modifiers == KeyModifiers::CONTROL
            && !self.input.get_value().is_empty()
        {
            self.input.set_value("");
            self.refresh(None);
            return;
        }
        if matches("tui.select.cancel")
            || (key.code == KeyCode::Char('c')
                && key.modifiers == KeyModifiers::CONTROL
                && self.input.get_value().is_empty())
        {
            (self.cancel)();
            return;
        }
        if matches("tui.select.confirm") || matches("app.models.save") {
            let scope = self
                .working
                .lock()
                .unwrap()
                .clone()
                .filter(|ids| !ids.is_empty());
            if let Err(error) = (self.apply)(scope, matches("app.models.save")) {
                *self.error.lock().unwrap() = Some(error);
            }
            return;
        }
        let selected = self.selected();
        if matches("app.models.toggle") {
            if let Some(id) = &selected {
                self.toggle(id);
            }
        } else if matches("app.models.reorderUp") || matches("app.models.reorderDown") {
            if let Some(id) = &selected {
                self.reorder(
                    id,
                    if matches("app.models.reorderUp") {
                        -1
                    } else {
                        1
                    },
                );
            }
        } else if matches("app.models.enableAll") || matches("app.models.clearAll") {
            let enabled = matches("app.models.enableAll");
            if enabled && self.input.get_value().is_empty() {
                *self.working.lock().unwrap() = None;
            } else {
                let targets: Vec<_> = self.items().into_iter().map(|item| item.value).collect();
                self.set_enabled(&targets, enabled);
            }
        } else if matches("app.models.toggleProvider") {
            if let Some(model) = selected
                .as_ref()
                .and_then(|id| self.catalog.iter().find(|model| full_id(model) == *id))
            {
                let targets: Vec<_> = self
                    .catalog
                    .iter()
                    .filter(|other| other.provider == model.provider)
                    .map(full_id)
                    .collect();
                let enabled = self
                    .working
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map_or(true, |ids| targets.iter().all(|id| ids.contains(id)));
                self.set_enabled(&targets, !enabled);
            }
        } else if matches("tui.select.up")
            || matches("tui.select.down")
            || matches("tui.select.pageUp")
            || matches("tui.select.pageDown")
        {
            let normalized = if matches("tui.select.up") {
                KeyCode::Up
            } else if matches("tui.select.down") {
                KeyCode::Down
            } else if matches("tui.select.pageUp") {
                KeyCode::PageUp
            } else {
                KeyCode::PageDown
            };
            self.list
                .lock()
                .unwrap()
                .handle_key(KeyEvent::new(normalized, KeyModifiers::NONE));
            return;
        } else {
            if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                self.input.set_value("");
            } else {
                self.input.handle_key(key);
            }
            self.refresh(None);
            return;
        }
        *self.error.lock().unwrap() = None;
        self.refresh(selected.as_deref());
    }
}

impl Component for ScopedModelsSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let bindings = &self.bindings;
        let hint = |id| {
            let keys = bindings.get_keys(id);
            if keys.is_empty() {
                "unbound".to_string()
            } else {
                keys.iter()
                    .map(|key| key.display())
                    .collect::<Vec<_>>()
                    .join("/")
            }
        };
        let scope = self.working.lock().unwrap().clone();
        let count = scope.as_ref().map_or_else(
            || "All models".to_string(),
            |ids| {
                if ids.is_empty() {
                    "0 selected; apply restores all models".to_string()
                } else {
                    format!("{} selected, in cycling order", ids.len())
                }
            },
        );
        let mut lines = Text::new(format!("Model scope — {count}"), 0, 0).render(width);
        lines.extend(self.input.render(width));
        lines.extend(self.list.lock().unwrap().render(width));
        let help = format!(
            "{} toggle · {} apply to session · {} save · {} cancel\n{} all · {} clear · {} provider · {}/{} reorder",
            hint("app.models.toggle"), hint("tui.select.confirm"), hint("app.models.save"),
            hint("tui.select.cancel"), hint("app.models.enableAll"), hint("app.models.clearAll"),
            hint("app.models.toggleProvider"), hint("app.models.reorderUp"), hint("app.models.reorderDown"),
        );
        lines.extend(Text::new(help, 0, 0).render(width));
        if let Some(error) = self.error.lock().unwrap().as_ref() {
            lines.extend(
                Text::new(format!("Could not save model scope: {error}"), 0, 0).render(width),
            );
        }
        lines
    }

    fn invalidate(&self) {
        self.input.invalidate();
        self.list.lock().unwrap().invalidate();
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Commit runtime state only after a requested durable save succeeds.
fn apply_scope(runtime: &Mutex<Scope>, scope: Scope, persist: bool) -> Result<(), String> {
    if persist {
        let mut settings = crate::settings::load_settings().map_err(|error| error.to_string())?;
        settings.scoped_models = scope.clone();
        crate::settings::save_settings(&settings)?;
    }
    *runtime.lock().unwrap() = scope;
    Ok(())
}

pub(super) fn open_scoped_models_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    catalog: &[ice_ai::Model],
    chat: &Arc<Container>,
) {
    if catalog.is_empty() {
        add_note_message(chat, "No models in the catalog.");
        tui.request_render(false);
        return;
    }
    let (state_apply, ec_apply, editor_apply, tui_apply, chat_apply) = (
        state.clone(),
        editor_container.clone(),
        editor.clone(),
        tui.clone(),
        chat.clone(),
    );
    let apply: ApplyScope = Arc::new(move |scope, persist| {
        apply_scope(&state_apply.scoped_models, scope, persist)?;
        add_note_message(
            &chat_apply,
            if persist {
                "Model scope saved to settings and applied to this session."
            } else {
                "Model scope applied to this session only."
            },
        );
        close_selector(&state_apply, &ec_apply, &editor_apply, &tui_apply);
        Ok(())
    });
    let (state_cancel, ec_cancel, editor_cancel, tui_cancel) = (
        state.clone(),
        editor_container.clone(),
        editor.clone(),
        tui.clone(),
    );
    let cancel = Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    });
    let selector = Arc::new(ScopedModelsSelector::new(
        catalog.to_vec(),
        state.scoped_models.lock().unwrap().clone(),
        ice_tui::get_keybindings().clone(),
        apply,
        cancel,
    ));
    let keys = selector.clone();
    open_selector_with_view(
        state,
        editor_container,
        editor,
        tui,
        SelectorView::Custom(Arc::new(move |key| keys.handle_key(key))),
        selector,
        SelectorKind::ScopedModels,
    );
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_isolated_config_dir;
    use super::*;

    fn catalog() -> Vec<ice_ai::Model> {
        vec![
            ice_ai::Model::new("alpha", "First model", ice_ai::Api::Faux, "p", ""),
            ice_ai::Model::new("beta", "Second model", ice_ai::Api::Faux, "p", ""),
            ice_ai::Model::new("alpha", "Other model", ice_ai::Api::Faux, "q", ""),
        ]
    }

    fn widget(seed: Scope, runtime: Arc<Mutex<Scope>>) -> ScopedModelsSelector {
        ScopedModelsSelector::new(
            catalog(),
            seed,
            ice_tui::Keybindings::new(),
            Arc::new(move |scope, persist| apply_scope(&runtime, scope, persist)),
            Arc::new(|| {}),
        )
    }

    fn key(widget: &ScopedModelsSelector, code: KeyCode, modifiers: KeyModifiers) {
        widget.handle_key(KeyEvent::new(code, modifiers));
    }

    // goal.md cancellation deliberately differs from TS's immediate onChange.
    #[test]
    fn scoped_models_escape_discards_working_copy_and_never_writes() {
        with_isolated_config_dir(|| {
            let initial = Some(vec!["p/beta".to_string()]);
            let mut settings = crate::settings::Settings::default();
            settings.scoped_models = initial.clone();
            crate::settings::save_settings(&settings).unwrap();
            let runtime = Arc::new(Mutex::new(initial.clone()));
            let selector = widget(initial.clone(), runtime.clone());
            key(&selector, KeyCode::Char(' '), KeyModifiers::NONE);
            assert_eq!(
                *selector.working.lock().unwrap(),
                Some(Vec::<String>::new())
            );
            key(&selector, KeyCode::Esc, KeyModifiers::NONE);
            assert_eq!(*runtime.lock().unwrap(), initial);
            assert_eq!(
                crate::settings::load_settings().unwrap().scoped_models,
                initial
            );
        });
    }

    #[test]
    fn scoped_models_enter_applies_session_only_and_reopening_uses_it() {
        with_isolated_config_dir(|| {
            let runtime = Arc::new(Mutex::new(None));
            let selector = widget(None, runtime.clone());
            key(&selector, KeyCode::Char(' '), KeyModifiers::NONE);
            key(&selector, KeyCode::Enter, KeyModifiers::NONE);
            assert_eq!(*runtime.lock().unwrap(), Some(vec!["p/alpha".into()]));
            assert_eq!(
                crate::settings::load_settings().unwrap().scoped_models,
                None
            );
            let reopened = widget(runtime.lock().unwrap().clone(), runtime.clone());
            assert_eq!(
                *reopened.working.lock().unwrap(),
                Some(vec!["p/alpha".into()])
            );
        });
    }

    // scoped-models-selector.ts: move and getSortedIds preserve enabled order.
    #[test]
    fn scoped_models_save_keeps_reorder_and_unavailable_ids() {
        with_isolated_config_dir(|| {
            let runtime = Arc::new(Mutex::new(None));
            let selector = widget(
                Some(vec!["p/alpha".into(), "gone/model".into(), "p/beta".into()]),
                runtime.clone(),
            );
            key(&selector, KeyCode::Down, KeyModifiers::NONE);
            key(&selector, KeyCode::Down, KeyModifiers::NONE);
            key(&selector, KeyCode::Up, KeyModifiers::ALT);
            key(&selector, KeyCode::Char('s'), KeyModifiers::CONTROL);
            let expected = Some(vec!["p/alpha".into(), "p/beta".into(), "gone/model".into()]);
            assert_eq!(*runtime.lock().unwrap(), expected);
            assert_eq!(
                crate::settings::load_settings().unwrap().scoped_models,
                expected
            );
        });
    }

    #[test]
    fn scoped_models_unavailable_rows_survive_toggle_clear_and_enable_all() {
        let selector = widget(
            Some(vec!["gone/model".into(), "p/beta".into()]),
            Arc::new(Mutex::new(None)),
        );
        key(&selector, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(selector.selected().as_deref(), Some("gone/model"));
        assert!(!selector
            .working
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains(&"gone/model".into()));
        key(&selector, KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(selector
            .working
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains(&"gone/model".into()));
        key(&selector, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(selector.items().len(), 4);
        key(&selector, KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(*selector.working.lock().unwrap(), None);
        assert_eq!(selector.items().len(), 4);
        for ch in "gone/model".chars() {
            key(&selector, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(selector.selected().as_deref(), Some("gone/model"));
        key(&selector, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(selector.items().len(), 1);
        assert!(!selector
            .working
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains(&"gone/model".into()));
        key(&selector, KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert!(selector
            .working
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains(&"gone/model".into()));
    }

    #[test]
    fn scoped_models_failed_save_preserves_runtime_and_working_selection() {
        with_isolated_config_dir(|| {
            let runtime = Arc::new(Mutex::new(None));
            let selector = widget(None, runtime.clone());
            key(&selector, KeyCode::Char(' '), KeyModifiers::NONE);
            let dir = std::env::var_os(crate::config::CONFIG_DIR_ENV).unwrap();
            std::fs::create_dir(std::path::Path::new(&dir).join("settings.json")).unwrap();
            key(&selector, KeyCode::Char('s'), KeyModifiers::CONTROL);
            assert_eq!(*runtime.lock().unwrap(), None);
            assert_eq!(
                *selector.working.lock().unwrap(),
                Some(vec!["p/alpha".into()])
            );
            assert!(selector.error.lock().unwrap().is_some());
        });
    }

    // TS search includes both provider/id and the friendly model name.
    #[test]
    fn scoped_models_search_filtered_all_and_clear_leave_other_models_alone() {
        let selector = widget(Some(vec!["q/alpha".into()]), Arc::new(Mutex::new(None)));
        for ch in "Second".chars() {
            key(&selector, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(selector.selected(), Some("p/beta".into()));
        key(&selector, KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(
            *selector.working.lock().unwrap(),
            Some(vec!["q/alpha".into(), "p/beta".into()])
        );
        key(&selector, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(
            *selector.working.lock().unwrap(),
            Some(vec!["q/alpha".into()])
        );
        key(&selector, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(selector.input.get_value(), "");
        assert_eq!(selector.items().len(), 3);
    }

    #[test]
    fn scoped_models_unmatched_search_and_edge_reorders_are_safe() {
        let selector = widget(
            Some(vec!["p/alpha".into(), "p/beta".into()]),
            Arc::new(Mutex::new(None)),
        );
        key(&selector, KeyCode::Up, KeyModifiers::ALT);
        assert_eq!(
            *selector.working.lock().unwrap(),
            Some(vec!["p/alpha".into(), "p/beta".into()])
        );
        for ch in "no-such-model".chars() {
            key(&selector, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert!(selector.selected().is_none());
        key(&selector, KeyCode::Char(' '), KeyModifiers::NONE);
        key(&selector, KeyCode::Down, KeyModifiers::ALT);
        assert_eq!(
            *selector.working.lock().unwrap(),
            Some(vec!["p/alpha".into(), "p/beta".into()])
        );
    }

    #[test]
    fn scoped_models_provider_toggle_and_empty_apply_preserve_legacy_all_semantics() {
        let runtime = Arc::new(Mutex::new(None));
        let selector = widget(None, runtime.clone());
        key(&selector, KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(
            *selector.working.lock().unwrap(),
            Some(vec!["q/alpha".into()])
        );
        key(&selector, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert!(selector
            .render(80)
            .join("\n")
            .contains("apply restores all"));
        key(&selector, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(*runtime.lock().unwrap(), None);
    }

    #[test]
    fn scoped_models_cycle_uses_full_identity_and_saved_order() {
        let models = catalog();
        let ids = vec![
            "p/beta".into(),
            "q/alpha".into(),
            "p/alpha".into(),
            "missing/model".into(),
        ];
        let scope = scoped_catalog(&models, Some(&ids));
        assert_eq!(
            scope.iter().map(full_id).collect::<Vec<_>>(),
            vec!["p/beta", "q/alpha", "p/alpha"]
        );
        let provider = same_provider_catalog(&models, "beta");
        let cycle = scoped_catalog(&provider, Some(&ids));
        assert_eq!(
            cycle.iter().map(full_id).collect::<Vec<_>>(),
            vec!["p/beta", "p/alpha"]
        );
        assert_eq!(cycle_next_model(&cycle, "beta").unwrap().id, "alpha");
        assert_eq!(cycle_prev_model(&cycle, "beta").unwrap().id, "alpha");
        assert_eq!(scoped_catalog(&models, None), models);
        assert_eq!(
            canonical_scope(&models, Some(vec!["beta".into()])),
            Some(vec!["p/beta".into()])
        );
    }
}
