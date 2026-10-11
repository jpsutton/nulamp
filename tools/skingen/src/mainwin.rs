//! Main-window bitmaps of the built-in skin (layouts from spec 02).

use crate::canvas::{Canvas, pal};

/// Transport icon kinds, shared with the playlist and shade-mode mini transports.
#[derive(Clone, Copy)]
pub enum Icon {
    Prev,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
}

/// Transport icons as hand-placed pixels, one set per size: vector shapes rasterised at 5–9 px
/// come out lopsided. `#` is ink.
fn glyph(kind: Icon, size: i32) -> &'static [&'static str] {
    use Icon::*;
    match (size, kind) {
        // Main window buttons (9 px).
        (9, Prev) => &[
            "##....#....#",
            "##...##...##",
            "##..###..###",
            "##.####.####",
            "############",
            "##.####.####",
            "##..###..###",
            "##...##...##",
            "##....#....#",
        ],
        (9, Next) => &[
            "#....#....##",
            "##...##...##",
            "###..###..##",
            "####.####.##",
            "############",
            "####.####.##",
            "###..###..##",
            "##...##...##",
            "#....#....##",
        ],
        (9, Play) => &[
            "#.......", "###.....", "#####...", "#######.", "########", "#######.", "#####...",
            "###.....", "#.......",
        ],
        (9, Pause) => &[
            "###..###", "###..###", "###..###", "###..###", "###..###", "###..###", "###..###",
            "###..###", "###..###",
        ],
        (9, Stop) => &[
            "........", ".#######", ".#######", ".#######", ".#######", ".#######", ".#######",
            ".#######", "........",
        ],
        (9, Eject) => &[
            "....#....",
            "...###...",
            "..#####..",
            ".#######.",
            "#########",
            ".........",
            "#########",
            "#########",
        ],
        // Status display (7 px).
        (7, Play) => &["#...", "##..", "###.", "####", "###.", "##..", "#..."],
        (7, Pause) => &[
            "##.##", "##.##", "##.##", "##.##", "##.##", "##.##", "##.##",
        ],
        (7, Stop) => &[
            ".....", "#####", "#####", "#####", "#####", "#####", ".....",
        ],
        // Playlist mini transport and windowshade bar (5 px).
        (_, Prev) => &["#..#..#", "#.##.##", "#######", "#.##.##", "#..#..#"],
        (_, Next) => &["#..#..#", "##.##.#", "#######", "##.##.#", "#..#..#"],
        (_, Play) => &["#..", "##.", "###", "##.", "#.."],
        (_, Pause) => &["##.##", "##.##", "##.##", "##.##", "##.##"],
        (_, Stop) => &["####", "####", "####", "####"],
        (_, Eject) => &["..#..", ".###.", "#####", ".....", "#####"],
    }
}

/// Draws a transport icon of `size` (9, 7 or 5) centred in the box `(x, y) w×h`.
pub fn icon(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, kind: Icon, size: i32, ink: u32) {
    let rows = glyph(kind, size);
    let gw = rows.iter().map(|r| r.len()).max().unwrap_or(0) as i32;
    let gh = rows.len() as i32;
    let (ox, oy) = (x + (w - gw) / 2, y + (h - gh) / 2);
    for (dy, row) in rows.iter().enumerate() {
        for (dx, b) in row.bytes().enumerate() {
            if b == b'#' {
                c.px(ox + dx as i32, oy + dy as i32, ink);
            }
        }
    }
}

/// `main.bmp` 275×116.
pub fn main_bmp() -> Canvas {
    let mut c = Canvas::new(275, 116, pal::FACE);
    c.bevel(0, 0, 275, 116, false);
    // Left display: status, time, visualizer, on dotted display black. The dots sit on even
    // screen pixels, like the visualizer's grid and the dotted cells of numbers.bmp and
    // playpaus.bmp, so the pieces join without seams.
    c.lcd_dots(22, 23, 82, 39, 0, 0);
    c.bevel(21, 22, 84, 41, true);
    // Time colon (between minutes ones at x 60..68 and seconds tens at 78).
    for y in [30, 35] {
        c.rect(71, y, 2, 2, pal::LCD_FG);
    }
    // Song title box and info row.
    c.well(110, 24, 156, 12, pal::LCD_BG);
    c.well(110, 42, 17, 8, pal::LCD_BG);
    c.text(130, 43, "KBPS", pal::LABEL);
    c.well(155, 42, 13, 8, pal::LCD_BG);
    c.text(170, 43, "KHZ", pal::LABEL);
    // Mono/stereo and slider recesses.
    c.bevel(211, 40, 59, 14, true);
    c.bevel(106, 56, 70, 15, true);
    c.bevel(176, 56, 40, 15, true);
    c.bevel(218, 57, 48, 14, true);
    // Seek bar recess.
    c.bevel(15, 71, 250, 12, true);
    // Transport recess.
    c.bevel(15, 87, 116, 20, true);
    c.rect(16, 88, 114, 18, pal::LCD_BG);
    // About badge: the app icon.
    c.app_icon_13(253, 92);
    c
}

/// Draws one title strip (275×14) into the titlebar canvas.
fn title_strip(c: &mut Canvas, y: i32, active: bool, label: &str, shade: bool) {
    let x0 = 27;
    if shade {
        let bg = if active {
            pal::TITLE_BG
        } else {
            pal::TITLE_BG_IN
        };
        c.rect(x0, y, 275, 14, bg);
        c.hline(x0, y + 13, 275, pal::TITLE_LINE);
    } else {
        c.title_bar(x0, y, 275, active, label, x0 + 20, x0 + 240);
    }
}

fn small_button(c: &mut Canvas, x: i32, y: i32, pressed: bool, kind: char) {
    c.button(x, y, 9, 9, pressed);
    let ink = if pressed {
        pal::LABEL_DIM
    } else {
        pal::TITLE_TEXT
    };
    match kind {
        'm' => {
            for i in 0..3 {
                c.hline(x + 2, y + 2 + i * 2, 5, ink);
            }
        }
        '_' => c.hline(x + 2, y + 6, 5, ink),
        'x' => {
            for i in 0..5 {
                c.px(x + 2 + i, y + 2 + i, ink);
                c.px(x + 6 - i, y + 2 + i, ink);
            }
        }
        's' => {
            c.hline(x + 2, y + 2, 5, ink);
            c.hline(x + 2, y + 3, 5, ink);
        }
        'u' => {
            c.rect(x + 2, y + 2, 5, 5, ink);
            c.rect(
                x + 3,
                y + 4,
                3,
                2,
                if pressed { pal::FACE_DARK } else { pal::BTN },
            );
        }
        _ => {}
    }
}

/// `titlebar.bmp` 344×87.
pub fn titlebar() -> Canvas {
    let mut c = Canvas::new(344, 87, pal::FACE_DARK);
    // Normal and easter-egg strips with their unpressed buttons.
    for (y, active, label) in [
        (0, true, "NULAMP"),
        (15, false, "NULAMP"),
        (57, true, "* NULAMP * NULAMP *"),
        (72, false, "* NULAMP * NULAMP *"),
    ] {
        title_strip(&mut c, y, active, label, false);
        small_button(&mut c, 27 + 6, y + 3, false, 'm');
        small_button(&mut c, 27 + 244, y + 3, false, '_');
        small_button(&mut c, 27 + 254, y + 3, false, 's');
        small_button(&mut c, 27 + 264, y + 3, false, 'x');
    }
    // Shade strips (y 29 and 42 overlap by one row; draw inactive first, then active).
    for (y, active) in [(42, false), (29, true)] {
        title_strip(&mut c, y, active, "", true);
        let x0 = 27;
        small_button(&mut c, x0 + 6, y + 3, false, 'm');
        // Mini visualizer well (79,5) 38×5.
        c.rect(x0 + 79, y + 5, 38, 5, pal::BLACK);
        // Time well around x 126..157, y 4..9; colon at x 144..146.
        c.rect(x0 + 125, y + 3, 33, 8, pal::LCD_BG);
        c.px(x0 + 145, y + 5, pal::LCD_FG);
        c.px(x0 + 145, y + 8, pal::LCD_FG);
        // Mini transport 168..224.
        let ink = if active {
            pal::TITLE_TEXT
        } else {
            pal::TITLE_TEXT_IN
        };
        let boxes = [
            (168, 8, Icon::Prev),
            (176, 10, Icon::Play),
            (186, 9, Icon::Pause),
            (195, 9, Icon::Stop),
            (204, 11, Icon::Next),
            (215, 10, Icon::Eject),
        ];
        for (bx, bw, kind) in boxes {
            icon(&mut c, x0 + bx, y + 3, bw, 7, kind, 5, ink);
        }
        // Position-bar groove (226,4) 17×7 drawn from the sprite at (0,36) below.
        small_button(&mut c, x0 + 244, y + 3, false, '_');
        small_button(&mut c, x0 + 254, y + 3, false, 'u');
        small_button(&mut c, x0 + 264, y + 3, false, 'x');
    }
    // Pressed (and unpressed) title buttons.
    for (x, kind) in [(0, 'm'), (9, '_'), (18, 'x')] {
        small_button(&mut c, x, 0, false, kind);
        small_button(&mut c, x, 9, true, kind);
    }
    small_button(&mut c, 0, 18, false, 's');
    small_button(&mut c, 9, 18, true, 's');
    small_button(&mut c, 0, 27, false, 'u');
    small_button(&mut c, 9, 27, true, 'u');
    // Shade-mode position bar: groove (0,36) 17×7 and three thumbs 3×7.
    c.rect(0, 36, 17, 7, pal::SHADOW);
    c.hline(1, 39, 15, pal::FACE);
    for (i, col) in [pal::ACCENT_DARK, pal::ACCENT, 0xA6F5B0].iter().enumerate() {
        c.rect(17 + 3 * i as i32, 36, 3, 7, *col);
    }
    // Copy the groove into both shade strips.
    for y in [29, 42] {
        c.copy(0, 36, 17, 7, 27 + 226, y + 4);
    }
    // Clutterbar: shown (304,0), hidden (312,0), pressed variants at y 44.
    clutterbar(&mut c, 304, 0, None);
    c.rect(312, 0, 8, 43, pal::FACE);
    c.vline(312, 0, 43, pal::HILITE);
    for (i, x) in [304, 312, 320, 328, 336].into_iter().enumerate() {
        clutterbar(&mut c, x, 44, Some(i));
    }
    c
}

fn clutterbar(c: &mut Canvas, x: i32, y: i32, pressed: Option<usize>) {
    c.rect(x, y, 8, 43, pal::LCD_BG);
    c.vline(x, y, 43, pal::SHADOW);
    for (i, letter) in ['O', 'A', 'I', 'D', 'V'].into_iter().enumerate() {
        let ly = y + 3 + i as i32 * 8;
        let down = pressed == Some(i);
        c.rect(
            x + 1,
            ly - 1,
            7,
            8,
            if down { pal::ACCENT_DARK } else { pal::LCD_BG },
        );
        c.glyph(x + 2, ly, letter, if down { pal::LCD_FG } else { pal::CAP });
    }
}

/// `cbuttons.bmp` 136×36.
pub fn cbuttons() -> Canvas {
    let mut c = Canvas::new(136, 36, pal::FACE);
    let buttons = [
        (0, 23, Icon::Prev),
        (23, 23, Icon::Play),
        (46, 23, Icon::Pause),
        (69, 23, Icon::Stop),
        (92, 22, Icon::Next),
    ];
    for (row, pressed) in [(0, false), (18, true)] {
        for (x, w, kind) in buttons {
            c.button(x, row, w, 18, pressed);
            icon(
                &mut c,
                x + pressed as i32,
                row + pressed as i32,
                w,
                18,
                kind,
                9,
                if pressed || matches!(kind, Icon::Play) {
                    pal::ACCENT
                } else {
                    pal::TITLE_TEXT
                },
            );
        }
    }
    for (row, pressed) in [(0, false), (16, true)] {
        c.button(114, row, 22, 16, pressed);
        icon(
            &mut c,
            114,
            row,
            22,
            16,
            Icon::Eject,
            9,
            if pressed {
                pal::ACCENT
            } else {
                pal::TITLE_TEXT
            },
        );
    }
    c
}

/// `playpaus.bmp` 42×9.
pub fn playpaus() -> Canvas {
    let mut c = Canvas::new(42, 9, pal::LCD_BG);
    // Every cell is drawn at an even screen x (26 for icons, 24 for the strips) and y 28, so
    // dots on even cell-local pixels line up with the display behind them.
    for (x, w) in [(0, 9), (9, 9), (18, 9), (27, 9), (36, 3), (39, 3)] {
        c.lcd_dots(x, 0, w, 9, x, 0);
    }
    icon(&mut c, 0, 0, 9, 9, Icon::Play, 7, pal::LCD_FG);
    icon(&mut c, 9, 0, 9, 9, Icon::Pause, 7, pal::LCD_FG);
    icon(&mut c, 18, 0, 9, 9, Icon::Stop, 7, pal::LCD_FG);
    c.rect(36, 2, 2, 5, pal::LED_ON);
    c.rect(39, 2, 2, 5, pal::RED);
    c
}

/// `monoster.bmp` 58×24.
pub fn monoster() -> Canvas {
    let mut c = Canvas::new(58, 24, pal::LCD_BG);
    for (y, ink) in [(0, pal::LCD_FG), (12, pal::LCD_DIM)] {
        c.rect(0, y, 58, 12, pal::LCD_BG);
        c.text_centered(0, y, 29, 12, "STEREO", ink);
        c.text_centered(29, y, 28, 12, "MONO", ink);
    }
    c
}

/// `shufrep.bmp` 92×85 (layout: docs/decisions.md D8).
pub fn shufrep() -> Canvas {
    let mut c = Canvas::new(92, 85, pal::FACE);
    for (row, on, pressed) in [
        (0, false, false),
        (15, false, true),
        (30, true, false),
        (45, true, true),
    ] {
        for (x, w, label) in [(0, 28, "REP"), (28, 47, "SHUFFLE")] {
            c.button(x, row, w, 15, pressed);
            let led = if on { pal::LED_ON } else { pal::LED_OFF };
            c.rect(x + 3, row + 6, 3, 3, led);
            c.text(
                x + 8 + pressed as i32,
                row + 5 + pressed as i32,
                label,
                if on { pal::TITLE_TEXT } else { pal::LABEL },
            );
        }
    }
    for (row, on) in [(61, false), (73, true)] {
        for (x, pressed, label) in [
            (0, false, "EQ"),
            (23, false, "PL"),
            (46, true, "EQ"),
            (69, true, "PL"),
        ] {
            c.button(x, row, 23, 12, pressed);
            c.rect(
                x + 3,
                row + 5,
                2,
                2,
                if on { pal::LED_ON } else { pal::LED_OFF },
            );
            c.text(
                x + 9 + pressed as i32,
                row + 4,
                label,
                if on { pal::TITLE_TEXT } else { pal::LABEL },
            );
        }
    }
    c
}

/// Seven-segment digit into a 9×13 cell. Segment set: a b c d e f g.
fn seven_seg(c: &mut Canvas, x: i32, d: Option<u8>, ink: u32) {
    const SEGS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
    let Some(d) = d else { return };
    let s = SEGS[d as usize];
    let on = |bit: u8| s & (1 << bit) != 0;
    // Horizontal segments: a (y 0), g (y 6), d (y 12), x 2..6.
    if on(0) {
        c.hline(x + 2, 0, 5, ink);
        c.hline(x + 3, 1, 3, ink);
    }
    if on(6) {
        c.hline(x + 2, 6, 5, ink);
    }
    if on(3) {
        c.hline(x + 2, 12, 5, ink);
        c.hline(x + 3, 11, 3, ink);
    }
    // Vertical segments: f (x 1, y 1..5), b (x 7), e (x 1, y 7..11), c (x 7).
    for (bit, vx, vy) in [(5, 1, 1), (1, 7, 1), (4, 1, 7), (2, 7, 7)] {
        if on(bit) {
            c.vline(x + vx, vy, 5, ink);
            c.vline(x + vx + if vx == 1 { 1 } else { -1 }, vy + 1, 3, ink);
        }
    }
}

/// `numbers.bmp` 99×13. Cell 10 (x 90) is blank.
pub fn numbers() -> Canvas {
    let mut c = Canvas::new(99, 13, pal::LCD_BG);
    // Cells land at even screen x and y 26: dot the even cell-local pixels.
    for cell in 0..11 {
        c.lcd_dots(cell * 9, 0, 9, 13, cell * 9, 0);
    }
    for d in 0..10u8 {
        seven_seg(&mut c, d as i32 * 9, Some(d), pal::LCD_FG);
    }
    c
}

/// Volume/balance style bitmap 68×433: 28 frames and two thumbs. Each frame is a sunken
/// track; volume fills it with green up to the level, balance marks the centre.
fn slider_frames(balance: bool) -> Canvas {
    let mut c = Canvas::new(68, 433, pal::FACE);
    for n in 0..28 {
        let y = n * 15;
        c.rect(0, y, 68, 13, pal::FACE);
        c.rect(0, y + 3, 68, 6, pal::LCD_BG);
        c.edges(0, y + 3, 68, 6, pal::SHADOW, pal::HILITE);
        if balance {
            c.vline(34, y + 2, 9, pal::LABEL_DIM);
        } else {
            // Up to the thumb's centre: the thumb travels 0..54, so its centre is 7..61.
            let len = 7 + (n * 54) / 27 - 1;
            c.rect(1, y + 4, len, 4, pal::ACCENT);
        }
    }
    for (x, pressed) in [(15, false), (0, true)] {
        c.button(x, 422, 14, 11, pressed);
        let ink = if pressed { pal::ACCENT } else { pal::BTN_SH };
        for gx in [x + 5, x + 7, x + 9] {
            c.vline(gx, 425, 5, ink);
        }
    }
    c
}

pub fn volume() -> Canvas {
    slider_frames(false)
}

pub fn balance() -> Canvas {
    slider_frames(true)
}

/// `posbar.bmp` 307×10.
pub fn posbar() -> Canvas {
    let mut c = Canvas::new(307, 10, pal::FACE);
    c.rect(0, 2, 248, 6, pal::LCD_BG);
    c.edges(0, 2, 248, 6, pal::SHADOW, pal::HILITE);
    for (x, pressed) in [(248, false), (278, true)] {
        c.button(x, 0, 29, 10, pressed);
        for gx in [x + 12, x + 14, x + 16] {
            c.vline(gx, 3, 4, if pressed { pal::ACCENT } else { pal::BTN_SH });
        }
    }
    c
}

/// `text.bmp` 155×74 (rows 0–17 used).
pub fn text_bmp() -> Canvas {
    let mut c = Canvas::new(155, 74, pal::LCD_BG);
    let rows: [&[(u8, char)]; 3] = [
        &[(26, '"'), (27, '@')],
        &[
            (10, '…'),
            (11, '.'),
            (12, ':'),
            (13, '('),
            (14, ')'),
            (15, '-'),
            (16, '\''),
            (17, '!'),
            (18, '_'),
            (19, '+'),
            (20, '\\'),
            (21, '/'),
            (22, '['),
            (23, ']'),
            (24, '~'),
            (25, '&'),
            (26, '%'),
            (27, ','),
            (28, '='),
            (29, '$'),
            (30, '#'),
        ],
        &[(0, 'Å'), (1, 'Ö'), (2, 'Ä'), (3, '?'), (4, '*')],
    ];
    for i in 0..26u8 {
        c.glyph(i as i32 * 5, 0, (b'A' + i) as char, pal::LCD_FG);
    }
    for i in 0..10u8 {
        c.glyph(i as i32 * 5, 6, (b'0' + i) as char, pal::LCD_FG);
    }
    for (row, cells) in rows.iter().enumerate() {
        for &(col, ch) in cells.iter() {
            c.glyph(col as i32 * 5, row as i32 * 6, ch, pal::LCD_FG);
        }
    }
    c
}
