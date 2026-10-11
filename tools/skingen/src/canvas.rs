//! Drawing helpers for generating skin bitmaps.

use crate::font;
use nulamp_skin::Bitmap;

/// Colour palette of the built-in skin ("Graphite"). The display colours are shared with the
/// app icon: display black, its dot grid, the green of the analyzer bars and the grey of their
/// peak caps.
pub mod pal {
    pub const FACE: u32 = 0x2B2F37;
    pub const FACE_DARK: u32 = 0x23262D;
    pub const HILITE: u32 = 0x3B414D;
    pub const SHADOW: u32 = 0x101216;
    pub const BLACK: u32 = 0x000000;
    pub const BTN: u32 = 0x363B45;
    pub const BTN_HI: u32 = 0x4B515E;
    pub const BTN_SH: u32 = 0x14161B;
    pub const LCD_BG: u32 = 0x0A0D10;
    pub const LCD_DOT: u32 = 0x171C24;
    pub const LCD_DIM: u32 = 0x1E3A26;
    pub const LCD_FG: u32 = 0x4BE35D;
    pub const LCD_LINE: u32 = 0x252A33;
    pub const CAP: u32 = 0x6C7487;
    pub const LABEL: u32 = 0x8B919D;
    pub const LABEL_DIM: u32 = 0x5A606C;
    pub const ACCENT: u32 = 0x4BE35D;
    pub const ACCENT_DARK: u32 = 0x1E3A26;
    pub const TITLE_BG: u32 = 0x1D2027;
    pub const TITLE_BG_IN: u32 = 0x24272E;
    pub const TITLE_LINE: u32 = 0x0A0B0E;
    pub const GROOVE_HI: u32 = 0x353A44;
    pub const GROOVE_HI_IN: u32 = 0x2E323A;
    pub const GROOVE_LO: u32 = 0x0E1014;
    pub const TITLE_TEXT: u32 = 0xD9DDE5;
    pub const TITLE_TEXT_IN: u32 = 0x7A808C;
    pub const LED_ON: u32 = 0x4BE35D;
    pub const LED_OFF: u32 = 0x3F4550;
    pub const RED: u32 = 0xE8463C;
    pub const PL_TEXT: u32 = 0xA9AFBA;
    pub const PL_CURRENT: u32 = 0xFFFFFF;
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

    /// One-pixel bevel: light top/left, dark bottom/right (inverted when `sunken`).
    pub fn bevel(&mut self, x: i32, y: i32, w: i32, h: i32, sunken: bool) {
        let (a, b) = if sunken {
            (pal::SHADOW, pal::HILITE)
        } else {
            (pal::HILITE, pal::SHADOW)
        };
        self.edges(x, y, w, h, a, b);
    }

    /// One-pixel frame: `a` on the top and left edges, `b` on the bottom and right.
    pub fn edges(&mut self, x: i32, y: i32, w: i32, h: i32, a: u32, b: u32) {
        self.hline(x, y, w, a);
        self.vline(x, y, h, a);
        self.hline(x, y + h - 1, w, b);
        self.vline(x + w - 1, y, h, b);
    }

    /// A raised (or pressed) flat button with a one-pixel bevel.
    pub fn button(&mut self, x: i32, y: i32, w: i32, h: i32, pressed: bool) {
        if pressed {
            self.rect(x, y, w, h, pal::FACE_DARK);
            self.edges(x, y, w, h, pal::BTN_SH, pal::BTN_HI);
        } else {
            self.rect(x, y, w, h, pal::BTN);
            self.edges(x, y, w, h, pal::BTN_HI, pal::BTN_SH);
        }
    }

    /// A sunken well (LCD-like).
    pub fn well(&mut self, x: i32, y: i32, w: i32, h: i32, fill: u32) {
        self.rect(x, y, w, h, fill);
        self.bevel(x - 1, y - 1, w + 2, h + 2, true);
    }

    /// Display black with the icon's dot grid: a dot wherever both `x - ox` and `y - oy` are
    /// even. Pass the origin that lands on an even screen pixel, so every piece of the left
    /// display lines up with the others and with the visualizer's grid.
    pub fn lcd_dots(&mut self, x: i32, y: i32, w: i32, h: i32, ox: i32, oy: i32) {
        self.rect(x, y, w, h, pal::LCD_BG);
        for py in y..y + h {
            for px in x..x + w {
                if (px - ox) % 2 == 0 && (py - oy) % 2 == 0 {
                    self.px(px, py, pal::LCD_DOT);
                }
            }
        }
    }

    /// A 14-row title bar: flat fill, a dark bottom edge, and two grooves either side of
    /// a centred label (`x0..x1` is the span the grooves may use).
    #[allow(clippy::too_many_arguments)]
    pub fn title_bar(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        active: bool,
        label: &str,
        x0: i32,
        x1: i32,
    ) {
        let (bg, hi, ink) = if active {
            (pal::TITLE_BG, pal::GROOVE_HI, pal::TITLE_TEXT)
        } else {
            (pal::TITLE_BG_IN, pal::GROOVE_HI_IN, pal::TITLE_TEXT_IN)
        };
        self.rect(x, y, w, 14, bg);
        self.hline(x, y + 13, w, pal::TITLE_LINE);
        let tw = font::text_width(label);
        let tx = x + (w - tw) / 2;
        let spans = if label.is_empty() {
            vec![(x0, x1)]
        } else {
            vec![(x0, tx - 5), (tx + tw + 5, x1)]
        };
        for (a, b) in spans {
            self.grooves(a, y, b - a, hi);
        }
        self.text(tx, y + 4, label, ink);
    }

    /// The title bar's grip: two ridges (light row over dark row) at rows 4–5 and 7–8.
    pub fn grooves(&mut self, x: i32, y: i32, w: i32, hi: u32) {
        if w <= 0 {
            return;
        }
        for gy in [y + 4, y + 7] {
            self.hline(x, gy, w, hi);
            self.hline(x, gy + 1, w, pal::GROOVE_LO);
        }
    }

    /// The app icon (an "N" of analyzer bars with peak caps on a dotted display tile),
    /// drawn for 13×13 px.
    pub fn app_icon_13(&mut self, x: i32, y: i32) {
        self.rect(x + 1, y + 2, 11, 9, pal::LCD_BG);
        self.rect(x + 2, y + 1, 9, 11, pal::LCD_BG);
        self.hline(x + 2, y, 9, pal::HILITE);
        self.hline(x + 2, y + 12, 9, pal::HILITE);
        self.vline(x, y + 2, 9, pal::HILITE);
        self.vline(x + 12, y + 2, 9, pal::HILITE);
        for (dx, dy) in [(1, 1), (11, 1), (1, 11), (11, 11)] {
            self.px(x + dx, y + dy, pal::HILITE);
        }
        self.vline(x + 11, y + 2, 9, pal::LCD_LINE);
        self.hline(x + 2, y + 11, 9, pal::LCD_LINE);
        self.vline(x + 2, y + 2, 9, pal::LCD_FG);
        self.vline(x + 10, y + 2, 9, pal::LCD_FG);
        for (dx, dy) in [(4, 3), (6, 5), (8, 7)] {
            self.vline(x + dx, y + dy, 3, pal::LCD_FG);
        }
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
