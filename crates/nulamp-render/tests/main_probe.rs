//! Checks main-window sprite coordinates against spec 02 using probe skins.

use nulamp_render::main_window::*;
use nulamp_render::probe::{self, check_rect};
use nulamp_skin::SkinBitmap as B;

fn ok(r: Result<(), String>) {
    if let Err(e) = r {
        panic!("{e}");
    }
}

#[test]
fn stopped_main_window() {
    let skin = probe::skin(&[B::NumsEx]);
    let v = MainView::default();
    let f = draw_main(&skin, &v);
    // main.bmp shows where nothing dynamic is drawn (time area, kbps).
    ok(check_rect(&f, B::Main, 36, 26, 60, 13, 36, 26));
    ok(check_rect(&f, B::Main, 111, 43, 15, 6, 111, 43));
    ok(check_rect(&f, B::Main, 16, 72, 248, 10, 16, 72));
    // Title bar strip, active.
    ok(check_rect(&f, B::Titlebar, 27, 0, 275, 14, 0, 0));
    // Clutterbar shown, with A and D overlays off.
    ok(check_rect(&f, B::Titlebar, 304, 0, 8, 11, 10, 22));
    ok(check_rect(&f, B::Titlebar, 305, 11, 7, 8, 11, 33));
    ok(check_rect(&f, B::Titlebar, 305, 27, 7, 6, 11, 49));
    // Stop icon and 2-pixel strip.
    ok(check_rect(&f, B::Playpaus, 18, 0, 9, 9, 26, 28));
    ok(check_rect(&f, B::Playpaus, 27, 0, 2, 9, 24, 28));
    // Mono and stereo unlit.
    ok(check_rect(&f, B::Monoster, 0, 12, 29, 12, 239, 41));
    ok(check_rect(&f, B::Monoster, 29, 12, 27, 12, 212, 41));
    // Volume 200: frame 21, thumb x 40.
    ok(check_rect(&f, B::Volume, 0, 21 * 15, 68, 1, 107, 57));
    ok(check_rect(&f, B::Volume, 15, 422, 14, 11, 147, 58));
    // Balance 0: frame 0 from x 9, thumb at 12.
    ok(check_rect(&f, B::Balance, 9, 0, 38, 1, 177, 57));
    ok(check_rect(&f, B::Balance, 15, 422, 14, 11, 189, 58));
    // Transport and eject unpressed.
    ok(check_rect(&f, B::Cbuttons, 0, 0, 23, 18, 16, 88));
    ok(check_rect(&f, B::Cbuttons, 92, 0, 22, 18, 108, 88));
    ok(check_rect(&f, B::Cbuttons, 114, 0, 22, 16, 136, 89));
    // Shuffle/repeat off; EQ and PL on.
    ok(check_rect(&f, B::Shufrep, 28, 0, 46, 15, 164, 89));
    ok(check_rect(&f, B::Shufrep, 0, 0, 28, 15, 210, 89));
    ok(check_rect(&f, B::Shufrep, 0, 73, 23, 12, 219, 58));
    ok(check_rect(&f, B::Shufrep, 23, 73, 23, 12, 242, 58));
}

#[test]
fn playing_state_and_pressed_sprites() {
    let skin = probe::skin(&[B::NumsEx]);
    let v = MainView {
        active: false,
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
        seek: Some(128),
        seek_pressed: true,
        volume_pressed: true,
        shuffle: true,
        pressed: Some(MainButton::Play),
        clutterbar_pressed: Some(1),
        always_on_top: true,
        double_size: true,
        ..MainView::default()
    };
    let f = draw_main(&skin, &v);
    ok(check_rect(&f, B::Titlebar, 27, 15, 275, 14, 0, 0));
    // Clutterbar: A pressed frame; A overlay suppressed; D overlay "on".
    ok(check_rect(&f, B::Titlebar, 312, 44, 8, 11, 10, 22));
    ok(check_rect(&f, B::Titlebar, 312 + 1, 44 + 11, 7, 8, 11, 33));
    ok(check_rect(&f, B::Titlebar, 329, 71, 7, 6, 11, 49));
    // Play icon with green light overwriting column 26.
    ok(check_rect(&f, B::Playpaus, 36, 0, 3, 9, 24, 28));
    ok(check_rect(&f, B::Playpaus, 1, 0, 8, 9, 27, 28));
    // Time 01:23 with numbers.bmp: minus line cleared from (9,6).
    ok(check_rect(&f, B::Numbers, 0, 0, 9, 13, 48, 26));
    ok(check_rect(&f, B::Numbers, 9, 0, 9, 13, 60, 26));
    ok(check_rect(&f, B::Numbers, 18, 0, 9, 13, 78, 26));
    ok(check_rect(&f, B::Numbers, 27, 0, 9, 13, 90, 26));
    ok(check_rect(&f, B::Numbers, 9, 6, 5, 1, 40, 32));
    // kbps "128" and kHz "44" from text.bmp row 1.
    ok(check_rect(&f, B::Text, 5, 6, 5, 6, 111, 43));
    ok(check_rect(&f, B::Text, 10, 6, 5, 6, 116, 43));
    ok(check_rect(&f, B::Text, 40, 6, 5, 6, 121, 43));
    ok(check_rect(&f, B::Text, 20, 6, 5, 6, 156, 43));
    // Stereo lit.
    ok(check_rect(&f, B::Monoster, 0, 0, 29, 12, 239, 41));
    // Seek bar and pressed thumb at 128 * 219 / 256 = 109.
    ok(check_rect(&f, B::Posbar, 0, 0, 109, 10, 16, 72));
    ok(check_rect(&f, B::Posbar, 278, 0, 29, 10, 125, 72));
    // Pressed volume thumb.
    ok(check_rect(&f, B::Volume, 0, 422, 14, 11, 147, 58));
    // Play pressed, shuffle on.
    ok(check_rect(&f, B::Cbuttons, 23, 18, 23, 18, 39, 88));
    ok(check_rect(&f, B::Shufrep, 28, 30, 46, 15, 164, 89));
}

#[test]
fn time_display_variants() {
    let numbers = probe::skin(&[B::NumsEx]);
    let nums_ex = probe::skin(&[]);
    assert!(nums_ex.nums_ex && !numbers.nums_ex);
    let view = |time| MainView {
        state: PlayState::Playing,
        time,
        ..MainView::default()
    };

    // Remaining mode.
    let rem = TimeDisplay::Time {
        minutes: 9,
        seconds: 59,
        remaining: true,
    };
    let f = draw_main(&numbers, &view(rem));
    ok(check_rect(&f, B::Numbers, 20, 6, 5, 1, 40, 32));
    let f = draw_main(&nums_ex, &view(rem));
    ok(check_rect(&f, B::NumsEx, 99, 0, 9, 13, 36, 26));
    ok(check_rect(&f, B::NumsEx, 81, 0, 9, 13, 60, 26));

    // 100:00 shows the hundreds digit at 36.
    let f = draw_main(
        &numbers,
        &view(TimeDisplay::Time {
            minutes: 100,
            seconds: 0,
            remaining: false,
        }),
    );
    ok(check_rect(&f, B::Numbers, 9, 0, 9, 13, 36, 26));
    ok(check_rect(&f, B::Numbers, 0, 0, 9, 13, 48, 26));

    // Blank state; nums_ex quirk puts the first blank at x = 38.
    let f = draw_main(&numbers, &view(TimeDisplay::Blank));
    for x in [48, 60, 78, 90] {
        ok(check_rect(&f, B::Numbers, 90, 0, 9, 13, x, 26));
    }
    ok(check_rect(&f, B::Numbers, 9, 6, 5, 1, 40, 32));
    let f = draw_main(&nums_ex, &view(TimeDisplay::Blank));
    ok(check_rect(&f, B::NumsEx, 90, 0, 9, 13, 38, 26));
    ok(check_rect(&f, B::Main, 36, 26, 2, 13, 36, 26));
}

#[test]
fn balance_without_bitmap_uses_volume() {
    let skin = probe::skin(&[B::Balance]);
    let v = MainView {
        balance: -127,
        ..MainView::default()
    };
    let f = draw_main(&skin, &v);
    ok(check_rect(&f, B::Volume, 9, 27 * 15, 38, 1, 177, 57));
    ok(check_rect(&f, B::Volume, 15, 422, 14, 11, 177, 58));
}

#[test]
fn short_volume_bitmap_draws_black_thumb() {
    let files = vec![(
        "volume.bmp".to_string(),
        nulamp_skin::bmp::encode_24(&probe::bitmap(B::Volume, Some((68, 420)))),
    )];
    let sf = nulamp_skin::SkinFiles::from_entries(nulamp_skin::ContainerKind::Archive, files);
    let skin = nulamp_skin::Skin::load(&sf, &nulamp_skin::builtin::files());
    let f = draw_main(&skin, &MainView::default());
    for y in 58..69 {
        for x in 147..161 {
            assert_eq!(f.pixel(x, y), 0, "({x},{y})");
        }
    }
}

#[test]
fn bitmap_title_marquee() {
    let skin = probe::skin(&[]);
    let v = MainView {
        title: TitleArea::Bitmap {
            text: "AB".into(),
            offset: 2,
        },
        ..MainView::default()
    };
    let f = draw_main(&skin, &v);
    // 'A' scrolled by 2: columns 2..4 of the A cell at x 111..113.
    ok(check_rect(&f, B::Text, 2, 0, 3, 6, 111, 27));
    ok(check_rect(&f, B::Text, 5, 0, 5, 6, 114, 27));
    // Remaining area filled with column (4,0).
    for x in 119..265 {
        ok(check_rect(&f, B::Text, 4, 0, 1, 6, x, 27));
    }
    ok(check_rect(&f, B::Main, 265, 27, 1, 6, 265, 27));

    // Clip at 265: 31 characters, the last one starts at 261.
    let long: String = std::iter::repeat_n('A', 40).collect();
    let v = MainView {
        title: TitleArea::Bitmap {
            text: long,
            offset: 0,
        },
        ..MainView::default()
    };
    let f = draw_main(&skin, &v);
    ok(check_rect(&f, B::Text, 0, 0, 4, 6, 261, 27));
    ok(check_rect(&f, B::Main, 265, 27, 1, 6, 265, 27));
}

#[test]
fn shade_mode() {
    let skin = probe::skin(&[]);
    let v = MainView {
        shade: true,
        state: PlayState::Playing,
        time: TimeDisplay::Time {
            minutes: 12,
            seconds: 34,
            remaining: true,
        },
        seek: Some(256),
        pressed: Some(MainButton::Shade),
        ..MainView::default()
    };
    let f = draw_main(&skin, &v);
    assert_eq!((f.width(), f.height()), (275, 14));
    ok(check_rect(&f, B::Titlebar, 27, 29, 120, 4, 0, 0));
    ok(check_rect(&f, B::Titlebar, 9, 27, 9, 9, 254, 3));
    // Minus at 130 (minutes < 100), digits 1 2 3 4.
    ok(check_rect(&f, B::Text, 142, 0, 3, 6, 126, 4));
    ok(check_rect(&f, B::Text, 75, 6, 3, 6, 130, 4));
    ok(check_rect(&f, B::Text, 5, 6, 5, 6, 134, 4));
    ok(check_rect(&f, B::Text, 10, 6, 5, 6, 139, 4));
    ok(check_rect(&f, B::Text, 15, 6, 5, 6, 147, 4));
    ok(check_rect(&f, B::Text, 20, 6, 5, 6, 152, 4));
    // Mini seek bar: groove and the thumb at p = 13 from (23,36).
    ok(check_rect(&f, B::Titlebar, 0, 36, 13, 7, 226, 4));
    ok(check_rect(&f, B::Titlebar, 23, 36, 3, 7, 239, 4));
}

#[test]
fn builtin_skin_renders() {
    let skin = nulamp_skin::builtin::skin();
    let f = draw_main(&skin, &MainView::default());
    assert_eq!((f.width(), f.height()), (275, 116));
}
