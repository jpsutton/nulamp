//! Non-rectangular windows from `region.txt` (01 §4.4).
//!
//! X11 uses the SHAPE extension for both the visible (bounding) and the input shape. Other
//! platforms are not implemented yet; their windows stay rectangular.

use nulamp_render::region::Mask;
use raw_window_handle::HasWindowHandle;

/// Applies a shape to a window. `None` restores a plain rectangle.
pub fn apply(window: &impl HasWindowHandle, mask: Option<&Mask>) {
    #[cfg(all(unix, not(target_os = "macos")))]
    x11::apply(window, mask);
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let _ = (window, mask);
}

#[cfg(all(unix, not(target_os = "macos")))]
mod x11 {
    use super::*;
    use raw_window_handle::RawWindowHandle;
    use std::sync::{Mutex, OnceLock};
    use x11rb::connection::Connection;
    use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
    use x11rb::protocol::xproto::{ClipOrdering, Rectangle};
    use x11rb::rust_connection::RustConnection;

    fn connection() -> Option<&'static Mutex<RustConnection>> {
        static CONN: OnceLock<Option<Mutex<RustConnection>>> = OnceLock::new();
        CONN.get_or_init(|| x11rb::connect(None).ok().map(|(c, _)| Mutex::new(c)))
            .as_ref()
    }

    pub fn apply(window: &impl HasWindowHandle, mask: Option<&Mask>) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let id = match handle.as_raw() {
            RawWindowHandle::Xlib(h) => h.window as u32,
            RawWindowHandle::Xcb(h) => h.window.get(),
            _ => return,
        };
        let Some(conn) = connection() else { return };
        let Ok(conn) = conn.lock() else { return };
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            match mask {
                None => {
                    for kind in [SK::BOUNDING, SK::INPUT] {
                        conn.shape_mask(SO::SET, kind, id, 0, 0, x11rb::NONE)?;
                    }
                }
                Some(m) => {
                    let rects: Vec<Rectangle> = m
                        .runs()
                        .into_iter()
                        .map(|(x, y, w)| Rectangle {
                            x: x as i16,
                            y: y as i16,
                            width: w as u16,
                            height: 1,
                        })
                        .collect();
                    for kind in [SK::BOUNDING, SK::INPUT] {
                        conn.shape_rectangles(
                            SO::SET,
                            kind,
                            ClipOrdering::UNSORTED,
                            id,
                            0,
                            0,
                            &rects,
                        )?;
                    }
                }
            }
            conn.flush()?;
            Ok(())
        })();
        if let Err(e) = result {
            log::warn!("cannot apply window shape: {e}");
        }
    }
}
