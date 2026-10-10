//! Window shape masks from `region.txt` polygons (01 §4.4).

use nulamp_skin::region::Polygon;

/// Per-pixel shape mask, row by row. `true` = inside the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub bits: Vec<bool>,
}

impl Mask {
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bits: vec![true; (width * height) as usize],
        }
    }

    pub fn get(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return false;
        }
        self.bits[(y as u32 * self.width + x as u32) as usize]
    }

    /// Horizontal runs `(x, y, len)` of inside pixels; used by platform shape APIs.
    pub fn runs(&self) -> Vec<(u32, u32, u32)> {
        let mut out = Vec::new();
        for y in 0..self.height {
            let mut x = 0;
            while x < self.width {
                if self.bits[(y * self.width + x) as usize] {
                    let start = x;
                    while x < self.width && self.bits[(y * self.width + x) as usize] {
                        x += 1;
                    }
                    out.push((start, y, x - start));
                } else {
                    x += 1;
                }
            }
        }
        out
    }
}

/// Rasterises the union of polygons with the non-zero winding rule.
///
/// Coordinates are pixel edges; pixel (x, y) is inside when its centre is. Every coordinate
/// is multiplied by `scale` (double size).
pub fn rasterize(polys: &[Polygon], width: u32, height: u32, scale: i32) -> Mask {
    let mut bits = vec![false; (width * height) as usize];
    let edges: Vec<((f64, f64), (f64, f64))> = polys
        .iter()
        .flat_map(|p| {
            (0..p.len()).map(move |i| {
                let a = p[i];
                let b = p[(i + 1) % p.len()];
                (
                    ((a.0 * scale) as f64, (a.1 * scale) as f64),
                    ((b.0 * scale) as f64, (b.1 * scale) as f64),
                )
            })
        })
        .collect();
    for y in 0..height {
        let cy = y as f64 + 0.5;
        // Crossings with winding direction.
        let mut xs: Vec<(f64, i32)> = Vec::new();
        for &(a, b) in &edges {
            if (a.1 <= cy && b.1 > cy) || (b.1 <= cy && a.1 > cy) {
                let x = a.0 + (cy - a.1) / (b.1 - a.1) * (b.0 - a.0);
                xs.push((x, if b.1 > a.1 { 1 } else { -1 }));
            }
        }
        xs.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut winding = 0;
        for i in 0..xs.len() {
            winding += xs[i].1;
            if winding != 0 && i + 1 < xs.len() {
                let start = (xs[i].0 - 0.5).ceil().max(0.0) as i64;
                let end = ((xs[i + 1].0 - 0.5).ceil() as i64).min(width as i64);
                for x in start..end {
                    bits[(y as i64 * width as i64 + x) as usize] = true;
                }
            }
        }
    }
    Mask {
        width,
        height,
        bits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_rectangle() {
        let m = rasterize(&[vec![(0, 0), (275, 0), (275, 116), (0, 116)]], 275, 116, 1);
        assert!(m.bits.iter().all(|&b| b));
    }

    #[test]
    fn right_and_bottom_exclusive() {
        let m = rasterize(&[vec![(1, 1), (3, 1), (3, 3), (1, 3)]], 5, 5, 1);
        let inside: Vec<_> = (0..25)
            .filter(|&i| m.bits[i])
            .map(|i| (i % 5, i / 5))
            .collect();
        assert_eq!(inside, vec![(1, 1), (2, 1), (1, 2), (2, 2)]);
    }

    #[test]
    fn union_and_double() {
        let polys = vec![
            vec![(0, 0), (1, 0), (1, 1), (0, 1)],
            vec![(2, 0), (3, 0), (3, 1), (2, 1)],
        ];
        let m = rasterize(&polys, 6, 2, 2);
        assert_eq!(m.runs(), vec![(0, 0, 2), (4, 0, 2), (0, 1, 2), (4, 1, 2)]);
    }

    #[test]
    fn nonzero_winding_overlap() {
        // Two overlapping squares with the same orientation: the overlap stays inside.
        let polys = vec![
            vec![(0, 0), (4, 0), (4, 4), (0, 4)],
            vec![(2, 2), (6, 2), (6, 6), (2, 6)],
        ];
        let m = rasterize(&polys, 6, 6, 1);
        assert!(m.get(3, 3));
        assert!(m.get(5, 5));
        assert!(!m.get(5, 0));
    }
}
