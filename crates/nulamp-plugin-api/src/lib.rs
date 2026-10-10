//! Native plugins (08 §1–§7): the C ABI, the loader and the adapters into the player.
//!
//! - [`sys`]: the `#[repr(C)]` mirror of `include/nulamp_plugin.h`. Plugins written in Rust
//!   depend on this crate with `default-features = false` and use only this module.
//! - With the default `host` feature: [`Host`] (the services plugins call back into, built
//!   from a [`HostServices`] implementation), [`load_plugins`] and [`Plugins`] (discovery and
//!   loading), and the adapters [`PluginDecoderFactory`] (decoders), [`PluginDsp`] (a
//!   processing [`Stage`](nulamp_audio::chain::Stage)), [`PluginVis`] (visualization) and
//!   [`PluginGeneral`] (event dispatch).
//!
//! Panics never cross the C boundary: every host function a plugin calls catches them.

pub mod sys;

#[cfg(feature = "host")]
mod command;
#[cfg(feature = "host")]
mod dsp;
#[cfg(feature = "host")]
mod general;
#[cfg(feature = "host")]
mod host;
#[cfg(feature = "host")]
mod input;
#[cfg(feature = "host")]
mod loader;
#[cfg(feature = "host")]
mod module;
#[cfg(feature = "host")]
mod vis;

#[cfg(feature = "host")]
pub use command::{HostCommand, command_from_id};
#[cfg(feature = "host")]
pub use dsp::{DspModule, PluginDsp};
#[cfg(feature = "host")]
pub use general::{PluginEvent, PluginGeneral};
#[cfg(feature = "host")]
pub use host::{Host, HostServices};
#[cfg(feature = "host")]
pub use input::{FieldValue, InputModule, PluginDecoder, PluginDecoderFactory};
#[cfg(feature = "host")]
pub use loader::{LoadOptions, Plugins, discover, load_plugins};
#[cfg(feature = "host")]
pub use module::{Module, ModuleInfo, ModuleKind};
#[cfg(feature = "host")]
pub use vis::{PluginVis, VisModule, VisStatus};

/// Errors from loading or using a plugin.
#[cfg(feature = "host")]
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("cannot load {0}")]
    Load(String),
    #[error("no `nulamp_plugin_entry` export: {0}")]
    NoEntry(String),
    #[error("the entry point refused to load")]
    Refused,
    #[error("ABI version {found:#x} is not compatible with {expected:#x}")]
    Abi { found: u32, expected: u32 },
    #[error("invalid plugin data: {0}")]
    Invalid(String),
    #[error("module {module:?} failed to initialise (code {code})")]
    InitFailed { module: String, code: i32 },
    #[error("module {module:?} could not create an instance")]
    CreateFailed { module: String },
}
