//! Renders every glyph of both faces and some sample UI strings into a PPM file for review.
//!
//! Usage: `cargo run -p nulamp-pixelfont --example specimen -- out.ppm`

use nulamp_pixelfont::PixelFont;
use nulamp_skin::Bitmap;
use std::io::Write;

const WIDTH: i32 = 720;
const PAPER: u32 = 0xD4D0C8;
const INK: u32 = 0x000000;
const GUIDE: u32 = 0xC4BFB6;
const DARK: u32 = 0x10141C;
const LIGHT: u32 = 0xE8E8E8;
const SAMPLES: [&str; 4] = [
    "About nulamp...   Ctrl+F1",
    "Playlist Editor   Alt+E",
    "Björk – Jóga (5:05) €12 ★★★☆☆",
    "The quick brown fox jumps over the lazy dog 0123456789",
];

const CELL_W: i32 = 15;
const CELL_H: i32 = 17;
const PER_ROW: i32 = 36;

fn main() -> std::io::Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "specimen.ppm".into());
    let faces = [
        ("Regular", PixelFont::regular()),
        ("Bold", PixelFont::bold()),
    ];
    let lh = faces[0].1.line_height();
    let grid_rows = |f: &PixelFont| (f.chars().count() as i32 + PER_ROW - 1) / PER_ROW;
    let mut height = 8;
    for (_, f) in faces {
        height += lh + 4 + grid_rows(f) * CELL_H + 8;
    }
    height += 2 * (SAMPLES.len() as i32 * lh + 8) + 8;
    height += 2 * (2 * lh * 2 + 8) + 8;
    let mut bmp = Bitmap::new(WIDTH as u32, height as u32, PAPER);
    let clip = (0, 0, WIDTH, height);
    let mut y = 8;

    // Glyph grids, with the baseline marked in each cell.
    for (name, f) in faces {
        f.draw(
            &mut bmp,
            &format!("{name}: {} glyphs, line height {lh}", f.chars().count()),
            8,
            y + f.ascent(),
            INK,
            clip,
        );
        y += lh + 4;
        for (i, c) in f.chars().enumerate() {
            let (col, row) = (i as i32 % PER_ROW, i as i32 / PER_ROW);
            let (cx, cy) = (8 + col * CELL_W, y + row * CELL_H);
            bmp.fill_rect(cx, cy + f.ascent(), CELL_W - 2, 1, GUIDE);
            f.draw(&mut bmp, &c.to_string(), cx + 1, cy + f.ascent(), INK, clip);
        }
        y += grid_rows(f) * CELL_H + 8;
    }

    // Sample strings at 1×, dark on light, then light on dark.
    for (bg, fg) in [(PAPER, INK), (DARK, LIGHT)] {
        bmp.fill_rect(0, y - 4, WIDTH, SAMPLES.len() as i32 * lh + 8, bg);
        for s in SAMPLES {
            let x = 8 + faces[0].1.draw(&mut bmp, s, 8, y + lh - 2, fg, clip);
            faces[1]
                .1
                .draw(&mut bmp, s, x.max(WIDTH / 2), y + lh - 2, fg, clip);
            y += lh;
        }
        y += 8;
    }
    y += 4;

    // Double size.
    for (_, f) in faces {
        for s in &SAMPLES[2..] {
            f.draw_scaled(&mut bmp, s, 8, y + f.ascent() * 2, INK, clip, 2);
            y += lh * 2;
        }
        y += 8;
    }

    let mut o = std::fs::File::create(out)?;
    write!(o, "P6\n{} {}\n255\n", bmp.width(), bmp.height())?;
    let bytes: Vec<u8> = bmp
        .pixels()
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    o.write_all(&bytes)
}
