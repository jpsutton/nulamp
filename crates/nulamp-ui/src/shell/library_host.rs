//! The media-library window inside the shell: worker responses, input, drawing and the
//! actions on library tracks (play, enqueue, rate, remove, …), and play counting.

use std::time::Instant;

use nulamp_core::{Command, NewEntry, PlayState};
use nulamp_library::{LibraryEvent, PlaybackLocation, PlaylistInfo, Track};
use nulamp_render::Frame;
use winit::event::{ElementState, MouseButton};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::KeyCode;
use winit::window::WindowId;

use super::{Kind, PlayerHandle, Shell, show_in_folder};
use crate::library::{
    Heard, LibAction, LibCmd, LibraryService, LibraryView, LibraryWindow, MediaLibrary,
    PlayTracker, Purpose, Request, Response,
};
use crate::shell::LibraryPrefs;

pub(super) struct LibraryHost {
    pub service: LibraryService,
    pub view: LibraryView,
    pub win: LibraryWindow,
    pub tracker: PlayTracker,
    /// Stored playlists of every provider: (provider, playlist).
    pub playlists: Vec<(String, PlaylistInfo)>,
}

/// A playlist entry for a library track: local files get their title from the tags like
/// any file; streams use the library's title.
fn entry(t: &Track, loc: &PlaybackLocation) -> NewEntry {
    let mut e = NewEntry::new(loc.location.clone());
    if nulamp_core::playlist::is_url(&loc.location) {
        e.title = Some(match t.artist() {
            Some(a) => format!("{a} - {}", t.display_title()),
            None => t.display_title(),
        });
    }
    e.length_ms = (t.length_ms > 0).then_some(t.length_ms);
    e
}

impl<P: PlayerHandle> Shell<P> {
    /// Adds the media library: its window, worker thread, watch folders and play counting.
    pub fn with_library(mut self, lib: MediaLibrary) -> Self {
        let service = LibraryService::start(lib);
        let view = LibraryView::new(&service.providers);
        let win = LibraryWindow::new(
            self.prefs.lib_size,
            self.prefs.lib_tree_w,
            &self.prefs.lib_columns,
        );
        let p = &self.prefs.library;
        service.send(Request::SyncFolders {
            folders: p.folders.clone(),
            remove_others: false,
        });
        service.send(Request::SetWatching(p.watch));
        if p.rescan_on_start {
            service.send(Request::Rescan { full: false });
        }
        service.send(Request::Playlists);
        self.library = Some(LibraryHost {
            service,
            view,
            win,
            tracker: PlayTracker::new(),
            playlists: Vec::new(),
        });
        self
    }

    pub(super) fn library_send(&self, r: Request) {
        if let Some(lib) = &self.library {
            lib.service.send(r);
        }
    }

    /// The preferences dialog changed the library options.
    pub(super) fn library_prefs_changed(&self, old: &LibraryPrefs, new: &LibraryPrefs) {
        if old.folders != new.folders {
            self.library_send(Request::SyncFolders {
                folders: new.folders.clone(),
                remove_others: true,
            });
        }
        if old.watch != new.watch {
            self.library_send(Request::SetWatching(new.watch));
        }
    }

    /// Brings the widgets up to date and sends the queued requests to the worker.
    fn library_flush(&mut self) {
        if let Some(lib) = self.library.as_mut() {
            lib.win.sync(&mut lib.view);
            lib.service.set_generation(lib.view.generation);
            for r in lib.view.take_requests() {
                lib.service.send(r);
            }
        }
    }

    /// Worker responses and timers. Returns whether the window needs a redraw.
    pub(super) fn library_poll(&mut self, el: &ActiveEventLoop) -> bool {
        let Some(lib) = self.library.as_mut() else {
            return false;
        };
        let now = Instant::now();
        let mut dirty = lib.view.tick(now);
        let mut collected = Vec::new();
        for r in lib.service.poll() {
            dirty = true;
            match r {
                Response::Children {
                    provider,
                    parent,
                    result,
                } => lib.view.on_children(&provider, &parent, result),
                Response::Page {
                    generation,
                    offset,
                    result,
                } => lib.view.on_page(generation, offset, result),
                Response::Summary {
                    generation,
                    summary,
                } => lib.view.on_summary(generation, summary),
                Response::Collected { purpose, items } => collected.push((purpose, items)),
                Response::Event { event, .. } => {
                    if event == LibraryEvent::PlaylistsChanged {
                        lib.service.send(Request::Playlists);
                    }
                    lib.view.on_event(event, now);
                }
                Response::Playlists(p) => lib.playlists = p,
                Response::Folders(f) => {
                    self.prefs.library.folders =
                        f.into_iter().map(|f| (f.path, f.recursive)).collect();
                }
                Response::Error(e) => lib.view.message = Some(e),
            }
        }
        self.library_flush();
        for (purpose, items) in collected {
            self.library_collected(el, purpose, items);
        }
        dirty
    }

    pub(super) fn draw_library(&mut self) -> Frame {
        let active = self.is_active(Kind::Library);
        match self.library.as_mut() {
            Some(lib) => lib.win.draw(&self.skin, &mut self.fonts, &lib.view, active),
            None => Frame::new(275, 116, 0),
        }
    }

    pub(super) fn library_mouse(
        &mut self,
        id: WindowId,
        state: ElementState,
        button: MouseButton,
        x: i32,
        y: i32,
        el: &ActiveEventLoop,
    ) {
        let mods = self.mods;
        let Some(lib) = self.library.as_mut() else {
            return;
        };
        let now = Instant::now();
        let acts = match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => {
                lib.win.mouse_down(x, y, mods, &mut lib.view, now)
            }
            (MouseButton::Left, ElementState::Released) => {
                lib.win.mouse_up(x, y, mods, &mut lib.view)
            }
            (MouseButton::Right, ElementState::Pressed) => lib.win.right_click(x, y, &mut lib.view),
            _ => Vec::new(),
        };
        self.library_actions(id, acts, el);
    }

    pub(super) fn library_mouse_move(
        &mut self,
        id: WindowId,
        x: i32,
        y: i32,
        el: &ActiveEventLoop,
    ) {
        let Some(lib) = self.library.as_mut() else {
            return;
        };
        let acts = lib.win.mouse_move(x, y, &mut lib.view);
        if !acts.is_empty() {
            self.library_actions(id, acts, el);
        }
    }

    /// Returns true when the library window used the key.
    pub(super) fn library_key(
        &mut self,
        id: WindowId,
        code: KeyCode,
        text: Option<&str>,
        el: &ActiveEventLoop,
    ) -> bool {
        let mods = self.mods;
        let Some(lib) = self.library.as_mut() else {
            return false;
        };
        match lib.win.key(code, mods, text, &mut lib.view, Instant::now()) {
            Some(acts) => {
                self.library_actions(id, acts, el);
                true
            }
            None => false,
        }
    }

    pub(super) fn library_wheel(&mut self, notches: i32) {
        let shift = self.mods.shift;
        let (x, y) = self
            .ids
            .get(&Kind::Library)
            .and_then(|id| self.windows.get(id))
            .map(|w| w.cursor)
            .unwrap_or((0, 0));
        let Some(lib) = self.library.as_mut() else {
            return;
        };
        if lib.win.wheel(x, y, notches, shift, &mut lib.view) {
            self.library_flush();
            self.request_redraw(Kind::Library);
        }
    }

    fn library_actions(&mut self, id: WindowId, acts: Vec<LibAction>, el: &ActiveEventLoop) {
        for a in acts {
            match a {
                LibAction::Close => self.set_visible(Kind::Library, false),
                LibAction::BeginMove { x, y } => self.begin_drag(id, x, y),
                LibAction::Resize { w, h } => {
                    let changed = self
                        .library
                        .as_mut()
                        .is_some_and(|lib| lib.win.set_size(w, h));
                    if changed {
                        self.library_flush();
                        self.resize_window(Kind::Library);
                    }
                }
                LibAction::Selection(p) => {
                    let ranges = self
                        .library
                        .as_ref()
                        .map(|lib| lib.win.table.sel.ranges().to_vec())
                        .unwrap_or_default();
                    self.library_act(ranges, p);
                }
                LibAction::All(p) => {
                    let total = self
                        .library
                        .as_ref()
                        .and_then(|lib| lib.view.total)
                        .unwrap_or(0);
                    self.library_act(vec![(0, total)], p);
                }
                LibAction::ContextMenu { x, y } => {
                    let Some(lib) = &self.library else { continue };
                    let provider = lib.view.spec.as_ref().map(|s| s.provider.clone());
                    let caps = provider
                        .as_deref()
                        .map(|p| lib.view.capabilities(p))
                        .unwrap_or_default();
                    let playlists: Vec<(String, String)> = lib
                        .playlists
                        .iter()
                        .filter(|(p, _)| Some(p) == provider.as_ref())
                        .map(|(_, i)| (i.id.clone(), i.name.clone()))
                        .collect();
                    let items = crate::menus::library_context(&playlists, caps);
                    self.open_items_at(el, id, items, x, y);
                }
                LibAction::TreeMenu { x, y } => {
                    let Some(lib) = &self.library else { continue };
                    let caps = lib
                        .view
                        .spec
                        .as_ref()
                        .map(|s| lib.view.capabilities(&s.provider))
                        .unwrap_or_default();
                    let items = crate::menus::library_tree_menu(caps);
                    self.open_items_at(el, id, items, x, y);
                }
                LibAction::DropTracks { x, y } => self.library_drop(id, x, y),
                LibAction::Redraw => {}
            }
        }
        self.library_flush();
        self.request_redraw(Kind::Library);
    }

    /// Fetches the rows of `ranges` for an action; the action runs when they arrive.
    fn library_act(&mut self, mut ranges: Vec<(usize, usize)>, purpose: Purpose) {
        if matches!(purpose, Purpose::FileInfo | Purpose::ShowInFolder) {
            // One file is enough.
            ranges.truncate(1);
            if let Some(r) = ranges.first_mut() {
                r.1 = r.0 + 1;
            }
        }
        if let Some(lib) = self.library.as_mut() {
            lib.view.collect(&ranges, purpose);
        }
        self.library_flush();
    }

    /// Dragged rows released at a library-window point: over the playlist they are inserted
    /// at the row under the pointer; over the main window they replace the playlist and play
    /// (Shift: append), like dropped files (04 §7.4).
    fn library_drop(&mut self, id: WindowId, x: i32, y: i32) {
        let Some(w) = self.windows.get(&id) else {
            return;
        };
        let (sx, sy) = (w.actual.0 + x, w.actual.1 + y);
        let target = [Kind::Playlist, Kind::Main].into_iter().find_map(|k| {
            let win = self.ids.get(&k).and_then(|i| self.windows.get(i))?;
            let r = crate::geom::Rect::new(win.actual.0, win.actual.1, win.rect.w, win.rect.h);
            (win.visible && r.contains(sx, sy)).then_some((k, sy - win.actual.1))
        });
        let purpose = match target {
            Some((Kind::Playlist, ly)) => Purpose::Insert(self.pl_drop_row(ly)),
            Some(_) if self.mods.shift => Purpose::Enqueue,
            Some(_) => Purpose::Play,
            None => return,
        };
        let ranges = self
            .library
            .as_ref()
            .map(|lib| lib.win.table.sel.ranges().to_vec())
            .unwrap_or_default();
        self.library_act(ranges, purpose);
    }

    pub(super) fn library_menu_cmd(&mut self, el: &ActiveEventLoop, cmd: LibCmd) {
        let Some(&id) = self.ids.get(&Kind::Library) else {
            return;
        };
        let act = match cmd {
            LibCmd::Selection(p) => LibAction::Selection(p),
            LibCmd::All(p) => LibAction::All(p),
            LibCmd::SelectAll => {
                if let Some(lib) = self.library.as_mut() {
                    lib.win.table.select_all();
                }
                LibAction::Redraw
            }
            LibCmd::Rescan => {
                self.library_send(Request::Rescan { full: false });
                LibAction::Redraw
            }
        };
        self.library_actions(id, vec![act], el);
    }

    /// Carries out an action on fetched library tracks.
    fn library_collected(
        &mut self,
        el: &ActiveEventLoop,
        purpose: Purpose,
        items: Vec<(Track, PlaybackLocation)>,
    ) {
        let Some(first) = items.first() else { return };
        let provider = first.0.id.provider.clone();
        let first_location = first.1.location.clone();
        let entries = || items.iter().map(|(t, l)| entry(t, l)).collect::<Vec<_>>();
        let ids = || items.iter().map(|(t, _)| t.id.clone()).collect::<Vec<_>>();
        match purpose {
            Purpose::Play => {
                let e = self.player.engine_mut();
                e.clear();
                e.append(entries());
                self.player.command(Command::StartOfList);
                let i = self.player.engine().playlist.current_index();
                self.player.command(Command::PlayIndex(i));
            }
            Purpose::Enqueue => {
                self.player.engine_mut().append(entries());
            }
            Purpose::PlayNext => {
                let e = self.player.engine_mut();
                let new = e.append(entries());
                for eid in new.into_iter().rev() {
                    e.queue.push_front(eid);
                }
                if self.last_status.state == PlayState::Stopped {
                    self.player.command(Command::Play);
                }
            }
            Purpose::Insert(row) => {
                let n = self.player.engine().playlist.len();
                self.player.engine_mut().insert(row.min(n), entries());
            }
            Purpose::Rate(stars) => self.library_send(Request::SetRating {
                tracks: items.into_iter().map(|(t, _)| t).collect(),
                stars,
            }),
            Purpose::Remove => {
                let titles: Vec<String> = items
                    .iter()
                    .map(|(t, _)| match t.artist() {
                        Some(a) => format!("{a} - {}", t.display_title()),
                        None => t.display_title(),
                    })
                    .collect();
                self.pending_dialog = Some(Box::new(
                    crate::dialogs::ConfirmDialog::remove_from_library(&titles, ids()),
                ));
            }
            Purpose::FileInfo => {
                if !nulamp_core::playlist::is_url(&first_location) {
                    let d = crate::dialogs::FileInfoDialog::new(&first_location);
                    self.open_dialog(el, Box::new(d));
                }
            }
            Purpose::ShowInFolder => show_in_folder(&first_location),
            Purpose::AddToPlaylist(Some(playlist)) => {
                self.library_send(Request::AddToPlaylist {
                    provider,
                    playlist: Some(playlist),
                    name: String::new(),
                    ids: ids(),
                });
            }
            Purpose::AddToPlaylist(None) => {
                self.pending_dialog = Some(Box::new(crate::dialogs::LibraryPlaylistDialog::new(
                    provider,
                    ids(),
                )));
            }
        }
        let n = self.player.engine().playlist.len();
        self.pl.clamp_scroll(n);
        self.poll_player();
        self.request_redraw_all();
    }

    /// Follows the heard track and records a play in the library when it ends or is
    /// skipped (decision MLUI8).
    pub(super) fn track_library_play(&mut self) {
        let Some(lib) = self.library.as_mut() else {
            return;
        };
        let s = &self.last_status;
        let heard = match s.state {
            PlayState::Stopped => None,
            _ => self
                .player
                .engine()
                .playlist
                .get(s.position)
                .map(|e| Heard {
                    entry: e.id,
                    location: &e.location,
                    position_ms: s.position_ms,
                    length_ms: s.length_ms,
                }),
        };
        if let Some(done) = lib.tracker.update(heard, Instant::now()) {
            lib.service.send(Request::FinishPlay {
                location: done.location,
                played_ms: done.played_ms,
                length_ms: done.length_ms,
            });
        }
    }
}
