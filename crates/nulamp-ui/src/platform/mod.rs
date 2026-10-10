//! Platform glue: window shapes and presentation helpers.

pub mod shape;
#[cfg(all(unix, not(target_os = "macos")))]
pub mod wayland;
#[cfg(not(all(unix, not(target_os = "macos"))))]
#[path = "wayland_stub.rs"]
pub mod wayland;
