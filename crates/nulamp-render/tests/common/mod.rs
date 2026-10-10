//! Golden-image helpers (12 §3). Goldens live in `tests/golden/` at the workspace root and are
//! produced by nulamp itself; `NULAMP_BLESS=1` rewrites them. Mismatches write the actual image
//! to `target/golden-diff/` for inspection.

#![allow(dead_code)]

use std::path::PathBuf;

use nulamp_render::Frame;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn write_png(path: &std::path::Path, f: &Frame) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), f.width(), f.height());
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    let bytes: Vec<u8> = f
        .pixels()
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    w.write_image_data(&bytes).unwrap();
}

/// Reads an RGB or RGBA PNG as 0x00RRGGBB pixels.
pub fn read_png(path: &std::path::Path) -> Option<(u32, u32, Vec<u32>)> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let n = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        _ => return None,
    };
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let px = buf[..info.buffer_size()]
        .chunks(n)
        .map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)
        .collect();
    Some((info.width, info.height, px))
}

/// Compares `f` with `tests/golden/<name>.png` pixel for pixel.
pub fn check(name: &str, f: &Frame) {
    let path = root().join("tests/golden").join(format!("{name}.png"));
    if std::env::var_os("NULAMP_BLESS").is_some() {
        write_png(&path, f);
        return;
    }
    let Some((w, h, px)) = read_png(&path) else {
        panic!(
            "missing golden {}: run the tests with NULAMP_BLESS=1",
            path.display()
        );
    };
    let differing = if (w, h) != (f.width(), f.height()) {
        usize::MAX
    } else {
        px.iter().zip(f.pixels()).filter(|(a, b)| a != b).count()
    };
    if differing != 0 {
        let out = root().join(format!("target/golden-diff/{name}.png"));
        write_png(&out, f);
        panic!(
            "{name}: {} differs from the golden ({} pixels); actual image: {}",
            if differing == usize::MAX {
                format!("size {}x{} vs {w}x{h}", f.width(), f.height())
            } else {
                "content".to_string()
            },
            if differing == usize::MAX {
                0
            } else {
                differing
            },
            out.display()
        );
    }
}
