//! Global hotkeys registered with the operating system (11 §5).
//!
//! X11 (also XWayland), Windows and macOS through `global-hotkey`. Native Wayland sessions
//! have no global key grabs; registration fails there and is reported once in the log.

use std::collections::HashMap;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use nulamp_remote::hotkey::{Key, Modifiers as Mods};
use nulamp_remote::{Assignable, Hotkey, HotkeySettings};

/// The OS key code for a key, when the OS layer has one.
fn code(key: Key) -> Option<Code> {
    use Code as C;
    Some(match key {
        Key::Char(c) => match c {
            'A'..='Z' => [
                C::KeyA,
                C::KeyB,
                C::KeyC,
                C::KeyD,
                C::KeyE,
                C::KeyF,
                C::KeyG,
                C::KeyH,
                C::KeyI,
                C::KeyJ,
                C::KeyK,
                C::KeyL,
                C::KeyM,
                C::KeyN,
                C::KeyO,
                C::KeyP,
                C::KeyQ,
                C::KeyR,
                C::KeyS,
                C::KeyT,
                C::KeyU,
                C::KeyV,
                C::KeyW,
                C::KeyX,
                C::KeyY,
                C::KeyZ,
            ][(c as u8 - b'A') as usize],
            '0'..='9' => [
                C::Digit0,
                C::Digit1,
                C::Digit2,
                C::Digit3,
                C::Digit4,
                C::Digit5,
                C::Digit6,
                C::Digit7,
                C::Digit8,
                C::Digit9,
            ][(c as u8 - b'0') as usize],
            '\\' => C::Backslash,
            '[' => C::BracketLeft,
            ']' => C::BracketRight,
            '`' => C::Backquote,
            ',' => C::Comma,
            '=' => C::Equal,
            '-' => C::Minus,
            '.' => C::Period,
            '\'' => C::Quote,
            ';' => C::Semicolon,
            '/' => C::Slash,
            _ => return None,
        },
        Key::F(n) => [
            C::F1,
            C::F2,
            C::F3,
            C::F4,
            C::F5,
            C::F6,
            C::F7,
            C::F8,
            C::F9,
            C::F10,
            C::F11,
            C::F12,
            C::F13,
            C::F14,
            C::F15,
            C::F16,
            C::F17,
            C::F18,
            C::F19,
            C::F20,
            C::F21,
            C::F22,
            C::F23,
            C::F24,
        ]
        .get(usize::from(n).checked_sub(1)?)
        .copied()?,
        Key::Insert => C::Insert,
        Key::Delete => C::Delete,
        Key::Home => C::Home,
        Key::End => C::End,
        Key::PageUp => C::PageUp,
        Key::PageDown => C::PageDown,
        Key::Up => C::ArrowUp,
        Key::Down => C::ArrowDown,
        Key::Left => C::ArrowLeft,
        Key::Right => C::ArrowRight,
        Key::Space => C::Space,
        Key::Enter => C::Enter,
        Key::Tab => C::Tab,
        Key::Escape => C::Escape,
        Key::Backspace => C::Backspace,
        Key::Pause => C::Pause,
        Key::Numpad(n) => [
            C::Numpad0,
            C::Numpad1,
            C::Numpad2,
            C::Numpad3,
            C::Numpad4,
            C::Numpad5,
            C::Numpad6,
            C::Numpad7,
            C::Numpad8,
            C::Numpad9,
        ]
        .get(usize::from(n))
        .copied()?,
        Key::NumpadAdd => C::NumpadAdd,
        Key::NumpadSubtract => C::NumpadSubtract,
        Key::NumpadMultiply => C::NumpadMultiply,
        Key::NumpadDivide => C::NumpadDivide,
        Key::NumpadDecimal => C::NumpadDecimal,
        Key::NumpadEnter => C::NumpadEnter,
        // The X11 layer knows the play key only as "play"; it is the play/pause key there.
        #[cfg(all(unix, not(target_os = "macos")))]
        Key::MediaPlayPause => C::MediaPlay,
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        Key::MediaPlayPause => C::MediaPlayPause,
        Key::MediaStop => C::MediaStop,
        Key::MediaPrevious => C::MediaTrackPrevious,
        Key::MediaNext => C::MediaTrackNext,
        Key::VolumeUp => C::AudioVolumeUp,
        Key::VolumeDown => C::AudioVolumeDown,
        Key::VolumeMute => C::AudioVolumeMute,
    })
}

/// The OS hotkey for a binding.
pub fn to_os(h: Hotkey) -> Option<HotKey> {
    let mut m = Modifiers::empty();
    let Mods {
        ctrl,
        alt,
        shift,
        meta,
    } = h.mods;
    m.set(Modifiers::CONTROL, ctrl);
    m.set(Modifiers::ALT, alt);
    m.set(Modifiers::SHIFT, shift);
    m.set(Modifiers::SUPER, meta);
    Some(HotKey::new((!m.is_empty()).then_some(m), code(h.key)?))
}

/// Registered hotkeys and their actions.
pub struct GlobalHotkeys {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    actions: HashMap<u32, Assignable>,
}

impl GlobalHotkeys {
    /// Connects to the OS hotkey service.
    pub fn new() -> Option<Self> {
        match GlobalHotKeyManager::new() {
            Ok(manager) => Some(Self {
                manager,
                registered: Vec::new(),
                actions: HashMap::new(),
            }),
            Err(e) => {
                log::info!("global hotkeys not available: {e}");
                None
            }
        }
    }

    /// Replaces the registered hotkeys with the enabled bindings. Bindings the OS refuses
    /// (taken by another program, or no OS key) are skipped and logged.
    pub fn apply(&mut self, settings: &HotkeySettings) {
        if !self.registered.is_empty() {
            let _ = self.manager.unregister_all(&self.registered);
        }
        self.registered.clear();
        self.actions.clear();
        if !settings.enabled {
            return;
        }
        for b in &settings.bindings {
            let Some(os) = to_os(b.hotkey) else {
                log::info!("hotkey {} has no OS key", b.hotkey);
                continue;
            };
            if self.actions.contains_key(&os.id()) {
                continue;
            }
            match self.manager.register(os) {
                Ok(()) => {
                    self.registered.push(os);
                    self.actions.insert(os.id(), b.action);
                }
                Err(e) => log::info!("hotkey {} not registered: {e}", b.hotkey),
            }
        }
    }

    /// Actions of the hotkeys pressed since the last poll.
    pub fn poll(&self) -> Vec<Assignable> {
        let mut out = Vec::new();
        while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
            if ev.state() == HotKeyState::Pressed
                && let Some(a) = self.actions.get(&ev.id())
            {
                out.push(*a);
            }
        }
        out
    }
}

impl Drop for GlobalHotkeys {
    fn drop(&mut self) {
        let _ = self.manager.unregister_all(&self.registered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_binding_has_an_os_key() {
        for b in nulamp_remote::hotkey::default_bindings() {
            assert!(to_os(b.hotkey).is_some(), "{}", b.hotkey);
        }
    }

    #[test]
    fn modifiers_and_keys_map() {
        let h = Hotkey::parse("Ctrl+Alt+J").unwrap();
        let os = to_os(h).unwrap();
        assert_eq!(
            os,
            HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyJ)
        );
        assert_eq!(
            to_os(Hotkey::parse("F13").unwrap()).unwrap(),
            HotKey::new(None, Code::F13)
        );
        assert_eq!(to_os(Hotkey::parse("Shift+~").unwrap()), None);
    }
}
