//! DSP modules as a processing [`Stage`] (08 §5, 07 §4 stage 4).

use std::collections::VecDeque;
use std::ffi::c_void;

use nulamp_audio::chain::Stage;

use crate::PluginError;
use crate::module::Module;
use crate::sys::*;

/// A DSP (effect) module.
#[derive(Debug, Clone)]
pub struct DspModule {
    module: Module,
    api: NulampDspApi,
}

impl DspModule {
    pub(crate) fn new(module: Module) -> Self {
        // SAFETY: the loader checked that `api` is a DSP record of a sufficient size.
        let api = unsafe { *module.api().cast::<NulampDspApi>() };
        Self { module, api }
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Creates an effect instance for the audio chain.
    pub fn instantiate(&self) -> Result<PluginDsp, PluginError> {
        let create = self.api.create.expect("checked at load");
        // SAFETY: a valid module function with its `user` value.
        let inst = unsafe { create(self.module.user()) };
        if inst.is_null() {
            return Err(PluginError::CreateFailed {
                module: self.module.info().name.clone(),
            });
        }
        Ok(PluginDsp {
            dsp: self.clone(),
            inst,
            channels: 0,
            active: false,
            out: Vec::new(),
            fifo: VecDeque::new(),
            warned: false,
        })
    }
}

/// One instance of a DSP module, used as a [`Stage`] on the audio thread.
///
/// The effect may change the frame count (between half and twice the input is typical).
/// [`Stage::process_resizing`] passes that through; the in-place [`Stage::process`] instead
/// buffers the output and fills each block from that buffer, padding with silence at the start
/// while the buffer is short.
///
/// While the format is unusable or `configure` failed, and for blocks the effect rejects, the
/// audio passes through unchanged.
pub struct PluginDsp {
    dsp: DspModule,
    inst: *mut c_void,
    channels: usize,
    active: bool,
    out: Vec<f32>,
    fifo: VecDeque<f32>,
    warned: bool,
}

// SAFETY: the ABI lets an instance be used from one thread at a time; `&mut self` on every call
// guarantees that, and the type is not `Sync`.
unsafe impl Send for PluginDsp {}

impl std::fmt::Debug for PluginDsp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginDsp")
            .field("module", &self.dsp.module.info().name)
            .field("channels", &self.channels)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl PluginDsp {
    pub fn module(&self) -> &Module {
        &self.dsp.module
    }

    /// The effect's declared latency in frames (0 when it declares none).
    pub fn latency_frames(&self) -> u32 {
        match self.dsp.api.latency_frames {
            // SAFETY: a valid instance, used from the owning thread.
            Some(f) => unsafe { f(self.inst) },
            None => 0,
        }
    }

    /// Runs the effect over the whole frames of `input` into `self.out`. Returns the output
    /// frame count, or `None` to pass the input through.
    fn run(&mut self, input: &[f32]) -> Option<usize> {
        if !self.active {
            return None;
        }
        let ch = self.channels;
        let frames = input.len() / ch;
        if frames == 0 || frames > (u32::MAX / 2) as usize {
            return None;
        }
        let max_out = 2 * frames;
        self.out.resize(max_out * ch, 0.0);
        let process = self.dsp.api.process.expect("checked at load");
        // SAFETY: a valid, configured instance; `input` holds `frames` frames, `out` holds
        // `max_out` frames, and they do not overlap.
        let n = unsafe {
            process(
                self.inst,
                input.as_ptr(),
                frames as u32,
                self.out.as_mut_ptr(),
                max_out as u32,
            )
        };
        if n < 0 {
            if !self.warned {
                self.warned = true;
                log::warn!(
                    "DSP {}: process failed ({n}), passing audio through",
                    self.dsp.module.info().name
                );
            }
            return None;
        }
        Some((n as usize).min(max_out))
    }
}

impl Stage for PluginDsp {
    fn configure(&mut self, sample_rate: u32, channels: usize) {
        self.fifo.clear();
        self.channels = channels;
        self.active = false;
        if sample_rate == 0 || channels == 0 || channels > 8 {
            return;
        }
        let configure = self.dsp.api.configure.expect("checked at load");
        // SAFETY: a valid instance, used from the owning thread.
        let r = unsafe { configure(self.inst, sample_rate, channels as u32) };
        self.active = r == NULAMP_OK;
        if !self.active {
            log::warn!(
                "DSP {}: cannot handle {sample_rate} Hz, {channels} channels ({r})",
                self.dsp.module.info().name
            );
        }
    }

    fn process(&mut self, samples: &mut [f32]) {
        let Some(n) = self.run(samples) else { return };
        let ch = self.channels;
        self.fifo.extend(&self.out[..n * ch]);
        let whole = samples.len() / ch * ch;
        let short = whole.saturating_sub(self.fifo.len());
        samples[..short].fill(0.0);
        for (s, v) in samples[short..whole]
            .iter_mut()
            .zip(self.fifo.drain(..whole - short))
        {
            *s = v;
        }
    }

    fn process_resizing(&mut self, samples: &mut Vec<f32>) {
        let Some(n) = self.run(samples) else { return };
        let ch = self.channels;
        let whole = samples.len() / ch * ch;
        // A trailing partial frame is kept after the processed frames.
        samples.splice(..whole, self.out[..n * ch].iter().copied());
    }

    fn reset(&mut self) {
        self.fifo.clear();
        if let Some(flush) = self.dsp.api.flush {
            // SAFETY: a valid instance, used from the owning thread.
            unsafe { flush(self.inst) };
        }
    }
}

impl Drop for PluginDsp {
    fn drop(&mut self) {
        let destroy = self.dsp.api.destroy.expect("checked at load");
        // SAFETY: the instance came from `create` and is destroyed once; the module (and its
        // file) outlive it because `self.dsp` holds them.
        unsafe { destroy(self.inst) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::host::Host;
    use crate::host::tests::Recorder;
    use crate::loader::Plugins;
    use crate::sys::StaticPtrs;

    /// Test effect: repeats every frame (2× output), negated. Rejects 3 channels.
    struct Doubler {
        channels: usize,
        flushed: u32,
    }

    unsafe extern "C" fn init(_: *mut c_void, _: *const NulampHost, _: u64) -> i32 {
        NULAMP_OK
    }
    unsafe extern "C" fn quit(_: *mut c_void) {}
    unsafe extern "C" fn create(_: *mut c_void) -> *mut c_void {
        Box::into_raw(Box::new(Doubler {
            channels: 0,
            flushed: 0,
        }))
        .cast()
    }
    unsafe extern "C" fn destroy(inst: *mut c_void) {
        // SAFETY: from `create`, destroyed once.
        drop(unsafe { Box::from_raw(inst.cast::<Doubler>()) });
    }
    unsafe extern "C" fn configure(inst: *mut c_void, _rate: u32, channels: u32) -> i32 {
        // SAFETY: from `create`.
        let d = unsafe { &mut *inst.cast::<Doubler>() };
        d.channels = channels as usize;
        if channels == 3 {
            NULAMP_ERR_UNSUPPORTED
        } else {
            NULAMP_OK
        }
    }
    unsafe extern "C" fn process(
        inst: *mut c_void,
        input: *const f32,
        frames: u32,
        out: *mut f32,
        max_out: u32,
    ) -> i64 {
        // SAFETY: from `create`; the host passes buffers of the stated sizes.
        let d = unsafe { &*inst.cast::<Doubler>() };
        let ch = d.channels;
        let (input, out) = unsafe {
            (
                std::slice::from_raw_parts(input, frames as usize * ch),
                std::slice::from_raw_parts_mut(out, max_out as usize * ch),
            )
        };
        for (f, frame) in input.chunks_exact(ch).enumerate() {
            for rep in 0..2 {
                for (c, &s) in frame.iter().enumerate() {
                    out[(2 * f + rep) * ch + c] = -s;
                }
            }
        }
        2 * frames as i64
    }
    unsafe extern "C" fn latency(_: *mut c_void) -> u32 {
        7
    }
    unsafe extern "C" fn flush(inst: *mut c_void) {
        // SAFETY: from `create`.
        unsafe { (*inst.cast::<Doubler>()).flushed += 1 };
    }

    static API: NulampDspApi = NulampDspApi {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampDspApi>() as u32,
        create: Some(create),
        destroy: Some(destroy),
        configure: Some(configure),
        process: Some(process),
        latency_frames: Some(latency),
        flush: Some(flush),
    };
    static MODULE: NulampModule = NulampModule {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampModule>() as u32,
        kind: NULAMP_KIND_DSP,
        flags: 0,
        api: &API as *const NulampDspApi as *const c_void,
        name: c"Doubler".as_ptr(),
        version: c"1".as_ptr(),
        description: c"".as_ptr(),
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
        name: c"Doubler plugin".as_ptr(),
        version: c"1".as_ptr(),
        description: c"".as_ptr(),
        module_count: 1,
        reserved: 0,
        modules: MODULES.as_ptr(),
    };
    unsafe extern "C" fn entry(_: *const NulampHost) -> *const NulampPluginDesc {
        &DESC
    }

    fn instance() -> PluginDsp {
        let host = Host::new(Arc::new(Recorder::default()));
        let mut p = Plugins::default();
        // SAFETY: `entry` follows the ABI.
        unsafe { p.load_builtin(entry, "doubler", &host) }.unwrap();
        p.dsps[0].instantiate().unwrap()
    }

    #[test]
    fn resizing_path_changes_the_frame_count() {
        let mut d = instance();
        assert_eq!(d.latency_frames(), 7);
        d.configure(44_100, 2);
        let mut v = vec![1.0, 2.0, 3.0, 4.0, 9.0];
        d.process_resizing(&mut v);
        assert_eq!(v, [-1.0, -2.0, -1.0, -2.0, -3.0, -4.0, -3.0, -4.0, 9.0]);
        d.reset();
        // SAFETY: the instance is a `Doubler`.
        assert_eq!(unsafe { (*d.inst.cast::<Doubler>()).flushed }, 1);
    }

    #[test]
    fn in_place_path_buffers() {
        let mut d = instance();
        d.configure(44_100, 1);
        let mut a = [1.0, 2.0];
        d.process(&mut a);
        assert_eq!(a, [-1.0, -1.0]);
        let mut b = [3.0, 4.0];
        d.process(&mut b);
        assert_eq!(b, [-2.0, -2.0]);
        assert_eq!(d.fifo.len(), 4);
    }

    #[test]
    fn rejected_format_passes_through() {
        let mut d = instance();
        d.configure(44_100, 3);
        let mut v = vec![1.0, 2.0, 3.0];
        d.process_resizing(&mut v);
        assert_eq!(v, [1.0, 2.0, 3.0]);
        d.configure(0, 2);
        d.process(&mut v);
        assert_eq!(v, [1.0, 2.0, 3.0]);
    }
}
