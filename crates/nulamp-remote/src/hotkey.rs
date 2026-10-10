//! Global hotkey bindings (11 §5): key combinations, assignable actions and the default table.
//!
//! Only the model lives here; registering hotkeys with the OS is platform code elsewhere. Media
//! keys also reach the player through the media-session interface ([`crate::Mpris`]), which
//! works while global hotkeys are off.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Flag, NumberedCommand, Request};

/// Modifier keys of a hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Windows, Super or Command key.
    pub meta: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };
    pub const CTRL_ALT: Self = Self {
        ctrl: true,
        alt: true,
        shift: false,
        meta: false,
    };
}

/// A key, by its position-independent meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter (upper case), digit or ASCII punctuation key.
    Char(char),
    /// F1–F24.
    F(u8),
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    Space,
    Enter,
    Tab,
    Escape,
    Backspace,
    Pause,
    /// Numpad 0–9.
    Numpad(u8),
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    NumpadEnter,
    MediaPlayPause,
    MediaStop,
    MediaPrevious,
    MediaNext,
    VolumeUp,
    VolumeDown,
    VolumeMute,
}

/// Named keys: canonical name first, then aliases.
const NAMED: &[(Key, &[&str])] = &[
    (Key::Insert, &["Insert", "Ins"]),
    (Key::Delete, &["Delete", "Del"]),
    (Key::Home, &["Home"]),
    (Key::End, &["End"]),
    (Key::PageUp, &["Page Up", "PgUp", "Prior"]),
    (Key::PageDown, &["Page Down", "PgDn", "PgDown"]),
    (Key::Up, &["Up", "Arrow Up"]),
    (Key::Down, &["Down", "Arrow Down"]),
    (Key::Left, &["Left", "Arrow Left"]),
    (Key::Right, &["Right", "Arrow Right"]),
    (Key::Space, &["Space"]),
    (Key::Enter, &["Enter", "Return"]),
    (Key::Tab, &["Tab"]),
    (Key::Escape, &["Esc", "Escape"]),
    (Key::Backspace, &["Backspace"]),
    (Key::Pause, &["Pause", "Break"]),
    (Key::NumpadAdd, &["Num +", "Numpad +", "Numpad Add"]),
    (
        Key::NumpadSubtract,
        &["Num -", "Numpad -", "Numpad Subtract"],
    ),
    (
        Key::NumpadMultiply,
        &["Num *", "Numpad *", "Numpad Multiply"],
    ),
    (Key::NumpadDivide, &["Num /", "Numpad /", "Numpad Divide"]),
    (Key::NumpadDecimal, &["Num .", "Numpad .", "Numpad Decimal"]),
    (Key::NumpadEnter, &["Num Enter", "Numpad Enter"]),
    (
        Key::MediaPlayPause,
        &["Media Play/Pause", "Play/Pause", "Media Play Pause"],
    ),
    (Key::MediaStop, &["Media Stop"]),
    (
        Key::MediaPrevious,
        &["Media Previous", "Media Prev", "Media Previous Track"],
    ),
    (Key::MediaNext, &["Media Next", "Media Next Track"]),
    (Key::VolumeUp, &["Volume Up"]),
    (Key::VolumeDown, &["Volume Down"]),
    (Key::VolumeMute, &["Volume Mute", "Mute"]),
];

/// Lower case without spaces, for name comparison.
fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

impl Key {
    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return c
                .is_ascii_graphic()
                .then(|| Self::Char(c.to_ascii_uppercase()));
        }
        let f = fold(s);
        if let Some((k, _)) = NAMED
            .iter()
            .find(|(_, names)| names.iter().any(|n| fold(n) == f))
        {
            return Some(*k);
        }
        if let Some(n) = f.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            return (1..=24).contains(&n).then_some(Self::F(n));
        }
        let digit = f.strip_prefix("numpad").or_else(|| f.strip_prefix("num"))?;
        let n = digit.parse::<u8>().ok()?;
        (n <= 9 && digit.len() == 1).then_some(Self::Numpad(n))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char(c) => write!(f, "{c}"),
            Self::F(n) => write!(f, "F{n}"),
            Self::Numpad(n) => write!(f, "Num {n}"),
            k => {
                let (_, names) = NAMED.iter().find(|(n, _)| n == k).expect("named key");
                f.write_str(names[0])
            }
        }
    }
}

/// A key combination such as `Ctrl+Alt+Insert`. Serialised as that text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Hotkey {
    pub mods: Modifiers,
    pub key: Key,
}

impl Hotkey {
    pub fn new(mods: Modifiers, key: Key) -> Self {
        Self { mods, key }
    }

    /// Parses `Mod+Mod+Key`. Modifier names: Ctrl/Control, Alt/Option, Shift, and
    /// Meta/Win/Super/Cmd/Command; any case. `Ctrl++` binds the `+` key.
    pub fn parse(s: &str) -> Option<Self> {
        let mut mods = Modifiers::NONE;
        let mut rest = s.trim();
        while let Some((head, tail)) = rest.split_once('+') {
            let m = match fold(head).as_str() {
                "ctrl" | "control" => &mut mods.ctrl,
                "alt" | "option" => &mut mods.alt,
                "shift" => &mut mods.shift,
                "meta" | "win" | "super" | "cmd" | "command" => &mut mods.meta,
                _ => break,
            };
            if tail.trim().is_empty() {
                return None;
            }
            *m = true;
            rest = tail;
        }
        Some(Self {
            mods,
            key: Key::parse(rest)?,
        })
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = &self.mods;
        for (on, name) in [
            (m.ctrl, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.meta, "Meta"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

impl std::str::FromStr for Hotkey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Self::parse(s).ok_or_else(|| format!("not a hotkey: {s}"))
    }
}

impl From<Hotkey> for String {
    fn from(h: Hotkey) -> String {
        h.to_string()
    }
}

impl TryFrom<String> for Hotkey {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}

/// Actions a global hotkey can trigger (11 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assignable {
    Play,
    Pause,
    PlayPause,
    Stop,
    StopWithFade,
    StopAfterCurrent,
    Previous,
    Next,
    StartOfList,
    EndOfList,
    TenTracksBack,
    TenTracksForward,
    VolumeUp,
    VolumeDown,
    Forward5s,
    Rewind5s,
    RepeatOn,
    RepeatOff,
    RepeatToggle,
    ShuffleToggle,
    ManualAdvanceToggle,
    JumpToFile,
    OpenFile,
    OpenUrl,
    OpenFolder,
    ToggleEqualizer,
    TogglePlaylist,
    ToggleMainWindow,
    AlwaysOnTop,
    DoubleSize,
    WindowshadeMain,
    WindowshadePlaylist,
    Preferences,
    SkinBrowser,
    About,
    VisStartStop,
    VisConfigure,
    VisNextPreset,
    VisPreviousPreset,
    VisRandomPreset,
    VisFullScreen,
    CopyTitle,
    CopyPath,
    ShowInFolder,
    FileInfo,
    Rate0,
    Rate1,
    Rate2,
    Rate3,
    Rate4,
    Rate5,
    RateUp,
    RateDown,
    BringToFront,
    Hide,
    Quit,
}

impl Assignable {
    /// The settings-file name (snake case, as in the JSON protocol).
    pub fn name(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }

    /// Parses a name written by [`Assignable::name`].
    pub fn from_name(name: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(name.trim().to_string())).ok()
    }

    /// The remote-control request with the same effect, when there is one. The other actions
    /// are handled by the user interface itself.
    pub fn to_request(self) -> Option<Request> {
        use Assignable as A;
        use NumberedCommand as N;
        Some(match self {
            A::Play => Request::Play,
            A::Pause => Request::Pause,
            A::PlayPause => Request::PlayPause,
            A::Stop => Request::Stop,
            A::StopWithFade => Request::StopWithFade,
            A::StopAfterCurrent => Request::StopAfterCurrent,
            A::Previous => Request::Previous,
            A::Next => Request::Next,
            A::StartOfList => Request::StartOfList,
            A::EndOfList => Request::EndOfList,
            A::TenTracksBack => Request::Command(N::TenTracksBack),
            A::TenTracksForward => Request::Command(N::TenTracksForward),
            A::VolumeUp => Request::Command(N::VolumeUp),
            A::VolumeDown => Request::Command(N::VolumeDown),
            A::Forward5s => Request::SeekRelative(5000),
            A::Rewind5s => Request::SeekRelative(-5000),
            A::RepeatOn => Request::SetRepeat(Flag::On),
            A::RepeatOff => Request::SetRepeat(Flag::Off),
            A::RepeatToggle => Request::SetRepeat(Flag::Toggle),
            A::ShuffleToggle => Request::SetShuffle(Flag::Toggle),
            A::ManualAdvanceToggle => Request::SetManualAdvance(Flag::Toggle),
            A::JumpToFile => Request::JumpToFile,
            A::OpenFile => Request::Command(N::OpenFileDialog),
            A::OpenUrl => Request::Command(N::OpenUrlDialog),
            A::OpenFolder => Request::Command(N::OpenFolder),
            A::ToggleEqualizer => Request::Command(N::ToggleEqualizer),
            A::TogglePlaylist => Request::Command(N::TogglePlaylist),
            A::ToggleMainWindow => Request::Command(N::ToggleMainWindow),
            A::AlwaysOnTop => Request::Command(N::AlwaysOnTop),
            A::DoubleSize => Request::Command(N::DoubleSize),
            A::WindowshadeMain => Request::Command(N::MainWindowshade),
            A::WindowshadePlaylist => Request::Command(N::PlaylistWindowshade),
            A::Preferences => Request::Command(N::Preferences),
            A::SkinBrowser => Request::Command(N::SkinBrowser),
            A::About => Request::Command(N::About),
            A::VisStartStop => Request::Command(N::VisualizationStartStop),
            A::FileInfo => Request::Command(N::FileInfo),
            A::BringToFront => Request::Raise,
            A::Quit => Request::Quit,
            _ => return None,
        })
    }
}

/// One binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub hotkey: Hotkey,
    pub action: Assignable,
}

/// The default bindings (11 §5).
pub fn default_bindings() -> Vec<Binding> {
    let ca = |key| Hotkey::new(Modifiers::CTRL_ALT, key);
    let plain = |key| Hotkey::new(Modifiers::NONE, key);
    [
        (ca(Key::Insert), Assignable::Play),
        (ca(Key::Home), Assignable::Pause),
        (ca(Key::End), Assignable::Stop),
        (ca(Key::PageUp), Assignable::Previous),
        (ca(Key::PageDown), Assignable::Next),
        (ca(Key::Up), Assignable::VolumeUp),
        (ca(Key::Down), Assignable::VolumeDown),
        (ca(Key::Right), Assignable::Forward5s),
        (ca(Key::Left), Assignable::Rewind5s),
        (ca(Key::Char('J')), Assignable::JumpToFile),
        (ca(Key::Char('L')), Assignable::OpenFile),
        (plain(Key::MediaPlayPause), Assignable::PlayPause),
        (plain(Key::MediaStop), Assignable::Stop),
        (plain(Key::MediaPrevious), Assignable::Previous),
        (plain(Key::MediaNext), Assignable::Next),
    ]
    .into_iter()
    .map(|(hotkey, action)| Binding { hotkey, action })
    .collect()
}

/// Global hotkey preferences. Off by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeySettings {
    pub enabled: bool,
    pub bindings: Vec<Binding>,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bindings: default_bindings(),
        }
    }
}

impl HotkeySettings {
    /// The action bound to a hotkey.
    pub fn action(&self, hotkey: Hotkey) -> Option<Assignable> {
        self.bindings
            .iter()
            .find(|b| b.hotkey == hotkey)
            .map(|b| b.action)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn action_names_round_trip() {
        assert_eq!(Assignable::PlayPause.name(), "play_pause");
        for b in default_bindings() {
            assert_eq!(Assignable::from_name(&b.action.name()), Some(b.action));
        }
        assert_eq!(Assignable::from_name("nope"), None);
    }

    use super::*;

    fn hk(s: &str) -> Hotkey {
        Hotkey::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn parses_spec_examples() {
        assert_eq!(
            hk("Ctrl+Alt+Insert"),
            Hotkey::new(Modifiers::CTRL_ALT, Key::Insert)
        );
        assert_eq!(hk("Ctrl+Alt+Page Up").key, Key::PageUp);
        assert_eq!(hk("ctrl + alt + pgdn").key, Key::PageDown);
        assert_eq!(hk("Ctrl+Alt+J").key, Key::Char('J'));
        assert_eq!(hk("ctrl+alt+j").key, Key::Char('J'));
        assert_eq!(
            hk("Media Play/Pause"),
            Hotkey::new(Modifiers::NONE, Key::MediaPlayPause)
        );
        assert_eq!(
            hk("Shift+F10"),
            Hotkey::new(
                Modifiers {
                    shift: true,
                    ..Modifiers::NONE
                },
                Key::F(10)
            )
        );
        assert_eq!(hk("Win+Num 5").key, Key::Numpad(5));
        assert!(hk("Cmd+Numpad5").mods.meta);
        assert_eq!(
            hk("Control+Option+Del"),
            Hotkey::new(Modifiers::CTRL_ALT, Key::Delete)
        );
    }

    #[test]
    fn plus_key() {
        assert_eq!(
            hk("Ctrl++"),
            Hotkey::new(
                Modifiers {
                    ctrl: true,
                    ..Modifiers::NONE
                },
                Key::Char('+')
            )
        );
        assert_eq!(hk("+").key, Key::Char('+'));
        assert_eq!(hk("Ctrl+Num +").key, Key::NumpadAdd);
        assert_eq!(hk("Alt+Num -").key, Key::NumpadSubtract);
    }

    #[test]
    fn rejects_bad_input() {
        for s in [
            "", "Ctrl+", "Ctrl+Alt", "Hyper+X", "F25", "F0", "Num 10", "é", "Ctrl+Foo",
        ] {
            assert_eq!(Hotkey::parse(s), None, "{s}");
        }
    }

    #[test]
    fn display_round_trips() {
        for s in [
            "Ctrl+Alt+Insert",
            "Ctrl+Alt+Page Down",
            "Media Play/Pause",
            "Ctrl+Alt+Shift+Meta+F12",
            "Num 0",
            "Num .",
            "Ctrl++",
            "Esc",
        ] {
            assert_eq!(hk(s).to_string(), s);
        }
        assert_eq!(hk("ctrl+alt+pgup").to_string(), "Ctrl+Alt+Page Up");
        assert_eq!(hk("alt+ctrl+x").to_string(), "Ctrl+Alt+X");
    }

    #[test]
    fn every_named_key_round_trips() {
        for (k, names) in NAMED {
            for n in *names {
                assert_eq!(Key::parse(n), Some(*k), "{n}");
            }
            assert_eq!(Key::parse(&k.to_string()), Some(*k));
        }
    }

    #[test]
    fn default_table() {
        let s = HotkeySettings::default();
        assert!(!s.enabled);
        assert_eq!(s.bindings.len(), 15);
        assert_eq!(s.action(hk("Ctrl+Alt+Insert")), Some(Assignable::Play));
        assert_eq!(s.action(hk("Ctrl+Alt+Home")), Some(Assignable::Pause));
        assert_eq!(s.action(hk("Ctrl+Alt+End")), Some(Assignable::Stop));
        assert_eq!(s.action(hk("Ctrl+Alt+Page Up")), Some(Assignable::Previous));
        assert_eq!(s.action(hk("Ctrl+Alt+Page Down")), Some(Assignable::Next));
        assert_eq!(s.action(hk("Ctrl+Alt+Up")), Some(Assignable::VolumeUp));
        assert_eq!(s.action(hk("Ctrl+Alt+Down")), Some(Assignable::VolumeDown));
        assert_eq!(s.action(hk("Ctrl+Alt+Right")), Some(Assignable::Forward5s));
        assert_eq!(s.action(hk("Ctrl+Alt+Left")), Some(Assignable::Rewind5s));
        assert_eq!(s.action(hk("Ctrl+Alt+J")), Some(Assignable::JumpToFile));
        assert_eq!(s.action(hk("Ctrl+Alt+L")), Some(Assignable::OpenFile));
        assert_eq!(
            s.action(hk("Media Play/Pause")),
            Some(Assignable::PlayPause)
        );
        assert_eq!(s.action(hk("Media Stop")), Some(Assignable::Stop));
        assert_eq!(s.action(hk("Media Previous")), Some(Assignable::Previous));
        assert_eq!(s.action(hk("Media Next")), Some(Assignable::Next));
        assert_eq!(s.action(hk("Ctrl+Alt+X")), None);
    }

    #[test]
    fn settings_serialise_as_text() {
        let s = HotkeySettings::default();
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains(r#"{"hotkey":"Ctrl+Alt+Insert","action":"play"}"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<HotkeySettings>(&json).unwrap(), s);
        assert!(serde_json::from_str::<Hotkey>(r#""Ctrl+Nope""#).is_err());
    }

    #[test]
    fn actions_to_requests() {
        assert_eq!(Assignable::Play.to_request(), Some(Request::Play));
        assert_eq!(
            Assignable::Rewind5s.to_request(),
            Some(Request::SeekRelative(-5000))
        );
        assert_eq!(
            Assignable::RepeatOff.to_request(),
            Some(Request::SetRepeat(Flag::Off))
        );
        assert_eq!(
            Assignable::OpenFile.to_request(),
            Some(Request::Command(NumberedCommand::OpenFileDialog))
        );
        assert_eq!(Assignable::BringToFront.to_request(), Some(Request::Raise));
        assert_eq!(Assignable::CopyTitle.to_request(), None);
        assert_eq!(Assignable::Rate3.to_request(), None);
    }
}
