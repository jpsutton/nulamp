//! The local provider: watch folders indexed into SQLite.
//!
//! [`LocalLibrary::open`] creates or migrates the database (WAL mode, versioned migrations
//! in the `meta` table). Folders are added with [`LocalLibrary::add_folder`] and indexed by
//! [`LocalLibrary::scan`] on a background thread; [`LocalLibrary::set_watching`] keeps the
//! index current while files change. See `docs/media-library.md`.

mod browse;
mod db;
mod scan;
mod watch;

use std::fs;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use rusqlite::{Connection, OptionalExtension};

use crate::error::{Error, Result};
use crate::model::{
    BrowseNode, Capabilities, LibraryEvent, Page, PlaybackLocation, PlaylistInfo, ScanSummary,
    Track, TrackId, TrackPage, TrackSummary,
};
use crate::provider::{LibraryProvider, Subscribers};
use crate::query::{Query, Sort};

use db::{FileStat, Scanned};

/// Caller-supplied extension filter: gets the lower-case extension without the dot (empty
/// when the file has none) and returns whether the file is audio.
pub type ExtensionFilter = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// A watch folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchFolder {
    /// Absolute path, without a trailing separator.
    pub path: String,
    /// Whether subfolders are included.
    pub recursive: bool,
}

/// What a scan does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    /// Reads new files and files whose size or modification time changed; drops files that
    /// are gone (except under watch folders that are offline).
    Incremental,
    /// Like `Incremental` but re-reads every file.
    Full,
    /// Only drops tracks whose file no longer exists, wherever they are.
    RemoveMissing,
}

/// State shared with the scan and watch threads.
pub(crate) struct Shared {
    pub(crate) db_path: PathBuf,
    pub(crate) filter: ExtensionFilter,
    pub(crate) events: Subscribers,
    scanning: AtomicBool,
    cancel: Mutex<Arc<AtomicBool>>,
}

/// A running scan. Dropping the handle lets the scan finish in the background.
pub struct ScanHandle {
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<ScanSummary>>>,
}

impl ScanHandle {
    /// Asks the scan to stop after the current batch. Work already committed stays.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Waits for the scan and returns its summary.
    pub fn wait(mut self) -> Result<ScanSummary> {
        match self.thread.take() {
            Some(t) => t
                .join()
                .unwrap_or_else(|_| Err(Error::Provider("scan thread panicked".into()))),
            None => Ok(ScanSummary::default()),
        }
    }
}

/// The local SQLite library.
pub struct LocalLibrary {
    shared: Arc<Shared>,
    conn: Mutex<Connection>,
    watch: Mutex<Option<watch::Watch>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn load_folders(conn: &Connection) -> Result<Vec<WatchFolder>> {
    let mut st = conn.prepare("SELECT path, recursive FROM folders ORDER BY path")?;
    let rows = st.query_map([], |r| {
        Ok(WatchFolder {
            path: r.get(0)?,
            recursive: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Absolute form of a folder path, without a trailing separator.
fn folder_key(path: &Path) -> Result<String> {
    let abs = std::path::absolute(path)?;
    let s = abs
        .to_str()
        .ok_or_else(|| Error::InvalidArgument("folder path is not valid UTF-8".into()))?;
    let trimmed = s.trim_end_matches(MAIN_SEPARATOR);
    Ok(if trimmed.is_empty() {
        s.to_string()
    } else {
        trimmed.to_string()
    })
}

impl LocalLibrary {
    /// Provider id, used in [`TrackId::provider`].
    pub const ID: &'static str = "local";

    /// Opens (creating or migrating) the database at `db_path`. `filter` selects the files
    /// a scan indexes (see [`ExtensionFilter`]).
    pub fn open(
        db_path: impl AsRef<Path>,
        filter: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let mut conn = db::open_connection(&db_path)?;
        db::migrate(&mut conn)?;
        Ok(Self {
            shared: Arc::new(Shared {
                db_path,
                filter: Arc::new(filter),
                events: Subscribers::default(),
                scanning: AtomicBool::new(false),
                cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            }),
            conn: Mutex::new(conn),
            watch: Mutex::new(None),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        lock(&self.conn)
    }

    /// The schema version of the open database.
    pub fn schema_version(&self) -> Result<u32> {
        db::schema_version(&self.conn())
    }

    /// The schema version this build writes.
    pub const fn supported_schema_version() -> u32 {
        db::SCHEMA_VERSION
    }

    pub fn folders(&self) -> Result<Vec<WatchFolder>> {
        load_folders(&self.conn())
    }

    /// Adds a watch folder (or changes its `recursive` flag). Run [`LocalLibrary::scan`]
    /// to index it.
    pub fn add_folder(&self, path: impl AsRef<Path>, recursive: bool) -> Result<()> {
        let key = folder_key(path.as_ref())?;
        self.conn().execute(
            "INSERT INTO folders (path, recursive, added) VALUES (?, ?, ?)
             ON CONFLICT(path) DO UPDATE SET recursive = excluded.recursive",
            rusqlite::params![key, recursive, crate::now_secs()],
        )?;
        self.restart_watch()
    }

    /// Removes a watch folder and the tracks no remaining folder covers. Returns the number
    /// of tracks removed.
    pub fn remove_folder(&self, path: impl AsRef<Path>) -> Result<u64> {
        let key = folder_key(path.as_ref())?;
        let removed = {
            let mut conn = self.conn();
            if conn.execute("DELETE FROM folders WHERE path = ?", [&key])? == 0 {
                return Err(Error::NotFound(format!("watch folder `{key}`")));
            }
            let folders = load_folders(&conn)?;
            let tx = conn.transaction()?;
            let rows: Vec<(i64, String)> = tx
                .prepare("SELECT id, path FROM tracks")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut removed = 0;
            for (id, p) in rows {
                if !folders
                    .iter()
                    .any(|f| scan::covers(&f.path, f.recursive, &p))
                {
                    db::delete_track(&tx, id)?;
                    removed += 1;
                }
            }
            tx.commit()?;
            removed
        };
        if removed > 0 {
            self.shared.events.send(LibraryEvent::TracksChanged);
        }
        self.restart_watch()?;
        Ok(removed)
    }

    /// Starts a scan on a background thread. Progress and the result arrive as
    /// [`LibraryEvent`]s; the handle can also wait for or cancel it. Fails with
    /// [`Error::Busy`] while another scan runs.
    pub fn scan(&self, mode: ScanMode) -> Result<ScanHandle> {
        if self.shared.scanning.swap(true, Ordering::SeqCst) {
            return Err(Error::Busy);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *lock(&self.shared.cancel) = cancel.clone();
        let shared = self.shared.clone();
        let flag = cancel.clone();
        let spawned = thread::Builder::new()
            .name("library-scan".into())
            .spawn(move || {
                let result = scan::run(&shared, mode, &flag);
                let summary = match &result {
                    Ok(s) => s.clone(),
                    Err(e) => ScanSummary {
                        error: Some(e.to_string()),
                        ..ScanSummary::default()
                    },
                };
                shared.scanning.store(false, Ordering::SeqCst);
                shared.events.send(LibraryEvent::ScanFinished(summary));
                result
            });
        match spawned {
            Ok(t) => Ok(ScanHandle {
                cancel,
                thread: Some(t),
            }),
            Err(e) => {
                self.shared.scanning.store(false, Ordering::SeqCst);
                Err(e.into())
            }
        }
    }

    pub fn is_scanning(&self) -> bool {
        self.shared.scanning.load(Ordering::SeqCst)
    }

    /// Cancels the running scan, if any.
    pub fn cancel_scan(&self) {
        lock(&self.shared.cancel).store(true, Ordering::Relaxed);
    }

    /// Turns the live file watcher on or off. Fails with [`Error::Watch`] when the platform
    /// watcher is unavailable.
    pub fn set_watching(&self, on: bool) -> Result<()> {
        let mut w = lock(&self.watch);
        *w = None;
        if on {
            *w = Some(watch::Watch::start(self.shared.clone(), self.folders()?)?);
        }
        Ok(())
    }

    pub fn is_watching(&self) -> bool {
        lock(&self.watch).is_some()
    }

    fn restart_watch(&self) -> Result<()> {
        if self.is_watching() {
            self.set_watching(true)?;
        }
        Ok(())
    }

    /// Number of indexed tracks.
    pub fn track_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// The id of the track indexed at `path` (for example to record a play of a playlist
    /// entry).
    pub fn track_id_for_path(&self, path: impl AsRef<Path>) -> Result<Option<TrackId>> {
        let Some(p) = path.as_ref().to_str() else {
            return Ok(None);
        };
        let id: Option<i64> = self
            .conn()
            .query_row("SELECT id FROM tracks WHERE path = ?", [p], |r| r.get(0))
            .optional()?;
        Ok(id.map(|id| TrackId::new(Self::ID, id.to_string())))
    }

    fn row_id(id: &TrackId) -> Result<i64> {
        if id.provider != Self::ID {
            return Err(Error::InvalidArgument(format!(
                "track of provider `{}` given to `{}`",
                id.provider,
                Self::ID
            )));
        }
        id.id
            .parse()
            .map_err(|_| Error::InvalidArgument(format!("bad track id `{}`", id.id)))
    }

    fn playlist_id(id: &str) -> Result<i64> {
        id.parse()
            .map_err(|_| Error::NotFound(format!("playlist `{id}`")))
    }

    fn track_path(&self, id: &TrackId) -> Result<(i64, String)> {
        let rid = Self::row_id(id)?;
        let path: Option<String> = self
            .conn()
            .query_row("SELECT path FROM tracks WHERE id = ?", [rid], |r| r.get(0))
            .optional()?;
        path.map(|p| (rid, p))
            .ok_or_else(|| Error::NotFound(format!("track {}", id.id)))
    }

    fn notify(&self, event: LibraryEvent) {
        self.shared.events.send(event);
    }
}

impl Drop for LocalLibrary {
    fn drop(&mut self) {
        self.cancel_scan();
        *lock(&self.watch) = None;
    }
}

fn mime_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    }
}

impl LibraryProvider for LocalLibrary {
    fn id(&self) -> &str {
        Self::ID
    }

    fn name(&self) -> &str {
        "Local library"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::SCAN
            | Capabilities::WRITE_RATING
            | Capabilities::RECORD_PLAY
            | Capabilities::EDIT_TAGS
            | Capabilities::PLAYLISTS
            | Capabilities::ARTWORK
            | Capabilities::RATING_DB_ONLY
            | Capabilities::REMOVE
    }

    fn browse(&self, node: &BrowseNode) -> Result<Vec<BrowseNode>> {
        browse::children(&self.conn(), node)
    }

    fn tracks(
        &self,
        node: &BrowseNode,
        query: &Query,
        sort: Sort,
        page: Page,
    ) -> Result<TrackPage> {
        browse::tracks(&self.conn(), node, query, sort, page)
    }

    fn summary(&self, node: &BrowseNode, query: &Query) -> Result<TrackSummary> {
        browse::summary(&self.conn(), node, query)
    }

    fn track(&self, id: &TrackId) -> Result<Option<Track>> {
        browse::track(&self.conn(), Self::row_id(id)?)
    }

    /// Deletes the index rows (and their playlist entries). The files stay; a later scan
    /// adds them again while they are under a watch folder.
    fn remove_tracks(&self, ids: &[TrackId]) -> Result<u64> {
        let rows = ids.iter().map(Self::row_id).collect::<Result<Vec<_>>>()?;
        let removed = {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            let mut removed = 0;
            for rid in rows {
                let exists: Option<i64> = tx
                    .query_row("SELECT id FROM tracks WHERE id = ?", [rid], |r| r.get(0))
                    .optional()?;
                if exists.is_some() {
                    db::delete_track(&tx, rid)?;
                    removed += 1;
                }
            }
            tx.commit()?;
            removed
        };
        if removed > 0 {
            self.notify(LibraryEvent::TracksChanged);
            self.notify(LibraryEvent::PlaylistsChanged);
        }
        Ok(removed)
    }

    fn resolve_playback(&self, id: &TrackId) -> Result<PlaybackLocation> {
        let (_, path) = self.track_path(id)?;
        Ok(PlaybackLocation {
            location: path,
            headers: Vec::new(),
        })
    }

    /// The embedded front cover (cached in the database per file modification time), else
    /// the folder image found while scanning.
    fn artwork(&self, id: &TrackId) -> Result<Option<(String, Vec<u8>)>> {
        let rid = Self::row_id(id)?;
        type Row = (String, i64, Option<String>);
        let row: Option<Row> = self
            .conn()
            .query_row(
                "SELECT path, mtime, artwork FROM tracks WHERE id = ?",
                [rid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (path, mtime, folder) = row.ok_or_else(|| Error::NotFound(format!("track {rid}")))?;
        type Cached = (i64, Option<String>, Option<Vec<u8>>);
        let cached: Option<Cached> = self
            .conn()
            .query_row(
                "SELECT mtime, mime, data FROM artwork WHERE track_id = ?",
                [rid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let embedded = match cached {
            Some((m, mime, data)) if m == mtime => mime.zip(data),
            _ => {
                let cover = nulamp_meta::read_cover(&path).ok().flatten();
                let (mime, data) = cover.map(|c| (c.mime, c.data)).unzip();
                self.conn().execute(
                    "INSERT OR REPLACE INTO artwork (track_id, mtime, mime, data)
                     VALUES (?, ?, ?, ?)",
                    rusqlite::params![rid, mtime, mime, data],
                )?;
                mime.zip(data)
            }
        };
        if embedded.is_some() {
            return Ok(embedded);
        }
        match folder {
            Some(f) => match fs::read(&f) {
                Ok(data) => Ok(Some((mime_for(&f).to_string(), data))),
                Err(_) => Ok(None),
            },
            None => Ok(None),
        }
    }

    fn rescan(&self) -> Result<()> {
        self.scan(ScanMode::Incremental).map(drop)
    }

    fn set_rating(&self, id: &TrackId, stars: u8) -> Result<()> {
        if stars > 5 {
            return Err(Error::InvalidArgument(format!("rating {stars} is not 0–5")));
        }
        let rid = Self::row_id(id)?;
        let n = self.conn().execute(
            "UPDATE tracks SET rating = ? WHERE id = ?",
            rusqlite::params![stars, rid],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("track {rid}")));
        }
        self.notify(LibraryEvent::TracksChanged);
        Ok(())
    }

    fn record_play(&self, id: &TrackId, _played_ms: u64) -> Result<()> {
        let rid = Self::row_id(id)?;
        let n = self.conn().execute(
            "UPDATE tracks SET play_count = play_count + 1, last_played = ? WHERE id = ?",
            rusqlite::params![crate::now_secs(), rid],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("track {rid}")));
        }
        self.notify(LibraryEvent::TracksChanged);
        Ok(())
    }

    /// Writes the tags to the file with `nulamp-meta`, then re-reads it into the index.
    fn edit_tags(&self, id: &TrackId, changes: &[(String, Option<String>)]) -> Result<()> {
        let (_, path) = self.track_path(id)?;
        nulamp_meta::write(&path, changes)?;
        let p = Path::new(&path);
        let stat = FileStat::of(p, &fs::metadata(p)?)
            .ok_or_else(|| Error::InvalidArgument("path is not valid UTF-8".into()))?;
        let folder = p.parent().and_then(scan::folder_image);
        let (scanned, _) = Scanned::read(stat, folder);
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            db::upsert(&tx, &scanned, crate::now_secs())?;
            tx.commit()?;
        }
        self.notify(LibraryEvent::TracksChanged);
        Ok(())
    }

    fn playlists(&self) -> Result<Vec<PlaylistInfo>> {
        browse::playlists(&self.conn())
    }

    fn playlist_node(&self, id: &str) -> Result<BrowseNode> {
        let pid = Self::playlist_id(id)?.to_string();
        browse::playlists(&self.conn())?
            .into_iter()
            .find(|p| p.id == pid)
            .map(browse::playlist_node)
            .ok_or_else(|| Error::NotFound(format!("playlist `{id}`")))
    }

    fn create_playlist(&self, name: &str) -> Result<PlaylistInfo> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::InvalidArgument("empty playlist name".into()));
        }
        let id = {
            let conn = self.conn();
            let now = crate::now_secs();
            conn.execute(
                "INSERT INTO playlists (name, created, modified) VALUES (?, ?, ?)",
                rusqlite::params![name, now, now],
            )?;
            conn.last_insert_rowid()
        };
        self.notify(LibraryEvent::PlaylistsChanged);
        Ok(PlaylistInfo {
            id: id.to_string(),
            name: name.to_string(),
            track_count: 0,
        })
    }

    fn rename_playlist(&self, id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::InvalidArgument("empty playlist name".into()));
        }
        let n = self.conn().execute(
            "UPDATE playlists SET name = ?, modified = ? WHERE id = ?",
            rusqlite::params![name, crate::now_secs(), Self::playlist_id(id)?],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("playlist `{id}`")));
        }
        self.notify(LibraryEvent::PlaylistsChanged);
        Ok(())
    }

    fn delete_playlist(&self, id: &str) -> Result<()> {
        let n = self.conn().execute(
            "DELETE FROM playlists WHERE id = ?",
            [Self::playlist_id(id)?],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("playlist `{id}`")));
        }
        self.notify(LibraryEvent::PlaylistsChanged);
        Ok(())
    }

    fn set_playlist_tracks(&self, id: &str, tracks: &[TrackId]) -> Result<()> {
        let pid = Self::playlist_id(id)?;
        let rows = tracks
            .iter()
            .map(Self::row_id)
            .collect::<Result<Vec<_>>>()?;
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            let exists: Option<i64> = tx
                .query_row("SELECT id FROM playlists WHERE id = ?", [pid], |r| r.get(0))
                .optional()?;
            if exists.is_none() {
                return Err(Error::NotFound(format!("playlist `{id}`")));
            }
            tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?", [pid])?;
            {
                let mut check = tx.prepare("SELECT 1 FROM tracks WHERE id = ?")?;
                let mut insert = tx.prepare(
                    "INSERT INTO playlist_items (playlist_id, position, track_id) VALUES (?, ?, ?)",
                )?;
                for (pos, rid) in rows.iter().enumerate() {
                    if !check.exists([rid])? {
                        return Err(Error::NotFound(format!("track {rid}")));
                    }
                    insert.execute(rusqlite::params![pid, pos as i64, rid])?;
                }
            }
            tx.execute(
                "UPDATE playlists SET modified = ? WHERE id = ?",
                rusqlite::params![crate::now_secs(), pid],
            )?;
            tx.commit()?;
        }
        self.notify(LibraryEvent::PlaylistsChanged);
        Ok(())
    }

    fn subscribe(&self) -> Receiver<LibraryEvent> {
        self.shared.events.subscribe()
    }
}
