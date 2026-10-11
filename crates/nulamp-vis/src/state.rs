//! Per-column analyzer state, idle animation and refresh timing (06 §2.4, §2.5, §4.3).

use crate::BARS;
use crate::analysis::VisData;
use crate::settings::{AnalyzerStyle, VisMode, VisSettings};

/// Columns of the single-size windowshade analyzer (06 §5.2).
pub(crate) const SHADE_COLUMNS: usize = 37;

/// The visualizer surfaces (06 §1, §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisSize {
    /// Main window, 76×16, with the 2-row baseline offset (06 §4).
    #[default]
    Main,
    /// Main window in double size, 152×32, rendered natively (06 §5.1).
    MainDouble,
    /// Main window shade, 38×5 (06 §5.2).
    Shade,
    /// Main window shade in double size, 76×10 (06 §5.3): 75 one-pixel columns fed like
    /// [`VisSize::Main`]; the solid scope centre is level 5, where silence falls.
    ShadeDouble,
}

impl VisSize {
    /// Width and height in pixels.
    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Main => (76, 16),
            Self::MainDouble => (152, 32),
            Self::Shade => (38, 5),
            Self::ShadeDouble => (76, 10),
        }
    }

    /// The single-size shade runs the analyzer on every other state column (06 §5.2).
    fn half_columns(self) -> bool {
        self == Self::Shade
    }
}

/// What each state column shows on the current tick, before peak motion (06 §4.3 step 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shown {
    /// Bar level v, 0..15.
    pub(crate) level: [u8; BARS],
    /// Colour base c.
    pub(crate) colour: [u8; BARS],
    /// Peak level ph / 256, 0..15.
    pub(crate) peak: [u8; BARS],
}

/// Idle animation: a bouncing bar and a scrolling sine (06 §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Idle {
    pos: u8,
    falling: bool,
    phase: u32,
}

impl Idle {
    /// The idle frame for this tick; then moves one step.
    fn next(&mut self) -> VisData {
        let mut d = VisData::default();
        d.bars[self.pos as usize] = 15;
        for (x, s) in d.scope.iter_mut().enumerate() {
            *s = (7.0 * ((self.phase as f32 + x as f32) * 0.1).sin()).round() as i8;
        }
        if self.falling {
            self.pos -= 1;
            self.falling = self.pos > 0;
        } else {
            self.pos += 1;
            self.falling = self.pos as usize == BARS - 1;
        }
        self.phase = self.phase.wrapping_add(1);
        d
    }
}

/// The classic visualizer: analyzer falloff and peaks, oscilloscope values and the
/// "advance every tick, draw every Nth tick" refresh logic.
///
/// Call [`Visualizer::tick`] about every 16 ms while the visualizer runs (06 §2.4) and, when it
/// returns true, [`Visualizer::draw`]. On stop, call [`Visualizer::reset`] and draw one empty
/// frame with [`crate::draw_background`].
#[derive(Debug, Clone)]
pub struct Visualizer {
    settings: VisSettings,
    size: VisSize,
    bh: [i32; BARS],
    ph: [i32; BARS],
    pv: [f32; BARS],
    pub(crate) shown: Shown,
    pub(crate) scope: [i8; BARS],
    idle: Idle,
    ticks: u32,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new(VisSettings::default())
    }
}

impl Visualizer {
    pub fn new(settings: VisSettings) -> Self {
        Self {
            settings,
            size: VisSize::Main,
            bh: [0; BARS],
            ph: [0; BARS],
            pv: [0.0; BARS],
            shown: Shown {
                level: [0; BARS],
                colour: [17; BARS],
                peak: [0; BARS],
            },
            scope: [0; BARS],
            idle: Idle::default(),
            ticks: 0,
        }
    }

    pub fn settings(&self) -> &VisSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: VisSettings) {
        let restyle = settings.style != self.settings.style;
        self.settings = settings;
        // The shown colour bases belong to the old style; drawing a Line base (17 − v) with
        // the Normal or Fire gradient would run below colour 0 before the next tick.
        if restyle {
            for i in 0..BARS {
                self.shown.colour[i] =
                    colour_base(settings.style, self.shown.level[i] as i32) as u8;
            }
        }
    }

    pub fn size(&self) -> VisSize {
        self.size
    }

    /// Sets the surface; ticks and draws use it. The single-size shade feeds the analyzer
    /// state differently (06 §5.2), so set the size before ticking.
    pub fn set_size(&mut self, size: VisSize) {
        self.size = size;
    }

    /// Clears bars, peaks, the idle animation and the tick counter.
    pub fn reset(&mut self) {
        *self = Self {
            settings: self.settings,
            size: self.size,
            ..Self::new(self.settings)
        };
    }

    /// Advances one timer tick with the data for the current output position, or `None` when
    /// there is none within 400 ms (the idle animation is used). Returns true when the pixels
    /// should be redrawn on this tick. Does nothing and returns false when the mode is off.
    pub fn tick(&mut self, data: Option<&VisData>) -> bool {
        if self.settings.mode == VisMode::Off {
            return false;
        }
        let data = match data {
            Some(d) => *d,
            None => self.idle.next(),
        };
        match self.settings.mode {
            VisMode::Spectrum => self.advance_spectrum(&data.bars),
            VisMode::Oscilloscope => self.scope = data.scope,
            VisMode::Off => {}
        }
        let draw = self.ticks.is_multiple_of(self.settings.refresh.divisor());
        self.ticks = self.ticks.wrapping_add(1);
        draw
    }

    /// Current bar height of state column `i`, in 1/16 pixel.
    pub fn bar_height(&self, i: usize) -> i32 {
        self.bh[i]
    }

    /// Current peak height of state column `i`, in 1/256 pixel.
    pub fn peak_height(&self, i: usize) -> i32 {
        self.ph[i]
    }

    fn advance_spectrum(&mut self, bars: &[u8; BARS]) {
        let bar = |i: usize| bars.get(i).copied().unwrap_or(0) as i32;
        let thick = self.settings.thick_bands;
        if self.size.half_columns() {
            for x in 0..SHADE_COLUMNS {
                let v = if thick {
                    let t = 2 * (x & !3);
                    (bar(t) + bar(t + 1) + bar(t + 2) + bar(t + 3)) / 4
                } else {
                    (bar(2 * x) + bar(2 * x + 1)) / 2
                };
                self.step_column(2 * x, v);
            }
        } else {
            for x in 0..BARS {
                let v = if thick {
                    let t = x & !3;
                    (bar(t) + bar(t + 1) + bar(t + 2) + bar(t + 3)) / 4
                } else {
                    bar(x)
                };
                self.step_column(x, v);
            }
        }
    }

    /// 06 §4.3 steps 1–6 for one state column with input level `v`.
    fn step_column(&mut self, i: usize, v: i32) {
        let mut v = v.min(15);
        if v * 16 < self.bh[i] {
            self.bh[i] = (self.bh[i] - self.settings.falloff.falloff_step()).max(0);
            v = self.bh[i] / 16;
        } else {
            self.bh[i] = v * 16;
        }
        let v = v.max(0);
        let c = colour_base(self.settings.style, v);
        if self.ph[i] <= v * 256 {
            self.ph[i] = v * 256;
            self.pv[i] = 3.0;
        }
        self.shown.level[i] = v as u8;
        self.shown.colour[i] = c as u8;
        self.shown.peak[i] = (self.ph[i] / 256) as u8;
        self.ph[i] = (self.ph[i] - self.pv[i] as i32).max(0);
        self.pv[i] = (self.pv[i] * self.settings.peak_falloff.peak_factor()).min(4096.0);
    }
}

/// 06 §4.3 step 3: the colour base of a bar of level `v`.
fn colour_base(style: AnalyzerStyle, v: i32) -> i32 {
    match style {
        AnalyzerStyle::Normal => 17,
        AnalyzerStyle::Fire => v + 2,
        AnalyzerStyle::Line => 17 - v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{RefreshRate, Speed};

    fn data(bars: [u8; BARS]) -> VisData {
        VisData {
            bars,
            scope: [0; BARS],
        }
    }

    fn thin(settings: VisSettings) -> VisSettings {
        VisSettings {
            thick_bands: false,
            ..settings
        }
    }

    #[test]
    fn falloff_follows_step_for_each_setting() {
        for speed in Speed::ALL {
            let mut v = Visualizer::new(thin(VisSettings {
                falloff: speed,
                ..Default::default()
            }));
            v.tick(Some(&data([255; BARS])));
            assert_eq!(v.bar_height(0), 240);
            let step = speed.falloff_step();
            let mut expected = 240;
            while expected > 0 {
                v.tick(Some(&data([0; BARS])));
                expected = (expected - step).max(0);
                assert_eq!(v.bar_height(0), expected, "{speed:?}");
                assert_eq!(v.shown.level[0] as i32, expected / 16);
            }
        }
    }

    #[test]
    fn peak_holds_then_accelerates() {
        for speed in Speed::ALL {
            let mut v = Visualizer::new(thin(VisSettings {
                peak_falloff: speed,
                falloff: Speed::Fastest,
                ..Default::default()
            }));
            v.tick(Some(&data([255; BARS])));
            assert_eq!(v.shown.peak[0], 15);
            let f = speed.peak_factor();
            let (mut ph, mut pv) = (3840 - 3, 3.0 * f);
            assert_eq!(v.peak_height(0), ph);
            let mut drops = Vec::new();
            while ph > 0 {
                v.tick(Some(&data([0; BARS])));
                let before = ph;
                ph = (ph - pv as i32).max(0);
                pv *= f;
                drops.push(before - ph);
                assert_eq!(v.peak_height(0), ph, "{speed:?}");
            }
            assert_eq!(drops[0], (3.0 * f) as i32);
            assert!(
                drops
                    .windows(2)
                    .all(|w| w[1] >= w[0] || w[1] == *drops.last().unwrap())
            );
        }
    }

    #[test]
    fn peak_falls_slower_with_slower_factor() {
        let ticks_to_floor = |speed: Speed| {
            let mut v = Visualizer::new(thin(VisSettings {
                peak_falloff: speed,
                falloff: Speed::Fastest,
                ..Default::default()
            }));
            v.tick(Some(&data([255; BARS])));
            let mut n = 0;
            while v.peak_height(0) > 0 {
                v.tick(Some(&data([0; BARS])));
                n += 1;
            }
            n
        };
        let n: Vec<i32> = Speed::ALL.iter().map(|&s| ticks_to_floor(s)).collect();
        assert!(n.windows(2).all(|w| w[0] > w[1]), "{n:?}");
    }

    #[test]
    fn thick_bands_average_groups_of_four() {
        let mut bars = [0; BARS];
        bars[0..4].copy_from_slice(&[4, 8, 12, 16]);
        bars[72..75].copy_from_slice(&[12, 12, 12]);
        let mut v = Visualizer::default();
        v.tick(Some(&data(bars)));
        assert_eq!(&v.shown.level[0..4], &[10, 10, 10, 10]);
        assert_eq!(&v.shown.level[72..75], &[9, 9, 9]);
    }

    #[test]
    fn changing_style_restyles_shown_bars() {
        let mut v = Visualizer::new(VisSettings {
            style: AnalyzerStyle::Line,
            ..Default::default()
        });
        v.tick(Some(&data([255; BARS])));
        assert_eq!(v.shown.colour[0], 2);
        for (style, c) in [(AnalyzerStyle::Normal, 17), (AnalyzerStyle::Fire, 17)] {
            v.set_settings(VisSettings {
                style,
                ..Default::default()
            });
            assert_eq!(v.shown.colour[0], c, "{style:?}");
            // Drawing before the next tick stays inside the 24 colours.
            let mut b = nulamp_skin::Bitmap::new(76, 16, 0);
            v.draw(&[0; 24], &mut b, 0, 0);
        }
    }

    #[test]
    fn styles_set_colour_base() {
        for (style, c) in [
            (AnalyzerStyle::Normal, 17),
            (AnalyzerStyle::Fire, 12),
            (AnalyzerStyle::Line, 7),
        ] {
            let mut v = Visualizer::new(VisSettings {
                style,
                ..Default::default()
            });
            v.tick(Some(&data([10; BARS])));
            assert_eq!(v.shown.colour[0], c, "{style:?}");
        }
    }

    #[test]
    fn refresh_draws_every_nth_tick() {
        for (rate, n) in [
            (RefreshRate::Full, 1),
            (RefreshRate::Half, 2),
            (RefreshRate::Quarter, 3),
            (RefreshRate::Eighth, 4),
        ] {
            let mut v = Visualizer::new(VisSettings {
                refresh: rate,
                ..Default::default()
            });
            let draws: Vec<bool> = (0..12).map(|_| v.tick(None)).collect();
            let expected: Vec<bool> = (0..12).map(|i| i % n == 0).collect();
            assert_eq!(draws, expected);
        }
    }

    #[test]
    fn state_advances_on_non_drawing_ticks() {
        let mut v = Visualizer::new(thin(VisSettings {
            refresh: RefreshRate::Eighth,
            ..Default::default()
        }));
        v.tick(Some(&data([255; BARS])));
        for _ in 0..3 {
            assert!(!v.tick(Some(&data([0; BARS]))));
        }
        assert_eq!(v.bar_height(0), 240 - 3 * 12);
    }

    #[test]
    fn off_mode_does_nothing() {
        let mut v = Visualizer::new(VisSettings {
            mode: VisMode::Off,
            ..Default::default()
        });
        assert!(!v.tick(Some(&data([255; BARS]))));
        assert_eq!(v.bar_height(0), 0);
    }

    #[test]
    fn idle_bar_bounces() {
        let mut idle = Idle::default();
        let positions: Vec<usize> = (0..160)
            .map(|_| idle.next().bars.iter().position(|&b| b == 15).unwrap())
            .collect();
        let expected: Vec<usize> = (0..74).chain((1..=74).rev()).chain(0..12).collect();
        assert_eq!(positions, expected);
        let mut idle = Idle::default();
        for _ in 0..5 {
            idle.next();
        }
        let d = idle.next();
        assert_eq!(d.bars.iter().filter(|&&b| b != 0).count(), 1);
        for (x, &s) in d.scope.iter().enumerate() {
            let expected = (7.0 * ((5 + x) as f32 * 0.1).sin()).round() as i8;
            assert_eq!(s, expected);
        }
    }

    #[test]
    fn idle_feeds_the_analyzer() {
        let mut v = Visualizer::new(thin(VisSettings::default()));
        v.tick(None);
        assert_eq!(v.shown.level[0], 15);
        v.tick(None);
        assert_eq!(v.shown.level[1], 15);
        assert_eq!(v.shown.level[0], (240 - 12) / 16);
    }

    #[test]
    fn idle_scope_scrolls() {
        let mut v = Visualizer::new(VisSettings {
            mode: VisMode::Oscilloscope,
            ..Default::default()
        });
        v.tick(None);
        v.tick(None);
        for (x, &s) in v.scope.iter().enumerate() {
            assert_eq!(s, (7.0 * ((1 + x) as f32 * 0.1).sin()).round() as i8);
        }
        let d = VisData {
            bars: [0; BARS],
            scope: [3; BARS],
        };
        v.tick(Some(&d));
        assert_eq!(v.scope, [3; BARS]);
    }

    #[test]
    fn shade_uses_every_other_state_column() {
        let mut bars = [0; BARS];
        bars[8..12].copy_from_slice(&[15, 15, 15, 15]);
        bars[20] = 15;
        bars[21] = 5;
        let mut v = Visualizer::default();
        v.set_size(VisSize::Shade);
        v.tick(Some(&data(bars)));
        assert_eq!(&v.shown.level[8..16], &[15, 0, 15, 0, 15, 0, 15, 0]);
        assert_eq!(v.shown.level[16], 0);
        let mut v = Visualizer::new(thin(VisSettings::default()));
        v.set_size(VisSize::Shade);
        v.tick(Some(&data(bars)));
        assert_eq!(v.shown.level[20], 10);
        assert_eq!(v.shown.level[8], 15);
    }

    #[test]
    fn reset_keeps_settings() {
        let s = VisSettings {
            style: AnalyzerStyle::Fire,
            ..Default::default()
        };
        let mut v = Visualizer::new(s);
        v.set_size(VisSize::MainDouble);
        v.tick(Some(&data([255; BARS])));
        v.reset();
        assert_eq!(v.bar_height(0), 0);
        assert_eq!(*v.settings(), s);
        assert_eq!(v.size(), VisSize::MainDouble);
        assert!(v.tick(None));
    }
}
