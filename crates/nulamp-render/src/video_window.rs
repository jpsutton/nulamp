//! Video window frame (`video.bmp`, spec 05 §2).

use crate::frame::{Frame, blit, blit_stretch};
use nulamp_skin::{Skin, SkinBitmap as B};

/// Default size (05 §2).
pub const DEFAULT_SIZE: (i32, i32) = (275, 232);

/// The five buttons of the bottom bar, left to right (05 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoButton {
    FullScreen,
    Zoom100,
    Zoom200,
    /// The "TV" button: video options.
    Misc,
    Menu,
}

impl VideoButton {
    pub const ALL: [Self; 5] = [
        Self::FullScreen,
        Self::Zoom100,
        Self::Zoom200,
        Self::Misc,
        Self::Menu,
    ];

    pub fn index(self) -> i32 {
        Self::ALL.iter().position(|b| *b == self).unwrap_or(0) as i32
    }
}

/// Picture area of a w×h video window: (x, y, width, height).
pub fn video_rect(w: i32, h: i32) -> (i32, i32, i32, i32) {
    (11, 20, w - 19, h - 58)
}

/// Info text area of a w×h video window: (x, y, width, height).
pub fn info_rect(w: i32, h: i32) -> (i32, i32, i32, i32) {
    (92, h - 27, w - 25 - 92, 14)
}

/// The button at a point, if any.
pub fn button_at(x: i32, y: i32, h: i32) -> Option<VideoButton> {
    if !(9..89).contains(&x) || !(h - 29..h - 11).contains(&y) {
        return None;
    }
    VideoButton::ALL.get(((x - 9) / 15) as usize).copied()
}

/// Whether a point is on the close button.
pub fn in_close(x: i32, y: i32, w: i32) -> bool {
    (w - 11..w - 2).contains(&x) && (3..12).contains(&y)
}

/// Whether a point is on the resize grip: the bottom-right corner, as in the playlist editor
/// (04 §1), where (w − x) + (h − y) ≤ 30.
pub fn in_grip(x: i32, y: i32, w: i32, h: i32) -> bool {
    x >= w - 20 && y >= h - 20 && (w - x) + (h - y) <= 30
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoView {
    pub w: i32,
    pub h: i32,
    pub active: bool,
    pub pressed: Option<VideoButton>,
    pub close_pressed: bool,
}

/// Draws the frame. The picture area is black and the info area filled with `mbBG`; the
/// caller draws the picture and the info text.
pub fn draw_video_frame(skin: &Skin, v: &VideoView) -> Frame {
    let b = skin.bitmap(B::Video);
    let (w, h) = (v.w, v.h);
    let mut f = Frame::new(w as u32, h as u32, 0);
    let ty = if v.active { 0 } else { 21 };

    // Title bar, tiled like the playlist editor's (04 §2.2).
    blit(&mut f, b, 0, ty, 25, 20, 0, 0);
    let n = ((w - 150) / 25).max(0);
    let (half, odd) = (n / 2, n % 2 == 1);
    let mut x = 25;
    if odd {
        blit(&mut f, b, 127, ty, 12, 20, x, 0);
        x += 12;
    }
    for _ in 0..half {
        blit(&mut f, b, 127, ty, 25, 20, x, 0);
        x += 25;
    }
    blit(&mut f, b, 26, ty, 100, 20, x, 0);
    x += 100;
    if odd {
        blit(&mut f, b, 127 + 12, ty, 13, 20, x, 0);
        x += 13;
    }
    for _ in 0..half {
        blit(&mut f, b, 127, ty, 25, 20, x, 0);
        x += 25;
    }
    if x < w - 25 {
        blit_stretch(&mut f, b, 127, ty, 25, 20, x, 0, w - 25 - x, 20);
    }
    blit(&mut f, b, 153, ty, 25, 20, w - 25, 0);
    if v.close_pressed {
        blit(&mut f, b, 148, 42, 9, 9, w - 11, 3);
    } else {
        blit(&mut f, b, 167, 3 + ty, 9, 9, w - 11, 3);
    }

    // Sides: whole tiles, then the remainder stretched.
    let body = (h - 58).max(0);
    let (tiles, rem) = (body / 29, body % 29);
    for k in 0..tiles {
        let y = 20 + 29 * k;
        blit(&mut f, b, 127, 42, 11, 29, 0, y);
        blit(&mut f, b, 139, 42, 8, 29, w - 8, y);
    }
    if rem > 0 {
        let y = 20 + 29 * tiles;
        blit_stretch(&mut f, b, 127, 42, 11, 29, 0, y, 11, rem);
        blit_stretch(&mut f, b, 139, 42, 8, 29, w - 8, y, 8, rem);
    }

    // Bottom bar: left piece with the buttons, fill tiles, right piece.
    blit(&mut f, b, 0, 42, 125, 38, 0, h - 38);
    let mut x = 125;
    while x + 25 <= w - 125 {
        blit(&mut f, b, 127, 81, 25, 38, x, h - 38);
        x += 25;
    }
    if x < w - 125 {
        blit_stretch(&mut f, b, 127, 81, 25, 38, x, h - 38, w - 125 - x, 38);
    }
    blit(&mut f, b, 0, 81, 125, 38, w - 125, h - 38);
    for btn in VideoButton::ALL {
        let i = btn.index();
        if v.pressed == Some(btn) {
            blit(&mut f, b, 158 + 15 * i, 42, 15, 18, 9 + 15 * i, h - 29);
        } else {
            blit(&mut f, b, 9 + 15 * i, 51, 15, 18, 9 + 15 * i, h - 29);
        }
    }

    let (ix, iy, iw, ih) = info_rect(w, h);
    f.fill_rect(ix, iy, iw, ih, skin.pledit.mb_bg);
    let (vx, vy, vw, vh) = video_rect(w, h);
    f.fill_rect(vx, vy, vw, vh, 0);
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{self, check_rect};

    fn ok(r: Result<(), String>) {
        if let Err(e) = r {
            panic!("{e}");
        }
    }

    fn view(w: i32, h: i32) -> VideoView {
        VideoView {
            w,
            h,
            active: true,
            pressed: None,
            close_pressed: false,
        }
    }

    #[test]
    fn frame_pieces() {
        let skin = probe::skin(&[]);
        let f = draw_video_frame(&skin, &view(275, 232));
        // Title: n = 5 fill tiles, so a 12-pixel slice and two tiles left of the centre.
        ok(check_rect(&f, B::Video, 0, 0, 25, 20, 0, 0));
        ok(check_rect(&f, B::Video, 127, 0, 12, 20, 25, 0));
        ok(check_rect(&f, B::Video, 127, 0, 25, 20, 37, 0));
        ok(check_rect(&f, B::Video, 26, 0, 100, 20, 87, 0));
        ok(check_rect(&f, B::Video, 139, 0, 13, 20, 187, 0));
        ok(check_rect(&f, B::Video, 153, 0, 16, 3, 250, 0));
        ok(check_rect(&f, B::Video, 167, 3, 9, 9, 264, 3));
        // Sides: 174 rows = 6 tiles.
        ok(check_rect(&f, B::Video, 127, 42, 11, 29, 0, 20));
        ok(check_rect(&f, B::Video, 139, 42, 8, 29, 267, 165));
        // Bottom: left piece around the buttons, one fill tile, right piece.
        ok(check_rect(&f, B::Video, 0, 42, 9, 38, 0, 194));
        ok(check_rect(&f, B::Video, 9, 51, 75, 18, 9, 203));
        ok(check_rect(&f, B::Video, 0, 42, 92, 11, 0, 194));
        ok(check_rect(&f, B::Video, 127, 81, 25, 11, 125, 194));
        ok(check_rect(&f, B::Video, 127, 106, 25, 13, 125, 219));
        ok(check_rect(&f, B::Video, 0, 81, 67, 11, 150, 194));
        ok(check_rect(&f, B::Video, 100, 81, 25, 38, 250, 194));
    }

    #[test]
    fn pressed_states_and_inactive_title() {
        let skin = probe::skin(&[]);
        let mut v = view(275, 116);
        v.active = false;
        v.pressed = Some(VideoButton::Zoom200);
        v.close_pressed = true;
        let f = draw_video_frame(&skin, &v);
        ok(check_rect(&f, B::Video, 0, 21, 25, 20, 0, 0));
        ok(check_rect(&f, B::Video, 148, 42, 9, 9, 264, 3));
        ok(check_rect(&f, B::Video, 188, 42, 15, 18, 39, 87));
        ok(check_rect(&f, B::Video, 9, 51, 30, 18, 9, 87));
        // A remainder of the sides is stretched (58 body rows = 2 tiles exactly here).
        ok(check_rect(&f, B::Video, 127, 42, 11, 29, 0, 49));
    }

    #[test]
    fn areas_and_hits() {
        assert_eq!(video_rect(275, 232), (11, 20, 256, 174));
        assert_eq!(info_rect(275, 232), (92, 205, 158, 14));
        assert_eq!(button_at(9, 203, 232), Some(VideoButton::FullScreen));
        assert_eq!(button_at(83, 220, 232), Some(VideoButton::Menu));
        assert_eq!(button_at(85, 210, 232), None);
        assert_eq!(button_at(30, 202, 232), None);
        assert!(in_close(264, 3, 275));
        assert!(!in_close(263, 3, 275));
        assert!(in_grip(274, 231, 275, 232));
        assert!(!in_grip(256, 213, 275, 232));
    }

    #[test]
    fn picture_and_info_areas_are_filled() {
        let skin = nulamp_skin::builtin::skin();
        let f = draw_video_frame(&skin, &view(300, 261));
        assert_eq!(f.pixel(11, 20), 0);
        assert_eq!(f.pixel(280, 202), 0);
        assert_eq!(f.pixel(92, 234), skin.pledit.mb_bg);
        assert_eq!(f.pixel(274, 247), skin.pledit.mb_bg);
    }
}
