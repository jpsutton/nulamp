//! Popup menus: model, layout and drawing. Menus are drawn by nulamp (with 1-bit text and the
//! skin's `genex.bmp` colours) and shown in borderless popup windows by the shell.

use nulamp_render::Frame;
use nulamp_skin::Skin;
use nulamp_skinui::{PixelText, UiText};
use nulamp_text::FontSystem;

use crate::input::Action;
use crate::keys::KeyAction;
use crate::playlist::PlMenuChoice;

/// What a menu item does.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuCmd {
    Act(Action),
    Key(KeyAction),
    Pl(PlMenuChoice),
    EqEnabled,
    EqAutoLoad,
    EqLoadPreset(String),
    EqLoadAutoPreset,
    EqLoadDefault,
    EqSavePreset,
    EqSaveAutoPreset,
    EqSaveDefault,
    EqImportEqf,
    EqExportEqf,
    EqDeletePreset(String),
    SetVisMode(u8),
    SetAnalyzerStyle(u8),
    SetBands(bool),
    TogglePeaks,
    SetScopeStyle(u8),
    SetRefresh(u8),
    SetFalloff(u8),
    SetPeakFalloff(u8),
    SetTimeRemaining(bool),
    Rate(u8),
    LoadSkin(Option<String>),
    PlSortTitle,
    PlSortFileName,
    PlSortPath,
    PlReverse,
    PlRandomize,
    PlRemoveMissing,
    PlRemoveDuplicates,
    PlPhysicallyRemove,
    PlFileInfo,
    PlEditEntry,
    PlPlaySelected,
    PlEnqueueSelected,
    PlShowInFolder,
    PlGenerateHtml,
    PlRebuildTitles,
    PlRateSelected(u8),
    Bookmark(String),
    /// Play an audio CD (whole-disc location).
    PlayCd(String),
    Plugins,
    /// Media-library menus.
    Lib(crate::library::LibCmd),
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Cmd {
        label: String,
        shortcut: String,
        cmd: MenuCmd,
        check: Option<bool>,
        enabled: bool,
    },
    Sub {
        label: String,
        items: Vec<Item>,
    },
    Sep,
}

impl Item {
    pub fn cmd(label: &str, cmd: MenuCmd) -> Self {
        Item::Cmd {
            label: label.into(),
            shortcut: String::new(),
            cmd,
            check: None,
            enabled: true,
        }
    }

    pub fn key(label: &str, shortcut: &str, cmd: MenuCmd) -> Self {
        Item::Cmd {
            label: label.into(),
            shortcut: shortcut.into(),
            cmd,
            check: None,
            enabled: true,
        }
    }

    pub fn check(label: &str, shortcut: &str, on: bool, cmd: MenuCmd) -> Self {
        Item::Cmd {
            label: label.into(),
            shortcut: shortcut.into(),
            cmd,
            check: Some(on),
            enabled: true,
        }
    }

    pub fn disabled(label: &str) -> Self {
        Item::Cmd {
            label: label.into(),
            shortcut: String::new(),
            cmd: MenuCmd::None,
            check: None,
            enabled: false,
        }
    }

    pub fn sub(label: &str, items: Vec<Item>) -> Self {
        Item::Sub {
            label: label.into(),
            items,
        }
    }

    pub fn is_selectable(&self) -> bool {
        match self {
            Item::Cmd { enabled, .. } => *enabled,
            Item::Sub { .. } => true,
            Item::Sep => false,
        }
    }
}

const PAD_X: i32 = 18;
const SEP_H: i32 = 7;

/// Laid-out menu: size and the y range of each item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuLayout {
    pub w: i32,
    pub h: i32,
    /// (top, height) per item.
    pub rows: Vec<(i32, i32)>,
    label_w: i32,
}

pub fn layout(fonts: &mut FontSystem, items: &[Item]) -> MenuLayout {
    let mut text = PixelText { fonts, bold: false };
    let row_h = text.line_height() + 4;
    let (mut label_w, mut short_w) = (0, 0);
    let mut rows = Vec::new();
    let mut y = 2;
    for it in items {
        match it {
            Item::Sep => {
                rows.push((y, SEP_H));
                y += SEP_H;
            }
            Item::Cmd {
                label, shortcut, ..
            } => {
                label_w = label_w.max(text.measure(label));
                if !shortcut.is_empty() {
                    short_w = short_w.max(text.measure(shortcut));
                }
                rows.push((y, row_h));
                y += row_h;
            }
            Item::Sub { label, .. } => {
                label_w = label_w.max(text.measure(label));
                rows.push((y, row_h));
                y += row_h;
            }
        }
    }
    let gap = if short_w > 0 { 24 } else { 0 };
    let w = (PAD_X + label_w + gap + short_w + PAD_X).max(80);
    MenuLayout {
        w,
        h: y + 2,
        rows,
        label_w,
    }
}

/// Index of the item under a point.
pub fn item_at(layout: &MenuLayout, items: &[Item], x: i32, y: i32) -> Option<usize> {
    if x < 0 || x >= layout.w {
        return None;
    }
    layout
        .rows
        .iter()
        .position(|&(top, h)| y >= top && y < top + h)
        .filter(|&i| items[i].is_selectable())
}

pub fn draw(
    fonts: &mut FontSystem,
    skin: &Skin,
    items: &[Item],
    layout: &MenuLayout,
    hi: Option<usize>,
) -> Frame {
    let c = &skin.genex;
    let mut text = PixelText { fonts, bold: false };
    let mut f = Frame::new(layout.w as u32, layout.h as u32, c.item_bg);
    f.fill_rect(0, 0, layout.w, 1, c.divider);
    f.fill_rect(0, layout.h - 1, layout.w, 1, c.divider);
    f.fill_rect(0, 0, 1, layout.h, c.divider);
    f.fill_rect(layout.w - 1, 0, 1, layout.h, c.divider);
    let lh = text.line_height();
    for (i, it) in items.iter().enumerate() {
        let (top, h) = layout.rows[i];
        if let Item::Sep = it {
            f.fill_rect(4, top + h / 2, layout.w - 8, 1, c.divider);
            continue;
        }
        let selected = hi == Some(i);
        let (fg, bg) = if selected {
            (c.selbar_text_active, c.selbar_bg_active)
        } else {
            (c.item_fg, c.item_bg)
        };
        if selected {
            f.fill_rect(2, top, layout.w - 4, h, bg);
        }
        let ty = top + (h - lh) / 2;
        let clip = (1, top, layout.w - 1, top + h);
        match it {
            Item::Cmd {
                label,
                shortcut,
                check,
                enabled,
                ..
            } => {
                let fg = if *enabled { fg } else { c.divider };
                text.draw(&mut f, label, PAD_X, ty, fg, clip);
                if !shortcut.is_empty() {
                    let w = text.measure(shortcut);
                    text.draw(&mut f, shortcut, layout.w - PAD_X - w, ty, fg, clip);
                }
                if *check == Some(true) {
                    draw_check(&mut f, 6, top + h / 2 - 3, fg);
                }
            }
            Item::Sub { label, .. } => {
                text.draw(&mut f, label, PAD_X, ty, fg, clip);
                let ax = layout.w - 10;
                let ay = top + h / 2;
                for k in 0..4 {
                    f.fill_rect(ax + k, ay - 3 + k, 1, 7 - 2 * k, fg);
                }
            }
            Item::Sep => {}
        }
    }
    let _ = layout.label_w;
    f
}

fn draw_check(f: &mut Frame, x: i32, y: i32, color: u32) {
    let pts = [
        (0, 3),
        (1, 4),
        (2, 5),
        (3, 4),
        (4, 3),
        (5, 2),
        (6, 1),
        (1, 3),
        (2, 4),
        (3, 3),
        (4, 2),
        (5, 1),
        (6, 0),
    ];
    for (dx, dy) in pts {
        f.set(x + dx, y + dy, color);
    }
}

/// Moves the highlight to the next selectable item.
pub fn step(items: &[Item], hi: Option<usize>, down: bool) -> Option<usize> {
    let n = items.len();
    if n == 0 {
        return None;
    }
    let mut i = hi.unwrap_or(if down { n - 1 } else { 0 });
    for _ in 0..n {
        i = if down { (i + 1) % n } else { (i + n - 1) % n };
        if items[i].is_selectable() {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_hits() {
        let mut fonts = FontSystem::new();
        if !fonts.has_fonts() {
            return;
        }
        let items = vec![
            Item::key("Play", "X", MenuCmd::None),
            Item::Sep,
            Item::check("Repeat", "R", true, MenuCmd::None),
            Item::sub("Skins", vec![]),
            Item::disabled("(none)"),
        ];
        let l = layout(&mut fonts, &items);
        assert_eq!(l.rows.len(), 5);
        let (top, h) = l.rows[2];
        assert_eq!(item_at(&l, &items, 10, top + h / 2), Some(2));
        assert_eq!(item_at(&l, &items, 10, l.rows[1].0 + 1), None);
        assert_eq!(item_at(&l, &items, 10, l.rows[4].0 + 1), None);
        assert_eq!(step(&items, Some(0), true), Some(2));
        assert_eq!(step(&items, Some(3), true), Some(0));
        let skin = nulamp_skin::builtin::skin();
        let f = draw(&mut fonts, &skin, &items, &l, Some(0));
        assert_eq!((f.width() as i32, f.height() as i32), (l.w, l.h));
    }
}
