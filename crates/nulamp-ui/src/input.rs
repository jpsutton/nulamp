//! Shared input types and the actions windows ask the application to perform.

use nulamp_core::Command;

/// Keyboard modifiers held during a mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
    };
    pub const SHIFT: Mods = Mods {
        shift: true,
        ctrl: false,
        alt: false,
    };
    pub const CTRL: Mods = Mods {
        shift: false,
        ctrl: true,
        alt: false,
    };
}

/// Menus that windows can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    Main,
    Options,
    Visualization,
    SongTitle,
    TimeMode,
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
    Seek,
    Shuffle,
    Repeat,
    EqToggle,
    PlToggle,
}

/// Something a window asks the application to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Player(Command),
    OpenFiles,
    OpenFolder,
    OpenUrl,
    ToggleShuffle,
    ToggleRepeat,
    ToggleEqWindow,
    TogglePlaylistWindow,
    ToggleLibraryWindow,
    About,
    ToggleTimeMode,
    CycleVisMode,
    /// Undo a visualizer mode change (from the first click of a double-click).
    UndoVisMode,
    ToggleVisPlugin,
    /// Shade-mode double-click on the visualizer: toggle off/spectrum (06 §6.1).
    ToggleVisOffSpectrum,
    ToggleAlwaysOnTop,
    ToggleDoubleSize,
    ToggleShade,
    FileInfoCurrent,
    Minimize {
        to_tray: bool,
    },
    Exit,
    HideMainWindow,
    /// Open a menu at a point in window (1x) coordinates.
    Menu {
        kind: MenuKind,
        x: i32,
        y: i32,
    },
    /// Start moving the window (and its docked group) from this 1x point.
    BeginWindowDrag {
        x: i32,
        y: i32,
    },
    /// Show a transient marquee message for ~1 s.
    Message(String),
    /// Show a marquee message until the mouse is released.
    DragMessage(String),
    /// Mouse released after a drag message.
    ReleaseMessage,
    /// Start or continue scrolling the title by hand.
    TitleDrag {
        x: i32,
    },
    TitleDragEnd,
}

/// Ctrl+V or Shift+Insert: paste into the focused text field.
pub fn is_paste(code: winit::keyboard::KeyCode, mods: Mods) -> bool {
    use winit::keyboard::KeyCode;
    !mods.alt
        && ((code == KeyCode::KeyV && mods.ctrl && !mods.shift)
            || (code == KeyCode::Insert && mods.shift && !mods.ctrl))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_keys() {
        use winit::keyboard::KeyCode;
        let m = |shift, ctrl, alt| Mods { shift, ctrl, alt };
        assert!(is_paste(KeyCode::KeyV, m(false, true, false)));
        assert!(is_paste(KeyCode::Insert, m(true, false, false)));
        assert!(!is_paste(KeyCode::KeyV, m(false, false, false)));
        assert!(!is_paste(KeyCode::KeyV, m(false, true, true)));
        assert!(!is_paste(KeyCode::Insert, m(false, true, false)));
    }
}
