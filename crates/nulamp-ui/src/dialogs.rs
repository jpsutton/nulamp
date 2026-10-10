//! Dialogs shown in generic skinned windows: Open URL, Jump to time, Jump to file (10 §6),
//! About, and the skin browser (01 §1.4–§1.5).

use nulamp_core::EntryId;
use nulamp_skinui::{Form, FormEvent, Rect, Widget};

/// What a dialog asks the application to do.
#[derive(Debug, Clone)]
pub enum DialogResult {
    /// Open (replace and play) or add a location.
    OpenLocation {
        location: String,
        add: bool,
    },
    SeekTo(u64),
    PlayEntry(EntryId),
    EnqueueEntry(EntryId),
    LoadSkin(Option<String>),
    /// Tags of a file were written: refresh entries with this location.
    TagsWritten(String),
    /// New preferences from the preferences dialog.
    Prefs(Box<(crate::shell::UiPrefs, crate::prefs_dialog::PlaylistOptions)>),
    /// Save the current EQ under a name, in the preset or auto-load library.
    EqSave {
        name: String,
        auto: bool,
    },
}

pub enum Outcome {
    Keep,
    Close,
    Do(Vec<DialogResult>, bool),
}

pub trait Dialog {
    fn title(&self) -> String;
    /// Window size (including the generic frame).
    fn size(&self) -> (i32, i32);
    fn form(&mut self) -> &mut Form;
    fn handle(&mut self, ev: FormEvent) -> Outcome;
    /// Identifies single-instance dialogs (reopening brings the existing one to the front).
    fn kind(&self) -> &'static str;
}

const OK: u32 = 1;
const CANCEL: u32 = 2;
const EDIT: u32 = 3;
const LIST: u32 = 4;
const ENQUEUE: u32 = 5;

/// Client width for a window width.
fn cw(w: i32) -> i32 {
    w - 19
}

// ----- Open URL -----

pub struct UrlDialog {
    form: Form,
    add: bool,
}

impl UrlDialog {
    pub fn new(add: bool, recent: &[String]) -> Self {
        let w = cw(375);
        let mut form = Form::new(vec![
            Widget::Label {
                rect: Rect::new(6, 6, w - 12, 14),
                text: "Enter a URL or a path to open:".into(),
            },
            Widget::Edit {
                rect: Rect::new(6, 24, w - 12, 18),
                id: EDIT,
                text: String::new(),
                cursor: 0,
            },
            Widget::Button {
                rect: Rect::new(w - 128, 52, 58, 18),
                id: OK,
                label: if add { "Add" } else { "Open" }.into(),
                default: true,
            },
            Widget::Button {
                rect: Rect::new(w - 64, 52, 58, 18),
                id: CANCEL,
                label: "Cancel".into(),
                default: false,
            },
        ]);
        if let Some(r) = recent.first() {
            form.set_text(EDIT, r);
        }
        form.focus_id(EDIT);
        Self { form, add }
    }
}

/// Adds `http://` to input without a scheme (10 §2.1).
pub fn normalize_url(s: &str) -> String {
    let s = s.trim();
    if s.contains("://")
        || s.starts_with('/')
        || s.is_empty()
        || (s.len() > 1 && s.as_bytes()[1] == b':')
    {
        s.to_string()
    } else {
        format!("http://{s}")
    }
}

impl Dialog for UrlDialog {
    fn title(&self) -> String {
        if self.add {
            "Add URL".into()
        } else {
            "Open URL".into()
        }
    }

    fn size(&self) -> (i32, i32) {
        (375, 116)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "url"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(OK) | FormEvent::Submit => {
                let location = normalize_url(&self.form.text(EDIT));
                if location.is_empty() {
                    return Outcome::Keep;
                }
                Outcome::Do(
                    vec![DialogResult::OpenLocation {
                        location,
                        add: self.add,
                    }],
                    true,
                )
            }
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

// ----- Jump to time -----

pub struct JumpTimeDialog {
    form: Form,
    length_ms: u64,
}

/// Parses `SS` or `M:SS` (07 §2.5) into milliseconds.
pub fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.split_once(':') {
        Some((m, sec)) => {
            Some((m.trim().parse::<u64>().ok()? * 60 + sec.trim().parse::<u64>().ok()?) * 1000)
        }
        None => Some(s.parse::<u64>().ok()? * 1000),
    }
}

impl JumpTimeDialog {
    pub fn new(position_ms: u64, length_ms: u64) -> Self {
        let f = |ms: u64| crate::marquee::format_length(ms / 1000);
        let w = cw(275);
        let mut form = Form::new(vec![
            Widget::Label {
                rect: Rect::new(6, 4, w - 12, 14),
                text: format!("Track length: {}", f(length_ms)),
            },
            Widget::Label {
                rect: Rect::new(6, 18, w - 12, 14),
                text: "Jump to (M:SS):".into(),
            },
            Widget::Edit {
                rect: Rect::new(6, 34, 90, 18),
                id: EDIT,
                text: String::new(),
                cursor: 0,
            },
            Widget::Button {
                rect: Rect::new(w - 128, 60, 58, 18),
                id: OK,
                label: "Jump".into(),
                default: true,
            },
            Widget::Button {
                rect: Rect::new(w - 64, 60, 58, 18),
                id: CANCEL,
                label: "Cancel".into(),
                default: false,
            },
        ]);
        form.set_text(EDIT, &f(position_ms));
        form.focus_id(EDIT);
        Self { form, length_ms }
    }
}

impl Dialog for JumpTimeDialog {
    fn title(&self) -> String {
        "Jump to time".into()
    }

    fn size(&self) -> (i32, i32) {
        (275, 116)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "jumptime"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(OK) | FormEvent::Submit => match parse_time(&self.form.text(EDIT)) {
                Some(ms) => Outcome::Do(vec![DialogResult::SeekTo(ms.min(self.length_ms))], true),
                None => Outcome::Keep,
            },
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

// ----- Jump to file -----

/// Splits a query into tokens: runs of letters and digits, or "quoted phrases" (10 §6).
pub fn tokens(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in query.chars() {
        if c == '"' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            quoted = !quoted;
        } else if quoted || c.is_alphanumeric() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|t| t.to_lowercase()).collect()
}

/// Whether an entry matches every token (case-insensitive substring of "location title").
pub fn matches(tokens: &[String], location: &str, title: &str) -> bool {
    let hay = format!("{location} {title}").to_lowercase();
    tokens.iter().all(|t| hay.contains(t.as_str()))
}

pub struct JumpFileDialog {
    form: Form,
    /// (id, location, title) for every entry.
    entries: Vec<(EntryId, String, String)>,
    /// Ids shown in the list, in display order.
    shown: Vec<EntryId>,
    current: Option<EntryId>,
}

impl JumpFileDialog {
    pub fn new(entries: Vec<(EntryId, String, String)>, current: Option<EntryId>) -> Self {
        let (w, h) = (375, 290);
        let (cw, ch) = (cw(w), h - 34);
        let form = Form::new(vec![
            Widget::Edit {
                rect: Rect::new(6, 6, cw - 12, 18),
                id: EDIT,
                text: String::new(),
                cursor: 0,
            },
            Widget::List {
                rect: Rect::new(6, 30, cw - 12, ch - 62),
                id: LIST,
                rows: vec![],
                selected: None,
                first: 0,
            },
            Widget::Button {
                rect: Rect::new(cw - 196, ch - 26, 60, 18),
                id: OK,
                label: "Play".into(),
                default: true,
            },
            Widget::Button {
                rect: Rect::new(cw - 130, ch - 26, 60, 18),
                id: ENQUEUE,
                label: "Enqueue".into(),
                default: false,
            },
            Widget::Button {
                rect: Rect::new(cw - 64, ch - 26, 58, 18),
                id: CANCEL,
                label: "Close".into(),
                default: false,
            },
        ]);
        let mut d = Self {
            form,
            entries,
            shown: Vec::new(),
            current,
        };
        d.refilter(true);
        d.form.focus_id(EDIT);
        d
    }

    fn refilter(&mut self, initial: bool) {
        let t = tokens(&self.form.text(EDIT));
        let mut hits: Vec<&(EntryId, String, String)> = self
            .entries
            .iter()
            .filter(|(_, loc, title)| matches(&t, loc, title))
            .collect();
        hits.sort_by_key(|a| a.2.to_lowercase());
        self.shown = hits.iter().map(|e| e.0).collect();
        let rows = hits.iter().map(|e| e.2.clone()).collect();
        let sel = if initial {
            self.current
                .and_then(|c| self.shown.iter().position(|id| *id == c))
                .or(Some(0))
        } else {
            Some(0)
        };
        self.form.set_rows(LIST, rows, sel);
    }

    fn selected_id(&self) -> Option<EntryId> {
        self.form
            .selected(LIST)
            .and_then(|i| self.shown.get(i).copied())
    }
}

impl Dialog for JumpFileDialog {
    fn title(&self) -> String {
        "Jump to file".into()
    }

    fn size(&self) -> (i32, i32) {
        (375, 290)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "jumpfile"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Changed(EDIT) => {
                self.refilter(false);
                Outcome::Keep
            }
            FormEvent::Clicked(OK) | FormEvent::Submit | FormEvent::Activated(LIST, _) => {
                match self.selected_id() {
                    Some(id) if Some(id) != self.current => {
                        Outcome::Do(vec![DialogResult::PlayEntry(id)], true)
                    }
                    _ => Outcome::Close,
                }
            }
            FormEvent::Clicked(ENQUEUE) => match self.selected_id() {
                Some(id) => Outcome::Do(vec![DialogResult::EnqueueEntry(id)], false),
                None => Outcome::Keep,
            },
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

// ----- About -----

pub struct AboutDialog {
    form: Form,
}

impl AboutDialog {
    pub fn new(name: &str, version: &str) -> Self {
        let (w, h) = (300, 232);
        let (cw, ch) = (cw(w), h - 34);
        let lines = vec![
            format!("{name} {version}"),
            String::new(),
            "A cross-platform player for classic skins.".into(),
            "Independent clean-room implementation.".into(),
            "Not affiliated with the original player.".into(),
            String::new(),
            "Licence: MIT OR Apache-2.0".into(),
            String::new(),
            "Keys: Z X C V B = transport,".into(),
            "L = open, J = jump to file,".into(),
            "Ctrl+D = double size, Ctrl+W = shade.".into(),
        ];
        let form = Form::new(vec![
            Widget::TextBox {
                rect: Rect::new(6, 6, cw - 12, ch - 36),
                lines,
                first: 0,
            },
            Widget::Button {
                rect: Rect::new(cw - 64, ch - 26, 58, 18),
                id: OK,
                label: "OK".into(),
                default: true,
            },
        ]);
        Self { form }
    }
}

impl Dialog for AboutDialog {
    fn title(&self) -> String {
        "About".into()
    }

    fn size(&self) -> (i32, i32) {
        (300, 232)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "about"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(OK) | FormEvent::Submit | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

// ----- Skin browser -----

pub struct SkinBrowser {
    form: Form,
    skins: Vec<(String, Option<String>)>,
}

impl SkinBrowser {
    pub fn new(skins: Vec<(String, Option<String>)>, current: Option<&str>) -> Self {
        let (w, h) = (400, 290);
        let (cw, ch) = (cw(w), h - 34);
        let rows = skins
            .iter()
            .map(|(n, _)| {
                if n.is_empty() {
                    "<Base Skin>".to_string()
                } else {
                    n.clone()
                }
            })
            .collect();
        let sel = skins.iter().position(|(_, p)| p.as_deref() == current);
        let mut form = Form::new(vec![
            Widget::List {
                rect: Rect::new(6, 6, 150, ch - 40),
                id: LIST,
                rows: vec![],
                selected: None,
                first: 0,
            },
            Widget::TextBox {
                rect: Rect::new(162, 6, cw - 168, ch - 40),
                lines: vec![],
                first: 0,
            },
            Widget::Button {
                rect: Rect::new(cw - 64, ch - 26, 58, 18),
                id: CANCEL,
                label: "Close".into(),
                default: true,
            },
        ]);
        form.set_rows(LIST, rows, sel);
        form.focus_id(LIST);
        let mut d = Self { form, skins };
        d.show_readme(sel);
        d
    }

    fn show_readme(&mut self, i: Option<usize>) {
        let text = i
            .and_then(|i| self.skins.get(i))
            .and_then(|(_, p)| p.as_ref())
            .and_then(|p| nulamp_skin::SkinFiles::open(std::path::Path::new(p)).ok())
            .and_then(|f| f.readme())
            .unwrap_or_default();
        let lines = wrap(&text, 40);
        for w in &mut self.form.widgets {
            if let Widget::TextBox {
                lines: l, first, ..
            } = w
            {
                *l = lines.clone();
                *first = 0;
            }
        }
    }
}

/// Wraps text to at most `width` characters per line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

impl Dialog for SkinBrowser {
    fn title(&self) -> String {
        "Skin Browser".into()
    }

    fn size(&self) -> (i32, i32) {
        (400, 290)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "skins"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Selected(LIST, i) | FormEvent::Activated(LIST, i) => {
                self.show_readme(Some(i));
                let path = self.skins.get(i).and_then(|(_, p)| p.clone());
                Outcome::Do(vec![DialogResult::LoadSkin(path)], false)
            }
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel | FormEvent::Submit => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

// ----- File info (10 §8) -----

/// Editable fields of the basic tab: (key, label).
const INFO_FIELDS: [(&str, &str); 13] = [
    ("title", "Title"),
    ("artist", "Artist"),
    ("album", "Album"),
    ("albumartist", "Album artist"),
    ("year", "Year"),
    ("genre", "Genre"),
    ("track", "Track"),
    ("disc", "Disc"),
    ("composer", "Composer"),
    ("publisher", "Publisher"),
    ("bpm", "BPM"),
    ("rating", "Rating (0-5)"),
    ("comment", "Comment"),
];

const FIELD_BASE: u32 = 100;

pub struct FileInfoDialog {
    form: Form,
    location: String,
    original: Vec<(String, String)>,
    error: Option<String>,
}

impl FileInfoDialog {
    pub fn new(location: &str) -> Self {
        let (w, h) = (475, 348);
        let (cw, ch) = (cw(w), h - 34);
        let meta = nulamp_meta::read(location);
        let supported: Vec<&str> = match &meta {
            Ok(m) => m.supported_fields().to_vec(),
            Err(_) => Vec::new(),
        };
        let mut widgets = vec![Widget::Label {
            rect: Rect::new(6, 4, cw - 12, 14),
            text: location.to_string(),
        }];
        let mut original = Vec::new();
        for (i, (key, label)) in INFO_FIELDS.iter().enumerate() {
            let y = 22 + i as i32 * 20;
            let value = meta
                .as_ref()
                .ok()
                .and_then(|m| m.get(key))
                .unwrap_or_default();
            original.push((key.to_string(), value.clone()));
            let label = if supported.contains(key) {
                label.to_string()
            } else {
                format!("{label} (n/a)")
            };
            widgets.push(Widget::Label {
                rect: Rect::new(6, y + 2, 84, 14),
                text: label,
            });
            widgets.push(Widget::Edit {
                rect: Rect::new(92, y, 200, 18),
                id: FIELD_BASE + i as u32,
                text: value,
                cursor: 0,
            });
        }
        let info: Vec<String> = match &meta {
            Ok(m) => {
                let mut v: Vec<String> = m
                    .get("formatinformation")
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_string)
                    .collect();
                for k in ["replaygain_track_gain", "replaygain_album_gain"] {
                    v.push(format!(
                        "{k}: {}",
                        m.get(k).unwrap_or_else(|| "not present".into())
                    ));
                }
                v
            }
            Err(e) => vec![format!("Cannot read tags: {e}")],
        };
        widgets.push(Widget::TextBox {
            rect: Rect::new(300, 22, cw - 306, ch - 56),
            lines: info,
            first: 0,
        });
        widgets.push(Widget::Button {
            rect: Rect::new(cw - 128, ch - 26, 58, 18),
            id: OK,
            label: "OK".into(),
            default: true,
        });
        widgets.push(Widget::Button {
            rect: Rect::new(cw - 64, ch - 26, 58, 18),
            id: CANCEL,
            label: "Cancel".into(),
            default: false,
        });
        let mut form = Form::new(widgets);
        form.focus_id(FIELD_BASE);
        Self {
            form,
            location: location.to_string(),
            original,
            error: None,
        }
    }

    fn changes(&self) -> Vec<(String, Option<String>)> {
        self.original
            .iter()
            .enumerate()
            .filter_map(|(i, (key, old))| {
                let new = self.form.text(FIELD_BASE + i as u32);
                (new != *old).then(|| (key.clone(), (!new.is_empty()).then_some(new)))
            })
            .collect()
    }
}

impl Dialog for FileInfoDialog {
    fn title(&self) -> String {
        "File Info".into()
    }

    fn size(&self) -> (i32, i32) {
        (475, 348)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "fileinfo"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(OK) | FormEvent::Submit => {
                let changes = self.changes();
                if changes.is_empty() {
                    return Outcome::Close;
                }
                match nulamp_meta::write(&self.location, &changes) {
                    Ok(()) => {
                        Outcome::Do(vec![DialogResult::TagsWritten(self.location.clone())], true)
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        for w in &mut self.form.widgets {
                            if let Widget::Label { rect, text } = w
                                && rect.y == 4
                            {
                                *text = format!("Error: {e}");
                            }
                        }
                        Outcome::Keep
                    }
                }
            }
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

// ----- EQ preset save -----

pub struct PresetSaveDialog {
    form: Form,
    names: Vec<String>,
    auto: bool,
}

impl PresetSaveDialog {
    /// `initial` prefills the name (the current file name for auto-load presets, 03 §5.3).
    pub fn new(names: Vec<String>, initial: &str, auto: bool) -> Self {
        let (w, h) = (300, 261);
        let (cw, ch) = (cw(w), h - 34);
        let mut form = Form::new(vec![
            Widget::Edit {
                rect: Rect::new(6, 6, cw - 12, 18),
                id: EDIT,
                text: String::new(),
                cursor: 0,
            },
            Widget::List {
                rect: Rect::new(6, 30, cw - 12, ch - 62),
                id: LIST,
                rows: vec![],
                selected: None,
                first: 0,
            },
            Widget::Button {
                rect: Rect::new(cw - 128, ch - 26, 58, 18),
                id: OK,
                label: "Save".into(),
                default: true,
            },
            Widget::Button {
                rect: Rect::new(cw - 64, ch - 26, 58, 18),
                id: CANCEL,
                label: "Cancel".into(),
                default: false,
            },
        ]);
        form.set_rows(LIST, names.clone(), None);
        form.set_text(EDIT, initial);
        form.focus_id(EDIT);
        Self { form, names, auto }
    }
}

impl Dialog for PresetSaveDialog {
    fn title(&self) -> String {
        if self.auto {
            "Save auto-load preset".into()
        } else {
            "Save EQ preset".into()
        }
    }

    fn size(&self) -> (i32, i32) {
        (300, 261)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "eqsave"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Selected(LIST, i) => {
                if let Some(n) = self.names.get(i) {
                    let n = n.clone();
                    self.form.set_text(EDIT, &n);
                }
                Outcome::Keep
            }
            FormEvent::Clicked(OK) | FormEvent::Submit | FormEvent::Activated(LIST, _) => {
                let name = self.form.text(EDIT).trim().to_string();
                if name.is_empty() {
                    return Outcome::Keep;
                }
                Outcome::Do(
                    vec![DialogResult::EqSave {
                        name,
                        auto: self.auto,
                    }],
                    true,
                )
            }
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_normalisation() {
        assert_eq!(
            normalize_url("radio.example:8000/live"),
            "http://radio.example:8000/live"
        );
        assert_eq!(normalize_url("https://x"), "https://x");
        assert_eq!(normalize_url("/music/a.mp3"), "/music/a.mp3");
    }

    #[test]
    fn time_parsing() {
        assert_eq!(parse_time("75"), Some(75_000));
        assert_eq!(parse_time("1:15"), Some(75_000));
        assert_eq!(parse_time("x"), None);
    }

    #[test]
    fn jump_tokens() {
        assert_eq!(tokens("daft  punk-one"), ["daft", "punk", "one"]);
        assert_eq!(tokens("\"one more\" daft"), ["one more", "daft"]);
        assert!(tokens(" -- ").is_empty());
        assert!(matches(&tokens("björk"), "/m/Björk - Jóga.mp3", ""));
        assert!(matches(&[], "x", "y"));
        assert!(!matches(&tokens("abc"), "/m/x.mp3", "Y"));
    }

    #[test]
    fn jump_dialog_filters_and_sorts() {
        let e = |i: u64, t: &str| (EntryId(i), format!("/m/{t}.mp3"), t.to_string());
        let mut d = JumpFileDialog::new(
            vec![e(1, "Zebra"), e(2, "apple"), e(3, "Mango")],
            Some(EntryId(3)),
        );
        assert_eq!(d.shown, [EntryId(2), EntryId(3), EntryId(1)]);
        assert_eq!(d.selected_id(), Some(EntryId(3)));
        d.form.text_input("an");
        let ev = FormEvent::Changed(EDIT);
        d.handle(ev);
        assert_eq!(d.shown, [EntryId(3)]);
        match d.handle(FormEvent::Submit) {
            Outcome::Close => {}
            _ => panic!("playing entry: close without action"),
        }
    }
}
