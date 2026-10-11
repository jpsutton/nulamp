//! Decoder for formats the built-in decoders lack (09 §1): Windows Media Audio, WavPack,
//! Monkey's Audio, Musepack and TTA, and the sound of video files (silence for a video without
//! audio), through the system FFmpeg libraries. It is also the
//! fallback for local files another decoder fails to open (for example MP4 files Symphonia
//! cannot read), see [`crate::Registry::open`].
//!
//! The FFmpeg calls live in a small C shim (`ffmpeg/shim.c`) built against the installed
//! headers, so this side only sees opaque handles and plain structs. Output keeps the source
//! sample rate and channel count; the engine resamples.

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr::NonNull;

use crate::symphonia_decoder::local_path;
use crate::{DecodeError, Decoder, DecoderFactory, FileTags, ReadStatus, StreamInfo};

mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct Nlff {
        _private: [u8; 0],
    }

    /// Mirrors `NlffInfo` in `shim.c`.
    #[repr(C)]
    pub struct NlffInfo {
        pub sample_rate: i32,
        pub channels: i32,
        pub bits: i32,
        pub lossless: i32,
        pub seekable: i32,
        pub length_frames: i64,
        pub bit_rate: i64,
        pub codec: [c_char; 128],
    }

    unsafe extern "C" {
        pub fn nlff_open(
            path: *const c_char,
            decode: c_int,
            info: *mut NlffInfo,
            err: *mut c_char,
            err_len: c_int,
        ) -> *mut Nlff;
        pub fn nlff_close(h: *mut Nlff);
        pub fn nlff_read(
            h: *mut Nlff,
            data: *mut *const f32,
            channels: *mut c_int,
            bytes: *mut i64,
        ) -> c_int;
        pub fn nlff_seek(h: *mut Nlff, target: i64) -> c_int;
        pub fn nlff_tag_next(
            h: *mut Nlff,
            which: c_int,
            iter: *mut *const c_void,
            key: *mut *const c_char,
            value: *mut *const c_char,
        ) -> c_int;
        pub fn nlff_license() -> *const c_char;
    }
}

const EXTENSIONS: &[(&str, &str)] = &[
    ("wma;asf", "Windows Media Audio Files"),
    ("wv", "WavPack Files"),
    ("ape", "Monkey's Audio Files"),
    ("mpc;mp+;mpp", "Musepack Files"),
    ("tta", "TTA Files"),
    // Video files the built-in decoders do not claim (09 §1); their picture is shown by
    // `crate::video`.
    (
        "avi;mkv;m4v;mov;flv;nsv;wmv;mpg;mpeg;m2v;ogv",
        "Video Files",
    ),
];

/// The licence of the libavcodec in use, for example "LGPL version 2.1 or later".
pub fn license() -> String {
    // SAFETY: returns a static NUL-terminated string.
    unsafe { CStr::from_ptr(ffi::nlff_license()) }
        .to_string_lossy()
        .into_owned()
}

/// Owned shim handle.
struct Handle(NonNull<ffi::Nlff>);

// SAFETY: a handle is only used through `&mut self` or `&self` without shared mutation; FFmpeg
// contexts have no thread affinity.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: created by nlff_open and closed once.
        unsafe { ffi::nlff_close(self.0.as_ptr()) };
    }
}

impl Handle {
    fn open(location: &str, decode: bool) -> Result<(Self, StreamInfo, Option<u32>), DecodeError> {
        let path = local_path(location).ok_or_else(|| DecodeError::Unsupported(location.into()))?;
        if !std::path::Path::new(&path).is_file() {
            return Err(DecodeError::NotFound(location.into()));
        }
        let c_path = CString::new(path).map_err(|e| DecodeError::Open(e.to_string()))?;
        // SAFETY: plain-data struct; the shim fills it in.
        let mut info: ffi::NlffInfo = unsafe { std::mem::zeroed() };
        let mut err = [0 as c_char; 256];
        // SAFETY: valid NUL-terminated path and out buffers of the given sizes.
        let h = unsafe {
            ffi::nlff_open(
                c_path.as_ptr(),
                decode as c_int,
                &mut info,
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        let Some(h) = NonNull::new(h) else {
            // SAFETY: the shim writes a NUL-terminated message (the buffer starts zeroed).
            let msg = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy();
            return Err(DecodeError::Open(msg.into_owned()));
        };
        // SAFETY: `codec` is NUL-terminated by the shim's snprintf.
        let codec = unsafe { CStr::from_ptr(info.codec.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let rate = info.sample_rate.max(1) as u32;
        let length_ms = u64::try_from(info.length_frames)
            .ok()
            .map(|n| n * 1000 / rate as u64);
        let stream = StreamInfo {
            sample_rate: rate,
            channels: info.channels.clamp(1, u16::MAX as i32) as u16,
            bits: info.bits.clamp(0, u16::MAX as i32) as u16,
            length_ms,
            seekable: info.seekable != 0,
            codec,
            lossless: info.lossless != 0,
        };
        let kbps = u32::try_from((info.bit_rate + 500) / 1000)
            .ok()
            .filter(|k| *k > 0);
        Ok((Self(h), stream, kbps))
    }

    fn tags(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for which in 0..2 {
            let mut iter = std::ptr::null();
            let (mut key, mut value) = (std::ptr::null(), std::ptr::null());
            // SAFETY: valid handle and out pointers; the strings live as long as the handle and
            // are copied at once.
            while unsafe {
                ffi::nlff_tag_next(self.0.as_ptr(), which, &mut iter, &mut key, &mut value)
            } == 1
            {
                // SAFETY: FFmpeg dictionary keys and values are NUL-terminated.
                let (k, v) = unsafe { (CStr::from_ptr(key), CStr::from_ptr(value)) };
                out.push((
                    k.to_string_lossy().into_owned(),
                    v.to_string_lossy().into_owned(),
                ));
            }
        }
        out
    }
}

/// FFmpeg decoder factory.
pub struct FfmpegFactory;

impl DecoderFactory for FfmpegFactory {
    fn name(&self) -> &str {
        "FFmpeg"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        EXTENSIONS
            .iter()
            .map(|(e, d)| (e.to_string(), d.to_string()))
            .collect()
    }

    fn fallback(&self) -> bool {
        true
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        let (handle, info, kbps) = Handle::open(location, true)?;
        Ok(Box::new(FfmpegDecoder {
            handle,
            info,
            file_kbps: kbps,
            recent: VecDeque::new(),
            pos: 0,
        }))
    }
}

/// Tags and stream information of a local file, read by FFmpeg (see [`crate::ffmpeg_tags`]).
pub(crate) fn read_tags(location: &str) -> Option<FileTags> {
    let (handle, info, kbps) = Handle::open(location, false).ok()?;
    let mut fields: Vec<(String, String)> = Vec::new();
    for (key, value) in handle.tags() {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let Some((field, value)) = map_tag(&key, value) else {
            continue;
        };
        // The first entry wins (container tags come before stream tags).
        if !fields.iter().any(|(f, _)| f == field) {
            fields.push((field.to_string(), value));
        }
    }
    Some(FileTags { fields, info, kbps })
}

/// nulamp field for a tag key as FFmpeg reports it: FFmpeg's generic names, raw ASF attribute
/// names and APEv2/Vorbis keys (10 §7.2). Keys compare case-insensitively.
fn map_tag(key: &str, value: &str) -> Option<(&'static str, String)> {
    const KEYS: &[(&str, &[&str])] = &[
        ("title", &["title"]),
        ("artist", &["artist", "author"]),
        ("album", &["album", "wm/albumtitle"]),
        (
            "albumartist",
            &[
                "album_artist",
                "album artist",
                "albumartist",
                "wm/albumartist",
            ],
        ),
        ("year", &["date", "year", "wm/year"]),
        ("genre", &["genre", "wm/genre"]),
        ("comment", &["comment", "description"]),
        ("track", &["track", "tracknumber", "wm/tracknumber"]),
        ("disc", &["disc", "discnumber", "wm/partofset"]),
        ("composer", &["composer", "wm/composer"]),
        ("publisher", &["publisher", "organization", "wm/publisher"]),
        ("bpm", &["bpm", "wm/beatsperminute"]),
        ("conductor", &["conductor", "wm/conductor"]),
        ("lyricist", &["lyricist", "wm/writer"]),
        ("key", &["key", "initialkey", "wm/initialkey"]),
        ("mood", &["mood", "wm/mood"]),
        ("subtitle", &["subtitle", "wm/subtitle"]),
        ("category", &["grouping", "wm/category"]),
        ("isrc", &["isrc", "wm/isrc"]),
        ("encoder", &["encoder", "wm/encodingsettings"]),
        ("tool", &["encoded_by", "encoded-by", "wm/toolname"]),
        ("copyright", &["copyright"]),
        ("compilation", &["compilation"]),
        ("replaygain_track_gain", &["replaygain_track_gain"]),
        ("replaygain_track_peak", &["replaygain_track_peak"]),
        ("replaygain_album_gain", &["replaygain_album_gain"]),
        ("replaygain_album_peak", &["replaygain_album_peak"]),
    ];
    let key = key.to_ascii_lowercase();
    if key == "rating" {
        // APEv2 `Rating` holds 0–5 itself.
        return value
            .parse::<u8>()
            .ok()
            .filter(|r| (1..=5).contains(r))
            .map(|r| ("rating", r.to_string()));
    }
    if key == "wm/shareduserrating" {
        // 1, 25, 50, 75, 99 → 1–5 stars.
        let stars = match value.parse::<u32>().ok()? {
            0 => return None,
            1..=12 => 1,
            13..=37 => 2,
            38..=62 => 3,
            63..=86 => 4,
            _ => 5,
        };
        return Some(("rating", stars.to_string()));
    }
    KEYS.iter()
        .find(|(_, aliases)| aliases.contains(&key.as_str()))
        .map(|(field, _)| (*field, value.to_string()))
}

/// An open FFmpeg stream.
pub struct FfmpegDecoder {
    handle: Handle,
    info: StreamInfo,
    /// Container bit rate, used until packets have been read.
    file_kbps: Option<u32>,
    /// (packet bytes, frames) of recent blocks for the bit-rate window.
    recent: VecDeque<(u64, u64)>,
    /// Sample frame position of the next block.
    pos: u64,
}

impl Decoder for FfmpegDecoder {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError> {
        let mut data = std::ptr::null();
        let mut ch: c_int = 0;
        let mut bytes = 0i64;
        // SAFETY: valid handle and out pointers.
        let n = unsafe { ffi::nlff_read(self.handle.0.as_ptr(), &mut data, &mut ch, &mut bytes) };
        if n < 0 {
            return Err(DecodeError::Decode(format!("FFmpeg error {n}")));
        }
        if n == 0 || data.is_null() || ch <= 0 {
            return Ok(ReadStatus::End);
        }
        let (frames, ch) = (n as usize, ch as usize);
        // SAFETY: the shim returns `frames × ch` samples valid until the next call.
        let block = unsafe { std::slice::from_raw_parts(data, frames * ch) };
        // Keep the channel count announced at open.
        let want = self.info.channels as usize;
        if ch == want {
            out.extend_from_slice(block);
        } else {
            out.reserve(frames * want);
            for f in block.chunks_exact(ch) {
                out.extend((0..want).map(|c| f[c.min(ch - 1)]));
            }
        }
        self.pos += frames as u64;
        self.recent.push_back((bytes.max(0) as u64, frames as u64));
        while self.recent.len() > 64 {
            self.recent.pop_front();
        }
        Ok(ReadStatus::Frames(frames))
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        let rate = self.info.sample_rate as u64;
        let mut target = ms.saturating_mul(rate) / 1000;
        if let Some(len) = self.info.length_ms {
            target = target.min(len * rate / 1000);
        }
        // SAFETY: valid handle.
        let r = unsafe { ffi::nlff_seek(self.handle.0.as_ptr(), target as i64) };
        if r < 0 {
            return Err(DecodeError::Seek(format!("FFmpeg error {r}")));
        }
        self.pos = target;
        self.recent.clear();
        Ok((target * 1000 + rate / 2) / rate.max(1))
    }

    fn kbps(&self) -> Option<u32> {
        let (bytes, frames) = self
            .recent
            .iter()
            .fold((0u64, 0u64), |(b, f), (pb, pf)| (b + pb, f + pf));
        if bytes == 0 || frames == 0 {
            return self.file_kbps;
        }
        let secs = frames as f64 / self.info.sample_rate.max(1) as f64;
        Some((bytes as f64 * 8.0 / secs / 1000.0).round() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Registry;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const RATE: u32 = 44_100;
    const SECS: u32 = 3;

    /// A generated file, removed when dropped.
    struct Fixture(PathBuf);

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    impl Fixture {
        fn path(&self) -> &Path {
            &self.0
        }

        fn location(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    /// Encodes a 3 s stereo 440 Hz sine (amplitude 1/8, the same on both channels) with the
    /// `ffmpeg` tool; `None` (test skipped) when the tool or the encoder is missing.
    fn fixture(name: &str, args: &[&str]) -> Option<Fixture> {
        let path =
            std::env::temp_dir().join(format!("nulamp-ffmpeg-{}-{name}", std::process::id()));
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(format!(
                "sine=frequency=440:sample_rate={RATE}:duration={SECS}"
            ))
            .args(["-af", "pan=stereo|c0=c0|c1=c0"])
            .args(args)
            .arg(&path)
            .status();
        match status {
            Ok(s) if s.success() => Some(Fixture(path)),
            _ => {
                eprintln!("skipping: ffmpeg cannot write {name}");
                None
            }
        }
    }

    fn decode_all(d: &mut dyn Decoder) -> Vec<f32> {
        let mut out = Vec::new();
        while let ReadStatus::Frames(_) = d.read(&mut out).unwrap() {}
        out
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0, |m, v| m.max(v.abs()))
    }

    /// The sample the fixture's sine has at `frame`.
    fn sine_at(frame: u64) -> f32 {
        // lavfi's `sine` source has amplitude 1/8.
        (frame as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin() as f32 / 8.0
    }

    /// Checks length, sample count (within `tolerance_frames` of the source), non-silence and
    /// seeking. Lossless files must continue sample-exactly after a seek; lossy ones (with
    /// millisecond container timestamps) within 64 frames.
    fn check(path: &Path, lossless: bool, tolerance_frames: usize) {
        let total = (RATE * SECS) as usize;
        let mut d = FfmpegFactory.open(path.to_str().unwrap()).unwrap();
        let info = d.info().clone();
        assert_eq!(info.sample_rate, RATE);
        assert_eq!(info.channels, 2);
        assert_eq!(info.lossless, lossless, "{}", info.codec);
        assert!(info.seekable);
        let len = info.length_ms.unwrap();
        assert!(len.abs_diff(3000) <= 50, "length {len}");
        let all = decode_all(d.as_mut());
        let frames = all.len() / 2;
        assert!(
            frames.abs_diff(total) <= tolerance_frames,
            "{frames} frames, {}",
            info.codec
        );
        assert!(
            peak(&all) > 0.1,
            "silence: peak {} of {} frames",
            peak(&all),
            frames
        );
        assert!(d.kbps().is_some_and(|k| k > 0));

        // Seek to an exact frame: the next block continues the decoded stream exactly.
        let target_ms = 1234;
        let target = (target_ms * RATE as u64 / 1000) as usize;
        assert_eq!(d.seek(target_ms).unwrap(), target_ms);
        let rest = decode_all(d.as_mut());
        let left = rest.len() / 2;
        if lossless {
            assert_eq!(left, frames - target);
            assert_eq!(rest[..], all[target * 2..]);
            assert!((rest[0] - sine_at(target as u64)).abs() < 1e-3);
        } else {
            // Container timestamps are in milliseconds.
            assert!(left.abs_diff(frames - target) <= 64, "{left} after seek");
            assert!(peak(&rest[..4096]) > 0.1);
        }

        // Back to the start, and past the end.
        assert_eq!(d.seek(0).unwrap(), 0);
        let again = decode_all(d.as_mut());
        if lossless {
            assert_eq!(again, all);
        } else {
            assert!((again.len() / 2).abs_diff(frames) <= 64);
        }
        assert!(d.seek(60_000).unwrap() <= len);
        assert!(decode_all(d.as_mut()).len() <= 2 * 4096);
    }

    #[test]
    fn wavpack() {
        if let Some(p) = fixture("tone.wv", &["-c:a", "wavpack"]) {
            check(p.path(), true, 0);
            let d = FfmpegFactory.open(p.location()).unwrap();
            assert_eq!(d.info().bits, 16);
            assert!(d.info().codec.contains("WavPack"), "{}", d.info().codec);
        }
    }

    #[test]
    fn tta() {
        let args = [
            "-c:a",
            "tta",
            "-metadata",
            "title=Tone",
            "-metadata",
            "date=2001",
        ];
        if let Some(p) = fixture("tone.tta", &args) {
            check(p.path(), true, 0);
            // APEv2 tag, which the tag library does not read in TTA files.
            let tags = read_tags(p.location()).unwrap();
            assert!(tags.fields.contains(&("title".into(), "Tone".into())));
            assert!(tags.fields.contains(&("year".into(), "2001".into())));
            assert!(tags.info.lossless);
            assert_eq!(tags.info.bits, 16);
        }
    }

    #[test]
    fn wma() {
        let args = [
            "-c:a",
            "wmav2",
            "-b:a",
            "128k",
            "-metadata",
            "title=Tone",
            "-metadata",
            "artist=Sine",
            "-metadata",
            "album=Tests",
            "-metadata",
            "track=3",
            "-metadata",
            "album_artist=Band",
            "-metadata",
            "date=1999",
            "-metadata",
            "disc=1/2",
        ];
        if let Some(p) = fixture("tone.wma", &args) {
            // WMA frames are 2048 samples; the first frames are decoder delay.
            check(p.path(), false, 4096);
            let tags = read_tags(p.location()).unwrap();
            let get = |k: &str| {
                tags.fields
                    .iter()
                    .find(|(f, _)| f == k)
                    .map(|(_, v)| v.as_str())
            };
            assert_eq!(get("title"), Some("Tone"));
            assert_eq!(get("artist"), Some("Sine"));
            assert_eq!(get("album"), Some("Tests"));
            assert_eq!(get("track"), Some("3"));
            assert_eq!(get("albumartist"), Some("Band"));
            assert_eq!(get("year"), Some("1999"));
            assert_eq!(get("disc"), Some("1/2"));
            assert!(tags.info.codec.contains("Windows Media Audio"));
            assert!(tags.kbps.is_some());
        }
    }

    #[test]
    fn registry_order_and_fallback() {
        let r = Registry::with_defaults();
        for ext in ["wma", "asf", "wv", "ape", "mpc", "tta"] {
            assert!(r.is_supported_extension(ext), "{ext}");
            assert_eq!(
                r.select(&format!("/a/b.{ext}")).map(|f| f.name()),
                Some("FFmpeg")
            );
        }
        // Built-in decoders keep their own extensions.
        assert_eq!(r.select("/a/b.m4a").map(|f| f.name()), Some("Symphonia"));
        assert_eq!(r.select("/a/b.opus").map(|f| f.name()), Some("Opus"));
        // A WavPack file named .mp3: Symphonia fails, FFmpeg opens it.
        if let Some(p) = fixture("disguised.mp3", &["-c:a", "wavpack", "-f", "wv"]) {
            let d = r.open(p.location()).unwrap();
            assert!(d.info().codec.contains("WavPack"));
        }
        assert!(matches!(
            r.open("/no/such/file.wv"),
            Err(DecodeError::NotFound(_))
        ));
    }

    #[test]
    fn tag_mapping() {
        assert_eq!(
            map_tag("WM/AlbumArtist", "X"),
            Some(("albumartist", "X".into()))
        );
        assert_eq!(
            map_tag("Album Artist", "X"),
            Some(("albumartist", "X".into()))
        );
        assert_eq!(map_tag("date", "1999"), Some(("year", "1999".into())));
        assert_eq!(
            map_tag("WM/SharedUserRating", "75"),
            Some(("rating", "4".into()))
        );
        assert_eq!(map_tag("WM/SharedUserRating", "0"), None);
        assert_eq!(map_tag("Rating", "3"), Some(("rating", "3".into())));
        assert_eq!(map_tag("unknown", "x"), None);
    }

    #[test]
    fn licence_is_reported() {
        assert!(license().contains("GPL"));
    }
}
