//! Plugin discovery and loading (08 §2).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::PluginError;
use crate::dsp::DspModule;
use crate::general::{PluginEvent, PluginGeneral};
use crate::host::Host;
use crate::input::InputModule;
use crate::module::{LoadedLib, Module, ModuleKind, init_modules};
use crate::sys::{NULAMP_PLUGIN_ENTRY_NAME, NulampPluginEntryFn};
use crate::vis::VisModule;

/// Where to look for plugins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadOptions {
    pub system_dir: Option<PathBuf>,
    pub user_dir: Option<PathBuf>,
    /// Load nothing (08 §2 "safe mode").
    pub safe_mode: bool,
}

/// Plugin files in the two directories, in load order: sorted by file name, system directory
/// first. A file in the user directory replaces a system file with the same name. Only the
/// platform's shared-library extension (`so`, `dylib` or `dll`) is considered; subdirectories
/// are not searched. Missing directories are skipped.
pub fn discover(opts: &LoadOptions) -> Vec<PathBuf> {
    if opts.safe_mode {
        return Vec::new();
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in [&opts.system_dir, &opts.user_dir].into_iter().flatten() {
        let mut files = list_libraries(dir);
        files.sort();
        for f in files {
            found.retain(|p| p.file_name() != f.file_name());
            found.push(f);
        }
    }
    found
}

fn list_libraries(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(std::env::consts::DLL_EXTENSION))
        })
        .collect()
}

/// Every loaded module, by kind, and what failed to load.
#[derive(Debug, Default)]
pub struct Plugins {
    pub inputs: Vec<InputModule>,
    pub dsps: Vec<DspModule>,
    pub vis: Vec<VisModule>,
    pub generals: Vec<PluginGeneral>,
    /// Files and modules that were not loaded, with the reason.
    pub errors: Vec<(PathBuf, PluginError)>,
}

/// Loads every plugin of [`discover`] and initialises its modules, on the calling thread
/// (the user-interface thread). Failures are collected in [`Plugins::errors`] and logged.
///
/// # Safety
///
/// Loading a plugin runs its native code, which can do anything; the plugin directories must
/// be trusted, and the plugins must follow the ABI in `include/nulamp_plugin.h`.
pub unsafe fn load_plugins(opts: &LoadOptions, host: &Arc<Host>) -> Plugins {
    let mut plugins = Plugins::default();
    for path in discover(opts) {
        // SAFETY: forwarded to the caller.
        if let Err(e) = unsafe { plugins.load_file(&path, host) } {
            plugins.errors.push((path, e));
        }
    }
    for (path, e) in &plugins.errors {
        log::warn!("plugin {}: {e}", path.display());
    }
    plugins
}

impl Plugins {
    /// Loads one plugin file. Returns the number of modules added; modules that failed are
    /// added to [`Plugins::errors`].
    ///
    /// # Safety
    ///
    /// As [`load_plugins`].
    pub unsafe fn load_file(
        &mut self,
        path: &Path,
        host: &Arc<Host>,
    ) -> Result<usize, PluginError> {
        // SAFETY: forwarded to the caller (running the library's initialisers).
        let lib = unsafe { libloading::Library::new(path) }
            .map_err(|e| PluginError::Load(e.to_string()))?;
        // SAFETY: the ABI defines the entry point with this signature.
        let entry: NulampPluginEntryFn = unsafe {
            *lib.get::<NulampPluginEntryFn>(NULAMP_PLUGIN_ENTRY_NAME.as_bytes())
                .map_err(|e| PluginError::NoEntry(e.to_string()))?
        };
        let loaded = Arc::new(LoadedLib {
            path: path.to_owned(),
            host: host.clone(),
            _lib: Some(lib),
        });
        // SAFETY: as [`Self::load_builtin`]; `loaded` keeps the code mapped.
        unsafe { self.load_entry(entry, loaded) }
    }

    /// Loads a plugin linked into the program (no file). `name` stands in for the file name
    /// in settings sections.
    ///
    /// # Safety
    ///
    /// `entry` must follow the ABI.
    pub unsafe fn load_builtin(
        &mut self,
        entry: NulampPluginEntryFn,
        name: &str,
        host: &Arc<Host>,
    ) -> Result<usize, PluginError> {
        let loaded = Arc::new(LoadedLib {
            path: PathBuf::from(name),
            host: host.clone(),
            _lib: None,
        });
        // SAFETY: forwarded to the caller.
        unsafe { self.load_entry(entry, loaded) }
    }

    /// # Safety
    ///
    /// `entry` must follow the ABI and stay callable while `loaded` lives.
    unsafe fn load_entry(
        &mut self,
        entry: NulampPluginEntryFn,
        loaded: Arc<LoadedLib>,
    ) -> Result<usize, PluginError> {
        let path = loaded.path.clone();
        let host = loaded.host.clone();
        // SAFETY: the plugin's entry point with the live host record.
        let desc = unsafe { entry(host.raw()) };
        // SAFETY: `desc` comes from the entry point and `loaded` keeps the code mapped.
        let (modules, errors) = unsafe { init_modules(desc, loaded)? };
        self.errors
            .extend(errors.into_iter().map(|e| (path.clone(), e)));
        let n = modules.len();
        for m in modules {
            self.add(m);
        }
        Ok(n)
    }

    fn add(&mut self, m: Module) {
        match m.info().kind {
            ModuleKind::Input => self.inputs.push(InputModule::new(m)),
            ModuleKind::Dsp => self.dsps.push(DspModule::new(m)),
            ModuleKind::Vis => self.vis.push(VisModule::new(m)),
            ModuleKind::General => self.generals.push(PluginGeneral::new(m)),
        }
    }

    /// Every loaded module.
    pub fn modules(&self) -> impl Iterator<Item = &Module> {
        self.inputs
            .iter()
            .map(|m| m.module())
            .chain(self.dsps.iter().map(|m| m.module()))
            .chain(self.vis.iter().map(|m| m.module()))
            .chain(self.generals.iter().map(|m| m.module()))
    }

    /// Sends an event to every general module. User-interface thread only.
    pub fn broadcast(&self, event: &PluginEvent) {
        for g in &self.generals {
            g.send(event);
        }
    }

    /// Adds a decoder factory per input module to a registry, after the ones it has. Built-in
    /// decoders therefore win extension matches; a plugin's content check (`claims`) still
    /// comes first.
    pub fn register_decoders(&self, registry: &mut nulamp_decoders::Registry) {
        for i in &self.inputs {
            registry.add(Box::new(i.factory()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_order_and_override() {
        let base =
            std::env::temp_dir().join(format!("nulamp-plugin-discover-{}", std::process::id()));
        let (sys, user) = (base.join("sys"), base.join("user"));
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        let ext = std::env::consts::DLL_EXTENSION;
        for (dir, name) in [(&sys, "b"), (&sys, "a"), (&user, "b"), (&user, "c")] {
            std::fs::write(dir.join(format!("{name}.{ext}")), b"").unwrap();
        }
        std::fs::write(sys.join("readme.txt"), b"").unwrap();
        std::fs::create_dir_all(sys.join(format!("dir.{ext}"))).unwrap();
        let mut opts = LoadOptions {
            system_dir: Some(sys.clone()),
            user_dir: Some(user.clone()),
            safe_mode: false,
        };
        let found = discover(&opts);
        assert_eq!(
            found,
            [
                sys.join(format!("a.{ext}")),
                user.join(format!("b.{ext}")),
                user.join(format!("c.{ext}"))
            ]
        );
        opts.safe_mode = true;
        assert!(discover(&opts).is_empty());
        opts.safe_mode = false;
        opts.system_dir = Some(base.join("missing"));
        assert_eq!(discover(&opts).len(), 2);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn broken_files_are_reported() {
        let base =
            std::env::temp_dir().join(format!("nulamp-plugin-broken-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let ext = std::env::consts::DLL_EXTENSION;
        std::fs::write(base.join(format!("broken.{ext}")), b"not a library").unwrap();
        let host = Host::new(std::sync::Arc::new(crate::host::tests::Recorder::default()));
        let opts = LoadOptions {
            user_dir: Some(base.clone()),
            ..LoadOptions::default()
        };
        // SAFETY: the only file is not a library, so no code runs.
        let p = unsafe { load_plugins(&opts, &host) };
        assert_eq!(p.modules().count(), 0);
        assert_eq!(p.errors.len(), 1);
        assert!(matches!(p.errors[0].1, PluginError::Load(_)));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
