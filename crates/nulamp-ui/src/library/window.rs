//! The media-library window: a resizable generic skinned window (05 §1) holding a search
//! box, the provider tree, an artist and an album pane above the track list, Play/Enqueue
//! buttons and a status line. Input is turned into changes of the [`LibraryView`] and into
//! [`LibAction`]s for the shell.

use std::time::Instant;

use nulamp_library::Sort;
use nulamp_render::Frame;
use nulamp_render::gen_window::{self, GenView};
use nulamp_skin::Skin;
use nulamp_skinui::{
    Column, Form, FormEvent, Key, PixelText, Rect, Table, TableEvent, TreeEvent, TreeView, UiText,
    Widget,
};
use nulamp_text::FontSystem;
use winit::keyboard::KeyCode;

use super::service::Purpose;
use super::view::{self, COLUMNS, LibraryView, NodeKey};
use crate::input::Mods;
use crate::shell::{ActivateAction, LibraryPrefs};

pub const TITLE: &str = "Media Library";
/// Default size: 625×406 (the 25×29 steps of the generic window).
pub const DEFAULT_SIZE: (i32, i32) = (625, 406);
pub const DEFAULT_TREE_W: i32 = 130;
pub const MIN_TREE_W: i32 = 60;
pub const DEFAULT_PANE_H: i32 = 96;
const MIN_PANE_H: i32 = 40;
const MIN_PANE_W: i32 = 60;
/// Least height of the track list below the panes.
const MIN_TABLE_H: i32 = 40;
/// Client-area origin inside the generic frame (05 §1.1).
const CLIENT_X: i32 = 11;
const CLIENT_Y: i32 = 20;
const SPLIT: i32 = 5;
const SEARCH: u32 = 1;
const CLEAR: u32 = 2;
const PLAY: u32 = 3;
const ENQUEUE: u32 = 4;
const DROP: u32 = 5;
/// Index of the search field in the form.
const EDIT_INDEX: usize = 1;
/// Width of the "Clear Search" button.
const CLEAR_W: i32 = 76;

/// Which part of the window has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Search,
    Tree,
    Artists,
    Albums,
    List,
}

const ZONES: [Zone; 5] = [
    Zone::Search,
    Zone::Tree,
    Zone::Artists,
    Zone::Albums,
    Zone::List,
];

/// What the window asks the shell to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibAction {
    Close,
    /// Start moving the window from this window point.
    BeginMove {
        x: i32,
        y: i32,
    },
    /// Resize to this (unsnapped) size.
    Resize {
        w: i32,
        h: i32,
    },
    /// Act on the selected rows.
    Selection(Purpose),
    /// Act on every row of the listing.
    All(Purpose),
    /// Replace the playlist with every row of the listing and play from this row.
    PlayAllFrom(usize),
    /// Track-list menu at a window point.
    ContextMenu {
        x: i32,
        y: i32,
    },
    /// Tree menu at a window point.
    TreeMenu {
        x: i32,
        y: i32,
    },
    /// The menu of the Play button's arrow, below this window point.
    PlayMenu {
        x: i32,
        y: i32,
    },
    /// Dragged rows were released at this window point (may be outside the window).
    DropTracks {
        x: i32,
        y: i32,
    },
    Redraw,
}

/// Sizes the user sets by dragging splitters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub tree_w: i32,
    pub pane_h: i32,
    /// Width of the artist pane; 0 splits the right side in half.
    pub artist_w: i32,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            tree_w: DEFAULT_TREE_W,
            pane_h: DEFAULT_PANE_H,
            artist_w: 0,
        }
    }
}

/// Client-area rectangles of the window's parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub label: Rect,
    pub search: Rect,
    pub clear: Rect,
    pub tree: Rect,
    /// Between the tree and everything right of it.
    pub splitter: Rect,
    pub artists: Rect,
    /// Between the artist and the album pane.
    pub pane_split: Rect,
    pub albums: Rect,
    /// Between the panes and the track list.
    pub hsplit: Rect,
    pub table: Rect,
    pub play: Rect,
    /// The Play button's arrow (grouped buttons) or the Enqueue button.
    pub extra: Rect,
    pub status: Rect,
}

fn clamp_tree_w(tree_w: i32, cw: i32) -> i32 {
    tree_w.clamp(MIN_TREE_W, (cw - 120).max(MIN_TREE_W))
}

/// Lays out a window of `w`×`h`.
pub fn layout(w: i32, h: i32, g: Geometry, grouped: bool) -> Layout {
    let (cw, ch) = (w - 19, h - 34);
    let tree_w = clamp_tree_w(g.tree_w, cw);
    let top = 24;
    let body_h = (ch - 24 - top).max(20);
    let (rx, rw) = (4 + tree_w + SPLIT, cw - 8 - tree_w - SPLIT);
    let max_pane = (body_h - SPLIT - MIN_TABLE_H.min(body_h / 2)).max(0);
    let pane_h = g.pane_h.clamp(MIN_PANE_H.min(max_pane), max_pane);
    let aw = if g.artist_w > 0 {
        g.artist_w
    } else {
        (rw - SPLIT) / 2
    };
    let aw = aw.clamp(
        MIN_PANE_W.min(rw / 2),
        (rw - SPLIT - MIN_PANE_W).max(MIN_PANE_W.min(rw / 2)),
    );
    let table_y = top + pane_h + SPLIT;
    let (play_w, extra) = if grouped {
        (44, Rect::new(48, ch - 21, 16, 18))
    } else {
        (48, Rect::new(56, ch - 21, 60, 18))
    };
    let status_x = extra.x + extra.w + 6;
    Layout {
        label: Rect::new(4, 5, 42, 14),
        search: Rect::new(48, 2, cw - 52 - CLEAR_W - 4, 18),
        clear: Rect::new(cw - 4 - CLEAR_W, 2, CLEAR_W, 18),
        tree: Rect::new(4, top, tree_w, body_h),
        splitter: Rect::new(4 + tree_w, top, SPLIT, body_h),
        artists: Rect::new(rx, top, aw, pane_h),
        pane_split: Rect::new(rx + aw, top, SPLIT, pane_h),
        albums: Rect::new(rx + aw + SPLIT, top, rw - aw - SPLIT, pane_h),
        hsplit: Rect::new(rx, top + pane_h, rw, SPLIT),
        table: Rect::new(rx, table_y, rw, body_h - pane_h - SPLIT),
        play: Rect::new(4, ch - 21, play_w, 18),
        extra,
        status: Rect::new(status_x, ch - 18, cw - status_x - 4, 14),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Resize {
        grab_x: i32,
        grab_y: i32,
    },
    /// Tree | right side.
    Splitter {
        grab: i32,
    },
    /// Panes / track list.
    PaneH {
        grab: i32,
    },
    /// Artist | album pane.
    PaneV {
        grab: i32,
    },
    Close,
    Button,
    Tree,
    Artists,
    Albums,
    Table,
}

fn set_rect(w: &mut Widget, r: Rect) {
    match w {
        Widget::Label { rect, .. }
        | Widget::Button { rect, .. }
        | Widget::Edit { rect, .. }
        | Widget::List { rect, .. }
        | Widget::Check { rect, .. }
        | Widget::TextBox { rect, .. } => *rect = r,
    }
}

fn edit_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        _ => return None,
    })
}

fn nav_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Enter | KeyCode::NumpadEnter => Key::Enter,
        _ => return None,
    })
}

fn build_form(l: &Layout, grouped: bool, search: &str) -> Form {
    let button = |rect, id, label: &str| Widget::Button {
        rect,
        id,
        label: label.into(),
        default: false,
    };
    let mut widgets = vec![
        Widget::Label {
            rect: l.label,
            text: "Search:".into(),
        },
        Widget::Edit {
            rect: l.search,
            id: SEARCH,
            text: search.to_string(),
            cursor: search.chars().count(),
            secret: false,
        },
        button(l.clear, CLEAR, "Clear Search"),
        button(l.play, PLAY, "Play"),
    ];
    widgets.push(if grouped {
        button(l.extra, DROP, "")
    } else {
        button(l.extra, ENQUEUE, "Enqueue")
    });
    Form::new(widgets)
}

/// Fits the first column of a pane to the width left by the other two.
fn fit_pane(t: &mut Table) {
    let others: i32 = t.columns.iter().skip(1).map(|c| c.width).sum();
    let avail = t.rect.w - 2 - nulamp_skinui::SCROLLBAR_W;
    t.columns[0].width = (avail - others).max(nulamp_skinui::table::MIN_COL_W);
}

fn pane(cols: [(&str, i32, bool); 3], rect: Rect) -> Table {
    let mut t = Table::new(
        rect,
        cols.iter()
            .map(|&(name, w, right)| Column::new(name, w, right))
            .collect(),
    );
    fit_pane(&mut t);
    t
}

pub struct LibraryWindow {
    pub w: i32,
    pub h: i32,
    pub geometry: Geometry,
    pub focus: Zone,
    pub form: Form,
    pub tree: TreeView,
    pub artists: Table,
    pub albums: Table,
    pub table: Table,
    tree_keys: Vec<NodeKey>,
    drag: Option<Drag>,
    pub close_pressed: bool,
    pub prefs: LibraryPrefs,
}

impl LibraryWindow {
    /// A window of the given size (snapped), splitter positions and track-list column
    /// widths (defaults for a missing or mismatched list).
    pub fn new(size: (i32, i32), geometry: Geometry, widths: &[i32]) -> Self {
        let (w, h) = gen_window::snap_size(size.0, size.1);
        let prefs = LibraryPrefs::default();
        let l = layout(w, h, geometry, prefs.group_buttons);
        let columns = COLUMNS
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let width = match widths.get(i) {
                    Some(&w) if widths.len() == COLUMNS.len() => {
                        w.max(nulamp_skinui::table::MIN_COL_W)
                    }
                    _ => c.width,
                };
                Column::new(c.title, width, c.right)
            })
            .collect();
        let mut win = Self {
            w,
            h,
            geometry,
            focus: Zone::List,
            form: build_form(&l, prefs.group_buttons, ""),
            tree: TreeView::new(l.tree),
            artists: pane(
                [
                    ("Artist", 100, false),
                    ("Albums", 48, true),
                    ("Tracks", 48, true),
                ],
                l.artists,
            ),
            albums: pane(
                [
                    ("Album", 100, false),
                    ("Year", 40, true),
                    ("Tracks", 48, true),
                ],
                l.albums,
            ),
            table: Table::new(l.table, columns),
            tree_keys: Vec::new(),
            drag: None,
            close_pressed: false,
            prefs,
        };
        win.apply_prefs(&LibraryPrefs::default());
        win
    }

    /// Takes over the options of the preferences dialog.
    pub fn apply_prefs(&mut self, p: &LibraryPrefs) {
        let regroup = self.prefs.group_buttons != p.group_buttons;
        self.prefs = p.clone();
        for t in [&mut self.artists, &mut self.albums, &mut self.table] {
            t.alt_rows = p.alt_rows;
            t.resize_mode = p.resize_mode;
        }
        self.tree.icons = p.tree_icons;
        self.tree.full_row = p.tree_full_row;
        self.tree.item_height = p.tree_item_height;
        if regroup {
            let text = self.search_text();
            self.form = build_form(&self.layout(), p.group_buttons, &text);
        }
        self.relayout();
    }

    pub fn layout(&self) -> Layout {
        layout(self.w, self.h, self.geometry, self.prefs.group_buttons)
    }

    /// Applies a new size (snapped to 25×29 steps, 05 §1.4). Returns whether it changed.
    pub fn set_size(&mut self, w: i32, h: i32) -> bool {
        let (w, h) = gen_window::snap_size(w, h);
        if (w, h) == (self.w, self.h) {
            return false;
        }
        self.w = w;
        self.h = h;
        self.relayout();
        true
    }

    fn relayout(&mut self) {
        let l = self.layout();
        self.geometry.tree_w = l.tree.w;
        self.geometry.pane_h = l.artists.h;
        if self.geometry.artist_w > 0 {
            self.geometry.artist_w = l.artists.w;
        }
        for (widget, r) in self
            .form
            .widgets
            .iter_mut()
            .zip([l.label, l.search, l.clear, l.play, l.extra])
        {
            set_rect(widget, r);
        }
        self.tree.set_rect(l.tree);
        self.artists.set_rect(l.artists);
        self.albums.set_rect(l.albums);
        fit_pane(&mut self.artists);
        fit_pane(&mut self.albums);
        self.table.set_rect(l.table);
    }

    pub fn column_widths(&self) -> Vec<i32> {
        self.table.columns.iter().map(|c| c.width).collect()
    }

    /// The text of the search box.
    pub fn search_text(&self) -> String {
        self.form.text(SEARCH)
    }

    /// True while rows are dragged out of the list.
    pub fn dragging_tracks(&self) -> bool {
        self.table.is_dragging_rows()
    }

    /// Close button (05 §1.4: x w−10 … w−1, y 3–12).
    pub fn in_close(&self, x: i32, y: i32) -> bool {
        x >= self.w - 10 && x < self.w && (3..=12).contains(&y)
    }

    /// Resize grip: the bottom-right triangle (05 §1.4).
    pub fn in_grip(&self, x: i32, y: i32) -> bool {
        x >= self.w - 20 && y >= self.h - 20 && (self.w - x) + (self.h - y) <= 30
    }

    /// Brings the widgets up to date with the view and asks for the visible rows.
    pub fn sync(&mut self, view: &mut LibraryView) {
        if view.tree_dirty {
            let (rows, keys, sel) = view.tree_rows();
            self.tree.set_rows(rows, sel);
            if let Some(s) = sel {
                self.tree.scroll.ensure_visible(s);
            }
            self.tree_keys = keys;
            view.tree_dirty = false;
        }
        if view.panes_dirty {
            self.artists.set_rows(view.artists.len() + 1);
            self.artists.sel.set_only(view.artist_row());
            self.artists.sort = Some(view.artist_sort);
            self.albums.set_rows(view.albums.len() + 1);
            self.albums.sel.set_only(view.album_row());
            self.albums.sort = Some(view.album_sort);
            view.panes_dirty = false;
        }
        if view.list_reset {
            self.table.sel.clear();
            self.table.scroll.first = 0;
            view.list_reset = false;
        }
        let total = view.total.unwrap_or(0);
        if self.table.rows != total {
            self.table.set_rows(total);
        }
        self.table.sort = view
            .sort()
            .and_then(|s| view::column_for_sort(s.key).map(|c| (c, s.descending)));
        view.want_rows(self.table.first(), self.table.visible_rows());
    }

    fn tree_event(&mut self, ev: TreeEvent, view: &mut LibraryView) {
        let key = |i: usize| self.tree_keys.get(i).cloned();
        match ev {
            TreeEvent::Selected(i) => {
                if let Some(k) = key(i) {
                    view.select(&k);
                }
            }
            TreeEvent::Expand(i) => {
                if let Some(k) = key(i) {
                    view.expand(&k);
                }
            }
            TreeEvent::Collapse(i) => {
                if let Some(k) = key(i) {
                    view.collapse(&k);
                }
            }
            TreeEvent::Activated(i) => {
                if let Some(k) = key(i) {
                    view.toggle(&k);
                }
            }
        }
        self.sync(view);
    }

    /// What double-click or Enter does with a listing: `row` is the activated row.
    fn activate(&self, row: usize, shift: bool, view: &LibraryView) -> Vec<LibAction> {
        let playlist = view
            .spec
            .as_ref()
            .is_some_and(|s| s.node.kind == nulamp_library::NodeKind::Playlist);
        let all = if playlist {
            self.prefs.play_all_playlists
        } else {
            self.prefs.play_all_media
        };
        vec![match (self.prefs.activate, shift) {
            (_, true) | (ActivateAction::Enqueue, _) => LibAction::Selection(Purpose::Enqueue),
            (ActivateAction::PlayNext, _) => LibAction::Selection(Purpose::PlayNext),
            (ActivateAction::Play, _) if all => LibAction::PlayAllFrom(row),
            (ActivateAction::Play, _) => LibAction::Selection(Purpose::Play),
        }]
    }

    /// The Play/Enqueue buttons act on the selection, or on everything without one.
    fn auto(&self, p: Purpose) -> LibAction {
        if self.table.sel.is_empty() {
            LibAction::All(p)
        } else {
            LibAction::Selection(p)
        }
    }

    /// What a pane's double-click or Enter does: act on the listing it filters.
    fn pane_activate(&self, shift: bool) -> Vec<LibAction> {
        let p = match (self.prefs.activate, shift) {
            (_, true) | (ActivateAction::Enqueue, _) => Purpose::Enqueue,
            (ActivateAction::PlayNext, _) => Purpose::PlayNext,
            (ActivateAction::Play, _) => Purpose::Play,
        };
        vec![LibAction::All(p)]
    }

    fn table_event(
        &mut self,
        ev: TableEvent,
        view: &mut LibraryView,
        shift: bool,
        x: i32,
        y: i32,
    ) -> Vec<LibAction> {
        let out = match ev {
            TableEvent::Sort { column, descending } => {
                view.set_sort(Sort {
                    key: COLUMNS[column].sort,
                    descending,
                });
                vec![]
            }
            TableEvent::Activated(row) => self.activate(row, shift, view),
            TableEvent::Dropped => vec![LibAction::DropTracks { x, y }],
            TableEvent::ContextMenu(_) => vec![LibAction::ContextMenu { x, y }],
            _ => vec![],
        };
        self.sync(view);
        out
    }

    /// An event of the artist (`artists`) or album pane.
    fn pane_event(
        &mut self,
        artists: bool,
        ev: TableEvent,
        view: &mut LibraryView,
        shift: bool,
    ) -> Vec<LibAction> {
        let t = if artists {
            &mut self.artists
        } else {
            &mut self.albums
        };
        let mut out = vec![];
        match ev {
            TableEvent::SelectionChanged | TableEvent::ContextMenu(_) => {
                if let Some(row) = t.sel.first() {
                    if artists {
                        view.select_artist(view.artist_at(row));
                    } else {
                        view.select_album(view.album_at(row));
                    }
                }
            }
            TableEvent::Sort { column, descending } => {
                if artists {
                    view.set_artist_sort(column, descending);
                } else {
                    view.set_album_sort(column, descending);
                }
            }
            TableEvent::Activated(_) => out = self.pane_activate(shift),
            _ => {}
        }
        // A click below the rows must not leave the pane without a selection.
        view.panes_dirty = true;
        self.sync(view);
        out
    }

    /// Left button pressed at a window point.
    pub fn mouse_down(
        &mut self,
        x: i32,
        y: i32,
        mods: Mods,
        view: &mut LibraryView,
        now: Instant,
    ) -> Vec<LibAction> {
        if self.in_close(x, y) {
            self.close_pressed = true;
            self.drag = Some(Drag::Close);
            return vec![LibAction::Redraw];
        }
        if y < 14 {
            return vec![LibAction::BeginMove { x, y }];
        }
        if self.in_grip(x, y) {
            self.drag = Some(Drag::Resize {
                grab_x: self.w - x,
                grab_y: self.h - y,
            });
            return vec![];
        }
        let (cx, cy) = (x - CLIENT_X, y - CLIENT_Y);
        let (cw, ch) = (self.w - 19, self.h - 34);
        if !Rect::new(0, 0, cw, ch).contains(cx, cy) {
            return vec![LibAction::BeginMove { x, y }];
        }
        let l = self.layout();
        if l.splitter.contains(cx, cy) {
            self.drag = Some(Drag::Splitter {
                grab: cx - self.geometry.tree_w,
            });
            return vec![];
        }
        if l.hsplit.contains(cx, cy) {
            self.drag = Some(Drag::PaneH {
                grab: cy - l.artists.h - l.artists.y,
            });
            return vec![];
        }
        if l.pane_split.contains(cx, cy) {
            self.drag = Some(Drag::PaneV {
                grab: cx - l.artists.x - l.artists.w,
            });
            return vec![];
        }
        if l.search.contains(cx, cy) {
            self.focus = Zone::Search;
            self.form.focus_id(SEARCH);
            self.form.mouse_down(cx, cy);
            return vec![LibAction::Redraw];
        }
        if [l.clear, l.play, l.extra]
            .iter()
            .any(|r| r.contains(cx, cy))
        {
            self.form.mouse_down(cx, cy);
            self.drag = Some(Drag::Button);
            return vec![LibAction::Redraw];
        }
        if l.tree.contains(cx, cy) {
            self.focus = Zone::Tree;
            self.drag = Some(Drag::Tree);
            if let Some(ev) = self.tree.mouse_down(cx, cy, now) {
                self.tree_event(ev, view);
            }
            return vec![LibAction::Redraw];
        }
        for (artists, rect) in [(true, l.artists), (false, l.albums)] {
            if !rect.contains(cx, cy) {
                continue;
            }
            self.focus = if artists { Zone::Artists } else { Zone::Albums };
            self.drag = Some(if artists { Drag::Artists } else { Drag::Albums });
            let t = if artists {
                &mut self.artists
            } else {
                &mut self.albums
            };
            let ev = t.mouse_down(cx, cy, false, false, now);
            let mut out = match ev {
                Some(ev) => self.pane_event(artists, ev, view, mods.shift),
                None => vec![],
            };
            out.push(LibAction::Redraw);
            return out;
        }
        if l.table.contains(cx, cy) {
            self.focus = Zone::List;
            self.drag = Some(Drag::Table);
            let mut out = match self.table.mouse_down(cx, cy, mods.ctrl, mods.shift, now) {
                Some(ev) => self.table_event(ev, view, mods.shift, x, y),
                None => vec![],
            };
            out.push(LibAction::Redraw);
            return out;
        }
        vec![]
    }

    /// Pointer moved to a window point.
    pub fn mouse_move(&mut self, x: i32, y: i32, view: &mut LibraryView) -> Vec<LibAction> {
        let (cx, cy) = (x - CLIENT_X, y - CLIENT_Y);
        match self.drag {
            Some(Drag::Resize { grab_x, grab_y }) => vec![LibAction::Resize {
                w: x + grab_x,
                h: y + grab_y,
            }],
            Some(Drag::Splitter { grab }) => {
                let before = self.geometry.tree_w;
                self.geometry.tree_w = cx - grab;
                self.relayout();
                if self.geometry.tree_w != before {
                    self.sync(view);
                    vec![LibAction::Redraw]
                } else {
                    vec![]
                }
            }
            Some(Drag::PaneH { grab }) => {
                let before = self.geometry.pane_h;
                self.geometry.pane_h = cy - grab - self.artists.rect.y;
                self.relayout();
                if self.geometry.pane_h != before {
                    self.sync(view);
                    vec![LibAction::Redraw]
                } else {
                    vec![]
                }
            }
            Some(Drag::PaneV { grab }) => {
                let before = self.geometry.artist_w;
                self.geometry.artist_w = (cx - grab - self.artists.rect.x).max(1);
                self.relayout();
                if self.geometry.artist_w != before {
                    vec![LibAction::Redraw]
                } else {
                    vec![]
                }
            }
            Some(Drag::Close) => {
                let inside = self.in_close(x, y);
                if inside != self.close_pressed {
                    self.close_pressed = inside;
                    vec![LibAction::Redraw]
                } else {
                    vec![]
                }
            }
            Some(Drag::Button) => {
                self.form.mouse_move(cx, cy);
                vec![LibAction::Redraw]
            }
            Some(Drag::Tree) => {
                if self.tree.mouse_move(cx, cy) {
                    vec![LibAction::Redraw]
                } else {
                    vec![]
                }
            }
            Some(d @ (Drag::Artists | Drag::Albums)) => {
                let artists = d == Drag::Artists;
                let t = if artists {
                    &mut self.artists
                } else {
                    &mut self.albums
                };
                match t.mouse_move(cx, cy) {
                    Some(ev) => {
                        let mut out = self.pane_event(artists, ev, view, false);
                        out.push(LibAction::Redraw);
                        out
                    }
                    None => vec![],
                }
            }
            Some(Drag::Table) => match self.table.mouse_move(cx, cy) {
                Some(ev) => {
                    let mut out = self.table_event(ev, view, false, x, y);
                    out.push(LibAction::Redraw);
                    out
                }
                None => vec![],
            },
            None => vec![],
        }
    }

    /// Left button released at a window point.
    pub fn mouse_up(
        &mut self,
        x: i32,
        y: i32,
        mods: Mods,
        view: &mut LibraryView,
    ) -> Vec<LibAction> {
        let (cx, cy) = (x - CLIENT_X, y - CLIENT_Y);
        match self.drag.take() {
            Some(Drag::Close) => {
                self.close_pressed = false;
                if self.in_close(x, y) {
                    vec![LibAction::Close]
                } else {
                    vec![LibAction::Redraw]
                }
            }
            Some(Drag::Button) => {
                let mut out = vec![LibAction::Redraw];
                match self.form.mouse_up(cx, cy) {
                    Some(FormEvent::Clicked(PLAY)) => out.push(self.auto(Purpose::Play)),
                    Some(FormEvent::Clicked(ENQUEUE)) => out.push(self.auto(Purpose::Enqueue)),
                    Some(FormEvent::Clicked(DROP)) => {
                        let l = self.layout();
                        out.push(LibAction::PlayMenu {
                            x: CLIENT_X + l.play.x,
                            y: CLIENT_Y + l.play.y,
                        });
                    }
                    Some(FormEvent::Clicked(CLEAR)) => self.clear_search(view),
                    _ => {}
                }
                out
            }
            Some(Drag::Tree) => {
                self.tree.mouse_up();
                vec![]
            }
            Some(d @ (Drag::Artists | Drag::Albums)) => {
                let artists = d == Drag::Artists;
                let t = if artists {
                    &mut self.artists
                } else {
                    &mut self.albums
                };
                let mut out = match t.mouse_up(cx, cy) {
                    Some(ev) => self.pane_event(artists, ev, view, mods.shift),
                    None => vec![],
                };
                out.push(LibAction::Redraw);
                out
            }
            Some(Drag::Table) => {
                let mut out = match self.table.mouse_up(cx, cy) {
                    Some(ev) => self.table_event(ev, view, mods.shift, x, y),
                    None => vec![],
                };
                out.push(LibAction::Redraw);
                out
            }
            Some(
                Drag::Resize { .. }
                | Drag::Splitter { .. }
                | Drag::PaneH { .. }
                | Drag::PaneV { .. },
            ) => {
                vec![LibAction::Redraw]
            }
            None => vec![],
        }
    }

    /// Empties the search box and shows everything again at once.
    fn clear_search(&mut self, view: &mut LibraryView) {
        self.form.set_text(SEARCH, "");
        view.set_search("", Instant::now());
        view.apply_search();
        self.sync(view);
    }

    /// Right button pressed at a window point.
    pub fn right_click(&mut self, x: i32, y: i32, view: &mut LibraryView) -> Vec<LibAction> {
        let (cx, cy) = (x - CLIENT_X, y - CLIENT_Y);
        let l = self.layout();
        if l.table.contains(cx, cy) {
            self.focus = Zone::List;
            return match self.table.right_click(cx, cy) {
                Some(ev) => {
                    let mut out = self.table_event(ev, view, false, x, y);
                    out.push(LibAction::Redraw);
                    out
                }
                None => vec![],
            };
        }
        for (artists, rect) in [(true, l.artists), (false, l.albums)] {
            if rect.contains(cx, cy) {
                self.focus = if artists { Zone::Artists } else { Zone::Albums };
                let t = if artists {
                    &mut self.artists
                } else {
                    &mut self.albums
                };
                if let Some(ev) = t.right_click(cx, cy) {
                    self.pane_event(artists, ev, view, false);
                }
                return vec![LibAction::Redraw];
            }
        }
        if l.tree.contains(cx, cy) {
            self.focus = Zone::Tree;
            if let Some(row) = self.tree.row_at(cx, cy) {
                if self.tree.selected != Some(row) {
                    self.tree.selected = Some(row);
                    self.tree_event(TreeEvent::Selected(row), view);
                }
                return vec![LibAction::TreeMenu { x, y }, LibAction::Redraw];
            }
        }
        vec![]
    }

    /// Mouse wheel over a window point (positive = up; with Shift the list scrolls
    /// sideways). Returns whether to redraw.
    pub fn wheel(
        &mut self,
        x: i32,
        y: i32,
        notches: i32,
        shift: bool,
        view: &mut LibraryView,
    ) -> bool {
        let (cx, cy) = (x - CLIENT_X, y - CLIENT_Y);
        let l = self.layout();
        if l.tree.contains(cx, cy) {
            return self.tree.wheel(notches);
        }
        for (rect, t) in [(l.artists, &mut self.artists), (l.albums, &mut self.albums)] {
            if rect.contains(cx, cy) {
                return if shift {
                    t.hwheel(notches).is_some()
                } else {
                    t.wheel(notches).is_some()
                };
            }
        }
        if shift {
            return self.table.hwheel(notches).is_some();
        }
        if self.table.wheel(notches).is_some() {
            self.sync(view);
            return true;
        }
        false
    }

    fn cursor_row_point(&self) -> (i32, i32) {
        let b = self.table.body_rect();
        let row = self.table.sel.cursor.unwrap_or(self.table.first());
        let k = row.saturating_sub(self.table.first()) as i32;
        (
            CLIENT_X + b.x + 20,
            CLIENT_Y + b.y + (k + 1) * self.table.row_height(),
        )
    }

    /// A key press. `None` when the window does not use the key (the shell then applies
    /// global and main-window shortcuts).
    pub fn key(
        &mut self,
        code: KeyCode,
        mods: Mods,
        text: Option<&str>,
        view: &mut LibraryView,
        now: Instant,
    ) -> Option<Vec<LibAction>> {
        use KeyCode::*;
        let plain = !mods.ctrl && !mods.alt;
        if mods.ctrl && !mods.alt && code == F4 {
            return Some(vec![LibAction::Close]);
        }
        if code == Tab && plain {
            let n = ZONES.len();
            let i = ZONES.iter().position(|z| *z == self.focus).unwrap_or(0);
            self.focus = ZONES[if mods.shift {
                (i + n - 1) % n
            } else {
                (i + 1) % n
            }];
            return Some(vec![LibAction::Redraw]);
        }
        let redraw = Some(vec![LibAction::Redraw]);
        match self.focus {
            Zone::Search => {
                if !plain {
                    return None;
                }
                self.form.focus_id(SEARCH);
                match code {
                    Enter | NumpadEnter => {
                        view.set_search(&self.search_text(), now);
                        view.apply_search();
                        self.sync(view);
                        self.focus = Zone::List;
                        redraw
                    }
                    ArrowDown => {
                        self.focus = Zone::List;
                        if self.table.sel.is_empty() && self.table.key(Key::Down, false).is_some() {
                            self.sync(view);
                        }
                        redraw
                    }
                    Escape => {
                        self.clear_search(view);
                        redraw
                    }
                    _ => {
                        let ev = match (edit_key(code), text) {
                            (Some(k), _) => self.form.key(k),
                            (None, Some(t)) if !t.chars().all(char::is_control) => {
                                self.form.text_input(t)
                            }
                            _ => return None,
                        };
                        if ev == Some(FormEvent::Changed(SEARCH)) {
                            view.set_search(&self.search_text(), now);
                        }
                        redraw
                    }
                }
            }
            Zone::Tree => {
                if code == ContextMenu || (code == F10 && mods.shift && plain) {
                    let row = self.tree.selected.unwrap_or(0);
                    let k = row.saturating_sub(self.tree.scroll.first) as i32;
                    let t = self.tree.rect;
                    return Some(vec![LibAction::TreeMenu {
                        x: CLIENT_X + t.x + 20,
                        y: CLIENT_Y + t.y + (k + 1) * self.tree.row_height(),
                    }]);
                }
                if !plain {
                    return None;
                }
                let k = nav_key(code)?;
                if let Some(ev) = self.tree.key(k) {
                    self.tree_event(ev, view);
                }
                redraw
            }
            Zone::Artists | Zone::Albums => {
                if !(plain || (mods.shift && !mods.ctrl && !mods.alt)) {
                    return None;
                }
                let artists = self.focus == Zone::Artists;
                let k = nav_key(code)?;
                let t = if artists {
                    &mut self.artists
                } else {
                    &mut self.albums
                };
                let mut out = vec![LibAction::Redraw];
                if let Some(ev) = t.key(k, false) {
                    out.extend(self.pane_event(artists, ev, view, mods.shift));
                }
                Some(out)
            }
            Zone::List => {
                let (c, a, s) = (mods.ctrl, mods.alt, mods.shift);
                let act = |p| Some(vec![LibAction::Selection(p)]);
                match (code, c, a, s) {
                    (KeyA, true, false, false) => {
                        self.table.select_all();
                        redraw
                    }
                    (Enter | NumpadEnter, false, false, s) => {
                        let row = self.table.sel.cursor.or(self.table.sel.first())?;
                        Some(self.activate(row, s, view))
                    }
                    (KeyQ, false, false, false) => act(Purpose::PlayNext),
                    (Delete, false, false, false) => act(Purpose::Remove),
                    (Digit3, false, true, false) => act(Purpose::FileInfo),
                    (KeyF, true, false, false) => act(Purpose::ShowInFolder),
                    (ContextMenu, false, false, false) | (F10, false, false, true) => {
                        let (x, y) = self.cursor_row_point();
                        Some(vec![LibAction::ContextMenu { x, y }])
                    }
                    (_, false, false, s) => {
                        let k = nav_key(code)?;
                        if self.table.key(k, s).is_some() {
                            self.sync(view);
                        }
                        redraw
                    }
                    _ => None,
                }
            }
        }
    }

    /// Draws the whole window.
    pub fn draw(
        &mut self,
        skin: &Skin,
        fonts: &mut FontSystem,
        view: &LibraryView,
        active: bool,
    ) -> Frame {
        let mut f = gen_window::draw_gen_frame(
            skin,
            &GenView {
                w: self.w,
                h: self.h,
                title: TITLE.into(),
                active,
                close_pressed: self.close_pressed,
                fixed_size: false,
            },
        );
        let (cx, cy, cw, ch) = gen_window::client_rect(self.w, self.h);
        f.fill_rect(cx, cy, cw, ch, skin.genex.window_bg);
        let mut text = PixelText { fonts, bold: false };
        self.form.focus = (active && self.focus == Zone::Search).then_some(EDIT_INDEX);
        self.form.draw(&mut f, cx, cy, skin, &mut text);
        if self.prefs.group_buttons {
            // The arrow of the split Play button.
            let r = self.layout().extra;
            let (ax, ay) = (cx + r.x + r.w / 2 - 2, cy + r.y + r.h / 2 - 1);
            for i in 0..3 {
                f.fill_rect(ax + i, ay + i, 5 - 2 * i, 1, skin.genex.window_text);
            }
        }
        self.tree.draw(
            &mut f,
            cx,
            cy,
            skin,
            &mut text,
            active && self.focus == Zone::Tree,
        );
        let mut artist_cell = |row: usize, col: usize| view.artist_cell(row, col);
        self.artists.draw(
            &mut f,
            cx,
            cy,
            skin,
            &mut text,
            active && self.focus == Zone::Artists,
            &mut artist_cell,
        );
        let mut album_cell = |row: usize, col: usize| view.album_cell(row, col);
        self.albums.draw(
            &mut f,
            cx,
            cy,
            skin,
            &mut text,
            active && self.focus == Zone::Albums,
            &mut album_cell,
        );
        let mut cell = |row: usize, col: usize| view.track(row).map(|t| view::cell(t, col));
        self.table.draw(
            &mut f,
            cx,
            cy,
            skin,
            &mut text,
            active && self.focus == Zone::List,
            &mut cell,
        );
        let s = self.layout().status;
        let status = text.fit(&view.status_text(), s.w);
        let (sx, sy) = (cx + s.x, cy + s.y);
        text.draw(
            &mut f,
            &status,
            sx,
            sy,
            skin.genex.window_text,
            (sx, sy, sx + s.w, sy + s.h),
        );
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::service::Request;
    use nulamp_library::{
        AlbumFacet, ArtistFacet, Capabilities, SortKey, Track, TrackId, TrackPage, TrackSummary,
    };
    use std::time::Duration;

    fn view_with_rows(n: usize) -> LibraryView {
        let mut v =
            LibraryView::new(&[("local".into(), "Local library".into(), Capabilities::SCAN)]);
        let reqs = v.take_requests();
        let token = |want_artists: bool| {
            reqs.iter()
                .find_map(|r| match r {
                    Request::Artists { token, .. } if want_artists => Some(*token),
                    Request::Albums { token, .. } if !want_artists => Some(*token),
                    _ => None,
                })
                .unwrap()
        };
        let tracks = (0..n.min(view::PAGE_ROWS))
            .map(|i| {
                let mut t = Track {
                    id: TrackId::new("local", i.to_string()),
                    location: format!("/music/{i:03}.mp3"),
                    length_ms: 125_000,
                    rating: (i % 6) as u8,
                    play_count: i as u64,
                    last_played: Some(1_760_227_200 + i as i64 * 60),
                    ..Track::default()
                };
                for (k, val) in [
                    ("title", format!("Song {i}")),
                    ("artist", "The Band".to_string()),
                    ("album", "First Album".to_string()),
                    ("year", "1999".to_string()),
                    ("genre", "Rock".to_string()),
                    ("track", (i + 1).to_string()),
                ] {
                    t.tags.insert(k.into(), val);
                }
                t
            })
            .collect();
        v.on_page(
            1,
            0,
            Ok(TrackPage {
                tracks,
                total: n as u64,
                offset: 0,
            }),
        );
        v.on_summary(
            1,
            TrackSummary {
                count: n as u64,
                length_ms: n as u64 * 125_000,
            },
        );
        v.on_artists(
            token(true),
            Ok(vec![
                ArtistFacet {
                    name: "Other".into(),
                    albums: 1,
                    tracks: 2,
                },
                ArtistFacet {
                    name: "The Band".into(),
                    albums: 2,
                    tracks: n as u64,
                },
            ]),
        );
        v.on_albums(
            token(false),
            Ok(vec![
                AlbumFacet {
                    name: "First Album".into(),
                    year: Some(1999),
                    tracks: n as u64,
                },
                AlbumFacet {
                    name: "Second".into(),
                    year: None,
                    tracks: 1,
                },
            ]),
        );
        v.on_children(
            "local",
            "",
            Ok(vec![nulamp_library::BrowseNode::new(
                "artists",
                "Artists",
                nulamp_library::NodeKind::Category,
                true,
            )]),
        );
        v
    }

    fn window(v: &mut LibraryView) -> LibraryWindow {
        let mut w = LibraryWindow::new(DEFAULT_SIZE, Geometry::default(), &[]);
        w.sync(v);
        v.take_requests();
        w
    }

    /// Window point of a list row's first column.
    fn row_point(w: &LibraryWindow, row: usize) -> (i32, i32) {
        let b = w.table.body_rect();
        (CLIENT_X + b.x + 10, CLIENT_Y + b.y + row as i32 * 14 + 4)
    }

    /// Window point of a pane row's first column.
    fn pane_point(t: &Table, row: usize) -> (i32, i32) {
        let b = t.body_rect();
        (CLIENT_X + b.x + 10, CLIENT_Y + b.y + row as i32 * 14 + 4)
    }

    /// Every pixel of a client-area rectangle uses one of `allowed`; returns how many pixels
    /// use `text`.
    fn only_colors(f: &Frame, r: Rect, allowed: &[u32], text: u32, what: &str) -> usize {
        let mut text_px = 0;
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let p = f.pixel(CLIENT_X + x, CLIENT_Y + y);
                assert!(
                    allowed.contains(&p),
                    "{what}: pixel ({x},{y}) is {p:06x}, not one of {allowed:06x?}"
                );
                text_px += usize::from(p == text);
            }
        }
        text_px
    }

    #[test]
    fn renders_text_without_antialiasing() {
        let skin = nulamp_skin::builtin::skin();
        let mut fonts = FontSystem::new();
        let mut v = view_with_rows(50);
        let mut w = window(&mut v);
        w.table.sel.click(1, false, false);
        w.table.sel.click(3, false, true);
        v.message = Some("Scan done: 50 added, 0 updated, 0 removed".into());
        let f = w.draw(&skin, &mut fonts, &v, true);
        assert_eq!((f.width(), f.height()), (w.w as u32, w.h as u32));
        let c = &skin.genex;
        // List rows: fills, 1-bit text and the cursor frame.
        let body = w.table.body_rect();
        let rows = Rect::new(
            body.x,
            body.y,
            body.w,
            50.min(w.table.visible_rows()) as i32 * 14,
        );
        let list = [
            c.item_bg,
            c.item_fg,
            c.alt_row_bg,
            c.alt_row_fg,
            c.selbar_bg_active,
            c.selbar_text_active,
            c.divider,
        ];
        assert!(only_colors(&f, rows, &list, c.item_fg, "list") > 500);
        // Header.
        let t = w.table.rect;
        let header = Rect::new(body.x, t.y + 1, body.w, w.table.header_height());
        let head = [
            c.header_bg,
            c.header_text,
            c.header_frame_top,
            c.header_frame_middle,
            c.header_frame_bottom,
            c.empty_header_bg,
        ];
        assert!(only_colors(&f, header, &head, c.header_text, "header") > 100);
        // Panes: "All" row, facets and the picked row (inactive selection colours).
        for (pane, what) in [(&w.artists, "artists"), (&w.albums, "albums")] {
            let b = pane.body_rect();
            let area = Rect::new(b.x, b.y, b.w, 3 * 14);
            let pc = [
                c.item_bg,
                c.item_fg,
                c.alt_row_bg,
                c.alt_row_fg,
                c.selbar_bg_inactive,
                c.selbar_text_inactive,
                c.divider,
            ];
            let n = only_colors(&f, area, &pc, c.item_fg, what)
                + only_colors(&f, area, &pc, c.selbar_text_inactive, what);
            assert!(n > 30, "{what} text");
        }
        // Tree (not focused: inactive selection colours), with its icons.
        let tr = w.tree.rect;
        let tree = Rect::new(tr.x + 1, tr.y + 1, tr.w - 2, tr.h - 2);
        let tc = [
            c.item_bg,
            c.item_fg,
            c.divider,
            c.selbar_bg_inactive,
            c.selbar_text_inactive,
        ];
        let tree_text = only_colors(&f, tree, &tc, c.selbar_text_inactive, "tree")
            + only_colors(&f, tree, &tc, c.item_fg, "tree");
        assert!(tree_text > 50);
        // Status line and search label.
        let l = w.layout();
        let wc = [c.window_bg, c.window_text];
        assert!(only_colors(&f, l.status, &wc, c.window_text, "status") > 100);
        assert!(only_colors(&f, l.label, &wc, c.window_text, "label") > 10);
    }

    #[test]
    fn layout_fits_minimum_and_default_sizes() {
        for grouped in [true, false] {
            for (w, h) in [(275, 116), (275, 232), DEFAULT_SIZE, (1000, 725)] {
                let l = layout(w, h, Geometry::default(), grouped);
                let (cw, ch) = (w - 19, h - 34);
                for r in [
                    l.search, l.clear, l.tree, l.artists, l.albums, l.table, l.play, l.extra,
                    l.status,
                ] {
                    assert!(r.x >= 0 && r.y >= 0, "{w}x{h} {r:?}");
                    assert!(r.x + r.w <= cw && r.y + r.h <= ch, "{w}x{h} {r:?}");
                }
                assert!(l.tree.x + l.tree.w <= l.artists.x);
                assert!(l.artists.x + l.artists.w <= l.albums.x);
                assert!(l.artists.y + l.artists.h <= l.table.y);
                assert!(l.search.x + l.search.w <= l.clear.x, "{w}x{h}");
                assert!(l.play.x + l.play.w <= l.extra.x);
            }
        }
        let g = |tree_w| Geometry {
            tree_w,
            ..Geometry::default()
        };
        assert_eq!(layout(275, 116, g(500), true).tree.w, 256 - 120);
        assert_eq!(layout(275, 116, g(10), true).tree.w, MIN_TREE_W);
        // The panes fill the right side and the list keeps a minimum height.
        let l = layout(DEFAULT_SIZE.0, DEFAULT_SIZE.1, Geometry::default(), true);
        assert_eq!(l.artists.y, l.albums.y);
        assert_eq!(l.albums.x + l.albums.w, l.table.x + l.table.w);
        let tall = Geometry {
            pane_h: 5000,
            ..Geometry::default()
        };
        assert!(layout(DEFAULT_SIZE.0, DEFAULT_SIZE.1, tall, true).table.h >= MIN_TABLE_H);
    }

    #[test]
    fn resize_snaps_and_splitters_move() {
        let mut v = view_with_rows(3);
        let mut w = window(&mut v);
        assert!(w.in_grip(w.w - 3, w.h - 3));
        assert!(!w.in_grip(w.w - 19, w.h - 19));
        let (gx, gy) = (w.w - 5, w.h - 5);
        assert!(
            w.mouse_down(gx, gy, Mods::NONE, &mut v, Instant::now())
                .is_empty()
        );
        assert_eq!(
            w.mouse_move(gx + 30, gy + 10, &mut v),
            [LibAction::Resize {
                w: DEFAULT_SIZE.0 + 30,
                h: DEFAULT_SIZE.1 + 10
            }]
        );
        assert!(w.set_size(DEFAULT_SIZE.0 + 30, DEFAULT_SIZE.1 + 10));
        assert_eq!((w.w, w.h), (DEFAULT_SIZE.0 + 50, DEFAULT_SIZE.1 + 29));
        w.mouse_up(gx + 30, gy + 10, Mods::NONE, &mut v);
        let now = Instant::now();
        // Tree splitter.
        let s = w.layout().splitter;
        let (sx, sy) = (CLIENT_X + s.x + 2, CLIENT_Y + s.y + 10);
        w.mouse_down(sx, sy, Mods::NONE, &mut v, now);
        w.mouse_move(sx + 40, sy, &mut v);
        w.mouse_up(sx + 40, sy, Mods::NONE, &mut v);
        assert_eq!(w.geometry.tree_w, DEFAULT_TREE_W + 40);
        assert_eq!(w.artists.rect.x, 4 + w.geometry.tree_w + SPLIT);
        // Splitter between the panes and the list.
        let h = w.layout().hsplit;
        let (hx, hy) = (CLIENT_X + h.x + 20, CLIENT_Y + h.y + 2);
        let before = (w.artists.rect.h, w.table.rect.h);
        w.mouse_down(hx, hy, Mods::NONE, &mut v, now);
        w.mouse_move(hx, hy + 20, &mut v);
        w.mouse_up(hx, hy + 20, Mods::NONE, &mut v);
        assert_eq!(w.artists.rect.h, before.0 + 20);
        assert_eq!(w.table.rect.h, before.1 - 20);
        assert_eq!(w.geometry.pane_h, before.0 + 20);
        // Splitter between the artist and the album pane.
        let p = w.layout().pane_split;
        let (px, py) = (CLIENT_X + p.x + 2, CLIENT_Y + p.y + 20);
        let aw = w.artists.rect.w;
        w.mouse_down(px, py, Mods::NONE, &mut v, now);
        w.mouse_move(px - 30, py, &mut v);
        w.mouse_up(px - 30, py, Mods::NONE, &mut v);
        assert_eq!(w.artists.rect.w, aw - 30);
        assert_eq!(w.geometry.artist_w, aw - 30);
        assert_eq!(w.albums.rect.x, w.artists.rect.x + w.artists.rect.w + SPLIT);
        // The first pane column follows the pane's width.
        let cols: i32 = w.artists.columns.iter().map(|c| c.width).sum();
        assert_eq!(cols, w.artists.rect.w - 2 - nulamp_skinui::SCROLLBAR_W);
    }

    #[test]
    fn close_button_and_title_bar() {
        let mut v = view_with_rows(0);
        let mut w = window(&mut v);
        let now = Instant::now();
        assert_eq!(
            w.mouse_down(100, 5, Mods::NONE, &mut v, now),
            [LibAction::BeginMove { x: 100, y: 5 }]
        );
        w.mouse_down(w.w - 5, 6, Mods::NONE, &mut v, now);
        assert!(w.close_pressed);
        assert_eq!(
            w.mouse_up(w.w - 5, 6, Mods::NONE, &mut v),
            [LibAction::Close]
        );
        w.mouse_down(w.w - 5, 6, Mods::NONE, &mut v, now);
        w.mouse_move(w.w - 50, 60, &mut v);
        assert!(!w.close_pressed);
        assert_eq!(
            w.mouse_up(w.w - 50, 60, Mods::NONE, &mut v),
            [LibAction::Redraw]
        );
    }

    #[test]
    fn list_clicks_play_and_sort() {
        let mut v = view_with_rows(50);
        let mut w = window(&mut v);
        assert_eq!(w.table.rows, 50);
        let now = Instant::now();
        let (x, y) = row_point(&w, 2);
        w.mouse_down(x, y, Mods::NONE, &mut v, now);
        w.mouse_up(x, y, Mods::NONE, &mut v);
        let (x5, y5) = row_point(&w, 5);
        w.mouse_down(x5, y5, Mods::SHIFT, &mut v, now + Duration::from_secs(1));
        w.mouse_up(x5, y5, Mods::SHIFT, &mut v);
        assert_eq!(w.table.sel.ranges(), [(2, 6)]);
        // Double-click plays the whole list from that row; with Shift it enqueues the
        // selection.
        let t = now + Duration::from_secs(2);
        w.mouse_down(x5, y5, Mods::NONE, &mut v, t);
        w.mouse_up(x5, y5, Mods::NONE, &mut v);
        let acts = w.mouse_down(x5, y5, Mods::NONE, &mut v, t + Duration::from_millis(50));
        assert!(acts.contains(&LibAction::PlayAllFrom(5)), "{acts:?}");
        w.table.sel.set_only(5);
        let acts = w.key(KeyCode::Enter, Mods::SHIFT, None, &mut v, t).unwrap();
        assert_eq!(acts, [LibAction::Selection(Purpose::Enqueue)]);
        // The listing starts in the node's order (artist), shown in the header.
        assert_eq!(w.table.sort, Some((0, false)));
        // A header click sorts by that column; a second click reverses.
        let b = w.table.body_rect();
        let hx = CLIENT_X + b.x + COLUMNS[0].width + COLUMNS[1].width + COLUMNS[2].width + 30;
        let hy = CLIENT_Y + w.table.rect.y + 5;
        w.mouse_down(hx, hy, Mods::NONE, &mut v, t);
        w.mouse_up(hx, hy, Mods::NONE, &mut v);
        assert_eq!(v.sort(), Some(Sort::asc(SortKey::Title)));
        assert_eq!(w.table.sort, Some((3, false)));
        assert!(w.table.sel.is_empty(), "a new order clears the selection");
        w.mouse_down(hx, hy, Mods::NONE, &mut v, t);
        w.mouse_up(hx, hy, Mods::NONE, &mut v);
        assert_eq!(v.sort(), Some(Sort::desc(SortKey::Title)));
        assert_eq!(w.table.sort, Some((3, true)));
    }

    #[test]
    fn activation_follows_the_options() {
        let mut v = view_with_rows(20);
        let mut w = window(&mut v);
        let now = Instant::now();
        let mut p = LibraryPrefs::default();
        let enter = |w: &mut LibraryWindow, v: &mut LibraryView, shift| {
            w.table.sel.set_only(7);
            let mods = Mods {
                shift,
                ..Mods::NONE
            };
            w.key(KeyCode::Enter, mods, None, v, now).unwrap()
        };
        assert_eq!(enter(&mut w, &mut v, false), [LibAction::PlayAllFrom(7)]);
        p.play_all_media = false;
        w.apply_prefs(&p);
        assert_eq!(
            enter(&mut w, &mut v, false),
            [LibAction::Selection(Purpose::Play)]
        );
        p.activate = ActivateAction::Enqueue;
        w.apply_prefs(&p);
        assert_eq!(
            enter(&mut w, &mut v, false),
            [LibAction::Selection(Purpose::Enqueue)]
        );
        p.activate = ActivateAction::PlayNext;
        w.apply_prefs(&p);
        assert_eq!(
            enter(&mut w, &mut v, false),
            [LibAction::Selection(Purpose::PlayNext)]
        );
        // Shift always enqueues.
        assert_eq!(
            enter(&mut w, &mut v, true),
            [LibAction::Selection(Purpose::Enqueue)]
        );
        // Play-all has its own switch for stored playlists.
        p = LibraryPrefs {
            play_all_media: false,
            ..LibraryPrefs::default()
        };
        w.apply_prefs(&p);
        v.spec.as_mut().unwrap().node.kind = nulamp_library::NodeKind::Playlist;
        assert_eq!(enter(&mut w, &mut v, false), [LibAction::PlayAllFrom(7)]);
    }

    #[test]
    fn buttons_act_on_selection_or_everything() {
        let mut v = view_with_rows(5);
        let mut w = window(&mut v);
        let p = w.layout().play;
        let (px, py) = (CLIENT_X + p.x + 5, CLIENT_Y + p.y + 5);
        w.mouse_down(px, py, Mods::NONE, &mut v, Instant::now());
        let acts = w.mouse_up(px, py, Mods::NONE, &mut v);
        assert!(acts.contains(&LibAction::All(Purpose::Play)));
        w.table.sel.set_only(1);
        w.mouse_down(px, py, Mods::NONE, &mut v, Instant::now());
        let acts = w.mouse_up(px, py, Mods::NONE, &mut v);
        assert!(acts.contains(&LibAction::Selection(Purpose::Play)));
        // Grouped buttons: the arrow opens the menu next to the Play button.
        let a = w.layout().extra;
        let (ax, ay) = (CLIENT_X + a.x + 5, CLIENT_Y + a.y + 5);
        w.mouse_down(ax, ay, Mods::NONE, &mut v, Instant::now());
        let acts = w.mouse_up(ax, ay, Mods::NONE, &mut v);
        assert!(
            acts.contains(&LibAction::PlayMenu {
                x: CLIENT_X + p.x,
                y: CLIENT_Y + p.y
            }),
            "{acts:?}"
        );
        // Separate buttons.
        w.apply_prefs(&LibraryPrefs {
            group_buttons: false,
            ..LibraryPrefs::default()
        });
        let e = w.layout().extra;
        let (ex, ey) = (CLIENT_X + e.x + 5, CLIENT_Y + e.y + 5);
        w.mouse_down(ex, ey, Mods::NONE, &mut v, Instant::now());
        let acts = w.mouse_up(ex, ey, Mods::NONE, &mut v);
        assert!(acts.contains(&LibAction::Selection(Purpose::Enqueue)));
        assert!(e.w >= 50, "room for the label");
    }

    #[test]
    fn typing_searches_after_the_pause() {
        let mut v = view_with_rows(5);
        let mut w = window(&mut v);
        let s = w.layout().search;
        let now = Instant::now();
        w.mouse_down(
            CLIENT_X + s.x + 5,
            CLIENT_Y + s.y + 5,
            Mods::NONE,
            &mut v,
            now,
        );
        assert_eq!(w.focus, Zone::Search);
        // Letters go to the search box, not to the player shortcuts.
        for c in ["a", "b", "b", "a"] {
            assert!(
                w.key(KeyCode::KeyA, Mods::NONE, Some(c), &mut v, now)
                    .is_some()
            );
        }
        w.key(KeyCode::Backspace, Mods::NONE, None, &mut v, now);
        assert_eq!(w.search_text(), "abb");
        assert!(!v.tick(now + Duration::from_millis(100)));
        assert!(v.tick(now + Duration::from_millis(300)));
        assert_eq!(v.spec.as_ref().unwrap().query, "abb");
        // The panes are reloaded for the search.
        let reqs = v.take_requests();
        assert!(reqs.iter().any(|r| matches!(r, Request::Artists { .. })));
        assert!(reqs.iter().any(|r| matches!(r, Request::Albums { .. })));
        // Escape clears the search at once; F-keys are left to the shell.
        w.key(KeyCode::Escape, Mods::NONE, None, &mut v, now);
        assert_eq!(v.spec.as_ref().unwrap().query, "");
        assert!(w.key(KeyCode::F5, Mods::NONE, None, &mut v, now).is_none());
        assert!(
            w.key(
                KeyCode::KeyL,
                Mods {
                    alt: true,
                    ..Mods::NONE
                },
                None,
                &mut v,
                now
            )
            .is_none()
        );
    }

    #[test]
    fn clear_search_button() {
        let mut v = view_with_rows(5);
        let mut w = window(&mut v);
        let now = Instant::now();
        w.form.set_text(SEARCH, "beat");
        v.set_search("beat", now);
        v.apply_search();
        assert_eq!(v.spec.as_ref().unwrap().query, "beat");
        let c = w.layout().clear;
        let (x, y) = (CLIENT_X + c.x + 5, CLIENT_Y + c.y + 5);
        w.mouse_down(x, y, Mods::NONE, &mut v, now);
        w.mouse_up(x, y, Mods::NONE, &mut v);
        assert_eq!(w.search_text(), "");
        assert_eq!(v.spec.as_ref().unwrap().query, "");
    }

    #[test]
    fn panes_filter_the_list() {
        let mut v = view_with_rows(30);
        let mut w = window(&mut v);
        // "All", then the two artists; the "All" row is picked.
        assert_eq!(w.artists.rows, 3);
        assert_eq!(w.albums.rows, 3);
        assert_eq!(w.artists.sel.ranges(), [(0, 1)]);
        let now = Instant::now();
        // Clicking an artist filters the list and the albums.
        let (x, y) = pane_point(&w.artists, 2);
        w.mouse_down(x, y, Mods::NONE, &mut v, now);
        w.mouse_up(x, y, Mods::NONE, &mut v);
        assert_eq!(w.focus, Zone::Artists);
        let spec = v.spec.as_ref().unwrap();
        assert_eq!(spec.artist.as_deref(), Some("The Band"));
        assert_eq!(spec.album, None);
        let reqs = v.take_requests();
        assert!(reqs.iter().any(|r| matches!(
            r,
            Request::Albums { spec, .. } if spec.artist.as_deref() == Some("The Band")
        )));
        assert!(reqs.iter().any(|r| matches!(r, Request::Tracks { .. })));
        assert_eq!(w.artists.sel.ranges(), [(2, 3)]);
        assert!(w.table.sel.is_empty());
        // The albums of the artist arrive.
        let token = reqs
            .iter()
            .find_map(|r| match r {
                Request::Albums { token, .. } => Some(*token),
                _ => None,
            })
            .unwrap();
        v.on_albums(
            token,
            Ok(vec![AlbumFacet {
                name: "First Album".into(),
                year: Some(1999),
                tracks: 30,
            }]),
        );
        w.sync(&mut v);
        // An album narrows the list further without reloading the albums.
        let (x, y) = pane_point(&w.albums, 1);
        w.mouse_down(x, y, Mods::NONE, &mut v, now + Duration::from_secs(1));
        w.mouse_up(x, y, Mods::NONE, &mut v);
        assert_eq!(
            v.spec.as_ref().unwrap().album.as_deref(),
            Some("First Album")
        );
        let reqs = v.take_requests();
        assert!(!reqs.iter().any(|r| matches!(r, Request::Albums { .. })));
        // Clicking "All" removes the filter; clicking below the rows keeps the pick.
        let (x, y) = pane_point(&w.albums, 0);
        w.mouse_down(x, y, Mods::NONE, &mut v, now + Duration::from_secs(2));
        w.mouse_up(x, y, Mods::NONE, &mut v);
        assert_eq!(v.spec.as_ref().unwrap().album, None);
        let (x, y) = pane_point(&w.artists, 10);
        w.mouse_down(x, y, Mods::NONE, &mut v, now + Duration::from_secs(3));
        w.mouse_up(x, y, Mods::NONE, &mut v);
        assert_eq!(w.artists.sel.ranges(), [(2, 3)]);
        // Keys move through the pane; Enter plays everything the pane shows.
        w.focus = Zone::Artists;
        w.key(KeyCode::ArrowUp, Mods::NONE, None, &mut v, now);
        assert_eq!(v.spec.as_ref().unwrap().artist.as_deref(), Some("Other"));
        let acts = w
            .key(KeyCode::Enter, Mods::NONE, None, &mut v, now)
            .unwrap();
        assert!(acts.contains(&LibAction::All(Purpose::Play)));
        // A double-click does the same.
        let t = now + Duration::from_secs(10);
        let (x, y) = pane_point(&w.artists, 1);
        w.mouse_down(x, y, Mods::NONE, &mut v, t);
        w.mouse_up(x, y, Mods::NONE, &mut v);
        let acts = w.mouse_down(x, y, Mods::NONE, &mut v, t + Duration::from_millis(50));
        assert!(acts.contains(&LibAction::All(Purpose::Play)), "{acts:?}");
    }

    #[test]
    fn pane_headers_sort_the_panes() {
        let mut v = view_with_rows(30);
        let mut w = window(&mut v);
        let now = Instant::now();
        // The Tracks column header of the artist pane: most tracks first after two clicks.
        let b = w.artists.body_rect();
        let tracks_x =
            CLIENT_X + b.x + w.artists.columns[0].width + w.artists.columns[1].width + 10;
        let hy = CLIENT_Y + w.artists.rect.y + 5;
        for _ in 0..2 {
            w.mouse_down(tracks_x, hy, Mods::NONE, &mut v, now);
            w.mouse_up(tracks_x, hy, Mods::NONE, &mut v);
        }
        assert_eq!(v.artist_sort, (2, true));
        assert_eq!(v.artists[0].name, "The Band");
        assert_eq!(w.artists.sort, Some((2, true)));
        // The picked artist stays picked when the order changes.
        v.select_artist(Some("Other".into()));
        w.sync(&mut v);
        assert_eq!(w.artists.sel.ranges(), [(2, 3)]);
    }

    #[test]
    fn options_reach_the_widgets() {
        let mut v = view_with_rows(3);
        let mut w = window(&mut v);
        assert!(w.table.alt_rows && w.artists.alt_rows && w.albums.alt_rows);
        assert!(w.tree.icons && w.tree.full_row);
        w.apply_prefs(&LibraryPrefs {
            alt_rows: false,
            tree_icons: false,
            tree_full_row: false,
            tree_item_height: 24,
            resize_mode: nulamp_skinui::ResizeMode::WithNext,
            ..LibraryPrefs::default()
        });
        assert!(!w.table.alt_rows && !w.artists.alt_rows && !w.albums.alt_rows);
        assert!(!w.tree.icons && !w.tree.full_row);
        assert_eq!(w.tree.item_height, 24);
        assert_eq!(w.table.resize_mode, nulamp_skinui::ResizeMode::WithNext);
        // The search text survives the buttons being regrouped.
        w.form.set_text(SEARCH, "kept");
        w.apply_prefs(&LibraryPrefs {
            group_buttons: false,
            ..LibraryPrefs::default()
        });
        assert_eq!(w.search_text(), "kept");
    }

    #[test]
    fn keyboard_in_list_and_tree() {
        let mut v = view_with_rows(30);
        let mut w = window(&mut v);
        w.focus = Zone::List;
        let now = Instant::now();
        w.key(KeyCode::ArrowDown, Mods::NONE, None, &mut v, now);
        w.key(KeyCode::ArrowDown, Mods::SHIFT, None, &mut v, now);
        assert_eq!(w.table.sel.ranges(), [(0, 2)]);
        let ctrl = Mods::CTRL;
        w.key(KeyCode::KeyA, ctrl, None, &mut v, now);
        assert_eq!(w.table.sel.count(), 30);
        assert_eq!(
            w.key(KeyCode::Delete, Mods::NONE, None, &mut v, now),
            Some(vec![LibAction::Selection(Purpose::Remove)])
        );
        assert_eq!(
            w.key(KeyCode::KeyQ, Mods::NONE, None, &mut v, now),
            Some(vec![LibAction::Selection(Purpose::PlayNext)])
        );
        // Unused letters go to the player shortcuts.
        assert!(
            w.key(KeyCode::KeyX, Mods::NONE, Some("x"), &mut v, now)
                .is_none()
        );
        // Tab moves through search, tree, artists, albums and list.
        for zone in [
            Zone::Search,
            Zone::Tree,
            Zone::Artists,
            Zone::Albums,
            Zone::List,
        ] {
            w.key(KeyCode::Tab, Mods::NONE, None, &mut v, now);
            assert_eq!(w.focus, zone);
        }
        w.key(KeyCode::Tab, Mods::SHIFT, None, &mut v, now);
        assert_eq!(w.focus, Zone::Albums);
        w.focus = Zone::Tree;
        // Down selects "Artists" and lists it.
        w.key(KeyCode::ArrowDown, Mods::NONE, None, &mut v, now);
        assert_eq!(v.spec.as_ref().unwrap().node.id, "artists");
        assert!(w.table.sel.is_empty());
        w.key(KeyCode::ArrowRight, Mods::NONE, None, &mut v, now);
        assert!(v.take_requests().iter().any(|r| matches!(
            r,
            super::super::service::Request::Browse { node, .. } if node.id == "artists"
        )));
        assert_eq!(
            w.key(KeyCode::F4, ctrl, None, &mut v, now),
            Some(vec![LibAction::Close])
        );
    }

    #[test]
    fn dragging_rows_out_and_context_menu() {
        let mut v = view_with_rows(10);
        let mut w = window(&mut v);
        let now = Instant::now();
        let (x, y) = row_point(&w, 3);
        w.mouse_down(x, y, Mods::NONE, &mut v, now);
        w.mouse_move(x - 200, y, &mut v);
        assert!(w.dragging_tracks());
        let acts = w.mouse_up(-150, 40, Mods::NONE, &mut v);
        assert!(acts.contains(&LibAction::DropTracks { x: -150, y: 40 }));
        let (x7, y7) = row_point(&w, 7);
        let acts = w.right_click(x7, y7, &mut v);
        assert!(acts.contains(&LibAction::ContextMenu { x: x7, y: y7 }));
        assert_eq!(w.table.sel.ranges(), [(7, 8)]);
        // Right-click in the tree selects the node and opens the tree menu.
        let t = w.tree.rect;
        let (tx, ty) = (CLIENT_X + t.x + 40, CLIENT_Y + t.y + 1 + 18 + 4);
        let acts = w.right_click(tx, ty, &mut v);
        assert!(acts.contains(&LibAction::TreeMenu { x: tx, y: ty }));
        assert_eq!(v.spec.as_ref().unwrap().node.id, "artists");
    }
}
