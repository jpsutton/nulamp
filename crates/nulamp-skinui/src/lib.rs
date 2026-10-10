//! Widget kit for dialogs inside generic skinned windows, drawn with `genex.bmp` colours and
//! widgets (05 §1.5) and 1-bit text.
//!
//! A [`Form`] holds widgets at fixed positions in the client area. It draws itself into a frame
//! and turns mouse and keyboard input into [`FormEvent`]s for the owning dialog.

use nulamp_render::{Frame, blit};
use nulamp_skin::{Skin, SkinBitmap};

pub mod scrollbar;
pub mod selection;
pub mod table;
pub mod tree;

pub use scrollbar::{SCROLLBAR_W, ScrollBar, ScrollPart};
pub use selection::Selection;
pub use table::{Column, HeaderHit, Table, TableEvent};
pub use tree::{TreeEvent, TreeRow, TreeView};

/// Text measuring and drawing used by the widgets (1-bit only).
pub trait UiText {
    fn line_height(&self) -> i32;
    fn measure(&mut self, text: &str) -> i32;
    /// Draws `text` with its top at `y`.
    fn draw(
        &mut self,
        frame: &mut Frame,
        text: &str,
        x: i32,
        y: i32,
        color: u32,
        clip: (i32, i32, i32, i32),
    );
    /// Truncates with an ellipsis to fit `max` pixels.
    fn fit(&mut self, text: &str, max: i32) -> String {
        if self.measure(text) <= max {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let mut n = chars.len();
        while n > 0 {
            n -= 1;
            let s: String = chars[..n].iter().collect::<String>() + "…";
            if self.measure(&s) <= max {
                return s;
            }
        }
        String::new()
    }
}

/// [`UiText`] using nulamp's pixel font, with the FreeType 1-bit renderer as a fallback for
/// strings containing characters the pixel font does not have.
pub struct PixelText<'a> {
    pub fonts: &'a mut nulamp_text::FontSystem,
    pub bold: bool,
}

impl PixelText<'_> {
    fn font(&self) -> &'static nulamp_pixelfont::PixelFont {
        if self.bold {
            nulamp_pixelfont::PixelFont::bold()
        } else {
            nulamp_pixelfont::PixelFont::regular()
        }
    }

    fn covered(&self, text: &str) -> bool {
        let f = self.font();
        text.chars().all(|c| f.has_char(c))
    }

    fn fallback(&mut self) -> FreeTypeText<'_> {
        FreeTypeText {
            fonts: self.fonts,
            spec: nulamp_text::FontSpec::new("", 11),
        }
    }
}

impl UiText for PixelText<'_> {
    fn line_height(&self) -> i32 {
        self.font().line_height() + 1
    }

    fn measure(&mut self, text: &str) -> i32 {
        if self.covered(text) {
            self.font().measure(text)
        } else {
            self.fallback().measure(text)
        }
    }

    fn draw(
        &mut self,
        frame: &mut Frame,
        text: &str,
        x: i32,
        y: i32,
        color: u32,
        clip: (i32, i32, i32, i32),
    ) {
        if self.covered(text) {
            let f = self.font();
            f.draw(frame, text, x, y + 1 + f.ascent(), color, clip);
        } else {
            self.fallback().draw(frame, text, x, y, color, clip);
        }
    }
}

/// [`UiText`] backed by the FreeType 1-bit renderer.
pub struct FreeTypeText<'a> {
    pub fonts: &'a mut nulamp_text::FontSystem,
    pub spec: nulamp_text::FontSpec,
}

impl UiText for FreeTypeText<'_> {
    fn line_height(&self) -> i32 {
        14
    }

    fn measure(&mut self, text: &str) -> i32 {
        self.fonts.measure(text, &self.spec)
    }

    fn draw(
        &mut self,
        frame: &mut Frame,
        text: &str,
        x: i32,
        y: i32,
        color: u32,
        clip: (i32, i32, i32, i32),
    ) {
        let l = self
            .fonts
            .layout(text, &self.spec, nulamp_text::Direction::Auto);
        let m = l.metrics;
        let baseline = y + (self.line_height() - (m.ascent + m.descent)) / 2 + m.ascent;
        self.fonts.draw(frame, &l, x, baseline, color, clip);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Widget identifiers chosen by the dialog.
pub type Id = u32;

#[derive(Debug, Clone, PartialEq)]
pub enum Widget {
    Label {
        rect: Rect,
        text: String,
    },
    Button {
        rect: Rect,
        id: Id,
        label: String,
        default: bool,
    },
    Edit {
        rect: Rect,
        id: Id,
        text: String,
        cursor: usize,
        /// Shows `*` for every character (passwords).
        secret: bool,
    },
    List {
        rect: Rect,
        id: Id,
        rows: Vec<String>,
        selected: Option<usize>,
        first: usize,
    },
    Check {
        rect: Rect,
        id: Id,
        label: String,
        on: bool,
    },
    /// Read-only multi-line text with vertical scrolling.
    TextBox {
        rect: Rect,
        lines: Vec<String>,
        first: usize,
    },
}

impl Widget {
    pub fn rect(&self) -> Rect {
        match self {
            Widget::Label { rect, .. }
            | Widget::Button { rect, .. }
            | Widget::Edit { rect, .. }
            | Widget::List { rect, .. }
            | Widget::Check { rect, .. }
            | Widget::TextBox { rect, .. } => *rect,
        }
    }

    pub fn id(&self) -> Option<Id> {
        match self {
            Widget::Button { id, .. }
            | Widget::Edit { id, .. }
            | Widget::List { id, .. }
            | Widget::Check { id, .. } => Some(*id),
            _ => None,
        }
    }

    fn focusable(&self) -> bool {
        self.id().is_some()
    }
}

/// What happened in a form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormEvent {
    /// A button was clicked (or a check box toggled).
    Clicked(Id),
    /// An edit field's text changed.
    Changed(Id),
    /// A list selection moved.
    Selected(Id, usize),
    /// Enter or double-click on a list row.
    Activated(Id, usize),
    /// Enter outside a list: the default button.
    Submit,
    /// Escape.
    Cancel,
}

/// Keys the forms understand, independent of the windowing library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Left,
    Right,
    Backspace,
    Delete,
    Enter,
    Escape,
    Tab,
    ShiftTab,
}

#[derive(Debug, Clone, Default)]
pub struct Form {
    pub widgets: Vec<Widget>,
    pub focus: Option<usize>,
    pressed: Option<usize>,
    pressed_inside: bool,
    last_click: Option<(usize, usize, std::time::Instant)>,
    row_h: i32,
}

const ROW_PAD: i32 = 0;

impl Form {
    pub fn new(widgets: Vec<Widget>) -> Self {
        let focus = widgets.iter().position(Widget::focusable);
        Self {
            widgets,
            focus,
            row_h: 14,
            ..Self::default()
        }
    }

    pub fn find(&self, id: Id) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.id() == Some(id))
    }

    pub fn find_mut(&mut self, id: Id) -> Option<&mut Widget> {
        self.widgets.iter_mut().find(|w| w.id() == Some(id))
    }

    pub fn text(&self, id: Id) -> String {
        match self.find(id) {
            Some(Widget::Edit { text, .. }) => text.clone(),
            _ => String::new(),
        }
    }

    pub fn set_text(&mut self, id: Id, value: &str) {
        if let Some(Widget::Edit { text, cursor, .. }) = self.find_mut(id) {
            *text = value.to_string();
            *cursor = text.chars().count();
        }
    }

    pub fn checked(&self, id: Id) -> bool {
        matches!(self.find(id), Some(Widget::Check { on: true, .. }))
    }

    pub fn selected(&self, id: Id) -> Option<usize> {
        match self.find(id) {
            Some(Widget::List { selected, .. }) => *selected,
            _ => None,
        }
    }

    /// Replaces a list's rows and selects `selected`.
    pub fn set_rows(&mut self, id: Id, new_rows: Vec<String>, sel: Option<usize>) {
        let row_h = self.row_h;
        if let Some(Widget::List {
            rows,
            selected,
            first,
            rect,
            ..
        }) = self.find_mut(id)
        {
            *rows = new_rows;
            *selected = sel.filter(|&s| s < rows.len());
            *first = 0;
            let visible = ((rect.h - 2) / row_h).max(1) as usize;
            if let Some(s) = *selected
                && s >= visible
            {
                *first = s + 1 - visible;
            }
        }
    }

    pub fn focus_id(&mut self, id: Id) {
        self.focus = self.widgets.iter().position(|w| w.id() == Some(id));
    }

    fn list_visible(&self, rect: Rect) -> usize {
        ((rect.h - 2) / (self.row_h + ROW_PAD)).max(1) as usize
    }

    // ----- Drawing -----

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        ox: i32,
        oy: i32,
        skin: &Skin,
        text: &mut dyn UiText,
    ) {
        self.row_h = text.line_height();
        let c = &skin.genex;
        let genex = skin.bitmap(SkinBitmap::Genex);
        for (i, w) in self.widgets.iter().enumerate() {
            let r = w.rect();
            let (x, y) = (ox + r.x, oy + r.y);
            let focused = self.focus == Some(i);
            match w {
                Widget::Label { text: t, .. } => {
                    text.draw(frame, t, x, y, c.window_text, (x, y, x + r.w, y + r.h));
                }
                Widget::Button { label, default, .. } => {
                    let down = self.pressed == Some(i) && self.pressed_inside;
                    nine_slice(
                        frame,
                        genex,
                        0,
                        if down { 15 } else { 0 },
                        47,
                        15,
                        x,
                        y,
                        r.w,
                        r.h,
                        4,
                    );
                    let tw = text.measure(label);
                    let off = if down { 1 } else { 0 };
                    let ty = y + (r.h - text.line_height()) / 2 + off;
                    text.draw(
                        frame,
                        label,
                        x + (r.w - tw) / 2 + off,
                        ty,
                        c.button_text,
                        (x, y, x + r.w, y + r.h),
                    );
                    if focused || *default {
                        frame_rect(
                            frame,
                            x - 1,
                            y - 1,
                            r.w + 2,
                            r.h + 2,
                            if focused { c.window_text } else { c.divider },
                        );
                    }
                }
                Widget::Edit {
                    text: t,
                    cursor,
                    secret,
                    ..
                } => {
                    let masked;
                    let t = if *secret {
                        masked = "*".repeat(t.chars().count());
                        &masked
                    } else {
                        t
                    };
                    frame.fill_rect(x, y, r.w, r.h, c.item_bg);
                    frame_rect(
                        frame,
                        x,
                        y,
                        r.w,
                        r.h,
                        if focused { c.window_text } else { c.divider },
                    );
                    let clip = (x + 2, y + 1, x + r.w - 2, y + r.h - 1);
                    let before: String = t.chars().take(*cursor).collect();
                    let cw = text.measure(&before);
                    // Scroll so the cursor stays visible.
                    let shift = (cw - (r.w - 8)).max(0);
                    let ty = y + (r.h - text.line_height()) / 2;
                    text.draw(frame, t, x + 3 - shift, ty, c.item_fg, clip);
                    if focused {
                        frame.fill_rect(x + 3 + cw - shift, y + 3, 1, r.h - 6, c.item_fg);
                    }
                }
                Widget::List {
                    rows,
                    selected,
                    first,
                    ..
                } => {
                    frame.fill_rect(x, y, r.w, r.h, c.item_bg);
                    frame_rect(
                        frame,
                        x,
                        y,
                        r.w,
                        r.h,
                        if focused { c.window_text } else { c.divider },
                    );
                    let vis = self.list_visible(r);
                    let sb = rows.len() > vis;
                    let inner_w = r.w - 2 - if sb { 8 } else { 0 };
                    for k in 0..vis {
                        let idx = first + k;
                        let Some(row) = rows.get(idx) else { break };
                        let ry = y + 1 + k as i32 * self.row_h;
                        let sel = *selected == Some(idx);
                        let fg = if sel {
                            frame.fill_rect(x + 1, ry, inner_w, self.row_h, c.list_selection);
                            c.selbar_text_active
                        } else if idx % 2 == 1 {
                            frame.fill_rect(x + 1, ry, inner_w, self.row_h, c.alt_row_bg);
                            c.alt_row_fg
                        } else {
                            c.item_fg
                        };
                        let shown = text.fit(row, inner_w - 4);
                        text.draw(
                            frame,
                            &shown,
                            x + 3,
                            ry,
                            fg,
                            (x + 1, ry, x + 1 + inner_w, ry + self.row_h),
                        );
                    }
                    if sb {
                        let tx = x + r.w - 9;
                        frame.fill_rect(tx, y + 1, 8, r.h - 2, c.scroll_bg);
                        let track = r.h - 2;
                        let thumb_h = (track * vis as i32 / rows.len() as i32).max(8);
                        let max_first = rows.len() - vis;
                        let ty = y
                            + 1
                            + (track - thumb_h) * (*first).min(max_first) as i32
                                / max_first.max(1) as i32;
                        frame.fill_rect(tx + 1, ty, 6, thumb_h, c.scroll_fg_inverted);
                    }
                }
                Widget::Check { label, on, .. } => {
                    let by = y + (r.h - 11) / 2;
                    frame.fill_rect(x, by, 11, 11, c.item_bg);
                    frame_rect(
                        frame,
                        x,
                        by,
                        11,
                        11,
                        if focused { c.window_text } else { c.divider },
                    );
                    if *on {
                        for (dx, dy) in [
                            (2, 5),
                            (3, 6),
                            (4, 7),
                            (5, 6),
                            (6, 5),
                            (7, 4),
                            (8, 3),
                            (3, 5),
                            (4, 6),
                            (5, 5),
                            (6, 4),
                            (7, 3),
                            (8, 2),
                        ] {
                            frame.set(x + dx, by + dy, c.item_fg);
                        }
                    }
                    let ty = y + (r.h - text.line_height()) / 2;
                    text.draw(
                        frame,
                        label,
                        x + 16,
                        ty,
                        c.window_text,
                        (x, y, x + r.w, y + r.h),
                    );
                }
                Widget::TextBox { lines, first, .. } => {
                    frame.fill_rect(x, y, r.w, r.h, c.item_bg);
                    frame_rect(frame, x, y, r.w, r.h, c.divider);
                    let vis = self.list_visible(r);
                    for (k, line) in lines.iter().skip(*first).take(vis).enumerate() {
                        let ry = y + 1 + k as i32 * self.row_h;
                        text.draw(
                            frame,
                            line,
                            x + 3,
                            ry,
                            c.item_fg,
                            (x + 1, y + 1, x + r.w - 1, y + r.h - 1),
                        );
                    }
                }
            }
        }
    }

    // ----- Input -----

    fn widget_at(&self, x: i32, y: i32) -> Option<usize> {
        self.widgets.iter().position(|w| w.rect().contains(x, y))
    }

    /// Left button pressed at client coordinates.
    pub fn mouse_down(&mut self, x: i32, y: i32) -> Option<FormEvent> {
        let i = self.widget_at(x, y)?;
        if self.widgets[i].focusable() {
            self.focus = Some(i);
        }
        let row_h = self.row_h;
        let vis = self.list_visible(self.widgets[i].rect());
        match &mut self.widgets[i] {
            Widget::Button { .. } | Widget::Check { .. } => {
                self.pressed = Some(i);
                self.pressed_inside = true;
                None
            }
            Widget::List {
                rect,
                id,
                rows,
                selected,
                first,
            } => {
                let row = *first + ((y - rect.y - 1).max(0) / row_h) as usize;
                if row >= rows.len() {
                    return None;
                }
                let _ = vis;
                *selected = Some(row);
                let id = *id;
                let now = std::time::Instant::now();
                let double = matches!(self.last_click, Some((li, lr, t)) if li == i && lr == row && t.elapsed().as_millis() < 450);
                self.last_click = Some((i, row, now));
                Some(if double {
                    FormEvent::Activated(id, row)
                } else {
                    FormEvent::Selected(id, row)
                })
            }
            Widget::Edit { text, cursor, .. } => {
                *cursor = text.chars().count();
                None
            }
            _ => None,
        }
    }

    pub fn mouse_move(&mut self, x: i32, y: i32) {
        if let Some(i) = self.pressed {
            self.pressed_inside = self.widgets[i].rect().contains(x, y);
        }
    }

    pub fn mouse_up(&mut self, x: i32, y: i32) -> Option<FormEvent> {
        let i = self.pressed.take()?;
        if !self.widgets[i].rect().contains(x, y) {
            return None;
        }
        match &mut self.widgets[i] {
            Widget::Button { id, .. } => Some(FormEvent::Clicked(*id)),
            Widget::Check { id, on, .. } => {
                *on = !*on;
                Some(FormEvent::Clicked(*id))
            }
            _ => None,
        }
    }

    /// Mouse wheel over the form (positive = up).
    pub fn wheel(&mut self, x: i32, y: i32, notches: i32) {
        let Some(i) = self.widget_at(x, y) else {
            return;
        };
        let vis = self.list_visible(self.widgets[i].rect());
        match &mut self.widgets[i] {
            Widget::List { rows, first, .. } => {
                let max = rows.len().saturating_sub(vis);
                *first = (*first as i64 - notches as i64 * 3).clamp(0, max as i64) as usize;
            }
            Widget::TextBox { lines, first, .. } => {
                let max = lines.len().saturating_sub(vis);
                *first = (*first as i64 - notches as i64 * 3).clamp(0, max as i64) as usize;
            }
            _ => {}
        }
    }

    fn move_focus(&mut self, back: bool) {
        let n = self.widgets.len();
        if n == 0 {
            return;
        }
        let mut i = self.focus.unwrap_or(0);
        for _ in 0..n {
            i = if back { (i + n - 1) % n } else { (i + 1) % n };
            if self.widgets[i].focusable() {
                self.focus = Some(i);
                return;
            }
        }
    }

    /// Typed text (from the keyboard layout), inserted into the focused edit field.
    pub fn text_input(&mut self, s: &str) -> Option<FormEvent> {
        let i = self.focus?;
        if let Widget::Edit {
            id, text, cursor, ..
        } = &mut self.widgets[i]
        {
            let clean: String = s.chars().filter(|c| !c.is_control()).collect();
            if clean.is_empty() {
                return None;
            }
            let byte = text
                .char_indices()
                .nth(*cursor)
                .map(|(b, _)| b)
                .unwrap_or(text.len());
            text.insert_str(byte, &clean);
            *cursor += clean.chars().count();
            return Some(FormEvent::Changed(*id));
        }
        None
    }

    /// A navigation or editing key.
    pub fn key(&mut self, key: Key) -> Option<FormEvent> {
        match key {
            Key::Escape => return Some(FormEvent::Cancel),
            Key::Tab => {
                self.move_focus(false);
                return None;
            }
            Key::ShiftTab => {
                self.move_focus(true);
                return None;
            }
            _ => {}
        }
        let Some(i) = self.focus else {
            return (key == Key::Enter).then_some(FormEvent::Submit);
        };
        let vis = self.list_visible(self.widgets[i].rect());
        match &mut self.widgets[i] {
            Widget::Edit {
                id, text, cursor, ..
            } => {
                let len = text.chars().count();
                let id = *id;
                match key {
                    Key::Left => *cursor = cursor.saturating_sub(1),
                    Key::Right => *cursor = (*cursor + 1).min(len),
                    Key::Home => *cursor = 0,
                    Key::End => *cursor = len,
                    Key::Backspace if *cursor > 0 => {
                        let b = text
                            .char_indices()
                            .nth(*cursor - 1)
                            .map(|(b, _)| b)
                            .unwrap_or(0);
                        text.remove(b);
                        *cursor -= 1;
                        return Some(FormEvent::Changed(id));
                    }
                    Key::Delete if *cursor < len => {
                        let b = text
                            .char_indices()
                            .nth(*cursor)
                            .map(|(b, _)| b)
                            .unwrap_or(text.len());
                        text.remove(b);
                        return Some(FormEvent::Changed(id));
                    }
                    Key::Enter => return Some(FormEvent::Submit),
                    // Up/Down in an edit field move the selection of the form's first list
                    // (jump-to-file behaviour, 10 §6).
                    Key::Up | Key::Down | Key::PageUp | Key::PageDown => {
                        return self.nudge_list(key);
                    }
                    _ => {}
                }
                None
            }
            Widget::List {
                id,
                rows,
                selected,
                first,
                ..
            } => {
                let n = rows.len();
                if n == 0 {
                    return (key == Key::Enter).then_some(FormEvent::Submit);
                }
                let cur = selected.unwrap_or(0);
                let next = match key {
                    Key::Up => cur.saturating_sub(1),
                    Key::Down => (cur + 1).min(n - 1),
                    Key::PageUp => cur.saturating_sub(vis),
                    Key::PageDown => (cur + vis).min(n - 1),
                    Key::Home => 0,
                    Key::End => n - 1,
                    Key::Enter => return Some(FormEvent::Activated(*id, cur)),
                    _ => return None,
                };
                *selected = Some(next);
                if next < *first {
                    *first = next;
                } else if next >= *first + vis {
                    *first = next + 1 - vis;
                }
                Some(FormEvent::Selected(*id, next))
            }
            Widget::Button { id, .. } if key == Key::Enter => Some(FormEvent::Clicked(*id)),
            Widget::Check { id, on, .. } if key == Key::Enter => {
                *on = !*on;
                Some(FormEvent::Clicked(*id))
            }
            _ => (key == Key::Enter).then_some(FormEvent::Submit),
        }
    }

    fn nudge_list(&mut self, key: Key) -> Option<FormEvent> {
        let li = self
            .widgets
            .iter()
            .position(|w| matches!(w, Widget::List { .. }))?;
        let saved = self.focus;
        self.focus = Some(li);
        let ev = self.key(key);
        self.focus = saved;
        ev
    }
}

fn frame_rect(f: &mut Frame, x: i32, y: i32, w: i32, h: i32, c: u32) {
    f.fill_rect(x, y, w, 1, c);
    f.fill_rect(x, y + h - 1, w, 1, c);
    f.fill_rect(x, y, 1, h, c);
    f.fill_rect(x + w - 1, y, 1, h, c);
}

/// Draws a source rectangle as a 9-slice with fixed `edge`-pixel borders.
#[allow(clippy::too_many_arguments)]
fn nine_slice(
    f: &mut Frame,
    src: &Frame,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
    edge: i32,
) {
    let e = edge;
    let (mw, mh) = (sw - 2 * e, sh - 2 * e);
    let (dmw, dmh) = ((dw - 2 * e).max(0), (dh - 2 * e).max(0));
    // Corners.
    blit(f, src, sx, sy, e, e, dx, dy);
    blit(f, src, sx + sw - e, sy, e, e, dx + dw - e, dy);
    blit(f, src, sx, sy + sh - e, e, e, dx, dy + dh - e);
    blit(
        f,
        src,
        sx + sw - e,
        sy + sh - e,
        e,
        e,
        dx + dw - e,
        dy + dh - e,
    );
    // Edges and centre.
    use nulamp_render::frame::blit_stretch;
    blit_stretch(f, src, sx + e, sy, mw, e, dx + e, dy, dmw, e);
    blit_stretch(
        f,
        src,
        sx + e,
        sy + sh - e,
        mw,
        e,
        dx + e,
        dy + dh - e,
        dmw,
        e,
    );
    blit_stretch(f, src, sx, sy + e, e, mh, dx, dy + e, e, dmh);
    blit_stretch(
        f,
        src,
        sx + sw - e,
        sy + e,
        e,
        mh,
        dx + dw - e,
        dy + e,
        e,
        dmh,
    );
    blit_stretch(f, src, sx + e, sy + e, mw, mh, dx + e, dy + e, dmw, dmh);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form::new(vec![
            Widget::Edit {
                rect: Rect::new(0, 0, 100, 16),
                id: 1,
                text: String::new(),
                cursor: 0,
                secret: false,
            },
            Widget::List {
                rect: Rect::new(0, 20, 100, 44),
                id: 2,
                rows: (0..10).map(|i| format!("row {i}")).collect(),
                selected: Some(0),
                first: 0,
            },
            Widget::Button {
                rect: Rect::new(0, 70, 40, 15),
                id: 3,
                label: "OK".into(),
                default: true,
            },
            Widget::Check {
                rect: Rect::new(50, 70, 60, 15),
                id: 4,
                label: "x".into(),
                on: false,
            },
        ])
    }

    #[test]
    fn typing_and_editing() {
        let mut f = form();
        assert_eq!(f.text_input("héllo"), Some(FormEvent::Changed(1)));
        f.key(Key::Left);
        f.key(Key::Backspace);
        assert_eq!(f.text(1), "hélo");
        f.key(Key::Home);
        f.key(Key::Delete);
        assert_eq!(f.text(1), "élo");
        assert_eq!(f.key(Key::Enter), Some(FormEvent::Submit));
        assert_eq!(f.key(Key::Escape), Some(FormEvent::Cancel));
    }

    #[test]
    fn list_navigation_from_edit() {
        let mut f = form();
        assert_eq!(f.key(Key::Down), Some(FormEvent::Selected(2, 1)));
        assert_eq!(f.key(Key::PageDown), Some(FormEvent::Selected(2, 4)));
        assert_eq!(f.selected(2), Some(4));
        f.key(Key::Tab);
        assert_eq!(f.key(Key::Enter), Some(FormEvent::Activated(2, 4)));
    }

    #[test]
    fn click_button_and_check() {
        let mut f = form();
        f.mouse_down(10, 75);
        assert_eq!(f.mouse_up(10, 75), Some(FormEvent::Clicked(3)));
        f.mouse_down(60, 75);
        f.mouse_up(60, 75);
        assert!(f.checked(4));
        f.mouse_down(10, 75);
        f.mouse_move(200, 200);
        assert_eq!(f.mouse_up(200, 200), None);
        assert_eq!(
            f.mouse_down(10, 20 + 1 + 14 * 2),
            Some(FormEvent::Selected(2, 2))
        );
    }
}
