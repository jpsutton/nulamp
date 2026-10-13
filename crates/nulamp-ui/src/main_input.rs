//! Main window mouse interaction (02 §12–13), tooltips and cursor areas (01 §7).
//!
//! All coordinates are 1x skin pixels; the caller halves them in double size.

use crate::geom::hit;
use crate::input::{Action, MenuKind, Mods};
use crate::marquee::{balance_message, seek_message, volume_message};
use nulamp_core::Command;
use nulamp_render::main_window::{MainButton, balance_geometry, volume_geometry};
use nulamp_skin::CursorRole;

/// Player and window state the controller reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MainContext {
    pub playing: bool,
    /// Playing or paused (an item is open).
    pub open: bool,
    pub seekable: bool,
    pub length_ms: Option<u64>,
    /// Current seek position 0..=256.
    pub seek_pos: u32,
    pub volume: u8,
    pub balance: i8,
    pub shade: bool,
    pub easymove: bool,
    pub clutterbar_shown: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Transport,
    Button(MainButton),
    Clutter(u8),
    About,
    Menu,
    Seek { offset: i32 },
    ShadeSeek,
    Volume { offset: i32 },
    Balance { offset: i32 },
    Title,
    Window,
}

/// Delay between pressing the menu button and opening the main menu (02 §11).
pub const MENU_DELAY_MS: u64 = 155;

/// Clutterbar hint texts (02 §8.4).
const CLUTTER_HINTS: [&str; 5] = [
    "Options Menu",
    "Enable/Disable Always-On-Top",
    "File Info Box",
    "Enable/Disable Doublesize Mode",
    "Visualization Menu",
];

/// Main window interaction state. Pressed-sprite fields feed `MainView`.
#[derive(Debug, Clone, Default)]
pub struct MainInput {
    drag: Option<Drag>,
    pub pressed: Option<MainButton>,
    pub clutter_pressed: Option<u8>,
    pub volume_pressed: bool,
    pub balance_pressed: bool,
    /// Seek drag thumb pixel offset (0..=218) while dragging.
    pub seek_drag: Option<i32>,
    seek_target_ms: u64,
    menu_pressed_at: Option<u64>,
    /// Volume/balance applied during the current drag.
    drag_volume: u8,
    drag_balance: i8,
}

fn transport_at(x: i32, y: i32) -> Option<MainButton> {
    if !hit(x, y, 15, 130, 87, 106) {
        return None;
    }
    Some(match x {
        x if x < 38 => MainButton::Previous,
        x if x < 60 => MainButton::Play,
        x if x < 84 => MainButton::Pause,
        x if x < 107 => MainButton::Stop,
        _ => MainButton::Next,
    })
}

fn title_button_at(x: i32, y: i32) -> Option<MainButton> {
    if !(3..=12).contains(&y) {
        return None;
    }
    match x {
        244..=253 => Some(MainButton::Minimize),
        254..=262 => Some(MainButton::Shade),
        264..=272 => Some(MainButton::Close),
        _ => None,
    }
}

fn clutter_at(x: i32, y: i32) -> Option<u8> {
    if !(11..=19).contains(&x) {
        return None;
    }
    match y {
        24..=31 => Some(0),
        32..=39 => Some(1),
        40..=47 => Some(2),
        48..=55 => Some(3),
        56..=62 => Some(4),
        _ => None,
    }
}

/// The command for a transport button with modifiers (07 §2).
pub fn transport_action(b: MainButton, mods: Mods, playing: bool) -> Option<Action> {
    Some(match (b, mods.shift, mods.ctrl) {
        (MainButton::Previous, true, _) => Action::Player(Command::SeekRelative(-5000)),
        (MainButton::Previous, _, true) => Action::Player(Command::StartOfList),
        (MainButton::Previous, ..) => Action::Player(Command::Previous),
        (MainButton::Play, true, _) => Action::OpenFiles,
        (MainButton::Play, _, true) => Action::OpenUrl,
        (MainButton::Play, ..) => Action::Player(Command::Play),
        (MainButton::Pause, ..) => Action::Player(Command::Pause),
        (MainButton::Stop, true, _) => Action::Player(Command::StopWithFade),
        (MainButton::Stop, _, true) if playing => Action::Player(Command::ToggleStopAfterCurrent),
        (MainButton::Stop, _, true) => return None,
        (MainButton::Stop, ..) => Action::Player(Command::Stop),
        (MainButton::Next, true, _) => Action::Player(Command::SeekRelative(5000)),
        (MainButton::Next, _, true) => Action::Player(Command::EndOfList),
        (MainButton::Next, ..) => Action::Player(Command::Next),
        (MainButton::Eject, _, true) => Action::OpenUrl,
        (MainButton::Eject, true, _) => Action::OpenFolder,
        (MainButton::Eject, ..) => Action::OpenFiles,
        _ => return None,
    })
}

impl MainInput {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a mouse button is captured by this window.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// True while the title is being scrolled by hand.
    pub fn title_dragging(&self) -> bool {
        self.drag == Some(Drag::Title)
    }

    fn volume_at(&mut self, x: i32, offset: i32) -> Vec<Action> {
        let v = ((x - offset - 107) * 255 / 51).clamp(0, 255) as u8;
        self.drag_volume = v;
        vec![
            Action::Player(Command::SetVolume(v)),
            Action::DragMessage(volume_message(v)),
        ]
    }

    fn balance_at(&mut self, x: i32, offset: i32, mods: Mods) -> Vec<Action> {
        let mut v = (x - offset - 177).clamp(0, 24) - 12;
        if !mods.shift && v.abs() < 2 {
            v = 0;
        }
        let b = (v * 127 / 12) as i8;
        self.drag_balance = b;
        vec![
            Action::Player(Command::SetBalance(b)),
            Action::DragMessage(balance_message(b)),
        ]
    }

    fn seek_at(&mut self, a: i32, ctx: &MainContext) -> Vec<Action> {
        let a = a.clamp(0, 218);
        let len = ctx.length_ms.unwrap_or(0);
        self.seek_drag = Some(a);
        self.seek_target_ms = len * a as u64 / 218;
        vec![Action::DragMessage(seek_message(self.seek_target_ms, len))]
    }

    /// Left mouse button pressed at (x, y).
    pub fn mouse_down(
        &mut self,
        x: i32,
        y: i32,
        mods: Mods,
        ctx: &MainContext,
        now: u64,
    ) -> Vec<Action> {
        if self.drag.is_some() {
            return vec![];
        }
        if ctx.shade {
            return self.shade_down(x, y, mods, ctx);
        }
        // 2. Drag controls.
        if ctx.open && ctx.seekable && ctx.length_ms.is_some() && hit(x, y, 18, 263, 73, 81) {
            let thumb = 16 + nulamp_render::main_window::seek_thumb_x(ctx.seek_pos);
            // On the thumb: keep the grab offset (drawing uses 219 px of travel, hit math 218).
            let offset = if x >= thumb && x < thumb + 29 {
                x - thumb + 1
            } else {
                14
            };
            self.drag = Some(Drag::Seek { offset });
            return self.seek_at(x - offset - 17, ctx);
        }
        if hit(x, y, 107, 175, 58, 66) {
            let (_, tx) = volume_geometry(ctx.volume);
            let thumb = 107 + tx;
            let offset = if x >= thumb && x <= thumb + 14 {
                x - 107 - tx
            } else {
                7
            };
            self.drag = Some(Drag::Volume { offset });
            self.volume_pressed = true;
            return self.volume_at(x, offset);
        }
        if hit(x, y, 177, 215, 58, 66) {
            let (_, tx) = balance_geometry(ctx.balance);
            let thumb = 177 + tx;
            let offset = if x >= thumb && x <= thumb + 14 {
                x - 177 - tx
            } else {
                7
            };
            self.drag = Some(Drag::Balance { offset });
            self.balance_pressed = true;
            return self.balance_at(x, offset, mods);
        }
        if hit(x, y, 117, 266, 24, 35) {
            self.drag = Some(Drag::Title);
            return vec![Action::TitleDrag { x }];
        }
        // 3. Buttons.
        if let Some(b) = transport_at(x, y) {
            self.drag = Some(Drag::Transport);
            self.pressed = Some(b);
            return vec![];
        }
        let simple = [
            (MainButton::Eject, hit(x, y, 136, 158, 89, 105)),
            (MainButton::Shuffle, hit(x, y, 165, 209, 90, 102)),
            (MainButton::Repeat, hit(x, y, 210, 239, 90, 102)),
            (MainButton::Eq, hit(x, y, 219, 242, 58, 70)),
            (MainButton::Pl, hit(x, y, 243, 265, 58, 70)),
        ];
        if let Some((b, _)) = simple.iter().find(|(_, h)| *h) {
            self.drag = Some(Drag::Button(*b));
            self.pressed = Some(*b);
            return vec![];
        }
        if hit(x, y, 253, 265, 91, 105) {
            self.drag = Some(Drag::About);
            return vec![];
        }
        if ctx.clutterbar_shown
            && let Some(i) = clutter_at(x, y)
        {
            self.drag = Some(Drag::Clutter(i));
            self.clutter_pressed = Some(i);
            return vec![Action::DragMessage(CLUTTER_HINTS[i as usize].into())];
        }
        if hit(x, y, 36, 96, 26, 39) {
            return vec![Action::ToggleTimeMode];
        }
        if hit(x, y, 27, 99, 40, 61) {
            return if ctx.playing {
                vec![Action::CycleVisMode]
            } else {
                vec![]
            };
        }
        // 4. Title bar buttons.
        if hit(x, y, 5, 17, 3, 13) {
            self.drag = Some(Drag::Menu);
            self.pressed = Some(MainButton::Menu);
            self.menu_pressed_at = Some(now);
            return vec![];
        }
        if let Some(b) = title_button_at(x, y) {
            self.drag = Some(Drag::Button(b));
            self.pressed = Some(b);
            return vec![];
        }
        // 5. Window move.
        if y < 14 || ctx.easymove {
            self.drag = Some(Drag::Window);
            return vec![Action::BeginWindowDrag { x, y }];
        }
        vec![]
    }

    fn shade_down(&mut self, x: i32, y: i32, mods: Mods, ctx: &MainContext) -> Vec<Action> {
        if hit(x, y, 168, 224, 2, 11) {
            let b = match x {
                x if x < 176 => MainButton::Previous,
                x if x < 186 => MainButton::Play,
                x if x < 195 => MainButton::Pause,
                x if x < 204 => MainButton::Stop,
                x if x < 215 => MainButton::Next,
                _ => MainButton::Eject,
            };
            return transport_action(b, mods, ctx.playing).into_iter().collect();
        }
        if hit(x, y, 129, 157, 3, 9) {
            return vec![Action::ToggleTimeMode];
        }
        if hit(x, y, 78, 116, 4, 11) {
            return vec![Action::CycleVisMode];
        }
        if ctx.open && ctx.seekable && ctx.length_ms.is_some() && hit(x, y, 228, 242, 3, 12) {
            self.drag = Some(Drag::ShadeSeek);
            return self.seek_at((x - 228) * 218 / 13, ctx);
        }
        if hit(x, y, 5, 17, 3, 13) {
            self.drag = Some(Drag::Menu);
            self.pressed = Some(MainButton::Menu);
            self.menu_pressed_at = Some(0);
            return vec![];
        }
        if let Some(b) = title_button_at(x, y) {
            self.drag = Some(Drag::Button(b));
            self.pressed = Some(b);
            return vec![];
        }
        self.drag = Some(Drag::Window);
        vec![Action::BeginWindowDrag { x, y }]
    }

    /// Pointer moved (with or without a button held).
    pub fn mouse_move(&mut self, x: i32, y: i32, mods: Mods, ctx: &MainContext) -> Vec<Action> {
        match self.drag {
            Some(Drag::Transport) => {
                self.pressed = transport_at(x, y);
                vec![]
            }
            Some(Drag::Button(b)) => {
                let inside = match b {
                    MainButton::Eject => hit(x, y, 136, 158, 89, 105),
                    MainButton::Shuffle => hit(x, y, 165, 209, 90, 102),
                    MainButton::Repeat => hit(x, y, 210, 239, 90, 102),
                    MainButton::Eq => hit(x, y, 219, 242, 58, 70),
                    MainButton::Pl => hit(x, y, 243, 265, 58, 70),
                    _ => title_button_at(x, y) == Some(b),
                };
                self.pressed = inside.then_some(b);
                vec![]
            }
            Some(Drag::Clutter(i)) => {
                self.clutter_pressed = (clutter_at(x, y) == Some(i)).then_some(i);
                vec![]
            }
            Some(Drag::Menu) => {
                if !hit(x, y, 5, 17, 3, 13) {
                    self.drag = None;
                    self.pressed = None;
                    self.menu_pressed_at = None;
                }
                vec![]
            }
            Some(Drag::Seek { offset }) => {
                let mut acts = self.seek_at(x - offset - 17, ctx);
                if mods.shift {
                    acts.push(Action::Player(Command::SeekTo(self.seek_target_ms)));
                }
                acts
            }
            Some(Drag::ShadeSeek) => {
                let mut acts = self.seek_at((x - 228) * 218 / 13, ctx);
                if mods.shift {
                    acts.push(Action::Player(Command::SeekTo(self.seek_target_ms)));
                }
                acts
            }
            Some(Drag::Volume { offset }) => self.volume_at(x, offset),
            Some(Drag::Balance { offset }) => self.balance_at(x, offset, mods),
            Some(Drag::Title) => vec![Action::TitleDrag { x }],
            _ => vec![],
        }
    }

    /// Left mouse button released.
    pub fn mouse_up(&mut self, x: i32, y: i32, mods: Mods, ctx: &MainContext) -> Vec<Action> {
        let drag = self.drag.take();
        let pressed = self.pressed.take();
        self.menu_pressed_at = None;
        let mut acts = Vec::new();
        match drag {
            Some(Drag::Transport) => {
                if let Some(b) = transport_at(x, y) {
                    acts.extend(transport_action(b, mods, ctx.playing));
                }
            }
            Some(Drag::Button(b)) if pressed == Some(b) => {
                acts.extend(match b {
                    MainButton::Eject => transport_action(b, mods, ctx.playing),
                    MainButton::Shuffle => Some(Action::ToggleShuffle),
                    MainButton::Repeat => Some(Action::ToggleRepeat),
                    MainButton::Eq => Some(Action::ToggleEqWindow),
                    MainButton::Pl => Some(Action::TogglePlaylistWindow),
                    MainButton::Minimize => Some(Action::Minimize {
                        to_tray: mods.shift,
                    }),
                    MainButton::Shade => Some(Action::ToggleShade),
                    MainButton::Close if mods.shift => Some(Action::HideMainWindow),
                    MainButton::Close => Some(Action::Exit),
                    _ => None,
                });
            }
            Some(Drag::About) if hit(x, y, 253, 265, 91, 105) => acts.push(Action::Bolt),
            Some(Drag::Clutter(i)) => {
                acts.push(Action::ReleaseMessage);
                if clutter_at(x, y) == Some(i) {
                    acts.push(match i {
                        0 => Action::Menu {
                            kind: MenuKind::Options,
                            x: 14,
                            y: 25,
                        },
                        1 => Action::ToggleAlwaysOnTop,
                        2 => Action::FileInfoCurrent,
                        3 => Action::ToggleDoubleSize,
                        _ => Action::Menu {
                            kind: MenuKind::Visualization,
                            x: 14,
                            y: 56,
                        },
                    });
                }
            }
            Some(Drag::Seek { .. } | Drag::ShadeSeek) => {
                acts.push(Action::Player(Command::SeekTo(self.seek_target_ms)));
                acts.push(Action::ReleaseMessage);
            }
            Some(Drag::Volume { .. }) => acts.push(Action::ReleaseMessage),
            Some(Drag::Balance { .. }) => {
                if (self.drag_balance as i32).abs() < 27 && self.drag_balance != 0 {
                    acts.push(Action::Player(Command::SetBalance(0)));
                    acts.push(Action::DragMessage(balance_message(0)));
                }
                acts.push(Action::ReleaseMessage);
            }
            Some(Drag::Title) => acts.push(Action::TitleDragEnd),
            _ => {}
        }
        self.clutter_pressed = None;
        self.volume_pressed = false;
        self.balance_pressed = false;
        self.seek_drag = None;
        acts
    }

    /// Timer tick: opens the main menu after the menu-button delay.
    pub fn tick(&mut self, now: u64) -> Vec<Action> {
        match self.menu_pressed_at {
            Some(t) if now >= t + MENU_DELAY_MS => {
                self.menu_pressed_at = None;
                self.drag = None;
                self.pressed = None;
                vec![Action::Menu {
                    kind: MenuKind::Main,
                    x: 6,
                    y: 13,
                }]
            }
            _ => vec![],
        }
    }

    /// Double-click (02 §12.6, §13). Returns `None` when it counts as a second mouse-down.
    pub fn double_click(&mut self, x: i32, y: i32, ctx: &MainContext) -> Option<Vec<Action>> {
        if ctx.shade {
            if hit(x, y, 78, 116, 4, 11) {
                return Some(vec![Action::ToggleVisOffSpectrum, Action::ToggleVisPlugin]);
            }
            if y < 15
                && x < 244
                && !hit(x, y, 129, 157, 3, 9)
                && !hit(x, y, 168, 224, 2, 11)
                && !hit(x, y, 5, 17, 3, 13)
            {
                return Some(vec![Action::ToggleShade]);
            }
            if hit(x, y, 6, 16, 4, 12) {
                return Some(vec![Action::Exit]);
            }
            return None;
        }
        if hit(x, y, 6, 16, 4, 12) {
            self.drag = None;
            self.pressed = None;
            self.menu_pressed_at = None;
            return Some(vec![Action::Exit]);
        }
        if hit(x, y, 118, 265, 25, 34) {
            return Some(vec![Action::FileInfoCurrent]);
        }
        if hit(x, y, 24, 99, 43, 58) {
            let mut v = Vec::new();
            if ctx.playing {
                v.push(Action::UndoVisMode);
            }
            v.push(Action::ToggleVisPlugin);
            return Some(v);
        }
        if y < 15 && x < 244 {
            return Some(vec![Action::ToggleShade]);
        }
        None
    }

    /// Right-click menu for a point (02 §12.7). Ignored while the left button is held.
    pub fn right_click(&self, x: i32, y: i32, ctx: &MainContext) -> Option<Action> {
        if self.drag.is_some() {
            return None;
        }
        let menu = |kind| Some(Action::Menu { kind, x, y });
        if ctx.shade {
            return match () {
                _ if hit(x, y, 129, 156, 3, 8) => menu(MenuKind::TimeMode),
                _ if hit(x, y, 78, 115, 4, 10) => menu(MenuKind::Visualization),
                _ if hit(x, y, 167, 175, 3, 11) => menu(MenuKind::Previous),
                _ if hit(x, y, 176, 185, 3, 11) => menu(MenuKind::Play),
                _ if hit(x, y, 186, 195, 3, 11) => menu(MenuKind::Pause),
                _ if hit(x, y, 196, 205, 3, 11) => menu(MenuKind::Stop),
                _ if hit(x, y, 206, 214, 3, 11) => menu(MenuKind::Next),
                _ if hit(x, y, 215, 224, 3, 11) => menu(MenuKind::Eject),
                _ => menu(MenuKind::Main),
            };
        }
        let seek_ok = ctx.open && ctx.seekable;
        match () {
            _ if hit(x, y, 106, 265, 25, 34) => menu(MenuKind::SongTitle),
            _ if hit(x, y, 36, 95, 26, 38) => menu(MenuKind::TimeMode),
            _ if hit(x, y, 27, 98, 40, 60) => menu(MenuKind::Visualization),
            _ if hit(x, y, 16, 36, 88, 105) => menu(MenuKind::Previous),
            _ if hit(x, y, 37, 61, 88, 105) => menu(MenuKind::Play),
            _ if hit(x, y, 62, 88, 88, 105) => menu(MenuKind::Pause),
            _ if hit(x, y, 89, 106, 88, 105) => menu(MenuKind::Stop),
            _ if hit(x, y, 107, 129, 88, 105) => menu(MenuKind::Next),
            _ if hit(x, y, 136, 157, 89, 104) => menu(MenuKind::Eject),
            _ if seek_ok && hit(x, y, 18, 262, 73, 80) => menu(MenuKind::Seek),
            _ if hit(x, y, 164, 210, 89, 103) => menu(MenuKind::Shuffle),
            _ if hit(x, y, 211, 238, 89, 103) => menu(MenuKind::Repeat),
            _ if hit(x, y, 219, 241, 58, 69) => menu(MenuKind::EqToggle),
            _ if hit(x, y, 242, 264, 58, 69) => menu(MenuKind::PlToggle),
            _ => menu(MenuKind::Main),
        }
    }
}

/// Tooltip for a point (02 §12.8).
pub fn tooltip(x: i32, y: i32, shade: bool) -> Option<&'static str> {
    let table: &[((i32, i32, i32, i32), &str)] = if shade {
        &[
            ((226, 4, 243, 11), "Seeking Bar"),
            ((216, 2, 224, 11), "Open File(s)"),
            ((168, 2, 176, 11), "Previous"),
            ((177, 2, 186, 11), "Play"),
            ((187, 2, 195, 11), "Pause"),
            ((196, 2, 204, 11), "Stop"),
            ((205, 2, 215, 11), "Next"),
            ((125, 4, 157, 10), "Time Display"),
            ((254, 3, 262, 12), "Toggle Windowshade Mode"),
            ((244, 3, 253, 12), "Minimize"),
            ((264, 3, 272, 12), "Close"),
            ((5, 3, 17, 13), "Main Menu"),
        ]
    } else {
        &[
            ((254, 3, 262, 12), "Toggle Windowshade Mode"),
            ((244, 3, 253, 12), "Minimize"),
            ((264, 3, 272, 12), "Close"),
            ((5, 3, 17, 13), "Main Menu"),
            ((219, 58, 242, 70), "Toggle Graphical Equalizer"),
            ((242, 58, 265, 70), "Toggle Playlist Editor"),
            ((11, 24, 19, 31), "Options Menu"),
            ((11, 32, 19, 39), "Toggle Always-On-Top"),
            ((11, 40, 19, 47), "File Info Box"),
            ((11, 48, 19, 55), "Toggle Doublesize Mode"),
            ((11, 56, 19, 62), "Visualization Menu"),
            ((107, 58, 175, 66), "Volume Bar"),
            ((177, 58, 215, 66), "Panning Bar"),
            ((18, 73, 263, 81), "Seeking Bar"),
            ((165, 90, 209, 102), "Toggle Shuffle"),
            ((210, 90, 239, 102), "Toggle Repeat"),
            ((253, 91, 265, 105), "About"),
            ((136, 89, 158, 105), "Open File(s)"),
            ((15, 87, 38, 106), "Previous Track"),
            ((39, 87, 60, 106), "Play"),
            ((61, 87, 84, 106), "Pause"),
            ((85, 87, 107, 106), "Stop"),
            ((108, 87, 130, 106), "Next Track"),
            (
                (36, 26, 96, 39),
                "Time Display (click to toggle elapsed/remaining)",
            ),
            ((105, 24, 266, 35), "Song Title (right-click for options)"),
        ]
    };
    table
        .iter()
        .find(|((x1, y1, x2, y2), _)| hit(x, y, *x1, *x2, *y1, *y2))
        .map(|(_, t)| *t)
}

/// Cursor role for a point (01 §7; first match wins).
pub fn cursor(x: i32, y: i32, shade: bool) -> CursorRole {
    if shade {
        return match () {
            _ if hit(x, y, 5, 17, 3, 13) => CursorRole::MMenu,
            _ if hit(x, y, 244, 253, 3, 12) => CursorRole::Min,
            _ if hit(x, y, 254, 262, 3, 12) => CursorRole::WinBut,
            _ if hit(x, y, 264, 272, 3, 12) => CursorRole::Close,
            _ if hit(x, y, 228, 242, 3, 12) => CursorRole::WsPosbar,
            _ => CursorRole::WsNormal,
        };
    }
    match () {
        _ if hit(x, y, 5, 17, 3, 13) => CursorRole::MainMenu,
        _ if hit(x, y, 244, 253, 3, 12) => CursorRole::Min,
        _ if hit(x, y, 254, 262, 3, 12) => CursorRole::WinBut,
        _ if hit(x, y, 264, 272, 3, 12) => CursorRole::Close,
        _ if hit(x, y, 0, 275, 0, 13) => CursorRole::TitleBar,
        _ if hit(x, y, 107, 215, 58, 66) => CursorRole::VolBal,
        _ if hit(x, y, 18, 263, 73, 81) => CursorRole::Posbar,
        _ if hit(x, y, 105, 266, 24, 35) => CursorRole::SongName,
        _ => CursorRole::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> MainContext {
        MainContext {
            playing: true,
            open: true,
            seekable: true,
            length_ms: Some(218_000),
            seek_pos: 0,
            volume: 200,
            balance: 0,
            shade: false,
            easymove: true,
            clutterbar_shown: true,
        }
    }

    fn click(m: &mut MainInput, x: i32, y: i32, mods: Mods) -> Vec<Action> {
        let c = ctx();
        let mut a = m.mouse_down(x, y, mods, &c, 0);
        a.extend(m.mouse_up(x, y, mods, &c));
        a
    }

    #[test]
    fn transport_fires_on_up_with_modifiers() {
        let mut m = MainInput::new();
        let c = ctx();
        assert!(m.mouse_down(20, 95, Mods::NONE, &c, 0).is_empty());
        assert_eq!(m.pressed, Some(MainButton::Previous));
        // Pressed sprite follows the pointer.
        m.mouse_move(50, 95, Mods::NONE, &c);
        assert_eq!(m.pressed, Some(MainButton::Play));
        assert_eq!(
            m.mouse_up(50, 95, Mods::NONE, &c),
            vec![Action::Player(Command::Play)]
        );
        assert_eq!(
            click(&mut m, 20, 95, Mods::SHIFT),
            vec![Action::Player(Command::SeekRelative(-5000))]
        );
        assert_eq!(
            click(&mut m, 20, 95, Mods::CTRL),
            vec![Action::Player(Command::StartOfList)]
        );
        assert_eq!(click(&mut m, 45, 95, Mods::SHIFT), vec![Action::OpenFiles]);
        assert_eq!(
            click(&mut m, 90, 95, Mods::SHIFT),
            vec![Action::Player(Command::StopWithFade)]
        );
        assert_eq!(
            click(&mut m, 90, 95, Mods::CTRL),
            vec![Action::Player(Command::ToggleStopAfterCurrent)]
        );
        assert_eq!(
            click(&mut m, 120, 95, Mods::CTRL),
            vec![Action::Player(Command::EndOfList)]
        );
        // Released outside: nothing.
        m.mouse_down(20, 95, Mods::NONE, &c, 0);
        assert!(m.mouse_up(200, 20, Mods::NONE, &c).is_empty());
    }

    #[test]
    fn buttons() {
        let mut m = MainInput::new();
        assert_eq!(click(&mut m, 140, 95, Mods::NONE), vec![Action::OpenFiles]);
        assert_eq!(click(&mut m, 140, 95, Mods::CTRL), vec![Action::OpenUrl]);
        assert_eq!(
            click(&mut m, 140, 95, Mods::SHIFT),
            vec![Action::OpenFolder]
        );
        assert_eq!(
            click(&mut m, 170, 95, Mods::NONE),
            vec![Action::ToggleShuffle]
        );
        assert_eq!(
            click(&mut m, 215, 95, Mods::NONE),
            vec![Action::ToggleRepeat]
        );
        assert_eq!(
            click(&mut m, 242, 60, Mods::NONE),
            vec![Action::ToggleEqWindow]
        );
        assert_eq!(
            click(&mut m, 243, 60, Mods::NONE),
            vec![Action::TogglePlaylistWindow]
        );
        assert_eq!(click(&mut m, 260, 100, Mods::NONE), vec![Action::Bolt]);
        assert_eq!(
            click(&mut m, 250, 5, Mods::SHIFT),
            vec![Action::Minimize { to_tray: true }]
        );
        assert_eq!(click(&mut m, 258, 5, Mods::NONE), vec![Action::ToggleShade]);
        assert_eq!(click(&mut m, 268, 5, Mods::NONE), vec![Action::Exit]);
        assert_eq!(
            click(&mut m, 268, 5, Mods::SHIFT),
            vec![Action::HideMainWindow]
        );
    }

    #[test]
    fn mouse_down_actions() {
        let mut m = MainInput::new();
        let c = ctx();
        assert_eq!(
            m.mouse_down(50, 30, Mods::NONE, &c, 0),
            vec![Action::ToggleTimeMode]
        );
        assert_eq!(
            m.mouse_down(50, 50, Mods::NONE, &c, 0),
            vec![Action::CycleVisMode]
        );
        let stopped = MainContext {
            playing: false,
            open: false,
            ..c
        };
        assert!(m.mouse_down(50, 50, Mods::NONE, &stopped, 0).is_empty());
    }

    #[test]
    fn clutterbar() {
        let mut m = MainInput::new();
        let c = ctx();
        let a = m.mouse_down(15, 34, Mods::NONE, &c, 0);
        assert_eq!(
            a,
            vec![Action::DragMessage("Enable/Disable Always-On-Top".into())]
        );
        assert_eq!(m.clutter_pressed, Some(1));
        assert_eq!(
            m.mouse_up(15, 34, Mods::NONE, &c),
            vec![Action::ReleaseMessage, Action::ToggleAlwaysOnTop]
        );
        let a = click(&mut m, 15, 26, Mods::NONE);
        assert_eq!(
            a[2],
            Action::Menu {
                kind: MenuKind::Options,
                x: 14,
                y: 25
            }
        );
    }

    #[test]
    fn menu_button_delay_and_cancel() {
        let mut m = MainInput::new();
        let c = ctx();
        m.mouse_down(10, 8, Mods::NONE, &c, 1000);
        assert_eq!(m.pressed, Some(MainButton::Menu));
        assert!(m.tick(1154).is_empty());
        assert_eq!(
            m.tick(1155),
            vec![Action::Menu {
                kind: MenuKind::Main,
                x: 6,
                y: 13
            }]
        );
        m.mouse_down(10, 8, Mods::NONE, &c, 2000);
        m.mouse_move(30, 30, Mods::NONE, &c);
        assert!(m.tick(3000).is_empty());
        assert_eq!(m.pressed, None);
    }

    #[test]
    fn seek_release_positions() {
        let c = ctx();
        for (x, want) in [(31, 0), (109 + 31, 109_000), (218 + 31, 218_000)] {
            let mut m = MainInput::new();
            m.mouse_down(x, 76, Mods::NONE, &c, 0);
            let acts = m.mouse_up(x, 76, Mods::NONE, &c);
            assert_eq!(acts[0], Action::Player(Command::SeekTo(want)), "x={x}");
        }
        // Shift seeks live.
        let mut m = MainInput::new();
        m.mouse_down(100, 76, Mods::NONE, &c, 0);
        let a = m.mouse_move(120, 76, Mods::SHIFT, &c);
        assert!(matches!(a.last(), Some(Action::Player(Command::SeekTo(_)))));
        // Not seekable: the bar does not react (falls through to window move).
        let mut m = MainInput::new();
        let ns = MainContext {
            seekable: false,
            ..c
        };
        assert_eq!(
            m.mouse_down(100, 76, Mods::NONE, &ns, 0),
            vec![Action::BeginWindowDrag { x: 100, y: 76 }]
        );
    }

    #[test]
    fn volume_drag() {
        let c = ctx();
        let mut m = MainInput::new();
        // Volume 200: thumb at 107 + 40 = 147. Grab at 150 keeps offset 3.
        let a = m.mouse_down(150, 60, Mods::NONE, &c, 0);
        assert_eq!(a[0], Action::Player(Command::SetVolume(200)));
        let a = m.mouse_move(160, 60, Mods::NONE, &c);
        assert_eq!(a[0], Action::Player(Command::SetVolume(250)));
        let a = m.mouse_move(0, 60, Mods::NONE, &c);
        assert_eq!(a[0], Action::Player(Command::SetVolume(0)));
        assert_eq!(
            m.mouse_up(0, 60, Mods::NONE, &c),
            vec![Action::ReleaseMessage]
        );
    }

    #[test]
    fn balance_drag_and_snap() {
        let c = ctx();
        let mut m = MainInput::new();
        // Balance 0: thumb at 189. Click at 210: offset 7 → v = 210-7-177 = 26 → 24 → 12 → 127.
        let a = m.mouse_down(210, 60, Mods::NONE, &c, 0);
        assert_eq!(a[0], Action::Player(Command::SetBalance(127)));
        // Move to a small value: v = 197-7-177-12 = 1 → snaps to 0 without shift.
        let a = m.mouse_move(197, 60, Mods::NONE, &c);
        assert_eq!(a[0], Action::Player(Command::SetBalance(0)));
        let a = m.mouse_move(200, 60, Mods::NONE, &c);
        assert_eq!(a[0], Action::Player(Command::SetBalance(42)));
        // Release below 27 would snap; 42 stays.
        assert_eq!(
            m.mouse_up(200, 60, Mods::NONE, &c),
            vec![Action::ReleaseMessage]
        );
        let mut m = MainInput::new();
        m.mouse_down(214, 60, Mods::SHIFT, &c, 0); // off the thumb: offset 7
        let a = m.mouse_move(198, 60, Mods::SHIFT, &c); // v = 2 → 21
        assert_eq!(a[0], Action::Player(Command::SetBalance(21)));
        let a = m.mouse_up(198, 60, Mods::SHIFT, &c);
        assert_eq!(a[0], Action::Player(Command::SetBalance(0)));
    }

    #[test]
    fn shade_mode() {
        let c = MainContext {
            shade: true,
            ..ctx()
        };
        let mut m = MainInput::new();
        assert_eq!(
            m.mouse_down(180, 6, Mods::NONE, &c, 0),
            vec![Action::Player(Command::Play)]
        );
        assert_eq!(
            m.mouse_down(220, 6, Mods::CTRL, &c, 0),
            vec![Action::OpenUrl]
        );
        let a = m.mouse_down(241, 6, Mods::NONE, &c, 0);
        assert!(matches!(a[0], Action::DragMessage(_)));
        let a = m.mouse_up(241, 6, Mods::NONE, &c);
        assert_eq!(
            a[0],
            Action::Player(Command::SeekTo(218_000 * (13 * 218 / 13) as u64 / 218))
        );
        assert_eq!(
            m.mouse_down(100, 6, Mods::NONE, &c, 0),
            vec![Action::CycleVisMode]
        );
        assert_eq!(
            m.mouse_down(60, 6, Mods::NONE, &c, 0),
            vec![Action::BeginWindowDrag { x: 60, y: 6 }]
        );
    }

    #[test]
    fn double_clicks_and_menus() {
        let c = ctx();
        let mut m = MainInput::new();
        assert_eq!(m.double_click(10, 8, &c), Some(vec![Action::Exit]));
        assert_eq!(
            m.double_click(150, 30, &c),
            Some(vec![Action::FileInfoCurrent])
        );
        assert_eq!(m.double_click(100, 5, &c), Some(vec![Action::ToggleShade]));
        assert_eq!(m.double_click(50, 95, &c), None);
        assert_eq!(
            m.right_click(150, 30, &c),
            Some(Action::Menu {
                kind: MenuKind::SongTitle,
                x: 150,
                y: 30
            })
        );
        assert_eq!(
            m.right_click(100, 76, &c),
            Some(Action::Menu {
                kind: MenuKind::Seek,
                x: 100,
                y: 76
            })
        );
        let stopped = MainContext {
            open: false,
            playing: false,
            ..c
        };
        assert_eq!(
            m.right_click(100, 76, &stopped),
            Some(Action::Menu {
                kind: MenuKind::Main,
                x: 100,
                y: 76
            })
        );
    }

    #[test]
    fn tooltips_and_cursors() {
        assert_eq!(tooltip(258, 5, false), Some("Toggle Windowshade Mode"));
        assert_eq!(tooltip(240, 100, false), None);
        assert_eq!(tooltip(230, 6, true), Some("Seeking Bar"));
        assert_eq!(cursor(10, 8, false), CursorRole::MainMenu);
        assert_eq!(cursor(100, 5, false), CursorRole::TitleBar);
        assert_eq!(cursor(150, 60, false), CursorRole::VolBal);
        assert_eq!(cursor(150, 100, false), CursorRole::Normal);
        assert_eq!(cursor(230, 6, true), CursorRole::WsPosbar);
    }
}
