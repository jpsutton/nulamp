//! Folder walking and the background scan.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;

use crate::error::Result;
use crate::model::{LibraryEvent, ScanSummary};

use super::db::{self, FileStat, Scanned};
use super::{ScanMode, Shared, WatchFolder};

/// Files read per transaction.
const BATCH: usize = 64;

/// Folder image names, in order of preference (decision ML7).
const IMAGE_NAMES: [&str; 3] = ["cover", "folder", "front"];
const IMAGE_EXTS: [&str; 3] = ["jpg", "jpeg", "png"];

/// True when `path` lies under `root`: anywhere below it when `recursive`, else directly in it.
pub(crate) fn covers(root: &str, recursive: bool, path: &str) -> bool {
    let Some(rest) = path.strip_prefix(root) else {
        return false;
    };
    let rest = if root.ends_with(MAIN_SEPARATOR) {
        rest
    } else {
        match rest.strip_prefix(MAIN_SEPARATOR) {
            Some(r) => r,
            None => return false,
        }
    };
    !rest.is_empty() && (recursive || !rest.contains(MAIN_SEPARATOR))
}

/// Whether the caller's extension filter accepts `path` (lower-case extension, no dot;
/// empty when the file has none).
pub(crate) fn accepts(shared: &Shared, path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    (shared.filter)(&ext)
}

/// Adds every accepted file below `root` to `out`. Symbolic links are followed; each
/// directory is visited once. Stops early when `cancel` is set.
pub(crate) fn walk(
    shared: &Shared,
    root: &Path,
    recursive: bool,
    cancel: &AtomicBool,
    out: &mut HashMap<String, FileStat>,
) {
    let mut visited = HashSet::new();
    if let Ok(c) = fs::canonicalize(root) {
        visited.insert(c);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(md) = fs::metadata(&path) else {
                continue;
            };
            if md.is_dir() {
                if recursive && fs::canonicalize(&path).is_ok_and(|c| visited.insert(c)) {
                    stack.push(path);
                }
            } else if md.is_file()
                && accepts(shared, &path)
                && let Some(stat) = FileStat::of(&path, &md)
            {
                out.insert(stat.path.clone(), stat);
            }
        }
    }
}

/// The preferred cover image file in `dir`, if any.
pub(crate) fn folder_image(dir: &Path) -> Option<String> {
    let names: HashMap<String, PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().to_lowercase(), e.path()))
        .collect();
    IMAGE_NAMES
        .iter()
        .flat_map(|n| IMAGE_EXTS.iter().map(move |e| format!("{n}.{e}")))
        .find_map(|n| names.get(&n).and_then(|p| p.to_str()).map(str::to_string))
}

/// Folder images looked up once per directory.
#[derive(Default)]
pub(crate) struct ImageCache(HashMap<PathBuf, Option<String>>);

impl ImageCache {
    pub(crate) fn get(&mut self, file: &str) -> Option<String> {
        let dir = Path::new(file).parent()?.to_path_buf();
        self.0
            .entry(dir)
            .or_insert_with_key(|d| folder_image(d))
            .clone()
    }
}

/// Reads `files` and writes them in batches of [`BATCH`] per transaction.
/// Calls `progress(done)` after each batch; returns (added, updated, unreadable, cancelled).
pub(crate) fn index_files(
    conn: &mut Connection,
    files: &[FileStat],
    cancel: &AtomicBool,
    images: &mut ImageCache,
    mut progress: impl FnMut(u64),
) -> Result<(u64, u64, u64, bool)> {
    let (mut added, mut updated, mut unreadable) = (0, 0, 0);
    let mut done = 0u64;
    for chunk in files.chunks(BATCH) {
        if cancel.load(Ordering::Relaxed) {
            return Ok((added, updated, unreadable, true));
        }
        let read: Vec<Scanned> = chunk
            .iter()
            .map(|f| {
                let (s, ok) = Scanned::read(f.clone(), images.get(&f.path));
                if !ok {
                    unreadable += 1;
                }
                s
            })
            .collect();
        let now = crate::now_secs();
        let tx = conn.transaction()?;
        for s in &read {
            if db::upsert(&tx, s, now)?.1 {
                added += 1;
            } else {
                updated += 1;
            }
        }
        tx.commit()?;
        done += chunk.len() as u64;
        progress(done);
    }
    Ok((added, updated, unreadable, false))
}

/// Deletes the tracks `ids` in batches. Returns the number deleted.
fn delete_ids(conn: &mut Connection, ids: &[i64]) -> Result<u64> {
    for chunk in ids.chunks(512) {
        let tx = conn.transaction()?;
        for id in chunk {
            db::delete_track(&tx, *id)?;
        }
        tx.commit()?;
    }
    Ok(ids.len() as u64)
}

/// Runs one scan on the calling thread (the scan thread).
pub(crate) fn run(shared: &Shared, mode: ScanMode, cancel: &AtomicBool) -> Result<ScanSummary> {
    let mut conn = db::open_connection(&shared.db_path)?;
    let mut summary = ScanSummary::default();
    let existing: Vec<(i64, String, i64, i64)> = conn
        .prepare("SELECT id, path, size, mtime FROM tracks")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;

    if mode == ScanMode::RemoveMissing {
        let total = existing.len() as u64;
        shared
            .events
            .send(LibraryEvent::ScanProgress { done: 0, total });
        let mut missing = Vec::new();
        for (i, (id, path, _, _)) in existing.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                summary.cancelled = true;
                break;
            }
            if !Path::new(path).is_file() {
                missing.push(*id);
            } else {
                summary.unchanged += 1;
            }
            if i % 256 == 255 {
                let done = i as u64 + 1;
                shared
                    .events
                    .send(LibraryEvent::ScanProgress { done, total });
            }
        }
        summary.removed = delete_ids(&mut conn, &missing)?;
        if summary.removed > 0 {
            shared.events.send(LibraryEvent::TracksChanged);
        }
        return Ok(summary);
    }

    let folders = super::load_folders(&conn)?;
    let mut seen = HashMap::new();
    let mut offline: Vec<&WatchFolder> = Vec::new();
    for f in &folders {
        let root = Path::new(&f.path);
        if !root.is_dir() {
            offline.push(f);
            continue;
        }
        walk(shared, root, f.recursive, cancel, &mut seen);
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            return Ok(summary);
        }
    }

    let known: HashMap<&str, (i64, i64)> = existing
        .iter()
        .map(|(_, p, size, mtime)| (p.as_str(), (*size, *mtime)))
        .collect();
    let mut work: Vec<FileStat> = seen
        .values()
        .filter(|f| {
            mode == ScanMode::Full || known.get(f.path.as_str()) != Some(&(f.size, f.mtime))
        })
        .cloned()
        .collect();
    work.sort_by(|a, b| a.path.cmp(&b.path));
    summary.unchanged = (seen.len() - work.len()) as u64;

    let total = work.len() as u64;
    shared
        .events
        .send(LibraryEvent::ScanProgress { done: 0, total });
    let mut images = ImageCache::default();
    let (added, updated, unreadable, cancelled) =
        index_files(&mut conn, &work, cancel, &mut images, |done| {
            shared
                .events
                .send(LibraryEvent::ScanProgress { done, total });
            shared.events.send(LibraryEvent::TracksChanged);
        })?;
    summary.added = added;
    summary.updated = updated;
    summary.unreadable = unreadable;
    if cancelled {
        summary.cancelled = true;
        return Ok(summary);
    }

    // Files no longer found. Tracks under an offline root (unmounted drive) are kept until
    // a "remove missing" pass (decision ML6).
    let gone: Vec<i64> = existing
        .iter()
        .filter(|(_, p, _, _)| !seen.contains_key(p.as_str()))
        .filter(|(_, p, _, _)| !offline.iter().any(|f| covers(&f.path, f.recursive, p)))
        .map(|(id, _, _, _)| *id)
        .collect();
    summary.removed = delete_ids(&mut conn, &gone)?;
    if summary.removed > 0 {
        shared.events.send(LibraryEvent::TracksChanged);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage() {
        let s = MAIN_SEPARATOR;
        let root = format!("{s}m");
        assert!(covers(&root, true, &format!("{s}m{s}a{s}b.mp3")));
        assert!(!covers(&root, false, &format!("{s}m{s}a{s}b.mp3")));
        assert!(covers(&root, false, &format!("{s}m{s}b.mp3")));
        assert!(!covers(&root, true, &format!("{s}music{s}b.mp3")));
        assert!(!covers(&root, true, &root));
        assert!(covers(&s.to_string(), false, &format!("{s}b.mp3")));
    }
}
