//! Data types shared by every provider.

use std::collections::BTreeMap;
use std::ops::{BitOr, BitOrAssign};

use crate::query::{Sort, SortKey};

/// Identifies a track: the provider's id plus the provider's own track id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackId {
    pub provider: String,
    pub id: String,
}

impl TrackId {
    pub fn new(provider: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            id: id.into(),
        }
    }
}

/// ReplayGain values parsed from the `replaygain_*` tags (gains in dB, peaks linear).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ReplayGain {
    pub track_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain: Option<f32>,
    pub album_peak: Option<f32>,
}

impl ReplayGain {
    /// Parses the four `replaygain_*` keys of a tag map (`-6.5 dB`, `+1.2`, `0.988`).
    pub fn from_tags(tags: &BTreeMap<String, String>) -> Self {
        let get = |k: &str| tags.get(k).and_then(|v| parse_gain(v));
        Self {
            track_gain: get("replaygain_track_gain"),
            track_peak: get("replaygain_track_peak"),
            album_gain: get("replaygain_album_gain"),
            album_peak: get("replaygain_album_peak"),
        }
    }
}

/// `[+-]digits[.digits]` with an optional `dB` suffix (any case).
fn parse_gain(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = if s.len() >= 2 && s[s.len() - 2..].eq_ignore_ascii_case("db") {
        s[..s.len() - 2].trim_end()
    } else {
        s
    };
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    s.parse().ok()
}

/// The leading decimal number of a text value (`3/12` → 3, `1999-05-01` → 1999).
pub(crate) fn leading_number(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s[..end].parse().ok()
}

/// One track as listed by a provider.
///
/// `tags` holds the spec 10 §7.1 keys (`title`, `artist`, `album`, `albumartist`, `year`,
/// `genre`, `comment`, `track`, `disc`, `composer`, `publisher`, …) except `rating`, which is
/// the typed [`Track::rating`] field. Times are unix seconds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub id: TrackId,
    /// File path or provider URL, for display; play through
    /// [`crate::LibraryProvider::resolve_playback`].
    pub location: String,
    pub tags: BTreeMap<String, String>,
    /// 0 (unrated) to 5 stars.
    pub rating: u8,
    pub length_ms: u64,
    /// Kilobits per second.
    pub bitrate: Option<u32>,
    pub samplerate: Option<u32>,
    pub channels: Option<u8>,
    /// Format family, for example `MPEG-1 Layer 3` or `FLAC`.
    pub codec: Option<String>,
    pub file_size: Option<u64>,
    /// File modification time.
    pub modified: Option<i64>,
    pub date_added: Option<i64>,
    pub play_count: u64,
    pub last_played: Option<i64>,
    pub replaygain: ReplayGain,
    /// Provider-specific artwork reference (local: a folder image path; remote: an image id
    /// or URL). Fetch the image with [`crate::LibraryProvider::artwork`].
    pub artwork: Option<String>,
}

impl Track {
    /// A tag value, `None` when missing or empty.
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    pub fn title(&self) -> Option<&str> {
        self.tag("title")
    }

    pub fn artist(&self) -> Option<&str> {
        self.tag("artist")
    }

    pub fn album(&self) -> Option<&str> {
        self.tag("album")
    }

    pub fn albumartist(&self) -> Option<&str> {
        self.tag("albumartist")
    }

    pub fn genre(&self) -> Option<&str> {
        self.tag("genre")
    }

    /// Album artist, else artist: the artist the browse tree groups by.
    pub fn group_artist(&self) -> Option<&str> {
        self.albumartist().or(self.artist())
    }

    /// Year as a number (leading digits of the `year` tag).
    pub fn year(&self) -> Option<i32> {
        self.tag("year")
            .and_then(leading_number)
            .and_then(|n| i32::try_from(n).ok())
    }

    /// Track number (leading digits of the `track` tag, `3/12` → 3).
    pub fn track_number(&self) -> Option<u32> {
        self.tag("track")
            .and_then(leading_number)
            .and_then(|n| u32::try_from(n).ok())
    }

    /// Disc number (leading digits of the `disc` tag).
    pub fn disc_number(&self) -> Option<u32> {
        self.tag("disc")
            .and_then(leading_number)
            .and_then(|n| u32::try_from(n).ok())
    }

    /// Last component of the location.
    pub fn file_name(&self) -> &str {
        let loc = self.location.trim_end_matches(['/', '\\']);
        loc.rsplit(['/', '\\']).next().unwrap_or(loc)
    }

    /// The title, or the file name when the track has none.
    pub fn display_title(&self) -> String {
        self.title()
            .map(str::to_string)
            .unwrap_or_else(|| self.file_name().to_string())
    }

    /// A field by name: tag keys, `rating`, technical keys (`length`, `bitrate`, `samplerate`,
    /// `channels`, `family`), `filename`, `filesize`, `playcount`, `lastplayed`, `dateadded`.
    pub fn get(&self, name: &str) -> Option<String> {
        match name {
            "rating" => (self.rating > 0).then(|| self.rating.to_string()),
            "length" => (self.length_ms > 0).then(|| self.length_ms.to_string()),
            "bitrate" => self.bitrate.map(|v| v.to_string()),
            "samplerate" => self.samplerate.map(|v| v.to_string()),
            "channels" => self.channels.map(|v| v.to_string()),
            "family" | "codec" => self.codec.clone(),
            "filename" => Some(self.location.clone()),
            "filesize" => self.file_size.map(|v| v.to_string()),
            "playcount" => Some(self.play_count.to_string()),
            "lastplayed" => self.last_played.map(|v| v.to_string()),
            "dateadded" => self.date_added.map(|v| v.to_string()),
            _ => self.tag(name).map(str::to_string),
        }
    }
}

/// Fields for the title formatter (spec 10 §5.3).
impl nulamp_tf::FieldSource for Track {
    fn field(&self, name: &str) -> Option<String> {
        self.get(name)
    }
}

/// What a provider can do beyond reading. Combine with `|`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Capabilities(u32);

impl Capabilities {
    pub const NONE: Self = Self(0);
    /// The provider indexes sources itself and supports [`crate::LibraryProvider::rescan`].
    pub const SCAN: Self = Self(1);
    /// [`crate::LibraryProvider::set_rating`] works.
    pub const WRITE_RATING: Self = Self(1 << 1);
    /// [`crate::LibraryProvider::record_play`] works.
    pub const RECORD_PLAY: Self = Self(1 << 2);
    /// [`crate::LibraryProvider::edit_tags`] works.
    pub const EDIT_TAGS: Self = Self(1 << 3);
    /// The playlist methods work.
    pub const PLAYLISTS: Self = Self(1 << 4);
    /// [`crate::LibraryProvider::artwork`] can return images.
    pub const ARTWORK: Self = Self(1 << 5);
    /// Tracks are streamed from a server; playback needs the returned URL and headers.
    pub const REMOTE: Self = Self(1 << 6);
    /// Ratings are stored only in the library, not in the files' tags; an app that wants
    /// them in the tags writes them itself.
    pub const RATING_DB_ONLY: Self = Self(1 << 7);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// True when every flag of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Capabilities {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Capabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// What a browse node stands for (for icons and behaviour; the id is provider-specific).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// The tree root ([`BrowseNode::root`]).
    Root,
    /// A grouping such as "Artists" or "Genres".
    Category,
    Artist,
    Album,
    Genre,
    Year,
    /// A computed list such as "Most played".
    SmartList,
    Playlist,
    /// A provider-specific node (for example a server folder).
    Other,
}

/// A node of a provider's browse tree. Pass it back to the same provider's
/// [`crate::LibraryProvider::browse`] for its children and
/// [`crate::LibraryProvider::tracks`] for its tracks.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowseNode {
    /// Provider-specific id; the empty string is the root.
    pub id: String,
    pub label: String,
    /// Secondary text, for example an album's artist.
    pub detail: Option<String>,
    pub kind: NodeKind,
    /// Number of tracks below the node, when cheap to know.
    pub track_count: Option<u64>,
    /// True when [`crate::LibraryProvider::browse`] can return children.
    pub expandable: bool,
    /// The order a view of this node should start with.
    pub default_sort: Sort,
}

impl BrowseNode {
    /// The root node of every provider.
    pub fn root() -> Self {
        Self::new("", "Library", NodeKind::Root, true)
    }

    /// A node with no detail or count, sorted by artist.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        kind: NodeKind,
        expandable: bool,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: None,
            kind,
            track_count: None,
            expandable,
            default_sort: Sort::asc(SortKey::Artist),
        }
    }

    pub fn with_detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail;
        self
    }

    pub fn with_count(mut self, count: u64) -> Self {
        self.track_count = Some(count);
        self
    }

    pub fn with_sort(mut self, sort: Sort) -> Self {
        self.default_sort = sort;
        self
    }
}

/// A window into a result list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub offset: u64,
    pub limit: u64,
}

impl Page {
    /// Every row.
    pub const fn all() -> Self {
        Self {
            offset: 0,
            limit: u64::MAX,
        }
    }

    pub const fn new(offset: u64, limit: u64) -> Self {
        Self { offset, limit }
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::all()
    }
}

/// One page of tracks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackPage {
    pub tracks: Vec<Track>,
    /// Number of matching tracks over all pages.
    pub total: u64,
    pub offset: u64,
}

/// Where to play a track from: a file path, or a URL plus the HTTP headers the request
/// needs (for example `Authorization`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaybackLocation {
    pub location: String,
    pub headers: Vec<(String, String)>,
}

/// A playlist stored by a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistInfo {
    pub id: String,
    pub name: String,
    pub track_count: u64,
}

/// Result of a scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub added: u64,
    pub updated: u64,
    pub removed: u64,
    pub unchanged: u64,
    /// Files indexed without tags because they could not be read.
    pub unreadable: u64,
    pub cancelled: bool,
    /// Set when the scan stopped on an error.
    pub error: Option<String>,
}

/// Change notification from [`crate::LibraryProvider::subscribe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryEvent {
    /// Tracks were added, changed or removed; refresh views.
    TracksChanged,
    /// Playlists were created, renamed, deleted or edited.
    PlaylistsChanged,
    /// `done` of `total` files read.
    ScanProgress {
        done: u64,
        total: u64,
    },
    ScanFinished(ScanSummary),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains() {
        assert_eq!(parse_gain("-6.5 dB"), Some(-6.5));
        assert_eq!(parse_gain("+1.25"), Some(1.25));
        assert_eq!(parse_gain("0.988"), Some(0.988));
        assert_eq!(parse_gain("1e3"), None);
        assert_eq!(parse_gain(""), None);
    }

    #[test]
    fn numbers() {
        assert_eq!(leading_number("3/12"), Some(3));
        assert_eq!(leading_number(" 1999-05-01"), Some(1999));
        assert_eq!(leading_number("x"), None);
    }

    #[test]
    fn capability_flags() {
        let c = Capabilities::SCAN | Capabilities::PLAYLISTS;
        assert!(c.contains(Capabilities::SCAN));
        assert!(!c.contains(Capabilities::REMOTE));
        assert!(c.contains(Capabilities::NONE));
    }
}
