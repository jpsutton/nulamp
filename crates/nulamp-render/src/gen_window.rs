//! Generic ("embedded") skinned window frame (spec 05 §1).

use crate::frame::{Frame, blit, blit_stretch};
use nulamp_skin::{Skin, SkinBitmap as B};

/// Client area of a generic window of size w×h: (x, y, width, height).
pub fn client_rect(w: i32, h: i32) -> (i32, i32, i32, i32) {
    (11, 20, w - 19, h - 34)
}

/// Snaps a size to the 25×29 steps (05 §1.4), minimum 275×116.
pub fn snap_size(w: i32, h: i32) -> (i32, i32) {
    let w = w.max(275);
    let h = h.max(116);
    (
        275 + (w - 275 + 24) / 25 * 25,
        116 + (h - 116 + 28) / 29 * 29,
    )
}

/// Glyph (x, width) in `gen.bmp` for a title character, and whether it is a digit-row glyph.
fn gen_glyph(skin: &Skin, c: char) -> Option<(i32, i32, bool)> {
    let f = &skin.gen_font;
    match c {
        'A'..='Z' => {
            let g = f.letters[(c as u8 - b'A') as usize];
            Some((g.x, g.width, false))
        }
        '0'..='9' | '-' | ':' if skin.pledit.use_gen_nums => {
            let i = match c {
                '-' => 10,
                ':' => 11,
                d => (d as u8 - b'0') as usize,
            };
            let g = f.digits[i];
            Some((g.x, g.width, true))
        }
        _ => None,
    }
}

/// Exact title text width with the `gen.bmp` font; unknown characters advance 5 px (05 §1.2).
pub fn title_width(skin: &Skin, title: &str) -> i32 {
    title
        .chars()
        .map(|c| gen_glyph(skin, c).map(|g| g.1).unwrap_or(5))
        .sum()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenView {
    pub w: i32,
    pub h: i32,
    pub title: String,
    pub active: bool,
    pub close_pressed: bool,
    /// Draws the "not resizable" marker.
    pub fixed_size: bool,
}

pub fn draw_gen_frame(skin: &Skin, v: &GenView) -> Frame {
    let g = skin.bitmap(B::Gen);
    let (w, h) = (v.w, v.h);
    let mut f = Frame::new(w as u32, h as u32, 0);
    let ty = if v.active { 0 } else { 21 };
    let title: String = v.title.to_uppercase().chars().take(31).collect();
    let exact = title_width(skin, &title);
    let block = ((exact + 24) / 25 * 25).min(w - 100).max(0);
    let fill = ((w - 100 - block) / 25).max(0);
    let (left_fill, odd) = (fill / 2, fill % 2 == 1);

    // Title bar.
    blit(&mut f, g, 0, ty, 25, 20, 0, 0);
    let mut x = 25;
    if odd {
        blit(&mut f, g, 104, ty, 12, 20, x, 0);
        x += 12;
    }
    for _ in 0..left_fill {
        blit(&mut f, g, 104, ty, 25, 20, x, 0);
        x += 25;
    }
    blit(&mut f, g, 26, ty, 25, 20, x, 0);
    x += 25;
    let text_start = x;
    for _ in 0..block / 25 {
        blit(&mut f, g, 52, ty, 25, 20, x, 0);
        x += 25;
    }
    blit(&mut f, g, 78, ty, 25, 20, x, 0);
    x += 25;
    if odd {
        blit(&mut f, g, 104 + 12, ty, 13, 20, x, 0);
        x += 13;
    }
    for _ in 0..left_fill {
        blit(&mut f, g, 104, ty, 25, 20, x, 0);
        x += 25;
    }
    if x < w - 25 {
        blit_stretch(&mut f, g, 104, ty, 25, 20, x, 0, w - 25 - x, 20);
    }
    blit(&mut f, g, 130, ty, 25, 20, w - 25, 0);
    // Title text.
    let mut tx = text_start + (block - exact) / 2 + if (block - exact) % 2 != 0 { 1 } else { 0 };
    for c in title.chars() {
        match gen_glyph(skin, c) {
            Some((gx, gw, digit)) => {
                let gy = match (digit, v.active) {
                    (false, true) => 88,
                    (false, false) => 96,
                    (true, true) => 72,
                    (true, false) => 80,
                };
                blit(&mut f, g, gx, gy, gw, 7, tx, 4);
                tx += gw;
            }
            None => tx += 5,
        }
    }
    // Close button.
    if v.close_pressed {
        blit(&mut f, g, 148, 42, 9, 9, w - 11, 3);
    } else {
        blit(
            &mut f,
            g,
            144,
            if v.active { 3 } else { 24 },
            9,
            9,
            w - 11,
            3,
        );
    }

    // Sides: whole tiles, stretched remainder, then the 24-px lower pieces.
    let body = h - 20 - 38;
    let tiles = body.max(0) / 29;
    let rem = body.max(0) % 29;
    for k in 0..tiles {
        let y = 20 + 29 * k;
        blit(&mut f, g, 127, 42, 11, 29, 0, y);
        blit(&mut f, g, 139, 42, 8, 29, w - 8, y);
    }
    if rem > 0 {
        let y = 20 + 29 * tiles;
        blit_stretch(&mut f, g, 127, 42, 11, 29, 0, y, 11, rem);
        blit_stretch(&mut f, g, 139, 42, 8, 29, w - 8, y, 8, rem);
    }
    blit(&mut f, g, 158, 42, 11, 24, 0, h - 38);
    blit(&mut f, g, 170, 42, 8, 24, w - 8, h - 38);

    // Bottom bar.
    blit(&mut f, g, 0, 42, 125, 14, 0, h - 14);
    let mut x = 125;
    while x + 25 <= w - 125 {
        blit(&mut f, g, 127, 72, 25, 14, x, h - 14);
        x += 25;
    }
    if x < w - 125 {
        blit_stretch(&mut f, g, 127, 72, 25, 14, x, h - 14, w - 125 - x, 14);
    }
    blit(&mut f, g, 0, 57, 125, 14, w - 125, h - 14);
    if v.fixed_size {
        blit(&mut f, g, 118, 72, 7, 7, w - 125 + 112, h - 14 + 2);
    }
    // Client background.
    let (cx, cy, cw, ch) = client_rect(w, h);
    f.fill_rect(cx, cy, cw, ch, 0);
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{self, check_rect};

    fn ok(r: Result<(), String>) {
        if let Err(e) = r {
            panic!("{e}");
        }
    }

    #[test]
    fn frame_pieces() {
        let skin = probe::skin(&[]);
        let v = GenView {
            w: 275,
            h: 116,
            title: String::new(),
            active: true,
            close_pressed: false,
            fixed_size: true,
        };
        let f = draw_gen_frame(&skin, &v);
        ok(check_rect(&f, B::Gen, 0, 0, 25, 20, 0, 0));
        ok(check_rect(&f, B::Gen, 130, 0, 14, 3, 250, 0));
        ok(check_rect(&f, B::Gen, 144, 3, 9, 9, 264, 3));
        ok(check_rect(&f, B::Gen, 127, 42, 11, 29, 0, 20));
        ok(check_rect(&f, B::Gen, 139, 42, 8, 29, 267, 49));
        ok(check_rect(&f, B::Gen, 158, 42, 11, 24, 0, 78));
        ok(check_rect(&f, B::Gen, 0, 42, 125, 14, 0, 102));
        ok(check_rect(&f, B::Gen, 0, 57, 112, 2, 150, 102));
        ok(check_rect(&f, B::Gen, 118, 72, 7, 7, 262, 104));
        assert_eq!(client_rect(275, 116), (11, 20, 256, 82));
    }

    #[test]
    fn title_layout_uses_glyph_widths() {
        let skin = nulamp_skin::builtin::skin();
        assert!(title_width(&skin, "AB") > 0);
        assert_eq!(title_width(&skin, "  "), 10);
        assert_eq!(snap_size(300, 120), (300, 145));
    }
}
