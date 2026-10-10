//! Player commands (07 §2, 11 §6) and the player state the user interface shows.

/// Playback state. Remote-control values: 0 stopped, 1 playing, 3 paused (07 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

impl PlayState {
    pub fn remote_value(self) -> i32 {
        match self {
            Self::Stopped => 0,
            Self::Playing => 1,
            Self::Paused => 3,
        }
    }
}

/// Commands handled by the player core.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Previous,
    /// Play: start, unpause, or restart (07 §2.2).
    Play,
    Pause,
    Stop,
    StopWithFade,
    ToggleStopAfterCurrent,
    Next,
    /// Seek relative to the current position, in milliseconds.
    SeekRelative(i64),
    /// Seek to an absolute position in milliseconds.
    SeekTo(u64),
    StartOfList,
    EndOfList,
    /// Move by n entries (±10 for Numpad 1/3).
    JumpEntries(i32),
    SetVolume(u8),
    /// Change volume by a delta, clamped.
    VolumeBy(i32),
    SetBalance(i8),
    SetShuffle(bool),
    SetRepeat(bool),
    SetManualAdvance(bool),
    /// Play the playlist entry at an index.
    PlayIndex(usize),
}

/// What the player reports to the user interface.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerStatus {
    pub state: PlayState,
    /// Heard position in ms.
    pub position_ms: u64,
    /// Length in ms; `None` when unknown (streams).
    pub length_ms: Option<u64>,
    pub seekable: bool,
    pub kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub in_sync: bool,
    pub volume: u8,
    pub balance: i8,
    pub shuffle: bool,
    pub repeat: bool,
    pub manual_advance: bool,
    pub stop_after_current: bool,
    /// 0-based playlist position.
    pub position: usize,
    /// Title of the current entry (already formatted, without number).
    pub title: String,
    pub playlist_len: usize,
}
