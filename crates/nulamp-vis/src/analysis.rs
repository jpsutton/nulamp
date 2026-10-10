//! Audio blocks to oscilloscope values and spectrum bars (06 §2.1–§2.3).

use std::collections::VecDeque;

use crate::fft::{BINS, Fft512};
use crate::{BARS, FFT_LEN};

/// Data older or newer than this, relative to the output position, is not shown (06 §2.1).
pub const IDLE_TIMEOUT_MS: u64 = 400;

const HOP: usize = FFT_LEN / 2;
const SPECTRUM_HISTORY: usize = 1024;
const SCOPE_HISTORY: usize = 512;

/// One frame of visualizer input: 75 bar values (0..255) and 75 scope values (about −16..16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisData {
    pub bars: [u8; BARS],
    pub scope: [i8; BARS],
}

impl Default for VisData {
    fn default() -> Self {
        Self {
            bars: [0; BARS],
            scope: [0; BARS],
        }
    }
}

/// Converts a sample in [−1, 1) to signed 8-bit: rounded to 16-bit, then the high byte.
pub fn to_signed_8bit(x: f32) -> i8 {
    let s = (x * 32768.0).round().clamp(-32768.0, 32767.0) as i32;
    (s >> 8) as i8
}

/// The 75 oscilloscope values of one interleaved block (06 §2.2).
///
/// Column x uses frame `floor(x × frames / 75)` (frames = 576 for a full block). Each
/// channel is reduced to signed 8-bit, the channels are averaged, divided by 8 and rounded
/// half away from zero, giving −16..16.
pub fn scope_values(samples: &[f32], channels: usize) -> [i8; BARS] {
    let mut out = [0; BARS];
    if channels == 0 {
        return out;
    }
    let frames = samples.len() / channels;
    for (x, o) in out.iter_mut().enumerate() {
        let i = x * frames / BARS;
        let sum: i32 = samples[i * channels..(i + 1) * channels]
            .iter()
            .map(|&s| to_signed_8bit(s) as i32)
            .sum();
        *o = (sum as f32 / (channels * 8) as f32).round() as i8;
    }
    out
}

/// The fractional bin range `[W(x) + 1, W(x + 1) + 1)` covered by bar `x` (06 §2.3 step 5).
pub fn bar_range(x: usize) -> (f64, f64) {
    let s = 255.0 / 2f64.powf(BARS as f64 / 12.0);
    let w = |x: f64| (2f64.powf(x / 12.0) - 1.0) * s;
    (w(x as f64) + 1.0, w(x as f64 + 1.0) + 1.0)
}

/// The classic 512-point spectrum: L+R window to 75 bars (06 §2.3 steps 2–5).
#[derive(Debug)]
pub struct Spectrum {
    fft: Fft512,
    mags: [f32; BINS],
    ranges: [(f32, f32); BARS],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self::new()
    }
}

impl Spectrum {
    pub fn new() -> Self {
        let mut ranges = [(0.0, 0.0); BARS];
        for (x, r) in ranges.iter_mut().enumerate() {
            let (lo, hi) = bar_range(x);
            *r = (lo as f32, hi as f32);
        }
        Self {
            fft: Fft512::new(),
            mags: [0.0; BINS],
            ranges,
        }
    }

    /// Bars from 512 samples of L + R (not yet windowed).
    pub fn compute(&mut self, sum_lr: &[f32; FFT_LEN]) -> [u8; BARS] {
        self.fft.magnitudes(sum_lr, 0.5, &mut self.mags);
        self.bars_from_magnitudes(&self.mags)
    }

    /// Maps 256 bin magnitudes to 75 bars: the sum of the covered bins, with partial edge
    /// bins interpolated (Catmull-Rom) and weighted by the covered fraction.
    ///
    /// The first partial bin is interpolated at the range start, the last at the range end; a
    /// range inside one bin uses its start. Neighbours outside 0..255 are clamped.
    pub fn bars_from_magnitudes(&self, m: &[f32; BINS]) -> [u8; BARS] {
        let mut out = [0; BARS];
        for (o, &(lo, hi)) in out.iter_mut().zip(&self.ranges) {
            *o = range_sum(m, lo, hi).round().clamp(0.0, 255.0) as u8;
        }
        out
    }
}

fn range_sum(m: &[f32; BINS], lo: f32, hi: f32) -> f32 {
    let b0 = lo.floor() as usize;
    let b1 = hi.floor() as usize;
    let t0 = lo - b0 as f32;
    if b0 == b1 {
        return catmull_rom(m, b0, t0) * (hi - lo);
    }
    let mut sum = catmull_rom(m, b0, t0) * (1.0 - t0);
    sum += m[b0 + 1..b1.min(BINS)].iter().sum::<f32>();
    let t1 = hi - b1 as f32;
    if t1 > 0.0 {
        sum += catmull_rom(m, b1, t1) * t1;
    }
    sum
}

fn catmull_rom(m: &[f32; BINS], b: usize, t: f32) -> f32 {
    let at = |i: isize| m[i.clamp(0, BINS as isize - 1) as usize];
    let b = b as isize;
    let (p0, p1, p2, p3) = (at(b - 1), at(b), at(b + 1), at(b + 2));
    0.5 * (2.0 * p1
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
}

/// Streaming analyzer: fed every audio block in order, it keeps time-stamped spectra and
/// scope values so the frame closest to the output position can be drawn (06 §2.1, §2.3).
///
/// Spectra use a 512-sample window advanced by 256 samples (50% overlap) across block
/// boundaries; each is stamped with the output frame of its first sample. Left is channel 0
/// and right is channel 1 (mono uses channel 0 twice); further channels are not used by the
/// spectrum. A block that repeats the previous block's stamp is ignored. A block that does not
/// follow the previous one restarts the window; a block stamped before the previous one, or a
/// sample-rate change, also clears the history.
#[derive(Debug)]
pub struct Analyzer {
    spectrum: Spectrum,
    window: [f32; FFT_LEN],
    fill: usize,
    window_frame: u64,
    rate: u32,
    last_block: Option<u64>,
    next_frame: u64,
    spectra: VecDeque<(u64, [u8; BARS])>,
    scopes: VecDeque<(u64, [i8; BARS])>,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            spectrum: Spectrum::new(),
            window: [0.0; FFT_LEN],
            fill: 0,
            window_frame: 0,
            rate: 0,
            last_block: None,
            next_frame: 0,
            spectra: VecDeque::with_capacity(SPECTRUM_HISTORY),
            scopes: VecDeque::with_capacity(SCOPE_HISTORY),
        }
    }

    /// Forgets all buffered audio and history (stop, seek).
    pub fn reset(&mut self) {
        self.fill = 0;
        self.last_block = None;
        self.spectra.clear();
        self.scopes.clear();
    }

    /// Feeds one interleaved block heard at output frame `frame`.
    pub fn push(&mut self, frame: u64, rate: u32, channels: usize, samples: &[f32]) {
        if channels == 0 || samples.len() < channels {
            return;
        }
        if rate != self.rate {
            self.reset();
            self.rate = rate;
        }
        match self.last_block {
            Some(last) if last == frame => return,
            Some(last) if frame < last => self.reset(),
            _ => {}
        }
        if self.last_block.is_none() || frame != self.next_frame {
            self.fill = 0;
        }
        let frames = samples.len() / channels;
        self.last_block = Some(frame);
        self.next_frame = frame + frames as u64;

        if self.scopes.len() == SCOPE_HISTORY {
            self.scopes.pop_front();
        }
        self.scopes
            .push_back((frame, scope_values(samples, channels)));

        for (i, f) in samples.chunks_exact(channels).enumerate() {
            if self.fill == 0 {
                self.window_frame = frame + i as u64;
            }
            let l = f[0];
            let r = if channels > 1 { f[1] } else { l };
            self.window[self.fill] = l + r;
            self.fill += 1;
            if self.fill == FFT_LEN {
                let bars = self.spectrum.compute(&self.window);
                if self.spectra.len() == SPECTRUM_HISTORY {
                    self.spectra.pop_front();
                }
                self.spectra.push_back((self.window_frame, bars));
                self.window.copy_within(HOP.., 0);
                self.fill = FFT_LEN - HOP;
                self.window_frame += HOP as u64;
            }
        }
    }

    /// Sample rate of the blocks fed so far (0 before the first block).
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The newest spectrum and scope values, if any.
    pub fn latest(&self) -> Option<VisData> {
        Some(VisData {
            bars: self.spectra.back()?.1,
            scope: self.scopes.back()?.1,
        })
    }

    /// The spectrum and scope values stamped closest to output frame `frame`, or `None` when
    /// nothing lies within [`IDLE_TIMEOUT_MS`] (the caller then shows the idle animation).
    pub fn at(&self, frame: u64) -> Option<VisData> {
        let max = self.rate as u64 * IDLE_TIMEOUT_MS / 1000;
        Some(VisData {
            bars: closest(&self.spectra, frame, max)?,
            scope: closest(&self.scopes, frame, max)?,
        })
    }
}

fn closest<T: Copy>(q: &VecDeque<(u64, T)>, frame: u64, max: u64) -> Option<T> {
    let i = q.partition_point(|e| e.0 < frame);
    let before = i.checked_sub(1).and_then(|j| q.get(j));
    [before, q.get(i)]
        .into_iter()
        .flatten()
        .min_by_key(|e| e.0.abs_diff(frame))
        .filter(|e| e.0.abs_diff(frame) <= max)
        .map(|e| e.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BLOCK_FRAMES;

    const RATE: u32 = 44_100;

    fn sine_blocks(freq: f32, amp: f32, blocks: usize) -> Analyzer {
        let mut a = Analyzer::new();
        let mut buf = vec![0.0; BLOCK_FRAMES * 2];
        for b in 0..blocks {
            for i in 0..BLOCK_FRAMES {
                let n = (b * BLOCK_FRAMES + i) as f32;
                let s = amp * (2.0 * std::f32::consts::PI * freq * n / RATE as f32).sin();
                buf[2 * i] = s;
                buf[2 * i + 1] = s;
            }
            a.push((b * BLOCK_FRAMES) as u64, RATE, 2, &buf);
        }
        a
    }

    fn bar_for_freq(freq: f64) -> usize {
        let bin = freq * FFT_LEN as f64 / RATE as f64;
        (0..BARS)
            .find(|&x| {
                let (lo, hi) = bar_range(x);
                lo <= bin && bin < hi
            })
            .unwrap()
    }

    #[test]
    fn bar_ranges() {
        let (lo, _) = bar_range(0);
        assert!((lo - 1.0).abs() < 1e-12);
        for x in 1..BARS {
            assert!((bar_range(x).0 - bar_range(x - 1).1).abs() < 1e-9);
        }
        let (_, hi) = bar_range(74);
        assert!((hi - 252.65).abs() < 0.01, "{hi}");
    }

    #[test]
    fn constant_magnitude_sums_covered_width() {
        let s = Spectrum::new();
        let bars = s.bars_from_magnitudes(&[1.0; BINS]);
        for (x, &b) in bars.iter().enumerate() {
            let (lo, hi) = bar_range(x);
            assert_eq!(b, (hi - lo).round() as u8, "bar {x}");
        }
        let bars = s.bars_from_magnitudes(&[100.0; BINS]);
        assert_eq!(bars[0], 20);
        assert_eq!(bars[74], 255);
    }

    #[test]
    fn sine_1khz_peaks_at_its_bar() {
        let a = sine_blocks(1000.0, 0.5, 8);
        let bars = a.latest().unwrap().bars;
        let expected = bar_for_freq(1000.0);
        assert_eq!(expected, 24);
        let (peak, &value) = bars.iter().enumerate().max_by_key(|e| *e.1).unwrap();
        assert_eq!(peak, expected, "{bars:?}");
        assert!((20..=60).contains(&value), "peak value {value}");
        assert!(bars[..20].iter().all(|&b| b == 0), "{bars:?}");
        assert!(bars[30..].iter().all(|&b| b <= 2), "{bars:?}");
    }

    #[test]
    fn white_noise_fills_the_range() {
        let mut a = Analyzer::new();
        let mut seed = 0x1234_5678u32;
        let mut buf = vec![0.0; BLOCK_FRAMES * 2];
        let blocks = 2 * RATE as usize / BLOCK_FRAMES;
        for b in 0..blocks {
            for s in buf.iter_mut() {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *s = (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            }
            a.push((b * BLOCK_FRAMES) as u64, RATE, 2, &buf);
        }
        let mut mean = [0.0f32; BARS];
        for (_, bars) in &a.spectra {
            for (m, &b) in mean.iter_mut().zip(bars) {
                *m += b as f32 / a.spectra.len() as f32;
            }
        }
        // Low bars cover a fraction of one bin each, so they stay small; summed high bars
        // read larger (06 §2.3).
        assert!(mean.iter().all(|&m| m > 0.5), "{mean:?}");
        assert!(mean.windows(10).all(|w| w[9] > w[0]), "{mean:?}");
        assert!(mean[60..].iter().all(|&m| m > 15.0), "{mean:?}");
        let bars = a.latest().unwrap().bars;
        assert!(bars.iter().filter(|&&b| b > 0).count() >= 60, "{bars:?}");
    }

    #[test]
    fn silence_gives_zero_bars() {
        let a = sine_blocks(1000.0, 0.0, 2);
        assert_eq!(a.latest().unwrap(), VisData::default());
    }

    #[test]
    fn eight_bit_conversion() {
        assert_eq!(to_signed_8bit(0.0), 0);
        assert_eq!(to_signed_8bit(0.5), 64);
        assert_eq!(to_signed_8bit(-0.5), -64);
        assert_eq!(to_signed_8bit(-1.0), -128);
        assert_eq!(to_signed_8bit(1.0), 127);
        assert_eq!(to_signed_8bit(-0.001), -1);
        assert_eq!(to_signed_8bit(0.001), 0);
    }

    #[test]
    fn scope_values_pick_nearest_sample_and_average() {
        let mut block = vec![0.0; BLOCK_FRAMES * 2];
        for i in 0..BLOCK_FRAMES {
            block[2 * i] = i as f32 / BLOCK_FRAMES as f32;
            block[2 * i + 1] = -0.25;
        }
        let v = scope_values(&block, 2);
        for (x, &s) in v.iter().enumerate() {
            let i = x * 576 / 75;
            let l = to_signed_8bit(i as f32 / 576.0) as f32;
            let expected = ((l - 32.0) / 16.0).round() as i8;
            assert_eq!(s, expected, "column {x}");
        }
        let mono = scope_values(&vec![-1.0; BLOCK_FRAMES], 1);
        assert!(mono.iter().all(|&s| s == -16));
        let mono = scope_values(&vec![0.999; BLOCK_FRAMES], 1);
        assert!(mono.iter().all(|&s| s == 16));
    }

    #[test]
    fn spectra_overlap_and_are_stamped() {
        let a = sine_blocks(440.0, 0.25, 4);
        let stamps: Vec<u64> = a.spectra.iter().map(|e| e.0).collect();
        let expected: Vec<u64> = (0..).map(|k| k * 256).take(stamps.len()).collect();
        assert_eq!(stamps, expected);
        assert_eq!(stamps.len(), (4 * BLOCK_FRAMES - 512) / 256 + 1);
    }

    #[test]
    fn at_selects_closest_within_timeout() {
        let a = sine_blocks(440.0, 0.25, 4);
        assert!(a.at(0).is_some());
        let last = a.spectra.back().unwrap().0.min(a.scopes.back().unwrap().0);
        let max = RATE as u64 * 400 / 1000;
        assert!(a.at(last + max).is_some());
        assert!(a.at(last + max + 1).is_none());
        let sel = a.at(300).unwrap();
        assert_eq!(sel.bars, a.spectra[1].1);
    }

    #[test]
    fn gaps_restart_window_and_duplicates_are_ignored() {
        let mut a = Analyzer::new();
        let block = vec![0.1; BLOCK_FRAMES];
        a.push(0, RATE, 1, &block);
        assert_eq!(a.spectra.len(), 1);
        a.push(0, RATE, 1, &block);
        assert_eq!(a.scopes.len(), 1);
        a.push(10_000, RATE, 1, &block);
        assert_eq!(a.spectra.back().unwrap().0, 10_000);
        a.push(5_000, RATE, 1, &block);
        assert_eq!(a.spectra.len(), 1);
        a.push(5_576, 48_000, 1, &block);
        assert_eq!(a.rate(), 48_000);
        assert_eq!(a.scopes.len(), 1);
    }
}
