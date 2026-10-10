//! The EQ processor: combined gain stage, 10 filter sections and limiter (03 §4, 07 §4).
//!
//! Each section adds a band-pass component to the dry signal: y = x + g × bandpass(x), with
//! g = 10^(dB / 20) − 1. The band-pass is the constant 0 dB peak gain design from the audio
//! EQ cookbook, so the section gain at the centre frequency is exactly 10^(dB / 20).
//!
//! The constant-Q engine (03 §4.3) is the same structure: its analog prototype
//! (s² + (G/Q)s + 1) / (s² + (1/Q)s + 1) equals 1 + (G − 1) × (s/Q) / (s² + s/Q + 1), and the
//! bilinear transform with pre-warping of that band-pass is the cookbook band-pass.

use crate::limiter::Limiter;
use crate::settings::{EqSettings, FilterEngine};
use crate::value::{BANDS, band_db, preamp_gain};

/// Maximum interleaved channel count.
pub const MAX_CHANNELS: usize = 8;

/// Lowest sample rate used for filter design, Hz.
pub const MIN_DESIGN_RATE: u32 = 4000;

/// Highest sample rate used for filter design, Hz.
pub const MAX_DESIGN_RATE: u32 = 384_000;

/// Lowest band centre frequency, Hz.
pub const MIN_BAND_FREQ: f64 = 20.0;

/// Bands whose centre is at or above this fraction of the sample rate are disabled.
pub const MAX_BAND_RATIO: f64 = 0.499;

/// Base Q of the default design; boost uses 2 × Q, cut uses Q / 2.
pub const BASE_Q: f64 = 1.41;

/// Constant-Q bands with |G − 1| below this are bypassed.
pub const CONSTANT_Q_BYPASS: f32 = 0.02;

/// Time constant of gain changes, seconds (decision EQ2).
const SMOOTH_SECONDS: f64 = 0.01;

/// A smoothed gain closer than this to its target snaps to it.
const SNAP: f32 = 1e-6;

/// Filter states below this magnitude are flushed to zero after each block (decision EQ4).
const FLUSH: f32 = 1e-15;

/// Normalised band-pass coefficients. b1 = 0 and b2 = −b0 for this design.
#[derive(Debug, Clone, Copy, Default)]
struct Bandpass {
    b0: f32,
    a1: f32,
    a2: f32,
}

impl Bandpass {
    fn design(freq: f64, q: f64, rate: f64) -> Self {
        let w0 = std::f64::consts::TAU * freq / rate;
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: (alpha / a0) as f32,
            a1: (-2.0 * w0.cos() / a0) as f32,
            a2: ((1.0 - alpha) / a0) as f32,
        }
    }
}

/// A gain that moves towards its target with a one-pole curve.
#[derive(Debug, Clone, Copy)]
struct Smoothed {
    cur: f32,
    target: f32,
}

impl Smoothed {
    const fn new(v: f32) -> Self {
        Self { cur: v, target: v }
    }

    #[inline]
    fn step(&mut self, k: f32) -> f32 {
        if self.cur != self.target {
            self.cur += (self.target - self.cur) * k;
            if (self.target - self.cur).abs() < SNAP {
                self.cur = self.target;
            }
        }
        self.cur
    }

    fn snap(&mut self) {
        self.cur = self.target;
    }
}

/// One band-pass design inside a band, with its own gain.
#[derive(Debug, Clone, Copy)]
struct Path {
    coef: Bandpass,
    gain: Smoothed,
    running: bool,
}

impl Path {
    const fn new() -> Self {
        Self {
            coef: Bandpass {
                b0: 0.0,
                a1: 0.0,
                a2: 0.0,
            },
            gain: Smoothed::new(0.0),
            running: false,
        }
    }
}

/// Per-channel history: shared input history and the output history of each path.
#[derive(Debug, Clone, Copy, Default)]
struct ChannelState {
    x1: f32,
    x2: f32,
    boost: [f32; 2],
    cut: [f32; 2],
}

#[derive(Debug, Clone)]
struct Band {
    enabled: bool,
    boost: Path,
    cut: Path,
    state: [ChannelState; MAX_CHANNELS],
}

impl Band {
    fn new() -> Self {
        Self {
            enabled: false,
            boost: Path::new(),
            cut: Path::new(),
            state: [ChannelState::default(); MAX_CHANNELS],
        }
    }

    fn running(&self) -> bool {
        self.boost.running || self.cut.running
    }

    /// Starts paths whose target became non-zero, clearing the history they need.
    fn wake(&mut self) {
        let was_running = self.running();
        if !was_running && (self.boost.gain.target != 0.0 || self.cut.gain.target != 0.0) {
            self.state = [ChannelState::default(); MAX_CHANNELS];
        }
        if !self.boost.running && self.boost.gain.target != 0.0 {
            self.boost.running = true;
            if was_running {
                self.state.iter_mut().for_each(|s| s.boost = [0.0; 2]);
            }
        }
        if !self.cut.running && self.cut.gain.target != 0.0 {
            self.cut.running = true;
            if was_running {
                self.state.iter_mut().for_each(|s| s.cut = [0.0; 2]);
            }
        }
    }

    /// Stops paths that have faded to zero and flushes tiny states.
    fn settle(&mut self) {
        for p in [&mut self.boost, &mut self.cut] {
            if p.gain.cur == 0.0 && p.gain.target == 0.0 {
                p.running = false;
            }
        }
        let flush = |v: &mut f32| {
            if v.abs() < FLUSH {
                *v = 0.0;
            }
        };
        for s in &mut self.state {
            flush(&mut s.x1);
            flush(&mut s.x2);
            s.boost.iter_mut().chain(s.cut.iter_mut()).for_each(flush);
        }
    }

    #[inline]
    fn process_frame(&mut self, frame: &mut [f32], k: f32) {
        let gb = self.boost.gain.step(k);
        let gc = self.cut.gain.step(k);
        let (rb, rc) = (self.boost.running, self.cut.running);
        let (cb, cc) = (self.boost.coef, self.cut.coef);
        for (s, st) in frame.iter_mut().zip(self.state.iter_mut()) {
            let x = *s;
            let d = x - st.x2;
            let mut y = x;
            if rb {
                let v = cb.b0 * d - cb.a1 * st.boost[0] - cb.a2 * st.boost[1];
                st.boost = [v, st.boost[0]];
                y += gb * v;
            }
            if rc {
                let v = cc.b0 * d - cc.a1 * st.cut[0] - cc.a2 * st.cut[1];
                st.cut = [v, st.cut[0]];
                y += gc * v;
            }
            st.x2 = st.x1;
            st.x1 = x;
            *s = y;
        }
    }
}

/// The EQ processor for interleaved `f32` audio with 1..8 channels.
///
/// Stage order: combined gain (EQ preamp when active × [`Self::set_pre_gain`]), the 10 band
/// sections, then the limiter. The bands and limiter are bypassed when the EQ is off or flat
/// (03 §4.5); the pre-gain still applies. Gain changes glide over about 10 ms without
/// resetting filter state, so they do not click.
#[derive(Debug, Clone)]
pub struct Equalizer {
    sample_rate: u32,
    channels: usize,
    settings: EqSettings,
    bands: [Band; BANDS],
    pre_gain: f32,
    gain: Smoothed,
    smooth_k: f32,
    limiter: Limiter,
}

impl Equalizer {
    /// Creates a processor with default (off) settings.
    ///
    /// # Panics
    ///
    /// If `channels` is not in 1..=8.
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let mut eq = Self {
            sample_rate,
            channels,
            settings: EqSettings::default(),
            bands: std::array::from_fn(|_| Band::new()),
            pre_gain: 1.0,
            gain: Smoothed::new(1.0),
            smooth_k: 0.0,
            limiter: Limiter::new(sample_rate),
        };
        eq.set_format(sample_rate, channels);
        eq
    }

    /// Changes the stream format: rebuilds coefficients and clears all filter state.
    ///
    /// # Panics
    ///
    /// If `channels` is not in 1..=8.
    pub fn set_format(&mut self, sample_rate: u32, channels: usize) {
        assert!(
            (1..=MAX_CHANNELS).contains(&channels),
            "channel count {channels} not in 1..={MAX_CHANNELS}"
        );
        self.sample_rate = sample_rate;
        self.channels = channels;
        let sr = f64::from(sample_rate.max(1));
        self.smooth_k = (1.0 - (-1.0 / (SMOOTH_SECONDS * sr)).exp()) as f32;
        self.limiter.set_sample_rate(sample_rate);
        self.rebuild();
        self.update_targets();
        self.reset();
    }

    /// Sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Interleaved channel count.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Current settings.
    pub fn settings(&self) -> &EqSettings {
        &self.settings
    }

    /// Applies new settings. Values above 63 are clamped.
    ///
    /// Gains glide to their new values; filter state is kept. Coefficients are rebuilt when
    /// the band mode or engine changes.
    pub fn set_settings(&mut self, settings: &EqSettings) {
        let mut s = *settings;
        s.clamp();
        let redesign = s.iso != self.settings.iso || s.engine != self.settings.engine;
        self.settings = s;
        if redesign {
            self.rebuild();
        }
        self.update_targets();
    }

    /// Sets the extra linear gain folded into the gain stage (ReplayGain × its preamp, 07 §4).
    ///
    /// Applies whether or not the EQ is active. Non-finite or negative values count as 1.0.
    pub fn set_pre_gain(&mut self, gain: f32) {
        self.pre_gain = if gain.is_finite() && gain >= 0.0 {
            gain
        } else {
            1.0
        };
        self.update_targets();
    }

    /// The extra gain set with [`Self::set_pre_gain`].
    pub fn pre_gain(&self) -> f32 {
        self.pre_gain
    }

    /// True when the band sections and limiter process audio (EQ on and not flat).
    pub fn is_active(&self) -> bool {
        self.settings.is_active()
    }

    /// False when band `index` is above the usable range for this sample rate.
    pub fn band_enabled(&self, index: usize) -> bool {
        self.bands.get(index).is_some_and(|b| b.enabled)
    }

    /// Clears filter and limiter state and jumps all gains to their targets.
    pub fn reset(&mut self) {
        self.gain.snap();
        for b in &mut self.bands {
            b.boost.gain.snap();
            b.cut.gain.snap();
            b.boost.running = false;
            b.cut.running = false;
            b.state = [ChannelState::default(); MAX_CHANNELS];
        }
        self.limiter.reset();
    }

    /// Processes interleaved samples in place. A trailing partial frame is left unchanged.
    pub fn process(&mut self, buf: &mut [f32]) {
        for b in &mut self.bands {
            b.wake();
        }
        let any_band = self.bands.iter().any(Band::running);
        let limit = self.settings.limiter && (self.settings.is_active() || any_band);
        if !limit {
            self.limiter.reset();
        }
        let gain_idle = self.gain.cur == 1.0 && self.gain.target == 1.0;
        if !any_band && !limit && gain_idle {
            return;
        }
        let k = self.smooth_k;
        for frame in buf.chunks_exact_mut(self.channels) {
            let g = self.gain.step(k);
            if g != 1.0 {
                frame.iter_mut().for_each(|s| *s *= g);
            }
            if any_band {
                for band in self.bands.iter_mut().filter(|b| b.running()) {
                    band.process_frame(frame, k);
                }
            }
            if limit {
                self.limiter.process_frame(frame);
            }
        }
        for b in &mut self.bands {
            b.settle();
        }
    }

    fn rebuild(&mut self) {
        let rate = f64::from(self.sample_rate.clamp(MIN_DESIGN_RATE, MAX_DESIGN_RATE));
        let (q_boost, q_cut) = match self.settings.engine {
            FilterEngine::Default => (BASE_Q * 2.0, BASE_Q * 0.5),
            FilterEngine::ConstantQ => (BASE_Q, BASE_Q),
        };
        let centres = self.settings.band_mode().centres();
        for (band, &centre) in self.bands.iter_mut().zip(centres) {
            let f = f64::from(centre).max(MIN_BAND_FREQ);
            band.enabled = f < MAX_BAND_RATIO * rate;
            if band.enabled {
                band.boost.coef = Bandpass::design(f, q_boost, rate);
                band.cut.coef = Bandpass::design(f, q_cut, rate);
            } else {
                *band = Band::new();
            }
        }
    }

    fn update_targets(&mut self) {
        let s = &self.settings;
        let active = s.is_active();
        let preamp = if active { preamp_gain(s.preamp) } else { 1.0 };
        self.gain.target = self.pre_gain * preamp;
        for (band, &value) in self.bands.iter_mut().zip(&s.bands) {
            let (boost, cut) = if active && band.enabled {
                let big_g = 10f32.powf(band_db(value) / 20.0);
                let g = big_g - 1.0;
                match s.engine {
                    FilterEngine::Default if g > 0.0 => (g, 0.0),
                    FilterEngine::Default => (0.0, g),
                    FilterEngine::ConstantQ if g.abs() < CONSTANT_Q_BYPASS => (0.0, 0.0),
                    FilterEngine::ConstantQ => (g, 0.0),
                }
            } else {
                (0.0, 0.0)
            };
            band.boost.gain.target = boost;
            band.cut.gain.target = cut;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limiter::LIMITER_THRESHOLD;
    use crate::value::BandMode;

    fn sine(freq: f64, rate: u32, amp: f32, frames: usize, channels: usize) -> Vec<f32> {
        let w = std::f64::consts::TAU * freq / f64::from(rate);
        (0..frames * channels)
            .map(|i| amp * ((i / channels) as f64 * w).sin() as f32)
            .collect()
    }

    fn noise(len: usize, amp: f32) -> Vec<f32> {
        let mut x: u32 = 0x1234_5678;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                amp * (x as f32 / u32::MAX as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    fn run(eq: &mut Equalizer, buf: &mut [f32]) {
        let block = 512 * eq.channels();
        for chunk in buf.chunks_mut(block) {
            eq.process(chunk);
        }
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    fn one_band(index: usize, value: u8, iso: bool, engine: FilterEngine) -> EqSettings {
        let mut s = EqSettings {
            enabled: true,
            iso,
            engine,
            ..EqSettings::default()
        };
        s.bands[index] = value;
        s
    }

    fn centre_gain_db(rate: u32, s: &EqSettings, index: usize) -> f64 {
        let f = f64::from(s.band_mode().centres()[index]);
        let mut eq = Equalizer::new(rate, 1);
        eq.set_settings(s);
        let input = sine(f, rate, 0.1, rate as usize * 3 / 2, 1);
        let mut out = input.clone();
        run(&mut eq, &mut out);
        let from = rate as usize;
        20.0 * (rms(&out[from..]) / rms(&input[from..])).log10()
    }

    #[test]
    fn response_at_each_centre() {
        for engine in [FilterEngine::Default, FilterEngine::ConstantQ] {
            for iso in [false, true] {
                for rate in [44100, 48000] {
                    for index in 0..BANDS {
                        for (value, want) in [(0u8, 12.0), (63, -12.0)] {
                            let s = one_band(index, value, iso, engine);
                            let got = centre_gain_db(rate, &s, index);
                            assert!(
                                (got - want).abs() <= 0.3,
                                "{engine:?} iso={iso} {rate} Hz band {index} value {value}: {got:.3} dB"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn high_bands_disabled_at_low_rate() {
        let mut s = EqSettings {
            enabled: true,
            limiter: false,
            ..EqSettings::default()
        };
        s.bands = [0; BANDS];
        let mut eq = Equalizer::new(22050, 2);
        eq.set_settings(&s);
        assert!(eq.band_enabled(6));
        assert!(!eq.band_enabled(7) && !eq.band_enabled(8) && !eq.band_enabled(9));
        let mut buf = noise(22050 * 2 * 2, 0.05);
        run(&mut eq, &mut buf);
        assert!(buf.iter().all(|v| v.is_finite() && v.abs() < 10.0));
        let tail = &buf[buf.len() / 2..];
        assert!(rms(tail) > 0.0);

        let mut silence = vec![0.0f32; 22050 * 2];
        run(&mut eq, &mut silence);
        assert!(silence[silence.len() - 2..].iter().all(|v| v.abs() < 1e-6));
    }

    #[test]
    fn extreme_rates_are_stable() {
        let mut s = EqSettings {
            enabled: true,
            ..EqSettings::default()
        };
        s.bands = [0; BANDS];
        for rate in [1000, 8000, 384_000, 768_000] {
            for engine in [FilterEngine::Default, FilterEngine::ConstantQ] {
                s.engine = engine;
                let mut eq = Equalizer::new(rate, 1);
                eq.set_settings(&s);
                let mut buf = noise(rate as usize / 2, 0.1);
                run(&mut eq, &mut buf);
                assert!(
                    buf.iter()
                        .all(|v| v.is_finite() && v.abs() <= LIMITER_THRESHOLD)
                );
            }
        }
    }

    #[test]
    fn limiter_caps_full_scale_input() {
        let mut s = EqSettings {
            enabled: true,
            ..EqSettings::default()
        };
        s.bands = [0; BANDS];
        for preamp in [31, 0] {
            s.preamp = preamp;
            for f in [70.0, 1000.0, 14000.0] {
                let mut eq = Equalizer::new(44100, 2);
                eq.set_settings(&s);
                let mut buf = sine(f, 44100, 1.0, 44100, 2);
                run(&mut eq, &mut buf);
                let peak = buf.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(peak <= LIMITER_THRESHOLD, "{f} Hz peak {peak}");
                assert!(peak > 0.9);
            }
        }
        s.limiter = false;
        let mut eq = Equalizer::new(44100, 1);
        eq.set_settings(&s);
        let mut buf = sine(1000.0, 44100, 1.0, 44100, 1);
        run(&mut eq, &mut buf);
        assert!(buf.iter().any(|v| v.abs() > 2.0));
    }

    #[test]
    fn bypass_leaves_samples_untouched() {
        let input = noise(4096, 0.5);
        let s = EqSettings {
            bands: [0; BANDS],
            ..EqSettings::default()
        };
        let mut eq = Equalizer::new(44100, 2);
        eq.set_settings(&s);
        let mut buf = input.clone();
        eq.process(&mut buf);
        assert_eq!(buf, input);

        let flat = EqSettings {
            enabled: true,
            ..EqSettings::default()
        };
        eq.set_settings(&flat);
        assert!(!eq.is_active());
        eq.process(&mut buf);
        assert_eq!(buf, input);
    }

    #[test]
    fn pre_gain_applies_when_eq_off() {
        let mut eq = Equalizer::new(48000, 1);
        eq.set_pre_gain(0.5);
        eq.reset();
        let mut buf = vec![0.8f32; 64];
        eq.process(&mut buf);
        assert!(buf.iter().all(|&v| (v - 0.4).abs() < 1e-6));
    }

    #[test]
    fn preamp_only_when_active() {
        let s = EqSettings {
            enabled: true,
            preamp: 63,
            ..EqSettings::default()
        };
        let mut eq = Equalizer::new(48000, 1);
        eq.set_settings(&s);
        eq.set_pre_gain(2.0);
        eq.reset();
        let mut buf = vec![0.4f32; 64];
        eq.process(&mut buf);
        assert!(buf.iter().all(|&v| (v - 0.2).abs() < 1e-6));
    }

    #[test]
    fn channels_are_independent() {
        let rate = 48000;
        let mut s = EqSettings {
            enabled: true,
            limiter: false,
            ..EqSettings::default()
        };
        s.bands = [0, 10, 20, 31, 40, 50, 63, 5, 31, 45];
        let mono: Vec<Vec<f32>> = (0..MAX_CHANNELS)
            .map(|c| sine(100.0 + 900.0 * c as f64, rate, 0.05, 4096, 1))
            .collect();
        let mut inter: Vec<f32> = (0..4096)
            .flat_map(|i| mono.iter().map(move |m| m[i]))
            .collect();
        let mut eq = Equalizer::new(rate, MAX_CHANNELS);
        eq.set_settings(&s);
        eq.reset();
        eq.process(&mut inter);
        let mut ref_eq = Equalizer::new(rate, 1);
        ref_eq.set_settings(&s);
        for (c, m) in mono.iter().enumerate() {
            let mut m = m.clone();
            ref_eq.reset();
            ref_eq.process(&mut m);
            for (i, v) in m.iter().enumerate() {
                assert!((inter[i * MAX_CHANNELS + c] - v).abs() < 1e-6);
            }
        }
    }

    fn max_step(x: &[f32]) -> f32 {
        x.windows(2).fold(0.0, |m, w| m.max((w[1] - w[0]).abs()))
    }

    #[test]
    fn gain_changes_do_not_click() {
        let rate = 44100;
        let f = 100.0;
        let amp = 0.2;
        let mut eq = Equalizer::new(rate, 1);
        let mut on = EqSettings {
            enabled: true,
            limiter: false,
            ..EqSettings::default()
        };
        on.bands[0] = 0;
        on.bands[1] = 0;
        on.preamp = 0;
        let mut cut = on;
        cut.bands = [63; BANDS];
        cut.preamp = 63;
        let mut out = Vec::new();
        let input = sine(f, rate, amp, rate as usize, 1);
        let steps = [on, cut, EqSettings::default(), on, cut, on];
        for (chunk, s) in input.chunks(rate as usize / steps.len()).zip(steps) {
            eq.set_settings(&s);
            let mut c = chunk.to_vec();
            eq.process(&mut c);
            out.extend(c);
        }
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let natural = (std::f64::consts::TAU * f / f64::from(rate)) as f32 * peak;
        let step = max_step(&out);
        assert!(step < natural * 1.5, "step {step} vs natural {natural}");
    }

    #[test]
    fn silence_stays_clean() {
        let mut s = EqSettings {
            enabled: true,
            ..EqSettings::default()
        };
        s.bands = [0; BANDS];
        let mut eq = Equalizer::new(44100, 2);
        eq.set_settings(&s);
        let mut buf = noise(8192, 0.5);
        eq.process(&mut buf);
        let mut silence = vec![0.0f32; 44100 * 4];
        run(&mut eq, &mut silence);
        let tail = &silence[silence.len() - 1024..];
        assert!(tail.iter().all(|v| *v == 0.0 || v.is_normal()));
    }

    #[test]
    fn iso_mode_switch_keeps_running() {
        let mut s = one_band(0, 0, false, FilterEngine::Default);
        let mut eq = Equalizer::new(48000, 1);
        eq.set_settings(&s);
        let mut buf = sine(1000.0, 48000, 0.1, 4800, 1);
        eq.process(&mut buf);
        s.iso = true;
        eq.set_settings(&s);
        assert_eq!(eq.settings().band_mode(), BandMode::Iso);
        eq.process(&mut buf);
        assert!(buf.iter().all(|v| v.is_finite()));
    }
}
