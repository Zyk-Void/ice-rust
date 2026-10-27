//! Completion suggestions and editor cursor operations.

use super::*;

struct SkillAutocomplete {
    skills: Vec<ice_harness::types::Skill>,
}

impl ice_tui::AutocompleteProvider for SkillAutocomplete {
    fn name(&self) -> &str {
        "skills"
    }

    fn get_suggestions(
        &self,
        input: &str,
        cursor: usize,
    ) -> Option<ice_tui::AutocompleteSuggestions> {
        let before = input.get(..cursor)?;
        let start = before.rfind('$')?;
        if start > 0 && !before[..start].chars().next_back()?.is_whitespace() {
            return None;
        }
        let query = &before[start + 1..];
        if query.chars().any(char::is_whitespace) {
            return None;
        }
        let query = query.to_lowercase();
        let items: Vec<_> = self
            .skills
            .iter()
            .filter(|s| s.name.to_lowercase().contains(&query))
            .map(|s| {
                ice_tui::AutocompleteItem::new(&format!("⟦skill:{}⟧", s.name))
                    .with_label(&s.name)
                    .with_description(&format!("Skill: {}", s.description))
            })
            .collect();
        (!items.is_empty()).then_some(ice_tui::AutocompleteSuggestions {
            items,
            start,
            end: cursor,
            is_path: false,
        })
    }
}

#[cfg(test)]
mod skill_tests {
    use super::*;
    use ice_tui::AutocompleteProvider;

    fn skill() -> ice_harness::types::Skill {
        ice_harness::types::Skill {
            name: "review".into(),
            description: "Review code".into(),
            content: "Instructions".into(),
            file_path: "/skills/review/SKILL.md".into(),
            disable_model_invocation: None,
        }
    }

    #[test]
    fn dollar_suggests_and_expands_skills() {
        let skill = skill();
        let provider = SkillAutocomplete {
            skills: vec![skill.clone()],
        };
        let suggestions = provider.get_suggestions("use $rev", 8).unwrap();
        assert_eq!(suggestions.items[0].text, "⟦skill:review⟧");
        assert!(provider.get_suggestions("use review", 10).is_none());
        let resources = ice_harness::types::AgentHarnessResources {
            skills: Some(vec![skill]),
            ..Default::default()
        };
        let expanded =
            expand_skill_references(&resources, "use ⟦skill:review⟧ now ⟦skill:missing⟧");
        assert!(expanded.contains("<skill name=\"review\""));
        assert!(expanded.contains("Instructions"));
        assert!(expanded.ends_with(" now ⟦skill:missing⟧"));
    }
}

pub(super) struct McpAutocomplete {
    pub manager: Option<crate::mcp::McpManager>,
}
impl ice_tui::AutocompleteProvider for McpAutocomplete {
    fn name(&self) -> &str {
        "mcp"
    }
    fn get_suggestions(
        &self,
        input: &str,
        cursor: usize,
    ) -> Option<ice_tui::AutocompleteSuggestions> {
        let before = input.get(..cursor)?.strip_prefix("/mcp ")?;
        let (prefix, fragment) = before
            .rsplit_once(' ')
            .map_or(("", before), |(a, b)| (a, b));
        let candidates: Vec<String> = if prefix.is_empty() {
            [
                "status",
                "tools",
                "prompts",
                "reconnect",
                "enable",
                "disable",
                "add",
                "remove",
                "edit",
                "setup",
                "auth",
                "logout",
            ]
            .iter()
            .map(|s| (*s).into())
            .collect()
        } else if matches!(
            prefix,
            "reconnect" | "enable" | "disable" | "remove" | "prompts" | "tools" | "auth" | "logout"
        ) {
            self.manager
                .as_ref()?
                .snapshot()
                .into_iter()
                .map(|s| s.name)
                .collect()
        } else {
            return None;
        };
        Some(ice_tui::AutocompleteSuggestions {
            items: candidates
                .into_iter()
                .filter(|s| s.starts_with(fragment))
                .map(|s| ice_tui::AutocompleteItem::new(&s))
                .collect(),
            start: cursor - fragment.len(),
            end: cursor,
            is_path: false,
        })
    }
}

// ===========================================================================
// Autocomplete
// ===========================================================================

/// Editor completion: the registry's visible slash commands, then the prompt
/// templates in `resources` as `/name` entries (built-ins first, so they win
/// a fuzzy tie), then file paths rooted at `cwd`. Built at startup and again
/// after a successful `/reload`, so templates added since then are offered.
#[cfg(test)]
pub(super) fn build_autocomplete_provider(
    registry: &CommandRegistry,
    resources: &ice_harness::types::AgentHarnessResources,
    cwd: &std::path::Path,
    on_ready: Option<Arc<dyn Fn() + Send + Sync>>,
) -> Arc<dyn ice_tui::AutocompleteProvider> {
    build_autocomplete_with_mcp(registry, resources, cwd, on_ready, None)
}

pub(super) fn build_autocomplete_with_mcp(
    registry: &CommandRegistry,
    resources: &ice_harness::types::AgentHarnessResources,
    cwd: &std::path::Path,
    on_ready: Option<Arc<dyn Fn() + Send + Sync>>,
    manager: Option<crate::mcp::McpManager>,
) -> Arc<dyn ice_tui::AutocompleteProvider> {
    let mut commands = registry.visible_entries();
    // A built-in or extension command wins over a template of the same name.
    let taken: std::collections::HashSet<String> =
        commands.iter().map(|c| c.name.clone()).collect();
    commands.extend(
        resources
            .prompt_templates
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|t| SlashCommandEntry {
                name: format!("/{}", t.name),
                description: t
                    .description
                    .clone()
                    .unwrap_or_else(|| "Expand prompt template".to_string()),
            })
            .filter(|entry| !taken.contains(&entry.name)),
    );
    let mut combined = CombinedAutocompleteProvider::new();
    combined.add_provider(Arc::new(SkillAutocomplete {
        skills: resources.skills.clone().unwrap_or_default(),
    }));
    combined.add_provider(Arc::new(McpAutocomplete { manager }));
    combined.add_provider(Arc::new(SlashCommandAutocompleteProvider::new(commands)));
    // Reinstall recursive completion on reload as well as startup. Its callback
    // wakes the same TUI queue after a background search finishes.
    if let Some(on_ready) = on_ready {
        if let Some(recursive) =
            ice_tui::RecursiveFileAutocompleteProvider::with_fd(cwd.to_path_buf(), on_ready)
        {
            combined.add_provider(Arc::new(recursive));
        }
    }
    combined.add_provider(Arc::new(FilePathAutocompleteProvider::with_root(
        cwd.to_path_buf(),
    )));
    Arc::new(combined)
}

/// Refresh the autocomplete suggestion list from the current editor text +
/// cursor. Renders the suggestions into `autocomplete_container` (above the
/// editor) or clears it when there are none.
pub(super) fn refresh_autocomplete(state: &Arc<TuiState>, editor: &Arc<Editor>) {
    let text = editor.get_text();
    let cursor = editor_cursor_offset(editor, &text);
    let suggestions = state.autocomplete.get_suggestions(&text, cursor);
    render_autocomplete(state, suggestions);
}

/// Convert the editor's logical `(row, byte-column)` caret into the absolute
/// byte offset expected by autocomplete providers.
pub(super) fn editor_cursor_offset(editor: &Editor, text: &str) -> usize {
    let (row, col) = editor.cursor_position();
    let mut offset = 0;
    for (index, line) in text.split('\n').enumerate() {
        if index == row {
            return (offset + col.min(line.len())).min(text.len());
        }
        offset = offset.saturating_add(line.len() + 1);
    }
    text.len()
}

/// Restore an editor caret from an absolute byte offset after autocomplete
/// replaces a span in a multi-line draft.
pub(super) fn set_editor_cursor_offset(editor: &Editor, text: &str, offset: usize) {
    let offset = offset.min(text.len());
    let mut consumed = 0;
    for (row, line) in text.split('\n').enumerate() {
        let end = consumed + line.len();
        if offset <= end {
            editor.set_cursor(row, offset - consumed);
            return;
        }
        consumed = end + 1;
    }
    let last_row = text.bytes().filter(|byte| *byte == b'\n').count();
    editor.set_cursor(
        last_row,
        text.rsplit('\n').next().map(str::len).unwrap_or(0),
    );
}

/// Render (or clear) the autocomplete suggestion list into the container.
pub(super) fn render_autocomplete(
    state: &Arc<TuiState>,
    suggestions: Option<AutocompleteSuggestions>,
) {
    state.autocomplete_container.clear();
    let Some(sugg) = suggestions else {
        *state.autocomplete_selection.lock().unwrap() = 0;
        return;
    };
    if sugg.items.is_empty() {
        *state.autocomplete_selection.lock().unwrap() = 0;
        return;
    }
    // Clamp the highlight to the full candidate list, then compute the visible
    // window (with a scroll offset) so ↓ can reach items past `max_visible`.
    let max_visible = state.autocomplete_max_visible;
    let total = sugg.items.len();
    let selection = {
        let mut sel = state.autocomplete_selection.lock().unwrap();
        *sel = (*sel).min(total.saturating_sub(1));
        *sel
    };
    let window_start = if total <= max_visible {
        0
    } else {
        selection
            .saturating_sub(max_visible / 2)
            .min(total - max_visible)
    };
    let window_end = (window_start + max_visible).min(total);
    // Build a compact list: the highlighted item marked with `→`, the rest `  `.
    // Cap the list so the dock doesn't swallow the transcript.
    let accent = state.theme_manager.get().colors.accent;
    let muted = state.theme_manager.get().colors.muted;
    for (i, item) in sugg
        .items
        .iter()
        .enumerate()
        .skip(window_start)
        .take(window_end - window_start)
    {
        let prefix = if i == selection { "→ " } else { "  " };
        let label = item.display_text();
        let line = if i == selection {
            format!(
                "{prefix}{} {}",
                accent.fg(label),
                muted.fg(item.description.as_deref().unwrap_or(""))
            )
        } else {
            format!(
                "{prefix}{} {}",
                muted.fg(label),
                muted.fg(item.description.as_deref().unwrap_or(""))
            )
        };
        state
            .autocomplete_container
            .add_child(Arc::new(Text::new(line, 1, 0)));
    }
}

/// Accept the highlighted autocomplete suggestion: replace `text[start..end]`
/// with the suggestion text, reposition the caret, and clear the suggestion
/// list. Returns `true` if a suggestion was accepted.
pub(super) fn accept_top_suggestion(state: &Arc<TuiState>, editor: &Arc<Editor>) -> bool {
    let text = editor.get_text();
    let cursor = editor_cursor_offset(editor, &text);
    let Some(sugg) = state.autocomplete.get_suggestions(&text, cursor) else {
        return false;
    };
    let index = {
        let sel = *state.autocomplete_selection.lock().unwrap();
        sel.min(sugg.items.len().saturating_sub(1))
    };
    let Some(top) = sugg.items.get(index) else {
        return false;
    };
    // Replace the [start, end) span with the suggestion text. `start`/`end`
    // are byte offsets emitted by the providers on char boundaries, so the
    // `text[..start]` / `text[end..]` slices are sound for multibyte input.
    let start = sugg.start.min(text.len());
    let end = sugg.end.min(text.len());
    let mut replaced = String::with_capacity(text.len() + top.text.len());
    replaced.push_str(&text[..start]);
    replaced.push_str(&top.text);
    let suffix = &text[end..];
    let mut new_cursor = replaced.len();
    if top.is_directory && top.text.ends_with('"') {
        // Keep typing inside a quoted directory path after accepting it.
        new_cursor -= 1;
    } else if top.insert_space && !top.is_directory {
        match suffix.chars().next() {
            Some(' ' | '\t') => new_cursor += 1,
            Some(ch) if ch.is_whitespace() => {}
            _ => {
                replaced.push(' ');
                new_cursor += 1;
            }
        }
    }
    // Preserve the suffix exactly, placing a new separator at the insertion
    // point instead of at the end of the user's remaining draft.
    replaced.push_str(suffix);
    editor.set_text(&replaced);
    set_editor_cursor_offset(editor, &replaced, new_cursor);
    state.autocomplete_container.clear();
    *state.autocomplete_selection.lock().unwrap() = 0;
    true
}

/// Move the autocomplete highlight up (`direction < 0`) or down (`direction > 0`)
/// and re-render the list. Returns `true` when the key was consumed.
///
/// Only takes over the arrow keys in an explicit completion context (`/` or
/// `@` prefix); a bare-word path completion (e.g. a draft containing `docs`)
/// must NOT swallow ↑/↓, which the editor uses for cursor/history movement.
pub(super) fn navigate_autocomplete(
    state: &Arc<TuiState>,
    editor: &Arc<Editor>,
    direction: i32,
) -> bool {
    let text = editor.get_text();
    let cursor = editor_cursor_offset(editor, &text);
    let Some(sugg) = state.autocomplete.get_suggestions(&text, cursor) else {
        return false;
    };
    if sugg.items.is_empty() {
        return false;
    }
    let start = sugg.start.min(text.len());
    // `/` commands keep the slash at `start`; the path provider keeps `@` by
    // placing `start` *after* it, so look just before `start` for `@`.
    let explicit = text[start..].starts_with('/') || text[..start].ends_with('@');
    if !explicit {
        return false;
    }
    let total = sugg.items.len();
    {
        let mut sel = state.autocomplete_selection.lock().unwrap();
        if direction < 0 {
            *sel = sel.saturating_sub(1);
        } else {
            *sel = (*sel + 1).min(total.saturating_sub(1));
        }
    }
    render_autocomplete(state, Some(sugg));
    true
}
