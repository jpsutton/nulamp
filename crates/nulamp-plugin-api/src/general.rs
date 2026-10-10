//! General modules: player events (08 §7, 11 §6.1).

use std::ffi::CString;

use crate::module::Module;
use crate::sys::*;

/// A player event for general modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginEvent {
    TrackStarted {
        location: String,
        position: usize,
    },
    /// `by_user`: the user stopped (false: the playlist ended).
    TrackStopped {
        by_user: bool,
    },
    TitleChanged(String),
    VolumeChanged(u8),
    EqChanged,
    PlaylistChanged {
        len: usize,
    },
    /// The skin's name or path.
    SkinChanged(String),
    Paused,
    Unpaused,
    InfoChanged {
        kbps: Option<u32>,
    },
}

impl PluginEvent {
    /// The event number, value and text of the C record.
    fn parts(&self) -> (u32, i64, &str) {
        let n = |v: usize| v.min(i64::MAX as usize) as i64;
        match self {
            Self::TrackStarted { location, position } => {
                (NULAMP_EVENT_TRACK_STARTED, n(*position), location)
            }
            Self::TrackStopped { by_user } => (NULAMP_EVENT_TRACK_STOPPED, *by_user as i64, ""),
            Self::TitleChanged(t) => (NULAMP_EVENT_TITLE_CHANGED, 0, t),
            Self::VolumeChanged(v) => (NULAMP_EVENT_VOLUME_CHANGED, *v as i64, ""),
            Self::EqChanged => (NULAMP_EVENT_EQ_CHANGED, 0, ""),
            Self::PlaylistChanged { len } => (NULAMP_EVENT_PLAYLIST_CHANGED, n(*len), ""),
            Self::SkinChanged(s) => (NULAMP_EVENT_SKIN_CHANGED, 0, s),
            Self::Paused => (NULAMP_EVENT_PAUSED, 0, ""),
            Self::Unpaused => (NULAMP_EVENT_UNPAUSED, 0, ""),
            Self::InfoChanged { kbps } => {
                (NULAMP_EVENT_INFO_CHANGED, kbps.map_or(-1, |k| k as i64), "")
            }
        }
    }
}

/// A general module.
#[derive(Debug, Clone)]
pub struct PluginGeneral {
    module: Module,
    api: NulampGeneralApi,
}

impl PluginGeneral {
    pub(crate) fn new(module: Module) -> Self {
        // SAFETY: the loader checked that `api` is a general record of a sufficient size.
        let api = unsafe { *module.api().cast::<NulampGeneralApi>() };
        Self { module, api }
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Delivers an event. User-interface thread only. NUL characters in the text are dropped.
    pub fn send(&self, event: &PluginEvent) {
        let (kind, value, text) = event.parts();
        let text = CString::new(text.replace('\0', "")).expect("NULs removed");
        let ev = NulampEvent {
            abi_version: NULAMP_ABI_VERSION,
            struct_size: size_of::<NulampEvent>() as u32,
            kind,
            reserved: 0,
            value,
            text: text.as_ptr(),
        };
        let on_event = self.api.on_event.expect("checked at load");
        // SAFETY: a valid module function on the user-interface thread (documented); the event
        // and its text live across the call.
        unsafe { on_event(self.module.user(), &ev) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_parts() {
        let e = PluginEvent::TrackStarted {
            location: "/a.mp3".into(),
            position: 4,
        };
        assert_eq!(e.parts(), (NULAMP_EVENT_TRACK_STARTED, 4, "/a.mp3"));
        assert_eq!(
            PluginEvent::TrackStopped { by_user: true }.parts(),
            (NULAMP_EVENT_TRACK_STOPPED, 1, "")
        );
        assert_eq!(
            PluginEvent::InfoChanged { kbps: None }.parts(),
            (NULAMP_EVENT_INFO_CHANGED, -1, "")
        );
        assert_eq!(
            PluginEvent::VolumeChanged(200).parts(),
            (NULAMP_EVENT_VOLUME_CHANGED, 200, "")
        );
    }
}
