//! The host side of `NulampHost`: what plugins call back into.

use std::borrow::Cow;
use std::ffi::{CStr, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, RwLock, Weak};

use nulamp_core::command::PlayerStatus;

use crate::command::{HostCommand, command_from_id};
use crate::module::LoadedLib;
use crate::sys::*;

/// What the application provides to plugins. Every method may be called from any thread,
/// including from inside a call the host made into a plugin, so implementations must not block
/// on the user-interface thread.
pub trait HostServices: Send + Sync {
    /// Queues a command for the user-interface thread. Must not run it synchronously.
    fn command(&self, command: HostCommand);

    /// The current player state.
    fn status(&self) -> PlayerStatus;

    /// The current title.
    fn title(&self) -> String {
        self.status().title
    }

    /// One log line from a module (`module` is its display label).
    fn log(&self, module: &str, level: log::Level, message: &str) {
        log::log!(target: "nulamp::plugin", level, "{module}: {message}");
    }

    /// A value from the settings store (11 §1).
    fn setting(&self, section: &str, key: &str) -> Option<String>;

    /// Stores a value; `None` deletes the key.
    fn set_setting(&self, section: &str, key: &str, value: Option<&str>);

    /// Runs `task` later on the user-interface thread.
    fn post_to_ui(&self, task: Box<dyn FnOnce() + Send>);
}

struct ModuleSlot {
    label: String,
    section: String,
    lib: Weak<LoadedLib>,
}

/// The `NulampHost` record handed to plugins, with the services behind it. Shared by every
/// loaded plugin; it lives as long as any of them.
pub struct Host {
    raw: NulampHost,
    services: Arc<dyn HostServices>,
    modules: RwLock<Vec<ModuleSlot>>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host").finish_non_exhaustive()
    }
}

impl Host {
    pub fn new(services: Arc<dyn HostServices>) -> Arc<Self> {
        Arc::new_cyclic(|me: &Weak<Self>| Self {
            raw: NulampHost {
                abi_version: NULAMP_ABI_VERSION,
                struct_size: size_of::<NulampHost>() as u32,
                // The allocation address is fixed by `new_cyclic` before construction.
                ctx: me.as_ptr() as *mut c_void,
                send_command: Some(host_send_command),
                get_status: Some(host_get_status),
                get_title: Some(host_get_title),
                log: Some(host_log),
                settings_get: Some(host_settings_get),
                settings_set: Some(host_settings_set),
                post_to_ui: Some(host_post_to_ui),
            },
            services,
            modules: RwLock::new(Vec::new()),
        })
    }

    pub fn services(&self) -> &Arc<dyn HostServices> {
        &self.services
    }

    /// The record passed to plugins.
    pub(crate) fn raw(&self) -> *const NulampHost {
        &self.raw
    }

    /// The settings section of a module: `plugin:<file stem>:<module name>`, with characters
    /// other than ASCII letters, digits, `-`, `_` and `.` replaced by `_`.
    pub fn section_name(file_stem: &str, module_name: &str) -> String {
        let clean = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        };
        format!("plugin:{}:{}", clean(file_stem), clean(module_name))
    }

    /// Registers a module and returns its id (from 1).
    pub(crate) fn register(&self, label: String, section: String, lib: Weak<LoadedLib>) -> u64 {
        let mut m = self.modules.write().unwrap_or_else(|e| e.into_inner());
        m.push(ModuleSlot {
            label,
            section,
            lib,
        });
        m.len() as u64
    }

    fn with_slot<R>(&self, id: u64, f: impl FnOnce(&ModuleSlot) -> R) -> Option<R> {
        let m = self.modules.read().unwrap_or_else(|e| e.into_inner());
        let i = usize::try_from(id).ok()?.checked_sub(1)?;
        m.get(i).map(f)
    }
}

/// Runs `f`, turning a panic into `fallback` so that it never unwinds into C.
fn guard<R>(fallback: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or({
        // The panic message was already printed by the panic hook.
        fallback
    })
}

/// The host behind a `NulampHost` pointer.
///
/// # Safety
///
/// `raw` must be null or point to a `NulampHost` created by [`Host::new`] (or a copy of one)
/// whose `Host` is still alive.
unsafe fn host_of<'a>(raw: *const NulampHost) -> Option<&'a Host> {
    if raw.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees `raw` points to a live record.
    let ctx = unsafe { (*raw).ctx };
    if ctx.is_null() {
        return None;
    }
    // SAFETY: `ctx` was set by `Host::new` to the address of the `Host`, which is alive.
    Some(unsafe { &*(ctx as *const Host) })
}

/// A string argument from a plugin (lossy UTF-8).
///
/// # Safety
///
/// `p` must be null or point to a NUL-terminated string valid for the returned lifetime.
pub(crate) unsafe fn str_arg<'a>(p: *const c_char) -> Option<Cow<'a, str>> {
    if p.is_null() {
        None
    } else {
        // SAFETY: guaranteed by the caller.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy())
    }
}

/// Copies an output record into a caller-prepared one, writing only the bytes that fit in the
/// caller's `struct_size` and keeping that size. Returns false when the size is too small.
///
/// # Safety
///
/// `out` must be null or point to a writable record of type `T` (or a shorter, older version
/// of it whose `struct_size` says so). `T` must start with `abi_version: u32, struct_size: u32`.
pub(crate) unsafe fn fill_out<T: Copy>(out: *mut T, rec: T) -> bool {
    if out.is_null() {
        return false;
    }
    let header = out.cast::<u32>();
    // SAFETY: every record starts with two `u32` fields, which the caller prepared.
    let size = unsafe { header.add(1).read() } as usize;
    if size < 8 {
        return false;
    }
    let n = size.min(size_of::<T>());
    // SAFETY: `out` is writable for `size` bytes and `n <= size`; `rec` is a local `T` of at
    // least `n` bytes; the regions do not overlap.
    unsafe {
        std::ptr::copy_nonoverlapping((&rec as *const T).cast::<u8>(), out.cast::<u8>(), n);
        header.write(NULAMP_ABI_VERSION);
        header.add(1).write(size as u32);
    }
    true
}

fn status_record(s: &PlayerStatus) -> NulampStatus {
    let opt = |v: Option<u32>| v.unwrap_or(0);
    NulampStatus {
        state: s.state.remote_value(),
        volume: s.volume as i32,
        balance: s.balance as i32,
        kbps: s.kbps.map_or(-1, |k| k.min(i32::MAX as u32) as i32),
        position_ms: s.position_ms.min(i64::MAX as u64) as i64,
        length_ms: s.length_ms.map_or(-1, |l| l.min(i64::MAX as u64) as i64),
        sample_rate: opt(s.sample_rate),
        channels: opt(s.channels),
        playlist_position: s.position.min(u32::MAX as usize) as u32,
        playlist_length: s.playlist_len.min(u32::MAX as usize) as u32,
        seekable: s.seekable as u8,
        in_sync: s.in_sync as u8,
        shuffle: s.shuffle as u8,
        repeat: s.repeat as u8,
        manual_advance: s.manual_advance as u8,
        stop_after_current: s.stop_after_current as u8,
        ..NulampStatus::prepared()
    }
}

fn level_of(level: i32) -> log::Level {
    match level {
        ..=NULAMP_LOG_ERROR => log::Level::Error,
        NULAMP_LOG_WARN => log::Level::Warn,
        NULAMP_LOG_INFO => log::Level::Info,
        NULAMP_LOG_DEBUG => log::Level::Debug,
        _ => log::Level::Trace,
    }
}

unsafe extern "C" fn host_send_command(host: *const NulampHost, command: u32, arg: i64) -> i32 {
    guard(NULAMP_ERR_FAILED, || {
        // SAFETY: plugins pass the record they were given; it outlives them.
        let Some(h) = (unsafe { host_of(host) }) else {
            return NULAMP_ERR_INVALID;
        };
        match command_from_id(command, arg) {
            Some(c) => {
                h.services.command(c);
                NULAMP_OK
            }
            None => NULAMP_ERR_UNSUPPORTED,
        }
    })
}

unsafe extern "C" fn host_get_status(host: *const NulampHost, out: *mut NulampStatus) -> i32 {
    guard(NULAMP_ERR_FAILED, || {
        // SAFETY: as in `host_send_command`.
        let Some(h) = (unsafe { host_of(host) }) else {
            return NULAMP_ERR_INVALID;
        };
        let rec = status_record(&h.services.status());
        // SAFETY: the ABI requires `out` to be a prepared, writable record.
        if unsafe { fill_out(out, rec) } {
            NULAMP_OK
        } else {
            NULAMP_ERR_INVALID
        }
    })
}

unsafe extern "C" fn host_get_title(host: *const NulampHost, buf: *mut c_char, len: u32) -> i32 {
    guard(NULAMP_ERR_FAILED, || {
        // SAFETY: as in `host_send_command`.
        let Some(h) = (unsafe { host_of(host) }) else {
            return NULAMP_ERR_INVALID;
        };
        let title = h.services.title();
        // SAFETY: the ABI requires `buf` to be writable for `len` bytes.
        unsafe { write_str_out(&title, buf, len) }
    })
}

unsafe extern "C" fn host_log(
    host: *const NulampHost,
    module_id: u64,
    level: i32,
    msg: *const c_char,
) {
    guard((), || {
        // SAFETY: as in `host_send_command`.
        let Some(h) = (unsafe { host_of(host) }) else {
            return;
        };
        // SAFETY: the ABI requires a NUL-terminated string valid during the call.
        let Some(msg) = (unsafe { str_arg(msg) }) else {
            return;
        };
        let label = h
            .with_slot(module_id, |s| s.label.clone())
            .unwrap_or_else(|| "plugin".into());
        h.services.log(&label, level_of(level), &msg);
    })
}

unsafe extern "C" fn host_settings_get(
    host: *const NulampHost,
    module_id: u64,
    key: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> i32 {
    guard(NULAMP_ERR_FAILED, || {
        // SAFETY: as in `host_send_command`.
        let Some(h) = (unsafe { host_of(host) }) else {
            return NULAMP_ERR_INVALID;
        };
        // SAFETY: as in `host_log`.
        let Some(key) = (unsafe { str_arg(key) }) else {
            return NULAMP_ERR_INVALID;
        };
        let Some(section) = h.with_slot(module_id, |s| s.section.clone()) else {
            return NULAMP_ERR_INVALID;
        };
        match h.services.setting(&section, &key) {
            // SAFETY: the ABI requires `buf` to be writable for `len` bytes.
            Some(v) => unsafe { write_str_out(&v, buf, len) },
            None => NULAMP_ERR_NOT_FOUND,
        }
    })
}

unsafe extern "C" fn host_settings_set(
    host: *const NulampHost,
    module_id: u64,
    key: *const c_char,
    value: *const c_char,
) -> i32 {
    guard(NULAMP_ERR_FAILED, || {
        // SAFETY: as in `host_send_command`.
        let Some(h) = (unsafe { host_of(host) }) else {
            return NULAMP_ERR_INVALID;
        };
        // SAFETY: as in `host_log`; `value` may be null.
        let (key, value) = unsafe { (str_arg(key), str_arg(value)) };
        let Some(key) = key else {
            return NULAMP_ERR_INVALID;
        };
        let Some(section) = h.with_slot(module_id, |s| s.section.clone()) else {
            return NULAMP_ERR_INVALID;
        };
        h.services.set_setting(&section, &key, value.as_deref());
        NULAMP_OK
    })
}

/// A plugin callback queued for the user-interface thread.
struct UiCall {
    cb: NulampUiCallback,
    data: *mut c_void,
    /// Keeps the plugin file loaded until the callback has run.
    _lib: Option<Arc<LoadedLib>>,
}

// SAFETY: the ABI makes the plugin responsible for `data` being usable on the user-interface
// thread; the host only carries the pointer there.
unsafe impl Send for UiCall {}

impl UiCall {
    fn run(self) {
        // SAFETY: `cb` is the plugin's callback, its file is still loaded (`_lib`), and the
        // plugin asked for exactly this call.
        unsafe { (self.cb)(self.data) };
    }
}

unsafe extern "C" fn host_post_to_ui(
    host: *const NulampHost,
    module_id: u64,
    cb: Option<NulampUiCallback>,
    data: *mut c_void,
) -> i32 {
    guard(NULAMP_ERR_FAILED, || {
        // SAFETY: as in `host_send_command`.
        let Some(h) = (unsafe { host_of(host) }) else {
            return NULAMP_ERR_INVALID;
        };
        let Some(cb) = cb else {
            return NULAMP_ERR_INVALID;
        };
        let Some(lib) = h.with_slot(module_id, |s| s.lib.upgrade()) else {
            return NULAMP_ERR_INVALID;
        };
        let call = UiCall {
            cb,
            data,
            _lib: lib,
        };
        h.services.post_to_ui(Box::new(move || call.run()));
        NULAMP_OK
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nulamp_core::command::{Command, PlayState};

    /// A recording implementation of the host services.
    #[derive(Default)]
    pub(crate) struct Recorder {
        pub commands: Mutex<Vec<HostCommand>>,
        pub logs: Mutex<Vec<(String, log::Level, String)>>,
        pub settings: Mutex<Vec<(String, String, String)>>,
        pub ui: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
        pub panic_on_status: bool,
    }

    impl Recorder {
        pub fn run_ui(&self) {
            let tasks: Vec<_> = self.ui.lock().unwrap().drain(..).collect();
            for t in tasks {
                t();
            }
        }
    }

    impl HostServices for Recorder {
        fn command(&self, command: HostCommand) {
            self.commands.lock().unwrap().push(command);
        }
        fn status(&self) -> PlayerStatus {
            assert!(!self.panic_on_status, "status panics");
            PlayerStatus {
                state: PlayState::Paused,
                volume: 200,
                balance: -3,
                length_ms: None,
                kbps: Some(128),
                position: 2,
                playlist_len: 9,
                title: "Ünïcode title".into(),
                shuffle: true,
                ..PlayerStatus::default()
            }
        }
        fn log(&self, module: &str, level: log::Level, message: &str) {
            self.logs
                .lock()
                .unwrap()
                .push((module.into(), level, message.into()));
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

    fn host() -> (Arc<Recorder>, Arc<Host>) {
        let rec = Arc::new(Recorder::default());
        let host = Host::new(rec.clone());
        (rec, host)
    }

    #[test]
    fn commands_and_status() {
        let (rec, host) = host();
        let raw = host.raw();
        // SAFETY: `raw` is a live host record; the functions are the host's own.
        unsafe {
            let r = &*raw;
            assert_eq!(r.send_command.unwrap()(raw, NULAMP_CMD_PLAY, 0), NULAMP_OK);
            assert_eq!(
                r.send_command.unwrap()(raw, NULAMP_CMD_SET_VOLUME, 77),
                NULAMP_OK
            );
            assert_eq!(
                r.send_command.unwrap()(raw, 12345, 0),
                NULAMP_ERR_UNSUPPORTED
            );
            let mut st = NulampStatus::prepared();
            assert_eq!(r.get_status.unwrap()(raw, &mut st), NULAMP_OK);
            assert_eq!(st.state, NULAMP_STATE_PAUSED);
            assert_eq!((st.volume, st.balance, st.kbps), (200, -3, 128));
            assert_eq!((st.length_ms, st.playlist_position, st.shuffle), (-1, 2, 1));
            assert_eq!(st.struct_size as usize, size_of::<NulampStatus>());

            // An older, shorter record only gets the fields that fit.
            let mut short = NulampStatus {
                struct_size: 16,
                ..NulampStatus::default()
            };
            assert_eq!(r.get_status.unwrap()(raw, &mut short), NULAMP_OK);
            assert_eq!((short.state, short.volume, short.balance), (3, 200, 0));
            assert_eq!(short.struct_size, 16);

            let mut buf = [0 as c_char; 64];
            let n = r.get_title.unwrap()(raw, buf.as_mut_ptr(), 64);
            assert_eq!(n as usize, "Ünïcode title".len());
            assert_eq!(
                CStr::from_ptr(buf.as_ptr()).to_str().unwrap(),
                "Ünïcode title"
            );
        }
        assert_eq!(
            *rec.commands.lock().unwrap(),
            [
                HostCommand::Player(Command::Play),
                HostCommand::Player(Command::SetVolume(77))
            ]
        );
    }

    #[test]
    fn settings_logs_and_ui_calls() {
        let (rec, host) = host();
        let id = host.register(
            "Test module".into(),
            Host::section_name("libtest", "Test module"),
            Weak::new(),
        );
        assert_eq!(id, 1);
        let raw = host.raw();
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn on_ui(data: *mut c_void) {
            CALLS.fetch_add(data as usize, Ordering::SeqCst);
        }
        // SAFETY: as in `commands_and_status`; the strings are NUL-terminated literals.
        unsafe {
            let r = &*raw;
            assert_eq!(
                r.settings_set.unwrap()(raw, id, c"k".as_ptr(), c"välue".as_ptr()),
                NULAMP_OK
            );
            let mut buf = [0 as c_char; 4];
            let n = r.settings_get.unwrap()(raw, id, c"k".as_ptr(), buf.as_mut_ptr(), 4);
            assert_eq!(n, 6);
            assert_eq!(CStr::from_ptr(buf.as_ptr()).to_bytes(), b"v\xc3\xa4");
            assert_eq!(
                r.settings_get.unwrap()(raw, id, c"missing".as_ptr(), buf.as_mut_ptr(), 4),
                NULAMP_ERR_NOT_FOUND
            );
            assert_eq!(
                r.settings_get.unwrap()(raw, 99, c"k".as_ptr(), buf.as_mut_ptr(), 4),
                NULAMP_ERR_INVALID
            );
            assert_eq!(
                r.settings_set.unwrap()(raw, id, c"k".as_ptr(), std::ptr::null()),
                NULAMP_OK
            );
            r.log.unwrap()(raw, id, NULAMP_LOG_WARN, c"hello".as_ptr());
            r.log.unwrap()(raw, 0, 99, c"anon".as_ptr());
            assert_eq!(
                r.post_to_ui.unwrap()(raw, id, Some(on_ui), 5 as *mut c_void),
                NULAMP_OK
            );
            assert_eq!(
                r.post_to_ui.unwrap()(raw, id, None, std::ptr::null_mut()),
                NULAMP_ERR_INVALID
            );
        }
        assert!(rec.settings.lock().unwrap().is_empty());
        assert_eq!(
            *rec.logs.lock().unwrap(),
            [
                ("Test module".into(), log::Level::Warn, "hello".into()),
                ("plugin".into(), log::Level::Trace, "anon".into())
            ]
        );
        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
        rec.run_ui();
        assert_eq!(CALLS.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn panics_do_not_cross_the_boundary() {
        let rec = Arc::new(Recorder {
            panic_on_status: true,
            ..Recorder::default()
        });
        let host = Host::new(rec);
        let raw = host.raw();
        let mut st = NulampStatus::prepared();
        // SAFETY: as in `commands_and_status`.
        let r = unsafe { (*raw).get_status.unwrap()(raw, &mut st) };
        assert_eq!(r, NULAMP_ERR_FAILED);
        // SAFETY: a null host is rejected.
        let r = unsafe { (*raw).send_command.unwrap()(std::ptr::null(), NULAMP_CMD_PLAY, 0) };
        assert_eq!(r, NULAMP_ERR_INVALID);
    }

    #[test]
    fn section_names() {
        assert_eq!(
            Host::section_name("libgain", "Vocal remover [v2]"),
            "plugin:libgain:Vocal_remover__v2_"
        );
    }
}
