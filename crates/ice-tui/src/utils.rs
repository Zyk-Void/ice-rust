//! Utilities for TUI rendering.
//!
//! Provides functions for text manipulation with ANSI awareness.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::ansi::StyleTracker;

/// Get the visible width of a string, ignoring ANSI escape sequences.
///
/// Same measurement as [`crate::ansi::visible_width`]; both names stay public.
pub fn visible_width(s: &str) -> usize {
    crate::ansi::visible_width(s)
}

/// Slice a string by column positions, accounting for ANSI codes.
/// Returns the portion of the string that fits within start_col..end_col.
pub fn slice_by_column(s: &str, start_col: usize, max_cols: usize, pad: bool) -> String {
    let mut result = String::new();
    let mut col = 0;
    let mut in_escape = false;
    let mut pending_styles = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if in_escape {
            pending_styles.push(c);

            // Check for sequence end
            if c == '\x07' {
                in_escape = false;
            } else if c == '\\' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                pending_styles.push('\\');
                i += 1;
                in_escape = false;
            } else if c >= '@' && c <= '~' && c != '[' {
                in_escape = false;
            }
            // OSC sequence
            else if c == ']' {
                while i + 1 < chars.len() {
                    i += 1;
                    let next = chars[i];
                    pending_styles.push(next);
                    if next == '\x07' {
                        in_escape = false;
                        break;
                    } else if next == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                        i += 1;
                        pending_styles.push('\\');
                        in_escape = false;
                        break;
                    }
                }
            }
            i += 1;
            continue;
        }

        if c == '\x1b' {
            in_escape = true;
            pending_styles.push(c);
            i += 1;
            continue;
        }

        // Skip control characters
        if c < ' ' || c == '\x7f' {
            i += 1;
            continue;
        }

        // Get grapheme width
        let char_width = UnicodeWidthStr::width(c.to_string().as_str());

        // Check if we're at or past start_col
        if col >= start_col && col < start_col + max_cols {
            // Add pending styles before the character
            if !pending_styles.is_empty() {
                result.push_str(&pending_styles);
                pending_styles.clear();
            }
            result.push(c);
        }

        col += char_width;
        i += 1;
    }

    // Pad if needed
    if pad && col < start_col + max_cols {
        let padding = start_col + max_cols - col;
        for _ in 0..padding {
            result.push(' ');
        }
    }

    result
}

/// Truncate a string to fit within a maximum width.
/// Returns the truncated string with optional suffix.
pub fn truncate_to_width(s: &str, max_width: usize, suffix: &str) -> String {
    if max_width == 0 {
        return String::new();
    }

    let width = visible_width(s);
    if width <= max_width {
        return s.to_string();
    }

    let suffix_width = visible_width(suffix);
    let target_width = max_width.saturating_sub(suffix_width);

    if target_width == 0 {
        return suffix.to_string();
    }

    // Check if input has ANSI codes
    let has_ansi = s.contains("\x1b");

    let mut result = String::new();
    let mut current_width = 0;
    let mut in_escape = false;
    let mut style_stack = String::new();

    for c in s.chars() {
        if in_escape {
            style_stack.push(c);
            if c == '\x07' || (c >= '@' && c <= '~' && c != '[') {
                in_escape = false;
            }
            continue;
        }

        if c == '\x1b' {
            in_escape = true;
            style_stack.push(c);
            continue;
        }

        if c < ' ' || c == '\x7f' {
            continue;
        }

        let char_width = UnicodeWidthStr::width(c.to_string().as_str());
        if current_width + char_width > target_width {
            break;
        }

        if !style_stack.is_empty() {
            result.push_str(&style_stack);
            style_stack.clear();
        }
        result.push(c);
        current_width += char_width;
    }

    // Add suffix
    result.push_str(suffix);

    // Only reset style if input had ANSI codes
    if has_ansi {
        result.push_str("\x1b[0m");
    }
    result
}

/// Strip ANSI escape sequences from a string.
///
/// Same as [`crate::ansi::strip_ansi`]; both names stay public.
pub fn strip_ansi(s: &str) -> String {
    crate::ansi::strip_ansi(s)
}

/// Wrap text to `max_width` cells, preserving ANSI styling across rows.
///
/// Port of ice `wrapTextWithAnsi`: breaks at spaces between words and at
/// every CJK grapheme, splits a word wider than the row at grapheme
/// boundaries, drops the spaces at a break, and splits on CRLF/CR/LF. Each
/// continuation row re-opens the active SGR state and hyperlink. A row that
/// wraps closes only underline and an open hyperlink, so the caller's
/// background padding keeps its colour; renderers reset every written row
/// (see [`crate::ansi::SEGMENT_RESET`]). Rows are not padded.
///
/// A `max_width` of 0 is treated as 1, and a grapheme wider than the row
/// gets a row of its own, so the result can only exceed `max_width` there.
pub fn wrap_text_with_ansi(s: &str, max_width: usize) -> Vec<String> {
    let width = max_width.max(1);
    let mut result = Vec::new();
    let mut tracker = StyleTracker::default();
    for (index, line) in split_lines(s).enumerate() {
        let prefix = if index > 0 {
            tracker.active_codes()
        } else {
            String::new()
        };
        result.extend(wrap_single_line(&format!("{prefix}{line}"), width));
        tracker.process_text(line);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

/// Split on CRLF, CR or LF, like `/\r\n|\r|\n/`.
fn split_lines(s: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(s);
    std::iter::from_fn(move || {
        let current = rest?;
        match current.find(['\r', '\n']) {
            Some(i) => {
                let skip = if current[i..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
                rest = Some(&current[i + skip..]);
                Some(&current[..i])
            }
            None => {
                rest = None;
                Some(current)
            }
        }
    })
}

/// Han, Hiragana, Katakana, Hangul and Bopomofo graphemes are each their own
/// break opportunity (ice `cjkBreakRegex`).
fn is_cjk_break(grapheme: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"[\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}\p{scx=Hangul}\p{scx=Bopomofo}]",
        )
        .expect("valid CJK class")
    })
    .is_match(grapheme)
}

/// Visible pieces of `text` with escape sequences returned separately.
enum Piece<'a> {
    Escape(&'a str),
    Grapheme(&'a str),
}

fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with('\x1b') {
            let len = crate::ansi::escape_len(rest);
            out.push(Piece::Escape(&rest[..len]));
            rest = &rest[len..];
        } else {
            let end = rest.find('\x1b').unwrap_or(rest.len());
            out.extend(rest[..end].graphemes(true).map(Piece::Grapheme));
            rest = &rest[end..];
        }
    }
    out
}

/// Space runs, word runs and single CJK graphemes, with escape sequences
/// attached to the visible content that follows them.
fn tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_is_space = None;
    let mut pending = String::new();
    for piece in pieces(text) {
        let grapheme = match piece {
            Piece::Escape(code) => {
                pending.push_str(code);
                continue;
            }
            Piece::Grapheme(g) => g,
        };
        let is_space = grapheme == " ";
        if !is_space && is_cjk_break(grapheme) {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            current_is_space = None;
            tokens.push(std::mem::take(&mut pending) + grapheme);
            continue;
        }
        if !current.is_empty() && current_is_space != Some(is_space) {
            tokens.push(std::mem::take(&mut current));
        }
        current.push_str(&std::mem::take(&mut pending));
        current_is_space = Some(is_space);
        current.push_str(grapheme);
    }
    if !pending.is_empty() {
        if !current.is_empty() {
            current.push_str(&pending);
        } else if let Some(last) = tokens.last_mut() {
            last.push_str(&pending);
        } else {
            current = pending;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    if visible_width(line) <= width {
        return vec![line.to_string()];
    }
    let mut wrapped = Vec::new();
    let mut tracker = StyleTracker::default();
    let mut current = String::new();
    let mut current_width = 0;
    for token in tokens(line) {
        let token_width = visible_width(&token);
        let is_whitespace = token.trim().is_empty();
        if token_width > width && !is_whitespace {
            if !current.is_empty() {
                current.push_str(&tracker.line_end_reset());
                wrapped.push(std::mem::take(&mut current));
            }
            let mut broken = break_long_word(&token, width, &mut tracker);
            current = broken.pop().unwrap_or_default();
            current_width = visible_width(&current);
            wrapped.extend(broken);
            continue;
        }
        if current_width + token_width > width && current_width > 0 {
            let mut row = current.trim_end().to_string();
            row.push_str(&tracker.line_end_reset());
            wrapped.push(row);
            current = tracker.active_codes();
            if is_whitespace {
                current_width = 0;
            } else {
                current.push_str(&token);
                current_width = token_width;
            }
        } else {
            current.push_str(&token);
            current_width += token_width;
        }
        tracker.process_text(&token);
    }
    if !current.is_empty() {
        wrapped.push(current);
    }
    if wrapped.is_empty() {
        return vec![String::new()];
    }
    wrapped
        .into_iter()
        .map(|row| row.trim_end().to_string())
        .collect()
}

/// Split a token wider than the row at grapheme boundaries, carrying style.
fn break_long_word(word: &str, width: usize, tracker: &mut StyleTracker) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = tracker.active_codes();
    let mut current_width = 0;
    for piece in pieces(word) {
        match piece {
            Piece::Escape(code) => {
                current.push_str(code);
                tracker.process(code);
            }
            Piece::Grapheme(grapheme) => {
                let grapheme_width = crate::ansi::grapheme_width(grapheme);
                if current_width + grapheme_width > width && current_width > 0 {
                    current.push_str(&tracker.line_end_reset());
                    rows.push(std::mem::replace(&mut current, tracker.active_codes()));
                    current_width = 0;
                }
                current.push_str(grapheme);
                current_width += grapheme_width;
            }
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// Punctuation that ends a word for editor word motion (ice
/// `PUNCTUATION_REGEX`).
pub fn is_punctuation_char(c: char) -> bool {
    "(){}[]<>.,;:'\"!?+-=*/\\|&%^$#@~`".contains(c)
}

/// Get the grapheme segmenter for Unicode-aware text handling.
pub fn get_grapheme_segmenter() -> GraphemeSegmenter {
    GraphemeSegmenter
}

/// Grapheme segmenter wrapper.
pub struct GraphemeSegmenter;

impl GraphemeSegmenter {
    /// Segment a string into graphemes.
    pub fn segment<'a>(&'a self, s: &'a str) -> impl Iterator<Item = Grapheme<'a>> + 'a {
        s.graphemes(true).enumerate().map(|(idx, g)| Grapheme {
            segment: g,
            index: idx,
        })
    }
}

/// A grapheme cluster.
#[derive(Debug, Clone)]
pub struct Grapheme<'a> {
    pub segment: &'a str,
    pub index: usize,
}

/// Check if a character is whitespace.
pub fn is_whitespace_char(c: char) -> bool {
    c.is_whitespace() || c == '\t' || c == '\n' || c == '\r'
}

/// Get an OSC 8 hyperlink at a specific column position.
pub fn get_osc8_link_at_column(s: &str, column: usize) -> Option<String> {
    let mut col = 0;
    let mut in_escape = false;
    let mut in_osc8 = false;
    let mut osc8_link = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if in_escape {
            if c == ']' {
                // OSC start - check for OSC 8
                let mut osc_content = String::new();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '\x07' {
                        break;
                    } else if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                    osc_content.push(next);
                }
                // OSC 8 format: 8;params;uri
                if osc_content.starts_with("8;") {
                    in_osc8 = true;
                    if let Some(uri_start) = osc_content.find(';').map(|i| i + 1) {
                        if uri_start < osc_content.len() {
                            osc8_link = osc_content[uri_start..].to_string();
                        }
                    }
                }
                in_escape = false;
                continue;
            } else if c >= '@' && c <= '~' && c != '[' {
                in_escape = false;
            }
            continue;
        }

        if c == '\x1b' {
            in_escape = true;
            continue;
        }

        if c < ' ' || c == '\x7f' {
            continue;
        }

        if col == column && in_osc8 && !osc8_link.is_empty() {
            return Some(osc8_link);
        }

        col += UnicodeWidthStr::width(c.to_string().as_str());
    }

    None
}

/// Apply background color to a line, respecting existing styling.
pub fn apply_background_to_line(
    line: &str,
    width: usize,
    bg_fn: impl Fn(&str) -> String,
) -> String {
    let line_width = visible_width(line);
    let padding = " ".repeat(width.saturating_sub(line_width));
    let padded = format!("{}{}", line, padding);
    bg_fn(&padded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_width() {
        assert_eq!(visible_width("Hello"), 5);
        assert_eq!(visible_width("\x1b[31mHello\x1b[0m"), 5);
        assert_eq!(visible_width("你好"), 4); // Chinese characters are wide
    }

    #[test]
    fn test_truncate_to_width() {
        assert_eq!(truncate_to_width("Hello World", 5, ""), "Hello");
        assert_eq!(truncate_to_width("Hello World", 8, "..."), "Hello...");
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[31mHello\x1b[0m"), "Hello");
        assert_eq!(strip_ansi("\x1b[1;31mWorld\x1b[0m"), "World");
    }

    #[test]
    fn test_wrap_text() {
        let lines = wrap_text_with_ansi("Hello World", 5);
        assert_eq!(lines, vec!["Hello", "World"]);
    }

    // The tests below mirror ice packages/tui/test/wrap-ansi.test.ts.

    const UL_ON: &str = "\x1b[4m";
    const UL_OFF: &str = "\x1b[24m";
    const RESET: &str = "\x1b[0m";
    const RED: &str = "\x1b[31m";

    #[test]
    fn underline_starts_with_the_styled_text() {
        let url = "https://example.com/very/long/path/that/will/wrap";
        let wrapped = wrap_text_with_ansi(&format!("read this thread {UL_ON}{url}{UL_OFF}"), 40);
        assert_eq!(wrapped[0], "read this thread");
        assert!(wrapped[1].starts_with(UL_ON));
        assert!(wrapped[1].contains("https://"));
    }

    #[test]
    fn no_whitespace_before_underline_reset() {
        let wrapped = wrap_text_with_ansi(&format!("{UL_ON}underlined text here {UL_OFF}more"), 18);
        assert!(!wrapped[0].contains(&format!(" {UL_OFF}")));
    }

    #[test]
    fn wrapped_underline_closes_with_underline_off_not_full_reset() {
        let url = "https://example.com/very/long/path/that/will/definitely/wrap";
        let wrapped = wrap_text_with_ansi(&format!("prefix {UL_ON}{url}{UL_OFF} suffix"), 30);
        for line in &wrapped[1..wrapped.len() - 1] {
            if line.contains(UL_ON) {
                assert!(line.ends_with(UL_OFF), "{line:?}");
                assert!(!line.ends_with(RESET), "{line:?}");
            }
        }
    }

    #[test]
    fn background_survives_wraps_without_full_reset() {
        let bg = "\x1b[44m";
        let wrapped = wrap_text_with_ansi(
            &format!("{bg}hello world this is blue background text{RESET}"),
            15,
        );
        for line in &wrapped {
            assert!(line.contains(bg), "{line:?}");
        }
        for line in &wrapped[..wrapped.len() - 1] {
            assert!(!line.ends_with(RESET), "{line:?}");
        }
    }

    #[test]
    fn underline_inside_background_resets_only_underline() {
        let text =
            format!("\x1b[41mprefix {UL_ON}UNDERLINED_CONTENT_THAT_WRAPS{UL_OFF} suffix{RESET}");
        let wrapped = wrap_text_with_ansi(&text, 20);
        for line in &wrapped {
            assert!(
                line.contains("[41m") || line.contains(";41m") || line.contains("[41;"),
                "{line:?}"
            );
        }
        for line in &wrapped[..wrapped.len() - 1] {
            let underlined = line.contains("[4m") || line.contains("[4;") || line.contains(";4m");
            if underlined && !line.contains(UL_OFF) {
                assert!(line.ends_with(UL_OFF), "{line:?}");
            }
        }
    }

    #[test]
    fn splits_lf_crlf_and_cr() {
        assert_eq!(
            wrap_text_with_ansi("first\nsecond\r\nthird\rfourth", 80),
            vec!["first", "second", "third", "fourth"]
        );
    }

    #[test]
    fn carries_style_across_crlf_and_cr() {
        assert_eq!(
            wrap_text_with_ansi(&format!("{RED}first\r\nsecond\rthird{RESET}"), 80),
            vec![
                format!("{RED}first"),
                format!("{RED}second"),
                format!("{RED}third{RESET}")
            ]
        );
    }

    #[test]
    fn wraps_plain_text_within_width() {
        let wrapped = wrap_text_with_ansi("hello world this is a test", 10);
        assert!(wrapped.len() > 1);
        assert!(wrapped.iter().all(|line| visible_width(line) <= 10));
    }

    #[test]
    fn breaks_cjk_runs_at_grapheme_boundaries() {
        let text = "This is an example 中文汉字测试段落内容中文汉字测试段落内容.";
        assert_eq!(
            wrap_text_with_ansi(text, 40),
            vec![
                "This is an example 中文汉字测试段落内容",
                "中文汉字测试段落内容."
            ]
        );
        let styled = wrap_text_with_ansi(&format!("{RED}{text}{RESET}"), 40);
        assert_eq!(
            styled,
            vec![
                format!("{RED}This is an example 中文汉字测试段落内容"),
                format!("{RED}中文汉字测试段落内容.{RESET}")
            ]
        );
    }

    #[test]
    fn osc_markers_take_no_width() {
        assert_eq!(visible_width("\x1b]133;A\x07hello\x1b]133;B\x07"), 5);
        assert_eq!(visible_width("\x1b]133;A\x1b\\hello\x1b]133;B\x1b\\"), 5);
    }

    #[test]
    fn trailing_whitespace_does_not_exceed_width() {
        assert!(visible_width(&wrap_text_with_ansi("  ", 1)[0]) <= 1);
    }

    #[test]
    fn colour_reopens_on_every_continuation() {
        let wrapped = wrap_text_with_ansi(&format!("{RED}hello world this is red{RESET}"), 10);
        for line in &wrapped[1..] {
            assert!(line.starts_with(RED), "{line:?}");
        }
        for line in &wrapped[..wrapped.len() - 1] {
            assert!(!line.ends_with(RESET), "{line:?}");
        }
    }

    #[test]
    fn hyperlink_reopens_and_closes_on_each_row() {
        let open = "\x1b]8;;https://example.com\x1b\\";
        let close = "\x1b]8;;\x1b\\";
        let lines = wrap_text_with_ansi(&format!("{open}0123456789{close}"), 6);
        assert!(lines.len() > 1);
        for line in &lines {
            if !strip_ansi(line).trim().is_empty() {
                assert!(line.contains(open), "{line:?}");
            }
        }
        for line in &lines[..lines.len() - 1] {
            assert!(line.ends_with(close), "{line:?}");
        }
    }

    #[test]
    fn hyperlink_keeps_bel_terminator() {
        let url = format!("https://example.com/oauth/{}", "a".repeat(32));
        let open = format!("\x1b]8;;{url}\x07");
        let lines = wrap_text_with_ansi(&format!("{open}{url}\x1b]8;;\x07"), 20);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(line.contains(&open), "{line:?}");
            assert!(!line.contains(&format!("\x1b]8;;{url}\x1b\\")), "{line:?}");
        }
        for line in &lines[..lines.len() - 1] {
            assert!(line.ends_with("\x1b]8;;\x07"), "{line:?}");
        }
    }

    #[test]
    fn fitting_hyperlink_is_not_reopened() {
        let open = "\x1b]8;;https://example.com\x1b\\";
        let lines = wrap_text_with_ansi(&format!("before {open}link\x1b]8;;\x1b\\ after"), 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].matches(open).count(), 1);
        assert_eq!(lines[0].matches("\x1b]8;;\x1b\\").count(), 1);
    }

    #[test]
    fn rgb_colours_carry_with_five_parameters() {
        // ice AnsiCodeTracker consumes 38;2;r;g;b as one colour; a following
        // parameter is applied on its own.
        let wrapped = wrap_text_with_ansi("\x1b[38;2;1;2;3;1mhello world", 6);
        assert_eq!(wrapped[1], "\x1b[1;38;2;1;2;3mworld");
    }

    #[test]
    fn width_zero_wraps_like_width_one_and_keeps_text() {
        assert_eq!(wrap_text_with_ansi("ab", 0), vec!["a", "b"]);
        assert_eq!(wrap_text_with_ansi("界", 1), vec!["界"]);
        assert_eq!(wrap_text_with_ansi("", 5), vec![""]);
    }
}
