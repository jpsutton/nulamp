//! Blitting and scaling primitives. Frames are `Bitmap`s of `0x00RRGGBB` pixels.

pub use nulamp_skin::Bitmap as Frame;

/// Copies `w`×`h` pixels from `src` at (sx, sy) to `dst` at (dx, dy).
///
/// Source pixels outside `src` are black (01 §2.1). Destination pixels outside `dst` are clipped.
pub fn blit(dst: &mut Frame, src: &Frame, sx: i32, sy: i32, w: i32, h: i32, dx: i32, dy: i32) {
    for y in 0..h {
        let ty = dy + y;
        if ty < 0 || ty >= dst.height() as i32 {
            continue;
        }
        for x in 0..w {
            let tx = dx + x;
            if tx < 0 || tx >= dst.width() as i32 {
                continue;
            }
            dst.set(tx, ty, src.pixel(sx + x, sy + y));
        }
    }
}

/// Stretches a `sw`×`sh` source rectangle to `dw`×`dh` with nearest-neighbour sampling.
pub fn blit_stretch(
    dst: &mut Frame,
    src: &Frame,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
) {
    if sw <= 0 || sh <= 0 {
        return;
    }
    for y in 0..dh {
        let syy = sy + y * sh / dh.max(1);
        for x in 0..dw {
            let sxx = sx + x * sw / dw.max(1);
            dst.set(dx + x, dy + y, src.pixel(sxx, syy));
        }
    }
}

/// Integer nearest-neighbour upscaling (double size, 02 §9).
pub fn scale(src: &Frame, factor: u32) -> Frame {
    if factor == 1 {
        return src.clone();
    }
    let (w, h) = (src.width() * factor, src.height() * factor);
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            px.push(src.pixel((x / factor) as i32, (y / factor) as i32));
        }
    }
    Frame::from_pixels(w, h, px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blit_out_of_bounds_is_black_and_clipped() {
        let src = Frame::new(2, 2, 0xFFFFFF);
        let mut dst = Frame::new(4, 4, 0x123456);
        blit(&mut dst, &src, 1, 1, 3, 3, 2, 2);
        assert_eq!(dst.pixel(2, 2), 0xFFFFFF);
        assert_eq!(dst.pixel(3, 3), 0);
        assert_eq!(dst.pixel(1, 1), 0x123456);
    }

    #[test]
    fn scale_doubles_pixels() {
        let mut src = Frame::new(2, 1, 0);
        src.set(1, 0, 0xFF);
        let d = scale(&src, 2);
        assert_eq!((d.width(), d.height()), (4, 2));
        assert_eq!(d.pixels(), &[0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF]);
    }

    #[test]
    fn stretch() {
        let mut src = Frame::new(2, 1, 0);
        src.set(1, 0, 7);
        let mut dst = Frame::new(4, 1, 9);
        blit_stretch(&mut dst, &src, 0, 0, 2, 1, 0, 0, 4, 1);
        assert_eq!(dst.pixels(), &[0, 0, 7, 7]);
    }
}
