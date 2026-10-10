//! Software renderer and sprite tables for skinned windows.
//!
//! Drawing functions are pure: they take a skin and a view state and return a frame. This makes
//! every window testable without a display.

pub mod eq_window;
pub mod frame;
pub mod main_window;
pub mod probe;
pub mod region;

pub use frame::{Frame, blit, scale};
