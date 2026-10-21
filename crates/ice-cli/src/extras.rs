//! First-launch setup banner — thin host wrapper over the ice-tui components.
//!
//! Kept out of the library crate so ice-tui stays cli-free (project constraint).

use std::sync::Arc;

use ice_tui::Container;

use crate::config;

/// Path of the "first-time setup done" sentinel, under the agent dir
/// (`~/.icers/agent/.setup_done`).
pub fn setup_done_path() -> Option<std::path::PathBuf> {
    config::agent_dir().ok().map(|d| d.join(".setup_done"))
}

/// Whether first-time setup has already been completed (sentinel present).
pub fn setup_done() -> bool {
    setup_done_path().map(|p| p.exists()).unwrap_or(false)
}

/// Mark first-time setup complete (write the sentinel). Best-effort.
pub fn mark_setup_done() -> std::io::Result<()> {
    let path = setup_done_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir for .icers/agent")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, b"1")
}

/// If first-time setup hasn't run yet, show a brief setup note in the chat
/// container. Returns `true` if anything was shown.
pub fn maybe_first_time_setup(chat: &Arc<Container>) -> bool {
    use ice_tui::{DynamicBorder, Spacer, Text};
    if setup_done() {
        return false;
    }
    let accent = ice_tui::theme().colors.accent;
    let muted = ice_tui::theme().colors.muted;
    // Borders are components so they fit the real width; each line used to
    // print its text twice (" {colored} {plain}") between fixed 80-column
    // borders.
    chat.add_child(Arc::new(DynamicBorder::with_color(accent)));
    for line in welcome_lines(&accent, &muted) {
        chat.add_child(Arc::new(Text::new(line, 1, 0)));
    }
    chat.add_child(Arc::new(DynamicBorder::with_color(accent)));
    chat.add_child(Arc::new(Spacer::new(1)));
    let _ = mark_setup_done();
    true
}

/// The first-run welcome text, one styled line each.
fn welcome_lines(accent: &ice_tui::Color, muted: &ice_tui::Color) -> Vec<String> {
    vec![
        accent.fg(&bold("Welcome to icers!")),
        muted.fg("Pick a theme with /theme (dark/light/monochrome)."),
        muted.fg("Type /help for commands."),
    ]
}

fn bold(s: &str) -> String {
    format!("\x1b[1m{s}\x1b[22m")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each welcome line used to print its text twice ("Welcome to icers!
    /// Welcome to icers!").
    #[test]
    fn welcome_lines_say_each_thing_once() {
        let theme = ice_tui::theme();
        for line in welcome_lines(&theme.colors.accent, &theme.colors.muted) {
            let plain = ice_tui::strip_ansi(&line);
            let half = plain
                .char_indices()
                .nth(plain.chars().count() / 2)
                .map_or(plain.len(), |(i, _)| i);
            assert!(
                !plain[half..].contains(plain[..half].trim()),
                "duplicated: {plain:?}"
            );
        }
        let first =
            ice_tui::strip_ansi(&welcome_lines(&theme.colors.accent, &theme.colors.muted)[0]);
        assert_eq!(first, "Welcome to icers!");
    }

    #[test]
    fn test_sentinel_paths_under_agent_dir() {
        // The sentinel lives directly under the resolved agent dir and honors
        // the `ICERS_CODING_AGENT_DIR` override (delegated to `config::agent_dir`).
        let _guard = crate::config::test_support::env_lock().lock().unwrap();
        let prev = std::env::var_os(crate::config::CONFIG_DIR_ENV);
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var(crate::config::CONFIG_DIR_ENV, tmp.path());
        let agent = crate::config::agent_dir().unwrap();
        assert_eq!(agent.as_path(), tmp.path());
        if let Some(p) = setup_done_path() {
            assert!(p.ends_with(".setup_done"));
            assert_eq!(p.parent().unwrap(), agent);
        }
        match prev {
            Some(v) => std::env::set_var(crate::config::CONFIG_DIR_ENV, v),
            None => std::env::remove_var(crate::config::CONFIG_DIR_ENV),
        }
    }
}
