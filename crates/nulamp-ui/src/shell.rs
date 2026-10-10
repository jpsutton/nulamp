//! The windowing shell: owns the OS windows, routes input to window controllers, draws frames
//! and talks to the player through [`PlayerHandle`].

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use nulamp_core::{Command, PlayState, PlayerStatus};
use nulamp_eq::EqSettings;
use nulamp_render::eq_window::{self as ew, EqView};
use nulamp_render::main_window::{self as mw, MainView, TimeDisplay, TitleArea};
use nulamp_render::{Frame, region, scale};
use nulamp_skin::{CursorRole, CursorShape, Skin};
use nulamp_text::FontSystem;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Cursor, CursorIcon, CustomCursor, Window, WindowId, WindowLevel};

use crate::docking::{self, DEFAULT_SNAP};
use crate::eq_input::{EqAction, EqContext, EqInput};
use crate::geom::Rect;
use crate::input::{Action, Mods};
use crate::keys::{self, Focus, KeyAction};
use crate::main_input::{self, MainContext, MainInput};
use crate::marquee::{self, Marquee};
use crate::platform::shape;
use crate::playlist::{self as pl, PlAction, PlMenuChoice, PlaylistUi};
use crate::title;
use nulamp_render::playlist_window::{self as pw, MiniTime, PlFrameView};

/// The player as seen by the user interface.
pub trait PlayerHandle {
    fn status(&self) -> PlayerStatus;
    fn command(&mut self, command: Command);
    /// Called on every UI tick to process audio events.
    fn tick(&mut self) {}
    /// Equalizer settings changed.
    fn set_eq(&mut self, _settings: &EqSettings) {}
    /// The play-order engine and playlist.
    fn engine(&self) -> &nulamp_core::Engine;
    fn engine_mut(&mut self) -> &mut nulamp_core::Engine;
}

/// User-interface preferences (subset of 11 §1.3).
#[derive(Debug, Clone)]
pub struct UiPrefs {
    pub main_pos: (i32, i32),
    pub eq_pos: (i32, i32),
    pub pl_pos: (i32, i32),
    pub eq_visible: bool,
    pub pl_visible: bool,
    pub double_size: bool,
    /// Apply double size to the equalizer too (02 §9).
    pub eq_double: bool,
    pub shade: bool,
    pub eq_shade: bool,
    pub always_on_top: bool,
    pub easymove: bool,
    pub snap: bool,
    pub snap_distance: i32,
    pub keep_on_screen: bool,
    pub dim_inactive: bool,
    pub clutterbar_always: bool,
    /// Skinned bitmap font for the main-window title. Default on (docs/decisions.md D14);
    /// titles with characters the bitmap font cannot draw use the system font.
    pub bitmap_title: bool,
    pub show_number_in_title: bool,
    pub autoscroll: bool,
    pub time_remaining: bool,
    pub user_font: Option<String>,
    pub easter_egg_word: String,
    pub eq: EqSettings,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            main_pos: (26, 29),
            eq_pos: (26, 145),
            pl_pos: (26, 261),
            eq_visible: true,
            pl_visible: true,
            double_size: false,
            eq_double: true,
            shade: false,
            eq_shade: false,
            always_on_top: false,
            easymove: true,
            snap: true,
            snap_distance: DEFAULT_SNAP,
            keep_on_screen: true,
            dim_inactive: true,
            clutterbar_always: true,
            bitmap_title: true,
            show_number_in_title: true,
            autoscroll: true,
            time_remaining: false,
            user_font: None,
            easter_egg_word: "NULAMP".into(),
            eq: EqSettings::default(),
        }
    }
}

const APP_NAME: &str = "nulamp";
const DOUBLE_CLICK_MS: u128 = 450;
const TICK: Duration = Duration::from_millis(16);

/// The skinned windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Main,
    Equalizer,
    Playlist,
}

struct Win {
    kind: Kind,
    window: Rc<Window>,
    surface: softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>,
    /// Requested screen rectangle in physical pixels.
    rect: Rect,
    /// Last position reported by the window system.
    actual: (i32, i32),
    cursor: (i32, i32),
    shape_dirty: bool,
    visible: bool,
}

struct WindowDrag {
    leader: WindowId,
    grab: (i32, i32),
    group: Vec<WindowId>,
}

/// Visualizer mode (06 §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisMode {
    Off,
    Spectrum,
    Oscilloscope,
}

pub struct Shell<P: PlayerHandle> {
    player: P,
    prefs: UiPrefs,
    skin: Skin,
    fonts: FontSystem,
    context: Option<softbuffer::Context<OwnedDisplayHandle>>,
    windows: HashMap<WindowId, Win>,
    ids: HashMap<Kind, WindowId>,
    focused: Option<WindowId>,
    mods: Mods,
    main_input: MainInput,
    eq_input: EqInput,
    pl: PlaylistUi,
    middle_held: bool,
    marquee: Marquee,
    drag: Option<WindowDrag>,
    last_click: Option<(WindowId, Instant, i32, i32)>,
    start: Instant,
    cursors: HashMap<CursorRole, CustomCursor>,
    current_cursor: HashMap<WindowId, CursorRole>,
    vis_mode: VisMode,
    prev_vis_mode: VisMode,
    egg_typed: String,
    easter_egg: bool,
    last_status: PlayerStatus,
    marquee_dragging: bool,
    marquee_title_text: String,
    wheel_accum: f32,
    last_marquee_step: u64,
    last_blink: u64,
    exit: bool,
}

impl<P: PlayerHandle> Shell<P> {
    pub fn new(mut player: P, prefs: UiPrefs, skin: Skin) -> Self {
        player.set_eq(&prefs.eq);
        Self {
            last_status: player.status(),
            player,
            prefs,
            skin,
            fonts: FontSystem::new(),
            context: None,
            windows: HashMap::new(),
            ids: HashMap::new(),
            focused: None,
            mods: Mods::NONE,
            main_input: MainInput::new(),
            eq_input: EqInput::new(),
            pl: PlaylistUi::default(),
            middle_held: false,
            marquee: Marquee::new(),
            drag: None,
            last_click: None,
            start: Instant::now(),
            cursors: HashMap::new(),
            current_cursor: HashMap::new(),
            vis_mode: VisMode::Spectrum,
            prev_vis_mode: VisMode::Spectrum,
            egg_typed: String::new(),
            easter_egg: false,
            marquee_dragging: false,
            marquee_title_text: String::new(),
            wheel_accum: 0.0,
            last_marquee_step: 0,
            last_blink: 0,
            exit: false,
        }
    }

    /// Runs the event loop until the user exits.
    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        let event_loop = build_event_loop()?;
        event_loop.run_app(&mut self)?;
        Ok(())
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn kind_of(&self, id: WindowId) -> Option<Kind> {
        self.windows.get(&id).map(|w| w.kind)
    }

    /// Pixel scale of a window (double size applies to the main window and optionally the EQ).
    fn scale_of(&self, kind: Kind) -> i32 {
        let doubled = match kind {
            Kind::Main => self.prefs.double_size,
            Kind::Equalizer => self.prefs.double_size && self.prefs.eq_double,
            Kind::Playlist => false,
        };
        if doubled { 2 } else { 1 }
    }

    fn size_of(&self, kind: Kind) -> (i32, i32) {
        let (w, h) = match kind {
            Kind::Main if self.prefs.shade => mw::SHADE_SIZE,
            Kind::Main => mw::MAIN_SIZE,
            Kind::Equalizer if self.prefs.eq_shade => ew::EQ_SHADE_SIZE,
            Kind::Equalizer => ew::EQ_SIZE,
            Kind::Playlist => (
                self.pl.w as u32,
                if self.pl.shade { 14 } else { self.pl.h as u32 },
            ),
        };
        let s = self.scale_of(kind);
        (w as i32 * s, h as i32 * s)
    }

    fn create_window(&mut self, el: &ActiveEventLoop, kind: Kind, pos: (i32, i32), visible: bool) {
        let (w, h) = self.size_of(kind);
        let rect = Rect::new(pos.0, pos.1, w, h);
        let title = match kind {
            Kind::Main => APP_NAME.to_string(),
            Kind::Equalizer => format!("{APP_NAME} Equalizer"),
            Kind::Playlist => format!("{APP_NAME} Playlist Editor"),
        };
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(visible)
            .with_inner_size(PhysicalSize::new(rect.w as u32, rect.h as u32))
            .with_position(PhysicalPosition::new(rect.x, rect.y));
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use winit::platform::x11::{WindowAttributesExtX11, WindowType};
            attrs = attrs.with_name(APP_NAME, APP_NAME);
            if kind != Kind::Main {
                // Only the main window appears in the task bar (05 §6.5).
                attrs = attrs.with_x11_window_type(vec![WindowType::Utility]);
            }
        }
        if self.prefs.always_on_top {
            attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
        }
        let Ok(window) = el.create_window(attrs) else {
            log::error!("cannot create {kind:?} window");
            return;
        };
        let window = Rc::new(window);
        let context = self.context.get_or_insert_with(|| {
            softbuffer::Context::new(el.owned_display_handle()).expect("softbuffer context")
        });
        let Ok(surface) = softbuffer::Surface::new(context, window.clone()) else {
            return;
        };
        let id = window.id();
        self.windows.insert(
            id,
            Win {
                kind,
                window,
                surface,
                rect,
                actual: (rect.x, rect.y),
                cursor: (0, 0),
                shape_dirty: true,
                visible,
            },
        );
        self.ids.insert(kind, id);
    }

    fn main_context(&self) -> MainContext {
        let s = &self.last_status;
        let open = s.state != PlayState::Stopped;
        MainContext {
            playing: s.state == PlayState::Playing,
            open,
            seekable: s.seekable,
            length_ms: s.length_ms,
            seek_pos: seek_pos(s),
            volume: s.volume,
            balance: s.balance,
            shade: self.prefs.shade,
            easymove: self.prefs.easymove,
            clutterbar_shown: self.prefs.clutterbar_always,
        }
    }

    fn eq_values(&self) -> [u8; 11] {
        let mut v = [self.prefs.eq.preamp; 11];
        v[1..].copy_from_slice(&self.prefs.eq.bands);
        v
    }

    fn eq_context(&self) -> EqContext {
        EqContext {
            shade: self.prefs.eq_shade,
            easymove: self.prefs.easymove,
            values: self.eq_values(),
            iso: self.prefs.eq.iso,
        }
    }

    fn title_text(&self) -> String {
        let s = &self.last_status;
        if s.playlist_len == 0 && s.state == PlayState::Stopped {
            return format!("{} {}", APP_NAME, env!("CARGO_PKG_VERSION"));
        }
        let length = s.length_ms.map(|l| l / 1000);
        marquee::title_string(
            s.position,
            &s.title,
            length,
            self.prefs.show_number_in_title,
        )
    }

    /// Bitmap font when enabled and every character of the title is in `text.bmp`.
    fn use_bitmap_title(&self) -> bool {
        self.prefs.bitmap_title
            && self
                .marquee_title_text
                .chars()
                .all(nulamp_skin::fonts::text_can_draw)
    }

    fn measure_title(&mut self, text: &str) -> i32 {
        if self.use_bitmap_title() {
            text.chars().count() as i32 * 5
        } else {
            let spec = title::title_font(&self.skin, self.prefs.user_font.as_deref());
            self.fonts.measure(text, &spec)
        }
    }

    fn refresh_title(&mut self) {
        let text = self.title_text();
        self.marquee_title_text = text.clone();
        let now = self.now_ms();
        let w_title = self.measure_title(&text);
        let w_sep = self.measure_title(marquee::SEPARATOR);
        self.marquee.set_title(&text, now, |s| {
            if s == marquee::SEPARATOR {
                w_sep
            } else {
                w_title
            }
        });
        if let Some(w) = self
            .ids
            .get(&Kind::Main)
            .and_then(|id| self.windows.get(id))
        {
            let caption = if self.prefs.show_number_in_title && !self.last_status.title.is_empty() {
                format!(
                    "{}. {} - {}",
                    self.last_status.position + 1,
                    self.last_status.title,
                    APP_NAME
                )
            } else {
                APP_NAME.to_string()
            };
            w.window.set_title(&caption);
        }
    }

    fn is_active(&self, kind: Kind) -> bool {
        !self.prefs.dim_inactive || self.focused.and_then(|f| self.kind_of(f)) == Some(kind)
    }

    fn is_visible(&self, kind: Kind) -> bool {
        self.ids
            .get(&kind)
            .and_then(|id| self.windows.get(id))
            .is_some_and(|w| w.visible)
    }

    fn main_view(&mut self) -> MainView {
        let s = self.last_status.clone();
        let now = self.now_ms();
        let time = match s.state {
            PlayState::Stopped => TimeDisplay::Hidden,
            PlayState::Paused if (now / 1200) % 2 == 1 => TimeDisplay::Blank,
            _ => {
                let remaining = self.prefs.time_remaining && s.seekable && s.length_ms.is_some();
                let secs = if remaining {
                    s.length_ms.unwrap_or(0).saturating_sub(s.position_ms) / 1000
                } else {
                    s.position_ms / 1000
                };
                TimeDisplay::Time {
                    minutes: (secs / 60) as u32,
                    seconds: (secs % 60) as u32,
                    remaining,
                }
            }
        };
        let (text, offset) = self.marquee.visible();
        let title = if self.use_bitmap_title() {
            TitleArea::Bitmap { text, offset }
        } else {
            let spec = title::title_font(&self.skin, self.prefs.user_font.as_deref());
            TitleArea::Image(title::title_image(
                &mut self.fonts,
                &self.skin,
                &spec,
                &text,
                offset,
            ))
        };
        let open = s.state != PlayState::Stopped;
        MainView {
            active: self.is_active(Kind::Main),
            shade: self.prefs.shade,
            easter_egg: self.easter_egg,
            pressed: self.main_input.pressed,
            clutterbar_shown: self.prefs.clutterbar_always,
            clutterbar_pressed: self.main_input.clutter_pressed,
            always_on_top: self.prefs.always_on_top,
            double_size: self.prefs.double_size,
            state: match s.state {
                PlayState::Stopped => mw::PlayState::Stopped,
                PlayState::Playing => mw::PlayState::Playing,
                PlayState::Paused => mw::PlayState::Paused,
            },
            in_sync: s.in_sync,
            time,
            kbps: if open { s.kbps } else { None },
            khz: if open {
                s.sample_rate.map(|r| r / 1000)
            } else {
                None
            },
            channels: if open { s.channels } else { None },
            volume: s.volume,
            volume_pressed: self.main_input.volume_pressed,
            balance: s.balance,
            balance_pressed: self.main_input.balance_pressed,
            seek: (open && s.seekable && s.length_ms.is_some()).then(|| seek_pos(&s)),
            seek_pressed: self.main_input.seek_drag.is_some(),
            seek_drag: self.main_input.seek_drag,
            shuffle: s.shuffle,
            repeat: s.repeat,
            eq_visible: self.is_visible(Kind::Equalizer),
            pl_visible: self.is_visible(Kind::Playlist),
            title,
        }
    }

    fn eq_view(&self) -> EqView {
        EqView {
            active: self.is_active(Kind::Equalizer),
            shade: self.prefs.eq_shade,
            iso: self.prefs.eq.iso,
            enabled: self.prefs.eq.enabled,
            auto_load: self.prefs.eq.auto_load,
            values: self.eq_values(),
            pressed: self.eq_input.pressed,
            dragging: self.eq_input.dragging,
            volume: self.last_status.volume,
            balance: self.last_status.balance,
        }
    }

    fn redraw(&mut self, id: WindowId) {
        let Some(kind) = self.kind_of(id) else { return };
        let s = self.scale_of(kind) as u32;
        let frame = match kind {
            Kind::Main => {
                let view = self.main_view();
                scale(&mw::draw_main(&self.skin, &view), s)
            }
            Kind::Equalizer => scale(&ew::draw_eq(&self.skin, &self.eq_view()), s),
            Kind::Playlist => self.draw_playlist(),
        };
        let mask = self.shape_for(kind, &frame);
        let Some(w) = self.windows.get_mut(&id) else {
            return;
        };
        if w.shape_dirty {
            shape::apply(&*w.window, mask.as_ref());
            w.shape_dirty = false;
        }
        present(w, &frame);
    }

    fn shape_for(&self, kind: Kind, frame: &Frame) -> Option<region::Mask> {
        if self.skin.is_builtin {
            return None;
        }
        let r = &self.skin.regions;
        let polys = match kind {
            Kind::Main if self.prefs.shade => r.window_shade.as_ref(),
            Kind::Main => r.normal.as_ref(),
            Kind::Equalizer if self.prefs.eq_shade => r.equalizer_ws.as_ref(),
            Kind::Equalizer => r.equalizer.as_ref(),
            Kind::Playlist => None,
        }?;
        Some(region::rasterize(
            polys,
            frame.width(),
            frame.height(),
            self.scale_of(kind),
        ))
    }

    fn request_redraw_all(&self) {
        for w in self.windows.values().filter(|w| w.visible) {
            w.window.request_redraw();
        }
    }

    fn request_redraw(&self, kind: Kind) {
        if let Some(w) = self.ids.get(&kind).and_then(|id| self.windows.get(id)) {
            w.window.request_redraw();
        }
    }

    fn work_area(&self, id: WindowId) -> Option<Rect> {
        let w = self.windows.get(&id)?;
        let m = w.window.current_monitor()?;
        let (p, s) = (m.position(), m.size());
        Some(Rect::new(p.x, p.y, s.width as i32, s.height as i32))
    }

    fn set_rect(&mut self, id: WindowId, r: Rect) {
        if let Some(w) = self.windows.get_mut(&id) {
            if (w.rect.x, w.rect.y) != (r.x, r.y) {
                w.window.set_outer_position(PhysicalPosition::new(r.x, r.y));
            }
            if (w.rect.w, w.rect.h) != (r.w, r.h) {
                let _ = w
                    .window
                    .request_inner_size(PhysicalSize::new(r.w as u32, r.h as u32));
                w.shape_dirty = true;
            }
            w.rect = r;
        }
    }

    /// Visible windows with their rectangles, in a stable order.
    fn visible_rects(&self) -> (Vec<WindowId>, Vec<Rect>) {
        let mut ids: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, w)| w.visible)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_by_key(|id| self.windows[id].kind as u8);
        let rects = ids.iter().map(|i| self.windows[i].rect).collect();
        (ids, rects)
    }

    /// Applies a size change of one window and keeps docked windows attached (05 §6.3).
    fn resize_window(&mut self, kind: Kind) {
        let Some(&id) = self.ids.get(&kind) else {
            return;
        };
        let (w, h) = self.size_of(kind);
        let old = self.windows[&id].rect;
        let new = Rect::new(old.x, old.y, w, h);
        let (ids, mut rects) = self.visible_rects();
        match ids.iter().position(|i| *i == id) {
            Some(idx) => {
                let work = if self.prefs.keep_on_screen {
                    self.work_area(id)
                } else {
                    None
                };
                docking::resize_propagate(&mut rects, idx, old, new, work);
                for (i, r) in ids.into_iter().zip(rects) {
                    self.set_rect(i, r);
                }
            }
            None => self.set_rect(id, new),
        }
        self.request_redraw_all();
    }

    fn set_visible(&mut self, kind: Kind, visible: bool) {
        if let Some(w) = self.ids.get(&kind).and_then(|id| self.windows.get_mut(id)) {
            w.visible = visible;
            w.window.set_visible(visible);
            if visible {
                w.shape_dirty = true;
                w.window.request_redraw();
            }
        }
        match kind {
            Kind::Equalizer => self.prefs.eq_visible = visible,
            Kind::Playlist => self.prefs.pl_visible = visible,
            Kind::Main => {}
        }
        self.request_redraw(Kind::Main);
    }

    fn begin_drag(&mut self, id: WindowId, x: i32, y: i32) {
        let Some(kind) = self.kind_of(id) else { return };
        let s = self.scale_of(kind);
        let (ids, rects) = self.visible_rects();
        let snapping = self.prefs.snap != self.mods.shift;
        let group = match ids.iter().position(|i| *i == id) {
            Some(idx) if kind == Kind::Main && snapping => docking::attached_group(&rects, idx)
                .into_iter()
                .map(|i| ids[i])
                .collect(),
            _ => Vec::new(),
        };
        self.drag = Some(WindowDrag {
            leader: id,
            grab: (x * s, y * s),
            group,
        });
    }

    fn continue_drag(&mut self, id: WindowId) {
        let Some(drag) = &self.drag else { return };
        if drag.leader != id {
            return;
        }
        let Some(w) = self.windows.get(&id) else {
            return;
        };
        let screen = (w.actual.0 + w.cursor.0, w.actual.1 + w.cursor.1);
        let target = Rect::new(
            screen.0 - drag.grab.0,
            screen.1 - drag.grab.1,
            w.rect.w,
            w.rect.h,
        );
        let (ids, rects) = self.visible_rects();
        let Some(leader) = ids.iter().position(|i| *i == id) else {
            return;
        };
        let group: Vec<usize> = drag
            .group
            .iter()
            .filter_map(|g| ids.iter().position(|i| i == g))
            .collect();
        let snapping = self.prefs.snap != self.mods.shift;
        let dist = if snapping {
            self.prefs.snap_distance
        } else {
            -1
        };
        let work = if snapping && self.prefs.keep_on_screen {
            self.work_area(id)
        } else {
            None
        };
        let out = docking::move_group(&rects, leader, &group, target, work, dist);
        for (i, r) in ids.into_iter().zip(out) {
            self.set_rect(i, r);
        }
    }

    fn eq_changed(&mut self) {
        self.player.set_eq(&self.prefs.eq);
        self.request_redraw(Kind::Equalizer);
    }

    fn handle_eq_actions(&mut self, id: WindowId, actions: Vec<EqAction>, el: &ActiveEventLoop) {
        for a in actions {
            match a {
                EqAction::Do(a) => self.handle_action(id, a, el),
                EqAction::SetSlider(i, v) => {
                    if i == 0 {
                        self.prefs.eq.preamp = v;
                    } else {
                        self.prefs.eq.bands[i - 1] = v;
                    }
                    self.eq_changed();
                }
                EqAction::SetAllBands(v) => {
                    self.prefs.eq.bands = [v; 10];
                    self.eq_changed();
                }
                EqAction::ToggleEnabled => {
                    self.prefs.eq.enabled = !self.prefs.eq.enabled;
                    self.eq_changed();
                }
                EqAction::ToggleAutoLoad => {
                    self.prefs.eq.auto_load = !self.prefs.eq.auto_load;
                    self.eq_changed();
                }
                EqAction::CloseWindow => self.set_visible(Kind::Equalizer, false),
                EqAction::ToggleShade => {
                    self.prefs.eq_shade = !self.prefs.eq_shade;
                    self.resize_window(Kind::Equalizer);
                }
                other => log::info!("equalizer action not implemented yet: {other:?}"),
            }
        }
        self.request_redraw_all();
    }

    fn handle_actions(&mut self, id: WindowId, actions: Vec<Action>, el: &ActiveEventLoop) {
        for a in actions {
            self.handle_action(id, a, el);
        }
    }

    fn handle_action(&mut self, id: WindowId, a: Action, el: &ActiveEventLoop) {
        let now = self.now_ms();
        match a {
            Action::Player(c) => {
                self.player.command(c);
                self.poll_player();
            }
            Action::ToggleShuffle => {
                let on = !self.last_status.shuffle;
                self.player.command(Command::SetShuffle(on));
                self.marquee
                    .show_message(if on { "Shuffle: On" } else { "Shuffle: Off" }, now);
                self.poll_player();
            }
            Action::ToggleRepeat => {
                let s = &self.last_status;
                let on = !s.repeat;
                let msg = match (on, s.manual_advance) {
                    (false, false) => "Repeat: Off",
                    (true, false) => "Repeat: Playlist",
                    (true, true) => "Repeat: Track",
                    (false, true) => "Manual Advance: On",
                };
                self.player.command(Command::SetRepeat(on));
                self.marquee.show_message(msg, now);
                self.poll_player();
            }
            Action::ToggleEqWindow => {
                let v = !self.is_visible(Kind::Equalizer);
                self.set_visible(Kind::Equalizer, v);
            }
            Action::TogglePlaylistWindow => {
                let v = !self.is_visible(Kind::Playlist);
                self.set_visible(Kind::Playlist, v);
            }
            Action::ToggleTimeMode => self.prefs.time_remaining = !self.prefs.time_remaining,
            Action::CycleVisMode => {
                self.prev_vis_mode = self.vis_mode;
                self.vis_mode = match self.vis_mode {
                    VisMode::Off => VisMode::Spectrum,
                    VisMode::Spectrum => VisMode::Oscilloscope,
                    VisMode::Oscilloscope => VisMode::Off,
                };
            }
            Action::UndoVisMode => self.vis_mode = self.prev_vis_mode,
            Action::ToggleVisOffSpectrum => {
                self.vis_mode = if self.vis_mode == VisMode::Off {
                    VisMode::Spectrum
                } else {
                    VisMode::Off
                };
            }
            Action::ToggleAlwaysOnTop => {
                self.prefs.always_on_top = !self.prefs.always_on_top;
                let level = if self.prefs.always_on_top {
                    WindowLevel::AlwaysOnTop
                } else {
                    WindowLevel::Normal
                };
                for w in self.windows.values() {
                    w.window.set_window_level(level);
                }
            }
            Action::ToggleDoubleSize => {
                self.prefs.double_size = !self.prefs.double_size;
                self.resize_window(Kind::Main);
                self.resize_window(Kind::Equalizer);
            }
            Action::ToggleShade => match self.kind_of(id) {
                Some(Kind::Playlist) => self.toggle_pl_shade(),
                Some(Kind::Equalizer) => {
                    self.prefs.eq_shade = !self.prefs.eq_shade;
                    self.resize_window(Kind::Equalizer);
                }
                _ => {
                    self.prefs.shade = !self.prefs.shade;
                    self.resize_window(Kind::Main);
                }
            },
            Action::Minimize { .. } => {
                for w in self.windows.values() {
                    w.window.set_minimized(true);
                }
            }
            Action::Exit => {
                self.exit = true;
                el.exit();
            }
            Action::HideMainWindow => self.set_visible(Kind::Main, false),
            Action::BeginWindowDrag { x, y } => self.begin_drag(id, x, y),
            Action::Message(m) => self.marquee.show_message(m, now),
            Action::DragMessage(m) => self.marquee.show_drag_message(m),
            Action::ReleaseMessage => self.marquee.release(now),
            Action::TitleDrag { x } => {
                if self.marquee_dragging {
                    self.marquee.drag_move(x);
                } else {
                    self.marquee.drag_start(x);
                    self.marquee_dragging = true;
                }
            }
            Action::TitleDragEnd => {
                self.marquee.drag_end(now);
                self.marquee_dragging = false;
            }
            other => log::info!("action not implemented yet: {other:?}"),
        }
        self.request_redraw_all();
    }

    fn pl_font(&self) -> nulamp_text::FontSpec {
        let family = self
            .prefs
            .user_font
            .clone()
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| self.skin.pledit.font.clone());
        nulamp_text::FontSpec::new(family, self.pl.font_px)
    }

    fn update_pl_font(&mut self) {
        let spec = self.pl_font();
        self.pl.row_h = self.fonts.metrics(&spec).line_height.max(1);
        let n = self.player.engine().playlist.len();
        self.pl.clamp_scroll(n);
    }

    fn draw_playlist(&mut self) -> Frame {
        let s = &self.last_status;
        let time = match s.state {
            PlayState::Stopped => MiniTime::Hidden,
            _ => {
                let remaining = self.prefs.time_remaining && s.seekable && s.length_ms.is_some();
                let secs = if remaining {
                    s.length_ms.unwrap_or(0).saturating_sub(s.position_ms) / 1000
                } else {
                    s.position_ms / 1000
                };
                MiniTime::Time {
                    minutes: (secs / 60) as u32,
                    seconds: (secs % 60) as u32,
                    remaining,
                }
            }
        };
        let engine = self.player.engine();
        let view = PlFrameView {
            w: self.pl.w,
            h: if self.pl.shade { 14 } else { self.pl.h },
            active: self.is_active(Kind::Playlist),
            shade: self.pl.shade,
            pressed: self.pl.pressed,
            thumb: self.pl.thumb_offset(engine.playlist.len()),
            thumb_pressed: self.pl.thumb_pressed,
            info: pl::info_string(&engine.playlist),
            time,
            popup: self.pl.popup,
        };
        let mut f = pw::draw_playlist_frame(&self.skin, &view);
        if self.pl.shade {
            let i = self.last_status.position;
            let title = if engine.playlist.is_empty() {
                "<no file>".to_string()
            } else {
                format!(
                    "{}. {}",
                    i + 1,
                    engine
                        .playlist
                        .get(i)
                        .map(|e| e.title.as_str())
                        .unwrap_or("")
                )
            };
            let len = engine.playlist.get(i).and_then(|e| e.length_ms);
            let time = len
                .map(|l| marquee::format_length(l / 1000))
                .unwrap_or_default();
            pw::draw_shade_text_bitmap(&mut f, &self.skin, self.pl.w, &title, &time);
        } else {
            let spec = self.pl_font();
            pl::draw_rows(
                &mut f,
                &mut self.fonts,
                &self.skin,
                &spec,
                &self.pl,
                self.player.engine(),
            );
        }
        f
    }

    fn toggle_pl_shade(&mut self) {
        if self.pl.shade {
            self.pl.shade = false;
            self.pl.h = self.pl.unshaded_h.max(116);
        } else {
            self.pl.unshaded_h = self.pl.h;
            self.pl.shade = true;
        }
        self.resize_window(Kind::Playlist);
    }

    fn handle_pl_actions(&mut self, id: WindowId, actions: Vec<PlAction>, el: &ActiveEventLoop) {
        for a in actions {
            match a {
                PlAction::Do(a) => self.handle_action(id, a, el),
                PlAction::Close => self.set_visible(Kind::Playlist, false),
                PlAction::ToggleShade => self.toggle_pl_shade(),
                PlAction::Resize { w, h } => {
                    let (sw, sh) = pw::snap_size(w, if self.pl.shade { self.pl.h } else { h });
                    if (sw, sh) != (self.pl.w, self.pl.h) {
                        self.pl.w = sw;
                        if !self.pl.shade {
                            self.pl.h = sh;
                        }
                        let n = self.player.engine().playlist.len();
                        self.pl.clamp_scroll(n);
                        self.resize_window(Kind::Playlist);
                    }
                }
                PlAction::FontBigger => {
                    self.pl.font_px += 1;
                    self.update_pl_font();
                }
                PlAction::FontSmaller => {
                    self.pl.font_px = (self.pl.font_px - 1).max(2);
                    self.update_pl_font();
                }
                PlAction::FontReset => {
                    self.pl.font_px = 12;
                    self.update_pl_font();
                }
                PlAction::Menu(choice) => self.pl_menu_choice(choice),
                other => log::info!("playlist action not implemented yet: {other:?}"),
            }
        }
        self.poll_player();
        self.request_redraw_all();
    }

    fn pl_menu_choice(&mut self, choice: PlMenuChoice) {
        let e = self.player.engine_mut();
        match choice {
            PlMenuChoice::SelectAll => e.playlist.select_all(true),
            PlMenuChoice::SelectNone => e.playlist.select_all(false),
            PlMenuChoice::InvertSelection => e.playlist.invert_selection(),
            PlMenuChoice::RemoveSelected => {
                e.remove_where(|_, x| x.selected);
            }
            PlMenuChoice::Crop => {
                if e.playlist.selected_count() > 0 {
                    e.remove_where(|_, x| !x.selected);
                }
            }
            PlMenuChoice::RemoveAll | PlMenuChoice::NewList => e.clear(),
            other => log::info!("playlist menu choice not implemented yet: {other:?}"),
        }
        let n = self.player.engine().playlist.len();
        self.pl.clamp_scroll(n);
    }

    fn poll_player(&mut self) {
        self.player.tick();
        let s = self.player.status();
        let title_changed = s.title != self.last_status.title
            || s.position != self.last_status.position
            || s.length_ms != self.last_status.length_ms
            || s.playlist_len != self.last_status.playlist_len
            || s.state != self.last_status.state;
        self.last_status = s;
        if title_changed {
            self.refresh_title();
        }
    }

    fn cursor_role(&self, id: WindowId) -> Option<CursorRole> {
        let w = self.windows.get(&id)?;
        let s = self.scale_of(w.kind);
        let (x, y) = (w.cursor.0 / s, w.cursor.1 / s);
        Some(match w.kind {
            Kind::Main => main_input::cursor(x, y, self.prefs.shade),
            Kind::Equalizer => eq_cursor(x, y, self.prefs.eq_shade),
            Kind::Playlist => pl_cursor(x, y, self.pl.w, self.pl.h, self.pl.shade),
        })
    }

    fn update_cursor(&mut self, id: WindowId, el: &ActiveEventLoop) {
        let Some(role) = self.cursor_role(id) else {
            return;
        };
        if self.current_cursor.get(&id) == Some(&role) {
            return;
        }
        self.current_cursor.insert(id, role);
        let cursor: Cursor = match self.skin.cursors.get(&role) {
            Some(img) => {
                let custom = self.cursors.entry(role).or_insert_with(|| {
                    let rgba: Vec<u8> = img
                        .argb
                        .iter()
                        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8])
                        .collect();
                    let src = CustomCursor::from_rgba(
                        rgba,
                        img.width as u16,
                        img.height as u16,
                        img.hotspot.0 as u16,
                        img.hotspot.1 as u16,
                    )
                    .expect("valid cursor image");
                    el.create_custom_cursor(src)
                });
                custom.clone().into()
            }
            None => builtin_cursor(role.shape()).into(),
        };
        if let Some(w) = self.windows.get(&id) {
            w.window.set_cursor(cursor);
        }
    }

    fn local_pos(&self, id: WindowId) -> Option<(Kind, i32, i32)> {
        let w = self.windows.get(&id)?;
        let s = self.scale_of(w.kind);
        Some((w.kind, w.cursor.0 / s, w.cursor.1 / s))
    }

    fn mouse_input(
        &mut self,
        id: WindowId,
        state: ElementState,
        button: MouseButton,
        el: &ActiveEventLoop,
    ) {
        let Some((kind, x, y)) = self.local_pos(id) else {
            return;
        };
        let mods = self.mods;
        let now = self.now_ms();
        match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => {
                let double = matches!(self.last_click, Some((lid, t, lx, ly))
                    if lid == id && t.elapsed().as_millis() < DOUBLE_CLICK_MS && (lx - x).abs() <= 2 && (ly - y).abs() <= 2);
                self.last_click = if double {
                    None
                } else {
                    Some((id, Instant::now(), x, y))
                };
                match kind {
                    Kind::Main => {
                        let ctx = self.main_context();
                        let actions = match double
                            .then(|| self.main_input.double_click(x, y, &ctx))
                            .flatten()
                        {
                            Some(a) => a,
                            None => self.main_input.mouse_down(x, y, mods, &ctx, now),
                        };
                        self.handle_actions(id, actions, el);
                    }
                    Kind::Equalizer => {
                        let ctx = self.eq_context();
                        let actions =
                            match double.then(|| self.eq_input.double_click(x, y)).flatten() {
                                Some(a) => a,
                                None => self.eq_input.mouse_down(x, y, mods, &ctx),
                            };
                        self.handle_eq_actions(id, actions, el);
                    }
                    Kind::Playlist => {
                        let dbl = if double {
                            self.pl.double_click(x, y, self.player.engine_mut())
                        } else {
                            None
                        };
                        let actions = match dbl {
                            Some(a) => a,
                            None => self.pl.mouse_down(x, y, mods, self.player.engine_mut()),
                        };
                        self.handle_pl_actions(id, actions, el);
                    }
                }
            }
            (MouseButton::Left, ElementState::Released) => {
                self.drag = None;
                match kind {
                    Kind::Main => {
                        let ctx = self.main_context();
                        let actions = self.main_input.mouse_up(x, y, mods, &ctx);
                        self.handle_actions(id, actions, el);
                    }
                    Kind::Equalizer => {
                        let actions = self.eq_input.mouse_up(x, y, mods);
                        self.handle_eq_actions(id, actions, el);
                    }
                    Kind::Playlist => {
                        let actions = self.pl.mouse_up(x, y, mods, self.player.engine_mut());
                        self.handle_pl_actions(id, actions, el);
                    }
                }
            }
            (MouseButton::Right, ElementState::Pressed) => match kind {
                Kind::Main => {
                    let ctx = self.main_context();
                    if let Some(a) = self.main_input.right_click(x, y, &ctx) {
                        self.handle_action(id, a, el);
                    }
                }
                Kind::Equalizer => {
                    let ctx = self.eq_context();
                    let a = self.eq_input.right_click(x, y, &ctx);
                    self.handle_eq_actions(id, vec![a], el);
                }
                Kind::Playlist => {
                    let actions = self.pl.right_click(x, y, mods, self.player.engine_mut());
                    self.handle_pl_actions(id, actions, el);
                }
            },
            (MouseButton::Middle, st) => self.middle_held = st == ElementState::Pressed,
            _ => {}
        }
        self.request_redraw_all();
    }

    fn cursor_moved(&mut self, id: WindowId, el: &ActiveEventLoop) {
        if self.drag.is_some() {
            self.continue_drag(id);
            return;
        }
        let Some((kind, x, y)) = self.local_pos(id) else {
            return;
        };
        let mods = self.mods;
        match kind {
            Kind::Main => {
                let ctx = self.main_context();
                let acts = self.main_input.mouse_move(x, y, mods, &ctx);
                if !acts.is_empty() || self.main_input.is_dragging() {
                    self.handle_actions(id, acts, el);
                }
            }
            Kind::Equalizer => {
                let ctx = self.eq_context();
                let acts = self.eq_input.mouse_move(x, y, mods, &ctx);
                if !acts.is_empty() || self.eq_input.is_dragging() {
                    self.handle_eq_actions(id, acts, el);
                }
            }
            Kind::Playlist => {
                let dragging = self.pl.is_dragging() || self.pl.popup.is_some();
                let acts = self.pl.mouse_move(x, y, mods, self.player.engine_mut());
                if !acts.is_empty() || dragging {
                    self.handle_pl_actions(id, acts, el);
                }
            }
        }
        self.update_cursor(id, el);
    }

    fn key_input(&mut self, id: WindowId, event: winit::event::KeyEvent, el: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }
        let kind = self.kind_of(id).unwrap_or(Kind::Main);
        // Easter egg: typing the word in the main window toggles the alternate title strip.
        if kind == Kind::Main
            && let Some(text) = event.text.as_ref()
        {
            for c in text.chars() {
                self.egg_typed.push(c.to_ascii_uppercase());
            }
            let word = self.prefs.easter_egg_word.clone();
            if self.egg_typed.ends_with(&word) {
                self.easter_egg = !self.easter_egg;
                self.egg_typed.clear();
            }
            if self.egg_typed.len() > 32 {
                self.egg_typed.drain(..16);
            }
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        if kind == Kind::Equalizer && self.eq_key(id, code, el) {
            self.request_redraw_all();
            return;
        }
        if kind == Kind::Playlist {
            let mods = self.mods;
            if let Some(acts) = self.pl.key(code, mods, self.player.engine_mut()) {
                self.handle_pl_actions(id, acts, el);
                return;
            }
        }
        let playing = self.last_status.state == PlayState::Playing;
        let focus = match kind {
            Kind::Main => Focus::Main,
            Kind::Equalizer => Focus::Equalizer,
            Kind::Playlist => Focus::Playlist,
        };
        match keys::resolve(focus, code, self.mods, playing) {
            Some(KeyAction::Do(a)) => self.handle_action(id, a, el),
            Some(KeyAction::ShadeFocused) => self.handle_action(id, Action::ToggleShade, el),
            Some(KeyAction::ToggleEasyMove) => self.prefs.easymove = !self.prefs.easymove,
            Some(KeyAction::ToggleMainWindow) => {
                let v = !self.is_visible(Kind::Main);
                self.set_visible(Kind::Main, v);
            }
            Some(KeyAction::ToggleManualAdvance) => {
                let on = !self.last_status.manual_advance;
                self.player.command(Command::SetManualAdvance(on));
                let now = self.now_ms();
                self.marquee.show_message(
                    if on {
                        "Manual Advance: On"
                    } else {
                        "Manual Advance: Off"
                    },
                    now,
                );
                self.poll_player();
            }
            Some(other) => log::info!("key action not implemented yet: {other:?}"),
            None => {}
        }
        if matches!(
            code,
            KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::Numpad8 | KeyCode::Numpad2
        ) {
            let now = self.now_ms();
            self.marquee
                .show_message(marquee::volume_message(self.last_status.volume), now);
        }
        self.request_redraw_all();
    }

    /// Equalizer keys (03 §1.7). Returns true when handled.
    fn eq_key(&mut self, id: WindowId, code: KeyCode, el: &ActiveEventLoop) -> bool {
        use KeyCode::*;
        let m = self.mods;
        if m.ctrl || m.alt {
            if m.ctrl && code == F4 {
                self.set_visible(Kind::Equalizer, false);
                return true;
            }
            return false;
        }
        let raise = [
            Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9, Digit0,
        ];
        let lower = [KeyQ, KeyW, KeyE, KeyR, KeyT, KeyY, KeyU, KeyI, KeyO, KeyP];
        let step = |v: u8, up: bool| {
            if up {
                v.saturating_sub(2)
            } else {
                (v + 2).min(63)
            }
        };
        if let Some(i) = raise.iter().position(|k| *k == code) {
            self.prefs.eq.bands[i] = step(self.prefs.eq.bands[i], true);
        } else if let Some(i) = lower.iter().position(|k| *k == code) {
            self.prefs.eq.bands[i] = step(self.prefs.eq.bands[i], false);
        } else if code == Backquote {
            self.prefs.eq.preamp = step(self.prefs.eq.preamp, true);
        } else if code == Tab {
            self.prefs.eq.preamp = step(self.prefs.eq.preamp, false);
        } else if code == KeyN {
            self.prefs.eq.enabled = !self.prefs.eq.enabled;
        } else if code == KeyA {
            self.prefs.eq.auto_load = !self.prefs.eq.auto_load;
        } else if code == ArrowLeft || code == ArrowRight {
            let mut b = self.last_status.balance as i32 + if code == ArrowLeft { -4 } else { 4 };
            if b.abs() < 20 && b.signum() != (self.last_status.balance as i32).signum() {
                b = 0;
            }
            let b = b.clamp(-127, 127) as i8;
            self.handle_action(id, Action::Player(Command::SetBalance(b)), el);
            let now = self.now_ms();
            self.marquee.show_message(marquee::balance_message(b), now);
            return true;
        } else {
            return false;
        }
        self.eq_changed();
        true
    }

    fn wheel(&mut self, id: WindowId, delta: MouseScrollDelta, el: &ActiveEventLoop) {
        let notches = match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
        };
        if notches.signum() != self.wheel_accum.signum() && self.wheel_accum != 0.0 {
            self.wheel_accum = 0.0;
        }
        self.wheel_accum += notches;
        let whole = self.wheel_accum.trunc() as i32;
        if whole != 0 && self.kind_of(id) == Some(Kind::Playlist) {
            self.wheel_accum -= whole as f32;
            let middle = self.middle_held;
            let acts = self.pl.wheel(whole, middle, self.player.engine_mut());
            self.handle_pl_actions(id, acts, el);
            return;
        }
        if whole != 0 {
            self.wheel_accum -= whole as f32;
            self.handle_action(id, Action::Player(Command::VolumeBy(whole * 8)), el);
            let now = self.now_ms();
            self.marquee
                .show_message(marquee::volume_message(self.last_status.volume), now);
        }
    }
}

/// Equalizer cursor areas (01 §7).
fn eq_cursor(x: i32, y: i32, shade: bool) -> CursorRole {
    use crate::geom::hit;
    match () {
        _ if hit(x, y, 264, 273, 3, 12) => CursorRole::EqClose,
        _ if y < 14 => CursorRole::EqTitle,
        _ if !shade
            && nulamp_render::eq_window::SLIDER_X
                .iter()
                .any(|&sx| x >= sx && x <= sx + 12)
            && (39..=98).contains(&y) =>
        {
            CursorRole::EqSlid
        }
        _ => CursorRole::EqNormal,
    }
}

/// Playlist cursor areas (04 §7.5).
fn pl_cursor(x: i32, y: i32, w: i32, h: i32, shade: bool) -> CursorRole {
    use crate::geom::hit;
    if shade {
        return match () {
            _ if hit(x, y, w - 21, w - 13, 3, 12) => CursorRole::PWinBut,
            _ if hit(x, y, w - 11, w - 3, 3, 12) => CursorRole::PClose,
            _ if x >= w - 29 && x <= w - 20 => CursorRole::PwsSize,
            _ => CursorRole::PwsNorm,
        };
    }
    match () {
        _ if hit(x, y, w - 21, w - 13, 3, 12) => CursorRole::PWinBut,
        _ if hit(x, y, w - 11, w - 3, 3, 12) => CursorRole::PClose,
        _ if y <= 13 => CursorRole::PtBar,
        _ if hit(x, y, w - 15, w - 7, 20, h - 38) => CursorRole::PvScroll,
        _ if x >= w - 20 && y >= h - 20 => CursorRole::PSize,
        _ => CursorRole::PNormal,
    }
}

fn seek_pos(s: &PlayerStatus) -> u32 {
    match s.length_ms {
        Some(l) if l > 0 => (s.position_ms.min(l) * 256 / l) as u32,
        _ => 0,
    }
}

fn builtin_cursor(shape: CursorShape) -> CursorIcon {
    match shape {
        CursorShape::Arrow => CursorIcon::Default,
        CursorShape::Move => CursorIcon::Move,
        CursorShape::Close => CursorIcon::Pointer,
        CursorShape::HorizontalResize => CursorIcon::EwResize,
        CursorShape::VerticalResize => CursorIcon::NsResize,
        CursorShape::DiagonalResize => CursorIcon::NwseResize,
    }
}

fn present(w: &mut Win, frame: &Frame) {
    let size = w.window.inner_size();
    let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
    else {
        return;
    };
    if w.surface.resize(width, height).is_err() {
        return;
    }
    let Ok(mut buf) = w.surface.buffer_mut() else {
        return;
    };
    let (bw, bh) = (size.width as usize, size.height as usize);
    for y in 0..bh {
        for x in 0..bw {
            buf[y * bw + x] = frame.pixel(x as i32, y as i32);
        }
    }
    let _ = buf.present();
}

fn build_event_loop() -> Result<EventLoop<()>, winit::error::EventLoopError> {
    let mut builder = EventLoop::builder();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Classic docking needs windows that can position themselves: use X11 (also through
        // XWayland) unless native Wayland is requested (docs/decisions.md D5).
        if std::env::var_os("NULAMP_WAYLAND").is_none() {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }
    }
    builder.build()
}

impl<P: PlayerHandle> ApplicationHandler for Shell<P> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if !self.ids.is_empty() {
            return;
        }
        let p = self.prefs.clone();
        self.update_pl_font();
        self.create_window(el, Kind::Playlist, p.pl_pos, p.pl_visible);
        self.create_window(el, Kind::Equalizer, p.eq_pos, p.eq_visible);
        self.create_window(el, Kind::Main, p.main_pos, true);
        self.poll_player();
        self.refresh_title();
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TICK));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.exit = true;
                el.exit();
            }
            WindowEvent::RedrawRequested => self.redraw(id),
            WindowEvent::Moved(p) => {
                if let Some(w) = self.windows.get_mut(&id) {
                    w.actual = (p.x, p.y);
                }
            }
            WindowEvent::Resized(_) => {
                if let Some(w) = self.windows.get_mut(&id) {
                    w.shape_dirty = true;
                    w.window.request_redraw();
                }
            }
            WindowEvent::Focused(f) => {
                if f {
                    self.focused = Some(id);
                } else if self.focused == Some(id) {
                    self.focused = None;
                }
                self.request_redraw_all();
            }
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.mods = Mods {
                    shift: s.contains(ModifiersState::SHIFT),
                    ctrl: s.contains(ModifiersState::CONTROL),
                    alt: s.contains(ModifiersState::ALT),
                };
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(w) = self.windows.get_mut(&id) {
                    w.cursor = (position.x.floor() as i32, position.y.floor() as i32);
                }
                self.cursor_moved(id, el);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_input(id, state, button, el)
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(id, delta, el),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => self.key_input(id, event, el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.exit {
            return;
        }
        let now = self.now_ms();
        let before = self.last_status.clone();
        self.poll_player();
        let mut dirty = self.last_status != before;
        let autoscroll = self.prefs.autoscroll && !self.prefs.shade;
        if now / marquee::STEP_MS != self.last_marquee_step {
            self.last_marquee_step = now / marquee::STEP_MS;
            dirty |= self.marquee.tick(now, autoscroll);
        }
        if let Some(&id) = self.ids.get(&Kind::Main) {
            let acts = self.main_input.tick(now);
            if !acts.is_empty() {
                self.handle_actions(id, acts, el);
                dirty = true;
            }
        }
        if self.last_status.state == PlayState::Paused && now / 1200 != self.last_blink {
            self.last_blink = now / 1200;
            dirty = true;
        }
        if dirty {
            self.request_redraw(Kind::Main);
            self.request_redraw(Kind::Playlist);
            if self.prefs.eq_shade {
                self.request_redraw(Kind::Equalizer);
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TICK));
    }
}
