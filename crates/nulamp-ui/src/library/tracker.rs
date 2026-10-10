//! Measures how long each playlist entry was listened to, so the library can count plays
//! when a track ends or is skipped.

use std::time::Instant;

use nulamp_core::EntryId;

/// Playback that ended: the file, the time actually listened to and the track length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedPlay {
    pub location: String,
    pub played_ms: u64,
    pub length_ms: u64,
}

/// What the player is heard playing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heard<'a> {
    pub entry: EntryId,
    pub location: &'a str,
    pub position_ms: u64,
    pub length_ms: Option<u64>,
}

#[derive(Debug)]
struct Playing {
    entry: EntryId,
    location: String,
    last_pos: u64,
    last_at: Instant,
    played_ms: u64,
    length_ms: u64,
}

/// Follows the heard position. Only forward progress that matches the wall clock counts
/// as listening, so seeks do not add time; a jump back to the start right at the end of a
/// track is a repeat and ends the play.
#[derive(Debug, Default)]
pub struct PlayTracker {
    current: Option<Playing>,
}

/// Slack allowed between position and wall-clock progress.
const SLACK_MS: u64 = 1000;
/// "At the end" and "at the start" for detecting a repeated track.
const EDGE_MS: u64 = 3000;

impl PlayTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds the current state (`None` when stopped). Returns the play that just ended.
    pub fn update(&mut self, heard: Option<Heard<'_>>, now: Instant) -> Option<FinishedPlay> {
        let Some(h) = heard else {
            return self.finish();
        };
        let same = self.current.as_ref().is_some_and(|p| p.entry == h.entry);
        if !same {
            let done = self.finish();
            self.start(h, now);
            return done;
        }
        let p = self.current.as_mut().expect("current play");
        if let Some(l) = h.length_ms {
            p.length_ms = l;
        }
        let wall = now.duration_since(p.last_at).as_millis() as u64;
        if h.position_ms >= p.last_pos {
            let delta = h.position_ms - p.last_pos;
            if delta <= wall + SLACK_MS {
                p.played_ms += delta;
            }
        } else if p.length_ms > 0 && p.last_pos + EDGE_MS >= p.length_ms && h.position_ms < EDGE_MS
        {
            // The same entry started again (repeat one).
            let done = self.finish();
            self.start(h, now);
            return done;
        }
        let p = self.current.as_mut().expect("current play");
        p.last_pos = h.position_ms;
        p.last_at = now;
        None
    }

    fn start(&mut self, h: Heard<'_>, now: Instant) {
        self.current = Some(Playing {
            entry: h.entry,
            location: h.location.to_string(),
            last_pos: h.position_ms,
            last_at: now,
            played_ms: 0,
            length_ms: h.length_ms.unwrap_or(0),
        });
    }

    fn finish(&mut self) -> Option<FinishedPlay> {
        let p = self.current.take()?;
        (p.played_ms > 0).then_some(FinishedPlay {
            location: p.location,
            played_ms: p.played_ms,
            length_ms: p.length_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn heard(entry: u64, pos: u64) -> Option<Heard<'static>> {
        Some(Heard {
            entry: EntryId(entry),
            location: if entry == 1 { "/m/a.mp3" } else { "/m/b.mp3" },
            position_ms: pos,
            length_ms: Some(200_000),
        })
    }

    #[test]
    fn counts_listening_time_until_the_next_track() {
        let mut t = PlayTracker::new();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        assert_eq!(t.update(heard(1, 0), at(0)), None);
        for s in 1..=100 {
            assert_eq!(t.update(heard(1, s * 1000), at(s * 1000)), None);
        }
        // A seek forward does not count.
        t.update(heard(1, 150_000), at(100_100));
        t.update(heard(1, 151_000), at(101_100));
        let done = t.update(heard(2, 0), at(101_200)).unwrap();
        assert_eq!(done.location, "/m/a.mp3");
        assert_eq!(done.played_ms, 101_000);
        assert_eq!(done.length_ms, 200_000);
        // Stopping ends the second track; nothing was heard yet, so no play.
        assert_eq!(t.update(None, at(101_300)), None);
    }

    #[test]
    fn blocked_ui_still_counts_and_repeat_one_ends_a_play() {
        let mut t = PlayTracker::new();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        t.update(heard(1, 0), at(0));
        // The interface was blocked for 30 s (a file dialog): the progress is real.
        t.update(heard(1, 30_000), at(30_000));
        t.update(heard(1, 199_000), at(199_000));
        let done = t.update(heard(1, 500), at(199_600)).unwrap();
        assert_eq!(done.played_ms, 199_000);
        t.update(heard(1, 10_500), at(209_600));
        assert_eq!(t.update(None, at(209_700)).unwrap().played_ms, 10_000);
        // A seek back in the middle keeps the play going.
        t.update(heard(1, 50_000), at(300_000));
        t.update(heard(1, 51_000), at(301_000));
        assert_eq!(t.update(heard(1, 1_000), at(301_100)), None);
        t.update(heard(1, 2_000), at(302_100));
        assert_eq!(t.update(None, at(302_200)).unwrap().played_ms, 2_000);
    }
}
