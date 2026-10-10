//! Equalizer bitmaps of the built-in skin (layouts from spec 03).

use crate::canvas::{Canvas, lerp, pal};

fn eq_title(c: &mut Canvas, y: i32, active: bool, label: &str) {
    let (a, b, ink) = if active {
        (pal::TITLE_A, pal::TITLE_B, pal::TITLE_TEXT)
    } else {
        (pal::TITLE_IN_A, pal::TITLE_IN_B, pal::TITLE_TEXT_IN)
    };
    c.vgrad(0, y, 275, 14, a, b);
    c.hline(0, y + 13, 275, pal::SHADOW);
    c.text_centered(0, y, 275, 14, label, ink);
}

fn small_x(c: &mut Canvas, x: i32, y: i32, pressed: bool) {
    c.button(x, y, 9, 9, pressed);
    let ink = if pressed {
        pal::LABEL_DIM
    } else {
        pal::TITLE_TEXT
    };
    for i in 0..5 {
        c.px(x + 2 + i, y + 2 + i, ink);
        c.px(x + 6 - i, y + 2 + i, ink);
    }
}

fn small_shade(c: &mut Canvas, x: i32, y: i32, pressed: bool, unshade: bool) {
    c.button(x, y, 9, 9, pressed);
    let ink = if pressed {
        pal::LABEL_DIM
    } else {
        pal::TITLE_TEXT
    };
    if unshade {
        c.rect(x + 2, y + 2, 5, 5, ink);
        c.rect(x + 3, y + 4, 3, 2, pal::FACE_DARK);
    } else {
        c.hline(x + 2, y + 2, 5, ink);
        c.hline(x + 2, y + 3, 5, ink);
    }
}

/// Centre frequencies for the band labels under the sliders.
const WINAMP_LABELS: [&str; 10] = [
    "60", "170", "310", "600", "1K", "3K", "6K", "12K", "14K", "16K",
];
const ISO_LABELS: [&str; 10] = [
    "31", "63", "125", "250", "500", "1K", "2K", "4K", "8K", "16K",
];

/// `eqmain.bmp` / `eqmain_iso.bmp` 275×315.
pub fn eqmain(iso: bool) -> Canvas {
    let mut c = Canvas::new(275, 315, pal::FACE_DARK);
    // Window background (0,0) 275×116.
    c.vgrad(0, 0, 275, 116, pal::FACE, pal::FACE_DARK);
    c.bevel(0, 0, 275, 116, false);
    eq_title(&mut c, 0, true, "EQUALIZER");
    // Slider recesses and labels.
    let slider_x: Vec<i32> = std::iter::once(21)
        .chain((0..10).map(|i| 78 + i * 18))
        .collect();
    for &x in &slider_x {
        c.bevel(x - 1, 37, 16, 65, true);
    }
    let labels = if iso { ISO_LABELS } else { WINAMP_LABELS };
    for (i, label) in labels.iter().enumerate() {
        c.text_centered(78 + i as i32 * 18 - 3, 104, 20, 7, label, pal::LABEL);
    }
    c.text_centered(10, 104, 36, 7, "PREAMP", pal::LABEL);
    c.text(45, 35, "+12DB", pal::LABEL);
    c.text(49, 67, "0DB", pal::LABEL);
    c.text(45, 94, "-12DB", pal::LABEL);
    // Graph recess.
    c.bevel(85, 16, 115, 21, true);

    // Close button normal / pressed.
    small_x(&mut c, 0, 116, false);
    small_x(&mut c, 0, 125, true);
    // ON and AUTO buttons: off, on, off+pressed, on+pressed.
    for (i, (on, pressed)) in [(false, false), (true, false), (false, true), (true, true)]
        .into_iter()
        .enumerate()
    {
        let dx = i as i32 * 59;
        for (x, w, label) in [(10, 25, "ON"), (35, 33, "AUTO")] {
            c.button(x + dx, 119, w, 12, pressed);
            c.rect(
                x + dx + 3,
                124,
                2,
                2,
                if on { pal::LED_ON } else { pal::LED_OFF },
            );
            c.text(
                x + dx + 8,
                123,
                label,
                if on { pal::TITLE_TEXT } else { pal::LABEL },
            );
        }
    }
    // Title bars (y 134 active, 149 inactive), with the shade and close buttons drawn in.
    for (y, active) in [(134, true), (149, false)] {
        eq_title(&mut c, y, active, "EQUALIZER");
        small_shade(&mut c, 254, y + 3, false, false);
        small_x(&mut c, 264, y + 3, false);
    }
    // Copy the active title bar into the window background.
    c.copy(0, 134, 275, 14, 0, 0);
    // Slider thumbs (0,164) normal / (0,176) pressed, 11×11.
    for (y, pressed) in [(164, false), (176, true)] {
        c.button(0, y, 11, 11, pressed);
        c.hline(2, y + 5, 7, if pressed { pal::ACCENT } else { pal::LABEL });
    }
    // Slider background frames 0..27: frame n shows a fill proportional to n.
    for n in 0..28 {
        let (x, y) = if n < 14 {
            (13 + n * 15, 164)
        } else {
            (13 + (n - 14) * 15, 229)
        };
        c.rect(x, y, 14, 63, pal::SHADOW);
        c.vline(x + 6, y + 2, 59, pal::FACE_DARK);
        c.vline(x + 7, y + 2, 59, pal::FACE_DARK);
        let h = 1 + n * 58 / 27;
        for k in 0..h {
            let t = k as f32 / 58.0;
            let col = lerp(0x2E9E58, 0xE8B040, t);
            c.hline(x + 6, y + 61 - k, 2, col);
        }
    }
    // PRESETS button normal / pressed.
    for (y, pressed) in [(164, false), (176, true)] {
        c.button(224, y, 44, 12, pressed);
        c.text_centered(
            224 + pressed as i32,
            y + pressed as i32,
            44,
            12,
            "PRESETS",
            pal::TITLE_TEXT,
        );
    }
    // Graph background (0,294) 113×19 with grid.
    c.rect(0, 294, 113, 19, pal::LCD_BG);
    for gy in [294, 303, 312] {
        for gx in (0..113).step_by(2) {
            c.px(gx, gy, pal::LCD_DIM);
        }
    }
    // Preamp line (0,314) 113×1.
    c.hline(0, 314, 113, 0x8080A0);
    // Curve colours (115, 294..312): red at the top (+12 dB) to green at the bottom.
    for row in 0..19 {
        let t = row as f32 / 18.0;
        let col = if t < 0.5 {
            lerp(0xFF4030, 0xFFE040, t * 2.0)
        } else {
            lerp(0xFFE040, 0x40E060, (t - 0.5) * 2.0)
        };
        c.px(115, 294 + row, col);
    }
    c
}

/// `eq_ex.bmp` 275×82.
pub fn eq_ex() -> Canvas {
    let mut c = Canvas::new(275, 82, pal::FACE_DARK);
    for (y, active) in [(0, true), (15, false)] {
        let (a, b, ink) = if active {
            (pal::TITLE_A, pal::TITLE_B, pal::TITLE_TEXT)
        } else {
            (pal::TITLE_IN_A, pal::TITLE_IN_B, pal::TITLE_TEXT_IN)
        };
        c.vgrad(0, y, 275, 14, a, b);
        c.text(8, y + 5, "EQ", ink);
        // Volume groove 61..162 and balance groove 163..206.
        c.rect(61, y + 6, 101, 2, pal::SHADOW);
        c.rect(166, y + 6, 40, 2, pal::SHADOW);
        small_shade(&mut c, 254, y + 3, false, true);
        small_x(&mut c, 264, y + 3, false);
    }
    // Mini thumbs 3×7: volume (1+3k,30), balance (11+3k,30).
    for k in 0..3 {
        c.rect(
            1 + 3 * k,
            30,
            3,
            7,
            lerp(pal::ACCENT_DARK, pal::ACCENT, k as f32 / 2.0),
        );
        c.rect(
            11 + 3 * k,
            30,
            3,
            7,
            lerp(0x806020, 0xE8B040, k as f32 / 2.0),
        );
    }
    // Pressed shade button for normal mode (1,38), close normal/pressed, unshade pressed.
    small_shade(&mut c, 1, 38, true, false);
    small_x(&mut c, 11, 38, false);
    small_x(&mut c, 11, 47, true);
    small_shade(&mut c, 1, 47, true, true);
    c
}
