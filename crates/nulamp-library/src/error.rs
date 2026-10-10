//! Library errors.

/// Errors from library providers.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The provider does not support this operation (see [`crate::Capabilities`]).
    #[error("operation not supported by this provider")]
    Unsupported,
    /// A track, node or playlist does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// An argument is out of range or belongs to another provider.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// A scan is already running.
    #[error("a scan is already running")]
    Busy,
    /// SQLite error.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// The database was written by a newer build.
    #[error("database schema version {found} is newer than supported version {supported}")]
    SchemaTooNew { found: u32, supported: u32 },
    /// File system error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Tag reading or writing failed.
    #[error("tag error: {0}")]
    Meta(#[from] nulamp_meta::Error),
    /// The platform file watcher failed or is unavailable.
    #[error("file watcher error: {0}")]
    Watch(String),
    /// Provider-specific failure (for example a network error in a remote provider).
    #[error("{0}")]
    Provider(String),
}

/// Result of library operations.
pub type Result<T> = std::result::Result<T, Error>;
