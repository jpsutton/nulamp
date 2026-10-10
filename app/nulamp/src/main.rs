//! nulamp: a cross-platform player for classic skins.

mod player;
mod titles;

use std::sync::Arc;

use nulamp_audio::Player;
use nulamp_core::Engine;
use nulamp_decoders::Registry;
use nulamp_skin::{Skin, SkinFiles, builtin};
use nulamp_ui::{Shell, UiPrefs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut skin_path = None;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--skin" => skin_path = args.next(),
            _ => files.push(a),
        }
    }
    let skin = match skin_path {
        Some(path) => match SkinFiles::open(std::path::Path::new(&path)) {
            Ok(f) => Skin::load(&f, &builtin::files()),
            Err(e) => {
                log::error!("cannot load skin {path}: {e}; using the built-in skin");
                builtin::skin()
            }
        },
        None => builtin::skin(),
    };
    let registry = Arc::new(Registry::with_defaults());
    let mut player = Player::new(Engine::default(), registry.clone());
    let rg_settings = nulamp_eq::ReplayGainSettings::default();
    player.gain_for = Box::new(move |loc| {
        if !rg_settings.enabled {
            return 1.0;
        }
        nulamp_eq::replaygain::gain_factor(&rg_settings, &titles::replaygain_tags(loc))
    });
    let entries = player::expand(&files, &|ext| registry.is_supported_extension(ext));
    let start = !entries.is_empty();
    player.engine.append(entries);
    if start {
        player.command(nulamp_core::Command::Play);
    }
    let titles = titles::TitleReader::new(nulamp_tf::DEFAULT_FORMAT);
    Shell::new(
        player::AppPlayer { player, titles },
        UiPrefs::default(),
        skin,
    )
    .run()
}
