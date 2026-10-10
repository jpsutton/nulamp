//! Connects the audio player to the user interface.

use std::path::Path;

use nulamp_audio::{Player, PlayerEvent};
use nulamp_core::{Command, NewEntry, PlayerStatus};
use nulamp_ui::PlayerHandle;

pub struct AppPlayer {
    pub player: Player,
}

impl PlayerHandle for AppPlayer {
    fn status(&self) -> PlayerStatus {
        self.player.status()
    }

    fn command(&mut self, command: Command) {
        self.player.command(command);
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
        for e in self.player.poll() {
            match e {
                PlayerEvent::Error(msg) => log::error!("{msg}"),
                PlayerEvent::OpenFileDialog => log::info!("playlist is empty"),
                other => log::debug!("{other:?}"),
            }
        }
    }
}

/// Expands command-line arguments into playlist entries: folders recursively (sorted),
/// files and URLs as given (10 §2.5, simplified until playlist formats are wired).
pub fn expand(args: &[String], is_supported: &dyn Fn(&str) -> bool) -> Vec<NewEntry> {
    let mut out = Vec::new();
    for a in args {
        let p = Path::new(a);
        if p.is_dir() {
            let mut files = Vec::new();
            walk(p, &mut files, is_supported);
            files.sort_by(|a, b| nulamp_core::playlist::natural_cmp(a, b));
            out.extend(files.into_iter().map(NewEntry::new));
        } else if p.exists() {
            let abs = std::fs::canonicalize(p)
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_else(|_| a.clone());
            out.push(NewEntry::new(abs));
        } else {
            out.push(NewEntry::new(a.clone()));
        }
    }
    out
}

fn walk(dir: &Path, out: &mut Vec<String>, is_supported: &dyn Fn(&str) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out, is_supported);
        } else if let Some(ext) = p.extension().and_then(|x| x.to_str())
            && is_supported(ext)
        {
            out.push(p.to_string_lossy().into_owned());
        }
    }
}
