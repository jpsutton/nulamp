//! nulamp's application icon: an "N" built from spectrum-analyzer bars on an LCD tile.
//!
//! The artwork is original (docs/decisions.md ICON1). Small sizes are pixel art drawn for
//! each size, because the bars do not survive scaling: 16, 22, 24, 32 and 48 px have their
//! own masters, and 64 px is the 32 px master doubled. Larger sizes are rendered from the
//! vector design in [`SVG`], with antialiased edges and a soft glow behind the bars.
//!
//! [`icon`] returns any size; `tools/icongen` writes the packaged files from it.

/// An RGBA image, 8 bits per channel, straight (not premultiplied) alpha, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Sizes with a hand-drawn pixel master.
pub const PIXEL_SIZES: [u32; 5] = [16, 22, 24, 32, 48];

type Rgb = [u8; 3];

/// Display black, as in the built-in skin.
const DISPLAY: Rgb = [0x0a, 0x0d, 0x10];
/// The LCD's dot grid.
const DOT: Rgb = [0x17, 0x1c, 0x24];
/// The tile's darker lip on the lower and right edges.
const LIP: Rgb = [0x25, 0x2a, 0x33];
/// The tile's outer rim.
const RIM: Rgb = [0x3b, 0x41, 0x4d];
/// Peak caps above the bars.
const CAP: Rgb = [0x6c, 0x74, 0x87];
/// Lit bars.
const BAR: Rgb = [0x4b, 0xe3, 0x5d];
/// The lit top edge of each bar (vector sizes only).
const BAR_TOP: Rgb = [0xa6, 0xf5, 0xb0];

/// The icon at `size`×`size` pixels.
///
/// Sizes in [`PIXEL_SIZES`] are pixel art and 64 is the 32 px master doubled; every other
/// size is rendered from the vector design, which is meant for 128 px and up.
pub fn icon(size: u32) -> Image {
    assert!(size > 0, "icon size must be positive");
    match size {
        16 | 22 | 24 | 32 | 48 => pixel(size),
        64 => upscale(&pixel(32), 2),
        _ => vector(size),
    }
}

// ---------------------------------------------------------------------------------------
// Pixel masters

/// A square of optional colours; `None` is transparent.
struct Grid {
    n: i32,
    px: Vec<Option<Rgb>>,
}

impl Grid {
    fn new(n: u32) -> Self {
        Self {
            n: n as i32,
            px: vec![None; (n * n) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, c: Rgb) {
        if (0..self.n).contains(&x) && (0..self.n).contains(&y) {
            self.px[(y * self.n + x) as usize] = Some(c);
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        for j in y..y + h {
            for i in x..x + w {
                self.set(i, j, c);
            }
        }
    }

    /// Rounded tile: rim, a dark lip on the lower and right edges, the display inside,
    /// optionally with a dot on every other pixel.
    fn tile(&mut self, r: i32, dots: bool) {
        let n = self.n;
        let inside = |x: i32, y: i32| {
            let dx = 0.max(r - x).max(x - (n - 1 - r));
            let dy = 0.max(r - y).max(y - (n - 1 - r));
            dx + dy < r + 1
        };
        let edge = |x: i32, y: i32| {
            inside(x, y)
                && !(inside(x - 1, y) && inside(x + 1, y) && inside(x, y - 1) && inside(x, y + 1))
        };
        for y in 0..n {
            for x in 0..n {
                if !inside(x, y) {
                    continue;
                }
                let c = if edge(x, y) {
                    RIM
                } else if edge(x - 1, y) || edge(x, y - 1) {
                    DISPLAY
                } else if edge(x + 1, y) || edge(x, y + 1) {
                    LIP
                } else if dots && x % 2 == 0 && y % 2 == 0 {
                    DOT
                } else {
                    DISPLAY
                };
                self.set(x, y, c);
            }
        }
    }

    fn into_image(self) -> Image {
        let mut rgba = Vec::with_capacity(self.px.len() * 4);
        for p in self.px {
            match p {
                Some([r, g, b]) => rgba.extend_from_slice(&[r, g, b, 255]),
                None => rgba.extend_from_slice(&[0; 4]),
            }
        }
        Image {
            width: self.n as u32,
            height: self.n as u32,
            rgba,
        }
    }
}

/// The analyzer-bar N: two full-height posts and a diagonal of shorter bars, each with a
/// peak cap above it. Bars are `(x, top, height)`.
struct Bars {
    post_w: i32,
    mid_w: i32,
    cap_h: i32,
    cap_gap: i32,
    posts: &'static [(i32, i32, i32)],
    mids: &'static [(i32, i32, i32)],
}

const BARS_24: Bars = Bars {
    post_w: 2,
    mid_w: 2,
    cap_h: 1,
    cap_gap: 1,
    posts: &[(5, 6, 14), (17, 6, 14)],
    mids: &[(8, 7, 6), (11, 10, 6), (14, 13, 6)],
};

const BARS_32: Bars = Bars {
    post_w: 2,
    mid_w: 2,
    cap_h: 1,
    cap_gap: 1,
    posts: &[(6, 6, 20), (24, 6, 20)],
    mids: &[
        (9, 7, 6),
        (12, 10, 6),
        (15, 13, 6),
        (18, 16, 6),
        (21, 19, 6),
    ],
};

const BARS_48: Bars = Bars {
    post_w: 3,
    mid_w: 3,
    cap_h: 2,
    cap_gap: 1,
    posts: &[(10, 11, 29), (35, 11, 29)],
    mids: &[(15, 12, 9), (20, 18, 9), (25, 24, 9), (30, 30, 9)],
};

impl Bars {
    fn draw(&self, g: &mut Grid, dx: i32, dy: i32) {
        let bars = self.posts.iter().map(|b| (b, self.post_w));
        for (&(x, top, h), w) in bars.chain(self.mids.iter().map(|b| (b, self.mid_w))) {
            g.rect(x + dx, top + dy, w, h, BAR);
            let cap_y = top - self.cap_gap - self.cap_h;
            g.rect(x + dx, cap_y + dy, w, self.cap_h, CAP);
        }
    }
}

fn pixel(size: u32) -> Image {
    let mut g = Grid::new(size);
    match size {
        16 => {
            // 1 px posts centred on the display (columns 1–13: the lip takes column 14) and a
            // diagonal on every other column; caps and dots would only blur it at this size.
            g.tile(2, false);
            g.rect(2, 2, 1, 11, BAR);
            g.rect(12, 2, 1, 11, BAR);
            for x in [4, 6, 8, 10] {
                g.rect(x, x - 1, 1, 3, BAR);
            }
        }
        22 => {
            g.tile(3, true);
            BARS_24.draw(&mut g, -1, -1);
        }
        24 => {
            g.tile(3, true);
            BARS_24.draw(&mut g, 0, 0);
        }
        32 => {
            g.tile(3, true);
            BARS_32.draw(&mut g, 0, 0);
        }
        48 => {
            g.tile(4, true);
            BARS_48.draw(&mut g, 0, 0);
        }
        _ => unreachable!("no pixel master at {size} px"),
    }
    g.into_image()
}

/// Nearest-neighbour enlargement by an integer factor.
fn upscale(src: &Image, k: u32) -> Image {
    let (w, h) = (src.width * k, src.height * k);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = (((y / k) * src.width + x / k) * 4) as usize;
            rgba.extend_from_slice(&src.rgba[i..i + 4]);
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

// ---------------------------------------------------------------------------------------
// Vector design

/// The vector design, in the same 32-unit coordinates as the 32 px master.
pub const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="512" height="512">
<title>nulamp</title>
<defs>
<pattern id="lcd" width="2" height="2" patternUnits="userSpaceOnUse"><rect x="0.2" y="0.2" width="0.6" height="0.6" fill="#171c24"/></pattern>
<filter id="bloom" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="0.9"/></filter>
</defs>
<rect width="32" height="32" rx="3.6" fill="#3b414d"/>
<rect x="1" y="1" width="30" height="30" rx="2.6" fill="#252a33"/>
<rect x="1" y="1" width="29" height="29" rx="2.4" fill="#0a0d10"/>
<rect x="2" y="2" width="28" height="28" rx="0.5" fill="url(#lcd)"/>
<g fill="#4be35d" opacity="0.35" filter="url(#bloom)">
<rect x="6" y="6" width="2" height="20"/><rect x="24" y="6" width="2" height="20"/>
<rect x="9" y="7" width="2" height="6"/><rect x="12" y="10" width="2" height="6"/><rect x="15" y="13" width="2" height="6"/><rect x="18" y="16" width="2" height="6"/><rect x="21" y="19" width="2" height="6"/>
</g>
<g fill="#4be35d">
<rect x="6" y="6" width="2" height="20" rx="0.15"/><rect x="24" y="6" width="2" height="20" rx="0.15"/>
<rect x="9" y="7" width="2" height="6" rx="0.15"/><rect x="12" y="10" width="2" height="6" rx="0.15"/><rect x="15" y="13" width="2" height="6" rx="0.15"/><rect x="18" y="16" width="2" height="6" rx="0.15"/><rect x="21" y="19" width="2" height="6" rx="0.15"/>
</g>
<g fill="#a6f5b0">
<rect x="6" y="6" width="2" height="0.35"/><rect x="24" y="6" width="2" height="0.35"/>
<rect x="9" y="7" width="2" height="0.35"/><rect x="12" y="10" width="2" height="0.35"/><rect x="15" y="13" width="2" height="0.35"/><rect x="18" y="16" width="2" height="0.35"/><rect x="21" y="19" width="2" height="0.35"/>
</g>
<g fill="#6c7487">
<rect x="6" y="4" width="2" height="1" rx="0.15"/><rect x="24" y="4" width="2" height="1" rx="0.15"/>
<rect x="9" y="5" width="2" height="1" rx="0.15"/><rect x="12" y="8" width="2" height="1" rx="0.15"/><rect x="15" y="11" width="2" height="1" rx="0.15"/><rect x="18" y="14" width="2" height="1" rx="0.15"/><rect x="21" y="17" width="2" height="1" rx="0.15"/>
</g>
</svg>
"##;

/// A rounded rectangle in design units.
#[derive(Clone, Copy)]
struct RRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
}

impl RRect {
    const fn new(x: f32, y: f32, w: f32, h: f32, r: f32) -> Self {
        Self { x, y, w, h, r }
    }

    fn contains(&self, px: f32, py: f32) -> bool {
        if px < self.x || py < self.y || px >= self.x + self.w || py >= self.y + self.h {
            return false;
        }
        let dx = (self.x + self.r - px)
            .max(px - (self.x + self.w - self.r))
            .max(0.0);
        let dy = (self.y + self.r - py)
            .max(py - (self.y + self.h - self.r))
            .max(0.0);
        dx * dx + dy * dy <= self.r * self.r
    }
}

/// The bars of the 32 px master, `(x, top, height)`, all 2 units wide.
const VECTOR_BARS: [(f32, f32, f32); 7] = [
    (6.0, 6.0, 20.0),
    (24.0, 6.0, 20.0),
    (9.0, 7.0, 6.0),
    (12.0, 10.0, 6.0),
    (15.0, 13.0, 6.0),
    (18.0, 16.0, 6.0),
    (21.0, 19.0, 6.0),
];

/// Samples per pixel along each axis.
const SS: usize = 4;

/// Premultiplied RGBA in 0..1.
struct Canvas {
    n: usize,
    scale: f32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    /// Fraction of pixel `(x, y)` covered by `inside` (a test in design units).
    fn coverage(&self, x: usize, y: usize, inside: impl Fn(f32, f32) -> bool) -> f32 {
        let mut hits = 0;
        for j in 0..SS {
            for i in 0..SS {
                let u = (x as f32 + (i as f32 + 0.5) / SS as f32) / self.scale;
                let v = (y as f32 + (j as f32 + 0.5) / SS as f32) / self.scale;
                hits += inside(u, v) as u32;
            }
        }
        hits as f32 / (SS * SS) as f32
    }

    /// Pixel range covering design-unit span `a..b`, clamped to the canvas.
    fn span(&self, a: f32, b: f32) -> std::ops::Range<usize> {
        let lo = (a * self.scale).floor().max(0.0) as usize;
        let hi = ((b * self.scale).ceil() as usize).min(self.n);
        lo..hi
    }

    fn blend(&mut self, x: usize, y: usize, c: Rgb, a: f32) {
        if a <= 0.0 {
            return;
        }
        let p = &mut self.px[y * self.n + x];
        for k in 0..3 {
            p[k] = c[k] as f32 / 255.0 * a + p[k] * (1.0 - a);
        }
        p[3] = a + p[3] * (1.0 - a);
    }

    /// Paints `inside` within the design-unit box `bounds`.
    fn fill(&mut self, bounds: RRect, c: Rgb, inside: impl Fn(f32, f32) -> bool) {
        for y in self.span(bounds.y, bounds.y + bounds.h) {
            for x in self.span(bounds.x, bounds.x + bounds.w) {
                let a = self.coverage(x, y, &inside);
                self.blend(x, y, c, a);
            }
        }
    }

    fn fill_rrect(&mut self, r: RRect, c: Rgb) {
        self.fill(r, c, |u, v| r.contains(u, v));
    }
}

fn vector(size: u32) -> Image {
    let n = size as usize;
    let mut cv = Canvas {
        n,
        scale: size as f32 / 32.0,
        px: vec![[0.0; 4]; n * n],
    };
    cv.fill_rrect(RRect::new(0.0, 0.0, 32.0, 32.0, 3.6), RIM);
    cv.fill_rrect(RRect::new(1.0, 1.0, 30.0, 30.0, 2.6), LIP);
    cv.fill_rrect(RRect::new(1.0, 1.0, 29.0, 29.0, 2.4), DISPLAY);
    // The corner radius is small enough that no dot is cut by it.
    let screen = RRect::new(2.0, 2.0, 28.0, 28.0, 0.5);
    cv.fill(screen, DOT, |u, v| {
        let (fu, fv) = (u.rem_euclid(2.0), v.rem_euclid(2.0));
        (0.2..0.8).contains(&fu) && (0.2..0.8).contains(&fv) && screen.contains(u, v)
    });

    // Glow: the bars' coverage, blurred, at 35 %.
    let mut mask = vec![0.0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            mask[y * n + x] = cv.coverage(x, y, |u, v| {
                VECTOR_BARS
                    .iter()
                    .any(|&(bx, by, bh)| (bx..bx + 2.0).contains(&u) && (by..by + bh).contains(&v))
            });
        }
    }
    gaussian_blur(&mut mask, n, 0.9 * cv.scale);
    for y in 0..n {
        for x in 0..n {
            cv.blend(x, y, BAR, 0.35 * mask[y * n + x]);
        }
    }

    for &(x, y, h) in &VECTOR_BARS {
        cv.fill_rrect(RRect::new(x, y, 2.0, h, 0.15), BAR);
        cv.fill_rrect(RRect::new(x, y, 2.0, 0.35, 0.0), BAR_TOP);
        cv.fill_rrect(RRect::new(x, y - 2.0, 2.0, 1.0, 0.15), CAP);
    }

    let mut rgba = Vec::with_capacity(n * n * 4);
    for p in &cv.px {
        let a = p[3];
        let un = |c: f32| {
            if a > 0.0 {
                (c / a * 255.0).round().clamp(0.0, 255.0) as u8
            } else {
                0
            }
        };
        rgba.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round() as u8]);
    }
    Image {
        width: size,
        height: size,
        rgba,
    }
}

/// Approximates a Gaussian blur with three box blurs per axis.
fn gaussian_blur(buf: &mut [f32], n: usize, sigma: f32) {
    // Three boxes of width w have variance 3 (w² − 1) / 12.
    let r = (((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0).round() as usize;
    if r == 0 {
        return;
    }
    let mut tmp = vec![0.0f32; buf.len()];
    for _ in 0..3 {
        box_pass(buf, &mut tmp, n, r, 1, n);
        box_pass(&tmp, buf, n, r, n, 1);
    }
}

/// One box blur of radius `r` along lines whose elements are `step` apart and whose starts
/// are `stride` apart. Outside the image counts as zero.
fn box_pass(src: &[f32], dst: &mut [f32], n: usize, r: usize, step: usize, stride: usize) {
    let w = (2 * r + 1) as f32;
    for line in 0..n {
        let base = line * stride;
        let at = |i: usize| src[base + i * step];
        let mut sum: f32 = (0..=r.min(n - 1)).map(at).sum();
        for i in 0..n {
            dst[base + i * step] = sum / w;
            if i + r + 1 < n {
                sum += at(i + r + 1);
            }
            if i >= r {
                sum -= at(i - r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(img: &Image, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * img.width + x) * 4) as usize;
        img.rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn sizes() {
        for size in [16, 22, 24, 32, 48, 64, 128, 256] {
            let img = icon(size);
            assert_eq!((img.width, img.height), (size, size));
            assert_eq!(img.rgba.len(), (size * size * 4) as usize);
        }
    }

    #[test]
    fn pixel_masters_are_one_bit() {
        // Every pixel is either fully transparent or fully opaque: no antialiasing.
        for size in PIXEL_SIZES.into_iter().chain([64]) {
            let img = icon(size);
            assert!(
                img.rgba.chunks(4).all(|p| p[3] == 0 || p[3] == 255),
                "{size} px"
            );
        }
    }

    #[test]
    fn sixteen_px_posts_and_diagonal() {
        let img = icon(16);
        let bar = [BAR[0], BAR[1], BAR[2], 255];
        for y in 2..13 {
            assert_eq!(at(&img, 2, y), bar);
            assert_eq!(at(&img, 12, y), bar);
        }
        // The diagonal sits on even columns with display between.
        assert_eq!(at(&img, 4, 3), bar);
        assert_eq!(at(&img, 10, 11), bar);
        assert_ne!(at(&img, 5, 4), bar);
    }

    #[test]
    fn corners_are_transparent() {
        for size in [16, 32, 48, 256] {
            assert_eq!(at(&icon(size), 0, 0)[3], 0, "{size} px");
        }
    }

    #[test]
    fn doubled_master() {
        let (a, b) = (icon(32), icon(64));
        for (x, y) in [(6, 10), (12, 12), (0, 0), (31, 31)] {
            assert_eq!(at(&a, x, y), at(&b, 2 * x + 1, 2 * y));
        }
    }

    #[test]
    fn vector_has_lit_bars() {
        let img = icon(256);
        // Middle of the left post (design 7, 16), well inside the bar.
        let p = at(&img, 56, 128);
        assert_eq!(p, [BAR[0], BAR[1], BAR[2], 255]);
        // Between bars the display shows, lifted a little by the glow.
        let q = at(&img, 70, 128);
        assert!(q[1] > DISPLAY[1] && q[1] < BAR[1], "{q:?}");
    }
}
