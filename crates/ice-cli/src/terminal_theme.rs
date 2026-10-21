//! Startup background detection using the same crossterm reader as keyboard input.
//! Ordinary keys are queued intact while OSC 11 replies select the initial palette.
use std::collections::VecDeque;
use std::io::{self, IsTerminal, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ice_tui::terminal_colors::{parse_osc11_background_color, query_background_color, RgbColor};
use ice_tui::{apply_theme_preset, theme_revision, ThemeManager, ThemePreset};

fn saved_preset(saved: Option<&str>) -> Option<ThemePreset> {
    match saved {
        Some("light") => Some(ThemePreset::Light),
        Some("monochrome") => Some(ThemePreset::Monochrome),
        Some("dark") => Some(ThemePreset::Dark),
        _ => None,
    }
}

fn background_preset(color: RgbColor) -> ThemePreset {
    let linear = |channel: u8| {
        let value = channel as f64 / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    if 0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b) >= 0.5 {
        ThemePreset::Light
    } else {
        ThemePreset::Dark
    }
}

fn environment_preset(colorfgbg: Option<&str>) -> ThemePreset {
    let Some(index) = colorfgbg.and_then(|value| {
        value
            .split(';')
            .rev()
            .find_map(|part| part.trim().parse::<u8>().ok())
    }) else {
        return ThemePreset::Dark;
    };
    const BASIC: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let (r, g, b) = match index {
        0..=15 => BASIC[index as usize],
        16..=231 => {
            const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let value = (index - 16) as usize;
            (CUBE[value / 36], CUBE[value / 6 % 6], CUBE[value % 6])
        }
        _ => {
            let gray = 8 + 10 * (index - 232);
            (gray, gray, gray)
        }
    };
    background_preset(RgbColor::new(r, g, b))
}

#[derive(Default)]
struct BackgroundReply {
    text: String,
    held: Vec<Event>,
    began: Option<Instant>,
}

impl BackgroundReply {
    fn release(&mut self) -> Vec<Event> {
        self.text.clear();
        self.began = None;
        std::mem::take(&mut self.held)
    }

    fn expire(&mut self, now: Instant) -> Vec<Event> {
        if self
            .began
            .is_some_and(|began| now.duration_since(began) >= Duration::from_millis(50))
        {
            self.release()
        } else {
            Vec::new()
        }
    }

    fn push(&mut self, input: Event, now: Instant) -> (Vec<Event>, Option<RgbColor>) {
        // Focus and resize notifications are out of band. They may arrive
        // between bytes of a reply without ending its keyboard-input sequence.
        if matches!(
            input,
            Event::Resize(..) | Event::FocusGained | Event::FocusLost
        ) {
            return (vec![input], None);
        }
        let Event::Key(key) = &input else {
            let mut out = self.release();
            out.push(input);
            return (out, None);
        };
        if key.kind != KeyEventKind::Press {
            let mut out = self.release();
            out.push(input);
            return (out, None);
        }
        let fragment = match (key.code, key.modifiers) {
            (KeyCode::Esc, KeyModifiers::NONE) => "\x1b".to_string(),
            (KeyCode::Char(']'), KeyModifiers::ALT) => "\x1b]".to_string(),
            (KeyCode::Char('\\'), KeyModifiers::ALT) => "\x1b\\".to_string(),
            (KeyCode::Char('g'), KeyModifiers::CONTROL) => "\x07".to_string(),
            (KeyCode::Char(c), modifiers)
                if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
            {
                c.to_string()
            }
            _ => String::new(),
        };
        if self.text.is_empty() && fragment != "\x1b" && fragment != "\x1b]" {
            return (vec![input], None);
        }
        self.began.get_or_insert(now);
        self.text.push_str(&fragment);
        self.held.push(input);
        if self.text.ends_with('\x07') || self.text.ends_with("\x1b\\") {
            if let Some(color) = parse_osc11_background_color(&self.text) {
                self.release();
                return (Vec::new(), Some(color));
            }
        } else if !fragment.is_empty() && reply_prefix(&self.text) {
            return (Vec::new(), None);
        }
        (self.release(), None)
    }
}

fn reply_prefix(text: &str) -> bool {
    const PREFIX: &str = "\x1b]11;";
    if PREFIX.starts_with(text) {
        return true;
    }
    let Some(body) = text.strip_prefix(PREFIX) else {
        return false;
    };
    let body = body.strip_suffix('\x1b').unwrap_or(body);
    if "rgb:".starts_with(body) || body == "#" {
        return true;
    }
    if let Some(hex) = body.strip_prefix('#') {
        return hex.len() <= 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    if let Some(rgb) = body.strip_prefix("rgb:") {
        let parts: Vec<_> = rgb.split(['/', ',']).collect();
        return parts.len() <= 3
            && parts
                .iter()
                .all(|part| part.len() <= 4 && part.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    false
}

/// Owns the one keyboard-event reader, including keys received during the bounded
/// startup query. Unsupported terminals fall back to COLORFGBG, then dark.
pub(crate) struct ThemeInput {
    queued: VecDeque<Event>,
    reply: BackgroundReply,
    automatic: bool,
    applied_revision: u64,
    manager: Arc<ThemeManager>,
    on_change: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl ThemeInput {
    pub(crate) fn new(saved: Option<&str>, manager: Arc<ThemeManager>) -> Self {
        let fixed = saved_preset(saved);
        let automatic = saved.is_none() || matches!(saved, Some("auto" | "light/dark"));
        let initial =
            fixed.unwrap_or_else(|| environment_preset(std::env::var("COLORFGBG").ok().as_deref()));
        apply_theme_preset(initial);
        manager.apply_preset(initial);
        let mut input = Self {
            queued: VecDeque::new(),
            reply: BackgroundReply::default(),
            automatic,
            applied_revision: theme_revision(),
            manager,
            on_change: None,
        };
        if automatic && io::stdin().is_terminal() && io::stdout().is_terminal() {
            let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
            if was_raw || crossterm::terminal::enable_raw_mode().is_ok() {
                let mut stdout = io::stdout().lock();
                let queried = stdout
                    .write_all(query_background_color().as_bytes())
                    .is_ok()
                    && stdout.flush().is_ok();
                drop(stdout);
                if queried {
                    let deadline = Instant::now() + Duration::from_millis(100);
                    while Instant::now() < deadline {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        match event::poll(remaining) {
                            Ok(true) => match event::read() {
                                Ok(event) => {
                                    if input.receive(event) {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            },
                            _ => break,
                        }
                    }
                }
                if !was_raw {
                    let _ = crossterm::terminal::disable_raw_mode();
                }
            }
        }
        input.queued.extend(input.reply.expire(Instant::now()));
        input
    }

    /// Returns true when a complete background reply was consumed.
    fn receive(&mut self, event: Event) -> bool {
        if !self.automatic {
            self.queued.push_back(event);
            return false;
        }
        let (events, background) = self.reply.push(event, Instant::now());
        self.queued.extend(events);
        if let Some(color) = background {
            // A /theme selection made after startup always wins over a late reply.
            if theme_revision() == self.applied_revision {
                let preset = background_preset(color);
                apply_theme_preset(preset);
                self.manager.apply_preset(preset);
                self.applied_revision = theme_revision();
                if let Some(on_change) = &self.on_change {
                    on_change();
                }
            }
            return true;
        }
        false
    }

    pub(crate) fn on_change(&mut self, callback: Arc<dyn Fn() + Send + Sync>) {
        self.on_change = Some(callback);
    }

    pub(crate) fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            self.queued.extend(self.reply.expire(Instant::now()));
            if !self.queued.is_empty() {
                return Ok(true);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            if event::poll(remaining.min(Duration::from_millis(50)))? {
                self.receive(event::read()?);
            }
        }
    }

    /// Read an event previously made ready by poll.
    pub(crate) fn read(&mut self) -> io::Result<Event> {
        self.queued
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "no terminal event ready"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn feed(filter: &mut BackgroundReply, events: Vec<Event>) -> (Vec<Event>, Option<RgbColor>) {
        let mut kept = Vec::new();
        let mut background = None;
        for event in events {
            let (out, color) = filter.push(event, Instant::now());
            kept.extend(out);
            background = background.or(color);
        }
        (kept, background)
    }

    #[test]
    fn terminal_background_query_preserves_keys_and_paste_around_a_reply() {
        let mut events = vec![
            key('x'),
            Event::Paste("typed paste".into()),
            Event::Key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::ALT)),
        ];
        events.extend("11;rgb:ffff/ffff/ffff".chars().map(key));
        events.push(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        events.push(key('y'));
        let (kept, color) = feed(&mut BackgroundReply::default(), events);
        assert_eq!(
            kept,
            vec![key('x'), Event::Paste("typed paste".into()), key('y')]
        );
        assert_eq!(color, Some(RgbColor::new(255, 255, 255)));
    }

    #[test]
    fn terminal_background_query_accepts_split_escape_and_st_terminator() {
        let mut events = vec![Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))];
        events.extend("]11;rgb:00/00/00".chars().map(key));
        events.push(Event::Key(KeyEvent::new(
            KeyCode::Char('\\'),
            KeyModifiers::ALT,
        )));
        let (kept, color) = feed(&mut BackgroundReply::default(), events);
        assert!(kept.is_empty());
        assert_eq!(color, Some(RgbColor::new(0, 0, 0)));
    }

    #[test]
    fn terminal_background_query_keeps_out_of_band_events_inside_a_reply() {
        let mut events = vec![Event::Key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::ALT,
        ))];
        events.extend("11;rgb:ffff/".chars().map(key));
        events.extend([Event::Resize(100, 40), Event::FocusLost, Event::FocusGained]);
        events.extend("ffff/ffff".chars().map(key));
        events.push(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        let (kept, color) = feed(&mut BackgroundReply::default(), events);
        assert_eq!(
            kept,
            vec![Event::Resize(100, 40), Event::FocusLost, Event::FocusGained],
        );
        assert_eq!(color, Some(RgbColor::new(255, 255, 255)));
    }

    #[test]
    fn terminal_background_query_returns_malformed_or_incomplete_input_unchanged() {
        let mut filter = BackgroundReply::default();
        let escape = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let start = Instant::now();
        assert!(filter.push(escape.clone(), start).0.is_empty());
        assert_eq!(
            filter.expire(start + Duration::from_millis(50)),
            vec![escape]
        );
        let events = vec![
            Event::Key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::ALT)),
            key('x'),
        ];
        let (kept, color) = feed(&mut filter, events.clone());
        assert_eq!(kept, events);
        assert_eq!(color, None);
        assert!(!reply_prefix("\x1b]11;#€€"));
        assert!(!reply_prefix("\x1b]11;rgb:fffff"));
    }

    #[test]
    fn terminal_background_theme_uses_linear_luminance_and_environment_fallback() {
        assert!(matches!(
            background_preset(RgbColor::new(128, 128, 128)),
            ThemePreset::Dark
        ));
        assert!(matches!(
            background_preset(RgbColor::new(255, 255, 255)),
            ThemePreset::Light
        ));
        assert!(matches!(
            environment_preset(Some("0;15")),
            ThemePreset::Light
        ));
        assert!(matches!(
            environment_preset(Some("15;0")),
            ThemePreset::Dark
        ));
        assert!(matches!(
            environment_preset(Some("0;255")),
            ThemePreset::Light
        ));
        assert!(matches!(
            environment_preset(Some("invalid")),
            ThemePreset::Dark
        ));
        assert!(matches!(
            saved_preset(Some("dark")),
            Some(ThemePreset::Dark)
        ));
        assert!(matches!(
            saved_preset(Some("light")),
            Some(ThemePreset::Light)
        ));
        assert!(matches!(
            saved_preset(Some("monochrome")),
            Some(ThemePreset::Monochrome)
        ));
    }
}
