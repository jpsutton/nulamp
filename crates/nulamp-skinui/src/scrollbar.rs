//! Scroll bars drawn with the `genex.bmp` arrow buttons and thumbs (05 §1.5).

use nulamp_render::frame::blit_stretch;
use nulamp_render::{Frame, blit};
use nulamp_skin::{Skin, SkinBitmap};

use crate::Rect;

/// Thickness of a scroll bar (the `genex.bmp` arrow buttons are 14×14).
pub const SCROLLBAR_W: i32 = 14;
const ARROW: i32 = 14;
/// Fixed parts of the thumb sprite: 4 + 10 + 4 pixels.
const THUMB_MIN: i32 = 18;

/// Part of a scroll bar under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollPart {
    /// Up or left arrow.
    Up,
    /// Down or right arrow.
    Down,
    /// Track before the thumb.
    PageUp,
    /// Track after the thumb.
    PageDown,
    Thumb,
}

/// A scroll bar over `total` units (rows, or pixels for a horizontal bar) of which
/// `visible` fit; `first` is the first visible unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollBar {
    pub rect: Rect,
    pub horizontal: bool,
    pub total: usize,
    pub visible: usize,
    pub first: usize,
    /// Units scrolled by an arrow click.
    pub step: usize,
    pressed: Option<ScrollPart>,
    /// Pointer offset inside the thumb while it is dragged.
    grab: i32,
}

impl Default for ScrollBar {
    fn default() -> Self {
        Self {
            rect: Rect::default(),
            horizontal: false,
            total: 0,
            visible: 1,
            first: 0,
            step: 1,
            pressed: None,
            grab: 0,
        }
    }
}

impl ScrollBar {
    /// A vertical bar.
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            ..Self::default()
        }
    }

    /// A horizontal bar scrolling `step` units per arrow click.
    pub fn horizontal(rect: Rect, step: usize) -> Self {
        Self {
            rect,
            horizontal: true,
            step: step.max(1),
            ..Self::default()
        }
    }

    /// Sets the counts and keeps `first` in range.
    pub fn set_counts(&mut self, total: usize, visible: usize) {
        self.total = total;
        self.visible = visible.max(1);
        self.first = self.first.min(self.max_first());
    }

    pub fn max_first(&self) -> usize {
        self.total.saturating_sub(self.visible)
    }

    pub fn scrollable(&self) -> bool {
        self.total > self.visible
    }

    /// Scrolls so `first` is the first visible unit. Returns whether it moved.
    pub fn scroll_to(&mut self, first: usize) -> bool {
        let f = first.min(self.max_first());
        let moved = f != self.first;
        self.first = f;
        moved
    }

    pub fn scroll_by(&mut self, delta: i64) -> bool {
        let f = (self.first as i64 + delta).clamp(0, self.max_first() as i64);
        self.scroll_to(f as usize)
    }

    /// Scrolls the least amount that shows unit `row`.
    pub fn ensure_visible(&mut self, row: usize) -> bool {
        if row < self.first {
            self.scroll_to(row)
        } else if row >= self.first + self.visible {
            self.scroll_to(row + 1 - self.visible)
        } else {
            false
        }
    }

    /// Start and length of the bar along its axis.
    fn axis(&self) -> (i32, i32) {
        if self.horizontal {
            (self.rect.x, self.rect.w)
        } else {
            (self.rect.y, self.rect.h)
        }
    }

    fn along(&self, x: i32, y: i32) -> i32 {
        if self.horizontal { x } else { y }
    }

    /// The track between the arrow buttons: (start, length) along the axis.
    fn track(&self) -> (i32, i32) {
        let (start, len) = self.axis();
        (start + ARROW, (len - 2 * ARROW).max(0))
    }

    /// Thumb (start, length) along the axis; `None` when there is nothing to scroll or no
    /// room.
    pub fn thumb(&self) -> Option<(i32, i32)> {
        let (t0, len) = self.track();
        if !self.scrollable() || len < THUMB_MIN {
            return None;
        }
        let h =
            ((len as i64 * self.visible as i64 / self.total as i64) as i32).clamp(THUMB_MIN, len);
        let max = self.max_first().max(1) as i64;
        let p = t0 + ((len - h) as i64 * self.first as i64 / max) as i32;
        Some((p, h))
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<ScrollPart> {
        if !self.rect.contains(x, y) {
            return None;
        }
        let a = self.along(x, y);
        let (t0, len) = self.track();
        if a < t0 {
            return Some(ScrollPart::Up);
        }
        if a >= t0 + len {
            return Some(ScrollPart::Down);
        }
        match self.thumb() {
            Some((tp, _)) if a < tp => Some(ScrollPart::PageUp),
            Some((tp, th)) if a < tp + th => Some(ScrollPart::Thumb),
            Some(_) => Some(ScrollPart::PageDown),
            None => None,
        }
    }

    /// Left button pressed. Returns true when the point is on the bar.
    pub fn mouse_down(&mut self, x: i32, y: i32) -> bool {
        if !self.rect.contains(x, y) {
            return false;
        }
        let part = self.hit(x, y);
        let (step, page) = (self.step as i64, self.visible.max(1) as i64);
        match part {
            Some(ScrollPart::Up) => {
                self.scroll_by(-step);
            }
            Some(ScrollPart::Down) => {
                self.scroll_by(step);
            }
            Some(ScrollPart::PageUp) => {
                self.scroll_by(-page);
            }
            Some(ScrollPart::PageDown) => {
                self.scroll_by(page);
            }
            Some(ScrollPart::Thumb) => {
                let a = self.along(x, y);
                self.grab = a - self.thumb().map(|t| t.0).unwrap_or(a);
            }
            None => {}
        }
        self.pressed = part;
        true
    }

    /// Pointer moved while the button is held. Returns whether `first` changed.
    pub fn mouse_move(&mut self, x: i32, y: i32) -> bool {
        if self.pressed != Some(ScrollPart::Thumb) {
            return false;
        }
        let Some((_, h)) = self.thumb() else {
            return false;
        };
        let (t0, len) = self.track();
        let span = (len - h).max(1) as i64;
        let pos = (self.along(x, y) - self.grab - t0).clamp(0, len - h) as i64;
        let first = (pos * self.max_first() as i64 + span / 2) / span;
        self.scroll_to(first as usize)
    }

    pub fn mouse_up(&mut self) {
        self.pressed = None;
    }

    pub fn is_pressed(&self) -> bool {
        self.pressed.is_some()
    }

    pub fn draw(&self, frame: &mut Frame, ox: i32, oy: i32, skin: &Skin) {
        let g = skin.bitmap(SkinBitmap::Genex);
        let c = &skin.genex;
        let r = self.rect;
        let (x, y) = (ox + r.x, oy + r.y);
        let (t0, len) = self.track();
        let thumb = self.thumb();
        let track_color = if thumb.is_some() {
            c.scroll_bg
        } else {
            c.scroll_dead
        };
        let pressed = |p| if self.pressed == Some(p) { 28 } else { 0 };
        // Arrow sprites: up/down at y 31, left/right at y 45; pressed ones 28 px right.
        let arrow_y = if self.horizontal { 45 } else { 31 };
        let (first_at, last_at) = if self.horizontal {
            ((x, y), (x + r.w - ARROW, y))
        } else {
            ((x, y), (x, y + r.h - ARROW))
        };
        if self.horizontal {
            frame.fill_rect(ox + t0, y, len, r.h, track_color);
        } else {
            frame.fill_rect(x, oy + t0, r.w, len, track_color);
        }
        let up = pressed(ScrollPart::Up);
        let down = 14 + pressed(ScrollPart::Down);
        blit(frame, g, up, arrow_y, ARROW, ARROW, first_at.0, first_at.1);
        blit(frame, g, down, arrow_y, ARROW, ARROW, last_at.0, last_at.1);
        let Some((tp, th)) = thumb else { return };
        let thumb_pressed = self.pressed == Some(ScrollPart::Thumb);
        let stretch = th - THUMB_MIN;
        let (s1, s2) = (stretch / 2, stretch - stretch / 2);
        // (source offset, source length, destination length) along the axis.
        let pieces = [(0, 4, 4), (4, 5, s1), (9, 10, 10), (19, 5, s2), (24, 4, 4)];
        let mut d = 0;
        for (so, sl, dl) in pieces {
            if dl > 0 {
                if self.horizontal {
                    // Horizontal thumb at (84, 31), pressed at (84, 45).
                    let sy = if thumb_pressed { 45 } else { 31 };
                    blit_stretch(frame, g, 84 + so, sy, sl, ARROW, ox + tp + d, y, dl, ARROW);
                } else {
                    // Vertical thumb at (56, 31), pressed at (70, 31).
                    let sx = if thumb_pressed { 70 } else { 56 };
                    blit_stretch(frame, g, sx, 31 + so, ARROW, sl, x, oy + tp + d, ARROW, dl);
                }
            }
            d += dl;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> ScrollBar {
        let mut s = ScrollBar::new(Rect::new(100, 0, SCROLLBAR_W, 128));
        s.set_counts(1000, 10);
        s
    }

    #[test]
    fn thumb_geometry_and_hits() {
        let mut s = bar();
        assert_eq!(s.thumb(), Some((14, 18)));
        assert_eq!(s.hit(105, 3), Some(ScrollPart::Up));
        assert_eq!(s.hit(105, 20), Some(ScrollPart::Thumb));
        assert_eq!(s.hit(105, 60), Some(ScrollPart::PageDown));
        assert_eq!(s.hit(105, 120), Some(ScrollPart::Down));
        assert_eq!(s.hit(90, 20), None);
        s.scroll_to(990);
        assert_eq!(s.thumb(), Some((14 + 100 - 18, 18)));
        assert_eq!(s.hit(105, 20), Some(ScrollPart::PageUp));
        // Nothing to scroll: no thumb, no track hits.
        s.set_counts(5, 10);
        assert_eq!((s.thumb(), s.first), (None, 0));
        assert_eq!(s.hit(105, 60), None);
    }

    #[test]
    fn buttons_pages_and_thumb_drag() {
        let mut s = bar();
        assert!(s.mouse_down(105, 120));
        s.mouse_up();
        assert_eq!(s.first, 1);
        s.mouse_down(105, 60);
        s.mouse_up();
        assert_eq!(s.first, 11);
        s.mouse_down(105, 3);
        s.mouse_up();
        assert_eq!(s.first, 10);
        // Drag the thumb to the bottom of the track, then to the middle.
        s.scroll_to(0);
        assert!(s.mouse_down(105, 15));
        assert!(s.is_pressed());
        assert!(s.mouse_move(105, 500));
        assert_eq!(s.first, 990);
        s.mouse_move(105, 15 + 41);
        assert_eq!(s.first, 495);
        s.mouse_up();
        assert!(!s.mouse_move(105, 15));
        assert!(!s.mouse_down(0, 0));
    }

    #[test]
    fn horizontal_bar_scrolls_pixels() {
        // 400 px of columns in a 200 px view; the bar is 128 px long.
        let mut s = ScrollBar::horizontal(Rect::new(0, 50, 128, SCROLLBAR_W), 16);
        s.set_counts(400, 200);
        assert_eq!(s.thumb(), Some((14, 50)));
        assert_eq!(s.hit(3, 55), Some(ScrollPart::Up));
        assert_eq!(s.hit(120, 55), Some(ScrollPart::Down));
        assert_eq!(s.hit(30, 55), Some(ScrollPart::Thumb));
        assert_eq!(s.hit(30, 40), None);
        s.mouse_down(120, 55);
        s.mouse_up();
        assert_eq!(s.first, 16);
        s.mouse_down(100, 55);
        s.mouse_up();
        assert_eq!(s.first, 200);
        s.mouse_down(14 + 50 + 5, 55);
        s.mouse_move(14 + 50 + 5 - 25, 99);
        s.mouse_up();
        assert_eq!(s.first, 100);
    }

    #[test]
    fn ensure_visible_scrolls_least() {
        let mut s = bar();
        assert!(s.ensure_visible(15));
        assert_eq!(s.first, 6);
        assert!(!s.ensure_visible(10));
        assert!(s.ensure_visible(2));
        assert_eq!(s.first, 2);
        assert!(s.scroll_by(-100) && s.first == 0);
    }
}
