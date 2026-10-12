//! nulamp: a cross-platform player for classic skins.

mod import;
mod library_input;
mod player;
mod plugins;
mod settings;
mod titles;

use std::sync::Arc;

use nulamp_audio::Player;
use nulamp_core::{Command, Engine};
use nulamp_decoders::Registry;
use nulamp_skin::{Skin, SkinFiles, builtin};
use nulamp_ui::library::MediaLibrary;
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

fn absolutize(p: &str) -> String {
    if p.contains("://") {
        return p.to_string();
    }
    std::path::absolute(p)
        .map(|a| a.to_string_lossy().into_owned())
        .unwrap_or_else(|_| p.to_string())
}

/// Opens the local media library (`<config>/library.sqlite`; scans index the files some
/// decoder plays) and adds the signed-in Jellyfin account, all in `registry`.
fn open_library(
    config: &std::path::Path,
    decoders: &Arc<Registry>,
    registry: Arc<nulamp_library::Registry>,
    jellyfin: Option<&nulamp_library::jellyfin::Account>,
) -> MediaLibrary {
    let decoders = decoders.clone();
    let filter = move |ext: &str| !ext.is_empty() && decoders.is_supported_extension(ext);
    let local = match nulamp_library::LocalLibrary::open(config.join("library.sqlite"), filter) {
        Ok(local) => {
            let local = Arc::new(local);
            registry.register(local.clone());
            Some(local)
        }
        Err(e) => {
            log::error!("cannot open the media library: {e}");
            None
        }
    };
    let library = MediaLibrary { registry, local };
    library.set_jellyfin(jellyfin);
    library
}

/// Copies a skin archive into the skins folder and returns its new path (01 §1.4).
fn install_skin(path: &str, skins_dir: &std::path::Path) -> String {
    let src = std::path::Path::new(path);
    let Some(name) = src.file_name() else {
        return path.to_string();
    };
    let dst = skins_dir.join(name);
    if std::fs::create_dir_all(skins_dir)
        .and_then(|_| std::fs::copy(src, &dst))
        .is_ok()
    {
        dst.to_string_lossy().into_owned()
    } else {
        path.to_string()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    nulamp_meta::set_external_reader(titles::external_tags);
    if let Some(license) = nulamp_decoders::ffmpeg_license() {
        log::debug!("FFmpeg libraries: {license}");
    }
    let mut inv = nulamp_remote::parse_args(std::env::args().skip(1));
    if inv.help {
        println!("{}", nulamp_remote::args::HELP);
        return Ok(());
    }
    inv.files = inv.files.iter().map(|f| absolutize(f)).collect();
    inv.skins = inv.skins.iter().map(|f| absolutize(f)).collect();
    inv.skin = inv.skin.as_deref().map(absolutize);
    inv.bookmark = inv.bookmark.as_deref().map(absolutize);
    if let Some(dir) = inv.ini_dir.as_ref().or(inv.config.as_ref()) {
        // SAFETY: set before any other thread is started.
        unsafe { std::env::set_var("NULAMP_CONFIG_DIR", dir) };
    }
    let config = nulamp_ui::shell::config_dir();
    if inv.delete_m3u {
        let _ = std::fs::remove_file(settings::playlist_path(&config));
        return Ok(());
    }

    // Single instance (11 §4.1): hand the request to a running instance and exit.
    let server = if inv.new_instance {
        None
    } else {
        match nulamp_remote::connect_or_serve(inv.class()) {
            Ok(nulamp_remote::Role::Secondary(mut client)) => {
                for r in inv.requests() {
                    if let Err(e) = client.send(&r) {
                        log::error!("cannot reach the running instance: {e}");
                    }
                }
                return Ok(());
            }
            Ok(nulamp_remote::Role::Primary(s)) => Some(s),
            Err(e) => {
                log::warn!("single-instance check failed: {e}");
                None
            }
        }
    };

    let first_run = !settings::settings_path(&config).exists();
    let import_dir = inv
        .import_winamp
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            first_run
                .then(import::candidates)
                .and_then(|c| c.into_iter().next())
        });
    let (mut prefs, state) = match import_dir.map(|d| (import::import(&d, &config), d)) {
        Some((Ok((prefs, state, report)), dir)) => {
            log::info!(
                "imported settings from {} ({})",
                dir.display(),
                report.files.join(", ")
            );
            let _ = settings::save(&config, &prefs, &state);
            (prefs, state)
        }
        Some((Err(e), dir)) => {
            log::warn!("cannot import {}: {e}", dir.display());
            settings::load(&config)
        }
        None => settings::load(&config),
    };
    for s in &inv.skins {
        prefs.skin_path = Some(install_skin(s, &prefs.skins_dir));
    }
    if let Some(s) = &inv.skin {
        prefs.skin_path = Some(s.clone());
    }
    let files = inv.files.clone();
    let skin = load_skin(prefs.skin_path.as_deref());

    let streams = nulamp_decoders::stream::StreamFactory::default();
    let mut stream_config = streams.config();
    stream_config.proxy = nulamp_decoders::stream::Proxy::from_env();
    streams.set_config(stream_config);
    let (login_tx, login_rx) = std::sync::mpsc::channel();
    streams.set_credential_prompt(Arc::new(player::LoginPrompt(std::sync::Mutex::new(
        login_tx,
    ))));
    let plugins = plugins::PluginRuntime::load(&config, inv.safe);
    let mut registry = Registry::with_stream(streams.clone());
    let library_registry = Arc::new(nulamp_library::Registry::new());
    registry.add(Box::new(library_input::LibraryInput {
        registry: library_registry.clone(),
        streams: streams.clone(),
    }));
    plugins.plugins.register_decoders(&mut registry);
    let registry = Arc::new(registry);
    // Before anything plays: saved and command-line entries may name library tracks.
    let library = open_library(
        &config,
        &registry,
        library_registry,
        prefs.library.jellyfin.as_ref(),
    );
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
    player.album_of = Box::new(titles::album_key);
    if let Some(dsp) = plugins.active_dsp_stage() {
        player.set_dsp(Some(dsp));
    }
    player.command(Command::SetVolume(state.volume));
    player.command(Command::SetBalance(state.balance));
    player.engine.shuffle_on = state.shuffle;
    player.engine.repeat = state.repeat;
    player.engine.manual_advance = state.manual_advance;

    let supported = |ext: &str| registry.is_supported_extension(ext);
    let start = !files.is_empty();
    if inv.enqueue {
        if let Ok(entries) =
            nulamp_ui::files::load_playlist(&settings::playlist_path(&config).to_string_lossy())
        {
            player.engine.append(entries);
            player.engine.playlist.set_current(state.position);
        }
        let mut entries = Vec::new();
        for f in &files {
            entries.extend(player::expand_item(f, &supported));
        }
        player.engine.append(entries);
    } else if start {
        let mut entries = Vec::new();
        for f in &files {
            entries.extend(player::expand_item(f, &supported));
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
    let mut shell = Shell::new(
        player::AppPlayer {
            player,
            titles,
            replaygain,
            streams,
            library: library.registry.clone(),
            logins: login_rx,
            requests: Vec::new(),
            plugins,
        },
        prefs,
        skin,
    );
    shell = shell.with_library(library);
    if let Some(server) = server {
        // Commands given on the first launch run once the windows exist.
        let sender = server.sender();
        if let Some(b) = &inv.bookmark {
            sender.post(nulamp_remote::Request::Bookmark(b.clone()));
        }
        if let Some(c) = inv.effective_command() {
            sender.post(c.clone());
        }
        shell = shell.with_remote(server);
    }
    let (prefs, app) = shell.run()?;

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
