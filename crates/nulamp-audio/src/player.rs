//! The player: transport rules from the core [`Engine`] driving the [`AudioEngine`].

use std::sync::Arc;

use nulamp_core::engine::{Effect, Engine};
use nulamp_core::{Command, EntryId, PlayState, PlayerStatus};
use nulamp_decoders::{Registry, StreamInfo};

use crate::engine::{AudioEngine, Control, EngineStatus, Event, Token};

/// Notifications for the user interface and remote control.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// The playlist is empty and Play was pressed.
    OpenFileDialog,
    /// A track started being heard.
    TrackStarted(EntryId),
    Stopped,
    Error(String),
}

pub struct Player {
    pub engine: Engine,
    audio: AudioEngine,
    pub registry: Arc<Registry>,
    volume: u8,
    balance: i8,
    /// Entry being heard, and its stream info.
    heard: Option<(EntryId, StreamInfo)>,
    /// Entry the audio engine was last told to open (may be ahead of `heard`).
    opened: Option<EntryId>,
    /// The engine decided to stop after the current track; keep "playing" until it drains.
    draining: bool,
    pending_events: Vec<PlayerEvent>,
    /// Per-track pre-gain (ReplayGain); 1.0 until wired.
    pub gain_for: Box<dyn Fn(&str) -> f32 + Send>,
    eq: Arc<crate::eq_stage::SharedEq>,
}

fn token(id: EntryId) -> Token {
    id.0
}

impl Player {
    pub fn new(engine: Engine, registry: Arc<Registry>) -> Self {
        let audio = AudioEngine::start(registry.clone());
        audio.send(Control::Volume(200));
        let eq = crate::eq_stage::SharedEq::new(nulamp_eq::EqSettings::default());
        audio.send(Control::SetStages(vec![Box::new(
            crate::eq_stage::EqStage::new(eq.clone()),
        )]));
        Self {
            engine,
            audio,
            registry,
            volume: 200,
            balance: 0,
            heard: None,
            opened: None,
            draining: false,
            pending_events: Vec::new(),
            gain_for: Box::new(|_| 1.0),
            eq,
        }
    }

    pub fn audio(&self) -> &AudioEngine {
        &self.audio
    }

    fn location(&self, id: EntryId) -> Option<String> {
        self.engine.playlist.by_id(id).map(|e| e.location.clone())
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::Start(id) => {
                    let Some(loc) = self.location(id) else {
                        continue;
                    };
                    let gain = (self.gain_for)(&loc);
                    self.draining = false;
                    self.opened = Some(id);
                    self.audio.send(Control::Open {
                        token: token(id),
                        location: loc,
                        start_ms: 0,
                        gain,
                    });
                }
                Effect::Stop => {
                    self.draining = false;
                    self.opened = None;
                    self.audio.send(Control::Stop);
                    self.heard = None;
                    self.pending_events.push(PlayerEvent::Stopped);
                }
                Effect::StopWithFade => {
                    self.draining = false;
                    self.opened = None;
                    self.audio.send(Control::StopWithFade);
                    self.heard = None;
                    self.pending_events.push(PlayerEvent::Stopped);
                }
                Effect::Pause(on) => self.audio.send(Control::Pause(on)),
                Effect::OpenFileDialog => self.pending_events.push(PlayerEvent::OpenFileDialog),
            }
        }
    }

    /// Handles a user command.
    pub fn command(&mut self, c: Command) {
        let effects = match c {
            Command::Previous => self.engine.previous_track(),
            Command::Play => self.engine.play(),
            Command::Pause => self.engine.pause(),
            Command::Stop => self.engine.stop(),
            Command::StopWithFade => self.engine.stop_with_fade(),
            Command::ToggleStopAfterCurrent => {
                self.engine.toggle_stop_after_current();
                vec![]
            }
            Command::Next => self.engine.next_track(),
            Command::SeekRelative(d) => {
                let pos = self.audio.status().position_ms as i64;
                self.seek((pos + d).max(0) as u64);
                vec![]
            }
            Command::SeekTo(ms) => {
                self.seek(ms);
                vec![]
            }
            Command::StartOfList => self.engine.start_of_list(),
            Command::EndOfList => self.engine.end_of_list(),
            Command::JumpEntries(n) => self.engine.jump(n),
            Command::SetVolume(v) => {
                self.set_volume(v);
                vec![]
            }
            Command::VolumeBy(d) => {
                self.set_volume((self.volume as i32 + d).clamp(0, 255) as u8);
                vec![]
            }
            Command::SetBalance(b) => {
                self.balance = b.clamp(-127, 127);
                self.audio.send(Control::Balance(self.balance));
                vec![]
            }
            Command::SetShuffle(on) => {
                self.engine.set_shuffle(on);
                vec![]
            }
            Command::SetRepeat(on) => {
                self.engine.repeat = on;
                vec![]
            }
            Command::SetManualAdvance(on) => {
                self.engine.manual_advance = on;
                vec![]
            }
            Command::PlayIndex(i) => self.engine.play_index(i),
        };
        // A command that changes the track invalidates any queued gapless successor.
        self.apply(effects);
    }

    fn set_volume(&mut self, v: u8) {
        self.volume = v;
        self.audio.send(Control::Volume(v));
    }

    fn seek(&mut self, ms: u64) {
        if self.engine.state == PlayState::Stopped || self.heard.is_none() {
            return;
        }
        // Seeking re-anchors the stream on the heard track.
        if let Some((id, _)) = &self.heard
            && self.opened != Some(*id)
        {
            let id = *id;
            if let Some(i) = self.engine.playlist.index_of(id) {
                self.engine.playlist.set_current(i);
            }
            self.opened = Some(id);
        }
        self.draining = false;
        self.audio.send(Control::Seek(ms));
    }

    /// Processes audio events. Call regularly (every UI tick).
    pub fn poll(&mut self) -> Vec<PlayerEvent> {
        while let Ok(ev) = self.audio.events.try_recv() {
            self.on_event(ev);
        }
        std::mem::take(&mut self.pending_events)
    }

    fn on_event(&mut self, ev: Event) {
        let id_of = |t: Token| EntryId(t);
        match ev {
            Event::Started { token, info } => {
                if self.opened == Some(id_of(token)) {
                    self.heard = Some((id_of(token), info));
                    self.engine.track_started();
                    self.refresh_metadata(id_of(token));
                    self.pending_events
                        .push(PlayerEvent::TrackStarted(id_of(token)));
                }
            }
            Event::TrackChanged { token, info } => {
                self.heard = Some((id_of(token), info));
                self.engine.track_started();
                self.refresh_metadata(id_of(token));
                self.pending_events
                    .push(PlayerEvent::TrackStarted(id_of(token)));
            }
            Event::OpenFailed { token, error } => {
                log::warn!("cannot play entry {token}: {error}");
                if self.opened != Some(id_of(token)) {
                    return;
                }
                let effects = self.engine.open_failed();
                self.apply(effects);
            }
            Event::NeedNext { token } => {
                if self.opened != Some(id_of(token)) {
                    return;
                }
                // Decide the next track now so it can be written gaplessly.
                let effects = self.engine.track_ended();
                let mut queued = false;
                for e in &effects {
                    if let Effect::Start(id) = e
                        && let Some(loc) = self.location(*id)
                    {
                        let gain = (self.gain_for)(&loc);
                        self.opened = Some(*id);
                        self.audio.send(Control::Queue {
                            token: token_of(*id),
                            location: loc,
                            gain,
                        });
                        queued = true;
                    }
                }
                if !queued {
                    // Stop once the buffered audio has been heard.
                    self.engine.state = PlayState::Playing;
                    self.draining = true;
                    self.audio.send(Control::NoNext);
                }
            }
            Event::Ended { .. } => {
                if self.draining {
                    self.draining = false;
                    self.engine.state = PlayState::Stopped;
                    self.heard = None;
                    self.opened = None;
                    self.pending_events.push(PlayerEvent::Stopped);
                } else {
                    // A seek failed or the stream ended unexpectedly: advance.
                    let effects = self.engine.track_ended();
                    self.apply(effects);
                }
            }
            Event::DeviceError(e) => {
                self.engine.stop();
                self.heard = None;
                self.pending_events.push(PlayerEvent::Error(e));
            }
            Event::Title { token, title } => {
                if let Some(e) = self.engine.playlist.by_id_mut(id_of(token)) {
                    e.title = title;
                }
            }
        }
    }

    /// Updates the playlist entry with the stream's length when a track starts.
    fn refresh_metadata(&mut self, id: EntryId) {
        let Some((hid, info)) = &self.heard else {
            return;
        };
        if *hid != id {
            return;
        }
        let len = info.length_ms;
        if let Some(e) = self.engine.playlist.by_id_mut(id)
            && len.is_some()
        {
            e.length_ms = len;
        }
    }

    /// Updates fades and buffering (07 §6).
    pub fn set_output_prefs(&mut self, p: crate::engine::OutputPrefs) {
        self.audio.send(Control::SetPrefs(p));
    }

    /// Updates the equalizer.
    pub fn set_eq(&mut self, s: &nulamp_eq::EqSettings) {
        self.eq.set(s);
    }

    /// Puts an effect after the equalizer (08 §5), or removes it.
    pub fn set_dsp(&mut self, dsp: Option<Box<dyn crate::chain::Stage>>) {
        let mut stages: Vec<Box<dyn crate::chain::Stage>> =
            vec![Box::new(crate::eq_stage::EqStage::new(self.eq.clone()))];
        stages.extend(dsp);
        self.audio.send(Control::SetStages(stages));
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }

    pub fn balance(&self) -> i8 {
        self.balance
    }

    pub fn engine_status(&self) -> EngineStatus {
        self.audio.status()
    }

    /// Snapshot for the user interface.
    pub fn status(&self) -> PlayerStatus {
        let a = self.audio.status();
        let e = &self.engine;
        let heard_index = self
            .heard
            .as_ref()
            .and_then(|(id, _)| e.playlist.index_of(*id));
        let position = heard_index.unwrap_or_else(|| e.playlist.current_index());
        let entry = e.playlist.get(position);
        let info = self.heard.as_ref().map(|(_, i)| i);
        let state = if a.paused && e.state != PlayState::Stopped {
            PlayState::Paused
        } else {
            e.state
        };
        PlayerStatus {
            state,
            position_ms: if state == PlayState::Stopped {
                0
            } else {
                a.position_ms
            },
            length_ms: info
                .and_then(|i| i.length_ms)
                .or_else(|| entry.and_then(|x| x.length_ms)),
            seekable: info.is_some_and(|i| i.seekable),
            kbps: a.kbps,
            sample_rate: info.map(|i| i.sample_rate),
            channels: info.map(|i| i.channels as u32),
            in_sync: a.in_sync && state == PlayState::Playing,
            volume: self.volume,
            balance: self.balance,
            shuffle: e.shuffle_on,
            repeat: e.repeat,
            manual_advance: e.manual_advance,
            stop_after_current: e.stop_after_current,
            position,
            title: entry.map(|x| x.title.clone()).unwrap_or_default(),
            playlist_len: e.playlist.len(),
        }
    }
}

fn token_of(id: EntryId) -> Token {
    token(id)
}
