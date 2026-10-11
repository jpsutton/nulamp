//! The video window inside the shell (05 §2, 09 §8): follows the heard track, keeps the
//! picture on the audio clock, draws the window, and handles its input and full screen.

use std::time::{Duration, Instant};

use nulamp_core::{Command, EntryId, PlayState};
use nulamp_remote::screensaver::ScreenSaverInhibit;
use nulamp_render::video_window::{self as vw, VideoButton};
use nulamp_render::{Frame, scale};
use nulamp_text::Direction;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::KeyCode;
use winit::window::{Fullscreen, WindowId};

use super::{DOUBLE_CLICK_MS, Kind, PlayerHandle, Shell};
use crate::video::{
    VideoAction, VideoCmd, VideoMeta, VideoSource, VideoStatus, VideoWindow, Zoom, fit,
    window_size_for,
};

/// The pointer hides after this long without moving over a full-screen video.
const HIDE_CURSOR_AFTER: Duration = Duration::from_secs(2);

pub(super) struct VideoHost {
    pub win: VideoWindow,
    pub fullscreen: bool,
    pub flip: bool,
    /// The heard entry and its picture source (`None`: not a video file, or no picture).
    current: Option<(EntryId, Option<Box<dyn VideoSource>>)>,
    /// The source's stream, once it is ready.
    meta: Option<VideoMeta>,
    /// Audio stream chosen in the menu for the current entry.
    audio_choice: Option<usize>,
    seq: u64,
    last_click: Option<(Instant, i32, i32)>,
    last_motion: Instant,
    cursor_hidden: bool,
    inhibit: Option<ScreenSaverInhibit>,
}

impl VideoHost {
    pub fn new(size: (i32, i32)) -> Self {
        Self {
            win: VideoWindow::new(size),
            fullscreen: false,
            flip: false,
            current: None,
            meta: None,
            audio_choice: None,
            seq: 0,
            last_click: None,
            last_motion: Instant::now(),
            cursor_hidden: false,
            inhibit: None,
        }
    }

    fn source(&self) -> Option<&dyn VideoSource> {
        self.current.as_ref()?.1.as_deref()
    }
}

impl<P: PlayerHandle> Shell<P> {
    fn video_window(&self) -> Option<&winit::window::Window> {
        let id = self.ids.get(&Kind::Video)?;
        self.windows.get(id)?.window.as_deref()
    }

    /// Follows the heard track: opens its picture, passes the clock on, and redraws when a
    /// new picture is ready. Called on every tick.
    pub(super) fn video_tick(&mut self, el: &ActiveEventLoop) {
        let s = self.last_status.clone();
        let heard = (s.state != PlayState::Stopped)
            .then(|| self.player.engine().playlist.get(s.position))
            .flatten()
            .filter(|_| self.prefs.video.enabled)
            .map(|e| (e.id, e.location.clone()));
        let same = match (&self.video.current, &heard) {
            (Some((a, _)), Some((b, _))) => a == b,
            (None, None) => true,
            _ => false,
        };
        if !same {
            let had_picture = self.video.meta.is_some();
            self.video.current = heard.map(|(id, loc)| (id, self.player.open_video(&loc)));
            self.video.meta = None;
            self.video.audio_choice = None;
            self.video.seq = 0;
            let has_source = self.video.source().is_some();
            if had_picture && !has_source {
                self.video_ended();
            }
            self.request_redraw(Kind::Video);
        }

        let status = self.video.source().map(|src| src.status());
        match status {
            Some(VideoStatus::Ready(mut m)) => {
                if let Some(c) = self.video.audio_choice {
                    m.audio_current = Some(c);
                }
                if self.video.meta.as_ref() != Some(&m) {
                    let first = self.video.meta.is_none();
                    self.video.meta = Some(m);
                    if first {
                        self.video_started(el);
                    }
                    self.request_redraw(Kind::Video);
                }
            }
            Some(VideoStatus::NoVideo) => {
                if let Some((_, src)) = &mut self.video.current {
                    *src = None;
                }
            }
            Some(VideoStatus::Opening) | None => {}
        }

        if self.video.meta.is_some() {
            let (_, _, w, h) = self.video_picture_rect();
            let flip = self.video.flip;
            if let Some(src) = self.video.source() {
                src.set_clock(s.position_ms, s.state == PlayState::Playing);
                src.set_output(w, h, flip);
                let seq = src.seq();
                if seq != self.video.seq {
                    self.video.seq = seq;
                    self.request_redraw(Kind::Video);
                }
            }
        }

        let keep_awake = self.prefs.video.inhibit_screensaver
            && s.state == PlayState::Playing
            && self.video.meta.is_some();
        if keep_awake != self.video.inhibit.is_some() {
            self.video.inhibit =
                keep_awake.then(|| ScreenSaverInhibit::new(super::APP_NAME, "Playing a video"));
        }

        if self.video.fullscreen
            && !self.video.cursor_hidden
            && self.video.last_motion.elapsed() >= HIDE_CURSOR_AFTER
            && let Some(w) = self.video_window()
        {
            w.set_cursor_visible(false);
            self.video.cursor_hidden = true;
        }
    }

    /// A video's picture became available (05 §2 preferences).
    fn video_started(&mut self, el: &ActiveEventLoop) {
        let p = self.prefs.video.clone();
        if p.open_on_start && !self.is_visible(Kind::Video) {
            self.video_show(true);
        }
        if p.resize_to_video && !self.video.fullscreen {
            self.video_cmd(el, VideoCmd::Zoom(Zoom::Normal));
        }
        if p.fullscreen_on_start && !self.video.fullscreen {
            self.video_set_fullscreen(true);
        }
    }

    /// Playback moved on from a video to something without a picture, or stopped.
    fn video_ended(&mut self) {
        if self.prefs.video.leave_fullscreen_on_stop && self.video.fullscreen {
            self.video_set_fullscreen(false);
        }
        if self.prefs.video.close_on_end && self.is_visible(Kind::Video) {
            self.video_show(false);
        }
    }

    /// Where the picture goes in the window's frame (physical pixels): x, y, width, height.
    fn video_picture_rect(&self) -> (i32, i32, u32, u32) {
        let (ax, ay, aw, ah) = if self.video.fullscreen {
            let size = self.video_window().map(|w| w.inner_size());
            size.map_or((0, 0, 0, 0), |s| (0, 0, s.width, s.height))
        } else {
            let s = self.scale_of(Kind::Video);
            let (x, y, w, h) = vw::video_rect(self.video.win.w, self.video.win.h);
            (x * s, y * s, (w * s).max(0) as u32, (h * s).max(0) as u32)
        };
        let Some(m) = &self.video.meta else {
            return (ax, ay, aw, ah);
        };
        let (fx, fy, fw, fh) = if self.prefs.video.lock_aspect {
            fit(self.prefs.video.aspect.unwrap_or(m.aspect), aw, ah)
        } else {
            (0, 0, aw, ah)
        };
        (ax + fx as i32, ay + fy as i32, fw, fh)
    }

    /// The window's frame at its physical size.
    pub(super) fn draw_video(&mut self, _id: WindowId) -> Frame {
        let mut f = if self.video.fullscreen {
            let size = self.video_window().map(|w| w.inner_size());
            let (w, h) = size.map_or((1, 1), |s| (s.width.max(1), s.height.max(1)));
            Frame::new(w, h, 0)
        } else {
            let view = self.video.win.view(self.is_active(Kind::Video));
            let mut chrome = vw::draw_video_frame(&self.skin, &view);
            self.draw_video_info(&mut chrome);
            scale(&chrome, self.scale_of(Kind::Video) as u32)
        };
        let area = self.video_picture_rect();
        let drawn = match (self.video.source(), &self.video.meta) {
            (Some(src), Some(_)) => src.blit(&mut f, area),
            _ => false,
        };
        if !drawn && self.video.source().is_none() && self.prefs.video.logo_when_idle {
            draw_logo(&mut f, area);
        }
        f
    }

    /// The title and video information in the bottom bar (05 §2), `mbFG` on `mbBG`.
    fn draw_video_info(&mut self, f: &mut Frame) {
        let (x, y, w, h) = vw::info_rect(self.video.win.w, self.video.win.h);
        if w <= 2 {
            return;
        }
        let text = match &self.video.meta {
            Some(m) => format!("{}  ({})", self.last_status.title, m.summary()),
            None if self.video.source().is_some() => self.last_status.title.clone(),
            None => String::new(),
        };
        if text.is_empty() {
            return;
        }
        let spec = crate::title::title_font(&self.skin, self.prefs.user_font.as_deref());
        let layout = self.fonts.layout_fit(&text, &spec, Direction::Auto, w - 2);
        let m = &layout.metrics;
        let baseline = y + (h - (m.ascent + m.descent)) / 2 + m.ascent;
        let color = self.skin.pledit.mb_fg;
        self.fonts
            .draw(f, &layout, x + 1, baseline, color, (x, y, x + w, y + h));
    }

    pub(super) fn video_mouse(
        &mut self,
        id: WindowId,
        state: ElementState,
        button: MouseButton,
        el: &ActiveEventLoop,
    ) {
        let Some((_, x, y)) = self.local_pos(id) else {
            return;
        };
        let double = if (button, state) == (MouseButton::Left, ElementState::Pressed) {
            let d = matches!(self.video.last_click, Some((t, lx, ly))
                if t.elapsed().as_millis() < DOUBLE_CLICK_MS && (lx - x).abs() <= 2 && (ly - y).abs() <= 2);
            self.video.last_click = if d {
                None
            } else {
                Some((Instant::now(), x, y))
            };
            d
        } else {
            false
        };
        let acts = if self.video.fullscreen {
            match (button, state) {
                (MouseButton::Left, ElementState::Pressed) if double => {
                    vec![VideoAction::ToggleFullScreen]
                }
                (MouseButton::Right, ElementState::Pressed) => {
                    vec![VideoAction::ContextMenu { x, y }]
                }
                _ => Vec::new(),
            }
        } else {
            match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    self.video.win.mouse_down(x, y, double)
                }
                (MouseButton::Left, ElementState::Released) => self.video.win.mouse_up(x, y),
                (MouseButton::Right, ElementState::Pressed) => self.video.win.right_click(x, y),
                _ => Vec::new(),
            }
        };
        self.video_actions(id, acts, el);
    }

    pub(super) fn video_mouse_move(&mut self, id: WindowId, x: i32, y: i32) {
        self.video.last_motion = Instant::now();
        if self.video.cursor_hidden {
            if let Some(w) = self.video_window() {
                w.set_cursor_visible(true);
            }
            self.video.cursor_hidden = false;
        }
        if self.video.fullscreen {
            return;
        }
        for a in self.video.win.mouse_move(x, y) {
            match a {
                VideoAction::Resize { w, h } => {
                    if self.video.win.set_size(w, h) {
                        self.resize_window(Kind::Video);
                    }
                }
                VideoAction::Redraw => self.request_redraw(Kind::Video),
                _ => {}
            }
        }
        let _ = id;
    }

    /// Returns true when the video window used the key.
    pub(super) fn video_key(&mut self, _id: WindowId, code: KeyCode, el: &ActiveEventLoop) -> bool {
        match crate::video::key(code, self.mods, self.video.fullscreen) {
            Some(c) => {
                self.video_cmd(el, c);
                true
            }
            None => false,
        }
    }

    fn video_actions(&mut self, id: WindowId, acts: Vec<VideoAction>, el: &ActiveEventLoop) {
        for a in acts {
            match a {
                VideoAction::Close => self.video_close(),
                VideoAction::BeginMove { x, y } => self.begin_drag(id, x, y),
                VideoAction::Resize { w, h } => {
                    if self.video.win.set_size(w, h) {
                        self.resize_window(Kind::Video);
                    }
                }
                VideoAction::Button { button, x, y } => match button {
                    VideoButton::FullScreen => self.video_cmd(el, VideoCmd::FullScreen),
                    VideoButton::Zoom100 => self.video_cmd(el, VideoCmd::Zoom(Zoom::Normal)),
                    VideoButton::Zoom200 => self.video_cmd(el, VideoCmd::Zoom(Zoom::Double)),
                    VideoButton::Misc => self.video_cmd(el, VideoCmd::Options),
                    VideoButton::Menu => self.video_menu(el, id, x, y),
                },
                VideoAction::ContextMenu { x, y } => self.video_menu(el, id, x, y),
                VideoAction::ToggleFullScreen => self.video_cmd(el, VideoCmd::FullScreen),
                VideoAction::Redraw => {}
            }
        }
        self.request_redraw(Kind::Video);
    }

    fn video_menu(&mut self, el: &ActiveEventLoop, id: WindowId, x: i32, y: i32) {
        let items = crate::video::context_menu(
            self.video.meta.as_ref(),
            self.video.fullscreen,
            self.video.flip,
        );
        self.open_items_at(el, id, items, x, y);
    }

    /// Video window menu and key commands.
    pub(super) fn video_cmd(&mut self, el: &ActiveEventLoop, cmd: VideoCmd) {
        match cmd {
            VideoCmd::FullScreen => {
                let on = !self.video.fullscreen;
                self.video_set_fullscreen(on);
            }
            VideoCmd::LeaveFullScreen => self.video_set_fullscreen(false),
            VideoCmd::Zoom(z) => {
                if self.video.fullscreen {
                    self.video_set_fullscreen(false);
                }
                if let Some(m) = &self.video.meta {
                    let (w, h) = m.display_size(self.prefs.video.aspect);
                    let (ww, wh) = window_size_for(w, h, z, self.scale_of(Kind::Video));
                    if self.video.win.set_size(ww, wh) {
                        self.resize_window(Kind::Video);
                    }
                }
            }
            VideoCmd::Flip => self.video.flip = !self.video.flip,
            VideoCmd::VideoTrack(i) => {
                if let Some(src) = self.video.source() {
                    src.select_stream(i);
                }
            }
            VideoCmd::AudioTrack(i) => {
                let entry = self.video.current.as_ref().map(|(id, _)| *id);
                let location = entry
                    .and_then(|id| self.player.engine().playlist.by_id(id))
                    .map(|e| e.location.clone());
                if let Some(loc) = location {
                    self.video.audio_choice = Some(i);
                    if let Some(m) = &mut self.video.meta {
                        m.audio_current = Some(i);
                    }
                    self.player.select_audio_track(&loc, i);
                }
            }
            VideoCmd::Options => {
                let d = crate::prefs_dialog::PrefsDialog::new(
                    self.prefs.clone(),
                    self.playlist_options(),
                )
                .with_page("Video")
                .wayland_native(self.wayland);
                self.open_dialog(el, Box::new(d));
            }
            VideoCmd::TogglePause => {
                let c = match self.last_status.state {
                    PlayState::Stopped => Command::Play,
                    _ => Command::Pause,
                };
                self.player.command(c);
                self.poll_player();
            }
        }
        self.request_redraw(Kind::Video);
    }

    /// Shows or hides the window (a hidden window leaves full screen).
    pub(super) fn video_show(&mut self, visible: bool) {
        if !visible && self.video.fullscreen {
            self.video_set_fullscreen(false);
        }
        self.set_visible(Kind::Video, visible);
    }

    /// The close button (or the window manager) closes the window; with "stop playback when
    /// the window is closed" a playing video stops too.
    pub(super) fn video_close(&mut self) {
        self.video_show(false);
        if self.prefs.video.stop_on_close && self.video.meta.is_some() {
            self.player.command(Command::Stop);
            self.poll_player();
        }
    }

    pub(super) fn video_set_fullscreen(&mut self, on: bool) {
        if on && !self.is_visible(Kind::Video) {
            self.set_visible(Kind::Video, true);
        }
        self.video.fullscreen = on;
        self.video_apply_fullscreen();
        if !on {
            if let Some(w) = self.video_window() {
                w.set_cursor_visible(true);
            }
            self.video.cursor_hidden = false;
            // Back to the window's own place and size.
            if let Some(&id) = self.ids.get(&Kind::Video)
                && let Some(w) = self.windows.get(&id)
                && let Some(win) = &w.window
            {
                let r = w.rect;
                let _ = win.request_inner_size(PhysicalSize::new(r.w as u32, r.h as u32));
                win.set_outer_position(PhysicalPosition::new(r.x, r.y));
            }
        }
        self.video.last_motion = Instant::now();
        if let Some(&id) = self.ids.get(&Kind::Video)
            && let Some(w) = self.windows.get_mut(&id)
        {
            w.shape_dirty = true;
        }
        self.request_redraw(Kind::Video);
    }

    /// Asks the window system for the full-screen state wanted (also after the window was
    /// opened again).
    pub(super) fn video_apply_fullscreen(&self) {
        if let Some(w) = self.video_window() {
            let want = self.video.fullscreen;
            if want != w.fullscreen().is_some() {
                w.set_fullscreen(want.then_some(Fullscreen::Borderless(None)));
            }
            if want {
                w.focus_window();
            }
        }
    }
}

/// The application icon centred in an area, when no video plays (05 §2 "show logo when
/// idle").
fn draw_logo(f: &mut Frame, area: (i32, i32, u32, u32)) {
    let (x, y, w, h) = area;
    let room = w.min(h) / 2;
    let Some(size) = [256u32, 128, 64, 48, 32, 16]
        .into_iter()
        .find(|s| *s <= room)
    else {
        return;
    };
    let img = nulamp_icon::icon(size);
    let ox = x + ((w - img.width) / 2) as i32;
    let oy = y + ((h - img.height) / 2) as i32;
    for (i, px) in img.rgba.as_chunks::<4>().0.iter().enumerate() {
        let a = px[3] as u32;
        if a == 0 {
            continue;
        }
        let bg = f.pixel(
            ox + (i as u32 % img.width) as i32,
            oy + (i as u32 / img.width) as i32,
        );
        let mix = |c: u8, shift: u32| {
            let b = (bg >> shift) & 0xFF;
            ((c as u32 * a + b * (255 - a)) / 255) << shift
        };
        let rgb = mix(px[0], 16) | mix(px[1], 8) | mix(px[2], 0);
        f.set(
            ox + (i as u32 % img.width) as i32,
            oy + (i as u32 / img.width) as i32,
            rgb,
        );
    }
}
