//! String and number helpers. Lengths and positions count Unicode code points.

/// Longest string (in bytes) a value may reach.
pub(crate) const MAX_LEN: usize = 1 << 16;

/// Appends `s`, cutting at a character boundary so `out` stays within [`MAX_LEN`].
pub(crate) fn push_capped(out: &mut String, s: &str) {
    let room = MAX_LEN.saturating_sub(out.len());
    if s.len() <= room {
        out.push_str(s);
    } else {
        let mut end = room;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        out.push_str(&s[..end]);
    }
}

pub(crate) fn len(s: &str) -> usize {
    s.chars().count()
}

/// Leading integer: optional whitespace, optional sign, digits. Saturates at the
/// 32-bit range; anything else is 0.
pub(crate) fn int(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i64 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        v = (v * 10 + i64::from(b - b'0')).min(1 << 31);
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Count argument as `usize`; negative counts are 0.
pub(crate) fn count(n: i32) -> usize {
    usize::try_from(n).unwrap_or(0)
}

/// Case- and width-insensitive equality. An empty string equals nothing.
pub(crate) fn str_eq(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && fold(a) == fold(b)
}

/// Maps full-width ASCII (U+FF01–U+FF5E) to ASCII, then lower-cases.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            c => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// First `n` characters; negative `n` keeps the whole string.
pub(crate) fn left(s: &str, n: i32) -> String {
    match usize::try_from(n) {
        Ok(n) => s.chars().take(n).collect(),
        Err(_) => s.to_string(),
    }
}

/// Last `n` characters; the whole string when `n` is negative or at least the length.
pub(crate) fn right(s: &str, n: i32) -> String {
    let total = len(s);
    match usize::try_from(n) {
        Ok(n) if n < total => s.chars().skip(total - n).collect(),
        _ => s.to_string(),
    }
}

/// Characters `a..=b` (1-based). `a` below 1 counts as 1; `b` past the end stops there.
pub(crate) fn substr(s: &str, a: i32, b: i32) -> String {
    let a = a.max(1);
    if b < a {
        return String::new();
    }
    let skip = count(a - 1);
    let take = usize::try_from(i64::from(b) - i64::from(a) + 1).unwrap_or(0);
    s.chars().skip(skip).take(take).collect()
}

/// `s` padded to `n` characters with `fill` repeated cyclically (a space when empty).
pub(crate) fn pad(s: &str, n: i32, fill: &str, at_left: bool) -> String {
    let have = len(s);
    let want = count(n).min(MAX_LEN);
    if have >= want {
        return s.to_string();
    }
    let fill = if fill.is_empty() { " " } else { fill };
    let filler: String = fill.chars().cycle().take(want - have).collect();
    let mut out = String::new();
    if at_left {
        push_capped(&mut out, &filler);
        push_capped(&mut out, s);
    } else {
        push_capped(&mut out, s);
        push_capped(&mut out, &filler);
    }
    out
}

/// `s` repeated `n` times, within [`MAX_LEN`].
pub(crate) fn repeat(s: &str, n: i32) -> String {
    let mut out = String::new();
    if s.is_empty() {
        return out;
    }
    for _ in 0..count(n) {
        if out.len() >= MAX_LEN {
            break;
        }
        push_capped(&mut out, s);
    }
    out
}

/// Replaces every non-overlapping `find` (case-sensitive), within [`MAX_LEN`].
pub(crate) fn replace(s: &str, find: &str, with: &str) -> String {
    if find.is_empty() {
        return s.to_string();
    }
    let mut out = String::new();
    let mut last = 0;
    for (i, _) in s.match_indices(find) {
        if out.len() >= MAX_LEN {
            return out;
        }
        push_capped(&mut out, &s[last..i]);
        push_capped(&mut out, with);
        last = i + find.len();
    }
    push_capped(&mut out, &s[last..]);
    out
}

/// Upper-cases the first character of every word. A word starts at the beginning or
/// after any character that is not alphanumeric (whitespace or punctuation).
pub(crate) fn caps(s: &str, lower_first: bool) -> String {
    let base = if lower_first {
        s.to_lowercase()
    } else {
        s.to_string()
    };
    let mut out = String::with_capacity(base.len());
    let mut start = true;
    for c in base.chars() {
        if start {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        start = !c.is_alphanumeric();
    }
    out
}

/// Abbreviation: the first alphanumeric character of each whitespace-separated word.
///
/// Roman numerals (only `IVXLCDM`, upper-case), all-digit words and `RPG` are kept whole,
/// except in the last word (reference behaviour).
pub(crate) fn abbr(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        let whole = i + 1 < words.len()
            && (w.chars().all(|c| "IVXLCDM".contains(c))
                || w.chars().all(|c| c.is_ascii_digit())
                || *w == "RPG");
        if whole {
            out.push_str(w);
        } else if let Some(c) = w.chars().find(|c| c.is_alphanumeric()) {
            out.push(c);
        }
    }
    out
}

/// `n` rounded to nearest after dividing by `d` (ties away from zero). `d` must not be 0.
pub(crate) fn round_div(n: i64, d: i64) -> i64 {
    let (n, d) = (i128::from(n), i128::from(d));
    let q = (n.abs() + d.abs() / 2) / d.abs();
    let q = if (n < 0) != (d < 0) { -q } else { q };
    q.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_parsing() {
        assert_eq!(int("  42abc"), 42);
        assert_eq!(int("-7"), -7);
        assert_eq!(int("+3/12"), 3);
        assert_eq!(int("x1"), 0);
        assert_eq!(int(""), 0);
        assert_eq!(int("-"), 0);
        assert_eq!(int("99999999999999999999"), i32::MAX);
        assert_eq!(int("-99999999999999999999"), i32::MIN);
        assert_eq!(int("-2147483648"), i32::MIN);
    }

    #[test]
    fn comparison() {
        assert!(str_eq("Daft Punk", "daft PUNK"));
        assert!(str_eq("ＡＢＣ１", "abc1"));
        assert!(!str_eq("", ""));
        assert!(!str_eq("a", ""));
    }

    #[test]
    fn cap_respects_char_boundaries() {
        let mut s = "a".repeat(MAX_LEN - 1);
        push_capped(&mut s, "é");
        assert_eq!(s.len(), MAX_LEN - 1);
        assert_eq!(repeat("ab", i32::MAX).len(), MAX_LEN);
        assert_eq!(
            replace(&"a".repeat(1000), "a", &"b".repeat(1000)).len(),
            MAX_LEN
        );
    }

    #[test]
    fn rounding() {
        assert_eq!(round_div(7, 2), 4);
        assert_eq!(round_div(-7, 2), -4);
        assert_eq!(round_div(7, -2), -4);
        assert_eq!(round_div(5, 4), 1);
        assert_eq!(round_div(8, 3), 3);
    }
}
