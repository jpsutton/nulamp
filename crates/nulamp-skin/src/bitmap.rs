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

    /// Fills a rectangle, clipped to the bitmap.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, rgb);
            }
        }
    }
}
