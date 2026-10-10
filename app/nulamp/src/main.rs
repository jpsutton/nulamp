//! nulamp: a cross-platform player for classic skins.

mod player;
mod settings;
mod titles;

use std::sync::Arc;

use nulamp_audio::Player;
use nulamp_core::{Command, Engine};
use nulamp_decoders::Registry;
use nulamp_skin::{Skin, SkinFiles, builtin};
use nulamp_ui::{PlayerHandle, Shell};

fn load_skin(path: Option<&str>) -> Skin {
    match path {
        Some(path) => match SkinFiles::open(std::path::Path::new(path)) {
            Ok(f) => Skin::load(&f, &builtin::files()),
            Err(e) => {
                log::error!("cannot load skin {path}: {e}; using the built-in skin");
                builtin::skin()
            }
        },
        None => builtin::skin(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = nulamp_ui::shell::config_dir();
    let (mut prefs, state) = settings::load(&config);

    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--skin" => prefs.skin_path = args.next(),
            _ => files.push(a),
        }
    }
    let skin = load_skin(prefs.skin_path.as_deref());

    let registry = Arc::new(Registry::with_defaults());
    let mut player = Player::new(Engine::default(), registry.clone());
    let replaygain = std::sync::Arc::new(std::sync::Mutex::new(prefs.replaygain));
    let rg = replaygain.clone();
    player.gain_for = Box::new(move |loc| {
        let settings = *rg.lock().expect("replaygain lock");
        if !settings.enabled {
            return 1.0;
        }
        nulamp_eq::replaygain::gain_factor(&settings, &titles::replaygain_tags(loc))
    });
    player.command(Command::SetVolume(state.volume));
    player.command(Command::SetBalance(state.balance));
    player.engine.shuffle_on = state.shuffle;
    player.engine.repeat = state.repeat;
    player.engine.manual_advance = state.manual_advance;

    let supported = |ext: &str| registry.is_supported_extension(ext);
    let start = !files.is_empty();
    if start {
        let mut entries = Vec::new();
        for f in &files {
            entries.extend(nulamp_ui::files::add_anything(f, true, false, &supported));
        }
        player.engine.append(entries);
        player.command(Command::Play);
    } else if let Ok(entries) =
        nulamp_ui::files::load_playlist(&settings::playlist_path(&config).to_string_lossy())
    {
        // Resume the saved playlist and position (10 §4).
        player.engine.append(entries);
        player.engine.playlist.set_current(state.position);
        player.engine.playlist.modified = false;
    }

    let titles = titles::TitleReader::new(nulamp_tf::DEFAULT_FORMAT);
    let (prefs, app) = Shell::new(
        player::AppPlayer {
            player,
            titles,
            replaygain,
        },
        prefs,
        skin,
    )
    .run()?;

    let status = app.status();
    let state = settings::PlayerState {
        volume: status.volume,
        balance: status.balance,
        shuffle: status.shuffle,
        repeat: status.repeat,
        manual_advance: status.manual_advance,
        position: app.player.engine.playlist.current_index(),
    };
    if let Err(e) = settings::save(&config, &prefs, &state) {
        log::error!("cannot save settings: {e}");
    }
    let pl_path = settings::playlist_path(&config);
    if (app.player.engine.playlist.modified || !pl_path.exists())
        && let Err(e) = nulamp_ui::files::save_playlist(
            &app.player.engine.playlist,
            &pl_path.to_string_lossy(),
            false,
        )
    {
        log::error!("cannot save the playlist: {e}");
    }
    Ok(())
}
