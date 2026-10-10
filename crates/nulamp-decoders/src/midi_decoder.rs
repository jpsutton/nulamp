//! MIDI decoder (09 §6): standard MIDI files of types 0, 1 and 2, RIFF RMID and KAR, rendered
//! to PCM with the `rustysynth` SoundFont synthesizer. The file parser and the sequencer are
//! our own: they build a tempo-mapped event timeline, which gives the length, the loop points
//! (`loopStart`/`loopEnd` text or marker events, CC 111) and fast seeking without rendering.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use crate::symphonia_decoder::local_path;
use crate::{DecodeError, Decoder, DecoderFactory, ReadStatus, StreamInfo};

/// Environment variable naming a SoundFont, consulted after [`MidiOptions::soundfont`].
pub const SOUNDFONT_ENV: &str = "NULAMP_SOUNDFONT";

/// Frames rendered per `read` call at most.
const BLOCK: usize = 1024;

/// When the decoder loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    #[default]
    Never,
    /// Loop only files with a loop start marker.
    WhenDetected,
    /// Loop every file: from the loop start marker if any, else the whole song.
    Always,
}

/// MIDI options (09 §6).
#[derive(Debug, Clone, PartialEq)]
pub struct MidiOptions {
    /// Synthesizer sample rate, 16000..=192000.
    pub sample_rate: u32,
    pub reverb_chorus: bool,
    /// SoundFont (.sf2) to use. When unset, `NULAMP_SOUNDFONT` and then the system paths of
    /// [`soundfont_search_paths`] are searched.
    pub soundfont: Option<PathBuf>,
    pub loop_mode: LoopMode,
    /// Times the loop section repeats; `None` repeats forever.
    pub loop_count: Option<u32>,
    /// Silence rendered after the end, in milliseconds (release tails sound during it).
    pub end_silence_ms: u32,
    /// Keep the events read before a damaged or truncated part of a track.
    pub recover_incomplete: bool,
}

impl Default for MidiOptions {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            reverb_chorus: true,
            soundfont: None,
            loop_mode: LoopMode::Never,
            loop_count: Some(1),
            end_silence_ms: 0,
            recover_incomplete: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct MidiFactory {
    pub options: MidiOptions,
}

impl DecoderFactory for MidiFactory {
    fn name(&self) -> &str {
        "MIDI"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        vec![("mid;midi;rmi;kar".into(), "MIDI Files".into())]
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        Ok(Box::new(MidiDecoder::open(location, &self.options)?))
    }
}

/// Directories searched for SoundFonts, in order.
pub fn soundfont_search_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("nulamp").join("soundfonts"));
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            dirs.push(PathBuf::from(profile).join("soundfonts"));
        }
        dirs.push(PathBuf::from(r"C:\soundfonts"));
    } else if cfg!(target_os = "macos") {
        if let Some(home) = &home {
            dirs.push(home.join("Library/Audio/Sounds/Banks"));
        }
        dirs.push(PathBuf::from("/Library/Audio/Sounds/Banks"));
        dirs.push(PathBuf::from("/opt/homebrew/share/soundfonts"));
        dirs.push(PathBuf::from("/usr/local/share/soundfonts"));
    } else {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        if let Some(d) = data_home {
            dirs.push(d.join("soundfonts"));
        }
        for d in [
            "/usr/share/soundfonts",
            "/usr/share/sounds/sf2",
            "/usr/local/share/soundfonts",
            "/usr/share/sounds/sf2/fluid-soundfont",
        ] {
            dirs.push(PathBuf::from(d));
        }
    }
    dirs
}

/// Preferred file names inside a search directory; any other `.sf2` follows in name order.
const PREFERRED: &[&str] = &[
    "default.sf2",
    "FluidR3_GM.sf2",
    "default-GM.sf2",
    "FluidR3_GS.sf2",
    "TimGM6mb.sf2",
];

/// Finds the SoundFont to use: the explicit path, `NULAMP_SOUNDFONT`, then the search paths.
pub fn find_soundfont(explicit: Option<&Path>) -> Result<PathBuf, DecodeError> {
    if let Some(p) = explicit {
        return if p.is_file() {
            Ok(p.to_path_buf())
        } else {
            Err(DecodeError::Open(format!(
                "SoundFont not found: {}",
                p.display()
            )))
        };
    }
    if let Some(p) = std::env::var_os(SOUNDFONT_ENV).map(PathBuf::from) {
        if p.is_file() {
            return Ok(p);
        }
        log::warn!("{SOUNDFONT_ENV}={} is not a file", p.display());
    }
    for dir in soundfont_search_paths() {
        if let Some(p) = PREFERRED.iter().map(|n| dir.join(n)).find(|p| p.is_file()) {
            return Ok(p);
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sf2")) && p.is_file())
            .collect();
        found.sort();
        if let Some(p) = found.into_iter().next() {
            return Ok(p);
        }
    }
    Err(DecodeError::Open(format!(
        "no SoundFont found for MIDI playback: set the SoundFont option or {SOUNDFONT_ENV}, or \
         install a General MIDI SoundFont (for example FluidR3_GM.sf2) in {}",
        soundfont_search_paths()
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Loads a SoundFont, sharing it with decoders that still hold the same file.
fn load_soundfont(path: &Path) -> Result<Arc<SoundFont>, DecodeError> {
    static CACHE: Mutex<Option<(PathBuf, Weak<SoundFont>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, weak)) = cache.as_ref()
        && p == path
        && let Some(sf) = weak.upgrade()
    {
        return Ok(sf);
    }
    let file = File::open(path)
        .map_err(|e| DecodeError::Open(format!("SoundFont {}: {e}", path.display())))?;
    let sf = Arc::new(
        SoundFont::new(&mut BufReader::new(file))
            .map_err(|e| DecodeError::Open(format!("SoundFont {}: {e}", path.display())))?,
    );
    *cache = Some((path.to_path_buf(), Arc::downgrade(&sf)));
    Ok(sf)
}

/// A channel message at a point of the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Event {
    secs: f64,
    msg: [u8; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Msg([u8; 3]),
    Tempo(u32),
    LoopStart,
    LoopEnd,
    End,
}

#[derive(Debug, Default)]
struct Texts {
    track_name: Option<String>,
    first_text: Option<String>,
    kar_title: Option<String>,
    copyright: Option<String>,
}

/// Text of a meta event: UTF-8 when valid, else Latin-1.
fn decode_text(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
    .trim_matches(|c: char| c.is_whitespace() || c == '\0')
    .to_string()
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// Reads a variable-length quantity.
fn vlq(t: &[u8], pos: &mut usize) -> Option<u32> {
    let mut v = 0u32;
    for _ in 0..4 {
        let b = *t.get(*pos)?;
        *pos += 1;
        v = (v << 7) | (b & 0x7F) as u32;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// Parses one track chunk into (tick, event) pairs; returns them with the end tick.
fn parse_track(
    t: &[u8],
    first: bool,
    recover: bool,
    texts: &mut Texts,
) -> Result<(Vec<(u64, Kind)>, u64), String> {
    let mut events = Vec::new();
    let mut pos = 0;
    let mut tick = 0u64;
    let mut running: Option<u8> = None;
    let fail = |events: Vec<(u64, Kind)>, tick: u64, msg: &str| {
        if recover {
            Ok((events, tick))
        } else {
            Err(msg.to_string())
        }
    };
    loop {
        if pos >= t.len() {
            return fail(events, tick, "track ends without an end-of-track event");
        }
        let Some(delta) = vlq(t, &mut pos) else {
            return fail(events, tick, "truncated delta time");
        };
        tick += delta as u64;
        let Some(&b) = t.get(pos) else {
            return fail(events, tick, "truncated event");
        };
        match b {
            0xFF => {
                pos += 1;
                let Some(&kind) = t.get(pos) else {
                    return fail(events, tick, "truncated meta event");
                };
                pos += 1;
                let data = vlq(t, &mut pos)
                    .and_then(|len| t.get(pos..pos + len as usize))
                    .ok_or("truncated meta event");
                let data = match data {
                    Ok(d) => d,
                    Err(e) => return fail(events, tick, e),
                };
                pos += data.len();
                match kind {
                    0x2F => return Ok((events, tick)),
                    0x51 if data.len() >= 3 => {
                        let us = u32::from_be_bytes([0, data[0], data[1], data[2]]);
                        if us > 0 {
                            events.push((tick, Kind::Tempo(us)));
                        }
                    }
                    0x01 | 0x06 => {
                        let text = decode_text(data);
                        if text.eq_ignore_ascii_case("loopstart") {
                            events.push((tick, Kind::LoopStart));
                        } else if text.eq_ignore_ascii_case("loopend") {
                            events.push((tick, Kind::LoopEnd));
                        } else if kind == 0x01 {
                            if let Some(title) = text.strip_prefix("@T") {
                                if texts.kar_title.is_none() {
                                    texts.kar_title = non_empty(title.trim().to_string());
                                }
                            } else if first && texts.first_text.is_none() && !text.starts_with('@')
                            {
                                texts.first_text = non_empty(text);
                            }
                        }
                    }
                    0x02 if texts.copyright.is_none() => {
                        texts.copyright = non_empty(decode_text(data));
                    }
                    0x03 if first && texts.track_name.is_none() => {
                        texts.track_name = non_empty(decode_text(data));
                    }
                    _ => {}
                }
            }
            0xF0 | 0xF7 => {
                pos += 1;
                running = None;
                match vlq(t, &mut pos) {
                    Some(len) if pos + len as usize <= t.len() => pos += len as usize,
                    _ => return fail(events, tick, "truncated SysEx event"),
                }
            }
            _ => {
                let status = if b & 0x80 != 0 {
                    pos += 1;
                    if b >= 0xF0 {
                        if recover {
                            continue;
                        }
                        return Err(format!("unexpected status byte {b:#04x}"));
                    }
                    running = Some(b);
                    b
                } else if let Some(s) = running {
                    s
                } else {
                    if recover {
                        pos += 1;
                        continue;
                    }
                    return Err("data byte without running status".into());
                };
                let n = if matches!(status & 0xF0, 0xC0 | 0xD0) {
                    1
                } else {
                    2
                };
                let Some(data) = t.get(pos..pos + n) else {
                    return fail(events, tick, "truncated channel message");
                };
                pos += n;
                let msg = [status, data[0] & 0x7F, data.get(1).map_or(0, |d| d & 0x7F)];
                if status & 0xF0 == 0xB0 && msg[1] == 111 {
                    events.push((tick, Kind::LoopStart));
                }
                events.push((tick, Kind::Msg(msg)));
            }
        }
    }
}

/// Extracts the SMF data and the INFO title/copyright from a RIFF RMID file.
fn unwrap_rmid(data: &[u8]) -> (&[u8], Option<String>, Option<String>) {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"RMID" {
        return (data, None, None);
    }
    let (mut smf, mut title, mut copyright) = (None, None, None);
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap_or_default());
        let body = &data[pos + 8..(pos + 8 + len as usize).min(data.len())];
        if id == b"data" {
            smf.get_or_insert(body);
        } else if id == b"LIST" && body.get(..4) == Some(b"INFO") {
            let mut p = 4;
            while p + 8 <= body.len() {
                let sub = &body[p..p + 4];
                let n = u32::from_le_bytes(body[p + 4..p + 8].try_into().unwrap_or_default());
                let text = &body[p + 8..(p + 8 + n as usize).min(body.len())];
                match sub {
                    b"INAM" => title = non_empty(decode_text(text)),
                    b"ICOP" => copyright = non_empty(decode_text(text)),
                    _ => {}
                }
                p += 8 + n as usize + (n as usize & 1);
            }
        }
        pos += 8 + len as usize + (len as usize & 1);
    }
    (smf.unwrap_or(data), title, copyright)
}

fn find(data: &[u8], pat: &[u8]) -> Option<usize> {
    data.windows(pat.len()).position(|w| w == pat)
}

/// A parsed MIDI file: channel messages on a time line in seconds, plus metadata.
#[derive(Debug, Clone)]
pub struct MidiSong {
    /// SMF format (0, 1 or 2).
    pub format: u16,
    pub tracks: usize,
    events: Vec<Event>,
    duration: f64,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
    title: Option<String>,
    copyright: Option<String>,
}

impl MidiSong {
    pub fn from_file(path: &Path, recover: bool) -> Result<Self, DecodeError> {
        let data = std::fs::read(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DecodeError::NotFound(path.display().to_string()),
            _ => DecodeError::Open(e.to_string()),
        })?;
        Self::parse(&data, recover).map_err(|e| DecodeError::Open(format!("MIDI: {e}")))
    }

    /// Parses an SMF (optionally inside a RIFF RMID container).
    pub fn parse(data: &[u8], recover: bool) -> Result<Self, String> {
        let (smf, riff_title, riff_copyright) = unwrap_rmid(data);
        let start = find(smf, b"MThd").ok_or("not a MIDI file")?;
        let hdr = smf.get(start..start + 14).ok_or("truncated header")?;
        let hdr_len = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
        if hdr_len < 6 {
            return Err("invalid header".into());
        }
        let format = u16::from_be_bytes([hdr[8], hdr[9]]);
        let division = u16::from_be_bytes([hdr[12], hdr[13]]);
        let ticks_per_sec = if division & 0x8000 != 0 {
            let fps = match (division >> 8) as u8 as i8 {
                -29 => 29.97,
                f => -(f as f64),
            };
            Some(fps * (division & 0xFF) as f64)
        } else {
            None
        };
        if division == 0 || ticks_per_sec.is_some_and(|t| t <= 0.0) {
            return Err("invalid time division".into());
        }

        let mut chunks = Vec::new();
        let mut pos = start + 8 + hdr_len;
        while pos + 8 <= smf.len() {
            let len = u32::from_be_bytes(smf[pos + 4..pos + 8].try_into().unwrap_or_default());
            let body_start = pos + 8;
            let mut body_end = body_start.saturating_add(len as usize);
            if body_end > smf.len() {
                if !recover {
                    return Err("truncated track chunk".into());
                }
                body_end = smf.len();
            }
            if &smf[pos..pos + 4] == b"MTrk" {
                chunks.push(&smf[body_start..body_end]);
            }
            pos = body_end;
        }
        if chunks.is_empty() {
            return Err("no tracks".into());
        }

        let mut texts = Texts::default();
        let mut all: Vec<(u64, Kind)> = Vec::new();
        let mut offset = 0u64;
        for (i, chunk) in chunks.iter().enumerate() {
            let (events, end) = parse_track(chunk, i == 0, recover, &mut texts)?;
            all.extend(events.into_iter().map(|(t, k)| (t + offset, k)));
            all.push((end + offset, Kind::End));
            if format == 2 {
                offset += end;
            }
        }
        all.sort_by_key(|(t, _)| *t);

        let mut events = Vec::new();
        let (mut loop_start, mut loop_end) = (None, None);
        let mut duration = 0f64;
        let (mut tempo, mut cur_tick, mut cur_secs) = (500_000f64, 0u64, 0f64);
        for (tick, kind) in all {
            let secs = match ticks_per_sec {
                Some(tps) => tick as f64 / tps,
                None => cur_secs + (tick - cur_tick) as f64 * tempo / 1e6 / division as f64,
            };
            (cur_tick, cur_secs) = (tick, secs);
            match kind {
                Kind::Msg(msg) => events.push(Event { secs, msg }),
                Kind::Tempo(t) => tempo = t as f64,
                Kind::LoopStart => {
                    loop_start.get_or_insert(secs);
                }
                Kind::LoopEnd => {
                    loop_end.get_or_insert(secs);
                }
                Kind::End => {}
            }
            duration = duration.max(secs);
        }
        let loop_start = loop_start.filter(|&s| s < duration);
        let loop_end = loop_end.filter(|&e| loop_start.is_none_or(|s| e > s));

        Ok(Self {
            format,
            tracks: chunks.len(),
            events,
            duration,
            loop_start,
            loop_end,
            title: riff_title
                .or(texts.kar_title)
                .or(texts.track_name)
                .or(texts.first_text),
            copyright: riff_copyright.or(texts.copyright),
        })
    }

    /// Length of one pass in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        (self.duration * 1000.0).round() as u64
    }

    /// Loop start and end markers in milliseconds, when present.
    pub fn loop_markers_ms(&self) -> (Option<u64>, Option<u64>) {
        let ms = |s: f64| (s * 1000.0).round() as u64;
        (self.loop_start.map(ms), self.loop_end.map(ms))
    }

    /// Title: RMID `INAM`, KAR `@T`, the first track's name, or its first text event.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn copyright(&self) -> Option<&str> {
        self.copyright.as_deref()
    }
}

/// Playback plan in frames: the song, its loop section and the trailing silence.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Plan {
    end: u64,
    /// Loop section (start, end).
    section: Option<(u64, u64)>,
    /// Jumps back to the loop start; `None` = forever.
    jumps: Option<u32>,
    tail: u64,
}

impl Plan {
    fn new(song: &MidiSong, o: &MidiOptions) -> Self {
        let frames = |s: f64| (s * o.sample_rate as f64).round() as u64;
        let end = frames(song.duration);
        let marked = song
            .loop_start
            .map(|s| (frames(s), song.loop_end.map_or(end, frames)));
        let section = match o.loop_mode {
            LoopMode::Never => None,
            LoopMode::WhenDetected => marked,
            LoopMode::Always => Some(marked.unwrap_or((0, end))),
        }
        .filter(|(s, e)| e > s);
        Self {
            end,
            section,
            jumps: if section.is_some() {
                o.loop_count
            } else {
                Some(0)
            },
            tail: o.end_silence_ms as u64 * o.sample_rate as u64 / 1000,
        }
    }

    fn section_len(&self) -> u64 {
        self.section.map_or(0, |(s, e)| e - s)
    }

    /// Frames before the trailing silence; `None` when looping forever.
    fn body(&self) -> Option<u64> {
        self.jumps.map(|j| self.end + j as u64 * self.section_len())
    }

    fn total(&self) -> Option<u64> {
        self.body().map(|b| b + self.tail)
    }

    /// Maps an output frame to (timeline frame, jumps done, silence frames done).
    fn locate(&self, f: u64) -> (u64, u32, u64) {
        if let Some(body) = self.body()
            && f >= body
        {
            return (self.end, self.jumps.unwrap_or(0), (f - body).min(self.tail));
        }
        match self.section {
            Some((s, e)) if f >= e => {
                let mut j = (f - e) / (e - s) + 1;
                if let Some(max) = self.jumps {
                    j = j.min(max as u64);
                }
                (f - j * (e - s), j as u32, 0)
            }
            _ => (f, 0, 0),
        }
    }
}

/// Renders a [`MidiSong`] through a SoundFont synthesizer.
pub struct MidiDecoder {
    song: MidiSong,
    synth: Synthesizer,
    plan: Plan,
    rate: u32,
    /// Frame of each event.
    frames: Vec<u64>,
    idx: usize,
    pos: u64,
    jumps_done: u32,
    in_tail: bool,
    tail_done: u64,
    left: Vec<f32>,
    right: Vec<f32>,
    info: StreamInfo,
}

impl MidiDecoder {
    pub fn open(location: &str, options: &MidiOptions) -> Result<Self, DecodeError> {
        let path = local_path(location).ok_or_else(|| DecodeError::Unsupported(location.into()))?;
        let song = MidiSong::from_file(Path::new(&path), options.recover_incomplete)?;
        let sf_path = find_soundfont(options.soundfont.as_deref())?;
        let sf = load_soundfont(&sf_path)?;
        Self::new(song, sf, options)
    }

    /// Creates a decoder for a parsed song with a loaded SoundFont.
    pub fn new(
        song: MidiSong,
        soundfont: Arc<SoundFont>,
        options: &MidiOptions,
    ) -> Result<Self, DecodeError> {
        let mut settings = SynthesizerSettings::new(options.sample_rate as i32);
        settings.enable_reverb_and_chorus = options.reverb_chorus;
        let synth = Synthesizer::new(&soundfont, &settings)
            .map_err(|e| DecodeError::Open(e.to_string()))?;
        let rate = options.sample_rate;
        let plan = Plan::new(&song, options);
        let frames = song
            .events
            .iter()
            .map(|e| (e.secs * rate as f64).round() as u64)
            .collect();
        let info = StreamInfo {
            sample_rate: rate,
            channels: 2,
            bits: 0,
            length_ms: plan.total().map(|f| f * 1000 / rate as u64),
            seekable: true,
            codec: format!("MIDI (type {})", song.format),
            lossless: false,
        };
        Ok(Self {
            song,
            synth,
            plan,
            rate,
            frames,
            idx: 0,
            pos: 0,
            jumps_done: 0,
            in_tail: false,
            tail_done: 0,
            left: vec![0.0; BLOCK],
            right: vec![0.0; BLOCK],
            info,
        })
    }

    pub fn song(&self) -> &MidiSong {
        &self.song
    }

    fn send(&mut self, i: usize) {
        let [status, d1, d2] = self.song.events[i].msg;
        self.synth.process_midi_message(
            (status & 0x0F) as i32,
            (status & 0xF0) as i32,
            d1 as i32,
            d2 as i32,
        );
    }

    /// Re-applies controller, program and pitch-bend events of `range` (no notes).
    fn replay_state(&mut self, from: u64, to: u64) {
        for i in 0..self.frames.len() {
            let f = self.frames[i];
            if f >= to {
                break;
            }
            let [status, d1, _] = self.song.events[i].msg;
            let state = match status & 0xF0 {
                0xB0 => !matches!(d1, 120 | 123..=127),
                0xC0 | 0xE0 => true,
                _ => false,
            };
            if f >= from && state {
                self.send(i);
            }
        }
    }

    fn render(&mut self, n: usize, out: &mut Vec<f32>) {
        self.synth.render(&mut self.left[..n], &mut self.right[..n]);
        out.reserve(n * 2);
        for (l, r) in self.left[..n].iter().zip(&self.right[..n]) {
            out.push(*l);
            out.push(*r);
        }
    }
}

impl Decoder for MidiDecoder {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError> {
        self.read_frames(out).map(ReadStatus::from_frames)
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        let mut f = ms * self.rate as u64 / 1000;
        if let Some(total) = self.plan.total() {
            f = f.min(total);
        }
        let (pos, jumps, tail_done) = self.plan.locate(f);
        self.synth.reset();
        match self.plan.section {
            Some((s, e)) if jumps > 0 => {
                self.replay_state(0, e);
                self.replay_state(s, pos);
            }
            _ => self.replay_state(0, pos),
        }
        self.pos = pos;
        self.idx = self.frames.partition_point(|&x| x < pos);
        self.jumps_done = jumps;
        self.in_tail = self.plan.body().is_some_and(|b| f >= b);
        self.tail_done = tail_done;
        Ok(f * 1000 / self.rate as u64)
    }

    fn kbps(&self) -> Option<u32> {
        None
    }
}

impl MidiDecoder {
    /// Appends the next block; zero frames means the end.
    fn read_frames(&mut self, out: &mut Vec<f32>) -> Result<usize, DecodeError> {
        loop {
            if self.in_tail {
                let n = (self.plan.tail - self.tail_done).min(BLOCK as u64) as usize;
                if n > 0 {
                    self.render(n, out);
                    self.tail_done += n as u64;
                }
                return Ok(n);
            }
            let looping = self
                .plan
                .section
                .filter(|_| self.plan.jumps.is_none_or(|m| self.jumps_done < m));
            let boundary = looping.map_or(self.plan.end, |(_, e)| e);
            while self.idx < self.frames.len()
                && self.frames[self.idx] <= self.pos
                && (looping.is_none() || self.frames[self.idx] < boundary)
            {
                self.send(self.idx);
                self.idx += 1;
            }
            if self.pos >= boundary {
                match looping {
                    Some((start, _)) => {
                        self.synth.note_off_all(false);
                        self.pos = start;
                        self.idx = self.frames.partition_point(|&f| f < start);
                        self.jumps_done += 1;
                    }
                    None => self.in_tail = true,
                }
                continue;
            }
            let next = self
                .frames
                .get(self.idx)
                .map_or(boundary, |&f| f.min(boundary));
            let n = (next - self.pos).min(BLOCK as u64) as usize;
            self.render(n, out);
            self.pos += n as u64;
            return Ok(n);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn vlq_bytes(mut v: u32) -> Vec<u8> {
        let mut out = vec![(v & 0x7F) as u8];
        v >>= 7;
        while v > 0 {
            out.insert(0, (v & 0x7F) as u8 | 0x80);
            v >>= 7;
        }
        out
    }

    /// A track chunk from (delta, event bytes) pairs; appends end-of-track.
    fn track(events: &[(u32, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (d, e) in events {
            body.extend(vlq_bytes(*d));
            body.extend_from_slice(e);
        }
        body.extend_from_slice(&[0, 0xFF, 0x2F, 0]);
        let mut c = b"MTrk".to_vec();
        c.extend_from_slice(&(body.len() as u32).to_be_bytes());
        c.extend(body);
        c
    }

    fn smf(format: u16, division: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut f = b"MThd\0\0\0\x06".to_vec();
        f.extend_from_slice(&format.to_be_bytes());
        f.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        f.extend_from_slice(&division.to_be_bytes());
        for t in tracks {
            f.extend_from_slice(t);
        }
        f
    }

    fn meta(kind: u8, data: &[u8]) -> Vec<u8> {
        let mut m = vec![0xFF, kind];
        m.extend(vlq_bytes(data.len() as u32));
        m.extend_from_slice(data);
        m
    }

    /// Type 1, 480 PPQN: 120 BPM for 960 ticks (1 s), then 240 BPM for 960 ticks (0.5 s).
    /// Two notes; optional loop markers at ticks 480 (0.5 s) and 1440 (1.25 s).
    pub(crate) fn test_song(markers: bool) -> Vec<u8> {
        let name = meta(0x03, b"Test Song");
        let t120 = meta(0x51, &[0x07, 0xA1, 0x20]);
        let t240 = meta(0x51, &[0x03, 0xD0, 0x90]);
        let ls = meta(0x06, b"loopStart");
        let le = meta(0x06, b"loopEnd");
        let mut conductor: Vec<(u32, &[u8])> = vec![(0, &name), (0, &t120)];
        if markers {
            conductor.push((480, &ls));
            conductor.push((480, &t240));
            conductor.push((480, &le));
            conductor.push((480, &[0xFF, 0x01, 0x00]));
        } else {
            conductor.push((960, &t240));
            conductor.push((960, &[0xFF, 0x01, 0x00]));
        }
        let notes: Vec<(u32, &[u8])> = vec![
            (0, &[0xC0, 0]),
            (0, &[0x90, 69, 100]),
            (480, &[69, 0]), // running status, velocity 0 = note off
            (480, &[0x90, 72, 100]),
            (480, &[0x80, 72, 0]),
        ];
        smf(1, 480, &[track(&conductor), track(&notes)])
    }

    #[test]
    fn tempo_map_length_and_title() {
        let s = MidiSong::parse(&test_song(false), false).unwrap();
        assert_eq!(s.format, 1);
        assert_eq!(s.tracks, 2);
        assert_eq!(s.duration_ms(), 1500);
        assert_eq!(s.title(), Some("Test Song"));
        assert_eq!(s.events.len(), 5);
        assert!((s.events[3].secs - 1.0).abs() < 1e-9);
        assert!((s.events[4].secs - 1.25).abs() < 1e-9);
        assert_eq!(s.events[2].msg, [0x90, 69, 0]);
        assert_eq!(s.loop_markers_ms(), (None, None));
    }

    #[test]
    fn loop_markers_and_plan() {
        let s = MidiSong::parse(&test_song(true), false).unwrap();
        assert_eq!(s.duration_ms(), 1500);
        assert_eq!(s.loop_markers_ms(), (Some(500), Some(1250)));
        let mut o = MidiOptions::default();
        assert_eq!(Plan::new(&s, &o).total(), Some(66_150));
        o.loop_mode = LoopMode::WhenDetected;
        o.loop_count = Some(2);
        let p = Plan::new(&s, &o);
        assert_eq!(p.section, Some((22_050, 55_125)));
        assert_eq!(p.total(), Some(66_150 + 2 * 33_075));
        assert_eq!(p.locate(10), (10, 0, 0));
        assert_eq!(p.locate(55_125), (22_050, 1, 0));
        assert_eq!(p.locate(55_125 + 33_075 + 5), (22_055, 2, 0));
        assert_eq!(p.locate(132_299), (66_149, 2, 0));
        o.end_silence_ms = 100;
        let p = Plan::new(&s, &o);
        assert_eq!(p.total(), Some(132_300 + 4_410));
        assert_eq!(p.locate(132_310), (66_150, 2, 10));
        o.loop_count = None;
        let p = Plan::new(&s, &o);
        assert_eq!(p.total(), None);
        assert_eq!(p.locate(55_125 + 10 * 33_075 + 7), (22_057, 11, 0));

        let plain = MidiSong::parse(&test_song(false), false).unwrap();
        o.loop_count = Some(1);
        o.end_silence_ms = 0;
        assert_eq!(Plan::new(&plain, &o).section, None);
        o.loop_mode = LoopMode::Always;
        let p = Plan::new(&plain, &o);
        assert_eq!(p.section, Some((0, 66_150)));
        assert_eq!(p.total(), Some(132_300));
    }

    #[test]
    fn controller_111_marks_loop_start() {
        let notes: Vec<(u32, &[u8])> = vec![
            (0, &[0x90, 60, 100]),
            (96, &[0xB0, 111, 0]),
            (96, &[0x80, 60, 0]),
        ];
        let s = MidiSong::parse(&smf(0, 96, &[track(&notes)]), false).unwrap();
        assert_eq!(s.format, 0);
        assert_eq!(s.duration_ms(), 1000);
        assert_eq!(s.loop_markers_ms(), (Some(500), None));
        let o = MidiOptions {
            loop_mode: LoopMode::WhenDetected,
            ..MidiOptions::default()
        };
        assert_eq!(Plan::new(&s, &o).section, Some((22_050, 44_100)));
    }

    #[test]
    fn type_2_tracks_play_in_sequence() {
        let a: Vec<(u32, &[u8])> = vec![(0, &[0x90, 60, 100]), (96, &[0x80, 60, 0])];
        let s = MidiSong::parse(&smf(2, 96, &[track(&a), track(&a)]), false).unwrap();
        assert_eq!(s.duration_ms(), 1000);
        assert!((s.events[2].secs - 0.5).abs() < 1e-9);
    }

    #[test]
    fn smpte_division() {
        let a: Vec<(u32, &[u8])> = vec![(0, &[0x90, 60, 100]), (100, &[0x80, 60, 0])];
        // 25 fps, 40 ticks per frame = 1000 ticks per second.
        let div = ((-25i8 as u8 as u16) << 8) | 40;
        let s = MidiSong::parse(&smf(0, div, &[track(&a)]), false).unwrap();
        assert_eq!(s.duration_ms(), 100);
    }

    #[test]
    fn rmid_container_and_info() {
        let data = test_song(false);
        let mut info = b"INFO".to_vec();
        info.extend_from_slice(b"INAM\x06\0\0\0Riffy\0");
        let mut riff = b"RIFF\0\0\0\0RMID".to_vec();
        riff.extend_from_slice(b"LIST");
        riff.extend_from_slice(&(info.len() as u32).to_le_bytes());
        riff.extend(info);
        riff.extend_from_slice(b"data");
        riff.extend_from_slice(&(data.len() as u32).to_le_bytes());
        riff.extend(&data);
        let s = MidiSong::parse(&riff, false).unwrap();
        assert_eq!(s.duration_ms(), 1500);
        assert_eq!(s.title(), Some("Riffy"));
    }

    #[test]
    fn recovers_truncated_files() {
        let mut data = test_song(false);
        data.truncate(data.len() - 6);
        assert!(MidiSong::parse(&data, false).is_err());
        let s = MidiSong::parse(&data, true).unwrap();
        assert_eq!(s.events.len(), 4);
        assert_eq!(s.duration_ms(), 1500);
        assert!(MidiSong::parse(b"not midi", true).is_err());
    }

    #[test]
    fn kar_title_and_text() {
        let kar = meta(0x01, b"@TKaraoke Title");
        let txt = meta(0x01, b"Some text");
        let t: Vec<(u32, &[u8])> = vec![(0, &txt), (0, &kar)];
        let s = MidiSong::parse(&smf(0, 96, &[track(&t)]), false).unwrap();
        assert_eq!(s.title(), Some("Karaoke Title"));
        let t: Vec<(u32, &[u8])> = vec![(0, &txt)];
        let s = MidiSong::parse(&smf(0, 96, &[track(&t)]), false).unwrap();
        assert_eq!(s.title(), Some("Some text"));
    }

    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    fn list(kind: &[u8; 4], subs: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for s in subs {
            body.extend_from_slice(s);
        }
        chunk(b"LIST", &body)
    }

    fn name(n: &str, len: usize) -> Vec<u8> {
        let mut b = n.as_bytes().to_vec();
        b.resize(len, 0);
        b
    }

    fn words(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// A one-preset SoundFont with a looped 441 Hz sine (rooted at A4).
    pub(crate) fn test_soundfont() -> Vec<u8> {
        let mut smpl: Vec<i16> = (0..2000)
            .map(|i| ((i as f32 * std::f32::consts::TAU / 100.0).sin() * 16000.0) as i16)
            .collect();
        smpl.resize(2046, 0);
        let smpl: Vec<u8> = smpl.iter().flat_map(|s| s.to_le_bytes()).collect();
        let phdr = [
            [name("Sine", 20), words(&[0, 0, 0]), vec![0; 12]].concat(),
            [name("EOP", 20), words(&[0, 0, 1]), vec![0; 12]].concat(),
        ]
        .concat();
        let inst = [
            [name("Sine", 20), words(&[0])].concat(),
            [name("EOI", 20), words(&[1])].concat(),
        ]
        .concat();
        let mut shdr = name("Sine", 20);
        for v in [0u32, 2000, 100, 1900, 44_100] {
            shdr.extend_from_slice(&v.to_le_bytes());
        }
        shdr.extend_from_slice(&[69, 0]);
        shdr.extend(words(&[0, 1]));
        shdr.extend(vec![0; 46]);
        let body = [
            b"sfbk".to_vec(),
            list(b"INFO", &[chunk(b"ifil", &words(&[2, 1]))]),
            list(b"sdta", &[chunk(b"smpl", &smpl)]),
            list(
                b"pdta",
                &[
                    chunk(b"phdr", &phdr),
                    chunk(b"pbag", &words(&[0, 0, 1, 0])),
                    chunk(b"pmod", &[0; 10]),
                    chunk(b"pgen", &words(&[41, 0, 0, 0])),
                    chunk(b"inst", &inst),
                    chunk(b"ibag", &words(&[0, 0, 2, 0])),
                    chunk(b"imod", &[0; 10]),
                    chunk(b"igen", &words(&[54, 1, 53, 0, 0, 0])),
                    chunk(b"shdr", &shdr),
                ],
            ),
        ]
        .concat();
        chunk(b"RIFF", &body)
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nulamp-midi-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn decode_all(d: &mut dyn Decoder) -> Vec<f32> {
        let mut out = Vec::new();
        while d.read(&mut out).unwrap().frames() > 0 {}
        out
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn renders_and_seeks_with_a_soundfont() {
        let dir = tmp_dir("render");
        let sf = dir.join("test.sf2");
        std::fs::write(&sf, test_soundfont()).unwrap();
        let mid = dir.join("song.mid");
        std::fs::write(&mid, test_song(true)).unwrap();
        let options = MidiOptions {
            soundfont: Some(sf.clone()),
            reverb_chorus: false,
            ..MidiOptions::default()
        };
        let mut d = MidiDecoder::open(mid.to_str().unwrap(), &options).unwrap();
        assert_eq!(d.info().length_ms, Some(1500));
        assert_eq!(d.song().title(), Some("Test Song"));
        let pcm = decode_all(&mut d);
        assert_eq!(pcm.len(), 66_150 * 2);
        assert!(peak(&pcm[..22_050 * 2]) > 0.05, "first note sounds");
        assert!(
            peak(&pcm[66_000 * 2..]) < 1e-3,
            "silent after the last note-off"
        );

        assert_eq!(d.seek(1000).unwrap(), 1000);
        let rest = decode_all(&mut d);
        assert_eq!(rest.len(), 22_050 * 2);
        assert!(
            peak(&rest[..11_025 * 2]) > 0.05,
            "second note sounds after a seek"
        );

        let looped = MidiOptions {
            loop_mode: LoopMode::WhenDetected,
            loop_count: Some(1),
            end_silence_ms: 50,
            ..options.clone()
        };
        let mut d = MidiDecoder::open(mid.to_str().unwrap(), &looped).unwrap();
        assert_eq!(d.info().length_ms, Some(2300));
        assert_eq!(decode_all(&mut d).len(), (66_150 + 33_075 + 2_205) * 2);
        assert_eq!(d.seek(1600).unwrap(), 1600);
        assert_eq!(decode_all(&mut d).len(), (101_430 - 70_560) * 2);

        let missing = MidiOptions {
            soundfont: Some(dir.join("missing.sf2")),
            ..MidiOptions::default()
        };
        let err = MidiDecoder::open(mid.to_str().unwrap(), &missing)
            .err()
            .unwrap();
        assert!(err.to_string().contains("SoundFont"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renders_with_a_system_soundfont() {
        let Ok(sf) = find_soundfont(None) else {
            eprintln!("skipping: no system SoundFont found");
            return;
        };
        let dir = tmp_dir("system");
        let mid = dir.join("song.mid");
        std::fs::write(&mid, test_song(false)).unwrap();
        let options = MidiOptions {
            soundfont: Some(sf),
            ..MidiOptions::default()
        };
        let mut d = MidiDecoder::open(mid.to_str().unwrap(), &options).unwrap();
        let pcm = decode_all(&mut d);
        assert_eq!(pcm.len(), 66_150 * 2);
        assert!(peak(&pcm) > 0.01);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
