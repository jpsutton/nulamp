//! Media-library user interface (decision D4, `docs/media-library.md`): the worker thread
//! that calls the providers, the window state, the window and play counting.

pub mod clock;
pub mod service;
pub mod tracker;
pub mod view;
pub mod window;

pub use service::{LibraryService, ListSpec, MediaLibrary, Purpose, Request, Response};
pub use tracker::{FinishedPlay, Heard, PlayTracker};
pub use view::LibraryView;
pub use window::{Geometry, LibAction, LibraryWindow};

/// Commands of the library menus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibCmd {
    /// Act on the selected rows.
    Selection(Purpose),
    /// Act on every row of the listing.
    All(Purpose),
    /// Act on the selected rows, or on every row when none is selected (the Play button).
    Auto(Purpose),
    SelectAll,
    Rescan,
}
