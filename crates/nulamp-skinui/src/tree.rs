//! Tree view: indented rows with expand/collapse boxes, drawn with `genex.bmp` colours.
//!
//! The view does not own the tree. Its owner flattens the visible nodes into [`TreeRow`]s and
//! reacts to [`TreeEvent`]s (loading children, rebuilding the rows).

use std::time::Instant;

use nulamp_render::Frame;
use nulamp_skin::Skin;

use crate::scrollbar::{SCROLLBAR_W, ScrollBar};
use crate::{Key, Rect, UiText, frame_rect};

/// Indentation per level.
pub const INDENT: i32 = 12;
/// Size of the expand/collapse box.
const BOX: i32 = 9;
const DOUBLE_CLICK_MS: u128 = 450;

/// One visible row of a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: usize,
    pub label: String,
    pub expandable: bool,
    pub expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeEvent {
    /// The selection moved to a row.
    Selected(usize),
    /// Show a row's children.
    Expand(usize),
    /// Hide a row's children.
    Collapse(usize),
    /// Double-click or Enter on a row.
    Activated(usize),
}

#[derive(Debug, Clone)]
pub struct TreeView {
    pub rect: Rect,
    pub rows: Vec<TreeRow>,
    pub selected: Option<usize>,
    pub scroll: ScrollBar,
    row_h: i32,
    last_click: Option<(usize, Instant)>,
}

impl TreeView {
    pub fn new(rect: Rect) -> Self {
        let mut t = Self {
            rect,
            rows: Vec::new(),
            selected: None,
            scroll: ScrollBar::default(),
            row_h: 14,
            last_click: None,
        };
        t.layout();
        t
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.rect = rect;
        self.layout();
    }

    pub fn set_row_height(&mut self, h: i32) {
        if h > 0 && h != self.row_h {
            self.row_h = h;
            self.layout();
        }
    }

    /// Replaces the rows (the scroll position is kept when possible).
    pub fn set_rows(&mut self, rows: Vec<TreeRow>, selected: Option<usize>) {
        self.selected = selected.filter(|&s| s < rows.len());
        self.rows = rows;
        self.layout();
    }

    fn layout(&mut self) {
        let r = self.rect;
        self.scroll.rect = Rect::new(r.x + r.w - 1 - SCROLLBAR_W, r.y + 1, SCROLLBAR_W, r.h - 2);
        self.scroll.set_counts(self.rows.len(), self.visible());
    }

    /// Number of whole rows that fit.
    pub fn visible(&self) -> usize {
        ((self.rect.h - 2) / self.row_h).max(1) as usize
    }

    fn inner_width(&self) -> i32 {
        self.rect.w
            - 2
            - if self.scroll.scrollable() {
                SCROLLBAR_W
            } else {
                0
            }
    }

    /// Row under a point (client coordinates).
    pub fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        let r = self.rect;
        if x < r.x + 1 || x >= r.x + 1 + self.inner_width() || y < r.y + 1 || y >= r.y + r.h - 1 {
            return None;
        }
        let i = self.scroll.first + ((y - r.y - 1) / self.row_h) as usize;
        (i < self.rows.len()).then_some(i)
    }

    fn box_x(&self, row: usize) -> i32 {
        self.rect.x + 3 + self.rows[row].depth as i32 * INDENT
    }

    fn on_box(&self, row: usize, x: i32) -> bool {
        let bx = self.box_x(row);
        self.rows[row].expandable && x >= bx - 1 && x < bx + BOX + 1
    }

    /// The parent of a row: the nearest row above it with a smaller depth.
    pub fn parent_of(&self, row: usize) -> Option<usize> {
        let d = self.rows.get(row)?.depth;
        (0..row).rev().find(|&i| self.rows[i].depth < d)
    }

    fn select(&mut self, row: usize) -> TreeEvent {
        self.selected = Some(row);
        self.scroll.ensure_visible(row);
        TreeEvent::Selected(row)
    }

    fn toggle(&self, row: usize) -> Option<TreeEvent> {
        let r = &self.rows[row];
        match (r.expandable, r.expanded) {
            (true, false) => Some(TreeEvent::Expand(row)),
            (true, true) => Some(TreeEvent::Collapse(row)),
            _ => None,
        }
    }

    /// Left button pressed at client coordinates.
    pub fn mouse_down(&mut self, x: i32, y: i32, now: Instant) -> Option<TreeEvent> {
        if self.scroll.mouse_down(x, y) {
            return None;
        }
        let row = self.row_at(x, y)?;
        if self.on_box(row, x) {
            self.last_click = None;
            return self.toggle(row);
        }
        let double = matches!(self.last_click, Some((r, t)) if r == row && now.duration_since(t).as_millis() < DOUBLE_CLICK_MS);
        self.last_click = if double { None } else { Some((row, now)) };
        if double {
            return Some(TreeEvent::Activated(row));
        }
        Some(self.select(row))
    }

    pub fn mouse_move(&mut self, x: i32, y: i32) -> bool {
        self.scroll.mouse_move(x, y)
    }

    pub fn mouse_up(&mut self) {
        self.scroll.mouse_up();
    }

    /// Mouse wheel (positive = up).
    pub fn wheel(&mut self, notches: i32) -> bool {
        self.scroll.scroll_by(-(notches as i64) * 3)
    }

    /// Navigation keys: Up/Down/Page/Home/End move, Right expands or enters, Left collapses
    /// or goes to the parent, Enter activates.
    pub fn key(&mut self, key: Key) -> Option<TreeEvent> {
        let n = self.rows.len();
        if n == 0 {
            return None;
        }
        let cur = self.selected.unwrap_or(0).min(n - 1);
        let vis = self.visible();
        let target = match key {
            Key::Up => cur.saturating_sub(1),
            Key::Down => (cur + 1).min(n - 1),
            Key::PageUp => cur.saturating_sub(vis),
            Key::PageDown => (cur + vis).min(n - 1),
            Key::Home => 0,
            Key::End => n - 1,
            Key::Right => {
                let r = &self.rows[cur];
                if r.expandable && !r.expanded {
                    return Some(TreeEvent::Expand(cur));
                }
                if r.expanded && cur + 1 < n && self.rows[cur + 1].depth > r.depth {
                    cur + 1
                } else {
                    return None;
                }
            }
            Key::Left => {
                if self.rows[cur].expanded {
                    return Some(TreeEvent::Collapse(cur));
                }
                self.parent_of(cur)?
            }
            Key::Enter => return Some(TreeEvent::Activated(cur)),
            _ => return None,
        };
        if self.selected == Some(target) {
            return None;
        }
        Some(self.select(target))
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        ox: i32,
        oy: i32,
        skin: &Skin,
        text: &mut dyn UiText,
        focused: bool,
    ) {
        self.set_row_height(text.line_height());
        let c = &skin.genex;
        let r = self.rect;
        let (x, y) = (ox + r.x, oy + r.y);
        frame.fill_rect(x, y, r.w, r.h, c.item_bg);
        frame_rect(
            frame,
            x,
            y,
            r.w,
            r.h,
            if focused { c.window_text } else { c.divider },
        );
        let inner_w = self.inner_width();
        let first = self.scroll.first;
        for (k, row) in self
            .rows
            .iter()
            .enumerate()
            .skip(first)
            .take(self.visible())
        {
            let ry = y + 1 + (k - first) as i32 * self.row_h;
            let sel = self.selected == Some(k);
            let fg = if sel {
                let (bg, fg) = if focused {
                    (c.selbar_bg_active, c.selbar_text_active)
                } else {
                    (c.selbar_bg_inactive, c.selbar_text_inactive)
                };
                frame.fill_rect(x + 1, ry, inner_w, self.row_h, bg);
                fg
            } else {
                c.item_fg
            };
            let bx = ox + self.rect.x + 3 + row.depth as i32 * INDENT;
            let by = ry + (self.row_h - BOX) / 2;
            if row.expandable {
                frame_rect(frame, bx, by, BOX, BOX, c.divider);
                frame.fill_rect(bx + 2, by + 4, BOX - 4, 1, fg);
                if !row.expanded {
                    frame.fill_rect(bx + 4, by + 2, 1, BOX - 4, fg);
                }
            }
            let tx = bx + BOX + 4;
            let right = x + 1 + inner_w;
            let shown = text.fit(&row.label, right - tx - 2);
            text.draw(
                frame,
                &shown,
                tx,
                ry,
                fg,
                (x + 1, ry, right, ry + self.row_h),
            );
        }
        if self.scroll.scrollable() {
            self.scroll.draw(frame, ox, oy, skin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn row(depth: usize, label: &str, expandable: bool, expanded: bool) -> TreeRow {
        TreeRow {
            depth,
            label: label.into(),
            expandable,
            expanded,
        }
    }

    fn tree() -> TreeView {
        let mut t = TreeView::new(Rect::new(0, 0, 120, 2 + 14 * 4));
        t.set_rows(
            vec![
                row(0, "Local", true, true),
                row(1, "Artists", true, false),
                row(1, "Albums", true, true),
                row(2, "Abbey Road", false, false),
                row(1, "Most played", false, false),
                row(0, "Server", true, false),
            ],
            Some(0),
        );
        t
    }

    #[test]
    fn hit_testing_box_and_label() {
        let mut t = tree();
        let now = Instant::now();
        // Row 1 ("Artists", depth 1): box at x = 3 + 12.
        let y1 = 1 + 14 + 5;
        assert_eq!(t.row_at(40, y1), Some(1));
        assert_eq!(t.mouse_down(16, y1, now), Some(TreeEvent::Expand(1)));
        assert_eq!(t.selected, Some(0), "the box does not select");
        assert_eq!(t.mouse_down(60, y1, now), Some(TreeEvent::Selected(1)));
        assert_eq!(
            t.mouse_down(60, y1, now + Duration::from_millis(100)),
            Some(TreeEvent::Activated(1))
        );
        // Expanded row: the box collapses.
        assert_eq!(
            t.mouse_down(16, 1 + 28 + 5, now),
            Some(TreeEvent::Collapse(2))
        );
        // A leaf has no box.
        assert_eq!(
            t.mouse_down(28, 1 + 42 + 5, now),
            Some(TreeEvent::Selected(3))
        );
        assert_eq!(t.row_at(40, 200), None);
    }

    #[test]
    fn keyboard_navigation() {
        let mut t = tree();
        assert_eq!(t.key(Key::Left), Some(TreeEvent::Collapse(0)));
        assert_eq!(t.key(Key::Right), Some(TreeEvent::Selected(1)));
        assert_eq!(t.key(Key::Right), Some(TreeEvent::Expand(1)));
        assert_eq!(t.key(Key::Down), Some(TreeEvent::Selected(2)));
        assert_eq!(t.key(Key::Right), Some(TreeEvent::Selected(3)));
        assert_eq!(t.key(Key::Left), Some(TreeEvent::Selected(2)));
        assert_eq!(t.key(Key::End), Some(TreeEvent::Selected(5)));
        assert_eq!(t.scroll.first, 2, "the selection is scrolled into view");
        assert_eq!(t.key(Key::Down), None);
        assert_eq!(t.key(Key::Home), Some(TreeEvent::Selected(0)));
        assert_eq!(t.key(Key::Enter), Some(TreeEvent::Activated(0)));
        assert_eq!(t.parent_of(4), Some(0));
        assert_eq!(t.parent_of(0), None);
    }

    #[test]
    fn scrolling() {
        let mut t = tree();
        assert!(t.scroll.scrollable());
        assert!(t.wheel(-1));
        assert_eq!(t.scroll.first, 2);
        assert_eq!(t.row_at(40, 2), Some(2));
        t.set_rows(vec![row(0, "Only", false, false)], None);
        assert_eq!(t.scroll.first, 0);
        assert!(!t.wheel(-1));
    }
}
