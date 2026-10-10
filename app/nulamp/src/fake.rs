//! A simulated player used until the real playback engine is wired in (milestone M1).

use std::time::Instant;

use nulamp_core::{Command, PlayState, PlayerStatus};
use nulamp_ui::PlayerHandle;

pub struct FakePlayer {
    status: PlayerStatus,
    started: Option<(Instant, u64)>,
    titles: Vec<(&'static str, u64)>,
}

impl FakePlayer {
    pub fn new() -> Self {
        let titles = vec![
            ("Example Artist - First Track", 225_000),
            (
                "Another Artist - A Much Longer Title That Needs To Scroll Across The Display",
                312_000,
            ),
            ("Third Artist - Short", 95_000),
        ];
        let mut status = PlayerStatus {
            volume: 200,
            playlist_len: titles.len(),
            ..PlayerStatus::default()
        };
        status.title = titles[0].0.into();
        status.length_ms = Some(titles[0].1);
        Self {
            status,
            started: None,
            titles,
        }
    }

    fn position(&self) -> u64 {
        match self.started {
            Some((t, base)) => base + t.elapsed().as_millis() as u64,
            None => self.status.position_ms,
        }
    }

    fn select(&mut self, i: usize) {
        let i = i.min(self.titles.len() - 1);
        self.status.position = i;
        self.status.title = self.titles[i].0.into();
        self.status.length_ms = Some(self.titles[i].1);
        self.status.position_ms = 0;
        if self.status.state == PlayState::Playing {
            self.started = Some((Instant::now(), 0));
        }
    }

    fn seek(&mut self, ms: u64) {
        let ms = ms.min(self.status.length_ms.unwrap_or(0));
        self.status.position_ms = ms;
        if self.status.state == PlayState::Playing {
            self.started = Some((Instant::now(), ms));
        }
    }
}

impl PlayerHandle for FakePlayer {
    fn status(&self) -> PlayerStatus {
        let mut s = self.status.clone();
        s.position_ms = self.position().min(s.length_ms.unwrap_or(u64::MAX));
        s
    }

    fn command(&mut self, c: Command) {
        let s = &mut self.status;
        match c {
            Command::Play => {
                s.state = PlayState::Playing;
                s.seekable = true;
                s.in_sync = true;
                s.kbps = Some(128);
                s.sample_rate = Some(44_100);
                s.channels = Some(2);
                if self.started.is_none() {
                    let base = s.position_ms;
                    self.started = Some((Instant::now(), base));
                }
            }
            Command::Pause => match s.state {
                PlayState::Playing => {
                    s.position_ms = self
                        .started
                        .map(|(t, b)| b + t.elapsed().as_millis() as u64)
                        .unwrap_or(0);
                    self.started = None;
                    s.state = PlayState::Paused;
                }
                PlayState::Paused => {
                    self.started = Some((Instant::now(), s.position_ms));
                    s.state = PlayState::Playing;
                }
                PlayState::Stopped => {}
            },
            Command::Stop | Command::StopWithFade => {
                s.state = PlayState::Stopped;
                s.position_ms = 0;
                self.started = None;
            }
            Command::Next => {
                let i = (s.position + 1) % self.titles.len();
                self.select(i);
            }
            Command::Previous => {
                let i = (s.position + self.titles.len() - 1) % self.titles.len();
                self.select(i);
            }
            Command::StartOfList => self.select(0),
            Command::EndOfList => self.select(usize::MAX),
            Command::SeekTo(ms) => self.seek(ms),
            Command::SeekRelative(d) => {
                let p = (self.position() as i64 + d).max(0) as u64;
                self.seek(p);
            }
            Command::SetVolume(v) => s.volume = v,
            Command::VolumeBy(d) => s.volume = (s.volume as i32 + d).clamp(0, 255) as u8,
            Command::SetBalance(b) => s.balance = b,
            Command::SetShuffle(v) => s.shuffle = v,
            Command::SetRepeat(v) => s.repeat = v,
            Command::SetManualAdvance(v) => s.manual_advance = v,
            Command::ToggleStopAfterCurrent => s.stop_after_current = !s.stop_after_current,
            other => log::info!("fake player ignores {other:?}"),
        }
    }
}
