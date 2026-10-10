//! Metadata field mapping and tag access (spec 10 §7, 09 §1–§3, 08 §3.4).
//!
//! Fields use nulamp's lower-case keys ([`FIELDS`]) whatever the tag format; the mapping to
//! ID3v2 frames, APEv2 items, Vorbis comments and MP4 atoms follows spec 10 §7.2 and is
//! kept here, independent of the tag library's own key names. Tags are parsed and
//! serialized with `lofty`; ID3v1 is handled by [`id3v1`] in this crate.
//!
//! Reading merges tags field by field: ID3v2, then APEv2, then ID3v1 (MPEG and AAC
//! files); APEv2 then ID3v1 (Monkey's Audio, WavPack, Musepack); ID3v2 then RIFF `INFO`
//! (WAV). The first tag that has a field wins.
//!
//! Writing never truncates a file in place before the new data is ready. Files with tags at
//! both ends (MPEG, AAC, Monkey's Audio, WavPack, Musepack) are written by this crate: in
//! place when the new ID3v2 tag fits in the old tag and its padding, otherwise through a
//! temporary file renamed over the original. Other formats are written by `lofty` (which
//! reads the whole file and rewrites it) into a temporary copy that then replaces the
//! original; an emptied MP4 tag is blanked in place. ASF/WMA files are not supported
//! (`lofty` has no ASF support); the application can register an [`ExternalReader`] (FFmpeg)
//! that reads them, and other formats `lofty` does not know, read-only. Choices the spec leaves open are recorded as `MD*` rows in
//! `docs/decisions.md`.

pub mod id3v1;

mod ape;
mod genre;
mod id3v2;
mod layout;
mod mp4;
mod rating;
mod riff;
mod tech;
mod vorbis;

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use lofty::ape::{ApeFile, ApeTag};
use lofty::config::{ParseOptions, ParsingMode, WriteOptions};
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::id3::v2::{Id3v2Tag, Id3v2TagFlags, Id3v2Version};
use lofty::iff::aiff::AiffFile;
use lofty::iff::wav::{RiffInfoList, WavFile, WavFormat};
use lofty::mp4::{Ilst, Mp4Codec, Mp4File};
use lofty::mpeg::{ChannelMode, Layer, MpegFile, MpegVersion};
use lofty::musepack::MpcFile;
use lofty::ogg::{OggPictureStorage, OpusFile, SpeexFile, VorbisComments, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::TagExt;
use lofty::wavpack::WavPackFile;

pub use genre::{GENRES, NO_GENRE, decode_tcon, genre_index, genre_name};
pub use id3v1::Id3v1;
pub use rating::{percent_to_stars, popm_to_stars, stars_to_percent, stars_to_popm};
pub use tech::{TECH_FIELDS, TechInfo};

use layout::{EndTagsEdit, Layout, Part};

/// Tag fields of spec 10 §7.1. `rating` holds 0–5 stars; `track` and `disc` hold `n` or `n/N`.
pub const FIELDS: &[&str] = &[
    "title",
    "artist",
    "album",
    "albumartist",
    "year",
    "genre",
    "comment",
    "track",
    "disc",
    "composer",
    "publisher",
    "conductor",
    "bpm",
    "rating",
    "category",
    "lyricist",
    "key",
    "mood",
    "subtitle",
    "isrc",
    "remixing",
    "originalartist",
    "encoder",
    "copyright",
    "compilation",
    "url",
    "tool",
    "replaygain_track_gain",
    "replaygain_track_peak",
    "replaygain_album_gain",
    "replaygain_album_peak",
];

/// Opus loudness fields: raw `R128_TRACK_GAIN` / `R128_ALBUM_GAIN` values (Q7.8 dB relative
/// to −23 LUFS, spec 07 §5.3).
pub const R128_FIELDS: [&str; 2] = ["r128_track_gain", "r128_album_gain"];

/// Errors from reading or writing metadata.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// File system error.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// The tag library could not parse or write the file.
    #[error("tag error: {0}")]
    Tag(String),
    /// The file is not in a format with tag support.
    #[error("unsupported file format")]
    UnsupportedFormat,
    /// A value was given for a field the format cannot store (or a technical field).
    #[error("field `{0}` is not supported by this format")]
    UnsupportedField(String),
    /// A value that the field cannot hold, for example a rating outside 0–5.
    #[error("invalid value `{value}` for field `{field}`")]
    InvalidValue { field: String, value: String },
    /// The file is read-only.
    #[error("file is read-only")]
    ReadOnly,
}

impl From<lofty::error::LoftyError> for Error {
    fn from(e: lofty::error::LoftyError) -> Self {
        match e.kind() {
            lofty::error::ErrorKind::Io(io) => Error::Io(io::Error::new(io.kind(), e.to_string())),
            lofty::error::ErrorKind::UnknownFormat => Error::UnsupportedFormat,
            _ => Error::Tag(e.to_string()),
        }
    }
}

/// Result of this crate's functions.
pub type Result<T> = std::result::Result<T, Error>;

/// Reading and writing options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// E-mail of the ID3v2 `POPM` frame holding the rating; empty uses any `POPM` frame
    /// (spec 10 §7.3). New frames are written with this e-mail.
    pub rating_email: String,
    /// Write ID3v2.4 even when the file has no tag or an ID3v2.2/2.3 tag (spec 09 §2 option).
    pub id3v2_always_v24: bool,
    /// Read ID3v1 tags.
    pub read_id3v1: bool,
    /// Read ID3v2 tags.
    pub read_id3v2: bool,
    /// Read APEv2 tags.
    pub read_ape: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rating_email: String::new(),
            id3v2_always_v24: false,
            read_id3v1: true,
            read_id3v2: true,
            read_ape: true,
        }
    }
}

/// An embedded picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cover {
    /// MIME type, for example `image/jpeg`.
    pub mime: String,
    pub data: Vec<u8>,
}

impl Cover {
    fn from_picture(picture: &Picture) -> Self {
        let data = picture.data().to_vec();
        let mime = picture
            .mime_type()
            .map(|m| m.as_str().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| sniff_mime(&data).to_string());
        Self { mime, data }
    }

    fn to_picture(&self) -> Picture {
        Picture::unchecked(self.data.clone())
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::from_str(&self.mime))
            .build()
    }
}

/// MIME type from an image's magic bytes.
fn sniff_mime(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'B', b'M', ..] => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// Tag fields and technical information of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    fields: BTreeMap<String, String>,
    tech: TechInfo,
    supported: Vec<&'static str>,
    path: Option<PathBuf>,
}

impl Metadata {
    /// Empty metadata that supports every field.
    pub fn new() -> Self {
        Self {
            supported: FIELDS.to_vec(),
            ..Self::default()
        }
    }

    /// Sets the path answered as `filename`.
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// The file's path, when known.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Value of a tag field, a technical field ([`TECH_FIELDS`]) or `filename`.
    /// `None` when missing or empty.
    pub fn get(&self, name: &str) -> Option<String> {
        if let Some(v) = self.fields.get(name) {
            return Some(v.clone());
        }
        if name == "filename" {
            return self.path.as_ref().map(|p| p.to_string_lossy().into_owned());
        }
        if TECH_FIELDS.contains(&name) {
            return self.tech.get(name);
        }
        None
    }

    /// Tag field values by key (only non-empty values).
    pub fn fields(&self) -> &BTreeMap<String, String> {
        &self.fields
    }

    pub fn tech(&self) -> &TechInfo {
        &self.tech
    }

    /// Tag fields the file's format can store; the others are disabled in the file-info dialog.
    pub fn supported_fields(&self) -> &[&'static str] {
        &self.supported
    }

    /// True when `name` is a tag field the format can store, or a technical field.
    pub fn supports(&self, name: &str) -> bool {
        self.supported.contains(&name) || TECH_FIELDS.contains(&name)
    }

    /// Sets (or with `None` or empty, clears) a tag field in memory.
    pub fn set(&mut self, name: &str, value: Option<&str>) {
        match value.filter(|v| !v.is_empty()) {
            Some(v) => self.fields.insert(name.to_string(), v.to_string()),
            None => self.fields.remove(name),
        };
    }
}

/// Fields for the title formatter: tag fields, technical fields and `filename`.
impl nulamp_tf::FieldSource for Metadata {
    fn field(&self, name: &str) -> Option<String> {
        self.get(name)
    }
}

/// Reads files whose format the tag library does not support (ASF/WMA, TTA): tag fields by
/// nulamp key ([`FIELDS`]) and technical information, or `None` when it cannot read the file.
pub type ExternalReader = fn(&Path) -> Option<(Vec<(String, String)>, TechInfo)>;

static EXTERNAL_READER: OnceLock<ExternalReader> = OnceLock::new();

/// Registers the reader [`read`] uses for formats the tag library does not support. Such files
/// are read-only: they report no supported fields, and writes fail with
/// [`Error::UnsupportedFormat`]. Only the first registration counts (returns `false` after).
pub fn set_external_reader(reader: ExternalReader) -> bool {
    EXTERNAL_READER.set(reader).is_ok()
}

fn read_external(path: &Path) -> Option<Metadata> {
    let (fields, tech) = EXTERNAL_READER.get()?(path)?;
    Some(Metadata {
        fields: fields
            .into_iter()
            .filter(|(k, v)| FIELDS.contains(&k.as_str()) && !v.is_empty())
            .collect(),
        tech,
        supported: Vec::new(),
        path: Some(path.to_path_buf()),
    })
}

/// Reads the tags and technical fields of `path` with default [`Options`].
pub fn read(path: impl AsRef<Path>) -> Result<Metadata> {
    read_with(path, &Options::default())
}

/// Reads the tags and technical fields of `path`.
pub fn read_with(path: impl AsRef<Path>, options: &Options) -> Result<Metadata> {
    let path = path.as_ref();
    let kind = match probe(path) {
        Err(Error::UnsupportedFormat) => {
            return read_external(path).ok_or(Error::UnsupportedFormat);
        }
        kind => kind?,
    };
    let tags = Tags::load(path, kind, false)?;
    Ok(Metadata {
        fields: tags.fields(options),
        supported: supported(tags.kind),
        tech: tags.tech,
        path: Some(path.to_path_buf()),
    })
}

/// Tag fields the format of `path` can store; empty when the format is not supported.
pub fn supported_fields(path: impl AsRef<Path>) -> Vec<&'static str> {
    probe(path.as_ref()).map(supported).unwrap_or_default()
}

/// Writes field changes to `path` with default [`Options`]. `None` or an empty value
/// deletes the field and its aliases.
pub fn write(path: impl AsRef<Path>, changes: &[(String, Option<String>)]) -> Result<()> {
    write_with(path, changes, &Options::default())
}

/// Writes field changes to `path`. `None` or an empty value deletes the field and its
/// aliases; setting a field also removes its aliases.
///
/// MPEG and AAC: ID3v2 is created when missing and written as ID3v2.4 only when the file
/// already had a 2.4 tag (or [`Options::id3v2_always_v24`]), otherwise as 2.3 (UTF-16 text);
/// ID3v1 is created when missing; an existing APEv2 tag is updated, none is created.
/// Monkey's Audio, WavPack and Musepack: APEv2 is created when missing; an existing ID3v1
/// tag is updated. An ID3v1 tag left with no fields is removed. A newly created tag starts
/// with the values read from the file's other tags.
///
/// Fails without touching the file when a value is given for a field the format cannot
/// store, or a `rating` is not 0–5.
pub fn write_with(
    path: impl AsRef<Path>,
    changes: &[(String, Option<String>)],
    options: &Options,
) -> Result<()> {
    write_impl(path.as_ref(), changes, None, options)
}

/// Reads the embedded cover: the first front cover, else the first picture.
pub fn read_cover(path: impl AsRef<Path>) -> Result<Option<Cover>> {
    let path = path.as_ref();
    let tags = Tags::load(path, probe(path)?, true)?;
    Ok(tags.cover())
}

/// Replaces the embedded front cover, or removes it with `None`.
pub fn write_cover(path: impl AsRef<Path>, cover: Option<&Cover>) -> Result<()> {
    write_impl(path.as_ref(), &[], Some(cover), &Options::default())
}

/// Joins NUL-separated values (ID3v2.4, APEv2) with `; `.
pub(crate) fn join_values(s: &str) -> String {
    s.split('\0')
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// File formats with tag support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mpeg,
    Aac,
    Ape,
    WavPack,
    Mpc,
    Flac,
    Vorbis,
    Opus,
    Speex,
    Mp4,
    Wav,
    Aiff,
}

impl Kind {
    /// Formats with tags at the start and end of the file, written by [`layout`].
    fn has_end_tags(self) -> bool {
        matches!(
            self,
            Kind::Mpeg | Kind::Aac | Kind::Ape | Kind::WavPack | Kind::Mpc
        )
    }

    /// Formats whose leading ID3v2 tag is written.
    fn writes_id3v2(self) -> bool {
        matches!(self, Kind::Mpeg | Kind::Aac | Kind::Wav | Kind::Aiff)
    }
}

/// Detects the format from the file's content, falling back to the extension.
fn probe(path: &Path) -> Result<Kind> {
    let file_type = Probe::open(path)?.guess_file_type()?.file_type();
    Ok(match file_type.ok_or(Error::UnsupportedFormat)? {
        FileType::Mpeg => Kind::Mpeg,
        FileType::Aac => Kind::Aac,
        FileType::Ape => Kind::Ape,
        FileType::WavPack => Kind::WavPack,
        FileType::Mpc => Kind::Mpc,
        FileType::Flac => Kind::Flac,
        FileType::Vorbis => Kind::Vorbis,
        FileType::Opus => Kind::Opus,
        FileType::Speex => Kind::Speex,
        FileType::Mp4 => Kind::Mp4,
        FileType::Wav => Kind::Wav,
        FileType::Aiff => Kind::Aiff,
        _ => return Err(Error::UnsupportedFormat),
    })
}

fn supported(kind: Kind) -> Vec<&'static str> {
    match kind {
        Kind::Mpeg | Kind::Aac | Kind::Wav | Kind::Aiff => FIELDS.to_vec(),
        Kind::Ape | Kind::WavPack | Kind::Mpc => ape::FIELDS.to_vec(),
        Kind::Flac | Kind::Vorbis | Kind::Speex => vorbis::FIELDS.to_vec(),
        Kind::Opus => [vorbis::FIELDS, &R128_FIELDS[..]].concat(),
        Kind::Mp4 => mp4::FIELDS.to_vec(),
    }
}

/// All tags of a file, as parsed.
struct Tags {
    kind: Kind,
    id3v2: Option<Id3v2Tag>,
    ape: Option<ApeTag>,
    id3v1: Option<Id3v1>,
    vorbis: Option<VorbisComments>,
    ilst: Option<Ilst>,
    riff: Option<RiffInfoList>,
    layout: Option<Layout>,
    tech: TechInfo,
}

impl Tags {
    fn load(path: &Path, kind: Kind, read_cover_art: bool) -> Result<Self> {
        let mut reader = BufReader::new(File::open(path)?);
        let options = ParseOptions::new()
            .read_cover_art(read_cover_art)
            .implicit_conversions(false)
            .parsing_mode(ParsingMode::Relaxed);
        let mut tags = Tags {
            kind,
            id3v2: None,
            ape: None,
            id3v1: None,
            vorbis: None,
            ilst: None,
            riff: None,
            layout: None,
            tech: TechInfo::default(),
        };
        let lossless = |props, family: &str, mime| TechInfo {
            lossless: true,
            ..TechInfo::new(props, family, mime)
        };
        match kind {
            Kind::Mpeg => {
                let mut f = MpegFile::read_from(&mut reader, options)?;
                tags.tech = mpeg_tech(f.properties());
                tags.id3v2 = f.remove_id3v2();
                tags.ape = f.remove_ape();
            }
            Kind::Aac => {
                let mut f = lofty::aac::AacFile::read_from(&mut reader, options)?;
                tags.tech = TechInfo::new((*f.properties()).into(), "AAC (ADTS)", "audio/aac");
                tags.id3v2 = f.remove_id3v2();
            }
            Kind::Ape => {
                let mut f = ApeFile::read_from(&mut reader, options)?;
                tags.tech = lossless(
                    f.properties().clone().into(),
                    "Monkey's Audio",
                    "audio/x-ape",
                );
                tags.ape = f.remove_ape();
            }
            Kind::WavPack => {
                let mut f = WavPackFile::read_from(&mut reader, options)?;
                tags.tech = TechInfo::new((*f.properties()).into(), "WavPack", "audio/x-wavpack");
                tags.tech.lossless = f.properties().is_lossless();
                tags.ape = f.remove_ape();
            }
            Kind::Mpc => {
                let mut f = MpcFile::read_from(&mut reader, options)?;
                tags.tech = TechInfo::new(
                    f.properties().clone().into(),
                    "Musepack",
                    "audio/x-musepack",
                );
                tags.ape = f.remove_ape();
            }
            Kind::Flac => {
                let mut f = FlacFile::read_from(&mut reader, options)?;
                tags.tech = lossless((*f.properties()).into(), "FLAC", "audio/flac");
                let mut comments = f.remove_vorbis_comments().unwrap_or_default();
                for (picture, info) in f.remove_pictures() {
                    comments.insert_picture(picture, Some(info))?;
                }
                tags.vorbis = Some(comments);
            }
            Kind::Vorbis => {
                let mut f = VorbisFile::read_from(&mut reader, options)?;
                tags.tech = TechInfo::new((*f.properties()).into(), "Ogg Vorbis", "audio/ogg");
                tags.vorbis = Some(f.remove_vorbis_comments());
            }
            Kind::Opus => {
                let mut f = OpusFile::read_from(&mut reader, options)?;
                tags.tech = TechInfo::new((*f.properties()).into(), "Ogg Opus", "audio/ogg");
                tags.vorbis = Some(f.remove_vorbis_comments());
            }
            Kind::Speex => {
                let mut f = SpeexFile::read_from(&mut reader, options)?;
                tags.tech = TechInfo::new((*f.properties()).into(), "Ogg Speex", "audio/ogg");
                tags.vorbis = Some(f.remove_vorbis_comments());
            }
            Kind::Mp4 => {
                let mut f = Mp4File::read_from(&mut reader, options)?;
                let props = f.properties();
                let (family, is_lossless) = match props.codec() {
                    Mp4Codec::AAC => ("MPEG-4 AAC", false),
                    Mp4Codec::ALAC => ("MPEG-4 ALAC", true),
                    Mp4Codec::MP3 => ("MPEG-4 MP3", false),
                    Mp4Codec::FLAC => ("MPEG-4 FLAC", true),
                    _ => ("MPEG-4 Audio", false),
                };
                tags.tech = TechInfo::new(props.clone().into(), family, "audio/mp4");
                tags.tech.lossless = is_lossless;
                tags.ilst = f.remove_ilst();
            }
            Kind::Wav => {
                let mut f = WavFile::read_from(&mut reader, options)?;
                let props = f.properties();
                let (family, is_lossless) = match props.format() {
                    WavFormat::PCM => ("WAV (PCM)".to_string(), true),
                    WavFormat::IEEE_FLOAT => ("WAV (IEEE float)".to_string(), true),
                    WavFormat::Other(tag) => (format!("WAV (format 0x{tag:04X})"), false),
                };
                tags.tech = TechInfo::new((*props).into(), family, "audio/wav");
                tags.tech.lossless = is_lossless;
                tags.id3v2 = f.remove_id3v2();
                tags.riff = f.remove_riff_info();
            }
            Kind::Aiff => {
                let mut f = AiffFile::read_from(&mut reader, options)?;
                tags.tech = lossless(f.properties().clone().into(), "AIFF", "audio/aiff");
                tags.id3v2 = f.remove_id3v2();
            }
        }
        let file = reader.get_mut();
        tags.tech.file_size = Some(file.seek(SeekFrom::End(0))?);
        if kind.has_end_tags() {
            let layout = Layout::scan(file)?;
            if layout.id3v1.is_some() {
                tags.id3v1 = Id3v1::read_from(file)?;
            }
            tags.layout = Some(layout);
        }
        Ok(tags)
    }

    /// Merged field values, first tag that has a field winning.
    fn fields(&self, options: &Options) -> BTreeMap<String, String> {
        FIELDS
            .iter()
            .chain(&R128_FIELDS)
            .filter_map(|&f| self.get(f, options).map(|v| (f.to_string(), v)))
            .collect()
    }

    fn get(&self, field: &str, options: &Options) -> Option<String> {
        let id3v2 = self.id3v2.as_ref().filter(|_| options.read_id3v2);
        let ape = self.ape.as_ref().filter(|_| options.read_ape);
        let id3v1 = self.id3v1.as_ref().filter(|_| options.read_id3v1);
        id3v2
            .and_then(|t| id3v2::get(t, field, &options.rating_email))
            .or_else(|| ape.and_then(|t| ape::get(t, field)))
            .or_else(|| self.vorbis.as_ref().and_then(|t| vorbis::get(t, field)))
            .or_else(|| self.ilst.as_ref().and_then(|t| mp4::get(t, field)))
            .or_else(|| self.riff.as_ref().and_then(|t| riff::get(t, field)))
            .or_else(|| id3v1.and_then(|t| t.get(field)))
    }

    fn cover(&self) -> Option<Cover> {
        self.id3v2
            .as_ref()
            .and_then(id3v2::cover)
            .or_else(|| self.ape.as_ref().and_then(ape::cover))
            .or_else(|| {
                self.vorbis
                    .as_ref()
                    .and_then(|t| vorbis::cover_in(t.pictures()))
            })
            .or_else(|| self.ilst.as_ref().and_then(mp4::cover))
    }

    /// Applies changes; returns whether ID3v2 is written as version 2.4.
    fn apply(
        &mut self,
        changes: &[(&str, Option<&str>)],
        cover: Option<Option<&Cover>>,
        options: &Options,
    ) -> bool {
        let v24 = options.id3v2_always_v24
            || self
                .id3v2
                .as_ref()
                .is_some_and(|t| t.original_version() == Id3v2Version::V4);
        let email = options.rating_email.as_str();
        let merged = self.fields(&Options {
            rating_email: options.rating_email.clone(),
            ..Options::default()
        });
        let created = match self.kind {
            k if k.writes_id3v2() => self
                .id3v2
                .is_none()
                .then(|| self.id3v2 = Some(Id3v2Tag::new())),
            Kind::Ape | Kind::WavPack | Kind::Mpc => {
                self.ape.is_none().then(|| self.ape = Some(ApeTag::new()))
            }
            Kind::Flac | Kind::Vorbis | Kind::Opus | Kind::Speex => self
                .vorbis
                .is_none()
                .then(|| self.vorbis = Some(VorbisComments::new())),
            _ => self.ilst.is_none().then(|| self.ilst = Some(Ilst::new())),
        };
        if created.is_some() {
            for (field, value) in &merged {
                let value = Some(value.as_str());
                if let Some(t) = &mut self.id3v2 {
                    id3v2::set(t, field, value, v24, email);
                } else if let Some(t) = &mut self.ape {
                    ape::set(t, field, value);
                }
            }
        }
        for &(field, value) in changes {
            if let Some(t) = &mut self.id3v2 {
                id3v2::set(t, field, value, v24, email);
            }
            if let Some(t) = &mut self.ape {
                ape::set(t, field, value);
            }
            if let Some(t) = &mut self.vorbis {
                vorbis::set(t, field, value);
            }
            if let Some(t) = &mut self.ilst {
                mp4::set(t, field, value);
            }
            if let Some(t) = &mut self.riff {
                riff::set(t, field, value);
            }
            if let Some(t) = &mut self.id3v1 {
                t.set(field, value);
            }
        }
        if let Some(cover) = cover {
            if let Some(t) = &mut self.id3v2 {
                id3v2::set_cover(t, cover);
            }
            if let Some(t) = &mut self.ape {
                ape::set_cover(t, cover);
            }
            if let Some(t) = &mut self.vorbis {
                vorbis::set_cover_in(t, cover);
            }
            if let Some(t) = &mut self.ilst {
                mp4::set_cover(t, cover);
            }
        }
        if matches!(self.kind, Kind::Mpeg | Kind::Aac) && self.id3v1.is_none() {
            let mut tag = Id3v1::default();
            for field in id3v1::FIELDS {
                tag.set(field, self.get(field, &Options::default()).as_deref());
            }
            self.id3v1 = Some(tag);
        }
        if self.id3v1.as_ref().is_some_and(Id3v1::is_empty) {
            self.id3v1 = None;
        }
        v24
    }

    fn save(&self, path: &Path, v24: bool) -> Result<()> {
        let options = WriteOptions::new()
            .respect_read_only(false)
            .use_id3v23(!v24);
        let id3v2 = self.id3v2.as_ref().map(|t| {
            let mut t = t.clone();
            t.set_flags(Id3v2TagFlags::default());
            t
        });
        if let Some(layout) = &self.layout {
            let front = match &id3v2 {
                Some(t) if self.kind.writes_id3v2() => {
                    let mut bytes = Vec::new();
                    t.dump_to(&mut bytes, options.preferred_padding(0))?;
                    Part::Replace(bytes)
                }
                _ => Part::Keep,
            };
            let ape = match (&self.ape, self.kind) {
                (_, Kind::Aac) | (None, _) => Part::Keep,
                (Some(t), _) => Part::Replace(ape_bytes(t, options)?),
            };
            let id3v1 = Part::Replace(
                self.id3v1
                    .as_ref()
                    .map(|t| t.to_bytes().to_vec())
                    .unwrap_or_default(),
            );
            layout::write_end_tags(path, layout, &EndTagsEdit { front, ape, id3v1 })?;
            return Ok(());
        }
        if self.ilst.as_ref().is_some_and(Ilst::is_empty) {
            let mut file = OpenOptions::new().read(true).write(true).open(path)?;
            mp4::blank_ilst(&mut file)?;
            file.sync_all()?;
            return Ok(());
        }
        layout::edit_copy(path, |file: &mut File| -> Result<()> {
            if let Some(t) = &self.vorbis {
                t.save_to(file, options)?;
            }
            if let Some(t) = &self.ilst {
                t.save_to(file, options)?;
            }
            if let Some(t) = &self.riff {
                file.rewind()?;
                t.save_to(file, options)?;
            }
            if let Some(t) = &id3v2 {
                file.rewind()?;
                t.save_to(file, options)?;
            }
            Ok(())
        })
    }
}

/// Serialized APEv2 tag with header and footer; empty when the tag has no items.
///
/// Flag bit 30 ("tag contains no footer" in APEv2) is cleared in both, as the footer is
/// always written.
fn ape_bytes(tag: &ApeTag, options: WriteOptions) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    tag.dump_to(&mut bytes, options)?;
    if bytes.len() >= 64 {
        let footer = bytes.len() - 32;
        for flags in [20, footer + 20] {
            bytes[flags + 3] &= !0x40;
        }
    }
    Ok(bytes)
}

fn mpeg_tech(props: &lofty::mpeg::MpegProperties) -> TechInfo {
    let version = match props.version() {
        MpegVersion::V1 => "1",
        MpegVersion::V2 => "2",
        MpegVersion::V2_5 => "2.5",
        _ => "4",
    };
    let layer = match props.layer() {
        Layer::Layer1 => 1,
        Layer::Layer2 => 2,
        Layer::Layer3 => 3,
    };
    let mut tech = TechInfo::new(
        (*props).into(),
        format!("MPEG-{version} Layer {layer}"),
        "audio/mpeg",
    );
    tech.mode = Some(
        match props.channel_mode() {
            ChannelMode::Stereo => "Stereo",
            ChannelMode::JointStereo => "Joint Stereo",
            ChannelMode::DualChannel => "Dual Channel",
            ChannelMode::SingleChannel => "Mono",
        }
        .to_string(),
    );
    tech
}

fn write_impl(
    path: &Path,
    changes: &[(String, Option<String>)],
    cover: Option<Option<&Cover>>,
    options: &Options,
) -> Result<()> {
    let kind = probe(path)?;
    let supported = supported(kind);
    let mut edits = Vec::with_capacity(changes.len());
    for (name, value) in changes {
        let value = value.as_deref().filter(|v| !v.is_empty());
        let Some(&field) = supported.iter().find(|f| **f == name.as_str()) else {
            if value.is_some() {
                return Err(Error::UnsupportedField(name.clone()));
            }
            continue;
        };
        if let Some(v) = value
            && field == "rating"
            && rating::parse_stars(v).is_none()
        {
            return Err(Error::InvalidValue {
                field: name.clone(),
                value: v.to_string(),
            });
        }
        edits.push((field, value));
    }
    if edits.is_empty() && cover.is_none() {
        return Ok(());
    }
    if std::fs::metadata(path)?.permissions().readonly() {
        return Err(Error::ReadOnly);
    }
    let mut tags = Tags::load(path, kind, true)?;
    let v24 = tags.apply(&edits, cover, options);
    tags.save(path, v24)
}
