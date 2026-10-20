//! Utilities for TUI rendering.
//!
//! Provides functions for text manipulation with ANSI awareness.

use unicode_segmentation::UnicodeSegmentation;

use crate::ansi::StyleTracker;

/// Get the visible width of a string, ignoring ANSI escape sequences.
///
/// Same measurement as [`crate::ansi::visible_width`]; both names stay public.
pub fn visible_width(s: &str) -> usize {
    crate::ansi::visible_width(s)
}

/// Slice a string by column positions, keeping its styling.
///
/// Takes the graphemes that start in `start_col..start_col + max_cols`,
/// leaving out a wide grapheme that would cross either edge, so the result
/// never exceeds `max_cols` cells. With `pad` the result is padded with
/// spaces to exactly `max_cols` cells. See [`slice_with_width`] for the
/// non-strict form.
pub fn slice_by_column(s: &str, start_col: usize, max_cols: usize, pad: bool) -> String {
    let (mut text, width) = slice_with_width(s, start_col, max_cols, true);
    if pad {
        text.push_str(&" ".repeat(max_cols.saturating_sub(width)));
    }
    text
}

pub use crate::ansi::slice_with_width;

const TRUNCATION_RESET: &str = "\x1b[0m";

/// Truncate to `max_width` cells, ending with `suffix` when text is cut.
///
/// Port of ice `truncateToWidth` with `pad = false`; see
/// [`truncate_to_width_padded`].
pub fn truncate_to_width(s: &str, max_width: usize, suffix: &str) -> String {
    truncate(s, max_width, suffix, false)
}

/// [`truncate_to_width`], then padded with spaces to exactly `max_width`
/// cells.
pub fn truncate_to_width_padded(s: &str, max_width: usize, suffix: &str) -> String {
    truncate(s, max_width, suffix, true)
}

/// Text that fits is returned unchanged. Otherwise the longest contiguous
/// grapheme prefix that leaves room for `suffix` is kept (a wide grapheme
/// that does not fit ends the prefix; nothing later is kept), then a reset,
/// the suffix and another reset. A suffix wider than the target is itself
/// cut to whole graphemes. Escape sequences are kept only ahead of kept text.
fn truncate(text: &str, max_width: usize, suffix: &str, pad: bool) -> String {
    let padded = |s: String, width: usize| {
        if pad {
            s + &" ".repeat(max_width.saturating_sub(width))
        } else {
            s
        }
    };
    if max_width == 0 {
        return String::new();
    }
    if text.is_empty() {
        return padded(String::new(), 0);
    }
    let suffix_width = visible_width(suffix);
    if suffix_width >= max_width {
        let text_width = visible_width(text);
        if text_width <= max_width {
            return padded(text.to_string(), text_width);
        }
        let (clipped, clipped_width) = crate::ansi::slice_with_width(suffix, 0, max_width, true);
        if clipped_width == 0 {
            return padded(String::new(), 0);
        }
        return padded(
            format!("{TRUNCATION_RESET}{clipped}{TRUNCATION_RESET}"),
            clipped_width,
        );
    }
    let target = max_width - suffix_width;
    let mut kept = String::new();
    let mut kept_width = 0;
    let mut pending = String::new();
    let mut contiguous = true;
    let mut seen = 0;
    let mut rest = text;
    'scan: while !rest.is_empty() {
        if rest.starts_with('\x1b') {
            let len = crate::ansi::escape_len(rest);
            pending.push_str(&rest[..len]);
            rest = &rest[len..];
            continue;
        }
        let end = rest.find('\x1b').unwrap_or(rest.len());
        for grapheme in rest[..end].graphemes(true) {
            let w = crate::ansi::grapheme_width(grapheme);
            if contiguous && kept_width + w <= target {
                kept.push_str(&std::mem::take(&mut pending));
                kept.push_str(grapheme);
                kept_width += w;
            } else {
                contiguous = false;
                pending.clear();
            }
            seen += w;
            if seen > max_width {
                break 'scan;
            }
        }
        rest = &rest[end..];
    }
    if seen <= max_width {
        return padded(text.to_string(), seen);
    }
    let result = if suffix.is_empty() {
        format!("{kept}{TRUNCATION_RESET}")
    } else {
        format!("{kept}{TRUNCATION_RESET}{suffix}{TRUNCATION_RESET}")
    };
    padded(result, kept_width + suffix_width)
}

/// The text left of an overlay and the text under-and-after it, in one pass
/// (ice `extractSegments`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Segments {
    /// Graphemes that end at or before `before_end`.
    pub before: String,
    pub before_width: usize,
    /// Graphemes starting in `after_start..after_start + after_len`, opened
    /// with the style and hyperlink active at that point.
    pub after: String,
    pub after_width: usize,
}

/// Split `line` around an overlay. `before` never includes a grapheme that
/// crosses `before_end`; `strict_after` likewise keeps `after` within
/// `after_len` cells.
pub fn extract_segments(
    line: &str,
    before_end: usize,
    after_start: usize,
    after_len: usize,
    strict_after: bool,
) -> Segments {
    let mut out = Segments::default();
    let mut tracker = StyleTracker::default();
    let mut pending_before = String::new();
    let mut after_started = false;
    let after_end = after_start + after_len;
    let done = |col: usize| {
        if after_len == 0 {
            col >= before_end
        } else {
            col >= after_end
        }
    };
    let mut col = 0;
    let mut rest = line;
    while !rest.is_empty() && !done(col) {
        if rest.starts_with('\x1b') {
            let len = crate::ansi::escape_len(rest);
            let code = &rest[..len];
            tracker.process(code);
            if col < before_end {
                pending_before.push_str(code);
            } else if col >= after_start && col < after_end && after_started {
                out.after.push_str(code);
            }
            rest = &rest[len..];
            continue;
        }
        let end = rest.find('\x1b').unwrap_or(rest.len());
        for grapheme in rest[..end].graphemes(true) {
            let w = crate::ansi::grapheme_width(grapheme);
            if col < before_end && col + w <= before_end {
                out.before.push_str(&std::mem::take(&mut pending_before));
                out.before.push_str(grapheme);
                out.before_width += w;
            } else if col >= after_start
                && col < after_end
                && (!strict_after || col + w <= after_end)
            {
                if !after_started {
                    out.after.push_str(&tracker.active_codes());
                    after_started = true;
                }
                out.after.push_str(grapheme);
                out.after_width += w;
            }
            col += w;
            if done(col) {
                break;
            }
        }
        rest = &rest[end..];
    }
    out
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

/// URL of an OSC 8 open sequence, `Some("")` for a close, `None` for any
/// other sequence. The URL follows the second `;` (`8;params;url`).
fn osc8_url(code: &str) -> Option<&str> {
    let body = code.strip_prefix("\x1b]8;")?;
    let body = body
        .strip_suffix('\x07')
        .or_else(|| body.strip_suffix("\x1b\\"))?;
    let (_, url) = body.split_once(';')?;
    (!url.contains(['\x07', '\x1b'])).then_some(url)
}

/// The OSC 8 hyperlink URL covering visible `column`, for every cell of a
/// wide grapheme (ice `getOsc8LinkAtColumn`).
pub fn get_osc8_link_at_column(s: &str, column: usize) -> Option<String> {
    let mut active: Option<&str> = None;
    let mut col = 0;
    let mut rest = s;
    while !rest.is_empty() {
        if rest.starts_with('\x1b') {
            let len = crate::ansi::escape_len(rest);
            if let Some(url) = osc8_url(&rest[..len]) {
                active = (!url.is_empty()).then_some(url);
            }
            rest = &rest[len..];
            continue;
        }
        let end = rest.find('\x1b').unwrap_or(rest.len());
        for grapheme in rest[..end].graphemes(true) {
            let w = crate::ansi::grapheme_width(grapheme);
            if (col..col + w).contains(&column) {
                return active.map(str::to_string);
            }
            col += w;
        }
        rest = &rest[end..];
    }
    None
}

/// Rewrite a line's OSC 8 links: `visit` maps each text run (with the URL
/// around it) and `keep_link` decides which link sequences stay. A close
/// sequence is judged by the link it closes.
fn rewrite_osc8_links(
    line: &str,
    keep_link: impl Fn(&str) -> bool,
    visit: impl Fn(&str, Option<&str>) -> String,
) -> String {
    if !line.contains("\x1b]8;") {
        return line.to_string();
    }
    let mut result = String::with_capacity(line.len());
    let mut active: Option<&str> = None;
    let mut rest = line;
    while !rest.is_empty() {
        if rest.starts_with('\x1b') {
            let len = crate::ansi::escape_len(rest);
            let code = &rest[..len];
            match osc8_url(code) {
                Some(url) => {
                    let url = (!url.is_empty()).then_some(url);
                    if keep_link(url.or(active).unwrap_or("")) {
                        result.push_str(code);
                    }
                    active = url;
                }
                None => result.push_str(code),
            }
            rest = &rest[len..];
            continue;
        }
        let end = rest.find('\x1b').unwrap_or(rest.len());
        result.push_str(&visit(&rest[..end], active));
        rest = &rest[end..];
    }
    result
}

/// Remove the OSC 8 wrappers of links whose URL matches, keeping their text
/// and styling (ice `stripOsc8Links`).
pub fn strip_osc8_links(line: &str, matches: impl Fn(&str) -> bool) -> String {
    rewrite_osc8_links(line, |url| !matches(url), |text, _| text.to_string())
}

/// Wrap every text run inside the link to `url` in `open`/`close`, so the
/// decoration survives inner style changes (ice `styleOsc8Link`).
pub fn style_osc8_link(line: &str, url: &str, open: &str, close: &str) -> String {
    rewrite_osc8_links(
        line,
        |_| true,
        |text, active| {
            if active == Some(url) {
                format!("{open}{text}{close}")
            } else {
                text.to_string()
            }
        },
    )
}

/// Whether an SGR parameter list resets the background: empty, `0` or
/// `49`, skipping the arguments of 38/48 extended colours.
fn sgr_resets_background(params: &str) -> bool {
    if params.is_empty() {
        return true;
    }
    let codes: Vec<&str> = params.split(';').collect();
    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            "38" | "48" => {
                i += if codes.get(i + 1) == Some(&"2") { 5 } else { 3 };
                continue;
            }
            "" | "0" | "49" => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// Pad `line` to `width` cells and apply `bg_fn`, re-applying it after
/// every SGR that resets the background so padding stays covered. An
/// already-wider line is not truncated (ice `applyBackgroundToLine`).
pub fn apply_background_to_line(
    line: &str,
    width: usize,
    bg_fn: impl Fn(&str) -> String,
) -> String {
    let padded = format!(
        "{line}{}",
        " ".repeat(width.saturating_sub(visible_width(line)))
    );
    let mut result = String::with_capacity(padded.len() + 16);
    let mut span_start = 0;
    let mut offset = 0;
    while let Some(i) = padded[offset..].find("\x1b[") {
        let start = offset + i;
        let len = crate::ansi::escape_len(&padded[start..]);
        let code = &padded[start..start + len];
        offset = start + len;
        let Some(params) = code[2..].strip_suffix('m') else {
            continue;
        };
        if params.bytes().all(|b| b.is_ascii_digit() || b == b';') && sgr_resets_background(params)
        {
            result.push_str(&bg_fn(&padded[span_start..start]));
            result.push_str(code);
            span_start = offset;
        }
    }
    result.push_str(&bg_fn(&padded[span_start..]));
    result
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
        assert_eq!(truncate_to_width("Hello World", 5, ""), "Hello\x1b[0m");
        assert_eq!(
            truncate_to_width("Hello World", 8, "..."),
            "Hello\x1b[0m...\x1b[0m"
        );
        assert_eq!(truncate_to_width("Hello", 8, "..."), "Hello");
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

    // truncate-to-width.test.ts ("truncateToWidth"), background-line.test.ts
    // and tab-width.test.ts.

    #[test]
    fn truncation_stays_within_width_for_huge_unicode_input() {
        let text = "🙂界".repeat(100_000);
        let truncated = truncate_to_width(&text, 40, "…");
        assert!(visible_width(&truncated) <= 40);
        assert!(truncated.ends_with("…\x1b[0m"));
    }

    #[test]
    fn truncation_keeps_styles_and_brackets_the_ellipsis_with_resets() {
        let text = format!("\x1b[31m{}\x1b[0m", "hello ".repeat(1000));
        let truncated = truncate_to_width(&text, 20, "…");
        assert!(visible_width(&truncated) <= 20);
        assert!(truncated.contains("\x1b[31m"));
        assert!(truncated.ends_with("\x1b[0m…\x1b[0m"));
    }

    #[test]
    fn truncation_survives_malformed_escapes() {
        let text = format!("abc\x1bnot-ansi {}", "🙂".repeat(1000));
        assert!(visible_width(&truncate_to_width(&text, 20, "…")) <= 20);
    }

    #[test]
    fn wide_ellipsis_is_clipped_and_bracketed() {
        assert_eq!(truncate_to_width("abcdef", 1, "🙂"), "");
        assert_eq!(truncate_to_width("abcdef", 2, "🙂"), "\x1b[0m🙂\x1b[0m");
        assert_eq!(truncate_to_width("a", 2, "🙂"), "a");
        assert_eq!(truncate_to_width("界", 2, "🙂"), "界");
    }

    #[test]
    fn padded_truncation_fills_the_width() {
        assert_eq!(
            visible_width(&truncate_to_width_padded("🙂界🙂界🙂界", 8, "…")),
            8
        );
        assert_eq!(truncate_to_width_padded("", 3, "…"), "   ");
        assert_eq!(truncate_to_width_padded("ab", 4, "…"), "ab  ");
    }

    #[test]
    fn truncation_without_ellipsis_ends_with_reset() {
        let truncated = truncate_to_width(&format!("\x1b[31m{}", "hello".repeat(100)), 10, "");
        assert!(visible_width(&truncated) <= 10);
        assert!(truncated.ends_with("\x1b[0m"));
    }

    #[test]
    fn truncation_keeps_a_contiguous_prefix() {
        assert_eq!(
            truncate_to_width_padded("🙂\t界 \x1b_abc\x07", 7, "…"),
            "🙂\t\x1b[0m…\x1b[0m "
        );
    }

    #[test]
    fn background_is_reapplied_after_full_resets() {
        let bg = |text: &str| format!("\x1b[44m{text}\x1b[49m");
        let rendered = apply_background_to_line("before\x1b[0mafter", 12, bg);
        assert!(rendered.contains("\x1b[0m\x1b[44m"), "{rendered:?}");
        assert!(rendered.contains("\x1b[44mafter \x1b[49m"), "{rendered:?}");
    }

    #[test]
    fn background_is_reapplied_after_default_background_resets() {
        let bg = |text: &str| format!("\x1b[44m{text}\x1b[49m");
        let rendered = apply_background_to_line("\x1b[100mcode\x1b[49m tail", 12, bg);
        assert!(
            rendered.contains("\x1b[49m\x1b[44m tail   \x1b[49m"),
            "{rendered:?}"
        );
    }

    #[test]
    fn extended_colour_arguments_are_not_background_resets() {
        let bg = |text: &str| format!("\x1b[44m{text}\x1b[49m");
        assert_eq!(
            apply_background_to_line("\x1b[38;5;0mx\x1b[48;2;0;0;49my", 4, bg),
            bg("\x1b[38;5;0mx\x1b[48;2;0;0;49my  ")
        );
    }

    #[test]
    fn slice_widths_agree_with_visible_width() {
        let text = "out 192M\t.ice/skill-tests/results-ha";
        let (slice, width) = slice_with_width(text, 0, 10, true);
        assert_eq!((slice.as_str(), width), ("out 192M", 8));
        assert_eq!(visible_width(&slice), width);

        let segments = extract_segments(text, 10, 13, 10, true);
        assert_eq!(
            (segments.before.as_str(), segments.before_width),
            ("out 192M", 8)
        );
        let tab_fits = extract_segments(text, 11, 13, 10, true);
        assert_eq!(tab_fits.before, "out 192M\t");
        assert_eq!(tab_fits.before_width, 11);
        assert_eq!(visible_width(&tab_fits.before), tab_fits.before_width);
    }

    #[test]
    fn slices_keep_styles_and_respect_wide_edges() {
        let line = "\x1b[31mab界cd\x1b[0m";
        assert_eq!(
            slice_with_width(line, 1, 2, false),
            ("\x1b[31mb界".into(), 3)
        );
        assert_eq!(slice_with_width(line, 1, 2, true), ("\x1b[31mb".into(), 1));
        // A glyph starting left of the span is never included.
        assert_eq!(slice_with_width(line, 3, 2, true), ("\x1b[31mc".into(), 1));
        assert_eq!(slice_by_column(line, 1, 2, true), "\x1b[31mb ");
        assert_eq!(crate::ansi::slice_by_column(line, 0, 2, false), "ab");
    }

    #[test]
    fn after_segment_inherits_style_set_under_the_overlay() {
        let segments = extract_segments("ab\x1b[1mcd\x1b]8;;https://x\x07ef", 1, 4, 2, true);
        assert_eq!(segments.before, "a");
        assert_eq!(segments.after, "\x1b[1m\x1b]8;;https://x\x07ef");
        assert_eq!(segments.after_width, 2);
    }

    #[test]
    fn osc8_hit_test_reads_url_after_params_and_clears_on_close() {
        let open = "\x1b]8;id=1;https://example.com\x1b\\";
        let line = format!("a{open}界b\x1b]8;;\x1b\\c");
        assert_eq!(get_osc8_link_at_column(&line, 0), None);
        for column in [1, 2, 3] {
            assert_eq!(
                get_osc8_link_at_column(&line, column).as_deref(),
                Some("https://example.com")
            );
        }
        assert_eq!(get_osc8_link_at_column(&line, 4), None);
        assert_eq!(get_osc8_link_at_column(&line, 9), None);
    }

    #[test]
    fn strip_osc8_links_removes_only_matching_wrappers() {
        let a = "\x1b]8;;https://a\x07";
        let b = "\x1b]8;;https://b\x1b\\";
        let line = format!("{a}\x1b[1mA\x1b]8;;\x07 {b}B\x1b]8;;\x1b\\");
        assert_eq!(
            strip_osc8_links(&line, |url| url == "https://a"),
            format!("\x1b[1mA {b}B\x1b]8;;\x1b\\")
        );
    }

    #[test]
    fn style_osc8_link_wraps_each_run_of_the_link() {
        let a = "\x1b]8;;https://a\x07";
        let line = format!("x{a}one\x1b[1mtwo\x1b]8;;\x07y");
        assert_eq!(
            style_osc8_link(&line, "https://a", "<", ">"),
            format!("x{a}<one>\x1b[1m<two>\x1b]8;;\x07y")
        );
    }

    #[test]
    fn width_zero_wraps_like_width_one_and_keeps_text() {
        assert_eq!(wrap_text_with_ansi("ab", 0), vec!["a", "b"]);
        assert_eq!(wrap_text_with_ansi("界", 1), vec!["界"]);
        assert_eq!(wrap_text_with_ansi("", 5), vec![""]);
    }
}
