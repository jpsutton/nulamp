//! Native plugins in the app (08): loading, host services, events and the active effect.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use nulamp_core::command::{PlayState, PlayerStatus};
use nulamp_plugin_api::{
    Host, HostCommand, HostServices, LoadOptions, Module, ModuleKind, PluginEvent, Plugins,
};
use nulamp_ui::plugins_dialog::{PluginAction, PluginRow};

/// Settings section of the host's own plugin choices.
const HOST_SECTION: &str = "nulamp";

/// Module settings, one section per module, in `plugins.ini` (11 §1, 08 §2).
#[derive(Debug, Default)]
struct Store {
    path: Option<PathBuf>,
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl Store {
    fn load(path: PathBuf) -> Self {
        let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut current = String::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = name.to_string();
            } else if let Some((k, v)) = line.split_once('=') {
                sections
                    .entry(current.clone())
                    .or_default()
                    .insert(k.trim().to_string(), v.to_string());
            }
        }
        Self {
            path: Some(path),
            sections,
        }
    }

    fn get(&self, section: &str, key: &str) -> Option<String> {
        self.sections.get(section)?.get(key).cloned()
    }

    fn set(&mut self, section: &str, key: &str, value: Option<&str>) {
        let clean = |s: &str| s.replace(['\n', '\r'], " ");
        match value {
            Some(v) => {
                self.sections
                    .entry(section.to_string())
                    .or_default()
                    .insert(clean(key), clean(v));
            }
            None => {
                if let Some(s) = self.sections.get_mut(section) {
                    s.remove(key);
                }
            }
        }
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut out = String::new();
        for (name, keys) in &self.sections {
            if keys.is_empty() {
                continue;
            }
            out.push_str(&format!("[{name}]\n"));
            for (k, v) in keys {
                out.push_str(&format!("{k}={v}\n"));
            }
        }
        let tmp = path.with_extension("ini.tmp");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&tmp, out).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

type Task = Box<dyn FnOnce() + Send>;

struct Services {
    commands: Mutex<Sender<HostCommand>>,
    tasks: Mutex<Sender<Task>>,
    status: Arc<Mutex<PlayerStatus>>,
    store: Arc<Mutex<Store>>,
}

impl HostServices for Services {
    fn command(&self, command: HostCommand) {
        if let Ok(tx) = self.commands.lock() {
            let _ = tx.send(command);
        }
    }

    fn status(&self) -> PlayerStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn setting(&self, section: &str, key: &str) -> Option<String> {
        self.store.lock().ok()?.get(section, key)
    }

    fn set_setting(&self, section: &str, key: &str, value: Option<&str>) {
        if let Ok(mut s) = self.store.lock() {
            s.set(section, key, value);
        }
    }

    fn post_to_ui(&self, task: Task) {
        if let Ok(tx) = self.tasks.lock() {
            let _ = tx.send(task);
        }
    }
}

/// What general modules last heard, to send only changes.
#[derive(Debug, Default)]
struct Seen {
    playing: Option<(usize, String)>,
    state: Option<PlayState>,
    title: String,
    volume: Option<u8>,
    playlist_len: Option<usize>,
    kbps: Option<Option<u32>>,
}

/// Loaded plugins and their connection to the player.
pub struct PluginRuntime {
    pub plugins: Plugins,
    commands: Receiver<HostCommand>,
    tasks: Receiver<Task>,
    status: Arc<Mutex<PlayerStatus>>,
    store: Arc<Mutex<Store>>,
    seen: Seen,
}

/// The system plugin folder: `<prefix>/lib/nulamp/plugins` for an installed binary in
/// `<prefix>/bin`, else `plugins` next to the executable.
fn system_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    if dir.file_name().is_some_and(|n| n == "bin")
        && let Some(prefix) = dir.parent()
    {
        let lib = prefix.join("lib/nulamp/plugins");
        if lib.is_dir() {
            return Some(lib);
        }
    }
    Some(dir.join("plugins"))
}

impl PluginRuntime {
    /// Loads the plugins of the system folder and `<config>/plugins` (none in safe mode).
    pub fn load(config: &Path, safe_mode: bool) -> Self {
        let (cmd_tx, commands) = channel();
        let (task_tx, tasks) = channel();
        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let store = Arc::new(Mutex::new(Store::load(config.join("plugins.ini"))));
        let services = Arc::new(Services {
            commands: Mutex::new(cmd_tx),
            tasks: Mutex::new(task_tx),
            status: status.clone(),
            store: store.clone(),
        });
        let host = Host::new(services);
        let opts = LoadOptions {
            system_dir: system_dir(),
            user_dir: Some(config.join("plugins")),
            safe_mode,
        };
        // SAFETY: plugins come from the user's own plugin folders, which the user trusts like
        // the program itself (08 §2); `/SAFE` starts without them.
        let plugins = unsafe { nulamp_plugin_api::load_plugins(&opts, &host) };
        for m in plugins.modules() {
            log::info!(
                "plugin module: {} ({:?}, {})",
                m.info().name,
                m.info().kind,
                m.info().section
            );
        }
        Self {
            plugins,
            commands,
            tasks,
            status,
            store,
            seen: Seen::default(),
        }
    }

    /// The modules in dialog order.
    fn modules(&self) -> Vec<Module> {
        self.plugins.modules().cloned().collect()
    }

    /// The saved active effect (its settings section).
    pub fn active_dsp(&self) -> Option<String> {
        self.store.lock().ok()?.get(HOST_SECTION, "active_dsp")
    }

    /// A new instance of the saved active effect.
    pub fn active_dsp_stage(&self) -> Option<Box<dyn nulamp_audio::chain::Stage>> {
        let section = self.active_dsp()?;
        let dsp = self
            .plugins
            .dsps
            .iter()
            .find(|d| d.module().info().section == section)?;
        match dsp.instantiate() {
            Ok(stage) => Some(Box::new(stage)),
            Err(e) => {
                log::warn!("effect {section}: {e}");
                None
            }
        }
    }

    pub fn rows(&self) -> (Vec<PluginRow>, usize) {
        let active = self.active_dsp();
        let rows = self
            .modules()
            .iter()
            .map(|m| {
                let i = m.info();
                PluginRow {
                    kind: match i.kind {
                        ModuleKind::Input => "Input",
                        ModuleKind::Dsp => "DSP",
                        ModuleKind::Vis => "Visualization",
                        ModuleKind::General => "General",
                    }
                    .into(),
                    name: i.name.clone(),
                    file: i
                        .path
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    can_configure: i.has_configure,
                    can_about: i.has_about,
                    selectable: i.kind == ModuleKind::Dsp,
                    active: i.kind == ModuleKind::Dsp && active.as_deref() == Some(&i.section),
                }
            })
            .collect();
        (rows, self.plugins.errors.len())
    }

    /// Runs a dialog action. Returns true when the active effect changed.
    pub fn action(&mut self, index: usize, action: PluginAction) -> bool {
        let Some(m) = self.modules().get(index).cloned() else {
            return false;
        };
        match action {
            PluginAction::Configure => {
                m.configure();
                false
            }
            PluginAction::About => {
                m.about();
                false
            }
            PluginAction::ToggleActive => {
                let section = m.info().section.clone();
                let on = self.active_dsp().as_deref() != Some(&section);
                if let Ok(mut s) = self.store.lock() {
                    s.set(HOST_SECTION, "active_dsp", on.then_some(section.as_str()));
                }
                true
            }
        }
    }

    /// Updates the status plugins read, runs queued UI tasks, and returns the commands
    /// plugins sent. UI thread.
    pub fn tick(&mut self, status: &PlayerStatus, location: Option<&str>) -> Vec<HostCommand> {
        if let Ok(mut s) = self.status.lock() {
            *s = status.clone();
        }
        while let Ok(task) = self.tasks.try_recv() {
            task();
        }
        self.send_changes(status, location);
        self.commands.try_iter().collect()
    }

    fn send_changes(&mut self, s: &PlayerStatus, location: Option<&str>) {
        if self.plugins.generals.is_empty() {
            return;
        }
        let mut events = Vec::new();
        let seen = &mut self.seen;
        let playing = (s.state != PlayState::Stopped)
            .then(|| location.map(|l| (s.position, l.to_string())))
            .flatten();
        if playing.is_some() && playing != seen.playing {
            let (position, location) = playing.clone().unwrap_or_default();
            events.push(PluginEvent::TrackStarted { location, position });
        }
        if seen.state != Some(s.state) {
            match (seen.state, s.state) {
                (Some(PlayState::Playing), PlayState::Paused) => events.push(PluginEvent::Paused),
                (Some(PlayState::Paused), PlayState::Playing) => events.push(PluginEvent::Unpaused),
                (Some(_), PlayState::Stopped) => events.push(PluginEvent::TrackStopped {
                    by_user: s.position + 1 < s.playlist_len || s.repeat,
                }),
                _ => {}
            }
        }
        seen.playing = playing;
        seen.state = Some(s.state);
        if s.title != seen.title {
            seen.title = s.title.clone();
            events.push(PluginEvent::TitleChanged(s.title.clone()));
        }
        if seen.volume != Some(s.volume) {
            seen.volume = Some(s.volume);
            events.push(PluginEvent::VolumeChanged(s.volume));
        }
        if seen.playlist_len != Some(s.playlist_len) {
            seen.playlist_len = Some(s.playlist_len);
            events.push(PluginEvent::PlaylistChanged {
                len: s.playlist_len,
            });
        }
        if seen.kbps != Some(s.kbps) {
            seen.kbps = Some(s.kbps);
            events.push(PluginEvent::InfoChanged { kbps: s.kbps });
        }
        for e in events {
            self.plugins.broadcast(&e);
        }
    }

    pub fn broadcast(&self, event: &PluginEvent) {
        self.plugins.broadcast(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trip() {
        let dir = std::env::temp_dir().join(format!("nulamp-plugstore-{}", std::process::id()));
        let path = dir.join("plugins.ini");
        let mut s = Store::load(path.clone());
        s.set("plugin:x:Gain", "gain", Some("0.5"));
        s.set("plugin:x:Gain", "note", Some("a\nb"));
        s.set("nulamp", "active_dsp", Some("plugin:x:Gain"));
        s.set("nulamp", "active_dsp", None);
        let s2 = Store::load(path);
        assert_eq!(s2.get("plugin:x:Gain", "gain").as_deref(), Some("0.5"));
        assert_eq!(s2.get("plugin:x:Gain", "note").as_deref(), Some("a b"));
        assert_eq!(s2.get("nulamp", "active_dsp"), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
