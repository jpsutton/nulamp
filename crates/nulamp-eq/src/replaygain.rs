//! ReplayGain settings, tag parsing and gain computation (07 §5.1–§5.3).
//!
//! The result of [`gain_factor`] is meant for [`crate::Equalizer::set_pre_gain`], which folds
//! it into the single gain stage of 07 §4.

/// Lowest allowed value of the no-tag gain and the preamp, dB.
pub const MIN_DB: f32 = -14.0;

/// Highest allowed value of the no-tag gain and the preamp, dB.
pub const MAX_DB: f32 = 6.0;

/// Offset added to Opus R128 gains to reach the ReplayGain reference (−23 → −18 LUFS).
pub const R128_TO_REPLAYGAIN_DB: f32 = 5.0;

/// Which tag set is preferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// Per-track gain and peak.
    #[default]
    Track,
    /// Per-album gain and peak.
    Album,
}

/// How the gain and peak are combined (07 §5.2 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 10^(dB / 20).
    ApplyGain,
    /// min(10^(dB / 20), 1 / peak).
    #[default]
    ApplyGainPreventClipping,
    /// 1 / peak.
    Normalize,
    /// 1 / peak if peak > 1, else 1.
    PreventClipping,
}

/// ReplayGain preferences with the reference defaults (07 §5.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplayGainSettings {
    /// Off by default.
    pub enabled: bool,
    /// Preferred source, default track.
    pub source: Source,
    /// Default "apply gain, prevent clipping".
    pub mode: Mode,
    /// Do not fall back to the other source. Default off.
    pub preferred_only: bool,
    /// Gain for files without ReplayGain, dB. Default −6.0, range −14..+6.
    pub no_tag_db: f32,
    /// Preamp for files with ReplayGain, dB. Default 0.0, range −14..+6.
    pub preamp_db: f32,
}

impl Default for ReplayGainSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            source: Source::Track,
            mode: Mode::ApplyGainPreventClipping,
            preferred_only: false,
            no_tag_db: -6.0,
            preamp_db: 0.0,
        }
    }
}

/// ReplayGain values read from a file's tags. Gains in dB, peaks linear.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReplayGainTags {
    /// `replaygain_track_gain`.
    pub track_gain: Option<f32>,
    /// `replaygain_track_peak`.
    pub track_peak: Option<f32>,
    /// `replaygain_album_gain`.
    pub album_gain: Option<f32>,
    /// `replaygain_album_peak`.
    pub album_peak: Option<f32>,
    /// `R128_TRACK_GAIN`, already converted to the ReplayGain reference.
    pub r128_track_gain: Option<f32>,
    /// `R128_ALBUM_GAIN`, already converted to the ReplayGain reference.
    pub r128_album_gain: Option<f32>,
}

impl ReplayGainTags {
    /// Stores one tag if `key` is a ReplayGain or R128 key (case-insensitive).
    ///
    /// Returns false for other keys and for values that do not parse.
    pub fn set_tag(&mut self, key: &str, value: &str) -> bool {
        let key = key.trim().to_ascii_lowercase();
        let (slot, parsed) = match key.as_str() {
            "replaygain_track_gain" => (&mut self.track_gain, parse_gain(value)),
            "replaygain_track_peak" => (&mut self.track_peak, parse_peak(value)),
            "replaygain_album_gain" => (&mut self.album_gain, parse_gain(value)),
            "replaygain_album_peak" => (&mut self.album_peak, parse_peak(value)),
            "r128_track_gain" => (&mut self.r128_track_gain, parse_r128_gain(value)),
            "r128_album_gain" => (&mut self.r128_album_gain, parse_r128_gain(value)),
            _ => return false,
        };
        *slot = parsed;
        parsed.is_some()
    }

    /// Gain and peak of one source. ReplayGain tags win over R128 tags (decision EQ7).
    pub fn source(&self, source: Source) -> (Option<f32>, Option<f32>) {
        match source {
            Source::Track => (self.track_gain.or(self.r128_track_gain), self.track_peak),
            Source::Album => (self.album_gain.or(self.r128_album_gain), self.album_peak),
        }
    }
}

/// Linear gain factor for one track (07 §5.2). Returns 1.0 when ReplayGain is disabled.
pub fn gain_factor(settings: &ReplayGainSettings, tags: &ReplayGainTags) -> f32 {
    if !settings.enabled {
        return 1.0;
    }
    let other = match settings.source {
        Source::Track => Source::Album,
        Source::Album => Source::Track,
    };
    let mut chosen = tags.source(settings.source);
    if chosen.0.is_none() && !settings.preferred_only {
        chosen = tags.source(other);
    }
    let (Some(db), peak) = chosen else {
        return db_to_linear(settings.no_tag_db.clamp(MIN_DB, MAX_DB));
    };
    let peak = f64::from(peak.filter(|p| p.is_finite() && *p > 0.0).unwrap_or(1.0));
    let gain = 10f64.powf(f64::from(db) / 20.0);
    let factor = match settings.mode {
        Mode::ApplyGain => gain,
        Mode::ApplyGainPreventClipping => gain.min(1.0 / peak),
        Mode::Normalize => 1.0 / peak,
        Mode::PreventClipping if peak > 1.0 => 1.0 / peak,
        Mode::PreventClipping => 1.0,
    };
    let preamp = 10f64.powf(f64::from(settings.preamp_db.clamp(MIN_DB, MAX_DB)) / 20.0);
    (factor * preamp) as f32
}

/// 10^(dB / 20).
pub fn db_to_linear(db: f32) -> f32 {
    10f64.powf(f64::from(db) / 20.0) as f32
}

/// Parses a gain such as `-7.89 dB`, `+2.5dB` or `3`.
///
/// Accepts surrounding spaces, a leading sign and a trailing `dB` (any case, optional space).
/// The decimal separator is always `.`; exponents are not accepted.
pub fn parse_gain(text: &str) -> Option<f32> {
    let t = text.trim();
    let (num, rest) = split_number(t)?;
    let rest = rest.trim_start();
    if !(rest.is_empty() || rest.eq_ignore_ascii_case("db")) {
        return None;
    }
    num.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Parses a linear peak such as `0.988`. Negative values are rejected.
pub fn parse_peak(text: &str) -> Option<f32> {
    let (num, rest) = split_number(text.trim())?;
    if !rest.is_empty() {
        return None;
    }
    num.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

/// Converts an Opus R128 gain (Q7.8 dB relative to −23 LUFS) to a ReplayGain gain in dB.
pub fn r128_to_replaygain_db(q78: i16) -> f32 {
    f32::from(q78) / 256.0 + R128_TO_REPLAYGAIN_DB
}

/// Parses an `R128_TRACK_GAIN` / `R128_ALBUM_GAIN` value (a signed Q7.8 integer) and converts
/// it with [`r128_to_replaygain_db`].
pub fn parse_r128_gain(text: &str) -> Option<f32> {
    text.trim().parse::<i16>().ok().map(r128_to_replaygain_db)
}

/// Splits `[+-]digits[.digits]` off the front of `t`. Needs at least one digit.
fn split_number(t: &str) -> Option<(&str, &str)> {
    let b = t.as_bytes();
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let start_digits = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut digits = i - start_digits;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let frac = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        digits += i - frac;
    }
    (digits > 0).then(|| t.split_at(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(x: f32) -> f64 {
        20.0 * f64::from(x).log10()
    }

    fn close(factor: f32, want_db: f64) {
        assert!(
            (db(factor) - want_db).abs() < 0.01,
            "{} dB vs {want_db} dB",
            db(factor)
        );
    }

    fn tags() -> ReplayGainTags {
        let mut t = ReplayGainTags::default();
        assert!(t.set_tag("REPLAYGAIN_TRACK_GAIN", "+3.50 dB"));
        assert!(t.set_tag("replaygain_track_peak", "0.5"));
        assert!(t.set_tag("Replaygain_Album_Gain", "-4.25 dB"));
        assert!(t.set_tag("replaygain_album_peak", "1.2"));
        t
    }

    fn on(source: Source, mode: Mode) -> ReplayGainSettings {
        ReplayGainSettings {
            enabled: true,
            source,
            mode,
            ..ReplayGainSettings::default()
        }
    }

    #[test]
    fn defaults() {
        let s = ReplayGainSettings::default();
        assert!(!s.enabled && !s.preferred_only);
        assert_eq!(s.source, Source::Track);
        assert_eq!(s.mode, Mode::ApplyGainPreventClipping);
        assert_eq!((s.no_tag_db, s.preamp_db), (-6.0, 0.0));
        assert_eq!(gain_factor(&s, &tags()), 1.0);
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_gain("+3.50 dB"), Some(3.5));
        assert_eq!(parse_gain("-7.89dB"), Some(-7.89));
        assert_eq!(parse_gain(" -1 DB "), Some(-1.0));
        assert_eq!(parse_gain(".5"), Some(0.5));
        assert_eq!(parse_gain("2."), Some(2.0));
        assert_eq!(parse_gain("-7,89 dB"), None);
        assert_eq!(parse_gain("dB"), None);
        assert_eq!(parse_gain("1e3"), None);
        assert_eq!(parse_gain(""), None);
        assert_eq!(parse_peak("0.988547"), Some(0.988547));
        assert_eq!(parse_peak("1.5 dB"), None);
        assert_eq!(parse_peak("-1"), None);
        assert_eq!(parse_r128_gain("-2560"), Some(-5.0));
        assert_eq!(parse_r128_gain("256"), Some(6.0));
        assert_eq!(parse_r128_gain("40000"), None);
        assert_eq!(r128_to_replaygain_db(0), 5.0);
    }

    #[test]
    fn four_modes_track_and_album() {
        let t = tags();
        // Track: +3.5 dB, peak 0.5 (headroom +6.02 dB).
        close(gain_factor(&on(Source::Track, Mode::ApplyGain), &t), 3.5);
        close(
            gain_factor(&on(Source::Track, Mode::ApplyGainPreventClipping), &t),
            3.5,
        );
        close(
            gain_factor(&on(Source::Track, Mode::Normalize), &t),
            -20.0 * 0.5f64.log10(),
        );
        close(
            gain_factor(&on(Source::Track, Mode::PreventClipping), &t),
            0.0,
        );
        // Album: −4.25 dB, peak 1.2.
        let peak_db = -20.0 * 1.2f64.log10();
        close(gain_factor(&on(Source::Album, Mode::ApplyGain), &t), -4.25);
        close(
            gain_factor(&on(Source::Album, Mode::ApplyGainPreventClipping), &t),
            -4.25,
        );
        close(
            gain_factor(&on(Source::Album, Mode::Normalize), &t),
            peak_db,
        );
        close(
            gain_factor(&on(Source::Album, Mode::PreventClipping), &t),
            peak_db,
        );
    }

    #[test]
    fn clipping_check_then_preamp() {
        let mut t = ReplayGainTags::default();
        t.set_tag("replaygain_track_gain", "+9 dB");
        t.set_tag("replaygain_track_peak", "0.8");
        let mut s = on(Source::Track, Mode::ApplyGainPreventClipping);
        let limit = -20.0 * 0.8f64.log10();
        close(gain_factor(&s, &t), limit);
        s.preamp_db = 3.0;
        close(gain_factor(&s, &t), limit + 3.0);
        s.preamp_db = 10.0;
        close(gain_factor(&s, &t), limit + 6.0);
    }

    #[test]
    fn fallback_and_missing() {
        let mut album_only = ReplayGainTags::default();
        album_only.set_tag("replaygain_album_gain", "-2 dB");
        let mut s = on(Source::Track, Mode::ApplyGain);
        close(gain_factor(&s, &album_only), -2.0);
        s.preferred_only = true;
        close(gain_factor(&s, &album_only), -6.0);

        let mut track_only = ReplayGainTags::default();
        track_only.set_tag("replaygain_track_gain", "1.5 dB");
        let mut s = on(Source::Album, Mode::ApplyGain);
        close(gain_factor(&s, &track_only), 1.5);
        s.preferred_only = true;
        s.no_tag_db = -3.0;
        s.preamp_db = 4.0;
        close(gain_factor(&s, &track_only), -3.0);

        let none = ReplayGainTags::default();
        let mut s = on(Source::Track, Mode::Normalize);
        close(gain_factor(&s, &none), -6.0);
        s.no_tag_db = -20.0;
        close(gain_factor(&s, &none), -14.0);

        // Missing peak counts as 1.0.
        let mut s = on(Source::Track, Mode::Normalize);
        close(gain_factor(&s, &track_only), 0.0);
        s.mode = Mode::ApplyGainPreventClipping;
        close(gain_factor(&s, &track_only), 0.0);
    }

    #[test]
    fn r128_tags() {
        let mut t = ReplayGainTags::default();
        assert!(t.set_tag("R128_TRACK_GAIN", "-1280"));
        let s = on(Source::Track, Mode::ApplyGain);
        close(gain_factor(&s, &t), 0.0);
        t.set_tag("REPLAYGAIN_TRACK_GAIN", "-2 dB");
        close(gain_factor(&s, &t), -2.0);
        assert!(!t.set_tag("TITLE", "x"));
    }
}
