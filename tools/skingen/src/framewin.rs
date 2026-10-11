//! Playlist, generic-window, `genex.bmp` and video bitmaps of the built-in skin
//! (layouts from spec 04 and 05).

use crate::canvas::{Canvas, pal};
use crate::font;
use crate::mainwin::{Icon, icon};

/// A 20-row title tile: a 14-row bar like the main and equalizer windows' title bars, then
/// six rows of window face above the list.
fn title_piece(c: &mut Canvas, x: i32, y: i32, w: i32, active: bool) {
    let bg = if active {
        pal::TITLE_BG
    } else {
        pal::TITLE_BG_IN
    };
    c.rect(x, y, w, 14, bg);
    c.hline(x, y + 13, w, pal::TITLE_LINE);
    c.rect(x, y + 14, w, 6, pal::FACE);
}

fn grip(c: &mut Canvas, x: i32, y: i32, w: i32, active: bool) {
    let hi = if active {
        pal::GROOVE_HI
    } else {
        pal::GROOVE_HI_IN
    };
    c.grooves(x, y, w, hi);
}

/// A centred title in a title tile, with grooves filling the tile on both sides of it.
fn title_label(c: &mut Canvas, x: i32, y: i32, w: i32, label: &str, active: bool) {
    let ink = if active {
        pal::TITLE_TEXT
    } else {
        pal::TITLE_TEXT_IN
    };
    let tw = font::text_width(label);
    let tx = x + (w - tw) / 2;
    grip(c, x, y, tx - 5 - x, active);
    grip(c, tx + tw + 5, y, x + w - (tx + tw + 5), active);
    c.text(tx, y + 4, label, ink);
}

fn btn_x(c: &mut Canvas, x: i32, y: i32, pressed: bool) {
    c.button(x, y, 9, 9, pressed);
    let ink = if pressed {
        pal::LABEL_DIM
    } else {
        pal::TITLE_TEXT
    };
    for i in 0..5 {
        c.px(x + 2 + i, y + 2 + i, ink);
        c.px(x + 6 - i, y + 2 + i, ink);
    }
}

fn btn_shade(c: &mut Canvas, x: i32, y: i32, pressed: bool, unshade: bool) {
    c.button(x, y, 9, 9, pressed);
    let ink = if pressed {
        pal::LABEL_DIM
    } else {
        pal::TITLE_TEXT
    };
    if unshade {
        c.rect(x + 2, y + 2, 5, 5, ink);
        c.rect(
            x + 3,
            y + 4,
            3,
            2,
            if pressed { pal::FACE_DARK } else { pal::BTN },
        );
    } else {
        c.hline(x + 2, y + 2, 5, ink);
        c.hline(x + 2, y + 3, 5, ink);
    }
}

/// Side tiles: frame colour with an inner edge.
fn side(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, inner_right: bool) {
    c.rect(x, y, w, h, pal::FACE);
    let edge = if inner_right { x + w - 1 } else { x };
    c.vline(edge, y, h, pal::SHADOW);
    let outer = if inner_right { x } else { x + w - 1 };
    c.vline(outer, y, h, pal::HILITE);
}

fn bottom_bar(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32) {
    c.rect(x, y, w, h, pal::FACE);
    c.hline(x, y, w, pal::HILITE);
}

/// `pledit.bmp` 280×186.
pub fn pledit() -> Canvas {
    let mut c = Canvas::new(280, 186, pal::FACE_DARK);
    for (y, active) in [(0, true), (21, false)] {
        title_piece(&mut c, 0, y, 25, active);
        title_piece(&mut c, 26, y, 100, active);
        title_piece(&mut c, 127, y, 25, active);
        title_piece(&mut c, 153, y, 25, active);
        grip(&mut c, 4, y, 21, active);
        grip(&mut c, 127, y, 25, active);
        grip(&mut c, 153, y, 4, active);
        title_label(&mut c, 26, y, 100, "PLAYLIST", active);
        btn_shade(&mut c, 158, y + 3, false, false);
        btn_x(&mut c, 167, y + 3, false);
        // Outer edges below the bar, like the side tiles under them.
        c.vline(0, y + 14, 6, pal::HILITE);
        c.vline(177, y + 14, 6, pal::SHADOW);
        c.vline(177, y, 14, pal::TITLE_LINE);
    }
    // Side tiles and scroll bar parts (y 42, 29 tall).
    side(&mut c, 0, 42, 12, 29, true);
    side(&mut c, 31, 42, 5, 29, false);
    c.rect(36, 42, 8, 29, pal::LCD_BG);
    side(&mut c, 44, 42, 7, 29, false);
    c.vline(50, 42, 29, pal::SHADOW);
    // Scroll thumb normal / pressed 8×18.
    for (x, pressed) in [(52, false), (61, true)] {
        c.button(x, 53, 8, 18, pressed);
        for gy in [60, 62, 64] {
            c.hline(
                x + 2,
                gy,
                4,
                if pressed { pal::ACCENT } else { pal::BTN_SH },
            );
        }
    }
    btn_x(&mut c, 52, 42, true);
    btn_shade(&mut c, 62, 42, true, false);
    // Shade bar pieces.
    for (y, active) in [(42, true), (57, false)] {
        let bg = if active {
            pal::TITLE_BG
        } else {
            pal::TITLE_BG_IN
        };
        c.rect(99, y, 50, 14, bg);
        c.hline(99, y + 13, 50, pal::TITLE_LINE);
        btn_shade(&mut c, 99 + 29, y + 3, false, true);
        btn_x(&mut c, 99 + 39, y + 3, false);
    }
    for y in [42, 57] {
        c.rect(72, y, 25, 14, pal::TITLE_BG);
        c.hline(72, y + 13, 25, pal::TITLE_LINE);
    }
    btn_shade(&mut c, 128, 45, false, true);
    btn_shade(&mut c, 150, 42, true, true);

    // Bottom left (0,72) 125×38 with ADD REM SEL MISC buttons at x 14/43/72/101, y +8..+26.
    bottom_bar(&mut c, 0, 72, 125, 38);
    c.vline(11, 72, 1, pal::HILITE);
    c.vline(0, 72, 38, pal::HILITE);
    c.hline(0, 72 + 37, 125, pal::SHADOW);
    for (x, label) in [(14, "ADD"), (43, "REM"), (72, "SEL"), (101, "MISC")] {
        c.button(x, 80, 22, 18, false);
        c.text_centered(x, 80, 22, 18, label, pal::TITLE_TEXT);
    }
    // Bottom right (126,72) 150×38.
    let (bx, by) = (126, 72);
    bottom_bar(&mut c, bx, by, 150, 38);
    c.hline(bx, by + 37, 150, pal::SHADOW);
    c.vline(bx + 149, by, 38, pal::SHADOW);
    c.well(bx + 7, by + 9, 90, 8, pal::LCD_BG);
    // Mini transport (rel x 6..59, y 23..30) and time well (rel 64..95).
    let boxes = [
        (6, 7, Icon::Prev),
        (13, 10, Icon::Play),
        (23, 10, Icon::Pause),
        (33, 9, Icon::Stop),
        (42, 9, Icon::Next),
        (51, 9, Icon::Eject),
    ];
    for (x, w, kind) in boxes {
        icon(&mut c, bx + x, by + 23, w, 8, kind, 5, pal::LABEL);
    }
    c.rect(bx + 63, by + 22, 34, 8, pal::LCD_BG);
    c.px(bx + 84, by + 24, pal::LCD_FG);
    c.px(bx + 84, by + 26, pal::LCD_FG);
    // Scroll arrows (rel 135..143, 2..6 and 7..11).
    icon_arrow(&mut c, bx + 135, by + 2, true);
    icon_arrow(&mut c, bx + 135, by + 7, false);
    // LIST button (rel 106..128, 8..26).
    c.button(bx + 106, by + 8, 23, 18, false);
    c.text_centered(bx + 106, by + 8, 23, 18, "LIST", pal::TITLE_TEXT);
    // Resize grip.
    for i in 0..4 {
        let o = i * 3;
        for k in 0..=o {
            c.px(bx + 147 - k, by + 35 - (o - k), pal::LABEL_DIM);
        }
    }
    // Bottom fill (179,0) 25×38 and visualizer piece (205,0) 75×38.
    bottom_bar(&mut c, 179, 0, 25, 38);
    bottom_bar(&mut c, 205, 0, 75, 38);
    c.hline(179, 37, 25, pal::SHADOW);
    c.hline(205, 37, 75, pal::SHADOW);
    c.vline(279, 0, 38, pal::SHADOW);
    c.well(205 + 3, 38 - 26, 72, 16, pal::BLACK);

    // Pop-up menu items 22×18 (normal at X, highlighted at X+23) and bars 3 px wide.
    let menus: [(i32, i32, &[&str]); 5] = [
        (0, 48, &["URL", "DIR", "FILE"]),
        (54, 100, &["ALL", "CROP", "SEL", "MISC"]),
        (104, 150, &["INV", "NONE", "ALL"]),
        (154, 200, &["SORT", "INFO", "OPTS"]),
        (204, 250, &["NEW", "SAVE", "LOAD"]),
    ];
    for (x, bar_x, labels) in menus {
        // Labels listed top to bottom for rows y 111, 130, 149 (and 168 for REM row 3).
        let ys: Vec<i32> = if labels.len() == 4 {
            vec![111, 130, 149, 168]
        } else {
            vec![111, 130, 149]
        };
        let order: Vec<&str> = if labels.len() == 4 {
            // REM: row 2 (y 111) remove all, row 1 crop, row 0 remove selected, row 3 (y 168) misc.
            vec![labels[0], labels[1], labels[2], labels[3]]
        } else {
            labels.to_vec()
        };
        for (y, label) in ys.iter().zip(order) {
            for (dx, hi) in [(0, false), (23, true)] {
                c.rect(
                    x + dx,
                    *y,
                    22,
                    18,
                    if hi { pal::ACCENT_DARK } else { pal::FACE_DARK },
                );
                c.bevel(x + dx, *y, 22, 18, false);
                c.text_centered(
                    x + dx,
                    *y,
                    22,
                    18,
                    label,
                    if hi { pal::TITLE_TEXT } else { pal::LABEL },
                );
            }
        }
        let bar_h = if labels.len() == 4 { 72 } else { 54 };
        c.rect(bar_x, 111, 3, bar_h, pal::ACCENT);
    }
    c
}

fn icon_arrow(c: &mut Canvas, x: i32, y: i32, up: bool) {
    let ink = pal::LABEL;
    for i in 0..3 {
        let row = if up { y + 1 + i } else { y + 3 - i };
        c.hline(x + 4 - i, row, 1 + 2 * i, ink);
    }
}

/// `gen.bmp` 194×109.
pub fn gen_bmp() -> Canvas {
    let mut c = Canvas::new(194, 109, pal::FACE_DARK);
    for (y, active) in [(0, true), (21, false)] {
        for x in [0, 26, 52, 78, 104, 130] {
            title_piece(&mut c, x, y, 25, active);
        }
        grip(&mut c, 4, y, 21, active);
        grip(&mut c, 104, y, 25, active);
        grip(&mut c, 130, y, 10, active);
        let close_y = if active { 3 } else { 24 };
        btn_x(&mut c, 144, close_y, false);
    }
    btn_x(&mut c, 148, 42, true);
    side(&mut c, 127, 42, 11, 29, true);
    side(&mut c, 139, 42, 8, 29, false);
    side(&mut c, 158, 42, 11, 24, true);
    side(&mut c, 170, 42, 8, 24, false);
    bottom_bar(&mut c, 0, 42, 125, 14);
    bottom_bar(&mut c, 0, 57, 125, 14);
    for i in 0..3 {
        let o = i * 3;
        for k in 0..=o {
            c.px(122 - k, 57 + 12 - (o - k), pal::LABEL_DIM);
        }
    }
    bottom_bar(&mut c, 127, 72, 25, 14);
    // "Not resizable" marker 7×7.
    c.rect(118, 72, 7, 7, pal::FACE_DARK);
    c.bevel(118, 72, 7, 7, true);
    // Fonts: digits at rows 72/80, letters at 88/96, 7 rows each. Glyph cells use the
    // title colour; delimiter columns use a colour that never appears in a cell.
    let delim = 0xFF00FF;
    let rows: [(i32, &str); 2] = [(72, "0123456789-:"), (88, "ABCDEFGHIJKLMNOPQRSTUVWXYZ")];
    for (base, chars) in rows {
        for (y, active) in [(base, true), (base + 8, false)] {
            let cell_bg = if active {
                pal::TITLE_BG
            } else {
                pal::TITLE_BG_IN
            };
            let ink = if active {
                pal::TITLE_TEXT
            } else {
                pal::TITLE_TEXT_IN
            };
            let mut x = 0;
            c.rect(0, y, if base == 72 { 117 } else { 194 }, 7, delim);
            x += 1;
            for ch in chars.chars() {
                let w = font::advance(ch) - 1;
                c.rect(x, y, w + 1, 7, cell_bg);
                c.glyph(x, y + 1, ch, ink);
                x += w + 2;
            }
        }
    }
    c
}

/// `genex.bmp` 130×75.
pub fn genex() -> Canvas {
    let mut c = Canvas::new(130, 75, pal::FACE_DARK);
    c.button(0, 0, 47, 15, false);
    c.button(0, 15, 47, 15, true);
    // Colours at y 0, x 48 + 2i.
    let colors = [
        pal::LCD_BG,
        pal::PL_TEXT,
        pal::FACE_DARK,
        pal::TITLE_TEXT,
        pal::TITLE_TEXT,
        pal::HILITE,
        pal::ACCENT_DARK,
        pal::FACE,
        pal::TITLE_TEXT,
        pal::HILITE,
        pal::FACE,
        pal::SHADOW,
        pal::FACE_DARK,
        pal::HILITE,
        pal::SHADOW,
        pal::TITLE_TEXT_IN,
        pal::FACE,
        pal::FACE_DARK,
        pal::PL_CURRENT,
        0x2E6B3A,
        pal::TITLE_TEXT,
        0x26352B,
        0x0E1014,
        pal::PL_TEXT,
    ];
    for (i, col) in colors.iter().enumerate() {
        c.rect(48 + 2 * i as i32, 0, 2, 1, *col);
    }
    // Arrow buttons 14×14 and pressed (+28).
    for (x, y, dir) in [(0, 31, 'u'), (14, 31, 'd'), (0, 45, 'l'), (14, 45, 'r')] {
        for (dx, pressed) in [(0, false), (28, true)] {
            c.button(x + dx, y, 14, 14, pressed);
            arrow(&mut c, x + dx + 4, y + 4, dir);
        }
    }
    // Vertical thumb 14×28 at x 56 / 70; horizontal thumb 28×14 at (84,31) / (84,45).
    for (x, pressed) in [(56, false), (70, true)] {
        c.button(x, 31, 14, 28, pressed);
        for gy in [42, 44, 46] {
            c.hline(x + 4, gy, 6, pal::BTN_SH);
        }
    }
    for (y, pressed) in [(31, false), (45, true)] {
        c.button(84, y, 28, 14, pressed);
        for gx in [95, 97, 99] {
            c.vline(gx, y + 4, 6, pal::BTN_SH);
        }
    }
    // Pixel (111,0) marks "not set": a colour none of the entries use.
    c.px(111, 0, 0x00C6FF);
    c
}

fn arrow(c: &mut Canvas, x: i32, y: i32, dir: char) {
    for i in 0..3 {
        match dir {
            'u' => c.hline(x + 3 - i, y + 1 + i, 1 + 2 * i, pal::LABEL),
            'd' => c.hline(x + 3 - i, y + 4 - i, 1 + 2 * i, pal::LABEL),
            'l' => c.vline(x + 1 + i, y + 3 - i, 1 + 2 * i, pal::LABEL),
            _ => c.vline(x + 4 - i, y + 3 - i, 1 + 2 * i, pal::LABEL),
        }
    }
}

/// `video.bmp` and `mb.bmp` 234×119.
pub fn video_like(label: &str) -> Canvas {
    let mut c = Canvas::new(234, 119, pal::FACE_DARK);
    for (y, active) in [(0, true), (21, false)] {
        title_piece(&mut c, 0, y, 25, active);
        title_piece(&mut c, 26, y, 100, active);
        title_piece(&mut c, 127, y, 25, active);
        title_piece(&mut c, 153, y, 25, active);
        grip(&mut c, 4, y, 21, active);
        grip(&mut c, 127, y, 25, active);
        grip(&mut c, 153, y, 10, active);
        title_label(&mut c, 26, y, 100, label, active);
        btn_x(&mut c, 167, y + 3, false);
    }
    side(&mut c, 127, 42, 11, 29, true);
    side(&mut c, 139, 42, 8, 29, false);
    btn_x(&mut c, 148, 42, true);
    bottom_bar(&mut c, 0, 42, 125, 38);
    bottom_bar(&mut c, 0, 81, 125, 38);
    bottom_bar(&mut c, 127, 81, 25, 38);
    for i in 0..3 {
        let o = i * 3;
        for k in 0..=o {
            c.px(122 - k, 81 + 35 - (o - k), pal::LABEL_DIM);
        }
    }
    c.well(92, 42 + 15, 30, 14, pal::BLACK);
    let glyphs = ['F', '1', '2', 'T', 'M'];
    for (i, g) in glyphs.iter().enumerate() {
        let x = 9 + 15 * i as i32;
        c.button(x, 51, 15, 18, false);
        c.glyph(x + 5, 51 + 6, *g, pal::TITLE_TEXT);
        let px = 158 + 15 * i as i32;
        c.button(px, 42, 15, 18, true);
        c.glyph(px + 6, 42 + 7, *g, pal::ACCENT);
    }
    c
}
