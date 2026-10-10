//! Windows BMP decoding (01 §2.1) and a small encoder used by tools and tests.
//!
//! Supported: 1, 4, 8, 16, 24 and 32 bits per pixel; top-down and bottom-up rows; palettes;
//! RLE4 and RLE8; BITFIELDS masks; all common header versions. Alpha is ignored.

use crate::bitmap::Bitmap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BmpError {
    #[error("not a BMP file")]
    NotBmp,
    #[error("truncated BMP header")]
    Truncated,
    #[error("unsupported BMP: {0}")]
    Unsupported(&'static str),
}

const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32le(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One channel mask, as shift and bit width.
#[derive(Clone, Copy)]
struct Mask {
    shift: u32,
    bits: u32,
}

impl Mask {
    fn new(m: u32) -> Self {
        if m == 0 {
            return Self { shift: 0, bits: 0 };
        }
        let shift = m.trailing_zeros();
        let bits = (m >> shift).trailing_ones();
        Self { shift, bits }
    }

    /// Extracts the channel and scales it to 0..255.
    fn get(self, v: u32) -> u32 {
        if self.bits == 0 {
            return 0;
        }
        let max = if self.bits >= 32 {
            u32::MAX
        } else {
            (1u32 << self.bits) - 1
        };
        let c = (v >> self.shift) & max;
        if self.bits == 8 {
            c
        } else {
            ((c as u64 * 255 + max as u64 / 2) / max as u64) as u32
        }
    }
}

/// Decodes a BMP file (with its 14-byte file header).
pub fn decode(data: &[u8]) -> Result<Bitmap, BmpError> {
    if data.len() < 2 || &data[0..2] != b"BM" {
        return Err(BmpError::NotBmp);
    }
    let off_bits = u32le(data, 10).ok_or(BmpError::Truncated)? as usize;
    decode_dib(&data[14..], off_bits.checked_sub(14), false)
}

/// Decodes a device-independent bitmap without a file header.
///
/// `pixel_offset` is the offset of the pixel data from the start of `dib`, if known.
/// When `icon` is true, the stored height is twice the image height (XOR + AND masks);
/// only the colour part is decoded.
pub(crate) fn decode_dib(
    dib: &[u8],
    pixel_offset: Option<usize>,
    icon: bool,
) -> Result<Bitmap, BmpError> {
    let hsize = u32le(dib, 0).ok_or(BmpError::Truncated)? as usize;
    let (width, raw_height, bpp, compression, clr_used, core);
    if hsize == 12 {
        width = u16le(dib, 4).ok_or(BmpError::Truncated)? as i32;
        raw_height = u16le(dib, 6).ok_or(BmpError::Truncated)? as i16 as i32;
        bpp = u16le(dib, 10).ok_or(BmpError::Truncated)? as u32;
        compression = BI_RGB;
        clr_used = 0;
        core = true;
    } else if hsize >= 40 {
        width = u32le(dib, 4).ok_or(BmpError::Truncated)? as i32;
        raw_height = u32le(dib, 8).ok_or(BmpError::Truncated)? as i32;
        bpp = u16le(dib, 14).ok_or(BmpError::Truncated)? as u32;
        compression = u32le(dib, 16).ok_or(BmpError::Truncated)?;
        clr_used = u32le(dib, 32).ok_or(BmpError::Truncated)?;
        core = false;
    } else {
        return Err(BmpError::Unsupported("header size"));
    }
    if width <= 0 || raw_height == 0 || raw_height == i32::MIN {
        return Err(BmpError::Unsupported("dimensions"));
    }
    let top_down = raw_height < 0;
    let mut height = raw_height.unsigned_abs() as i32;
    if icon {
        height /= 2;
        if height == 0 {
            return Err(BmpError::Unsupported("dimensions"));
        }
    }
    let width = width as u32;
    let height = height as u32;
    if (width as u64) * (height as u64) > 64 * 1024 * 1024 {
        return Err(BmpError::Unsupported("too large"));
    }

    // Masks for 16/32-bit images.
    let mut masks = None;
    let mut after_header = hsize;
    match compression {
        BI_BITFIELDS | BI_ALPHABITFIELDS => {
            let (r, g, b) = if hsize >= 52 {
                (u32le(dib, 40), u32le(dib, 44), u32le(dib, 48))
            } else {
                after_header = hsize
                    + if compression == BI_ALPHABITFIELDS {
                        16
                    } else {
                        12
                    };
                (
                    u32le(dib, hsize),
                    u32le(dib, hsize + 4),
                    u32le(dib, hsize + 8),
                )
            };
            let (r, g, b) = (
                r.ok_or(BmpError::Truncated)?,
                g.ok_or(BmpError::Truncated)?,
                b.ok_or(BmpError::Truncated)?,
            );
            masks = Some([Mask::new(r), Mask::new(g), Mask::new(b)]);
        }
        BI_RGB | BI_RLE8 | BI_RLE4 => {}
        _ => return Err(BmpError::Unsupported("compression")),
    }
    if masks.is_none() {
        masks = match bpp {
            16 => Some([Mask::new(0x7C00), Mask::new(0x03E0), Mask::new(0x001F)]),
            32 => Some([Mask::new(0xFF_0000), Mask::new(0xFF00), Mask::new(0xFF)]),
            _ => None,
        };
    }

    // Palette.
    let mut palette = Vec::new();
    if bpp <= 8 {
        let max = 1usize << bpp;
        let n = if clr_used == 0 || clr_used as usize > max {
            max
        } else {
            clr_used as usize
        };
        let entry = if core { 3 } else { 4 };
        for i in 0..n {
            let o = after_header + i * entry;
            match dib.get(o..o + 3) {
                Some(p) => palette.push(((p[2] as u32) << 16) | ((p[1] as u32) << 8) | p[0] as u32),
                None => break,
            }
        }
        palette.resize(max, 0);
        after_header += n * entry;
    }

    let off = match pixel_offset {
        Some(o) if o >= after_header && o < dib.len() => o,
        _ => after_header,
    };
    let pixels = dib.get(off..).unwrap_or(&[]);

    let mut out = Bitmap::new(width, height, 0);
    let row_y = |r: u32| -> i32 {
        if top_down {
            r as i32
        } else {
            (height - 1 - r) as i32
        }
    };

    match (compression, bpp) {
        (BI_RLE8, 8) => decode_rle(pixels, width, height, false, &palette, &mut out, top_down),
        (BI_RLE4, 4) => decode_rle(pixels, width, height, true, &palette, &mut out, top_down),
        (BI_RLE8 | BI_RLE4, _) => return Err(BmpError::Unsupported("RLE depth")),
        (_, 1 | 4 | 8) => {
            let stride = ((width * bpp).div_ceil(32) * 4) as usize;
            for r in 0..height {
                let row = r as usize * stride;
                let y = row_y(r);
                for x in 0..width {
                    let bit = x as usize * bpp as usize;
                    let Some(&byte) = pixels.get(row + bit / 8) else {
                        break;
                    };
                    let idx = (byte >> (8 - bpp as usize - bit % 8)) & ((1u16 << bpp) - 1) as u8;
                    out.set(x as i32, y, palette[idx as usize]);
                }
            }
        }
        (_, 24) => {
            let stride = (width * 3).div_ceil(4) as usize * 4;
            for r in 0..height {
                let y = row_y(r);
                for x in 0..width {
                    let o = r as usize * stride + x as usize * 3;
                    let Some(p) = pixels.get(o..o + 3) else { break };
                    out.set(
                        x as i32,
                        y,
                        ((p[2] as u32) << 16) | ((p[1] as u32) << 8) | p[0] as u32,
                    );
                }
            }
        }
        (_, 16 | 32) => {
            let m = masks.expect("masks set for 16/32 bpp");
            let bytes = (bpp / 8) as usize;
            let stride = (width as usize * bytes).div_ceil(4) * 4;
            for r in 0..height {
                let y = row_y(r);
                for x in 0..width {
                    let o = r as usize * stride + x as usize * bytes;
                    let v = if bytes == 2 {
                        match u16le(pixels, o) {
                            Some(v) => v as u32,
                            None => break,
                        }
                    } else {
                        match u32le(pixels, o) {
                            Some(v) => v,
                            None => break,
                        }
                    };
                    out.set(
                        x as i32,
                        y,
                        (m[0].get(v) << 16) | (m[1].get(v) << 8) | m[2].get(v),
                    );
                }
            }
        }
        _ => return Err(BmpError::Unsupported("bit depth")),
    }
    Ok(out)
}

fn decode_rle(
    data: &[u8],
    width: u32,
    height: u32,
    four: bool,
    palette: &[u32],
    out: &mut Bitmap,
    top_down: bool,
) {
    let (w, h) = (width as i32, height as i32);
    let put = |out: &mut Bitmap, x: i32, r: i32, idx: u8| {
        if x < w && r < h {
            let y = if top_down { r } else { h - 1 - r };
            out.set(x, y, palette[idx as usize]);
        }
    };
    // Pixels not written by the RLE stream take palette index 0.
    for y in 0..h {
        for x in 0..w {
            out.set(x, y, palette[0]);
        }
    }
    let (mut x, mut r, mut i) = (0i32, 0i32, 0usize);
    while i + 1 < data.len() && r < h {
        let (n, v) = (data[i], data[i + 1]);
        i += 2;
        if n > 0 {
            for k in 0..n as i32 {
                let idx = if four {
                    if k % 2 == 0 { v >> 4 } else { v & 0xF }
                } else {
                    v
                };
                put(out, x, r, idx);
                x += 1;
            }
            continue;
        }
        match v {
            0 => {
                x = 0;
                r += 1;
            }
            1 => break,
            2 => {
                let (Some(&dx), Some(&dy)) = (data.get(i), data.get(i + 1)) else {
                    break;
                };
                i += 2;
                x += dx as i32;
                r += dy as i32;
            }
            count => {
                let count = count as usize;
                let bytes = if four { count.div_ceil(2) } else { count };
                for k in 0..count {
                    let Some(&b) = data.get(i + if four { k / 2 } else { k }) else {
                        break;
                    };
                    let idx = if four {
                        if k % 2 == 0 { b >> 4 } else { b & 0xF }
                    } else {
                        b
                    };
                    put(out, x, r, idx);
                    x += 1;
                }
                i += bytes.div_ceil(2) * 2; // runs are word aligned
            }
        }
    }
}

/// Encodes a 24-bit bottom-up BMP.
pub fn encode_24(bmp: &Bitmap) -> Vec<u8> {
    let (w, h) = (bmp.width(), bmp.height());
    let stride = (w * 3).div_ceil(4) * 4;
    let size = 54 + stride * h;
    let mut v = Vec::with_capacity(size as usize);
    write_headers(&mut v, w, h as i32, 24, BI_RGB, size, 54, stride * h, 0);
    for r in (0..h).rev() {
        let start = v.len();
        for x in 0..w {
            let p = bmp.pixel(x as i32, r as i32);
            v.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8]);
        }
        v.resize(start + stride as usize, 0);
    }
    v
}

/// Encodes an 8-bit palettised BMP, optionally RLE8-compressed.
///
/// Returns `None` when the image has more than 256 colours.
pub fn encode_8(bmp: &Bitmap, rle: bool) -> Option<Vec<u8>> {
    let mut palette: Vec<u32> = Vec::new();
    let mut idx = Vec::with_capacity(bmp.pixels().len());
    for &p in bmp.pixels() {
        let i = match palette.iter().position(|&c| c == p) {
            Some(i) => i,
            None => {
                if palette.len() == 256 {
                    return None;
                }
                palette.push(p);
                palette.len() - 1
            }
        };
        idx.push(i as u8);
    }
    let (w, h) = (bmp.width(), bmp.height());
    let mut body = Vec::new();
    for r in (0..h).rev() {
        let row = &idx[(r * w) as usize..((r + 1) * w) as usize];
        if rle {
            let mut i = 0;
            while i < row.len() {
                let mut n = 1;
                while i + n < row.len() && n < 255 && row[i + n] == row[i] {
                    n += 1;
                }
                body.extend_from_slice(&[n as u8, row[i]]);
                i += n;
            }
            body.extend_from_slice(&[0, 0]);
        } else {
            body.extend_from_slice(row);
            body.resize(body.len().div_ceil(4) * 4, 0);
        }
    }
    if rle {
        body.extend_from_slice(&[0, 1]);
    }
    let pal_bytes = 256 * 4;
    let off = 54 + pal_bytes;
    let mut v = Vec::new();
    let comp = if rle { BI_RLE8 } else { BI_RGB };
    write_headers(
        &mut v,
        w,
        h as i32,
        8,
        comp,
        off + body.len() as u32,
        off,
        body.len() as u32,
        256,
    );
    for i in 0..256 {
        let p = palette.get(i).copied().unwrap_or(0);
        v.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8, 0]);
    }
    v.extend_from_slice(&body);
    Some(v)
}

fn write_headers(
    v: &mut Vec<u8>,
    w: u32,
    h: i32,
    bpp: u16,
    comp: u32,
    file_size: u32,
    off: u32,
    image_size: u32,
    colors: u32,
) {
    v.extend_from_slice(b"BM");
    v.extend_from_slice(&file_size.to_le_bytes());
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&off.to_le_bytes());
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&(w as i32).to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&bpp.to_le_bytes());
    v.extend_from_slice(&comp.to_le_bytes());
    v.extend_from_slice(&image_size.to_le_bytes());
    v.extend_from_slice(&2835u32.to_le_bytes());
    v.extend_from_slice(&2835u32.to_le_bytes());
    v.extend_from_slice(&colors.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Bitmap {
        let mut b = Bitmap::new(7, 5, 0x102030);
        b.set(0, 0, 0xFF0000);
        b.set(6, 4, 0x00FF00);
        b.set(3, 2, 0x0000FF);
        b
    }

    #[test]
    fn round_trip_24() {
        let b = sample();
        assert_eq!(decode(&encode_24(&b)).unwrap(), b);
    }

    #[test]
    fn round_trip_8_and_rle8() {
        let b = sample();
        assert_eq!(decode(&encode_8(&b, false).unwrap()).unwrap(), b);
        assert_eq!(decode(&encode_8(&b, true).unwrap()).unwrap(), b);
    }

    #[test]
    fn top_down_32_bitfields() {
        // 2x2, top-down, 32-bit with BITFIELDS in a 40-byte header.
        let mut v = Vec::new();
        let pixels: [u32; 4] = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0x00FF_FFFF];
        write_headers(&mut v, 2, -2, 32, BI_BITFIELDS, 0, 54 + 12, 16, 0);
        v.extend_from_slice(&0x00FF_0000u32.to_le_bytes());
        v.extend_from_slice(&0x0000_FF00u32.to_le_bytes());
        v.extend_from_slice(&0x0000_00FFu32.to_le_bytes());
        for p in pixels {
            v.extend_from_slice(&p.to_le_bytes());
        }
        let b = decode(&v).unwrap();
        assert_eq!(b.pixels(), &pixels);
    }

    #[test]
    fn sixteen_bit_565() {
        let mut v = Vec::new();
        write_headers(&mut v, 1, 1, 16, BI_BITFIELDS, 0, 54 + 12, 4, 0);
        for m in [0xF800u32, 0x07E0, 0x001F] {
            v.extend_from_slice(&m.to_le_bytes());
        }
        v.extend_from_slice(&[0x1F, 0xF8, 0, 0]); // pure red + full blue... 0xF81F
        let b = decode(&v).unwrap();
        assert_eq!(b.pixel(0, 0), 0xFF00FF);
    }

    #[test]
    fn one_and_four_bit() {
        // 1-bit 3x1: pixels 1,0,1.
        let mut v = Vec::new();
        write_headers(&mut v, 3, 1, 1, BI_RGB, 0, 54 + 8, 4, 2);
        v.extend_from_slice(&[0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0]);
        v.extend_from_slice(&[0b1010_0000, 0, 0, 0]);
        let b = decode(&v).unwrap();
        assert_eq!(b.pixels(), &[0xFFFFFF, 0, 0xFFFFFF]);

        // RLE4: 4 pixels alternating index 1 and 2, then end.
        let mut v = Vec::new();
        write_headers(&mut v, 4, 1, 4, BI_RLE4, 0, 54 + 12, 4, 3);
        v.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0xFF, 0, 0xFF, 0, 0, 0]);
        v.extend_from_slice(&[4, 0x12, 0, 1]);
        let b = decode(&v).unwrap();
        assert_eq!(b.pixels(), &[0xFF0000, 0x0000FF, 0xFF0000, 0x0000FF]);
    }

    #[test]
    fn truncated_pixels_are_black() {
        let b = Bitmap::new(4, 4, 0xFFFFFF);
        let mut data = encode_24(&b);
        data.truncate(54 + 16); // keep only the bottom row
        let d = decode(&data).unwrap();
        assert_eq!(d.pixel(0, 3), 0xFFFFFF);
        assert_eq!(d.pixel(0, 0), 0);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(decode(b"PNG"), Err(BmpError::NotBmp));
        assert!(decode(b"BM\0\0").is_err());
    }
}
