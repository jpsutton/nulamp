//! Native Wayland compound window: not available on this platform.

use nulamp_render::Frame;
use nulamp_render::region::Mask;
use nulamp_skin::CursorShape;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::event::MouseButton;

pub const MAIN: usize = 0;

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
    Wheel {
        part: usize,
        lines: f32,
    },
    Moved {
        part: usize,
        x: i32,
        y: i32,
    },
}

pub enum Compound {}

impl Compound {
    pub fn new(_: &impl HasDisplayHandle, _: &impl HasWindowHandle) -> Option<Self> {
        None
    }

    pub fn add_part(&mut self) -> usize {
        match *self {}
    }

    pub fn add_part_on(&mut self, _: &impl HasWindowHandle) -> Option<usize> {
        match *self {}
    }

    pub fn remove_part(&mut self, _: usize) {
        match *self {}
    }

    pub fn set_scale(&mut self, _: f64) {
        match *self {}
    }

    pub fn set_position(&mut self, _: usize, _: i32, _: i32) {
        match *self {}
    }

    pub fn set_shape(&mut self, _: usize, _: Option<Mask>) {
        match *self {}
    }

    pub fn present(&mut self, _: usize, _: &Frame) -> bool {
        match *self {}
    }

    pub fn hide(&mut self, _: usize) {
        match *self {}
    }

    pub fn set_cursor(&mut self, _: usize, _: CursorShape) {
        match *self {}
    }

    pub fn dispatch(&mut self) -> Vec<Event> {
        match *self {}
    }
}
