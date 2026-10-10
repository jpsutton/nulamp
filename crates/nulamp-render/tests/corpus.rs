//! Skin corpus run (12 §1, §3). Skipped unless `NULAMP_SKIN_CORPUS` names a folder of skins
//! (`.wsz`/`.zip` files or skin folders). Every skin must load and draw every window without
//! panicking, and its regions must rasterize. With `NULAMP_REFERENCE_DIR`, the main window in
//! the fixed state is compared with `<dir>/<skin name>/main.png` (a screenshot of the reference
//! player), ignoring the song title box.

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use nulamp_render::eq_window::{EqView, draw_eq};
use nulamp_render::gen_window::{GenView, draw_gen_frame};
use nulamp_render::main_window::*;
use nulamp_render::playlist_window::{PlFrameView, draw_playlist_frame};
use nulamp_skin::{Skin, SkinFiles};

fn skins(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| {
        p.is_dir()
            || p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wsz") || e.eq_ignore_ascii_case("zip"))
    });
    v.sort();
    v
}

fn fixed() -> MainView {
    MainView {
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
        seek: Some(83 * 256 / 225),
        shuffle: true,
        ..MainView::default()
    }
}

fn exercise(skin: &Skin) {
    let v = fixed();
    let _ = draw_main(skin, &v);
    let _ = draw_main(
        skin,
        &MainView {
            shade: true,
            ..v.clone()
        },
    );
    let _ = draw_eq(skin, &EqView::default());
    let _ = draw_eq(
        skin,
        &EqView {
            shade: true,
            ..EqView::default()
        },
    );
    let _ = draw_playlist_frame(skin, &PlFrameView::default());
    let _ = draw_gen_frame(
        skin,
        &GenView {
            w: 275,
            h: 116,
            title: "Media Library".into(),
            active: true,
            close_pressed: false,
            fixed_size: false,
        },
    );
    let r = &skin.regions;
    for (polys, w, h) in [
        (&r.normal, 275, 116),
        (&r.window_shade, 275, 14),
        (&r.equalizer, 275, 116),
        (&r.equalizer_ws, 275, 14),
    ] {
        if let Some(p) = polys {
            let _ = nulamp_render::region::rasterize(p, w, h, 1);
        }
    }
}

/// Pixels that differ from the reference, outside the title box (111,24)–(265,36).
fn compare(reference: &Path, skin: &Skin) -> Option<usize> {
    let (w, h, px) = common::read_png(reference)?;
    let ours = draw_main(skin, &fixed());
    if (w, h) != (ours.width(), ours.height()) {
        return Some(usize::MAX);
    }
    let differing = (0..h as i32)
        .flat_map(|y| (0..w as i32).map(move |x| (x, y)))
        .filter(|&(x, y)| !((111..265).contains(&x) && (24..36).contains(&y)))
        .filter(|&(x, y)| px[(y * w as i32 + x) as usize] != ours.pixel(x, y))
        .count();
    Some(differing)
}

#[test]
fn corpus() {
    let Some(dir) = std::env::var_os("NULAMP_SKIN_CORPUS").map(PathBuf::from) else {
        eprintln!("NULAMP_SKIN_CORPUS not set: skipped");
        return;
    };
    let reference = std::env::var_os("NULAMP_REFERENCE_DIR").map(PathBuf::from);
    let defaults = nulamp_skin::builtin::files();
    let mut failures = Vec::new();
    let mut compared = 0;
    let list = skins(&dir);
    for path in &list {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let files = SkinFiles::open(path).map_err(|e| e.to_string())?;
            let skin = Skin::load(&files, &defaults);
            exercise(&skin);
            Ok::<Skin, String>(skin)
        }));
        let skin = match result {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                // Unreadable archives are reported but are not engine failures.
                eprintln!("{name}: cannot open: {e}");
                continue;
            }
            Err(_) => {
                failures.push(format!("{name}: panicked"));
                continue;
            }
        };
        if let Some(r) = &reference {
            let png = r.join(&name).join("main.png");
            if let Some(n) = compare(&png, &skin) {
                compared += 1;
                if n != 0 {
                    failures.push(format!("{name}: {n} pixels differ from the reference"));
                }
            }
        }
    }
    eprintln!(
        "{} skins, {compared} compared with references, {} failures",
        list.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
