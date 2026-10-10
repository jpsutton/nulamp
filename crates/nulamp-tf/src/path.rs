//! Path helpers. Both `/` and `\` separate components.

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Byte offsets of every separator.
fn seps(p: &str) -> Vec<usize> {
    p.match_indices(is_sep).map(|(i, _)| i).collect()
}

/// Last component.
fn last(p: &str) -> &str {
    p.rsplit(is_sep).next().unwrap_or(p)
}

/// Splits a file name at its last `.`. A leading dot does not start an extension.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// File name without folders and extension.
pub(crate) fn filepart(p: &str) -> &str {
    split_ext(last(p)).0
}

/// Extension without the dot; empty when there is none.
pub(crate) fn fileext(p: &str) -> &str {
    split_ext(last(p)).1
}

/// Name of the `n`-th folder above the file (1 = parent). Empty when out of range or `n < 1`.
pub(crate) fn directory(p: &str, n: i32) -> &str {
    let Ok(n) = usize::try_from(n) else { return "" };
    if n == 0 {
        return "";
    }
    let comps: Vec<&str> = p.split(is_sep).collect();
    comps.len().checked_sub(n + 1).map_or("", |i| comps[i])
}

/// Drops the first `n` components. `n ≤ 0` keeps everything.
pub(crate) fn ltrim(p: &str, n: i32) -> &str {
    match nth(n) {
        None => p,
        Some(k) => seps(p).get(k).map_or("", |&i| &p[i + 1..]),
    }
}

/// Keeps the first `n` components. `n ≤ 0` keeps nothing.
pub(crate) fn lpart(p: &str, n: i32) -> &str {
    match nth(n) {
        None => "",
        Some(k) => seps(p).get(k).map_or(p, |&i| &p[..i]),
    }
}

/// Drops the last `n` components. `n ≤ 0` keeps everything.
pub(crate) fn rtrim(p: &str, n: i32) -> &str {
    match nth(n) {
        None => p,
        Some(k) => from_right(p, k).map_or("", |i| &p[..i]),
    }
}

/// Keeps the last `n` components. `n ≤ 0` keeps nothing.
pub(crate) fn rpart(p: &str, n: i32) -> &str {
    match nth(n) {
        None => "",
        Some(k) => from_right(p, k).map_or(p, |i| &p[i + 1..]),
    }
}

/// Zero-based separator index for a component count, or `None` when `n ≤ 0`.
fn nth(n: i32) -> Option<usize> {
    usize::try_from(n).ok()?.checked_sub(1)
}

/// Byte offset of the `k`-th separator counted from the right (0 = last).
fn from_right(p: &str, k: usize) -> Option<usize> {
    let s = seps(p);
    s.len().checked_sub(k + 1).map(|i| s[i])
}

/// Makes `s` usable as a file name.
pub(crate) fn safe(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '?' | '*' | '|' => '_',
            '/' | '\\' | ':' => '-',
            '"' => '\'',
            '<' => '(',
            '>' => ')',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "C:\\a\\b\\c.mp3";

    #[test]
    fn spec_examples() {
        assert_eq!(filepart(P), "c");
        assert_eq!(fileext(P), "mp3");
        assert_eq!(directory(P, 1), "b");
        assert_eq!(directory(P, 2), "a");
        assert_eq!(ltrim(P, 1), "a\\b\\c.mp3");
        assert_eq!(lpart(P, 2), "C:\\a");
        assert_eq!(rtrim(P, 1), "C:\\a\\b");
        assert_eq!(rpart(P, 1), "c.mp3");
        assert_eq!(rpart(P, 2), "b\\c.mp3");
    }

    #[test]
    fn edges() {
        assert_eq!(directory(P, 0), "");
        assert_eq!(directory(P, 3), "C:");
        assert_eq!(directory(P, 4), "");
        assert_eq!(directory("x.mp3", 1), "");
        assert_eq!(ltrim(P, 0), P);
        assert_eq!(ltrim(P, 9), "");
        assert_eq!(lpart(P, 0), "");
        assert_eq!(lpart(P, 9), P);
        assert_eq!(rtrim(P, -1), P);
        assert_eq!(rtrim(P, 9), "");
        assert_eq!(rpart(P, 0), "");
        assert_eq!(rpart(P, 9), P);
        assert_eq!(filepart(".hidden"), ".hidden");
        assert_eq!(fileext("a/b.c/noext"), "");
        assert_eq!(filepart("/x/a.b.c"), "a.b");
    }
}
