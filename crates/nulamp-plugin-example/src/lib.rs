//! An example nulamp plugin written in Rust against the C ABI (`nulamp_plugin_api::sys`).
//!
//! One file, four modules:
//!
//! - DSP "Gain": multiplies every sample by the `gain` setting (default 0.5).
//! - DSP "Vocal remover": both stereo channels become left − right; refuses other layouts.
//! - DSP "Half speed": repeats every frame, so the output has twice the frames.
//! - General "Event logger": logs every player event through the host. With the setting
//!   `restart` = `1` it starts the playlist again when it ends.
//!
//! Every exported function catches panics: none may unwind into the host.

use std::ffi::{CStr, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

use nulamp_plugin_api::sys::*;

/// The host record from the entry point; valid until the file is unloaded.
static HOST: AtomicPtr<NulampHost> = AtomicPtr::new(null_mut());

/// Module ids from `init`, one per module (each module's `user` points to its own).
static GAIN_ID: AtomicU64 = AtomicU64::new(0);
static VOCAL_ID: AtomicU64 = AtomicU64::new(0);
static HALF_ID: AtomicU64 = AtomicU64::new(0);
static LOGGER_ID: AtomicU64 = AtomicU64::new(0);

/// Runs `f`, turning a panic into `fallback`.
fn guard<R>(fallback: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn host() -> Option<&'static NulampHost> {
    let p = HOST.load(Ordering::Acquire);
    // SAFETY: the host record stays valid while this file is loaded (ABI).
    unsafe { p.as_ref() }
}

fn log(module_id: u64, level: i32, msg: &str) {
    let Some(h) = host() else { return };
    let Some(f) = h.log else { return };
    let mut bytes = msg.replace('\0', "").into_bytes();
    bytes.push(0);
    // SAFETY: a host function with its own record and a NUL-terminated string.
    unsafe { f(h, module_id, level, bytes.as_ptr().cast()) };
}

fn setting(module_id: u64, key: &CStr) -> Option<String> {
    let h = host()?;
    let f = h.settings_get?;
    let mut buf = [0u8; 256];
    // SAFETY: a host function; `buf` is writable for its length.
    let n = unsafe {
        f(
            h,
            module_id,
            key.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
        )
    };
    if n < 0 || n as usize >= buf.len() {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

// ---- Common functions ----

unsafe extern "C" fn module_init(user: *mut c_void, host: *const NulampHost, id: u64) -> i32 {
    guard(NULAMP_ERR_FAILED, || {
        // SAFETY: the host passes its own record.
        let Some(h) = (unsafe { host.as_ref() }) else {
            return NULAMP_ERR_INVALID;
        };
        if abi_major(h.abi_version) != NULAMP_ABI_MAJOR {
            return NULAMP_ERR_UNSUPPORTED;
        }
        // SAFETY: every module's `user` is one of the `*_ID` statics.
        let slot = unsafe { &*user.cast::<AtomicU64>() };
        slot.store(id, Ordering::Release);
        log(id, NULAMP_LOG_DEBUG, "init");
        NULAMP_OK
    })
}

unsafe extern "C" fn module_quit(user: *mut c_void) {
    guard((), || {
        // SAFETY: as in `module_init`.
        let id = unsafe { &*user.cast::<AtomicU64>() }.load(Ordering::Acquire);
        log(id, NULAMP_LOG_DEBUG, "quit");
    })
}

/// The Gain module's settings dialog: this example has none to show.
unsafe extern "C" fn gain_configure(_user: *mut c_void) {
    guard((), || {
        log(
            GAIN_ID.load(Ordering::Acquire),
            NULAMP_LOG_INFO,
            "no settings dialog; set `gain` in the settings file",
        );
    })
}

// ---- DSP: shared instance ----

struct Effect {
    kind: EffectKind,
    channels: usize,
}

#[derive(Clone, Copy)]
enum EffectKind {
    Gain(f32),
    Vocal,
    Half,
}

fn create(kind: EffectKind) -> *mut c_void {
    Box::into_raw(Box::new(Effect { kind, channels: 0 })).cast()
}

unsafe extern "C" fn gain_create(_user: *mut c_void) -> *mut c_void {
    guard(null_mut(), || {
        let gain = setting(GAIN_ID.load(Ordering::Acquire), c"gain")
            .and_then(|s| s.trim().parse::<f32>().ok())
            .filter(|g| g.is_finite())
            .unwrap_or(0.5);
        create(EffectKind::Gain(gain))
    })
}

unsafe extern "C" fn vocal_create(_user: *mut c_void) -> *mut c_void {
    guard(null_mut(), || create(EffectKind::Vocal))
}

unsafe extern "C" fn half_create(_user: *mut c_void) -> *mut c_void {
    guard(null_mut(), || create(EffectKind::Half))
}

unsafe extern "C" fn effect_destroy(inst: *mut c_void) {
    guard((), || {
        if !inst.is_null() {
            // SAFETY: `inst` came from `create` and the host destroys it once.
            drop(unsafe { Box::from_raw(inst.cast::<Effect>()) });
        }
    })
}

unsafe extern "C" fn effect_configure(inst: *mut c_void, _rate: u32, channels: u32) -> i32 {
    guard(NULAMP_ERR_FAILED, || {
        // SAFETY: `inst` came from `create`; one thread at a time (ABI).
        let e = unsafe { &mut *inst.cast::<Effect>() };
        if matches!(e.kind, EffectKind::Vocal) && channels != 2 {
            return NULAMP_ERR_UNSUPPORTED;
        }
        e.channels = channels as usize;
        NULAMP_OK
    })
}

unsafe extern "C" fn effect_process(
    inst: *mut c_void,
    input: *const f32,
    frames: u32,
    out: *mut f32,
    max_out_frames: u32,
) -> i64 {
    guard(NULAMP_ERR_FAILED as i64, || {
        // SAFETY: `inst` came from `create`; one thread at a time (ABI).
        let e = unsafe { &*inst.cast::<Effect>() };
        let ch = e.channels;
        if ch == 0 || input.is_null() || out.is_null() {
            return NULAMP_ERR_INVALID as i64;
        }
        // SAFETY: the host passes `frames` input frames and room for `max_out_frames` output
        // frames of `ch` channels, in buffers that do not overlap.
        let (input, out) = unsafe {
            (
                std::slice::from_raw_parts(input, frames as usize * ch),
                std::slice::from_raw_parts_mut(out, max_out_frames as usize * ch),
            )
        };
        match e.kind {
            EffectKind::Gain(g) => {
                for (o, &i) in out.iter_mut().zip(input) {
                    *o = i * g;
                }
                frames as i64
            }
            EffectKind::Vocal => {
                for (o, i) in out
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip(input.as_chunks::<2>().0)
                {
                    let d = i[0] - i[1];
                    o[0] = d;
                    o[1] = d;
                }
                frames as i64
            }
            EffectKind::Half => {
                if max_out_frames < 2 * frames {
                    return NULAMP_ERR_INVALID as i64;
                }
                for (f, frame) in input.chunks_exact(ch).enumerate() {
                    out[2 * f * ch..(2 * f + 1) * ch].copy_from_slice(frame);
                    out[(2 * f + 1) * ch..(2 * f + 2) * ch].copy_from_slice(frame);
                }
                2 * frames as i64
            }
        }
    })
}

// ---- General: event logger ----

fn event_name(kind: u32) -> &'static str {
    match kind {
        NULAMP_EVENT_TRACK_STARTED => "track started",
        NULAMP_EVENT_TRACK_STOPPED => "track stopped",
        NULAMP_EVENT_TITLE_CHANGED => "title changed",
        NULAMP_EVENT_VOLUME_CHANGED => "volume changed",
        NULAMP_EVENT_EQ_CHANGED => "EQ changed",
        NULAMP_EVENT_PLAYLIST_CHANGED => "playlist changed",
        NULAMP_EVENT_SKIN_CHANGED => "skin changed",
        NULAMP_EVENT_PAUSED => "paused",
        NULAMP_EVENT_UNPAUSED => "unpaused",
        NULAMP_EVENT_INFO_CHANGED => "info changed",
        _ => "unknown event",
    }
}

unsafe extern "C" fn on_ui(_data: *mut c_void) {
    guard((), || {
        log(
            LOGGER_ID.load(Ordering::Acquire),
            NULAMP_LOG_INFO,
            "ui callback",
        );
    })
}

unsafe extern "C" fn logger_event(_user: *mut c_void, event: *const NulampEvent) {
    guard((), || {
        // SAFETY: the host passes a valid event during the call.
        let Some(ev) = (unsafe { event.as_ref() }) else {
            return;
        };
        let id = LOGGER_ID.load(Ordering::Acquire);
        let text = if ev.text.is_null() {
            String::new()
        } else {
            // SAFETY: `text` is a NUL-terminated string valid during the call.
            unsafe { CStr::from_ptr(ev.text) }
                .to_string_lossy()
                .into_owned()
        };
        log(
            id,
            NULAMP_LOG_INFO,
            &format!("{} value={} text={text:?}", event_name(ev.kind), ev.value),
        );
        let Some(h) = host() else { return };
        match ev.kind {
            NULAMP_EVENT_VOLUME_CHANGED => {
                let mut st = NulampStatus::prepared();
                // SAFETY: a host function with a prepared record.
                if let Some(f) = h.get_status
                    && unsafe { f(h, &mut st) } == NULAMP_OK
                {
                    log(id, NULAMP_LOG_INFO, &format!("status volume={}", st.volume));
                }
            }
            NULAMP_EVENT_SKIN_CHANGED => {
                if let Some(f) = h.post_to_ui {
                    // SAFETY: a host function; `on_ui` ignores its data.
                    unsafe { f(h, id, Some(on_ui), null_mut()) };
                }
            }
            NULAMP_EVENT_TRACK_STOPPED if ev.value == 0 => {
                if setting(id, c"restart").as_deref() == Some("1")
                    && let Some(f) = h.send_command
                {
                    // SAFETY: a host function.
                    unsafe { f(h, NULAMP_CMD_PLAY_INDEX, 0) };
                }
            }
            _ => {}
        }
    })
}

// ---- Records ----

const fn dsp_api(create: unsafe extern "C" fn(*mut c_void) -> *mut c_void) -> NulampDspApi {
    NulampDspApi {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampDspApi>() as u32,
        create: Some(create),
        destroy: Some(effect_destroy),
        configure: Some(effect_configure),
        process: Some(effect_process),
        latency_frames: None,
        flush: None,
    }
}

static GAIN_API: NulampDspApi = dsp_api(gain_create);
static VOCAL_API: NulampDspApi = dsp_api(vocal_create);
static HALF_API: NulampDspApi = dsp_api(half_create);
static LOGGER_API: NulampGeneralApi = NulampGeneralApi {
    abi_version: NULAMP_ABI_VERSION,
    struct_size: size_of::<NulampGeneralApi>() as u32,
    on_event: Some(logger_event),
};

const fn module(
    kind: u32,
    api: *const c_void,
    name: &'static CStr,
    description: &'static CStr,
    id: &'static AtomicU64,
) -> NulampModule {
    NulampModule {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampModule>() as u32,
        kind,
        flags: 0,
        api,
        name: name.as_ptr(),
        version: c"1.0".as_ptr(),
        description: description.as_ptr(),
        user: id as *const AtomicU64 as *mut c_void,
        init: Some(module_init),
        quit: Some(module_quit),
        configure: None,
        about: None,
    }
}

static GAIN: NulampModule = NulampModule {
    configure: Some(gain_configure),
    ..module(
        NULAMP_KIND_DSP,
        &GAIN_API as *const NulampDspApi as *const c_void,
        c"Gain",
        c"Multiplies the signal by the `gain` setting",
        &GAIN_ID,
    )
};
static VOCAL: NulampModule = module(
    NULAMP_KIND_DSP,
    &VOCAL_API as *const NulampDspApi as *const c_void,
    c"Vocal remover",
    c"Left minus right on both channels (stereo only)",
    &VOCAL_ID,
);
static HALF: NulampModule = module(
    NULAMP_KIND_DSP,
    &HALF_API as *const NulampDspApi as *const c_void,
    c"Half speed",
    c"Repeats every frame",
    &HALF_ID,
);
static LOGGER: NulampModule = module(
    NULAMP_KIND_GENERAL,
    &LOGGER_API as *const NulampGeneralApi as *const c_void,
    c"Event logger",
    c"Logs player events",
    &LOGGER_ID,
);

static MODULES: StaticPtrs<NulampModule, 4> = StaticPtrs([&GAIN, &VOCAL, &HALF, &LOGGER]);

static DESC: NulampPluginDesc = NulampPluginDesc {
    abi_version: NULAMP_ABI_VERSION,
    struct_size: size_of::<NulampPluginDesc>() as u32,
    name: c"nulamp example plugin".as_ptr(),
    version: c"1.0".as_ptr(),
    description: c"Gain, vocal remover and half-speed effects; event logger".as_ptr(),
    module_count: 4,
    reserved: 0,
    modules: MODULES.as_ptr(),
};

/// The plugin entry point.
///
/// # Safety
///
/// `host` must be null or a host record that stays valid while this file is loaded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nulamp_plugin_entry(host: *const NulampHost) -> *const NulampPluginDesc {
    guard(null(), || {
        // SAFETY: guaranteed by the caller.
        let Some(h) = (unsafe { host.as_ref() }) else {
            return null();
        };
        if abi_major(h.abi_version) != NULAMP_ABI_MAJOR {
            return null();
        }
        HOST.store(host.cast_mut(), Ordering::Release);
        &DESC
    })
}
