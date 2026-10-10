//! Input modules as [`DecoderFactory`] / [`Decoder`] (08 §3, 07 §7).

use std::ffi::{CString, c_char, c_void};

use nulamp_decoders::{DecodeError, Decoder, DecoderFactory, ReadStatus, StreamInfo};

use crate::module::{Module, plugin_str};
use crate::sys::*;

/// Frames asked for per `read` call.
const READ_FRAMES: u32 = 1152;
/// Longest string the host reads back from a plugin.
const MAX_STRING: usize = 1 << 20;

/// A metadata field as an input module reports it (08 §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Found(String),
    /// The module understands the field but it has no value.
    Empty,
    /// The module does not understand the field (disabled in the file-info dialog).
    Unsupported,
}

/// An input (decoder) module.
#[derive(Debug, Clone)]
pub struct InputModule {
    module: Module,
    api: NulampInputApi,
    extensions: Vec<(String, String)>,
}

impl InputModule {
    pub(crate) fn new(module: Module) -> Self {
        // SAFETY: the loader checked that `api` is an input record of a sufficient size.
        let api = unsafe { *module.api().cast::<NulampInputApi>() };
        // SAFETY: the ABI defines `extensions` as NULL or a NULL-terminated list of strings.
        let extensions = unsafe { read_extensions(api.extensions) };
        Self {
            module,
            api,
            extensions,
        }
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    /// (extension list, description) pairs.
    pub fn extensions(&self) -> &[(String, String)] {
        &self.extensions
    }

    /// A decoder factory for the [`nulamp_decoders::Registry`].
    pub fn factory(&self) -> PluginDecoderFactory {
        PluginDecoderFactory {
            input: self.clone(),
        }
    }

    /// Reads a metadata field. Any thread.
    pub fn get_field(&self, location: &str, name: &str) -> FieldValue {
        let (Some(f), Ok(loc), Ok(name)) = (
            self.api.get_field,
            CString::new(location),
            CString::new(name),
        ) else {
            return FieldValue::Unsupported;
        };
        let user = self.module.user();
        let mut found = true;
        let mut empty = false;
        let text = read_string(|buf, len| {
            // SAFETY: a valid module function; `buf` is writable for `len` bytes and the
            // strings live across the call. `get_field` may run on any thread (ABI).
            let r = unsafe { f(user, loc.as_ptr(), name.as_ptr(), buf, len) };
            if r < 0 {
                found = false;
            } else if r == 0 {
                empty = true;
            }
            r
        });
        match (found, empty, text) {
            (false, _, _) | (true, false, None) => FieldValue::Unsupported,
            (true, true, _) => FieldValue::Empty,
            (true, false, Some(t)) => FieldValue::Found(t),
        }
    }

    /// Queues a field change (an empty value deletes it). Any thread. False when unsupported
    /// or failed.
    pub fn set_field(&self, location: &str, name: &str, value: &str) -> bool {
        let (Some(f), Ok(loc), Ok(name), Ok(value)) = (
            self.api.set_field,
            CString::new(location),
            CString::new(name),
            CString::new(value),
        ) else {
            return false;
        };
        // SAFETY: a valid module function with live strings; any thread (ABI).
        unsafe {
            f(
                self.module.user(),
                loc.as_ptr(),
                name.as_ptr(),
                value.as_ptr(),
            ) == NULAMP_OK
        }
    }

    /// Commits queued changes. Any thread.
    pub fn write_tags(&self, location: &str) -> bool {
        let (Some(f), Ok(loc)) = (self.api.write_tags, CString::new(location)) else {
            return false;
        };
        // SAFETY: as in `set_field`.
        unsafe { f(self.module.user(), loc.as_ptr()) == NULAMP_OK }
    }
}

/// # Safety
///
/// `p` must be null or a NULL-terminated array of NUL-terminated strings.
unsafe fn read_extensions(p: *const *const c_char) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if p.is_null() {
        return out;
    }
    let mut i = 0;
    loop {
        // SAFETY: the list is NULL-terminated; reading stops at the first NULL.
        let ext = unsafe { *p.add(i) };
        if ext.is_null() {
            break;
        }
        // SAFETY: as above.
        let desc = unsafe { *p.add(i + 1) };
        // SAFETY: the entries are NUL-terminated strings.
        let (e, d) = unsafe { (plugin_str(ext), plugin_str(desc)) };
        out.push((e, d));
        if desc.is_null() {
            break;
        }
        i += 2;
    }
    out
}

/// Calls a string-output function, growing the buffer once when the text did not fit.
/// Returns `None` when the function returns a negative code or 0.
pub(crate) fn read_string(mut f: impl FnMut(*mut c_char, u32) -> i32) -> Option<String> {
    let mut buf = vec![0u8; 256];
    loop {
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        if n <= 0 {
            return None;
        }
        let n = n as usize;
        if n < buf.len() {
            buf.truncate(n);
            return Some(String::from_utf8_lossy(&buf).into_owned());
        }
        if n >= MAX_STRING || buf.len() > 256 {
            // Too long, or the plugin keeps asking for more: keep what was written.
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            buf.truncate(end);
            return Some(String::from_utf8_lossy(&buf).into_owned());
        }
        buf = vec![0u8; n + 1];
    }
}

/// An input module as a decoder factory.
#[derive(Debug, Clone)]
pub struct PluginDecoderFactory {
    input: InputModule,
}

impl DecoderFactory for PluginDecoderFactory {
    fn name(&self) -> &str {
        &self.input.module.info().name
    }

    fn extensions(&self) -> Vec<(String, String)> {
        self.input.extensions.clone()
    }

    fn claims(&self, location: &str) -> bool {
        let (Some(f), Ok(loc)) = (self.input.api.claims, CString::new(location)) else {
            return false;
        };
        // SAFETY: a valid module function; `claims` may run on any thread (ABI).
        unsafe { f(self.input.module.user(), loc.as_ptr()) == 1 }
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        let loc = CString::new(location).map_err(|_| DecodeError::NotFound(location.into()))?;
        let open = self.input.api.open.expect("checked at load");
        let mut error = NULAMP_ERR_FAILED;
        // SAFETY: a valid module function; `error` is a writable i32; any thread (ABI).
        let handle = unsafe { open(self.input.module.user(), loc.as_ptr(), &mut error) };
        if handle.is_null() {
            return Err(match error {
                NULAMP_ERR_NOT_FOUND => DecodeError::NotFound(location.into()),
                NULAMP_ERR_UNSUPPORTED => DecodeError::Unsupported(location.into()),
                code => DecodeError::Open(format!("{location}: plugin error {code}")),
            });
        }
        let mut dec = PluginDecoder {
            input: self.input.clone(),
            handle,
            info: StreamInfo::default(),
            codec: [0; 64],
            buf: Vec::new(),
        };
        dec.refresh_info()
            .map_err(|e| DecodeError::Open(format!("{location}: {e}")))?;
        Ok(Box::new(dec))
    }
}

/// An opened plugin decoder.
pub struct PluginDecoder {
    input: InputModule,
    handle: *mut c_void,
    info: StreamInfo,
    codec: [c_char; 64],
    buf: Vec<f32>,
}

// SAFETY: the ABI lets a decoder handle move between threads as long as one thread uses it at
// a time, which `&mut self` (and the absence of `Sync`) guarantees.
unsafe impl Send for PluginDecoder {}

impl PluginDecoder {
    fn refresh_info(&mut self) -> Result<(), String> {
        let f = self.input.api.info.expect("checked at load");
        let mut raw = NulampStreamInfo::prepared();
        // SAFETY: a valid decoder handle and a prepared, writable record.
        let r = unsafe { f(self.handle, &mut raw) };
        if r != NULAMP_OK {
            return Err(format!("info failed ({r})"));
        }
        if raw.sample_rate == 0 || !(1..=8).contains(&raw.channels) {
            return Err(format!(
                "unusable format: {} Hz, {} channels",
                raw.sample_rate, raw.channels
            ));
        }
        if raw.codec != self.codec || self.info.sample_rate == 0 {
            raw.codec[63] = 0;
            // SAFETY: the last byte was just set to NUL.
            self.info.codec = unsafe { plugin_str(raw.codec.as_ptr()) };
            self.codec = raw.codec;
        }
        self.info.sample_rate = raw.sample_rate;
        self.info.channels = raw.channels;
        self.info.bits = raw.bits;
        self.info.length_ms = u64::try_from(raw.length_ms).ok();
        self.info.seekable = raw.seekable != 0;
        self.info.lossless = raw.lossless != 0;
        Ok(())
    }
}

impl Drop for PluginDecoder {
    fn drop(&mut self) {
        let close = self.input.api.close.expect("checked at load");
        // SAFETY: the handle came from `open` and is closed exactly once.
        unsafe { close(self.handle) };
    }
}

impl Decoder for PluginDecoder {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError> {
        self.refresh_info().map_err(DecodeError::Decode)?;
        let ch = self.info.channels as usize;
        self.buf.resize(READ_FRAMES as usize * ch, 0.0);
        let read = self.input.api.read.expect("checked at load");
        // SAFETY: a valid decoder handle; `buf` holds `READ_FRAMES` frames of the layout
        // `info` just reported.
        let n = unsafe { read(self.handle, self.buf.as_mut_ptr(), READ_FRAMES) };
        if n == NULAMP_PENDING {
            return Ok(ReadStatus::Pending);
        }
        if n < 0 {
            return Err(DecodeError::Decode(format!("plugin error {n}")));
        }
        let n = (n as usize).min(READ_FRAMES as usize);
        out.extend_from_slice(&self.buf[..n * ch]);
        Ok(ReadStatus::from_frames(n))
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        let seek = self.input.api.seek.expect("checked at load");
        // SAFETY: a valid decoder handle.
        let r = unsafe { seek(self.handle, ms) };
        if r < 0 {
            return Err(DecodeError::Seek(format!("plugin error {r}")));
        }
        self.refresh_info().map_err(DecodeError::Seek)?;
        Ok(r as u64)
    }

    fn kbps(&self) -> Option<u32> {
        let f = self.input.api.kbps?;
        // SAFETY: a valid decoder handle; `&self` is only reachable from the owning thread.
        let k = unsafe { f(self.handle) };
        u32::try_from(k).ok()
    }

    fn in_sync(&self) -> bool {
        match self.input.api.in_sync {
            // SAFETY: as in `kbps`.
            Some(f) => unsafe { f(self.handle) != 0 },
            None => true,
        }
    }

    fn take_title(&mut self) -> Option<String> {
        let f = self.input.api.take_title?;
        let handle = self.handle;
        // SAFETY: a valid decoder handle; `buf` is writable for `len` bytes. A title that did
        // not fit is not consumed, so the second call gets it whole.
        read_string(|buf, len| unsafe { f(handle, buf, len) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

    use crate::host::Host;
    use crate::host::tests::Recorder;
    use crate::loader::Plugins;
    use crate::sys::StaticPtrs;

    /// A test decoder: a ramp of `total` frames in stereo, pending once every other read.
    struct Ramp {
        pos: u32,
        total: u32,
        pending_next: bool,
        title: Option<&'static str>,
    }

    static CLOSED: AtomicUsize = AtomicUsize::new(0);
    static OPEN_ERROR: AtomicI32 = AtomicI32::new(0);

    unsafe extern "C" fn init(_: *mut c_void, _: *const NulampHost, _: u64) -> i32 {
        NULAMP_OK
    }
    unsafe extern "C" fn quit(_: *mut c_void) {}
    unsafe extern "C" fn claims(_: *mut c_void, loc: *const c_char) -> i32 {
        // SAFETY: the host passes a NUL-terminated string.
        let loc = unsafe { std::ffi::CStr::from_ptr(loc) }.to_bytes();
        loc.starts_with(b"ramp://") as i32
    }
    unsafe extern "C" fn open(_: *mut c_void, loc: *const c_char, error: *mut i32) -> *mut c_void {
        // SAFETY: as in `claims`.
        let loc = unsafe { std::ffi::CStr::from_ptr(loc) }.to_bytes();
        if loc == b"ramp://missing" {
            // SAFETY: the host passes a writable error.
            unsafe { *error = OPEN_ERROR.load(Ordering::SeqCst) };
            return std::ptr::null_mut();
        }
        Box::into_raw(Box::new(Ramp {
            pos: 0,
            total: 1500,
            pending_next: true,
            title: Some("A rather long stream title that does not fit in a tiny buffer"),
        }))
        .cast()
    }
    unsafe extern "C" fn info(dec: *mut c_void, out: *mut NulampStreamInfo) -> i32 {
        // SAFETY: `dec` is a `Ramp` from `open`; `out` is a prepared record.
        unsafe {
            let _ = &*dec.cast::<Ramp>();
            let o = &mut *out;
            o.sample_rate = 48_000;
            o.channels = 2;
            o.bits = 24;
            o.length_ms = 31;
            o.seekable = 1;
            for (d, s) in o.codec.iter_mut().zip(b"Ramp\0") {
                *d = *s as c_char;
            }
        }
        NULAMP_OK
    }
    unsafe extern "C" fn read(dec: *mut c_void, out: *mut f32, max: u32) -> i64 {
        // SAFETY: as in `info`; `out` holds `max` stereo frames.
        let r = unsafe { &mut *dec.cast::<Ramp>() };
        if r.pending_next {
            r.pending_next = false;
            return NULAMP_PENDING;
        }
        r.pending_next = true;
        let n = max.min(r.total - r.pos);
        for i in 0..n {
            let v = (r.pos + i) as f32;
            // SAFETY: `i < max`.
            unsafe {
                *out.add(2 * i as usize) = v;
                *out.add(2 * i as usize + 1) = -v;
            }
        }
        r.pos += n;
        n as i64
    }
    unsafe extern "C" fn seek(dec: *mut c_void, ms: u64) -> i64 {
        // SAFETY: as in `info`.
        let r = unsafe { &mut *dec.cast::<Ramp>() };
        r.pos = (ms * 48).min(r.total as u64) as u32;
        ms as i64
    }
    unsafe extern "C" fn kbps(_: *mut c_void) -> i32 {
        -1
    }
    unsafe extern "C" fn take_title(dec: *mut c_void, buf: *mut c_char, len: u32) -> i32 {
        // SAFETY: as in `info`.
        let r = unsafe { &mut *dec.cast::<Ramp>() };
        let Some(t) = r.title else { return 0 };
        // SAFETY: the host passes a buffer of `len` bytes.
        let n = unsafe { write_str_out(t, buf, len) };
        if (n as u32) < len {
            r.title = None;
        }
        n
    }
    unsafe extern "C" fn close(dec: *mut c_void) {
        // SAFETY: `dec` is a `Ramp` from `open`, closed once.
        drop(unsafe { Box::from_raw(dec.cast::<Ramp>()) });
        CLOSED.fetch_add(1, Ordering::SeqCst);
    }
    unsafe extern "C" fn get_field(
        _: *mut c_void,
        _: *const c_char,
        name: *const c_char,
        buf: *mut c_char,
        len: u32,
    ) -> i32 {
        // SAFETY: NUL-terminated from the host.
        match unsafe { std::ffi::CStr::from_ptr(name) }.to_bytes() {
            // SAFETY: the host passes a buffer of `len` bytes.
            b"title" => unsafe { write_str_out("Ramp title", buf, len) },
            b"album" => 0,
            _ => NULAMP_ERR_UNSUPPORTED,
        }
    }

    static EXTS: StaticPtrs<c_char, 5> = StaticPtrs([
        c"ramp;rmp".as_ptr(),
        c"Ramp files".as_ptr(),
        c"ramp2".as_ptr(),
        c"More ramps".as_ptr(),
        std::ptr::null(),
    ]);
    static API: NulampInputApi = NulampInputApi {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampInputApi>() as u32,
        extensions: EXTS.as_ptr(),
        claims: Some(claims),
        open: Some(open),
        info: Some(info),
        read: Some(read),
        seek: Some(seek),
        kbps: Some(kbps),
        in_sync: None,
        take_title: Some(take_title),
        close: Some(close),
        get_field: Some(get_field),
        set_field: None,
        write_tags: None,
    };
    static MODULE: NulampModule = NulampModule {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampModule>() as u32,
        kind: NULAMP_KIND_INPUT,
        flags: 0,
        api: &API as *const NulampInputApi as *const c_void,
        name: c"Ramp".as_ptr(),
        version: c"1".as_ptr(),
        description: c"Test ramps".as_ptr(),
        user: std::ptr::null_mut(),
        init: Some(init),
        quit: Some(quit),
        configure: None,
        about: None,
    };
    static MODULES: StaticPtrs<NulampModule, 1> = StaticPtrs([&MODULE]);
    static DESC: NulampPluginDesc = NulampPluginDesc {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampPluginDesc>() as u32,
        name: c"Ramp plugin".as_ptr(),
        version: c"1.0".as_ptr(),
        description: c"".as_ptr(),
        module_count: 1,
        reserved: 0,
        modules: MODULES.as_ptr(),
    };
    unsafe extern "C" fn entry(_: *const NulampHost) -> *const NulampPluginDesc {
        &DESC
    }

    #[test]
    fn decoder_adapter() {
        let host = Host::new(Arc::new(Recorder::default()));
        let mut p = Plugins::default();
        // SAFETY: `entry` follows the ABI.
        assert_eq!(unsafe { p.load_builtin(entry, "ramp", &host) }.unwrap(), 1);
        let input = &p.inputs[0];
        assert_eq!(
            input.extensions(),
            [
                ("ramp;rmp".to_string(), "Ramp files".to_string()),
                ("ramp2".into(), "More ramps".into())
            ]
        );
        assert_eq!(
            input.get_field("x.ramp", "title"),
            FieldValue::Found("Ramp title".into())
        );
        assert_eq!(input.get_field("x.ramp", "album"), FieldValue::Empty);
        assert_eq!(input.get_field("x.ramp", "bpm"), FieldValue::Unsupported);
        assert!(!input.set_field("x.ramp", "title", "x"));

        let mut reg = nulamp_decoders::Registry::empty();
        p.register_decoders(&mut reg);
        assert!(reg.select("ramp://live").is_some());
        assert!(reg.is_supported_extension("RMP"));

        OPEN_ERROR.store(NULAMP_ERR_NOT_FOUND, Ordering::SeqCst);
        assert!(matches!(
            reg.open("ramp://missing"),
            Err(DecodeError::NotFound(_))
        ));

        let closed = CLOSED.load(Ordering::SeqCst);
        let mut dec = reg.open("ramp://live").unwrap();
        let info = dec.info().clone();
        assert_eq!(
            (info.sample_rate, info.channels, info.bits),
            (48_000, 2, 24)
        );
        assert_eq!(
            (info.length_ms, info.seekable, info.codec.as_str()),
            (Some(31), true, "Ramp")
        );
        assert_eq!(dec.kbps(), None);
        assert!(dec.in_sync());

        let mut out = Vec::new();
        // The test plugin reports "pending" before every block.
        assert_eq!(dec.read(&mut out).unwrap(), ReadStatus::Pending);
        assert_eq!(dec.read(&mut out).unwrap(), ReadStatus::Frames(1152));
        assert_eq!(dec.read(&mut out).unwrap(), ReadStatus::Pending);
        assert_eq!(dec.read(&mut out).unwrap(), ReadStatus::Frames(348));
        assert_eq!(dec.read(&mut out).unwrap(), ReadStatus::Pending);
        assert_eq!(dec.read(&mut out).unwrap(), ReadStatus::End);
        assert_eq!(out.len(), 3000);
        assert_eq!((out[2 * 1499], out[2 * 1499 + 1]), (1499.0, -1499.0));

        assert_eq!(dec.seek(10).unwrap(), 10);
        out.clear();
        while dec.read(&mut out).unwrap() == ReadStatus::Pending {}
        assert_eq!(out[0], 480.0);

        assert_eq!(
            dec.take_title().as_deref(),
            Some("A rather long stream title that does not fit in a tiny buffer")
        );
        assert_eq!(dec.take_title(), None);
        drop(dec);
        assert_eq!(CLOSED.load(Ordering::SeqCst), closed + 1);
    }

    #[test]
    fn read_string_grows_once() {
        let text = "x".repeat(300);
        let mut calls = 0;
        let s = read_string(|buf, len| {
            calls += 1;
            // SAFETY: `read_string` passes a buffer of `len` bytes.
            unsafe { write_str_out(&text, buf, len) }
        });
        assert_eq!(s.as_deref(), Some(text.as_str()));
        assert_eq!(calls, 2);
        assert_eq!(read_string(|_, _| NULAMP_ERR_FAILED), None);
    }
}
