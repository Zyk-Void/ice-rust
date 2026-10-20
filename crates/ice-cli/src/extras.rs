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

/// The tmux settings that stop modified Enter keys (Shift+Enter) reaching
/// icers, given `tmux show -gv` for `extended-keys` and
/// `extended-keys-format` (ice `checkTmuxKeyboardSetup`). `None` when
/// tmux could not be asked or is set up well.
pub fn tmux_keyboard_warning(
    extended_keys: Option<&str>,
    extended_keys_format: Option<&str>,
) -> Option<&'static str> {
    let extended_keys = extended_keys?;
    if extended_keys != "on" && extended_keys != "always" {
        return Some(
            "tmux extended-keys is off. Modified Enter keys may not work. Add \
             `set -g extended-keys on` to ~/.tmux.conf and restart tmux.",
        );
    }
    if extended_keys_format == Some("xterm") {
        return Some(
            "tmux extended-keys-format is xterm. icers works best with csi-u. Add \
             `set -g extended-keys-format csi-u` to ~/.tmux.conf and restart tmux.",
        );
    }
    None
}

/// `tmux show -gv <option>`, or `None` if tmux fails or takes over 2 s.
pub fn tmux_show(option: &str) -> Option<String> {
    let mut child = std::process::Command::new("tmux")
        .args(["show", "-gv", option])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string())
}

/// If first-time setup hasn't run yet, show a brief setup note in the chat
/// container. Returns `true` if anything was shown.
pub fn maybe_first_time_setup(chat: &Arc<Container>) -> bool {
    use ice_tui::Component;
    use ice_tui::{DynamicBorder, Spacer, Text};
    if setup_done() {
        return false;
    }
    let accent = ice_tui::theme().colors.accent;
    let muted = ice_tui::theme().colors.muted;
    let border = DynamicBorder::with_color(accent);
    let mut lines: Vec<String> = Vec::new();
    lines.extend(border.render(80));
    lines.push(format!(
        " {} Welcome to icers!",
        accent.fg(&bold("Welcome to icers!"))
    ));
    lines.push(format!(
        " {} Pick a theme with /theme (dark/light/monochrome).",
        muted.fg("Pick a theme with /theme (dark/light/monochrome).")
    ));
    lines.push(format!(
        " {} Type /help for commands.",
        muted.fg("Type /help for commands.")
    ));
    lines.extend(border.render(80));
    for line in lines {
        chat.add_child(Arc::new(Text::new(line, 1, 0)));
    }
    chat.add_child(Arc::new(Spacer::new(1)));
    let _ = mark_setup_done();
    true
}

fn bold(s: &str) -> String {
    format!("\x1b[1m{s}\x1b[22m")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ice warns about the two tmux settings that break modified Enter keys,
    /// and stays quiet when tmux could not be asked.
    #[test]
    fn tmux_keyboard_warning_matches_ice() {
        assert_eq!(tmux_keyboard_warning(None, None), None);
        assert!(tmux_keyboard_warning(Some("off"), Some("csi-u"))
            .unwrap()
            .contains("set -g extended-keys on"));
        assert!(tmux_keyboard_warning(Some("on"), Some("xterm"))
            .unwrap()
            .contains("set -g extended-keys-format csi-u"));
        assert_eq!(tmux_keyboard_warning(Some("always"), Some("csi-u")), None);
        assert_eq!(tmux_keyboard_warning(Some("on"), None), None);
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
