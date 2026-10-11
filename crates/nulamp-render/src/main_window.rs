//! Main window drawing (spec 02): normal mode 275×116 and windowshade mode 275×14.

use crate::frame::{Frame, blit};
use nulamp_skin::fonts::text_cell;
use nulamp_skin::{Skin, SkinBitmap as B};

/// Main window size in normal mode.
pub const MAIN_SIZE: (u32, u32) = (275, 116);
/// Main window size in windowshade mode.
pub const SHADE_SIZE: (u32, u32) = (275, 14);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// A button that can show a pressed sprite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainButton {
    Menu,
    Minimize,
    Shade,
    Close,
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
    Shuffle,
    Repeat,
    Eq,
    Pl,
}

/// What the large (and small) time display shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeDisplay {
    /// Nothing drawn: `main.bmp` shows through (stopped).
    #[default]
    Hidden,
    /// Blank cells (pause blink "off" phase).
    Blank,
    /// A time; `remaining` draws the minus sign.
    Time {
        minutes: u32,
        seconds: u32,
        remaining: bool,
    },
}

/// The song title area.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TitleArea {
    /// Nothing drawn.
    #[default]
    None,
    /// Bitmap-font mode: the scroll buffer text and the scroll position in pixels.
    Bitmap { text: String, offset: i32 },
    /// System-font mode: a pre-rendered 154×12 image of the box (111,24)–(265,36).
    Image(Frame),
}

/// Everything the main window shows. Pure data; drawing has no side effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainView {
    /// The window has focus (or "dim inactive title bars" is off).
    pub active: bool,
    pub shade: bool,
    pub easter_egg: bool,
    pub pressed: Option<MainButton>,
    pub clutterbar_shown: bool,
    /// Index 0–4 of the pressed clutterbar button (O A I D V).
    pub clutterbar_pressed: Option<u8>,
    pub always_on_top: bool,
    pub double_size: bool,
    pub state: PlayState,
    pub in_sync: bool,
    pub time: TimeDisplay,
    pub kbps: Option<u32>,
    pub khz: Option<u32>,
    /// Channel count of the playing stream; `None` when stopped.
    pub channels: Option<u32>,
    pub volume: u8,
    pub volume_pressed: bool,
    pub balance: i8,
    pub balance_pressed: bool,
    /// Seek position 0..=256 while playing an item of known length; `None` hides the thumb.
    pub seek: Option<u32>,
    /// The downloaded part of a network stream, as positions 0..=256 (`start`, `end`): drawn
    /// as a translucent overlay on the seek bar.
    pub seek_buffered: Option<(u32, u32)>,
    pub seek_pressed: bool,
    /// While dragging the seek bar: the thumb's pixel offset (0..=218), overriding `seek`.
    pub seek_drag: Option<i32>,
    pub shuffle: bool,
    pub repeat: bool,
    pub eq_visible: bool,
    pub pl_visible: bool,
    pub title: TitleArea,
}

impl Default for MainView {
    fn default() -> Self {
        Self {
            active: true,
            shade: false,
            easter_egg: false,
            pressed: None,
            clutterbar_shown: true,
            clutterbar_pressed: None,
            always_on_top: false,
            double_size: false,
            state: PlayState::Stopped,
            in_sync: false,
            time: TimeDisplay::Hidden,
            kbps: None,
            khz: None,
            channels: None,
            volume: 200,
            volume_pressed: false,
            balance: 0,
            balance_pressed: false,
            seek: None,
            seek_buffered: None,
            seek_pressed: false,
            seek_drag: None,
            shuffle: false,
            repeat: false,
            eq_visible: true,
            pl_visible: true,
            title: TitleArea::None,
        }
    }
}

/// Draws the main window at 1x (275×116, or 275×14 in windowshade mode).
///
/// The visualizer is not drawn here: it is composited separately, natively at 2x in double size.
pub fn draw_main(skin: &Skin, v: &MainView) -> Frame {
    if v.shade {
        return draw_shade(skin, v);
    }
    let mut f = Frame::new(MAIN_SIZE.0, MAIN_SIZE.1, 0);
    blit(&mut f, skin.bitmap(B::Main), 0, 0, 275, 116, 0, 0);
    draw_titlebar(&mut f, skin, v);
    draw_clutterbar(&mut f, skin, v);
    draw_status(&mut f, skin, v);
    draw_time(&mut f, skin, v.time);
    if v.state != PlayState::Stopped {
        draw_info(&mut f, skin, v);
    }
    draw_monoster(&mut f, skin, v.channels);
    draw_volume(&mut f, skin, v.volume, v.volume_pressed);
    draw_balance(&mut f, skin, v.balance, v.balance_pressed);
    if let Some(pos) = v.seek {
        let x = v.seek_drag.unwrap_or_else(|| seek_thumb_x(pos));
        draw_seek(&mut f, skin, x, v.seek_pressed, v.seek_buffered);
    }
    draw_buttons(&mut f, skin, v);
    draw_title_area(&mut f, skin, &v.title);
    f
}

fn title_src_y(v: &MainView) -> i32 {
    match (v.shade, v.active, v.easter_egg) {
        (true, true, _) => 29,
        (true, false, _) => 42,
        (false, true, false) => 0,
        (false, false, false) => 15,
        (false, true, true) => 57,
        (false, false, true) => 72,
    }
}

fn draw_title_buttons(f: &mut Frame, skin: &Skin, v: &MainView) {
    let t = skin.bitmap(B::Titlebar);
    let (dx, sx, sy) = match v.pressed {
        Some(MainButton::Menu) => (6, 0, 9),
        Some(MainButton::Minimize) => (244, 9, 9),
        Some(MainButton::Close) => (264, 18, 9),
        Some(MainButton::Shade) if v.shade => (254, 9, 27),
        Some(MainButton::Shade) => (254, 9, 18),
        _ => return,
    };
    blit(f, t, sx, sy, 9, 9, dx, 3);
}

fn draw_titlebar(f: &mut Frame, skin: &Skin, v: &MainView) {
    blit(
        f,
        skin.bitmap(B::Titlebar),
        27,
        title_src_y(v),
        275,
        14,
        0,
        0,
    );
    draw_title_buttons(f, skin, v);
}

fn draw_clutterbar(f: &mut Frame, skin: &Skin, v: &MainView) {
    let t = skin.bitmap(B::Titlebar);
    if !v.clutterbar_shown {
        blit(f, t, 312, 0, 8, 43, 10, 22);
        return;
    }
    match v.clutterbar_pressed {
        Some(i) => blit(f, t, 304 + 8 * i as i32, 44, 8, 43, 10, 22),
        None => blit(f, t, 304, 0, 8, 43, 10, 22),
    }
    if v.clutterbar_pressed != Some(1) {
        let (sx, sy) = if v.always_on_top {
            (313, 55)
        } else {
            (305, 11)
        };
        blit(f, t, sx, sy, 7, 8, 11, 33);
    }
    if v.clutterbar_pressed != Some(3) {
        let (sx, sy) = if v.double_size { (329, 71) } else { (305, 27) };
        blit(f, t, sx, sy, 7, 6, 11, 49);
    }
}

fn draw_status(f: &mut Frame, skin: &Skin, v: &MainView) {
    let p = skin.bitmap(B::Playpaus);
    let icon_x = match v.state {
        PlayState::Playing => 0,
        PlayState::Paused => 9,
        PlayState::Stopped => 18,
    };
    blit(f, p, icon_x, 0, 9, 9, 26, 28);
    match (v.state, v.in_sync) {
        (PlayState::Playing, true) => blit(f, p, 36, 0, 3, 9, 24, 28),
        (PlayState::Playing, false) => blit(f, p, 39, 0, 3, 9, 24, 28),
        _ => blit(f, p, 27, 0, 2, 9, 24, 28),
    }
}

fn draw_time(f: &mut Frame, skin: &Skin, t: TimeDisplay) {
    let n = skin.bitmap(B::Numbers);
    let ex = skin.nums_ex;
    let digit = |f: &mut Frame, d: u32, x: i32| blit(f, n, d as i32 * 9, 0, 9, 13, x, 26);
    let blank = |f: &mut Frame, x: i32| blit(f, n, 90, 0, 9, 13, x, 26);
    match t {
        TimeDisplay::Hidden => {}
        TimeDisplay::Blank => {
            blank(f, if ex { 38 } else { 36 });
            for x in [48, 60, 78, 90] {
                blank(f, x);
            }
            if !ex {
                blit(f, n, 9, 6, 5, 1, 40, 32);
            }
        }
        TimeDisplay::Time {
            minutes,
            seconds,
            remaining,
        } => {
            if remaining {
                if ex {
                    blit(f, n, 99, 0, 9, 13, 36, 26);
                } else {
                    blit(f, n, 20, 6, 5, 1, 40, 32);
                }
            } else if minutes >= 100 {
                digit(f, (minutes / 100) % 10, 36);
            } else if ex {
                blank(f, 36);
            } else {
                blit(f, n, 9, 6, 5, 1, 40, 32);
            }
            let m = minutes % 100;
            digit(f, m / 10, 48);
            digit(f, m % 10, 60);
            digit(f, (seconds / 10) % 10, 78);
            digit(f, seconds % 10, 90);
        }
    }
}

/// Draws one `text.bmp` 5×6 cell.
fn text_glyph(f: &mut Frame, skin: &Skin, cell: (u8, u8), x: i32, y: i32) {
    blit(
        f,
        skin.bitmap(B::Text),
        cell.0 as i32 * 5,
        cell.1 as i32 * 6,
        5,
        6,
        x,
        y,
    );
}

/// A small-digit cell: a digit, a letter, or the blank cell (100,12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmallCell {
    Digit(u32),
    Letter(char),
    Blank,
}

/// kbps cells (02 §7.3).
pub fn kbps_cells(kbps: u32) -> [SmallCell; 3] {
    use SmallCell::*;
    if kbps >= 10000 {
        let ht = (kbps / 100_000) % 10;
        [
            if ht == 0 { Blank } else { Digit(ht) },
            Digit((kbps / 10_000) % 10),
            Letter('C'),
        ]
    } else if kbps >= 1000 {
        [Digit(kbps / 1000), Digit((kbps / 100) % 10), Letter('H')]
    } else {
        let lead = |d: u32, on: bool| if on { Digit(d) } else { Blank };
        [
            lead(kbps / 100, kbps >= 100),
            lead((kbps / 10) % 10, kbps >= 10),
            Digit(kbps % 10),
        ]
    }
}

/// kHz cells (02 §7.3).
pub fn khz_cells(khz: u32) -> [SmallCell; 2] {
    use SmallCell::*;
    let tens = (khz / 10) % 10;
    [if khz >= 10 { Digit(tens) } else { Blank }, Digit(khz % 10)]
}

fn small_cell(f: &mut Frame, skin: &Skin, c: SmallCell, x: i32, y: i32) {
    match c {
        SmallCell::Digit(d) => text_glyph(f, skin, (d as u8, 1), x, y),
        SmallCell::Letter(l) => text_glyph(f, skin, text_cell(l), x, y),
        SmallCell::Blank => blit(f, skin.bitmap(B::Text), 100, 12, 5, 6, x, y),
    }
}

fn draw_info(f: &mut Frame, skin: &Skin, v: &MainView) {
    if let Some(k) = v.kbps {
        for (i, c) in kbps_cells(k).into_iter().enumerate() {
            small_cell(f, skin, c, 111 + 5 * i as i32, 43);
        }
    }
    if let Some(k) = v.khz {
        for (i, c) in khz_cells(k).into_iter().enumerate() {
            small_cell(f, skin, c, 156 + 5 * i as i32, 43);
        }
    }
}

fn draw_monoster(f: &mut Frame, skin: &Skin, channels: Option<u32>) {
    let m = skin.bitmap(B::Monoster);
    let stereo_lit = channels == Some(2);
    let mono_lit = channels == Some(1);
    // The two indicators overlap in column 239; stereo is drawn last (docs/decisions.md D12).
    blit(f, m, 29, if mono_lit { 0 } else { 12 }, 28, 12, 212, 41);
    blit(f, m, 0, if stereo_lit { 0 } else { 12 }, 29, 12, 239, 41);
}

/// Volume slider frame index and thumb x offset (02 §6.1).
pub fn volume_geometry(volume: u8) -> (i32, i32) {
    let v = volume as i32;
    (v * 27 / 255, (v * 51 / 255).clamp(0, 54))
}

/// Balance slider frame index and thumb x offset (02 §6.2).
pub fn balance_geometry(balance: i8) -> (i32, i32) {
    let b = (balance as i32).clamp(-127, 127);
    (b.abs() * 27 / 127, (b * 12 / 127 + 12).clamp(0, 24))
}

fn draw_volume(f: &mut Frame, skin: &Skin, volume: u8, pressed: bool) {
    let b = skin.bitmap(B::Volume);
    let (frame, x) = volume_geometry(volume);
    blit(f, b, 0, frame * 15, 68, 13, 107, 57);
    blit(f, b, if pressed { 0 } else { 15 }, 422, 14, 11, 107 + x, 58);
}

fn draw_balance(f: &mut Frame, skin: &Skin, balance: i8, pressed: bool) {
    let b = skin.bitmap(B::Balance);
    let (frame, x) = balance_geometry(balance);
    blit(f, b, 9, frame * 15, 38, 13, 177, 57);
    blit(f, b, if pressed { 0 } else { 15 }, 422, 14, 11, 177 + x, 58);
}

/// Seek thumb x offset for a position 0..=256 (02 §6.3).
pub fn seek_thumb_x(pos: u32) -> i32 {
    (pos as i32 * 219 / 256).clamp(0, 219)
}

fn draw_seek(
    f: &mut Frame,
    skin: &Skin,
    thumb_x: i32,
    pressed: bool,
    buffered: Option<(u32, u32)>,
) {
    let p = skin.bitmap(B::Posbar);
    blit(f, p, 0, 0, 248, 10, 16, 72);
    if let Some(span) = buffered {
        draw_buffered(f, span, (16, 72, 248, 10), MAIN_THUMB);
    }
    blit(
        f,
        p,
        if pressed { 278 } else { 248 },
        0,
        29,
        10,
        16 + thumb_x,
        72,
    );
}

fn draw_buttons(f: &mut Frame, skin: &Skin, v: &MainView) {
    let cb = skin.bitmap(B::Cbuttons);
    let is = |b: MainButton| v.pressed == Some(b);
    for (b, sx, w, dx) in [
        (MainButton::Previous, 0, 23, 16),
        (MainButton::Play, 23, 23, 39),
        (MainButton::Pause, 46, 23, 62),
        (MainButton::Stop, 69, 23, 85),
        (MainButton::Next, 92, 22, 108),
    ] {
        blit(f, cb, sx, if is(b) { 18 } else { 0 }, w, 18, dx, 88);
    }
    blit(
        f,
        cb,
        114,
        if is(MainButton::Eject) { 16 } else { 0 },
        22,
        16,
        136,
        89,
    );

    let sr = skin.bitmap(B::Shufrep);
    let row = |on: bool, pressed: bool| (if on { 30 } else { 0 }) + (if pressed { 15 } else { 0 });
    blit(
        f,
        sr,
        28,
        row(v.shuffle, is(MainButton::Shuffle)),
        47,
        15,
        164,
        89,
    );
    blit(
        f,
        sr,
        0,
        row(v.repeat, is(MainButton::Repeat)),
        28,
        15,
        210,
        89,
    );
    let small = |on: bool| if on { 73 } else { 61 };
    let eq_x = if is(MainButton::Eq) { 46 } else { 0 };
    let pl_x = if is(MainButton::Pl) { 69 } else { 23 };
    blit(f, sr, eq_x, small(v.eq_visible), 23, 12, 219, 58);
    blit(f, sr, pl_x, small(v.pl_visible), 23, 12, 242, 58);
}

/// Width of the bitmap-font scroll gap `"  ***  "`.
pub const MARQUEE_SEPARATOR: &str = "  ***  ";

fn draw_title_area(f: &mut Frame, skin: &Skin, title: &TitleArea) {
    match title {
        TitleArea::None => {}
        TitleArea::Image(img) => blit(f, img, 0, 0, 154, 12, 111, 24),
        TitleArea::Bitmap { text, offset } => {
            let chars: Vec<char> = text.chars().collect();
            let text_bmp = skin.bitmap(B::Text);
            let offset = (*offset).max(0);
            let first = (offset / 5) as usize;
            let mut x = 111 - offset % 5;
            let mut i = first;
            while x < 265 && i < chars.len() {
                let cell = text_cell(chars[i]);
                // A glyph starting at x >= 260 is clipped at x = 265.
                let w = (265 - x).min(5);
                let skip = (111 - x).max(0);
                blit(
                    f,
                    text_bmp,
                    cell.0 as i32 * 5 + skip,
                    cell.1 as i32 * 6,
                    w - skip,
                    6,
                    x + skip,
                    27,
                );
                x += 5;
                i += 1;
            }
            while x < 265 {
                let x0 = x.max(111);
                blit(f, text_bmp, 4, 0, 1, 6, x0, 27);
                x = x0 + 1;
            }
        }
    }
}

/// Opacity of the buffered overlay, out of 256.
const BUFFERED_ALPHA: u32 = 56;

/// Rows (or columns) whose mean luminance differs by at most this belong to the same groove.
const GROOVE_STEP: u32 = 4;

/// How a seek thumb moves along its bar: (x offset at position 0, travel in pixels, width).
type ThumbTravel = (i32, i32, i32);
const MAIN_THUMB: ThumbTravel = (0, 219, 29);
const SHADE_THUMB: ThumbTravel = (1, 12, 3);

fn luminance(p: u32) -> u32 {
    ((p >> 16 & 255) * 299 + (p >> 8 & 255) * 587 + (p & 255) * 114) / 1000
}

/// The longest run of neighbours that differ by at most [`GROOVE_STEP`], as `start..end`.
fn longest_run(values: &[u32]) -> (usize, usize) {
    let (mut best, mut start) = ((0, values.len().min(1)), 0);
    for i in 1..=values.len() {
        if i == values.len() || values[i].abs_diff(values[i - 1]) > GROOVE_STEP {
            if i - start > best.1 - best.0 {
                best = (start, i);
            }
            start = i;
        }
    }
    best
}

/// The groove of the `w`×`h` bar at (x, y) as (x, y, w, h): the longest run of rows of even
/// brightness (measured over the middle half, away from the end caps), then the longest run
/// of columns within those rows. Any skin's posbar has one; a flat bar is all groove.
fn groove(f: &Frame, (x, y, w, h): (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let rows: Vec<u32> = (y..y + h)
        .map(|ty| {
            let xs = x + w / 4..x + w - w / 4;
            let n = xs.len().max(1) as u32;
            xs.map(|tx| luminance(f.pixel(tx, ty))).sum::<u32>() / n
        })
        .collect();
    let (r0, r1) = longest_run(&rows);
    let cols: Vec<u32> = (x..x + w)
        .map(|tx| {
            let n = (r1 - r0).max(1) as u32;
            (r0..r1)
                .map(|r| luminance(f.pixel(tx, y + r as i32)))
                .sum::<u32>()
                / n
        })
        .collect();
    let (c0, c1) = longest_run(&cols);
    (
        x + c0 as i32,
        y + r0 as i32,
        (c1 - c0) as i32,
        (r1 - r0) as i32,
    )
}

/// Draws the buffered span (positions 0..=256) of the `w`×`h` seek bar at (x, y) as a thin
/// translucent band centred in the bar's groove: lighter in a dark groove, darker in a light
/// one, so any skin shows it. A position maps to where the thumb's centre would be, so the
/// band ends under the thumb when the playhead reaches the end of the download.
fn draw_buffered(f: &mut Frame, span: (u32, u32), bar: (i32, i32, i32, i32), thumb: ThumbTravel) {
    let (gx, gy, gw, gh) = groove(f, bar);
    // One pixel in from the groove's ends; a third of its height, at least 2 rows.
    let (left, right) = (gx + 1, gx + gw - 1);
    let bh = ((gh + 1) / 3).max(2).min(gh);
    let by = gy + (gh - bh) / 2;
    let (off, travel, tw) = thumb;
    let px = |pos: u32| bar.0 + off + pos.min(256) as i32 * travel / 256 + tw / 2;
    let x0 = if span.0 == 0 {
        left
    } else {
        px(span.0).max(left)
    };
    let x1 = if span.1 >= 256 {
        right
    } else {
        px(span.1).min(right)
    };
    if x1 <= x0 || bh <= 0 {
        return;
    }
    let (mut sum, mut n) = (0u32, 0u32);
    for ty in by..by + bh {
        for tx in left..right {
            sum += luminance(f.pixel(tx, ty));
            n += 1;
        }
    }
    let to = if sum / n.max(1) < 128 { 255 } else { 0 };
    let mix = |c: u32| (c * (256 - BUFFERED_ALPHA) + to * BUFFERED_ALPHA) / 256;
    for ty in by..by + bh {
        for tx in x0..x1 {
            let p = f.pixel(tx, ty);
            f.set(
                tx,
                ty,
                mix(p >> 16 & 255) << 16 | mix(p >> 8 & 255) << 8 | mix(p & 255),
            );
        }
    }
}

// ----- Windowshade mode -----

/// Shade-mode mini seek thumb (02 §2.4): x offset p (1..=13) and source x.
pub fn shade_seek_geometry(pos: u32) -> (i32, i32) {
    let p = (pos as i32 * 12 / 256 + 1).clamp(1, 13);
    let sx = if p < 6 {
        17
    } else if p < 9 {
        20
    } else {
        23
    };
    (p, sx)
}

fn draw_shade(skin: &Skin, v: &MainView) -> Frame {
    let mut f = Frame::new(SHADE_SIZE.0, SHADE_SIZE.1, 0);
    let t = skin.bitmap(B::Titlebar);
    blit(&mut f, t, 27, title_src_y(v), 275, 14, 0, 0);
    draw_title_buttons(&mut f, skin, v);
    draw_shade_time(&mut f, skin, v.time);
    if let Some(pos) = v.seek {
        blit(&mut f, t, 0, 36, 17, 7, 226, 4);
        if let Some(span) = v.seek_buffered {
            draw_buffered(&mut f, span, (226, 4, 17, 7), SHADE_THUMB);
        }
        let (p, sx) = shade_seek_geometry(pos);
        blit(&mut f, t, sx, 36, 3, 7, 226 + p, 4);
    }
    f
}

fn draw_shade_time(f: &mut Frame, skin: &Skin, t: TimeDisplay) {
    let text = skin.bitmap(B::Text);
    let blank3 = |f: &mut Frame, x: i32| blit(f, text, 142, 0, 3, 6, x, 4);
    let blank5 = |f: &mut Frame, x: i32| blit(f, text, 150, 0, 5, 6, x, 4);
    let digit = |f: &mut Frame, d: u32, x: i32| blit(f, text, d as i32 * 5, 6, 5, 6, x, 4);
    match t {
        TimeDisplay::Hidden => {}
        TimeDisplay::Blank => {
            blank3(f, 126);
            blank3(f, 130);
            for x in [134, 139, 147, 152] {
                blank5(f, x);
            }
        }
        TimeDisplay::Time {
            minutes,
            seconds,
            remaining,
        } => {
            blank3(f, 126);
            if minutes < 100 {
                blank3(f, 130);
            } else {
                digit(f, (minutes / 100) % 10, 129);
            }
            if remaining {
                blit(
                    f,
                    text,
                    75,
                    6,
                    3,
                    6,
                    if minutes >= 100 { 126 } else { 130 },
                    4,
                );
            }
            let m = minutes % 100;
            digit(f, m / 10, 134);
            digit(f, m % 10, 139);
            digit(f, (seconds / 10) % 10, 147);
            digit(f, seconds % 10, 152);
        }
    }
}

/// Visualizer rectangle in the main window for the given mode and scale (06 §1).
pub fn vis_rect(shade: bool, scale: i32) -> (i32, i32, i32, i32) {
    if shade {
        (79 * scale, 5 * scale, 38 * scale, 5 * scale)
    } else {
        (24 * scale, 43 * scale, 76 * scale, 16 * scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SmallCell::*;

    #[test]
    fn kbps_formatting() {
        assert_eq!(kbps_cells(8), [Blank, Blank, Digit(8)]);
        assert_eq!(kbps_cells(64), [Blank, Digit(6), Digit(4)]);
        assert_eq!(kbps_cells(128), [Digit(1), Digit(2), Digit(8)]);
        assert_eq!(kbps_cells(320), [Digit(3), Digit(2), Digit(0)]);
        assert_eq!(kbps_cells(1411), [Digit(1), Digit(4), Letter('H')]);
        assert_eq!(kbps_cells(9999), [Digit(9), Digit(9), Letter('H')]);
        assert_eq!(kbps_cells(10000), [Blank, Digit(1), Letter('C')]);
        assert_eq!(kbps_cells(123456), [Digit(1), Digit(2), Letter('C')]);
        assert_eq!(khz_cells(44), [Digit(4), Digit(4)]);
        assert_eq!(khz_cells(8), [Blank, Digit(8)]);
    }

    #[test]
    fn slider_geometry() {
        assert_eq!(volume_geometry(0), (0, 0));
        assert_eq!(volume_geometry(1), (0, 0));
        assert_eq!(volume_geometry(128), (13, 25));
        assert_eq!(volume_geometry(254), (26, 50));
        assert_eq!(volume_geometry(255), (27, 51));
        assert_eq!(balance_geometry(-127), (27, 0));
        assert_eq!(balance_geometry(-13), (2, 11));
        assert_eq!(balance_geometry(0), (0, 12));
        assert_eq!(balance_geometry(13), (2, 13));
        assert_eq!(balance_geometry(127), (27, 24));
        assert_eq!(seek_thumb_x(0), 0);
        assert_eq!(seek_thumb_x(256), 219);
        assert_eq!(shade_seek_geometry(0), (1, 17));
        assert_eq!(shade_seek_geometry(256), (13, 23));
        assert_eq!(shade_seek_geometry(128), (7, 20));
    }

    #[test]
    fn buffered_band_sits_in_the_groove() {
        // A dark groove in rows 1..5 between light borders.
        let mut f = Frame::new(20, 6, 0x80_80_80);
        for y in 1..5 {
            for x in 0..20 {
                f.set(x, y, 0x10_10_10);
            }
        }
        assert_eq!(groove(&f, (0, 0, 20, 6)), (0, 1, 20, 4));
        // Thumb 4 wide travelling 16: position 128 is at x 8 + 2.
        draw_buffered(&mut f, (0, 128), (0, 0, 20, 6), (0, 16, 4));
        let lit = 0x44_44_44;
        assert_eq!(f.pixel(1, 2), lit);
        assert_eq!(f.pixel(9, 3), lit);
        assert_eq!(f.pixel(0, 2), 0x10_10_10, "inset from the groove's end");
        assert_eq!(f.pixel(10, 2), 0x10_10_10, "ends at the thumb's centre");
        assert_eq!(f.pixel(5, 1), 0x10_10_10, "thinner than the groove");
        assert_eq!(f.pixel(5, 4), 0x10_10_10);
        assert_eq!(f.pixel(5, 0), 0x80_80_80);
        // A light groove gets darker.
        let mut f = Frame::new(8, 4, 0xff_ff_ff);
        draw_buffered(&mut f, (0, 256), (0, 0, 8, 4), (0, 4, 4));
        assert_eq!(f.pixel(3, 1), 0xc7_c7_c7);
        // An empty span changes nothing.
        let mut f = Frame::new(8, 4, 0xff_ff_ff);
        draw_buffered(&mut f, (100, 100), (0, 0, 8, 4), (0, 4, 4));
        assert!(f.pixels().iter().all(|&p| p == 0xff_ff_ff));
    }
}
