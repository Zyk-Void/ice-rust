//! JavaScript string semantics used by the Blackhole compiler.
//!
//! The TypeScript source measures and slices strings in UTF-16 code units.
//! These helpers reproduce `length`, `slice` and `lastIndexOf` on that unit.
//! A cut that would land inside a surrogate pair is moved back to the start
//! of the character, because a Rust `str` cannot hold a lone surrogate.

/// `text.length` in UTF-16 code units.
pub fn len16(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// Byte offset of UTF-16 index `index`, rounded down to a char boundary and
/// clamped to the end of `text`.
pub fn byte_at16(text: &str, index: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        let next = units + ch.len_utf16();
        if next > index {
            return byte;
        }
        units = next;
    }
    text.len()
}

/// UTF-16 index of byte offset `byte` (which must be a char boundary).
pub fn index16_of_byte(text: &str, byte: usize) -> usize {
    len16(&text[..byte])
}

/// `text.slice(start, end)` with non-negative bounds.
pub fn slice16(text: &str, start: usize, end: usize) -> &str {
    let from = byte_at16(text, start);
    let to = byte_at16(text, end).max(from);
    &text[from..to]
}

/// `text.slice(0, end)`.
pub fn prefix16(text: &str, end: usize) -> &str {
    &text[..byte_at16(text, end)]
}

/// `text.slice(start)`.
pub fn suffix_from16(text: &str, start: usize) -> &str {
    &text[byte_at16(text, start)..]
}

/// `text.slice(-count)` for a positive `count`; the whole text when shorter.
pub fn last16(text: &str, count: usize) -> &str {
    let len = len16(text);
    if count >= len {
        text
    } else {
        suffix_from16(text, len - count)
    }
}

/// `text.lastIndexOf(needle, position)` in UTF-16 units.
pub fn last_index_of16(text: &str, needle: char, position: usize) -> Option<usize> {
    let mut units = 0;
    let mut found = None;
    for ch in text.chars() {
        if units > position {
            break;
        }
        if ch == needle {
            found = Some(units);
        }
        units += ch.len_utf16();
    }
    found
}

/// `text.replace(/\s+/g, " ").trim()`.
pub fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Clip to `limit` units after collapsing whitespace, ending with `…` when cut.
/// Shared by the live-state and subagent digests.
pub fn clip_collapsed(text: &str, limit: usize) -> String {
    let collapsed = collapse_whitespace(text);
    if len16(&collapsed) > limit {
        format!("{}…", prefix16(&collapsed, limit.saturating_sub(1)))
    } else {
        collapsed
    }
}

/// JavaScript `Number.prototype.toString()` for a finite double.
pub fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if value == 0.0 {
        return "0".into();
    }
    if value.fract() == 0.0 && value.abs() < 1e21 {
        return format!("{}", value as i128);
    }
    let text = format!("{value}");
    if text.contains('e') {
        return text.replace("e", "e+").replace("e+-", "e-");
    }
    text
}

/// JavaScript `Number(raw)` for a string: trimmed, empty is 0, otherwise a
/// decimal/exponent literal, `Infinity`, or hex/octal/binary integer.
pub fn js_number(raw: &str) -> f64 {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    match trimmed {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = trimmed.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix)
                .map(|value| value as f64)
                .unwrap_or(f64::NAN);
        }
    }
    let valid = trimmed
        .chars()
        .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | 'e' | 'E' | '+' | '-'));
    if !valid || trimmed.ends_with('e') || trimmed.ends_with('E') {
        return f64::NAN;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

/// `new Date(ms).toISOString()` for in-range years 0000..=9999.
pub fn iso_from_ms(ms: i64) -> String {
    let total_seconds = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = total_seconds.div_euclid(86_400);
    let secs_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// `Date.parse` for the ISO forms ICE writes: `YYYY-MM-DD`, and
/// `YYYY-MM-DDTHH:MM[:SS[.fff]]` with `Z` or a `±HH:MM` offset. A date-time
/// without a zone is read as UTC (JavaScript would use local time). Digits
/// are ASCII only, like JavaScript's `\d`; the byte slices below rely on it.
pub fn parse_iso_ms(text: &str) -> Option<i64> {
    let captures = crate::blackhole::regex!(
        r"^([0-9]{4})-([0-9]{2})-([0-9]{2})(?:[T ]([0-9]{2}):([0-9]{2})(?::([0-9]{2})(?:\.([0-9]{1,9}))?)?(Z|[+-][0-9]{2}:[0-9]{2})?)?$"
    )
    .captures(text.trim())?;
    let number = |index: usize| -> Option<i64> {
        captures
            .get(index)
            .map_or(Some(0), |m| m.as_str().parse().ok())
    };
    let (year, month, day) = (number(1)?, number(2)? as u32, number(3)? as u32);
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    let (hour, minute, second) = (number(4)?, number(5)?, number(6)?);
    if hour > 24 || minute > 59 || second > 59 {
        return None;
    }
    let millis = captures.get(7).map_or(0, |m| {
        let digits = format!("{:0<3}", m.as_str());
        digits[..3].parse::<i64>().unwrap_or(0)
    });
    if hour == 24 && (minute != 0 || second != 0 || millis != 0) {
        return None;
    }
    let offset_minutes = match captures.get(8).map(|m| m.as_str()) {
        None | Some("Z") => 0,
        Some(zone) => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let hours: i64 = zone[1..3].parse().ok()?;
            let minutes: i64 = zone[4..6].parse().ok()?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            sign * (hours * 60 + minutes)
        }
    };
    let days = days_from_civil(year, month, day);
    Some(
        ((days * 86_400 + hour * 3600 + minute * 60 + second) - offset_minutes * 60) * 1000
            + millis,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_slicing_counts_astral_characters_as_two_units() {
        let text = "a😀b";
        assert_eq!(len16(text), 4);
        assert_eq!(prefix16(text, 1), "a");
        // A cut inside the surrogate pair rounds back to the character start.
        assert_eq!(prefix16(text, 2), "a");
        assert_eq!(prefix16(text, 3), "a😀");
        assert_eq!(last16(text, 1), "b");
    }

    #[test]
    fn last_index_of_matches_javascript_position_semantics() {
        assert_eq!(last_index_of16("a b c", ' ', 3), Some(3));
        assert_eq!(last_index_of16("a b c", ' ', 2), Some(1));
        assert_eq!(last_index_of16("abc", ' ', 2), None);
    }

    #[test]
    fn js_number_follows_number_coercion() {
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number(" 12.5 "), 12.5);
        assert_eq!(js_number("0x10"), 16.0);
        assert!(js_number("12abc").is_nan());
        assert!(js_number("Infinity").is_infinite());
    }

    #[test]
    fn iso_round_trips_and_accepts_offsets() {
        let ms = 1_761_393_600_123;
        let iso = iso_from_ms(ms);
        assert_eq!(iso, "2025-10-25T12:00:00.123Z");
        assert_eq!(parse_iso_ms(&iso), Some(ms));
        assert_eq!(parse_iso_ms("2025-10-25T14:00:00.123+02:00"), Some(ms));
        assert_eq!(parse_iso_ms("2025-10-25"), Some(1_761_350_400_000));
        assert_eq!(parse_iso_ms("not a date"), None);
        // Date.parse normalizes calendar overflow up to the accepted day 31.
        assert_eq!(parse_iso_ms("2024-02-31"), parse_iso_ms("2024-03-02"));
        assert_eq!(
            parse_iso_ms("2024-04-30T24:00:00Z"),
            parse_iso_ms("2024-05-01T00:00:00Z")
        );
        assert_eq!(parse_iso_ms("2024-04-30T24:01:00Z"), None);
        assert_eq!(parse_iso_ms("2024-04-30T00:00:00+23:60"), None);
        assert_eq!(parse_iso_ms("2024-04-30T00:00:00+24:00"), None);
        assert_eq!(parse_iso_ms("2024-04-30T00:00:00+99:99"), None);
        // Non-ASCII digits are not dates (and must not panic the slicing).
        assert_eq!(parse_iso_ms("2024-04-30T00:00:00.\u{6F5}\u{6F5}"), None);
        assert_eq!(parse_iso_ms("2024-04-30T00:00:00+\u{6F5}\u{6F5}:00"), None);
    }

    #[test]
    fn numbers_render_like_javascript() {
        assert_eq!(number_to_string(85.0), "85");
        assert_eq!(number_to_string(12.5), "12.5");
        assert_eq!(number_to_string(-3.0), "-3");
    }
}
