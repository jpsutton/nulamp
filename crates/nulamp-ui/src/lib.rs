//! Window manager, input handling, docking and platform layer.

pub mod docking;
pub mod eq_input;
pub mod geom;
pub mod input;
pub mod keys;
pub mod main_input;
pub mod marquee;
pub mod platform;
pub mod playlist;
pub mod shell;
pub mod title;

pub use shell::{PlayerHandle, Shell, UiPrefs};
