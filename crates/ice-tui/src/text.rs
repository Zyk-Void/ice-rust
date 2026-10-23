//! Text component for simple text rendering.

use std::sync::{Arc, Mutex};

use std::any::Any;

use super::component::{Component, RenderedDocument};

/// Background closure applied to each rendered row (see
/// [`crate::utils::apply_background_to_line`]).
pub type BgFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// Word-wrapped text with paired horizontal margins and blank rows above and
/// below (ice `Text`).
///
/// Every row is padded to the full width. Text that is empty or only
/// whitespace renders no rows at all. Tabs become three spaces.
pub struct Text {
    content: Mutex<String>,
    padding_x: usize,
    padding_y: usize,
    bg_fn: Mutex<Option<BgFn>>,
    /// Render cache keyed by (content, width, theme revision). A finalized
    /// `Text` row is re-rendered on every repaint of a long transcript;
    /// caching the wrapped lines keeps that cheap.
    cache: Mutex<Option<TextCache>>,
}

#[derive(Clone)]
struct TextCache {
    width: usize,
    theme_revision: u64,
    lines: Arc<Vec<String>>,
}

impl Default for Text {
    /// Empty text with one cell of margin and one blank row each side.
    fn default() -> Self {
        Self::new("", 1, 1)
    }
}

impl Text {
    /// Create a new text component.
    pub fn new(content: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            content: Mutex::new(content.into()),
            padding_x,
            padding_y,
            bg_fn: Mutex::new(None),
            cache: Mutex::new(None),
        }
    }

    /// Create a text component whose rows get a background.
    pub fn with_bg(
        content: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        bg_fn: BgFn,
    ) -> Self {
        let text = Self::new(content, padding_x, padding_y);
        text.set_custom_bg_fn(Some(bg_fn));
        text
    }

    /// Set the text content.
    pub fn set_text(&self, text: impl Into<String>) {
        if let Ok(mut content) = self.content.lock() {
            *content = text.into();
        }
        self.invalidate();
    }

    /// Get the text content.
    pub fn get_text(&self) -> String {
        self.content.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Set or clear the row background. A closure whose output changes
    /// later needs [`Component::invalidate`].
    pub fn set_custom_bg_fn(&self, bg_fn: Option<BgFn>) {
        if let Ok(mut slot) = self.bg_fn.lock() {
            *slot = bg_fn;
        }
        self.invalidate();
    }

    fn finish_row(&self, row: String, width: usize, bg_fn: Option<&BgFn>) -> String {
        match bg_fn {
            Some(f) => crate::utils::apply_background_to_line(&row, width, |s| f(s)),
            None => {
                let pad = width.saturating_sub(crate::ansi::visible_width(&row));
                row + &" ".repeat(pad)
            }
        }
    }
}

impl Text {
    fn shared_lines(&self, width: usize) -> Arc<Vec<String>> {
        // Hold content through cache lookup/publication so a setter cannot
        // publish new content while an old render is still being cached.
        let content = self.content.lock().unwrap_or_else(|e| e.into_inner());
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let theme_revision = crate::theme::theme_revision();

        if let Some(cached) = cache.as_ref() {
            if cached.width == width && cached.theme_revision == theme_revision {
                return Arc::clone(&cached.lines);
            }
        }

        let lines = if content.trim().is_empty() {
            Vec::new()
        } else {
            let bg_fn = self.bg_fn.lock().ok().and_then(|f| f.clone());
            let margin = " ".repeat(self.padding_x);
            let content_width = width.saturating_sub(self.padding_x * 2).max(1);
            let normalized = content.replace('\t', "   ");
            let blank = self.finish_row(String::new(), width, bg_fn.as_ref());
            let mut lines = vec![blank.clone(); self.padding_y];
            for line in crate::utils::wrap_text_with_ansi(&normalized, content_width) {
                let row = format!("{margin}{line}{margin}");
                lines.push(self.finish_row(row, width, bg_fn.as_ref()));
            }
            lines.extend(std::iter::repeat(blank).take(self.padding_y));
            lines
        };

        let lines = Arc::new(lines);
        *cache = Some(TextCache {
            width,
            theme_revision,
            lines: Arc::clone(&lines),
        });
        lines
    }
}

impl Component for Text {
    fn render(&self, width: usize) -> Vec<String> {
        self.shared_lines(width).as_ref().clone()
    }

    fn render_document(&self, width: usize) -> RenderedDocument {
        RenderedDocument::from_shared(self.shared_lines(width))
    }

    fn invalidate(&self) {
        if let Ok(_content) = self.content.lock() {
            if let Ok(mut cache) = self.cache.lock() {
                *cache = None;
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Component;

    #[test]
    fn shared_rows_reuse_and_regenerate_on_mutation_and_width() {
        let text = Text::new("abcdefgh", 0, 0);
        let first = text.shared_lines(80);
        let _document = text.render_document(80);
        assert!(Arc::ptr_eq(&first, &text.shared_lines(80)));
        assert_eq!(first.as_ref(), &text.render(80));
        let narrow = text.shared_lines(4);
        assert!(!Arc::ptr_eq(&first, &narrow));
        assert_eq!(narrow.as_ref(), &vec!["abcd", "efgh"]);
        text.set_text("new text");
        let changed = text.shared_lines(80);
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(crate::strip_ansi(changed[0].trim_end()), "new text");
        text.invalidate();
        let refreshed = text.shared_lines(80);
        assert!(!Arc::ptr_eq(&changed, &refreshed));
        assert_eq!(changed.as_ref(), refreshed.as_ref());
    }

    /// Regression: rendering a line wider than the layout column used to
    /// byte-slice `padded[..width]` and panic mid-character on multi-byte
    /// chars — "end byte index 120 is not a char boundary; it is inside
    /// '⏎' (bytes 119..122)". Truncation must never split a char: it keeps
    /// whole chars whose visible width fits the column.
    #[test]
    fn render_truncates_multibyte_at_char_boundary() {
        let t = Text::new("x⏎y", 0, 0);
        // Width 1: only 'x' fits (the 3-byte ⏎ is dropped whole, never split).
        let out = t.render(1);
        assert_eq!(out[0], "x");
        // Width 2: 'x' + '⏎' (⏎ is visible-width 1) — whole chars only.
        let out = t.render(2);
        assert_eq!(out[0], "x⏎");
        // Width 3: everything fits unchanged.
        let out = t.render(3);
        assert_eq!(out[0], "x⏎y");
    }

    #[test]
    fn render_keeps_ansi_and_truncates() {
        let styled = "\x1b[31mred\x1b[0m text";
        let t = Text::new(styled, 0, 0);
        let out = t.render(3);
        // No panic on ANSI content; visible width respected.
        assert!(crate::ansi::visible_width(&out[0]) <= 3);
    }

    #[test]
    fn blank_text_renders_no_rows() {
        assert!(Text::default().render(10).is_empty());
        assert!(Text::new(" \t\n ", 1, 1).render(10).is_empty());
    }

    #[test]
    fn margins_wrap_and_pad_every_row_to_width() {
        let lines = Text::new("hello world", 1, 1).render(9);
        assert_eq!(
            lines,
            vec!["         ", " hello   ", " world   ", "         "]
        );
    }

    #[test]
    fn tabs_become_three_spaces() {
        assert_eq!(Text::new("a\tb", 0, 0).render(6), vec!["a   b "]);
    }

    #[test]
    fn background_covers_padding_and_setter_invalidates() {
        let text = Text::new("hi", 1, 0);
        assert_eq!(text.render(5), vec![" hi  "]);
        text.set_custom_bg_fn(Some(Arc::new(|s: &str| format!("<{s}>"))));
        assert_eq!(text.render(5), vec!["< hi  >"]);
    }

    #[test]
    fn narrow_width_keeps_one_content_column() {
        let lines = Text::new("ab", 2, 0).render(3);
        assert_eq!(lines, vec!["  a  ", "  b  "]);
    }
}
