//! Golden-image tests (12 §3) with the built-in skin in the fixed state of 12 §3: a track at
//! 1:23 of 3:45, 128 kbps, 44 kHz, stereo, volume 200, balance 0, shuffle on, repeat off.

mod common;

use nulamp_render::eq_window::{EqView, draw_eq};
use nulamp_render::gen_window::{GenView, draw_gen_frame};
use nulamp_render::main_window::*;
use nulamp_render::playlist_window::{MiniTime, PlFrameView, PopupMenu, draw_playlist_frame};
use nulamp_render::{Frame, blit, scale};
use nulamp_skin::Skin;

fn skin() -> Skin {
    nulamp_skin::builtin::skin()
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
        // 83 s of 225 s.
        seek: Some(83 * 256 / 225),
        shuffle: true,
        title: TitleArea::Bitmap {
            text: "3. NULAMP - KNOWN TRACK (3:45)  ***  ".into(),
            offset: 0,
        },
        ..MainView::default()
    }
}

/// Cuts `(x, y) w×h` out of each frame and stacks the pieces vertically.
fn strip(frames: &[Frame], x: i32, y: i32, w: i32, h: i32) -> Frame {
    let mut out = Frame::new(w as u32, (h as usize * frames.len()) as u32, 0);
    for (i, f) in frames.iter().enumerate() {
        blit(&mut out, f, x, y, w, h, 0, h * i as i32);
    }
    out
}

#[test]
fn main_window_states() {
    let s = skin();
    common::check("main-fixed", &draw_main(&s, &fixed()));
    common::check("main-stopped", &draw_main(&s, &MainView::default()));
    common::check(
        "main-shade",
        &draw_main(
            &s,
            &MainView {
                shade: true,
                ..fixed()
            },
        ),
    );
    common::check(
        "main-inactive",
        &draw_main(
            &s,
            &MainView {
                active: false,
                ..fixed()
            },
        ),
    );
    common::check("main-double", &scale(&draw_main(&s, &fixed()), 2));
}

#[test]
fn time_display() {
    let s = skin();
    let times = [
        (0, 0, false),
        (9, 59, false),
        (99, 59, false),
        (100, 0, false),
        (1, 23, true),
    ];
    let mut frames: Vec<Frame> = times
        .iter()
        .map(|&(minutes, seconds, remaining)| {
            draw_main(
                &s,
                &MainView {
                    time: TimeDisplay::Time {
                        minutes,
                        seconds,
                        remaining,
                    },
                    ..fixed()
                },
            )
        })
        .collect();
    frames.push(draw_main(
        &s,
        &MainView {
            time: TimeDisplay::Blank,
            state: PlayState::Paused,
            ..fixed()
        },
    ));
    common::check("time-display", &strip(&frames, 24, 24, 78, 18));
}

#[test]
fn volume_and_balance() {
    let s = skin();
    let vol: Vec<Frame> = [0u8, 1, 128, 254, 255]
        .iter()
        .map(|&volume| draw_main(&s, &MainView { volume, ..fixed() }))
        .collect();
    common::check("volume", &strip(&vol, 107, 57, 68, 13));
    let bal: Vec<Frame> = [-127i8, -13, 0, 13, 127]
        .iter()
        .map(|&balance| draw_main(&s, &MainView { balance, ..fixed() }))
        .collect();
    common::check("balance", &strip(&bal, 177, 57, 38, 13));
}

#[test]
fn kbps_and_channels() {
    let s = skin();
    let kbps: Vec<Frame> = [8u32, 64, 128, 320, 1411, 9999, 10000]
        .iter()
        .map(|&k| {
            draw_main(
                &s,
                &MainView {
                    kbps: Some(k),
                    ..fixed()
                },
            )
        })
        .collect();
    common::check("kbps", &strip(&kbps, 111, 43, 15, 6));
    let ch: Vec<Frame> = [1u32, 2, 6]
        .iter()
        .map(|&c| {
            draw_main(
                &s,
                &MainView {
                    channels: Some(c),
                    ..fixed()
                },
            )
        })
        .collect();
    common::check("channels", &strip(&ch, 212, 41, 56, 12));
}

#[test]
fn equalizer() {
    let s = skin();
    // A fixed curve: a smile with the preamp slightly up.
    let v = EqView {
        enabled: true,
        values: [25, 12, 20, 28, 34, 38, 38, 34, 28, 20, 12],
        ..EqView::default()
    };
    common::check("eq", &draw_eq(&s, &v));
    common::check("eq-shade", &draw_eq(&s, &EqView { shade: true, ..v }));
}

#[test]
fn playlist_frames() {
    let s = skin();
    let base = PlFrameView {
        info: "1:23/45:00".into(),
        time: MiniTime::Time {
            minutes: 1,
            seconds: 23,
            remaining: false,
        },
        ..PlFrameView::default()
    };
    for (w, h) in [(275, 116), (300, 145), (450, 290)] {
        common::check(
            &format!("playlist-{w}x{h}"),
            &draw_playlist_frame(
                &s,
                &PlFrameView {
                    w,
                    h,
                    ..base.clone()
                },
            ),
        );
    }
    common::check(
        "playlist-shade",
        &draw_playlist_frame(
            &s,
            &PlFrameView {
                shade: true,
                h: 14,
                ..base.clone()
            },
        ),
    );
    for menu in [
        PopupMenu::Add,
        PopupMenu::Rem,
        PopupMenu::Sel,
        PopupMenu::Misc,
        PopupMenu::List,
    ] {
        let frames: Vec<Frame> = (0..menu.item_count())
            .map(|i| {
                draw_playlist_frame(
                    &s,
                    &PlFrameView {
                        popup: Some((menu, Some(i))),
                        ..base.clone()
                    },
                )
            })
            .collect();
        common::check(
            &format!("popup-{menu:?}").to_lowercase(),
            &strip(&frames, 0, 116 - 106, 275, 106),
        );
    }
}

#[test]
fn generic_window_titles() {
    let mut s = skin();
    let v = GenView {
        w: 275,
        h: 116,
        title: "Media Library 2.0".into(),
        active: true,
        close_pressed: false,
        fixed_size: false,
    };
    s.pledit.use_gen_nums = false;
    let off = draw_gen_frame(&s, &v);
    s.pledit.use_gen_nums = true;
    let on = draw_gen_frame(&s, &v);
    common::check("gen-title", &strip(&[off, on], 0, 0, 275, 20));
}

#[test]
fn region_mask() {
    let regions = nulamp_skin::region::Regions::parse(
        "[Normal]\nNumPoints=4, 3\nPointList=0,0, 275,0, 275,116, 0,116, 100,40, 137,90, 175,40\n",
    );
    let polys = regions.normal.expect("valid section");
    let mask = nulamp_render::region::rasterize(&polys, 275, 116, 1);
    let mut f = Frame::new(275, 116, 0);
    for y in 0..116 {
        for x in 0..275 {
            if mask.get(x, y) {
                f.set(x, y, 0xffffff);
            }
        }
    }
    common::check("region-mask", &f);
}

#[test]
fn png_round_trip() {
    let f = draw_main(&skin(), &fixed());
    let path = common::diff_dir().join("round-trip.png");
    common::write_png(&path, &f);
    let (w, h, px) = common::read_png(&path).expect("readable");
    assert_eq!((w, h), (275, 116));
    assert_eq!(px, f.pixels());
}

/// The visualizer frozen on one frame of a known input (12 §3): a sum of three sines.
#[test]
fn frozen_visualizer() {
    use nulamp_vis::*;
    let s = skin();
    let mut input = [0f32; FFT_LEN];
    for (i, x) in input.iter_mut().enumerate() {
        let t = i as f32 / 44_100.0;
        *x = 0.5 * (std::f32::consts::TAU * 110.0 * t).sin()
            + 0.25 * (std::f32::consts::TAU * 1_000.0 * t).sin()
            + 0.125 * (std::f32::consts::TAU * 8_000.0 * t).sin();
    }
    let bars = Spectrum::new().compute(&input);
    let scope = scope_values(&input[..BARS * 2], 2);
    let data = VisData { bars, scope };
    let mut frames = Vec::new();
    for mode in [VisMode::Spectrum, VisMode::Oscilloscope] {
        let mut v = Visualizer::new(VisSettings {
            mode,
            ..VisSettings::default()
        });
        v.tick(Some(&data));
        let mut f = Frame::new(76, 16, 0);
        v.draw(&s.vis_colors, &mut f, 0, 0);
        frames.push(f);
    }
    common::check("vis-frozen", &scale(&strip(&frames, 0, 0, 76, 16), 2));
}
