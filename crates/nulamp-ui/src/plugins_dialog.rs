//! The plugin list (08 §2): every loaded module, its configure and about actions, and the
//! choice of the active effect (08 §5).

use nulamp_skinui::{Form, FormEvent, Rect, Widget};

use crate::dialogs::{Dialog, DialogResult, Outcome};

/// One loaded module as the dialog shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    /// "Input", "DSP", "Visualization" or "General".
    pub kind: String,
    pub name: String,
    /// The plugin file name (empty for built-in modules).
    pub file: String,
    pub can_configure: bool,
    pub can_about: bool,
    /// An effect that can be made the active one.
    pub selectable: bool,
    /// The active effect.
    pub active: bool,
}

/// What the user asked of a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginAction {
    Configure,
    About,
    /// Make this effect the active one, or turn it off when it is.
    ToggleActive,
}

const LIST: u32 = 1;
const CONFIGURE: u32 = 2;
const ABOUT: u32 = 3;
const USE: u32 = 4;
const CLOSE: u32 = 5;

pub struct PluginsDialog {
    form: Form,
    rows: Vec<PluginRow>,
    errors: usize,
}

fn row_text(r: &PluginRow) -> String {
    let mark = if r.active { "* " } else { "" };
    if r.file.is_empty() {
        format!("{mark}[{}] {}", r.kind, r.name)
    } else {
        format!("{mark}[{}] {} ({})", r.kind, r.name, r.file)
    }
}

impl PluginsDialog {
    /// `errors`: plugin files or modules that failed to load (shown as a count).
    pub fn new(rows: Vec<PluginRow>, errors: usize) -> Self {
        let (w, h) = (Self::W - 19, Self::H - 34);
        let status = match (rows.len(), errors) {
            (0, 0) => "No plugins are loaded.".to_string(),
            (n, 0) => format!("{n} modules loaded."),
            (n, e) => format!("{n} modules loaded, {e} failed (see the log)."),
        };
        let form = Form::new(vec![
            Widget::List {
                rect: Rect::new(6, 6, w - 12, h - 56),
                id: LIST,
                rows: rows.iter().map(row_text).collect(),
                selected: (!rows.is_empty()).then_some(0),
                first: 0,
            },
            Widget::Label {
                rect: Rect::new(6, h - 44, w - 12, 14),
                text: status,
            },
            Widget::Button {
                rect: Rect::new(6, h - 24, 70, 18),
                id: CONFIGURE,
                label: "Configure".into(),
                default: false,
            },
            Widget::Button {
                rect: Rect::new(82, h - 24, 58, 18),
                id: ABOUT,
                label: "About".into(),
                default: false,
            },
            Widget::Button {
                rect: Rect::new(146, h - 24, 70, 18),
                id: USE,
                label: "Use/Stop".into(),
                default: false,
            },
            Widget::Button {
                rect: Rect::new(w - 64, h - 24, 58, 18),
                id: CLOSE,
                label: "Close".into(),
                default: true,
            },
        ]);
        Self { form, rows, errors }
    }

    const W: i32 = 375;
    const H: i32 = 261;

    fn selected(&self) -> Option<usize> {
        self.form.selected(LIST).filter(|&i| i < self.rows.len())
    }

    fn act(&mut self, action: PluginAction) -> Outcome {
        let Some(i) = self.selected() else {
            return Outcome::Keep;
        };
        let r = &self.rows[i];
        let allowed = match action {
            PluginAction::Configure => r.can_configure,
            PluginAction::About => r.can_about,
            PluginAction::ToggleActive => r.selectable,
        };
        if !allowed {
            return Outcome::Keep;
        }
        if action == PluginAction::ToggleActive {
            // Only one effect is active (08 §5).
            let on = !self.rows[i].active;
            for (j, r) in self.rows.iter_mut().enumerate() {
                r.active = on && j == i;
            }
            let rows = self.rows.clone();
            let keep = self.form.selected(LIST);
            *self = Self::new(rows, self.errors);
            if let Some(Widget::List { selected, .. }) = self.form.find_mut(LIST) {
                *selected = keep;
            }
        }
        Outcome::Do(vec![DialogResult::Plugin(i, action)], false)
    }
}

impl Dialog for PluginsDialog {
    fn title(&self) -> String {
        "Plugins".into()
    }

    fn size(&self) -> (i32, i32) {
        (Self::W, Self::H)
    }

    fn form(&mut self) -> &mut Form {
        &mut self.form
    }

    fn kind(&self) -> &'static str {
        "plugins"
    }

    fn handle(&mut self, ev: FormEvent) -> Outcome {
        match ev {
            FormEvent::Clicked(CONFIGURE) | FormEvent::Activated(LIST, _) => {
                self.act(PluginAction::Configure)
            }
            FormEvent::Clicked(ABOUT) => self.act(PluginAction::About),
            FormEvent::Clicked(USE) => self.act(PluginAction::ToggleActive),
            FormEvent::Clicked(CLOSE) | FormEvent::Submit | FormEvent::Cancel => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<PluginRow> {
        let row = |kind: &str, name: &str, selectable| PluginRow {
            kind: kind.into(),
            name: name.into(),
            file: "libexample.so".into(),
            can_configure: true,
            can_about: false,
            selectable,
            active: false,
        };
        vec![row("DSP", "Gain", true), row("General", "Logger", false)]
    }

    #[test]
    fn one_active_effect() {
        let mut d = PluginsDialog::new(rows(), 0);
        let out = d.handle(FormEvent::Clicked(USE));
        assert!(matches!(
            out,
            Outcome::Do(ref r, false)
                if matches!(r[..], [DialogResult::Plugin(0, PluginAction::ToggleActive)])
        ));
        assert!(d.rows[0].active);
        assert!(d.form.selected(LIST) == Some(0));
        d.handle(FormEvent::Clicked(USE));
        assert!(!d.rows[0].active);
    }

    #[test]
    fn unavailable_actions_do_nothing() {
        let mut d = PluginsDialog::new(rows(), 1);
        assert!(matches!(d.handle(FormEvent::Clicked(ABOUT)), Outcome::Keep));
        d.form.key(nulamp_skinui::Key::Down);
        d.form.focus_id(LIST);
        if let Some(Widget::List { selected, .. }) = d.form.find_mut(LIST) {
            *selected = Some(1);
        }
        assert!(matches!(d.handle(FormEvent::Clicked(USE)), Outcome::Keep));
        assert!(matches!(
            d.handle(FormEvent::Clicked(CONFIGURE)),
            Outcome::Do(_, false)
        ));
    }
}
