//! Native Wayland compound window (docs/decisions.md D5, 05 §6.5).
//!
//! Wayland clients cannot place their own top-level windows, so classic docking cannot work
//! with one OS window per skinned window. In this mode the main window is the only top-level
//! window; the equalizer and playlist are subsurfaces of it ("parts"). The app may place a
//! subsurface anywhere relative to its parent, and the compositor moves and stacks it with the
//! parent.
//!
//! Parts live on a private event queue on winit's connection. winit only handles input for
//! surfaces it created, so pointer events on parts reach [`Compound::dispatch`] only. All
//! surfaces get ARGB buffers: pixels outside the skin's shape are transparent, and the input
//! region follows the same shape.

use std::os::fd::AsFd;

use nulamp_render::Frame;
use nulamp_render::region::Mask;
use nulamp_skin::CursorShape;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_subcompositor, wl_subsurface, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{self, Shape},
    wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use winit::event::MouseButton;

/// The part that stands for the main (top-level) window.
pub const MAIN: usize = 0;

/// Input on a part, in physical pixels relative to the part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Motion {
        part: usize,
        x: i32,
        y: i32,
    },
    Leave {
        part: usize,
    },
    Button {
        part: usize,
        button: MouseButton,
        pressed: bool,
    },
    /// Wheel movement in lines, positive = up (winit's convention).
    Wheel {
        part: usize,
        lines: f32,
    },
    /// A position from [`Compound::set_position`] took effect: physical offset from the main
    /// window. Pointer events after this one are relative to the new position.
    Moved {
        part: usize,
        x: i32,
        y: i32,
    },
}

/// The main window plus its subsurface parts.
pub struct Compound {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
}

struct State {
    qh: QueueHandle<State>,
    compositor: wl_compositor::WlCompositor,
    subcompositor: wl_subcompositor::WlSubcompositor,
    shm: wl_shm::WlShm,
    viewporter: Option<wp_viewporter::WpViewporter>,
    cursor_shapes: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    /// Kept bound so seat capability events keep arriving.
    _seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    cursor: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    /// Indexed by part number; `None` for removed parts, whose numbers are reused.
    parts: Vec<Option<Part>>,
    /// The part under the pointer and the serial of its enter event.
    focus: Option<(usize, u32)>,
    scale: f64,
    events: Vec<Event>,
}

struct Part {
    surface: wl_surface::WlSurface,
    /// `None` for the main window.
    subsurface: Option<wl_subsurface::WlSubsurface>,
    viewport: Option<wp_viewport::WpViewport>,
    buffers: Vec<Buffer>,
    /// Logical size last given to the viewport.
    dest: (i32, i32),
    shape: Option<Mask>,
    shape_dirty: bool,
    cursor: Shape,
}

struct Buffer {
    buffer: wl_buffer::WlBuffer,
    map: memmap2::MmapMut,
    size: (u32, u32),
    busy: bool,
}

impl Compound {
    /// Attaches to winit's Wayland connection. `None` when not running on Wayland or when the
    /// compositor lacks subsurfaces.
    pub fn new(display: &impl HasDisplayHandle, main: &impl HasWindowHandle) -> Option<Self> {
        let RawDisplayHandle::Wayland(d) = display.display_handle().ok()?.as_raw() else {
            return None;
        };
        let RawWindowHandle::Wayland(w) = main.window_handle().ok()?.as_raw() else {
            return None;
        };
        // SAFETY: winit's display stays alive for the whole event loop, which outlives `self`.
        let backend = unsafe { Backend::from_foreign_display(d.display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        // SAFETY: the pointer is winit's live `wl_surface` for the main window.
        let main_id = unsafe {
            ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                w.surface.as_ptr().cast(),
            )
        }
        .ok()?;
        let main_surface = wl_surface::WlSurface::from_id(&conn, main_id).ok()?;
        let compositor = globals.bind(&qh, 4..=6, ()).ok()?;
        let subcompositor = globals.bind(&qh, 1..=1, ()).ok()?;
        let shm = globals.bind(&qh, 1..=1, ()).ok()?;
        let viewporter = globals.bind(&qh, 1..=1, ()).ok();
        let cursor_shapes = globals.bind(&qh, 1..=1, ()).ok();
        let seat = globals.bind(&qh, 1..=7, ()).ok();
        let state = State {
            qh,
            compositor,
            subcompositor,
            shm,
            viewporter,
            cursor_shapes,
            _seat: seat,
            pointer: None,
            cursor: None,
            parts: vec![Some(Part {
                surface: main_surface,
                subsurface: None,
                viewport: None,
                buffers: Vec::new(),
                dest: (0, 0),
                shape: None,
                shape_dirty: true,
                cursor: Shape::Default,
            })],
            focus: None,
            scale: 1.0,
            events: Vec::new(),
        };
        let mut c = Self { conn, queue, state };
        // Seat capabilities arrive in this round trip and create the pointer.
        c.queue.roundtrip(&mut c.state).ok()?;
        Some(c)
    }

    /// Adds a hidden subsurface part above the main window and the other parts, and returns
    /// its number.
    pub fn add_part(&mut self) -> usize {
        let s = &mut self.state;
        let index = s
            .parts
            .iter()
            .position(Option::is_none)
            .unwrap_or(s.parts.len());
        let surface = s.compositor.create_surface(&s.qh, index);
        let Some(main) = &s.parts[MAIN] else {
            unreachable!("the main part is never removed");
        };
        let parent = &main.surface;
        let subsurface = s.subcompositor.get_subsurface(&surface, parent, &s.qh, ());
        // Parts redraw on their own; only position changes wait for a main-window commit.
        subsurface.set_desync();
        let viewport = s
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(&surface, &s.qh, ()));
        let part = Part {
            surface,
            subsurface: Some(subsurface),
            viewport,
            buffers: Vec::new(),
            dest: (0, 0),
            shape: None,
            shape_dirty: true,
            cursor: Shape::Default,
        };
        match s.parts.get_mut(index) {
            Some(slot) => *slot = Some(part),
            None => s.parts.push(Some(part)),
        }
        index
    }

    /// Destroys a part. Its number may be returned by a later [`Compound::add_part`].
    pub fn remove_part(&mut self, part: usize) {
        if part == MAIN {
            return;
        }
        if let Some(p) = self.state.parts.get_mut(part).and_then(Option::take) {
            p.destroy();
        }
        if self.state.focus.is_some_and(|(f, _)| f == part) {
            self.state.focus = None;
        }
        let _ = self.conn.flush();
    }

    /// The main window's scale factor (physical pixels per logical pixel).
    pub fn set_scale(&mut self, scale: f64) {
        if scale > 0.0 && scale != self.state.scale {
            self.state.scale = scale;
            for p in self.state.parts.iter_mut().flatten() {
                p.dest = (0, 0);
                p.shape_dirty = true;
            }
        }
    }

    /// Places a part at a physical offset from the main window. An [`Event::Moved`] follows
    /// once the compositor has applied it.
    pub fn set_position(&mut self, part: usize, x: i32, y: i32) {
        let s = &mut self.state;
        let Some(sub) = s.part(part).and_then(|p| p.subsurface.as_ref()) else {
            return;
        };
        sub.set_position(logical(x, s.scale), logical(y, s.scale));
        // Subsurface positions are applied with the parent's next commit.
        if let Some(main) = s.part(MAIN) {
            main.surface.commit();
        }
        self.conn.display().sync(&s.qh, (part, x, y));
        let _ = self.conn.flush();
    }

    /// Sets the shape used for transparency and input. `None` is the full rectangle.
    pub fn set_shape(&mut self, part: usize, mask: Option<Mask>) {
        if let Some(p) = self.state.part_mut(part) {
            p.shape = mask;
            p.shape_dirty = true;
        }
    }

    /// Shows a frame on a part (mapping it if hidden). Returns false when every buffer is
    /// still in use by the compositor; draw again later.
    pub fn present(&mut self, part: usize, frame: &Frame) -> bool {
        let s = &mut self.state;
        let scale = s.scale;
        let (shm, qh) = (&s.shm, &s.qh);
        let Some(p) = s.parts.get_mut(part).and_then(Option::as_mut) else {
            return true;
        };
        let size = (frame.width(), frame.height());
        if size.0 == 0 || size.1 == 0 {
            return true;
        }
        p.buffers.retain(|b| b.busy || b.size == size);
        let slot = match p.buffers.iter().position(|b| !b.busy) {
            Some(i) => i,
            None if p.buffers.len() < 3 => {
                let Some(b) = Buffer::new(shm, qh, part, p.buffers.len(), size) else {
                    return true;
                };
                p.buffers.push(b);
                p.buffers.len() - 1
            }
            None => return false,
        };
        let buf = &mut p.buffers[slot];
        let shape = p.shape.as_ref().filter(|m| (m.width, m.height) == size);
        for (i, px) in frame.pixels().iter().enumerate() {
            let (x, y) = ((i as u32 % size.0) as i32, (i as u32 / size.0) as i32);
            let argb = if shape.is_none_or(|m| m.get(x, y)) {
                0xFF00_0000 | px
            } else {
                0
            };
            buf.map[i * 4..i * 4 + 4].copy_from_slice(&argb.to_le_bytes());
        }
        buf.busy = true;
        if p.shape_dirty {
            let region = s.compositor.create_region(qh, ());
            match shape {
                Some(m) => {
                    for (x, y, w) in m.runs() {
                        let (x0, y0) = (logical(x as i32, scale), logical(y as i32, scale));
                        let x1 = logical_ceil((x + w) as i32, scale);
                        let y1 = logical_ceil(y as i32 + 1, scale);
                        region.add(x0, y0, x1 - x0, y1 - y0);
                    }
                }
                None => region.add(0, 0, i32::MAX / 2, i32::MAX / 2),
            }
            p.surface.set_input_region(Some(&region));
            region.destroy();
            p.shape_dirty = false;
        }
        let dest = (
            logical_ceil(size.0 as i32, scale),
            logical_ceil(size.1 as i32, scale),
        );
        if let Some(v) = &p.viewport
            && dest != p.dest
        {
            v.set_destination(dest.0, dest.1);
            p.dest = dest;
        }
        p.surface.attach(Some(&buf.buffer), 0, 0);
        p.surface.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
        p.surface.commit();
        let _ = self.conn.flush();
        true
    }

    /// Unmaps a part.
    pub fn hide(&mut self, part: usize) {
        if part == MAIN {
            return;
        }
        if let Some(p) = self.state.part(part) {
            p.surface.attach(None, 0, 0);
            p.surface.commit();
            let _ = self.conn.flush();
        }
    }

    /// The pointer shape over a part.
    pub fn set_cursor(&mut self, part: usize, shape: CursorShape) {
        let s = &mut self.state;
        let Some(p) = s.parts.get_mut(part).and_then(Option::as_mut) else {
            return;
        };
        p.cursor = cursor_shape(shape);
        if let (Some((focus, serial)), Some(dev)) = (s.focus, &s.cursor)
            && focus == part
        {
            dev.set_shape(serial, p.cursor);
            let _ = self.conn.flush();
        }
    }

    /// Handles queued Wayland events and returns the input for parts, oldest first.
    pub fn dispatch(&mut self) -> Vec<Event> {
        // winit reads the shared socket; events for this queue are already waiting in it.
        if let Err(e) = self.queue.dispatch_pending(&mut self.state) {
            log::error!("wayland: {e}");
        }
        let _ = self.conn.flush();
        std::mem::take(&mut self.state.events)
    }
}

impl Drop for Compound {
    fn drop(&mut self) {
        let s = &mut self.state;
        for p in s.parts.drain(..).skip(1).flatten() {
            p.destroy();
        }
        if let Some(c) = s.cursor.take() {
            c.destroy();
        }
        if let Some(p) = s.pointer.take() {
            p.release();
        }
        // The main surface belongs to winit, which destroys it with the window.
        let _ = self.conn.flush();
    }
}

impl Part {
    fn destroy(self) {
        if let Some(v) = self.viewport {
            v.destroy();
        }
        if let Some(sub) = self.subsurface {
            sub.destroy();
        }
        drop(self.buffers);
        self.surface.destroy();
    }
}

impl Buffer {
    fn new(
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<State>,
        part: usize,
        slot: usize,
        size: (u32, u32),
    ) -> Option<Self> {
        let stride = size.0 as i32 * 4;
        let len = stride as usize * size.1 as usize;
        let fd = rustix::fs::memfd_create("nulamp-part", rustix::fs::MemfdFlags::CLOEXEC).ok()?;
        rustix::fs::ftruncate(&fd, len as u64).ok()?;
        let file = std::fs::File::from(fd);
        // SAFETY: the memory file is private to this process and the compositor, and its
        // size is fixed for the lifetime of the mapping.
        let map = unsafe { memmap2::MmapMut::map_mut(&file) }.ok()?;
        let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            size.0 as i32,
            size.1 as i32,
            stride,
            wl_shm::Format::Argb8888,
            qh,
            (part, slot),
        );
        pool.destroy();
        Some(Self {
            buffer,
            map,
            size,
            busy: false,
        })
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

fn logical(v: i32, scale: f64) -> i32 {
    (v as f64 / scale).floor() as i32
}

fn logical_ceil(v: i32, scale: f64) -> i32 {
    (v as f64 / scale).ceil() as i32
}

fn cursor_shape(shape: CursorShape) -> Shape {
    match shape {
        CursorShape::Arrow => Shape::Default,
        CursorShape::Move => Shape::Move,
        CursorShape::Close => Shape::Pointer,
        CursorShape::HorizontalResize => Shape::EwResize,
        CursorShape::VerticalResize => Shape::NsResize,
        CursorShape::DiagonalResize => Shape::NwseResize,
    }
}

impl State {
    /// The part a surface belongs to; `None` for surfaces this module did not create.
    fn part_of(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        surface.data::<usize>().copied()
    }

    fn part(&self, part: usize) -> Option<&Part> {
        self.parts.get(part)?.as_ref()
    }

    fn part_mut(&mut self, part: usize) -> Option<&mut Part> {
        self.parts.get_mut(part)?.as_mut()
    }

    fn physical(&self, v: f64) -> i32 {
        (v * self.scale).floor() as i32
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, usize> for State {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        else {
            return;
        };
        let has_pointer = caps.contains(wl_seat::Capability::Pointer);
        if has_pointer && state.pointer.is_none() {
            let pointer = seat.get_pointer(qh, ());
            state.cursor = state
                .cursor_shapes
                .as_ref()
                .map(|m| m.get_pointer(&pointer, qh, ()));
            state.pointer = Some(pointer);
        } else if !has_pointer && let Some(p) = state.pointer.take() {
            if let Some(c) = state.cursor.take() {
                c.destroy();
            }
            p.release();
            state.focus = None;
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_pointer::Event as E;
        match event {
            E::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                state.focus = state.part_of(&surface).map(|p| (p, serial));
                if let Some((part, _)) = state.focus {
                    if let Some(dev) = &state.cursor
                        && let Some(p) = state.part(part)
                    {
                        dev.set_shape(serial, p.cursor);
                    }
                    let (x, y) = (state.physical(surface_x), state.physical(surface_y));
                    state.events.push(Event::Motion { part, x, y });
                }
            }
            E::Leave { .. } => {
                if let Some((part, _)) = state.focus.take() {
                    state.events.push(Event::Leave { part });
                }
            }
            E::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if let Some((part, _)) = state.focus {
                    let (x, y) = (state.physical(surface_x), state.physical(surface_y));
                    state.events.push(Event::Motion { part, x, y });
                }
            }
            E::Button {
                button,
                state: pressed,
                ..
            } => {
                let Some((part, _)) = state.focus else { return };
                // Linux input codes BTN_LEFT, BTN_RIGHT, BTN_MIDDLE.
                let button = match button {
                    0x110 => MouseButton::Left,
                    0x111 => MouseButton::Right,
                    0x112 => MouseButton::Middle,
                    b => MouseButton::Other(b as u16),
                };
                let pressed = pressed == WEnum::Value(wl_pointer::ButtonState::Pressed);
                state.events.push(Event::Button {
                    part,
                    button,
                    pressed,
                });
            }
            E::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } => {
                if let Some((part, _)) = state.focus {
                    // About 10 units per wheel notch; Wayland's positive is down.
                    let lines = -(value / 10.0) as f32;
                    state.events.push(Event::Wheel { part, lines });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, (usize, usize)> for State {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(usize, usize),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            for p in state.parts.iter_mut().flatten() {
                if let Some(b) = p.buffers.iter_mut().find(|b| b.buffer == *buffer) {
                    b.busy = false;
                }
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, (usize, i32, i32)> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        &(part, x, y): &(usize, i32, i32),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.events.push(Event::Moved { part, x, y });
        }
    }
}

delegate_noop!(State: wl_compositor::WlCompositor);
delegate_noop!(State: wl_subcompositor::WlSubcompositor);
delegate_noop!(State: wl_subsurface::WlSubsurface);
delegate_noop!(State: wl_shm_pool::WlShmPool);
delegate_noop!(State: wl_region::WlRegion);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: wp_viewporter::WpViewporter);
delegate_noop!(State: wp_viewport::WpViewport);
delegate_noop!(State: wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
delegate_noop!(State: wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);
