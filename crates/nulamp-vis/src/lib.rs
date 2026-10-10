//! Classic spectrum analyzer and oscilloscope (spec 06).
//!
//! Pure logic: no window, no audio device.
//!
//! - [`Analyzer`]: turns 576-frame audio blocks into 75 spectrum bars and 75 oscilloscope
//!   values, stamped with the output frame at which they are heard (06 §2).
//! - [`Visualizer`]: bar falloff, peaks, idle animation and refresh timing (06 §2.4–§4.4),
//!   and drawing into a [`nulamp_skin::Bitmap`] at every size (06 §4–§5).
//! - [`VisSettings`]: the visualization menu preferences (06 §6).
//! - [`PluginAnalyzer`]: the data handed to external visualization plugins (06 §7.1).

mod analysis;
mod draw;
mod fft;
mod plugin;
mod settings;
mod state;

pub use analysis::{
    Analyzer, IDLE_TIMEOUT_MS, Spectrum, VisData, bar_range, scope_values, to_signed_8bit,
};
pub use draw::draw_background;
pub use plugin::{PLUGIN_SAMPLES, PluginAnalyzer, PluginData};
pub use settings::{AnalyzerStyle, RefreshRate, ScopeStyle, Speed, VisMode, VisSettings};
pub use state::{VisSize, Visualizer};

/// Number of spectrum bars and oscilloscope values per frame (06 §2.2, §2.3).
pub const BARS: usize = 75;

/// Frames per audio block fed to the analyzer (06 §2.1).
pub const BLOCK_FRAMES: usize = 576;

/// FFT length of the classic analyzer and of plugin spectra (06 §2.3, §7.1).
pub const FFT_LEN: usize = 512;

/// Index of each colour in the 24-entry `viscolor.txt` palette (06 §3).
pub mod color {
    /// Background.
    pub const BACKGROUND: u8 = 0;
    /// Grid dots.
    pub const GRID: u8 = 1;
    /// Top (loudest) analyzer colour.
    pub const BAR_TOP: u8 = 2;
    /// Bottom analyzer colour.
    pub const BAR_BOTTOM: u8 = 17;
    /// Oscilloscope centre (brightest).
    pub const SCOPE_CENTRE: u8 = 18;
    /// Analyzer peak dots.
    pub const PEAK: u8 = 23;
}
