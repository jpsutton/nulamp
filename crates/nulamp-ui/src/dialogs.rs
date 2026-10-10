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
    /// The user confirmed moving these files to the trash.
    DeleteFiles(Vec<String>),
    /// Change an entry's location (edit playlist entry, 04 §9).
    SetLocation(EntryId, String),
    /// Replace the bookmark list.
    SetBookmarks(Vec<(String, String)>),
    /// Play a bookmark (clear, add, play).
    PlayBookmark(String),
    /// New preferences from the preferences dialog.
    Prefs(Box<(crate::shell::UiPrefs, crate::prefs_dialog::PlaylistOptions)>),
    /// Save the current EQ under a name, in the preset or auto-load library.
    EqSave {
        name: String,
        auto: bool,
    },
    /// Re-index the media library's watch folders now.
    LibraryRescan,
    /// The user confirmed removing tracks from the media library.
    LibraryRemove(Vec<nulamp_library::TrackId>),
    /// Create a library playlist with these tracks.
    LibraryNewPlaylist {
        provider: String,
        name: String,
        tracks: Vec<nulamp_library::TrackId>,
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
                secret: false,
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

// ----- Network login (07 §8.5) -----

const USER: u32 = 6;
const PASSWORD: u32 = 7;

/// Asks for a user name and password for a stream server. Closing it without OK answers
/// "no credentials".
pub struct LoginDialog {
    form: Form,
    reply: Option<std::sync::mpsc::Sender<Option<(String, String)>>>,
}

impl LoginDialog {
    pub fn new(request: crate::shell::CredentialRequest) -> Self {
        let w = cw(300);
        let host = request
            .url
            .split_once("://")
            .map(|(_, r)| r.split('/').next().unwrap_or(r))
            .unwrap_or(&request.url)
            .to_string();
        let mut form = Form::new(vec![
            Widget::Label {
                rect: Rect::new(6, 6, w - 12, 14),
                text: format!("{host} asks for a login"),
            },
            Widget::Label {
                rect: Rect::new(6, 20, w - 12, 14),
                text: format!("Realm: {}", request.realm),
            },
            Widget::Label {
                rect: Rect::new(6, 42, 70, 14),
                text: "User name:".into(),
            },
            Widget::Edit {
                rect: Rect::new(80, 40, w - 86, 18),
                id: USER,
                text: String::new(),
                cursor: 0,
                secret: false,
            },
            Widget::Label {
                rect: Rect::new(6, 66, 70, 14),
                text: "Password:".into(),
            },
            Widget::Edit {
                rect: Rect::new(80, 64, w - 86, 18),
                id: PASSWORD,
                text: String::new(),
                cursor: 0,
                secret: true,
            },
            Widget::Button {
                rect: Rect::new(w - 128, 92, 58, 18),
                id: OK,
                label: "OK".into(),
                default: true,
            },
            Widget::Button {
                rect: Rect::new(w - 64, 92, 58, 18),
                id: CANCEL,
                label: "Cancel".into(),
                default: false,
            },
        ]);
        form.focus_id(USER);
        Self {
            form,
            reply: Some(request.reply),
        }
    }

    fn answer(&mut self, value: Option<(String, String)>) {
        if let Some(r) = self.reply.take() {
            let _ = r.send(value);
        }
    }
}

impl Drop for LoginDialog {
    fn drop(&mut self) {
        self.answer(None);
    }
}

impl Dialog for LoginDialog {
    fn title(&self) -> String {
        "Login".into()
    }

    fn size(&self) -> (i32, i32) {
        (300, 145)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "login"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(OK) | FormEvent::Submit => {
                let user = self.form.text(USER);
                let password = self.form.text(PASSWORD);
                self.answer(Some((user, password)));
                Outcome::Close
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
                secret: false,
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
                secret: false,
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
                secret: false,
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

// ----- Confirmation -----

pub struct ConfirmDialog {
    form: Form,
    /// What OK does.
    on_ok: DialogResult,
}

impl ConfirmDialog {
    /// Asks before moving files to the trash (04 §9).
    pub fn delete_files(files: Vec<String>) -> Self {
        let question = format!("Move {} file(s) to the trash?", files.len());
        Self::new(
            &question,
            &files,
            "Delete",
            DialogResult::DeleteFiles(files.clone()),
        )
    }

    /// Asks before removing tracks from the media library (the files stay).
    pub fn remove_from_library(titles: &[String], ids: Vec<nulamp_library::TrackId>) -> Self {
        let question = format!(
            "Remove {} track(s) from the library?\nThe files stay on disk.",
            ids.len()
        );
        Self::new(
            &question,
            titles,
            "Remove",
            DialogResult::LibraryRemove(ids),
        )
    }

    fn new(question: &str, items: &[String], ok: &str, on_ok: DialogResult) -> Self {
        let w = cw(375);
        let mut lines: Vec<String> = question.lines().map(str::to_string).collect();
        lines.push(String::new());
        lines.extend(items.iter().take(6).cloned());
        if items.len() > 6 {
            lines.push("…".into());
        }
        let form = Form::new(vec![
            Widget::TextBox {
                rect: Rect::new(6, 6, w - 12, 116),
                lines,
                first: 0,
            },
            Widget::Button {
                rect: Rect::new(w - 128, 130, 58, 18),
                id: OK,
                label: ok.into(),
                default: false,
            },
            Widget::Button {
                rect: Rect::new(w - 64, 130, 58, 18),
                id: CANCEL,
                label: "Cancel".into(),
                default: true,
            },
        ]);
        let mut d = Self { form, on_ok };
        d.form.focus_id(CANCEL);
        d
    }
}

impl Dialog for ConfirmDialog {
    fn title(&self) -> String {
        "Confirm".into()
    }

    fn size(&self) -> (i32, i32) {
        (375, 203)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "confirm"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(OK) => Outcome::Do(vec![self.on_ok.clone()], true),
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel | FormEvent::Submit => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

// ----- Edit playlist entry -----

pub struct EditEntryDialog {
    form: Form,
    /// Entries left to edit; the first is shown.
    queue: Vec<(EntryId, String)>,
}

impl EditEntryDialog {
    pub fn new(queue: Vec<(EntryId, String)>) -> Self {
        let w = cw(475);
        let form = Form::new(vec![
            Widget::Label {
                rect: Rect::new(6, 6, w - 12, 14),
                text: "Location (file or URL):".into(),
            },
            Widget::Edit {
                rect: Rect::new(6, 24, w - 12, 18),
                id: EDIT,
                text: String::new(),
                cursor: 0,
                secret: false,
            },
            Widget::Button {
                rect: Rect::new(w - 128, 52, 58, 18),
                id: OK,
                label: "OK".into(),
                default: true,
            },
            Widget::Button {
                rect: Rect::new(w - 64, 52, 58, 18),
                id: CANCEL,
                label: "Cancel".into(),
                default: false,
            },
        ]);
        let mut d = Self { form, queue };
        d.show();
        d
    }

    fn show(&mut self) {
        if let Some((_, loc)) = self.queue.first() {
            let loc = loc.clone();
            self.form.set_text(EDIT, &loc);
            self.form.focus_id(EDIT);
        }
    }
}

impl Dialog for EditEntryDialog {
    fn title(&self) -> String {
        "Edit playlist entry".into()
    }

    fn size(&self) -> (i32, i32) {
        (475, 116)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "editentry"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(OK) | FormEvent::Submit => {
                if self.queue.is_empty() {
                    return Outcome::Close;
                }
                let (id, _) = self.queue.remove(0);
                let loc = self.form.text(EDIT).trim().to_string();
                let done = self.queue.is_empty();
                self.show();
                Outcome::Do(vec![DialogResult::SetLocation(id, loc)], done)
            }
            FormEvent::Clicked(CANCEL) | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

// ----- Bookmarks (10 §9) -----

const REMOVE: u32 = 6;
const UP: u32 = 7;
const DOWN: u32 = 8;

pub struct BookmarksDialog {
    form: Form,
    /// (location, title).
    items: Vec<(String, String)>,
}

impl BookmarksDialog {
    pub fn new(items: Vec<(String, String)>) -> Self {
        let (w, h) = (400, 290);
        let (cw, ch) = (cw(w), h - 34);
        let mut d = Self {
            form: Form::new(vec![
                Widget::List {
                    rect: Rect::new(6, 6, cw - 12, ch - 40),
                    id: LIST,
                    rows: vec![],
                    selected: None,
                    first: 0,
                },
                Widget::Button {
                    rect: Rect::new(6, ch - 26, 50, 18),
                    id: UP,
                    label: "Up".into(),
                    default: false,
                },
                Widget::Button {
                    rect: Rect::new(60, ch - 26, 50, 18),
                    id: DOWN,
                    label: "Down".into(),
                    default: false,
                },
                Widget::Button {
                    rect: Rect::new(cw - 196, ch - 26, 60, 18),
                    id: OK,
                    label: "Play".into(),
                    default: true,
                },
                Widget::Button {
                    rect: Rect::new(cw - 130, ch - 26, 60, 18),
                    id: REMOVE,
                    label: "Remove".into(),
                    default: false,
                },
                Widget::Button {
                    rect: Rect::new(cw - 64, ch - 26, 58, 18),
                    id: CANCEL,
                    label: "Close".into(),
                    default: false,
                },
            ]),
            items,
        };
        d.refresh(Some(0));
        d.form.focus_id(LIST);
        d
    }

    fn refresh(&mut self, sel: Option<usize>) {
        let rows = self
            .items
            .iter()
            .map(|(loc, title)| format!("{title}  —  {loc}"))
            .collect();
        self.form.set_rows(LIST, rows, sel);
    }
}

impl Dialog for BookmarksDialog {
    fn title(&self) -> String {
        "Edit Bookmarks".into()
    }

    fn size(&self) -> (i32, i32) {
        (400, 290)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "bookmarks"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        let sel = self.form.selected(LIST);
        match ev {
            FormEvent::Clicked(OK) | FormEvent::Activated(LIST, _) | FormEvent::Submit => match sel
                .and_then(|i| self.items.get(i))
            {
                Some((loc, _)) => Outcome::Do(vec![DialogResult::PlayBookmark(loc.clone())], false),
                None => Outcome::Keep,
            },
            FormEvent::Clicked(REMOVE) => match sel {
                Some(i) if i < self.items.len() => {
                    self.items.remove(i);
                    let next = if self.items.is_empty() {
                        None
                    } else {
                        Some(i.min(self.items.len() - 1))
                    };
                    self.refresh(next);
                    Outcome::Do(vec![DialogResult::SetBookmarks(self.items.clone())], false)
                }
                _ => Outcome::Keep,
            },
            FormEvent::Clicked(UP) | FormEvent::Clicked(DOWN) => {
                let Some(i) = sel else { return Outcome::Keep };
                let j = if ev == FormEvent::Clicked(UP) {
                    i.checked_sub(1)
                } else {
                    (i + 1 < self.items.len()).then_some(i + 1)
                };
                match j {
                    Some(j) => {
                        self.items.swap(i, j);
                        self.refresh(Some(j));
                        Outcome::Do(vec![DialogResult::SetBookmarks(self.items.clone())], false)
                    }
                    None => Outcome::Keep,
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
                secret: false,
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

// ----- New library playlist -----

/// Asks for the name of a new media-library playlist for some tracks.
pub struct LibraryPlaylistDialog {
    form: Form,
    provider: String,
    tracks: Vec<nulamp_library::TrackId>,
}

impl LibraryPlaylistDialog {
    pub fn new(provider: String, tracks: Vec<nulamp_library::TrackId>) -> Self {
        let w = cw(375);
        let mut form = Form::new(vec![
            Widget::Label {
                rect: Rect::new(6, 6, w - 12, 14),
                text: format!("Name of the new playlist ({} tracks):", tracks.len()),
            },
            Widget::Edit {
                rect: Rect::new(6, 24, w - 12, 18),
                id: EDIT,
                text: String::new(),
                cursor: 0,
                secret: false,
            },
            Widget::Button {
                rect: Rect::new(w - 128, 52, 58, 18),
                id: OK,
                label: "Create".into(),
                default: true,
            },
            Widget::Button {
                rect: Rect::new(w - 64, 52, 58, 18),
                id: CANCEL,
                label: "Cancel".into(),
                default: false,
            },
        ]);
        form.focus_id(EDIT);
        Self {
            form,
            provider,
            tracks,
        }
    }
}

impl Dialog for LibraryPlaylistDialog {
    fn title(&self) -> String {
        "New playlist".into()
    }

    fn size(&self) -> (i32, i32) {
        (375, 116)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "libplaylist"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(OK) | FormEvent::Submit => {
                let name = self.form.text(EDIT).trim().to_string();
                if name.is_empty() {
                    return Outcome::Keep;
                }
                Outcome::Do(
                    vec![DialogResult::LibraryNewPlaylist {
                        provider: self.provider.clone(),
                        name,
                        tracks: self.tracks.clone(),
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
    fn library_dialogs() {
        let ids = vec![nulamp_library::TrackId::new("local", "7")];
        let mut d = ConfirmDialog::remove_from_library(&["Song".into()], ids.clone());
        match d.handle(FormEvent::Clicked(OK)) {
            Outcome::Do(r, true) => {
                assert!(matches!(&r[..], [DialogResult::LibraryRemove(x)] if *x == ids))
            }
            _ => panic!("expected the removal"),
        }
        assert!(matches!(d.handle(FormEvent::Submit), Outcome::Close));
        let mut p = LibraryPlaylistDialog::new("local".into(), ids.clone());
        assert!(matches!(p.handle(FormEvent::Submit), Outcome::Keep));
        p.form().text_input(" Road trip ");
        match p.handle(FormEvent::Submit) {
            Outcome::Do(r, true) => match &r[..] {
                [DialogResult::LibraryNewPlaylist { name, tracks, .. }] => {
                    assert_eq!(name, "Road trip");
                    assert_eq!(tracks, &ids);
                }
                _ => panic!(),
            },
            _ => panic!("expected a new playlist"),
        }
    }

    fn login() -> (
        LoginDialog,
        std::sync::mpsc::Receiver<Option<(String, String)>>,
    ) {
        let (reply, answer) = std::sync::mpsc::channel();
        let d = LoginDialog::new(crate::shell::CredentialRequest {
            url: "http://radio.example:8000/live".into(),
            realm: "Members".into(),
            reply,
        });
        (d, answer)
    }

    #[test]
    fn login_answers_once() {
        let (mut d, answer) = login();
        d.form().text_input("bob");
        d.form().key(nulamp_skinui::Key::Tab);
        d.form().text_input("pw");
        assert!(matches!(d.handle(FormEvent::Submit), Outcome::Close));
        drop(d);
        assert_eq!(answer.recv().unwrap(), Some(("bob".into(), "pw".into())));
        assert!(answer.recv().is_err());
    }

    #[test]
    fn closed_login_gives_up() {
        let (d, answer) = login();
        drop(d);
        assert_eq!(answer.recv().unwrap(), None);
    }

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
