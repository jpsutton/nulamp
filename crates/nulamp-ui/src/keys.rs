//! Keyboard shortcuts (11 §2). Letter keys use physical key positions.

use crate::input::{Action, MenuKind, Mods};
use nulamp_core::Command;
use winit::keyboard::KeyCode;

/// Which player window has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Main,
    Equalizer,
    Playlist,
    Other,
}

/// Extra actions that only the shell understands.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyAction {
    Do(Action),
    Preferences,
    JumpToTime,
    JumpToFile,
    OpenPlaylist,
    SavePlaylist,
    FocusNextWindow,
    BookmarkCurrent,
    EditBookmarks,
    ToggleMainWindow,
    ToggleVideoWindow,
    /// Video full screen (11 §2.1 Alt+Enter).
    VideoFullScreen,
    ShadeFocused,
    ToggleEasyMove,
    SkinBrowser,
    ReloadSkin,
    ToggleManualAdvance,
    Help,
}

use KeyAction::Do;

fn player(c: Command) -> Option<KeyAction> {
    Some(Do(Action::Player(c)))
}

/// Global shortcuts, active in every player window (11 §2.1).
pub fn global(key: KeyCode, m: Mods) -> Option<KeyAction> {
    use KeyCode::*;
    match (key, m.ctrl, m.alt, m.shift) {
        (KeyP, true, false, false) => Some(KeyAction::Preferences),
        (KeyD, true, false, false) => Some(Do(Action::ToggleDoubleSize)),
        (KeyW, true, false, false) => Some(KeyAction::ShadeFocused),
        (KeyT, true, false, false) => Some(Do(Action::ToggleTimeMode)),
        (KeyJ, true, false, false) => Some(KeyAction::JumpToTime),
        (KeyO, true, false, false) => Some(KeyAction::OpenPlaylist),
        (KeyS, true, false, false) => Some(KeyAction::SavePlaylist),
        (Tab, true, false, _) => Some(KeyAction::FocusNextWindow),
        (KeyB, true, true, false) => Some(KeyAction::BookmarkCurrent),
        (KeyW, false, true, false) => Some(KeyAction::ToggleMainWindow),
        (KeyG, false, true, false) => Some(Do(Action::ToggleEqWindow)),
        (KeyE, false, true, false) => Some(Do(Action::TogglePlaylistWindow)),
        // Not in the spec (decision MLUI6).
        (KeyL, false, true, false) => Some(Do(Action::ToggleLibraryWindow)),
        (KeyV, false, true, false) => Some(KeyAction::ToggleVideoWindow),
        (Enter | NumpadEnter, false, true, false) => Some(KeyAction::VideoFullScreen),
        (KeyF, false, true, false) => Some(Do(Action::Menu {
            kind: MenuKind::Main,
            x: 6,
            y: 13,
        })),
        (KeyM, false, true, s) => Some(Do(Action::Minimize { to_tray: s })),
        (F1, true, false, false) => Some(Do(Action::About)),
        (F1, false, false, false) => Some(KeyAction::Help),
        (F4, false, true, false) => Some(Do(Action::Exit)),
        _ => None,
    }
}

/// Main-window shortcuts (11 §2.2). Also used by the playlist and EQ for keys they do not
/// handle themselves (11 §2.4).
pub fn main_window(key: KeyCode, m: Mods, playing: bool) -> Option<KeyAction> {
    use KeyCode::*;
    let (c, a, s) = (m.ctrl, m.alt, m.shift);
    match (key, c, a, s) {
        (KeyZ, false, false, false) | (Numpad4, false, false, false) => player(Command::Previous),
        (KeyX, false, false, false) | (Numpad5, false, false, false) => player(Command::Play),
        (KeyC, false, false, false) => player(Command::Pause),
        (KeyV, false, false, false) => player(Command::Stop),
        (KeyB, false, false, false) | (Numpad6, false, false, false) => player(Command::Next),
        (KeyZ, false, false, true) => player(Command::SeekRelative(-5000)),
        (KeyB, false, false, true) => player(Command::SeekRelative(5000)),
        (KeyZ, true, false, false) => player(Command::StartOfList),
        (KeyB, true, false, false) => player(Command::EndOfList),
        (KeyV, false, false, true) => player(Command::StopWithFade),
        (KeyV, true, false, false) if playing => player(Command::ToggleStopAfterCurrent),
        (ArrowLeft, false, false, false) | (Numpad7, false, false, false) => {
            player(Command::SeekRelative(-5000))
        }
        (ArrowRight, false, false, false) | (Numpad9, false, false, false) => {
            player(Command::SeekRelative(5000))
        }
        (ArrowUp, false, false, false) | (Numpad8, false, false, false) => {
            player(Command::VolumeBy(4))
        }
        (ArrowDown, false, false, false) | (Numpad2, false, false, false) => {
            player(Command::VolumeBy(-4))
        }
        (Numpad1, false, false, false) => player(Command::JumpEntries(-10)),
        (Numpad3, false, false, false) => player(Command::JumpEntries(10)),
        (KeyL, false, false, false)
        | (Numpad0, false, false, false)
        | (KeyX, false, false, true) => Some(Do(Action::OpenFiles)),
        (KeyL, false, false, true) | (Insert, false, false, false) => Some(Do(Action::OpenFolder)),
        (KeyL, true, false, false) | (Numpad0, true, false, false) | (KeyX, true, false, false) => {
            Some(Do(Action::OpenUrl))
        }
        (KeyR, false, false, false) => Some(Do(Action::ToggleRepeat)),
        (KeyS, false, false, false) => Some(Do(Action::ToggleShuffle)),
        (KeyR, false, false, true) => Some(KeyAction::ToggleManualAdvance),
        (KeyJ, false, false, false)
        | (NumpadDecimal, false, false, false)
        | (F3, false, false, false) => Some(KeyAction::JumpToFile),
        (KeyA, true, false, false) => Some(Do(Action::ToggleAlwaysOnTop)),
        (KeyE, true, false, false) => Some(KeyAction::ToggleEasyMove),
        (Digit3, false, true, false) => Some(Do(Action::FileInfoCurrent)),
        (KeyS, false, true, false) => Some(KeyAction::SkinBrowser),
        (F5, false, false, false) => Some(KeyAction::ReloadSkin),
        (KeyI, true, true, false) => Some(KeyAction::EditBookmarks),
        (ContextMenu, false, false, false) | (F10, false, false, true) => Some(Do(Action::Menu {
            kind: MenuKind::Main,
            x: 6,
            y: 13,
        })),
        _ => None,
    }
}

/// Resolves a key for the focused window: the window's own keys win, then globals, then the
/// main window's keys (11 §2.4).
pub fn resolve(focus: Focus, key: KeyCode, m: Mods, playing: bool) -> Option<KeyAction> {
    let _ = focus;
    global(key, m).or_else(|| main_window(key, m, playing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_keys() {
        assert_eq!(
            resolve(Focus::Main, KeyCode::KeyX, Mods::NONE, false),
            player(Command::Play)
        );
        assert_eq!(
            resolve(Focus::Main, KeyCode::KeyB, Mods::SHIFT, false),
            player(Command::SeekRelative(5000))
        );
        assert_eq!(resolve(Focus::Main, KeyCode::KeyV, Mods::CTRL, false), None);
        assert_eq!(
            resolve(Focus::Main, KeyCode::KeyV, Mods::CTRL, true),
            player(Command::ToggleStopAfterCurrent)
        );
        assert_eq!(
            resolve(Focus::Main, KeyCode::KeyD, Mods::CTRL, false),
            Some(Do(Action::ToggleDoubleSize))
        );
        assert_eq!(
            resolve(Focus::Main, KeyCode::ArrowUp, Mods::NONE, false),
            player(Command::VolumeBy(4))
        );
        assert_eq!(
            resolve(Focus::Main, KeyCode::KeyS, Mods::NONE, false),
            Some(Do(Action::ToggleShuffle))
        );
        assert_eq!(
            resolve(Focus::Main, KeyCode::KeyS, Mods::CTRL, false),
            Some(KeyAction::SavePlaylist)
        );
        let alt = Mods {
            alt: true,
            ..Mods::NONE
        };
        assert_eq!(
            resolve(Focus::Other, KeyCode::KeyL, alt, false),
            Some(Do(Action::ToggleLibraryWindow))
        );
        assert_eq!(
            resolve(Focus::Main, KeyCode::KeyL, Mods::NONE, false),
            Some(Do(Action::OpenFiles))
        );
    }
}
