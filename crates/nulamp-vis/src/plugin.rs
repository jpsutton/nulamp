//! Data handed to external visualization plugins (06 §7.1).

use crate::FFT_LEN;
use crate::analysis::to_signed_8bit;
use crate::fft::{BINS, Fft512};

/// Samples per channel of plugin waveform and spectrum data.
pub const PLUGIN_SAMPLES: usize = 576;

/// One render call's worth of plugin data. Only the first [`PluginData::channels`] rows are
/// meaningful.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginData {
    /// 1 or 2.
    pub channels: usize,
    /// Signed 8-bit waveform, 576 samples per channel.
    pub waveform: [[i8; PLUGIN_SAMPLES]; 2],
    /// Unsigned 8-bit magnitudes, low to high frequency, 576 values per channel.
    pub spectrum: [[u8; PLUGIN_SAMPLES]; 2],
    /// Full-resolution PCM in [−1, 1), 576 frames per channel.
    pub pcm: [[f32; PLUGIN_SAMPLES]; 2],
    /// Full-resolution magnitudes of bins 0..255 of a Hann-windowed 512-point FFT of the first
    /// 512 PCM frames, scaled so a sine of amplitude A centred on a bin reads A.
    pub fft: [[f32; BINS]; 2],
}

impl Default for PluginData {
    fn default() -> Self {
        Self {
            channels: 2,
            waveform: [[0; PLUGIN_SAMPLES]; 2],
            spectrum: [[0; PLUGIN_SAMPLES]; 2],
            pcm: [[0.0; PLUGIN_SAMPLES]; 2],
            fft: [[0.0; BINS]; 2],
        }
    }
}

/// Computes [`PluginData`] from one 576-frame block, reusing its buffers.
///
/// Channel selection: one requested channel is the average of all source channels; two
/// requested channels are source channels 0 and 1 (a mono source fills both). Frames past the
/// end of a short block are silent.
///
/// The 8-bit spectrum follows 06 §7.1: the first 512 waveform values of the channel, a DC
/// high-pass `y = x − x₋₁ + 0.99 y₋₁` started at `x₋₁ = x₀, y₋₁ = 0` (so a constant input gives
/// zero), the Hann window, the FFT, magnitude / 16 truncated and clamped to 255. Each bin i
/// gives `(m[i] + m[i − 1]) / 2` (bin 0 pairs with itself) then `m[i]`; values 512..575 each
/// halve the one before.
#[derive(Debug)]
pub struct PluginAnalyzer {
    fft: Fft512,
    buf: [f32; FFT_LEN],
    mags: [f32; BINS],
    data: Box<PluginData>,
}

impl Default for PluginAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginAnalyzer {
    pub fn new() -> Self {
        Self {
            fft: Fft512::new(),
            buf: [0.0; FFT_LEN],
            mags: [0.0; BINS],
            data: Box::default(),
        }
    }

    /// Fills the data for an interleaved block of `channels` channels, giving `want` (1 or 2)
    /// channels to the plugin.
    pub fn compute(&mut self, samples: &[f32], channels: usize, want: usize) -> &PluginData {
        let want = want.clamp(1, 2);
        let channels = channels.max(1);
        let frames = samples.len() / channels;
        self.data.channels = want;
        let norm = 2.0 / self.fft.window_sum();
        for ch in 0..want {
            let pcm = &mut self.data.pcm[ch];
            for (i, p) in pcm.iter_mut().enumerate() {
                *p = if i >= frames {
                    0.0
                } else {
                    let f = &samples[i * channels..(i + 1) * channels];
                    if want == 1 {
                        f.iter().sum::<f32>() / channels as f32
                    } else {
                        f[ch.min(channels - 1)]
                    }
                };
            }
            for (w, &p) in self.data.waveform[ch].iter_mut().zip(pcm.iter()) {
                *w = to_signed_8bit(p);
            }

            self.buf.copy_from_slice(&pcm[..FFT_LEN]);
            self.fft.magnitudes(&self.buf, norm, &mut self.mags);
            self.data.fft[ch] = self.mags;

            let wave = &self.data.waveform[ch];
            let (mut px, mut py) = (wave[0] as f32, 0.0);
            for (b, &w) in self.buf.iter_mut().zip(wave.iter()) {
                let x = w as f32;
                py = x - px + 0.99 * py;
                px = x;
                *b = py;
            }
            self.fft.magnitudes(&self.buf, 1.0 / 16.0, &mut self.mags);
            let mut bins = [0u8; BINS];
            for (b, &m) in bins.iter_mut().zip(&self.mags) {
                *b = m.min(255.0) as u8;
            }
            expand_bins(&bins, &mut self.data.spectrum[ch]);
        }
        &self.data
    }
}

/// 256 bins to 576 values: (average with the previous bin, bin) pairs, then halving padding.
fn expand_bins(bins: &[u8; BINS], out: &mut [u8; PLUGIN_SAMPLES]) {
    for (i, &b) in bins.iter().enumerate() {
        let prev = bins[i.saturating_sub(1)];
        out[2 * i] = ((b as u16 + prev as u16) / 2) as u8;
        out[2 * i + 1] = b;
    }
    for i in 2 * BINS..PLUGIN_SAMPLES {
        out[i] = out[i - 1] / 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(bin: f32, amp: f32, channels: usize) -> Vec<f32> {
        (0..PLUGIN_SAMPLES * channels)
            .map(|i| {
                let n = (i / channels) as f32;
                amp * (2.0 * std::f32::consts::PI * bin * n / FFT_LEN as f32).sin()
            })
            .collect()
    }

    #[test]
    fn bins_expand_and_pad_by_halving() {
        let mut bins = [0u8; BINS];
        bins[0] = 10;
        bins[1] = 21;
        bins[255] = 200;
        bins[254] = 100;
        let mut out = [0u8; PLUGIN_SAMPLES];
        expand_bins(&bins, &mut out);
        assert_eq!(&out[..4], &[10, 10, 15, 21]);
        assert_eq!(&out[508..512], &[50, 100, 150, 200]);
        let pad: Vec<u8> = out[512..522].to_vec();
        assert_eq!(pad, [100, 50, 25, 12, 6, 3, 1, 0, 0, 0]);
        assert!(out[522..].iter().all(|&v| v == 0));
    }

    #[test]
    fn padding_halves_on_real_data() {
        let mut seed = 99u32;
        let noise: Vec<f32> = (0..PLUGIN_SAMPLES * 2)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 * 1.8 - 0.9
            })
            .collect();
        let mut p = PluginAnalyzer::new();
        let d = p.compute(&noise, 2, 2);
        for ch in 0..2 {
            let s = &d.spectrum[ch];
            assert!(s[511] > 0);
            for i in 512..PLUGIN_SAMPLES {
                assert_eq!(s[i], s[i - 1] / 2);
            }
        }
    }

    #[test]
    fn dc_is_removed() {
        let mut p = PluginAnalyzer::new();
        let d = p.compute(&[0.4; PLUGIN_SAMPLES], 1, 1);
        assert!(d.spectrum[0].iter().all(|&v| v == 0));
        assert!(d.waveform[0].iter().all(|&v| v == 51));
    }

    #[test]
    fn sine_peaks_at_its_bin() {
        let mut p = PluginAnalyzer::new();
        let d = p.compute(&sine(32.0, 0.05, 2), 2, 2);
        for ch in 0..2 {
            let s = &d.spectrum[ch];
            let (i, &max) = s.iter().enumerate().max_by_key(|e| *e.1).unwrap();
            assert_eq!(i, 65, "{:?}", &s[..80]);
            assert!((30..=70).contains(&max), "{max}");
            assert!((d.fft[ch][32] - 0.05).abs() < 0.002, "{}", d.fft[ch][32]);
        }
    }

    #[test]
    fn channel_selection() {
        let block: Vec<f32> = (0..PLUGIN_SAMPLES).flat_map(|_| [0.5, -0.25]).collect();
        let mut p = PluginAnalyzer::new();
        let d = p.compute(&block, 2, 2).clone();
        assert_eq!(d.channels, 2);
        assert!(d.waveform[0].iter().all(|&v| v == 64));
        assert!(d.waveform[1].iter().all(|&v| v == -32));
        let d = p.compute(&block, 2, 1).clone();
        assert_eq!(d.channels, 1);
        assert!(d.pcm[0].iter().all(|&v| v == 0.125));
        assert!(d.waveform[0].iter().all(|&v| v == 16));
        let d = p.compute(&[0.5; PLUGIN_SAMPLES], 1, 2);
        assert_eq!(d.waveform[0], d.waveform[1]);
        let d = p.compute(&[0.5; 10], 1, 1);
        assert_eq!(d.waveform[0][9], 64);
        assert_eq!(d.waveform[0][10], 0);
    }
}
