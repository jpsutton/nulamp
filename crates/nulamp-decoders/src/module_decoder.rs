//! Tracker module decoder (09 §7) using the system libopenmpt through a small FFI layer. Every
//! format the library reports is claimed. Repeats, the end behaviour and the fade-out are
//! handled here so the position and length stay known.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::OnceLock;

use crate::symphonia_decoder::local_path;
use crate::{DecodeError, Decoder, DecoderFactory, StreamInfo};

mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct OpenmptModule {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct InitialCtl {
        pub ctl: *const c_char,
        pub value: *const c_char,
    }

    pub type LogFunc = Option<unsafe extern "C" fn(message: *const c_char, user: *mut c_void)>;
    pub type ErrorFunc = Option<unsafe extern "C" fn(error: c_int, user: *mut c_void) -> c_int>;

    pub const RENDER_STEREOSEPARATION_PERCENT: c_int = 2;
    pub const RENDER_INTERPOLATIONFILTER_LENGTH: c_int = 3;
    pub const RENDER_VOLUMERAMPING_STRENGTH: c_int = 4;

    unsafe extern "C" {
        pub fn openmpt_get_supported_extensions() -> *const c_char;
        pub fn openmpt_free_string(s: *const c_char);
        pub fn openmpt_module_create_from_memory2(
            filedata: *const c_void,
            filesize: usize,
            logfunc: LogFunc,
            loguser: *mut c_void,
            errfunc: ErrorFunc,
            erruser: *mut c_void,
            error: *mut c_int,
            error_message: *mut *const c_char,
            ctls: *const InitialCtl,
        ) -> *mut OpenmptModule;
        pub fn openmpt_module_destroy(m: *mut OpenmptModule);
        pub fn openmpt_module_read_float_mono(
            m: *mut OpenmptModule,
            samplerate: i32,
            count: usize,
            mono: *mut f32,
        ) -> usize;
        pub fn openmpt_module_read_interleaved_float_stereo(
            m: *mut OpenmptModule,
            samplerate: i32,
            count: usize,
            interleaved: *mut f32,
        ) -> usize;
        pub fn openmpt_module_read_interleaved_float_quad(
            m: *mut OpenmptModule,
            samplerate: i32,
            count: usize,
            interleaved: *mut f32,
        ) -> usize;
        pub fn openmpt_module_set_position_seconds(m: *mut OpenmptModule, seconds: f64) -> f64;
        pub fn openmpt_module_get_duration_seconds(m: *mut OpenmptModule) -> f64;
        pub fn openmpt_module_get_metadata(
            m: *mut OpenmptModule,
            key: *const c_char,
        ) -> *const c_char;
        pub fn openmpt_module_ctl_set_boolean(
            m: *mut OpenmptModule,
            ctl: *const c_char,
            value: c_int,
        ) -> c_int;
        pub fn openmpt_module_ctl_set_text(
            m: *mut OpenmptModule,
            ctl: *const c_char,
            value: *const c_char,
        ) -> c_int;
        pub fn openmpt_module_set_render_param(
            m: *mut OpenmptModule,
            param: c_int,
            value: i32,
        ) -> c_int;
        pub fn openmpt_module_set_repeat_count(m: *mut OpenmptModule, count: i32) -> c_int;
    }
}

/// Takes ownership of a string allocated by libopenmpt.
fn take_string(s: *const c_char) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: libopenmpt returns NUL-terminated strings that the caller must free once.
    let text = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    // SAFETY: `s` came from libopenmpt and is not used after this call.
    unsafe { ffi::openmpt_free_string(s) };
    Some(text)
}

unsafe extern "C" fn log_message(message: *const c_char, _user: *mut c_void) {
    if !message.is_null() {
        // SAFETY: libopenmpt passes a NUL-terminated message valid for this call.
        log::debug!(
            "libopenmpt: {}",
            unsafe { CStr::from_ptr(message) }.to_string_lossy()
        );
    }
}

/// Extensions libopenmpt can load, lower-case.
pub fn supported_extensions() -> &'static [String] {
    static EXTS: OnceLock<Vec<String>> = OnceLock::new();
    EXTS.get_or_init(|| {
        // SAFETY: no arguments; the result is freed by `take_string`.
        take_string(unsafe { ffi::openmpt_get_supported_extensions() })
            .unwrap_or_default()
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_lowercase())
            .collect()
    })
}

/// Interpolation filter (09 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interpolation {
    None,
    Linear,
    Cubic,
    /// Windowed sinc with 8 taps.
    #[default]
    WindowedSinc,
}

impl Interpolation {
    fn filter_length(self) -> i32 {
        match self {
            Self::None => 1,
            Self::Linear => 2,
            Self::Cubic => 4,
            Self::WindowedSinc => 8,
        }
    }
}

/// What happens after the last repeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EndBehaviour {
    #[default]
    Stop,
    /// Keep playing from the loop point while fading out over [`ModuleOptions::fade_ms`].
    FadeOut,
    /// Keep playing from the loop point until stopped.
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModuleChannels {
    Mono,
    #[default]
    Stereo,
    /// L, R, rear L, rear R.
    Quad,
}

impl ModuleChannels {
    fn count(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Quad => 4,
        }
    }
}

/// Module options (09 §7).
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleOptions {
    pub sample_rate: u32,
    /// Stereo separation in percent, 0..=200.
    pub stereo_separation: u32,
    pub interpolation: Interpolation,
    /// Volume ramping strength 0..=10; -1 is the library default.
    pub volume_ramping: i32,
    /// Extra passes through the song: 0 plays once, -1 repeats forever.
    pub repeat_count: i32,
    /// Amiga (Paula) resampler emulation for Amiga formats such as MOD.
    pub amiga_resampler: bool,
    pub end: EndBehaviour,
    pub fade_ms: u32,
    pub channels: ModuleChannels,
}

impl Default for ModuleOptions {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            stereo_separation: 100,
            interpolation: Interpolation::WindowedSinc,
            volume_ramping: -1,
            repeat_count: 0,
            amiga_resampler: false,
            end: EndBehaviour::Stop,
            fade_ms: 1000,
            channels: ModuleChannels::Stereo,
        }
    }
}

#[derive(Debug, Default)]
pub struct ModuleFactory {
    pub options: ModuleOptions,
}

impl DecoderFactory for ModuleFactory {
    fn name(&self) -> &str {
        "libopenmpt"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        vec![(supported_extensions().join(";"), "Tracker Modules".into())]
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        Ok(Box::new(ModuleDecoder::open(location, &self.options)?))
    }
}

/// Module metadata from libopenmpt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModuleMetadata {
    pub title: String,
    pub artist: String,
    pub tracker: String,
    pub message: String,
    /// Format name, for example "ProTracker MOD".
    pub format: String,
}

/// Owned libopenmpt module.
struct Module(NonNull<ffi::OpenmptModule>);

// SAFETY: a module is only used through `&mut self` here; libopenmpt has no thread affinity.
unsafe impl Send for Module {}

impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: created by openmpt_module_create_from_memory2 and destroyed once.
        unsafe { ffi::openmpt_module_destroy(self.0.as_ptr()) };
    }
}

impl Module {
    fn load(data: &[u8]) -> Result<Self, String> {
        let mut error = 0;
        let mut message: *const c_char = std::ptr::null();
        // SAFETY: `data` is valid for the call (libopenmpt copies what it needs); the out
        // pointers are valid; a null ctl list is allowed.
        let m = unsafe {
            ffi::openmpt_module_create_from_memory2(
                data.as_ptr().cast(),
                data.len(),
                Some(log_message),
                std::ptr::null_mut(),
                None,
                std::ptr::null_mut(),
                &mut error,
                &mut message,
                std::ptr::null(),
            )
        };
        let message = take_string(message);
        NonNull::new(m)
            .map(Self)
            .ok_or_else(|| message.unwrap_or_else(|| format!("libopenmpt error {error}")))
    }

    fn ptr(&self) -> *mut ffi::OpenmptModule {
        self.0.as_ptr()
    }

    fn metadata(&self, key: &str) -> String {
        let key = CString::new(key).unwrap_or_default();
        // SAFETY: valid module and key; the result is freed by `take_string`.
        take_string(unsafe { ffi::openmpt_module_get_metadata(self.ptr(), key.as_ptr()) })
            .unwrap_or_default()
    }

    fn ctl_text(&self, ctl: &str, value: &str) -> bool {
        let (ctl, value) = (CString::new(ctl), CString::new(value));
        let (Ok(ctl), Ok(value)) = (ctl, value) else {
            return false;
        };
        // SAFETY: valid module and NUL-terminated strings.
        unsafe { ffi::openmpt_module_ctl_set_text(self.ptr(), ctl.as_ptr(), value.as_ptr()) == 1 }
    }

    fn ctl_bool(&self, ctl: &str, value: bool) -> bool {
        let Ok(ctl) = CString::new(ctl) else {
            return false;
        };
        // SAFETY: valid module and NUL-terminated string.
        unsafe {
            ffi::openmpt_module_ctl_set_boolean(self.ptr(), ctl.as_ptr(), value as c_int) == 1
        }
    }

    fn render_param(&self, param: c_int, value: i32) -> bool {
        // SAFETY: valid module.
        unsafe { ffi::openmpt_module_set_render_param(self.ptr(), param, value) == 1 }
    }

    /// Renders up to `buf.len() / channels` frames; 0 at the end of the song.
    fn read(&mut self, rate: u32, channels: ModuleChannels, buf: &mut [f32]) -> usize {
        let count = buf.len() / channels.count();
        let (m, out) = (self.ptr(), buf.as_mut_ptr());
        // SAFETY: `buf` holds `count * channels` samples.
        unsafe {
            match channels {
                ModuleChannels::Mono => {
                    ffi::openmpt_module_read_float_mono(m, rate as i32, count, out)
                }
                ModuleChannels::Stereo => {
                    ffi::openmpt_module_read_interleaved_float_stereo(m, rate as i32, count, out)
                }
                ModuleChannels::Quad => {
                    ffi::openmpt_module_read_interleaved_float_quad(m, rate as i32, count, out)
                }
            }
        }
    }

    fn set_position(&mut self, secs: f64) -> f64 {
        // SAFETY: valid module.
        unsafe { ffi::openmpt_module_set_position_seconds(self.ptr(), secs) }
    }

    fn duration(&self) -> f64 {
        // SAFETY: valid module.
        unsafe { ffi::openmpt_module_get_duration_seconds(self.ptr()) }
    }
}

const BLOCK: usize = 1024;

/// Decoder for one tracker module.
pub struct ModuleDecoder {
    module: Module,
    options: ModuleOptions,
    metadata: ModuleMetadata,
    info: StreamInfo,
    /// Length of one pass in frames (0 when unknown).
    pass: u64,
    /// Passes still to play after the current one; `None` = forever.
    passes_left: Option<u32>,
    /// Frames of fade-out already played, once fading.
    fading: Option<u64>,
    ended: bool,
    pos: u64,
    /// Frames to render and drop after a seek that landed early.
    skip: u64,
    buf: Vec<f32>,
}

impl ModuleDecoder {
    pub fn open(location: &str, options: &ModuleOptions) -> Result<Self, DecodeError> {
        let path = local_path(location).ok_or_else(|| DecodeError::Unsupported(location.into()))?;
        let data = std::fs::read(Path::new(&path)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DecodeError::NotFound(location.into()),
            _ => DecodeError::Open(e.to_string()),
        })?;
        Self::from_memory(&data, options)
    }

    pub fn from_memory(data: &[u8], options: &ModuleOptions) -> Result<Self, DecodeError> {
        let module = Module::load(data).map_err(DecodeError::Open)?;
        let o = options.clone();
        // SAFETY: valid module.
        unsafe { ffi::openmpt_module_set_repeat_count(module.ptr(), 0) };
        let ok = module.ctl_text("play.at_end", "continue")
            & module.render_param(
                ffi::RENDER_STEREOSEPARATION_PERCENT,
                o.stereo_separation.min(200) as i32,
            )
            & module.render_param(
                ffi::RENDER_INTERPOLATIONFILTER_LENGTH,
                o.interpolation.filter_length(),
            )
            & module.render_param(
                ffi::RENDER_VOLUMERAMPING_STRENGTH,
                o.volume_ramping.clamp(-1, 10),
            );
        if !ok {
            log::warn!("libopenmpt rejected a render setting");
        }
        if !module.ctl_bool("render.resampler.emulate_amiga", o.amiga_resampler) {
            log::debug!("libopenmpt has no Amiga resampler setting");
        }
        let metadata = ModuleMetadata {
            title: module.metadata("title"),
            artist: module.metadata("artist"),
            tracker: module.metadata("tracker"),
            message: module.metadata("message_raw"),
            format: module.metadata("type_long"),
        };
        let duration = module.duration();
        let rate = o.sample_rate.clamp(8_000, 192_000);
        let pass = if duration.is_finite() && duration > 0.0 {
            (duration * rate as f64).round() as u64
        } else {
            0
        };
        let passes_left = u32::try_from(o.repeat_count).ok();
        let fade = o.fade_ms as u64 * rate as u64 / 1000;
        let length = match (passes_left, o.end) {
            (Some(r), EndBehaviour::Stop) if pass > 0 => Some(pass * (r as u64 + 1)),
            (Some(r), EndBehaviour::FadeOut) if pass > 0 => Some(pass * (r as u64 + 1) + fade),
            _ => None,
        };
        let info = StreamInfo {
            sample_rate: rate,
            channels: o.channels.count() as u16,
            bits: 0,
            length_ms: length.map(|f| f * 1000 / rate as u64),
            seekable: pass > 0,
            codec: if metadata.format.is_empty() {
                "Tracker module".into()
            } else {
                metadata.format.clone()
            },
            lossless: false,
        };
        Ok(Self {
            module,
            buf: vec![0.0; BLOCK * o.channels.count()],
            options: ModuleOptions {
                sample_rate: rate,
                ..o
            },
            metadata,
            info,
            pass,
            passes_left,
            fading: None,
            ended: false,
            pos: 0,
            skip: 0,
        })
    }

    pub fn metadata(&self) -> &ModuleMetadata {
        &self.metadata
    }

    fn fade_len(&self) -> u64 {
        self.options.fade_ms as u64 * self.options.sample_rate as u64 / 1000
    }
}

impl Decoder for ModuleDecoder {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<usize, DecodeError> {
        let ch = self.options.channels.count();
        let mut ends_in_a_row = 0;
        while !self.ended {
            let mut want = BLOCK;
            if let Some(done) = self.fading {
                want = want.min((self.fade_len() - done) as usize);
                if want == 0 {
                    self.ended = true;
                    break;
                }
            }
            let n = self.module.read(
                self.options.sample_rate,
                self.options.channels,
                &mut self.buf[..want * ch],
            );
            if n == 0 {
                ends_in_a_row += 1;
                if ends_in_a_row > 1 {
                    // An empty song: nothing to continue from.
                    self.ended = true;
                    break;
                }
                match (self.passes_left, self.options.end) {
                    (None, _) | (_, EndBehaviour::Continue) => {}
                    (Some(left), _) if left > 0 => self.passes_left = Some(left - 1),
                    (Some(_), EndBehaviour::FadeOut) if self.fading.is_none() => {
                        self.fading = Some(0);
                    }
                    _ => self.ended = true,
                }
                continue;
            }
            let fade_len = self.fade_len() as f32;
            let samples = &mut self.buf[..n * ch];
            if let Some(done) = self.fading {
                let len = fade_len;
                for (i, frame) in samples.chunks_exact_mut(ch).enumerate() {
                    let g = 1.0 - (done + i as u64) as f32 / len;
                    frame.iter_mut().for_each(|s| *s *= g);
                }
                self.fading = Some(done + n as u64);
            }
            let skip = (self.skip as usize).min(n);
            self.skip -= skip as u64;
            if skip == n {
                continue;
            }
            out.extend_from_slice(&samples[skip * ch..]);
            self.pos += (n - skip) as u64;
            return Ok(n - skip);
        }
        Ok(0)
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        if self.pass == 0 {
            return Err(DecodeError::Seek("unknown module length".into()));
        }
        let rate = self.options.sample_rate as u64;
        let mut target = ms * rate / 1000;
        let repeats = u32::try_from(self.options.repeat_count).ok();
        let endless = repeats.is_none() || self.options.end == EndBehaviour::Continue;
        let body = self.pass * (repeats.unwrap_or(0) as u64 + 1);
        if !endless {
            target = target.min(body);
        }
        let mut pass_no = target / self.pass;
        if !endless && target == body {
            pass_no -= 1;
        }
        let within = (target - pass_no * self.pass) as f64 / rate as f64;
        // The library seeks to row starts; start early and render up to the exact frame.
        let mut reached = self.module.set_position(within).max(0.0);
        if reached > within {
            reached = self.module.set_position((within - 1.0).max(0.0)).max(0.0);
        }
        let start = pass_no * self.pass + (reached * rate as f64).round() as u64;
        self.skip = target.saturating_sub(start);
        self.pos = start + self.skip;
        self.passes_left = repeats.map(|r| r.saturating_sub(pass_no as u32));
        self.fading = None;
        self.ended = false;
        Ok(self.pos * 1000 / rate)
    }

    fn kbps(&self) -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4-channel ProTracker module: one looped square-wave sample, four notes on channel 1
    /// and a pattern break after row 15, so the song lasts 16 rows (1.92 s at speed 6, 125 BPM).
    fn test_mod() -> Vec<u8> {
        let mut m = b"nulamp test".to_vec();
        m.resize(20, 0);
        for i in 0..31 {
            let mut h = if i == 0 {
                b"square".to_vec()
            } else {
                Vec::new()
            };
            h.resize(22, 0);
            let (len, vol, loop_len) = if i == 0 {
                (32u16, 64u8, 32u16)
            } else {
                (0, 0, 1)
            };
            h.extend_from_slice(&len.to_be_bytes());
            h.extend_from_slice(&[0, vol]);
            h.extend_from_slice(&0u16.to_be_bytes());
            h.extend_from_slice(&loop_len.to_be_bytes());
            m.extend(h);
        }
        m.extend_from_slice(&[1, 127]);
        m.extend_from_slice(&[0; 128]);
        m.extend_from_slice(b"M.K.");
        let mut pattern = vec![0u8; 64 * 4 * 4];
        for row in [0usize, 4, 8, 12] {
            let period: u16 = [428, 381, 339, 320][row / 4];
            let cell = &mut pattern[row * 16..row * 16 + 4];
            cell.copy_from_slice(&[(period >> 8) as u8, period as u8, 0x10, 0]);
        }
        pattern[15 * 16 + 4..15 * 16 + 8].copy_from_slice(&[0, 0, 0x0D, 0]);
        m.extend(pattern);
        m.extend((0..64).map(|i| if i < 32 { 100u8 } else { 156 }));
        m
    }

    fn decode_all(d: &mut dyn Decoder) -> Vec<f32> {
        let mut out = Vec::new();
        while d.read(&mut out).unwrap() > 0 {}
        out
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn plays_a_protracker_module() {
        let mut d = ModuleDecoder::from_memory(&test_mod(), &ModuleOptions::default()).unwrap();
        assert_eq!(d.metadata().title, "nulamp test");
        assert!(
            d.metadata().format.contains("ProTracker"),
            "{:?}",
            d.metadata()
        );
        assert_eq!(d.info().channels, 2);
        let len = d.info().length_ms.unwrap();
        assert!((1900..=1940).contains(&len), "{len}");
        let pcm = decode_all(&mut d);
        let frames = pcm.len() / 2;
        assert!((84_000..=85_400).contains(&frames), "{frames}");
        assert!(peak(&pcm) > 0.05);
        assert_eq!(d.read(&mut Vec::new()).unwrap(), 0);

        assert_eq!(d.seek(1000).unwrap(), 1000);
        let rest = decode_all(&mut d);
        assert_eq!(rest.len(), pcm.len() - 88_200);
        let diff = rest
            .iter()
            .zip(&pcm[88_200..])
            .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(diff < 1e-4, "{diff}");
    }

    #[test]
    fn repeats_and_fades_out() {
        let options = ModuleOptions {
            repeat_count: 1,
            end: EndBehaviour::FadeOut,
            channels: ModuleChannels::Mono,
            ..ModuleOptions::default()
        };
        let mut d = ModuleDecoder::from_memory(&test_mod(), &options).unwrap();
        assert_eq!(d.info().channels, 1);
        let len = d.info().length_ms.unwrap();
        assert!((4800..=4900).contains(&len), "{len}");
        let pcm = decode_all(&mut d);
        let expected = (d.pass * 2 + 44_100) as usize;
        assert!(
            pcm.len().abs_diff(expected) < 1500,
            "{} vs {expected}",
            pcm.len()
        );
        assert!(peak(&pcm[pcm.len() - 2000..]) < 0.02, "faded out");
        assert!(peak(&pcm[pcm.len() - 40_000..pcm.len() - 30_000]) > 0.01);

        assert_eq!(d.seek(2500).unwrap(), 2500);
        let rest = decode_all(&mut d).len();
        let expected = expected - 2500 * 441 / 10;
        assert!(rest.abs_diff(expected) < 1500, "{rest} vs {expected}");
    }

    #[test]
    fn endless_and_quad() {
        let options = ModuleOptions {
            repeat_count: -1,
            channels: ModuleChannels::Quad,
            interpolation: Interpolation::None,
            amiga_resampler: true,
            ..ModuleOptions::default()
        };
        let mut d = ModuleDecoder::from_memory(&test_mod(), &options).unwrap();
        assert_eq!(d.info().length_ms, None);
        assert_eq!(d.info().channels, 4);
        let mut out = Vec::new();
        let mut frames = 0;
        while frames < 44_100 * 5 {
            let n = d.read(&mut out).unwrap();
            assert!(n > 0);
            frames += n;
        }
        assert!(peak(&out[out.len() - 44_100..]) > 0.01);
    }

    #[test]
    fn registry_claims_module_extensions() {
        let r = crate::Registry::with_defaults();
        for ext in ["mod", "s3m", "xm", "it", "mptm", "umx", "mo3", "669"] {
            assert!(r.is_supported_extension(ext), "{ext}");
        }
        assert_eq!(r.select("/x/a.mod").unwrap().name(), "libopenmpt");
        assert_eq!(r.select("/x/a.mid").unwrap().name(), "MIDI");
        assert!(ModuleDecoder::from_memory(b"not a module", &ModuleOptions::default()).is_err());
    }
}
