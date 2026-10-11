//! Drawing helpers for generating skin bitmaps.

use crate::font;
use nulamp_skin::Bitmap;

/// Colour palette of the built-in skin.
pub mod pal {
    pub const FACE: u32 = 0x3A3F4B;
    pub const FACE_DARK: u32 = 0x2C3039;
    pub const HILITE: u32 = 0x5D6578;
    pub const SHADOW: u32 = 0x191B21;
    pub const BLACK: u32 = 0x000000;
    pub const LCD_BG: u32 = 0x0A0D10;
    pub const LCD_DIM: u32 = 0x173A22;
    pub const LCD_FG: u32 = 0x3CF06E;
    pub const LABEL: u32 = 0xA8B4C8;
    pub const LABEL_DIM: u32 = 0x6C7488;
    pub const ACCENT: u32 = 0x4AA8FF;
    pub const ACCENT_DARK: u32 = 0x1E4E80;
    pub const TITLE_A: u32 = 0x2F4C7A;
    pub const TITLE_B: u32 = 0x1B2C49;
    pub const TITLE_IN_A: u32 = 0x3C4048;
    pub const TITLE_IN_B: u32 = 0x2A2D33;
    pub const TITLE_TEXT: u32 = 0xE6ECF5;
    pub const TITLE_TEXT_IN: u32 = 0x8E949E;
    pub const LED_ON: u32 = 0x3CF06E;
    pub const LED_OFF: u32 = 0x1E3A27;
    pub const RED: u32 = 0xE8463C;
}

pub fn lerp(a: u32, b: u32, t: f32) -> u32 {
    let c = |s: u32| {
        let x = ((a >> s) & 0xFF) as f32;
        let y = ((b >> s) & 0xFF) as f32;
        ((x + (y - x) * t).round().clamp(0.0, 255.0) as u32) << s
    };
    c(16) | c(8) | c(0)
}

pub struct Canvas(pub Bitmap);

impl Canvas {
    pub fn new(w: u32, h: u32, fill: u32) -> Self {
        Self(Bitmap::new(w, h, fill))
    }

    pub fn px(&mut self, x: i32, y: i32, c: u32) {
        self.0.set(x, y, c);
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        self.0.fill_rect(x, y, w, h, c);
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, c: u32) {
        self.rect(x, y, w, 1, c);
    }

    pub fn vline(&mut self, x: i32, y: i32, h: i32, c: u32) {
        self.rect(x, y, 1, h, c);
    }

    /// Vertical gradient fill.
    pub fn vgrad(&mut self, x: i32, y: i32, w: i32, h: i32, top: u32, bottom: u32) {
        for i in 0..h {
            let t = if h > 1 {
                i as f32 / (h - 1) as f32
            } else {
                0.0
            };
            self.hline(x, y + i, w, lerp(top, bottom, t));
        }
    }

    /// One-pixel bevel: light top/left, dark bottom/right (inverted when `sunken`).
    pub fn bevel(&mut self, x: i32, y: i32, w: i32, h: i32, sunken: bool) {
        let (a, b) = if sunken {
            (pal::SHADOW, pal::HILITE)
        } else {
            (pal::HILITE, pal::SHADOW)
        };
        self.hline(x, y, w, a);
        self.vline(x, y, h, a);
        self.hline(x, y + h - 1, w, b);
        self.vline(x + w - 1, y, h, b);
    }

    /// A raised (or pressed) button face with bevel.
    pub fn button(&mut self, x: i32, y: i32, w: i32, h: i32, pressed: bool) {
        if pressed {
            self.vgrad(x, y, w, h, pal::FACE_DARK, pal::FACE);
        } else {
            self.vgrad(x, y, w, h, pal::HILITE, pal::FACE_DARK);
        }
        self.bevel(x, y, w, h, pressed);
    }

    /// A sunken well (LCD-like).
    pub fn well(&mut self, x: i32, y: i32, w: i32, h: i32, fill: u32) {
        self.rect(x, y, w, h, fill);
        self.bevel(x - 1, y - 1, w + 2, h + 2, true);
    }

    /// Draws a glyph of the pixel font at (x, y) with the given ink.
    pub fn glyph(&mut self, x: i32, y: i32, c: char, ink: u32) {
        if let Some(g) = font::glyph(c) {
            for (row, line) in g.iter().enumerate() {
                for (col, ch) in line.bytes().enumerate() {
                    if ch == b'#' {
                        self.px(x + col as i32, y + row as i32, ink);
                    }
                }
            }
        }
    }

    /// Draws a proportional label; returns its width.
    pub fn text(&mut self, x: i32, y: i32, s: &str, ink: u32) -> i32 {
        let mut cx = x;
        for c in s.chars() {
            self.glyph(cx, y, c, ink);
            cx += font::advance(c);
        }
        font::text_width(s)
    }

    /// Draws a label centred in a box.
    pub fn text_centered(&mut self, x: i32, y: i32, w: i32, h: i32, s: &str, ink: u32) {
        let tw = font::text_width(s);
        self.text(x + (w - tw) / 2, y + (h - 5) / 2, s, ink);
    }

    /// Copies a rectangle from another canvas.
    pub fn blit(&mut self, src: &Canvas, sx: i32, sy: i32, w: i32, h: i32, dx: i32, dy: i32) {
        for y in 0..h {
            for x in 0..w {
                self.px(dx + x, dy + y, src.0.pixel(sx + x, sy + y));
            }
        }
    }

    /// Copies a rectangle within this canvas.
    pub fn copy(&mut self, sx: i32, sy: i32, w: i32, h: i32, dx: i32, dy: i32) {
        let tmp = Canvas(self.0.clone());
        self.blit(&tmp, sx, sy, w, h, dx, dy);
    }
}
