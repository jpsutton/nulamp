//! The video window (05 §2): preferences, the picture source the application provides, and the
//! window's input. The shell wires it up in `shell/video_host.rs`.

pub use nulamp_render::Frame;
use nulamp_render::gen_window::snap_size;
use nulamp_render::video_window::{self as vw, VideoButton};
use winit::keyboard::KeyCode;

use crate::input::Mods;
use crate::menu::{Item, MenuCmd};

/// Video preferences (05 §2, 09 §8).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoPrefs {
    /// Show the picture of video files; off, they play their audio only.
    pub enabled: bool,
    pub open_on_start: bool,
    pub close_on_end: bool,
    pub stop_on_close: bool,
    /// Resize the window to the video's own size when one starts.
    pub resize_to_video: bool,
    pub fullscreen_on_start: bool,
    pub leave_fullscreen_on_stop: bool,
    /// Keep the picture's aspect ratio (else it fills the area).
    pub lock_aspect: bool,
    /// Screen aspect ratio to show every video with; `None`: the video's own.
    pub aspect: Option<f64>,
    pub logo_when_idle: bool,
    pub inhibit_screensaver: bool,
}

impl Default for VideoPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            open_on_start: true,
            close_on_end: true,
            stop_on_close: false,
            resize_to_video: false,
            fullscreen_on_start: false,
            leave_fullscreen_on_stop: true,
            lock_aspect: true,
            aspect: None,
            logo_when_idle: true,
            inhibit_screensaver: true,
        }
    }
}

/// Reads an aspect ratio typed as `16:9`, `16/9` or `1.85`. Empty text is the video's own
/// ratio (`Some(None)`); text that is not a ratio gives `None`.
pub fn parse_aspect(text: &str) -> Option<Option<f64>> {
    let t = text.trim();
    if t.is_empty() {
        return Some(None);
    }
    let v = match t.split_once([':', '/', 'x']) {
        Some((a, b)) => a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?,
        None => t.parse::<f64>().ok()?,
    };
    (v.is_finite() && (0.2..=10.0).contains(&v)).then_some(Some(v))
}

/// An aspect ratio as [`parse_aspect`] reads it back: the common ratios by name.
pub fn format_aspect(aspect: Option<f64>) -> String {
    let Some(a) = aspect else {
        return String::new();
    };
    for (w, h) in [(4, 3), (16, 9), (16, 10), (21, 9), (5, 4), (3, 2), (1, 1)] {
        if (a - w as f64 / h as f64).abs() < 1e-3 {
            return format!("{w}:{h}");
        }
    }
    format!("{a:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// What the application knows about the video stream shown.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoMeta {
    pub width: u32,
    pub height: u32,
    /// Display aspect ratio of the picture.
    pub aspect: f64,
    pub fps: Option<f64>,
    pub codec: String,
    /// Video streams for the video track menu, and the one shown.
    pub streams: Vec<String>,
    pub current: usize,
    /// Audio streams for the audio track menu, and the one heard.
    pub audio_streams: Vec<String>,
    pub audio_current: Option<usize>,
}

impl VideoMeta {
    /// Picture size as shown: the coded height, and the width that gives the aspect ratio.
    pub fn display_size(&self, aspect: Option<f64>) -> (u32, u32) {
        let a = aspect.unwrap_or(self.aspect);
        if a > 0.0 && self.height > 0 {
            (
                (self.height as f64 * a).round().max(1.0) as u32,
                self.height,
            )
        } else {
            (self.width, self.height)
        }
    }

    /// Short description for the info text, for example "640x360 25 fps".
    pub fn summary(&self) -> String {
        let (w, h) = self.display_size(None);
        match self.fps {
            Some(f) if (f - f.round()).abs() < 0.01 => format!("{w}x{h} {} fps", f.round()),
            Some(f) => format!("{w}x{h} {f:.2} fps"),
            None => format!("{w}x{h}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VideoStatus {
    /// The file is being opened.
    Opening,
    Ready(VideoMeta),
    /// The file has no picture (or it cannot be decoded).
    NoVideo,
}

/// The picture of the playing file, provided by the application (see
/// `PlayerHandle::open_video`).
pub trait VideoSource {
    fn status(&self) -> VideoStatus;
    /// The heard position in the file, and whether it advances.
    fn set_clock(&self, position_ms: u64, playing: bool);
    /// The picture size wanted (aspect ratio already applied), and vertical flip.
    fn set_output(&self, width: u32, height: u32, flip: bool);
    fn select_stream(&self, index: usize);
    /// Changes with every new picture.
    fn seq(&self) -> u64;
    /// Copies the latest picture into `frame`, centred in the area (x, y, width, height) and
    /// clipped to it. Returns false when there is no picture yet.
    fn blit(&self, frame: &mut Frame, area: (i32, i32, u32, u32)) -> bool;
}

/// Fits a picture of display aspect `aspect` into `w`×`h`, centred: (x, y, width, height).
/// The picture fills the area when `aspect` is not positive.
pub fn fit(aspect: f64, w: u32, h: u32) -> (u32, u32, u32, u32) {
    if w == 0 || h == 0 || aspect.is_nan() || aspect <= 0.0 {
        return (0, 0, w, h);
    }
    let (fw, fh) = if w as f64 / h as f64 > aspect {
        (((h as f64 * aspect).round() as u32).clamp(1, w), h)
    } else {
        (w, ((w as f64 / aspect).round() as u32).clamp(1, h))
    };
    ((w - fw) / 2, (h - fh) / 2, fw, fh)
}

/// Copies a `pw`×`ph` picture centred into `area` of `frame`, clipped to the area.
pub fn blit_centred(frame: &mut Frame, area: (i32, i32, u32, u32), pw: u32, ph: u32, px: &[u32]) {
    let (ax, ay, aw, ah) = area;
    let (w, h) = (pw.min(aw), ph.min(ah));
    let (sx, sy) = ((pw - w) / 2, (ph - h) / 2);
    let (dx, dy) = (ax + ((aw - w) / 2) as i32, ay + ((ah - h) / 2) as i32);
    for row in 0..h {
        let start = ((sy + row) * pw + sx) as usize;
        frame.put_pixels(dx, dy + row as i32, w, 1, &px[start..start + w as usize]);
    }
}

/// Picture zoom (05 §2 buttons and keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    Half,
    Normal,
    Double,
}

impl Zoom {
    fn factor(self) -> f64 {
        match self {
            Self::Half => 0.5,
            Self::Normal => 1.0,
            Self::Double => 2.0,
        }
    }
}

/// Window size (in window units at pixel scale `scale`) whose picture area shows a
/// `w`×`h` picture at `zoom`, snapped to the 25×29 steps.
pub fn window_size_for(w: u32, h: u32, zoom: Zoom, scale: i32) -> (i32, i32) {
    let f = zoom.factor() / scale.max(1) as f64;
    let (aw, ah) = ((w as f64 * f).round() as i32, (h as f64 * f).round() as i32);
    snap_size(aw + 19, ah + 58)
}

/// Menu and key commands of the video window.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoCmd {
    FullScreen,
    LeaveFullScreen,
    Zoom(Zoom),
    Flip,
    VideoTrack(usize),
    AudioTrack(usize),
    Options,
    TogglePause,
}

/// The video window's context menu (05 §2).
pub fn context_menu(meta: Option<&VideoMeta>, fullscreen: bool, flip: bool) -> Vec<Item> {
    let v = |c: VideoCmd| MenuCmd::Video(c);
    let tracks = |names: &[String], current: Option<usize>, cmd: fn(usize) -> VideoCmd| {
        let items: Vec<Item> = names
            .iter()
            .enumerate()
            .map(|(i, n)| Item::check(n, "", current == Some(i), v(cmd(i))))
            .collect();
        if items.is_empty() {
            vec![Item::disabled("(none)")]
        } else {
            items
        }
    };
    let (audio, video) = match meta {
        Some(m) => (
            tracks(&m.audio_streams, m.audio_current, VideoCmd::AudioTrack),
            tracks(&m.streams, Some(m.current), VideoCmd::VideoTrack),
        ),
        None => (
            tracks(&[], None, VideoCmd::AudioTrack),
            tracks(&[], None, VideoCmd::VideoTrack),
        ),
    };
    vec![
        Item::check(
            "Full screen",
            "Alt+Enter",
            fullscreen,
            v(VideoCmd::FullScreen),
        ),
        Item::key("Normal size", "1", v(VideoCmd::Zoom(Zoom::Normal))),
        Item::key("Half size", "3", v(VideoCmd::Zoom(Zoom::Half))),
        Item::key("Double size", "2", v(VideoCmd::Zoom(Zoom::Double))),
        Item::check("Vertically flip", "Shift+F", flip, v(VideoCmd::Flip)),
        Item::Sep,
        Item::sub("Audio track", audio),
        Item::sub("Video track", video),
        Item::Sep,
        Item::cmd("Video options...", v(VideoCmd::Options)),
    ]
}

/// Keys of the video window (05 §2). Other keys work as in the main window.
pub fn key(code: KeyCode, m: Mods, fullscreen: bool) -> Option<VideoCmd> {
    use KeyCode::*;
    match (code, m.ctrl, m.alt, m.shift) {
        (Escape, false, false, false) if fullscreen => Some(VideoCmd::LeaveFullScreen),
        (Enter | NumpadEnter, false, true, false) => Some(VideoCmd::FullScreen),
        (Digit1 | Numpad1, false, false, false) => Some(VideoCmd::Zoom(Zoom::Normal)),
        (Digit2 | Numpad2, false, false, false) => Some(VideoCmd::Zoom(Zoom::Double)),
        (Digit3 | Numpad3 | Backquote, false, false, false) => Some(VideoCmd::Zoom(Zoom::Half)),
        (Space, false, false, false) => Some(VideoCmd::TogglePause),
        (KeyF, false, false, true) => Some(VideoCmd::Flip),
        _ => None,
    }
}

/// What a mouse event in the video window asks the shell to do.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoAction {
    Close,
    BeginMove { x: i32, y: i32 },
    Resize { w: i32, h: i32 },
    Button { button: VideoButton, x: i32, y: i32 },
    ContextMenu { x: i32, y: i32 },
    ToggleFullScreen,
    Redraw,
}

/// Window size and pointer state.
#[derive(Debug, Clone)]
pub struct VideoWindow {
    pub w: i32,
    pub h: i32,
    /// Button held down, and whether the pointer is still over it.
    held: Option<(VideoButton, bool)>,
    close_held: Option<bool>,
    /// Resizing: the pointer's offset from the bottom-right corner.
    resize: Option<(i32, i32)>,
}

impl VideoWindow {
    pub fn new(size: (i32, i32)) -> Self {
        let (w, h) = snap_size(size.0, size.1);
        Self {
            w,
            h,
            held: None,
            close_held: None,
            resize: None,
        }
    }

    pub fn view(&self, active: bool) -> vw::VideoView {
        vw::VideoView {
            w: self.w,
            h: self.h,
            active,
            pressed: self.held.filter(|h| h.1).map(|h| h.0),
            close_pressed: self.close_held == Some(true),
        }
    }

    /// Sets the size (snapped). Returns whether it changed.
    pub fn set_size(&mut self, w: i32, h: i32) -> bool {
        let s = snap_size(w, h);
        let changed = s != (self.w, self.h);
        (self.w, self.h) = s;
        changed
    }

    pub fn is_busy(&self) -> bool {
        self.held.is_some() || self.close_held.is_some() || self.resize.is_some()
    }

    pub fn in_close(&self, x: i32, y: i32) -> bool {
        vw::in_close(x, y, self.w)
    }

    pub fn in_grip(&self, x: i32, y: i32) -> bool {
        vw::in_grip(x, y, self.w, self.h)
    }

    pub fn in_picture(&self, x: i32, y: i32) -> bool {
        let (vx, vy, vw_, vh) = vw::video_rect(self.w, self.h);
        (vx..vx + vw_).contains(&x) && (vy..vy + vh).contains(&y)
    }

    pub fn mouse_down(&mut self, x: i32, y: i32, double: bool) -> Vec<VideoAction> {
        if self.in_close(x, y) {
            self.close_held = Some(true);
            return vec![VideoAction::Redraw];
        }
        if let Some(b) = vw::button_at(x, y, self.h) {
            self.held = Some((b, true));
            return vec![VideoAction::Redraw];
        }
        if self.in_grip(x, y) {
            self.resize = Some((self.w - x, self.h - y));
            return Vec::new();
        }
        if self.in_picture(x, y) {
            return if double {
                vec![VideoAction::ToggleFullScreen]
            } else {
                Vec::new()
            };
        }
        vec![VideoAction::BeginMove { x, y }]
    }

    pub fn mouse_move(&mut self, x: i32, y: i32) -> Vec<VideoAction> {
        if let Some((dx, dy)) = self.resize {
            let (w, h) = snap_size(x + dx, y + dy);
            if (w, h) != (self.w, self.h) {
                return vec![VideoAction::Resize { w, h }];
            }
            return Vec::new();
        }
        if let Some((b, over)) = self.held {
            let now = vw::button_at(x, y, self.h) == Some(b);
            if now != over {
                self.held = Some((b, now));
                return vec![VideoAction::Redraw];
            }
        }
        if let Some(over) = self.close_held {
            let now = self.in_close(x, y);
            if now != over {
                self.close_held = Some(now);
                return vec![VideoAction::Redraw];
            }
        }
        Vec::new()
    }

    pub fn mouse_up(&mut self, x: i32, y: i32) -> Vec<VideoAction> {
        self.resize = None;
        let mut acts = vec![VideoAction::Redraw];
        if self.close_held.take().is_some() && self.in_close(x, y) {
            acts.push(VideoAction::Close);
        }
        if let Some((b, _)) = self.held.take()
            && vw::button_at(x, y, self.h) == Some(b)
        {
            acts.push(VideoAction::Button { button: b, x, y });
        }
        acts
    }

    pub fn right_click(&mut self, x: i32, y: i32) -> Vec<VideoAction> {
        vec![VideoAction::ContextMenu { x, y }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_aspect_ratio() {
        assert_eq!(fit(16.0 / 9.0, 256, 174), (0, 15, 256, 144));
        assert_eq!(fit(4.0 / 3.0, 400, 150), (100, 0, 200, 150));
        assert_eq!(fit(0.0, 10, 20), (0, 0, 10, 20));
        assert_eq!(fit(2.0, 0, 20), (0, 0, 0, 20));
    }

    #[test]
    fn centred_copies_are_clipped() {
        let mut f = Frame::new(6, 4, 9);
        // A 4×2 picture into a 2×4 area at (1, 0): the middle two columns, centred vertically.
        blit_centred(&mut f, (1, 0, 2, 4), 4, 2, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(f.pixel(1, 1), 2);
        assert_eq!(f.pixel(2, 2), 7);
        assert_eq!(f.pixel(0, 1), 9);
        assert_eq!(f.pixel(1, 0), 9);
        assert_eq!(f.pixel(3, 1), 9);
    }

    #[test]
    fn aspect_ratios() {
        assert_eq!(parse_aspect(""), Some(None));
        assert_eq!(parse_aspect("2"), Some(Some(2.0)));
        assert_eq!(parse_aspect(" 16:9 "), Some(Some(16.0 / 9.0)));
        assert_eq!(parse_aspect("4/3"), Some(Some(4.0 / 3.0)));
        assert_eq!(parse_aspect("wide"), None);
        assert_eq!(parse_aspect("0:1"), None);
        assert_eq!(format_aspect(Some(16.0 / 9.0)), "16:9");
        assert_eq!(format_aspect(Some(1.85)), "1.85");
        assert_eq!(format_aspect(None), "");
    }

    fn meta() -> VideoMeta {
        VideoMeta {
            width: 720,
            height: 576,
            aspect: 16.0 / 9.0,
            fps: Some(25.0),
            codec: "MPEG-2 video".into(),
            streams: vec!["1: mpeg2video 720x576".into()],
            current: 0,
            audio_streams: vec!["1: mp2 2ch".into(), "2: ac3 6ch [fra]".into()],
            audio_current: Some(0),
        }
    }

    #[test]
    fn display_size_and_summary() {
        let m = meta();
        assert_eq!(m.display_size(None), (1024, 576));
        assert_eq!(m.display_size(Some(4.0 / 3.0)), (768, 576));
        assert_eq!(m.summary(), "1024x576 25 fps");
    }

    #[test]
    fn zoom_sizes_snap_to_steps() {
        // 320×240 at 100%: area 320×240 needs 339×298, snapped up to 350×319.
        assert_eq!(window_size_for(320, 240, Zoom::Normal, 1), (350, 319));
        assert_eq!(window_size_for(320, 240, Zoom::Half, 1), (275, 203));
        // At double size, window units are two pixels.
        assert_eq!(window_size_for(320, 240, Zoom::Normal, 2), (275, 203));
    }

    #[test]
    fn mouse() {
        let mut w = VideoWindow::new((275, 232));
        assert_eq!(
            w.mouse_down(5, 5, false),
            vec![VideoAction::BeginMove { x: 5, y: 5 }]
        );
        assert!(w.mouse_down(50, 50, false).is_empty());
        assert_eq!(
            w.mouse_down(50, 50, true),
            vec![VideoAction::ToggleFullScreen]
        );
        // Press a button, slide off and back, release on it.
        w.mouse_down(40, 210, false);
        assert_eq!(w.view(true).pressed, Some(VideoButton::Zoom200));
        w.mouse_move(120, 210);
        assert_eq!(w.view(true).pressed, None);
        w.mouse_move(40, 210);
        let acts = w.mouse_up(40, 210);
        assert!(acts.contains(&VideoAction::Button {
            button: VideoButton::Zoom200,
            x: 40,
            y: 210
        }));
        // Close.
        w.mouse_down(265, 5, false);
        assert!(w.view(true).close_pressed);
        assert!(w.mouse_up(265, 5).contains(&VideoAction::Close));
        // Resize from the grip, in 25×29 steps.
        w.mouse_down(272, 229, false);
        assert_eq!(
            w.mouse_move(300, 260),
            vec![VideoAction::Resize { w: 325, h: 290 }]
        );
        w.mouse_up(300, 260);
        assert!(!w.is_busy());
    }

    #[test]
    fn keys() {
        let none = Mods::NONE;
        let alt = Mods { alt: true, ..none };
        let shift = Mods {
            shift: true,
            ..none
        };
        assert_eq!(
            key(KeyCode::Escape, none, true),
            Some(VideoCmd::LeaveFullScreen)
        );
        assert_eq!(key(KeyCode::Escape, none, false), None);
        assert_eq!(key(KeyCode::Enter, alt, false), Some(VideoCmd::FullScreen));
        assert_eq!(
            key(KeyCode::Backquote, none, false),
            Some(VideoCmd::Zoom(Zoom::Half))
        );
        assert_eq!(key(KeyCode::KeyF, shift, false), Some(VideoCmd::Flip));
        assert_eq!(key(KeyCode::KeyF, none, false), None);
    }

    #[test]
    fn menu_lists_tracks() {
        let items = context_menu(Some(&meta()), false, true);
        let sub = |label: &str| {
            items.iter().find_map(|i| match i {
                Item::Sub { label: l, items } if l == label => Some(items.clone()),
                _ => None,
            })
        };
        let audio = sub("Audio track").unwrap();
        assert_eq!(audio.len(), 2);
        assert!(matches!(
            &audio[0],
            Item::Cmd {
                check: Some(true),
                ..
            }
        ));
        assert!(matches!(
            &audio[1],
            Item::Cmd {
                cmd: MenuCmd::Video(VideoCmd::AudioTrack(1)),
                ..
            }
        ));
        assert!(matches!(
            items
                .iter()
                .find(|i| matches!(i, Item::Cmd { label, .. } if label == "Vertically flip")),
            Some(Item::Cmd {
                check: Some(true),
                ..
            })
        ));
        let none = context_menu(None, false, false);
        assert!(matches!(&none[6], Item::Sub { items, .. } if items.len() == 1));
    }
}
