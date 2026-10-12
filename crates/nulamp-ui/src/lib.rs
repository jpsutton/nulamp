//! Window manager, input handling, docking and platform layer.

pub mod dialogs;
pub mod docking;
pub mod eq_input;
pub mod files;
pub mod geom;
pub mod hotkeys;
pub mod input;
pub mod keys;
pub mod library;
pub mod main_input;
pub mod marquee;
pub mod menu;
pub mod menus;
pub mod platform;
pub mod playlist;
pub mod plugins_dialog;
pub mod prefs_dialog;
pub mod prefs_jellyfin;
pub mod prefs_vis;
pub mod shell;
pub mod title;
pub mod tray;
pub mod video;
pub mod wayland_limits;

pub use shell::{PlayerHandle, Shell, UiPrefs};
