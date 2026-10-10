//! Decoder for local files using Symphonia: MPEG audio, WAV/AIFF, Vorbis, FLAC, AAC/MP4, ALAC,
//! CAF. Symphonia trims encoder delay and padding, so album tracks join gaplessly (09 §2).

use std::collections::VecDeque;
use std::fs::File;

use symphonia::core::codecs::audio::AudioCodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::Track;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTag};
use symphonia::core::units::{Time, TimeBase};

use crate::{DecodeError, Decoder, DecoderFactory, ReadStatus, StreamInfo};

pub struct SymphoniaFactory;

const EXTENSIONS: &[(&str, &str)] = &[
    ("mp3;mp2;mp1;mpa", "MPEG Audio Files"),
    (
        "wav;wave;aif;aiff;aifc;caf;w64;rf64",
        "Uncompressed Audio Files",
    ),
    ("ogg;oga", "Ogg Vorbis Files"),
    ("flac", "FLAC Files"),
    ("m4a;mp4;m4b;m4r;aac;adts", "MPEG-4 Audio Files"),
    ("mka;webm", "Matroska Audio Files"),
];

impl DecoderFactory for SymphoniaFactory {
    fn name(&self) -> &str {
        "Symphonia"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        EXTENSIONS
            .iter()
            .map(|(e, d)| (e.to_string(), d.to_string()))
            .collect()
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        let path = local_path(location).ok_or_else(|| DecodeError::Unsupported(location.into()))?;
        let file = File::open(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DecodeError::NotFound(location.into()),
            _ => DecodeError::Open(e.to_string()),
        })?;
        let mut hint = Hint::new();
        if let Some(ext) = crate::extension_of(&path) {
            hint.with_extension(&ext);
        }
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| DecodeError::Open(e.to_string()))?;
        Ok(Box::new(SymphoniaDecoder::new(format)?))
    }
}

/// Converts a `file://` URI or a plain path to a local path; `None` for other URLs.
pub fn local_path(location: &str) -> Option<String> {
    if let Some(rest) = location.strip_prefix("file://") {
        return Some(percent_decode(
            rest.strip_prefix("localhost").unwrap_or(rest),
        ));
    }
    if location.contains("://") {
        return None;
    }
    Some(location.to_string())
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub struct SymphoniaDecoder {
    format: Box<dyn FormatReader + 'static>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    info: StreamInfo,
    /// (packet bytes, packet duration in seconds) for the bit-rate window.
    recent: VecDeque<(usize, f64)>,
    scratch: Vec<f32>,
    /// Frames to drop after an accurate seek landed before the requested time.
    skip_frames: u64,
    seek_mode: SeekMode,
    /// Title of a new chained Ogg stream, not yet taken.
    chain_title: Option<String>,
}

fn codec_name(id: symphonia::core::codecs::audio::AudioCodecId) -> (String, bool) {
    let name = symphonia::default::get_codecs()
        .get_audio_decoder(id)
        .map(|d| d.codec.info.long_name.to_string())
        .unwrap_or_else(|| "Audio".into());
    let lower = name.to_lowercase();
    let lossless = lower.contains("pcm")
        || lower.contains("flac")
        || lower.contains("alac")
        || lower.contains("lossless");
    (name, lossless)
}

/// The default audio track of `format`, its parameters and a decoder for them.
fn open_track(
    format: &dyn FormatReader,
) -> Result<(Track, AudioCodecParameters, Box<dyn AudioDecoder>), DecodeError> {
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| DecodeError::Open("no audio track".into()))?
        .clone();
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| DecodeError::Open("no audio codec parameters".into()))?
        .clone();
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| DecodeError::Unsupported(e.to_string()))?;
    Ok((track, params, decoder))
}

impl SymphoniaDecoder {
    pub(crate) fn new(format: Box<dyn FormatReader + 'static>) -> Result<Self, DecodeError> {
        let (track, params, decoder) = open_track(format.as_ref())?;
        let sample_rate = params.sample_rate.unwrap_or(44_100);
        let channels = params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .unwrap_or(2);
        let length_ms = match (track.num_frames, track.duration, track.time_base) {
            (Some(n), _, _) if sample_rate > 0 => Some(n * 1000 / sample_rate as u64),
            (_, Some(d), Some(tb)) => tb
                .calc_time(symphonia::core::units::Timestamp::from(d.get() as i64))
                .map(|t| (t.as_secs_f64() * 1000.0) as u64),
            _ => None,
        };
        let (codec, lossless) = codec_name(params.codec);
        let info = StreamInfo {
            sample_rate,
            channels,
            bits: params.bits_per_sample.unwrap_or(0) as u16,
            length_ms,
            seekable: length_ms.is_some(),
            codec,
            lossless,
        };
        Ok(Self {
            format,
            decoder,
            track_id: track.id,
            time_base: track.time_base,
            info,
            recent: VecDeque::new(),
            scratch: Vec::new(),
            skip_frames: 0,
            seek_mode: SeekMode::Accurate,
            chain_title: None,
        })
    }

    /// Seeks by a byte position estimate instead of parsing from the start (network streams).
    pub(crate) fn with_coarse_seek(mut self) -> Self {
        self.seek_mode = SeekMode::Coarse;
        self
    }

    /// A chained Ogg stream started a new logical stream: select its track and take its title
    /// from the newest metadata.
    fn restart_track(&mut self) -> Result<(), DecodeError> {
        let (track, params, decoder) = open_track(self.format.as_ref())?;
        self.decoder = decoder;
        self.track_id = track.id;
        self.time_base = track.time_base;
        if let Some(rate) = params.sample_rate {
            self.info.sample_rate = rate;
        }
        if let Some(ch) = &params.channels {
            self.info.channels = ch.count() as u16;
        }
        self.recent.clear();
        let mut meta = self.format.metadata();
        if let Some(rev) = meta.skip_to_latest() {
            let tags = rev
                .media
                .tags
                .iter()
                .chain(rev.per_track.iter().flat_map(|t| t.metadata.tags.iter()));
            let (mut artist, mut title) = (None, None);
            for tag in tags {
                match &tag.std {
                    Some(StandardTag::Artist(a)) if artist.is_none() => artist = Some(a.clone()),
                    Some(StandardTag::TrackTitle(t)) if title.is_none() => title = Some(t.clone()),
                    _ => {}
                }
            }
            self.chain_title = match (artist, title) {
                (Some(a), Some(t)) => Some(format!("{a} - {t}")),
                (None, Some(t)) => Some(t.to_string()),
                _ => None,
            };
        }
        Ok(())
    }

    fn packet_seconds(&self, dur: u64) -> f64 {
        match self.time_base {
            Some(tb) => tb
                .calc_time(symphonia::core::units::Timestamp::from(dur as i64))
                .map(|t| t.as_secs_f64())
                .unwrap_or(0.0),
            None => dur as f64 / self.info.sample_rate.max(1) as f64,
        }
    }
}

impl Decoder for SymphoniaDecoder {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(ReadStatus::End),
                Err(SymError::ResetRequired) => {
                    self.restart_track()?;
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(ReadStatus::End);
                }
                Err(e) => return Err(DecodeError::Decode(e.to_string())),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let secs = self.packet_seconds(packet.dur.get());
            self.recent.push_back((packet.data.len(), secs));
            while self.recent.len() > 64 {
                self.recent.pop_front();
            }
            match self.decoder.decode(&packet) {
                Ok(buf) => {
                    let frames = buf.frames();
                    if frames == 0 {
                        continue;
                    }
                    let ch = buf.spec().channels().count().max(1);
                    let n = buf.samples_interleaved();
                    self.scratch.resize(n, 0.0);
                    buf.copy_to_slice_interleaved(&mut self.scratch[..]);
                    let skip = (self.skip_frames as usize).min(frames);
                    self.skip_frames -= skip as u64;
                    if skip == frames {
                        continue;
                    }
                    self.scratch.drain(..skip * ch);
                    let frames = frames - skip;
                    // Keep a constant channel count (the one announced at open).
                    let want = self.info.channels as usize;
                    if ch == want {
                        out.extend_from_slice(&self.scratch);
                    } else {
                        for f in 0..frames {
                            for c in 0..want {
                                out.push(self.scratch[f * ch + c.min(ch - 1)]);
                            }
                        }
                    }
                    return Ok(ReadStatus::Frames(frames));
                }
                Err(SymError::DecodeError(_)) => continue,
                Err(e) => return Err(DecodeError::Decode(e.to_string())),
            }
        }
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        let time = Time::try_new((ms / 1000) as i64, ((ms % 1000) * 1_000_000) as u32)
            .ok_or_else(|| DecodeError::Seek("time out of range".into()))?;
        let seeked = self
            .format
            .seek(
                self.seek_mode,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| DecodeError::Seek(e.to_string()))?;
        self.decoder.reset();
        self.recent.clear();
        let to_ms = |ts| match self.time_base {
            Some(tb) => tb.calc_time(ts).map(|t: Time| t.as_secs_f64() * 1000.0),
            None => None,
        };
        let (Some(actual), Some(required)) = (to_ms(seeked.actual_ts), to_ms(seeked.required_ts))
        else {
            return Ok(ms);
        };
        if required > actual {
            self.skip_frames =
                ((required - actual) * self.info.sample_rate as f64 / 1000.0).round() as u64;
            return Ok(required as u64);
        }
        Ok(actual as u64)
    }

    fn kbps(&self) -> Option<u32> {
        let (bytes, secs) = self
            .recent
            .iter()
            .fold((0usize, 0f64), |(b, s), (pb, ps)| (b + pb, s + ps));
        (secs > 0.0).then(|| ((bytes as f64 * 8.0 / secs) / 1000.0).round() as u32)
    }

    fn take_title(&mut self) -> Option<String> {
        self.chain_title.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a 16-bit stereo WAV with a sine wave.
    pub(crate) fn write_wav(path: &std::path::Path, rate: u32, secs: f32) {
        let frames = (rate as f32 * secs) as u32;
        let mut data = Vec::new();
        for i in 0..frames {
            let v =
                ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 16000.0) as i16;
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut f = Vec::new();
        f.extend_from_slice(b"RIFF");
        f.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        f.extend_from_slice(b"WAVEfmt ");
        f.extend_from_slice(&16u32.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&2u16.to_le_bytes());
        f.extend_from_slice(&rate.to_le_bytes());
        f.extend_from_slice(&(rate * 4).to_le_bytes());
        f.extend_from_slice(&4u16.to_le_bytes());
        f.extend_from_slice(&16u16.to_le_bytes());
        f.extend_from_slice(b"data");
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(&data);
        std::fs::write(path, f).unwrap();
    }

    #[test]
    fn decodes_and_seeks_wav() {
        let dir = std::env::temp_dir().join(format!("nulamp-dec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        write_wav(&path, 44_100, 2.0);
        let mut d = SymphoniaFactory.open(path.to_str().unwrap()).unwrap();
        assert_eq!(d.info().sample_rate, 44_100);
        assert_eq!(d.info().channels, 2);
        assert_eq!(d.info().length_ms, Some(2000));
        assert!(d.info().seekable);
        let mut out = Vec::new();
        let mut frames = 0;
        while let ReadStatus::Frames(n) = d.read(&mut out).unwrap() {
            frames += n;
        }
        assert_eq!(frames, 88_200);
        assert_eq!(out.len(), 88_200 * 2);
        assert!(out.iter().any(|s| s.abs() > 0.4));
        let pos = d.seek(1000).unwrap();
        assert!((990..=1010).contains(&pos), "{pos}");
        let mut rest = 0;
        let mut out = Vec::new();
        while let ReadStatus::Frames(n) = d.read(&mut out).unwrap() {
            rest += n;
        }
        assert!((44_000..=44_200).contains(&rest), "{rest}");
        assert!(d.kbps().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file() {
        assert!(matches!(
            SymphoniaFactory.open("/no/such/file.mp3"),
            Err(DecodeError::NotFound(_))
        ));
    }

    #[test]
    fn file_uri() {
        assert_eq!(
            local_path("file:///a/b%20c.mp3").as_deref(),
            Some("/a/b c.mp3")
        );
        assert_eq!(local_path("http://x/y").as_deref(), None);
    }
}
