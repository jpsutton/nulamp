//! "Probe" skins for coordinate tests.
//!
//! Every pixel of every probe bitmap encodes which bitmap it belongs to and its own source
//! coordinates. After drawing, each destination pixel can be decoded back to (bitmap, x, y),
//! which checks blit coordinates against the spec without reference screenshots.

use nulamp_skin::container::{ContainerKind, SkinFiles};
use nulamp_skin::{Bitmap, Skin, SkinBitmap, bmp};

/// Encodes (bitmap index, x, y) into a 24-bit colour.
pub fn encode(index: usize, x: u32, y: u32) -> u32 {
    ((index as u32 + 1) << 18) | ((x & 0x1FF) << 9) | (y & 0x1FF)
}

/// Decodes a probe pixel. Returns `None` for black (out-of-bounds) pixels.
pub fn decode(p: u32) -> Option<(SkinBitmap, i32, i32)> {
    let index = (p >> 18) as usize;
    if index == 0 {
        return None;
    }
    let which = *SkinBitmap::ALL.get(index - 1)?;
    Some((which, ((p >> 9) & 0x1FF) as i32, (p & 0x1FF) as i32))
}

/// A probe bitmap for one role, at its default size (or a custom size).
pub fn bitmap(which: SkinBitmap, size: Option<(u32, u32)>) -> Bitmap {
    let index = SkinBitmap::ALL
        .iter()
        .position(|w| *w == which)
        .expect("known bitmap");
    let (w, h) = size.unwrap_or(which.default_size());
    let px = (0..h)
        .flat_map(|y| (0..w).map(move |x| encode(index, x, y)))
        .collect();
    Bitmap::from_pixels(w, h, px)
}

/// Skin files where every bitmap (except those in `omit`) is a probe bitmap.
pub fn files(omit: &[SkinBitmap]) -> SkinFiles {
    let entries = SkinBitmap::ALL
        .iter()
        .filter(|w| !omit.contains(w))
        .map(|&w| (w.file_name().to_string(), bmp::encode_24(&bitmap(w, None))));
    SkinFiles::from_entries(ContainerKind::Archive, entries)
}

/// A probe skin, with the built-in skin as fallback for omitted bitmaps.
pub fn skin(omit: &[SkinBitmap]) -> Skin {
    Skin::load(&files(omit), &nulamp_skin::builtin::files())
}

/// Checks that the destination rectangle (dx, dy, w, h) of `frame` shows the source rectangle
/// (sx, sy) of `which`. Returns a description of the first mismatch.
pub fn check_rect(
    frame: &Bitmap,
    which: SkinBitmap,
    sx: i32,
    sy: i32,
    w: i32,
    h: i32,
    dx: i32,
    dy: i32,
) -> Result<(), String> {
    for y in 0..h {
        for x in 0..w {
            let got = decode(frame.pixel(dx + x, dy + y));
            let want = Some((which, sx + x, sy + y));
            if got != want {
                return Err(format!(
                    "dest ({},{}): expected {:?}, got {:?}",
                    dx + x,
                    dy + y,
                    want,
                    got
                ));
            }
        }
    }
    Ok(())
}
