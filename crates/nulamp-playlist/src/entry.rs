//! Playlist entry data model (10 §1).

/// One entry read from, or to be written to, a playlist file.
///
/// `title` doubles as the "metadata read" flag: writers emit title and length
/// (`#EXTINF`, `Title<n>`/`Length<n>`, …) only for entries whose `title` is `Some`.
/// Titles are kept raw (no number prefix, no formatting).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistEntry {
    /// Resolved location: absolute path, URL, or `cda://` location.
    pub location: String,
    /// Display title, `None` when the file gives none.
    pub title: Option<String>,
    /// Length in seconds, `None` when unknown.
    pub length_s: Option<u64>,
    /// Extra plays after the first (PLS `Repeat<n>`, ASX `REPEAT COUNT`).
    pub repeat: u32,
    /// Start of the play range in milliseconds (PLS `Start<n>`).
    pub start_ms: Option<u64>,
    /// End of the play range in milliseconds (PLS `End<n>`).
    pub end_ms: Option<u64>,
    /// `#EXTINF` attributes in file order, for example `tvg-id`, `group-title` or the
    /// stream extension hint `ext`. Written back by the M3U writer.
    pub extras: Vec<(String, String)>,
    /// Artist (XSPF `creator`, ASX `AUTHOR`, M3U `#EXTART`). Already part of `title`
    /// when the format combines them.
    pub artist: Option<String>,
    /// Album (XSPF `album`, M3U `#EXTALB`).
    pub album: Option<String>,
    /// Track number (XSPF `trackNum`).
    pub track_number: Option<u32>,
    /// The location is itself a playlist to be loaded in place (ASX `ENTRYREF`).
    pub nested: bool,
}

impl PlaylistEntry {
    /// Entry with only a location.
    pub fn new(location: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            ..Self::default()
        }
    }

    /// Entry with a location, a title and an optional length.
    pub fn with_title(
        location: impl Into<String>,
        title: impl Into<String>,
        length_s: Option<u64>,
    ) -> Self {
        Self {
            location: location.into(),
            title: Some(title.into()),
            length_s,
            ..Self::default()
        }
    }

    /// Value of the `#EXTINF` attribute `key` (case-insensitive).
    pub fn extra(&self, key: &str) -> Option<&str> {
        self.extras
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Length in seconds from milliseconds, rounded to the nearest second.
pub(crate) fn ms_to_s(ms: u64) -> u64 {
    ms.saturating_add(500) / 1000
}

/// `Some(title)` when `title` is non-empty.
pub(crate) fn non_empty(title: &str) -> Option<String> {
    (!title.is_empty()).then(|| title.to_string())
}
