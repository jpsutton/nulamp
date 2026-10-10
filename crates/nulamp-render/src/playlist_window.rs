//! Playlist editor frame drawing (spec 04 §1–§6): tiling, bottom bar, scroll bar, pop-ups.
//!
//! The list rows are drawn by the caller (they need the system font) into the list area.

use crate::frame::{Frame, blit, blit_stretch};
use nulamp_skin::fonts::text_cell;
use nulamp_skin::{Skin, SkinBitmap as B};

pub const MIN_SIZE: (i32, i32) = (275, 116);

/// Snaps a size to the 25×29 steps, rounding up (04 §1).
pub fn snap_size(w: i32, h: i32) -> (i32, i32) {
    let w = w.max(MIN_SIZE.0);
    let h = h.max(MIN_SIZE.1);
    let sw = 275 + (w - 275 + 24) / 25 * 25;
    let sh = 116 + (h - 116 + 28) / 29 * 29;
    (sw, sh)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlButton {
    Close,
    Shade,
}

/// The five bottom-bar pop-up menus (04 §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupMenu {
    Add,
    Rem,
    Sel,
    Misc,
    List,
}

impl PopupMenu {
    /// (bar source x, bar height, bar destination x, items x, item source x values bottom to top
    /// as (x, y)).
    fn layout(self, w: i32) -> (i32, i32, i32, i32, &'static [(i32, i32)]) {
        match self {
            Self::Add => (48, 54, 11, 14, &[(0, 149), (0, 130), (0, 111)]),
            Self::Rem => (
                100,
                72,
                40,
                43,
                &[(54, 149), (54, 130), (54, 111), (54, 168)],
            ),
            Self::Sel => (150, 54, 69, 72, &[(104, 149), (104, 130), (104, 111)]),
            Self::Misc => (200, 54, 98, 101, &[(154, 149), (154, 130), (154, 111)]),
            Self::List => (
                250,
                54,
                w - 47,
                w - 44,
                &[(204, 149), (204, 130), (204, 111)],
            ),
        }
    }

    pub fn item_count(self) -> usize {
        if self == Self::Rem { 4 } else { 3 }
    }

    /// Items x position.
    pub fn items_x(self, w: i32) -> i32 {
        self.layout(w).3
    }

    /// Row index for a window y, if over an item row.
    pub fn row_at(self, w: i32, h: i32, x: i32, y: i32) -> Option<usize> {
        let ix = self.items_x(w);
        if x < ix || x >= ix + 22 {
            return None;
        }
        (0..self.item_count()).find(|&r| {
            let top = h - 30 - 18 * r as i32;
            y >= top && y < top + 18
        })
    }
}

/// Mini time display content (04 §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiniTime {
    Hidden,
    Time {
        minutes: u32,
        seconds: u32,
        remaining: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlFrameView {
    pub w: i32,
    pub h: i32,
    pub active: bool,
    pub shade: bool,
    pub pressed: Option<PlButton>,
    /// Scroll thumb offset 0..=track−18 in pixels, and pressed state.
    pub thumb: i32,
    pub thumb_pressed: bool,
    /// Info string, up to 18 characters (04 §4.1).
    pub info: String,
    pub time: MiniTime,
    pub popup: Option<(PopupMenu, Option<usize>)>,
}

impl Default for PlFrameView {
    fn default() -> Self {
        Self {
            w: 275,
            h: 116,
            active: true,
            shade: false,
            pressed: None,
            thumb: 0,
            thumb_pressed: false,
            info: String::new(),
            time: MiniTime::Hidden,
            popup: None,
        }
    }
}

/// Draws a row of `text.bmp` glyphs.
pub fn draw_text_bmp(f: &mut Frame, skin: &Skin, text: &str, x: i32, y: i32) {
    let t = skin.bitmap(B::Text);
    for (i, c) in text.chars().enumerate() {
        let cell = text_cell(c);
        blit(
            f,
            t,
            cell.0 as i32 * 5,
            cell.1 as i32 * 6,
            5,
            6,
            x + 5 * i as i32,
            y,
        );
    }
}

/// Number of fill tiles left and right of the title centre and the slice widths (04 §2.2).
fn title_layout(w: i32) -> (i32, bool) {
    let n = ((w - 150) / 25).max(0);
    (n / 2, n % 2 == 1)
}

pub fn draw_playlist_frame(skin: &Skin, v: &PlFrameView) -> Frame {
    if v.shade {
        return draw_shade(skin, v);
    }
    let p = skin.bitmap(B::Pledit);
    let (w, h) = (v.w, v.h);
    let mut f = Frame::new(w as u32, h as u32, skin.pledit.normal_bg);
    let ty = if v.active { 0 } else { 21 };

    // Title bar.
    blit(&mut f, p, 0, ty, 25, 20, 0, 0);
    let (half, odd) = title_layout(w);
    let mut x = 25;
    if odd {
        blit(&mut f, p, 127, ty, 12, 20, x, 0);
        x += 12;
    }
    for _ in 0..half {
        blit(&mut f, p, 127, ty, 25, 20, x, 0);
        x += 25;
    }
    blit(&mut f, p, 26, ty, 100, 20, x, 0);
    x += 100;
    if odd {
        blit(&mut f, p, 127 + 12, ty, 13, 20, x, 0);
        x += 13;
    }
    for _ in 0..half {
        blit(&mut f, p, 127, ty, 25, 20, x, 0);
        x += 25;
    }
    if x < w - 25 {
        blit_stretch(&mut f, p, 127, ty, 25, 20, x, 0, w - 25 - x, 20);
    }
    blit(&mut f, p, 153, ty, 25, 20, w - 25, 0);

    // Sides.
    let tiles = (h - 58) / 29;
    let rem = (h - 58) % 29;
    for k in 0..tiles {
        let y = 20 + 29 * k;
        side_row(&mut f, p, w, 42, 29, y, 29);
    }
    if rem > 0 {
        side_row(&mut f, p, w, 42, 29, 20 + 29 * tiles, rem);
    }

    // Bottom bar.
    let by = h - 38;
    blit(&mut f, p, 0, 72, 125, 38, 0, by);
    let fill_w = w - 275;
    let k = fill_w / 25;
    let vis = k >= 3;
    let mut x = 125;
    let tiles = if vis { k - 3 } else { k };
    for _ in 0..tiles {
        blit(&mut f, p, 179, 0, 25, 38, x, by);
        x += 25;
    }
    let remainder = fill_w % 25;
    if remainder > 0 {
        blit_stretch(&mut f, p, 179, 0, 25, 38, x, by, remainder, 38);
        x += remainder;
    }
    if vis {
        blit(&mut f, p, 205, 0, 75, 38, x, by);
    }
    blit(&mut f, p, 126, 72, 150, 38, w - 150, by);

    // Title buttons.
    match v.pressed {
        Some(PlButton::Close) => blit(&mut f, p, 52, 42, 9, 9, w - 11, 3),
        Some(PlButton::Shade) => blit(&mut f, p, 62, 42, 9, 9, w - 20, 3),
        None => {}
    }

    // Scroll thumb.
    let thumb_sx = if v.thumb_pressed { 61 } else { 52 };
    blit(&mut f, p, thumb_sx, 53, 8, 18, w - 15, 20 + v.thumb);

    // Info string and mini time.
    let info: String = format!("{:<18}", v.info).chars().take(18).collect();
    draw_text_bmp(&mut f, skin, &info, w - 143, h - 28);
    draw_mini_time(&mut f, skin, v.time, w - 86, h - 15);

    if let Some((menu, hi)) = v.popup {
        draw_popup(&mut f, p, menu, hi, w, h);
    }
    f
}

fn side_row(f: &mut Frame, p: &Frame, w: i32, sy: i32, sh: i32, y: i32, rows: i32) {
    let draw = |f: &mut Frame, sx: i32, sw: i32, dx: i32| {
        if rows == sh {
            blit(f, p, sx, sy, sw, sh, dx, y);
        } else {
            blit_stretch(f, p, sx, sy, sw, sh, dx, y, sw, rows);
        }
    };
    draw(f, 0, 12, 0);
    draw(f, 31, 5, w - 20);
    draw(f, 36, 8, w - 15);
    draw(f, 44, 7, w - 7);
}

fn draw_mini_time(f: &mut Frame, skin: &Skin, t: MiniTime, x: i32, y: i32) {
    let MiniTime::Time {
        minutes,
        seconds,
        remaining,
    } = t
    else {
        return;
    };
    let text = skin.bitmap(B::Text);
    let digit = |f: &mut Frame, d: u32, dx: i32| blit(f, text, d as i32 * 5, 6, 5, 6, dx, y);
    let sign = |f: &mut Frame, dx: i32| {
        if remaining {
            blit(f, text, 75, 6, 5, 6, dx, y);
        } else {
            blit(f, text, 142, 0, 5, 6, dx, y);
        }
    };
    if minutes < 100 {
        blit(f, text, 142, 0, 4, 6, x, y);
        sign(f, x + 4);
    } else {
        sign(f, x);
        digit(f, (minutes / 100) % 10, x + 4);
    }
    let m = minutes % 100;
    digit(f, m / 10, x + 9);
    digit(f, m % 10, x + 14);
    digit(f, (seconds / 10) % 10, x + 22);
    digit(f, seconds % 10, x + 27);
}

fn draw_popup(f: &mut Frame, p: &Frame, menu: PopupMenu, hi: Option<usize>, w: i32, h: i32) {
    let (bar_sx, bar_h, bar_dx, items_x, items) = menu.layout(w);
    blit(f, p, bar_sx, 111, 3, bar_h, bar_dx, h - 30 - (bar_h - 18));
    for (row, &(sx, sy)) in items.iter().enumerate() {
        let x = if hi == Some(row) { sx + 23 } else { sx };
        blit(f, p, x, sy, 22, 18, items_x, h - 30 - 18 * row as i32);
    }
}

fn draw_shade(skin: &Skin, v: &PlFrameView) -> Frame {
    let p = skin.bitmap(B::Pledit);
    let w = v.w;
    let mut f = Frame::new(w as u32, 14, 0);
    blit(&mut f, p, 72, 42, 25, 14, 0, 0);
    let mut x = 25;
    while x + 25 <= w - 50 {
        blit(&mut f, p, 72, 57, 25, 14, x, 0);
        x += 25;
    }
    if x < w - 50 {
        blit_stretch(&mut f, p, 72, 57, 24, 14, x, 0, w - 50 - x, 14);
    }
    blit(
        &mut f,
        p,
        99,
        if v.active { 42 } else { 57 },
        50,
        14,
        w - 50,
        0,
    );
    let (unshade_sx, unshade_sy) = if v.pressed == Some(PlButton::Shade) {
        (150, 42)
    } else {
        (128, 45)
    };
    blit(&mut f, p, unshade_sx, unshade_sy, 9, 9, w - 20, 3);
    let (close_sx, close_sy) = if v.pressed == Some(PlButton::Close) {
        (52, 42)
    } else {
        (167, 3)
    };
    blit(&mut f, p, close_sx, close_sy, 9, 9, w - 11, 3);
    f
}

/// Bitmap-font shade-mode text (04 §3.4): `N. TITLE` and the time right-aligned at w − 29.
pub fn draw_shade_text_bitmap(f: &mut Frame, skin: &Skin, w: i32, title: &str, time: &str) {
    let time_x = w - 29 - 5 * time.chars().count() as i32;
    let max_chars = ((time_x - 4) / 5).max(0) as usize;
    let mut chars: Vec<char> = title.to_uppercase().chars().collect();
    if chars.len() > max_chars {
        chars.truncate(max_chars);
        if let Some(last) = chars.last_mut() {
            *last = '…';
        }
    }
    let t: String = chars.into_iter().collect();
    draw_text_bmp(f, skin, &t, 4, 4);
    draw_text_bmp(f, skin, time, time_x, 4);
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
    fn snapping() {
        assert_eq!(snap_size(275, 116), (275, 116));
        assert_eq!(snap_size(276, 117), (300, 145));
        assert_eq!(snap_size(100, 50), (275, 116));
        assert_eq!(snap_size(450, 290), (450, 290));
    }

    #[test]
    fn minimal_frame() {
        let skin = probe::skin(&[]);
        let f = draw_playlist_frame(&skin, &PlFrameView::default());
        ok(check_rect(&f, B::Pledit, 0, 0, 25, 20, 0, 0));
        // 275 wide: n = 5 fill tiles: 12-px slice + 2 tiles left, 13-px slice + 2 tiles right.
        ok(check_rect(&f, B::Pledit, 127, 0, 12, 20, 25, 0));
        ok(check_rect(&f, B::Pledit, 26, 0, 100, 20, 25 + 12 + 50, 0));
        ok(check_rect(&f, B::Pledit, 139, 0, 13, 20, 187, 0));
        ok(check_rect(&f, B::Pledit, 153, 0, 25, 3, 250, 0));
        // Sides: (116 − 58) / 29 = 2 tiles.
        ok(check_rect(&f, B::Pledit, 0, 42, 12, 29, 0, 20));
        ok(check_rect(&f, B::Pledit, 0, 42, 12, 29, 0, 49));
        ok(check_rect(&f, B::Pledit, 31, 42, 5, 29, 255, 49));
        ok(check_rect(&f, B::Pledit, 44, 42, 7, 29, 268, 20));
        // Bottom: left and right pieces meet at 125.
        ok(check_rect(&f, B::Pledit, 0, 72, 125, 38, 0, 78));
        ok(check_rect(&f, B::Pledit, 126, 72, 150, 10, 125, 78));
    }

    #[test]
    fn wide_frame_has_visualizer_piece() {
        let skin = probe::skin(&[]);
        let v = PlFrameView {
            w: 450,
            h: 290,
            ..PlFrameView::default()
        };
        let f = draw_playlist_frame(&skin, &v);
        // k = 7 fill tiles: 4 tiles, then the 75-px piece left of the bottom-right piece.
        ok(check_rect(&f, B::Pledit, 179, 0, 25, 38, 125, 252));
        ok(check_rect(&f, B::Pledit, 205, 0, 75, 38, 225, 252));
        ok(check_rect(&f, B::Pledit, 126, 72, 150, 10, 300, 252));
        // Title: n = 12, even: 6 tiles each side.
        ok(check_rect(&f, B::Pledit, 26, 0, 100, 20, 25 + 150, 0));
    }

    #[test]
    fn popup_items() {
        let skin = probe::skin(&[]);
        let v = PlFrameView {
            popup: Some((PopupMenu::Rem, Some(3))),
            ..PlFrameView::default()
        };
        let f = draw_playlist_frame(&skin, &v);
        ok(check_rect(&f, B::Pledit, 100, 111, 3, 72, 40, 116 - 84));
        ok(check_rect(&f, B::Pledit, 54, 149, 22, 18, 43, 116 - 30));
        ok(check_rect(&f, B::Pledit, 77, 168, 22, 18, 43, 116 - 84));
        assert_eq!(PopupMenu::Add.row_at(275, 116, 20, 116 - 30), Some(0));
        assert_eq!(PopupMenu::Add.row_at(275, 116, 20, 116 - 66), Some(2));
        assert_eq!(PopupMenu::List.items_x(300), 256);
    }

    #[test]
    fn mini_time_and_thumb() {
        let skin = probe::skin(&[]);
        let v = PlFrameView {
            time: MiniTime::Time {
                minutes: 3,
                seconds: 45,
                remaining: true,
            },
            thumb: 10,
            thumb_pressed: true,
            ..PlFrameView::default()
        };
        let f = draw_playlist_frame(&skin, &v);
        let (x, y) = (275 - 86, 116 - 15);
        ok(check_rect(&f, B::Text, 75, 6, 5, 6, x + 4, y));
        ok(check_rect(&f, B::Text, 0, 6, 5, 6, x + 9, y));
        ok(check_rect(&f, B::Text, 15, 6, 5, 6, x + 14, y));
        ok(check_rect(&f, B::Text, 20, 6, 5, 6, x + 22, y));
        ok(check_rect(&f, B::Text, 25, 6, 5, 6, x + 27, y));
        ok(check_rect(&f, B::Pledit, 61, 53, 8, 18, 260, 30));
    }

    #[test]
    fn shade_frame() {
        let skin = probe::skin(&[]);
        let v = PlFrameView {
            shade: true,
            w: 300,
            ..PlFrameView::default()
        };
        let f = draw_playlist_frame(&skin, &v);
        assert_eq!(f.height(), 14);
        ok(check_rect(&f, B::Pledit, 72, 42, 25, 14, 0, 0));
        ok(check_rect(&f, B::Pledit, 99, 42, 20, 3, 250, 0));
        ok(check_rect(&f, B::Pledit, 128, 45, 9, 9, 280, 3));
    }
}
