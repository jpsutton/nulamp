//! Single instance, command line, remote control and OS integration (spec 11 §4–§6).
//!
//! - [`parse_args`] reads the command line (11 §4.2) into an [`Invocation`].
//! - [`connect_or_serve`] makes the process the primary instance ([`Server`]) or connects it to
//!   the running one ([`Client`]) (11 §4.1). Both speak the line-based JSON protocol of
//!   [`Request`] and [`Response`] over a local socket.
//! - [`Mpris`] exports the MPRIS 2 media-player interface on the session bus (Linux and BSD).
//!   Its commands arrive on the same channel as the socket's.
//! - [`hotkey`] holds the global hotkey binding model (11 §5).
//!
//! The application drains `(Request, ReplySender)` pairs on its user-interface thread; a waker
//! callback set with [`RequestSender::set_waker`] tells the event loop that requests are waiting.
//!
//! Choices where the spec is silent are recorded as `RC1`… in `docs/decisions.md`.

pub mod args;
pub mod coalesce;
pub mod hotkey;
pub mod instance;
pub mod mpris;
pub mod numbered;
pub mod request;
pub mod screensaver;
pub mod snapshot;

pub use args::{Invocation, StartMode, parse_args};
pub use coalesce::{OpenCoalescer, OpenPlan};
pub use hotkey::{Assignable, Hotkey, HotkeySettings};
pub use instance::{
    Client, Incoming, ReplySender, RequestSender, Role, Server, channel, connect_or_serve,
};
pub use mpris::Mpris;
pub use numbered::NumberedCommand;
pub use request::{EqValues, Flag, Request, Response, State, Status};
pub use snapshot::PlayerSnapshot;

/// Default single-instance class (11 §4.2 `/CLASS=`).
pub const DEFAULT_CLASS: &str = "default";

/// Version reported by [`Request::GetVersion`].
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Errors from the remote-control interfaces.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// A message could not be encoded or decoded.
    #[error("protocol error: {0}")]
    Protocol(#[from] serde_json::Error),
    /// The other side closed the connection without answering.
    #[error("connection closed")]
    Closed,
    /// The peer runs as another user.
    #[error("peer belongs to another user")]
    ForeignPeer,
    /// No local-socket name could be made for this platform.
    #[error("no usable socket name: {0}")]
    Name(String),
    /// D-Bus error (MPRIS).
    #[error("d-bus error: {0}")]
    Bus(String),
    /// The feature is not available on this platform.
    #[error("not supported on this platform")]
    Unsupported,
}

/// Result type of this crate.
pub type Result<T> = std::result::Result<T, Error>;
