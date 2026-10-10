//! Decoder modules and decoder selection (07 §7, 08 §3).
//!
//! Decoders use a pull model: the host asks for audio, seeks and closes. Decoders contain no
//! threading code.

pub mod symphonia_decoder;

use std::fmt;

/// Technical information about an opened stream.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamInfo {
    pub sample_rate: u32,
    pub channels: u16,
    /// Bits per sample of the source (0 when not meaningful).
    pub bits: u16,
    pub length_ms: Option<u64>,
    pub seekable: bool,
    /// Format description, for example "MPEG-1 Layer 3".
    pub codec: String,
    pub lossless: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("no decoder for {0}")]
    Unsupported(String),
    #[error("cannot open: {0}")]
    Open(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("seek failed: {0}")]
    Seek(String),
}

/// An opened stream that produces interleaved 32-bit float PCM.
pub trait Decoder: Send {
    fn info(&self) -> &StreamInfo;

    /// Decodes the next block and appends its interleaved samples to `out`.
    /// Returns the number of frames appended; 0 means the end of the stream.
    fn read(&mut self, out: &mut Vec<f32>) -> Result<usize, DecodeError>;

    /// Seeks to a position in milliseconds. Returns the position actually reached.
    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError>;

    /// Current bit rate in kbps (changes for VBR).
    fn kbps(&self) -> Option<u32>;

    /// "In sync" state for the status light (02 §4). Network decoders clear it while buffering.
    fn in_sync(&self) -> bool {
        true
    }

    /// A changed stream title (ICY metadata, chained Ogg), if any since the last call.
    fn take_title(&mut self) -> Option<String> {
        None
    }
}

/// Creates decoders for the locations it claims.
pub trait DecoderFactory: Send + Sync {
    fn name(&self) -> &str;

    /// (extension list separated by `;`, description) pairs for file dialogs.
    fn extensions(&self) -> Vec<(String, String)>;

    /// Optional content check, consulted before extension matching.
    fn claims(&self, _location: &str) -> bool {
        false
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError>;
}

/// The extension of a location, lower-case, ignoring a URL query string.
pub fn extension_of(location: &str) -> Option<String> {
    let no_query = if location.contains("://") {
        location.split(['?', '#']).next().unwrap_or(location)
    } else {
        location
    };
    let name = no_query.rsplit(['/', '\\']).next()?;
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty()).then(|| ext.to_lowercase())
}

/// Ordered list of decoder factories with the selection rules of 07 §7.
pub struct Registry {
    factories: Vec<Box<dyn DecoderFactory>>,
    /// Extension used for locations no decoder claims (default `mp3`).
    pub assumed_extension: String,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.factories.iter().map(|d| d.name()))
            .finish()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl Registry {
    pub fn empty() -> Self {
        Self {
            factories: Vec::new(),
            assumed_extension: "mp3".into(),
        }
    }

    /// The built-in decoders.
    pub fn with_defaults() -> Self {
        let mut r = Self::empty();
        r.add(Box::new(symphonia_decoder::SymphoniaFactory));
        r
    }

    pub fn add(&mut self, f: Box<dyn DecoderFactory>) {
        self.factories.push(f);
    }

    fn handles_ext(f: &dyn DecoderFactory, ext: &str) -> bool {
        f.extensions()
            .iter()
            .any(|(list, _)| list.split(';').any(|e| e.eq_ignore_ascii_case(ext)))
    }

    /// Picks the factory for a location: content check, then extension, then the assumed
    /// extension of unknown files.
    pub fn select(&self, location: &str) -> Option<&dyn DecoderFactory> {
        if let Some(f) = self.factories.iter().find(|f| f.claims(location)) {
            return Some(f.as_ref());
        }
        let ext = extension_of(location);
        if let Some(ext) = &ext
            && let Some(f) = self
                .factories
                .iter()
                .find(|f| Self::handles_ext(f.as_ref(), ext))
        {
            return Some(f.as_ref());
        }
        self.factories
            .iter()
            .find(|f| Self::handles_ext(f.as_ref(), &self.assumed_extension))
            .map(|f| f.as_ref())
    }

    pub fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        let f = self
            .select(location)
            .ok_or_else(|| DecodeError::Unsupported(location.into()))?;
        f.open(location)
    }

    /// Whether some decoder claims this extension (folder scanning, 10 §2.2).
    pub fn is_supported_extension(&self, ext: &str) -> bool {
        self.factories
            .iter()
            .any(|f| Self::handles_ext(f.as_ref(), ext))
    }

    /// All (extensions, description) pairs.
    pub fn all_extensions(&self) -> Vec<(String, String)> {
        self.factories.iter().flat_map(|f| f.extensions()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(extension_of("/a/b.MP3").as_deref(), Some("mp3"));
        assert_eq!(extension_of("http://x/y.pls?id=3").as_deref(), Some("pls"));
        assert_eq!(extension_of("http://x/stream").as_deref(), None);
        assert_eq!(extension_of("C:\\a.b\\c").as_deref(), None);
    }

    #[test]
    fn unknown_extension_uses_assumed() {
        let r = Registry::with_defaults();
        assert!(r.select("http://radio.example/live").is_some());
        assert!(r.is_supported_extension("FLAC"));
        assert!(!r.is_supported_extension("txt"));
    }
}
