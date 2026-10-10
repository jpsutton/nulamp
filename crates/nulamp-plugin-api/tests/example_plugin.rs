//! Loads `nulamp-plugin-example` (built by cargo as a workspace member and dev-dependency) and
//! drives it through the adapters.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nulamp_audio::chain::{Stage, run_stages};
use nulamp_core::command::{Command, PlayerStatus};
use nulamp_plugin_api::{
    Host, HostCommand, HostServices, LoadOptions, ModuleKind, PluginEvent, Plugins, load_plugins,
};

#[derive(Default)]
struct Services {
    commands: Mutex<Vec<HostCommand>>,
    logs: Mutex<Vec<(String, String)>>,
    settings: Mutex<Vec<(String, String, String)>>,
    ui: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl Services {
    fn logs_of(&self, module: &str) -> Vec<String> {
        self.logs
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m.ends_with(module))
            .map(|(_, l)| l.clone())
            .collect()
    }

    fn set(&self, section: &str, key: &str, value: &str) {
        self.set_setting(section, key, Some(value));
    }
}

impl HostServices for Services {
    fn command(&self, command: HostCommand) {
        self.commands.lock().unwrap().push(command);
    }
    fn status(&self) -> PlayerStatus {
        PlayerStatus {
            volume: 123,
            title: "Artist - Title".into(),
            ..PlayerStatus::default()
        }
    }
    fn log(&self, module: &str, _level: log::Level, message: &str) {
        self.logs
            .lock()
            .unwrap()
            .push((module.into(), message.into()));
    }
    fn setting(&self, section: &str, key: &str) -> Option<String> {
        self.settings
            .lock()
            .unwrap()
            .iter()
            .find(|(s, k, _)| s == section && k == key)
            .map(|(_, _, v)| v.clone())
    }
    fn set_setting(&self, section: &str, key: &str, value: Option<&str>) {
        let mut s = self.settings.lock().unwrap();
        s.retain(|(sec, k, _)| !(sec == section && k == key));
        if let Some(v) = value {
            s.push((section.into(), key.into(), v.into()));
        }
    }
    fn post_to_ui(&self, task: Box<dyn FnOnce() + Send>) {
        self.ui.lock().unwrap().push(task);
    }
}

/// The example's shared library next to the test binary (`target/<profile>/deps/..`).
fn example_library() -> Option<PathBuf> {
    let name = format!(
        "{}nulamp_plugin_example.{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_EXTENSION
    );
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .skip(1)
        .take(3)
        .map(|d| d.join(&name))
        .find(|p| p.is_file())
}

/// Checks the example's modules: DSP processing, settings sections, events and host calls.
fn exercise(services: &Arc<Services>, plugins: Plugins, stem: &str) {
    assert!(plugins.errors.is_empty(), "{:?}", plugins.errors);
    let names: Vec<_> = plugins
        .modules()
        .map(|m| (m.info().kind, m.info().name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            (ModuleKind::Dsp, "Gain"),
            (ModuleKind::Dsp, "Vocal remover"),
            (ModuleKind::Dsp, "Half speed"),
            (ModuleKind::General, "Event logger"),
        ]
    );
    let gain = &plugins.dsps[0];
    assert_eq!(gain.module().info().section, format!("plugin:{stem}:Gain"));
    assert_eq!(gain.module().info().plugin_name, "nulamp example plugin");
    assert!(gain.module().info().has_configure);
    assert!(gain.module().configure());
    assert!(!gain.module().about());
    assert!(!plugins.dsps[1].module().configure());

    // Gain: the default, then the module's own setting.
    let mut g = gain.instantiate().unwrap();
    g.configure(44_100, 2);
    let mut v = vec![1.0, -0.5, 0.25, 2.0];
    g.process_resizing(&mut v);
    assert_eq!(v, [0.5, -0.25, 0.125, 1.0]);
    drop(g);
    services.set(&format!("plugin:{stem}:Gain"), "gain", "0.25");
    let mut g = gain.instantiate().unwrap();
    g.configure(44_100, 1);
    let mut v = [4.0, -8.0];
    g.process(&mut v);
    assert_eq!(v, [1.0, -2.0]);

    // Vocal remover: stereo only; other layouts pass through.
    let mut vr = plugins.dsps[1].instantiate().unwrap();
    vr.configure(48_000, 2);
    let mut v = vec![0.5, 0.25, -1.0, 1.0];
    vr.process_resizing(&mut v);
    assert_eq!(v, [0.25, 0.25, -2.0, -2.0]);
    vr.configure(48_000, 1);
    let mut v = vec![0.5, 0.25];
    vr.process_resizing(&mut v);
    assert_eq!(v, [0.5, 0.25]);

    // A chain whose middle stage doubles the frame count.
    let mut chain: Vec<Box<dyn Stage>> = vec![
        Box::new(plugins.dsps[2].instantiate().unwrap()),
        Box::new(gain.instantiate().unwrap()),
    ];
    for s in &mut chain {
        s.configure(44_100, 2);
    }
    let mut v = vec![1.0, 2.0, 3.0, 4.0];
    run_stages(&mut chain, &mut v);
    assert_eq!(v, [0.25, 0.5, 0.25, 0.5, 0.75, 1.0, 0.75, 1.0]);
    drop(chain);

    // General module: events, host status, posted UI calls and commands.
    services.set(&format!("plugin:{stem}:Event_logger"), "restart", "1");
    plugins.broadcast(&PluginEvent::TrackStarted {
        location: "/music/a.flac".into(),
        position: 3,
    });
    plugins.broadcast(&PluginEvent::VolumeChanged(123));
    plugins.broadcast(&PluginEvent::SkinChanged("Base".into()));
    plugins.broadcast(&PluginEvent::Paused);
    plugins.broadcast(&PluginEvent::TrackStopped { by_user: false });
    assert_eq!(
        services.logs_of("Event logger"),
        [
            "init",
            "track started value=3 text=\"/music/a.flac\"",
            "volume changed value=123 text=\"\"",
            "status volume=123",
            "skin changed value=0 text=\"Base\"",
            "paused value=0 text=\"\"",
            "track stopped value=0 text=\"\"",
        ]
    );
    assert_eq!(
        *services.commands.lock().unwrap(),
        [HostCommand::Player(Command::PlayIndex(0))]
    );
    let tasks: Vec<_> = services.ui.lock().unwrap().drain(..).collect();
    assert_eq!(tasks.len(), 1);
    for t in tasks {
        t();
    }
    assert_eq!(
        services.logs_of("Event logger").last().unwrap(),
        "ui callback"
    );

    // Dropping the modules runs `quit` (the file stays loaded until then).
    drop(g);
    drop(vr);
    drop(plugins);
    assert_eq!(services.logs_of("Event logger").last().unwrap(), "quit");
    assert_eq!(services.logs_of("Gain").last().unwrap(), "quit");
}

#[test]
fn shared_library_plugin() {
    let Some(lib) = example_library() else {
        println!("skipped: the example plugin library was not found next to the test binary");
        return;
    };
    let dir = std::env::temp_dir().join(format!("nulamp-plugin-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(lib.file_name().unwrap());
    std::fs::copy(&lib, &file).unwrap();
    let stem = file.file_stem().unwrap().to_string_lossy().into_owned();

    let services = Arc::new(Services::default());
    let host = Host::new(services.clone());
    let mut opts = LoadOptions {
        system_dir: None,
        user_dir: Some(dir.clone()),
        safe_mode: true,
    };
    // SAFETY: the directory holds only the example plugin, which follows the ABI.
    let none = unsafe { load_plugins(&opts, &host) };
    assert_eq!(none.modules().count(), 0);
    opts.safe_mode = false;
    // SAFETY: as above.
    let plugins = unsafe { load_plugins(&opts, &host) };
    assert_eq!(plugins.modules().next().unwrap().info().path, file);
    exercise(&services, plugins, &stem);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn built_in_plugin() {
    let services = Arc::new(Services::default());
    let host = Host::new(services.clone());
    let mut plugins = Plugins::default();
    // SAFETY: the example's entry point follows the ABI.
    let n = unsafe {
        plugins.load_builtin(nulamp_plugin_example::nulamp_plugin_entry, "example", &host)
    }
    .unwrap();
    assert_eq!(n, 4);
    exercise(&services, plugins, "example");
}
