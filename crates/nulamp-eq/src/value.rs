//! Slider value model: 0..63 encoding, dB mapping, preamp table, labels (03 §1.3, §3, §4.1).

/// Number of EQ bands.
pub const BANDS: usize = 10;

/// Highest slider value (−12 dB).
pub const MAX_VALUE: u8 = 63;

/// Centre slider value (0 dB).
pub const FLAT: u8 = 31;

/// Preamp gain multiplier by slider value (03 §3.2).
pub const PREAMP_TABLE: [f32; 64] = [
    4.000000, 3.610166, 3.320019, 3.088821, 2.896617, 2.732131, 2.588368, 2.460685, 2.345845,
    2.241498, 2.145887, 2.057_66, 1.975_76, 1.899338, 1.827707, 1.760303, 1.696653, 1.636363,
    1.579094, 1.524558, 1.472507, 1.422724, 1.375019, 1.329225, 1.285197, 1.242801, 1.201923,
    1.162456, 1.124306, 1.087389, 1.051628, 1.000000, 0.983296, 0.950604, 0.918821, 0.887898,
    0.857789, 0.828454, 0.799853, 0.771950, 0.744712, 0.718108, 0.692110, 0.666689, 0.641822,
    0.617485, 0.593655, 0.570311, 0.547435, 0.525008, 0.503013, 0.481433, 0.460253, 0.439458,
    0.419035, 0.398970, 0.379252, 0.359868, 0.340807, 0.322060, 0.303614, 0.285462, 0.267593,
    0.250000,
];

/// Band centre frequencies in the default (Winamp) mode, Hz.
pub const WINAMP_CENTRES: [f32; BANDS] = [
    70.0, 180.0, 320.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0,
];

/// Band centre frequencies in ISO mode, Hz. Uses 31.5 and 63 rather than 31 and 62.
pub const ISO_CENTRES: [f32; BANDS] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// Marquee band labels in the default (Winamp) mode.
pub const WINAMP_LABELS: [&str; BANDS] = [
    "70 Hz", "180 Hz", "320 Hz", "600 Hz", "1 kHz", "3 kHz", "6 kHz", "12 kHz", "14 kHz", "16 kHz",
];

/// Marquee band labels in ISO mode.
pub const ISO_LABELS: [&str; BANDS] = [
    "31.5 Hz", "63 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz",
];

/// Band centre frequency set (03 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BandMode {
    /// 70 Hz .. 16 kHz.
    #[default]
    Winamp,
    /// ISO octave centres, 31.5 Hz .. 16 kHz.
    Iso,
}

impl BandMode {
    /// Returns `Iso` when `iso` is set, else `Winamp`.
    pub fn from_iso(iso: bool) -> Self {
        if iso { Self::Iso } else { Self::Winamp }
    }

    /// Band centre frequencies in Hz, low band first.
    pub fn centres(self) -> &'static [f32; BANDS] {
        match self {
            Self::Winamp => &WINAMP_CENTRES,
            Self::Iso => &ISO_CENTRES,
        }
    }

    /// Marquee labels, low band first.
    pub fn labels(self) -> &'static [&'static str; BANDS] {
        match self {
            Self::Winamp => &WINAMP_LABELS,
            Self::Iso => &ISO_LABELS,
        }
    }
}

/// Band gain in dB for a slider value (03 §3.1). Values above 63 count as 63.
pub fn band_db(value: u8) -> f32 {
    let v = (i32::from(value) - i32::from(FLAT)).clamp(-31, 32);
    match v.cmp(&0) {
        std::cmp::Ordering::Greater => -12.0 * v as f32 / 32.0,
        std::cmp::Ordering::Less => -12.0 * v as f32 / 31.0,
        std::cmp::Ordering::Equal => 0.0,
    }
}

/// Preamp linear gain for a slider value, from [`PREAMP_TABLE`]. Values above 63 count as 63.
pub fn preamp_gain(value: u8) -> f32 {
    PREAMP_TABLE[usize::from(value.min(MAX_VALUE))]
}

/// The dB value shown in the marquee while dragging a slider (03 §1.3).
///
/// −12 × (value − 31.5) / 31.5, forced to 0.0 when its magnitude is ≤ 0.32.
pub fn slider_db(value: u8) -> f32 {
    let db = -12.0 * (f32::from(value.min(MAX_VALUE)) - 31.5) / 31.5;
    if db.abs() <= 0.32 { 0.0 } else { db }
}

/// [`slider_db`] formatted with a sign and one decimal, for example `+12.0`, `-0.6`, `+0.0`.
pub fn slider_db_label(value: u8) -> String {
    format!("{:+.1}", slider_db(value))
}

/// Which slider a marquee message is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slider {
    /// The preamp slider.
    Preamp,
    /// Band 0..9, low band first.
    Band(usize),
}

/// Marquee text while dragging: `EQ: <label>: <sign>X.X dB` or `Preamp: <sign>X.X dB`.
///
/// Band indexes above 9 are clamped to 9.
pub fn marquee_text(slider: Slider, value: u8, mode: BandMode) -> String {
    let db = slider_db_label(value);
    match slider {
        Slider::Preamp => format!("Preamp: {db} dB"),
        Slider::Band(i) => format!("EQ: {}: {db} dB", mode.labels()[i.min(BANDS - 1)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_db_endpoints() {
        assert_eq!(band_db(0), 12.0);
        assert_eq!(band_db(31), 0.0);
        assert_eq!(band_db(63), -12.0);
        assert_eq!(band_db(200), -12.0);
        assert!((band_db(30) - 12.0 / 31.0).abs() < 1e-6);
        assert!((band_db(32) + 12.0 / 32.0).abs() < 1e-6);
    }

    #[test]
    fn preamp_endpoints() {
        assert_eq!(preamp_gain(0), 4.0);
        assert_eq!(preamp_gain(31), 1.0);
        assert_eq!(preamp_gain(63), 0.25);
        assert_eq!(preamp_gain(64), 0.25);
        assert!(PREAMP_TABLE.windows(2).all(|w| w[0] > w[1]));
    }

    #[test]
    fn marquee_labels() {
        assert_eq!(slider_db_label(0), "+12.0");
        assert_eq!(slider_db_label(31), "+0.0");
        assert_eq!(slider_db_label(32), "+0.0");
        assert_eq!(slider_db_label(30), "+0.6");
        assert_eq!(slider_db_label(33), "-0.6");
        assert_eq!(slider_db_label(63), "-12.0");
        assert_eq!(
            marquee_text(Slider::Band(0), 0, BandMode::Winamp),
            "EQ: 70 Hz: +12.0 dB"
        );
        assert_eq!(
            marquee_text(Slider::Band(0), 63, BandMode::Iso),
            "EQ: 31.5 Hz: -12.0 dB"
        );
        assert_eq!(
            marquee_text(Slider::Preamp, 31, BandMode::Winamp),
            "Preamp: +0.0 dB"
        );
    }
}
