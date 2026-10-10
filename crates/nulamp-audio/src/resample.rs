//! Streaming sample-rate converter: windowed-sinc interpolation with a precomputed polyphase
//! table. Works on interleaved f32 with any channel count.

const HALF_TAPS: usize = 24;
const PHASES: usize = 512;

pub struct Resampler {
    /// Input frames per output frame.
    step: f64,
    channels: usize,
    /// `(PHASES + 1) × 2·HALF_TAPS` filter coefficients.
    table: Vec<f32>,
    /// Pending input, interleaved.
    buf: Vec<f32>,
    /// Position of the next output frame in `buf`, in input frames.
    pos: f64,
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let y = x * x / 4.0;
    for k in 1..50 {
        term *= y / (k * k) as f64;
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32, channels: usize) -> Self {
        let step = from_rate as f64 / to_rate as f64;
        let cutoff = if step > 1.0 { 0.95 / step } else { 0.95 };
        let beta = 8.0;
        let taps = 2 * HALF_TAPS;
        let mut table = vec![0f32; (PHASES + 1) * taps];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let mut sum = 0.0;
            for k in 0..taps {
                // Tap k sits at input offset (k - HALF_TAPS + 1) relative to floor(pos).
                let x = (k as f64 - HALF_TAPS as f64 + 1.0) - frac;
                let sinc = if x.abs() < 1e-9 {
                    1.0
                } else {
                    (std::f64::consts::PI * x * cutoff).sin() / (std::f64::consts::PI * x)
                };
                let w = {
                    let r = x / HALF_TAPS as f64;
                    if r.abs() >= 1.0 {
                        0.0
                    } else {
                        bessel_i0(beta * (1.0 - r * r).sqrt()) / bessel_i0(beta)
                    }
                };
                let v = sinc * w * if x.abs() < 1e-9 { cutoff } else { 1.0 };
                table[p * taps + k] = v as f32;
                sum += v;
            }
            // Normalise each phase to unity DC gain.
            for k in 0..taps {
                table[p * taps + k] /= sum as f32;
            }
        }
        Self {
            step,
            channels: channels.max(1),
            table,
            buf: vec![0.0; (HALF_TAPS - 1) * channels.max(1)],
            pos: (HALF_TAPS - 1) as f64,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.step == 1.0
    }

    /// Delay in output frames introduced by the filter.
    pub fn latency_frames(&self) -> usize {
        ((HALF_TAPS - 1) as f64 / self.step) as usize
    }

    /// Resamples `input` and appends the output to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let ch = self.channels;
        self.buf.extend_from_slice(input);
        let frames = self.buf.len() / ch;
        let taps = 2 * HALF_TAPS;
        loop {
            let base = self.pos.floor() as usize;
            if base + HALF_TAPS >= frames {
                break;
            }
            let frac = self.pos - base as f64;
            let phase_f = frac * PHASES as f64;
            let p = phase_f.floor() as usize;
            let t = (phase_f - p as f64) as f32;
            let (c0, c1) = (
                &self.table[p * taps..(p + 1) * taps],
                &self.table[(p + 1) * taps..(p + 2) * taps],
            );
            let start = base + 1 - HALF_TAPS;
            for c in 0..ch {
                let mut acc = 0f32;
                for k in 0..taps {
                    let coef = c0[k] + (c1[k] - c0[k]) * t;
                    acc += coef * self.buf[(start + k) * ch + c];
                }
                out.push(acc);
            }
            self.pos += self.step;
        }
        // Drop input no longer needed.
        let keep_from = (self.pos.floor() as usize)
            .saturating_sub(HALF_TAPS - 1)
            .min(frames);
        self.buf.drain(..keep_from * ch);
        self.pos -= keep_from as f64;
    }

    /// Flushes the filter's tail with silence (end of stream).
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        let silence = vec![0f32; HALF_TAPS * self.channels];
        self.process(&silence, out);
    }

    pub fn reset(&mut self) {
        let ch = self.channels;
        self.buf = vec![0.0; (HALF_TAPS - 1) * ch];
        self.pos = (HALF_TAPS - 1) as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * 0.5)
            .collect()
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn length_and_level() {
        for (from, to) in [
            (44_100, 48_000),
            (48_000, 44_100),
            (96_000, 48_000),
            (22_050, 48_000),
        ] {
            let mut r = Resampler::new(from, to, 1);
            let input = sine(from, 1000.0, from as usize);
            let mut out = Vec::new();
            for chunk in input.chunks(1000) {
                r.process(chunk, &mut out);
            }
            r.flush(&mut out);
            let expected = to as f32;
            assert!(
                (out.len() as f32 - expected).abs() < 40.0,
                "{from}->{to}: {}",
                out.len()
            );
            let mid = &out[out.len() / 4..out.len() * 3 / 4];
            let level = rms(mid) / (0.5 / 2f32.sqrt());
            assert!((level - 1.0).abs() < 0.02, "{from}->{to}: level {level}");
        }
    }

    #[test]
    fn stereo_channels_stay_separate() {
        let mut r = Resampler::new(44_100, 48_000, 2);
        let input: Vec<f32> = (0..4410).flat_map(|_| [0.5f32, -0.25]).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        let tail = &out[out.len() - 200..];
        for f in tail.chunks(2) {
            assert!((f[0] - 0.5).abs() < 1e-3 && (f[1] + 0.25).abs() < 1e-3);
        }
    }

    #[test]
    fn downsampling_suppresses_aliases() {
        // 30 kHz at 96 kHz must not alias into 48 kHz output.
        let mut r = Resampler::new(96_000, 48_000, 1);
        let mut out = Vec::new();
        r.process(&sine(96_000, 30_000.0, 48_000), &mut out);
        let mid = &out[2000..out.len() - 2000];
        assert!(rms(mid) < 0.01, "{}", rms(mid));
    }
}
