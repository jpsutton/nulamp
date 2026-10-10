//! The output-stage logic that runs in the device callback: fades, pause, prebuffering, volume
//! and balance (07 §6). Independent of the device library so it can be tested.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

/// Commands from the engine thread to the callback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MixCmd {
    /// Ramp the fade gain linearly to `to` over `ms` milliseconds (0 = jump).
    Fade { to: f32, ms: u32 },
    /// Pause once the fade gain reaches 0 (`true`), or resume (`false`).
    Pause(bool),
    /// Discard all buffered audio; acknowledged through `Shared::flush_ack`.
    Flush(u64),
    /// Output silence until this many frames are buffered (or the data ends).
    Prebuffer(u32),
    /// No more data will be written for now (end of the last track); stop waiting to prebuffer.
    EndOfData,
}

/// State shared between the engine thread and the callback.
#[derive(Debug, Default)]
pub struct Shared {
    /// 0..=255.
    pub volume: AtomicU32,
    /// −127..=127.
    pub balance: AtomicI32,
    /// Frames consumed from the ring buffer (heard position clock).
    pub frames_played: AtomicU64,
    /// Last acknowledged flush generation.
    pub flush_ack: AtomicU64,
    /// Current fade gain, f32 bits.
    pub fade_gain: AtomicU32,
    pub paused: AtomicBool,
    /// Callback ran out of data while not paused or prebuffering.
    pub underruns: AtomicU64,
}

impl Shared {
    pub fn new() -> Arc<Self> {
        let s = Self::default();
        s.volume.store(200, Ordering::Relaxed);
        s.fade_gain.store(1f32.to_bits(), Ordering::Relaxed);
        Arc::new(s)
    }

    pub fn fade_gain(&self) -> f32 {
        f32::from_bits(self.fade_gain.load(Ordering::Relaxed))
    }
}

/// Balance gains (left, right) for b in −127..=127 (07 §6.6).
pub fn balance_gains(b: i32) -> (f32, f32) {
    let b = b.clamp(-127, 127) as f32;
    if b > 0.0 {
        (1.0 - b / 128.0, 1.0)
    } else if b < 0.0 {
        (1.0, 1.0 + b / 128.0)
    } else {
        (1.0, 1.0)
    }
}

/// Callback state.
pub struct Mixer {
    shared: Arc<Shared>,
    rate: u32,
    channels: usize,
    fade: f32,
    fade_to: f32,
    fade_step: f32,
    pause_pending: bool,
    paused: bool,
    prebuffer: u32,
    /// Smoothed volume gain.
    vol: f32,
    vol_step: f32,
    scratch: Vec<f32>,
}

/// Volume change smoothing time (07 §6.1).
const VOLUME_RAMP_MS: f32 = 150.0;

impl Mixer {
    pub fn new(shared: Arc<Shared>, rate: u32, channels: usize) -> Self {
        let vol = shared.volume.load(Ordering::Relaxed) as f32 / 255.0;
        Self {
            shared,
            rate,
            channels: channels.max(1),
            fade: 1.0,
            fade_to: 1.0,
            fade_step: 0.0,
            pause_pending: false,
            paused: false,
            prebuffer: 0,
            vol,
            vol_step: 1.0 / (VOLUME_RAMP_MS / 1000.0 * rate as f32),
            scratch: Vec::new(),
        }
    }

    /// Applies a command. Returns the flush generation to acknowledge, if any.
    pub fn command(&mut self, cmd: MixCmd) -> Option<u64> {
        match cmd {
            MixCmd::Fade { to, ms } => {
                self.fade_to = to;
                if ms == 0 {
                    self.fade = to;
                    self.fade_step = 0.0;
                } else {
                    let frames = ms as f32 / 1000.0 * self.rate as f32;
                    self.fade_step = ((to - self.fade).abs() / frames).max(1e-7);
                }
            }
            MixCmd::Pause(true) => self.pause_pending = true,
            MixCmd::Pause(false) => {
                self.pause_pending = false;
                self.paused = false;
            }
            MixCmd::Flush(g) => return Some(g),
            MixCmd::Prebuffer(f) => self.prebuffer = f,
            MixCmd::EndOfData => self.prebuffer = 0,
        }
        None
    }

    /// Fills an interleaved device buffer. `pull` copies up to `n` samples from the ring into the
    /// slice it is given and returns how many it copied; `available` is the number of samples
    /// buffered.
    pub fn fill(
        &mut self,
        out: &mut [f32],
        available: usize,
        mut pull: impl FnMut(&mut [f32]) -> usize,
    ) {
        let ch = self.channels;
        let frames = out.len() / ch;
        let silent = |out: &mut [f32]| out.iter_mut().for_each(|s| *s = 0.0);
        if self.paused {
            silent(out);
            self.publish();
            return;
        }
        if self.prebuffer > 0 {
            if available / ch >= self.prebuffer as usize {
                self.prebuffer = 0;
            } else {
                silent(out);
                self.publish();
                return;
            }
        }
        self.scratch.resize(frames * ch, 0.0);
        let got = pull(&mut self.scratch[..frames * ch]) / ch;
        if got < frames {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.shared
            .frames_played
            .fetch_add(got as u64, Ordering::Relaxed);
        let target_vol = self.shared.volume.load(Ordering::Relaxed).min(255) as f32 / 255.0;
        let (gl, gr) = balance_gains(self.shared.balance.load(Ordering::Relaxed));
        for f in 0..frames {
            if self.fade != self.fade_to {
                if self.fade < self.fade_to {
                    self.fade = (self.fade + self.fade_step).min(self.fade_to);
                } else {
                    self.fade = (self.fade - self.fade_step).max(self.fade_to);
                }
            }
            if self.vol != target_vol {
                if self.vol < target_vol {
                    self.vol = (self.vol + self.vol_step).min(target_vol);
                } else {
                    self.vol = (self.vol - self.vol_step).max(target_vol);
                }
            }
            let g = self.fade * self.vol;
            for c in 0..ch {
                let s = if f < got {
                    self.scratch[f * ch + c]
                } else {
                    0.0
                };
                let bal = match c {
                    0 if ch >= 2 => gl,
                    1 => gr,
                    _ => 1.0,
                };
                out[f * ch + c] = s * g * bal;
            }
        }
        if self.pause_pending && self.fade <= 0.0 {
            self.pause_pending = false;
            self.paused = true;
        }
        self.publish();
    }

    fn publish(&self) {
        self.shared
            .fade_gain
            .store(self.fade.to_bits(), Ordering::Relaxed);
        self.shared.paused.store(self.paused, Ordering::Relaxed);
    }
}

/// TPDF dither and conversion of a [-1, 1) float to a signed integer of `bits` bits.
pub fn to_int_dithered(x: f32, bits: u32, rng: &mut u32) -> i32 {
    let mut next = || {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 17;
        *rng ^= *rng << 5;
        (*rng as f32 / u32::MAX as f32) - 0.5
    };
    let scale = (1i64 << (bits - 1)) as f32;
    let d = next() + next();
    let v = (x * scale + d).round();
    v.clamp(-scale, scale - 1.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(m: &mut Mixer, ring: &mut Vec<f32>, frames: usize) -> Vec<f32> {
        let mut out = vec![0f32; frames * 2];
        let avail = ring.len();
        m.fill(&mut out, avail, |dst| {
            let n = dst.len().min(ring.len());
            dst[..n].copy_from_slice(&ring[..n]);
            ring.drain(..n);
            n
        });
        out
    }

    #[test]
    fn volume_200_and_balance() {
        let shared = Shared::new();
        let mut m = Mixer::new(shared.clone(), 48_000, 2);
        let mut ring = vec![1.0f32; 2 * 1000];
        let out = run(&mut m, &mut ring, 1000);
        assert!((out[0] - 200.0 / 255.0).abs() < 1e-6);
        shared.balance.store(64, Ordering::Relaxed);
        let mut ring = vec![1.0f32; 2 * 10];
        let out = run(&mut m, &mut ring, 10);
        assert!((out[0] - 0.5 * 200.0 / 255.0).abs() < 1e-6);
        assert!((out[1] - 200.0 / 255.0).abs() < 1e-6);
        assert_eq!(shared.frames_played.load(Ordering::Relaxed), 1010);
    }

    #[test]
    fn fade_out_333ms_then_pause() {
        let shared = Shared::new();
        shared.volume.store(255, Ordering::Relaxed);
        let rate = 48_000;
        let mut m = Mixer::new(shared.clone(), rate, 2);
        m.command(MixCmd::Fade { to: 0.0, ms: 333 });
        m.command(MixCmd::Pause(true));
        let frames = (rate as f32 * 0.333) as usize;
        let mut ring = vec![1.0f32; 2 * (frames + 10)];
        let out = run(&mut m, &mut ring, frames + 5);
        // Linear ramp with no step larger than the ramp slope (no clicks).
        let max_step = out
            .chunks(2)
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[0][0] - w[1][0]).abs())
            .fold(0f32, f32::max);
        assert!(max_step < 1e-4, "{max_step}");
        assert!(out[(frames - 10) * 2] > 0.0);
        assert!(out[(frames + 4) * 2].abs() < 1e-6);
        assert!(shared.paused.load(Ordering::Relaxed));
        // Paused: consumes nothing.
        let before = ring.len();
        run(&mut m, &mut ring, 100);
        assert_eq!(ring.len(), before);
    }

    #[test]
    fn prebuffer_waits() {
        let shared = Shared::new();
        let mut m = Mixer::new(shared.clone(), 1000, 2);
        m.command(MixCmd::Prebuffer(500));
        let mut ring = vec![1.0f32; 2 * 100];
        let out = run(&mut m, &mut ring, 10);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(ring.len(), 200);
        m.command(MixCmd::EndOfData);
        let out = run(&mut m, &mut ring, 10);
        assert!(out[0] > 0.0);
    }

    #[test]
    fn dither_range() {
        let mut rng = 12345u32;
        assert!(to_int_dithered(0.0, 16, &mut rng).abs() <= 1);
        assert_eq!(to_int_dithered(1.0, 16, &mut rng), 32767);
        assert_eq!(to_int_dithered(-1.0, 16, &mut rng), -32768);
    }
}
