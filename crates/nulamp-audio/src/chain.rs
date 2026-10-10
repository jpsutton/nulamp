//! Processing stages between the decoder and the output (07 §4), and the visualization tap.

use std::collections::VecDeque;
use std::sync::Mutex;

/// A processing stage (equalizer, DSP effect). Runs on the audio engine thread.
pub trait Stage: Send {
    /// Called when the stream format changes.
    fn configure(&mut self, sample_rate: u32, channels: usize);
    /// Processes interleaved samples in place.
    fn process(&mut self, samples: &mut [f32]);
    /// Clears internal state after a seek.
    fn reset(&mut self) {}
}

/// One block of audio for visualizers, stamped with the output frame at which it is heard.
#[derive(Debug, Clone)]
pub struct VisBlock {
    pub frame: u64,
    pub rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

/// Recent processed audio, after EQ and DSP and before volume (06 §2.1).
#[derive(Debug, Default)]
pub struct VisTap {
    blocks: Mutex<VecDeque<VisBlock>>,
}

/// Frames per visualizer block (06 §2.1).
pub const VIS_BLOCK: usize = 576;

impl VisTap {
    pub fn push(&self, block: VisBlock) {
        let mut b = self.blocks.lock().expect("vis tap lock");
        b.push_back(block);
        while b.len() > 256 {
            b.pop_front();
        }
    }

    pub fn clear(&self) {
        self.blocks.lock().expect("vis tap lock").clear();
    }

    /// The block whose time stamp is closest to `frame`, if within `max_frames`.
    pub fn closest(&self, frame: u64, max_frames: u64) -> Option<VisBlock> {
        let b = self.blocks.lock().expect("vis tap lock");
        b.iter()
            .min_by_key(|v| v.frame.abs_diff(frame))
            .filter(|v| v.frame.abs_diff(frame) <= max_frames)
            .cloned()
    }
}

/// Maps interleaved audio from `from` channels to `to` channels.
pub fn map_channels(input: &[f32], from: usize, to: usize, out: &mut Vec<f32>) {
    if from == to {
        out.extend_from_slice(input);
        return;
    }
    let frames = input.len() / from.max(1);
    for f in 0..frames {
        let s = &input[f * from..(f + 1) * from];
        match (from, to) {
            (1, _) => out.extend(std::iter::repeat_n(s[0], to)),
            (_, 1) => out.push(s.iter().sum::<f32>() / from as f32),
            (_, 2) => {
                // Front pair plus centre and surrounds at −3 dB.
                let k = std::f32::consts::FRAC_1_SQRT_2;
                let (mut l, mut r) = (s[0], s[1]);
                if from >= 3 {
                    l += k * s[2];
                    r += k * s[2];
                }
                if from >= 6 {
                    l += k * s[4];
                    r += k * s[5];
                }
                let norm = if from >= 3 {
                    1.0 / (1.0 + k * if from >= 6 { 2.0 } else { 1.0 })
                } else {
                    1.0
                };
                out.push(l * norm);
                out.push(r * norm);
            }
            _ => {
                out.extend(s.iter().copied().chain(std::iter::repeat(0.0)).take(to));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_mapping() {
        let mut out = Vec::new();
        map_channels(&[0.5, 0.25], 1, 2, &mut out);
        assert_eq!(out, [0.5, 0.5, 0.25, 0.25]);
        let mut out = Vec::new();
        map_channels(&[1.0, -1.0], 2, 1, &mut out);
        assert_eq!(out, [0.0]);
        let mut out = Vec::new();
        map_channels(&[1.0, 1.0, 1.0, 0.0, 1.0, 1.0], 6, 2, &mut out);
        assert!(out.iter().all(|s| (*s - 1.0).abs() < 1e-6));
    }

    #[test]
    fn closest_block() {
        let t = VisTap::default();
        for f in [0u64, 576, 1152] {
            t.push(VisBlock {
                frame: f,
                rate: 44_100,
                channels: 2,
                samples: vec![],
            });
        }
        assert_eq!(t.closest(600, 1000).unwrap().frame, 576);
        assert!(t.closest(100_000, 1000).is_none());
    }
}
