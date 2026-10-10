//! Original proportional bitmap (pixel) UI font, Regular and Bold, with a 1-bit renderer.
//!
//! The glyphs were designed for nulamp and are not derived from any other font
//! (docs/decisions.md PF1). Every pixel is on or off; there are no grey levels (D2).
//!
//! Glyph data lives in `glyphs/regular.txt` and `glyphs/bold.txt`, a small text format that
//! is easy to edit by hand (see [`PixelFont::parse`]). Both faces share one line box:
//! ascent 11, descent 2, line height 13, cap height 8, x-height 6.
//!
//! Characters the font lacks are reported by [`PixelFont::has_char`], so callers can fall
//! back to the FreeType renderer per character; [`PixelFont::draw`] draws U+FFFD for them.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use nulamp_skin::Bitmap;

/// Replacement glyph drawn for characters the font lacks.
pub const REPLACEMENT: char = '\u{FFFD}';

/// Widest glyph the format accepts, in pixels.
const MAX_WIDTH: usize = 32;

/// One glyph: a 1-bit image as tall as the line box, plus its advance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    advance: i32,
    width: u32,
    /// One mask per row from the top of the line box; bit `x` is column `x`.
    rows: Vec<u32>,
}

impl Glyph {
    /// Pen advance in pixels, including the spacing column.
    pub fn advance(&self) -> i32 {
        self.advance
    }

    /// Width of the image in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of the image in pixels (the line height).
    pub fn height(&self) -> u32 {
        self.rows.len() as u32
    }

    /// Whether the pixel at (x, y) is on; y counts from the top of the line box.
    pub fn pixel(&self, x: u32, y: u32) -> bool {
        x < self.width
            && self
                .rows
                .get(y as usize)
                .is_some_and(|row| row >> x & 1 == 1)
    }
}

/// Error from [`PixelFont::parse`], with a 1-based line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// A parsed bitmap font face.
#[derive(Debug, Clone)]
pub struct PixelFont {
    ascent: i32,
    descent: i32,
    glyphs: BTreeMap<char, Glyph>,
}

impl PixelFont {
    /// The built-in Regular face (parsed on first use).
    pub fn regular() -> &'static PixelFont {
        static FACE: OnceLock<PixelFont> = OnceLock::new();
        FACE.get_or_init(|| {
            Self::parse(include_str!("../glyphs/regular.txt")).expect("built-in Regular face")
        })
    }

    /// The built-in Bold face (parsed on first use). Same metrics as Regular.
    pub fn bold() -> &'static PixelFont {
        static FACE: OnceLock<PixelFont> = OnceLock::new();
        FACE.get_or_init(|| {
            Self::parse(include_str!("../glyphs/bold.txt")).expect("built-in Bold face")
        })
    }

    /// Parses a face from its text form.
    ///
    /// Lines starting with `;` are comments; blank lines are ignored. The header gives
    /// `ascent N` and `descent N`. Each glyph is a line `U+XXXX <advance> [comment]`
    /// followed by exactly ascent + descent rows of `.` (off) and `#` (on), all the same
    /// width. The first row is the top of the line box. The face must contain U+FFFD.
    pub fn parse(src: &str) -> Result<Self, ParseError> {
        let err = |line: usize, message: String| ParseError { line, message };
        let mut ascent = None;
        let mut descent = None;
        let mut glyphs = BTreeMap::new();
        // Glyph being read: (header line, char, advance, rows, width).
        let mut cur: Option<(usize, char, i32, Vec<u32>, usize)> = None;

        let finish = |cur: Option<(usize, char, i32, Vec<u32>, usize)>,
                      height: usize,
                      glyphs: &mut BTreeMap<char, Glyph>|
         -> Result<(), ParseError> {
            let Some((line, c, advance, rows, width)) = cur else {
                return Ok(());
            };
            if rows.len() != height {
                return Err(err(
                    line,
                    format!(
                        "U+{:04X} has {} rows, expected {height}",
                        c as u32,
                        rows.len()
                    ),
                ));
            }
            if width as i32 > advance {
                return Err(err(
                    line,
                    format!("U+{:04X} is wider than its advance", c as u32),
                ));
            }
            if glyphs
                .insert(
                    c,
                    Glyph {
                        advance,
                        width: width as u32,
                        rows,
                    },
                )
                .is_some()
            {
                return Err(err(line, format!("U+{:04X} defined twice", c as u32)));
            }
            Ok(())
        };

        for (i, raw) in src.lines().enumerate() {
            let n = i + 1;
            let line = raw.trim_end();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if !line.is_empty() && line.chars().all(|c| c == '.' || c == '#') {
                let Some((_, _, _, rows, width)) = cur.as_mut() else {
                    return Err(err(n, "pixel row outside a glyph".into()));
                };
                if line.len() > MAX_WIDTH {
                    return Err(err(n, format!("row wider than {MAX_WIDTH} pixels")));
                }
                if rows.is_empty() {
                    *width = line.len();
                } else if line.len() != *width {
                    return Err(err(
                        n,
                        "row width differs from the glyph's first row".into(),
                    ));
                }
                let mask = line
                    .bytes()
                    .enumerate()
                    .fold(0u32, |m, (x, b)| m | u32::from(b == b'#') << x);
                rows.push(mask);
                continue;
            }
            let mut words = line.split_whitespace();
            let key = words.next().unwrap_or_default();
            let value = words.next().ok_or_else(|| err(n, "missing value".into()))?;
            if let Some(hex) = key.strip_prefix("U+") {
                let height = match (ascent, descent) {
                    (Some(a), Some(d)) => (a + d) as usize,
                    _ => return Err(err(n, "glyph before ascent and descent".into())),
                };
                finish(cur.take(), height, &mut glyphs)?;
                let c = u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| err(n, format!("bad code point {key}")))?;
                let advance: i32 = value
                    .parse()
                    .ok()
                    .filter(|a| *a > 0)
                    .ok_or_else(|| err(n, format!("bad advance {value}")))?;
                cur = Some((n, c, advance, Vec::new(), 0));
                continue;
            }
            let number: i32 = value
                .parse()
                .ok()
                .filter(|v| *v >= 0)
                .ok_or_else(|| err(n, format!("bad number {value}")))?;
            match key {
                "ascent" if glyphs.is_empty() && cur.is_none() => ascent = Some(number),
                "descent" if glyphs.is_empty() && cur.is_none() => descent = Some(number),
                _ => return Err(err(n, format!("unexpected line {line:?}"))),
            }
        }
        let (Some(ascent), Some(descent)) = (ascent, descent) else {
            return Err(err(0, "missing ascent or descent".into()));
        };
        finish(cur, (ascent + descent) as usize, &mut glyphs)?;
        if !glyphs.contains_key(&REPLACEMENT) {
            return Err(err(0, "missing U+FFFD".into()));
        }
        Ok(Self {
            ascent,
            descent,
            glyphs,
        })
    }

    /// Pixels from the top of the line box to the baseline.
    pub fn ascent(&self) -> i32 {
        self.ascent
    }

    /// Pixels from the baseline to the bottom of the line box.
    pub fn descent(&self) -> i32 {
        self.descent
    }

    /// Height of one line of text.
    pub fn line_height(&self) -> i32 {
        self.ascent + self.descent
    }

    /// Whether the font has its own glyph for `c`.
    pub fn has_char(&self, c: char) -> bool {
        self.glyphs.contains_key(&c)
    }

    /// The glyph for `c`, or `None` when the font lacks it.
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c)
    }

    /// All characters the font covers, in code point order.
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.glyphs.keys().copied()
    }

    fn glyph_or_replacement(&self, c: char) -> &Glyph {
        self.glyphs
            .get(&c)
            .unwrap_or_else(|| &self.glyphs[&REPLACEMENT])
    }

    /// Pen advance of one character (U+FFFD's for unknown characters).
    pub fn advance(&self, c: char) -> i32 {
        self.glyph_or_replacement(c).advance
    }

    /// Width in pixels of a line of text at scale 1.
    pub fn measure(&self, text: &str) -> i32 {
        text.chars().map(|c| self.advance(c)).sum()
    }

    /// Draws `text` with its pen origin at (x, baseline_y) and returns the advance.
    /// Pixels outside `clip` (x0, y0, x1, y1; exclusive) are not touched.
    pub fn draw(
        &self,
        target: &mut Bitmap,
        text: &str,
        x: i32,
        baseline_y: i32,
        color: u32,
        clip: (i32, i32, i32, i32),
    ) -> i32 {
        self.draw_scaled(target, text, x, baseline_y, color, clip, 1)
    }

    /// Like [`draw`](Self::draw), with every font pixel drawn as a `scale` × `scale` block.
    /// The returned advance is `measure(text) * scale`.
    pub fn draw_scaled(
        &self,
        target: &mut Bitmap,
        text: &str,
        x: i32,
        baseline_y: i32,
        color: u32,
        clip: (i32, i32, i32, i32),
        scale: i32,
    ) -> i32 {
        let s = scale.max(1);
        let x0 = clip.0.max(0);
        let y0 = clip.1.max(0);
        let x1 = clip.2.min(target.width() as i32);
        let y1 = clip.3.min(target.height() as i32);
        let top = baseline_y - self.ascent * s;
        let mut pen = x;
        for c in text.chars() {
            let g = self.glyph_or_replacement(c);
            if pen < x1 && pen + g.width as i32 * s > x0 {
                for (row, mask) in g.rows.iter().enumerate() {
                    let py = top + row as i32 * s;
                    if *mask == 0 || py >= y1 || py + s <= y0 {
                        continue;
                    }
                    for col in 0..g.width as i32 {
                        if mask >> col & 1 == 0 {
                            continue;
                        }
                        let px = pen + col * s;
                        for yy in py.max(y0)..(py + s).min(y1) {
                            for xx in px.max(x0)..(px + s).min(x1) {
                                target.set(xx, yy, color);
                            }
                        }
                    }
                }
            }
            pen += g.advance * s;
        }
        pen - x
    }

    /// Returns `text` unchanged when it fits `max_width`, otherwise the longest prefix
    /// (trailing spaces removed) followed by `…` that fits. Returns an empty string when
    /// not even `…` fits.
    pub fn fit(&self, text: &str, max_width: i32) -> String {
        if self.measure(text) <= max_width {
            return text.to_string();
        }
        let budget = max_width - self.advance('…');
        if budget < 0 {
            return String::new();
        }
        let mut width = 0;
        let mut end = 0;
        for (i, c) in text.char_indices() {
            width += self.advance(c);
            if width > budget {
                break;
            }
            end = i + c.len_utf8();
        }
        let mut out = text[..end].trim_end().to_string();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn faces() -> [(&'static str, &'static PixelFont); 2] {
        [
            ("regular", PixelFont::regular()),
            ("bold", PixelFont::bold()),
        ]
    }

    fn required() -> Vec<char> {
        let mut v: Vec<char> = (0x20..=0x7E)
            .chain(0xA0..=0xFF)
            .filter_map(char::from_u32)
            .collect();
        v.extend([
            '\u{2013}', '\u{2014}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}',
            '\u{2026}', '\u{20AC}', '\u{2122}', '\u{2190}', '\u{2191}', '\u{2192}', '\u{2193}',
            '\u{2713}', '\u{25B6}', '\u{2605}', '\u{2606}', '\u{FFFD}',
        ]);
        v
    }

    #[test]
    fn covers_required_characters() {
        for (name, f) in faces() {
            let missing: Vec<String> = required()
                .into_iter()
                .filter(|c| !f.has_char(*c))
                .map(|c| format!("U+{:04X}", c as u32))
                .collect();
            assert!(missing.is_empty(), "{name} lacks {missing:?}");
        }
    }

    #[test]
    fn glyphs_fit_the_line_box() {
        for (name, f) in faces() {
            assert_eq!(f.ascent(), 11, "{name}");
            assert_eq!(f.line_height(), 13, "{name}");
            for c in f.chars() {
                let g = f.glyph(c).unwrap();
                assert_eq!(g.height(), 13, "{name} U+{:04X}", c as u32);
                assert!(g.width() as i32 <= g.advance(), "{name} U+{:04X}", c as u32);
                assert!(g.rows.iter().all(|r| r >> g.width() == 0));
                let inked = g.rows.iter().any(|r| *r != 0);
                assert_eq!(inked, !c.is_whitespace(), "{name} U+{:04X}", c as u32);
            }
        }
    }

    #[test]
    fn metrics_and_shapes() {
        for (name, f) in faces() {
            let g = |c| f.glyph(c).unwrap();
            let top = |c| (0..13).find(|&y| (0..g(c).width()).any(|x| g(c).pixel(x, y)));
            let bottom = |c| {
                (0..13)
                    .rev()
                    .find(|&y| (0..g(c).width()).any(|x| g(c).pixel(x, y)))
            };
            // Cap height 8 and x-height 6 above the baseline (row 10 is the last row above it).
            assert_eq!((top('H'), bottom('H')), (Some(3), Some(10)), "{name}");
            assert_eq!((top('x'), bottom('x')), (Some(5), Some(10)), "{name}");
            for c in ['g', 'j', 'p', 'q', 'y'] {
                assert_eq!(bottom(c), Some(12), "{name} {c} descender");
            }
            for c in ['À', 'É', 'Î', 'Õ', 'Ü', 'Å'] {
                assert!(top(c).unwrap() < 3, "{name} {c} accent");
            }
            let digit = f.advance('0');
            assert!(
                ('0'..='9').all(|c| f.advance(c) == digit),
                "{name} tabular digits"
            );
            assert!(f.advance('i') < f.advance('n') && f.advance('n') < f.advance('m'));
        }
    }

    #[test]
    fn faces_differ_but_share_metrics() {
        let (r, b) = (PixelFont::regular(), PixelFont::bold());
        assert_eq!(r.ascent(), b.ascent());
        assert_eq!(r.descent(), b.descent());
        assert_ne!(r.glyph('a'), b.glyph('a'));
    }

    #[test]
    fn draws_one_colour_and_returns_advance() {
        let f = PixelFont::regular();
        let mut bmp = Bitmap::new(200, 20, 0);
        let text = "Björk – Jóga ★";
        let adv = f.draw(&mut bmp, text, 2, 12, 0x00FF00, (0, 0, 200, 20));
        assert_eq!(adv, f.measure(text));
        assert!(bmp.pixels().iter().all(|&p| p == 0 || p == 0x00FF00));
        assert!(bmp.pixels().contains(&0x00FF00));
    }

    #[test]
    fn clip_is_respected() {
        let f = PixelFont::bold();
        let mut bmp = Bitmap::new(40, 20, 0);
        f.draw(&mut bmp, "WWWWWW", 0, 12, 0xFFFFFF, (5, 4, 20, 9));
        for y in 0..20 {
            for x in 0..40 {
                if bmp.pixel(x, y) != 0 {
                    assert!((5..20).contains(&x) && (4..9).contains(&y));
                }
            }
        }
    }

    #[test]
    fn scaled_drawing_doubles_pixels() {
        let f = PixelFont::regular();
        let mut one = Bitmap::new(30, 13, 0);
        let mut two = Bitmap::new(60, 26, 0);
        let a1 = f.draw(&mut one, "Ag", 0, 11, 1, (0, 0, 30, 13));
        let a2 = f.draw_scaled(&mut two, "Ag", 0, 22, 1, (0, 0, 60, 26), 2);
        assert_eq!(a2, a1 * 2);
        for y in 0..26 {
            for x in 0..60 {
                assert_eq!(two.pixel(x, y), one.pixel(x / 2, y / 2));
            }
        }
    }

    #[test]
    fn unknown_characters_draw_replacement() {
        let f = PixelFont::regular();
        assert!(!f.has_char('漢'));
        assert_eq!(f.advance('漢'), f.advance(REPLACEMENT));
        let mut a = Bitmap::new(20, 13, 0);
        let mut b = Bitmap::new(20, 13, 0);
        f.draw(&mut a, "漢", 0, 11, 1, (0, 0, 20, 13));
        f.draw(&mut b, "\u{FFFD}", 0, 11, 1, (0, 0, 20, 13));
        assert_eq!(a, b);
    }

    #[test]
    fn fit_truncates_with_ellipsis() {
        let f = PixelFont::regular();
        let text = "A very long playlist title that does not fit";
        assert_eq!(f.fit("Hi", 100), "Hi");
        let full = f.measure(text);
        let cut = f.fit(text, full / 2);
        assert!(cut.ends_with('…') && f.measure(&cut) <= full / 2, "{cut}");
        assert!(!cut.trim_end_matches('…').ends_with(' '));
        assert_eq!(f.fit(text, f.advance('…')), "…");
        assert_eq!(f.fit(text, 1), "");
    }

    #[test]
    fn parse_errors() {
        let head = "ascent 2\ndescent 1\n";
        let ok = format!("{head}U+FFFD 3\n##\n#.\n##\n");
        assert!(PixelFont::parse(&ok).is_ok());
        let short = format!("{head}U+FFFD 3\n##\n#.\n");
        assert!(PixelFont::parse(&short).is_err());
        let ragged = format!("{head}U+FFFD 3\n##\n#\n##\n");
        assert_eq!(PixelFont::parse(&ragged).unwrap_err().line, 5);
        let wide = format!("{head}U+FFFD 1\n##\n#.\n##\n");
        assert!(PixelFont::parse(&wide).is_err());
        let no_fffd = format!("{head}U+0041 3\n##\n#.\n##\n");
        assert!(PixelFont::parse(&no_fffd).is_err());
    }
}
