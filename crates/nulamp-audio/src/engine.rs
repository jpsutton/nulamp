//! The audio engine thread: owns decoders, the processing chain and the device output, and
//! reports what is heard (07 §4, §6).
//!
//! The next track is requested when less than 500 ms of the current one remains buffered and is
//! written into the same output stream with no gap (07 §6.3).

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use nulamp_decoders::{Decoder, Registry, StreamInfo};

use crate::chain::{Stage, VIS_BLOCK, VisBlock, VisTap, map_channels, run_stages};
use crate::mixer::{MixCmd, Shared};
use crate::output::Output;
use crate::resample::Resampler;

/// Identifies a track instance (the playlist entry id from the core).
pub type Token = u64;

/// Fade and buffer preferences (07 §6.1–6.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputPrefs {
    pub buffer_ms: u32,
    pub prebuffer_ms: u32,
    pub fade_pause_stop: bool,
    pub fade_seek: bool,
    pub fade_ms: u32,
}

impl Default for OutputPrefs {
    fn default() -> Self {
        Self {
            buffer_ms: 2000,
            prebuffer_ms: 500,
            fade_pause_stop: true,
            fade_seek: true,
            fade_ms: 333,
        }
    }
}

pub enum Control {
    /// Stop what plays and start a track at `start_ms`, with a linear pre-gain.
    Open {
        token: Token,
        location: String,
        start_ms: u64,
        gain: f32,
    },
    /// The track to play after the current one (gapless).
    Queue {
        token: Token,
        location: String,
        gain: f32,
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
    /// Playback crossed into a queued track.
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
    pub fn start(registry: Arc<Registry>) -> Self {
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
            } => self.open(token, &location, start_ms, gain),
            Control::Queue {
                token,
                location,
                gain,
            } => match self.registry.open(&location) {
                Ok(decoder) => {
                    self.next = Some(Track {
                        token,
                        decoder,
                        gain,
                        eos: false,
                    });
                    self.no_next = false;
                }
                Err(e) => self.emit(Event::OpenFailed {
                    token,
                    error: e.to_string(),
                }),
            },
            Control::NoNext => {
                self.no_next = true;
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
                if let Some(o) = &mut self.out {
                    o.fade_and_wait(0.0, 2048);
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
            Control::SetPrefs(p) => self.prefs = p,
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
        match Output::open(rate, self.prefs.buffer_ms, self.shared.clone()) {
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

    fn open(&mut self, token: Token, location: &str, start_ms: u64, gain: f32) {
        // Starting a new track replaces whatever plays.
        self.flush_output();
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
        self.markers.push_back(Marker {
            token,
            info: info.clone(),
            start_frame: self.written,
            start_ms: pos,
        });
        self.current = Some(Track {
            token,
            decoder,
            gain,
            eos: false,
        });
        self.emit(Event::Started { token, info });
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
        self.markers.push_back(Marker {
            token,
            info,
            start_frame: self.written,
            start_ms: pos,
        });
        self.reset_chain();
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
            if self.ensure_output(rate)
                && let Some(c) = &self.current
            {
                let token = c.token;
                let _ = token;
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
                return;
            }
            self.decoded.clear();
            let cur = self.current.as_mut().expect("current track");
            match cur.decoder.read(&mut self.decoded) {
                Ok(0) => {
                    cur.eos = true;
                    continue;
                }
                Ok(_) => {}
                Err(e) => {
                    log::warn!("decode error: {e}");
                    cur.eos = true;
                    continue;
                }
            }
            if let Some(t) = cur.decoder.take_title() {
                let token = cur.token;
                self.emit(Event::Title { token, title: t });
            }
            self.process_block();
        }
    }

    /// Runs one decoded block through gain, stages, the visualization tap, channel mapping and
    /// resampling, into `pending`.
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
        if self.stage_format != (rate, ch) {
            for s in &mut self.stages {
                s.configure(rate, ch);
            }
            self.stage_format = (rate, ch);
        }
        run_stages(&mut self.stages, &mut self.decoded);
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
        self.tap(out_rate, out_ch);
        self.pending.extend_from_slice(&self.resampled);
    }

    fn tap(&mut self, rate: u32, ch: usize) {
        if self.vis_acc.is_empty() {
            self.vis_frame = self.written + (self.pending.len() / ch) as u64;
        }
        self.vis_acc.extend_from_slice(&self.resampled);
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

    /// The current decoder ended: continue with the queued track, or ask for one.
    fn switch_to_next(&mut self) -> bool {
        if let Some(next) = self.next.take() {
            let info = next.decoder.info().clone();
            let start = self.written
                + (self.pending.len() / self.out.as_ref().map(|o| o.channels).unwrap_or(2)) as u64;
            self.markers.push_back(Marker {
                token: next.token,
                info,
                start_frame: start,
                start_ms: 0,
            });
            self.current = Some(next);
            self.asked_next = false;
            return true;
        }
        let buffered_ms = self
            .out
            .as_ref()
            .map(|o| o.buffered_frames() as u64 * 1000 / o.rate.max(1) as u64)
            .unwrap_or(0);
        if !self.asked_next && !self.no_next && buffered_ms < 500 {
            self.asked_next = true;
            let token = self.current.as_ref().expect("current").token;
            self.emit(Event::NeedNext { token });
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
        if finished && self.pending.is_empty() && played >= self.written {
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
