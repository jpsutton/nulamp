//! Transport and play-order rules (07 §1–3), independent of audio.
//!
//! Every command returns the [`Effect`]s the audio side must carry out. The audio side reports
//! back with [`Engine::track_ended`], [`Engine::open_failed`] and [`Engine::track_started`].

use std::collections::VecDeque;

use crate::command::PlayState;
use crate::playlist::{EntryId, NewEntry, Playlist};
use crate::shuffle::{Shuffle, Step};

/// What the audio side must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Open and play an entry from the start.
    Start(EntryId),
    Stop,
    /// Stop with the 2-second fade-out (07 §2.3).
    StopWithFade,
    Pause(bool),
    /// The playlist is empty: show the open-file dialog (07 §2.2).
    OpenFileDialog,
}

#[derive(Debug, Clone)]
pub struct Engine {
    pub playlist: Playlist,
    shuffle: Shuffle,
    /// Playlist generation the shuffle order was built for.
    shuffle_generation: Option<u64>,
    pub state: PlayState,
    pub shuffle_on: bool,
    pub repeat: bool,
    pub manual_advance: bool,
    /// Shuffle morph rate 0..=50 (default 50).
    pub morph_rate: u32,
    pub stop_after_current: bool,
    /// Play queue extension (07 §3.4).
    pub queue: VecDeque<EntryId>,
    repeats_left: u32,
    failures: usize,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Shuffle::new())
    }
}

impl Engine {
    pub fn new(shuffle: Shuffle) -> Self {
        Self {
            playlist: Playlist::new(),
            shuffle,
            shuffle_generation: None,
            state: PlayState::Stopped,
            shuffle_on: false,
            repeat: false,
            manual_advance: false,
            morph_rate: 50,
            stop_after_current: false,
            queue: VecDeque::new(),
            repeats_left: 0,
            failures: 0,
        }
    }

    fn is_active(&self) -> bool {
        self.state != PlayState::Stopped
    }

    /// Rebuilds the shuffle order when the playlist changed structurally.
    fn ensure_shuffle(&mut self) {
        if self.shuffle_generation == Some(self.playlist.generation)
            && self.shuffle.len() == self.playlist.len()
        {
            return;
        }
        let current = (!self.playlist.is_empty()).then(|| self.playlist.current_index());
        self.shuffle.rebuild(self.playlist.len(), current);
        self.shuffle_generation = Some(self.playlist.generation);
    }

    fn sync_generation(&mut self) {
        if self.shuffle_generation.is_some() {
            self.shuffle_generation = Some(self.playlist.generation);
        }
    }

    /// Adds entries at `at`, keeping the shuffle order (each new entry gets a random slot).
    pub fn insert(&mut self, at: usize, items: Vec<NewEntry>) -> Vec<EntryId> {
        let at = at.min(self.playlist.len());
        let in_sync = self.shuffle_generation == Some(self.playlist.generation);
        let ids = self.playlist.insert(at, items);
        if in_sync {
            for k in 0..ids.len() {
                self.shuffle.on_insert(at + k);
            }
            self.sync_generation();
        }
        ids
    }

    pub fn append(&mut self, items: Vec<NewEntry>) -> Vec<EntryId> {
        let n = self.playlist.len();
        self.insert(n, items)
    }

    /// Removes entries matching a predicate, keeping the shuffle order and the queue.
    pub fn remove_where(
        &mut self,
        mut pred: impl FnMut(usize, &crate::playlist::Entry) -> bool,
    ) -> usize {
        let in_sync = self.shuffle_generation == Some(self.playlist.generation);
        let doomed: Vec<(usize, EntryId)> = self
            .playlist
            .entries()
            .iter()
            .enumerate()
            .filter(|(i, e)| pred(*i, e))
            .map(|(i, e)| (i, e.id))
            .collect();
        let ids: Vec<EntryId> = doomed.iter().map(|d| d.1).collect();
        let n = self.playlist.remove_where(|_, e| ids.contains(&e.id));
        self.queue.retain(|q| !ids.contains(q));
        if in_sync {
            for (i, _) in doomed.into_iter().rev() {
                self.shuffle.on_remove(i);
            }
            self.sync_generation();
        }
        n
    }

    /// Clears the playlist and the shuffle order. The playing track continues.
    pub fn clear(&mut self) {
        self.playlist.clear();
        self.queue.clear();
        self.shuffle_generation = None;
    }

    fn select(&mut self, index: usize) {
        self.playlist.set_current(index);
    }

    fn start_current(&mut self) -> Vec<Effect> {
        match self.playlist.current_id() {
            Some(id) => self.start(id),
            None => vec![],
        }
    }

    fn start(&mut self, id: EntryId) -> Vec<Effect> {
        if let Some(i) = self.playlist.index_of(id) {
            self.select(i);
            if self.shuffle_on && self.shuffle_generation == Some(self.playlist.generation) {
                self.shuffle.seek_to_entry(i);
            }
        }
        self.stop_after_current = false;
        self.repeats_left = self.playlist.by_id(id).map(|e| e.repeat).unwrap_or(0);
        self.state = PlayState::Playing;
        vec![Effect::Start(id)]
    }

    /// Moves to an index; starts it when playing, otherwise only moves the position.
    fn go(&mut self, index: usize) -> Vec<Effect> {
        self.select(index);
        if self.is_active() {
            self.start_current()
        } else {
            vec![]
        }
    }

    // ----- Commands (07 §2) -----

    pub fn play(&mut self) -> Vec<Effect> {
        match self.state {
            PlayState::Paused => {
                self.state = PlayState::Playing;
                vec![Effect::Pause(false)]
            }
            _ if self.playlist.is_empty() => vec![Effect::OpenFileDialog],
            _ => {
                if self.playlist.current_id().is_none() {
                    let i = self.playlist.current_index();
                    self.select(i);
                }
                self.start_current()
            }
        }
    }

    pub fn pause(&mut self) -> Vec<Effect> {
        match self.state {
            PlayState::Playing => {
                self.state = PlayState::Paused;
                vec![Effect::Pause(true)]
            }
            PlayState::Paused => {
                self.state = PlayState::Playing;
                vec![Effect::Pause(false)]
            }
            PlayState::Stopped => vec![],
        }
    }

    pub fn stop(&mut self) -> Vec<Effect> {
        self.state = PlayState::Stopped;
        vec![Effect::Stop]
    }

    pub fn stop_with_fade(&mut self) -> Vec<Effect> {
        if !self.is_active() {
            return vec![];
        }
        self.state = PlayState::Stopped;
        vec![Effect::StopWithFade]
    }

    pub fn toggle_stop_after_current(&mut self) {
        if self.state == PlayState::Playing {
            self.stop_after_current = !self.stop_after_current;
        }
    }

    pub fn previous(&mut self) -> Vec<Effect> {
        if self.playlist.is_empty() {
            return vec![];
        }
        if self.shuffle_on {
            self.ensure_shuffle();
            match self.shuffle.step(false) {
                Step::Moved(i) => self.go(i),
                _ => {
                    if let Some(i) = self.shuffle.go_first() {
                        self.select(i);
                    }
                    vec![]
                }
            }
        } else {
            let cur = self.playlist.current_index();
            if cur == 0 {
                if self.repeat {
                    self.go(self.playlist.len() - 1)
                } else {
                    vec![]
                }
            } else {
                self.go(cur - 1)
            }
        }
    }

    /// Next track (07 §2.4). Returns no effects at the end of the list without repeat.
    pub fn next(&mut self) -> Vec<Effect> {
        if let Some(id) = self.pop_queue() {
            return self.start(id);
        }
        match self.next_index() {
            Some(i) => self.go(i),
            None => vec![],
        }
    }

    fn pop_queue(&mut self) -> Option<EntryId> {
        while let Some(id) = self.queue.pop_front() {
            if self.playlist.index_of(id).is_some() {
                return Some(id);
            }
        }
        None
    }

    /// The entry the next "Next" moves to, advancing the shuffle cursor.
    fn next_index(&mut self) -> Option<usize> {
        let len = self.playlist.len();
        if len == 0 {
            return None;
        }
        if self.shuffle_on {
            self.ensure_shuffle();
            let old = self
                .playlist
                .current_id()
                .and_then(|id| self.playlist.index_of(id));
            let mut idx = match self.shuffle.step(true) {
                Step::Moved(i) => i,
                _ if self.repeat => {
                    self.shuffle.morph(self.morph_rate);
                    self.shuffle.go_first()?
                }
                _ => return None,
            };
            if Some(idx) == old && len > 1 {
                idx = match self.shuffle.step(true) {
                    Step::Moved(i) => i,
                    _ => idx,
                };
            }
            Some(idx)
        } else {
            let cur = self.playlist.current_index();
            let next = if self.playlist.current_removed && self.playlist.current_id().is_none() {
                cur
            } else {
                cur + 1
            };
            if next < len {
                Some(next)
            } else if self.repeat {
                Some(0)
            } else {
                None
            }
        }
    }

    pub fn start_of_list(&mut self) -> Vec<Effect> {
        if self.playlist.is_empty() {
            return vec![];
        }
        let i = if self.shuffle_on {
            self.ensure_shuffle();
            self.shuffle.go_first().unwrap_or(0)
        } else {
            0
        };
        self.go(i)
    }

    pub fn end_of_list(&mut self) -> Vec<Effect> {
        if self.playlist.is_empty() {
            return vec![];
        }
        let i = if self.shuffle_on {
            self.ensure_shuffle();
            self.shuffle.go_last().unwrap_or(0)
        } else {
            self.playlist.len() - 1
        };
        self.go(i)
    }

    /// Equal to `n` Previous (n < 0) or Next (n > 0) commands, starting playback once.
    pub fn jump(&mut self, n: i32) -> Vec<Effect> {
        let was = self.state;
        self.state = PlayState::Stopped;
        for _ in 0..n.unsigned_abs() {
            if n < 0 {
                self.previous();
            } else {
                self.next();
            }
        }
        self.state = was;
        if self.is_active() {
            self.start_current()
        } else {
            vec![]
        }
    }

    /// Plays the entry at `index` (double-click, Enter, remote control).
    pub fn play_index(&mut self, index: usize) -> Vec<Effect> {
        match self.playlist.get(index).map(|e| e.id) {
            Some(id) => self.start(id),
            None => vec![],
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on && !self.shuffle_on && self.state == PlayState::Stopped {
            // Turning shuffle on while stopped: rebuild and select slot 0 (07 §3.2).
            self.shuffle_on = true;
            self.shuffle_generation = None;
            self.ensure_shuffle();
            if let Some(i) = self.shuffle.go_first() {
                self.select(i);
            }
            return;
        }
        self.shuffle_on = on;
    }

    // ----- Events from the audio side -----

    /// A track started playing successfully.
    pub fn track_started(&mut self) {
        self.failures = 0;
    }

    /// The output has played all audio of the current track (07 §3.1).
    pub fn track_ended(&mut self) -> Vec<Effect> {
        if let Some(id) = self.pop_queue() {
            return self.start(id);
        }
        if self.playlist.is_empty() || self.stop_after_current {
            self.stop_after_current = false;
            return self.stop();
        }
        if self.manual_advance {
            return if self.repeat {
                self.start_current()
            } else {
                self.stop()
            };
        }
        if self.repeats_left > 0 && self.playlist.current_id().is_some() {
            let left = self.repeats_left - 1;
            let effects = self.start_current();
            self.repeats_left = left;
            return effects;
        }
        match self.next_index() {
            Some(i) => {
                self.select(i);
                self.start_current()
            }
            None => self.stop(),
        }
    }

    /// The current entry could not be opened (07 §3.3): skip as if it had ended, but stop once
    /// every entry has failed in a row.
    pub fn open_failed(&mut self) -> Vec<Effect> {
        self.failures += 1;
        if self.failures >= self.playlist.len().max(1) {
            self.failures = 0;
            return self.stop();
        }
        let saved = self.manual_advance;
        self.manual_advance = false;
        self.stop_after_current = false;
        let e = self.track_ended();
        self.manual_advance = saved;
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(n: usize) -> Engine {
        let mut e = Engine::new(Shuffle::seeded(42));
        e.append(
            (0..n)
                .map(|i| NewEntry::new(format!("/m/{i}.mp3")))
                .collect(),
        );
        e
    }

    fn started(effects: &[Effect], e: &Engine) -> Option<usize> {
        effects.iter().find_map(|f| match f {
            Effect::Start(id) => e.playlist.index_of(*id),
            _ => None,
        })
    }

    #[test]
    fn play_stopped_paused_playing() {
        let mut e = Engine::new(Shuffle::seeded(1));
        assert_eq!(e.play(), vec![Effect::OpenFileDialog]);
        let mut e = engine(3);
        let f = e.play();
        assert_eq!(started(&f, &e), Some(0));
        assert_eq!(e.pause(), vec![Effect::Pause(true)]);
        assert_eq!(e.play(), vec![Effect::Pause(false)]);
        // Playing: restart the current track.
        assert_eq!(started(&e.play(), &e), Some(0));
    }

    #[test]
    fn next_previous_without_shuffle() {
        let mut e = engine(3);
        // Stopped: only moves.
        assert!(e.next().is_empty());
        assert_eq!(e.playlist.current_index(), 1);
        e.play();
        assert_eq!(started(&e.next(), &e), Some(2));
        assert!(e.next().is_empty());
        e.repeat = true;
        assert_eq!(started(&e.next(), &e), Some(0));
        e.repeat = false;
        assert!(e.previous().is_empty());
        e.repeat = true;
        assert_eq!(started(&e.previous(), &e), Some(2));
    }

    #[test]
    fn repeat_and_manual_advance_table() {
        // Off/Off: play list once.
        let mut e = engine(2);
        e.play();
        assert_eq!(started(&e.track_ended(), &e), Some(1));
        assert_eq!(e.track_ended(), vec![Effect::Stop]);
        assert_eq!(e.playlist.current_index(), 1);
        // On/Off: loop the list.
        let mut e = engine(2);
        e.repeat = true;
        e.play();
        e.track_ended();
        assert_eq!(started(&e.track_ended(), &e), Some(0));
        // On/On: loop the current track.
        let mut e = engine(2);
        e.repeat = true;
        e.manual_advance = true;
        e.play();
        assert_eq!(started(&e.track_ended(), &e), Some(0));
        // Off/On: stop after each track.
        let mut e = engine(2);
        e.manual_advance = true;
        e.play();
        assert_eq!(e.track_ended(), vec![Effect::Stop]);
        assert_eq!(e.playlist.current_index(), 0);
    }

    #[test]
    fn stop_after_current_once() {
        let mut e = engine(3);
        e.play();
        e.toggle_stop_after_current();
        assert_eq!(e.track_ended(), vec![Effect::Stop]);
        assert!(!e.stop_after_current);
        e.play();
        assert!(started(&e.track_ended(), &e).is_some());
    }

    #[test]
    fn unplayable_list_stops() {
        let mut e = engine(3);
        e.play();
        let mut stops = 0;
        for _ in 0..10 {
            let f = e.open_failed();
            if f == vec![Effect::Stop] {
                stops += 1;
                break;
            }
        }
        assert_eq!(stops, 1);
    }

    #[test]
    fn per_entry_repeat() {
        let mut e = Engine::new(Shuffle::seeded(1));
        let mut n = NewEntry::new("/a.mp3");
        n.repeat = 2;
        e.append(vec![n, NewEntry::new("/b.mp3")]);
        e.play();
        assert_eq!(started(&e.track_ended(), &e), Some(0));
        assert_eq!(started(&e.track_ended(), &e), Some(0));
        assert_eq!(started(&e.track_ended(), &e), Some(1));
    }

    #[test]
    fn queue_bypasses_order() {
        let mut e = engine(5);
        e.play();
        let id = e.playlist.get(3).unwrap().id;
        e.queue.push_back(id);
        e.manual_advance = true;
        assert_eq!(started(&e.track_ended(), &e), Some(3));
        e.queue.push_back(e.playlist.get(1).unwrap().id);
        assert_eq!(started(&e.next(), &e), Some(1));
    }

    #[test]
    fn shuffle_pass_plays_each_once() {
        for n in [1, 2, 3, 100] {
            let mut e = engine(n);
            e.set_shuffle(true);
            let f = e.play();
            let mut seen = vec![started(&f, &e).unwrap()];
            loop {
                let f = e.track_ended();
                match started(&f, &e) {
                    Some(i) => seen.push(i),
                    None => break,
                }
            }
            seen.sort();
            assert_eq!(seen, (0..n).collect::<Vec<_>>(), "n={n}");
        }
    }

    #[test]
    fn shuffle_resumes_on_saved_track() {
        let mut e = engine(20);
        e.playlist.set_current(7);
        e.shuffle_on = true; // restored from settings, not toggled
        let f = e.play();
        assert_eq!(started(&f, &e), Some(7));
        // The first Next goes to slot 1 of an order that starts with entry 7.
        let first = e.next();
        assert_ne!(started(&first, &e), Some(7));
    }

    #[test]
    fn shuffle_previous_walks_back() {
        let mut e = engine(20);
        e.set_shuffle(true);
        e.play();
        let mut fwd = vec![e.playlist.current_index()];
        for _ in 0..5 {
            fwd.push(started(&e.next(), &e).unwrap());
        }
        let mut back = Vec::new();
        for _ in 0..5 {
            back.push(started(&e.previous(), &e).unwrap());
        }
        fwd.pop();
        fwd.reverse();
        assert_eq!(back, fwd);
    }

    #[test]
    fn shuffle_never_repeats_immediately() {
        let mut e = engine(2);
        e.set_shuffle(true);
        e.repeat = true;
        e.play();
        let mut last = e.playlist.current_index();
        for _ in 0..50 {
            let i = started(&e.track_ended(), &e).unwrap();
            assert_ne!(i, last);
            last = i;
        }
    }

    #[test]
    fn clear_while_playing_restarts_at_first() {
        let mut e = engine(3);
        e.play();
        e.next();
        e.clear();
        e.append(vec![NewEntry::new("/x.mp3"), NewEntry::new("/y.mp3")]);
        assert_eq!(started(&e.next(), &e), Some(0));
    }

    #[test]
    fn jump_ten() {
        let mut e = engine(30);
        e.play();
        let f = e.jump(10);
        assert_eq!(f.len(), 1);
        assert_eq!(started(&f, &e), Some(10));
        assert_eq!(started(&e.jump(-3), &e), Some(7));
    }
}
