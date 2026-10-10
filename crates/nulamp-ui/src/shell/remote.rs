//! Remote control: requests from other instances, the command line and MPRIS (11 §4, §6).

use std::time::Instant;

use nulamp_core::{Command, PlayState};
use nulamp_remote::{Assignable, EqValues, Flag, NumberedCommand as N, Request, Response};
use winit::event_loop::ActiveEventLoop;

use super::{Kind, PlayerHandle, Shell};
use crate::input::Action;
use crate::keys::KeyAction;

fn flag(f: Flag, current: bool) -> bool {
    match f {
        Flag::On => true,
        Flag::Off => false,
        Flag::Toggle => !current,
    }
}

impl<P: PlayerHandle> Shell<P> {
    /// Attaches the single-instance server; requests are handled on every UI tick.
    pub fn with_remote(mut self, server: nulamp_remote::Server) -> Self {
        match nulamp_remote::Mpris::start(server.sender()) {
            Ok(m) => self.mpris = Some(m),
            Err(e) => log::info!("MPRIS not available: {e}"),
        }
        self.remote = Some(server);
        self
    }

    pub(super) fn poll_remote(&mut self, el: &ActiveEventLoop) {
        while let Ok(req) = self.tray_rx.try_recv() {
            self.handle_request(el, req);
        }
        for req in self.player.take_requests() {
            self.handle_request(el, req);
        }
        let pressed = self
            .global_hotkeys
            .as_ref()
            .map(|h| h.poll())
            .unwrap_or_default();
        for action in pressed {
            self.hotkey_action(el, action);
        }
        loop {
            let Some((req, reply)) = self.remote.as_ref().and_then(|s| s.try_recv()) else {
                break;
            };
            let response = self.handle_request(el, req);
            reply.send(response);
        }
        if let Some(m) = &self.mpris
            && self.mpris_updated.elapsed().as_millis() >= 250
        {
            self.mpris_updated = Instant::now();
            let s = &self.last_status;
            let mut snap = nulamp_remote::PlayerSnapshot::from_status(s);
            let e = self.player.engine();
            if let Some(entry) = e.playlist.get(s.position) {
                snap.location = entry.location.clone();
                snap.track_id = Some(entry.id.0);
                if let Some((artist, title)) = entry.title.split_once(" - ") {
                    snap.artist = artist.to_string();
                    snap.title = title.to_string();
                }
            }
            if let Err(e) = m.update(&snap) {
                log::debug!("MPRIS update failed: {e}");
            }
        }
    }

    /// Runs the action of a global hotkey (11 §5).
    fn hotkey_action(&mut self, el: &ActiveEventLoop, action: Assignable) {
        use Assignable as A;
        if let Some(req) = action.to_request() {
            self.handle_request(el, req);
            return;
        }
        let current = self
            .player
            .engine()
            .playlist
            .get(self.last_status.position)
            .map(|e| (e.title.clone(), e.location.clone()));
        let rating = |loc: &str| {
            nulamp_meta::read(loc)
                .ok()
                .and_then(|m| m.get("rating"))
                .and_then(|r| r.trim().parse::<u8>().ok())
                .unwrap_or(0)
        };
        match action {
            A::CopyTitle | A::CopyPath => {
                let Some((title, location)) = current else {
                    return;
                };
                let text = if action == A::CopyTitle {
                    title
                } else {
                    location
                };
                if self.clipboard.is_none() {
                    self.clipboard = arboard::Clipboard::new().ok();
                }
                if let Some(c) = &mut self.clipboard
                    && let Err(e) = c.set_text(text)
                {
                    log::info!("clipboard not available: {e}");
                }
            }
            A::ShowInFolder => {
                if let Some((_, loc)) = current {
                    super::show_in_folder(&loc);
                }
            }
            A::Rate0
            | A::Rate1
            | A::Rate2
            | A::Rate3
            | A::Rate4
            | A::Rate5
            | A::RateUp
            | A::RateDown => {
                let Some((_, loc)) = current else {
                    return;
                };
                let stars = match action {
                    A::Rate0 => 0,
                    A::Rate1 => 1,
                    A::Rate2 => 2,
                    A::Rate3 => 3,
                    A::Rate4 => 4,
                    A::Rate5 => 5,
                    A::RateUp => (rating(&loc) + 1).min(5),
                    _ => rating(&loc).saturating_sub(1),
                };
                self.rate_files(&[loc], stars);
            }
            A::Hide => self.minimize_to_tray(),
            A::VisConfigure => {
                if let Some(id) = self.ids.get(&Kind::Main).copied() {
                    self.key_action(id, KeyAction::Preferences, el);
                }
            }
            // The classic visualizer has no presets and no full-screen mode.
            _ => log::debug!("hotkey action {action:?} does nothing here"),
        }
    }

    fn handle_request(&mut self, el: &ActiveEventLoop, req: Request) -> Response {
        let main = self.ids.get(&Kind::Main).copied();
        let cmd = |s: &mut Self, c: Command| {
            s.coalescer.reset();
            s.player.command(c);
        };
        match req {
            Request::Open { files, enqueue } => {
                let plan = self.coalescer.open(files, enqueue, Instant::now());
                if plan.clear {
                    self.add_items(plan.files, true);
                } else {
                    let n = self.player.engine().playlist.len();
                    self.add_items(plan.files, false);
                    if plan.play {
                        self.player.command(Command::PlayIndex(n));
                    }
                }
            }
            Request::LoadSkin(p) => self.load_skin(Some(p)),
            Request::Raise => self.raise(),
            Request::Play => cmd(self, Command::Play),
            Request::Pause => cmd(self, Command::Pause),
            Request::PlayPause => {
                let c = if self.last_status.state == PlayState::Playing {
                    Command::Pause
                } else {
                    Command::Play
                };
                cmd(self, c);
            }
            Request::Stop => cmd(self, Command::Stop),
            Request::StopWithFade => cmd(self, Command::StopWithFade),
            Request::StopAfterCurrent => cmd(self, Command::ToggleStopAfterCurrent),
            Request::Previous => cmd(self, Command::Previous),
            Request::Next => cmd(self, Command::Next),
            Request::StartOfList => cmd(self, Command::StartOfList),
            Request::EndOfList => cmd(self, Command::EndOfList),
            Request::StartPlayback => {
                cmd(self, Command::StartOfList);
                cmd(
                    self,
                    Command::PlayIndex(self.player.engine().playlist.current_index()),
                );
            }
            Request::SeekRelative(d) => cmd(self, Command::SeekRelative(d)),
            Request::SeekTo(ms) => cmd(self, Command::SeekTo(ms)),
            Request::SetVolume(v) => cmd(self, Command::SetVolume(v)),
            Request::VolumeBy(d) => cmd(self, Command::VolumeBy(d)),
            Request::SetBalance(b) => cmd(self, Command::SetBalance(b)),
            Request::BalanceBy(d) => {
                let b = (self.last_status.balance as i32 + d).clamp(-127, 127) as i8;
                cmd(self, Command::SetBalance(b));
            }
            Request::SetShuffle(f) => {
                cmd(self, Command::SetShuffle(flag(f, self.last_status.shuffle)))
            }
            Request::SetRepeat(f) => {
                cmd(self, Command::SetRepeat(flag(f, self.last_status.repeat)))
            }
            Request::SetManualAdvance(f) => cmd(
                self,
                Command::SetManualAdvance(flag(f, self.last_status.manual_advance)),
            ),
            Request::Clear => {
                self.coalescer.reset();
                self.player.engine_mut().clear();
            }
            Request::JumpToFile => {
                if let Some(id) = main {
                    self.key_action(id, KeyAction::JumpToFile, el);
                }
            }
            Request::Bookmark(f) => {
                let title = nulamp_core::playlist::placeholder_title(&f);
                let mut all = self.bookmarks();
                all.push((f, title));
                self.save_bookmarks(&all);
            }
            Request::Quit | Request::Kill => {
                self.exit = true;
                el.exit();
            }
            Request::Command(n) => {
                if let Some(id) = main {
                    self.numbered_command(el, id, n);
                }
            }
            Request::SetEqBand { band, value } => {
                if let Some(b) = self.prefs.eq.bands.get_mut(band as usize) {
                    *b = value.min(63);
                    self.eq_changed();
                }
            }
            Request::SetEqPreamp(v) => {
                self.prefs.eq.preamp = v.min(63);
                self.eq_changed();
            }
            Request::SetEqEnabled(on) => {
                self.prefs.eq.enabled = on;
                self.eq_changed();
            }
            Request::SetEqAutoLoad(on) => {
                self.prefs.eq.auto_load = on;
                self.eq_changed();
            }
            Request::SetPlaylistPosition(i) => self.player.engine_mut().playlist.set_current(i),
            Request::GetStatus => return Response::Status((&self.last_status).into()),
            Request::GetEq => {
                let e = &self.prefs.eq;
                return Response::Eq(EqValues {
                    bands: e.bands,
                    preamp: e.preamp,
                    enabled: e.enabled,
                    auto_load: e.auto_load,
                });
            }
            Request::GetPlaylistPosition => {
                return Response::PlaylistPosition {
                    position: self.last_status.position,
                    length: self.player.engine().playlist.len(),
                };
            }
            Request::GetEntry(i) => {
                return match self.player.engine().playlist.get(i) {
                    Some(e) => Response::Entry {
                        location: e.location.clone(),
                        title: e.title.clone(),
                    },
                    None => Response::Error("no such entry".into()),
                };
            }
            Request::GetCurrentTitle => return Response::Title(self.last_status.title.clone()),
            Request::PlayCd(n) => {
                // `/CD<n>` (11 §4.2): replace the list with the disc in drive n and play.
                match self.player.cd_drives().into_iter().nth(n as usize) {
                    Some((location, _)) => self.add_items(vec![location], true),
                    None => return Response::Error(format!("no CD drive {n}")),
                }
            }
            Request::SavePlaylist | Request::GetVersion => {
                return Response::Error("not supported".into());
            }
        }
        self.poll_player();
        self.request_redraw_all();
        Response::Ok
    }

    /// Numbered commands (11 §6.3).
    fn numbered_command(&mut self, el: &ActiveEventLoop, id: winit::window::WindowId, n: N) {
        let a = |s: &mut Self, a: Action| s.handle_action(id, a, el);
        let k = |s: &mut Self, k: KeyAction| s.key_action(id, k, el);
        let p = |s: &mut Self, c: Command| s.player.command(c);
        match n {
            N::Exit => a(self, Action::Exit),
            N::Preferences => k(self, KeyAction::Preferences),
            N::AlwaysOnTop => a(self, Action::ToggleAlwaysOnTop),
            N::Repeat => a(self, Action::ToggleRepeat),
            N::Shuffle => a(self, Action::ToggleShuffle),
            N::OpenFiles | N::OpenFileDialog => a(self, Action::OpenFiles),
            N::ToggleEqualizer => a(self, Action::ToggleEqWindow),
            N::TimeElapsed => self.prefs.time_remaining = false,
            N::TimeRemaining => self.prefs.time_remaining = true,
            N::TogglePlaylist => a(self, Action::TogglePlaylistWindow),
            N::About => a(self, Action::About),
            N::MainMenu => self.open_main_menu_at(el, id, 6, 13),
            N::Previous => p(self, Command::Previous),
            N::Play => p(self, Command::Play),
            N::Pause => p(self, Command::Pause),
            N::Stop => p(self, Command::Stop),
            N::Next => p(self, Command::Next),
            N::VolumeUp => p(self, Command::VolumeBy(4)),
            N::VolumeDown => p(self, Command::VolumeBy(-4)),
            N::Forward5s | N::FastForward5s => p(self, Command::SeekRelative(5000)),
            N::Rewind5s | N::Rewind5sAlt => p(self, Command::SeekRelative(-5000)),
            N::MainWindowshade => {
                self.prefs.shade = !self.prefs.shade;
                self.resize_window(Kind::Main);
            }
            N::WindowshadeFocused => k(self, KeyAction::ShadeFocused),
            N::StopWithFade => p(self, Command::StopWithFade),
            N::StartOfList => p(self, Command::StartOfList),
            N::OpenUrlDialog | N::OpenUrl => a(self, Action::OpenUrl),
            N::StopAfterCurrent => p(self, Command::ToggleStopAfterCurrent),
            N::EndOfList => p(self, Command::EndOfList),
            N::DoubleSize => a(self, Action::ToggleDoubleSize),
            N::EasyMove => k(self, KeyAction::ToggleEasyMove),
            N::OpenFolder => a(self, Action::OpenFolder),
            N::FileInfo => a(self, Action::FileInfoCurrent),
            N::JumpToTime => k(self, KeyAction::JumpToTime),
            N::JumpToFile => k(self, KeyAction::JumpToFile),
            N::TenTracksForward => p(self, Command::JumpEntries(10)),
            N::TenTracksBack => p(self, Command::JumpEntries(-10)),
            N::SkinBrowser => k(self, KeyAction::SkinBrowser),
            N::ToggleTimeDisplay => a(self, Action::ToggleTimeMode),
            N::ToggleMainWindow => k(self, KeyAction::ToggleMainWindow),
            N::PlaylistWindowshade => self.toggle_pl_shade(),
            N::ReloadSkin => k(self, KeyAction::ReloadSkin),
            N::BookmarkCurrent => k(self, KeyAction::BookmarkCurrent),
            N::ManualAdvance => k(self, KeyAction::ToggleManualAdvance),
            other => log::info!("numbered command not implemented yet: {other:?}"),
        }
    }
}
