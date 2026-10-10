//! Shuffle order (07 §3.2): a permutation of playlist indices with a cursor.

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Moved to a slot; the entry index in that slot.
    Moved(usize),
    /// Already at the last slot.
    EndReached,
    /// Already at slot 0.
    StartReached,
}

#[derive(Debug, Clone)]
pub struct Shuffle {
    order: Vec<usize>,
    cursor: usize,
    rng: StdRng,
    built_once: bool,
}

impl Default for Shuffle {
    fn default() -> Self {
        Self::new()
    }
}

impl Shuffle {
    /// A shuffle seeded from system entropy.
    pub fn new() -> Self {
        Self::with_rng(StdRng::from_os_rng())
    }

    /// A reproducible shuffle (tests).
    pub fn seeded(seed: u64) -> Self {
        Self::with_rng(StdRng::seed_from_u64(seed))
    }

    fn with_rng(rng: StdRng) -> Self {
        Self {
            order: Vec::new(),
            cursor: 0,
            rng,
            built_once: false,
        }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn order(&self) -> &[usize] {
        &self.order
    }

    /// Entry index at the cursor.
    pub fn current(&self) -> Option<usize> {
        self.order.get(self.cursor).copied()
    }

    fn shuffle_range(&mut self, from: usize, to: usize) {
        for i in (from + 1..to).rev() {
            let j = self.rng.random_range(from..=i);
            self.order.swap(i, j);
        }
    }

    /// Rebuilds the permutation for a list of `len` entries and sets the cursor to 0. On the
    /// first build, `current` goes to slot 0 so playback resumes on the same song.
    pub fn rebuild(&mut self, len: usize, current: Option<usize>) {
        self.order = (0..len).collect();
        self.cursor = 0;
        match current.filter(|&c| c < len && !self.built_once) {
            Some(c) => {
                self.order.swap(0, c);
                self.shuffle_range(1, len);
            }
            None => self.shuffle_range(0, len),
        }
        self.built_once = true;
    }

    /// Marks the start-up rule as used (a later rebuild shuffles everything).
    pub fn has_built(&self) -> bool {
        self.built_once
    }

    /// Steps the cursor by +1 or −1.
    pub fn step(&mut self, forward: bool) -> Step {
        if self.order.is_empty() {
            return if forward {
                Step::EndReached
            } else {
                Step::StartReached
            };
        }
        if forward {
            if self.cursor + 1 >= self.order.len() {
                return Step::EndReached;
            }
            self.cursor += 1;
        } else {
            if self.cursor == 0 {
                return Step::StartReached;
            }
            self.cursor -= 1;
        }
        Step::Moved(self.order[self.cursor])
    }

    /// Restarts a pass with a partial reshuffle so recent songs do not repeat soon.
    /// `rate` is the morph rate 0..=50.
    pub fn morph(&mut self, rate: u32) {
        self.cursor = 0;
        let len = self.order.len();
        if len <= 2 {
            return;
        }
        let window = (len * rate.min(50) as usize / 100).max(2);
        for x in 0..len - 1 {
            let end = (x + window).min(len);
            self.shuffle_range(x, end);
        }
    }

    pub fn go_first(&mut self) -> Option<usize> {
        self.cursor = 0;
        self.current()
    }

    pub fn go_last(&mut self) -> Option<usize> {
        self.cursor = self.order.len().saturating_sub(1);
        self.current()
    }

    /// Points the cursor at the slot holding `index`, if any.
    pub fn seek_to_entry(&mut self, index: usize) {
        if let Some(slot) = self.order.iter().position(|&i| i == index) {
            self.cursor = slot;
        }
    }

    /// A new entry was inserted at `index`: shift indices and put it at a random slot.
    pub fn on_insert(&mut self, index: usize) {
        for i in &mut self.order {
            if *i >= index {
                *i += 1;
            }
        }
        let slot = self.rng.random_range(0..=self.order.len());
        self.order.insert(slot, index);
        if slot <= self.cursor && self.order.len() > 1 {
            self.cursor += 1;
        }
    }

    /// The entry at `index` was removed.
    pub fn on_remove(&mut self, index: usize) {
        if let Some(slot) = self.order.iter().position(|&i| i == index) {
            self.order.remove(slot);
            if slot <= self.cursor && self.cursor > 0 {
                self.cursor -= 1;
            }
        }
        for i in &mut self.order {
            if *i > index {
                *i -= 1;
            }
        }
        self.cursor = self.cursor.min(self.order.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_perm(o: &[usize], n: usize) -> bool {
        let mut v = o.to_vec();
        v.sort();
        v == (0..n).collect::<Vec<_>>()
    }

    #[test]
    fn every_entry_once_per_pass() {
        for n in [1, 2, 3, 100] {
            let mut s = Shuffle::seeded(n as u64);
            s.rebuild(n, None);
            let mut seen = vec![s.current().unwrap()];
            while let Step::Moved(i) = s.step(true) {
                seen.push(i);
            }
            assert!(is_perm(&seen, n), "n={n}");
        }
    }

    #[test]
    fn first_build_keeps_current_first() {
        let mut s = Shuffle::seeded(7);
        s.rebuild(10, Some(6));
        assert_eq!(s.current(), Some(6));
        s.rebuild(10, Some(6));
        assert!(is_perm(s.order(), 10));
    }

    #[test]
    fn back_after_forward_walks_same_entries() {
        let mut s = Shuffle::seeded(3);
        s.rebuild(20, None);
        let mut fwd = vec![s.current().unwrap()];
        for _ in 0..5 {
            if let Step::Moved(i) = s.step(true) {
                fwd.push(i);
            }
        }
        let mut back = Vec::new();
        for _ in 0..5 {
            if let Step::Moved(i) = s.step(false) {
                back.push(i);
            }
        }
        fwd.pop();
        fwd.reverse();
        assert_eq!(back, fwd);
        assert_eq!(s.step(false), Step::StartReached);
    }

    #[test]
    fn morph_spreads_repeats() {
        // Over 1000 passes of 20 entries with rate 50, an entry plays twice within 9 tracks of
        // a pass boundary in at most 5 % of cases (12 §2.6).
        let mut s = Shuffle::seeded(11);
        s.rebuild(20, None);
        let mut bad = 0;
        for _ in 0..1000 {
            let old: Vec<usize> = s.order().to_vec();
            s.morph(50);
            assert!(is_perm(s.order(), 20));
            // Tracks between the two plays of each entry across the boundary.
            let repeats_soon = (0..20).any(|e| {
                let p = old.iter().position(|&x| x == e).unwrap();
                let q = s.order().iter().position(|&x| x == e).unwrap();
                (20 - p) + q <= 9
            });
            if repeats_soon {
                bad += 1;
            }
        }
        assert!(bad <= 50, "bad = {bad}");
    }

    #[test]
    fn insert_and_remove_keep_permutation() {
        let mut s = Shuffle::seeded(5);
        s.rebuild(5, None);
        s.step(true);
        let cur = s.current().unwrap();
        s.on_insert(0);
        assert!(is_perm(s.order(), 6));
        assert_eq!(s.current(), Some(cur + 1));
        s.on_remove(3);
        assert!(is_perm(s.order(), 5));
    }
}
