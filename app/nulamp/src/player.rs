//! Connects the audio player to the user interface.

use nulamp_audio::{Player, PlayerEvent};
use nulamp_core::{Command, PlayerStatus};
use nulamp_ui::PlayerHandle;

pub struct AppPlayer {
    pub player: Player,
    pub titles: crate::titles::TitleReader,
}

impl PlayerHandle for AppPlayer {
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

    fn set_eq(&mut self, settings: &nulamp_eq::EqSettings) {
        self.player.set_eq(settings);
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
    }
}
