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

mod dialog_host;
mod dialogs;
mod library_host;
mod popup;
mod remote;

use crate::docking::{self, DEFAULT_SNAP};
use crate::eq_input::{EqAction, EqContext, EqInput};
use crate::geom::Rect;
use crate::input::{Action, Mods};
use crate::keys::{self, Focus, KeyAction};
use crate::main_input::{self, MainContext, MainInput};
use crate::marquee::{self, Marquee};
use crate::platform::shape;
use crate::platform::wayland::{self, Compound};
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
    /// Playback preferences changed (ReplayGain, fades, buffer).
    fn set_playback_prefs(&mut self, _prefs: &UiPrefs) {}
    /// The play-order engine and playlist.
    fn engine(&self) -> &nulamp_core::Engine;
    fn engine_mut(&mut self) -> &mut nulamp_core::Engine;
    /// Visualizer blocks heard after `frame`: (frame, rate, channels, interleaved samples).
    fn vis_blocks_since(&self, _frame: u64) -> Vec<(u64, u32, usize, Vec<f32>)> {
        Vec::new()
    }
    /// Output frames heard so far (the visualizer clock).
    fn heard_frame(&self) -> u64 {
        0
    }
    /// Whether some decoder handles a file extension.
    fn is_supported_extension(&self, _ext: &str) -> bool {
        true
    }
    /// (extension list separated by `;`, description) pairs for file dialogs.
    fn extensions(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Remote-control requests from other sources (plugins), oldest first.
    fn take_requests(&mut self) -> Vec<nulamp_remote::Request> {
        Vec::new()
    }
    /// Loaded plugin modules, and how many files or modules failed to load.
    fn plugins(&self) -> (Vec<crate::plugins_dialog::PluginRow>, usize) {
        (Vec::new(), 0)
    }
    /// Runs an action on the n-th module of [`PlayerHandle::plugins`].
    fn plugin_action(&mut self, _index: usize, _action: crate::plugins_dialog::PluginAction) {}
    /// Network servers asking for a user name and password (07 §8.5), oldest first.
    fn credential_requests(&mut self) -> Vec<CredentialRequest> {
        Vec::new()
    }
    /// Starts expanding a playlist URL into its entries (10 §2.5) off the UI thread. `None`
    /// when the URL is not a playlist URL.
    fn resolve_playlist_url(&self, _url: &str) -> Option<std::sync::mpsc::Receiver<Vec<String>>> {
        None
    }
    /// Audio CD drives as (whole-disc location, label), in drive order (09 §5).
    fn cd_drives(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// The entries of a location that stands for several (a whole audio CD, 10 §2.5);
    /// `None` for other locations.
    fn expand_location(&self, _location: &str) -> Option<Vec<nulamp_core::NewEntry>> {
        None
    }
}

/// A server asks for Basic-authentication credentials. Answer through `reply`; `None` gives up.
pub struct CredentialRequest {
    pub url: String,
    pub realm: String,
    pub reply: std::sync::mpsc::Sender<Option<(String, String)>>,
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
    pub vis: nulamp_vis::VisSettings,
    /// User skins folder (01 §1.4).
    pub skins_dir: std::path::PathBuf,
    /// Current skin: a path, or `None` for the built-in skin.
    pub skin_path: Option<String>,
    pub last_dir: Option<std::path::PathBuf>,
    /// Most recently opened URLs, newest first.
    pub recent_urls: Vec<String>,
    pub pl_size: (i32, i32),
    pub pl_shade: bool,
    pub pl_font_px: u32,
    pub pl_bitmap_font: bool,
    pub pl_bitmap_scale: i32,
    pub pl_show_numbers: bool,
    pub pl_zero_pad: bool,
    pub pl_page_lines: usize,
    pub pl_wheel_double: bool,
    pub pl_old_school_buttons: bool,
    pub replaygain: nulamp_eq::ReplayGainSettings,
    /// Fade on pause and stop (07 §6.2).
    pub fade_pause_stop: bool,
    pub fade_seek: bool,
    /// Output buffer length in ms (07 §6.1).
    pub buffer_ms: u32,
    /// Crossfade at the end of each track (07 §6.4), its length, curve, manual changes.
    pub crossfade: bool,
    pub crossfade_ms: u32,
    pub crossfade_equal_power: bool,
    pub crossfade_manual: bool,
    /// Join tracks of one album gaplessly even with crossfade on.
    pub gapless_albums: bool,
    /// Show the tray icon (11 §7).
    pub show_tray: bool,
    /// Global hotkeys (11 §5).
    pub hotkeys: nulamp_remote::HotkeySettings,
    /// Media-library window position, size and visibility.
    pub lib_pos: (i32, i32),
    pub lib_size: (i32, i32),
    pub lib_visible: bool,
    /// Width of the library's tree pane.
    pub lib_tree_w: i32,
    /// Widths of the library's list columns (empty: defaults).
    pub lib_columns: Vec<i32>,
    pub library: LibraryPrefs,
}

/// Media-library options (preferences page "Media Library").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPrefs {
    /// Watch folders: (path, include subfolders).
    pub folders: Vec<(String, bool)>,
    pub rescan_on_start: bool,
    /// Follow file changes in the watch folders while running.
    pub watch: bool,
}

impl Default for LibraryPrefs {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            rescan_on_start: true,
            watch: true,
        }
    }
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
            vis: nulamp_vis::VisSettings::default(),
            skins_dir: default_skins_dir(),
            skin_path: None,
            last_dir: None,
            recent_urls: Vec::new(),
            pl_size: (275, 116),
            pl_shade: false,
            pl_font_px: 12,
            pl_bitmap_font: true,
            pl_bitmap_scale: 1,
            pl_show_numbers: true,
            pl_zero_pad: false,
            pl_page_lines: 1,
            pl_wheel_double: true,
            pl_old_school_buttons: false,
            replaygain: nulamp_eq::ReplayGainSettings::default(),
            fade_pause_stop: true,
            fade_seek: true,
            buffer_ms: 2000,
            crossfade: false,
            crossfade_ms: 333,
            crossfade_equal_power: false,
            crossfade_manual: false,
            gapless_albums: true,
            show_tray: false,
            hotkeys: nulamp_remote::HotkeySettings::default(),
            // Right of the default main/equalizer/playlist stack, as tall as the stack.
            lib_pos: (301, 29),
            lib_size: crate::library::window::DEFAULT_SIZE,
            lib_visible: false,
            lib_tree_w: crate::library::window::DEFAULT_TREE_W,
            lib_columns: Vec::new(),
            library: LibraryPrefs::default(),
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
    /// The media library (a generic window, 05 §1).
    Library,
}

struct Win {
    kind: Kind,
    /// The OS window; `None` for a part of the native Wayland compound window.
    window: Option<Rc<Window>>,
    /// Software presenter; `None` when the compound window presents this window.
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>>,
    /// Index in the native Wayland compound window (`wayland::MAIN` for the main window).
    part: Option<usize>,
    /// A redraw is pending for a compound part (parts have no `RedrawRequested` events).
    dirty: std::cell::Cell<bool>,
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

pub struct Shell<P: PlayerHandle> {
    player: P,
    prefs: UiPrefs,
    skin: Skin,
    fonts: FontSystem,
    context: Option<softbuffer::Context<OwnedDisplayHandle>>,
    /// Native Wayland: the main window with the equalizer and playlist as its parts (D5).
    compound: Option<Compound>,
    /// Running as a native Wayland client (not through XWayland).
    wayland: bool,
    /// Compound window: the window that gets key presses (the last one clicked).
    key_target: Option<WindowId>,
    /// Closed Wayland top-level windows to open again (in `about_to_wait`).
    reopen: Vec<Kind>,
    /// Compound part pressed with the mouse. Until the release, pointer events the compositor
    /// sends to the main surface belong to it (KWin moves the pointer focus to the top-level
    /// surface during a press).
    capture: Option<WindowId>,
    windows: HashMap<WindowId, Win>,
    ids: HashMap<Kind, WindowId>,
    focused: Option<WindowId>,
    mods: Mods,
    main_input: MainInput,
    eq_input: EqInput,
    pl: PlaylistUi,
    middle_held: bool,
    menus: Vec<popup::MenuWin>,
    dialogs: HashMap<WindowId, dialog_host::DialogWin>,
    /// A dialog requested by code without an event-loop handle; opened in `about_to_wait`.
    pending_dialog: Option<Box<dyn crate::dialogs::Dialog>>,
    remote: Option<nulamp_remote::Server>,
    mpris: Option<nulamp_remote::Mpris>,
    mpris_updated: Instant,
    coalescer: nulamp_remote::OpenCoalescer,
    tray: Option<crate::tray::Tray>,
    tray_tx: std::sync::mpsc::Sender<nulamp_remote::Request>,
    tray_rx: std::sync::mpsc::Receiver<nulamp_remote::Request>,
    /// Windows hidden by "minimize to tray", to show again on Raise.
    hidden_to_tray: Vec<Kind>,
    /// OS hotkey registration; created when global hotkeys are first enabled.
    global_hotkeys: Option<crate::hotkeys::GlobalHotkeys>,
    /// Playlist URLs being downloaded: (entries, replace the list).
    url_expansions: Vec<(std::sync::mpsc::Receiver<Vec<String>>, bool)>,
    /// Kept alive: on X11 the clipboard owner must stay to serve pastes.
    clipboard: Option<arboard::Clipboard>,
    /// Files dropped during this event batch, and the window they were dropped on.
    dropped: Vec<(Kind, String)>,
    menu_focus_check: Option<Instant>,
    /// A menu requested by code that has no event loop handle; opened in `about_to_wait`.
    pending_menu: Option<(Kind, Vec<crate::menu::Item>, i32, i32)>,
    marquee: Marquee,
    drag: Option<WindowDrag>,
    last_click: Option<(WindowId, Instant, i32, i32)>,
    start: Instant,
    cursors: HashMap<CursorRole, CustomCursor>,
    current_cursor: HashMap<WindowId, CursorRole>,
    prev_vis_mode: nulamp_vis::VisMode,
    analyzer: nulamp_vis::Analyzer,
    visualizer: nulamp_vis::Visualizer,
    vis_frame: u64,
    vis_was_running: bool,
    egg_typed: String,
    easter_egg: bool,
    last_status: PlayerStatus,
    marquee_dragging: bool,
    marquee_title_text: String,
    wheel_accum: f32,
    last_marquee_step: u64,
    last_blink: u64,
    exit: bool,
    /// The media library, when the application provides one.
    library: Option<library_host::LibraryHost>,
}

impl Win {
    fn request_redraw(&self) {
        match &self.window {
            // The main window of a compound window too: winit knows when it is configured.
            Some(w) => w.request_redraw(),
            None => self.dirty.set(true),
        }
    }
}

impl<P: PlayerHandle> Shell<P> {
    pub fn new(mut player: P, prefs: UiPrefs, skin: Skin) -> Self {
        let (tray_tx, tray_rx) = std::sync::mpsc::channel();
        player.set_eq(&prefs.eq);
        player.set_playback_prefs(&prefs);
        let mut pl = PlaylistUi::default();
        pl.w = prefs.pl_size.0.max(275);
        pl.h = prefs.pl_size.1.max(116);
        pl.unshaded_h = pl.h;
        pl.shade = prefs.pl_shade;
        pl.font_px = prefs.pl_font_px;
        pl.bitmap_font = prefs.pl_bitmap_font;
        pl.bitmap_scale = prefs.pl_bitmap_scale;
        pl.show_numbers = prefs.pl_show_numbers;
        pl.zero_pad = prefs.pl_zero_pad;
        pl.page_lines = prefs.pl_page_lines.max(1);
        pl.wheel_double = prefs.pl_wheel_double;
        pl.old_school_buttons = prefs.pl_old_school_buttons;
        let visualizer = nulamp_vis::Visualizer::new(prefs.vis);
        Self {
            last_status: player.status(),
            player,
            prefs,
            skin,
            fonts: FontSystem::new(),
            context: None,
            compound: None,
            wayland: false,
            key_target: None,
            reopen: Vec::new(),
            capture: None,
            windows: HashMap::new(),
            ids: HashMap::new(),
            focused: None,
            mods: Mods::NONE,
            main_input: MainInput::new(),
            eq_input: EqInput::new(),
            pl,
            middle_held: false,
            menus: Vec::new(),
            dialogs: HashMap::new(),
            pending_dialog: None,
            remote: None,
            mpris: None,
            mpris_updated: Instant::now(),
            coalescer: nulamp_remote::OpenCoalescer::new(std::time::Duration::from_secs(1)),
            tray: None,
            tray_tx,
            tray_rx,
            hidden_to_tray: Vec::new(),
            global_hotkeys: None,
            url_expansions: Vec::new(),
            clipboard: None,
            dropped: Vec::new(),
            menu_focus_check: None,
            pending_menu: None,
            marquee: Marquee::new(),
            drag: None,
            last_click: None,
            start: Instant::now(),
            cursors: HashMap::new(),
            current_cursor: HashMap::new(),
            prev_vis_mode: nulamp_vis::VisMode::Spectrum,
            analyzer: nulamp_vis::Analyzer::new(),
            visualizer,
            vis_frame: 0,
            vis_was_running: false,
            egg_typed: String::new(),
            easter_egg: false,
            marquee_dragging: false,
            marquee_title_text: String::new(),
            wheel_accum: 0.0,
            last_marquee_step: 0,
            last_blink: 0,
            exit: false,
            library: None,
        }
    }

    /// Runs the event loop until the user exits. Returns the final preferences and the player.
    pub fn run(mut self) -> Result<(UiPrefs, P), Box<dyn std::error::Error>> {
        let event_loop = build_event_loop()?;
        event_loop.run_app(&mut self)?;
        self.sync_prefs();
        Ok((self.prefs, self.player))
    }

    /// Copies window positions and playlist geometry into the preferences.
    fn sync_prefs(&mut self) {
        let pos = |k: Kind| {
            self.ids
                .get(&k)
                .and_then(|id| self.windows.get(id))
                .map(|w| (w.rect.x, w.rect.y))
        };
        if let Some(p) = pos(Kind::Main) {
            self.prefs.main_pos = p;
        }
        if let Some(p) = pos(Kind::Equalizer) {
            self.prefs.eq_pos = p;
        }
        if let Some(p) = pos(Kind::Playlist) {
            self.prefs.pl_pos = p;
        }
        if let Some(p) = pos(Kind::Library) {
            self.prefs.lib_pos = p;
        }
        if let Some(lib) = &self.library {
            self.prefs.lib_size = (lib.win.w, lib.win.h);
            self.prefs.lib_tree_w = lib.win.tree_w;
            self.prefs.lib_columns = lib.win.column_widths();
        }
        self.prefs.pl_size = (
            self.pl.w,
            if self.pl.shade {
                self.pl.unshaded_h
            } else {
                self.pl.h
            },
        );
        self.prefs.pl_shade = self.pl.shade;
        self.prefs.pl_font_px = self.pl.font_px;
        self.prefs.pl_bitmap_font = self.pl.bitmap_font;
        self.prefs.pl_bitmap_scale = self.pl.bitmap_scale;
        self.prefs.pl_show_numbers = self.pl.show_numbers;
        self.prefs.pl_zero_pad = self.pl.zero_pad;
        self.prefs.pl_page_lines = self.pl.page_lines;
        self.prefs.pl_wheel_double = self.pl.wheel_double;
        self.prefs.pl_old_school_buttons = self.pl.old_school_buttons;
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn kind_of(&self, id: WindowId) -> Option<Kind> {
        self.windows.get(&id).map(|w| w.kind)
    }

    /// Pixel scale of a window. Double size applies to every player window, the equalizer
    /// only when "double size applies to the equalizer" is on (docs/decisions.md D23).
    fn scale_of(&self, kind: Kind) -> i32 {
        let doubled = match kind {
            Kind::Main | Kind::Playlist | Kind::Library => self.prefs.double_size,
            Kind::Equalizer => self.prefs.double_size && self.prefs.eq_double,
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
            Kind::Library => match &self.library {
                Some(lib) => (lib.win.w as u32, lib.win.h as u32),
                None => (275, 116),
            },
        };
        let s = self.scale_of(kind);
        (w as i32 * s, h as i32 * s)
    }

    fn create_window(&mut self, el: &ActiveEventLoop, kind: Kind, pos: (i32, i32), visible: bool) {
        let (w, h) = self.size_of(kind);
        let rect = Rect::new(pos.0, pos.1, w, h);
        if self.wayland && !visible && kind != Kind::Main {
            // winit cannot hide Wayland top-level windows: open it when it is first shown.
            let id = WindowId::from(u64::MAX - 100 - kind as u64);
            self.insert_win(id, kind, None, None, None, rect, false);
            return;
        }
        let title = match kind {
            Kind::Main => APP_NAME.to_string(),
            Kind::Equalizer => format!("{APP_NAME} Equalizer"),
            Kind::Playlist => format!("{APP_NAME} Playlist Editor"),
            Kind::Library => format!("{APP_NAME} {}", crate::library::window::TITLE),
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
        let compound_main = kind == Kind::Main && self.wayland;
        if compound_main {
            // Transparent so pixels outside the skin's shape show what is behind.
            attrs = attrs.with_transparent(true);
        } else if self.prefs.always_on_top {
            attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
        }
        let Ok(window) = el.create_window(attrs) else {
            log::error!("cannot create {kind:?} window");
            return;
        };
        let window = Rc::new(window);
        if compound_main {
            self.compound = Compound::new(el, &*window);
            match &mut self.compound {
                Some(c) => c.set_scale(window.scale_factor()),
                None => log::warn!("no Wayland subsurfaces; windows will not dock"),
            }
        }
        let (surface, part) = if self.compound.is_some() && kind == Kind::Main {
            (None, Some(wayland::MAIN))
        } else {
            let context = self.softbuffer(el);
            let Ok(surface) = softbuffer::Surface::new(context, window.clone()) else {
                return;
            };
            (Some(surface), None)
        };
        let id = window.id();
        self.insert_win(id, kind, Some(window), surface, part, rect, visible);
    }

    /// The software presenter's connection, created on first use.
    fn softbuffer(&mut self, el: &ActiveEventLoop) -> &softbuffer::Context<OwnedDisplayHandle> {
        self.context.get_or_insert_with(|| {
            softbuffer::Context::new(el.owned_display_handle()).expect("softbuffer context")
        })
    }

    /// Adds the equalizer or playlist as a part of the native Wayland compound window.
    fn create_part(&mut self, kind: Kind, pos: (i32, i32), visible: bool) {
        let Some(c) = &mut self.compound else { return };
        let part = c.add_part();
        let (w, h) = self.size_of(kind);
        let rect = Rect::new(pos.0, pos.1, w, h);
        let id = part_window_id(part);
        self.insert_win(id, kind, None, None, Some(part), rect, visible);
        self.place_part(id);
    }

    fn insert_win(
        &mut self,
        id: WindowId,
        kind: Kind,
        window: Option<Rc<Window>>,
        surface: Option<softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>>,
        part: Option<usize>,
        rect: Rect,
        visible: bool,
    ) {
        self.windows.insert(
            id,
            Win {
                kind,
                window,
                surface,
                part,
                dirty: std::cell::Cell::new(visible),
                rect,
                actual: (rect.x, rect.y),
                cursor: (0, 0),
                shape_dirty: true,
                visible,
            },
        );
        self.ids.insert(kind, id);
    }

    /// Moves a compound part to its rectangle, relative to the main window.
    fn place_part(&mut self, id: WindowId) {
        let main = self.ids.get(&Kind::Main).map(|m| self.windows[m].rect);
        let (Some(c), Some(main), Some(w)) = (&mut self.compound, main, self.windows.get(&id))
        else {
            return;
        };
        if let Some(part) = w.part.filter(|p| *p != wayland::MAIN) {
            c.set_position(part, w.rect.x - main.x, w.rect.y - main.y);
        }
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
            let caption = self.window_caption();
            if let Some(win) = &w.window {
                win.set_title(&caption);
            }
            if let Some(t) = &self.tray {
                t.set_tooltip(&caption);
            }
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
                let mut f = scale(&mw::draw_main(&self.skin, &view), s);
                self.draw_vis(&mut f);
                f
            }
            Kind::Equalizer => scale(&ew::draw_eq(&self.skin, &self.eq_view()), s),
            Kind::Playlist => scale(&self.draw_playlist(), s),
            Kind::Library => scale(&self.draw_library(), s),
        };
        let mask = self.shape_for(kind, &frame);
        let Some(w) = self.windows.get_mut(&id) else {
            return;
        };
        if let (Some(part), Some(c)) = (w.part, &mut self.compound) {
            if w.shape_dirty {
                c.set_shape(part, mask);
                w.shape_dirty = false;
            }
            if !c.present(part, &frame) {
                w.dirty.set(true);
            }
            return;
        }
        if w.shape_dirty
            && let Some(window) = &w.window
        {
            shape::apply(&**window, mask.as_ref());
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
            Kind::Playlist | Kind::Library => None,
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
            w.request_redraw();
        }
    }

    fn request_redraw(&self, kind: Kind) {
        if let Some(w) = self.ids.get(&kind).and_then(|id| self.windows.get(id)) {
            w.request_redraw();
        }
    }

    /// The screen area for keeping a window on screen; `None` for compound parts, whose
    /// screen position is unknown.
    fn work_area(&self, id: WindowId) -> Option<Rect> {
        let w = self.windows.get(&id)?;
        if w.part.is_some() {
            return None;
        }
        let m = w.window.as_ref()?.current_monitor()?;
        let (p, s) = (m.position(), m.size());
        Some(Rect::new(p.x, p.y, s.width as i32, s.height as i32))
    }

    fn set_rect(&mut self, id: WindowId, r: Rect) {
        let Some(w) = self.windows.get_mut(&id) else {
            return;
        };
        let moved = (w.rect.x, w.rect.y) != (r.x, r.y);
        if (w.rect.w, w.rect.h) != (r.w, r.h) {
            if let Some(window) = &w.window {
                let _ = window.request_inner_size(PhysicalSize::new(r.w as u32, r.h as u32));
            }
            w.shape_dirty = true;
        }
        w.rect = r;
        match (w.part, &w.window) {
            // The compositor places the main window: keep the parts where they are
            // relative to it.
            (Some(wayland::MAIN), _) if moved => {
                let parts: Vec<WindowId> = self
                    .windows
                    .iter()
                    .filter(|(_, w)| w.part.is_some_and(|p| p != wayland::MAIN))
                    .map(|(id, _)| *id)
                    .collect();
                for p in parts {
                    self.place_part(p);
                }
            }
            (Some(_), _) if moved => self.place_part(id),
            (None, Some(window)) if moved => {
                window.set_outer_position(PhysicalPosition::new(r.x, r.y));
            }
            _ => {}
        }
    }

    /// Visible windows with their rectangles, in a stable order.
    fn visible_rects(&self) -> (Vec<WindowId>, Vec<Rect>) {
        let mut ids: Vec<WindowId> = self
            .windows
            .iter()
            // Compound parts dock only with each other: other windows' positions are unknown.
            .filter(|(_, w)| w.visible && (self.compound.is_none() || w.part.is_some()))
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
            match (w.part, &w.window, &mut self.compound) {
                (Some(part), None, Some(c)) if !visible => c.hide(part),
                // winit cannot hide Wayland top-level windows: close the window, and open it
                // again when it is shown.
                (None, Some(_), _) if self.wayland && !visible => {
                    w.surface = None;
                    w.window = None;
                }
                (None, None, _) if self.wayland && visible => self.reopen.push(kind),
                (_, Some(window), _) => window.set_visible(visible),
                _ => {}
            }
            if visible {
                w.shape_dirty = true;
                w.request_redraw();
            }
        }
        match kind {
            Kind::Equalizer => self.prefs.eq_visible = visible,
            Kind::Playlist => self.prefs.pl_visible = visible,
            Kind::Library => self.prefs.lib_visible = visible,
            Kind::Main => {}
        }
        self.request_redraw(Kind::Main);
    }

    fn begin_drag(&mut self, id: WindowId, x: i32, y: i32) {
        let Some(kind) = self.kind_of(id) else { return };
        if self.wayland
            && let Some(w) = self.windows.get(&id)
            && let Some(win) = &w.window
            && w.part.is_none_or(|p| p == wayland::MAIN)
        {
            // Only the compositor can move a top-level window; parts follow the main window.
            let _ = win.drag_window();
            return;
        }
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

    /// Window ids of the compound window's parts (not the main window).
    fn part_ids(&self) -> Vec<WindowId> {
        self.windows
            .iter()
            .filter(|(_, w)| w.part.is_some_and(|p| p != wayland::MAIN))
            .map(|(id, _)| *id)
            .collect()
    }

    /// The window that handles keys and focus for OS window `id`: in a compound window, the
    /// last-clicked part.
    fn key_window(&self, id: WindowId) -> WindowId {
        match self.key_target {
            Some(t) if self.compound.is_some() && self.kind_of(id) == Some(Kind::Main) => t,
            _ => id,
        }
    }

    fn set_key_target(&mut self, id: WindowId) {
        self.key_target = Some(id);
        let compound_focused = self
            .focused
            .and_then(|f| self.windows.get(&f))
            .is_some_and(|w| w.part.is_some());
        if compound_focused && self.focused != Some(id) {
            self.focused = Some(id);
            self.request_redraw_all();
        }
    }

    /// Handles input on the compound window's parts.
    fn compound_events(&mut self, el: &ActiveEventLoop) {
        let Some(c) = &mut self.compound else { return };
        for event in c.dispatch() {
            let part = match event {
                wayland::Event::Motion { part, .. }
                | wayland::Event::Leave { part }
                | wayland::Event::Button { part, .. }
                | wayland::Event::Wheel { part, .. } => Some(part),
                wayland::Event::Moved { .. } => None,
            };
            if let Some(level) = part.and_then(|p| self.menu_of_part(p)) {
                self.menu_part_event(el, level, event);
                continue;
            }
            match event {
                wayland::Event::Motion { part, x, y } => {
                    let id = part_window_id(part);
                    if let Some(w) = self.windows.get_mut(&id) {
                        w.cursor = (x, y);
                    }
                    self.cursor_moved(id, el);
                }
                wayland::Event::Leave { part } => {
                    self.current_cursor.remove(&part_window_id(part));
                }
                wayland::Event::Button {
                    part,
                    button,
                    pressed,
                } => {
                    let id = part_window_id(part);
                    if pressed {
                        self.close_menus();
                        self.set_key_target(id);
                    }
                    self.capture = pressed.then_some(id);
                    let state = if pressed {
                        ElementState::Pressed
                    } else {
                        ElementState::Released
                    };
                    self.mouse_input(id, state, button, el);
                }
                wayland::Event::Wheel { part, lines } => {
                    let delta = MouseScrollDelta::LineDelta(0.0, lines);
                    self.wheel(part_window_id(part), delta, el);
                }
                wayland::Event::Moved { part, x, y } => {
                    let main = self.ids.get(&Kind::Main).map(|m| self.windows[m].rect);
                    if let (Some(main), Some(w)) =
                        (main, self.windows.get_mut(&part_window_id(part)))
                    {
                        w.actual = (main.x + x, main.y + y);
                    }
                }
            }
        }
    }

    /// Pointer input on a menu drawn as a compound part.
    fn menu_part_event(&mut self, el: &ActiveEventLoop, level: usize, event: wayland::Event) {
        match event {
            wayland::Event::Motion { x, y, .. } => self.menu_motion(el, level, x, y),
            wayland::Event::Leave { .. } => self.menu_leave(level),
            wayland::Event::Button {
                part,
                button,
                pressed,
            } => {
                self.capture = pressed.then_some(part_window_id(part));
                if !pressed && button == MouseButton::Left {
                    self.menu_release(el, level);
                }
            }
            _ => {}
        }
    }

    /// Sends main-surface pointer events to the compound part being pressed. Returns true when
    /// the event was used.
    fn captured_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: &WindowEvent) -> bool {
        let Some(part) = self.capture else {
            return false;
        };
        let main = match self.windows.get(&id) {
            Some(w) if w.part == Some(wayland::MAIN) => w.rect,
            _ => return false,
        };
        if let Some(level) = self.menu_level(part) {
            let pos = self.menus[level].pos;
            match *event {
                WindowEvent::CursorMoved { position, .. } => {
                    let (x, y) = (main.x + position.x as i32, main.y + position.y as i32);
                    self.menu_motion(el, level, x - pos.0, y - pos.1);
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    if state == ElementState::Released {
                        self.capture = None;
                        if button == MouseButton::Left {
                            self.menu_release(el, level);
                        }
                    }
                }
                _ => return false,
            }
            return true;
        }
        match *event {
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(w) = self.windows.get_mut(&part) {
                    // Main-surface coordinates do not depend on where the part is now, so a
                    // part being dragged cannot feed back into its own motion.
                    let (px, py) = (position.x.floor() as i32, position.y.floor() as i32);
                    w.cursor = (main.x + px - w.actual.0, main.y + py - w.actual.1);
                }
                self.cursor_moved(part, el);
                true
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Released {
                    self.capture = None;
                }
                self.mouse_input(part, state, button, el);
                true
            }
            _ => false,
        }
    }

    /// Draws compound parts that asked for a redraw.
    fn redraw_parts(&mut self) {
        let ids: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, w)| w.visible && w.dirty.replace(false))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.redraw(id);
        }
    }

    fn eq_library_path(&self) -> std::path::PathBuf {
        config_dir().join(nulamp_eq::preset::PRESET_LIBRARY_FILE)
    }

    fn eq_auto_path(&self) -> std::path::PathBuf {
        config_dir().join(nulamp_eq::preset::AUTOLOAD_LIBRARY_FILE)
    }

    /// The preset library, created with the first-run presets when missing (03 §5.2).
    fn eq_library(&self) -> Vec<nulamp_eq::Preset> {
        let path = self.eq_library_path();
        let _ = std::fs::create_dir_all(config_dir());
        nulamp_eq::preset::load_or_create_library(&path).unwrap_or_default()
    }

    /// Loading a preset changes bands and preamp only (03 §5.3).
    fn apply_preset(&mut self, bands: [u8; 10], preamp: u8) {
        self.prefs.eq.bands = bands;
        self.prefs.eq.preamp = preamp;
        self.eq_changed();
    }

    fn eq_save(&mut self, name: &str, auto: bool) {
        let path = if auto {
            self.eq_auto_path()
        } else {
            self.eq_library_path()
        };
        let _ = std::fs::create_dir_all(config_dir());
        let p = nulamp_eq::Preset::new(name, self.prefs.eq.bands, self.prefs.eq.preamp);
        if let Err(e) = nulamp_eq::preset::save_preset(&path, &p) {
            log::error!("cannot save preset {name}: {e}");
        }
    }

    /// File name (with extension, no folders) of the current entry, for auto-load presets.
    fn current_file_name(&self) -> String {
        let e = self.player.engine();
        e.playlist
            .get(self.last_status.position)
            .map(|x| nulamp_eq::preset::track_file_name(&x.location).to_string())
            .unwrap_or_default()
    }

    /// EQ auto-load when a track starts (03 §5.4).
    fn eq_auto_load(&mut self) {
        if !self.prefs.eq.auto_load {
            return;
        }
        let q2 = nulamp_eq::preset::read_library_or_empty(&self.eq_auto_path());
        let q1 = nulamp_eq::preset::read_library_or_empty(&self.eq_library_path());
        let file = self.current_file_name();
        if let Some(p) = nulamp_eq::preset::auto_load(&q2, &q1, &file) {
            let (bands, preamp) = (p.bands, p.preamp);
            self.apply_preset(bands, preamp);
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
                EqAction::PresetsMenu { x, y } => {
                    let items = crate::menus::presets_menu(&self.menu_state());
                    self.open_items_at(el, id, items, x, y);
                }
                EqAction::ToggleMenu { auto, x, y } => {
                    let items = crate::menus::eq_toggle_menu(auto, &self.menu_state());
                    self.open_items_at(el, id, items, x, y);
                }
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
            Action::ToggleLibraryWindow if self.library.is_some() => {
                let v = !self.is_visible(Kind::Library);
                self.set_visible(Kind::Library, v);
                if v && let Some(&lid) = self.ids.get(&Kind::Library)
                    && let Some(win) = &self.windows[&lid].window
                {
                    win.focus_window();
                }
            }
            Action::Menu { kind, x, y } => {
                let target = if self.kind_of(id) == Some(Kind::Main) {
                    id
                } else {
                    self.ids[&Kind::Main]
                };
                let (x, y) = if target == id { (x, y) } else { (6, 13) };
                self.open_menu_kind(el, target, kind, x, y);
            }
            Action::OpenFiles => self.dialog_open_files(true),
            Action::OpenFolder => self.dialog_open_folder(true),
            Action::OpenUrl => {
                let d = crate::dialogs::UrlDialog::new(false, &self.prefs.recent_urls);
                self.open_dialog(el, Box::new(d));
            }
            Action::FileInfoCurrent => {
                let loc = self
                    .player
                    .engine()
                    .playlist
                    .get(self.last_status.position)
                    .map(|e| e.location.clone());
                if let Some(loc) = loc.filter(|l| !nulamp_core::playlist::is_url(l)) {
                    self.open_dialog(el, Box::new(crate::dialogs::FileInfoDialog::new(&loc)));
                }
            }
            Action::About => {
                let d = crate::dialogs::AboutDialog::new(APP_NAME, env!("CARGO_PKG_VERSION"));
                self.open_dialog(el, Box::new(d));
            }
            Action::ToggleTimeMode => self.prefs.time_remaining = !self.prefs.time_remaining,
            Action::CycleVisMode => {
                self.prev_vis_mode = self.prefs.vis.mode;
                self.set_vis_mode(self.prefs.vis.mode.next());
            }
            Action::UndoVisMode => self.set_vis_mode(self.prev_vis_mode),
            Action::ToggleVisOffSpectrum => {
                let m = if self.prefs.vis.mode == nulamp_vis::VisMode::Off {
                    nulamp_vis::VisMode::Spectrum
                } else {
                    nulamp_vis::VisMode::Off
                };
                self.set_vis_mode(m);
            }
            // Wayland has no way to keep a window above others (wayland_limits).
            Action::ToggleAlwaysOnTop if self.wayland => {}
            Action::ToggleAlwaysOnTop => {
                self.prefs.always_on_top = !self.prefs.always_on_top;
                let level = if self.prefs.always_on_top {
                    WindowLevel::AlwaysOnTop
                } else {
                    WindowLevel::Normal
                };
                for w in self.windows.values() {
                    if let Some(win) = &w.window {
                        win.set_window_level(level);
                    }
                }
            }
            Action::ToggleDoubleSize => {
                self.prefs.double_size = !self.prefs.double_size;
                for kind in [Kind::Main, Kind::Equalizer, Kind::Playlist, Kind::Library] {
                    self.resize_window(kind);
                }
            }
            Action::ToggleShade => match self.kind_of(id) {
                Some(Kind::Playlist) => self.toggle_pl_shade(),
                // Generic windows have no windowshade mode.
                Some(Kind::Library) => {}
                Some(Kind::Equalizer) => {
                    self.prefs.eq_shade = !self.prefs.eq_shade;
                    self.resize_window(Kind::Equalizer);
                }
                _ => {
                    self.prefs.shade = !self.prefs.shade;
                    self.resize_window(Kind::Main);
                }
            },
            Action::Minimize { to_tray: true } => self.minimize_to_tray(),
            Action::Minimize { .. } => {
                for w in self.windows.values() {
                    if let Some(win) = &w.window {
                        win.set_minimized(true);
                    }
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

    fn set_vis_mode(&mut self, m: nulamp_vis::VisMode) {
        self.prefs.vis.mode = m;
        self.visualizer.set_settings(self.prefs.vis);
        self.visualizer.reset();
        self.request_redraw(Kind::Main);
    }

    /// The playlist shows the visualizer when it is wide, unshaded and the main window is hidden.
    fn pl_mini_vis(&self) -> bool {
        self.is_visible(Kind::Playlist)
            && !self.is_visible(Kind::Main)
            && self.pl.w >= 350
            && !self.pl.shade
    }

    fn vis_size(&self) -> nulamp_vis::VisSize {
        use nulamp_vis::VisSize::*;
        if !self.is_visible(Kind::Main) {
            return Main;
        }
        match (self.prefs.shade, self.prefs.double_size) {
            (false, false) => Main,
            (false, true) => MainDouble,
            (true, false) => Shade,
            (true, true) => ShadeDouble,
        }
    }

    /// Draws the classic visualizer into the (already scaled) main-window frame.
    fn draw_vis(&mut self, f: &mut Frame) {
        if self.prefs.vis.mode == nulamp_vis::VisMode::Off {
            return;
        }
        let s = self.scale_of(Kind::Main);
        let (x, y, _, _) = mw::vis_rect(self.prefs.shade, s);
        let size = self.vis_size();
        if self.visualizer.size() != size {
            self.visualizer.set_size(size);
        }
        let colors = self.skin.vis_colors;
        if self.last_status.state == PlayState::Stopped {
            nulamp_vis::draw_background(size, &colors, f, x, y);
        } else {
            self.visualizer.draw(&colors, f, x, y);
        }
    }

    /// Feeds new audio to the analyzer and advances the visualizer. Returns true to redraw.
    fn tick_vis(&mut self) -> bool {
        let running = self.prefs.vis.mode != nulamp_vis::VisMode::Off
            && self.last_status.state == PlayState::Playing
            && (self.is_visible(Kind::Main) || self.pl_mini_vis());
        if !running {
            let changed = self.vis_was_running;
            if changed {
                self.visualizer.reset();
                self.analyzer.reset();
            }
            self.vis_was_running = false;
            return changed;
        }
        self.vis_was_running = true;
        for (frame, rate, ch, samples) in self.player.vis_blocks_since(self.vis_frame) {
            self.analyzer.push(frame, rate, ch, &samples);
            self.vis_frame = self.vis_frame.max(frame);
        }
        let heard = self.player.heard_frame();
        let data = self.analyzer.at(heard);
        log::trace!(
            "vis: heard {heard}, last block {}, data {:?}",
            self.vis_frame,
            data.as_ref().map(|d| d.bars.iter().copied().max())
        );
        self.visualizer.tick(data.as_ref())
    }

    /// Files dropped on a window (04 §7.4): the main window replaces the list and plays
    /// (Shift appends); the playlist inserts at the row under the pointer.
    fn handle_drop(&mut self) {
        let dropped = std::mem::take(&mut self.dropped);
        let kind = dropped[0].0;
        let items: Vec<String> = dropped.into_iter().map(|(_, p)| p).collect();
        // Skin archives dropped on any window are applied (01 §1.4).
        if items.len() == 1 && items[0].to_lowercase().ends_with(".wsz") {
            self.load_skin(Some(items[0].clone()));
            return;
        }
        match kind {
            Kind::Playlist => {
                let entries = self.expand(&items);
                let y = self
                    .ids
                    .get(&Kind::Playlist)
                    .and_then(|id| self.local_pos(*id))
                    .map_or(0, |(_, _, y)| y);
                let row = self.pl_drop_row(y);
                self.player.engine_mut().insert(row, entries);
                let n = self.player.engine().playlist.len();
                self.pl.clamp_scroll(n);
            }
            _ => self.add_items(items, !self.mods.shift),
        }
        self.poll_player();
        self.request_redraw_all();
    }

    /// The playlist position for items dropped at playlist-window y (04 §7.4): the row
    /// under the pointer, or the end.
    fn pl_drop_row(&self, y: i32) -> usize {
        let n = self.player.engine().playlist.len();
        let row = if self.pl.shade || y < 20 {
            n
        } else {
            ((y - 20).max(0) / self.pl.row_h.max(1)) as usize + self.pl.first
        };
        row.min(n)
    }

    fn bookmarks_path(&self) -> std::path::PathBuf {
        config_dir().join("nulamp.bm8")
    }

    /// Bookmarks as (location, title) pairs.
    pub(super) fn bookmarks(&self) -> Vec<(String, String)> {
        std::fs::read(self.bookmarks_path())
            .map(|b| {
                nulamp_playlist::read_bookmarks(&b)
                    .into_iter()
                    .map(|x| (x.location, x.title))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn save_bookmarks(&self, items: &[(String, String)]) {
        let list: Vec<nulamp_playlist::Bookmark> = items
            .iter()
            .map(|(location, title)| nulamp_playlist::Bookmark {
                location: location.clone(),
                title: title.clone(),
            })
            .collect();
        let _ = std::fs::create_dir_all(config_dir());
        let path = self.bookmarks_path();
        let tmp = path.with_extension("bm8.tmp");
        if std::fs::write(&tmp, nulamp_playlist::write_bookmarks(&list))
            .and_then(|_| std::fs::rename(&tmp, &path))
            .is_err()
        {
            log::error!("cannot save bookmarks");
        }
    }

    fn add_bookmarks(&mut self, new: Vec<(String, String)>) {
        let mut all = self.bookmarks();
        all.extend(new);
        self.save_bookmarks(&all);
    }

    /// Plays a bookmark: clear, add and play; with Shift, append (10 §9).
    pub(super) fn play_bookmark(&mut self, location: &str) {
        let replace = !self.mods.shift;
        self.add_items(vec![location.to_string()], replace);
    }

    fn selected_locations(&self) -> Vec<String> {
        self.player
            .engine()
            .playlist
            .entries()
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.location.clone())
            .collect()
    }

    /// Asks, then moves the selected local files to the trash (04 §9).
    pub(super) fn physically_remove_selected(&mut self) {
        let files: Vec<String> = self
            .selected_locations()
            .into_iter()
            .filter(|l| !nulamp_core::playlist::is_url(l))
            .collect();
        if !files.is_empty() {
            self.pending_dialog =
                Some(Box::new(crate::dialogs::ConfirmDialog::delete_files(files)));
        }
    }

    /// Deletes files (to the trash) and removes the entries whose file was deleted.
    pub(super) fn delete_files(&mut self, files: &[String]) {
        let deleted: Vec<String> = files
            .iter()
            .filter(|f| match trash::delete(f) {
                Ok(()) => true,
                Err(e) => {
                    log::error!("cannot delete {f}: {e}");
                    false
                }
            })
            .cloned()
            .collect();
        self.player
            .engine_mut()
            .remove_where(|_, e| deleted.contains(&e.location));
    }

    /// Writes an HTML page listing the playlist and opens it (04 §9).
    fn generate_html(&mut self) {
        let p = &self.player.engine().playlist;
        let esc = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        let mut total = 0;
        let mut rows = String::new();
        for (i, e) in p.entries().iter().enumerate() {
            let len = e
                .length_ms
                .map(|l| marquee::format_length(l / 1000))
                .unwrap_or_default();
            total += e.length_ms.unwrap_or(0) / 1000;
            rows.push_str(&format!(
                "<tr><td>{}.</td><td>{}</td><td>{}</td></tr>\n",
                i + 1,
                esc(&e.title),
                len
            ));
        }
        let html = format!(
            "<!doctype html><meta charset=\"utf-8\"><title>{APP_NAME} playlist</title>\n<h1>Playlist</h1>\n<p>{} entries, {}</p>\n<table>\n{rows}</table>\n",
            p.len(),
            crate::playlist::format_total(total)
        );
        let path = std::env::temp_dir().join("nulamp-playlist.html");
        if std::fs::write(&path, html).is_ok() {
            open_path(&path.to_string_lossy());
        }
    }

    /// Writes a 0–5 rating to the files' tags.
    pub(super) fn rate_files(&mut self, locations: &[String], rating: u8) {
        for loc in locations
            .iter()
            .filter(|l| !nulamp_core::playlist::is_url(l))
        {
            let value = (rating > 0).then(|| rating.to_string());
            if let Err(e) = nulamp_meta::write(loc, &[("rating".to_string(), value)]) {
                log::warn!("cannot rate {loc}: {e}");
            }
        }
    }

    /// Applies preferences from the preferences dialog.
    fn apply_prefs(&mut self, prefs: UiPrefs, pl: crate::prefs_dialog::PlaylistOptions) {
        let aot_changed = prefs.always_on_top != self.prefs.always_on_top;
        let eq_double_changed = prefs.eq_double != self.prefs.eq_double;
        // Keep window-owned state that the dialog does not edit.
        let keep = self.prefs.clone();
        self.library_prefs_changed(&keep.library, &prefs.library);
        self.prefs = UiPrefs {
            main_pos: keep.main_pos,
            eq_pos: keep.eq_pos,
            pl_pos: keep.pl_pos,
            lib_pos: keep.lib_pos,
            lib_size: keep.lib_size,
            lib_visible: keep.lib_visible,
            lib_tree_w: keep.lib_tree_w,
            lib_columns: keep.lib_columns,
            eq_visible: keep.eq_visible,
            pl_visible: keep.pl_visible,
            double_size: keep.double_size,
            shade: keep.shade,
            eq_shade: keep.eq_shade,
            time_remaining: keep.time_remaining,
            skin_path: keep.skin_path,
            last_dir: keep.last_dir,
            recent_urls: keep.recent_urls,
            ..prefs
        };
        self.pl.show_numbers = pl.show_numbers;
        self.pl.zero_pad = pl.zero_pad;
        self.pl.bitmap_font = pl.bitmap_font;
        self.pl.page_lines = pl.page_lines;
        self.pl.wheel_double = pl.wheel_double;
        self.pl.old_school_buttons = pl.old_school_buttons;
        self.update_pl_font();
        self.visualizer.set_settings(self.prefs.vis);
        self.player.set_eq(&self.prefs.eq);
        self.player.set_playback_prefs(&self.prefs);
        if aot_changed {
            let level = if self.prefs.always_on_top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            };
            for w in self.windows.values() {
                if let Some(win) = &w.window {
                    win.set_window_level(level);
                }
            }
        }
        if self.prefs.show_tray {
            self.ensure_tray();
        } else if self.hidden_to_tray.is_empty() {
            self.tray = None;
        }
        self.apply_hotkeys();
        if eq_double_changed {
            self.resize_window(Kind::Equalizer);
        }
        self.refresh_title_force();
        self.request_redraw_all();
    }

    /// Registers the enabled global hotkeys with the OS, or releases them.
    fn apply_hotkeys(&mut self) {
        if self.wayland {
            // No global key grabs on Wayland (wayland_limits).
            return;
        }
        if self.global_hotkeys.is_none() && self.prefs.hotkeys.enabled {
            self.global_hotkeys = crate::hotkeys::GlobalHotkeys::new();
        }
        if let Some(h) = &mut self.global_hotkeys {
            h.apply(&self.prefs.hotkeys);
        }
    }

    fn ensure_tray(&mut self) {
        if self.tray.is_none() {
            let tip = self.window_caption();
            self.tray = crate::tray::Tray::start(self.tray_tx.clone(), &tip);
        }
    }

    /// Hides every player window and shows the tray icon (02 §12.2, Shift+minimize).
    pub(super) fn minimize_to_tray(&mut self) {
        self.ensure_tray();
        if self.tray.is_none() {
            for w in self.windows.values() {
                if let Some(win) = &w.window {
                    win.set_minimized(true);
                }
            }
            return;
        }
        self.hidden_to_tray = [Kind::Main, Kind::Equalizer, Kind::Playlist, Kind::Library]
            .into_iter()
            .filter(|k| self.is_visible(*k))
            .collect();
        if self.wayland {
            self.close_windows();
            return;
        }
        for k in self.hidden_to_tray.clone() {
            self.show_os_window(k, false);
        }
    }

    /// Wayland: closes every player window, leaving only the tray icon. winit cannot hide
    /// Wayland top-level windows; [`Shell::raise`] opens them again.
    fn close_windows(&mut self) {
        self.close_menus();
        // The parts go first: they are subsurfaces of the main window.
        self.compound = None;
        self.capture = None;
        self.drag = None;
        self.current_cursor.clear();
        for w in self.windows.values_mut() {
            w.surface = None;
            w.window = None;
        }
    }

    /// Wayland: opens the main window again with its parts at their old places.
    fn reopen_main(&mut self, el: &ActiveEventLoop) {
        let Some(main) = self
            .ids
            .get(&Kind::Main)
            .and_then(|id| self.windows.remove(id))
        else {
            return;
        };
        self.create_window(el, Kind::Main, (main.rect.x, main.rect.y), true);
        // Same order as at start-up, so the parts get the same indices and window ids.
        for kind in [Kind::Playlist, Kind::Equalizer] {
            let Some(old) = self.ids.get(&kind).and_then(|id| self.windows.remove(id)) else {
                continue;
            };
            let pos = (old.rect.x, old.rect.y);
            if self.compound.is_some() {
                self.create_part(kind, pos, old.visible);
            } else {
                self.create_window(el, kind, pos, old.visible);
            }
        }
    }

    /// Shows or hides a window without changing its saved visibility.
    fn show_os_window(&mut self, kind: Kind, visible: bool) {
        let Some(w) = self.ids.get(&kind).and_then(|id| self.windows.get(id)) else {
            return;
        };
        match (w.part, &w.window, &mut self.compound) {
            (Some(part), None, Some(c)) if !visible => c.hide(part),
            (Some(_), None, _) => w.dirty.set(true),
            (_, Some(win), _) => win.set_visible(visible),
            _ => {}
        }
    }

    /// Shows windows hidden to the tray and brings the player to the front.
    pub(super) fn raise(&mut self) {
        for k in std::mem::take(&mut self.hidden_to_tray) {
            if self.wayland {
                // Closed by `close_windows`; the main window brings its parts back.
                if matches!(k, Kind::Main | Kind::Library) {
                    self.reopen.push(k);
                }
            } else {
                self.show_os_window(k, true);
            }
        }
        if !self.prefs.show_tray {
            self.tray = None;
        }
        for w in self.windows.values().filter(|w| w.visible) {
            if let Some(win) = &w.window {
                win.set_minimized(false);
            }
        }
        if let Some(win) = self
            .ids
            .get(&Kind::Main)
            .and_then(|id| self.windows.get(id)?.window.as_ref())
        {
            win.focus_window();
        }
        self.request_redraw_all();
    }

    fn window_caption(&self) -> String {
        if self.prefs.show_number_in_title && !self.last_status.title.is_empty() {
            format!(
                "{}. {} - {}",
                self.last_status.position + 1,
                self.last_status.title,
                APP_NAME
            )
        } else {
            APP_NAME.to_string()
        }
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
        self.pl.row_h = if self.pl.bitmap_font {
            pl::bitmap_row_height(self.pl.bitmap_scale)
        } else {
            self.fonts.metrics(&spec).line_height.max(1)
        };
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
            if self.pl_mini_vis() && self.prefs.vis.mode != nulamp_vis::VisMode::Off {
                // Left 72×16 of the 1x main-window visualizer image (06 §5.4).
                let mut img = Frame::new(76, 16, 0);
                if self.visualizer.size() != nulamp_vis::VisSize::Main {
                    self.visualizer.set_size(nulamp_vis::VisSize::Main);
                }
                let colors = self.skin.vis_colors;
                if self.last_status.state == PlayState::Stopped {
                    nulamp_vis::draw_background(nulamp_vis::VisSize::Main, &colors, &mut img, 0, 0);
                } else {
                    self.visualizer.draw(&colors, &mut img, 0, 0);
                }
                nulamp_render::blit(&mut f, &img, 0, 0, 72, 16, self.pl.w - 223, self.pl.h - 26);
            }
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
                PlAction::EnqueueSelected => {
                    let e = self.player.engine_mut();
                    let ids: Vec<_> = e
                        .playlist
                        .entries()
                        .iter()
                        .filter(|x| x.selected)
                        .map(|x| x.id)
                        .collect();
                    for eid in ids {
                        match e.queue.iter().position(|q| *q == eid) {
                            Some(pos) => {
                                e.queue.remove(pos);
                            }
                            None => e.queue.push_back(eid),
                        }
                    }
                }
                PlAction::ShowInFolder => {
                    let loc = self.selected_locations().into_iter().next();
                    if let Some(loc) = loc {
                        show_in_folder(&loc);
                    }
                }
                PlAction::GenerateHtml => self.generate_html(),
                PlAction::RebuildTitles => {
                    for e in self.player.engine_mut().playlist.entries_mut_selected() {
                        e.metadata_read = false;
                    }
                }
                PlAction::EditEntries => {
                    let q: Vec<_> = self
                        .player
                        .engine()
                        .playlist
                        .entries()
                        .iter()
                        .filter(|x| x.selected)
                        .map(|x| (x.id, x.location.clone()))
                        .collect();
                    if !q.is_empty() {
                        self.open_dialog(el, Box::new(crate::dialogs::EditEntryDialog::new(q)));
                    }
                }
                PlAction::BookmarkSelected => {
                    let items: Vec<(String, String)> = self
                        .player
                        .engine()
                        .playlist
                        .entries()
                        .iter()
                        .filter(|e| e.selected)
                        .map(|e| (e.location.clone(), e.title.clone()))
                        .collect();
                    self.add_bookmarks(items);
                }
                PlAction::FileInfoSelected => {
                    let e = self.player.engine();
                    let loc = e
                        .playlist
                        .first_selected()
                        .and_then(|i| e.playlist.get(i))
                        .map(|x| x.location.clone());
                    if let Some(loc) = loc.filter(|l| !nulamp_core::playlist::is_url(l)) {
                        self.open_dialog(el, Box::new(crate::dialogs::FileInfoDialog::new(&loc)));
                    }
                }
                PlAction::FontBigger => {
                    self.pl.bitmap_scale = (self.pl.bitmap_scale + 1).min(4);
                    self.pl.font_px += 1;
                    self.update_pl_font();
                }
                PlAction::FontSmaller => {
                    self.pl.bitmap_scale = (self.pl.bitmap_scale - 1).max(1);
                    self.pl.font_px = (self.pl.font_px - 1).max(2);
                    self.update_pl_font();
                }
                PlAction::FontReset => {
                    self.pl.bitmap_scale = 1;
                    self.pl.font_px = 12;
                    self.update_pl_font();
                }
                PlAction::Menu(choice) => self.pl_menu_choice(choice),
                PlAction::StandardMenu { menu, x, y } => {
                    self.open_playlist_menu(el, id, menu, x, y)
                }
                PlAction::ListContextMenu { x, y } => {
                    self.open_items_at(el, id, crate::menus::list_context(), x, y)
                }
                PlAction::MainMenu { x, y } => self.open_main_menu_at(el, id, x, y),
                PlAction::VisMenu { x, y } => {
                    let items = crate::menus::vis_menu(&self.menu_state());
                    self.open_items_at(el, id, items, x, y);
                }
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
            PlMenuChoice::AddFile => self.dialog_open_files(false),
            PlMenuChoice::AddFolder => self.dialog_open_folder(false),
            PlMenuChoice::AddUrl => {
                self.pending_dialog = Some(Box::new(crate::dialogs::UrlDialog::new(
                    true,
                    &self.prefs.recent_urls,
                )));
            }
            PlMenuChoice::LoadList => self.dialog_load_playlist(),
            PlMenuChoice::SaveList => self.dialog_save_playlist(),
            PlMenuChoice::Sort => {
                let items = crate::menus::sort_menu();
                self.pending_menu = Some((Kind::Playlist, items, 122, self.pl.h - 66));
            }
            PlMenuChoice::FileInfo => {
                let items = crate::menus::file_info_menu();
                self.pending_menu = Some((Kind::Playlist, items, 122, self.pl.h - 48));
            }
            PlMenuChoice::MiscOptions => {
                let items = crate::menus::misc_menu();
                self.pending_menu = Some((Kind::Playlist, items, 122, self.pl.h - 30));
            }
            PlMenuChoice::RemoveMisc => {
                let items = crate::menus::remove_misc();
                self.pending_menu = Some((Kind::Playlist, items, 64, self.pl.h - 84));
            }
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
        let started = s.state == PlayState::Playing
            && (self.last_status.state == PlayState::Stopped
                || s.position != self.last_status.position);
        self.last_status = s;
        self.track_library_play();
        if title_changed {
            self.refresh_title();
        }
        if started {
            self.eq_auto_load();
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
            // Generic-window cursors (05 §1.4).
            Kind::Library => match &self.library {
                Some(lib) if lib.win.in_close(x, y) => CursorRole::PClose,
                Some(_) if y < 14 => CursorRole::PtBar,
                Some(lib) if lib.win.in_grip(x, y) => CursorRole::PSize,
                _ => CursorRole::PNormal,
            },
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
        if let (Some(part), Some(c)) = (
            self.windows.get(&id).and_then(|w| w.part),
            &mut self.compound,
        ) && part != wayland::MAIN
        {
            // Parts use the compositor's cursor set; skin cursors need a cursor surface.
            c.set_cursor(part, role.shape());
            return;
        }
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
        if let Some(win) = self.windows.get(&id).and_then(|w| w.window.as_ref()) {
            win.set_cursor(cursor);
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
        log::debug!("mouse {button:?} {state:?} on {kind:?} at ({x},{y})");
        if kind == Kind::Library {
            if (button, state) == (MouseButton::Left, ElementState::Released) {
                self.drag = None;
            }
            self.library_mouse(id, state, button, x, y, el);
            return;
        }
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
                    Kind::Library => {}
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
                    Kind::Library => {}
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
                Kind::Library => {}
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
            Kind::Library => self.library_mouse_move(id, x, y, el),
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
        if kind == Kind::Library && self.library_key(id, code, event.text.as_deref(), el) {
            return;
        }
        let playing = self.last_status.state == PlayState::Playing;
        let focus = match kind {
            Kind::Main => Focus::Main,
            Kind::Equalizer => Focus::Equalizer,
            Kind::Playlist => Focus::Playlist,
            Kind::Library => Focus::Other,
        };
        if let Some(k) = keys::resolve(focus, code, self.mods, playing) {
            self.key_action(id, k, el);
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

    /// Carries out a keyboard (or menu) shortcut action.
    fn key_action(&mut self, id: WindowId, k: KeyAction, el: &ActiveEventLoop) {
        match k {
            KeyAction::Do(a) => self.handle_action(id, a, el),
            KeyAction::ShadeFocused => self.handle_action(id, Action::ToggleShade, el),
            KeyAction::ToggleEasyMove => self.prefs.easymove = !self.prefs.easymove,
            KeyAction::ToggleMainWindow => {
                let v = !self.is_visible(Kind::Main);
                self.set_visible(Kind::Main, v);
            }
            KeyAction::ToggleManualAdvance => {
                let on = !self.last_status.manual_advance;
                self.player.command(Command::SetManualAdvance(on));
                let now = self.now_ms();
                let msg = if on {
                    "Manual Advance: On"
                } else {
                    "Manual Advance: Off"
                };
                self.marquee.show_message(msg, now);
                self.poll_player();
            }
            KeyAction::JumpToFile => {
                let e = self.player.engine();
                let entries = e
                    .playlist
                    .entries()
                    .iter()
                    .map(|x| (x.id, x.location.clone(), x.title.clone()))
                    .collect();
                let d = crate::dialogs::JumpFileDialog::new(entries, e.playlist.current_id());
                self.open_dialog(el, Box::new(d));
            }
            KeyAction::JumpToTime => {
                let s = &self.last_status;
                if s.seekable && s.state != PlayState::Stopped {
                    let d = crate::dialogs::JumpTimeDialog::new(
                        s.position_ms,
                        s.length_ms.unwrap_or(0),
                    );
                    self.open_dialog(el, Box::new(d));
                }
            }
            KeyAction::SkinBrowser => {
                let skins = nulamp_skin::list_skins(&self.prefs.skins_dir)
                    .into_iter()
                    .map(|e| (e.name, e.path.map(|p| p.to_string_lossy().into_owned())))
                    .collect();
                let d = crate::dialogs::SkinBrowser::new(skins, self.prefs.skin_path.as_deref());
                self.open_dialog(el, Box::new(d));
            }
            KeyAction::Preferences => {
                let d = crate::prefs_dialog::PrefsDialog::new(
                    self.prefs.clone(),
                    self.playlist_options(),
                )
                .wayland_native(self.wayland);
                self.open_dialog(el, Box::new(d));
            }
            KeyAction::BookmarkCurrent => {
                let e = self.player.engine();
                if let Some(x) = e.playlist.get(self.last_status.position) {
                    let b = (x.location.clone(), x.title.clone());
                    self.add_bookmarks(vec![b]);
                }
            }
            KeyAction::EditBookmarks => {
                let d = crate::dialogs::BookmarksDialog::new(self.bookmarks());
                self.open_dialog(el, Box::new(d));
            }
            KeyAction::OpenPlaylist => self.dialog_load_playlist(),
            KeyAction::SavePlaylist => self.dialog_save_playlist(),
            KeyAction::ReloadSkin => {
                let p = self.prefs.skin_path.clone();
                self.load_skin(p);
            }
            other => log::info!("key action not implemented yet: {other:?}"),
        }
        self.request_redraw_all();
    }

    /// Playlist options edited by the preferences dialog.
    fn playlist_options(&self) -> crate::prefs_dialog::PlaylistOptions {
        crate::prefs_dialog::PlaylistOptions {
            show_numbers: self.pl.show_numbers,
            zero_pad: self.pl.zero_pad,
            bitmap_font: self.pl.bitmap_font,
            page_lines: self.pl.page_lines,
            wheel_double: self.pl.wheel_double,
            old_school_buttons: self.pl.old_school_buttons,
        }
    }

    /// Loads a skin (or the built-in one) and redraws everything (01 §3).
    fn load_skin(&mut self, path: Option<String>) {
        let builtin = nulamp_skin::builtin::files();
        let skin = match &path {
            None => nulamp_skin::builtin::skin(),
            Some(p) => match nulamp_skin::SkinFiles::open(std::path::Path::new(p)) {
                Ok(files) => Skin::load(&files, &builtin),
                Err(e) => {
                    log::error!("cannot load skin {p}: {e}");
                    return;
                }
            },
        };
        self.skin = skin;
        self.prefs.skin_path = path;
        self.cursors.clear();
        self.current_cursor.clear();
        for w in self.windows.values_mut() {
            w.shape_dirty = true;
        }
        self.update_pl_font();
        self.refresh_title_force();
        self.request_redraw_all();
    }

    fn refresh_title_force(&mut self) {
        self.marquee_title_text.clear();
        self.marquee = Marquee::new();
        self.refresh_title();
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
        if whole != 0 && self.kind_of(id) == Some(Kind::Library) {
            self.wheel_accum -= whole as f32;
            self.library_wheel(whole);
            return;
        }
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

/// Opens a file or URL with the desktop's default application.
pub fn open_path(path: &str) {
    let r = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", path])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    if let Err(e) = r {
        log::error!("cannot open {path}: {e}");
    }
}

/// Shows a file in the file manager.
pub(super) fn show_in_folder(location: &str) {
    if nulamp_core::playlist::is_url(location) {
        return;
    }
    if cfg!(target_os = "windows") {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{location}"))
            .spawn();
    } else if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("open")
            .args(["-R", location])
            .spawn();
    } else if let Some(dir) = std::path::Path::new(location).parent() {
        open_path(&dir.to_string_lossy());
    }
}

/// `$XDG_CONFIG_HOME/nulamp/skins` (or the platform equivalent).
pub fn default_skins_dir() -> std::path::PathBuf {
    config_dir().join("skins")
}

/// The per-user settings folder (11 §1.1).
pub fn config_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("NULAMP_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return std::path::PathBuf::from(d);
    }
    if let Some(d) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return std::path::PathBuf::from(d).join(APP_NAME);
    }
    if cfg!(target_os = "windows")
        && let Some(d) = std::env::var_os("APPDATA")
    {
        return std::path::PathBuf::from(d).join(APP_NAME);
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support").join(APP_NAME)
    } else {
        home.join(".config").join(APP_NAME)
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
    if let (Some(window), Some(surface)) = (&w.window, &mut w.surface) {
        present_raw(window, surface, frame);
    }
}

/// Copies a frame into a window surface.
fn present_raw(
    window: &Window,
    surface: &mut softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>,
    frame: &Frame,
) {
    let size = window.inner_size();
    let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
    else {
        return;
    };
    if surface.resize(width, height).is_err() {
        return;
    }
    let Ok(mut buf) = surface.buffer_mut() else {
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

/// Whether the event loop talks to a Wayland compositor directly (not through XWayland).
fn is_wayland(el: &ActiveEventLoop) -> bool {
    use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    el.display_handle()
        .is_ok_and(|d| matches!(d.as_raw(), RawDisplayHandle::Wayland(_)))
}

/// Window ids for compound parts; winit's Wayland ids are surface addresses, so these
/// values near `u64::MAX` cannot collide with them.
fn part_window_id(part: usize) -> WindowId {
    WindowId::from(u64::MAX - part as u64)
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
        self.wayland = is_wayland(el);
        if p.show_tray {
            self.ensure_tray();
        }
        self.apply_hotkeys();
        self.update_pl_font();
        if self.library.is_some() {
            self.create_window(el, Kind::Library, p.lib_pos, p.lib_visible);
        }
        if self.wayland {
            // The main window comes first: the equalizer and playlist are its parts.
            self.create_window(el, Kind::Main, p.main_pos, true);
        }
        for (kind, pos, visible) in [
            (Kind::Playlist, p.pl_pos, p.pl_visible),
            (Kind::Equalizer, p.eq_pos, p.eq_visible),
        ] {
            if self.compound.is_some() {
                self.create_part(kind, pos, visible);
            } else {
                self.create_window(el, kind, pos, visible);
            }
        }
        if !self.ids.contains_key(&Kind::Main) {
            self.create_window(el, Kind::Main, p.main_pos, true);
        }
        self.poll_player();
        self.refresh_title();
        // Development aid: open a dialog at start-up (NULAMP_OPEN_DIALOG=about|url|jumpfile|skins).
        if let Ok(which) = std::env::var("NULAMP_OPEN_DIALOG") {
            let id = self.ids[&Kind::Main];
            match which.as_str() {
                "about" => self.handle_action(id, Action::About, el),
                "url" => self.handle_action(id, Action::OpenUrl, el),
                "jumpfile" => self.key_action(id, KeyAction::JumpToFile, el),
                "skins" => self.key_action(id, KeyAction::SkinBrowser, el),
                "fileinfo" => self.handle_action(id, Action::FileInfoCurrent, el),
                "prefs" => self.key_action(id, KeyAction::Preferences, el),
                "plugins" => {
                    let (rows, errors) = self.player.plugins();
                    self.open_dialog(
                        el,
                        Box::new(crate::plugins_dialog::PluginsDialog::new(rows, errors)),
                    );
                }
                "library" if self.library.is_some() => self.set_visible(Kind::Library, true),
                "libprefs" => {
                    let d = crate::prefs_dialog::PrefsDialog::new(
                        self.prefs.clone(),
                        self.playlist_options(),
                    )
                    .with_page("Media Library")
                    .wayland_native(self.wayland);
                    self.open_dialog(el, Box::new(d));
                }
                _ => {}
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TICK));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.menu_event(el, id, &event) || self.dialog_event(el, id, &event) {
            return;
        }
        if self.captured_event(el, id, &event) {
            return;
        }
        if let WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } = event
            && !self.menus.is_empty()
        {
            self.close_menus();
        }
        match event {
            // Closing the library window (for example from the window manager) hides it.
            WindowEvent::CloseRequested if self.kind_of(id) == Some(Kind::Library) => {
                self.set_visible(Kind::Library, false);
            }
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
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. }
                if self.compound.is_some() && self.kind_of(id) == Some(Kind::Main) =>
            {
                if let Some(c) = &mut self.compound {
                    c.set_scale(scale_factor);
                }
                let parts: Vec<WindowId> = self.part_ids();
                for p in parts {
                    self.place_part(p);
                }
                self.request_redraw_all();
            }
            WindowEvent::Focused(f) => {
                if !f && self.compound.is_some() && self.kind_of(id) == Some(Kind::Main) {
                    // Menus drawn as parts cannot notice clicks in other applications.
                    self.close_menus();
                }
                // A compound window has one keyboard focus: give it to the last-clicked part.
                let id = self.key_window(id);
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
                if self.compound.is_some() && self.kind_of(id) == Some(Kind::Main) {
                    self.set_key_target(id);
                }
                self.mouse_input(id, state, button, el)
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(id, delta, el),
            WindowEvent::DroppedFile(path) => {
                if let Some(kind) = self.kind_of(id) {
                    self.dropped
                        .push((kind, path.to_string_lossy().into_owned()));
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                // Keys for menus drawn as compound parts arrive at the main window.
                if let Some(level) = self.menus.len().checked_sub(1)
                    && self.menus[level].part.is_some()
                {
                    if event.state == ElementState::Pressed
                        && let PhysicalKey::Code(code) = event.physical_key
                    {
                        self.menu_key(el, level, code);
                    }
                    return;
                }
                let id = self.key_window(id);
                self.key_input(id, event, el)
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.exit {
            return;
        }
        self.compound_events(el);
        self.check_menu_focus();
        self.poll_remote(el);
        self.poll_url_expansions();
        for kind in std::mem::take(&mut self.reopen) {
            if kind == Kind::Main {
                self.reopen_main(el);
            } else if let Some(old) = self.ids.get(&kind).and_then(|id| self.windows.remove(id)) {
                self.create_window(el, kind, (old.rect.x, old.rect.y), true);
            }
        }
        for request in self.player.credential_requests() {
            self.open_dialog(el, Box::new(crate::dialogs::LoginDialog::new(request)));
        }
        if !self.dropped.is_empty() {
            self.handle_drop();
        }
        if let Some(d) = self.pending_dialog.take() {
            self.open_dialog(el, d);
        }
        if let Some((kind, items, x, y)) = self.pending_menu.take()
            && let Some(&id) = self.ids.get(&kind)
        {
            self.open_items_at(el, id, items, x, y);
        }
        if self.library_poll(el) {
            self.request_redraw(Kind::Library);
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
        if self.tick_vis() {
            self.request_redraw(Kind::Main);
            if self.pl_mini_vis() {
                self.request_redraw(Kind::Playlist);
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
        self.redraw_parts();
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TICK));
    }
}
