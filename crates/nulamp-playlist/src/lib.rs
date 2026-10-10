//! Playlist file formats: reading, writing, detection and location resolution (spec 10 §3).
//!
//! Pure file-format logic. Readers turn bytes into [`PlaylistEntry`] values; writers turn
//! entries back into bytes. Locations are resolved through a [`Resolver`], which takes the
//! playlist's own location as the base for relative paths and a [`FileSystem`] for the
//! case-insensitive fallback. [`Loader`] expands nested playlists.
//!
//! Choices where the spec is silent are recorded as `PL1`… in `docs/decisions.md`.

pub mod asx;
pub mod b4s;
pub mod bookmarks;
pub mod entry;
pub mod format;
pub mod load;
pub mod location;
pub mod m3u;
pub mod pls;
pub mod text;
pub mod wpl;
pub mod xspf;

mod ini;
#[cfg(test)]
mod testutil;
mod xml;

pub use bookmarks::{Bookmark, read_bookmarks, read_legacy_bookmarks, write_bookmarks};
pub use entry::PlaylistEntry;
pub use format::{Format, is_hls};
pub use load::{Loader, parse, read_file};
pub use location::{FileSystem, NullFs, PathStyle, Resolver, StdFs, path_to_file_uri, relativize};

/// Errors from reading or writing playlists.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The data is not a valid playlist of the given format (for example PLS with
    /// `NumberOfEntries=0`, or an XML document without the expected root).
    #[error("not a playlist")]
    NotAPlaylist,
    /// The data is an HLS media or master playlist: play the location as one stream.
    #[error("HLS stream, not a playlist")]
    Stream,
    /// The format could not be determined from the location or the content.
    #[error("unknown playlist format")]
    UnknownFormat,
    /// The playlist could not be read.
    #[error("cannot read playlist {0}")]
    Unreadable(String),
    /// The format cannot be written.
    #[error("{0:?} playlists cannot be written")]
    Unsupported(Format),
}

/// Options shared by all writers.
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// Folder of the playlist being written. Files under it are written as relative paths
    /// (never with `..`); everything else is absolute. `None` writes absolute paths.
    pub relative_to: Option<String>,
    /// Path conventions of the locations.
    pub style: PathStyle,
    /// `.m3u` only: write Windows-1252 instead of UTF-8 (characters outside the code page
    /// become `?`).
    pub legacy_8bit: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            relative_to: None,
            style: PathStyle::native(),
            legacy_8bit: false,
        }
    }
}

impl WriteOptions {
    /// Location as it should appear in the written file.
    fn location(&self, location: &str) -> String {
        match &self.relative_to {
            Some(dir) => relativize(location, dir, self.style),
            None => location.to_string(),
        }
    }
}

/// Writes `entries` in `format`.
pub fn write(
    entries: &[PlaylistEntry],
    format: Format,
    opts: &WriteOptions,
) -> Result<Vec<u8>, Error> {
    match format {
        Format::M3u => Ok(m3u::write(entries, false, opts)),
        Format::M3u8 => Ok(m3u::write(entries, true, opts)),
        Format::Pls => Ok(pls::write(entries, opts)),
        Format::Xspf => Ok(xspf::write(entries, opts)),
        Format::B4s => Ok(b4s::write(entries, opts)),
        Format::Asx | Format::AsxReference | Format::Wpl => Err(Error::Unsupported(format)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_writable_format_round_trips() {
        let entries = vec![
            PlaylistEntry::with_title("/m/a b/ü.mp3", "Ü & <T>", Some(75)),
            PlaylistEntry::new("/m/x.ogg"),
            PlaylistEntry::with_title("http://h/s?a=1&b=2", "Radio", None),
        ];
        for format in Format::WRITABLE {
            let opts = WriteOptions {
                relative_to: Some("/m".into()),
                style: PathStyle::Unix,
                legacy_8bit: false,
            };
            let out = write(&entries, format, &opts).unwrap();
            let base = format!("/m/list.{}", format.extension());
            let back = parse(
                &out,
                format,
                &Resolver::new(Some(&base), PathStyle::Unix, &NullFs),
            );
            assert_eq!(back.unwrap(), entries, "{format:?}");
            assert_eq!(Format::detect(&base, &out), Some(format));
        }
    }

    #[test]
    fn read_only_formats() {
        for f in [Format::Asx, Format::AsxReference, Format::Wpl] {
            assert_eq!(
                write(&[], f, &WriteOptions::default()),
                Err(Error::Unsupported(f))
            );
        }
    }
}
