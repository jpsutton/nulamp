//! A reusable 512-point real FFT with a Hann window.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::FFT_LEN;

/// Half the FFT length: the number of magnitude bins used (0..255).
pub(crate) const BINS: usize = FFT_LEN / 2;

/// 512-point forward FFT with preallocated buffers.
pub(crate) struct Fft512 {
    plan: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    hann: [f32; FFT_LEN],
}

impl std::fmt::Debug for Fft512 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Fft512")
    }
}

impl Fft512 {
    pub(crate) fn new() -> Self {
        let plan = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_LEN);
        let mut hann = [0.0; FFT_LEN];
        for (i, h) in hann.iter_mut().enumerate() {
            let a = 2.0 * std::f64::consts::PI * i as f64 / (FFT_LEN - 1) as f64;
            *h = (0.5 - 0.5 * a.cos()) as f32;
        }
        Self {
            input: plan.make_input_vec(),
            output: plan.make_output_vec(),
            scratch: plan.make_scratch_vec(),
            plan,
            hann,
        }
    }

    /// Sum of the window, for amplitude normalisation.
    pub(crate) fn window_sum(&self) -> f32 {
        self.hann.iter().sum()
    }

    /// Applies `hann[i] = 0.5 − 0.5 cos(2π i / 511)` (06 §2.3), runs the unnormalised FFT and
    /// writes `|X[i]| × scale` for bins 0..255.
    pub(crate) fn magnitudes(
        &mut self,
        samples: &[f32; FFT_LEN],
        scale: f32,
        out: &mut [f32; BINS],
    ) {
        for ((d, &s), &w) in self.input.iter_mut().zip(samples).zip(&self.hann) {
            *d = s * w;
        }
        self.plan
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .expect("buffers sized by the plan");
        for (m, c) in out.iter_mut().zip(&self.output) {
            *m = c.norm() * scale;
        }
    }
}
