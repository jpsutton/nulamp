//! Visualization modules (08 §6). The application drives [`PluginVis`] on a dedicated thread
//! with [`PluginData`] from `nulamp_vis::PluginAnalyzer` and shows its RGBA framebuffer.

use std::ffi::c_void;

use nulamp_vis::PluginData;

use crate::PluginError;
use crate::module::Module;
use crate::sys::*;

/// A visualization module.
#[derive(Debug, Clone)]
pub struct VisModule {
    module: Module,
    api: NulampVisApi,
}

/// What a render call asks for next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisStatus {
    Continue,
    /// The plugin asked to stop (or failed); further renders do nothing. Drop the instance.
    Quit,
}

impl VisModule {
    pub(crate) fn new(module: Module) -> Self {
        // SAFETY: the loader checked that `api` is a visualization record of a sufficient size.
        let api = unsafe { *module.api().cast::<NulampVisApi>() };
        Self { module, api }
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Requested spectrum channels (0–2).
    pub fn spectrum_channels(&self) -> usize {
        self.api.spectrum_channels.min(2) as usize
    }

    /// Requested waveform channels (0–2).
    pub fn waveform_channels(&self) -> usize {
        self.api.waveform_channels.min(2) as usize
    }

    /// The channel count to compute data for (1 or 2): the larger request.
    pub fn data_channels(&self) -> usize {
        self.spectrum_channels()
            .max(self.waveform_channels())
            .clamp(1, 2)
    }

    /// The host gives data for "now + latency".
    pub fn latency_ms(&self) -> u32 {
        self.api.latency_ms
    }

    /// Wanted time between frames.
    pub fn delay_ms(&self) -> u32 {
        self.api.delay_ms
    }

    /// Starts an instance with a `width` × `height` surface. Call on the thread that will
    /// render: the instance cannot move to another thread.
    pub fn start(&self, width: u32, height: u32) -> Result<PluginVis, PluginError> {
        let start = self.api.start.expect("checked at load");
        // SAFETY: a valid module function; this thread becomes the instance's thread.
        let inst = unsafe { start(self.module.user(), width, height) };
        if inst.is_null() {
            return Err(PluginError::CreateFailed {
                module: self.module.info().name.clone(),
            });
        }
        let mut v = PluginVis {
            vis: self.clone(),
            inst,
            pixels: Vec::new(),
            width: 0,
            height: 0,
            frame: Box::new(NulampVisFrame {
                abi_version: NULAMP_ABI_VERSION,
                struct_size: size_of::<NulampVisFrame>() as u32,
                sample_rate: 0,
                channels: 0,
                spectrum: [[0; NULAMP_VIS_SAMPLES]; 2],
                waveform: [[0; NULAMP_VIS_SAMPLES]; 2],
                pcm: [std::ptr::null(); 2],
                fft: [std::ptr::null(); 2],
                pcm_frames: NULAMP_VIS_SAMPLES as u32,
                fft_bins: NULAMP_VIS_FFT_BINS as u32,
                pixels: std::ptr::null_mut(),
                width: 0,
                height: 0,
                stride: 0,
                reserved: 0,
            }),
            done: false,
        };
        v.resize(width, height);
        Ok(v)
    }
}

/// A running visualization. Not `Send`: start, render and stop happen on one thread (08 §6).
pub struct PluginVis {
    vis: VisModule,
    /// Raw pointer: also makes the type neither `Send` nor `Sync`.
    inst: *mut c_void,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    frame: Box<NulampVisFrame>,
    done: bool,
}

impl std::fmt::Debug for PluginVis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginVis")
            .field("module", &self.vis.module.info().name)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl PluginVis {
    pub fn module(&self) -> &VisModule {
        &self.vis
    }

    /// Changes the surface size. The pixels are cleared to black.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.pixels.clear();
        self.pixels.resize(width as usize * height as usize * 4, 0);
    }

    /// The RGBA surface (alpha is ignored), `stride()` bytes per row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes per row.
    pub fn stride(&self) -> u32 {
        self.width * 4
    }

    /// Hands one frame of data to the plugin, which draws into the surface. `data` comes from
    /// `PluginAnalyzer::compute` with [`VisModule::data_channels`] channels.
    pub fn render(&mut self, data: &PluginData, sample_rate: u32) -> VisStatus {
        if self.done {
            return VisStatus::Quit;
        }
        let f = &mut *self.frame;
        f.sample_rate = sample_rate;
        f.channels = data.channels.clamp(1, 2) as u32;
        f.spectrum = data.spectrum;
        f.waveform = data.waveform;
        f.pcm = [data.pcm[0].as_ptr(), data.pcm[1].as_ptr()];
        f.fft = [data.fft[0].as_ptr(), data.fft[1].as_ptr()];
        f.pixels = self.pixels.as_mut_ptr();
        f.width = self.width;
        f.height = self.height;
        f.stride = self.width * 4;
        let render = self.vis.api.render.expect("checked at load");
        // SAFETY: a valid instance on its own thread (the type is not `Send`); the frame and
        // every buffer it points to (`data`, `pixels`) live across the call.
        let r = unsafe { render(self.inst, &*self.frame) };
        // Pointers into `data` must not outlive this call.
        self.frame.pcm = [std::ptr::null(); 2];
        self.frame.fft = [std::ptr::null(); 2];
        if r == NULAMP_VIS_CONTINUE {
            VisStatus::Continue
        } else {
            if r < 0 {
                log::warn!(
                    "visualization {}: render failed ({r})",
                    self.vis.module.info().name
                );
            }
            self.done = true;
            VisStatus::Quit
        }
    }
}

impl Drop for PluginVis {
    fn drop(&mut self) {
        let stop = self.vis.api.stop.expect("checked at load");
        // SAFETY: the instance came from `start` on this thread and is stopped once.
        unsafe { stop(self.inst) };
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

    struct Bars {
        frames: u32,
    }

    unsafe extern "C" fn init(_: *mut c_void, _: *const NulampHost, _: u64) -> i32 {
        NULAMP_OK
    }
    unsafe extern "C" fn quit(_: *mut c_void) {}
    unsafe extern "C" fn start(_: *mut c_void, _: u32, _: u32) -> *mut c_void {
        Box::into_raw(Box::new(Bars { frames: 0 })).cast()
    }
    /// Draws column x with the spectrum value of bin x as red; quits on the third frame.
    unsafe extern "C" fn render(inst: *mut c_void, frame: *const NulampVisFrame) -> i32 {
        // SAFETY: from `start`; the host passes a valid frame during the call.
        let (b, f) = unsafe { (&mut *inst.cast::<Bars>(), &*frame) };
        b.frames += 1;
        for y in 0..f.height {
            for x in 0..f.width {
                let i = (y * f.stride + x * 4) as usize;
                // SAFETY: inside the `height` × `stride` surface.
                unsafe {
                    *f.pixels.add(i) = f.spectrum[0][x as usize];
                    *f.pixels.add(i + 1) = (*f.pcm[1].add(x as usize) * 100.0) as u8;
                    *f.pixels.add(i + 2) = f.channels as u8;
                }
            }
        }
        if b.frames >= 3 {
            NULAMP_VIS_QUIT
        } else {
            NULAMP_VIS_CONTINUE
        }
    }
    unsafe extern "C" fn stop(inst: *mut c_void) {
        // SAFETY: from `start`, stopped once.
        drop(unsafe { Box::from_raw(inst.cast::<Bars>()) });
    }

    static API: NulampVisApi = NulampVisApi {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampVisApi>() as u32,
        spectrum_channels: 1,
        waveform_channels: 2,
        latency_ms: 25,
        delay_ms: 16,
        start: Some(start),
        render: Some(render),
        stop: Some(stop),
    };
    static MODULE: NulampModule = NulampModule {
        abi_version: NULAMP_ABI_VERSION,
        struct_size: size_of::<NulampModule>() as u32,
        kind: NULAMP_KIND_VIS,
        flags: 0,
        api: &API as *const NulampVisApi as *const c_void,
        name: c"Bars".as_ptr(),
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
        name: c"Bars plugin".as_ptr(),
        version: c"1".as_ptr(),
        description: c"".as_ptr(),
        module_count: 1,
        reserved: 0,
        modules: MODULES.as_ptr(),
    };
    unsafe extern "C" fn entry(_: *const NulampHost) -> *const NulampPluginDesc {
        &DESC
    }

    #[test]
    fn renders_into_the_framebuffer() {
        let host = Host::new(Arc::new(Recorder::default()));
        let mut p = Plugins::default();
        // SAFETY: `entry` follows the ABI.
        unsafe { p.load_builtin(entry, "bars", &host) }.unwrap();
        let m = &p.vis[0];
        assert_eq!((m.spectrum_channels(), m.waveform_channels()), (1, 2));
        assert_eq!(
            (m.data_channels(), m.latency_ms(), m.delay_ms()),
            (2, 25, 16)
        );
        let mut v = m.start(4, 2).unwrap();
        let mut data = PluginData::default();
        data.spectrum[0][..4].copy_from_slice(&[10, 20, 30, 40]);
        data.pcm[1][3] = 0.5;
        assert_eq!(v.render(&data, 44_100), VisStatus::Continue);
        assert_eq!(v.stride(), 16);
        let px = v.pixels();
        assert_eq!(&px[..4], &[10, 0, 2, 0]);
        assert_eq!(&px[16 + 12..16 + 16], &[40, 50, 2, 0]);
        v.resize(2, 1);
        assert_eq!(v.pixels().len(), 8);
        assert_eq!(v.render(&data, 44_100), VisStatus::Continue);
        assert_eq!(v.pixels(), &[10, 0, 2, 0, 20, 0, 2, 0]);
        assert_eq!(v.render(&data, 44_100), VisStatus::Quit);
        assert_eq!(v.render(&data, 44_100), VisStatus::Quit);
    }
}
