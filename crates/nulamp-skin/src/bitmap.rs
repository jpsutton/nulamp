//! In-memory RGB bitmaps.

/// A decoded, opaque bitmap. Pixels are `0x00RRGGBB`, stored row by row from the top.
#[derive(Clone, PartialEq, Eq)]
pub struct Bitmap {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bitmap({}x{})", self.width, self.height)
    }
}

impl Bitmap {
    /// Creates a bitmap filled with one colour.
    pub fn new(width: u32, height: u32, fill: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![fill & 0xFF_FFFF; (width * height) as usize],
        }
    }

    /// Creates a bitmap from top-down rows of `0x00RRGGBB` pixels.
    ///
    /// # Panics
    /// Panics when `pixels.len() != width * height`.
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u32>) -> Self {
        assert_eq!(
            pixels.len(),
            (width * height) as usize,
            "pixel count mismatch"
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Pixel at (x, y), or `None` outside the bitmap.
    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        Some(self.pixels[(y as u32 * self.width + x as u32) as usize])
    }

    /// Pixel at (x, y). Pixels outside the bitmap are black (01 §2.1).
    pub fn pixel(&self, x: i32, y: i32) -> u32 {
        self.get(x, y).unwrap_or(0)
    }

    /// Sets a pixel. Out-of-range coordinates are ignored.
    pub fn set(&mut self, x: i32, y: i32, rgb: u32) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        self.pixels[(y as u32 * self.width + x as u32) as usize] = rgb & 0xFF_FFFF;
    }

    /// Copies `w`×`h` top-down rows of `0x00RRGGBB` pixels to (x, y), clipped to the bitmap.
    ///
    /// # Panics
    /// Panics when `src` holds fewer than `w * h` pixels.
    pub fn put_pixels(&mut self, x: i32, y: i32, w: u32, h: u32, src: &[u32]) {
        assert!(src.len() >= w as usize * h as usize, "pixel count mismatch");
        let x0 = x.max(0);
        let x1 = (x + w as i32).min(self.width as i32);
        if x1 <= x0 {
            return;
        }
        for row in 0..h as i32 {
            let ty = y + row;
            if ty < 0 || ty >= self.height as i32 {
                continue;
            }
            let s = row as usize * w as usize + (x0 - x) as usize;
            let d = (ty as u32 * self.width + x0 as u32) as usize;
            let n = (x1 - x0) as usize;
            self.pixels[d..d + n].copy_from_slice(&src[s..s + n]);
        }
    }

    /// Fills a rectangle, clipped to the bitmap.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, rgb);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_pixels_clips() {
        let mut b = Bitmap::new(4, 3, 0);
        b.put_pixels(-1, 1, 3, 3, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(b.pixels(), &[0, 0, 0, 0, 2, 3, 0, 0, 5, 6, 0, 0]);
        b.put_pixels(3, 0, 2, 1, &[7, 8]);
        assert_eq!(b.pixel(3, 0), 7);
        b.put_pixels(9, 9, 1, 1, &[1]);
    }
}
