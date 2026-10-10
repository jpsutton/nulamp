//! The windowing shell: owns the OS windows, routes input to window controllers, draws frames
//! and talks to the player through [`PlayerHandle`].

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use nulamp_core::{Command, PlayState, PlayerStatus};
use nulamp_render::main_window::{self as mw, MainView, TimeDisplay, TitleArea};
use nulamp_render::{Frame, region, scale};
use nulamp_skin::{CursorRole, CursorShape, Skin};
use nulamp_text::FontSystem;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::{Cursor, CursorIcon, CustomCursor, Window, WindowId, WindowLevel};

use crate::docking::{self, DEFAULT_SNAP};
use crate::geom::Rect;
use crate::input::{Action, Mods};
use crate::keys::{self, Focus, KeyAction};
use crate::main_input::{self, MainContext, MainInput};
use crate::marquee::{self, Marquee};
use crate::platform::shape;
use crate::title;

/// The player as seen by the user interface.
pub trait PlayerHandle {
    fn status(&self) -> PlayerStatus;
    fn command(&mut self, command: Command);
}

/// User-interface preferences (subset of 11 §1.3).
#[derive(Debug, Clone)]
pub struct UiPrefs {
    pub main_pos: (i32, i32),
    pub double_size: bool,
    pub shade: bool,
    pub always_on_top: bool,
    pub easymove: bool,
    pub snap: bool,
    pub snap_distance: i32,
    pub keep_on_screen: bool,
    pub dim_inactive: bool,
    pub clutterbar_always: bool,
    pub bitmap_title: bool,
    pub show_number_in_title: bool,
    pub autoscroll: bool,
    pub time_remaining: bool,
    pub user_font: Option<String>,
    pub easter_egg_word: String,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            main_pos: (26, 29),
            double_size: false,
            shade: false,
            always_on_top: false,
            easymove: true,
            snap: true,
            snap_distance: DEFAULT_SNAP,
            keep_on_screen: true,
            dim_inactive: true,
            clutterbar_always: true,
            bitmap_title: false,
            show_number_in_title: true,
            autoscroll: true,
            time_remaining: false,
            user_font: None,
            easter_egg_word: "NULAMP".into(),
        }
    }
}

const APP_NAME: &str = "nulamp";
const DOUBLE_CLICK_MS: u128 = 450;
const TICK: Duration = Duration::from_millis(16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Main,
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
    main_id: Option<WindowId>,
    focused: Option<WindowId>,
    mods: Mods,
    main_input: MainInput,
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
    wheel_accum: f32,
    last_marquee_step: u64,
    last_blink: u64,
    exit: bool,
}

impl<P: PlayerHandle> Shell<P> {
    pub fn new(player: P, prefs: UiPrefs, skin: Skin) -> Self {
        Self {
            last_status: player.status(),
            player,
            prefs,
            skin,
            fonts: FontSystem::new(),
            context: None,
            windows: HashMap::new(),
            main_id: None,
            focused: None,
            mods: Mods::NONE,
            main_input: MainInput::new(),
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

    fn scale(&self) -> i32 {
        if self.prefs.double_size { 2 } else { 1 }
    }

    fn main_size(&self) -> (i32, i32) {
        let (w, h) = if self.prefs.shade {
            mw::SHADE_SIZE
        } else {
            mw::MAIN_SIZE
        };
        (w as i32 * self.scale(), h as i32 * self.scale())
    }

    fn create_window(
        &mut self,
        el: &ActiveEventLoop,
        kind: Kind,
        rect: Rect,
        title: &str,
    ) -> Option<WindowId> {
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(PhysicalSize::new(rect.w as u32, rect.h as u32))
            .with_position(PhysicalPosition::new(rect.x, rect.y));
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use winit::platform::x11::WindowAttributesExtX11;
            attrs = attrs.with_name(APP_NAME, APP_NAME);
        }
        if self.prefs.always_on_top {
            attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
        }
        let window = Rc::new(el.create_window(attrs).ok()?);
        let context = self.context.get_or_insert_with(|| {
            softbuffer::Context::new(el.owned_display_handle()).expect("softbuffer context")
        });
        let surface = softbuffer::Surface::new(context, window.clone()).ok()?;
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
            },
        );
        Some(id)
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

    fn measure_title(&mut self, text: &str) -> i32 {
        if self.prefs.bitmap_title {
            text.chars().count() as i32 * 5
        } else {
            let spec = title::title_font(&self.skin, self.prefs.user_font.as_deref());
            self.fonts.measure(text, &spec)
        }
    }

    fn refresh_title(&mut self) {
        let text = self.title_text();
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
        if let Some(id) = self.main_id {
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
            if let Some(w) = self.windows.get(&id) {
                w.window.set_title(&caption);
            }
        }
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
        let title = if self.prefs.bitmap_title {
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
        let active = !self.prefs.dim_inactive || self.focused.is_some();
        MainView {
            active,
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
            eq_visible: false,
            pl_visible: false,
            title,
        }
    }

    fn redraw(&mut self, id: WindowId) {
        let Some(kind) = self.windows.get(&id).map(|w| w.kind) else {
            return;
        };
        let frame = match kind {
            Kind::Main => {
                let view = self.main_view();
                scale(&mw::draw_main(&self.skin, &view), self.scale() as u32)
            }
        };
        let shape_mask = self.shape_for(kind, &frame);
        let Some(w) = self.windows.get_mut(&id) else {
            return;
        };
        if w.shape_dirty {
            shape::apply(&*w.window, shape_mask.as_ref());
            w.shape_dirty = false;
        }
        present(w, &frame);
    }

    fn shape_for(&self, kind: Kind, frame: &Frame) -> Option<region::Mask> {
        if self.skin.is_builtin {
            return None;
        }
        let polys = match kind {
            Kind::Main if self.prefs.shade => self.skin.regions.window_shade.as_ref(),
            Kind::Main => self.skin.regions.normal.as_ref(),
        }?;
        Some(region::rasterize(
            polys,
            frame.width(),
            frame.height(),
            self.scale(),
        ))
    }

    fn request_redraw_all(&self) {
        for w in self.windows.values() {
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

    /// Resizes the main window after a shade/double-size change and keeps docked windows
    /// attached (05 §6.3).
    fn resize_main(&mut self) {
        let Some(id) = self.main_id else { return };
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        let mut rects: Vec<Rect> = ids.iter().map(|i| self.windows[i].rect).collect();
        let idx = ids
            .iter()
            .position(|i| *i == id)
            .expect("main window exists");
        let old = rects[idx];
        let (w, h) = self.main_size();
        let new = Rect::new(old.x, old.y, w, h);
        let work = if self.prefs.keep_on_screen {
            self.work_area(id)
        } else {
            None
        };
        docking::resize_propagate(&mut rects, idx, old, new, work);
        for (i, r) in ids.into_iter().zip(rects) {
            self.set_rect(i, r);
        }
        self.request_redraw_all();
    }

    fn begin_drag(&mut self, id: WindowId, x: i32, y: i32) {
        let s = self.scale();
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        let rects: Vec<Rect> = ids.iter().map(|i| self.windows[i].rect).collect();
        let snapping = self.prefs.snap != self.mods.shift;
        let group = if Some(id) == self.main_id && snapping {
            let idx = ids.iter().position(|i| *i == id).expect("window exists");
            docking::attached_group(&rects, idx)
                .into_iter()
                .map(|i| ids[i])
                .collect()
        } else {
            Vec::new()
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
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        let rects: Vec<Rect> = ids.iter().map(|i| self.windows[i].rect).collect();
        let leader = ids.iter().position(|i| *i == id).expect("window exists");
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
                self.resize_main();
            }
            Action::ToggleShade => {
                self.prefs.shade = !self.prefs.shade;
                self.resize_main();
            }
            Action::Minimize { .. } => {
                if let Some(w) = self.windows.get(&id) {
                    w.window.set_minimized(true);
                }
            }
            Action::Exit => {
                self.exit = true;
                el.exit();
            }
            Action::HideMainWindow => {
                if let Some(w) = self.windows.get(&id) {
                    w.window.set_visible(false);
                }
            }
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

    fn poll_player(&mut self) {
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

    fn update_cursor(&mut self, id: WindowId, el: &ActiveEventLoop) {
        let Some(w) = self.windows.get(&id) else {
            return;
        };
        let s = self.scale();
        let role = match w.kind {
            Kind::Main => main_input::cursor(w.cursor.0 / s, w.cursor.1 / s, self.prefs.shade),
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
        w.window.set_cursor(cursor);
    }

    fn mouse_input(
        &mut self,
        id: WindowId,
        state: ElementState,
        button: MouseButton,
        el: &ActiveEventLoop,
    ) {
        let Some(w) = self.windows.get(&id) else {
            return;
        };
        let s = self.scale();
        let (x, y) = (w.cursor.0 / s, w.cursor.1 / s);
        let ctx = self.main_context();
        let now = self.now_ms();
        let mods = self.mods;
        match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => {
                let double = matches!(self.last_click, Some((lid, t, lx, ly))
                    if lid == id && t.elapsed().as_millis() < DOUBLE_CLICK_MS && (lx - x).abs() <= 2 && (ly - y).abs() <= 2);
                self.last_click = if double {
                    None
                } else {
                    Some((id, Instant::now(), x, y))
                };
                let actions = if double {
                    match self.main_input.double_click(x, y, &ctx) {
                        Some(a) => a,
                        None => self.main_input.mouse_down(x, y, mods, &ctx, now),
                    }
                } else {
                    self.main_input.mouse_down(x, y, mods, &ctx, now)
                };
                self.handle_actions(id, actions, el);
            }
            (MouseButton::Left, ElementState::Released) => {
                self.drag = None;
                let actions = self.main_input.mouse_up(x, y, mods, &ctx);
                self.handle_actions(id, actions, el);
            }
            (MouseButton::Right, ElementState::Pressed) => {
                if let Some(a) = self.main_input.right_click(x, y, &ctx) {
                    self.handle_action(id, a, el);
                }
            }
            _ => {}
        }
        self.request_redraw_all();
    }

    fn key_input(&mut self, event: winit::event::KeyEvent, el: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }
        // Easter egg: typing the word toggles the alternate title strip (02 §2.1).
        if let Some(text) = event.text.as_ref() {
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
        let playing = self.last_status.state == PlayState::Playing;
        let Some(id) = self.focused.or(self.main_id) else {
            return;
        };
        match keys::resolve(Focus::Main, code, self.mods, playing) {
            Some(KeyAction::Do(a)) => self.handle_action(id, a, el),
            Some(KeyAction::ShadeFocused) => self.handle_action(id, Action::ToggleShade, el),
            Some(KeyAction::ToggleEasyMove) => self.prefs.easymove = !self.prefs.easymove,
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
        // Volume keys show the volume message.
        if matches!(
            code,
            winit::keyboard::KeyCode::ArrowUp | winit::keyboard::KeyCode::ArrowDown
        ) {
            let now = self.now_ms();
            self.marquee
                .show_message(marquee::volume_message(self.last_status.volume), now);
        }
        self.request_redraw_all();
    }

    fn wheel(&mut self, id: WindowId, delta: MouseScrollDelta, el: &ActiveEventLoop) {
        let notches = match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
        };
        self.wheel_accum += notches;
        let whole = self.wheel_accum.trunc() as i32;
        if whole != 0 {
            self.wheel_accum -= whole as f32;
            self.handle_action(id, Action::Player(Command::VolumeBy(whole * 8)), el);
            let now = self.now_ms();
            self.marquee
                .show_message(marquee::volume_message(self.last_status.volume), now);
        }
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
        if self.main_id.is_some() {
            return;
        }
        let (w, h) = self.main_size();
        let rect = Rect::new(self.prefs.main_pos.0, self.prefs.main_pos.1, w, h);
        self.main_id = self.create_window(el, Kind::Main, rect, APP_NAME);
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
                if self.drag.is_some() {
                    self.continue_drag(id);
                } else {
                    let s = self.scale();
                    let (x, y) = self
                        .windows
                        .get(&id)
                        .map(|w| (w.cursor.0 / s, w.cursor.1 / s))
                        .unwrap_or_default();
                    let ctx = self.main_context();
                    let mods = self.mods;
                    let acts = self.main_input.mouse_move(x, y, mods, &ctx);
                    if !acts.is_empty() || self.main_input.is_dragging() {
                        self.handle_actions(id, acts, el);
                    }
                    self.update_cursor(id, el);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_input(id, state, button, el)
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(id, delta, el),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => self.key_input(event, el),
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
        if let Some(id) = self.main_id {
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
            self.request_redraw_all();
        }
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TICK));
    }
}
