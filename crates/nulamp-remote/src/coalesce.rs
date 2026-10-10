//! Collecting file launches into one playlist (11 §4.1).
//!
//! A file manager may start one process per selected file. Each sends an
//! [`Request::Open`](crate::Request::Open); the first clears the playlist and plays, and the
//! ones that follow within the window only append, so playback starts once.

use std::time::{Duration, Instant};

/// Default collection window (RC3).
pub const WINDOW: Duration = Duration::from_secs(1);

/// What to do with the files of one open request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPlan {
    /// Clear the playlist first.
    pub clear: bool,
    pub files: Vec<String>,
    /// Start playing the first added file.
    pub play: bool,
}

/// Turns consecutive open requests into one batch.
///
/// The window slides: each open request within [`WINDOW`] of the previous one joins the batch.
/// Call [`reset`](Self::reset) when another request changes the playlist or the transport.
#[derive(Debug, Clone)]
pub struct OpenCoalescer {
    window: Duration,
    last_open: Option<Instant>,
}

impl Default for OpenCoalescer {
    fn default() -> Self {
        Self::new(WINDOW)
    }
}

impl OpenCoalescer {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            last_open: None,
        }
    }

    /// Plans an [`Request::Open`](crate::Request::Open) received at `now`.
    pub fn open(&mut self, files: Vec<String>, enqueue: bool, now: Instant) -> OpenPlan {
        if enqueue {
            return OpenPlan {
                clear: false,
                files,
                play: false,
            };
        }
        let joins = self
            .last_open
            .is_some_and(|t| now.saturating_duration_since(t) <= self.window);
        self.last_open = Some(now);
        OpenPlan {
            clear: !joins,
            files,
            play: !joins,
        }
    }

    /// Ends the current batch.
    pub fn reset(&mut self) {
        self.last_open = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(f: &str) -> Vec<String> {
        vec![f.to_string()]
    }

    #[test]
    fn burst_plays_once() {
        let t = Instant::now();
        let mut c = OpenCoalescer::default();
        let a = c.open(files("a"), false, t);
        assert_eq!(
            a,
            OpenPlan {
                clear: true,
                files: files("a"),
                play: true
            }
        );
        let b = c.open(files("b"), false, t + Duration::from_millis(300));
        assert_eq!(
            b,
            OpenPlan {
                clear: false,
                files: files("b"),
                play: false
            }
        );
        // The window slides with each request.
        let d = c.open(files("d"), false, t + Duration::from_millis(1200));
        assert!(!d.clear && !d.play);
    }

    #[test]
    fn later_launch_starts_new_batch() {
        let t = Instant::now();
        let mut c = OpenCoalescer::default();
        c.open(files("a"), false, t);
        let b = c.open(files("b"), false, t + Duration::from_millis(1500));
        assert!(b.clear && b.play);
    }

    #[test]
    fn enqueue_never_clears_and_keeps_batch() {
        let t = Instant::now();
        let mut c = OpenCoalescer::default();
        let a = c.open(files("a"), true, t);
        assert!(!a.clear && !a.play);
        let b = c.open(files("b"), false, t);
        assert!(b.clear && b.play);
    }

    #[test]
    fn reset_ends_batch() {
        let t = Instant::now();
        let mut c = OpenCoalescer::default();
        c.open(files("a"), false, t);
        c.reset();
        assert!(c.open(files("b"), false, t).play);
    }
}
