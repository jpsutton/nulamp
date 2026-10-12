//! Playlist editor: list layout and text (04 §3), info string (04 §4.1), and mouse and keyboard
//! interaction (04 §4–§8).

use nulamp_core::Command;
use nulamp_core::engine::Engine;
use nulamp_core::playlist::Playlist;
use nulamp_render::playlist_window::{PlButton, PopupMenu};
use winit::keyboard::KeyCode;

use crate::geom::hit;
use crate::input::{Action, Mods};

/// Formats `M:SS` below an hour, else `H:MM:SS` (04 §4.1).
pub fn format_total(secs: u64) -> String {
    if secs < 3600 {
        format!("{}:{:02}", secs / 60, secs % 60)
    } else {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    }
}

fn sum_part(lengths: impl Iterator<Item = Option<u64>>) -> String {
    let (mut known, mut unknown, mut any) = (0u64, false, false);
    for l in lengths {
        any = true;
        match l {
            Some(ms) => known += ms / 1000,
            None => unknown = true,
        }
    }
    if !any {
        return "0:00".into();
    }
    if known == 0 && unknown {
        return "?".into();
    }
    let mut s = format_total(known);
    if unknown && known > 0 {
        s.push('+');
    }
    s
}

/// The `SELECTED/TOTAL` info string (04 §4.1).
pub fn info_string(p: &Playlist) -> String {
    let sel = sum_part(
        p.entries()
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.length_ms),
    );
    let total = sum_part(p.entries().iter().map(|e| e.length_ms));
    format!("{sel}/{total}")
}

/// Row number prefix: right-aligned to the digit count of the list length (04 §3.2).
pub fn number_prefix(index: usize, count: usize, zero_pad: bool) -> String {
    let digits = count.max(1).to_string().len().min(5);
    if zero_pad {
        format!("{:0width$}. ", index + 1, width = digits)
    } else {
        format!("{:>width$}. ", index + 1, width = digits)
    }
}

/// Requests from the playlist window that the shell carries out.
#[derive(Debug, Clone, PartialEq)]
pub enum PlAction {
    Do(Action),
    /// A pop-up item or menu-button choice.
    Menu(PlMenuChoice),
    /// Open a standard menu at a window point.
    StandardMenu {
        menu: PopupMenu,
        x: i32,
        y: i32,
    },
    ListContextMenu {
        x: i32,
        y: i32,
    },
    MainMenu {
        x: i32,
        y: i32,
    },
    VisMenu {
        x: i32,
        y: i32,
    },
    Close,
    ToggleShade,
    /// Resize the window to this (unsnapped) size.
    Resize {
        w: i32,
        h: i32,
    },
    FontBigger,
    FontSmaller,
    FontReset,
    JumpToFile,
    FileInfoSelected,
    BookmarkSelected,
    EnqueueSelected,
    ShowInFolder,
    GenerateHtml,
    RebuildTitles,
    EditEntries,
    ScrollToPlaying,
}

/// Pop-up choices (04 §6.1), named after the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlMenuChoice {
    AddFile,
    AddFolder,
    AddUrl,
    RemoveSelected,
    Crop,
    RemoveAll,
    RemoveMisc,
    SelectAll,
    SelectNone,
    InvertSelection,
    MiscOptions,
    FileInfo,
    Sort,
    LoadList,
    SaveList,
    NewList,
}

impl PlMenuChoice {
    pub fn from_popup(menu: PopupMenu, row: usize) -> Option<Self> {
        use PlMenuChoice::*;
        Some(match (menu, row) {
            (PopupMenu::Add, 0) => AddFile,
            (PopupMenu::Add, 1) => AddFolder,
            (PopupMenu::Add, 2) => AddUrl,
            (PopupMenu::Rem, 0) => RemoveSelected,
            (PopupMenu::Rem, 1) => Crop,
            (PopupMenu::Rem, 2) => RemoveAll,
            (PopupMenu::Rem, 3) => RemoveMisc,
            (PopupMenu::Sel, 0) => SelectAll,
            (PopupMenu::Sel, 1) => SelectNone,
            (PopupMenu::Sel, 2) => InvertSelection,
            (PopupMenu::Misc, 0) => MiscOptions,
            (PopupMenu::Misc, 1) => FileInfo,
            (PopupMenu::Misc, 2) => Sort,
            (PopupMenu::List, 0) => LoadList,
            (PopupMenu::List, 1) => SaveList,
            (PopupMenu::List, 2) => NewList,
            _ => return None,
        })
    }

    /// Double-click on a bottom button (04 §6.2).
    pub fn double_click(menu: PopupMenu) -> Self {
        match menu {
            PopupMenu::Add => Self::AddFile,
            PopupMenu::Rem => Self::RemoveSelected,
            PopupMenu::Sel => Self::SelectAll,
            PopupMenu::Misc => Self::MiscOptions,
            PopupMenu::List => Self::LoadList,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Window,
    Resize {
        grab_x: i32,
        grab_y: i32,
    },
    Scroll {
        offset: i32,
    },
    /// Selecting or reordering rows; the last row the pointer was on.
    Rows {
        last: usize,
        moved: bool,
        row: usize,
    },
    Button(PlButton),
    /// Old-school pop-up: press, drag to an item, release.
    PopupHold,
}

/// Playlist window state and interaction.
#[derive(Debug, Clone)]
pub struct PlaylistUi {
    pub w: i32,
    pub h: i32,
    pub shade: bool,
    /// Height to restore when unshading.
    pub unshaded_h: i32,
    pub first: usize,
    pub row_h: i32,
    pub font_px: u32,
    pub page_lines: usize,
    pub wheel_double: bool,
    pub old_school_buttons: bool,
    pub show_numbers: bool,
    pub zero_pad: bool,
    /// Draw rows with the skin's `text.bmp` font (docs/decisions.md D15).
    pub bitmap_font: bool,
    /// Integer glyph scale for the bitmap font (Ctrl+Numpad +/−).
    pub bitmap_scale: i32,
    pub pressed: Option<PlButton>,
    pub thumb_pressed: bool,
    pub popup: Option<(PopupMenu, Option<usize>)>,
    drag: Option<Drag>,
    /// Pointer y while reordering, for auto-scroll.
    pointer_y: i32,
}

impl Default for PlaylistUi {
    fn default() -> Self {
        Self {
            w: 275,
            h: 116,
            shade: false,
            unshaded_h: 116,
            first: 0,
            row_h: 13,
            font_px: 12,
            page_lines: 1,
            wheel_double: true,
            old_school_buttons: false,
            show_numbers: true,
            zero_pad: false,
            bitmap_font: true,
            bitmap_scale: 1,
            pressed: None,
            thumb_pressed: false,
            popup: None,
            drag: None,
            pointer_y: 0,
        }
    }
}

const BUTTONS: [(PopupMenu, i32); 4] = [
    (PopupMenu::Add, 14),
    (PopupMenu::Rem, 43),
    (PopupMenu::Sel, 72),
    (PopupMenu::Misc, 101),
];

impl PlaylistUi {
    /// Number of whole rows that fit (04 §3.1).
    pub fn visible_rows(&self) -> usize {
        ((self.h - 38 - 20 - 2) / self.row_h.max(1)).max(0) as usize
    }

    pub fn track_height(&self) -> i32 {
        self.h - 58
    }

    /// Scroll thumb offset in pixels (04 §5).
    pub fn thumb_offset(&self, count: usize) -> i32 {
        let vis = self.visible_rows();
        if count <= vis {
            return 0;
        }
        let max_first = count - vis;
        (self.track_height() - 18) * self.first.min(max_first) as i32 / max_first as i32
    }

    pub fn clamp_scroll(&mut self, count: usize) {
        let vis = self.visible_rows();
        self.first = self.first.min(count.saturating_sub(vis));
    }

    pub fn scroll_by(&mut self, delta: i64, count: usize) {
        self.first = (self.first as i64 + delta).max(0) as usize;
        self.clamp_scroll(count);
    }

    /// Scrolls so `row` is visible.
    pub fn ensure_visible(&mut self, row: usize, count: usize) {
        let vis = self.visible_rows().max(1);
        if row < self.first {
            self.first = row;
        } else if row >= self.first + vis {
            self.first = row + 1 - vis;
        }
        self.clamp_scroll(count);
    }

    fn row_at(&self, y: i32) -> usize {
        ((y - 22).max(0) / self.row_h.max(1)) as usize + self.first
    }

    fn button_at(&self, x: i32, y: i32) -> Option<PopupMenu> {
        if !(y >= self.h - 30 && y <= self.h - 12) {
            return None;
        }
        for (menu, bx) in BUTTONS {
            if x >= bx && x <= bx + 21 {
                return Some(menu);
            }
        }
        (x >= self.w - 44 && x <= self.w - 22).then_some(PopupMenu::List)
    }

    fn in_list(&self, x: i32, y: i32) -> bool {
        x >= 12 && x < self.w - 20 && y >= 20 && y < self.h - 38
    }

    fn in_resize(&self, x: i32, y: i32) -> bool {
        if self.shade {
            return x >= self.w - 29 && x <= self.w - 20;
        }
        x >= self.w - 20 && y >= self.h - 20 && (self.w - x) + (self.h - y) <= 30
    }

    /// Mini transport button index (04 §4.2) for a point.
    fn mini_transport(&self, x: i32, y: i32) -> Option<usize> {
        let t = self.w - 144;
        if !(y >= self.h - 15 && y <= self.h - 8 && x >= t && x <= self.w - 91) {
            return None;
        }
        let r = x - t;
        Some(match r {
            0..=6 => 0,
            7..=16 => 1,
            17..=26 => 2,
            27..=35 => 3,
            36..=44 => 4,
            _ => 5,
        })
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Left button pressed.
    pub fn mouse_down(&mut self, x: i32, y: i32, mods: Mods, engine: &mut Engine) -> Vec<PlAction> {
        // An open (click-to-choose) pop-up takes the next click.
        if let Some((menu, _)) = self.popup {
            self.popup = None;
            if let Some(row) = menu.row_at(self.w, self.h, x, y) {
                return PlMenuChoice::from_popup(menu, row)
                    .map(PlAction::Menu)
                    .into_iter()
                    .collect();
            }
            return vec![];
        }
        if self.shade {
            if hit(x, y, self.w - 20, self.w - 12, 3, 12) {
                self.press(PlButton::Shade);
                return vec![];
            }
            if hit(x, y, self.w - 11, self.w - 3, 3, 12) {
                self.press(PlButton::Close);
                return vec![];
            }
            if self.in_resize(x, y) {
                self.drag = Some(Drag::Resize {
                    grab_x: self.w - x,
                    grab_y: 0,
                });
                return vec![];
            }
            self.drag = Some(Drag::Window);
            return vec![PlAction::Do(Action::BeginWindowDrag { x, y })];
        }
        if hit(x, y, self.w - 21, self.w - 13, 3, 12) {
            self.press(PlButton::Shade);
            return vec![];
        }
        if hit(x, y, self.w - 11, self.w - 3, 3, 12) {
            self.press(PlButton::Close);
            return vec![];
        }
        if self.in_resize(x, y) {
            self.drag = Some(Drag::Resize {
                grab_x: self.w - x,
                grab_y: self.h - y,
            });
            return vec![];
        }
        let count = engine.playlist.len();
        if x >= self.w - 15 && x < self.w - 7 && y >= 20 && y < self.h - 38 {
            let thumb = 20 + self.thumb_offset(count);
            let offset = if y >= thumb && y < thumb + 18 {
                y - thumb
            } else {
                9
            };
            self.drag = Some(Drag::Scroll { offset });
            self.thumb_pressed = true;
            self.scroll_drag(y, offset, count);
            return vec![];
        }
        if let Some(menu) = self.button_at(x, y) {
            self.popup = Some((menu, menu.row_at(self.w, self.h, x, y)));
            if self.old_school_buttons {
                self.drag = Some(Drag::PopupHold);
            }
            return vec![];
        }
        if let Some(b) = self.mini_transport(x, y) {
            return mini_transport_action(b, mods);
        }
        if hit(x, y, self.w - 87, self.w - 53, self.h - 18, self.h - 9) {
            return vec![PlAction::Do(Action::ToggleTimeMode)];
        }
        if hit(x, y, self.w - 15, self.w - 7, self.h - 36, self.h - 32) {
            self.scroll_by(-(self.page_lines as i64), count);
            return vec![];
        }
        if hit(x, y, self.w - 15, self.w - 7, self.h - 31, self.h - 27) {
            self.scroll_by(self.page_lines as i64, count);
            return vec![];
        }
        if self.in_list(x, y) {
            let row = self.row_at(y);
            if row < count {
                select_on_down(&mut engine.playlist, row, mods);
            } else if !mods.ctrl {
                engine.playlist.select_all(false);
            }
            self.drag = Some(Drag::Rows {
                last: row,
                moved: false,
                row,
            });
            self.pointer_y = y;
            return vec![];
        }
        if y < 14 || y < 20 {
            self.drag = Some(Drag::Window);
            return vec![PlAction::Do(Action::BeginWindowDrag { x, y })];
        }
        self.drag = Some(Drag::Window);
        vec![PlAction::Do(Action::BeginWindowDrag { x, y })]
    }

    fn press(&mut self, b: PlButton) {
        self.drag = Some(Drag::Button(b));
        self.pressed = Some(b);
    }

    fn scroll_drag(&mut self, y: i32, offset: i32, count: usize) {
        let vis = self.visible_rows();
        if count <= vis {
            self.first = 0;
            return;
        }
        let span = (self.h - 76).max(1);
        let first = (y - offset - 20) as i64 * (count - vis) as i64 / span as i64;
        self.first = first.clamp(0, (count - vis) as i64) as usize;
    }

    pub fn mouse_move(&mut self, x: i32, y: i32, mods: Mods, engine: &mut Engine) -> Vec<PlAction> {
        if let Some((menu, hi)) = &mut self.popup {
            *hi = menu.row_at(self.w, self.h, x, y);
            return vec![];
        }
        match self.drag {
            Some(Drag::Resize { grab_x, grab_y }) => {
                let w = x + grab_x;
                let h = if self.shade { 14 } else { y + grab_y };
                vec![PlAction::Resize { w, h }]
            }
            Some(Drag::Scroll { offset }) => {
                self.scroll_drag(y, offset, engine.playlist.len());
                vec![]
            }
            Some(Drag::Button(b)) => {
                let inside = match b {
                    PlButton::Shade => hit(x, y, self.w - 21, self.w - 12, 3, 12),
                    PlButton::Close => hit(x, y, self.w - 11, self.w - 3, 3, 12),
                };
                self.pressed = inside.then_some(b);
                vec![]
            }
            Some(Drag::Rows { last, moved, row }) => {
                self.pointer_y = y;
                let count = engine.playlist.len();
                let cur = self.row_at(y).min(count.saturating_sub(1));
                let held_selected = engine.playlist.get(row).is_some_and(|e| e.selected);
                if held_selected && !mods.shift && cur != last {
                    // Drag to reorder (04 §7.2): one step per row.
                    let up = cur < last;
                    let steps = cur.abs_diff(last);
                    let mut moved_now = false;
                    for _ in 0..steps {
                        if engine.playlist.move_selected(up) {
                            moved_now = true;
                        }
                    }
                    let new_last = if moved_now { cur } else { last };
                    self.drag = Some(Drag::Rows {
                        last: new_last,
                        moved: moved || moved_now,
                        row: new_last,
                    });
                    if y < 10 + 20 {
                        self.scroll_by(-1, count);
                    } else if y > self.h - 28 - 10 {
                        self.scroll_by(1, count);
                    }
                }
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn mouse_up(&mut self, x: i32, y: i32, mods: Mods, engine: &mut Engine) -> Vec<PlAction> {
        let drag = self.drag.take();
        let pressed = self.pressed.take();
        self.thumb_pressed = false;
        match drag {
            Some(Drag::PopupHold) => {
                let Some((menu, _)) = self.popup.take() else {
                    return vec![];
                };
                let row = menu
                    .row_at(self.w, self.h, x, y)
                    .or_else(|| self.button_at(x, y).map(|_| 0));
                row.and_then(|r| PlMenuChoice::from_popup(menu, r))
                    .map(PlAction::Menu)
                    .into_iter()
                    .collect()
            }
            Some(Drag::Button(b)) if pressed == Some(b) => match b {
                PlButton::Close => vec![PlAction::Close],
                PlButton::Shade => vec![PlAction::ToggleShade],
            },
            Some(Drag::Rows {
                moved: false, row, ..
            }) if !mods.shift && !mods.ctrl => {
                // Clicking inside a multi-selection collapses it (04 §7.1).
                if engine.playlist.get(row).is_some_and(|e| e.selected) {
                    engine.playlist.select_all(false);
                    if let Some(e) = engine.playlist.get_mut(row) {
                        e.selected = true;
                    }
                }
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn double_click(&mut self, x: i32, y: i32, engine: &mut Engine) -> Option<Vec<PlAction>> {
        if y <= 14 && x < self.w - 20 {
            return Some(vec![PlAction::ToggleShade]);
        }
        if self.shade {
            return None;
        }
        if let Some(menu) = self.button_at(x, y) {
            self.popup = None;
            return Some(vec![PlAction::Menu(PlMenuChoice::double_click(menu))]);
        }
        if self.in_list(x, y) {
            let row = self.row_at(y);
            if row < engine.playlist.len() {
                self.drag = None;
                return Some(vec![PlAction::Do(Action::Player(Command::PlayIndex(row)))]);
            }
        }
        None
    }

    pub fn right_click(
        &mut self,
        x: i32,
        y: i32,
        mods: Mods,
        engine: &mut Engine,
    ) -> Vec<PlAction> {
        if self.drag.is_some() {
            return vec![];
        }
        self.popup = None;
        if !self.shade {
            if let Some(menu) = self.button_at(x, y) {
                return vec![PlAction::StandardMenu { menu, x, y }];
            }
            if self.in_list(x, y) {
                let row = self.row_at(y);
                if row < engine.playlist.len() {
                    select_on_down(&mut engine.playlist, row, mods);
                }
                return vec![PlAction::ListContextMenu { x, y }];
            }
            if hit(x, y, self.w - 225, self.w - 150, self.h - 26, self.h - 8) && self.w >= 350 {
                return vec![PlAction::VisMenu { x, y }];
            }
        }
        vec![PlAction::MainMenu { x, y }]
    }

    /// Mouse wheel (04 §7.3). `notches` positive = up.
    pub fn wheel(&mut self, notches: i32, middle_held: bool, engine: &mut Engine) -> Vec<PlAction> {
        if self.shade {
            let c = if notches > 0 {
                Command::Previous
            } else {
                Command::Next
            };
            return vec![PlAction::Do(Action::Player(c))];
        }
        let count = engine.playlist.len();
        if middle_held {
            for _ in 0..notches.unsigned_abs() {
                engine.playlist.move_selected(notches > 0);
            }
            return vec![];
        }
        let per = self.page_lines as i64 * if self.wheel_double { 2 } else { 1 };
        self.scroll_by(-(notches as i64) * per, count);
        vec![]
    }

    /// Playlist keys (04 §8). Returns `None` when the key is not a playlist key.
    pub fn key(&mut self, code: KeyCode, m: Mods, engine: &mut Engine) -> Option<Vec<PlAction>> {
        use KeyCode::*;
        let count = engine.playlist.len();
        let p = &mut engine.playlist;
        let none: Vec<PlAction> = vec![];
        let cursor = p.first_selected().unwrap_or(0);
        let r = match (code, m.ctrl, m.alt, m.shift) {
            (ArrowUp | ArrowDown, false, false, _) if self.shade => {
                let c = if code == ArrowUp {
                    Command::Previous
                } else {
                    Command::Next
                };
                vec![PlAction::Do(Action::Player(c))]
            }
            (ArrowUp | ArrowDown, false, false, shift) => {
                if count == 0 {
                    return Some(none);
                }
                let cur = if code == ArrowUp {
                    p.anchor
                        .map(|a| a.saturating_sub(1))
                        .unwrap_or(cursor.saturating_sub(1))
                } else {
                    p.anchor.map(|a| (a + 1).min(count - 1)).unwrap_or(cursor)
                };
                if shift {
                    p.select_range(cursor, cur);
                } else {
                    p.select_all(false);
                    if let Some(e) = p.get_mut(cur) {
                        e.selected = true;
                    }
                }
                p.anchor = Some(cur);
                self.ensure_visible(cur, count);
                none
            }
            (ArrowUp, false, true, false) => {
                p.move_selected(true);
                none
            }
            (ArrowDown, false, true, false) => {
                p.move_selected(false);
                none
            }
            (PageUp, false, false, _) => {
                self.scroll_by(-(self.page_lines as i64), count);
                none
            }
            (PageDown, false, false, _) => {
                self.scroll_by(self.page_lines as i64, count);
                none
            }
            (Home, false, false, _) => {
                self.first = 0;
                none
            }
            (End, false, false, _) => {
                self.first = count;
                self.clamp_scroll(count);
                none
            }
            (Space, false, false, false) => {
                let cur = p.current_index();
                p.select_all(false);
                if let Some(e) = p.get_mut(cur) {
                    e.selected = true;
                }
                self.ensure_visible(cur, count);
                none
            }
            (Enter | NumpadEnter, false, false, false) => match p.first_selected() {
                Some(i) => vec![PlAction::Do(Action::Player(Command::PlayIndex(i)))],
                None => none,
            },
            (Enter, true, false, false) => vec![PlAction::FontReset],
            (Delete, false, false, false) => {
                engine.remove_where(|_, e| e.selected);
                none
            }
            (Delete, true, false, false) => {
                if engine.playlist.selected_count() > 0 {
                    engine.remove_where(|_, e| !e.selected);
                }
                none
            }
            (Delete, true, false, true) | (KeyN, true, false, false) => {
                engine.clear();
                none
            }
            (Delete, false, true, false) => {
                engine.remove_where(|_, e| {
                    !nulamp_core::playlist::is_url(&e.location)
                        && std::fs::File::open(&e.location).is_err()
                });
                none
            }
            (KeyA, true, false, false) => {
                p.select_all(true);
                none
            }
            (KeyI, true, false, false) => {
                p.invert_selection();
                none
            }
            (KeyR, true, false, false) => {
                p.reverse();
                none
            }
            (KeyR, true, false, true) => {
                p.randomize(&mut rand::rng());
                none
            }
            (Digit1, true, false, true) => {
                p.sort_by_title();
                none
            }
            (Digit2, true, false, true) => {
                p.sort_by_file_name();
                none
            }
            (Digit3, true, false, true) => {
                p.sort_by_path();
                none
            }
            (NumpadAdd, true, false, false) => vec![PlAction::FontBigger],
            (NumpadSubtract, true, false, false) => vec![PlAction::FontSmaller],
            (KeyL, false, false, false)
            | (Numpad0, false, false, false)
            | (Insert, true, false, false) => {
                vec![PlAction::Menu(PlMenuChoice::AddFile)]
            }
            (KeyL, false, false, true) | (Insert, false, false, false) => {
                vec![PlAction::Menu(PlMenuChoice::AddFolder)]
            }
            (KeyL, true, false, false)
            | (Numpad0, true, false, false)
            | (Insert, false, false, true) => {
                vec![PlAction::Menu(PlMenuChoice::AddUrl)]
            }
            (Digit3, false, true, false) => vec![PlAction::FileInfoSelected],
            (KeyI, false, true, false) => vec![PlAction::BookmarkSelected],
            (KeyQ, false, false, false) => vec![PlAction::EnqueueSelected],
            (KeyF, true, false, false) => vec![PlAction::ShowInFolder],
            (KeyG, true, true, false) => vec![PlAction::GenerateHtml],
            (KeyE, true, true, false) => vec![PlAction::RebuildTitles],
            (KeyE, true, false, false) => vec![PlAction::EditEntries],
            (F4, true, false, false) => vec![PlAction::Close],
            (Numpad8, false, false, false) => {
                vec![PlAction::Do(Action::Player(Command::VolumeBy(4)))]
            }
            (Numpad2, false, false, false) => {
                vec![PlAction::Do(Action::Player(Command::VolumeBy(-4)))]
            }
            _ => return None,
        };
        self.clamp_scroll(engine.playlist.len());
        Some(r)
    }
}

/// Selection on mouse-down in the list (04 §7.1).
fn select_on_down(p: &mut Playlist, row: usize, mods: Mods) {
    let selected = p.get(row).is_some_and(|e| e.selected);
    if mods.shift
        && let Some(anchor) = p.anchor
    {
        if !(mods.ctrl && selected) {
            if !mods.ctrl {
                p.select_all(false);
            }
            p.select_range(anchor, row);
        }
        return;
    }
    if !mods.ctrl && !selected {
        p.select_all(false);
    }
    if let Some(e) = p.get_mut(row) {
        e.selected = if mods.ctrl { !selected } else { true };
    }
    p.anchor = Some(row);
}

fn mini_transport_action(button: usize, mods: Mods) -> Vec<PlAction> {
    use crate::main_input::transport_action;
    use nulamp_render::main_window::MainButton::*;
    let b = [Previous, Play, Pause, Stop, Next, Eject][button.min(5)];
    transport_action(b, mods, true)
        .map(PlAction::Do)
        .into_iter()
        .collect()
}

/// Row height of the bitmap font at a scale: 6-pixel glyphs plus a 2-pixel gap.
pub fn bitmap_row_height(scale: i32) -> i32 {
    8 * scale.max(1)
}

/// Draws the list rows into a playlist frame (04 §3).
pub fn draw_rows(
    frame: &mut nulamp_render::Frame,
    fonts: &mut nulamp_text::FontSystem,
    skin: &nulamp_skin::Skin,
    spec: &nulamp_text::FontSpec,
    ui: &PlaylistUi,
    engine: &Engine,
) {
    use nulamp_render::playlist_window::draw_text_bmp_colored;
    use nulamp_skin::fonts::text_bmp_string;
    use nulamp_text::Direction;
    let p = &engine.playlist;
    let colors = &skin.pledit;
    let (x0, x1) = (12, ui.w - 20);
    let bottom = ui.h - 38;
    let metrics = fonts.metrics(spec);
    let current = if p.current_removed {
        None
    } else {
        p.current_id()
    };
    let s = ui.bitmap_scale.max(1);
    for i in 0..ui.visible_rows() + 1 {
        let index = ui.first + i;
        let Some(e) = p.get(index) else { break };
        let top = 22 + i as i32 * ui.row_h;
        if top >= bottom {
            break;
        }
        let row_bottom = (top + ui.row_h).min(bottom);
        let bg = if e.selected {
            colors.selected_bg
        } else {
            colors.normal_bg
        };
        frame.fill_rect(x0, top, x1 - x0, row_bottom - top, bg);
        let fg = if Some(e.id) == current {
            colors.current
        } else {
            colors.normal
        };
        let clip = (x0, top, x1, row_bottom);
        let duration = e
            .length_ms
            .map(|ms| format!(" {} ", crate::marquee::format_length(ms / 1000)));
        let mut text = String::new();
        if ui.show_numbers {
            text.push_str(&number_prefix(index, p.len(), ui.zero_pad));
        }
        if let Some(q) = engine.queue.iter().position(|id| *id == e.id) {
            text.push_str(&format!("[{}] ", q + 1));
        }
        text.push_str(&e.title);
        if ui.bitmap_font {
            let gy = top + (ui.row_h - 6 * s) / 2;
            let mut right = x1;
            if let Some(d) = &duration {
                let w = d.chars().count() as i32 * 5 * s;
                if w < (x1 - x0) / 2 {
                    right = x1 - w;
                    draw_text_bmp_colored(frame, skin, d, right, gy, fg, s, clip);
                }
            }
            let max_chars = ((right - x0) / (5 * s)).max(0) as usize;
            let mut chars: Vec<char> = text_bmp_string(&text).chars().collect();
            if chars.len() > max_chars {
                chars.truncate(max_chars);
                if let Some(last) = chars.last_mut() {
                    *last = '…';
                }
            }
            let t: String = chars.into_iter().collect();
            draw_text_bmp_colored(frame, skin, &t, x0, gy, fg, s, (x0, top, right, row_bottom));
            continue;
        }
        let baseline = top + (ui.row_h - (metrics.ascent + metrics.descent)) / 2 + metrics.ascent;
        let mut right = x1;
        if let Some(d) = &duration {
            let l = fonts.layout(d, spec, Direction::Ltr);
            if l.width < (x1 - x0) / 2 {
                right = x1 - l.width;
                fonts.draw(frame, &l, right, baseline, fg, clip);
            }
        }
        let l = fonts.layout_fit(&text, spec, Direction::Auto, right - x0);
        fonts.draw(frame, &l, x0, baseline, fg, (x0, top, right, row_bottom));
    }
}
