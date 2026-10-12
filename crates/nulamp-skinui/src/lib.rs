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
    /// A radio button: choosing it turns off the other radio buttons of the same `group`.
    Radio {
        rect: Rect,
        id: Id,
        group: Id,
        label: String,
        on: bool,
    },
    /// A horizontal track with a thumb at one of the steps `0..=max`.
    Slider {
        rect: Rect,
        id: Id,
        value: u32,
        max: u32,
    },
    /// A labelled frame around other widgets. It takes no input.
    Group {
        rect: Rect,
        label: String,
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
            | Widget::TextBox { rect, .. }
            | Widget::Radio { rect, .. }
            | Widget::Slider { rect, .. }
            | Widget::Group { rect, .. } => *rect,
        }
    }

    pub fn id(&self) -> Option<Id> {
        match self {
            Widget::Button { id, .. }
            | Widget::Edit { id, .. }
            | Widget::List { id, .. }
            | Widget::Check { id, .. }
            | Widget::Radio { id, .. }
            | Widget::Slider { id, .. } => Some(*id),
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
    /// A button was clicked (or a check box toggled, or a radio button chosen).
    Clicked(Id),
    /// An edit field's text or a slider's value changed.
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
    /// Widgets that are greyed out and ignore input.
    disabled: std::collections::HashSet<Id>,
    tooltips: std::collections::HashMap<Id, String>,
    /// Widget under the pointer whose tooltip is shown.
    tip_hover: Option<usize>,
}

const ROW_PAD: i32 = 0;
/// Widest tooltip box, in pixels.
const TIP_MAX_W: i32 = 240;
/// Space between a tooltip's border and its text.
const TIP_PAD: i32 = 2;

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

    /// Whether a check box or radio button is on.
    pub fn checked(&self, id: Id) -> bool {
        matches!(
            self.find(id),
            Some(Widget::Check { on: true, .. } | Widget::Radio { on: true, .. })
        )
    }

    /// The radio button that is on in `group`.
    pub fn radio(&self, group: Id) -> Option<Id> {
        self.widgets.iter().find_map(|w| match w {
            Widget::Radio {
                id,
                group: g,
                on: true,
                ..
            } if *g == group => Some(*id),
            _ => None,
        })
    }

    /// A slider's value.
    pub fn value(&self, id: Id) -> Option<u32> {
        match self.find(id) {
            Some(Widget::Slider { value, .. }) => Some(*value),
            _ => None,
        }
    }

    /// Turns radio button `id` on and the others of its group off.
    fn choose_radio(&mut self, id: Id) {
        let Some(Widget::Radio { group, .. }) = self.find(id) else {
            return;
        };
        let group = *group;
        for w in &mut self.widgets {
            if let Widget::Radio {
                id: i,
                group: g,
                on,
                ..
            } = w
                && *g == group
            {
                *on = *i == id;
            }
        }
    }

    /// Moves slider `i` to the step under client x `px`. Returns true when the value changed.
    fn slide_to(&mut self, i: usize, px: i32) -> bool {
        let Widget::Slider {
            rect, value, max, ..
        } = &mut self.widgets[i]
        else {
            return false;
        };
        let (x0, span) = slider_track(*rect);
        let v = if *max == 0 || span <= 0 {
            0
        } else {
            let t = (px - x0).clamp(0, span);
            ((t * *max as i32 + span / 2) / span) as u32
        };
        let changed = v != *value;
        *value = v;
        changed
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
        if self.is_enabled(id) {
            self.focus = self.widgets.iter().position(|w| w.id() == Some(id));
        }
    }

    /// Enables or disables a widget. Disabled widgets are drawn greyed, ignore the mouse and the
    /// keyboard and are skipped by Tab.
    pub fn set_enabled(&mut self, id: Id, enabled: bool) {
        if enabled {
            self.disabled.remove(&id);
            return;
        }
        self.disabled.insert(id);
        let focused = self.focus.is_some_and(|i| self.widgets[i].id() == Some(id));
        if focused {
            self.move_focus(false);
            if self.focus.is_some_and(|i| !self.can_focus(i)) {
                self.focus = None;
            }
        }
    }

    pub fn is_enabled(&self, id: Id) -> bool {
        !self.disabled.contains(&id)
    }

    /// Text shown in a box while the pointer is over the widget (also when it is disabled).
    pub fn set_tooltip(&mut self, id: Id, tip: impl Into<String>) {
        self.tooltips.insert(id, tip.into());
    }

    /// The tooltip currently shown, if the pointer is over a widget that has one.
    pub fn hovered_tooltip(&self) -> Option<&str> {
        let id = self.widgets.get(self.tip_hover?)?.id()?;
        self.tooltips.get(&id).map(String::as_str)
    }

    fn widget_enabled(&self, i: usize) -> bool {
        self.widgets[i].id().is_none_or(|id| self.is_enabled(id))
    }

    fn can_focus(&self, i: usize) -> bool {
        self.widgets[i].focusable() && self.widget_enabled(i)
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
            let enabled = w.id().is_none_or(|id| !self.disabled.contains(&id));
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
                        if enabled { c.button_text } else { c.divider },
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
                    let fg = if enabled { c.item_fg } else { c.divider };
                    text.draw(frame, t, x + 3 - shift, ty, fg, clip);
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
                        let fg = if enabled { fg } else { c.divider };
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
                    // Disabled check boxes are greyed like disabled menu items.
                    let (mark, label_fg) = if enabled {
                        (c.item_fg, c.window_text)
                    } else {
                        (c.divider, c.divider)
                    };
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
                            frame.set(x + dx, by + dy, mark);
                        }
                    }
                    let ty = y + (r.h - text.line_height()) / 2;
                    text.draw(frame, label, x + 16, ty, label_fg, (x, y, x + r.w, y + r.h));
                }
                Widget::Radio { label, on, .. } => {
                    let (mark, label_fg) = if enabled {
                        (c.item_fg, c.window_text)
                    } else {
                        (c.divider, c.divider)
                    };
                    let by = y + (r.h - 11) / 2;
                    let edge = if focused { c.window_text } else { c.divider };
                    // An 11×11 circle outline; the inside is filled like a check box.
                    const CIRCLE: [&str; 11] = [
                        "....###....",
                        "..##...##..",
                        ".#.......#.",
                        ".#.......#.",
                        "#.........#",
                        "#.........#",
                        "#.........#",
                        ".#.......#.",
                        ".#.......#.",
                        "..##...##..",
                        "....###....",
                    ];
                    for (dy, row) in CIRCLE.iter().enumerate() {
                        let yy = by + dy as i32;
                        let l = row.find('#').unwrap_or(0) as i32;
                        let rr = row.rfind('#').unwrap_or(0) as i32;
                        frame.fill_rect(x + l, yy, rr - l + 1, 1, c.item_bg);
                        for (dx, ch) in row.bytes().enumerate() {
                            if ch == b'#' {
                                frame.set(x + dx as i32, yy, edge);
                            }
                        }
                    }
                    if *on {
                        frame.fill_rect(x + 4, by + 3, 3, 5, mark);
                        frame.fill_rect(x + 3, by + 4, 5, 3, mark);
                    }
                    let ty = y + (r.h - text.line_height()) / 2;
                    text.draw(frame, label, x + 16, ty, label_fg, (x, y, x + r.w, y + r.h));
                }
                Widget::Slider { value, max, .. } => {
                    let fg = if enabled { c.item_fg } else { c.divider };
                    let edge = if focused { c.window_text } else { c.divider };
                    let (x0, span) = slider_track(r);
                    let (x0, ty) = (ox + x0, y + (r.h - 4) / 2 - 2);
                    frame.fill_rect(x0 - 2, ty, span + 5, 4, c.item_bg);
                    frame_rect(frame, x0 - 2, ty, span + 5, 4, c.divider);
                    if *max > 0 {
                        for k in 0..=*max {
                            let tx = x0 + span * k as i32 / *max as i32;
                            frame.fill_rect(tx, y + r.h - 2, 1, 2, c.divider);
                        }
                    }
                    let tx = x0
                        + if *max > 0 {
                            span * (*value).min(*max) as i32 / *max as i32
                        } else {
                            0
                        };
                    let th = r.h - 4;
                    frame.fill_rect(tx - 3, y, 7, th, c.item_bg);
                    frame_rect(frame, tx - 3, y, 7, th, edge);
                    frame.fill_rect(tx, y + 2, 1, th - 4, fg);
                }
                Widget::Group { label, .. } => {
                    let lh = text.line_height();
                    let fy = y + lh / 2;
                    frame_rect(frame, x, fy, r.w, r.h - lh / 2, c.divider);
                    let tw = text.measure(label);
                    frame.fill_rect(x + 6, fy, tw + 4, 1, c.window_bg);
                    text.draw(
                        frame,
                        label,
                        x + 8,
                        y,
                        c.window_text,
                        (x, y, x + r.w, y + lh),
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
        if let Some(tip) = self.hovered_tooltip() {
            let r = self.widgets[self.tip_hover.expect("hovered")].rect();
            let anchor = Rect::new(ox + r.x, oy + r.y, r.w, r.h);
            // Keep the box inside the area the widgets cover (and the frame).
            let area = self
                .widgets
                .iter()
                .map(Widget::rect)
                .reduce(|a, b| {
                    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
                    let (x1, y1) = ((a.x + a.w).max(b.x + b.w), (a.y + a.h).max(b.y + b.h));
                    Rect::new(x, y, x1 - x, y1 - y)
                })
                .unwrap_or(r);
            let (x0, y0) = ((ox + area.x).max(0), (oy + area.y).max(0));
            let x1 = (ox + area.x + area.w).min(frame.width() as i32);
            let y1 = (oy + area.y + area.h).min(frame.height() as i32);
            let bounds = Rect::new(x0, y0, x1 - x0, y1 - y0);
            draw_tooltip(frame, skin, text, tip, anchor, bounds);
        }
    }

    // ----- Input -----

    fn widget_at(&self, x: i32, y: i32) -> Option<usize> {
        self.widgets
            .iter()
            .position(|w| !matches!(w, Widget::Group { .. }) && w.rect().contains(x, y))
    }

    /// Left button pressed at client coordinates.
    pub fn mouse_down(&mut self, x: i32, y: i32) -> Option<FormEvent> {
        let i = self.widget_at(x, y)?;
        if !self.widget_enabled(i) {
            return None;
        }
        if self.widgets[i].focusable() {
            self.focus = Some(i);
        }
        let row_h = self.row_h;
        let vis = self.list_visible(self.widgets[i].rect());
        match &mut self.widgets[i] {
            Widget::Button { .. } | Widget::Check { .. } | Widget::Radio { .. } => {
                self.pressed = Some(i);
                self.pressed_inside = true;
                None
            }
            Widget::Slider { id, .. } => {
                let id = *id;
                self.pressed = Some(i);
                self.pressed_inside = true;
                self.slide_to(i, x).then_some(FormEvent::Changed(id))
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

    /// Pointer moved to client coordinates. Returns true when the form needs redrawing (a
    /// pressed button changed state, or a tooltip appeared or went away).
    pub fn mouse_move(&mut self, x: i32, y: i32) -> bool {
        let mut changed = false;
        if let Some(i) = self.pressed {
            if matches!(self.widgets[i], Widget::Slider { .. }) {
                // A dragged thumb follows the pointer anywhere; mouse_up reports the change.
                changed = self.slide_to(i, x);
            } else {
                let inside = self.widgets[i].rect().contains(x, y);
                changed = inside != self.pressed_inside;
                self.pressed_inside = inside;
            }
        }
        let hover = self.widget_at(x, y).filter(|&i| {
            self.widgets[i]
                .id()
                .is_some_and(|id| self.tooltips.contains_key(&id))
        });
        if hover != self.tip_hover {
            self.tip_hover = hover;
            changed = true;
        }
        changed
    }

    /// Pointer left the form's window. Returns true when a tooltip was hidden.
    pub fn mouse_leave(&mut self) -> bool {
        self.tip_hover.take().is_some()
    }

    pub fn mouse_up(&mut self, x: i32, y: i32) -> Option<FormEvent> {
        let i = self.pressed.take()?;
        if let Widget::Slider { id, .. } = self.widgets[i] {
            // Dragging moved the thumb without events; report where it ended.
            self.slide_to(i, x);
            return Some(FormEvent::Changed(id));
        }
        if !self.widgets[i].rect().contains(x, y) || !self.widget_enabled(i) {
            return None;
        }
        match &mut self.widgets[i] {
            Widget::Button { id, .. } => Some(FormEvent::Clicked(*id)),
            Widget::Check { id, on, .. } => {
                *on = !*on;
                Some(FormEvent::Clicked(*id))
            }
            Widget::Radio { id, .. } => {
                let id = *id;
                self.choose_radio(id);
                Some(FormEvent::Clicked(id))
            }
            _ => None,
        }
    }

    /// Mouse wheel over the form (positive = up).
    pub fn wheel(&mut self, x: i32, y: i32, notches: i32) {
        let Some(i) = self.widget_at(x, y).filter(|&i| self.widget_enabled(i)) else {
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
            if self.can_focus(i) {
                self.focus = Some(i);
                return;
            }
        }
    }

    /// Typed text (from the keyboard layout), inserted into the focused edit field.
    pub fn text_input(&mut self, s: &str) -> Option<FormEvent> {
        let i = self.focus.filter(|&i| self.widget_enabled(i))?;
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
        let Some(i) = self.focus.filter(|&i| self.widget_enabled(i)) else {
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
            Widget::Radio { group, .. } => {
                let group = *group;
                let step: i32 = match key {
                    Key::Enter => 0,
                    Key::Up | Key::Left => -1,
                    Key::Down | Key::Right => 1,
                    _ => return None,
                };
                // Arrows move to the neighbouring radio button of the group and choose it.
                let members: Vec<usize> = (0..self.widgets.len())
                    .filter(|&k| {
                        matches!(self.widgets[k], Widget::Radio { group: g, .. } if g == group)
                            && self.widget_enabled(k)
                    })
                    .collect();
                let at = members.iter().position(|&k| k == i).unwrap_or(0) as i32;
                let to = members[(at + step).clamp(0, members.len() as i32 - 1) as usize];
                if step != 0 && to == i {
                    return None;
                }
                let to_id = self.widgets[to].id().expect("radio id");
                self.focus = Some(to);
                self.choose_radio(to_id);
                Some(FormEvent::Clicked(to_id))
            }
            Widget::Slider { id, value, max, .. } => {
                let id = *id;
                let v = match key {
                    Key::Left | Key::Down => value.saturating_sub(1),
                    Key::Right | Key::Up => (*value + 1).min(*max),
                    Key::Home => 0,
                    Key::End => *max,
                    Key::Enter => return Some(FormEvent::Submit),
                    _ => return None,
                };
                let changed = v != *value;
                *value = v;
                changed.then_some(FormEvent::Changed(id))
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

/// Splits `s` into lines no wider than `max` pixels, breaking at spaces. A word wider than `max`
/// gets a line of its own.
pub fn wrap_text(text: &mut dyn UiText, s: &str, max: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if line.is_empty() {
            line = word.to_string();
            continue;
        }
        let joined = format!("{line} {word}");
        if text.measure(&joined) <= max {
            line = joined;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Draws a bordered tooltip box with `tip` below `anchor` (above it when there is no room below),
/// kept inside `bounds`. The text is word-wrapped; what does not fit in `bounds` is cut off with an
/// ellipsis.
pub fn draw_tooltip(
    frame: &mut Frame,
    skin: &Skin,
    text: &mut dyn UiText,
    tip: &str,
    anchor: Rect,
    bounds: Rect,
) {
    let c = &skin.genex;
    let lh = text.line_height();
    let edge = 1 + TIP_PAD;
    let inner_max = bounds.w.min(TIP_MAX_W) - 2 * edge - 1;
    let max_lines = (bounds.h - 2 * edge) / lh;
    if inner_max <= 0 || max_lines <= 0 {
        return;
    }
    let mut lines = wrap_text(text, tip, inner_max);
    if lines.is_empty() {
        return;
    }
    let max_lines = max_lines as usize;
    if lines.len() > max_lines {
        let rest = lines[max_lines - 1..].join(" ");
        lines.truncate(max_lines - 1);
        lines.push(rest);
    }
    for l in &mut lines {
        *l = text.fit(l, inner_max);
    }
    let inner_w = lines.iter().map(|l| text.measure(l)).max().unwrap_or(0);
    // One extra pixel on the right: glyphs can reach past their measured advance.
    let w = inner_w + 2 * edge + 1;
    let h = lines.len() as i32 * lh + 2 * edge;
    let (right, bottom) = (bounds.x + bounds.w, bounds.y + bounds.h);
    let x = anchor.x.min(right - w).max(bounds.x);
    let below = anchor.y + anchor.h + 1;
    let above = anchor.y - 1 - h;
    let y = if below + h <= bottom {
        below
    } else if above >= bounds.y {
        above
    } else {
        (bottom - h).max(bounds.y)
    };
    frame.fill_rect(x, y, w, h, c.item_bg);
    frame_rect(frame, x, y, w, h, c.divider);
    let clip = (x + 1, y + 1, x + w - 1, y + h - 1);
    for (k, line) in lines.iter().enumerate() {
        text.draw(
            frame,
            line,
            x + edge,
            y + edge + k as i32 * lh,
            c.item_fg,
            clip,
        );
    }
}

/// A slider's track as (first step x, width from first to last step), in the slider's
/// coordinates. The thumb is 7 pixels wide, so the track keeps 3 pixels clear at each end.
fn slider_track(r: Rect) -> (i32, i32) {
    (r.x + 3, (r.w - 7).max(0))
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

    /// Fixed-width text that records what was drawn.
    #[derive(Default)]
    struct FakeText(Vec<String>);

    impl UiText for FakeText {
        fn line_height(&self) -> i32 {
            12
        }
        fn measure(&mut self, text: &str) -> i32 {
            text.chars().count() as i32 * 5
        }
        fn draw(&mut self, _: &mut Frame, text: &str, _: i32, _: i32, _: u32, _: Clip) {
            self.0.push(text.to_string());
        }
    }

    type Clip = (i32, i32, i32, i32);

    #[test]
    fn disabled_check_ignores_input() {
        let mut f = form();
        f.set_enabled(4, false);
        assert!(!f.is_enabled(4));
        assert_eq!(f.mouse_down(60, 75), None);
        assert_eq!(f.mouse_up(60, 75), None);
        assert!(!f.checked(4));
        // Tab skips it: edit → list → button → edit.
        f.key(Key::Tab);
        f.key(Key::Tab);
        f.key(Key::Tab);
        assert_eq!(f.focus, Some(0));
        // A disabled widget cannot be focused, so Enter does not toggle it.
        f.focus_id(4);
        assert_eq!(f.focus, Some(0));
        f.focus = Some(3);
        assert_eq!(f.key(Key::Enter), Some(FormEvent::Submit));
        assert!(!f.checked(4));
        // Disabling the focused widget moves the focus on.
        f.focus_id(3);
        f.set_enabled(3, false);
        assert_eq!(f.focus, Some(0));
        f.set_enabled(4, true);
        f.mouse_down(60, 75);
        assert_eq!(f.mouse_up(60, 75), Some(FormEvent::Clicked(4)));
        assert!(f.checked(4));
    }

    fn choices() -> Form {
        let radio = |id, x, on| Widget::Radio {
            rect: Rect::new(x, 0, 40, 16),
            id,
            group: 10,
            label: "r".into(),
            on,
        };
        Form::new(vec![
            Widget::Group {
                rect: Rect::new(0, 0, 200, 60),
                label: "g".into(),
            },
            radio(11, 0, true),
            radio(12, 50, false),
            radio(13, 100, false),
            Widget::Slider {
                rect: Rect::new(0, 30, 47, 16),
                id: 20,
                value: 1,
                max: 4,
            },
        ])
    }

    #[test]
    fn radio_group_has_one_choice() {
        let mut f = choices();
        assert_eq!(f.radio(10), Some(11));
        // A click inside the group frame lands on the radio button, not the frame.
        f.mouse_down(60, 8);
        assert_eq!(f.mouse_up(60, 8), Some(FormEvent::Clicked(12)));
        assert_eq!(f.radio(10), Some(12));
        assert!(f.checked(12) && !f.checked(11));
        // Arrows move along the group and choose; the ends stop.
        assert_eq!(f.key(Key::Right), Some(FormEvent::Clicked(13)));
        assert_eq!(f.key(Key::Right), None);
        assert_eq!(f.radio(10), Some(13));
        f.key(Key::Up);
        assert_eq!(f.key(Key::Left), Some(FormEvent::Clicked(11)));
        assert_eq!(f.radio(10), Some(11));
        // A disabled member is skipped.
        f.set_enabled(12, false);
        f.focus_id(11);
        assert_eq!(f.key(Key::Down), Some(FormEvent::Clicked(13)));
    }

    #[test]
    fn slider_steps() {
        let mut f = choices();
        // Track: steps 0..=4 at x 3, 13, 23, 33, 43.
        assert_eq!(f.mouse_down(33, 36), Some(FormEvent::Changed(20)));
        assert_eq!(f.value(20), Some(3));
        // Dragging follows the pointer outside the slider; release reports it.
        assert!(f.mouse_move(-40, 90));
        assert_eq!(f.value(20), Some(0));
        assert_eq!(f.mouse_up(-40, 90), Some(FormEvent::Changed(20)));
        assert_eq!(f.key(Key::Left), None);
        assert_eq!(f.key(Key::Right), Some(FormEvent::Changed(20)));
        assert_eq!(f.key(Key::End), Some(FormEvent::Changed(20)));
        assert_eq!(f.value(20), Some(4));
        assert_eq!(f.key(Key::Right), None);
        // Nearest step: x 17 is closer to step 1 (13) than step 2 (23).
        f.mouse_down(17, 36);
        f.mouse_up(17, 36);
        assert_eq!(f.value(20), Some(1));
    }

    #[test]
    fn tooltip_only_while_hovered() {
        let skin = nulamp_skin::builtin::skin();
        let mut frame = Frame::new(200, 200, 0);
        let tip = "Not available here because of reasons that take more than one line";
        let mut f = form();
        f.set_enabled(4, false);
        f.set_tooltip(4, tip);
        let drawn = |f: &mut Form, frame: &mut Frame| {
            let mut t = FakeText::default();
            f.draw(frame, 0, 0, &skin, &mut t);
            t.0
        };
        let base = drawn(&mut f, &mut frame);
        assert!(!f.mouse_move(10, 75), "no tooltip over the button");
        assert_eq!(f.hovered_tooltip(), None);
        assert!(
            f.mouse_move(60, 75),
            "entering the check box shows its tooltip"
        );
        assert!(!f.mouse_move(61, 76), "moving inside it needs no redraw");
        assert_eq!(f.hovered_tooltip(), Some(tip));
        let shown = drawn(&mut f, &mut frame);
        let extra: Vec<&String> = shown[base.len()..].iter().collect();
        assert!(extra.len() > 1, "wrapped: {extra:?}");
        assert!(
            extra
                .iter()
                .all(|l| l.chars().count() as i32 * 5 <= 110 - 6)
        );
        assert_eq!(
            extra
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            tip
        );
        assert!(f.mouse_move(10, 75));
        assert_eq!(drawn(&mut f, &mut frame), base);
        f.mouse_move(60, 75);
        assert!(f.mouse_leave());
        assert_eq!(f.hovered_tooltip(), None);
    }
}
