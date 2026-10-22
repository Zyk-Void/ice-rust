//! Truncated text component.
//!
//! Displays text that is truncated to fit within a given width.

use std::any::Any;
use std::sync::Mutex;

use super::component::Component;
use crate::utils::{truncate_to_width, visible_width};

/// First line of text truncated to fit, with paired margins and blank rows
/// above and below (ice `TruncatedText`). Every row is padded to the full
/// width; empty text still renders one padded row.
pub struct TruncatedText {
    text: Mutex<String>,
    padding_x: usize,
    padding_y: usize,
    /// Optional cap below the render width.
    max_width: Option<usize>,
    ellipsis: String,
    style: Option<String>,
}

impl TruncatedText {
    /// Create with the given margins and the default `...` ellipsis.
    pub fn new(text: &str, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: Mutex::new(text.to_string()),
            padding_x,
            padding_y,
            max_width: None,
            ellipsis: "...".to_string(),
            style: None,
        }
    }

    /// Create without margins, capped at `max_width` cells, with a custom
    /// ellipsis.
    pub fn with_ellipsis(text: &str, max_width: usize, ellipsis: &str) -> Self {
        Self {
            max_width: Some(max_width),
            ellipsis: ellipsis.to_string(),
            ..Self::new(text, 0, 0)
        }
    }

    /// Set text.
    pub fn set_text(&self, text: &str) {
        if let Ok(mut t) = self.text.lock() {
            *t = text.to_string();
        }
    }

    /// Get text.
    pub fn get_text(&self) -> String {
        self.text.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// Cap the text at `width` cells even when more room is available.
    pub fn set_max_width(&mut self, width: usize) {
        self.max_width = Some(width);
    }

    /// Set style (ANSI escape codes) applied to the text.
    pub fn set_style(&mut self, style: &str) {
        self.style = Some(style.to_string());
    }

    /// Whether the first line exceeds the configured cap.
    pub fn is_truncated(&self) -> bool {
        let text = self.get_text();
        let first = text.split('\n').next().unwrap_or("");
        self.max_width
            .is_some_and(|max_width| visible_width(first) > max_width)
    }
}

impl Component for TruncatedText {
    fn render(&self, width: usize) -> Vec<String> {
        let text = self.get_text();
        let first = text.split('\n').next().unwrap_or("");
        let mut available = width.saturating_sub(self.padding_x * 2).max(1);
        if let Some(max_width) = self.max_width {
            available = available.min(max_width);
        }
        let truncated = truncate_to_width(first, available, &self.ellipsis);
        let styled = match &self.style {
            Some(style) => format!("{style}{truncated}\x1b[0m"),
            None => truncated,
        };
        let margin = " ".repeat(self.padding_x);
        let row = format!("{margin}{styled}{margin}");
        let pad = width.saturating_sub(visible_width(&row));
        let blank = " ".repeat(width);
        let mut lines = vec![blank.clone(); self.padding_y];
        lines.push(row + &" ".repeat(pad));
        lines.extend(std::iter::repeat(blank).take(self.padding_y));
        lines
    }

    fn invalidate(&self) {
        // No cached state
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_truncated() {
        assert!(TruncatedText::with_ellipsis("Hello World", 5, "…").is_truncated());
        assert!(!TruncatedText::with_ellipsis("Hi", 10, "…").is_truncated());
        assert!(!TruncatedText::new("Hello World", 0, 0).is_truncated());
    }

    #[test]
    fn cap_and_custom_ellipsis_still_apply() {
        let lines = TruncatedText::with_ellipsis("Hello World", 5, "…").render(10);
        assert_eq!(lines, vec!["Hell\x1b[0m…\x1b[0m     "]);
    }

    // Mirrors ice packages/tui/test/truncated-text.test.ts.

    #[test]
    fn pads_output_to_width() {
        let lines = TruncatedText::new("Hello world", 1, 0).render(50);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 50);
    }

    #[test]
    fn vertical_padding_rows_are_full_width() {
        let lines = TruncatedText::new("Hello", 0, 2).render(40);
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|line| visible_width(line) == 40));
    }

    #[test]
    fn truncates_long_text_and_pads() {
        let long =
            "This is a very long piece of text that will definitely exceed the available width";
        let lines = TruncatedText::new(long, 1, 0).render(30);
        assert_eq!(visible_width(&lines[0]), 30);
        assert!(crate::ansi::strip_ansi(&lines[0]).contains("..."));
    }

    #[test]
    fn keeps_ansi_and_resets_before_ellipsis() {
        let styled = "\x1b[31mHello\x1b[39m \x1b[34mworld\x1b[39m";
        let lines = TruncatedText::new(styled, 1, 0).render(40);
        assert_eq!(visible_width(&lines[0]), 40);
        assert!(lines[0].contains("\x1b["));

        let long = "\x1b[31mThis is a very long red text that will be truncated\x1b[39m";
        let lines = TruncatedText::new(long, 1, 0).render(20);
        assert_eq!(visible_width(&lines[0]), 20);
        assert!(lines[0].contains("\x1b[0m..."));
    }

    #[test]
    fn text_that_fits_and_empty_text() {
        let lines = TruncatedText::new("Hello world", 1, 0).render(30);
        assert_eq!(visible_width(&lines[0]), 30);
        assert!(!lines[0].contains("..."));
        let lines = TruncatedText::new("", 1, 0).render(30);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 30);
    }

    #[test]
    fn shows_only_the_first_line() {
        let lines = TruncatedText::new("First line\nSecond line\nThird line", 1, 0).render(40);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 40);
        assert!(lines[0].contains("First line"));
        assert!(!lines[0].contains("Second line"));

        let long = "This is a very long first line that needs truncation\nSecond line";
        let lines = TruncatedText::new(long, 1, 0).render(25);
        assert_eq!(visible_width(&lines[0]), 25);
        assert!(lines[0].contains("..."));
        assert!(!lines[0].contains("Second line"));
    }
}
