//! Audio CD playback (09 §5) by digital audio extraction: the PCM goes through the normal
//! chain, so the equalizer, effects and visualization work.
//!
//! Locations (docs/decisions.md CD2): `cda://<drive>` names a whole disc and expands to one
//! playlist entry per audio track ([`CdFactory::disc_entries`]); `cda://<drive>,<track>` names
//! one track. `<drive>` is a drive index (`0` = first drive), a Windows drive letter (`D`), a
//! device name under `/dev` (`sr0`) or an absolute device path. `X:\TrackNN.cda` stubs are
//! accepted too.
//!
//! The drive is reached through the [`CdBackend`] / [`CdDrive`] traits, so everything above
//! the device (TOC, titles, seeking, track boundaries) is tested with a fake drive. The only
//! backend is libcdio, built with the opt-in `cdio` feature (docs/decisions.md CD1).

#[cfg(nulamp_cdio)]
pub mod cdio;

use std::fmt;
use std::sync::Arc;

use crate::{DecodeError, Decoder, DecoderFactory, ReadStatus, StreamInfo};

/// Location scheme of audio CD tracks and discs.
pub const SCHEME: &str = "cda://";
/// Red Book audio: 44.1 kHz, 16-bit, stereo.
pub const SAMPLE_RATE: u32 = 44_100;
/// Bytes in one raw audio sector.
pub const SECTOR_BYTES: usize = 2352;
/// Stereo frames in one sector (1/75 s).
pub const SECTOR_FRAMES: u64 = 588;
/// Sectors between the end of the audio session of an Enhanced CD (CD-Extra) and the start of
/// its data track: lead-out (6750) + lead-in (4500) + pregap (150).
pub const CD_EXTRA_GAP: u32 = 11_400;
/// Pregap in front of a data track in the same session (2 s).
pub const DATA_PREGAP: u32 = 150;

/// A parsed `cda://` location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdLocation {
    /// Drive token as written in the location (see the module documentation).
    pub drive: String,
    /// Track number; `None` for the whole disc.
    pub track: Option<u8>,
}

impl CdLocation {
    pub fn disc(drive: impl Into<String>) -> Self {
        Self {
            drive: drive.into(),
            track: None,
        }
    }

    pub fn track(drive: impl Into<String>, track: u8) -> Self {
        Self {
            drive: drive.into(),
            track: Some(track),
        }
    }

    /// Parses `cda://<drive>[,<track>]` (scheme case-insensitive) or an `X:\TrackNN.cda` stub.
    pub fn parse(location: &str) -> Option<Self> {
        let location = location.trim();
        let Some(rest) = strip_prefix_ci(location, SCHEME) else {
            return parse_stub(location);
        };
        let (drive, track) = match rest.rsplit_once(',') {
            Some((d, t)) => (d, Some(parse_track(t)?)),
            None => (rest, None),
        };
        let drive = normalize_drive(drive)?;
        Some(Self { drive, track })
    }

    /// The whole-disc location of the same drive.
    pub fn whole_disc(&self) -> Self {
        Self::disc(self.drive.clone())
    }
}

impl fmt::Display for CdLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.track {
            Some(t) => write!(f, "{SCHEME}{},{t}", self.drive),
            None => write!(f, "{SCHEME}{}", self.drive),
        }
    }
}

/// Whether a location names a whole disc (`cda://<drive>`), which expands to its tracks.
pub fn is_whole_disc(location: &str) -> bool {
    CdLocation::parse(location).is_some_and(|l| l.track.is_none())
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn parse_track(t: &str) -> Option<u8> {
    let t = t.trim();
    if t.is_empty() || t.len() > 2 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok().filter(|n| (1..=99).contains(n))
}

/// Trims the drive token; `D`, `d:`, `D:\` and `D:/` become `D`.
fn normalize_drive(d: &str) -> Option<String> {
    let d = d.trim();
    if d.is_empty() {
        return None;
    }
    let b = d.as_bytes();
    let letter =
        b[0].is_ascii_alphabetic() && matches!(&b[1..], [] | [b':'] | [b':', b'\\'] | [b':', b'/']);
    Some(if letter {
        (b[0] as char).to_ascii_uppercase().to_string()
    } else {
        d.to_string()
    })
}

/// `X:\TrackNN.cda` (the stub files Windows shows for audio CD tracks).
fn parse_stub(location: &str) -> Option<CdLocation> {
    let b = location.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_alphabetic() || b[1] != b':' || !matches!(b[2], b'\\' | b'/') {
        return None;
    }
    let name = &location[3..];
    let stem = strip_prefix_ci(name, "track")?;
    let digits = stem.get(..stem.len().checked_sub(4)?)?;
    strip_prefix_ci(&stem[digits.len()..], ".cda")?;
    let track = parse_track(digits)?;
    Some(CdLocation::track(
        (b[0] as char).to_ascii_uppercase(),
        track,
    ))
}

/// The drive token used in locations for a device: a drive letter on Windows (`D:` → `D`),
/// the device name for devices directly under `/dev` (`/dev/sr0` → `sr0`), else the path.
pub fn drive_token(device: &str) -> String {
    if let Some(name) = device.strip_prefix("/dev/")
        && !name.is_empty()
        && !name.contains('/')
    {
        return name.to_string();
    }
    normalize_drive(device).unwrap_or_default()
}

/// The device for a drive token: an index into `devices`, a drive letter (`D:`), an absolute
/// path, or a device name under `/dev`.
pub fn device_for(token: &str, devices: &[String]) -> Option<String> {
    if !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit()) {
        return token
            .parse::<usize>()
            .ok()
            .and_then(|i| devices.get(i).cloned());
    }
    if token.len() == 1 && token.as_bytes()[0].is_ascii_alphabetic() {
        return Some(format!("{}:", token.to_ascii_uppercase()));
    }
    if token.starts_with('/') || token.starts_with('\\') {
        return Some(token.to_string());
    }
    Some(format!("/dev/{token}"))
}

/// One track in the table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocTrack {
    pub number: u8,
    /// First sector (LSN, index 1).
    pub start: u32,
    pub audio: bool,
}

/// The table of contents of a disc.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Toc {
    /// Tracks in disc order.
    pub tracks: Vec<TocTrack>,
    /// First sector of the lead-out.
    pub leadout: u32,
    /// First sector of the last session, on multi-session discs (Enhanced CD).
    pub last_session_start: Option<u32>,
}

impl Toc {
    pub fn track(&self, number: u8) -> Option<&TocTrack> {
        self.tracks.iter().find(|t| t.number == number)
    }

    pub fn audio_tracks(&self) -> impl Iterator<Item = &TocTrack> {
        self.tracks.iter().filter(|t| t.audio)
    }

    /// Sectors `[start, end)` of an audio track. A track runs up to the start of the next
    /// track (so its successor's pregap belongs to it and tracks join without a gap), or to the
    /// lead-out. Before a data track the data track's pregap is left out, and on an Enhanced CD
    /// the session gap too.
    pub fn audio_range(&self, number: u8) -> Option<(u32, u32)> {
        let i = self.tracks.iter().position(|t| t.number == number)?;
        let track = &self.tracks[i];
        if !track.audio {
            return None;
        }
        let end = match self.tracks.get(i + 1) {
            Some(next) if !next.audio => {
                let gap = if self.last_session_start == Some(next.start) {
                    CD_EXTRA_GAP
                } else {
                    DATA_PREGAP
                };
                match next.start.checked_sub(gap) {
                    Some(e) if e > track.start => e,
                    _ => next.start,
                }
            }
            Some(next) => next.start,
            None => self.leadout,
        };
        (end > track.start).then_some((track.start, end))
    }

    /// Length of an audio track in milliseconds.
    pub fn length_ms(&self, number: u8) -> Option<u64> {
        self.audio_range(number)
            .map(|(s, e)| frames_to_ms((e - s) as u64 * SECTOR_FRAMES))
    }
}

/// Converts milliseconds to a frame position (rounding down).
pub fn ms_to_frames(ms: u64) -> u64 {
    ms * SAMPLE_RATE as u64 / 1000
}

/// Converts a frame position to milliseconds (rounding to nearest).
pub fn frames_to_ms(frames: u64) -> u64 {
    (frames * 1000 + SAMPLE_RATE as u64 / 2) / SAMPLE_RATE as u64
}

/// CD-Text of one track (or of the disc, as track 0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CdTextEntry {
    pub title: Option<String>,
    pub performer: Option<String>,
}

/// CD-Text of a disc (first language block).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CdText {
    pub disc: CdTextEntry,
    pub tracks: Vec<(u8, CdTextEntry)>,
}

impl CdText {
    pub fn track(&self, number: u8) -> Option<&CdTextEntry> {
        self.tracks
            .iter()
            .find(|(n, _)| *n == number)
            .map(|(_, e)| e)
    }
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Playlist title of a track (09 §5): `Performer - Title` from CD-Text (the disc performer
/// when the track has none), the CD-Text title alone, else `Track NN`.
pub fn track_title(number: u8, text: Option<&CdText>) -> String {
    let entry = text.and_then(|t| t.track(number));
    let title = entry.and_then(|e| non_empty(&e.title));
    let artist = entry
        .and_then(|e| non_empty(&e.performer))
        .or_else(|| text.and_then(|t| non_empty(&t.disc.performer)));
    match (artist, title) {
        (Some(a), Some(t)) => format!("{a} - {t}"),
        (None, Some(t)) => t.to_string(),
        _ => format!("Track {number:02}"),
    }
}

/// One playlist entry of a disc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdEntry {
    pub location: String,
    pub title: String,
    pub length_ms: Option<u64>,
}

/// The playlist entries of a disc: one per audio track, in disc order.
pub fn entries_for(drive: &str, toc: &Toc, text: Option<&CdText>) -> Vec<CdEntry> {
    toc.audio_tracks()
        .filter(|t| toc.audio_range(t.number).is_some())
        .map(|t| CdEntry {
            location: CdLocation::track(drive, t.number).to_string(),
            title: track_title(t.number, text),
            length_ms: toc.length_ms(t.number),
        })
        .collect()
}

/// An opened CD drive.
pub trait CdDrive: Send {
    /// Reads the table of contents. Fails when there is no disc.
    fn toc(&mut self) -> Result<Toc, DecodeError>;

    /// CD-Text, when the disc has it.
    fn cd_text(&mut self) -> Option<CdText> {
        None
    }

    /// Reads `buf.len() / SECTOR_BYTES` raw audio sectors starting at `lsn`
    /// (16-bit little-endian stereo).
    fn read_audio(&mut self, lsn: u32, buf: &mut [u8]) -> Result<(), DecodeError>;
}

/// Access to the CD drives of the system.
pub trait CdBackend: Send + Sync {
    fn name(&self) -> &str;

    /// Device names of the drives, in system order.
    fn devices(&self) -> Vec<String>;

    fn open(&self, device: &str) -> Result<Box<dyn CdDrive>, DecodeError>;
}

/// A CD drive for menus and the `/CD<n>` switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdDriveInfo {
    /// Whole-disc location, for example `cda://sr0`.
    pub location: String,
    /// Device name for display, for example `/dev/sr0` or `D:`.
    pub label: String,
}

/// Decoder factory for `cda://` locations, also the entry point for drive enumeration and
/// disc expansion.
#[derive(Clone, Default)]
pub struct CdFactory {
    backend: Option<Arc<dyn CdBackend>>,
}

impl fmt::Debug for CdFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CdFactory")
            .field("backend", &self.backend.as_ref().map(|b| b.name()))
            .finish()
    }
}

impl CdFactory {
    /// The factory with the system backend (libcdio when built with it; else none).
    pub fn system() -> Self {
        #[cfg(nulamp_cdio)]
        {
            Self::with_backend(Arc::new(cdio::CdioBackend))
        }
        #[cfg(not(nulamp_cdio))]
        {
            Self::default()
        }
    }

    pub fn with_backend(backend: Arc<dyn CdBackend>) -> Self {
        Self {
            backend: Some(backend),
        }
    }

    /// Whether audio CD support is available in this build.
    pub fn available(&self) -> bool {
        self.backend.is_some()
    }

    fn backend(&self) -> Result<&dyn CdBackend, DecodeError> {
        self.backend
            .as_deref()
            .ok_or_else(|| DecodeError::Unsupported("audio CD (not built in)".into()))
    }

    /// The drives of the system, in order (index n is drive n of `/CD<n>`).
    pub fn drives(&self) -> Vec<CdDriveInfo> {
        let Some(b) = &self.backend else {
            return Vec::new();
        };
        b.devices()
            .into_iter()
            .map(|d| CdDriveInfo {
                location: CdLocation::disc(drive_token(&d)).to_string(),
                label: d,
            })
            .collect()
    }

    /// Opens the drive of a location; returns it with its canonical drive token.
    fn open_drive(&self, loc: &CdLocation) -> Result<(Box<dyn CdDrive>, String), DecodeError> {
        let b = self.backend()?;
        let devices = if loc.drive.bytes().all(|c| c.is_ascii_digit()) {
            b.devices()
        } else {
            Vec::new()
        };
        let device = device_for(&loc.drive, &devices)
            .ok_or_else(|| DecodeError::NotFound(format!("no CD drive {}", loc.drive)))?;
        Ok((b.open(&device)?, drive_token(&device)))
    }

    /// Expands a whole-disc location (or a track location, to that track) into playlist
    /// entries. Fails when there is no disc or no audio track.
    pub fn disc_entries(&self, location: &str) -> Result<Vec<CdEntry>, DecodeError> {
        let loc =
            CdLocation::parse(location).ok_or_else(|| DecodeError::Unsupported(location.into()))?;
        let (mut drive, token) = self.open_drive(&loc)?;
        let toc = drive.toc()?;
        let text = drive.cd_text();
        let mut entries = entries_for(&token, &toc, text.as_ref());
        if let Some(t) = loc.track {
            let one = CdLocation::track(&token, t).to_string();
            entries.retain(|e| e.location == one);
        }
        if entries.is_empty() {
            return Err(DecodeError::NotFound(format!(
                "no audio tracks in {location}"
            )));
        }
        Ok(entries)
    }
}

impl DecoderFactory for CdFactory {
    fn name(&self) -> &str {
        "Audio CD"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        if self.available() {
            vec![("cda".into(), "Audio CD tracks".into())]
        } else {
            Vec::new()
        }
    }

    fn claims(&self, location: &str) -> bool {
        strip_prefix_ci(location.trim(), SCHEME).is_some()
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        let loc =
            CdLocation::parse(location).ok_or_else(|| DecodeError::Unsupported(location.into()))?;
        let Some(track) = loc.track else {
            return Err(DecodeError::Open(format!(
                "{location} is a whole disc; add it to the playlist to list its tracks"
            )));
        };
        let (drive, _) = self.open_drive(&loc)?;
        Ok(Box::new(CdDecoder::new(drive, track)?))
    }
}

/// Sectors read per [`Decoder::read`] call (about 0.2 s).
const READ_SECTORS: u32 = 16;
/// Blocks in a row that may fail (each replaced by silence) before playback stops.
const MAX_FAILED_BLOCKS: u32 = 8;

/// Plays one audio track.
pub struct CdDecoder {
    drive: Box<dyn CdDrive>,
    info: StreamInfo,
    /// Sectors `[start, end)` of the track.
    start: u32,
    end: u32,
    /// Next sector to read.
    sector: u32,
    /// Frames to drop from the next block (seeking inside a sector).
    skip: usize,
    failed: u32,
    buf: Vec<u8>,
}

impl CdDecoder {
    /// Opens track `track` on an opened drive.
    pub fn new(mut drive: Box<dyn CdDrive>, track: u8) -> Result<Self, DecodeError> {
        let toc = drive.toc()?;
        let t = toc
            .track(track)
            .ok_or_else(|| DecodeError::NotFound(format!("no track {track} on the disc")))?;
        if !t.audio {
            return Err(DecodeError::Open(format!("track {track} is a data track")));
        }
        let (start, end) = toc
            .audio_range(track)
            .ok_or_else(|| DecodeError::Open(format!("track {track} is empty")))?;
        let info = StreamInfo {
            sample_rate: SAMPLE_RATE,
            channels: 2,
            bits: 16,
            length_ms: Some(frames_to_ms((end - start) as u64 * SECTOR_FRAMES)),
            seekable: true,
            codec: "Audio CD".into(),
            lossless: true,
        };
        Ok(Self {
            drive,
            info,
            start,
            end,
            sector: start,
            skip: 0,
            failed: 0,
            buf: vec![0; READ_SECTORS as usize * SECTOR_BYTES],
        })
    }

    fn total_frames(&self) -> u64 {
        (self.end - self.start) as u64 * SECTOR_FRAMES
    }

    /// Reads a block, retrying once; a block that still fails becomes silence.
    fn read_block(&mut self, sectors: u32) -> Result<(), DecodeError> {
        let buf = &mut self.buf[..sectors as usize * SECTOR_BYTES];
        let mut result = self.drive.read_audio(self.sector, buf);
        if result.is_err() {
            result = self.drive.read_audio(self.sector, buf);
        }
        match result {
            Ok(()) => self.failed = 0,
            Err(e) => {
                self.failed += 1;
                if self.failed >= MAX_FAILED_BLOCKS {
                    return Err(e);
                }
                log::warn!(
                    "CD read error at sector {}: {e}; playing silence",
                    self.sector
                );
                buf.fill(0);
            }
        }
        Ok(())
    }
}

/// Appends 16-bit little-endian stereo PCM as floats, dropping the first `skip` frames.
fn append_pcm(bytes: &[u8], skip: usize, out: &mut Vec<f32>) -> usize {
    let frames = bytes.len() / 4;
    let skip = skip.min(frames);
    out.extend(
        bytes[skip * 4..frames * 4]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| i16::from_le_bytes(*s) as f32 / 32768.0),
    );
    frames - skip
}

impl Decoder for CdDecoder {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError> {
        while self.sector < self.end {
            let sectors = READ_SECTORS.min(self.end - self.sector);
            self.read_block(sectors)?;
            self.sector += sectors;
            let n = append_pcm(&self.buf[..sectors as usize * SECTOR_BYTES], self.skip, out);
            self.skip = 0;
            if n > 0 {
                return Ok(ReadStatus::Frames(n));
            }
        }
        Ok(ReadStatus::End)
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        let frame = ms_to_frames(ms).min(self.total_frames());
        self.sector = self.start + (frame / SECTOR_FRAMES) as u32;
        self.skip = (frame % SECTOR_FRAMES) as usize;
        self.failed = 0;
        Ok(frames_to_ms(frame))
    }

    fn kbps(&self) -> Option<u32> {
        Some(1411)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A disc in memory: sample n of the disc (frame n, left) is `n as i16`, right is `-n`.
    #[derive(Clone)]
    struct FakeDisc {
        toc: Toc,
        text: Option<CdText>,
        /// Sectors that fail to read.
        bad: Vec<u32>,
        reads: Arc<Mutex<Vec<(u32, u32)>>>,
    }

    impl CdDrive for FakeDisc {
        fn toc(&mut self) -> Result<Toc, DecodeError> {
            if self.toc.tracks.is_empty() {
                return Err(DecodeError::Open("no disc".into()));
            }
            Ok(self.toc.clone())
        }

        fn cd_text(&mut self) -> Option<CdText> {
            self.text.clone()
        }

        fn read_audio(&mut self, lsn: u32, buf: &mut [u8]) -> Result<(), DecodeError> {
            let count = (buf.len() / SECTOR_BYTES) as u32;
            self.reads.lock().unwrap().push((lsn, count));
            if (lsn..lsn + count).any(|s| self.bad.contains(&s)) {
                return Err(DecodeError::Decode("bad sector".into()));
            }
            let first = lsn as u64 * SECTOR_FRAMES;
            for (i, f) in buf.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let v = (first + i as u64) as i16;
                f[..2].copy_from_slice(&v.to_le_bytes());
                f[2..].copy_from_slice(&v.wrapping_neg().to_le_bytes());
            }
            Ok(())
        }
    }

    struct FakeBackend(FakeDisc);

    impl CdBackend for FakeBackend {
        fn name(&self) -> &str {
            "fake"
        }

        fn devices(&self) -> Vec<String> {
            vec!["/dev/sr0".into(), "/dev/sr1".into()]
        }

        fn open(&self, device: &str) -> Result<Box<dyn CdDrive>, DecodeError> {
            match device {
                "/dev/sr0" => Ok(Box::new(self.0.clone())),
                "/dev/sr1" => Ok(Box::new(FakeDisc {
                    toc: Toc::default(),
                    ..self.0.clone()
                })),
                _ => Err(DecodeError::NotFound(device.into())),
            }
        }
    }

    fn track(number: u8, start: u32, audio: bool) -> TocTrack {
        TocTrack {
            number,
            start,
            audio,
        }
    }

    /// Three audio tracks (10 s, 1 s + 1 sector, 20 s) and a CD-Extra data session.
    fn toc() -> Toc {
        Toc {
            tracks: vec![
                track(1, 0, true),
                track(2, 750, true),
                track(3, 826, true),
                track(4, 826 + 1500 + CD_EXTRA_GAP, false),
            ],
            leadout: 30_000,
            last_session_start: Some(826 + 1500 + CD_EXTRA_GAP),
        }
    }

    fn text() -> CdText {
        let e = |t: Option<&str>, p: Option<&str>| CdTextEntry {
            title: t.map(String::from),
            performer: p.map(String::from),
        };
        CdText {
            disc: e(Some("Album"), Some("Band")),
            tracks: vec![
                (1, e(Some("Opening"), None)),
                (2, e(Some("Duet"), Some("Guest"))),
                (3, e(Some("  "), None)),
            ],
        }
    }

    fn disc() -> FakeDisc {
        FakeDisc {
            toc: toc(),
            text: None,
            bad: Vec::new(),
            reads: Arc::default(),
        }
    }

    fn factory(d: FakeDisc) -> CdFactory {
        CdFactory::with_backend(Arc::new(FakeBackend(d)))
    }

    fn decode_all(d: &mut dyn Decoder) -> Vec<f32> {
        let mut out = Vec::new();
        while d.read(&mut out).unwrap().frames() > 0 {}
        out
    }

    fn sample(n: u64) -> f32 {
        (n as i16) as f32 / 32768.0
    }

    #[test]
    fn parses_locations() {
        let p = CdLocation::parse;
        assert_eq!(p("cda://D,3"), Some(CdLocation::track("D", 3)));
        assert_eq!(p("CDA://d:,12"), Some(CdLocation::track("D", 12)));
        assert_eq!(p(" cda://sr0 "), Some(CdLocation::disc("sr0")));
        assert_eq!(p("cda://0,1"), Some(CdLocation::track("0", 1)));
        assert_eq!(
            p("cda:///dev/disk/by-id/x,02"),
            Some(CdLocation::track("/dev/disk/by-id/x", 2))
        );
        assert_eq!(p("cda://E:\\"), Some(CdLocation::disc("E")));
        assert_eq!(p("D:\\Track03.cda"), Some(CdLocation::track("D", 3)));
        assert_eq!(p("e:/track12.CDA"), Some(CdLocation::track("E", 12)));
        for bad in [
            "cda://",
            "cda://,3",
            "cda://D,0",
            "cda://D,100",
            "cda://D,x",
            "cda://D,",
            "http://D,3",
            "/music/Track01.cda",
            "D:\\Track.cda",
            "D:\\Song01.cda",
        ] {
            assert_eq!(p(bad), None, "{bad}");
        }
        assert_eq!(CdLocation::track("sr0", 7).to_string(), "cda://sr0,7");
        assert_eq!(CdLocation::disc("D").to_string(), "cda://D");
        assert!(is_whole_disc("cda://D"));
        assert!(!is_whole_disc("cda://D,1"));
        assert!(!is_whole_disc("/a.mp3"));
    }

    #[test]
    fn drive_tokens_and_devices() {
        assert_eq!(drive_token("/dev/sr0"), "sr0");
        assert_eq!(drive_token("/dev/disk/by-id/x"), "/dev/disk/by-id/x");
        assert_eq!(drive_token("D:"), "D");
        let devs = vec!["/dev/sr0".to_string(), "/dev/sr1".to_string()];
        assert_eq!(device_for("1", &devs).as_deref(), Some("/dev/sr1"));
        assert_eq!(device_for("2", &devs), None);
        assert_eq!(device_for("d", &devs).as_deref(), Some("D:"));
        assert_eq!(device_for("sr0", &devs).as_deref(), Some("/dev/sr0"));
        assert_eq!(
            device_for("/dev/cdrom", &devs).as_deref(),
            Some("/dev/cdrom")
        );
    }

    #[test]
    fn toc_ranges() {
        let t = toc();
        assert_eq!(t.audio_range(1), Some((0, 750)));
        assert_eq!(t.audio_range(2), Some((750, 826)));
        // The last audio track ends before the Enhanced CD session gap.
        assert_eq!(t.audio_range(3), Some((826, 826 + 1500)));
        assert_eq!(t.audio_range(4), None);
        assert_eq!(t.audio_range(9), None);
        assert_eq!(t.length_ms(1), Some(10_000));
        assert_eq!(t.length_ms(2), Some(1013));
        assert_eq!(t.length_ms(3), Some(20_000));

        // A data track in the same session: only its pregap is left out.
        let mut same = toc();
        same.last_session_start = None;
        assert_eq!(
            same.audio_range(3),
            Some((826, 826 + 1500 + CD_EXTRA_GAP - 150))
        );
        // A plain audio disc: the last track runs to the lead-out.
        let mut audio = toc();
        audio.tracks.pop();
        assert_eq!(audio.audio_range(3), Some((826, 30_000)));
    }

    #[test]
    fn titles_fall_back() {
        let t = text();
        assert_eq!(track_title(1, Some(&t)), "Band - Opening");
        assert_eq!(track_title(2, Some(&t)), "Guest - Duet");
        assert_eq!(track_title(3, Some(&t)), "Track 03");
        assert_eq!(track_title(4, Some(&t)), "Track 04");
        assert_eq!(track_title(7, None), "Track 07");
        let no_artist = CdText {
            tracks: vec![(
                1,
                CdTextEntry {
                    title: Some("Solo".into()),
                    performer: None,
                },
            )],
            ..CdText::default()
        };
        assert_eq!(track_title(1, Some(&no_artist)), "Solo");
    }

    #[test]
    fn toc_to_entries() {
        let entries = entries_for("sr0", &toc(), Some(&text()));
        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.location.as_str(), e.title.as_str(), e.length_ms))
            .collect();
        assert_eq!(
            summary,
            [
                ("cda://sr0,1", "Band - Opening", Some(10_000)),
                ("cda://sr0,2", "Guest - Duet", Some(1013)),
                ("cda://sr0,3", "Track 03", Some(20_000)),
            ]
        );
    }

    #[test]
    fn factory_expands_discs_and_lists_drives() {
        let f = factory(FakeDisc {
            text: Some(text()),
            ..disc()
        });
        assert!(f.available());
        assert_eq!(
            f.drives(),
            [
                CdDriveInfo {
                    location: "cda://sr0".into(),
                    label: "/dev/sr0".into(),
                },
                CdDriveInfo {
                    location: "cda://sr1".into(),
                    label: "/dev/sr1".into(),
                },
            ]
        );
        // Drive indices resolve to the device's canonical token.
        let all = f.disc_entries("cda://0").unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].location, "cda://sr0,1");
        let one = f.disc_entries("cda://sr0,2").unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].title, "Guest - Duet");
        // No disc in the second drive; no third drive.
        assert!(matches!(
            f.disc_entries("cda://sr1"),
            Err(DecodeError::Open(_))
        ));
        assert!(matches!(
            f.disc_entries("cda://2"),
            Err(DecodeError::NotFound(_))
        ));
        assert!(f.open("cda://sr1,1").is_err());
        assert!(f.open("cda://sr0").is_err(), "whole disc is not playable");
        assert!(matches!(f.open("cda://sr0,4"), Err(DecodeError::Open(_))));
        assert!(matches!(
            f.open("cda://sr0,9"),
            Err(DecodeError::NotFound(_))
        ));

        let none = CdFactory::default();
        assert!(!none.available());
        assert!(none.drives().is_empty());
        assert!(none.extensions().is_empty());
        assert!(none.claims("CDA://D,1"));
        assert!(matches!(
            none.open("cda://D,1"),
            Err(DecodeError::Unsupported(_))
        ));
    }

    #[test]
    fn registry_routes_cd_locations() {
        let mut r = crate::Registry::empty();
        r.add(Box::new(factory(disc())));
        assert_eq!(r.select("cda://sr0,1").map(|f| f.name()), Some("Audio CD"));
        assert!(r.is_supported_extension("cda"));
        let d = r.open("cda://sr0,2").unwrap();
        assert_eq!(d.info().length_ms, Some(1013));
        assert_eq!(
            crate::Registry::with_defaults()
                .select("cda://D,1")
                .map(|f| f.name()),
            Some("Audio CD")
        );
    }

    #[test]
    fn decodes_whole_tracks_without_gaps() {
        let f = factory(disc());
        let mut pcm = Vec::new();
        for t in 1..=3 {
            let mut d = f.open(&format!("cda://sr0,{t}")).unwrap();
            assert_eq!(d.info().sample_rate, 44_100);
            assert_eq!(d.info().channels, 2);
            assert_eq!(d.info().bits, 16);
            assert!(d.info().seekable && d.info().lossless);
            pcm.extend(decode_all(d.as_mut()));
            assert_eq!(d.read(&mut Vec::new()).unwrap(), ReadStatus::End);
        }
        // Consecutive tracks join sample-exactly.
        assert_eq!(pcm.len() as u64, 2 * 2326 * SECTOR_FRAMES);
        for n in [0u64, 1, 441_000 - 1, 441_000, 1_000_000, 2326 * 588 - 1] {
            assert_eq!(pcm[n as usize * 2], sample(n), "frame {n}");
            assert_eq!(pcm[n as usize * 2 + 1], -sample(n), "frame {n}");
        }
    }

    #[test]
    fn seeks_to_exact_frames() {
        let d = disc();
        let reads = d.reads.clone();
        let f = factory(d);
        let mut d = f.open("cda://sr0,1").unwrap();
        // 1234 ms = frame 54_419 = sector 92 + 323 frames.
        assert_eq!(ms_to_frames(1234), 54_419);
        assert_eq!(d.seek(1234).unwrap(), 1234);
        let mut out = Vec::new();
        let n = d.read(&mut out).unwrap().frames();
        assert_eq!(n, 16 * 588 - 323);
        assert_eq!(out[0], sample(54_419));
        assert_eq!(reads.lock().unwrap().last(), Some(&(92, 16)));
        let rest = decode_all(d.as_mut());
        assert_eq!((out.len() + rest.len()) / 2, 441_000 - 54_419);

        // Past the end: clamped to the end of the track.
        assert_eq!(d.seek(60_000).unwrap(), 10_000);
        assert_eq!(d.read(&mut Vec::new()).unwrap(), ReadStatus::End);
        assert_eq!(d.seek(0).unwrap(), 0);
        let mut out = Vec::new();
        d.read(&mut out).unwrap();
        assert_eq!(out[2], sample(1));

        // A track not starting at sector 0.
        let mut d = f.open("cda://sr0,3").unwrap();
        assert_eq!(d.seek(10).unwrap(), 10);
        let mut out = Vec::new();
        d.read(&mut out).unwrap();
        assert_eq!(out[0], sample(826 * 588 + 441));
    }

    #[test]
    fn seek_math_round_trips() {
        for ms in [0, 1, 9, 10, 999, 1000, 59_999, 3_600_000] {
            let f = ms_to_frames(ms);
            assert_eq!(frames_to_ms(f), ms, "{ms}");
        }
        assert_eq!(frames_to_ms(SECTOR_FRAMES * 75), 1000);
        assert_eq!(frames_to_ms(588), 13);
    }

    #[test]
    fn read_errors_become_silence_then_fail() {
        let f = factory(FakeDisc {
            bad: vec![20],
            ..disc()
        });
        let mut d = f.open("cda://sr0,1").unwrap();
        let pcm = decode_all(d.as_mut());
        assert_eq!(pcm.len(), 2 * 441_000);
        assert_eq!(pcm[2 * 16 * 588 + 2], 0.0, "bad block is silent");
        assert_eq!(pcm[2 * 32 * 588], sample(32 * 588));

        let f = factory(FakeDisc {
            bad: (0..750).collect(),
            ..disc()
        });
        let mut d = f.open("cda://sr0,1").unwrap();
        let mut out = Vec::new();
        let mut result = Ok(ReadStatus::End);
        for _ in 0..MAX_FAILED_BLOCKS {
            result = d.read(&mut out);
        }
        assert!(result.is_err());
    }
}
