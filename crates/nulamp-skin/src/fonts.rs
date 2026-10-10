//! Bitmap fonts and colours derived from skin bitmaps:
//! `text.bmp` glyph cells and title colours (01 §6.1–6.2), `gen.bmp` glyph metrics (05 §1.3)
//! and `genex.bmp` colours (05 §1.5).

use crate::bitmap::Bitmap;
use unicode_normalization::UnicodeNormalization;

/// Column and row of a 5×6 cell in `text.bmp`.
pub type Cell = (u8, u8);

/// The space cell; also the fallback for every unmapped character.
pub const SPACE_CELL: Cell = (30, 0);

fn direct_cell(c: char) -> Option<Cell> {
    Some(match c {
        'A'..='Z' => (c as u8 - b'A', 0),
        'a'..='z' => (c as u8 - b'a', 0),
        '"' => (26, 0),
        '@' => (27, 0),
        ' ' => (30, 0),
        '0'..='9' => (c as u8 - b'0', 1),
        '…' => (10, 1),
        '.' => (11, 1),
        ':' => (12, 1),
        '(' => (13, 1),
        ')' => (14, 1),
        '-' => (15, 1),
        '\'' | '`' => (16, 1),
        '!' => (17, 1),
        '_' => (18, 1),
        '+' => (19, 1),
        '\\' => (20, 1),
        '/' => (21, 1),
        '[' | '{' | '<' => (22, 1),
        ']' | '}' | '>' => (23, 1),
        '~' | '^' => (24, 1),
        '&' => (25, 1),
        '%' => (26, 1),
        ',' => (27, 1),
        '=' => (28, 1),
        '$' => (29, 1),
        '#' => (30, 1),
        'Å' | 'å' => (0, 2),
        'Ö' | 'ö' => (1, 2),
        'Ä' | 'ä' => (2, 2),
        '?' => (3, 2),
        '*' => (4, 2),
        _ => return None,
    })
}

fn fold(c: char) -> Option<char> {
    Some(match c {
        '°' => '0',
        'Ç' => 'C',
        'É' => 'E',
        'Æ' => 'A',
        'Ü' => 'U',
        'ü' | 'ú' | 'ù' | 'û' => 'u',
        'è' | 'ë' | 'ê' | 'é' => 'e',
        'á' | 'à' | 'â' | 'æ' => 'a',
        'ç' => 'c',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' => 'o',
        'ÿ' => 'y',
        'ƒ' => 'f',
        'Ñ' | 'ñ' => 'n',
        _ => return None,
    })
}

/// The `text.bmp` cell for a character, after accent folding.
pub fn text_cell(c: char) -> Cell {
    if let Some(cell) = direct_cell(c) {
        return cell;
    }
    if let Some(cell) = fold(c).and_then(direct_cell) {
        return cell;
    }
    // Other Latin accented letters: canonical decomposition, keep the base letter.
    if let Some(base) = c
        .nfd()
        .next()
        .filter(|b| *b != c && b.is_ascii_alphabetic())
    {
        return direct_cell(base).unwrap_or(SPACE_CELL);
    }
    SPACE_CELL
}

/// Colours for system-font text on the main window, from `text.bmp` (01 §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleColors {
    pub fg: u32,
    pub bg: u32,
}

pub fn title_colors(text: &Bitmap) -> TitleColors {
    let bg = text.pixel(150, 4);
    let dist = |p: u32| {
        let d = |s: u32| ((p >> s) & 0xFF) as i32 - ((bg >> s) & 0xFF) as i32;
        d(16) * d(16) + d(8) * d(8) + d(0) * d(0)
    };
    let mut fg = bg;
    let mut best = -1;
    for y in 0..6 {
        for x in 0..20 {
            let p = text.pixel(x, y);
            let dd = dist(p);
            if dd > best {
                best = dd;
                fg = p;
            }
        }
    }
    TitleColors { fg, bg }
}

/// Glyph position in `gen.bmp`: x and width. Height is 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GenGlyph {
    pub x: i32,
    pub width: i32,
}

/// Variable-width font metrics scanned from `gen.bmp` (05 §1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenFont {
    /// A–Z.
    pub letters: [GenGlyph; 26],
    /// 0–9, `-`, `:`.
    pub digits: [GenGlyph; 12],
}

impl GenFont {
    pub fn scan(gen_bmp: &Bitmap) -> Self {
        let mut letters = [GenGlyph::default(); 26];
        let mut digits = [GenGlyph::default(); 12];
        scan_row(gen_bmp, 90, &mut letters);
        scan_row(gen_bmp, 74, &mut digits);
        Self { letters, digits }
    }
}

fn scan_row(bmp: &Bitmap, y: i32, out: &mut [GenGlyph]) {
    let delim = bmp.pixel(0, y);
    let w = bmp.width() as i32;
    let mut x = 0;
    for g in out.iter_mut() {
        while x < w && bmp.pixel(x, y) == delim {
            x += 1;
        }
        let start = x;
        while x < w && bmp.pixel(x, y) != delim {
            x += 1;
        }
        *g = GenGlyph {
            x: start,
            width: x - start,
        };
    }
}

/// Colours for generic-window content from `genex.bmp` (05 §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenExColors {
    pub item_bg: u32,
    pub item_fg: u32,
    pub window_bg: u32,
    pub button_text: u32,
    pub window_text: u32,
    pub divider: u32,
    pub list_selection: u32,
    pub header_bg: u32,
    pub header_text: u32,
    pub header_frame_top: u32,
    pub header_frame_middle: u32,
    pub header_frame_bottom: u32,
    pub empty_header_bg: u32,
    pub scroll_fg: u32,
    pub scroll_bg: u32,
    pub scroll_fg_inverted: u32,
    pub scroll_bg_inverted: u32,
    pub scroll_dead: u32,
    pub selbar_text_active: u32,
    pub selbar_bg_active: u32,
    pub selbar_text_inactive: u32,
    pub selbar_bg_inactive: u32,
    pub alt_row_bg: u32,
    pub alt_row_fg: u32,
}

impl GenExColors {
    pub fn read(genex: &Bitmap) -> Self {
        let unset = genex.pixel(111, 0);
        let px = |i: i32| {
            let p = genex.pixel(48 + 2 * i, 0);
            (p != 0x00C6FF && p != unset).then_some(p)
        };
        let item_bg = px(0).unwrap_or(0x000000);
        let item_fg = px(1).unwrap_or(0x00FF00);
        let window_bg = px(2).unwrap_or(0x24243C);
        let window_text = px(4).unwrap_or(0xFFFFFF);
        let list_selection = px(6).unwrap_or(0x0000C6);
        let avg = |a: u32, b: u32| {
            let c = |s: u32| ((((a >> s) & 0xFF) + ((b >> s) & 0xFF)) / 2) << s;
            c(16) | c(8) | c(0)
        };
        Self {
            item_bg,
            item_fg,
            window_bg,
            button_text: px(3).unwrap_or(0x393842),
            window_text,
            divider: px(5).unwrap_or(0x8494A5),
            list_selection,
            header_bg: px(7).unwrap_or(0x484878),
            header_text: px(8).unwrap_or(0xFFFFFF),
            header_frame_top: px(9).unwrap_or(0x6C6CB4),
            header_frame_middle: px(10).unwrap_or(0x24243C),
            header_frame_bottom: px(11).unwrap_or(0x12121E),
            empty_header_bg: px(12).unwrap_or(0x24243C),
            scroll_fg: px(13).unwrap_or(0x24243C),
            scroll_bg: px(14).unwrap_or(0x24243C),
            scroll_fg_inverted: px(15).unwrap_or(0x798296),
            scroll_bg_inverted: px(16).unwrap_or(0x4E586E),
            scroll_dead: px(17).unwrap_or(0x24243C),
            selbar_text_active: px(18).unwrap_or(window_text),
            selbar_bg_active: px(19).unwrap_or(list_selection),
            selbar_text_inactive: px(20).unwrap_or(window_text),
            selbar_bg_inactive: px(21).unwrap_or(avg(list_selection, window_bg)),
            alt_row_bg: px(22).unwrap_or(item_bg),
            alt_row_fg: px(23).unwrap_or(item_fg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells() {
        assert_eq!(text_cell('A'), (0, 0));
        assert_eq!(text_cell('z'), (25, 0));
        assert_eq!(text_cell('7'), (7, 1));
        assert_eq!(text_cell('{'), (22, 1));
        assert_eq!(text_cell('ö'), (1, 2));
        assert_eq!(text_cell('é'), (4, 0));
        assert_eq!(text_cell('°'), (0, 1));
        assert_eq!(text_cell('Ł'), SPACE_CELL); // no decomposition
        assert_eq!(text_cell('Ś'), (18, 0)); // decomposes to S
        assert_eq!(text_cell('€'), SPACE_CELL);
    }

    #[test]
    fn title_colors_pick_farthest_first() {
        let mut b = Bitmap::new(155, 74, 0x101010);
        b.set(150, 4, 0x000000);
        b.set(3, 1, 0x00FF00);
        b.set(5, 2, 0x00FF00); // tie: first wins (same colour anyway)
        b.set(19, 5, 0x00FE00);
        let c = title_colors(&b);
        assert_eq!(
            c,
            TitleColors {
                fg: 0x00FF00,
                bg: 0x000000
            }
        );
    }

    #[test]
    fn gen_font_scan() {
        let mut b = Bitmap::new(194, 109, 0x123456);
        // Delimiter colour at (0, 90) = 0x123456; letters of width 3, gaps of 1.
        for i in 0..26 {
            b.fill_rect(1 + i * 4, 90, 3, 1, 0xFFFFFF);
        }
        let f = GenFont::scan(&b);
        assert_eq!(f.letters[0], GenGlyph { x: 1, width: 3 });
        assert_eq!(f.letters[25], GenGlyph { x: 101, width: 3 });
    }

    #[test]
    fn genex_fallbacks() {
        let mut b = Bitmap::new(130, 75, 0);
        b.set(111, 0, 0x777777);
        b.set(48, 0, 0x00C6FF); // not set
        b.set(50, 0, 0x777777); // equal to (111,0): not set
        b.set(52, 0, 0x010101);
        let c = GenExColors::read(&b);
        assert_eq!(c.item_bg, 0x000000);
        assert_eq!(c.item_fg, 0x00FF00);
        assert_eq!(c.window_bg, 0x010101);
        assert_eq!(c.button_text, 0x000000); // black pixel is a real colour
    }
}
