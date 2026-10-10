//! Visualization preferences (06 §6.2, §6.3).

/// What the visualizer area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisMode {
    Off,
    /// Spectrum analyzer (default).
    #[default]
    Spectrum,
    Oscilloscope,
}

impl VisMode {
    /// The mode after a left click: off → spectrum → oscilloscope → off (06 §6.1).
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Spectrum,
            Self::Spectrum => Self::Oscilloscope,
            Self::Oscilloscope => Self::Off,
        }
    }
}

/// Analyzer bar colouring (06 §4.3 step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnalyzerStyle {
    /// Gradient fixed to the window (default).
    #[default]
    Normal,
    /// Gradient moves with the bar; every top pixel is colour 3.
    Fire,
    /// One flat colour per bar, chosen by its height.
    Line,
}

/// Oscilloscope drawing (06 §4.4 step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeStyle {
    Dots,
    /// Vertical runs joining neighbouring columns (default).
    #[default]
    Lines,
    Solid,
}

/// Five-step speed used by both falloff menus (06 §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    Slowest,
    Slow,
    Moderate,
    Fast,
    Fastest,
}

impl Speed {
    /// All speeds, slowest first.
    pub const ALL: [Self; 5] = [
        Self::Slowest,
        Self::Slow,
        Self::Moderate,
        Self::Fast,
        Self::Fastest,
    ];

    /// Bar falloff per tick in 1/16 pixel: 3, 6, 12, 16, 32 (06 §4.3 step 2).
    pub fn falloff_step(self) -> i32 {
        match self {
            Self::Slowest => 3,
            Self::Slow => 6,
            Self::Moderate => 12,
            Self::Fast => 16,
            Self::Fastest => 32,
        }
    }

    /// Peak speed factor per tick: 1.05, 1.1, 1.2, 1.4, 1.6 (06 §4.3 step 6).
    pub fn peak_factor(self) -> f32 {
        match self {
            Self::Slowest => 1.05,
            Self::Slow => 1.1,
            Self::Moderate => 1.2,
            Self::Fast => 1.4,
            Self::Fastest => 1.6,
        }
    }
}

/// How often pixels are redrawn; state still advances every tick (06 §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefreshRate {
    /// Every tick ("70 fps").
    Full,
    /// Every 2nd tick ("35 fps", default).
    #[default]
    Half,
    /// Every 3rd tick ("18 fps").
    Quarter,
    /// Every 4th tick ("9 fps").
    Eighth,
}

impl RefreshRate {
    /// N: pixels are redrawn on every Nth tick.
    pub fn divisor(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::Half => 2,
            Self::Quarter => 3,
            Self::Eighth => 4,
        }
    }
}

/// Visualizer preferences, with the defaults of 06 §6.3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisSettings {
    /// Default [`VisMode::Spectrum`].
    pub mode: VisMode,
    /// Default [`AnalyzerStyle::Normal`].
    pub style: AnalyzerStyle,
    /// Thick (3-pixel) bands instead of thin ones. Default on.
    pub thick_bands: bool,
    /// Peak dots. Default on.
    pub peaks: bool,
    /// Bar falloff. Default [`Speed::Moderate`] (step 12).
    pub falloff: Speed,
    /// Peak falloff. Default [`Speed::Slow`] (factor 1.1).
    pub peak_falloff: Speed,
    /// Default [`ScopeStyle::Lines`].
    pub scope: ScopeStyle,
    /// Default [`RefreshRate::Half`].
    pub refresh: RefreshRate,
}

impl Default for VisSettings {
    fn default() -> Self {
        Self {
            mode: VisMode::Spectrum,
            style: AnalyzerStyle::Normal,
            thick_bands: true,
            peaks: true,
            falloff: Speed::Moderate,
            peak_falloff: Speed::Slow,
            scope: ScopeStyle::Lines,
            refresh: RefreshRate::Half,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let s = VisSettings::default();
        assert_eq!(s.mode, VisMode::Spectrum);
        assert_eq!(s.style, AnalyzerStyle::Normal);
        assert!(s.thick_bands);
        assert!(s.peaks);
        assert_eq!(s.falloff.falloff_step(), 12);
        assert_eq!(s.peak_falloff.peak_factor(), 1.1);
        assert_eq!(s.scope, ScopeStyle::Lines);
        assert_eq!(s.refresh.divisor(), 2);
    }

    #[test]
    fn mode_cycles() {
        assert_eq!(VisMode::Off.next(), VisMode::Spectrum);
        assert_eq!(VisMode::Spectrum.next(), VisMode::Oscilloscope);
        assert_eq!(VisMode::Oscilloscope.next(), VisMode::Off);
    }

    #[test]
    fn speed_tables() {
        let steps: Vec<i32> = Speed::ALL.iter().map(|s| s.falloff_step()).collect();
        assert_eq!(steps, [3, 6, 12, 16, 32]);
        let factors: Vec<f32> = Speed::ALL.iter().map(|s| s.peak_factor()).collect();
        assert_eq!(factors, [1.05, 1.1, 1.2, 1.4, 1.6]);
    }
}
