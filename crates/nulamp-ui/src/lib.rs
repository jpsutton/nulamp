//! Window manager, input handling, docking and platform layer.

pub mod docking;
pub mod geom;
pub mod input;
pub mod keys;
pub mod main_input;
pub mod marquee;
pub mod platform;
pub mod shell;
pub mod title;

pub use shell::{PlayerHandle, Shell, UiPrefs};
