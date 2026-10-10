//! The audio engine thread: owns decoders, the processing chain and the device output, and
//! reports what is heard (07 §4, §6).
//!
//! The next track is requested when less than 500 ms of the current one remains buffered and is
//! written into the same output stream with no gap (07 §6.3).
//!
//! Every track is decoded, given its own gain, mapped to the output channels and resampled to
//! the output rate first. With crossfade on (07 §6.4) the last F of each track is then held
//! back (the "hold"): when the decoder ends, the hold is exactly the end still to be heard, and
//! it fades out under the first F of the next track. The processing stages (EQ, DSP), the
//! visualization tap and the output follow, on the mixed audio.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nulamp_decoders::{Decoder, ReadStatus, Registry, StreamInfo};
use nulamp_eq::Limiter;

use crate::chain::{Stage, VIS_BLOCK, VisBlock, VisTap, map_channels, run_stages};
use crate::mixer::{MixCmd, Shared};
use crate::output::{DeviceSide, Output};
use crate::resample::Resampler;

/// Identifies a track instance (the playlist entry id from the core).
pub type Token = u64;

/// Longest crossfade, in milliseconds.
pub const MAX_CROSSFADE_MS: u32 = 20_000;

/// Gain curve of a crossfade (07 §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FadeCurve {
    /// Gains 1 − t and t.
    #[default]
    Linear,
    /// Gains cos and sin of t·π/2: two unrelated signals keep their combined loudness.
    EqualPower,
}

impl FadeCurve {
    /// (outgoing gain, incoming gain) at fraction `t` (0..=1) of the overlap.
    pub fn gains(self, t: f32) -> (f32, f32) {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => (1.0 - t, t),
            Self::EqualPower => {
                let a = t * std::f32::consts::FRAC_PI_2;
                (a.cos(), a.sin())
            }
        }
    }
}

/// Fade, crossfade and buffer preferences (07 §6.1–6.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputPrefs {
    pub buffer_ms: u32,
    pub prebuffer_ms: u32,
    pub fade_pause_stop: bool,
    pub fade_seek: bool,
    pub fade_ms: u32,
    /// Crossfade at the end of each track (default off).
    pub crossfade: bool,
    /// Crossfade length, 0..=[`MAX_CROSSFADE_MS`] (default 333).
    pub crossfade_ms: u32,
    pub crossfade_curve: FadeCurve,
    /// Also crossfade when the user changes the track (default off).
    pub crossfade_manual: bool,
    /// Join tracks of the same album gaplessly instead of crossfading (default on).
    pub gapless_albums: bool,
}

impl Default for OutputPrefs {
    fn default() -> Self {
        Self {
            buffer_ms: 2000,
            prebuffer_ms: 500,
            fade_pause_stop: true,
            fade_seek: true,
            fade_ms: 333,
            crossfade: false,
            crossfade_ms: 333,
            crossfade_curve: FadeCurve::Linear,
            crossfade_manual: false,
            gapless_albums: true,
        }
    }
}

/// Where the engine sends its audio.
pub enum Backend {
    /// The default audio device.
    Device,
    /// No device: the consuming end of each output the engine opens is put into `slot`, for the
    /// caller to drive (tests, offline rendering). `rate` `None` takes the first track's rate.
    Offline {
        rate: Option<u32>,
        channels: usize,
        slot: OfflineSlot,
    },
}

/// Receives the device side of an offline output.
pub type OfflineSlot = Arc<Mutex<Option<DeviceSide>>>;

pub enum Control {
    /// Stop what plays and start a track at `start_ms`, with a linear pre-gain. With
    /// `crossfade` (a manual change while crossfade on manual changes is enabled), the track
    /// being heard fades out under the new one.
    Open {
        token: Token,
        location: String,
        start_ms: u64,
        gain: f32,
        crossfade: bool,
    },
    /// The track to play after the current one: gapless, or crossfaded when `crossfade` is set
    /// and crossfade is on.
    Queue {
        token: Token,
        location: String,
        gain: f32,
        crossfade: bool,
    },
    /// No track follows the current one.
    NoNext,
    Seek(u64),
    Pause(bool),
    Stop,
    /// Stop with the 2-second fade-out (07 §2.3).
    StopWithFade,
    Volume(u8),
    Balance(i8),
    SetStages(Vec<Box<dyn Stage>>),
    SetPrefs(OutputPrefs),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Started {
        token: Token,
        info: StreamInfo,
    },
    OpenFailed {
        token: Token,
        error: String,
    },
    /// The current track is almost fully buffered: what comes next?
    NeedNext {
        token: Token,
    },
    /// Playback crossed into a queued track (the start of a crossfade).
    TrackChanged {
        token: Token,
        info: StreamInfo,
    },
    /// Everything was played; the output is idle.
    Ended {
        token: Token,
    },
    /// The output device failed.
    DeviceError(String),
    /// A stream title changed (network streams).
    Title {
        token: Token,
        title: String,
    },
}

/// What the user interface polls.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineStatus {
    pub token: Option<Token>,
    pub position_ms: u64,
    pub kbps: Option<u32>,
    pub in_sync: bool,
    pub paused: bool,
    /// Output frames heard so far (visualizer clock).
    pub frames_played: u64,
    pub output_rate: u32,
}

struct Track {
    token: Token,
    decoder: Box<dyn Decoder>,
    gain: f32,
    eos: bool,
    /// Can take part in a crossfade: a local track of known length (not a network stream).
    xf_ok: bool,
    /// Crossfade into this track (from `Control::Queue`).
    crossfade: bool,
}

impl Track {
    fn new(token: Token, location: &str, decoder: Box<dyn Decoder>, gain: f32) -> Self {
        let network = nulamp_core::playlist::is_url(location)
            && !location.starts_with("file://")
            && !location.starts_with("cda://");
        let xf_ok = !network && decoder.info().length_ms.is_some();
        Self {
            token,
            decoder,
            gain,
            eos: false,
            xf_ok,
            crossfade: false,
        }
    }
}

/// The end of the previous track, fading out under the start of the current one.
struct Crossfade {
    /// Output-format samples of the previous track.
    tail: Vec<f32>,
    /// Frames mixed so far.
    pos: usize,
    /// Overlap length in frames.
    len: usize,
    curve: FadeCurve,
    /// Equal-power sums can exceed full scale, so they pass a limiter (03 §4.4).
    limiter: Option<Limiter>,
}

impl Crossfade {
    fn new(tail: Vec<f32>, channels: usize, curve: FadeCurve, rate: u32) -> Self {
        let len = tail.len() / channels.max(1);
        Self {
            tail,
            pos: 0,
            len,
            curve,
            limiter: (curve == FadeCurve::EqualPower).then(|| Limiter::new(rate)),
        }
    }

    fn done(&self) -> bool {
        self.pos >= self.len
    }

    /// Mixes the start of `block` (the incoming track) with the tail.
    fn mix(&mut self, block: &mut [f32], ch: usize) {
        let frames = (block.len() / ch).min(self.len - self.pos);
        for (i, frame) in block.chunks_exact_mut(ch).take(frames).enumerate() {
            let k = self.pos + i;
            let t = (k + 1) as f32 / (self.len + 1) as f32;
            let (g_out, g_in) = self.curve.gains(t);
            let old = &self.tail[k * ch..(k + 1) * ch];
            for (s, o) in frame.iter_mut().zip(old) {
                *s = *s * g_in + *o * g_out;
            }
            if let Some(l) = &mut self.limiter {
                l.process_frame(frame);
            }
        }
        self.pos += frames;
    }

    /// The rest of the overlap when the incoming track ended early: the tail fading out alone.
    fn finish(&mut self, ch: usize, out: &mut Vec<f32>) {
        let start = out.len();
        out.resize(start + (self.len - self.pos) * ch, 0.0);
        self.mix(&mut out[start..], ch);
    }
}

/// A track boundary in the output stream.
struct Marker {
    token: Token,
    info: StreamInfo,
    start_frame: u64,
    start_ms: u64,
}

/// Handle to the engine thread.
pub struct AudioEngine {
    tx: Sender<Control>,
    pub events: Receiver<Event>,
    pub status: Arc<Mutex<EngineStatus>>,
    pub vis: Arc<VisTap>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl AudioEngine {
    /// Starts the engine on the default audio device.
    pub fn start(registry: Arc<Registry>) -> Self {
        Self::start_with(registry, Backend::Device)
    }

    /// Starts the engine without a device. The returned slot receives the device side of the
    /// output once the first track is opened; the caller renders the audio from it.
    pub fn start_offline(
        registry: Arc<Registry>,
        rate: Option<u32>,
        channels: usize,
    ) -> (Self, OfflineSlot) {
        let slot = OfflineSlot::default();
        let engine = Self::start_with(
            registry,
            Backend::Offline {
                rate,
                channels,
                slot: slot.clone(),
            },
        );
        (engine, slot)
    }

    pub fn start_with(registry: Arc<Registry>, backend: Backend) -> Self {
        let (tx, rx) = channel();
        let (etx, events) = channel();
        let status = Arc::new(Mutex::new(EngineStatus::default()));
        let vis = Arc::new(VisTap::default());
        let shared = Shared::new();
        let worker = Worker {
            registry,
            rx,
            events: etx,
            status: status.clone(),
            vis: vis.clone(),
            shared: shared.clone(),
            out: None,
            backend,
            current: None,
            next: None,
            no_next: false,
            asked_next: false,
            markers: VecDeque::new(),
            written: 0,
            flush_gen: 0,
            stages: Vec::new(),
            stage_format: (0, 0),
            resampler: None,
            prefs: OutputPrefs::default(),
            paused: false,
            decoded: Vec::new(),
            mapped: Vec::new(),
            resampled: Vec::new(),
            hold: VecDeque::new(),
            hold_marks: VecDeque::new(),
            hold_cap: 0,
            hold_target: 0,
            fade: None,
            block: Vec::new(),
            pending: Vec::new(),
            vis_acc: Vec::new(),
            vis_frame: 0,
        };
        let thread = std::thread::Builder::new()
            .name("nulamp-audio".into())
            .spawn(move || worker.run())
            .expect("spawn audio thread");
        Self {
            tx,
            events,
            status,
            vis,
            shared,
            thread: Some(thread),
        }
    }

    pub fn send(&self, c: Control) {
        let _ = self.tx.send(c);
    }

    pub fn status(&self) -> EngineStatus {
        let mut s = self.status.lock().expect("status lock").clone();
        s.frames_played = self.shared.frames_played.load(Ordering::Relaxed);
        s
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(Control::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    registry: Arc<Registry>,
    rx: Receiver<Control>,
    events: Sender<Event>,
    status: Arc<Mutex<EngineStatus>>,
    vis: Arc<VisTap>,
    shared: Arc<Shared>,
    out: Option<Output>,
    backend: Backend,
    current: Option<Track>,
    next: Option<Track>,
    no_next: bool,
    asked_next: bool,
    markers: VecDeque<Marker>,
    /// Output frames written to the ring.
    written: u64,
    flush_gen: u64,
    stages: Vec<Box<dyn Stage>>,
    stage_format: (u32, usize),
    resampler: Option<(u32, Resampler)>,
    prefs: OutputPrefs,
    paused: bool,
    decoded: Vec<f32>,
    mapped: Vec<f32>,
    resampled: Vec<f32>,
    /// Output-format audio held back for a crossfade: at most `hold_cap` frames stay after
    /// each block.
    hold: VecDeque<f32>,
    /// Track boundaries inside `hold`: (frame offset from its front, marker).
    hold_marks: VecDeque<(usize, Marker)>,
    hold_cap: usize,
    /// F for a track that may crossfade, else 0. `hold_cap` starts at 0 after a start, seek or
    /// track change and rises to this once the output buffer is full, so holding back never
    /// delays the start of playback.
    hold_target: usize,
    fade: Option<Crossfade>,
    /// Scratch block between the hold and the stages.
    block: Vec<f32>,
    /// Processed output samples not yet accepted by the ring.
    pending: Vec<f32>,
    vis_acc: Vec<f32>,
    vis_frame: u64,
}

impl Worker {
    fn run(mut self) {
        loop {
            let busy = self.current.is_some() && !self.paused;
            let timeout = if busy {
                Duration::from_millis(2)
            } else {
                Duration::from_millis(20)
            };
            match self.rx.recv_timeout(timeout) {
                Ok(Control::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(c) => {
                    self.control(c);
                    while let Ok(c) = self.rx.try_recv() {
                        if matches!(c, Control::Shutdown) {
                            return;
                        }
                        self.control(c);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.pump();
            self.track_heard();
            self.publish();
        }
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    fn played(&self) -> u64 {
        self.shared.frames_played.load(Ordering::Relaxed)
    }

    fn control(&mut self, c: Control) {
        match c {
            Control::Open {
                token,
                location,
                start_ms,
                gain,
                crossfade,
            } => self.open(token, &location, start_ms, gain, crossfade),
            Control::Queue {
                token,
                location,
                gain,
                crossfade,
            } => match self.registry.open(&location) {
                Ok(decoder) => {
                    let mut t = Track::new(token, &location, decoder, gain);
                    t.crossfade = crossfade;
                    self.next = Some(t);
                    self.no_next = false;
                }
                Err(e) => self.emit(Event::OpenFailed {
                    token,
                    error: e.to_string(),
                }),
            },
            Control::NoNext => {
                self.no_next = true;
                // Nothing to crossfade into: the held end plays as it is.
                self.hold_cap = 0;
                self.release_overflow();
                if let Some(o) = &mut self.out {
                    o.send(MixCmd::EndOfData);
                }
            }
            Control::Seek(ms) => self.seek(ms),
            Control::Pause(on) => {
                self.paused = on;
                let fade = self.prefs.fade_pause_stop;
                let ms = self.prefs.fade_ms;
                if let Some(o) = &mut self.out {
                    if on {
                        o.send(MixCmd::Fade {
                            to: 0.0,
                            ms: if fade { ms } else { 0 },
                        });
                        o.send(MixCmd::Pause(true));
                    } else {
                        o.send(MixCmd::Pause(false));
                        o.send(MixCmd::Fade {
                            to: 1.0,
                            ms: if fade { ms } else { 0 },
                        });
                    }
                }
            }
            Control::Stop => {
                if self.prefs.fade_pause_stop && !self.paused {
                    let ms = self.prefs.fade_ms;
                    if let Some(o) = &mut self.out {
                        o.fade_and_wait(0.0, ms);
                    }
                }
                self.stop_now();
            }
            Control::StopWithFade => {
                // The old audio fades out in the background (07 §2.3). Keep the buffer filled
                // so the whole ramp is heard.
                if let Some(o) = &mut self.out {
                    o.send(MixCmd::Fade { to: 0.0, ms: 2048 });
                }
                let t = Instant::now();
                while self
                    .out
                    .as_ref()
                    .is_some_and(|o| o.shared.fade_gain() > 1e-4)
                    && t.elapsed() < Duration::from_millis(2048 + 150)
                {
                    self.pump();
                    std::thread::sleep(Duration::from_millis(2));
                }
                std::thread::sleep(Duration::from_millis(100));
                self.stop_now();
            }
            Control::Volume(v) => self.shared.volume.store(v as u32, Ordering::Relaxed),
            Control::Balance(b) => self.shared.balance.store(b as i32, Ordering::Relaxed),
            Control::SetStages(s) => {
                self.stages = s;
                self.stage_format = (0, 0);
            }
            Control::SetPrefs(p) => {
                self.prefs = p;
                self.update_hold_cap(false);
            }
            Control::Shutdown => {}
        }
    }

    fn flush_output(&mut self) {
        self.flush_gen += 1;
        let g = self.flush_gen;
        if let Some(o) = &mut self.out {
            o.flush(g);
        }
        self.written = self.played();
        self.pending.clear();
        self.hold.clear();
        self.hold_marks.clear();
        self.fade = None;
        self.vis.clear();
        self.vis_acc.clear();
    }

    fn stop_now(&mut self) {
        self.flush_output();
        self.current = None;
        self.next = None;
        self.markers.clear();
        self.paused = false;
        if let Some(o) = &mut self.out {
            o.send(MixCmd::Pause(false));
            o.send(MixCmd::Fade { to: 1.0, ms: 0 });
        }
        let mut s = self.status.lock().expect("status lock");
        *s = EngineStatus {
            output_rate: s.output_rate,
            ..EngineStatus::default()
        };
    }

    fn ensure_output(&mut self, rate: u32) -> bool {
        if let Some(o) = &self.out {
            if !o.failed.load(Ordering::Relaxed) {
                return true;
            }
            log::warn!("audio device lost; reopening the default device");
            self.out = None;
        }
        let opened = match &self.backend {
            Backend::Device => Output::open(rate, self.prefs.buffer_ms, self.shared.clone()),
            Backend::Offline {
                rate: fixed,
                channels,
                slot,
            } => {
                let (o, device) = Output::offline(
                    fixed.unwrap_or(rate),
                    *channels,
                    self.prefs.buffer_ms,
                    self.shared.clone(),
                );
                *slot.lock().expect("offline slot") = Some(device);
                Ok(o)
            }
        };
        match opened {
            Ok(o) => {
                self.written = self.played();
                self.out = Some(o);
                true
            }
            Err(e) => {
                self.emit(Event::DeviceError(e.to_string()));
                false
            }
        }
    }

    fn open(&mut self, token: Token, location: &str, start_ms: u64, gain: f32, crossfade: bool) {
        // Starting a new track replaces whatever plays. After the flush, the heard position is
        // final: a manual crossfade continues the old track from there.
        self.flush_output();
        let tail = if crossfade {
            self.capture_heard_tail()
        } else {
            None
        };
        self.current = None;
        self.next = None;
        self.markers.clear();
        self.no_next = false;
        self.asked_next = false;
        let mut decoder = match self.registry.open(location) {
            Ok(d) => d,
            Err(e) => {
                self.emit(Event::OpenFailed {
                    token,
                    error: e.to_string(),
                });
                return;
            }
        };
        let info = decoder.info().clone();
        if !self.ensure_output(info.sample_rate) {
            return;
        }
        let mut pos = 0;
        if start_ms > 0 && info.seekable {
            pos = decoder.seek(start_ms).unwrap_or(0);
        }
        self.paused = false;
        let prebuffer = self.prebuffer_frames();
        if let Some(o) = &mut self.out {
            o.send(MixCmd::Pause(false));
            o.send(MixCmd::Fade { to: 1.0, ms: 0 });
            o.send(MixCmd::Prebuffer(prebuffer));
        }
        self.reset_chain();
        self.add_marker(Marker {
            token,
            info: info.clone(),
            start_frame: 0,
            start_ms: pos,
        });
        self.current = Some(Track::new(token, location, decoder, gain));
        if let (Some(tail), Some(o)) = (tail, &self.out)
            && self.current.as_ref().is_some_and(|c| c.xf_ok)
        {
            self.fade = Some(Crossfade::new(
                tail,
                o.channels,
                self.prefs.crossfade_curve,
                o.rate,
            ));
        }
        self.update_hold_cap(true);
        self.emit(Event::Started { token, info });
    }

    /// For a crossfade on a manual track change: decodes the next F of the track being heard
    /// again, from the heard position, as output-format audio. `None` when that is not possible
    /// (paused, nothing heard, a stream, not seekable).
    fn capture_heard_tail(&mut self) -> Option<Vec<f32>> {
        if !self.prefs.crossfade || !self.prefs.crossfade_manual || self.paused {
            return None;
        }
        let (out_rate, out_ch) = self.out.as_ref().map(|o| (o.rate, o.channels))?;
        let played = self.played();
        let heard = self.markers.front()?;
        let pos = heard.start_ms
            + played.saturating_sub(heard.start_frame) * 1000 / out_rate.max(1) as u64;
        let heard_token = heard.token;
        let frames = self.crossfade_frames(out_rate);
        let cur = self.current.as_mut()?;
        if cur.token != heard_token || !cur.xf_ok || !cur.decoder.info().seekable || frames == 0 {
            return None;
        }
        cur.decoder.seek(pos).ok()?;
        let mut tail = Vec::with_capacity(frames * out_ch);
        let mut resampler: Option<(u32, Resampler)> = None;
        let (mut buf, mut mapped) = (Vec::new(), Vec::new());
        while tail.len() < frames * out_ch {
            buf.clear();
            if !matches!(cur.decoder.read(&mut buf), Ok(ReadStatus::Frames(_))) {
                break;
            }
            let info = cur.decoder.info();
            let (rate, ch) = (info.sample_rate, info.channels.max(1) as usize);
            buf.iter_mut().for_each(|s| *s *= cur.gain);
            mapped.clear();
            map_channels(&buf, ch, out_ch, &mut mapped);
            if rate == out_rate {
                tail.extend_from_slice(&mapped);
            } else {
                if resampler.as_ref().map(|(r, _)| *r) != Some(rate) {
                    resampler = Some((rate, Resampler::new(rate, out_rate, out_ch)));
                }
                let (_, r) = resampler.as_mut().expect("resampler");
                r.process(&mapped, &mut tail);
            }
        }
        tail.truncate(frames * out_ch);
        Some(tail)
    }

    fn crossfade_frames(&self, rate: u32) -> usize {
        (rate as u64 * self.prefs.crossfade_ms.min(MAX_CROSSFADE_MS) as u64 / 1000) as usize
    }

    /// Holds back the last F of the current track when it may crossfade into the next one.
    /// `restart`: a track started or the stream was flushed; hold nothing until the output
    /// buffer is full.
    fn update_hold_cap(&mut self, restart: bool) {
        let rate = self.out.as_ref().map(|o| o.rate);
        self.hold_target = match (rate, &self.current) {
            (Some(rate), Some(c)) if self.prefs.crossfade && c.xf_ok => self.crossfade_frames(rate),
            _ => 0,
        };
        if restart {
            self.hold_cap = 0;
        }
        self.hold_cap = self.hold_cap.min(self.hold_target);
        self.release_overflow();
    }

    fn out_channels(&self) -> usize {
        self.out.as_ref().map(|o| o.channels).unwrap_or(2)
    }

    /// Output frame at which the next released audio will be heard.
    fn out_pos(&self) -> u64 {
        self.written + (self.pending.len() / self.out_channels()) as u64
    }

    /// Records a track boundary after everything processed so far.
    fn add_marker(&mut self, mut m: Marker) {
        if self.hold.is_empty() {
            m.start_frame = self.out_pos();
            self.markers.push_back(m);
        } else {
            let at = self.hold.len() / self.out_channels();
            self.hold_marks.push_back((at, m));
        }
    }

    /// Releases held audio beyond `hold_cap`.
    fn release_overflow(&mut self) {
        let frames = self.hold.len() / self.out_channels();
        if frames > self.hold_cap || (self.hold_cap == 0 && !self.hold_marks.is_empty()) {
            self.release(frames.saturating_sub(self.hold_cap));
        }
    }

    /// Sends the first `frames` held frames on to the stages, registering the track boundaries
    /// among them.
    fn release(&mut self, frames: usize) {
        let ch = self.out_channels();
        let mut left = frames.min(self.hold.len() / ch);
        loop {
            while self.hold_marks.front().is_some_and(|(at, _)| *at == 0) {
                let (_, mut m) = self.hold_marks.pop_front().expect("mark");
                m.start_frame = self.out_pos();
                self.markers.push_back(m);
            }
            if left == 0 {
                break;
            }
            let seg = self
                .hold_marks
                .front()
                .map_or(left, |(at, _)| (*at).min(left));
            let mut block = std::mem::take(&mut self.block);
            block.clear();
            block.extend(self.hold.drain(..seg * ch));
            for (at, _) in &mut self.hold_marks {
                *at -= seg;
            }
            left -= seg;
            self.finish_block(&mut block);
            self.block = block;
        }
    }

    /// Runs output-format audio through the stages and the visualization tap into `pending`.
    fn finish_block(&mut self, block: &mut Vec<f32>) {
        let (rate, ch) = match &self.out {
            Some(o) => (o.rate, o.channels),
            None => return,
        };
        if self.stage_format != (rate, ch) {
            for s in &mut self.stages {
                s.configure(rate, ch);
            }
            self.stage_format = (rate, ch);
        }
        run_stages(&mut self.stages, block);
        self.tap(block, rate, ch);
        self.pending.extend_from_slice(block);
    }

    fn prebuffer_frames(&self) -> u32 {
        let rate = self.out.as_ref().map(|o| o.rate).unwrap_or(44_100);
        (rate as u64 * self.prefs.prebuffer_ms as u64 / 1000) as u32
    }

    fn reset_chain(&mut self) {
        for s in &mut self.stages {
            s.reset();
        }
        if let Some((_, r)) = &mut self.resampler {
            r.reset();
        }
    }

    fn seek(&mut self, ms: u64) {
        let Some(cur) = &self.current else { return };
        if !cur.decoder.info().seekable {
            return;
        }
        let fade = self.prefs.fade_seek && !self.paused;
        let fade_ms = self.prefs.fade_ms;
        if fade && let Some(o) = &mut self.out {
            o.fade_and_wait(0.0, fade_ms);
        }
        // Seeking discards a queued next track and reopens the current one's timeline.
        self.flush_output();
        self.next = None;
        self.asked_next = false;
        self.no_next = false;
        let cur = self.current.as_mut().expect("current track");
        let token = cur.token;
        cur.eos = false;
        let pos = match cur.decoder.seek(ms) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("seek failed: {e}");
                // A failed seek counts as the end of the track (07 §2.5).
                self.current = None;
                self.markers.clear();
                self.emit(Event::Ended { token });
                return;
            }
        };
        let info = cur.decoder.info().clone();
        self.markers.clear();
        self.add_marker(Marker {
            token,
            info,
            start_frame: 0,
            start_ms: pos,
        });
        self.reset_chain();
        self.update_hold_cap(true);
        let prebuffer = self.prebuffer_frames();
        if let Some(o) = &mut self.out {
            o.send(MixCmd::Prebuffer(prebuffer));
            if fade {
                o.send(MixCmd::Fade {
                    to: 1.0,
                    ms: fade_ms,
                });
            } else {
                o.send(MixCmd::Fade { to: 1.0, ms: 0 });
            }
        }
    }

    /// Decodes and writes while there is room in the output buffer.
    fn pump(&mut self) {
        if self.paused || self.out.is_none() {
            return;
        }
        if let Some(o) = &self.out
            && o.failed.load(Ordering::Relaxed)
        {
            let rate = o.rate;
            // Reopen and continue from the heard position.
            let pos = self.status.lock().expect("status lock").position_ms;
            self.out = None;
            if self.ensure_output(rate) && self.current.is_some() {
                self.seek(pos);
            }
            return;
        }
        for _ in 0..16 {
            if !self.pending.is_empty() {
                let out = self.out.as_mut().expect("output");
                let n = out.write(&self.pending);
                self.written += (n / out.channels) as u64;
                self.pending.drain(..n);
                if !self.pending.is_empty() {
                    return;
                }
            }
            let Some(cur) = &mut self.current else { return };
            if cur.eos {
                if !self.switch_to_next() {
                    return;
                }
                continue;
            }
            let free = self.out.as_ref().map(|o| o.free_frames()).unwrap_or(0);
            if free < 4096 {
                // The output buffer is full: now hold back the end for a crossfade. Decoding
                // continues into the hold while the buffer plays.
                self.hold_cap = self.hold_target;
                if self.hold.len() / self.out_channels() >= self.hold_cap {
                    return;
                }
            }
            self.decoded.clear();
            let cur = self.current.as_mut().expect("current track");
            let status = cur.decoder.read(&mut self.decoded).unwrap_or_else(|e| {
                log::warn!("decode error: {e}");
                ReadStatus::End
            });
            let title = cur.decoder.take_title();
            let token = cur.token;
            if status == ReadStatus::End {
                cur.eos = true;
            }
            if let Some(title) = title {
                self.emit(Event::Title { token, title });
            }
            match status {
                ReadStatus::Frames(_) => self.process_block(),
                // A network stream is buffering: try again on the next round.
                ReadStatus::Pending => return,
                ReadStatus::End => {}
            }
        }
    }

    /// Runs one decoded block through gain, channel mapping and resampling to the output
    /// format, mixes it with a fading-out previous track, and holds or releases it.
    fn process_block(&mut self) {
        let cur = self.current.as_ref().expect("current track");
        let info = cur.decoder.info();
        let (rate, ch) = (info.sample_rate, info.channels.max(1) as usize);
        let gain = cur.gain;
        if gain != 1.0 {
            for s in &mut self.decoded {
                *s *= gain;
            }
        }
        let out = self.out.as_ref().expect("output");
        let (out_rate, out_ch) = (out.rate, out.channels);
        self.mapped.clear();
        map_channels(&self.decoded, ch, out_ch, &mut self.mapped);
        self.resampled.clear();
        if rate == out_rate {
            self.resampled.extend_from_slice(&self.mapped);
        } else {
            if self.resampler.as_ref().map(|(r, _)| *r) != Some(rate) {
                self.resampler = Some((rate, Resampler::new(rate, out_rate, out_ch)));
            }
            let (_, r) = self.resampler.as_mut().expect("resampler");
            r.process(&self.mapped, &mut self.resampled);
        }
        if let Some(f) = &mut self.fade {
            f.mix(&mut self.resampled, out_ch);
            if f.done() {
                self.fade = None;
            }
        }
        self.hold.extend(self.resampled.iter().copied());
        self.release_overflow();
    }

    fn tap(&mut self, block: &[f32], rate: u32, ch: usize) {
        if self.vis_acc.is_empty() {
            self.vis_frame = self.written + (self.pending.len() / ch) as u64;
        }
        self.vis_acc.extend_from_slice(block);
        while self.vis_acc.len() >= VIS_BLOCK * ch {
            let samples: Vec<f32> = self.vis_acc.drain(..VIS_BLOCK * ch).collect();
            self.vis.push(VisBlock {
                frame: self.vis_frame,
                rate,
                channels: ch,
                samples,
            });
            self.vis_frame += VIS_BLOCK as u64;
        }
    }

    /// The current decoder ended: continue with the queued track (gapless or crossfaded), or
    /// ask for one.
    fn switch_to_next(&mut self) -> bool {
        let (out_rate, ch) = match &self.out {
            Some(o) => (o.rate, o.channels),
            None => return false,
        };
        // The track ended inside its own fade-in: the previous one fades out alone.
        if let Some(mut f) = self.fade.take() {
            let mut rest = Vec::new();
            f.finish(ch, &mut rest);
            self.hold.extend(rest);
            self.release_overflow();
        }
        if let Some(next) = self.next.take() {
            let marker = Marker {
                token: next.token,
                info: next.decoder.info().clone(),
                start_frame: 0,
                start_ms: 0,
            };
            // A next track shorter than the crossfade is joined gaplessly.
            let long_enough = next
                .decoder
                .info()
                .length_ms
                .is_some_and(|l| l >= self.prefs.crossfade_ms as u64);
            let crossfade = self.prefs.crossfade
                && next.crossfade
                && next.xf_ok
                && long_enough
                && self.current.as_ref().is_some_and(|c| c.xf_ok)
                && !self.hold.is_empty();
            if crossfade {
                // The tail must belong to the ending track only.
                if let Some(&(at, _)) = self.hold_marks.back() {
                    self.release(at);
                }
                let tail: Vec<f32> = self.hold.drain(..).collect();
                self.fade = Some(Crossfade::new(
                    tail,
                    ch,
                    self.prefs.crossfade_curve,
                    out_rate,
                ));
            }
            self.add_marker(marker);
            self.current = Some(next);
            self.asked_next = false;
            // Not a restart: the output buffer is already in steady state, so the next track's
            // first blocks may be held at once. Resetting the hold here released them whenever
            // the buffer was not quite full at the change, which shortened a following
            // crossfade by one decoded block.
            self.update_hold_cap(false);
            return true;
        }
        let buffered_ms = self
            .out
            .as_ref()
            .map(|o| o.buffered_frames() as u64 * 1000 / o.rate.max(1) as u64)
            .unwrap_or(0);
        // With a held end, the next track is needed now to mix it in.
        let held = !self.hold.is_empty();
        if !self.asked_next && !self.no_next && (buffered_ms < 500 || held) {
            self.asked_next = true;
            let token = self.current.as_ref().expect("current").token;
            self.emit(Event::NeedNext { token });
        }
        if held && (self.no_next || buffered_ms < 300) {
            // No next track, or it comes too late: play the held end as it is.
            self.hold_cap = 0;
            self.release_overflow();
        }
        false
    }

    /// Reports track changes and the end of playback as they are heard.
    fn track_heard(&mut self) {
        let played = self.played();
        while self.markers.len() > 1 && played >= self.markers[1].start_frame {
            self.markers.pop_front();
            let m = &self.markers[0];
            self.emit(Event::TrackChanged {
                token: m.token,
                info: m.info.clone(),
            });
        }
        let finished =
            self.current.as_ref().is_some_and(|c| c.eos) && self.next.is_none() && self.no_next;
        if finished
            && self.pending.is_empty()
            && self.hold.is_empty()
            && self.fade.is_none()
            && played >= self.written
        {
            let token = self.current.as_ref().expect("current").token;
            self.current = None;
            self.markers.clear();
            self.emit(Event::Ended { token });
        }
    }

    fn publish(&mut self) {
        let played = self.played();
        let rate = self.out.as_ref().map(|o| o.rate).unwrap_or(0);
        let mut s = self.status.lock().expect("status lock");
        s.output_rate = rate;
        s.paused = self.paused;
        match self.markers.front() {
            Some(m) => {
                s.token = Some(m.token);
                let frames = played.saturating_sub(m.start_frame);
                s.position_ms = m.start_ms + frames * 1000 / rate.max(1) as u64;
                let heard_is_current = self.current.as_ref().is_some_and(|c| c.token == m.token);
                if heard_is_current {
                    let c = self.current.as_ref().expect("current");
                    s.kbps = c.decoder.kbps();
                    s.in_sync = c.decoder.in_sync();
                }
            }
            None => {
                if self.current.is_none() {
                    s.token = None;
                    s.position_ms = 0;
                    s.kbps = None;
                    s.in_sync = false;
                }
            }
        }
    }
}
