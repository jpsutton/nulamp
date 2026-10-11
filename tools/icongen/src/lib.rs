//! Builds nulamp's packaged icon files from `nulamp-icon`.
//!
//! Linux gets the hicolor sizes as PNG plus the scalable SVG; the AppImage reads the 256 px
//! PNG at `assets/linux/nulamp.png`; Windows gets a multi-size `.ico`.

use nulamp_icon::{Image, icon};

/// hicolor theme sizes installed on Linux.
pub const LINUX_SIZES: [u32; 9] = [16, 22, 24, 32, 48, 64, 128, 256, 512];

/// Sizes in the Windows `.ico`.
pub const ICO_SIZES: [u32; 5] = [16, 24, 32, 48, 256];

/// Every generated file, by path relative to the repository root.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = LINUX_SIZES
        .iter()
        .map(|&s| {
            (
                format!("assets/linux/icons/nulamp-{s}.png"),
                encode_png(&icon(s)),
            )
        })
        .collect();
    out.push((
        "assets/linux/icons/nulamp.svg".into(),
        nulamp_icon::SVG.as_bytes().to_vec(),
    ));
    out.push(("assets/linux/nulamp.png".into(), encode_png(&icon(256))));
    out.push(("assets/windows/nulamp.ico".into(), encode_ico(&ICO_SIZES)));
    out
}

/// 8-bit RGBA PNG.
pub fn encode_png(img: &Image) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    let mut w = enc.write_header().expect("PNG header");
    w.write_image_data(&img.rgba).expect("PNG data");
    w.finish().expect("PNG end");
    out
}

/// Windows icon: 32-bit DIB entries below 256 px (read by every Windows version), a PNG
/// entry at 256 px.
pub fn encode_ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let img = icon(s);
            if s >= 256 {
                encode_png(&img)
            } else {
                encode_dib(&img)
            }
        })
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (&s, data) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for data in images {
        out.extend_from_slice(&data);
    }
    out
}

/// A BITMAPINFOHEADER, BGRA rows bottom-up, then the 1-bit AND mask (set where transparent).
fn encode_dib(img: &Image) -> Vec<u8> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mask_row = w.div_ceil(32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(2 * h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((w * h * 4 + mask_row * h) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for y in (0..h).rev() {
        for x in 0..w {
            let p = &img.rgba[(y * w + x) * 4..][..4];
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    for y in (0..h).rev() {
        let mut row = vec![0u8; mask_row];
        for x in 0..w {
            if img.rgba[(y * w + x) * 4 + 3] == 0 {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    out
}
