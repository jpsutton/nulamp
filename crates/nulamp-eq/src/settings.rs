//! Persisted EQ state (03 §3, §7).

use crate::value::{BANDS, BandMode, FLAT, MAX_VALUE};

/// Filter design used by [`crate::Equalizer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterEngine {
    /// Band-pass sections with Q 2.82 for boost and 0.705 for cut (03 §4.2).
    #[default]
    Default,
    /// Constant-Q peaking sections, Q 1.41 (03 §4.3).
    ConstantQ,
}

/// EQ values and switches, as stored in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EqSettings {
    /// Band values 0..63, low band first. 0 = +12 dB, 31 = 0 dB, 63 = −12 dB.
    pub bands: [u8; BANDS],
    /// Preamp value 0..63, same encoding as the bands.
    pub preamp: u8,
    /// EQ on/off. Default off.
    pub enabled: bool,
    /// Auto-load presets by file name. Default off.
    pub auto_load: bool,
    /// ISO band centres instead of the Winamp ones. Default off.
    pub iso: bool,
    /// Limiter after the bands. Default on.
    pub limiter: bool,
    /// Filter design. Default [`FilterEngine::Default`].
    pub engine: FilterEngine,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            bands: [FLAT; BANDS],
            preamp: FLAT,
            enabled: false,
            auto_load: false,
            iso: false,
            limiter: true,
            engine: FilterEngine::Default,
        }
    }
}

impl EqSettings {
    /// Band centre set selected by [`Self::iso`].
    pub fn band_mode(&self) -> BandMode {
        BandMode::from_iso(self.iso)
    }

    /// True when the preamp and every band are at 31 (0 dB).
    pub fn is_flat(&self) -> bool {
        self.preamp == FLAT && self.bands.iter().all(|&b| b == FLAT)
    }

    /// True when the EQ stage processes audio: enabled and not flat (03 §4.5).
    pub fn is_active(&self) -> bool {
        self.enabled && !self.is_flat()
    }

    /// Clamps the preamp and band values to 0..63.
    pub fn clamp(&mut self) {
        self.preamp = self.preamp.min(MAX_VALUE);
        for b in &mut self.bands {
            *b = (*b).min(MAX_VALUE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = EqSettings::default();
        assert_eq!(s.bands, [31; 10]);
        assert_eq!(s.preamp, 31);
        assert!(!s.enabled && !s.auto_load && !s.iso && s.limiter);
        assert!(s.is_flat() && !s.is_active());
    }
}
