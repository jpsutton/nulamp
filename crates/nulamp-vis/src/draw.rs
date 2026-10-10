//! Drawing the analyzer and oscilloscope at every size (06 §4, §5).
//!
//! Levels count up from the bottom. Every drawing call paints the whole surface: background
//! first, then the content of the current mode.

use nulamp_skin::Bitmap;

use crate::BARS;
use crate::color::{BACKGROUND, GRID, PEAK, SCOPE_CENTRE};
use crate::settings::{AnalyzerStyle, ScopeStyle, VisMode, VisSettings};
use crate::state::{SHADE_COLUMNS, VisSize, Visualizer};

/// Width of the playlist mini area: the left part of the single-size image (06 §5.4).
const PLAYLIST_WIDTH: i32 = 72;

/// A clipped drawing area inside a target bitmap.
struct Canvas<'a> {
    bmp: &'a mut Bitmap,
    colors: &'a [u32; 24],
    ox: i32,
    oy: i32,
    w: i32,
    h: i32,
}

impl Canvas<'_> {
    fn put(&mut self, x: i32, y: i32, color: u8) {
        if (0..self.w).contains(&x) && (0..self.h).contains(&y) {
            self.bmp
                .set(self.ox + x, self.oy + y, self.colors[color as usize]);
        }
    }

    /// Plots a `px`×`px` cell at column `x` and `level` (from the bottom), where `base` is the
    /// row of level 0's lowest pixel.
    fn cell(&mut self, x: i32, level: i32, px: i32, base: i32, color: u8) {
        for dy in 0..px {
            for dx in 0..px {
                self.put(x * px + dx, base - level * px - dy, color);
            }
        }
    }

    fn background(&mut self, size: VisSize) {
        for y in 0..self.h {
            for x in 0..self.w {
                let grid = match size {
                    VisSize::Main => x % 2 == 0 && x < 75 && y % 2 == 1,
                    VisSize::MainDouble => x % 4 < 2 && x < 150 && y % 4 >= 2,
                    VisSize::Shade | VisSize::ShadeDouble => false,
                };
                self.put(x, y, if grid { GRID } else { BACKGROUND });
            }
        }
    }
}

/// Draws one empty frame (background and, for the main window, grid) at (`ox`, `oy`) in
/// `target`, as shown on stop (06 §2.4, §4.1, §5.1–§5.3).
pub fn draw_background(size: VisSize, colors: &[u32; 24], target: &mut Bitmap, ox: i32, oy: i32) {
    let (w, h) = size.dimensions();
    let mut cv = Canvas {
        bmp: target,
        colors,
        ox,
        oy,
        w: w as i32,
        h: h as i32,
    };
    cv.background(size);
}

/// Levels plotted for one oscilloscope column (06 §4.4 step 3), inclusive.
///
/// `centre` is the (low, high) centre pair: solid fills to `high` from above and to `low`
/// from below.
fn scope_run(style: ScopeStyle, prev: i32, v: i32, centre: (i32, i32)) -> (i32, i32) {
    match style {
        ScopeStyle::Dots => (v, v),
        ScopeStyle::Lines => (prev.min(v), prev.max(v)),
        ScopeStyle::Solid if v >= centre.1 => (centre.1, v),
        ScopeStyle::Solid => (v, centre.0),
    }
}

impl Visualizer {
    /// Draws the current frame for [`Visualizer::size`] at (`ox`, `oy`) in `target`, using
    /// the 24 `viscolor.txt` colours. Draws nothing when the mode is off (the skin's own
    /// pixels stay visible).
    pub fn draw(&self, colors: &[u32; 24], target: &mut Bitmap, ox: i32, oy: i32) {
        let (w, h) = self.size().dimensions();
        self.draw_clipped(self.size(), colors, target, ox, oy, w as i32, h as i32);
    }

    /// Draws the playlist mini area: the left 72×16 of the single-size main-window image
    /// (06 §5.4). Tick with [`VisSize::Main`] for this surface.
    pub fn draw_playlist(&self, colors: &[u32; 24], target: &mut Bitmap, ox: i32, oy: i32) {
        self.draw_clipped(VisSize::Main, colors, target, ox, oy, PLAYLIST_WIDTH, 16);
    }

    fn draw_clipped(
        &self,
        size: VisSize,
        colors: &[u32; 24],
        target: &mut Bitmap,
        ox: i32,
        oy: i32,
        w: i32,
        h: i32,
    ) {
        let s = self.settings();
        if s.mode == VisMode::Off {
            return;
        }
        let mut cv = Canvas {
            bmp: target,
            colors,
            ox,
            oy,
            w,
            h,
        };
        cv.background(size);
        match s.mode {
            VisMode::Spectrum => self.draw_spectrum(&mut cv, size, s),
            VisMode::Oscilloscope => self.draw_scope(&mut cv, size, s),
            VisMode::Off => {}
        }
    }

    fn draw_spectrum(&self, cv: &mut Canvas, size: VisSize, s: &VisSettings) {
        let flat = s.style == AnalyzerStyle::Line;
        match size {
            VisSize::Main | VisSize::MainDouble => {
                let (px, base) = if size == VisSize::Main {
                    (1, 17)
                } else {
                    (2, 31)
                };
                for x in 0..BARS {
                    if s.thick_bands && x % 4 == 3 {
                        continue;
                    }
                    let (v, c) = (self.shown.level[x] as i32, self.shown.colour[x] as i32);
                    for y in 0..v * px {
                        let col = if flat { c } else { c - y / px };
                        for dx in 0..px {
                            cv.put(x as i32 * px + dx, base - y, col as u8);
                        }
                    }
                    let p = self.shown.peak[x] as i32;
                    if s.peaks && (size == VisSize::Main || p > 0) {
                        cv.cell(x as i32, p, px, base, PEAK);
                    }
                }
            }
            VisSize::Shade | VisSize::ShadeDouble => {
                let (cols, step, rows) = if size == VisSize::Shade {
                    (SHADE_COLUMNS, 2, 5)
                } else {
                    (BARS, 1, 10)
                };
                for x in 0..cols {
                    if s.thick_bands && x % 4 == 3 {
                        continue;
                    }
                    let i = x * step;
                    let (v, c) = (self.shown.level[i] as i32, self.shown.colour[i] as i32);
                    let height = (v * rows / 15).min(rows);
                    for y in 0..height {
                        let col = if flat { c } else { c - y * 15 / rows };
                        cv.put(x as i32, rows - 1 - y, col as u8);
                    }
                    let p = self.shown.peak[i] as i32;
                    if s.peaks && p > 0 {
                        let row = (p * rows / 15).min(rows - 1);
                        cv.put(x as i32, rows - 1 - row, PEAK);
                    }
                }
            }
        }
    }

    fn draw_scope(&self, cv: &mut Canvas, size: VisSize, s: &VisSettings) {
        let (cols, px, base, max, centre) = match size {
            VisSize::Main => (BARS, 1, 17, 15, (7, 8)),
            VisSize::MainDouble => (BARS, 2, 31, 15, (7, 8)),
            VisSize::Shade => (38, 1, 4, 4, (2, 2)),
            VisSize::ShadeDouble => (BARS, 1, 9, 9, (5, 5)),
        };
        let level = |s: i8| -> i32 {
            let v = s as i32 + 8;
            match size {
                VisSize::Main | VisSize::MainDouble => v.clamp(0, 15),
                VisSize::Shade => (v * 5 / 16).clamp(0, max),
                VisSize::ShadeDouble => (v * 10 / 16).clamp(0, max),
            }
        };
        let mut prev = level(self.scope[0]);
        for x in 0..cols {
            let v = level(self.scope[x]);
            let color = match size {
                VisSize::Main | VisSize::MainDouble => {
                    SCOPE_CENTRE + (v / 2 - 4).unsigned_abs() as u8
                }
                VisSize::Shade | VisSize::ShadeDouble => SCOPE_CENTRE,
            };
            let (lo, hi) = scope_run(s.scope, prev, v, centre);
            for l in lo..=hi {
                cv.cell(x as i32, l, px, base, color);
            }
            prev = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::VisData;
    use crate::settings::VisMode;

    /// Palette where each colour's value is its index, so pixels read back as indices.
    const IDX: [u32; 24] = {
        let mut p = [0; 24];
        let mut i = 0;
        while i < 24 {
            p[i] = i as u32;
            i += 1;
        }
        p
    };
    const NONE: u32 = 0xFF_FFFF;

    fn render(v: &Visualizer) -> Bitmap {
        let (w, h) = v.size().dimensions();
        let mut b = Bitmap::new(w, h, NONE);
        v.draw(&IDX, &mut b, 0, 0);
        b
    }

    fn column(b: &Bitmap, x: i32) -> Vec<u32> {
        (0..b.height() as i32).map(|y| b.pixel(x, y)).collect()
    }

    fn spectrum(settings: VisSettings, size: VisSize, bars: [u8; BARS]) -> Bitmap {
        let mut v = Visualizer::new(settings);
        v.set_size(size);
        v.tick(Some(&VisData {
            bars,
            scope: [0; BARS],
        }));
        render(&v)
    }

    fn scope(style: ScopeStyle, size: VisSize, values: [i8; BARS]) -> Bitmap {
        let mut v = Visualizer::new(VisSettings {
            mode: VisMode::Oscilloscope,
            scope: style,
            ..Default::default()
        });
        v.set_size(size);
        v.tick(Some(&VisData {
            bars: [0; BARS],
            scope: values,
        }));
        render(&v)
    }

    fn thin() -> VisSettings {
        VisSettings {
            thick_bands: false,
            peaks: false,
            ..Default::default()
        }
    }

    /// Background and grid of 1x column `x`.
    fn bg_column(x: i32, h: i32) -> Vec<u32> {
        (0..h)
            .map(|y| u32::from(x % 2 == 0 && x < 75 && y % 2 == 1))
            .collect()
    }

    #[test]
    fn background_grid_main() {
        let mut b = Bitmap::new(76, 16, NONE);
        draw_background(VisSize::Main, &IDX, &mut b, 0, 0);
        for y in 0..16 {
            for x in 0..76 {
                let grid = x % 2 == 0 && x <= 74 && y % 2 == 1;
                assert_eq!(b.pixel(x, y), grid as u32, "({x},{y})");
            }
        }
    }

    #[test]
    fn background_grid_double_and_shade() {
        let mut b = Bitmap::new(152, 32, NONE);
        draw_background(VisSize::MainDouble, &IDX, &mut b, 0, 0);
        for y in 0..32 {
            for x in 0..152 {
                let small = (x / 2) % 2 == 0 && x / 2 <= 74 && (y / 2) % 2 == 1;
                assert_eq!(b.pixel(x, y), small as u32, "({x},{y})");
            }
        }
        for size in [VisSize::Shade, VisSize::ShadeDouble] {
            let (w, h) = size.dimensions();
            let mut b = Bitmap::new(w, h, NONE);
            draw_background(size, &IDX, &mut b, 0, 0);
            assert!(b.pixels().iter().all(|&p| p == 0));
        }
    }

    #[test]
    fn baseline_offset_hides_two_levels() {
        let mut bars = [0; BARS];
        bars[..5].copy_from_slice(&[1, 2, 3, 4, 15]);
        let b = spectrum(thin(), VisSize::Main, bars);
        assert_eq!(column(&b, 0), bg_column(0, 16));
        assert_eq!(column(&b, 1), bg_column(1, 16));
        let mut c2 = bg_column(2, 16);
        c2[15] = 15;
        assert_eq!(column(&b, 2), c2);
        let mut c3 = bg_column(3, 16);
        c3[15] = 15;
        c3[14] = 14;
        assert_eq!(column(&b, 3), c3);
        let c4 = column(&b, 4);
        assert_eq!(&c4[3..], &(3..=15).collect::<Vec<u32>>()[..]);
        assert_eq!(&c4[..3], &bg_column(4, 3)[..]);
    }

    #[test]
    fn peaks_appear_from_level_two() {
        let mut bars = [0; BARS];
        bars[..4].copy_from_slice(&[1, 2, 3, 15]);
        let b = spectrum(
            VisSettings {
                thick_bands: false,
                ..Default::default()
            },
            VisSize::Main,
            bars,
        );
        assert!(!column(&b, 0).contains(&23));
        assert_eq!(column(&b, 1)[15], 23);
        assert_eq!(column(&b, 2)[14], 23);
        assert_eq!(column(&b, 3)[2], 23);
        assert_eq!(column(&b, 3)[3], 3);
    }

    #[test]
    fn style_colours_main() {
        let mut bars = [0; BARS];
        bars[0] = 10;
        bars[1] = 6;
        let col = |style| {
            let b = spectrum(VisSettings { style, ..thin() }, VisSize::Main, bars);
            (column(&b, 0)[8..].to_vec(), column(&b, 1)[12..].to_vec())
        };
        // Level y is at row 17 − y; levels 2..v−1 are visible.
        let range = |a: u32, b: u32| (a..=b).collect::<Vec<u32>>();
        assert_eq!(col(AnalyzerStyle::Normal), (range(8, 15), range(12, 15)));
        assert_eq!(col(AnalyzerStyle::Fire), (range(3, 10), range(3, 6)));
        assert_eq!(col(AnalyzerStyle::Line), (vec![7; 8], vec![11; 4]));
    }

    #[test]
    fn thick_bands_show_nineteen_bars() {
        let b = spectrum(
            VisSettings {
                peaks: false,
                ..Default::default()
            },
            VisSize::Main,
            [15; BARS],
        );
        let lit: Vec<bool> = (0..76).map(|x| b.pixel(x, 15) == 15).collect();
        for (x, &l) in lit.iter().enumerate() {
            assert_eq!(l, x < 75 && x % 4 != 3, "column {x}");
        }
        let starts = (0..75)
            .filter(|x| lit[*x] && (*x == 0 || !lit[x - 1]))
            .count();
        assert_eq!(starts, 19);
        assert_eq!(b.pixel(3, 15), 0);
        let b = spectrum(thin(), VisSize::Main, [15; BARS]);
        assert!((0..75).all(|x| b.pixel(x, 15) == 15));
    }

    #[test]
    fn scope_dots_lines_solid_main() {
        let mut values = [0i8; BARS];
        values[0] = 0;
        values[1] = 4;
        values[2] = -2;
        values[3] = -6;
        let dots = scope(ScopeStyle::Dots, VisSize::Main, values);
        let lit = |b: &Bitmap, x| -> Vec<(i32, u32)> {
            (0..16)
                .filter(|&y| b.pixel(x, y) >= 18)
                .map(|y| (y, b.pixel(x, y)))
                .collect()
        };
        // v = 8 → row 9, colour 18; v = 12 → row 5, colour 20; v = 6 → row 11, colour 19;
        // v = 2 → row 15, colour 21.
        assert_eq!(lit(&dots, 0), [(9, 18)]);
        assert_eq!(lit(&dots, 1), [(5, 20)]);
        assert_eq!(lit(&dots, 2), [(11, 19)]);
        assert_eq!(lit(&dots, 3), [(15, 21)]);
        let lines = scope(ScopeStyle::Lines, VisSize::Main, values);
        assert_eq!(lit(&lines, 0), [(9, 18)]);
        assert_eq!(lit(&lines, 1), (5..=9).map(|y| (y, 20)).collect::<Vec<_>>());
        assert_eq!(
            lit(&lines, 2),
            (5..=11).map(|y| (y, 19)).collect::<Vec<_>>()
        );
        assert_eq!(
            lit(&lines, 3),
            (11..=15).map(|y| (y, 21)).collect::<Vec<_>>()
        );
        let solid = scope(ScopeStyle::Solid, VisSize::Main, values);
        assert_eq!(lit(&solid, 0), [(9, 18)]);
        assert_eq!(lit(&solid, 1), (5..=9).map(|y| (y, 20)).collect::<Vec<_>>());
        assert_eq!(
            lit(&solid, 2),
            (10..=11).map(|y| (y, 19)).collect::<Vec<_>>()
        );
        assert_eq!(
            lit(&solid, 3),
            (10..=15).map(|y| (y, 21)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn scope_below_level_two_is_hidden() {
        let mut values = [0i8; BARS];
        values[0] = -7;
        values[1] = -16;
        let b = scope(ScopeStyle::Dots, VisSize::Main, values);
        assert!((0..16).all(|y| b.pixel(0, y) < 18));
        assert!((0..16).all(|y| b.pixel(1, y) < 18));
    }

    #[test]
    fn double_size_spectrum() {
        let mut bars = [0; BARS];
        bars[0] = 1;
        bars[1] = 15;
        bars[2] = 3;
        let b = spectrum(
            VisSettings {
                thick_bands: false,
                ..Default::default()
            },
            VisSize::MainDouble,
            bars,
        );
        assert_eq!((b.width(), b.height()), (152, 32));
        for x in [0, 1] {
            assert_eq!(b.pixel(x, 31), 17);
            assert_eq!(b.pixel(x, 30), 17);
            assert_eq!(b.pixel(x, 29), 23);
            assert_eq!(b.pixel(x, 28), 23);
        }
        for x in [2, 3] {
            let c = column(&b, x);
            let expected: Vec<u32> = (0..30).map(|r| 17 - r / 2).rev().collect();
            assert_eq!(&c[2..], &expected[..]);
            assert_eq!(&c[..2], &[23, 23]);
        }
        let c = column(&b, 4);
        assert_eq!(&c[24..], &[23, 23, 15, 15, 16, 16, 17, 17]);
        assert_eq!(b.pixel(150, 31), 0);
    }

    #[test]
    fn double_size_peak_zero_hidden_and_scope_blocks() {
        let b = spectrum(VisSettings::default(), VisSize::MainDouble, [0; BARS]);
        assert!(!b.pixels().contains(&23));
        let mut values = [0i8; BARS];
        values[0] = 4;
        let b = scope(ScopeStyle::Dots, VisSize::MainDouble, values);
        let lit: Vec<(i32, i32)> = (0..152)
            .flat_map(|x| (0..32).map(move |y| (x, y)))
            .filter(|&(x, y)| x < 2 && b.pixel(x, y) >= 18)
            .collect();
        assert_eq!(lit, [(0, 6), (0, 7), (1, 6), (1, 7)]);
        assert_eq!(b.pixel(2, 15), 18);
        assert_eq!(b.pixel(2, 14), 18);
    }

    #[test]
    fn shade_spectrum() {
        let mut bars = [0; BARS];
        bars[0] = 15;
        bars[1] = 15;
        bars[2] = 9;
        bars[3] = 9;
        bars[4] = 3;
        bars[5] = 3;
        let b = spectrum(
            VisSettings {
                thick_bands: false,
                ..Default::default()
            },
            VisSize::Shade,
            bars,
        );
        assert_eq!((b.width(), b.height()), (38, 5));
        assert_eq!(column(&b, 0), [23, 8, 11, 14, 17]);
        assert_eq!(column(&b, 1), [0, 23, 11, 14, 17]);
        assert_eq!(column(&b, 2), [0, 0, 0, 23, 17]);
        assert_eq!(column(&b, 3), [0; 5]);
        assert_eq!(column(&b, 37), [0; 5]);
    }

    #[test]
    fn shade_full_scale_without_peaks() {
        let b = spectrum(thin(), VisSize::Shade, [15; BARS]);
        assert_eq!(column(&b, 0), [5, 8, 11, 14, 17]);
    }

    #[test]
    fn shade_double_spectrum() {
        let mut bars = [0; BARS];
        bars[0] = 15;
        bars[1] = 6;
        let b = spectrum(
            VisSettings {
                thick_bands: false,
                peaks: false,
                ..Default::default()
            },
            VisSize::ShadeDouble,
            bars,
        );
        assert_eq!((b.width(), b.height()), (76, 10));
        assert_eq!(column(&b, 0), [4, 5, 7, 8, 10, 11, 13, 14, 16, 17]);
        assert_eq!(column(&b, 1), [0, 0, 0, 0, 0, 0, 13, 14, 16, 17]);
        assert_eq!(column(&b, 75), [0; 10]);
    }

    #[test]
    fn shade_scopes() {
        let mut values = [0i8; BARS];
        values[0] = 8;
        values[1] = -8;
        values[37] = 8;
        values[38] = 8;
        let b = scope(ScopeStyle::Solid, VisSize::Shade, values);
        assert_eq!(column(&b, 0), [18, 18, 18, 0, 0]);
        assert_eq!(column(&b, 1), [0, 0, 18, 18, 18]);
        assert_eq!(column(&b, 2), [0, 0, 18, 0, 0]);
        assert_eq!(column(&b, 37), [18, 18, 18, 0, 0]);
        let b = scope(ScopeStyle::Dots, VisSize::ShadeDouble, values);
        assert_eq!(column(&b, 0), [18, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(column(&b, 1), [0, 0, 0, 0, 0, 0, 0, 0, 0, 18]);
        assert_eq!(column(&b, 2), [0, 0, 0, 0, 18, 0, 0, 0, 0, 0]);
        assert_eq!(column(&b, 38), [18, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let b = scope(ScopeStyle::Solid, VisSize::ShadeDouble, values);
        assert_eq!(column(&b, 0), [18, 18, 18, 18, 18, 0, 0, 0, 0, 0]);
        assert_eq!(column(&b, 1), [0, 0, 0, 0, 18, 18, 18, 18, 18, 18]);
    }

    #[test]
    fn playlist_mini_is_left_72_columns() {
        let mut v = Visualizer::new(thin());
        v.tick(Some(&VisData {
            bars: [15; BARS],
            scope: [0; BARS],
        }));
        let main = render(&v);
        let mut pl = Bitmap::new(80, 20, NONE);
        v.draw_playlist(&IDX, &mut pl, 3, 2);
        for y in 0..20 {
            for x in 0..80 {
                let expected = if (3..75).contains(&x) && (2..18).contains(&y) {
                    main.pixel(x - 3, y - 2)
                } else {
                    NONE
                };
                assert_eq!(pl.pixel(x, y), expected, "({x},{y})");
            }
        }
    }

    #[test]
    fn off_draws_nothing() {
        let mut v = Visualizer::new(VisSettings {
            mode: VisMode::Off,
            ..Default::default()
        });
        v.tick(None);
        let b = render(&v);
        assert!(b.pixels().iter().all(|&p| p == NONE));
    }
}
