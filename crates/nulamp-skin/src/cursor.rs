//! Windows `.cur` cursor files (01 §7). Animated cursor data (RIFF `ACON`) yields its first frame.

use crate::bmp;

/// A decoded cursor image. Pixels are `0xAARRGGBB`, top-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub hotspot: (u32, u32),
    pub argb: Vec<u32>,
}

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32le(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Decodes a `.cur` (or `.ico`) file, or the first frame of an `ACON` file.
pub fn decode(data: &[u8]) -> Option<CursorImage> {
    if data.get(0..4) == Some(b"RIFF") && data.get(8..12) == Some(b"ACON") {
        return first_acon_frame(data).and_then(decode);
    }
    let kind = u16le(data, 2)?;
    let count = u16le(data, 4)?;
    if u16le(data, 0)? != 0 || !(kind == 1 || kind == 2) || count == 0 {
        return None;
    }
    // Pick the largest entry.
    let mut best: Option<(u32, usize)> = None;
    for i in 0..count as usize {
        let e = 6 + i * 16;
        let w = *data.get(e)? as u32;
        let w = if w == 0 { 256 } else { w };
        if best.is_none_or(|(bw, _)| w > bw) {
            best = Some((w, e));
        }
    }
    let (_, e) = best?;
    let (hx, hy) = (u16le(data, e + 4)? as u32, u16le(data, e + 6)? as u32);
    let size = u32le(data, e + 8)? as usize;
    let off = u32le(data, e + 12)? as usize;
    let img = data.get(off..off.checked_add(size)?.min(data.len()))?;
    if img.starts_with(b"\x89PNG") {
        return None; // PNG cursor images are not used by classic skins
    }
    let color = bmp::decode_dib(img, None, true).ok()?;
    let (w, h) = (color.width(), color.height());
    let bpp = u16le(img, 14)? as u32;
    let hsize = u32le(img, 0)? as usize;
    let palette = if bpp <= 8 { (1usize << bpp) * 4 } else { 0 };
    let xor_stride = ((w * bpp).div_ceil(32) * 4) as usize;
    let and_off = hsize + palette + xor_stride * h as usize;
    let and_stride = w.div_ceil(32) as usize * 4;

    // 32-bit images may carry their own alpha.
    let mut own_alpha = false;
    let mut alpha = vec![0u8; (w * h) as usize];
    if bpp == 32 {
        for r in 0..h {
            for x in 0..w {
                let o = hsize + (r * w + x) as usize * 4 + 3;
                let a = *img.get(o).unwrap_or(&0);
                alpha[((h - 1 - r) * w + x) as usize] = a;
                own_alpha |= a != 0;
            }
        }
    }
    let mut argb = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let rgb = color.pixel(x as i32, y as i32);
            let a = if own_alpha {
                alpha[(y * w + x) as usize] as u32
            } else {
                let r = (h - 1 - y) as usize;
                let byte = img
                    .get(and_off + r * and_stride + x as usize / 8)
                    .copied()
                    .unwrap_or(0);
                let transparent = byte & (0x80 >> (x % 8)) != 0;
                if transparent { 0 } else { 255 }
            };
            argb.push((a << 24) | rgb);
        }
    }
    Some(CursorImage {
        width: w,
        height: h,
        hotspot: (hx.min(w - 1), hy.min(h - 1)),
        argb,
    })
}

fn first_acon_frame(data: &[u8]) -> Option<&[u8]> {
    // Walk chunks inside the RIFF form looking for LIST 'fram' → first 'icon'.
    fn walk(b: &[u8]) -> Option<&[u8]> {
        let mut i = 0;
        while i + 8 <= b.len() {
            let id = &b[i..i + 4];
            let len = u32le(b, i + 4)? as usize;
            let body = b.get(i + 8..(i + 8).saturating_add(len).min(b.len()))?;
            if id == b"icon" {
                return Some(body);
            }
            if id == b"LIST"
                && body.len() >= 4
                && let Some(f) = walk(&body[4..])
            {
                return Some(f);
            }
            i += 8 + len + (len & 1);
        }
        None
    }
    walk(data.get(12..)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a 2×2 24-bit `.cur` with a hotspot; pixel (1,0) is masked out.
    pub(crate) fn sample_cur() -> Vec<u8> {
        let mut img = Vec::new();
        img.extend_from_slice(&40u32.to_le_bytes());
        img.extend_from_slice(&2i32.to_le_bytes());
        img.extend_from_slice(&4i32.to_le_bytes());
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&24u16.to_le_bytes());
        img.extend_from_slice(&[0; 24]);
        // XOR rows, bottom-up: bottom row red,red; top row green,blue.
        img.extend_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0]);
        img.extend_from_slice(&[0, 255, 0, 255, 0, 0, 0, 0]);
        // AND rows, bottom-up, 4-byte stride: top row has pixel x=1 transparent.
        img.extend_from_slice(&[0, 0, 0, 0]);
        img.extend_from_slice(&[0b0100_0000, 0, 0, 0]);
        let mut f = Vec::new();
        f.extend_from_slice(&[0, 0, 2, 0, 1, 0]);
        f.extend_from_slice(&[2, 2, 0, 0]);
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&(img.len() as u32).to_le_bytes());
        f.extend_from_slice(&22u32.to_le_bytes());
        f.extend_from_slice(&img);
        f
    }

    #[test]
    fn decodes_cur() {
        let c = decode(&sample_cur()).unwrap();
        assert_eq!((c.width, c.height, c.hotspot), (2, 2, (1, 1)));
        assert_eq!(c.argb, vec![0xFF00FF00, 0x000000FF, 0xFFFF0000, 0xFFFF0000]);
    }

    #[test]
    fn decodes_acon_first_frame() {
        let cur = sample_cur();
        let mut list = b"fram".to_vec();
        list.extend_from_slice(b"icon");
        list.extend_from_slice(&(cur.len() as u32).to_le_bytes());
        list.extend_from_slice(&cur);
        let mut riff = b"RIFF\0\0\0\0ACON".to_vec();
        riff.extend_from_slice(b"anih");
        riff.extend_from_slice(&4u32.to_le_bytes());
        riff.extend_from_slice(&[0; 4]);
        riff.extend_from_slice(b"LIST");
        riff.extend_from_slice(&(list.len() as u32).to_le_bytes());
        riff.extend_from_slice(&list);
        assert_eq!(decode(&riff), decode(&cur));
    }
}
