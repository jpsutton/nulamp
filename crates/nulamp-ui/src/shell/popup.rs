//! Popup menu windows and menu commands.

use std::rc::Rc;
use std::time::Instant;

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId, WindowLevel};

use super::{Kind, PlayerHandle, Shell, present_raw};
use crate::input::MenuKind;
use crate::menu::{self, Item, MenuCmd, MenuLayout};
use crate::menus::{self, MenuState};
use crate::playlist::PlAction;
use nulamp_core::Command;
use nulamp_render::playlist_window::PopupMenu;

pub(super) struct MenuWin {
    pub window: Rc<Window>,
    pub surface: softbuffer::Surface<winit::event_loop::OwnedDisplayHandle, Rc<Window>>,
    pub items: Vec<Item>,
    pub layout: MenuLayout,
    pub hi: Option<usize>,
    /// Screen position (physical pixels).
    pub pos: (i32, i32),
    pub opened: Instant,
}

impl<P: PlayerHandle> Shell<P> {
    pub(super) fn menu_state(&self) -> MenuState {
        let s = &self.last_status;
        let skins = nulamp_skin::list_skins(&self.prefs.skins_dir)
            .into_iter()
            .map(|e| (e.name, e.path.map(|p| p.to_string_lossy().into_owned())))
            .collect();
        MenuState {
            main_visible: self.is_visible(Kind::Main),
            eq_visible: self.is_visible(Kind::Equalizer),
            pl_visible: self.is_visible(Kind::Playlist),
            always_on_top: self.prefs.always_on_top,
            double_size: self.prefs.double_size,
            easymove: self.prefs.easymove,
            repeat: s.repeat,
            shuffle: s.shuffle,
            manual_advance: s.manual_advance,
            stop_after_current: s.stop_after_current,
            time_remaining: self.prefs.time_remaining,
            autoscroll: self.prefs.autoscroll,
            playing: s.state == nulamp_core::PlayState::Playing,
            vis_mode: match self.prefs.vis.mode {
                nulamp_vis::VisMode::Off => 0,
                nulamp_vis::VisMode::Spectrum => 1,
                nulamp_vis::VisMode::Oscilloscope => 2,
            },
            analyzer_style: self.prefs.vis.style as u8,
            thick_bands: self.prefs.vis.thick_bands,
            peaks: self.prefs.vis.peaks,
            scope_style: self.prefs.vis.scope as u8,
            refresh: self.prefs.vis.refresh as u8,
            falloff: self.prefs.vis.falloff as u8,
            peak_falloff: self.prefs.vis.peak_falloff as u8,
            skins,
            current_skin: self.prefs.skin_path.clone(),
            bookmarks: self.bookmarks(),
            eq_enabled: self.prefs.eq.enabled,
            eq_auto: self.prefs.eq.auto_load,
            eq_presets: self.eq_library().into_iter().map(|p| p.name).collect(),
            app_name: super::APP_NAME.into(),
        }
    }

    /// Screen position of a window-local (1x) point.
    pub(super) fn screen_point(&self, id: WindowId, x: i32, y: i32) -> (i32, i32) {
        match self.windows.get(&id) {
            Some(w) => {
                let s = self.scale_of(w.kind);
                (w.actual.0 + x * s, w.actual.1 + y * s)
            }
            None => (x, y),
        }
    }

    pub(super) fn open_menu_kind(
        &mut self,
        el: &ActiveEventLoop,
        id: WindowId,
        kind: MenuKind,
        x: i32,
        y: i32,
    ) {
        let items = menus::for_kind(kind, &self.menu_state());
        let p = self.screen_point(id, x, y);
        self.open_menu(el, items, p);
    }

    pub(super) fn open_menu(&mut self, el: &ActiveEventLoop, items: Vec<Item>, pos: (i32, i32)) {
        self.close_menus();
        self.push_menu(el, items, pos);
    }

    fn push_menu(&mut self, el: &ActiveEventLoop, items: Vec<Item>, pos: (i32, i32)) {
        let layout = menu::layout(&mut self.fonts, &items);
        // Keep the menu on the monitor.
        let mut pos = pos;
        if let Some(m) = self
            .windows
            .values()
            .next()
            .and_then(|w| w.window.current_monitor())
        {
            let (mp, ms) = (m.position(), m.size());
            if pos.0 + layout.w > mp.x + ms.width as i32 {
                pos.0 = (mp.x + ms.width as i32 - layout.w).max(mp.x);
            }
            if pos.1 + layout.h > mp.y + ms.height as i32 {
                pos.1 = (pos.1 - layout.h).max(mp.y);
            }
        }
        let mut attrs = Window::default_attributes()
            .with_title("menu")
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(PhysicalSize::new(layout.w as u32, layout.h as u32))
            .with_position(PhysicalPosition::new(pos.0, pos.1));
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use winit::platform::x11::{WindowAttributesExtX11, WindowType};
            attrs = attrs
                .with_name(super::APP_NAME, super::APP_NAME)
                .with_x11_window_type(vec![WindowType::PopupMenu]);
        }
        let window = match el.create_window(attrs) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("cannot create menu window: {e}");
                return;
            }
        };
        let window = Rc::new(window);
        let Some(context) = self.context.as_ref() else {
            return;
        };
        let Ok(surface) = softbuffer::Surface::new(context, window.clone()) else {
            return;
        };
        log::debug!("menu opened at {pos:?} ({}x{})", layout.w, layout.h);
        window.focus_window();
        self.menus.push(MenuWin {
            window,
            surface,
            items,
            layout,
            hi: None,
            pos,
            opened: Instant::now(),
        });
        let last = self.menus.len() - 1;
        self.draw_menu(last);
    }

    pub(super) fn close_menus(&mut self) {
        self.menus.clear();
    }

    fn close_menus_from(&mut self, level: usize) {
        self.menus.truncate(level);
    }

    pub(super) fn menu_level(&self, id: WindowId) -> Option<usize> {
        self.menus.iter().position(|m| m.window.id() == id)
    }

    fn draw_menu(&mut self, level: usize) {
        let Some(m) = self.menus.get(level) else {
            return;
        };
        let frame = menu::draw(&mut self.fonts, &self.skin, &m.items, &m.layout, m.hi);
        let m = &mut self.menus[level];
        present_raw(&m.window, &mut m.surface, &frame);
    }

    fn open_submenu_of(&mut self, el: &ActiveEventLoop, level: usize, index: usize) {
        let m = &self.menus[level];
        let Some(Item::Sub { items, .. }) = m.items.get(index) else {
            return;
        };
        let items = items.clone();
        let pos = (
            m.pos.0 + m.layout.w - 3,
            m.pos.1 + m.layout.rows[index].0 - 2,
        );
        self.close_menus_from(level + 1);
        self.push_menu(el, items, pos);
    }

    /// Events for menu windows. Returns true when the event was for a menu.
    pub(super) fn menu_event(
        &mut self,
        el: &ActiveEventLoop,
        id: WindowId,
        event: &WindowEvent,
    ) -> bool {
        let Some(level) = self.menu_level(id) else {
            return false;
        };
        match event {
            WindowEvent::RedrawRequested => self.draw_menu(level),
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x as i32, position.y as i32);
                let m = &self.menus[level];
                let hi = menu::item_at(&m.layout, &m.items, x, y);
                if hi != m.hi {
                    self.menus[level].hi = hi;
                    self.draw_menu(level);
                    match hi.and_then(|i| self.menus[level].items.get(i).cloned()) {
                        Some(Item::Sub { .. }) => {
                            self.open_submenu_of(el, level, hi.expect("item"))
                        }
                        _ => self.close_menus_from(level + 1),
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                // Ignore the release of the click that opened the menu.
                if self.menus[level].opened.elapsed().as_millis() < 250 {
                    return true;
                }
                self.activate(el, level);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.menu_key(el, level, code);
                }
            }
            WindowEvent::Focused(false) => self.menu_focus_check = Some(Instant::now()),
            WindowEvent::Focused(true) => self.menu_focus_check = None,
            _ => {}
        }
        true
    }

    fn menu_key(&mut self, el: &ActiveEventLoop, level: usize, code: KeyCode) {
        match code {
            KeyCode::Escape => self.close_menus_from(level),
            KeyCode::ArrowDown | KeyCode::ArrowUp => {
                let m = &mut self.menus[level];
                m.hi = menu::step(&m.items, m.hi, code == KeyCode::ArrowDown);
                self.draw_menu(level);
            }
            KeyCode::ArrowRight => {
                if let Some(i) = self.menus[level].hi
                    && matches!(self.menus[level].items[i], Item::Sub { .. })
                {
                    self.open_submenu_of(el, level, i);
                }
            }
            KeyCode::ArrowLeft if level > 0 => self.close_menus_from(level),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.activate(el, level),
            _ => {}
        }
    }

    fn activate(&mut self, el: &ActiveEventLoop, level: usize) {
        let Some(i) = self.menus[level].hi else {
            return;
        };
        match self.menus[level].items[i].clone() {
            Item::Cmd {
                cmd, enabled: true, ..
            } => {
                self.close_menus();
                let target = self
                    .focused
                    .filter(|f| self.windows.contains_key(f))
                    .or_else(|| self.ids.get(&Kind::Main).copied());
                if let Some(t) = target {
                    self.menu_cmd(el, t, cmd);
                }
            }
            Item::Sub { .. } => self.open_submenu_of(el, level, i),
            _ => {}
        }
    }

    /// Closes menus when focus went to something that is not a menu.
    pub(super) fn check_menu_focus(&mut self) {
        if let Some(t) = self.menu_focus_check
            && t.elapsed().as_millis() > 120
        {
            self.menu_focus_check = None;
            let any_focused = self.menus.iter().any(|m| m.window.has_focus());
            if !any_focused {
                self.close_menus();
            }
        }
    }

    pub(super) fn menu_cmd(&mut self, el: &ActiveEventLoop, id: WindowId, cmd: MenuCmd) {
        let cmd_clone_kind = |c: &MenuCmd| match c {
            MenuCmd::PlShowInFolder => 0,
            MenuCmd::PlGenerateHtml => 1,
            MenuCmd::PlRebuildTitles => 2,
            MenuCmd::PlEditEntry => 3,
            _ => 4,
        };
        match cmd {
            MenuCmd::Act(a) => self.handle_action(id, a, el),
            MenuCmd::Key(k) => self.key_action(id, k, el),
            MenuCmd::Pl(choice) => {
                let pl_id = self.ids.get(&Kind::Playlist).copied().unwrap_or(id);
                self.handle_pl_actions(pl_id, vec![PlAction::Menu(choice)], el);
            }
            MenuCmd::EqEnabled => {
                self.prefs.eq.enabled = !self.prefs.eq.enabled;
                self.eq_changed();
            }
            MenuCmd::EqAutoLoad => {
                self.prefs.eq.auto_load = !self.prefs.eq.auto_load;
                self.eq_changed();
            }
            MenuCmd::SetTimeRemaining(on) => self.prefs.time_remaining = on,
            MenuCmd::EqLoadPreset(name) => {
                let lib = self.eq_library();
                if let Some(p) = nulamp_eq::preset::find(&lib, &name) {
                    self.apply_preset(p.bands, p.preamp);
                }
            }
            MenuCmd::EqLoadDefault => {
                let lib = self.eq_library();
                if let Some(p) =
                    nulamp_eq::preset::find(&lib, nulamp_eq::preset::DEFAULT_PRESET_NAME)
                {
                    self.apply_preset(p.bands, p.preamp);
                }
            }
            MenuCmd::EqLoadAutoPreset => {
                let q2 = nulamp_eq::preset::read_library_or_empty(&self.eq_auto_path());
                let file = self.current_file_name();
                if let Some(p) = nulamp_eq::preset::find(&q2, &file) {
                    self.apply_preset(p.bands, p.preamp);
                }
            }
            MenuCmd::EqSavePreset => {
                let names = self.eq_library().into_iter().map(|p| p.name).collect();
                self.pending_dialog = Some(Box::new(crate::dialogs::PresetSaveDialog::new(
                    names, "", false,
                )));
            }
            MenuCmd::EqSaveAutoPreset => {
                let names = nulamp_eq::preset::read_library_or_empty(&self.eq_auto_path())
                    .into_iter()
                    .map(|p| p.name)
                    .collect();
                let file = self.current_file_name();
                self.pending_dialog = Some(Box::new(crate::dialogs::PresetSaveDialog::new(
                    names, &file, true,
                )));
            }
            MenuCmd::EqSaveDefault => self.eq_save(nulamp_eq::preset::DEFAULT_PRESET_NAME, false),
            MenuCmd::EqDeletePreset(name) => {
                if let Err(e) = nulamp_eq::preset::delete_preset(&self.eq_library_path(), &name) {
                    log::error!("cannot delete preset: {e}");
                }
            }
            MenuCmd::EqImportEqf => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("EQ preset", &["eqf", "EQF"])
                    .pick_file()
                {
                    match nulamp_eq::preset::import_eqf(&path) {
                        Ok(p) => self.apply_preset(p.bands, p.preamp),
                        Err(e) => log::error!("cannot import {}: {e}", path.display()),
                    }
                }
            }
            MenuCmd::EqExportEqf => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("preset.eqf")
                    .add_filter("EQ preset", &["eqf"])
                    .save_file()
                    && let Err(e) = nulamp_eq::preset::export_eqf(
                        &path,
                        self.prefs.eq.bands,
                        self.prefs.eq.preamp,
                    )
                {
                    log::error!("cannot export {}: {e}", path.display());
                }
            }
            MenuCmd::SetVisMode(m) => {
                let mode = match m {
                    1 => nulamp_vis::VisMode::Spectrum,
                    2 => nulamp_vis::VisMode::Oscilloscope,
                    _ => nulamp_vis::VisMode::Off,
                };
                self.set_vis_mode(mode);
            }
            MenuCmd::SetAnalyzerStyle(i) => {
                use nulamp_vis::AnalyzerStyle::*;
                self.prefs.vis.style = [Normal, Fire, Line][i.min(2) as usize];
                self.visualizer.set_settings(self.prefs.vis);
            }
            MenuCmd::SetBands(thick) => {
                self.prefs.vis.thick_bands = thick;
                self.visualizer.set_settings(self.prefs.vis);
            }
            MenuCmd::TogglePeaks => {
                self.prefs.vis.peaks = !self.prefs.vis.peaks;
                self.visualizer.set_settings(self.prefs.vis);
            }
            MenuCmd::SetScopeStyle(i) => {
                use nulamp_vis::ScopeStyle::*;
                self.prefs.vis.scope = [Dots, Lines, Solid][i.min(2) as usize];
                self.visualizer.set_settings(self.prefs.vis);
            }
            MenuCmd::SetRefresh(i) => {
                use nulamp_vis::RefreshRate::*;
                self.prefs.vis.refresh = [Full, Half, Quarter, Eighth][i.min(3) as usize];
                self.visualizer.set_settings(self.prefs.vis);
            }
            MenuCmd::SetFalloff(i) => {
                self.prefs.vis.falloff = nulamp_vis::Speed::ALL[i.min(4) as usize];
                self.visualizer.set_settings(self.prefs.vis);
            }
            MenuCmd::SetPeakFalloff(i) => {
                self.prefs.vis.peak_falloff = nulamp_vis::Speed::ALL[i.min(4) as usize];
                self.visualizer.set_settings(self.prefs.vis);
            }
            MenuCmd::LoadSkin(path) => self.load_skin(path),
            MenuCmd::PlSortTitle => self.player.engine_mut().playlist.sort_by_title(),
            MenuCmd::PlSortFileName => self.player.engine_mut().playlist.sort_by_file_name(),
            MenuCmd::PlSortPath => self.player.engine_mut().playlist.sort_by_path(),
            MenuCmd::PlReverse => self.player.engine_mut().playlist.reverse(),
            MenuCmd::PlRandomize => self
                .player
                .engine_mut()
                .playlist
                .randomize(&mut rand::rng()),
            MenuCmd::PlRemoveDuplicates => {
                let mut seen = std::collections::HashSet::new();
                self.player
                    .engine_mut()
                    .remove_where(|_, e| !seen.insert(e.location.clone()));
            }
            MenuCmd::PlRemoveMissing => {
                self.player.engine_mut().remove_where(|_, e| {
                    !nulamp_core::playlist::is_url(&e.location)
                        && std::fs::File::open(&e.location).is_err()
                });
            }
            MenuCmd::PlFileInfo => {
                let pl = self.ids.get(&Kind::Playlist).copied().unwrap_or(id);
                self.handle_pl_actions(pl, vec![PlAction::FileInfoSelected], el);
            }
            MenuCmd::Bookmark(loc) => self.play_bookmark(&loc),
            MenuCmd::Plugins => {
                let (rows, errors) = self.player.plugins();
                self.open_dialog(
                    el,
                    Box::new(crate::plugins_dialog::PluginsDialog::new(rows, errors)),
                );
            }
            MenuCmd::Rate(r) => {
                let loc = self
                    .player
                    .engine()
                    .playlist
                    .get(self.last_status.position)
                    .map(|e| e.location.clone());
                if let Some(loc) = loc {
                    self.rate_files(&[loc], r);
                }
            }
            MenuCmd::PlRateSelected(r) => {
                let locs: Vec<String> = self
                    .player
                    .engine()
                    .playlist
                    .entries()
                    .iter()
                    .filter(|e| e.selected)
                    .map(|e| e.location.clone())
                    .collect();
                self.rate_files(&locs, r);
            }
            MenuCmd::PlPlaySelected => {
                if let Some(i) = self.player.engine().playlist.first_selected() {
                    self.player.command(Command::PlayIndex(i));
                }
            }
            MenuCmd::PlShowInFolder
            | MenuCmd::PlGenerateHtml
            | MenuCmd::PlRebuildTitles
            | MenuCmd::PlEditEntry
            | MenuCmd::PlPhysicallyRemove => {
                let pl = self.ids.get(&Kind::Playlist).copied().unwrap_or(id);
                let a = match cmd_clone_kind(&cmd) {
                    0 => PlAction::ShowInFolder,
                    1 => PlAction::GenerateHtml,
                    2 => PlAction::RebuildTitles,
                    3 => PlAction::EditEntries,
                    _ => {
                        self.physically_remove_selected();
                        return;
                    }
                };
                self.handle_pl_actions(pl, vec![a], el);
            }
            MenuCmd::PlEnqueueSelected => {
                let e = self.player.engine_mut();
                let ids: Vec<_> = e
                    .playlist
                    .entries()
                    .iter()
                    .filter(|x| x.selected)
                    .map(|x| x.id)
                    .collect();
                for id in ids {
                    if let Some(pos) = e.queue.iter().position(|q| *q == id) {
                        e.queue.remove(pos);
                    } else {
                        e.queue.push_back(id);
                    }
                }
            }
            other => log::info!("menu command not implemented yet: {other:?}"),
        }
        self.poll_player();
        self.request_redraw_all();
    }

    /// Standard playlist menu at a playlist-window point.
    pub(super) fn open_playlist_menu(
        &mut self,
        el: &ActiveEventLoop,
        id: WindowId,
        menu: PopupMenu,
        x: i32,
        y: i32,
    ) {
        let items = menus::playlist_menu(menu);
        let p = self.screen_point(id, x, y);
        self.open_menu(el, items, p);
    }

    pub(super) fn open_items_at(
        &mut self,
        el: &ActiveEventLoop,
        id: WindowId,
        items: Vec<Item>,
        x: i32,
        y: i32,
    ) {
        let p = self.screen_point(id, x, y);
        self.open_menu(el, items, p);
    }

    pub(super) fn open_main_menu_at(&mut self, el: &ActiveEventLoop, id: WindowId, x: i32, y: i32) {
        self.open_menu_kind(el, id, MenuKind::Main, x, y);
    }
}
