//! Multi-column list with a header: sortable and resizable columns, multi-selection,
//! a scroll bar and virtual rows (the owner supplies cell text while drawing).

use std::time::Instant;

use nulamp_render::{Frame, blit};
use nulamp_skin::Skin;

use crate::scrollbar::{SCROLLBAR_W, ScrollBar};
use crate::selection::Selection;
use crate::{Key, Rect, UiText, frame_rect};

/// Narrowest a column can be dragged.
pub const MIN_COL_W: i32 = 16;
/// Pixels on each side of a header border that start a resize.
const BORDER_GRIP: i32 = 2;
/// Pointer travel that turns a press on a row into a drag.
const DRAG_START: i32 = 4;
const DOUBLE_CLICK_MS: u128 = 450;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub title: String,
    pub width: i32,
    /// Right-aligned cells (numbers, times).
    pub right: bool,
}

impl Column {
    pub fn new(title: &str, width: i32, right: bool) -> Self {
        Self {
            title: title.into(),
            width,
            right,
        }
    }
}

/// What dragging a header border does to the other columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizeMode {
    /// Only the dragged column changes; the table gets wider or narrower.
    #[default]
    Selected,
    /// The next column gives or takes the width, so the total stays the same.
    WithNext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableEvent {
    /// A header was clicked; the table's sort state is already updated.
    Sort {
        column: usize,
        descending: bool,
    },
    SelectionChanged,
    /// Double-click or Enter on a row.
    Activated(usize),
    ColumnResized(usize),
    /// The scroll position changed.
    Scrolled,
    /// The pointer moved far enough with the button held on selected rows.
    DragStart,
    /// The button was released after [`TableEvent::DragStart`].
    Dropped,
    /// Right-click, on a row or below the rows.
    ContextMenu(Option<usize>),
}

/// What part of the header is under a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderHit {
    Column(usize),
    /// The right border of a column (resize grip).
    Border(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Header(usize),
    Resize {
        col: usize,
        grab_x: i32,
        start_w: i32,
        /// Width of the next column when the drag started (0: none).
        next_w: i32,
    },
    Rows {
        row: usize,
        x: i32,
        y: i32,
        /// A plain press on an already selected row: select only it on release unless the
        /// rows get dragged.
        deferred: bool,
        dragging: bool,
    },
    Scroll,
    HScroll,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub rect: Rect,
    pub columns: Vec<Column>,
    /// Number of rows (cells are supplied while drawing).
    pub rows: usize,
    pub sel: Selection,
    /// Vertical scroll bar (rows).
    pub scroll: ScrollBar,
    /// Horizontal scroll bar (pixels), shown when the columns are wider than the body.
    pub hscroll: ScrollBar,
    /// Sorted column and direction (descending), shown in the header.
    pub sort: Option<(usize, bool)>,
    /// Every other row has the alternate colours.
    pub alt_rows: bool,
    pub resize_mode: ResizeMode,
    row_h: i32,
    drag: Option<Drag>,
    last_click: Option<(usize, Instant)>,
}

/// Pixels scrolled by a horizontal arrow click or key.
const H_STEP: usize = 16;

impl Table {
    pub fn new(rect: Rect, columns: Vec<Column>) -> Self {
        let mut t = Self {
            rect,
            columns,
            rows: 0,
            sel: Selection::new(),
            scroll: ScrollBar::default(),
            hscroll: ScrollBar::horizontal(Rect::default(), H_STEP),
            sort: None,
            alt_rows: true,
            resize_mode: ResizeMode::Selected,
            row_h: 14,
            drag: None,
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

    pub fn row_height(&self) -> i32 {
        self.row_h
    }

    /// Sets the number of rows; the selection loses rows past the end.
    pub fn set_rows(&mut self, n: usize) {
        self.rows = n;
        self.sel.clamp(n);
        self.layout();
    }

    pub fn header_height(&self) -> i32 {
        self.row_h + 3
    }

    /// Total width of the columns.
    pub fn content_width(&self) -> i32 {
        self.columns.iter().map(|c| c.width).sum()
    }

    /// Area of the rows: below the header, left of the vertical scroll bar and above the
    /// horizontal one (when the columns do not fit).
    pub fn body_rect(&self) -> Rect {
        let r = self.rect;
        let top = r.y + 1 + self.header_height();
        let w = (r.w - 2 - SCROLLBAR_W).max(0);
        let hbar = if self.content_width() > w {
            SCROLLBAR_W
        } else {
            0
        };
        Rect::new(r.x + 1, top, w, (r.y + r.h - 1 - top - hbar).max(0))
    }

    fn layout(&mut self) {
        let b = self.body_rect();
        self.scroll.rect = Rect::new(b.x + b.w, b.y, SCROLLBAR_W, b.h);
        let vis = self.visible_rows();
        self.scroll.set_counts(self.rows, vis);
        self.hscroll.rect = Rect::new(b.x, b.y + b.h, b.w, SCROLLBAR_W);
        self.hscroll
            .set_counts(self.content_width().max(0) as usize, b.w.max(1) as usize);
    }

    /// Number of whole rows that fit.
    pub fn visible_rows(&self) -> usize {
        (self.body_rect().h / self.row_h).max(1) as usize
    }

    /// Top visible row.
    pub fn first(&self) -> usize {
        self.scroll.first
    }

    /// Row under a point, when it is inside the body and below no more than the last row.
    pub fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.body_rect().contains(x, y) {
            return None;
        }
        self.row_at_y(y)
    }

    fn row_at_y(&self, y: i32) -> Option<usize> {
        let b = self.body_rect();
        if y < b.y {
            return None;
        }
        let i = self.scroll.first + ((y - b.y) / self.row_h) as usize;
        (i < self.rows).then_some(i)
    }

    /// Left x of each column (after horizontal scrolling) and its width, in client
    /// coordinates.
    fn column_spans(&self) -> impl Iterator<Item = (usize, i32, i32)> + '_ {
        let mut x = self.body_rect().x - self.hscroll.first as i32;
        self.columns.iter().enumerate().map(move |(i, c)| {
            let cx = x;
            x += c.width;
            (i, cx, c.width)
        })
    }

    pub fn header_hit(&self, x: i32, y: i32) -> Option<HeaderHit> {
        let b = self.body_rect();
        // The header row runs on above the vertical scroll bar, so the last border can be
        // grabbed at the body's right edge.
        if y < self.rect.y + 1 || y >= b.y || x < b.x || x >= self.rect.x + self.rect.w - 1 {
            return None;
        }
        let spans: Vec<_> = self.column_spans().collect();
        if let Some(&(i, _, _)) = spans
            .iter()
            .find(|(_, cx, w)| (x - (cx + w)).abs() <= BORDER_GRIP)
        {
            return Some(HeaderHit::Border(i));
        }
        spans
            .iter()
            .find(|(_, cx, w)| x >= *cx && x < cx + w)
            .map(|&(i, _, _)| HeaderHit::Column(i))
    }

    /// Next sort state for a click on `column`: the same column flips the direction, another
    /// column sorts ascending.
    pub fn toggle_sort(&mut self, column: usize) -> (usize, bool) {
        let next = match self.sort {
            Some((c, d)) if c == column => (column, !d),
            _ => (column, false),
        };
        self.sort = Some(next);
        next
    }

    pub fn select_all(&mut self) {
        self.sel.select_all(self.rows);
    }

    /// True while selected rows are being dragged out.
    pub fn is_dragging_rows(&self) -> bool {
        matches!(self.drag, Some(Drag::Rows { dragging: true, .. }))
    }

    /// True while the left button is held after a press inside the table.
    pub fn is_pressed(&self) -> bool {
        self.drag.is_some()
    }

    /// Left button pressed at client coordinates.
    pub fn mouse_down(
        &mut self,
        x: i32,
        y: i32,
        ctrl: bool,
        shift: bool,
        now: Instant,
    ) -> Option<TableEvent> {
        if self.scroll.mouse_down(x, y) {
            self.drag = Some(Drag::Scroll);
            return Some(TableEvent::Scrolled);
        }
        if self.hscroll.scrollable() && self.hscroll.mouse_down(x, y) {
            self.drag = Some(Drag::HScroll);
            return Some(TableEvent::Scrolled);
        }
        match self.header_hit(x, y) {
            Some(HeaderHit::Border(col)) => {
                self.drag = Some(Drag::Resize {
                    col,
                    grab_x: x,
                    start_w: self.columns[col].width,
                    next_w: self.columns.get(col + 1).map_or(0, |c| c.width),
                });
                return None;
            }
            Some(HeaderHit::Column(col)) => {
                self.drag = Some(Drag::Header(col));
                return None;
            }
            None => {}
        }
        if !self.body_rect().contains(x, y) {
            return None;
        }
        let Some(row) = self.row_at_y(y) else {
            self.last_click = None;
            if ctrl || shift || self.sel.is_empty() {
                return None;
            }
            self.sel.clear();
            return Some(TableEvent::SelectionChanged);
        };
        let double = matches!(self.last_click, Some((r, t)) if r == row && now.duration_since(t).as_millis() < DOUBLE_CLICK_MS);
        self.last_click = if double { None } else { Some((row, now)) };
        if double && !ctrl && !shift {
            if !self.sel.contains(row) {
                self.sel.set_only(row);
            }
            return Some(TableEvent::Activated(row));
        }
        let deferred = !ctrl && !shift && self.sel.contains(row);
        self.drag = Some(Drag::Rows {
            row,
            x,
            y,
            deferred,
            dragging: false,
        });
        if deferred {
            self.sel.cursor = Some(row);
            return None;
        }
        self.sel.click(row, ctrl, shift);
        Some(TableEvent::SelectionChanged)
    }

    /// Pointer moved (button held or not).
    pub fn mouse_move(&mut self, x: i32, y: i32) -> Option<TableEvent> {
        match self.drag.as_mut()? {
            Drag::Resize {
                col,
                grab_x,
                start_w,
                next_w,
            } => {
                let (col, next_w) = (*col, *next_w);
                let mut w = (*start_w + x - *grab_x).max(MIN_COL_W);
                let shared = self.resize_mode == ResizeMode::WithNext && next_w > 0;
                if shared {
                    // The next column keeps at least its minimum width.
                    w = w.min(*start_w + next_w - MIN_COL_W);
                }
                if self.columns[col].width == w {
                    return None;
                }
                self.columns[col].width = w;
                if shared {
                    self.columns[col + 1].width = *start_w + next_w - w;
                }
                self.layout();
                Some(TableEvent::ColumnResized(col))
            }
            Drag::Scroll => self.scroll.mouse_move(x, y).then_some(TableEvent::Scrolled),
            Drag::HScroll => self
                .hscroll
                .mouse_move(x, y)
                .then_some(TableEvent::Scrolled),
            Drag::Rows {
                x: sx,
                y: sy,
                dragging,
                ..
            } => {
                if *dragging || ((x - *sx).abs() <= DRAG_START && (y - *sy).abs() <= DRAG_START) {
                    return None;
                }
                *dragging = true;
                self.last_click = None;
                Some(TableEvent::DragStart)
            }
            Drag::Header(_) => None,
        }
    }

    /// Left button released.
    pub fn mouse_up(&mut self, x: i32, y: i32) -> Option<TableEvent> {
        match self.drag.take()? {
            Drag::Header(col) if self.header_hit(x, y) == Some(HeaderHit::Column(col)) => {
                let (column, descending) = self.toggle_sort(col);
                Some(TableEvent::Sort { column, descending })
            }
            Drag::Rows { dragging: true, .. } => Some(TableEvent::Dropped),
            Drag::Rows {
                row,
                deferred: true,
                ..
            } => {
                self.sel.set_only(row);
                Some(TableEvent::SelectionChanged)
            }
            Drag::Scroll => {
                self.scroll.mouse_up();
                None
            }
            Drag::HScroll => {
                self.hscroll.mouse_up();
                None
            }
            _ => None,
        }
    }

    /// Right button pressed: a row that is not selected becomes the selection.
    pub fn right_click(&mut self, x: i32, y: i32) -> Option<TableEvent> {
        if !self.body_rect().contains(x, y) {
            return None;
        }
        let row = self.row_at_y(y);
        if let Some(r) = row
            && !self.sel.contains(r)
        {
            self.sel.set_only(r);
        }
        Some(TableEvent::ContextMenu(row))
    }

    /// Mouse wheel (positive = up).
    pub fn wheel(&mut self, notches: i32) -> Option<TableEvent> {
        self.scroll
            .scroll_by(-(notches as i64) * 3)
            .then_some(TableEvent::Scrolled)
    }

    /// Horizontal wheel or Shift+wheel (positive = left).
    pub fn hwheel(&mut self, notches: i32) -> Option<TableEvent> {
        self.hscroll
            .scroll_by(-(notches as i64) * 3 * H_STEP as i64)
            .then_some(TableEvent::Scrolled)
    }

    /// Navigation keys move the cursor (Shift extends the selection); Left and Right scroll
    /// sideways; Enter activates.
    pub fn key(&mut self, key: Key, shift: bool) -> Option<TableEvent> {
        match key {
            Key::Left | Key::Right => {
                let d = if key == Key::Left { -1 } else { 1 } * H_STEP as i64;
                return self.hscroll.scroll_by(d).then_some(TableEvent::Scrolled);
            }
            _ => {}
        }
        if self.rows == 0 {
            return None;
        }
        let n = self.rows;
        let cur = self.sel.cursor.unwrap_or(0).min(n - 1);
        let vis = self.visible_rows();
        let target = match key {
            Key::Up if self.sel.cursor.is_none() => 0,
            Key::Up => cur.saturating_sub(1),
            Key::Down if self.sel.cursor.is_none() => 0,
            Key::Down => (cur + 1).min(n - 1),
            Key::PageUp => cur.saturating_sub(vis.saturating_sub(1).max(1)),
            Key::PageDown => (cur + vis.saturating_sub(1).max(1)).min(n - 1),
            Key::Home => 0,
            Key::End => n - 1,
            Key::Enter => return Some(TableEvent::Activated(cur)),
            _ => return None,
        };
        self.sel.move_cursor(target, shift);
        self.scroll.ensure_visible(target);
        Some(TableEvent::SelectionChanged)
    }

    /// Draws header, rows and scroll bar. `cell(row, column)` gives the text of a cell, or
    /// `None` while the row is not loaded.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        frame: &mut Frame,
        ox: i32,
        oy: i32,
        skin: &Skin,
        text: &mut dyn UiText,
        focused: bool,
        cell: &mut dyn FnMut(usize, usize) -> Option<String>,
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
        let b = self.body_rect();
        let hh = self.header_height();
        frame.fill_rect(x + 1, y + 1, r.w - 2, hh, c.empty_header_bg);
        // Header and rows are drawn into a buffer the size of the visible columns, which
        // clips horizontally scrolled columns at both edges.
        let mut buf = Frame::new(b.w.max(1) as u32, (hh + b.h).max(1) as u32, c.item_bg);
        self.draw_columns(&mut buf, b.x, skin, text, focused, cell);
        blit(frame, &buf, 0, 0, b.w, hh + b.h, ox + b.x, y + 1);
        self.scroll.draw(frame, ox, oy, skin);
        if self.hscroll.rect.h > 0 && b.y + b.h < r.y + r.h - 1 {
            self.hscroll.draw(frame, ox, oy, skin);
            let s = self.scroll.rect;
            frame.fill_rect(
                ox + s.x,
                oy + b.y + b.h,
                SCROLLBAR_W,
                SCROLLBAR_W,
                c.scroll_dead,
            );
        }
    }

    /// Header (at y 0) and rows (below it) into `buf`, whose x 0 is the body's left edge
    /// `left`.
    fn draw_columns(
        &self,
        buf: &mut Frame,
        left: i32,
        skin: &Skin,
        text: &mut dyn UiText,
        focused: bool,
        cell: &mut dyn FnMut(usize, usize) -> Option<String>,
    ) {
        let c = &skin.genex;
        let b = self.body_rect();
        let right = b.w;
        let hh = self.header_height();
        let (hy, by) = (0, hh);
        buf.fill_rect(0, 0, b.w, hh, c.empty_header_bg);
        let spans: Vec<_> = self
            .column_spans()
            .map(|(i, cx, w)| (i, cx - left, w))
            .collect();
        for &(i, cx, w) in &spans {
            if cx >= right {
                break;
            }
            if cx + w <= 0 {
                continue;
            }
            buf.fill_rect(cx, hy, w, hh, c.header_bg);
            buf.fill_rect(cx, hy, w, 1, c.header_frame_top);
            buf.fill_rect(cx, hy, 1, hh, c.header_frame_top);
            buf.fill_rect(cx, hy + hh - 1, w, 1, c.header_frame_bottom);
            buf.fill_rect(cx + w - 1, hy, 1, hh, c.header_frame_bottom);
            buf.fill_rect(cx + 1, hy + hh - 2, w - 2, 1, c.header_frame_middle);
            let arrow = match self.sort {
                Some((sc, desc)) if sc == i => Some(desc),
                _ => None,
            };
            let avail = w - 6 - if arrow.is_some() { 8 } else { 0 };
            let title = text.fit(&self.columns[i].title, avail);
            let clip = (cx + 1, hy, cx + w - 1, hy + hh);
            text.draw(buf, &title, cx + 3, hy + 1, c.header_text, clip);
            if let Some(desc) = arrow {
                let tx = cx + 3 + text.measure(&title) + 3;
                if tx + 5 <= cx + w - 2 {
                    sort_arrow(buf, tx, hy + hh / 2 - 2, desc, c.header_text);
                }
            }
        }
        // Rows.
        let first = self.scroll.first;
        for k in 0..self.visible_rows() {
            let row = first + k;
            if row >= self.rows {
                break;
            }
            let ry = by + k as i32 * self.row_h;
            if ry + self.row_h > by + b.h {
                break;
            }
            let fg = if self.sel.contains(row) {
                let (bg, fg) = if focused {
                    (c.selbar_bg_active, c.selbar_text_active)
                } else {
                    (c.selbar_bg_inactive, c.selbar_text_inactive)
                };
                buf.fill_rect(0, ry, b.w, self.row_h, bg);
                fg
            } else if self.alt_rows && row % 2 == 1 {
                buf.fill_rect(0, ry, b.w, self.row_h, c.alt_row_bg);
                c.alt_row_fg
            } else {
                c.item_fg
            };
            for &(i, cx, w) in &spans {
                if cx >= right {
                    break;
                }
                if cx + w <= 0 {
                    continue;
                }
                let Some(s) = cell(row, i) else { continue };
                if w <= 6 || s.is_empty() {
                    continue;
                }
                let s = text.fit(&s, w - 6);
                let tx = if self.columns[i].right {
                    cx + w - 3 - text.measure(&s)
                } else {
                    cx + 3
                };
                let clip = (cx + 1, ry, cx + w - 1, ry + self.row_h);
                text.draw(buf, &s, tx, ry, fg, clip);
            }
            if focused && self.sel.cursor == Some(row) {
                frame_rect(buf, 0, ry, b.w, self.row_h, c.divider);
            }
        }
    }
}

/// A 5×3 triangle: pointing up for ascending, down for descending.
fn sort_arrow(frame: &mut Frame, x: i32, y: i32, descending: bool, color: u32) {
    for i in 0..3 {
        let row = if descending { i } else { 2 - i };
        frame.fill_rect(x + i, y + row, 5 - 2 * i, 1, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 3 columns 40 px wide; header 17 px; body from y = 18; 10 rows visible.
    fn table(rows: usize) -> Table {
        let mut t = Table::new(
            Rect::new(0, 0, 2 + 120 + SCROLLBAR_W, 1 + 17 + 140 + 1),
            vec![
                Column::new("#", 40, true),
                Column::new("Artist", 40, false),
                Column::new("Title", 40, false),
            ],
        );
        t.set_rows(rows);
        t
    }

    fn row_y(i: usize) -> i32 {
        18 + i as i32 * 14 + 3
    }

    #[test]
    fn hit_testing() {
        let t = table(100);
        assert_eq!(t.visible_rows(), 10);
        assert_eq!(t.header_hit(10, 5), Some(HeaderHit::Column(0)));
        assert_eq!(t.header_hit(41, 5), Some(HeaderHit::Border(0)));
        assert_eq!(t.header_hit(60, 5), Some(HeaderHit::Column(1)));
        assert_eq!(t.header_hit(60, 20), None);
        assert_eq!(t.row_at(60, row_y(0)), Some(0));
        assert_eq!(t.row_at(60, row_y(9)), Some(9));
        assert_eq!(t.row_at(60, 2), None);
        assert_eq!(table(3).row_at(60, row_y(5)), None);
    }

    #[test]
    fn header_click_toggles_sort() {
        let mut t = table(5);
        let now = Instant::now();
        t.mouse_down(60, 5, false, false, now);
        assert_eq!(
            t.mouse_up(60, 5),
            Some(TableEvent::Sort {
                column: 1,
                descending: false
            })
        );
        t.mouse_down(60, 5, false, false, now);
        assert_eq!(
            t.mouse_up(60, 5),
            Some(TableEvent::Sort {
                column: 1,
                descending: true
            })
        );
        t.mouse_down(90, 5, false, false, now);
        assert_eq!(
            t.mouse_up(90, 5),
            Some(TableEvent::Sort {
                column: 2,
                descending: false
            })
        );
        // Released elsewhere: no sort.
        t.mouse_down(10, 5, false, false, now);
        assert_eq!(t.mouse_up(60, 5), None);
        assert_eq!(t.sort, Some((2, false)));
    }

    #[test]
    fn column_resize_by_dragging_the_border() {
        let mut t = table(5);
        let now = Instant::now();
        assert_eq!(t.mouse_down(40, 5, false, false, now), None);
        assert_eq!(t.mouse_move(65, 5), Some(TableEvent::ColumnResized(0)));
        assert_eq!(t.columns[0].width, 65);
        assert_eq!(t.mouse_move(-100, 5), Some(TableEvent::ColumnResized(0)));
        assert_eq!(t.columns[0].width, MIN_COL_W);
        assert_eq!(t.mouse_up(-100, 5), None, "a resize never sorts");
        assert_eq!(t.sort, None);
        // Column 1 now starts at x = 1 + 16.
        assert_eq!(t.header_hit(17 + 40, 5), Some(HeaderHit::Border(1)));
    }

    #[test]
    fn resizing_with_the_next_column_keeps_the_total_width() {
        let mut t = table(5);
        t.resize_mode = ResizeMode::WithNext;
        let now = Instant::now();
        t.mouse_down(40, 5, false, false, now);
        assert_eq!(t.mouse_move(55, 5), Some(TableEvent::ColumnResized(0)));
        assert_eq!((t.columns[0].width, t.columns[1].width), (55, 25));
        assert_eq!(t.content_width(), 120);
        // The neighbour stops at its minimum width.
        t.mouse_move(500, 5);
        assert_eq!((t.columns[0].width, t.columns[1].width), (64, MIN_COL_W));
        t.mouse_move(-500, 5);
        assert_eq!((t.columns[0].width, t.columns[1].width), (MIN_COL_W, 64));
        t.mouse_up(0, 5);
        // The last column has no neighbour and resizes alone.
        let end: i32 = t.columns.iter().map(|c| c.width).sum::<i32>() + 1;
        t.mouse_down(end, 5, false, false, now);
        t.mouse_move(end + 10, 5);
        assert_eq!(t.content_width(), 120 + 10);
    }

    #[test]
    fn click_selection_with_modifiers() {
        let mut t = table(50);
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        t.mouse_down(60, row_y(2), false, false, now);
        t.mouse_up(60, row_y(2));
        t.mouse_down(60, row_y(5), false, true, later);
        t.mouse_up(60, row_y(5));
        assert_eq!(t.sel.ranges(), [(2, 6)]);
        t.mouse_down(60, row_y(8), true, false, later);
        t.mouse_up(60, row_y(8));
        assert_eq!(t.sel.ranges(), [(2, 6), (8, 9)]);
        // A plain press on a selected row keeps the selection until release (for dragging).
        assert_eq!(t.mouse_down(60, row_y(3), false, false, later), None);
        assert_eq!(t.sel.count(), 5);
        assert_eq!(t.mouse_up(60, row_y(3)), Some(TableEvent::SelectionChanged));
        assert_eq!(t.sel.ranges(), [(3, 4)]);
        // Double-click activates.
        let t2 = later + Duration::from_millis(100);
        assert_eq!(
            t.mouse_down(60, row_y(3), false, false, t2),
            Some(TableEvent::Activated(3))
        );
        // Below the last row clears.
        let mut short = table(2);
        short.mouse_down(60, row_y(0), false, false, now);
        short.mouse_up(60, row_y(0));
        assert_eq!(
            short.mouse_down(60, row_y(6), false, false, later),
            Some(TableEvent::SelectionChanged)
        );
        assert!(short.sel.is_empty());
    }

    #[test]
    fn dragging_selected_rows() {
        let mut t = table(50);
        let now = Instant::now();
        t.mouse_down(60, row_y(1), false, false, now);
        t.mouse_up(60, row_y(1));
        t.mouse_down(60, row_y(4), false, true, now + Duration::from_secs(1));
        t.mouse_up(60, row_y(4));
        t.mouse_down(60, row_y(2), false, false, now + Duration::from_secs(2));
        assert_eq!(t.mouse_move(61, row_y(2) + 2), None);
        assert_eq!(t.mouse_move(200, 400), Some(TableEvent::DragStart));
        assert!(t.is_dragging_rows());
        assert_eq!(t.mouse_move(210, 410), None);
        assert_eq!(t.mouse_up(210, 410), Some(TableEvent::Dropped));
        assert_eq!(t.sel.ranges(), [(1, 5)], "a drag keeps the selection");
    }

    #[test]
    fn scrolling_and_keys() {
        let mut t = table(100);
        assert_eq!(t.wheel(-2), Some(TableEvent::Scrolled));
        assert_eq!(t.first(), 6);
        assert_eq!(t.row_at(60, row_y(0)), Some(6));
        assert_eq!(t.wheel(10), Some(TableEvent::Scrolled));
        assert_eq!(t.wheel(1), None);
        assert_eq!(t.key(Key::Down, false), Some(TableEvent::SelectionChanged));
        assert_eq!(t.sel.ranges(), [(0, 1)]);
        t.key(Key::PageDown, false);
        assert_eq!(t.sel.cursor, Some(9));
        t.key(Key::Down, true);
        t.key(Key::Down, true);
        assert_eq!(t.sel.ranges(), [(9, 12)]);
        assert_eq!(t.first(), 2, "the cursor is scrolled into view");
        t.key(Key::End, false);
        assert_eq!((t.sel.ranges(), t.first()), (&[(99, 100)][..], 90));
        assert_eq!(t.key(Key::Enter, false), Some(TableEvent::Activated(99)));
        t.select_all();
        assert_eq!(t.sel.count(), 100);
        t.set_rows(10);
        assert_eq!((t.sel.count(), t.first()), (10, 0));
        // The scroll bar thumb drags the view.
        let mut t = table(1000);
        let thumb = t.scroll.thumb().unwrap();
        let sx = t.scroll.rect.x + 5;
        assert_eq!(
            t.mouse_down(sx, thumb.0 + 1, false, false, now()),
            Some(TableEvent::Scrolled)
        );
        assert_eq!(t.mouse_move(sx, 1000), Some(TableEvent::Scrolled));
        assert_eq!(t.first(), 990);
        t.mouse_up(sx, 1000);
    }

    #[test]
    fn wide_columns_scroll_sideways() {
        let mut t = table(100);
        assert!(!t.hscroll.scrollable());
        assert_eq!(t.body_rect().h, 140);
        // Widening a column past the body adds the horizontal bar under the rows.
        t.mouse_down(121, 5, false, false, now());
        assert_eq!(t.mouse_move(181, 5), Some(TableEvent::ColumnResized(2)));
        t.mouse_up(181, 5);
        assert_eq!(t.content_width(), 180);
        assert!(t.hscroll.scrollable());
        assert_eq!(t.body_rect().h, 140 - SCROLLBAR_W);
        assert_eq!(t.visible_rows(), 9);
        assert_eq!(t.hscroll.rect, Rect::new(1, 18 + 126, 120, SCROLLBAR_W));
        // Right scrolls 16 px: columns move left under the header.
        assert_eq!(t.key(Key::Right, false), Some(TableEvent::Scrolled));
        assert_eq!(t.header_hit(41 - 16, 5), Some(HeaderHit::Border(0)));
        assert_eq!(t.hwheel(-10), Some(TableEvent::Scrolled));
        assert_eq!(t.hscroll.first, 60);
        assert_eq!(t.key(Key::Right, false), None, "at the end");
        // The bar's arrows scroll too.
        let (lx, ly) = (t.hscroll.rect.x + 3, t.hscroll.rect.y + 3);
        assert_eq!(
            t.mouse_down(lx, ly, false, false, now()),
            Some(TableEvent::Scrolled)
        );
        t.mouse_up(lx, ly);
        assert_eq!(t.hscroll.first, 44);
    }

    #[test]
    fn right_click_selects_unselected_row() {
        let mut t = table(20);
        t.sel.click(1, false, false);
        t.sel.click(3, false, true);
        assert_eq!(
            t.right_click(60, row_y(2)),
            Some(TableEvent::ContextMenu(Some(2)))
        );
        assert_eq!(t.sel.ranges(), [(1, 4)]);
        t.right_click(60, row_y(7));
        assert_eq!(t.sel.ranges(), [(7, 8)]);
        assert_eq!(t.right_click(60, 5), None, "not on the header");
    }

    fn now() -> Instant {
        Instant::now()
    }
}
