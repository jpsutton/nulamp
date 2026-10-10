//! Loaded modules: validation of the plugin's records, `init`/`quit`, and the shared handle the
//! adapters keep.

use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::PluginError;
use crate::host::Host;
use crate::sys::*;

/// A loaded plugin file (or a built-in plugin, with no library). Dropped after every module
/// of the file, which unloads the file.
pub(crate) struct LoadedLib {
    pub(crate) path: PathBuf,
    pub(crate) host: Arc<Host>,
    /// Dropped last: unloading invalidates every pointer into the plugin.
    pub(crate) _lib: Option<libloading::Library>,
}

/// Module kinds (08 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleKind {
    Input,
    Dsp,
    Vis,
    General,
}

impl ModuleKind {
    fn from_raw(kind: u32) -> Option<Self> {
        Some(match kind {
            NULAMP_KIND_INPUT => Self::Input,
            NULAMP_KIND_DSP => Self::Dsp,
            NULAMP_KIND_VIS => Self::Vis,
            NULAMP_KIND_GENERAL => Self::General,
            _ => return None,
        })
    }
}

/// What a module reports about itself (08 §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    pub kind: ModuleKind,
    pub name: String,
    pub version: String,
    pub description: String,
    /// The plugin file's own name, version and description.
    pub plugin_name: String,
    pub plugin_version: String,
    pub plugin_description: String,
    /// The plugin file; empty for built-in plugins.
    pub path: PathBuf,
    /// Settings section of the module (`plugin:<file stem>:<module name>`).
    pub section: String,
    /// The module's ABI version.
    pub abi_version: u32,
    pub has_configure: bool,
    pub has_about: bool,
}

pub(crate) struct ModuleInner {
    pub(crate) raw: NulampModule,
    pub(crate) info: ModuleInfo,
    pub(crate) id: u64,
    /// Declared after the fields above so the file stays loaded until `quit` has run.
    pub(crate) lib: Arc<LoadedLib>,
}

// SAFETY: the module record is immutable plugin data. Module-level functions follow the ABI's
// threading rules: the adapters call each function only from the threads the ABI allows for
// it, and the plugin must make the functions the ABI calls "any thread" thread-safe.
unsafe impl Send for ModuleInner {}
// SAFETY: as above.
unsafe impl Sync for ModuleInner {}

impl Drop for ModuleInner {
    fn drop(&mut self) {
        if let Some(quit) = self.raw.quit {
            // SAFETY: `init` succeeded (otherwise no `ModuleInner` exists), every instance
            // created from this module is gone (they hold this module), and the file is still
            // loaded (`lib` is dropped after this).
            unsafe { quit(self.raw.user) };
        }
    }
}

/// A module whose `init` succeeded. Cloning shares it; `quit` runs when the last clone (and the
/// last instance created from it) is dropped, which should happen on the user-interface thread.
#[derive(Clone)]
pub struct Module(pub(crate) Arc<ModuleInner>);

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module")
            .field("id", &self.0.id)
            .field("info", &self.0.info)
            .finish()
    }
}

impl Module {
    pub fn info(&self) -> &ModuleInfo {
        &self.0.info
    }

    /// The host id passed to the module's `init`.
    pub fn id(&self) -> u64 {
        self.0.id
    }

    pub fn host(&self) -> &Arc<Host> {
        &self.0.lib.host
    }

    /// Shows the module's settings dialog. User-interface thread only. False when it has none.
    pub fn configure(&self) -> bool {
        let Some(f) = self.0.raw.configure else {
            return false;
        };
        // SAFETY: a valid module function, called on the user-interface thread as the ABI
        // requires (documented on this method).
        unsafe { f(self.0.raw.user) };
        true
    }

    /// Shows the module's about box. User-interface thread only. False when it has none.
    pub fn about(&self) -> bool {
        let Some(f) = self.0.raw.about else {
            return false;
        };
        // SAFETY: as in `configure`.
        unsafe { f(self.0.raw.user) };
        true
    }

    pub(crate) fn user(&self) -> *mut c_void {
        self.0.raw.user
    }

    pub(crate) fn api(&self) -> *const c_void {
        self.0.raw.api
    }
}

/// A plugin string (lossy UTF-8; null is empty).
///
/// # Safety
///
/// `p` must be null or a NUL-terminated string.
pub(crate) unsafe fn plugin_str(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

/// Checks the common header of a plugin record.
///
/// # Safety
///
/// `p` must be null or point to readable memory of at least 8 bytes.
pub(crate) unsafe fn check_header<T>(p: *const T, what: &str) -> Result<(), PluginError> {
    if p.is_null() {
        return Err(PluginError::Invalid(format!("{what} is null")));
    }
    let header = p.cast::<u32>();
    // SAFETY: every record starts with `abi_version: u32, struct_size: u32`.
    let (version, size) = unsafe { (header.read(), header.add(1).read()) };
    if abi_major(version) != NULAMP_ABI_MAJOR {
        return Err(PluginError::Abi {
            found: version,
            expected: NULAMP_ABI_VERSION,
        });
    }
    if (size as usize) < size_of::<T>() {
        return Err(PluginError::Invalid(format!(
            "{what} is {size} bytes, at least {} expected",
            size_of::<T>()
        )));
    }
    Ok(())
}

/// Most modules one file may hold.
const MAX_MODULES: u32 = 1024;

/// Validates a plugin description and initialises its modules. Modules that fail validation
/// or `init` are reported in the error list and not used.
///
/// # Safety
///
/// `desc` must be what the plugin's entry point returned for `host`, and `lib` must keep the
/// plugin's code loaded.
pub(crate) unsafe fn init_modules(
    desc: *const NulampPluginDesc,
    lib: Arc<LoadedLib>,
) -> Result<(Vec<Module>, Vec<PluginError>), PluginError> {
    if desc.is_null() {
        return Err(PluginError::Refused);
    }
    // SAFETY: the plugin returned this pointer; the header is read before the full record.
    unsafe { check_header(desc, "plugin description")? };
    // SAFETY: checked above: non-null and at least as large as the record.
    let d = unsafe { *desc };
    if d.module_count > MAX_MODULES || (d.module_count > 0 && d.modules.is_null()) {
        return Err(PluginError::Invalid("bad module list".into()));
    }
    // SAFETY: the description's strings are NUL-terminated (ABI).
    let (plugin_name, plugin_version, plugin_description) = unsafe {
        (
            plugin_str(d.name),
            plugin_str(d.version),
            plugin_str(d.description),
        )
    };
    let stem = file_stem(&lib.path, &plugin_name);
    let host = lib.host.clone();
    let mut modules = Vec::new();
    let mut errors = Vec::new();
    for i in 0..d.module_count as usize {
        // SAFETY: `modules` holds `module_count` pointers (checked non-null above).
        let m = unsafe { *d.modules.add(i) };
        // SAFETY: a module pointer from the description.
        if let Err(e) = unsafe { check_header(m, "module") } {
            errors.push(e);
            continue;
        }
        // SAFETY: checked above.
        let raw = unsafe { *m };
        // SAFETY: module strings are NUL-terminated (ABI).
        let (name, version, description) = unsafe {
            (
                plugin_str(raw.name),
                plugin_str(raw.version),
                plugin_str(raw.description),
            )
        };
        let name = if name.is_empty() {
            format!("{plugin_name} #{i}")
        } else {
            name
        };
        let Some(kind) = ModuleKind::from_raw(raw.kind) else {
            errors.push(PluginError::Invalid(format!(
                "module {name:?} has unknown kind {}",
                raw.kind
            )));
            continue;
        };
        // SAFETY: `api` points to the record of the module's kind (ABI).
        if let Err(e) = unsafe { check_api(kind, raw.api) } {
            errors.push(PluginError::Invalid(format!("module {name:?}: {e}")));
            continue;
        }
        let (Some(init), Some(_)) = (raw.init, raw.quit) else {
            errors.push(PluginError::Invalid(format!(
                "module {name:?} has no init or quit"
            )));
            continue;
        };
        let section = Host::section_name(&stem, &name);
        let id = host.register(
            format!("{plugin_name}/{name}"),
            section.clone(),
            Arc::downgrade(&lib),
        );
        // SAFETY: a valid module function with its own `user` value, the live host record and
        // the id just registered; called on the loading (user-interface) thread.
        let code = unsafe { init(raw.user, host.raw(), id) };
        if code != NULAMP_OK {
            errors.push(PluginError::InitFailed { module: name, code });
            continue;
        }
        let info = ModuleInfo {
            kind,
            name,
            version,
            description,
            plugin_name: plugin_name.clone(),
            plugin_version: plugin_version.clone(),
            plugin_description: plugin_description.clone(),
            path: lib.path.clone(),
            section,
            abi_version: raw.abi_version,
            has_configure: raw.configure.is_some(),
            has_about: raw.about.is_some(),
        };
        modules.push(Module(Arc::new(ModuleInner {
            raw,
            info,
            id,
            lib: lib.clone(),
        })));
    }
    Ok((modules, errors))
}

fn file_stem(path: &Path, plugin_name: &str) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| plugin_name.to_owned())
}

/// Checks the kind-specific record and its required functions.
///
/// # Safety
///
/// `api` must be null or point to the record of `kind`.
unsafe fn check_api(kind: ModuleKind, api: *const c_void) -> Result<(), PluginError> {
    let missing = |what: &str| Err(PluginError::Invalid(format!("missing {what}")));
    match kind {
        ModuleKind::Input => {
            let p = api.cast::<NulampInputApi>();
            // SAFETY: guaranteed by the caller.
            unsafe { check_header(p, "input record")? };
            // SAFETY: checked above.
            let a = unsafe { *p };
            if a.open.is_none()
                || a.info.is_none()
                || a.read.is_none()
                || a.seek.is_none()
                || a.close.is_none()
            {
                return missing("open, info, read, seek or close");
            }
        }
        ModuleKind::Dsp => {
            let p = api.cast::<NulampDspApi>();
            // SAFETY: guaranteed by the caller.
            unsafe { check_header(p, "DSP record")? };
            // SAFETY: checked above.
            let a = unsafe { *p };
            if a.create.is_none()
                || a.destroy.is_none()
                || a.configure.is_none()
                || a.process.is_none()
            {
                return missing("create, destroy, configure or process");
            }
        }
        ModuleKind::Vis => {
            let p = api.cast::<NulampVisApi>();
            // SAFETY: guaranteed by the caller.
            unsafe { check_header(p, "visualization record")? };
            // SAFETY: checked above.
            let a = unsafe { *p };
            if a.start.is_none() || a.render.is_none() || a.stop.is_none() {
                return missing("start, render or stop");
            }
        }
        ModuleKind::General => {
            let p = api.cast::<NulampGeneralApi>();
            // SAFETY: guaranteed by the caller.
            unsafe { check_header(p, "general record")? };
            // SAFETY: checked above.
            if unsafe { (*p).on_event }.is_none() {
                return missing("on_event");
            }
        }
    }
    Ok(())
}
