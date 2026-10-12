//! The "Visualization" page of the preferences dialog: the classic visualizer's mode, refresh
//! rate, analyzer and oscilloscope options (06 §6.2), laid out like Winamp's page.

use nulamp_skinui::{Form, FormEvent, Rect, Widget};
use nulamp_vis::{AnalyzerStyle, RefreshRate, ScopeStyle, Speed, VisMode, VisSettings};

/// Radio groups; a group's buttons follow it in order (`MODE + 1 + index`).
const MODE: u32 = 500;
const STYLE: u32 = 510;
const BAND: u32 = 520;
const SCOPE: u32 = 530;
const PEAKS: u32 = 540;
const REFRESH: u32 = 541;
const FALLOFF: u32 = 542;
const PEAK_FALLOFF: u32 = 543;

const MODES: [(VisMode, &str); 3] = [
    (VisMode::Spectrum, "Spectrum analyzer"),
    (VisMode::Oscilloscope, "Oscilloscope"),
    (VisMode::Off, "Off"),
];
const STYLES: [(AnalyzerStyle, &str); 3] = [
    (AnalyzerStyle::Normal, "Normal"),
    (AnalyzerStyle::Fire, "Fire"),
    (AnalyzerStyle::Line, "Line"),
];
const BANDS: [(bool, &str); 2] = [(false, "Thin"), (true, "Thick")];
const SCOPES: [(ScopeStyle, &str); 3] = [
    (ScopeStyle::Dots, "Dots"),
    (ScopeStyle::Lines, "Lines"),
    (ScopeStyle::Solid, "Solid"),
];
/// Refresh slider steps, slowest first, with the frame rates the menu shows.
const RATES: [(RefreshRate, &str); 4] = [
    (RefreshRate::Eighth, "9 fps"),
    (RefreshRate::Quarter, "18 fps"),
    (RefreshRate::Half, "35 fps"),
    (RefreshRate::Full, "70 fps"),
];

/// Radio buttons in a row from `x`, each `widths[i]` wide; button `i` gets id `group + 1 + i`.
fn radios<T: PartialEq>(
    widgets: &mut Vec<Widget>,
    group: u32,
    choices: &[(T, &str)],
    current: &T,
    mut x: i32,
    y: i32,
    widths: &[i32],
) {
    for (i, ((value, label), w)) in choices.iter().zip(widths).enumerate() {
        widgets.push(Widget::Radio {
            rect: Rect::new(x, y, *w - 4, 16),
            id: group + 1 + i as u32,
            group,
            label: (*label).into(),
            on: value == current,
        });
        x += w;
    }
}

/// The chosen value of a radio group.
fn chosen<T: Copy>(form: &Form, group: u32, choices: &[(T, &str)]) -> Option<T> {
    let id = form.radio(group)?;
    choices.get((id - group - 1) as usize).map(|c| c.0)
}

fn label(widgets: &mut Vec<Widget>, x: i32, y: i32, w: i32, text: &str) {
    widgets.push(Widget::Label {
        rect: Rect::new(x, y + 1, w, 14),
        text: text.into(),
    });
}

/// A "Slow [slider] Fast" row for a falloff speed.
fn speed_row(widgets: &mut Vec<Widget>, x: i32, y: i32, name: &str, id: u32, speed: Speed) {
    label(widgets, x, y, 80, name);
    label(widgets, x + 84, y, 28, "Slow");
    widgets.push(Widget::Slider {
        rect: Rect::new(x + 114, y, 140, 16),
        id,
        value: Speed::ALL.iter().position(|&s| s == speed).unwrap_or(0) as u32,
        max: Speed::ALL.len() as u32 - 1,
    });
    label(widgets, x + 260, y, 30, "Fast");
}

/// The page's widgets, from (`x`, `y`), `w` pixels wide.
pub fn widgets(v: &VisSettings, x: i32, y: i32, w: i32) -> Vec<Widget> {
    let mut out = Vec::new();
    let ix = x + 8;

    out.push(Widget::Group {
        rect: Rect::new(x, y, w, 62),
        label: "Classic visualization".into(),
    });
    radios(&mut out, MODE, &MODES, &v.mode, ix, y + 16, &[126, 98, 50]);
    label(&mut out, ix, y + 38, 80, "Refresh rate:");
    let rate = RATES.iter().position(|r| r.0 == v.refresh).unwrap_or(2);
    out.push(Widget::Slider {
        rect: Rect::new(ix + 84, y + 38, 160, 16),
        id: REFRESH,
        value: rate as u32,
        max: RATES.len() as u32 - 1,
    });
    out.push(Widget::Label {
        rect: Rect::new(ix + 252, y + 39, 48, 14),
        text: RATES[rate].1.into(),
    });

    let y = y + 66;
    out.push(Widget::Group {
        rect: Rect::new(x, y, w, 104),
        label: "Spectrum analyzer".into(),
    });
    label(&mut out, ix, y + 16, 80, "Coloring:");
    radios(
        &mut out,
        STYLE,
        &STYLES,
        &v.style,
        ix + 84,
        y + 16,
        &[62, 50, 50],
    );
    out.push(Widget::Check {
        rect: Rect::new(ix, y + 38, 80, 16),
        id: PEAKS,
        label: "Show peaks".into(),
        on: v.peaks,
    });
    label(&mut out, ix + 84, y + 38, 34, "Band:");
    radios(
        &mut out,
        BAND,
        &BANDS,
        &v.thick_bands,
        ix + 122,
        y + 38,
        &[50, 56],
    );
    speed_row(&mut out, ix, y + 60, "Falloff:", FALLOFF, v.falloff);
    speed_row(
        &mut out,
        ix,
        y + 82,
        "Peak falloff:",
        PEAK_FALLOFF,
        v.peak_falloff,
    );

    let y = y + 108;
    out.push(Widget::Group {
        rect: Rect::new(x, y, w, 40),
        label: "Oscilloscope".into(),
    });
    label(&mut out, ix, y + 16, 80, "Drawing:");
    radios(
        &mut out,
        SCOPE,
        &SCOPES,
        &v.scope,
        ix + 84,
        y + 16,
        &[50, 56, 56],
    );
    out
}

/// The refresh-rate label is the only widget without an id; find it by its place after the
/// slider.
fn refresh_label(form: &mut Form) -> Option<&mut String> {
    let at = form.widgets.iter().position(|w| w.id() == Some(REFRESH))?;
    match form.widgets.get_mut(at + 1) {
        Some(Widget::Label { text, .. }) => Some(text),
        _ => None,
    }
}

/// Keeps the frame-rate text next to the refresh slider current. Returns true when the event
/// was the slider's.
pub fn event(form: &mut Form, ev: &FormEvent) -> bool {
    if *ev != FormEvent::Changed(REFRESH) {
        return false;
    }
    let rate = form.value(REFRESH).unwrap_or(2) as usize;
    if let Some(text) = refresh_label(form) {
        *text = RATES[rate.min(RATES.len() - 1)].1.into();
    }
    true
}

/// Copies the page's controls into `v`.
pub fn collect(form: &Form, v: &mut VisSettings) {
    if let Some(m) = chosen(form, MODE, &MODES) {
        v.mode = m;
    }
    if let Some(s) = chosen(form, STYLE, &STYLES) {
        v.style = s;
    }
    if let Some(t) = chosen(form, BAND, &BANDS) {
        v.thick_bands = t;
    }
    if let Some(s) = chosen(form, SCOPE, &SCOPES) {
        v.scope = s;
    }
    if form.find(PEAKS).is_some() {
        v.peaks = form.checked(PEAKS);
    }
    let speed = |id| {
        form.value(id)
            .and_then(|i| Speed::ALL.get(i as usize).copied())
    };
    if let Some(s) = speed(FALLOFF) {
        v.falloff = s;
    }
    if let Some(s) = speed(PEAK_FALLOFF) {
        v.peak_falloff = s;
    }
    if let Some(r) = form.value(REFRESH).and_then(|i| RATES.get(i as usize)) {
        v.refresh = r.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(v: &VisSettings) -> Form {
        Form::new(widgets(v, 134, 6, 316))
    }

    #[test]
    fn shows_the_settings() {
        let v = VisSettings::default();
        let f = form(&v);
        assert_eq!(f.radio(MODE), Some(MODE + 1));
        assert_eq!(f.radio(STYLE), Some(STYLE + 1));
        assert_eq!(f.radio(BAND), Some(BAND + 2));
        assert_eq!(f.radio(SCOPE), Some(SCOPE + 2));
        assert!(f.checked(PEAKS));
        assert_eq!(f.value(REFRESH), Some(1));
        assert_eq!(f.value(FALLOFF), Some(2));
        assert_eq!(f.value(PEAK_FALLOFF), Some(1));
        let mut back = VisSettings {
            mode: VisMode::Off,
            peaks: false,
            ..v
        };
        collect(&f, &mut back);
        assert_eq!(back, v);
    }

    #[test]
    fn changes_reach_the_settings() {
        let mut f = form(&VisSettings::default());
        let click = |f: &mut Form, id: u32| {
            let r = f.find(id).unwrap().rect();
            f.mouse_down(r.x + 4, r.y + 8);
            f.mouse_up(r.x + 4, r.y + 8)
        };
        assert_eq!(click(&mut f, MODE + 2), Some(FormEvent::Clicked(MODE + 2)));
        click(&mut f, STYLE + 2);
        click(&mut f, BAND + 1);
        click(&mut f, SCOPE + 3);
        click(&mut f, PEAKS);
        assert!(!f.checked(MODE + 1), "one mode at a time");
        // The refresh slider's right end is the full rate; its label follows.
        let r = f.find(REFRESH).unwrap().rect();
        f.mouse_down(r.x + r.w - 1, r.y + 6);
        let ev = f.mouse_up(r.x + r.w - 1, r.y + 6).unwrap();
        assert!(event(&mut f, &ev));
        assert_eq!(refresh_label(&mut f).unwrap(), "70 fps");
        // Dragging the falloff thumb to the left end.
        let r = f.find(FALLOFF).unwrap().rect();
        f.mouse_down(r.x + r.w / 2, r.y + 6);
        f.mouse_move(r.x - 50, r.y + 30);
        f.mouse_up(r.x - 50, r.y + 30);
        let mut v = VisSettings::default();
        collect(&f, &mut v);
        assert_eq!(
            v,
            VisSettings {
                mode: VisMode::Oscilloscope,
                style: AnalyzerStyle::Fire,
                thick_bands: false,
                peaks: false,
                falloff: Speed::Slowest,
                peak_falloff: Speed::Slow,
                scope: ScopeStyle::Solid,
                refresh: RefreshRate::Full,
            }
        );
    }

    #[test]
    fn fits_the_page() {
        let ws = widgets(&VisSettings::default(), 134, 6, 316);
        for w in &ws {
            let r = w.rect();
            assert!(r.x >= 134 && r.x + r.w <= 134 + 316, "{w:?}");
            assert!(r.y + r.h <= 250, "{w:?}");
        }
    }
}
