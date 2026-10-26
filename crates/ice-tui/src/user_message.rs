//! User message component — port of `user-message.ts`.
//!
//! Renders the user's submitted text as markdown inside a padded [`Box`] with
//! a subtle background, wrapped in [OSC 133] prompt zone markers so capable
//! terminals can detect user/assistant boundaries. Where OSC 133 is
//! unsupported the sequences are harmless no-ops.
//!
//! Replaces the plain `> text` echo in the transcript.
//!
//! [OSC 133]: https://terminalguide.namepad.de/seq/osc-133/

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::component::Component;
use crate::box_component::Box;
use crate::markdown::Markdown;
use crate::theme::{theme, theme_revision};
use crate::utils::{apply_background_to_line, truncate_to_width};

/// Status an integration attaches to a user message (e.g. what memory did for
/// that prompt). The badge sits right-aligned on the message's first line;
/// detail rows, when there are any, render inside the message box under the
/// text. Both may carry OSC 8 links to make them click targets.
pub trait MessageAnnotation: Send + Sync {
    /// Badge for the first line, at most `width` columns, or `None` to hide.
    fn badge(&self, width: usize) -> Option<String>;
    /// Rows under the message text, each at most `width` columns.
    fn details(&self, width: usize) -> Vec<String>;
}

const OSC133_ZONE_START: &str = "\x1b]133;A\x07";
const OSC133_ZONE_END: &str = "\x1b]133;B\x07";
const OSC133_ZONE_FINAL: &str = "\x1b]133;C\x07";

/// Component that renders a user message with a background and OSC133 markers.
pub struct UserMessageComponent {
    text: String,
    output_pad: usize,
    /// Prebuilt inner box (background `Box` + `Markdown`).
    ///
    /// Upstream builds this **once** in the constructor (`user-message.ts`
    /// `rebuild()`); icers used to rebuild the `Box` + `Markdown` on every
    /// `render`, so the inner caches could never hit and each frame re-parsed
    /// every user message. Rebuild only on `invalidate` / `with_pad` / a theme
    /// change (the background colour is baked into the box).
    inner: Mutex<Box>,
    /// Theme revision the current inner box was built against.
    built_revision: AtomicU64,
    annotation: Mutex<Option<Arc<dyn MessageAnnotation>>>,
}

fn build_box(text: &str, output_pad: usize) -> Box {
    let bg = theme().colors.surface;
    let content_box = Box::new(output_pad, 1);
    content_box.set_bg_fn(Some(std::sync::Arc::new(move |s: &str| bg.bg(s))));
    let md = Markdown::new(text.to_string(), 0, 0);
    content_box.add_child(std::sync::Arc::new(md));
    content_box
}

impl UserMessageComponent {
    /// Create a new user message component.
    ///
    /// `output_pad` is the horizontal+vertical padding inside the background
    /// box (mirrors the TS `outputPad`, default 1).
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let inner = build_box(&text, 1);
        Self {
            text,
            output_pad: 1,
            inner: Mutex::new(inner),
            built_revision: AtomicU64::new(theme_revision()),
            annotation: Mutex::new(None),
        }
    }

    /// The submitted text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Attach (or with `None`, remove) the message's annotation.
    pub fn set_annotation(&self, annotation: Option<Arc<dyn MessageAnnotation>>) {
        if let Ok(mut slot) = self.annotation.lock() {
            *slot = annotation;
        }
    }

    pub fn annotation(&self) -> Option<Arc<dyn MessageAnnotation>> {
        self.annotation.lock().ok().and_then(|slot| slot.clone())
    }

    /// Lay the annotation over the box rows: details under the text, and the
    /// badge right-aligned on the box's bottom padding row, so a collapsed
    /// annotation adds no height.
    fn annotate(&self, lines: &mut Vec<String>, width: usize) {
        let Some(annotation) = self.annotation() else {
            return;
        };
        let bg = theme().colors.surface;
        let pad = " ".repeat(self.output_pad);
        let inner_width = width.saturating_sub(self.output_pad * 2).max(1);
        // Box(pad, 1): one background row above and below the text.
        let bottom = lines.len() - 1;
        for (offset, row) in annotation.details(inner_width).into_iter().enumerate() {
            lines.insert(
                bottom + offset,
                apply_background_to_line(&format!("{pad}{row}"), width, |s| bg.bg(s)),
            );
        }
        if let Some(badge) = annotation.badge(inner_width) {
            let badge = truncate_to_width(&badge, inner_width, "…");
            let gap = " ".repeat(inner_width - crate::visible_width(&badge));
            let last = lines.len() - 1;
            lines[last] = apply_background_to_line(
                &format!("{pad}{}{badge}{pad}", bg.bg(&gap)),
                width,
                |s| bg.bg(s),
            );
        }
    }
}

impl Component for UserMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        // The background colour is baked into the inner box; rebuild it when
        // the live theme has changed since it was built.
        let revision = theme_revision();
        if self.built_revision.load(Ordering::Acquire) != revision {
            if let Ok(mut inner) = self.inner.lock() {
                *inner = build_box(&self.text, self.output_pad);
            }
            self.built_revision.store(revision, Ordering::Release);
        }

        let mut lines = match self.inner.lock() {
            Ok(inner) => inner.render(width),
            Err(_) => return Vec::new(),
        };
        if lines.is_empty() {
            return lines;
        }
        self.annotate(&mut lines, width);
        // Prepend zone-start to the first line, append zone-end+final to the
        // last line. These are zero-width escapes so visible layout is unchanged.
        let first = lines.first_mut().unwrap();
        first.insert_str(0, OSC133_ZONE_START);
        let last = lines.last_mut().unwrap();
        last.insert_str(0, OSC133_ZONE_END);
        last.insert_str(OSC133_ZONE_END.len(), OSC133_ZONE_FINAL);
        lines
    }

    fn invalidate(&self) {
        // Rebuild the inner box so a theme change is picked up by the baked-in
        // background colour, and so the Markdown cache is dropped.
        if let Ok(mut inner) = self.inner.lock() {
            *inner = build_box(&self.text, self.output_pad);
        }
        self.built_revision
            .store(theme_revision(), Ordering::Release);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::strip_ansi;

    #[test]
    fn test_renders_non_empty() {
        let c = UserMessageComponent::new("Hello world");
        let lines = c.render(40);
        assert!(!lines.is_empty());
        // Should contain the text (after stripping ANSI/OSC).
        let joined = strip_ansi(&lines.join("\n"));
        assert!(joined.contains("Hello"));
    }

    #[test]
    fn test_osc133_markers_present() {
        let c = UserMessageComponent::new("Hi");
        let lines = c.render(40);
        let joined = lines.join("\n");
        assert!(joined.contains(OSC133_ZONE_START), "missing zone-start A");
        assert!(joined.contains(OSC133_ZONE_END), "missing zone-end B");
        assert!(joined.contains(OSC133_ZONE_FINAL), "missing zone-final C");
    }

    struct Fixed(Option<&'static str>, Vec<&'static str>);
    impl MessageAnnotation for Fixed {
        fn badge(&self, _width: usize) -> Option<String> {
            self.0.map(String::from)
        }
        fn details(&self, _width: usize) -> Vec<String> {
            self.1.iter().map(|row| row.to_string()).collect()
        }
    }

    fn plain_rows(c: &UserMessageComponent, width: usize) -> Vec<String> {
        c.render(width).iter().map(|row| strip_ansi(row)).collect()
    }

    #[test]
    fn annotation_badge_sits_bottom_right_and_details_go_inside_the_box() {
        let c = UserMessageComponent::new("hey");
        let before = plain_rows(&c, 30);
        c.set_annotation(Some(Arc::new(Fixed(Some("[mem]"), vec![]))));
        let collapsed = plain_rows(&c, 30);
        assert_eq!(collapsed.len(), before.len(), "a badge adds no rows");
        assert_eq!(collapsed[1], before[1], "the text row is untouched");
        assert_eq!(
            collapsed.last().unwrap(),
            &format!("{}[mem] ", " ".repeat(30 - 6))
        );

        c.set_annotation(Some(Arc::new(Fixed(Some("[mem]"), vec!["detail one"]))));
        let rows = plain_rows(&c, 30);
        assert_eq!(rows.len(), before.len() + 1, "{rows:?}");
        assert_eq!(rows[2].trim_end(), " detail one");
        assert!(rows.last().unwrap().ends_with("[mem] "), "{rows:?}");
        assert!(
            rows.iter().all(|row| crate::visible_width(row) == 30),
            "{rows:?}"
        );

        c.set_annotation(Some(Arc::new(Fixed(None, vec![]))));
        assert_eq!(plain_rows(&c, 30), before);
    }

    #[test]
    fn annotation_badge_is_cut_to_the_box_width() {
        let c = UserMessageComponent::new("hi");
        c.set_annotation(Some(Arc::new(Fixed(
            Some("[a badge far wider than the box]"),
            vec![],
        ))));
        let rows = plain_rows(&c, 20);
        assert!(
            rows.iter().all(|row| crate::visible_width(row) == 20),
            "{rows:?}"
        );
        assert!(rows.last().unwrap().contains('…'), "{rows:?}");
    }

    #[test]
    fn test_cjk_no_panic() {
        let c = UserMessageComponent::new("你好世界");
        let lines = c.render(6);
        assert!(!lines.is_empty());
    }
}
