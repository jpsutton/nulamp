//! Media library: a provider interface and a local SQLite provider (decision D4,
//! `docs/media-library.md`).
//!
//! The UI talks to [`LibraryProvider`] trait objects held in a [`Registry`]. Every provider
//! exposes the same browse tree ([`BrowseNode`]), track listing with [`Query`], [`Sort`] and
//! [`Page`], playback resolution ([`PlaybackLocation`]) and optional writes (ratings, play
//! counts, tags, playlists) announced through [`Capabilities`]. [`LocalLibrary`] indexes
//! watch folders into SQLite; [`JellyfinProvider`] browses the music libraries of a Jellyfin
//! server; [`MockProvider`] is an in-memory remote-style provider used by tests.

mod error;
pub(crate) mod facets;
mod model;
mod provider;
mod query;

pub mod jellyfin;
pub mod local;
pub mod mock;

pub use error::{Error, Result};
pub use jellyfin::JellyfinProvider;
pub use local::{ExtensionFilter, LocalLibrary, ScanHandle, ScanMode, WatchFolder};
pub use mock::MockProvider;
pub use model::{
    AlbumFacet, ArtistFacet, BrowseNode, Capabilities, LIBRARY_SCHEME, LibraryEvent, NodeKind,
    Page, PlaybackLocation, PlaylistInfo, ReplayGain, ScanSummary, Track, TrackId, TrackPage,
    TrackSummary,
};
pub use provider::{LibraryProvider, Registry, Subscribers};
pub use query::{CmpOp, NumField, Query, Sort, SortKey, Term, TextField, fold, tokenize};

/// Play-count rule: a play counts when at least half of the track (by `length_ms`) or at
/// least 240 seconds were played. With an unknown length (0, for example a stream) only the
/// 240-second rule applies.
pub fn should_count_play(played_ms: u64, length_ms: u64) -> bool {
    played_ms >= 240_000 || (length_ms > 0 && played_ms.saturating_mul(2) >= length_ms)
}

/// Current time in unix seconds.
pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Escapes `%` and `/` so a value can be one segment of a `/`-separated node id.
pub(crate) fn enc_segment(s: &str) -> String {
    s.replace('%', "%25").replace('/', "%2F")
}

/// Splits a node id into its decoded `/`-separated segments.
pub(crate) fn split_id(id: &str) -> Vec<String> {
    id.split('/')
        .map(|seg| {
            let mut out = String::with_capacity(seg.len());
            let mut rest = seg;
            while let Some(i) = rest.find('%') {
                out.push_str(&rest[..i]);
                let tail = &rest[i..];
                if let Some(r) = tail.strip_prefix("%25") {
                    out.push('%');
                    rest = r;
                } else if let Some(r) = tail.strip_prefix("%2F") {
                    out.push('/');
                    rest = r;
                } else {
                    out.push('%');
                    rest = &tail[1..];
                }
            }
            out.push_str(rest);
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_rule() {
        assert!(should_count_play(120_000, 240_000));
        assert!(!should_count_play(119_999, 240_000));
        assert!(should_count_play(240_000, 3_600_000));
        assert!(!should_count_play(239_999, 3_600_000));
        assert!(!should_count_play(10_000, 0));
        assert!(should_count_play(240_000, 0));
    }

    #[test]
    fn node_id_segments() {
        for s in ["a/b", "100%", "%2F", "%252F", "plain", ""] {
            let id = format!("x/{}", enc_segment(s));
            assert_eq!(split_id(&id), vec!["x".to_string(), s.to_string()], "{s}");
        }
    }
}
