//! Offline test harness: a [`Player`] on an engine without an audio device, driven by the test
//! thread in place of the device callback, and synthetic WAV files.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use nulamp_audio::{AudioEngine, OfflineSlot, OutputPrefs, Player, PlayerEvent};
use nulamp_core::{Command, Engine, EntryId, NewEntry};
use nulamp_decoders::{ReadStatus, Registry};

/// Frames per simulated device callback.
pub const BLOCK: usize = 1024;

/// A fresh directory for one test's files.
pub fn test_dir(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "conformance-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

/// Writes interleaved samples as a 16-bit PCM WAV file (x × 32768, rounded).
pub fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[f32]) {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes())
        .collect();
    let block_align = channels * 2;
    let mut f = Vec::with_capacity(44 + data.len());
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    f.extend_from_slice(b"WAVEfmt ");
    f.extend_from_slice(&16u32.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&channels.to_le_bytes());
    f.extend_from_slice(&rate.to_le_bytes());
    f.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    f.extend_from_slice(&block_align.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(&data);
    std::fs::write(path, f).expect("write wav");
}

/// `frames` frames of a constant value per channel.
pub fn dc(frames: usize, levels: &[f32]) -> Vec<f32> {
    (0..frames).flat_map(|_| levels.iter().copied()).collect()
}

/// A stereo sine: left sin, right cos.
pub fn sine(rate: u32, freq: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .flat_map(|i| {
            let p = i as f32 * freq * std::f32::consts::TAU / rate as f32;
            [amp * p.sin(), amp * p.cos()]
        })
        .collect()
}

/// Decodes a whole file with the default decoders.
pub fn decode_all(path: &Path) -> Vec<f32> {
    let reg = Registry::with_defaults();
    let mut d = reg.open(&path.to_string_lossy()).expect("open");
    let mut out = Vec::new();
    while let Ok(ReadStatus::Frames(_)) = d.read(&mut out) {}
    out
}

/// Largest absolute difference between consecutive samples of one channel.
pub fn max_step(samples: &[f32], ch: usize, c: usize) -> f32 {
    samples
        .chunks_exact(ch)
        .map(|f| f[c])
        .collect::<Vec<_>>()
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// The engine's status when the heard track changed.
pub struct Heard {
    /// Frames played according to the engine (the device clock).
    pub pulled: usize,
    pub token: Option<u64>,
    pub position_ms: u64,
}

pub struct Rig {
    pub player: Player,
    slot: OfflineSlot,
    pub ch: usize,
    /// Every rendered frame, as a device would play it (with prebuffer and pause silence).
    pub timeline: Vec<f32>,
    /// Only the frames taken from the output buffer.
    pub pulled: Vec<f32>,
    /// Player events with the number of frames pulled when they were seen.
    pub events: Vec<(usize, PlayerEvent)>,
    /// Each change of the heard track reported by the engine.
    pub heard: Vec<Heard>,
    starving: bool,
}

impl Rig {
    /// A player at volume 255 on an offline output (`rate` `None`: the first track's rate).
    pub fn new(rate: Option<u32>, channels: usize) -> Self {
        Self::with_registry(Registry::with_defaults(), rate, channels)
    }

    pub fn with_registry(registry: Registry, rate: Option<u32>, channels: usize) -> Self {
        let registry = Arc::new(registry);
        let (audio, slot) = AudioEngine::start_offline(registry.clone(), rate, channels);
        let mut player = Player::with_audio(Engine::default(), registry, audio);
        player.command(Command::SetVolume(255));
        Self {
            player,
            slot,
            ch: channels,
            timeline: Vec::new(),
            pulled: Vec::new(),
            events: Vec::new(),
            heard: Vec::new(),
            starving: false,
        }
    }

    pub fn set_prefs(&mut self, f: impl FnOnce(&mut OutputPrefs)) {
        let mut p = self.player.output_prefs();
        f(&mut p);
        self.player.set_output_prefs(p);
    }

    /// Adds files to the playlist and returns their entry ids.
    pub fn add(&mut self, paths: &[&Path]) -> Vec<EntryId> {
        self.player.engine.append(
            paths
                .iter()
                .map(|p| NewEntry::new(p.to_string_lossy()))
                .collect(),
        )
    }

    pub fn rate(&self) -> u32 {
        self.slot
            .lock()
            .expect("slot")
            .as_ref()
            .map(|d| d.rate)
            .unwrap_or(0)
    }

    pub fn pulled_frames(&self) -> usize {
        self.pulled.len() / self.ch
    }

    pub fn timeline_frames(&self) -> usize {
        self.timeline.len() / self.ch
    }

    pub fn saw(&self, e: &PlayerEvent) -> bool {
        self.events.iter().any(|(_, x)| x == e)
    }

    fn poll(&mut self) {
        let at = self.pulled_frames();
        for e in self.player.poll() {
            self.events.push((at, e));
        }
        let st = self.player.engine_status();
        if self.heard.last().map(|h| h.token) != Some(st.token) {
            self.heard.push(Heard {
                pulled: st.frames_played as usize,
                token: st.token,
                position_ms: st.position_ms,
            });
        }
    }

    fn buffered(&self) -> Option<(usize, usize)> {
        self.slot
            .lock()
            .expect("slot")
            .as_ref()
            .map(|d| (d.buffered_frames(), d.capacity_frames()))
    }

    /// Renders one device block. Waits first until the engine has filled the output buffer (or
    /// stops filling it), as it would with a device that plays in real time.
    pub fn step(&mut self) {
        let t = Instant::now();
        let limit = Duration::from_millis(if self.starving { 5 } else { 200 });
        let mut last = None;
        loop {
            self.poll();
            match self.buffered() {
                None => {
                    if t.elapsed() > Duration::from_secs(5) {
                        panic!("no output was opened");
                    }
                }
                Some((n, capacity)) => {
                    if n + 4096 >= capacity {
                        self.starving = false;
                        break;
                    }
                    if last.is_some_and(|l| n > l) {
                        self.starving = false;
                    }
                    last = Some(n);
                    if t.elapsed() > limit {
                        self.starving = true;
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_micros(300));
        }
        let mut buf = vec![0f32; BLOCK * self.ch];
        let got = self
            .slot
            .lock()
            .expect("slot")
            .as_mut()
            .expect("device")
            .render(&mut buf);
        self.timeline.extend_from_slice(&buf);
        self.pulled.extend_from_slice(&buf[..got * self.ch]);
        self.poll();
    }

    /// Steps until `done` holds; panics after `max_secs` of rendered audio.
    pub fn run_until(&mut self, max_secs: f32, mut done: impl FnMut(&Rig) -> bool) {
        while !done(self) {
            let rate = self.rate().max(8000) as f32;
            if self.timeline_frames() as f32 > max_secs * rate {
                panic!("condition not reached within {max_secs} s of audio");
            }
            self.step();
        }
    }

    /// Steps until playback stopped.
    pub fn run_to_stop(&mut self, max_secs: f32) {
        self.run_until(max_secs, |r| r.saw(&PlayerEvent::Stopped));
    }

    /// Steps until `secs` more seconds of audio were pulled.
    pub fn play_for(&mut self, secs: f32) {
        while self.rate() == 0 {
            self.step();
        }
        let rate = self.rate() as f32;
        let target = self.pulled_frames() + (secs * rate) as usize;
        self.run_until(
            secs * 4.0 + 10.0 + self.timeline_frames() as f32 / rate,
            |r| r.pulled_frames() >= target,
        );
    }
}

/// Channel `c` of interleaved samples.
pub fn channel(samples: &[f32], ch: usize, c: usize) -> Vec<f32> {
    samples.chunks_exact(ch).map(|f| f[c]).collect()
}

/// A fade in one channel of `samples`, searched from frame `from`: (first frame that left
/// `level`, first frame that reached `to`), comparing with a small tolerance.
pub fn find_fade(
    samples: &[f32],
    ch: usize,
    c: usize,
    from: usize,
    level: f32,
    to: f32,
) -> (usize, usize) {
    let v = channel(samples, ch, c);
    let tol = 1e-6;
    let start = (from..v.len())
        .find(|&i| (v[i] - level).abs() > tol)
        .expect("fade start");
    let end = (start..v.len())
        .find(|&i| (v[i] - to).abs() <= tol)
        .expect("fade end");
    (start, end)
}
