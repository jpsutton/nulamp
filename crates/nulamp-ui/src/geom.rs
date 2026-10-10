//! Screen geometry.

/// A rectangle in screen pixels. `right()` and `bottom()` are exclusive edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    /// Horizontal ranges overlap (inclusive of touching edges).
    pub fn overlaps_x(&self, o: &Rect) -> bool {
        self.x <= o.right() && o.x <= self.right()
    }

    /// Vertical ranges overlap (inclusive of touching edges).
    pub fn overlaps_y(&self, o: &Rect) -> bool {
        self.y <= o.bottom() && o.y <= self.bottom()
    }

    /// Horizontal ranges share at least one pixel column (corners alone do not count).
    pub fn shares_x(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right()
    }

    /// Vertical ranges share at least one pixel row (corners alone do not count).
    pub fn shares_y(&self, o: &Rect) -> bool {
        self.y < o.bottom() && o.y < self.bottom()
    }
}

/// Inclusive hit box `x1..=x2 × y1..=y2` (01 conventions).
pub fn hit(x: i32, y: i32, x1: i32, x2: i32, y1: i32, y2: i32) -> bool {
    x >= x1 && x <= x2 && y >= y1 && y <= y2
}
