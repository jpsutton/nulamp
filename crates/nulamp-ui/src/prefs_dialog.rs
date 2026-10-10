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

const PAGES: [&str; 6] = [
    "Classic skins",
    "Playlist",
    "Playback",
    "Equalizer",
    "Visualization",
    "Skins",
];
const PAGE_LIST: u32 = 1;
const OK: u32 = 2;
const CANCEL: u32 = 3;
const APPLY: u32 = 4;
const OPT: u32 = 100;
const NUM: u32 = 200;

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
        _ => vec![],
    }
}

pub struct PrefsDialog {
    form: Form,
    prefs: UiPrefs,
    pl: PlaylistOptions,
    page: usize,
}

impl PrefsDialog {
    pub fn new(prefs: UiPrefs, pl: PlaylistOptions) -> Self {
        let mut d = Self {
            form: Form::default(),
            prefs,
            pl,
            page: 0,
        };
        d.build();
        d
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
                });
            }
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
}
