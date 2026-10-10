//! Equalizer window drawing (spec 03 §1): normal mode 275×116 and windowshade mode 275×14.

use crate::frame::{Frame, blit};
use nulamp_skin::{Skin, SkinBitmap as B};

pub const EQ_SIZE: (u32, u32) = (275, 116);
pub const EQ_SHADE_SIZE: (u32, u32) = (275, 14);

/// X positions of the 11 sliders: preamp first, then bands 1–10 (03 §1.3).
pub const SLIDER_X: [i32; 11] = [21, 78, 96, 114, 132, 150, 168, 186, 204, 222, 240];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqButton {
    Close,
    Shade,
    On,
    Auto,
    Presets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqView {
    pub active: bool,
    pub shade: bool,
    pub iso: bool,
    pub enabled: bool,
    pub auto_load: bool,
    /// Preamp then bands 1–10, values 0..=63.
    pub values: [u8; 11],
    pub pressed: Option<EqButton>,
    /// Index into `values` of the slider being dragged.
    pub dragging: Option<usize>,
    /// Windowshade mini sliders.
    pub volume: u8,
    pub balance: i8,
}

impl Default for EqView {
    fn default() -> Self {
        Self {
            active: true,
            shade: false,
            iso: false,
            enabled: false,
            auto_load: false,
            values: [31; 11],
            pressed: None,
            dragging: None,
            volume: 200,
            balance: 0,
        }
    }
}

/// Background frame index for a slider value (03 §1.3).
pub fn slider_frame(value: u8) -> i32 {
    27 - (value as i32 * 28 / 64)
}

/// Thumb y for a slider value.
pub fn thumb_y(value: u8) -> i32 {
    38 + 63 - 12 - ((63 - value as i32) * 52 / 64)
}

pub fn draw_eq(skin: &Skin, v: &EqView) -> Frame {
    if v.shade {
        return draw_shade(skin, v);
    }
    let bmp = skin.eqmain(v.iso);
    let mut f = Frame::new(EQ_SIZE.0, EQ_SIZE.1, 0);
    blit(&mut f, bmp, 0, 0, 275, 116, 0, 0);
    blit(
        &mut f,
        bmp,
        0,
        if v.active { 134 } else { 149 },
        275,
        14,
        0,
        0,
    );
    let pressed = |b| v.pressed == Some(b);
    if pressed(EqButton::Close) {
        blit(&mut f, bmp, 0, 125, 9, 9, 264, 3);
    }
    if pressed(EqButton::Shade) && !skin.eq_shade_pressed_hidden {
        blit(&mut f, skin.bitmap(B::EqEx), 1, 38, 9, 9, 254, 3);
    }
    let toggle_x = |base: i32, on: bool, down: bool| {
        base + if on { 59 } else { 0 } + if down { 118 } else { 0 }
    };
    blit(
        &mut f,
        bmp,
        toggle_x(10, v.enabled, pressed(EqButton::On)),
        119,
        25,
        12,
        14,
        18,
    );
    blit(
        &mut f,
        bmp,
        toggle_x(35, v.auto_load, pressed(EqButton::Auto)),
        119,
        33,
        12,
        39,
        18,
    );
    blit(
        &mut f,
        bmp,
        224,
        if pressed(EqButton::Presets) { 176 } else { 164 },
        44,
        12,
        217,
        18,
    );
    for (i, &x) in SLIDER_X.iter().enumerate() {
        let n = slider_frame(v.values[i]);
        let (sx, sy) = if n < 14 {
            (13 + n * 15, 164)
        } else {
            (13 + (n - 14) * 15, 229)
        };
        blit(&mut f, bmp, sx, sy, 14, 63, x, 38);
        let thumb_src = if v.dragging == Some(i) { 176 } else { 164 };
        blit(
            &mut f,
            bmp,
            0,
            thumb_src,
            11,
            11,
            x + 1,
            thumb_y(v.values[i]),
        );
    }
    draw_graph(&mut f, bmp, &v.values);
    f
}

/// Kochanek–Bartels spline with tension 0.1, continuity 0, bias 0 (03 §1.4).
fn tcb(keys: &[f64; 12], t: f64) -> f64 {
    let tension = 0.1;
    let k = |i: i64| keys[i.rem_euclid(12) as usize];
    let i = t.floor() as i64;
    let s = t - i as f64;
    let (p0, p1) = (k(i), k(i + 1));
    let m0 = (1.0 - tension) / 2.0 * (k(i + 1) - k(i - 1));
    let m1 = (1.0 - tension) / 2.0 * (k(i + 2) - k(i));
    let (s2, s3) = (s * s, s * s * s);
    (2.0 * s3 - 3.0 * s2 + 1.0) * p0
        + (s3 - 2.0 * s2 + s) * m0
        + (-2.0 * s3 + 3.0 * s2) * p1
        + (s3 - s2) * m1
}

/// The curve's row (0..=18) for each of the 109 columns.
pub fn graph_rows(bands: &[u8]) -> Vec<i32> {
    let mut keys = [0f64; 12];
    for i in 0..10 {
        keys[i + 1] = bands[i] as f64 * 19.0 / 64.0;
    }
    keys[0] = keys[1];
    keys[11] = keys[10];
    (0..109)
        .map(|x| (tcb(&keys, 1.0 + x as f64 / 12.0) as i32).clamp(0, 18))
        .collect()
}

fn draw_graph(f: &mut Frame, bmp: &Frame, values: &[u8; 11]) {
    blit(f, bmp, 0, 294, 113, 19, 86, 17);
    let preamp_y = 35 - (values[0] as f64 * 19.0 / 64.0) as i32;
    blit(f, bmp, 0, 314, 113, 1, 86, preamp_y);
    let rows = graph_rows(&values[1..]);
    let mut prev: Option<i32> = None;
    for (x, &y) in rows.iter().enumerate() {
        let col = 86 + 2 + x as i32;
        let (lo, hi) = match prev {
            Some(p) if p != y => (p.min(y), p.max(y)),
            _ => (y, y),
        };
        for row in lo..=hi {
            f.set(col, 17 + row, bmp.pixel(115, 294 + row));
        }
        prev = Some(y);
    }
}

/// Windowshade volume thumb: x position and sprite index (03 §1.6).
pub fn shade_volume_geometry(volume: u8) -> (i32, i32) {
    let r = 63 + 94 * volume as i32 / 255;
    (r - 2, volume as i32 * 3 / 256)
}

/// Windowshade balance thumb: x position and sprite index.
pub fn shade_balance_geometry(balance: i8) -> (i32, i32) {
    let b = balance as i32 + 128;
    let r = 166 + 39 * b / 255;
    (r - 2, b * 3 / 256)
}

fn draw_shade(skin: &Skin, v: &EqView) -> Frame {
    let ex = skin.bitmap(B::EqEx);
    let mut f = Frame::new(EQ_SHADE_SIZE.0, EQ_SHADE_SIZE.1, 0);
    blit(&mut f, ex, 0, if v.active { 0 } else { 15 }, 275, 14, 0, 0);
    let (vx, vk) = shade_volume_geometry(v.volume);
    blit(&mut f, ex, 1 + 3 * vk, 30, 3, 7, vx, 4);
    let (bx, bk) = shade_balance_geometry(v.balance);
    blit(&mut f, ex, 11 + 3 * bk, 30, 3, 7, bx, 4);
    if v.pressed == Some(EqButton::Shade) {
        blit(&mut f, ex, 1, 47, 9, 9, 254, 3);
    }
    let close_y = if v.pressed == Some(EqButton::Close) {
        47
    } else {
        38
    };
    blit(&mut f, ex, 11, close_y, 9, 9, 264, 3);
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{self, check_rect};

    fn ok(r: Result<(), String>) {
        if let Err(e) = r {
            panic!("{e}");
        }
    }

    #[test]
    fn slider_math() {
        assert_eq!(slider_frame(0), 27);
        assert_eq!(slider_frame(63), 0);
        assert_eq!(thumb_y(0), 38);
        assert_eq!(thumb_y(63), 89);
        assert_eq!(shade_volume_geometry(255), (155, 2));
        assert_eq!(shade_balance_geometry(-127).1, 0);
        assert_eq!(shade_balance_geometry(127).1, 2);
    }

    #[test]
    fn sprites_at_spec_positions() {
        let skin = probe::skin(&[]);
        let v = EqView {
            enabled: true,
            values: [0, 63, 31, 31, 31, 31, 31, 31, 31, 31, 31],
            dragging: Some(1),
            ..EqView::default()
        };
        let f = draw_eq(&skin, &v);
        ok(check_rect(&f, B::Eqmain, 0, 134, 275, 14, 0, 0));
        ok(check_rect(&f, B::Eqmain, 69, 119, 25, 12, 14, 18));
        ok(check_rect(&f, B::Eqmain, 35, 119, 33, 12, 39, 18));
        ok(check_rect(&f, B::Eqmain, 224, 164, 44, 12, 217, 18));
        // Preamp at value 0: frame 27 (row 229, column 13), thumb at y 38.
        ok(check_rect(
            &f,
            B::Eqmain,
            13 + 13 * 15,
            229 + 62,
            14,
            1,
            21,
            38 + 62,
        ));
        ok(check_rect(&f, B::Eqmain, 0, 164, 11, 11, 22, 38));
        // Band 1 at 63: frame 0, pressed thumb at y 89.
        ok(check_rect(&f, B::Eqmain, 13, 164, 14, 1, 78, 38));
        ok(check_rect(&f, B::Eqmain, 0, 176, 11, 11, 79, 89));
    }

    #[test]
    fn flat_graph_is_a_line() {
        let rows = graph_rows(&[31; 10]);
        assert!(rows.iter().all(|&r| r == 9));
        let rows = graph_rows(&[0; 10]);
        assert!(rows.iter().all(|&r| r == 0));
        let rows = graph_rows(&[63; 10]);
        assert!(rows.iter().all(|&r| r == 18));
    }

    #[test]
    fn graph_pixels_use_row_colours() {
        let skin = probe::skin(&[]);
        let f = draw_eq(&skin, &EqView::default());
        // Flat: row 9, preamp 31 → line at y = 35 − 9 = 26.
        for x in 88..197 {
            ok(check_rect(&f, B::Eqmain, 115, 294 + 9, 1, 1, x, 26));
        }
        ok(check_rect(&f, B::Eqmain, 0, 314, 2, 1, 86, 26));
    }

    #[test]
    fn shade_mode() {
        let skin = probe::skin(&[]);
        let f = draw_eq(
            &skin,
            &EqView {
                shade: true,
                volume: 255,
                balance: 0,
                ..EqView::default()
            },
        );
        assert_eq!((f.width(), f.height()), (275, 14));
        ok(check_rect(&f, B::EqEx, 7, 30, 3, 7, 155, 4));
        ok(check_rect(&f, B::EqEx, 11, 38, 9, 9, 264, 3));
    }
}
