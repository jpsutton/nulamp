//! Live updates from the platform file watcher (`notify`), debounced.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::model::LibraryEvent;

use super::db::{self, FileStat};
use super::scan::{self, ImageCache};
use super::{Shared, WatchFolder};

/// Changes are applied once no event arrived for this long…
const QUIET: Duration = Duration::from_millis(500);
/// …or at the latest this long after the first pending event.
const MAX_DELAY: Duration = Duration::from_secs(3);

/// A watch root and its canonical form (some platforms report canonical paths).
struct Root {
    folder: WatchFolder,
    canonical: Option<String>,
}

/// A running watcher; dropping it stops the watch thread.
pub(crate) struct Watch {
    _watcher: RecommendedWatcher,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    pub(crate) fn start(shared: Arc<Shared>, folders: Vec<WatchFolder>) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let werr = |e: notify::Error| Error::Watch(e.to_string());
        let mut watcher = notify::recommended_watcher(tx).map_err(werr)?;
        let mut roots = Vec::new();
        for folder in folders {
            let path = Path::new(&folder.path);
            if path.is_dir() {
                let mode = if folder.recursive {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                watcher.watch(path, mode).map_err(werr)?;
            }
            let canonical = fs::canonicalize(path)
                .ok()
                .and_then(|c| c.to_str().map(str::to_string))
                .filter(|c| *c != folder.path);
            roots.push(Root { folder, canonical });
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = thread::Builder::new()
            .name("library-watch".into())
            .spawn(move || run(&shared, &rx, &stop2, &roots))?;
        Ok(Self {
            _watcher: watcher,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn relevant(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

fn run(
    shared: &Shared,
    rx: &Receiver<notify::Result<notify::Event>>,
    stop: &AtomicBool,
    roots: &[Root],
) {
    let Ok(mut conn) = db::open_connection(&shared.db_path) else {
        return;
    };
    let mut pending = BTreeSet::new();
    let mut first: Option<Instant> = None;
    let mut last = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) if relevant(&event.kind) => {
                pending.extend(event.paths);
                first.get_or_insert_with(Instant::now);
                last = Instant::now();
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if let Some(f) = first
            && (last.elapsed() >= QUIET || f.elapsed() >= MAX_DELAY)
        {
            first = None;
            let paths = std::mem::take(&mut pending);
            if apply(shared, &mut conn, roots, paths).unwrap_or(false) {
                shared.events.send(LibraryEvent::TracksChanged);
            }
        }
    }
}

/// `path` in the form the roots were added with.
fn map_path(path: &Path, roots: &[Root]) -> Option<String> {
    let s = path.to_str()?;
    for r in roots {
        if s == r.folder.path || scan::covers(&r.folder.path, true, s) {
            return Some(s.to_string());
        }
        if let Some(c) = &r.canonical
            && let Some(rest) = s.strip_prefix(c.as_str())
            && (rest.is_empty() || rest.starts_with(MAIN_SEPARATOR))
        {
            return Some(format!("{}{rest}", r.folder.path));
        }
    }
    None
}

/// Re-reads changed files and drops removed ones. Returns whether the index changed.
fn apply(
    shared: &Shared,
    conn: &mut Connection,
    roots: &[Root],
    paths: BTreeSet<PathBuf>,
) -> Result<bool> {
    let never = AtomicBool::new(false);
    let mut files: HashMap<String, FileStat> = HashMap::new();
    let mut gone = Vec::new();
    for p in paths {
        let Some(s) = map_path(&p, roots) else {
            continue;
        };
        let is_root = roots.iter().any(|r| r.folder.path == s);
        let covering: Vec<&WatchFolder> = roots
            .iter()
            .map(|r| &r.folder)
            .filter(|f| scan::covers(&f.path, f.recursive, &s))
            .collect();
        if !is_root && covering.is_empty() {
            continue;
        }
        let path = Path::new(&s);
        match fs::metadata(path) {
            Ok(md) if md.is_dir() => {
                if let Some(r) = roots.iter().find(|r| r.folder.path == s) {
                    scan::walk(shared, path, r.folder.recursive, &never, &mut files);
                } else if covering.iter().any(|f| f.recursive) {
                    scan::walk(shared, path, true, &never, &mut files);
                }
            }
            Ok(md) if md.is_file() && !is_root => {
                if scan::accepts(shared, path)
                    && let Some(stat) = FileStat::of(path, &md)
                {
                    files.insert(s, stat);
                }
            }
            Ok(_) => {}
            Err(_) => gone.push(s),
        }
    }

    let mut changed = Vec::new();
    for f in files.into_values() {
        if db::stored_stat(conn, &f.path)? != Some((f.size, f.mtime)) {
            changed.push(f);
        }
    }
    let mut images = ImageCache::default();
    let (added, updated, _, _) = scan::index_files(conn, &changed, &never, &mut images, |_| {})?;

    let mut removed = 0;
    if !gone.is_empty() {
        let tx = conn.transaction()?;
        for g in &gone {
            let lo = format!("{g}{MAIN_SEPARATOR}");
            let hi = format!("{g}{}", char::from(MAIN_SEPARATOR as u8 + 1));
            let ids: Vec<i64> = tx
                .prepare("SELECT id FROM tracks WHERE path = ? OR (path >= ? AND path < ?)")?
                .query_map([g, &lo, &hi], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for id in ids {
                db::delete_track(&tx, id)?;
                removed += 1;
            }
        }
        tx.commit()?;
    }
    Ok(added + updated + removed > 0)
}
