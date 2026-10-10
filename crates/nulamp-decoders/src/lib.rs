//! Decoder modules and decoder selection (07 §7, 08 §3).
//!
//! Decoders use a pull model: the host asks for audio, seeks and closes. File decoders contain
//! no threading code; the network stream input ([`stream`]) runs its own network and decoding
//! threads so that [`Decoder::read`] never waits for the network.

#[cfg(nulamp_ffmpeg)]
pub mod ffmpeg_decoder;
pub mod midi_decoder;
#[cfg(nulamp_openmpt)]
pub mod module_decoder;
pub mod opus_decoder;
pub mod stream;
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
    #[error(transparent)]
    Stream(#[from] stream::StreamError),
}

/// Outcome of [`Decoder::read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    /// This many frames (at least one) were appended.
    Frames(usize),
    /// No audio is ready yet (a network stream is connecting or buffering). The stream has not
    /// ended: call again later.
    Pending,
    /// All audio was decoded.
    End,
}

impl ReadStatus {
    /// A frame count from a decoder that is never pending: zero frames means the end.
    pub fn from_frames(n: usize) -> Self {
        if n == 0 { Self::End } else { Self::Frames(n) }
    }

    /// Frames appended (zero for `Pending` and `End`).
    pub fn frames(self) -> usize {
        match self {
            Self::Frames(n) => n,
            _ => 0,
        }
    }
}

/// An opened stream that produces interleaved 32-bit float PCM.
pub trait Decoder: Send {
    fn info(&self) -> &StreamInfo;

    /// Decodes the next block and appends its interleaved samples to `out`. Returns quickly;
    /// [`ReadStatus::Pending`] means nothing is ready yet, not the end.
    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError>;

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

    /// Station and song fields of a network stream (`streamtitle`, `streamname`, …).
    fn stream_fields(&self) -> Option<stream::StreamFields> {
        None
    }
}

/// Tags and stream information read by a decoder library, for files the tag library does not
/// support (see [`ffmpeg_tags`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileTags {
    /// (nulamp field key (10 §7.1), value) pairs, first value per key.
    pub fields: Vec<(String, String)>,
    pub info: StreamInfo,
    /// Average bit rate in kbps.
    pub kbps: Option<u32>,
}

/// Reads the tags of a local file with FFmpeg (ASF/WMA attributes, APEv2 in TTA, …). `None`
/// when the file cannot be opened or nulamp was built without FFmpeg.
pub fn ffmpeg_tags(location: &str) -> Option<FileTags> {
    #[cfg(nulamp_ffmpeg)]
    return ffmpeg_decoder::read_tags(location);
    #[cfg(not(nulamp_ffmpeg))]
    {
        let _ = location;
        None
    }
}

/// The licence of the FFmpeg libraries in use (for example "LGPL version 2.1 or later"), or
/// `None` when nulamp was built without FFmpeg.
pub fn ffmpeg_license() -> Option<String> {
    #[cfg(nulamp_ffmpeg)]
    return Some(ffmpeg_decoder::license());
    #[cfg(not(nulamp_ffmpeg))]
    None
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

    /// Whether this factory may also be tried for local files the selected decoder failed to
    /// open (its library probes the content). See [`Registry::open`].
    fn fallback(&self) -> bool {
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

    /// The built-in decoders, with a default network stream input.
    pub fn with_defaults() -> Self {
        Self::with_stream(stream::StreamFactory::default())
    }

    /// The built-in decoders, with `stream` as the network stream input. Keep a clone of the
    /// factory to change its settings, set the credentials prompt or read stream fields.
    pub fn with_stream(stream: stream::StreamFactory) -> Self {
        let mut r = Self::empty();
        r.add(Box::new(stream));
        r.add(Box::new(symphonia_decoder::SymphoniaFactory));
        r.add(Box::new(opus_decoder::OpusFactory));
        r.add(Box::new(midi_decoder::MidiFactory::default()));
        #[cfg(nulamp_openmpt)]
        r.add(Box::new(module_decoder::ModuleFactory::default()));
        // Last, so the decoders above keep their own extensions.
        #[cfg(nulamp_ffmpeg)]
        r.add(Box::new(ffmpeg_decoder::FfmpegFactory));
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

    /// Opens a location with the selected factory. When that fails (other than a missing file
    /// or a network error), the fallback factories ([`DecoderFactory::fallback`]) are tried in
    /// order; the first error is returned when none succeeds.
    pub fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        let selected = self.select(location);
        let err = match selected {
            Some(f) => match f.open(location) {
                Ok(d) => return Ok(d),
                Err(e @ (DecodeError::NotFound(_) | DecodeError::Stream(_))) => return Err(e),
                Err(e) => e,
            },
            None => DecodeError::Unsupported(location.into()),
        };
        let selected = selected.map(|f| f.name());
        for f in self
            .factories
            .iter()
            .filter(|f| f.fallback() && Some(f.name()) != selected)
        {
            if let Ok(d) = f.open(location) {
                log::debug!("{location}: opened by {} after: {err}", f.name());
                return Ok(d);
            }
        }
        Err(err)
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
        assert_eq!(
            r.select("http://radio.example/live").map(|f| f.name()),
            Some("Network stream")
        );
        assert_eq!(
            r.select("ICY://radio.example/x.mp3").map(|f| f.name()),
            Some("Network stream")
        );
        assert_eq!(r.select("/a/b.mp3").map(|f| f.name()), Some("Symphonia"));
        assert!(r.is_supported_extension("FLAC"));
        assert!(!r.is_supported_extension("txt"));
    }
}
