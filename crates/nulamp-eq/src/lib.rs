//! 10-band equalizer, limiter and preset files (spec 03 §3–§7, 07 §4–§5).
//!
//! - [`value`]: the 0..63 slider encoding, dB mapping, preamp table and marquee labels.
//! - [`Equalizer`]: gain stage, band sections and limiter for interleaved `f32` audio.
//! - [`replaygain`]: ReplayGain settings and the per-track gain factor.
//! - [`preset`]: EQF / q1 / q2 files, first-run presets and auto-load lookup.

pub mod filter;
pub mod limiter;
pub mod preset;
pub mod replaygain;
pub mod settings;
pub mod value;

pub use filter::{Equalizer, MAX_CHANNELS};
pub use limiter::{LIMITER_THRESHOLD, Limiter};
pub use preset::{Preset, PresetError};
pub use replaygain::{ReplayGainSettings, ReplayGainTags};
pub use settings::{EqSettings, FilterEngine};
pub use value::{BANDS, BandMode, Slider, band_db, preamp_gain, slider_db, slider_db_label};
