//! Player state pushed to the OS media-session interface ([`crate::Mpris`]).

use nulamp_core::{PlayState, PlayerStatus};

/// What the media-session interface shows about the player.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerSnapshot {
    pub state: PlayState,
    /// Heard position in ms.
    pub position_ms: u64,
    /// `None` when unknown (streams).
    pub length_ms: Option<u64>,
    pub seekable: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Path or URL of the current entry; empty when there is none.
    pub location: String,
    /// Volume 0–255.
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: bool,
    pub manual_advance: bool,
    pub playlist_len: usize,
    /// Identity of the current entry, stable while it stays in the playlist; `None` when there
    /// is no current entry.
    pub track_id: Option<u64>,
}

impl PlayerSnapshot {
    /// Fills the fields [`PlayerStatus`] has. `artist`, `album`, `location` and `track_id` are
    /// left for the caller.
    pub fn from_status(s: &PlayerStatus) -> Self {
        Self {
            state: s.state,
            position_ms: s.position_ms,
            length_ms: s.length_ms,
            seekable: s.seekable,
            title: s.title.clone(),
            volume: s.volume,
            shuffle: s.shuffle,
            repeat: s.repeat,
            manual_advance: s.manual_advance,
            playlist_len: s.playlist_len,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_status() {
        let s = PlayerStatus {
            state: PlayState::Playing,
            position_ms: 5,
            length_ms: Some(9),
            volume: 100,
            repeat: true,
            playlist_len: 2,
            title: "T".into(),
            ..Default::default()
        };
        let p = PlayerSnapshot::from_status(&s);
        assert_eq!(p.state, PlayState::Playing);
        assert_eq!((p.position_ms, p.length_ms, p.volume), (5, Some(9), 100));
        assert!(p.repeat && !p.shuffle);
        assert_eq!(p.title, "T");
        assert_eq!(p.track_id, None);
    }
}
