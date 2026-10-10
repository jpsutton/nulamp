//! `region.txt` window shapes (01 §4).

use crate::ini::Ini;

/// One polygon: vertices in skin pixels (pixel-edge coordinates).
pub type Polygon = Vec<(i32, i32)>;

/// Shapes from `region.txt`. `None` means "rectangular window".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Regions {
    pub normal: Option<Vec<Polygon>>,
    pub window_shade: Option<Vec<Polygon>>,
    pub equalizer: Option<Vec<Polygon>>,
    pub equalizer_ws: Option<Vec<Polygon>>,
}

impl Regions {
    pub fn parse(text: &str) -> Self {
        let ini = Ini::parse(text);
        Self {
            normal: section(&ini, "Normal"),
            window_shade: section(&ini, "WindowShade"),
            equalizer: section(&ini, "Equalizer"),
            equalizer_ws: section(&ini, "EqualizerWS"),
        }
    }
}

/// Parses numbers separated by any mix of spaces, tabs and commas.
/// Stops at the first token that is not a number (01 §4.2).
pub fn parse_numbers(value: &str) -> Vec<i32> {
    let mut out = Vec::new();
    let b = value.as_bytes();
    let mut i = 0;
    loop {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b',') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        if b[i] == b'-' {
            i += 1;
        }
        let digits = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == digits {
            break;
        }
        // A number must end at a separator or the end of the value.
        if i < b.len() && !matches!(b[i], b' ' | b'\t' | b',') {
            break;
        }
        let n: i64 = value[start..i].parse().unwrap_or(0);
        out.push(n.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
    }
    out
}

fn section(ini: &Ini, name: &str) -> Option<Vec<Polygon>> {
    let counts = parse_numbers(ini.get(name, "NumPoints")?);
    let points = parse_numbers(ini.get(name, "PointList")?);
    if !points.len().is_multiple_of(2) {
        return None;
    }
    let total: i64 = counts.iter().map(|&c| c as i64).sum();
    if counts.iter().any(|&c| c < 0) || total != (points.len() / 2) as i64 || counts.is_empty() {
        return None;
    }
    let mut polys = Vec::with_capacity(counts.len());
    let mut it = points.as_chunks::<2>().0.iter().map(|p| (p[0], p[1]));
    for &c in &counts {
        polys.push(it.by_ref().take(c as usize).collect());
    }
    Some(polys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_points() {
        let r = Regions::parse("[Normal]\nNumPoints=4\nPointList=0,0, 275,0, 275,116, 0,116\n");
        assert_eq!(
            r.normal,
            Some(vec![vec![(0, 0), (275, 0), (275, 116), (0, 116)]])
        );
        assert!(r.window_shade.is_none());
    }

    #[test]
    fn sum_mismatch_is_ignored() {
        let r = Regions::parse("[Normal]\nNumPoints=5\nPointList=0,0,1,0,1,1,0,1\n");
        assert!(r.normal.is_none());
    }

    #[test]
    fn mixed_separators_and_junk() {
        assert_eq!(parse_numbers(" 1,\t2 ,, -3\t4 junk 5"), vec![1, 2, -3, 4]);
        assert_eq!(parse_numbers("1 2x 3"), vec![1]);
        let r = Regions::parse(
            "[Equalizer]\nNumPoints = 2, 2 ; x\nPointList=0 0,\t1 1, 2 2 3 3 ; tail\n",
        );
        assert_eq!(
            r.equalizer,
            Some(vec![vec![(0, 0), (1, 1)], vec![(2, 2), (3, 3)]])
        );
    }

    #[test]
    fn playlist_sections_ignored() {
        let r = Regions::parse("[Playlist]\nNumPoints=4\nPointList=0,0,1,0,1,1,0,1\n");
        assert_eq!(r, Regions::default());
    }

    #[test]
    fn odd_point_count_invalid() {
        let r = Regions::parse("[WindowShade]\nNumPoints=1\nPointList=0,0,1\n");
        assert!(r.window_shade.is_none());
    }
}
