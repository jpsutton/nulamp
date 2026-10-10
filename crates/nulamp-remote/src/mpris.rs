//! MPRIS 2 media-player interface (11 §5, §6.1) on the D-Bus session bus.
//!
//! [`Mpris`] exports `org.mpris.MediaPlayer2` and `org.mpris.MediaPlayer2.Player` at
//! `/org/mpris/MediaPlayer2` under the bus name `org.mpris.MediaPlayer2.nulamp`, or
//! `org.mpris.MediaPlayer2.nulamp.instance<pid>` when that name is taken (RC4). Method calls
//! and property writes become [`Request`]s on a [`RequestSender`](crate::RequestSender); the
//! application pushes its state with [`Mpris::update`].
//!
//! Only Linux and the BSDs have the real interface; elsewhere [`Mpris::start`] returns
//! [`Error::Unsupported`](crate::Error::Unsupported). The mapping functions are pure and
//! available everywhere.

use std::time::Duration;

use nulamp_core::PlayState;

use crate::{Flag, PlayerSnapshot, Request};

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
mod dbus;
#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
pub use dbus::Mpris;

/// Preferred bus name.
pub const BUS_NAME: &str = "org.mpris.MediaPlayer2.nulamp";
/// Object path of both interfaces.
pub const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
/// Track id when there is no current track.
pub const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
/// A position this far from the expected one is reported with the `Seeked` signal.
pub const SEEK_TOLERANCE_MS: u64 = 1000;

/// `PlaybackStatus` value.
pub fn playback_status(state: PlayState) -> &'static str {
    match state {
        PlayState::Playing => "Playing",
        PlayState::Paused => "Paused",
        PlayState::Stopped => "Stopped",
    }
}

/// `LoopStatus` value: repeat off → `None`, repeat with manual advance → `Track` (the track
/// repeats), repeat → `Playlist`.
pub fn loop_status(repeat: bool, manual_advance: bool) -> &'static str {
    match (repeat, manual_advance) {
        (false, _) => "None",
        (true, true) => "Track",
        (true, false) => "Playlist",
    }
}

/// Requests that set a `LoopStatus` value; `None` for an unknown value.
pub fn loop_status_requests(status: &str) -> Option<Vec<Request>> {
    Some(match status {
        "None" => vec![Request::SetRepeat(Flag::Off)],
        "Track" => vec![
            Request::SetRepeat(Flag::On),
            Request::SetManualAdvance(Flag::On),
        ],
        "Playlist" => vec![
            Request::SetRepeat(Flag::On),
            Request::SetManualAdvance(Flag::Off),
        ],
        _ => return None,
    })
}

/// Volume 0–255 as an MPRIS volume 0.0–1.0.
pub fn volume_to_mpris(volume: u8) -> f64 {
    f64::from(volume) / 255.0
}

/// MPRIS volume as 0–255; values outside 0.0–1.0 are clamped.
pub fn volume_from_mpris(volume: f64) -> u8 {
    if volume.is_nan() {
        0
    } else {
        (volume.clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

/// `mpris:trackid` object path.
pub fn track_path(track_id: Option<u64>) -> String {
    match track_id {
        Some(id) => format!("/org/nulamp/track/{id}"),
        None => NO_TRACK.into(),
    }
}

/// A location as a URL for `xesam:url`: URLs are kept; paths become `file://` URLs.
pub fn location_to_url(location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let path = location.replace('\\', "/");
    let mut url = String::from("file://");
    if !path.starts_with('/') {
        url.push('/');
    }
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~' | b':') {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

/// A URI given to `OpenUri` as a location: `file://` URLs become paths, others are kept.
pub fn url_to_location(uri: &str) -> String {
    let Some(rest) = uri
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("file://"))
        .map(|_| &uri[7..])
    else {
        return uri.to_string();
    };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    let path = String::from_utf8_lossy(&out).into_owned();
    // `/C:/x` is a Windows drive path.
    let b = path.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return path[1..].to_string();
    }
    path
}

/// `CanGoNext`, `CanGoPrevious`, `CanPlay`, `CanPause`, `CanSeek`.
pub fn capabilities(s: &PlayerSnapshot) -> [bool; 5] {
    let any = s.playlist_len > 0;
    let seek = s.seekable && s.length_ms.is_some() && s.track_id.is_some();
    [any, any, any, any, seek]
}

/// Position now, given the snapshot and the time since it was taken.
pub fn position_at(s: &PlayerSnapshot, elapsed: Duration) -> u64 {
    if s.state != PlayState::Playing {
        return s.position_ms;
    }
    let p = s.position_ms.saturating_add(elapsed.as_millis() as u64);
    s.length_ms.map_or(p, |l| p.min(l))
}

/// Whether `new` jumped away from where `old` would be after `elapsed` on the same track.
pub fn is_seek(old: &PlayerSnapshot, elapsed: Duration, new: &PlayerSnapshot) -> bool {
    if new.track_id.is_none() || old.track_id != new.track_id {
        return false;
    }
    position_at(old, elapsed).abs_diff(new.position_ms) > SEEK_TOLERANCE_MS
}

/// Player properties whose value differs between two snapshots (`Position` never counts).
pub fn changed_properties(old: &PlayerSnapshot, new: &PlayerSnapshot) -> Vec<&'static str> {
    let mut out = Vec::new();
    if old.state != new.state {
        out.push("PlaybackStatus");
    }
    if loop_status(old.repeat, old.manual_advance) != loop_status(new.repeat, new.manual_advance) {
        out.push("LoopStatus");
    }
    if old.shuffle != new.shuffle {
        out.push("Shuffle");
    }
    if (
        &old.track_id,
        &old.length_ms,
        &old.title,
        &old.artist,
        &old.album,
        &old.location,
    ) != (
        &new.track_id,
        &new.length_ms,
        &new.title,
        &new.artist,
        &new.album,
        &new.location,
    ) {
        out.push("Metadata");
    }
    if old.volume != new.volume {
        out.push("Volume");
    }
    let names = [
        "CanGoNext",
        "CanGoPrevious",
        "CanPlay",
        "CanPause",
        "CanSeek",
    ];
    for ((a, b), name) in capabilities(old)
        .into_iter()
        .zip(capabilities(new))
        .zip(names)
    {
        if a != b {
            out.push(name);
        }
    }
    out
}

/// Stand-in where there is no D-Bus session bus.
#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)))]
pub struct Mpris(());

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)))]
impl Mpris {
    /// Always fails with [`Error::Unsupported`](crate::Error::Unsupported).
    pub fn start(_requests: crate::RequestSender) -> crate::Result<Self> {
        Err(crate::Error::Unsupported)
    }

    pub fn bus_name(&self) -> &str {
        BUS_NAME
    }

    pub fn update(&self, _snapshot: &PlayerSnapshot) -> crate::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> PlayerSnapshot {
        PlayerSnapshot {
            state: PlayState::Playing,
            position_ms: 10_000,
            length_ms: Some(60_000),
            seekable: true,
            title: "T".into(),
            location: "/m/a.mp3".into(),
            volume: 200,
            playlist_len: 3,
            track_id: Some(1),
            ..Default::default()
        }
    }

    #[test]
    fn loop_status_mapping() {
        assert_eq!(loop_status(false, false), "None");
        assert_eq!(loop_status(false, true), "None");
        assert_eq!(loop_status(true, true), "Track");
        assert_eq!(loop_status(true, false), "Playlist");
        assert_eq!(
            loop_status_requests("None"),
            Some(vec![Request::SetRepeat(Flag::Off)])
        );
        assert_eq!(
            loop_status_requests("Track"),
            Some(vec![
                Request::SetRepeat(Flag::On),
                Request::SetManualAdvance(Flag::On)
            ])
        );
        assert_eq!(
            loop_status_requests("Playlist"),
            Some(vec![
                Request::SetRepeat(Flag::On),
                Request::SetManualAdvance(Flag::Off)
            ])
        );
        assert_eq!(loop_status_requests("track"), None);
    }

    #[test]
    fn volume_mapping() {
        assert_eq!(volume_to_mpris(0), 0.0);
        assert_eq!(volume_to_mpris(255), 1.0);
        for v in 0..=255u8 {
            assert_eq!(volume_from_mpris(volume_to_mpris(v)), v);
        }
        assert_eq!(volume_from_mpris(-0.5), 0);
        assert_eq!(volume_from_mpris(1.7), 255);
        assert_eq!(volume_from_mpris(f64::NAN), 0);
        assert_eq!(volume_from_mpris(0.5), 128);
    }

    #[test]
    fn status_and_track() {
        assert_eq!(playback_status(PlayState::Playing), "Playing");
        assert_eq!(playback_status(PlayState::Paused), "Paused");
        assert_eq!(playback_status(PlayState::Stopped), "Stopped");
        assert_eq!(track_path(Some(42)), "/org/nulamp/track/42");
        assert_eq!(track_path(None), NO_TRACK);
    }

    #[test]
    fn urls() {
        assert_eq!(location_to_url("/m/a b#1.mp3"), "file:///m/a%20b%231.mp3");
        assert_eq!(location_to_url("/m/ü.ogg"), "file:///m/%C3%BC.ogg");
        assert_eq!(location_to_url("C:\\m\\a.mp3"), "file:///C:/m/a.mp3");
        assert_eq!(location_to_url("http://x/s?a=1"), "http://x/s?a=1");
        assert_eq!(url_to_location("file:///m/a%20b%231.mp3"), "/m/a b#1.mp3");
        assert_eq!(url_to_location("FILE://localhost/m/%C3%BC.ogg"), "/m/ü.ogg");
        assert_eq!(url_to_location("file:///C:/m/a.mp3"), "C:/m/a.mp3");
        assert_eq!(url_to_location("file:///a%zz"), "/a%zz");
        assert_eq!(url_to_location("https://x/y%20z"), "https://x/y%20z");
        for p in ["/m/a b.mp3", "/m/100%.flac", "/m/ü"] {
            assert_eq!(url_to_location(&location_to_url(p)), p);
        }
    }

    #[test]
    fn position_extrapolates_while_playing() {
        let s = snap();
        assert_eq!(position_at(&s, Duration::from_millis(500)), 10_500);
        assert_eq!(position_at(&s, Duration::from_secs(100)), 60_000);
        let p = PlayerSnapshot {
            state: PlayState::Paused,
            ..snap()
        };
        assert_eq!(position_at(&p, Duration::from_secs(5)), 10_000);
    }

    #[test]
    fn seek_detection() {
        let old = snap();
        let d = Duration::from_millis(500);
        let near = PlayerSnapshot {
            position_ms: 10_600,
            ..snap()
        };
        assert!(!is_seek(&old, d, &near));
        let jump = PlayerSnapshot {
            position_ms: 30_000,
            ..snap()
        };
        assert!(is_seek(&old, d, &jump));
        let back = PlayerSnapshot {
            position_ms: 0,
            ..snap()
        };
        assert!(is_seek(&old, d, &back));
        let other = PlayerSnapshot {
            position_ms: 0,
            track_id: Some(2),
            ..snap()
        };
        assert!(!is_seek(&old, d, &other));
    }

    #[test]
    fn changed_property_names() {
        let a = snap();
        assert!(changed_properties(&a, &a.clone()).is_empty());
        let b = PlayerSnapshot {
            position_ms: 1,
            ..snap()
        };
        assert!(changed_properties(&a, &b).is_empty());
        let b = PlayerSnapshot {
            state: PlayState::Paused,
            volume: 1,
            ..snap()
        };
        assert_eq!(changed_properties(&a, &b), vec!["PlaybackStatus", "Volume"]);
        let b = PlayerSnapshot {
            repeat: true,
            shuffle: true,
            ..snap()
        };
        assert_eq!(changed_properties(&a, &b), vec!["LoopStatus", "Shuffle"]);
        let b = PlayerSnapshot {
            manual_advance: true,
            ..snap()
        };
        assert!(changed_properties(&a, &b).is_empty());
        let b = PlayerSnapshot {
            artist: "A".into(),
            ..snap()
        };
        assert_eq!(changed_properties(&a, &b), vec!["Metadata"]);
        let b = PlayerSnapshot {
            playlist_len: 0,
            seekable: false,
            ..snap()
        };
        assert_eq!(
            changed_properties(&a, &b),
            vec![
                "CanGoNext",
                "CanGoPrevious",
                "CanPlay",
                "CanPause",
                "CanSeek"
            ]
        );
    }
}
