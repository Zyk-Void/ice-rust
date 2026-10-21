//! Transcript navigation shared by the local and driver-based TUI.
use std::sync::{Arc, Mutex, Weak};

use crossterm::event::KeyEvent;
use ice_tui::{keybindings::keys, Keybindings, TuiAltScreen};

#[derive(Default)]
struct MessageCountState {
    entries: usize,
    snapshot: Option<usize>,
    count: usize,
    current_assistant_entry: Option<usize>,
    previous_assistant: bool,
}

/// Tracks visible assistant turns added while the transcript is scrolled away.
#[derive(Default)]
pub(crate) struct NewMessageCount {
    state: Mutex<MessageCountState>,
}

impl NewMessageCount {
    pub(crate) fn begin_assistant_message(&self) {
        let mut state = self.state.lock().unwrap();
        state.entries += 1;
        state.current_assistant_entry = Some(state.entries);
    }

    pub(crate) fn assistant_message(&self, has_visible_text: bool) {
        let mut state = self.state.lock().unwrap();
        let is_new = state
            .snapshot
            .zip(state.current_assistant_entry)
            .is_some_and(|(snapshot, entry)| entry > snapshot);
        if is_new && has_visible_text && !state.previous_assistant {
            state.count += 1;
        }
        if is_new && has_visible_text {
            state.previous_assistant = true;
        }
    }

    pub(crate) fn other_message(&self) {
        let mut state = self.state.lock().unwrap();
        state.entries += 1;
        state.current_assistant_entry = None;
        state.previous_assistant = false;
    }

    pub(crate) fn custom_message(&self) {
        let mut state = self.state.lock().unwrap();
        state.entries += 1;
        state.current_assistant_entry = None;
    }

    pub(crate) fn reset(&self) {
        *self.state.lock().unwrap() = MessageCountState::default();
    }

    fn count_since_snapshot(&self, following_end: bool) -> usize {
        let mut state = self.state.lock().unwrap();
        if following_end {
            state.snapshot = None;
            state.count = 0;
            state.previous_assistant = false;
            return 0;
        }
        if state.snapshot.is_none() {
            state.snapshot = Some(state.entries);
            state.count = 0;
            state.previous_assistant = false;
        }
        state.count
    }
}

pub(crate) fn install(tui: &Arc<TuiAltScreen>, bindings: Arc<Keybindings>) {
    install_with_message_count(tui, bindings, None);
}

pub(crate) fn install_with_message_count(
    tui: &Arc<TuiAltScreen>,
    bindings: Arc<Keybindings>,
    message_count: Option<Arc<NewMessageCount>>,
) {
    let tui_weak: Weak<TuiAltScreen> = Arc::downgrade(tui);
    tui.set_scroll_to_end_indicator(Some(Arc::new(move || {
        let following_end = tui_weak
            .upgrade()
            .map_or(true, |tui| tui.is_following_output());
        let new_count = message_count.as_ref().map_or(0, |message_count| {
            message_count.count_since_snapshot(following_end)
        });
        let colors = ice_tui::theme().colors;
        format!(
            "{}{}{}\x1b[39m\x1b[49m",
            colors.selection.to_bg(),
            colors.text.to_fg(),
            label(&bindings, new_count)
        )
    })));
}

fn label(bindings: &Keybindings, new_count: usize) -> String {
    let shortcut = bindings
        .get_keys(keys::BOTTOM)
        .iter()
        .map(|key| key.display())
        .collect::<Vec<_>>()
        .join("/");
    let text = if new_count == 0 {
        "Jump to latest message".to_owned()
    } else {
        format!(
            "{new_count} new message{}",
            if new_count == 1 { "" } else { "s" }
        )
    };
    if shortcut.is_empty() {
        format!(" ↓ {text} ")
    } else {
        format!(" ↓ {text} · {shortcut} ")
    }
}

pub(crate) fn handle_key(tui: &TuiAltScreen, bindings: &Keybindings, key: &KeyEvent) -> bool {
    if bindings.matches(key, keys::BOTTOM) {
        tui.scroll_to_bottom();
    } else if bindings.matches(key, keys::TOP) {
        tui.scroll_to_top();
    } else if bindings.matches(key, keys::PAGE_UP) || bindings.matches(key, keys::PAGE_DOWN) {
        let height = tui
            .get_primary_scroll_view()
            .map_or(1, |scroll| scroll.viewport_height());
        let page = transcript_page_size(height);
        tui.scroll_by(if bindings.matches(key, keys::PAGE_UP) {
            -page
        } else {
            page
        });
    } else {
        return false;
    }
    true
}

pub(crate) fn transcript_page_size(viewport_height: usize) -> i32 {
    viewport_height
        .saturating_sub(4)
        .max(1)
        .min(i32::MAX as usize) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ice_tui::KeyCombo;

    #[test]
    fn indicator_hint_uses_the_actual_binding_and_hides_disabled_shortcuts() {
        let mut bindings = Keybindings::new();
        assert_eq!(label(&bindings, 0), " ↓ Jump to latest message · End ");
        bindings.set(
            keys::BOTTOM,
            vec![KeyCombo::new(KeyCode::End, KeyModifiers::CONTROL)],
        );
        assert_eq!(label(&bindings, 1), " ↓ 1 new message · Ctrl+End ");
        bindings.set(keys::BOTTOM, Vec::new());
        assert_eq!(label(&bindings, 0), " ↓ Jump to latest message ");
    }

    #[test]
    fn count_groups_adjacent_assistant_entries_and_ignores_blank_text() {
        let count = NewMessageCount::default();
        assert_eq!(count.count_since_snapshot(false), 0);
        count.begin_assistant_message();
        count.assistant_message(true);
        count.begin_assistant_message();
        count.assistant_message(false);
        count.assistant_message(true);
        assert_eq!(count.count_since_snapshot(false), 1);

        count.other_message();
        count.begin_assistant_message();
        count.assistant_message(true);
        assert_eq!(count.count_since_snapshot(false), 2);
    }

    #[test]
    fn response_started_before_scrolling_is_not_counted_when_text_arrives_later() {
        let count = NewMessageCount::default();
        count.begin_assistant_message();
        assert_eq!(count.count_since_snapshot(false), 0);
        count.assistant_message(true);
        assert_eq!(count.count_since_snapshot(false), 0);
    }

    #[test]
    fn count_snapshots_when_scrolled_away_and_clears_when_following_or_reset() {
        let count = NewMessageCount::default();
        count.begin_assistant_message();
        count.assistant_message(true);
        assert_eq!(count.count_since_snapshot(false), 0);
        count.other_message();
        count.begin_assistant_message();
        count.assistant_message(true);
        assert_eq!(count.count_since_snapshot(false), 1);
        assert_eq!(count.count_since_snapshot(true), 0);
        assert_eq!(count.count_since_snapshot(false), 0);
        count.other_message();
        count.begin_assistant_message();
        count.assistant_message(true);
        assert_eq!(count.count_since_snapshot(false), 1);
        count.reset();
        assert_eq!(count.count_since_snapshot(false), 0);
    }
}
