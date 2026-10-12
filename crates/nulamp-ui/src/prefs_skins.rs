//! The "Skins" page of the preferences dialog: the skins folder, the skins in it, and a
//! preview of the selected skin's main window. The skin is loaded by OK or Apply.

use std::path::{Path, PathBuf};

use nulamp_render::Frame;
use nulamp_render::main_window::{self as mw, MainView, TimeDisplay, TitleArea};
use nulamp_skin::{Skin, SkinFiles};
use nulamp_skinui::{Form, FormEvent, Rect, Widget};

pub const SKINS: u32 = 600;

pub struct SkinsPage {
    /// The folder `skins` was read from.
    dir: Option<PathBuf>,
    /// Name and path of each skin; the base skin has no path.
    skins: Vec<(String, Option<String>)>,
    /// The skin in use.
    current: Option<String>,
    /// The skin selected in the list, when it is not the one in use.
    chosen: Option<Option<String>>,
    /// The last preview drawn, and the skin it shows.
    preview: Option<(Option<String>, Option<Frame>)>,
}

impl SkinsPage {
    pub fn new(current: Option<String>) -> Self {
        Self {
            dir: None,
            skins: Vec::new(),
            current,
            chosen: None,
            preview: None,
        }
    }

    /// The skin selected in the list.
    fn selected(&self) -> Option<String> {
        self.chosen.clone().unwrap_or_else(|| self.current.clone())
    }

    /// The page's widgets below the folder box: the skin list from `y` and the preview under
    /// it, ending at `bottom`.
    pub fn widgets(&mut self, dir: &Path, x: i32, y: i32, w: i32, bottom: i32) -> Vec<Widget> {
        if self.dir.as_deref() != Some(dir) {
            self.skins = nulamp_skin::list_skins(dir)
                .into_iter()
                .map(|e| (e.name, e.path.map(|p| p.to_string_lossy().into_owned())))
                .collect();
            self.dir = Some(dir.to_path_buf());
        }
        let selected = self.selected();
        let preview_h = mw::MAIN_SIZE.1 as i32 + 2;
        let list_h = bottom - y - 6 - preview_h;
        vec![
            Widget::List {
                rect: Rect::new(x, y, w, list_h),
                id: SKINS,
                rows: self
                    .skins
                    .iter()
                    .map(|(n, _)| {
                        if n.is_empty() {
                            "<Base Skin>".to_string()
                        } else {
                            n.clone()
                        }
                    })
                    .collect(),
                selected: self.skins.iter().position(|(_, p)| *p == selected),
                first: 0,
            },
            Widget::Image {
                rect: Rect::new(x, bottom - preview_h, w, preview_h),
                image: self.preview_of(selected),
            },
        ]
    }

    /// Handles a choice in the skin list. Returns true when the event was for this page.
    pub fn event(&mut self, form: &mut Form, ev: &FormEvent) -> bool {
        let (FormEvent::Selected(SKINS, i) | FormEvent::Activated(SKINS, i)) = *ev else {
            return false;
        };
        let Some((_, path)) = self.skins.get(i) else {
            return true;
        };
        let path = path.clone();
        self.chosen = (path != self.current).then(|| path.clone());
        let image = self.preview_of(path);
        for w in &mut form.widgets {
            if let Widget::Image { image: img, .. } = w {
                *img = image.clone();
            }
        }
        true
    }

    /// The skin to load on OK or Apply; `None` when the choice is the skin in use.
    pub fn take_choice(&mut self) -> Option<Option<String>> {
        let choice = self.chosen.take()?;
        self.current = choice.clone();
        Some(choice)
    }

    fn preview_of(&mut self, path: Option<String>) -> Option<Frame> {
        match &self.preview {
            Some((p, image)) if *p == path => image.clone(),
            _ => {
                let image = preview(path.as_deref());
                self.preview = Some((path, image.clone()));
                image
            }
        }
    }
}

/// The main window of a skin (`None`: the base skin), as if playing a track.
fn preview(path: Option<&str>) -> Option<Frame> {
    let skin = match path {
        None => nulamp_skin::builtin::skin(),
        Some(p) => {
            let files = SkinFiles::open(Path::new(p)).ok()?;
            Skin::load(&files, &nulamp_skin::builtin::files())
        }
    };
    let name = path
        .and_then(|p| Path::new(p).file_stem())
        .map_or("Base Skin".into(), |n| n.to_string_lossy().into_owned());
    let view = MainView {
        state: mw::PlayState::Playing,
        time: TimeDisplay::Time {
            minutes: 1,
            seconds: 23,
            remaining: false,
        },
        kbps: Some(128),
        khz: Some(44),
        channels: Some(2),
        seek: Some(90),
        title: TitleArea::Bitmap {
            text: format!("1. {name} (3:45)"),
            offset: 0,
        },
        ..MainView::default()
    };
    Some(mw::draw_main(&skin, &view))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_skin_preview_is_the_main_window() {
        let f = preview(None).expect("base skin preview");
        assert_eq!((f.width(), f.height()), mw::MAIN_SIZE);
    }

    #[test]
    fn choosing_the_skin_in_use_changes_nothing() {
        let dir = Path::new("/nonexistent/nulamp-skins");
        let mut page = SkinsPage::new(None);
        let mut form = Form::new(page.widgets(dir, 0, 0, 300, 250));
        // Only the base skin is listed, and it is in use.
        assert!(page.event(&mut form, &FormEvent::Selected(SKINS, 0)));
        assert_eq!(page.take_choice(), None);
        page.chosen = Some(Some("/skins/a.wsz".into()));
        assert_eq!(page.take_choice(), Some(Some("/skins/a.wsz".into())));
        assert_eq!(page.take_choice(), None);
        assert_eq!(page.current.as_deref(), Some("/skins/a.wsz"));
    }
}
