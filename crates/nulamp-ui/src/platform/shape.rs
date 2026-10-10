//! Non-rectangular windows from `region.txt` (01 §4.4).
//!
//! X11 uses the SHAPE extension for both the visible (bounding) and the input shape. Windows
//! uses a window region (`SetWindowRgn`), which clips both drawing and input. Other platforms
//! are not implemented yet; their windows stay rectangular.

use nulamp_render::region::Mask;
use raw_window_handle::HasWindowHandle;

/// Applies a shape to a window. `None` restores a plain rectangle.
pub fn apply(window: &impl HasWindowHandle, mask: Option<&Mask>) {
    #[cfg(all(unix, not(target_os = "macos")))]
    x11::apply(window, mask);
    #[cfg(windows)]
    win32::apply(window, mask);
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    let _ = (window, mask);
}

#[cfg(windows)]
mod win32 {
    use super::*;
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, DeleteObject, RGN_OR, SetWindowRgn,
    };

    pub fn apply(window: &impl HasWindowHandle, mask: Option<&Mask>) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(h) = handle.as_raw() else {
            return;
        };
        let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
        // SAFETY: `hwnd` is a live window owned by this process; region handles are created
        // here and either handed to the system (SetWindowRgn takes ownership) or deleted.
        unsafe {
            let Some(m) = mask else {
                SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
                return;
            };
            let region = CreateRectRgn(0, 0, 0, 0);
            for (x, y, w) in m.runs() {
                let row = CreateRectRgn(x as i32, y as i32, (x + w) as i32, y as i32 + 1);
                CombineRgn(region, region, row, RGN_OR);
                DeleteObject(row);
            }
            SetWindowRgn(hwnd, region, 1);
        }
    }
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
