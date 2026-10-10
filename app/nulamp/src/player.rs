//! Connects the audio player to the user interface.

use nulamp_audio::{Player, PlayerEvent};
use nulamp_core::{Command, PlayerStatus};
use nulamp_ui::PlayerHandle;

pub struct AppPlayer {
    pub player: Player,
    pub titles: crate::titles::TitleReader,
    /// ReplayGain settings read by the player's gain hook.
    pub replaygain: std::sync::Arc<std::sync::Mutex<nulamp_eq::ReplayGainSettings>>,
    /// The network stream input (shared with the decoder registry).
    pub streams: nulamp_decoders::stream::StreamFactory,
    /// Login requests from network threads.
    pub logins: std::sync::mpsc::Receiver<nulamp_ui::shell::CredentialRequest>,
    /// Remote-control requests from plugins, for the shell.
    pub requests: Vec<nulamp_remote::Request>,
    /// Declared last: plugins are unloaded after the player (and its audio thread) is gone.
    pub plugins: crate::plugins::PluginRuntime,
}

/// Expands a whole audio CD (`cda://<drive>`) into one entry per track (09 §5). `None` for
/// other locations; an empty list (with the error logged) when the disc cannot be read.
pub fn cd_entries(location: &str) -> Option<Vec<nulamp_core::NewEntry>> {
    use nulamp_decoders::cd;
    if !cd::is_whole_disc(location) {
        return None;
    }
    match cd::CdFactory::system().disc_entries(location) {
        Ok(entries) => Some(
            entries
                .into_iter()
                .map(|e| nulamp_core::NewEntry {
                    title: Some(e.title),
                    length_ms: e.length_ms,
                    ..nulamp_core::NewEntry::new(e.location)
                })
                .collect(),
        ),
        Err(e) => {
            log::error!("cannot read the audio CD {location}: {e}");
            Some(Vec::new())
        }
    }
}

/// The "add anything" rule (10 §2.5) including audio CDs.
pub fn expand_item(item: &str, supported: &dyn Fn(&str) -> bool) -> Vec<nulamp_core::NewEntry> {
    cd_entries(item).unwrap_or_else(|| nulamp_ui::files::add_anything(item, true, false, supported))
}

/// Forwards login requests from network threads to the user interface and waits for the
/// answer (07 §8.5).
pub struct LoginPrompt(
    pub std::sync::Mutex<std::sync::mpsc::Sender<nulamp_ui::shell::CredentialRequest>>,
);

impl nulamp_decoders::stream::CredentialPrompt for LoginPrompt {
    fn credentials(&self, url: &str, realm: &str) -> Option<(String, String)> {
        let (reply, answer) = std::sync::mpsc::channel();
        let request = nulamp_ui::shell::CredentialRequest {
            url: url.to_string(),
            realm: realm.to_string(),
            reply,
        };
        self.0.lock().ok()?.send(request).ok()?;
        answer.recv().ok().flatten()
    }
}

impl PlayerHandle for AppPlayer {
    fn take_requests(&mut self) -> Vec<nulamp_remote::Request> {
        std::mem::take(&mut self.requests)
    }

    fn plugins(&self) -> (Vec<nulamp_ui::plugins_dialog::PluginRow>, usize) {
        self.plugins.rows()
    }

    fn plugin_action(&mut self, index: usize, action: nulamp_ui::plugins_dialog::PluginAction) {
        if self.plugins.action(index, action) {
            self.player.set_dsp(self.plugins.active_dsp_stage());
        }
    }

    fn credential_requests(&mut self) -> Vec<nulamp_ui::shell::CredentialRequest> {
        self.logins.try_iter().collect()
    }

    fn resolve_playlist_url(&self, url: &str) -> Option<std::sync::mpsc::Receiver<Vec<String>>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (streams, url) = (self.streams.clone(), url.to_string());
        std::thread::Builder::new()
            .name("nulamp-plurl".into())
            .spawn(move || {
                let entries = match streams.resolve_playlist_url(&url) {
                    Ok(entries) if !entries.is_empty() => entries,
                    Ok(_) => Vec::new(),
                    Err(e) => {
                        // Added as is: playing it shows the error in the title.
                        log::info!("cannot read playlist {url}: {e}");
                        vec![url]
                    }
                };
                let _ = tx.send(entries);
            })
            .ok()?;
        Some(rx)
    }

    fn cd_drives(&self) -> Vec<(String, String)> {
        // Menus ask on every opening; probing the devices is cached for a few seconds.
        type Cache = Option<(std::time::Instant, Vec<(String, String)>)>;
        static CACHE: std::sync::Mutex<Cache> = std::sync::Mutex::new(None);
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, drives)) = cache.as_ref()
            && at.elapsed() < std::time::Duration::from_secs(5)
        {
            return drives.clone();
        }
        let drives: Vec<_> = nulamp_decoders::cd::CdFactory::system()
            .drives()
            .into_iter()
            .map(|d| (d.location, d.label))
            .collect();
        *cache = Some((std::time::Instant::now(), drives.clone()));
        drives
    }

    fn expand_location(&self, location: &str) -> Option<Vec<nulamp_core::NewEntry>> {
        cd_entries(location)
    }

    fn status(&self) -> PlayerStatus {
        self.player.status()
    }

    fn command(&mut self, command: Command) {
        self.player.command(command);
    }

    fn vis_blocks_since(&self, frame: u64) -> Vec<(u64, u32, usize, Vec<f32>)> {
        self.player
            .audio()
            .vis
            .since(frame)
            .into_iter()
            .map(|b| (b.frame, b.rate, b.channels, b.samples))
            .collect()
    }

    fn heard_frame(&self) -> u64 {
        self.player.engine_status().frames_played
    }

    fn is_supported_extension(&self, ext: &str) -> bool {
        self.player.registry.is_supported_extension(ext)
    }

    fn extensions(&self) -> Vec<(String, String)> {
        self.player.registry.all_extensions()
    }

    fn engine(&self) -> &nulamp_core::Engine {
        &self.player.engine
    }

    fn engine_mut(&mut self) -> &mut nulamp_core::Engine {
        &mut self.player.engine
    }

    fn set_playback_prefs(&mut self, prefs: &nulamp_ui::UiPrefs) {
        if let Ok(mut rg) = self.replaygain.lock() {
            *rg = prefs.replaygain;
        }
        self.player.set_output_prefs(nulamp_audio::OutputPrefs {
            buffer_ms: prefs.buffer_ms,
            fade_pause_stop: prefs.fade_pause_stop,
            fade_seek: prefs.fade_seek,
            crossfade: prefs.crossfade,
            crossfade_ms: prefs.crossfade_ms,
            crossfade_curve: if prefs.crossfade_equal_power {
                nulamp_audio::FadeCurve::EqualPower
            } else {
                nulamp_audio::FadeCurve::Linear
            },
            crossfade_manual: prefs.crossfade_manual,
            gapless_albums: prefs.gapless_albums,
            ..nulamp_audio::OutputPrefs::default()
        });
    }

    fn set_eq(&mut self, settings: &nulamp_eq::EqSettings) {
        self.player.set_eq(settings);
        self.plugins
            .broadcast(&nulamp_plugin_api::PluginEvent::EqChanged);
    }

    fn tick(&mut self) {
        self.titles.tick(&mut self.player.engine);
        for e in self.player.poll() {
            match e {
                PlayerEvent::TrackStarted(id) => {
                    // Always re-read an entry's metadata when it starts playing (04 §10).
                    if let Some(loc) = self
                        .player
                        .engine
                        .playlist
                        .by_id(id)
                        .map(|e| e.location.clone())
                    {
                        self.titles.request(id, &loc);
                    }
                }
                PlayerEvent::Error(msg) => log::error!("{msg}"),
                PlayerEvent::OpenFileDialog => log::info!("playlist is empty"),
                other => log::debug!("{other:?}"),
            }
        }
        let status = self.player.status();
        let location = self
            .player
            .engine
            .playlist
            .get(status.position)
            .map(|e| e.location.clone());
        for c in self.plugins.tick(&status, location.as_deref()) {
            match c {
                nulamp_plugin_api::HostCommand::Player(c) => self.player.command(c),
                nulamp_plugin_api::HostCommand::Numbered(id) => {
                    match nulamp_remote::NumberedCommand::from_id(id) {
                        Some(n) => self.requests.push(nulamp_remote::Request::Command(n)),
                        None => log::debug!("plugin command {id} is not handled"),
                    }
                }
            }
        }
    }
}
