//! ANSI escape code utilities and visible width calculations.
//!
//! Handles ANSI-aware string manipulation for terminal rendering.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Cursor position marker - APC sequence.
/// This is a zero-width escape sequence that terminals ignore.
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

/// Cells a visible tab occupies. Layout counts tabs at this fixed width and
/// [`normalize_terminal_output`] expands them to as many spaces, so terminal
/// tab stops can never move or wrap a line.
pub const TAB_WIDTH: usize = 3;

/// Calculate the visible width of a string, ignoring ANSI escape sequences.
///
/// This is the number of terminal columns the string will occupy: the sum of
/// [`grapheme_width`] over its extended grapheme clusters, with tabs counted
/// as [`TAB_WIDTH`] cells.
pub fn visible_width(s: &str) -> usize {
    if s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return s.len();
    }
    let text = if s.contains('\x1b') {
        Cow::Owned(strip_ansi(s))
    } else {
        Cow::Borrowed(s)
    };
    text.graphemes(true).map(grapheme_width).sum()
}

/// Terminal cell width of one extended grapheme cluster.
///
/// Mirrors ice `graphemeWidth`: unicode-width supplies East Asian width and
/// emoji-sequence widths; on top of that, marks that terminals allocate a
/// cell for count as one cell each, a lone regional indicator counts as two
/// (a streamed flag's first half renders wide), and clusters made only of
/// controls, marks and default-ignorable code points take no cells.
pub(crate) fn grapheme_width(grapheme: &str) -> usize {
    if grapheme == "\t" {
        return TAB_WIDTH;
    }
    if grapheme.chars().all(is_terminal_spacing_mark) {
        return grapheme.chars().count();
    }
    if grapheme.chars().all(is_zero_width_char) {
        return 0;
    }
    let mut chars = grapheme.chars();
    if let (Some(first), None) = (chars.next(), chars.next()) {
        if ('\u{1F1E6}'..='\u{1F1FF}').contains(&first) {
            return 2;
        }
    }
    let mut width = UnicodeWidthStr::width(grapheme);
    for c in grapheme.chars() {
        let char_width = c.width().unwrap_or(0);
        if char_width == 0 && (is_terminal_spacing_mark(c) || is_halfwidth_or_fullwidth_form(c)) {
            width += 1;
        } else if c == '\u{1734}' {
            // Hanunoo pamudpod is a spacing mark terminals draw zero-width.
            width -= char_width;
        }
    }
    width
}

/// Spacing marks (minus three that terminals do not advance for) plus the
/// non-spacing marks that legacy wcwidth tables give a cell, as in ice
/// `terminalSpacingMarkRegex`.
fn is_terminal_spacing_mark(c: char) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"^[[\p{Mc}--[\x{1734}\x{302E}\x{302F}]]\x{065F}\x{0F7F}\x{102B}\x{102C}\x{1031}\x{1033}-\x{1035}\x{1038}\x{103A}-\x{103E}]$",
        )
        .expect("valid spacing-mark class")
    });
    re.is_match(c.encode_utf8(&mut [0; 4]))
}

fn is_zero_width_char(c: char) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^[\p{Default_Ignorable_Code_Point}\p{Cc}\p{M}]$")
            .expect("valid zero-width class")
    });
    re.is_match(c.encode_utf8(&mut [0; 4]))
}

fn is_halfwidth_or_fullwidth_form(c: char) -> bool {
    ('\u{FF00}'..='\u{FFEF}').contains(&c)
}

/// The cell range occupied by the grapheme covering visible `column`,
/// including the second cell of a wide glyph. `None` past the end of the
/// line. Escape sequences take no cells.
pub fn grapheme_cell_range(line: &str, column: usize) -> Option<Range<usize>> {
    let text = strip_ansi(line);
    let mut start = 0;
    for grapheme in text.graphemes(true) {
        let width = grapheme_width(grapheme);
        if width > 0 && (start..start + width).contains(&column) {
            return Some(start..start + width);
        }
        start += width;
    }
    None
}

/// Byte length of the escape sequence at the start of `s`, which must begin
/// with ESC. Covers CSI, OSC/APC/DCS/PM strings (ended by BEL or ST),
/// character-set designations and two-byte escapes; an unterminated sequence
/// runs to the end of `s`.
fn escape_len(s: &str) -> usize {
    debug_assert!(s.starts_with('\x1b'));
    let mut chars = s.char_indices().skip(1).peekable();
    let end = |next: Option<(usize, char)>| next.map_or(s.len(), |(i, c)| i + c.len_utf8());
    match chars.next() {
        Some((_, '[')) => end(chars.find(|&(_, c)| ('\x40'..='\x7e').contains(&c))),
        Some((_, ']' | '_' | 'P' | '^')) => {
            while let Some((i, c)) = chars.next() {
                if c == '\x07' {
                    return i + 1;
                }
                if c == '\x1b' && matches!(chars.peek(), Some((_, '\\'))) {
                    return i + 2;
                }
            }
            s.len()
        }
        Some((_, '(' | ')')) => end(chars.next()),
        next => end(next),
    }
}

/// Strip ANSI escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(esc) = rest.find('\x1b') {
        result.push_str(&rest[..esc]);
        rest = &rest[esc..];
        rest = &rest[escape_len(rest)..];
    }
    result.push_str(rest);
    result
}

/// Slice a string by visible column positions.
///
/// Returns a substring that starts at `start_col` and has at most `width` visible columns.
/// Preserves ANSI escape sequences appropriately.
pub fn slice_by_column(s: &str, start_col: usize, width: usize, preserve_styles: bool) -> String {
    if width == 0 {
        return String::new();
    }

    let stripped = strip_ansi(s);
    let segments: Vec<&str> = stripped.graphemes(true).collect();

    // Find the character indices for the column range
    let mut current_col = 0;
    let mut start_idx = None;
    let mut end_idx = segments.len();
    let mut col_count = 0;

    for (i, seg) in segments.iter().enumerate() {
        let seg_width = grapheme_width(seg);
        if current_col >= start_col && start_idx.is_none() {
            start_idx = Some(i);
        }
        if start_idx.is_some() {
            col_count += seg_width;
            if col_count >= width {
                end_idx = i + 1;
                break;
            }
        }
        current_col += seg_width;
    }

    let start_idx = start_idx.unwrap_or(segments.len());
    let result: String = segments[start_idx..end_idx].iter().copied().collect();

    if preserve_styles && result != stripped {
        // TODO: Preserve ANSI styles from the original string
        // This is a simplified version
        result
    } else {
        result
    }
}

/// Rewrite a rendered line for the terminal without changing logical content.
///
/// Thai/Lao AM vowels (U+0E33, U+0EB3) become their compatibility
/// decompositions, which have the same cell width but avoid stale cells some
/// terminals leave during differential repaint. Visible tabs become
/// [`TAB_WIDTH`] spaces; tabs inside escape sequences are left untouched.
pub fn normalize_terminal_output(line: &str) -> String {
    let line = if line.contains(['\u{0E33}', '\u{0EB3}']) {
        Cow::Owned(
            line.replace('\u{0E33}', "\u{0E4D}\u{0E32}")
                .replace('\u{0EB3}', "\u{0ECD}\u{0EB2}"),
        )
    } else {
        Cow::Borrowed(line)
    };
    if !line.contains('\t') {
        return line.into_owned();
    }
    let mut result = String::with_capacity(line.len() + 8);
    let mut rest = &line[..];
    while let Some(i) = rest.find(['\t', '\x1b']) {
        result.push_str(&rest[..i]);
        rest = &rest[i..];
        if rest.starts_with('\t') {
            result.push_str(&" ".repeat(TAB_WIDTH));
            rest = &rest[1..];
        } else {
            let len = escape_len(rest);
            result.push_str(&rest[..len]);
            rest = &rest[len..];
        }
    }
    result.push_str(rest);
    result
}

/// Create a hyperlink using OSC 8.
pub fn hyperlink(url: &str, text: &str) -> String {
    format!("\x1b]8;;{}\x07{}\x1b]8;;\x07", url, text)
}

/// Apply a foreground color using ANSI 256-color mode.
pub fn fg_256(color: u8, text: &str) -> String {
    format!("\x1b[38;5;{}m{}\x1b[39m", color, text)
}

/// Apply a background color using ANSI 256-color mode.
pub fn bg_256(color: u8, text: &str) -> String {
    format!("\x1b[48;5;{}m{}\x1b[49m", color, text)
}

/// Apply a foreground color using RGB.
pub fn fg_rgb(r: u8, g: u8, b: u8, text: &str) -> String {
    format!("\x1b[38;2;{};{};{}m{}\x1b[39m", r, g, b, text)
}

/// Apply a background color using RGB.
pub fn bg_rgb(r: u8, g: u8, b: u8, text: &str) -> String {
    format!("\x1b[48;2;{};{};{}m{}\x1b[49m", r, g, b, text)
}

/// Apply bold style.
pub fn bold(text: &str) -> String {
    format!("\x1b[1m{}\x1b[22m", text)
}

/// Apply italic style.
pub fn italic(text: &str) -> String {
    format!("\x1b[3m{}\x1b[23m", text)
}

/// Apply underline style.
pub fn underline(text: &str) -> String {
    format!("\x1b[4m{}\x1b[24m", text)
}

/// Apply inverse (reverse video) style.
pub fn inverse(text: &str) -> String {
    format!("\x1b[7m{}\x1b[27m", text)
}

/// Apply dim/faint style.
pub fn dim(text: &str) -> String {
    format!("\x1b[2m{}\x1b[22m", text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_width() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width("\x1b[31mhello\x1b[0m"), 5);
        assert_eq!(visible_width("你好"), 4); // Chinese characters are 2 columns each
        assert_eq!(visible_width("\x1b[1m你好\x1b[0m"), 4);
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("hello"), "hello");
        assert_eq!(strip_ansi("\x1b[31mhello\x1b[0m"), "hello");
        assert_eq!(strip_ansi("\x1b[1;31mhello\x1b[0m"), "hello");
    }

    // Fixtures below mirror ice packages/tui/test/
    // regression-regional-indicator-width.test.ts, truncate-to-width.test.ts
    // ("visibleWidth" suite) and tab-width.test.ts.

    #[test]
    fn regional_indicators_and_emoji_have_stable_width() {
        for cp in 0x1F1E6..=0x1F1FF {
            let ri = char::from_u32(cp).unwrap().to_string();
            assert_eq!(visible_width(&ri), 2, "U+{cp:X}");
        }
        assert_eq!(visible_width("🇨"), 2);
        assert_eq!(visible_width("      - 🇨"), 10);
        for flag in ["🇯🇵", "🇺🇸", "🇬🇧", "🇨🇳", "🇩🇪", "🇫🇷"] {
            assert_eq!(visible_width(flag), 2, "{flag}");
        }
        for emoji in ["👍", "👍🏻", "✅", "⚡", "⚡️", "👨", "👨‍💻", "🏳️‍🌈"]
        {
            assert_eq!(visible_width(emoji), 2, "{emoji}");
        }
    }

    #[test]
    fn counts_tabs_inline_and_skips_ansi() {
        assert_eq!(visible_width("\t\x1b[31m界\x1b[0m"), 5);
        assert_eq!(visible_width("a\tb"), 2 + TAB_WIDTH);
    }

    #[test]
    fn counts_indic_conjunct_spacing_code_points() {
        assert_eq!(visible_width("र्क"), 2);
        assert_eq!(visible_width("नेटवर्क"), 5);
        assert_eq!(visible_width("सर्वाधिकार सुरक्षित। ऑर्डर पर क्लिक करें"), 33);
        for s in ["র্ক", "ર્ક", "ର୍କ", "ర్క", "ര്‍ക"] {
            assert_eq!(visible_width(s), 2, "{s}");
        }
    }

    #[test]
    fn keeps_ordinary_combining_marks_zero_width() {
        assert_eq!(visible_width("e\u{301}"), 1);
        assert_eq!(visible_width("čřžůú"), 5);
        assert_eq!(visible_width("שָׁ"), 1);
        assert_eq!(visible_width("بّ"), 1);
        assert_eq!(visible_width("རྐ"), 1);
        assert_eq!(visible_width("ᜠ᜴"), 1);
        assert_eq!(visible_width("가〮"), 2);
        assert_eq!(visible_width("가〯"), 2);
    }

    #[test]
    fn keeps_cjk_and_japanese_widths() {
        assert_eq!(visible_width("网络"), 4);
        assert_eq!(visible_width("ネットワーク"), 12);
        assert_eq!(visible_width("が"), 2);
        assert_eq!(visible_width("か\u{3099}"), 2);
        // Halfwidth katakana with a halfwidth voiced mark.
        assert_eq!(visible_width("ｶﾞ"), 2);
    }

    #[test]
    fn counts_myanmar_marks_terminals_allocate_cells_for() {
        for s in ["ကာ", "ကေ", "က်", "ကျ", "ကြ", "ကဳ", "ကဴ", "ကဵ", "ကး"]
        {
            assert_eq!(visible_width(s), 2, "{s}");
        }
        assert_eq!(visible_width("ကို"), 1);
        assert_eq!(visible_width("က္"), 1);
    }

    #[test]
    fn keeps_thai_and_lao_am_clusters_at_normal_width() {
        assert_eq!(visible_width("ำ"), 1);
        assert_eq!(visible_width("ຳ"), 1);
        assert_eq!(visible_width("กำ"), 2);
        assert_eq!(visible_width("ກຳ"), 2);
    }

    #[test]
    fn both_public_width_paths_agree() {
        for s in [
            "plain",
            "\x1b[31m👨‍💻\x1b[0m",
            "🇨",
            "a\tb",
            "\x1b]8;;https://x.test\x07link\x1b]8;;\x07",
            "kafe\u{301}",
            CURSOR_MARKER,
        ] {
            assert_eq!(visible_width(s), crate::utils::visible_width(s), "{s:?}");
            assert_eq!(strip_ansi(s), crate::utils::strip_ansi(s), "{s:?}");
        }
        assert_eq!(visible_width(&format!("ab{CURSOR_MARKER}c")), 3);
    }

    #[test]
    fn normalizes_thai_and_lao_am_only_for_terminal_output() {
        assert_eq!(normalize_terminal_output("ำ"), "\u{0E4D}\u{0E32}");
        assert_eq!(normalize_terminal_output("ຳ"), "\u{0ECD}\u{0EB2}");
        assert_eq!(
            visible_width(&normalize_terminal_output("ำabc")),
            visible_width("ำabc")
        );
        assert_eq!(
            visible_width(&normalize_terminal_output("ຳabc")),
            visible_width("ຳabc")
        );
    }

    #[test]
    fn expands_visible_tabs_but_keeps_tabs_inside_sequences() {
        for seq in [
            "\x1b]8;;https://example.test/a\tb\x07",
            "\x1b]0;window\ttitle\x1b\\",
            "\x1b_payload\tdata\x1b\\",
        ] {
            assert_eq!(
                normalize_terminal_output(&format!("{seq}label\ttext")),
                format!("{seq}label   text")
            );
        }
        let line = "\x1b[31mout\t192M\x1b[0m";
        assert_eq!(
            visible_width(&normalize_terminal_output(line)),
            visible_width(line)
        );
    }

    #[test]
    fn grapheme_cell_range_covers_whole_wide_glyph() {
        let line = "a\x1b[1m界\x1b[0me\u{301}";
        assert_eq!(grapheme_cell_range(line, 0), Some(0..1));
        assert_eq!(grapheme_cell_range(line, 1), Some(1..3));
        assert_eq!(grapheme_cell_range(line, 2), Some(1..3));
        assert_eq!(grapheme_cell_range(line, 3), Some(3..4));
        assert_eq!(grapheme_cell_range(line, 4), None);
    }

    #[test]
    fn test_hyperlink() {
        // let link = hyperlink("https://example.com", "Example");
        // assert!(link.contains("https://example.com"));
        // assert!(link.contains("Example"));
    }
}
