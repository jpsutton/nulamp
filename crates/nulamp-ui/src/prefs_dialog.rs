//! Preferences dialog (11 §8): a page list and the page's options.

use nulamp_skinui::{Form, FormEvent, Rect, Widget};

use crate::dialogs::{Dialog, DialogResult, Outcome};
use crate::shell::UiPrefs;

/// Playlist options that live in the playlist window state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistOptions {
    pub show_numbers: bool,
    pub zero_pad: bool,
    pub bitmap_font: bool,
    pub page_lines: usize,
    pub wheel_double: bool,
    pub old_school_buttons: bool,
}

const PAGES: [&str; 8] = [
    "Classic skins",
    "Playlist",
    "Playback",
    "Equalizer",
    "Visualization",
    "Skins",
    "Crossfade",
    "Media Library",
];
const LIBRARY_PAGE: usize = 7;
const PAGE_LIST: u32 = 1;
const OK: u32 = 2;
const CANCEL: u32 = 3;
const APPLY: u32 = 4;
const OPT: u32 = 100;
const NUM: u32 = 200;
const FOLDERS: u32 = 300;
const ADD_FOLDER: u32 = 301;
const REMOVE_FOLDER: u32 = 302;
const SUBFOLDERS: u32 = 303;
const RESCAN: u32 = 304;

/// Asks for a folder to add to the library.
fn pick_library_folder() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .set_title("Add folder to the media library")
        .pick_folder()
}

/// How a watch folder is listed.
fn folder_row(path: &str, recursive: bool) -> String {
    if recursive {
        path.to_string()
    } else {
        format!("{path} (no subfolders)")
    }
}

/// One option on a page: a check box bound to a getter/setter pair.
struct CheckOpt {
    label: &'static str,
    get: fn(&UiPrefs, &PlaylistOptions) -> bool,
    set: fn(&mut UiPrefs, &mut PlaylistOptions, bool),
}

macro_rules! opt {
    ($label:expr, p.$($f:ident).+) => {
        CheckOpt { label: $label, get: |p, _| p.$($f).+, set: |p, _, v| p.$($f).+ = v }
    };
    ($label:expr, o.$f:ident) => {
        CheckOpt { label: $label, get: |_, o| o.$f, set: |_, o, v| o.$f = v }
    };
}

fn page_options(page: usize) -> Vec<CheckOpt> {
    match page {
        0 => vec![
            opt!("Dim inactive title bars", p.dim_inactive),
            opt!("Snap windows", p.snap),
            opt!("Keep windows on screen", p.keep_on_screen),
            opt!("EasyMove (drag windows from anywhere)", p.easymove),
            opt!("Always show clutterbar", p.clutterbar_always),
            opt!("Double size applies to the equalizer", p.eq_double),
            opt!("Skinned bitmap font for the song title", p.bitmap_title),
            opt!(
                "Show playlist position in the title",
                p.show_number_in_title
            ),
            opt!("Auto-scroll the song title", p.autoscroll),
            opt!("Always on top", p.always_on_top),
            opt!("Show icon in the system tray", p.show_tray),
            opt!(
                "Global hotkeys (Ctrl+Alt+Insert/Home/End...)",
                p.hotkeys.enabled
            ),
        ],
        1 => vec![
            opt!("Show numbers", o.show_numbers),
            opt!("Zero-pad numbers", o.zero_pad),
            opt!("Skinned bitmap font for playlist rows", o.bitmap_font),
            opt!("Mouse wheel scrolls double", o.wheel_double),
            opt!("Old-school playlist buttons", o.old_school_buttons),
        ],
        2 => vec![
            opt!("Fade on pause and stop", p.fade_pause_stop),
            opt!("Fade on seek", p.fade_seek),
            opt!("Use ReplayGain", p.replaygain.enabled),
            CheckOpt {
                label: "Use album gain (instead of track gain)",
                get: |p, _| p.replaygain.source == nulamp_eq::replaygain::Source::Album,
                set: |p, _, v| {
                    p.replaygain.source = if v {
                        nulamp_eq::replaygain::Source::Album
                    } else {
                        nulamp_eq::replaygain::Source::Track
                    }
                },
            },
            CheckOpt {
                label: "Prevent clipping",
                get: |p, _| {
                    p.replaygain.mode == nulamp_eq::replaygain::Mode::ApplyGainPreventClipping
                },
                set: |p, _, v| {
                    p.replaygain.mode = if v {
                        nulamp_eq::replaygain::Mode::ApplyGainPreventClipping
                    } else {
                        nulamp_eq::replaygain::Mode::ApplyGain
                    }
                },
            },
            opt!("Use only the preferred source", p.replaygain.preferred_only),
        ],
        3 => vec![
            opt!("Enable equalizer", p.eq.enabled),
            opt!("Auto-load presets", p.eq.auto_load),
            opt!("ISO band frequencies", p.eq.iso),
            opt!("Use limiter", p.eq.limiter),
        ],
        4 => vec![
            opt!("Show peaks", p.vis.peaks),
            opt!("Thick bands", p.vis.thick_bands),
        ],
        6 => vec![
            opt!("Crossfade at the end of each track", p.crossfade),
            opt!(
                "Equal-power curve (instead of linear)",
                p.crossfade_equal_power
            ),
            opt!("Also crossfade on manual track changes", p.crossfade_manual),
            opt!("No crossfade within an album (gapless)", p.gapless_albums),
        ],
        LIBRARY_PAGE => vec![
            opt!(
                "Rescan the watch folders on start",
                p.library.rescan_on_start
            ),
            opt!("Watch the folders for changes", p.library.watch),
        ],
        _ => vec![],
    }
}

pub struct PrefsDialog {
    form: Form,
    prefs: UiPrefs,
    pl: PlaylistOptions,
    page: usize,
    /// Selected watch folder on the media-library page.
    folder_sel: Option<usize>,
    /// "Include subfolders" while no folder is selected (used for added folders).
    default_recursive: bool,
    /// Folder picker (a native dialog; replaced in tests).
    pick_folder: fn() -> Option<std::path::PathBuf>,
}

impl PrefsDialog {
    pub fn new(prefs: UiPrefs, pl: PlaylistOptions) -> Self {
        let mut d = Self {
            form: Form::default(),
            prefs,
            pl,
            page: 0,
            folder_sel: None,
            default_recursive: true,
            pick_folder: pick_library_folder,
        };
        d.build();
        d
    }

    /// Opens on a page (by its name in the page list).
    pub fn with_page(mut self, name: &str) -> Self {
        if let Some(i) = PAGES.iter().position(|p| *p == name) {
            self.page = i;
            self.build();
        }
        self
    }

    /// Media-library page controls below the check boxes, from `y`.
    fn library_widgets(&self, widgets: &mut Vec<Widget>, x: i32, y: i32, w: i32, bottom: i32) {
        let folders = &self.prefs.library.folders;
        widgets.push(Widget::Label {
            rect: Rect::new(x, y, w, 14),
            text: "Watch folders:".into(),
        });
        let list_h = (bottom - (y + 18) - 54).max(30);
        widgets.push(Widget::List {
            rect: Rect::new(x, y + 18, w, list_h),
            id: FOLDERS,
            rows: folders.iter().map(|(p, r)| folder_row(p, *r)).collect(),
            selected: self.folder_sel.filter(|&s| s < folders.len()),
            first: 0,
        });
        let by = y + 18 + list_h + 6;
        widgets.push(Widget::Button {
            rect: Rect::new(x, by, 58, 18),
            id: ADD_FOLDER,
            label: "Add...".into(),
            default: false,
        });
        widgets.push(Widget::Button {
            rect: Rect::new(x + 64, by, 58, 18),
            id: REMOVE_FOLDER,
            label: "Remove".into(),
            default: false,
        });
        let recursive = self
            .folder_sel
            .and_then(|i| folders.get(i))
            .map_or(self.default_recursive, |f| f.1);
        widgets.push(Widget::Check {
            rect: Rect::new(x + 130, by + 1, w - 130, 16),
            id: SUBFOLDERS,
            label: "Include subfolders".into(),
            on: recursive,
        });
        widgets.push(Widget::Button {
            rect: Rect::new(x, by + 26, 80, 18),
            id: RESCAN,
            label: "Rescan now".into(),
            default: false,
        });
    }

    /// Handles the media-library page controls; `None` for other events.
    fn library_event(&mut self, ev: &FormEvent) -> Option<Outcome> {
        if self.page != LIBRARY_PAGE {
            return None;
        }
        match ev {
            FormEvent::Selected(FOLDERS, i) => {
                self.collect();
                self.folder_sel = Some(*i);
            }
            FormEvent::Clicked(ADD_FOLDER) => {
                self.collect();
                if let Some(dir) = (self.pick_folder)() {
                    let path = dir.to_string_lossy().into_owned();
                    let recursive = self.form.checked(SUBFOLDERS);
                    let folders = &mut self.prefs.library.folders;
                    match folders.iter().position(|f| f.0 == path) {
                        Some(i) => self.folder_sel = Some(i),
                        None => {
                            folders.push((path, recursive));
                            self.folder_sel = Some(folders.len() - 1);
                        }
                    }
                }
            }
            FormEvent::Clicked(REMOVE_FOLDER) => {
                self.collect();
                let folders = &mut self.prefs.library.folders;
                if let Some(i) = self.folder_sel.filter(|&i| i < folders.len()) {
                    folders.remove(i);
                    self.folder_sel = (!folders.is_empty()).then(|| i.min(folders.len() - 1));
                }
            }
            FormEvent::Clicked(SUBFOLDERS) => {
                let on = self.form.checked(SUBFOLDERS);
                match self
                    .folder_sel
                    .and_then(|i| self.prefs.library.folders.get_mut(i))
                {
                    Some(f) => f.1 = on,
                    None => self.default_recursive = on,
                }
                self.collect();
            }
            FormEvent::Clicked(RESCAN) => {
                return Some(Outcome::Do(vec![DialogResult::LibraryRescan], false));
            }
            _ => return None,
        }
        self.build();
        self.form.focus_id(FOLDERS);
        Some(Outcome::Keep)
    }

    fn build(&mut self) {
        let (w, h) = self.size();
        let (cw, ch) = (w - 19, h - 34);
        let mut widgets = vec![Widget::List {
            rect: Rect::new(6, 6, 120, ch - 40),
            id: PAGE_LIST,
            rows: PAGES.iter().map(|s| s.to_string()).collect(),
            selected: Some(self.page),
            first: 0,
        }];
        let x = 134;
        for (i, o) in page_options(self.page).iter().enumerate() {
            widgets.push(Widget::Check {
                rect: Rect::new(x, 6 + i as i32 * 20, cw - x - 6, 16),
                id: OPT + i as u32,
                label: o.label.into(),
                on: (o.get)(&self.prefs, &self.pl),
            });
        }
        let mut y = 6 + page_options(self.page).len() as i32 * 20 + 6;
        if self.page == LIBRARY_PAGE {
            self.library_widgets(&mut widgets, x, y, cw - x - 6, ch - 32);
        }
        let mut num = |label: &str, value: String, id: u32, widgets: &mut Vec<Widget>| {
            widgets.push(Widget::Label {
                rect: Rect::new(x, y + 2, 150, 14),
                text: label.into(),
            });
            widgets.push(Widget::Edit {
                rect: Rect::new(x + 156, y, 60, 18),
                id,
                text: value,
                cursor: 0,
                secret: false,
            });
            y += 24;
        };
        match self.page {
            0 => num(
                "Snap distance (pixels):",
                self.prefs.snap_distance.to_string(),
                NUM,
                &mut widgets,
            ),
            1 => num(
                "Page scroll lines:",
                self.pl.page_lines.to_string(),
                NUM,
                &mut widgets,
            ),
            2 => {
                num(
                    "ReplayGain preamp (dB):",
                    format!("{:.1}", self.prefs.replaygain.preamp_db),
                    NUM,
                    &mut widgets,
                );
                num(
                    "Gain without tags (dB):",
                    format!("{:.1}", self.prefs.replaygain.no_tag_db),
                    NUM + 1,
                    &mut widgets,
                );
                num(
                    "Buffer length (ms):",
                    self.prefs.buffer_ms.to_string(),
                    NUM + 2,
                    &mut widgets,
                );
            }
            5 => {
                widgets.push(Widget::Label {
                    rect: Rect::new(x, 8, cw - x - 6, 14),
                    text: "Skins folder:".into(),
                });
                widgets.push(Widget::Edit {
                    rect: Rect::new(x, 26, cw - x - 6, 18),
                    id: NUM,
                    text: self.prefs.skins_dir.to_string_lossy().into_owned(),
                    cursor: 0,
                    secret: false,
                });
            }
            6 => num(
                "Crossfade length (ms):",
                self.prefs.crossfade_ms.to_string(),
                NUM,
                &mut widgets,
            ),
            _ => {}
        }
        widgets.push(Widget::Button {
            rect: Rect::new(cw - 192, ch - 26, 58, 18),
            id: OK,
            label: "OK".into(),
            default: true,
        });
        widgets.push(Widget::Button {
            rect: Rect::new(cw - 128, ch - 26, 58, 18),
            id: CANCEL,
            label: "Cancel".into(),
            default: false,
        });
        widgets.push(Widget::Button {
            rect: Rect::new(cw - 64, ch - 26, 58, 18),
            id: APPLY,
            label: "Apply".into(),
            default: false,
        });
        self.form = Form::new(widgets);
        self.form.focus_id(PAGE_LIST);
    }

    /// Copies the visible page's controls back into the preferences.
    fn collect(&mut self) {
        for (i, o) in page_options(self.page).iter().enumerate() {
            let on = self.form.checked(OPT + i as u32);
            (o.set)(&mut self.prefs, &mut self.pl, on);
        }
        let text = self.form.text(NUM);
        match self.page {
            0 => {
                if let Ok(v) = text.trim().parse::<i32>() {
                    self.prefs.snap_distance = v.clamp(0, 100);
                }
            }
            1 => {
                if let Ok(v) = text.trim().parse::<usize>() {
                    self.pl.page_lines = v.clamp(1, 16);
                }
            }
            2 => {
                let db = |s: String| {
                    s.trim()
                        .parse::<f32>()
                        .ok()
                        .map(|v| (v.clamp(-14.0, 6.0) * 10.0).round() / 10.0)
                };
                if let Some(v) = db(text.clone()) {
                    self.prefs.replaygain.preamp_db = v;
                }
                if let Some(v) = db(self.form.text(NUM + 1)) {
                    self.prefs.replaygain.no_tag_db = v;
                }
                if let Ok(v) = self.form.text(NUM + 2).trim().parse::<u32>() {
                    self.prefs.buffer_ms = v.clamp(200, 20_000);
                }
            }
            5 if !text.trim().is_empty() => {
                self.prefs.skins_dir = std::path::PathBuf::from(text.trim());
            }
            6 => {
                if let Ok(v) = text.trim().parse::<u32>() {
                    self.prefs.crossfade_ms = v.min(20_000);
                }
            }
            _ => {}
        }
    }

    fn result(&self) -> DialogResult {
        DialogResult::Prefs(Box::new((self.prefs.clone(), self.pl.clone())))
    }
}

impl Dialog for PrefsDialog {
    fn title(&self) -> String {
        "Preferences".into()
    }

    fn size(&self) -> (i32, i32) {
        (475, 319)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "prefs"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        if let Some(o) = self.library_event(&ev) {
            return o;
        }
        match ev {
            FormEvent::Selected(PAGE_LIST, i) if i != self.page => {
                self.collect();
                self.page = i;
                self.build();
                Outcome::Keep
            }
            FormEvent::Clicked(OK) | FormEvent::Submit => {
                self.collect();
                Outcome::Do(vec![self.result()], true)
            }
            FormEvent::Clicked(APPLY) => {
                self.collect();
                Outcome::Do(vec![self.result()], false)
            }
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl() -> PlaylistOptions {
        PlaylistOptions {
            show_numbers: true,
            zero_pad: false,
            bitmap_font: true,
            page_lines: 1,
            wheel_double: true,
            old_school_buttons: false,
        }
    }

    fn result(o: Outcome) -> UiPrefs {
        match o {
            Outcome::Do(r, _) => match &r[0] {
                DialogResult::Prefs(b) => b.0.clone(),
                _ => panic!("expected preferences"),
            },
            _ => panic!("expected a result"),
        }
    }

    #[test]
    fn media_library_page() {
        let mut prefs = UiPrefs::default();
        prefs.library.folders = vec![("/music".into(), true)];
        let mut d = PrefsDialog::new(prefs, pl()).with_page("Media Library");
        d.pick_folder = || Some(std::path::PathBuf::from("/audiobooks"));
        assert_eq!(d.page, LIBRARY_PAGE);
        // The two options are the first check boxes.
        assert!(d.form.checked(OPT) && d.form.checked(OPT + 1));
        d.form.mouse_down(140, 30);
        d.form.mouse_up(140, 30);
        // Include subfolders off, then add a folder: it is added without subfolders.
        d.collect();
        d.default_recursive = false;
        d.build();
        assert!(!d.form.checked(SUBFOLDERS));
        assert!(matches!(
            d.handle(FormEvent::Clicked(ADD_FOLDER)),
            Outcome::Keep
        ));
        assert_eq!(d.folder_sel, Some(1));
        // Select the first folder, turn its subfolders off, then remove the second.
        d.handle(FormEvent::Selected(FOLDERS, 0));
        assert!(d.form.checked(SUBFOLDERS));
        if let Some(Widget::Check { on, .. }) = d.form.find_mut(SUBFOLDERS) {
            *on = false;
        }
        d.handle(FormEvent::Clicked(SUBFOLDERS));
        d.handle(FormEvent::Selected(FOLDERS, 1));
        d.handle(FormEvent::Clicked(REMOVE_FOLDER));
        assert!(matches!(
            d.handle(FormEvent::Clicked(RESCAN)),
            Outcome::Do(ref r, false) if matches!(r[..], [DialogResult::LibraryRescan])
        ));
        let p = result(d.handle(FormEvent::Clicked(OK)));
        assert_eq!(p.library.folders, [("/music".to_string(), false)]);
        assert!(p.library.rescan_on_start);
        assert!(!p.library.watch, "the watch option was turned off");
    }

    #[test]
    fn toggles_and_page_switch_keep_values() {
        let mut d = PrefsDialog::new(UiPrefs::default(), pl());
        // Page 0, first option: dim inactive (default on) → off.
        d.form.mouse_down(140, 10);
        d.form.mouse_up(140, 10);
        d.handle(FormEvent::Selected(PAGE_LIST, 1));
        d.form.mouse_down(140, 30);
        d.form.mouse_up(140, 30);
        match d.handle(FormEvent::Clicked(OK)) {
            Outcome::Do(r, true) => match &r[0] {
                DialogResult::Prefs(b) => {
                    assert!(!b.0.dim_inactive);
                    assert!(b.1.zero_pad);
                }
                _ => panic!(),
            },
            _ => panic!("expected OK result"),
        }
    }

    #[test]
    fn crossfade_page() {
        let mut d = PrefsDialog::new(UiPrefs::default(), pl());
        d.handle(FormEvent::Selected(PAGE_LIST, 6));
        // First option: crossfade (default off) → on.
        d.form.mouse_down(140, 10);
        d.form.mouse_up(140, 10);
        d.form.set_text(NUM, "5000");
        match d.handle(FormEvent::Clicked(OK)) {
            Outcome::Do(r, true) => match &r[0] {
                DialogResult::Prefs(b) => {
                    assert!(b.0.crossfade && b.0.gapless_albums);
                    assert_eq!(b.0.crossfade_ms, 5000);
                }
                _ => panic!(),
            },
            _ => panic!("expected OK result"),
        }
    }
}
