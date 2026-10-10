//! Audio kept from before the processing stages, with the stages' state along it, so that the
//! buffered audio can be processed again when the stage settings change (decision EQ12).

use std::collections::VecDeque;

use crate::chain::StageState;

/// Most frames processed between two saved stage states.
pub const SAVE_FRAMES: usize = 512;

/// Output-format audio before the stages for output frames `start..end()`.
#[derive(Default)]
pub struct Retained {
    samples: VecDeque<f32>,
    /// Output frame of the first kept frame.
    start: u64,
    /// The stages' state before output frame `.0`, oldest first. The first is at `start`.
    saves: VecDeque<(u64, Vec<StageState>)>,
}

/// What [`Retained::take_from`] hands back for processing again.
pub struct Redo {
    /// The stages' state at the first frame.
    pub states: Vec<StageState>,
    /// Frame counts of the blocks as first processed.
    pub blocks: Vec<usize>,
}

impl Retained {
    pub fn clear(&mut self) {
        self.samples.clear();
        self.saves.clear();
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.saves.is_empty()
    }

    /// Output frame after the last kept frame.
    pub fn end(&self, ch: usize) -> u64 {
        self.start + (self.samples.len() / ch) as u64
    }

    /// Keeps a block about to be processed at output frame `frame`, with the stages' state
    /// before it. A block that does not follow the kept audio replaces it.
    pub fn push(&mut self, frame: u64, states: Vec<StageState>, block: &[f32], ch: usize) {
        if self.saves.is_empty() || frame != self.end(ch) {
            self.clear();
            self.start = frame;
        }
        self.saves.push_back((frame, states));
        self.samples.extend(block.iter().copied());
    }

    /// Drops what is no longer needed: blocks that end at or before output frame `heard`.
    pub fn trim(&mut self, heard: u64, ch: usize) {
        while self.saves.len() > 1 && self.saves[1].0 <= heard {
            self.saves.pop_front();
            let next = self.saves[0].0;
            self.samples.drain(..(next - self.start) as usize * ch);
            self.start = next;
        }
        if self.saves.len() == 1 && self.end(ch) <= heard {
            self.clear();
            self.start = heard;
        }
    }

    /// The first block start at or after output frame `frame`.
    pub fn block_from(&self, frame: u64) -> Option<u64> {
        self.saves.iter().map(|s| s.0).find(|&f| f >= frame)
    }

    /// Removes the audio from block start `frame` on, appending it to `samples`.
    pub fn take_from(&mut self, frame: u64, ch: usize, samples: &mut Vec<f32>) -> Option<Redo> {
        let i = self.saves.iter().position(|s| s.0 == frame)?;
        let end = self.end(ch);
        let mut saves = self.saves.split_off(i).into_iter();
        let (_, states) = saves.next().expect("save");
        let mut blocks = Vec::new();
        let mut at = frame;
        for (f, _) in saves {
            blocks.push((f - at) as usize);
            at = f;
        }
        blocks.push((end - at) as usize);
        samples.extend(self.samples.drain((frame - self.start) as usize * ch..));
        Some(Redo { states, blocks })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(v: u32) -> Vec<StageState> {
        vec![Box::new(v)]
    }

    fn block(from: u64, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| (from + i as u64) as f32).collect()
    }

    #[test]
    fn keeps_blocks_and_hands_them_back() {
        let mut r = Retained::default();
        r.push(100, state(1), &block(100, 10), 1);
        r.push(110, state(2), &block(110, 5), 1);
        r.push(115, state(3), &block(115, 7), 1);
        assert_eq!(r.end(1), 122);
        assert_eq!(r.block_from(111), Some(115));
        r.trim(112, 1);
        assert_eq!(r.block_from(0), Some(110));
        let mut samples = Vec::new();
        let redo = r.take_from(110, 1, &mut samples).expect("redo");
        assert_eq!(*redo.states[0].downcast_ref::<u32>().unwrap(), 2);
        assert_eq!(redo.blocks, [5, 7]);
        assert_eq!(samples, block(110, 12));
        assert!(r.is_empty());
        // A gap starts over.
        r.push(500, state(4), &block(500, 4), 1);
        r.push(600, state(5), &block(600, 4), 1);
        assert_eq!(r.block_from(0), Some(600));
        r.trim(700, 1);
        assert!(r.is_empty());
    }
}
