//! Peak limiter after the EQ bands (03 §4.4).

/// Output ceiling, about −0.6 dB full scale.
pub const LIMITER_THRESHOLD: f32 = 0.930;

/// Release time in seconds: the detector falls to 0.001 of its value in this time.
pub const LIMITER_RELEASE: f64 = 0.7;

/// Peak-detecting limiter.
///
/// One detector is shared by all channels of a frame, so the stereo image does not shift
/// while limiting (decision EQ3). Output peaks never exceed [`LIMITER_THRESHOLD`].
#[derive(Debug, Clone)]
pub struct Limiter {
    detect: f32,
    decay: f32,
}

impl Limiter {
    /// Creates a limiter for `sample_rate` frames per second.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            detect: 0.0,
            decay: Self::decay_for(sample_rate),
        }
    }

    /// Per-frame decay factor 0.001^(1 / (sample rate × 0.7)).
    pub fn decay_for(sample_rate: u32) -> f32 {
        let sr = f64::from(sample_rate.max(1));
        0.001f64.powf(1.0 / (sr * LIMITER_RELEASE)) as f32
    }

    /// Changes the sample rate and clears the detector.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.decay = Self::decay_for(sample_rate);
        self.detect = 0.0;
    }

    /// Clears the detector.
    pub fn reset(&mut self) {
        self.detect = 0.0;
    }

    /// Limits one interleaved frame in place.
    #[inline]
    pub fn process_frame(&mut self, frame: &mut [f32]) {
        let peak = frame.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        self.detect = self.detect.max(peak);
        if self.detect > LIMITER_THRESHOLD {
            let scale = LIMITER_THRESHOLD / self.detect;
            for s in frame.iter_mut() {
                *s = (*s * scale).clamp(-LIMITER_THRESHOLD, LIMITER_THRESHOLD);
            }
        }
        self.detect *= self.decay;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_reaches_one_thousandth_after_release() {
        let sr = 44100;
        let d = f64::from(Limiter::decay_for(sr));
        let after = d.powf(f64::from(sr) * LIMITER_RELEASE);
        assert!((after - 0.001).abs() < 1e-5, "{after}");
    }

    #[test]
    fn caps_peaks() {
        let mut l = Limiter::new(48000);
        let mut frame = [2.0f32, -1.5];
        l.process_frame(&mut frame);
        assert!(frame.iter().all(|s| s.abs() <= LIMITER_THRESHOLD));
        assert!((frame[0] / frame[1] + 2.0 / 1.5).abs() < 1e-6);
        let mut quiet = [0.1f32, 0.1];
        l.process_frame(&mut quiet);
        assert!(quiet[0] < 0.1);
    }
}
