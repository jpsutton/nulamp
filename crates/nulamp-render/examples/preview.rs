//! Writes preview images (PPM) of the windows drawn with a skin.
//!
//! Usage: `cargo run -p nulamp-render --example preview -- <out dir> [skin.wsz]`

use nulamp_render::main_window::*;
use nulamp_render::{Frame, scale};
use std::io::Write;

fn write_ppm(path: &std::path::Path, f: &Frame) -> std::io::Result<()> {
    let mut out = std::fs::File::create(path)?;
    write!(out, "P6\n{} {}\n255\n", f.width(), f.height())?;
    let bytes: Vec<u8> = f
        .pixels()
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    out.write_all(&bytes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let skin = match args.next() {
        Some(p) => nulamp_skin::Skin::load(
            &nulamp_skin::SkinFiles::open(std::path::Path::new(&p))?,
            &nulamp_skin::builtin::files(),
        ),
        None => nulamp_skin::builtin::skin(),
    };
    std::fs::create_dir_all(&dir)?;
    let stopped = MainView::default();
    let playing = MainView {
        state: PlayState::Playing,
        in_sync: true,
        time: TimeDisplay::Time {
            minutes: 1,
            seconds: 23,
            remaining: false,
        },
        kbps: Some(128),
        khz: Some(44),
        channels: Some(2),
        seek: Some(94),
        seek_buffered: Some((0, 170)),
        shuffle: true,
        title: TitleArea::Bitmap {
            text: "3. NULAMP - TEST TRACK (3:45)".into(),
            offset: 0,
        },
        ..MainView::default()
    };
    write_ppm(
        &dir.join("main-stopped.ppm"),
        &scale(&draw_main(&skin, &stopped), 2),
    )?;
    write_ppm(
        &dir.join("main-playing.ppm"),
        &scale(&draw_main(&skin, &playing), 2),
    )?;
    let shade = MainView {
        shade: true,
        ..playing.clone()
    };
    write_ppm(
        &dir.join("main-shade.ppm"),
        &scale(&draw_main(&skin, &shade), 2),
    )?;
    println!("wrote previews to {}", dir.display());
    Ok(())
}
