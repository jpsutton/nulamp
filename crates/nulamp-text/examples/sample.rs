//! Renders sample lines at several sizes into a PPM file for visual inspection.
//!
//! Usage: `cargo run -p nulamp-text --example sample -- out.ppm [family]`

use nulamp_skin::Bitmap;
use nulamp_text::{Direction, FontSpec, FontSystem};
use std::io::Write;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "sample.ppm".into());
    let family = args.next().unwrap_or_default();
    let mut fs = FontSystem::new();
    let lines = [
        "1. Daft Punk - One More Time (5:20)",
        "12. Björk - Jóga (5:05)   Ünïcödé — ©",
        "3. 坂本龍一 - Merry Christmas Mr. Lawrence",
        "4. שלום עולם - Hebrew mixed (3:00)",
    ];
    let mut f = Bitmap::new(300, 120, 0x000000);
    let mut y = 0;
    for px in [10, 12] {
        let spec = FontSpec::new(family.clone(), px);
        let m = fs.metrics(&spec);
        for line in lines {
            let l = fs.layout(line, &spec, Direction::Auto);
            fs.draw(&mut f, &l, 2, y + m.ascent, 0x00FF00, (0, 0, 300, 120));
            y += m.line_height;
        }
    }
    let mut o = std::fs::File::create(out)?;
    write!(o, "P6\n{} {}\n255\n", f.width(), f.height())?;
    let bytes: Vec<u8> = f
        .pixels()
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    o.write_all(&bytes)
}
