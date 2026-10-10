//! Rating conversions (spec 10 §7.2, §7.3). nulamp's `rating` field holds 0–5 stars.

/// ID3v2 `POPM` byte written for `stars` (1–5; values above 5 count as 5). 0 gives 0.
pub fn stars_to_popm(stars: u8) -> u8 {
    match stars {
        0 => 0,
        1 => 1,
        2 => 64,
        3 => 128,
        4 => 196,
        _ => 255,
    }
}

/// Stars for an ID3v2 `POPM` byte: 224–255 = 5, 160–223 = 4, 96–159 = 3, 32–95 = 2,
/// 1–31 = 1, 0 = none.
pub fn popm_to_stars(popm: u8) -> u8 {
    match popm {
        0 => 0,
        1..=31 => 1,
        32..=95 => 2,
        96..=159 => 3,
        160..=223 => 4,
        _ => 5,
    }
}

/// Stars for a 0–100 rating (Vorbis `RATING`, MP4 `rate`): rounded to the nearest 20, at most 5.
pub fn percent_to_stars(percent: u32) -> u8 {
    (percent.saturating_add(10) / 20).min(5) as u8
}

/// 0–100 rating for `stars` (×20).
pub fn stars_to_percent(stars: u8) -> u32 {
    u32::from(stars.min(5)) * 20
}

/// Parses a `rating` field value: an integer 0–5 (surrounding spaces allowed).
pub(crate) fn parse_stars(value: &str) -> Option<u8> {
    value.trim().parse::<u8>().ok().filter(|s| *s <= 5)
}

/// Formats stars as the `rating` field; 0 means no rating.
pub(crate) fn stars_field(stars: u8) -> Option<String> {
    (stars > 0).then(|| stars.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popm_both_directions() {
        assert_eq!(
            [0, 1, 2, 3, 4, 5].map(stars_to_popm),
            [0, 1, 64, 128, 196, 255]
        );
        for stars in 0..=5 {
            assert_eq!(popm_to_stars(stars_to_popm(stars)), stars);
        }
        let ranges = [
            (0, 0, 0),
            (1, 31, 1),
            (32, 95, 2),
            (96, 159, 3),
            (160, 223, 4),
            (224, 255, 5),
        ];
        for (lo, hi, stars) in ranges {
            for b in lo..=hi {
                assert_eq!(popm_to_stars(b), stars, "{b}");
            }
        }
    }

    #[test]
    fn percent() {
        assert_eq!(stars_to_percent(3), 60);
        assert_eq!(percent_to_stars(60), 3);
        assert_eq!(percent_to_stars(0), 0);
        assert_eq!(percent_to_stars(9), 0);
        assert_eq!(percent_to_stars(10), 1);
        assert_eq!(percent_to_stars(100), 5);
        assert_eq!(percent_to_stars(250), 5);
        assert_eq!(parse_stars(" 4 "), Some(4));
        assert_eq!(parse_stars("6"), None);
        assert_eq!(stars_field(0), None);
    }
}
