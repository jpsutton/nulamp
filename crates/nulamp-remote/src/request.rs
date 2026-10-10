//! Remote-control requests and responses (11 §6.1), and their wire form.
//!
//! The protocol is line based: each message is one JSON object followed by `\n`. A client sends
//! a [`Request`] and reads one [`Response`]; it may send further requests on the same
//! connection. Requests are `{"cmd":"<name>"}` or `{"cmd":"<name>","arg":<value>}`.

use nulamp_core::{PlayState, PlayerStatus};
use serde::{Deserialize, Serialize};

use crate::NumberedCommand;

/// Turn an option on, off, or toggle it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flag {
    Off,
    On,
    Toggle,
}

impl Flag {
    /// The new value given the current one.
    pub fn apply(self, current: bool) -> bool {
        match self {
            Self::Off => false,
            Self::On => true,
            Self::Toggle => !current,
        }
    }
}

impl From<bool> for Flag {
    fn from(on: bool) -> Self {
        if on { Self::On } else { Self::Off }
    }
}

/// A command or query for the running player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "arg", rename_all = "snake_case")]
pub enum Request {
    /// Files, folders or URLs from a launch or an "open" call. Without `enqueue` the player
    /// clears the playlist, adds them and plays; with `enqueue` (`/ADD`) it only appends.
    Open {
        files: Vec<String>,
        enqueue: bool,
    },
    /// Install and apply a skin archive (after the confirmation preference, 11 §4.1).
    LoadSkin(String),
    /// Show the player and bring it to the front.
    Raise,
    Play,
    Pause,
    /// Pause if playing, else play.
    PlayPause,
    Stop,
    StopWithFade,
    /// Toggle stop after current.
    StopAfterCurrent,
    Previous,
    Next,
    StartOfList,
    EndOfList,
    /// Start playback at the beginning of the list, respecting shuffle.
    StartPlayback,
    /// Seek relative to the current position, in milliseconds.
    SeekRelative(i64),
    /// Seek to an absolute position in milliseconds.
    SeekTo(u64),
    /// Volume 0–255.
    SetVolume(u8),
    /// Change the volume by a delta, clamped to 0–255.
    VolumeBy(i32),
    /// Balance −127..127.
    SetBalance(i8),
    /// Change the balance by a delta, clamped.
    BalanceBy(i32),
    SetShuffle(Flag),
    SetRepeat(Flag),
    SetManualAdvance(Flag),
    /// Clear the playlist.
    Clear,
    /// Open the "jump to file" dialog.
    JumpToFile,
    /// Play the audio CD in drive n (0–3).
    PlayCd(u8),
    /// Add a location to the bookmarks.
    Bookmark(String),
    /// Save the default playlist.
    SavePlaylist,
    /// Close the player.
    Quit,
    /// Close the player; the server force-exits the process if it still runs after 3 s.
    Kill,
    /// A numbered command (11 §6.3).
    Command(NumberedCommand),
    /// Set an EQ band (0–9) to a value 0..63 (0 = +12 dB, 31 = 0 dB, 63 = −12 dB).
    SetEqBand {
        band: u8,
        value: u8,
    },
    /// Set the EQ preamp, 0..63 like the bands.
    SetEqPreamp(u8),
    SetEqEnabled(bool),
    SetEqAutoLoad(bool),
    /// Set the current playlist position without starting playback.
    SetPlaylistPosition(usize),
    /// Answered by [`Response::Version`].
    GetVersion,
    /// Answered by [`Response::Status`].
    GetStatus,
    /// Answered by [`Response::Eq`].
    GetEq,
    /// Answered by [`Response::PlaylistPosition`].
    GetPlaylistPosition,
    /// Location and title of an entry; answered by [`Response::Entry`].
    GetEntry(usize),
    /// Answered by [`Response::Title`].
    GetCurrentTitle,
}

impl Request {
    /// Whether the request only reads state.
    pub fn is_query(&self) -> bool {
        matches!(
            self,
            Self::GetVersion
                | Self::GetStatus
                | Self::GetEq
                | Self::GetPlaylistPosition
                | Self::GetEntry(_)
                | Self::GetCurrentTitle
        )
    }
}

/// Playback state on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    #[default]
    Stopped,
    Playing,
    Paused,
}

impl From<PlayState> for State {
    fn from(s: PlayState) -> Self {
        match s {
            PlayState::Stopped => Self::Stopped,
            PlayState::Playing => Self::Playing,
            PlayState::Paused => Self::Paused,
        }
    }
}

impl From<State> for PlayState {
    fn from(s: State) -> Self {
        match s {
            State::Stopped => Self::Stopped,
            State::Playing => Self::Playing,
            State::Paused => Self::Paused,
        }
    }
}

/// Answer to [`Request::GetStatus`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Status {
    pub state: State,
    pub position_ms: u64,
    /// `None` when unknown (streams).
    pub length_ms: Option<u64>,
    /// 0-based playlist position.
    pub playlist_position: usize,
    pub playlist_length: usize,
    pub volume: u8,
    pub balance: i8,
    pub shuffle: bool,
    pub repeat: bool,
    pub manual_advance: bool,
    pub stop_after_current: bool,
    pub sample_rate: Option<u32>,
    pub kbps: Option<u32>,
    pub channels: Option<u32>,
    pub title: String,
}

impl From<&PlayerStatus> for Status {
    fn from(s: &PlayerStatus) -> Self {
        Self {
            state: s.state.into(),
            position_ms: s.position_ms,
            length_ms: s.length_ms,
            playlist_position: s.position,
            playlist_length: s.playlist_len,
            volume: s.volume,
            balance: s.balance,
            shuffle: s.shuffle,
            repeat: s.repeat,
            manual_advance: s.manual_advance,
            stop_after_current: s.stop_after_current,
            sample_rate: s.sample_rate,
            kbps: s.kbps,
            channels: s.channels,
            title: s.title.clone(),
        }
    }
}

/// Answer to [`Request::GetEq`]. Values 0..63 (0 = +12 dB, 31 = 0 dB, 63 = −12 dB).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EqValues {
    pub bands: [u8; 10],
    pub preamp: u8,
    pub enabled: bool,
    pub auto_load: bool,
}

/// The answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Response {
    /// The command was accepted.
    Ok,
    /// The request failed or could not be read.
    Error(String),
    Version {
        version: String,
        pid: u32,
    },
    Status(Status),
    Eq(EqValues),
    PlaylistPosition {
        position: usize,
        length: usize,
    },
    Entry {
        location: String,
        title: String,
    },
    Title(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(r: Request) {
        let s = serde_json::to_string(&r).unwrap();
        assert!(!s.contains('\n'));
        assert_eq!(serde_json::from_str::<Request>(&s).unwrap(), r, "{s}");
    }

    #[test]
    fn wire_form() {
        assert_eq!(
            serde_json::to_string(&Request::Play).unwrap(),
            r#"{"cmd":"play"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::SetVolume(200)).unwrap(),
            r#"{"cmd":"set_volume","arg":200}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::Open {
                files: vec!["a b.mp3".into()],
                enqueue: true
            })
            .unwrap(),
            r#"{"cmd":"open","arg":{"files":["a b.mp3"],"enqueue":true}}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::Command(NumberedCommand::Next)).unwrap(),
            r#"{"cmd":"command","arg":40048}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::SetShuffle(Flag::Toggle)).unwrap(),
            r#"{"cmd":"set_shuffle","arg":"toggle"}"#
        );
        assert_eq!(
            serde_json::to_string(&Response::Ok).unwrap(),
            r#"{"kind":"ok"}"#
        );
    }

    #[test]
    fn requests_round_trip() {
        for r in [
            Request::Open {
                files: vec!["x\ny".into(), "ü".into()],
                enqueue: false,
            },
            Request::LoadSkin("a.wsz".into()),
            Request::SeekRelative(-5000),
            Request::SeekTo(12),
            Request::VolumeBy(-4),
            Request::SetBalance(-127),
            Request::BalanceBy(12),
            Request::SetRepeat(Flag::On),
            Request::SetManualAdvance(Flag::Off),
            Request::PlayCd(3),
            Request::Bookmark("/m/a.mp3".into()),
            Request::Kill,
            Request::SetEqBand { band: 9, value: 63 },
            Request::SetEqPreamp(31),
            Request::SetEqEnabled(true),
            Request::SetEqAutoLoad(false),
            Request::SetPlaylistPosition(7),
            Request::GetEntry(2),
            Request::GetCurrentTitle,
        ] {
            round_trip(r);
        }
    }

    #[test]
    fn responses_round_trip() {
        for r in [
            Response::Error("x".into()),
            Response::Version {
                version: "1.0".into(),
                pid: 4,
            },
            Response::Status(Status {
                length_ms: Some(3),
                ..Default::default()
            }),
            Response::Eq(EqValues {
                bands: [1; 10],
                preamp: 2,
                enabled: true,
                auto_load: true,
            }),
            Response::PlaylistPosition {
                position: 1,
                length: 2,
            },
            Response::Entry {
                location: "l".into(),
                title: "t".into(),
            },
            Response::Title("t".into()),
        ] {
            let s = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_json::from_str::<Response>(&s).unwrap(), r);
        }
    }

    #[test]
    fn flag_apply() {
        assert!(Flag::Toggle.apply(false));
        assert!(!Flag::Toggle.apply(true));
        assert!(Flag::On.apply(false));
        assert!(!Flag::Off.apply(true));
        assert_eq!(Flag::from(true), Flag::On);
    }

    #[test]
    fn status_from_player() {
        let p = PlayerStatus {
            state: PlayState::Paused,
            position_ms: 10,
            playlist_len: 3,
            position: 1,
            title: "T".into(),
            ..Default::default()
        };
        let s = Status::from(&p);
        assert_eq!(s.state, State::Paused);
        assert_eq!((s.playlist_position, s.playlist_length), (1, 3));
        assert_eq!(s.title, "T");
        assert_eq!(PlayState::from(s.state), PlayState::Paused);
    }

    #[test]
    fn queries() {
        assert!(Request::GetStatus.is_query());
        assert!(!Request::Play.is_query());
    }
}
