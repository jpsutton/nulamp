//! Raw C ABI of nulamp plugins: `#[repr(C)]` mirrors of `include/nulamp_plugin.h`.
//!
//! Plugin authors writing in Rust depend on this crate with `default-features = false` and use
//! only this module. Every function pointer is an `Option`, because C code may store NULL;
//! the ones the header does not mark optional must be `Some`. See `docs/plugins.md` for the
//! rules (versioning, threading, string output).

use std::ffi::{c_char, c_void};

// ---- Versioning ----

pub const NULAMP_ABI_MAJOR: u32 = 1;
pub const NULAMP_ABI_MINOR: u32 = 0;
pub const NULAMP_ABI_VERSION: u32 = (NULAMP_ABI_MAJOR << 16) | NULAMP_ABI_MINOR;

/// The major part of an ABI version.
pub const fn abi_major(v: u32) -> u32 {
    v >> 16
}

/// The minor part of an ABI version.
pub const fn abi_minor(v: u32) -> u32 {
    v & 0xFFFF
}

/// Name of the exported entry point.
pub const NULAMP_PLUGIN_ENTRY_NAME: &str = "nulamp_plugin_entry";

// ---- Result codes ----

pub const NULAMP_OK: i32 = 0;
pub const NULAMP_ERR_FAILED: i32 = -1;
pub const NULAMP_ERR_NOT_FOUND: i32 = -2;
pub const NULAMP_ERR_UNSUPPORTED: i32 = -3;
pub const NULAMP_ERR_INVALID: i32 = -4;
pub const NULAMP_ERR_IO: i32 = -5;
/// Returned by [`NulampInputApi::read`] when a stream has no data yet.
pub const NULAMP_PENDING: i64 = -100;

// ---- Module kinds ----

pub const NULAMP_KIND_INPUT: u32 = 1;
pub const NULAMP_KIND_DSP: u32 = 2;
pub const NULAMP_KIND_VIS: u32 = 3;
pub const NULAMP_KIND_GENERAL: u32 = 4;

// ---- Log levels ----

pub const NULAMP_LOG_ERROR: i32 = 1;
pub const NULAMP_LOG_WARN: i32 = 2;
pub const NULAMP_LOG_INFO: i32 = 3;
pub const NULAMP_LOG_DEBUG: i32 = 4;
pub const NULAMP_LOG_TRACE: i32 = 5;

// ---- Play states ----

pub const NULAMP_STATE_STOPPED: i32 = 0;
pub const NULAMP_STATE_PLAYING: i32 = 1;
pub const NULAMP_STATE_PAUSED: i32 = 3;

// ---- Commands (spec 11 §6.3 numbers below 50000, nulamp commands with an argument above) ----

pub const NULAMP_CMD_TOGGLE_REPEAT: u32 = 40022;
pub const NULAMP_CMD_TOGGLE_SHUFFLE: u32 = 40023;
pub const NULAMP_CMD_PREVIOUS: u32 = 40044;
pub const NULAMP_CMD_PLAY: u32 = 40045;
pub const NULAMP_CMD_PAUSE: u32 = 40046;
pub const NULAMP_CMD_STOP: u32 = 40047;
pub const NULAMP_CMD_NEXT: u32 = 40048;
pub const NULAMP_CMD_VOLUME_UP: u32 = 40058;
pub const NULAMP_CMD_VOLUME_DOWN: u32 = 40059;
pub const NULAMP_CMD_FORWARD_5S: u32 = 40060;
pub const NULAMP_CMD_REWIND_5S: u32 = 40061;
pub const NULAMP_CMD_REWIND_5S_ALT: u32 = 40144;
pub const NULAMP_CMD_STOP_WITH_FADE: u32 = 40147;
pub const NULAMP_CMD_FORWARD_5S_ALT: u32 = 40148;
pub const NULAMP_CMD_START_OF_LIST: u32 = 40154;
pub const NULAMP_CMD_STOP_AFTER_CURRENT: u32 = 40157;
pub const NULAMP_CMD_END_OF_LIST: u32 = 40158;
pub const NULAMP_CMD_FORWARD_10_TRACKS: u32 = 40195;
pub const NULAMP_CMD_BACK_10_TRACKS: u32 = 40197;
pub const NULAMP_CMD_TOGGLE_MANUAL_ADVANCE: u32 = 40395;

pub const NULAMP_CMD_SEEK_TO: u32 = 50001;
pub const NULAMP_CMD_SEEK_RELATIVE: u32 = 50002;
pub const NULAMP_CMD_SET_VOLUME: u32 = 50003;
pub const NULAMP_CMD_VOLUME_BY: u32 = 50004;
pub const NULAMP_CMD_SET_BALANCE: u32 = 50005;
pub const NULAMP_CMD_SET_SHUFFLE: u32 = 50006;
pub const NULAMP_CMD_SET_REPEAT: u32 = 50007;
pub const NULAMP_CMD_SET_MANUAL_ADVANCE: u32 = 50008;
pub const NULAMP_CMD_PLAY_INDEX: u32 = 50009;
pub const NULAMP_CMD_JUMP_ENTRIES: u32 = 50010;

// ---- Events ----

pub const NULAMP_EVENT_TRACK_STARTED: u32 = 1;
pub const NULAMP_EVENT_TRACK_STOPPED: u32 = 2;
pub const NULAMP_EVENT_TITLE_CHANGED: u32 = 3;
pub const NULAMP_EVENT_VOLUME_CHANGED: u32 = 4;
pub const NULAMP_EVENT_EQ_CHANGED: u32 = 5;
pub const NULAMP_EVENT_PLAYLIST_CHANGED: u32 = 6;
pub const NULAMP_EVENT_SKIN_CHANGED: u32 = 7;
pub const NULAMP_EVENT_PAUSED: u32 = 8;
pub const NULAMP_EVENT_UNPAUSED: u32 = 9;
pub const NULAMP_EVENT_INFO_CHANGED: u32 = 10;

// ---- Visualization ----

pub const NULAMP_VIS_SAMPLES: usize = 576;
pub const NULAMP_VIS_FFT_BINS: usize = 256;
pub const NULAMP_VIS_CONTINUE: i32 = 0;
pub const NULAMP_VIS_QUIT: i32 = 1;

// ---- Host API ----

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NulampStatus {
    pub abi_version: u32,
    pub struct_size: u32,
    pub state: i32,
    pub volume: i32,
    pub balance: i32,
    pub kbps: i32,
    pub position_ms: i64,
    pub length_ms: i64,
    pub sample_rate: u32,
    pub channels: u32,
    pub playlist_position: u32,
    pub playlist_length: u32,
    pub seekable: u8,
    pub in_sync: u8,
    pub shuffle: u8,
    pub repeat: u8,
    pub manual_advance: u8,
    pub stop_after_current: u8,
    pub reserved: [u8; 2],
}

impl NulampStatus {
    /// An output record prepared for a `get_status` call.
    pub fn prepared() -> Self {
        Self {
            abi_version: NULAMP_ABI_VERSION,
            struct_size: size_of::<Self>() as u32,
            ..Self::default()
        }
    }
}

pub type NulampUiCallback = unsafe extern "C" fn(data: *mut c_void);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampHost {
    pub abi_version: u32,
    pub struct_size: u32,
    pub ctx: *mut c_void,
    pub send_command:
        Option<unsafe extern "C" fn(host: *const NulampHost, command: u32, arg: i64) -> i32>,
    pub get_status:
        Option<unsafe extern "C" fn(host: *const NulampHost, out: *mut NulampStatus) -> i32>,
    pub get_title:
        Option<unsafe extern "C" fn(host: *const NulampHost, buf: *mut c_char, len: u32) -> i32>,
    pub log: Option<
        unsafe extern "C" fn(
            host: *const NulampHost,
            module_id: u64,
            level: i32,
            msg: *const c_char,
        ),
    >,
    pub settings_get: Option<
        unsafe extern "C" fn(
            host: *const NulampHost,
            module_id: u64,
            key: *const c_char,
            buf: *mut c_char,
            len: u32,
        ) -> i32,
    >,
    pub settings_set: Option<
        unsafe extern "C" fn(
            host: *const NulampHost,
            module_id: u64,
            key: *const c_char,
            value: *const c_char,
        ) -> i32,
    >,
    pub post_to_ui: Option<
        unsafe extern "C" fn(
            host: *const NulampHost,
            module_id: u64,
            cb: Option<NulampUiCallback>,
            data: *mut c_void,
        ) -> i32,
    >,
}

// ---- Common module record ----

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampModule {
    pub abi_version: u32,
    pub struct_size: u32,
    pub kind: u32,
    pub flags: u32,
    pub api: *const c_void,
    pub name: *const c_char,
    pub version: *const c_char,
    pub description: *const c_char,
    pub user: *mut c_void,
    pub init: Option<
        unsafe extern "C" fn(user: *mut c_void, host: *const NulampHost, module_id: u64) -> i32,
    >,
    pub quit: Option<unsafe extern "C" fn(user: *mut c_void)>,
    pub configure: Option<unsafe extern "C" fn(user: *mut c_void)>,
    pub about: Option<unsafe extern "C" fn(user: *mut c_void)>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampPluginDesc {
    pub abi_version: u32,
    pub struct_size: u32,
    pub name: *const c_char,
    pub version: *const c_char,
    pub description: *const c_char,
    pub module_count: u32,
    pub reserved: u32,
    pub modules: *const *const NulampModule,
}

// ---- Input modules ----

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampStreamInfo {
    pub abi_version: u32,
    pub struct_size: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub length_ms: i64,
    pub seekable: u8,
    pub lossless: u8,
    pub reserved: [u8; 6],
    pub codec: [c_char; 64],
}

impl NulampStreamInfo {
    /// An output record prepared for an `info` call.
    pub fn prepared() -> Self {
        Self {
            abi_version: NULAMP_ABI_VERSION,
            struct_size: size_of::<Self>() as u32,
            sample_rate: 0,
            channels: 0,
            bits: 0,
            length_ms: -1,
            seekable: 0,
            lossless: 0,
            reserved: [0; 6],
            codec: [0; 64],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampInputApi {
    pub abi_version: u32,
    pub struct_size: u32,
    pub extensions: *const *const c_char,
    pub claims: Option<unsafe extern "C" fn(user: *mut c_void, location: *const c_char) -> i32>,
    pub open: Option<
        unsafe extern "C" fn(
            user: *mut c_void,
            location: *const c_char,
            error: *mut i32,
        ) -> *mut c_void,
    >,
    pub info: Option<unsafe extern "C" fn(dec: *mut c_void, out: *mut NulampStreamInfo) -> i32>,
    pub read: Option<unsafe extern "C" fn(dec: *mut c_void, out: *mut f32, max_frames: u32) -> i64>,
    pub seek: Option<unsafe extern "C" fn(dec: *mut c_void, ms: u64) -> i64>,
    pub kbps: Option<unsafe extern "C" fn(dec: *mut c_void) -> i32>,
    pub in_sync: Option<unsafe extern "C" fn(dec: *mut c_void) -> i32>,
    pub take_title:
        Option<unsafe extern "C" fn(dec: *mut c_void, buf: *mut c_char, len: u32) -> i32>,
    pub close: Option<unsafe extern "C" fn(dec: *mut c_void)>,
    pub get_field: Option<
        unsafe extern "C" fn(
            user: *mut c_void,
            location: *const c_char,
            name: *const c_char,
            buf: *mut c_char,
            len: u32,
        ) -> i32,
    >,
    pub set_field: Option<
        unsafe extern "C" fn(
            user: *mut c_void,
            location: *const c_char,
            name: *const c_char,
            value: *const c_char,
        ) -> i32,
    >,
    pub write_tags: Option<unsafe extern "C" fn(user: *mut c_void, location: *const c_char) -> i32>,
}

// ---- DSP modules ----

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampDspApi {
    pub abi_version: u32,
    pub struct_size: u32,
    pub create: Option<unsafe extern "C" fn(user: *mut c_void) -> *mut c_void>,
    pub destroy: Option<unsafe extern "C" fn(inst: *mut c_void)>,
    pub configure:
        Option<unsafe extern "C" fn(inst: *mut c_void, sample_rate: u32, channels: u32) -> i32>,
    pub process: Option<
        unsafe extern "C" fn(
            inst: *mut c_void,
            input: *const f32,
            frames: u32,
            out: *mut f32,
            max_out_frames: u32,
        ) -> i64,
    >,
    pub latency_frames: Option<unsafe extern "C" fn(inst: *mut c_void) -> u32>,
    pub flush: Option<unsafe extern "C" fn(inst: *mut c_void)>,
}

// ---- Visualization modules ----

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampVisFrame {
    pub abi_version: u32,
    pub struct_size: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub spectrum: [[u8; NULAMP_VIS_SAMPLES]; 2],
    pub waveform: [[i8; NULAMP_VIS_SAMPLES]; 2],
    pub pcm: [*const f32; 2],
    pub fft: [*const f32; 2],
    pub pcm_frames: u32,
    pub fft_bins: u32,
    pub pixels: *mut u8,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampVisApi {
    pub abi_version: u32,
    pub struct_size: u32,
    pub spectrum_channels: u32,
    pub waveform_channels: u32,
    pub latency_ms: u32,
    pub delay_ms: u32,
    pub start:
        Option<unsafe extern "C" fn(user: *mut c_void, width: u32, height: u32) -> *mut c_void>,
    pub render:
        Option<unsafe extern "C" fn(inst: *mut c_void, frame: *const NulampVisFrame) -> i32>,
    pub stop: Option<unsafe extern "C" fn(inst: *mut c_void)>,
}

// ---- General modules ----

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampEvent {
    pub abi_version: u32,
    pub struct_size: u32,
    pub kind: u32,
    pub reserved: u32,
    pub value: i64,
    pub text: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NulampGeneralApi {
    pub abi_version: u32,
    pub struct_size: u32,
    pub on_event: Option<unsafe extern "C" fn(user: *mut c_void, event: *const NulampEvent)>,
}

// ---- Entry point ----

pub type NulampPluginEntryFn =
    unsafe extern "C" fn(host: *const NulampHost) -> *const NulampPluginDesc;

// SAFETY: these records are immutable descriptions that the ABI requires to stay valid and
// unchanged while the plugin is loaded, and which every thread may read. Their raw pointers are
// only dereferenced through `unsafe` code that follows the ABI's threading rules, so sharing
// the records themselves (for example as `static` items in a plugin) is sound.
unsafe impl Sync for NulampPluginDesc {}
// SAFETY: as above.
unsafe impl Send for NulampPluginDesc {}
// SAFETY: as above.
unsafe impl Sync for NulampModule {}
// SAFETY: as above.
unsafe impl Send for NulampModule {}
// SAFETY: as above.
unsafe impl Sync for NulampInputApi {}
// SAFETY: as above.
unsafe impl Send for NulampInputApi {}
// SAFETY: as above.
unsafe impl Sync for NulampDspApi {}
// SAFETY: as above.
unsafe impl Send for NulampDspApi {}
// SAFETY: as above.
unsafe impl Sync for NulampVisApi {}
// SAFETY: as above.
unsafe impl Send for NulampVisApi {}
// SAFETY: as above.
unsafe impl Sync for NulampGeneralApi {}
// SAFETY: as above.
unsafe impl Send for NulampGeneralApi {}
// SAFETY: every host function is thread-safe by contract and `ctx` is private to the host.
unsafe impl Sync for NulampHost {}
// SAFETY: as above.
unsafe impl Send for NulampHost {}

/// A list of pointers that can be stored in a `static` (for [`NulampPluginDesc::modules`] and
/// [`NulampInputApi::extensions`]).
#[repr(transparent)]
#[derive(Debug)]
pub struct StaticPtrs<T: 'static, const N: usize>(pub [*const T; N]);

impl<T, const N: usize> StaticPtrs<T, N> {
    pub const fn as_ptr(&self) -> *const *const T {
        self.0.as_ptr()
    }
}

// SAFETY: the pointers refer to immutable `static` data (module records or string literals).
unsafe impl<T, const N: usize> Sync for StaticPtrs<T, N> {}

/// Writes `text` into a caller buffer following the ABI's string output convention: at most
/// `len - 1` bytes cut at a UTF-8 character boundary, then a NUL. Returns the full length.
///
/// # Safety
///
/// `buf` must be null or valid for writes of `len` bytes.
pub unsafe fn write_str_out(text: &str, buf: *mut c_char, len: u32) -> i32 {
    let full = text.len().min(i32::MAX as usize);
    if !buf.is_null() && len > 0 {
        let mut n = text.len().min(len as usize - 1);
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        // SAFETY: the caller guarantees `len` writable bytes; `n < len`, so the bytes and the
        // NUL fit, and the source is a valid `str` of at least `n` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), buf.cast::<u8>(), n);
            *buf.add(n) = 0;
        }
    }
    full as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn layouts_match_the_c_header() {
        // Sizes and offsets as a C compiler lays out the header on 64-bit targets.
        assert_eq!(size_of::<NulampStatus>(), 64);
        assert_eq!(offset_of!(NulampStatus, position_ms), 24);
        assert_eq!(offset_of!(NulampStatus, seekable), 56);
        assert_eq!(size_of::<NulampHost>(), 72);
        assert_eq!(size_of::<NulampModule>(), 88);
        assert_eq!(offset_of!(NulampModule, api), 16);
        assert_eq!(offset_of!(NulampModule, init), 56);
        assert_eq!(size_of::<NulampPluginDesc>(), 48);
        assert_eq!(offset_of!(NulampPluginDesc, modules), 40);
        assert_eq!(size_of::<NulampStreamInfo>(), 96);
        assert_eq!(offset_of!(NulampStreamInfo, length_ms), 16);
        assert_eq!(offset_of!(NulampStreamInfo, codec), 32);
        assert_eq!(size_of::<NulampInputApi>(), 112);
        assert_eq!(size_of::<NulampDspApi>(), 56);
        assert_eq!(size_of::<NulampVisFrame>(), 2384);
        assert_eq!(offset_of!(NulampVisFrame, waveform), 1168);
        assert_eq!(offset_of!(NulampVisFrame, pcm), 2320);
        assert_eq!(offset_of!(NulampVisFrame, pixels), 2360);
        assert_eq!(size_of::<NulampVisApi>(), 48);
        assert_eq!(size_of::<NulampEvent>(), 32);
        assert_eq!(size_of::<NulampGeneralApi>(), 16);
    }

    #[test]
    fn versions() {
        assert_eq!(abi_major(NULAMP_ABI_VERSION), 1);
        assert_eq!(abi_minor(NULAMP_ABI_VERSION), 0);
    }

    #[test]
    fn string_output_cuts_at_char_boundary() {
        let mut buf = [0x55 as c_char; 5];
        // "aé€": 1 + 2 + 3 bytes; 4 bytes of room cut before the euro sign.
        // SAFETY: `buf` has 5 writable bytes.
        let n = unsafe { write_str_out("aé€", buf.as_mut_ptr(), 5) };
        assert_eq!(n, 6);
        assert_eq!(
            buf[..4].iter().map(|&c| c as u8).collect::<Vec<_>>(),
            b"a\xc3\xa9\0"
        );
        // SAFETY: a null buffer is allowed and only measures.
        assert_eq!(unsafe { write_str_out("abc", std::ptr::null_mut(), 0) }, 3);
    }
}
