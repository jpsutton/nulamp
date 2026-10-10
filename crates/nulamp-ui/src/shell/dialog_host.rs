//! Dialog windows: generic skinned frames hosting a [`Form`](nulamp_skinui::Form).

use std::rc::Rc;

use nulamp_render::gen_window::{self, GenView};
use nulamp_skinui::{Key, PixelText};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use super::{Kind, PlayerHandle, Shell, present_raw};
use crate::dialogs::{Dialog, DialogResult, Outcome};
use nulamp_core::Command;

pub(super) struct DialogWin {
    pub window: Rc<Window>,
    pub surface: softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>,
    pub dialog: Box<dyn Dialog>,
    pub close_pressed: bool,
    pub cursor: (i32, i32),
}

fn map_key(code: KeyCode, shift: bool) -> Option<Key> {
    Some(match code {
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Enter | KeyCode::NumpadEnter => Key::Enter,
        KeyCode::Escape => Key::Escape,
        KeyCode::Tab if shift => Key::ShiftTab,
        KeyCode::Tab => Key::Tab,
        _ => return None,
    })
}

impl<P: PlayerHandle> Shell<P> {
    /// Opens a dialog, or brings an open one of the same kind to the front.
    pub(super) fn open_dialog(&mut self, el: &ActiveEventLoop, dialog: Box<dyn Dialog>) {
        if let Some(d) = self
            .dialogs
            .values()
            .find(|d| d.dialog.kind() == dialog.kind())
        {
            d.window.focus_window();
            return;
        }
        let (w, h) = dialog.size();
        let (mx, my) = self
            .ids
            .get(&Kind::Main)
            .and_then(|id| self.windows.get(id))
            .map(|m| (m.rect.x + (m.rect.w - w) / 2, m.rect.y + m.rect.h))
            .unwrap_or((100, 100));
        let mut attrs = Window::default_attributes()
            .with_title(format!("{} - {}", dialog.title(), super::APP_NAME))
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(PhysicalSize::new(w as u32, h as u32))
            .with_position(PhysicalPosition::new(mx.max(0), my.max(0)));
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use winit::platform::x11::{WindowAttributesExtX11, WindowType};
            attrs = attrs
                .with_name(super::APP_NAME, super::APP_NAME)
                .with_x11_window_type(vec![WindowType::Dialog]);
        }
        let Ok(window) = el.create_window(attrs) else {
            return;
        };
        let window = Rc::new(window);
        let Some(context) = self.context.as_ref() else {
            return;
        };
        let Ok(surface) = softbuffer::Surface::new(context, window.clone()) else {
            return;
        };
        window.focus_window();
        let id = window.id();
        self.dialogs.insert(
            id,
            DialogWin {
                window,
                surface,
                dialog,
                close_pressed: false,
                cursor: (0, 0),
            },
        );
        self.draw_dialog(id);
    }

    fn draw_dialog(&mut self, id: WindowId) {
        let Some(d) = self.dialogs.get_mut(&id) else {
            return;
        };
        let (w, h) = d.dialog.size();
        let view = GenView {
            w,
            h,
            title: d.dialog.title(),
            active: d.window.has_focus(),
            close_pressed: d.close_pressed,
            fixed_size: true,
        };
        let mut frame = gen_window::draw_gen_frame(&self.skin, &view);
        let (cx, cy, cw, ch) = gen_window::client_rect(w, h);
        frame.fill_rect(cx, cy, cw, ch, self.skin.genex.window_bg);
        let mut text = PixelText {
            fonts: &mut self.fonts,
            bold: false,
        };
        d.dialog
            .form()
            .draw(&mut frame, cx, cy, &self.skin, &mut text);
        present_raw(&d.window, &mut d.surface, &frame);
    }

    fn apply_outcome(&mut self, el: &ActiveEventLoop, id: WindowId, outcome: Outcome) {
        match outcome {
            Outcome::Keep => {}
            Outcome::Close => {
                self.dialogs.remove(&id);
            }
            Outcome::Do(results, close) => {
                if close {
                    self.dialogs.remove(&id);
                }
                for r in results {
                    self.dialog_result(el, r);
                }
            }
        }
        if self.dialogs.contains_key(&id) {
            self.draw_dialog(id);
        }
    }

    fn dialog_result(&mut self, _el: &ActiveEventLoop, r: DialogResult) {
        match r {
            DialogResult::OpenLocation { location, add } => {
                self.prefs.recent_urls.retain(|u| *u != location);
                self.prefs.recent_urls.insert(0, location.clone());
                self.prefs.recent_urls.truncate(16);
                self.add_items(vec![location], !add);
            }
            DialogResult::SeekTo(ms) => self.player.command(Command::SeekTo(ms)),
            DialogResult::PlayEntry(eid) => {
                if let Some(i) = self.player.engine().playlist.index_of(eid) {
                    self.player.command(Command::PlayIndex(i));
                }
            }
            DialogResult::EnqueueEntry(eid) => {
                let q = &mut self.player.engine_mut().queue;
                if !q.contains(&eid) {
                    q.push_back(eid);
                }
            }
            DialogResult::LoadSkin(path) => self.load_skin(path),
            DialogResult::EqSave { name, auto } => self.eq_save(&name, auto),
            DialogResult::SetBookmarks(items) => self.save_bookmarks(&items),
            DialogResult::DeleteFiles(files) => self.delete_files(&files),
            DialogResult::SetLocation(eid, loc) => {
                if let Some(e) = self.player.engine_mut().playlist.by_id_mut(eid) {
                    e.title = nulamp_core::playlist::placeholder_title(&loc);
                    e.location = loc;
                    e.metadata_read = false;
                    e.length_ms = None;
                }
                self.player.engine_mut().playlist.modified = true;
            }
            DialogResult::PlayBookmark(loc) => self.play_bookmark(&loc),
            DialogResult::Plugin(i, action) => self.player.plugin_action(i, action),
            DialogResult::Prefs(b) => {
                let (prefs, pl) = *b;
                self.apply_prefs(prefs, pl);
            }
            DialogResult::TagsWritten(location) => {
                // Re-read titles of every entry with this file (10 §8).
                for e in self
                    .player
                    .engine_mut()
                    .playlist
                    .entries_mut_by_location(&location)
                {
                    e.metadata_read = false;
                }
            }
        }
        self.poll_player();
        self.request_redraw_all();
    }

    /// Events for dialog windows. Returns true when the event was for a dialog.
    pub(super) fn dialog_event(
        &mut self,
        el: &ActiveEventLoop,
        id: WindowId,
        event: &WindowEvent,
    ) -> bool {
        if !self.dialogs.contains_key(&id) {
            return false;
        }
        let shift = self.mods.shift;
        let plain = !self.mods.ctrl && !self.mods.alt;
        match event {
            WindowEvent::RedrawRequested | WindowEvent::Focused(_) => self.draw_dialog(id),
            WindowEvent::CloseRequested => {
                self.dialogs.remove(&id);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let d = self.dialogs.get_mut(&id).expect("dialog");
                d.cursor = (position.x as i32, position.y as i32);
                let (cx, cy, _, _) = gen_window::client_rect(d.dialog.size().0, d.dialog.size().1);
                d.dialog.form().mouse_move(d.cursor.0 - cx, d.cursor.1 - cy);
                self.draw_dialog(id);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let d = self.dialogs.get_mut(&id).expect("dialog");
                let (w, h) = d.dialog.size();
                let (x, y) = d.cursor;
                let (cx, cy, _, _) = gen_window::client_rect(w, h);
                let on_close = x >= w - 11 && x < w - 1 && (3..=12).contains(&y);
                let outcome = match state {
                    ElementState::Pressed if on_close => {
                        d.close_pressed = true;
                        None
                    }
                    ElementState::Pressed if y < 20 => {
                        let _ = d.window.drag_window();
                        None
                    }
                    ElementState::Pressed => d.dialog.form().mouse_down(x - cx, y - cy),
                    ElementState::Released if d.close_pressed => {
                        d.close_pressed = false;
                        if on_close {
                            self.dialogs.remove(&id);
                            return true;
                        }
                        None
                    }
                    ElementState::Released => d.dialog.form().mouse_up(x - cx, y - cy),
                };
                let outcome = outcome
                    .map(|ev| d.dialog.handle(ev))
                    .unwrap_or(Outcome::Keep);
                self.apply_outcome(el, id, outcome);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = self.dialogs.get_mut(&id).expect("dialog");
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y as i32,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as i32,
                };
                let (cx, cy, _, _) = gen_window::client_rect(d.dialog.size().0, d.dialog.size().1);
                d.dialog
                    .form()
                    .wheel(d.cursor.0 - cx, d.cursor.1 - cy, notches);
                self.draw_dialog(id);
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } if event.state == ElementState::Pressed => {
                let d = self.dialogs.get_mut(&id).expect("dialog");
                let mut ev = None;
                if let PhysicalKey::Code(code) = event.physical_key {
                    if let Some(k) = map_key(code, shift) {
                        ev = d.dialog.form().key(k);
                    } else if plain && let Some(t) = event.text.as_ref() {
                        ev = d.dialog.form().text_input(t);
                    }
                }
                let outcome = ev.map(|e| d.dialog.handle(e)).unwrap_or(Outcome::Keep);
                self.apply_outcome(el, id, outcome);
            }
            _ => {}
        }
        true
    }
}
