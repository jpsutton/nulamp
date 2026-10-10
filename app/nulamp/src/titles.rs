//! Background reading of tags for playlist titles and lengths (04 §10, 10 §5).

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, channel};

use nulamp_core::{Engine, EntryId};
use nulamp_tf::FieldSource;

/// The result of reading one entry.
struct TitleResult {
    id: EntryId,
    title: Option<String>,
    length_ms: Option<u64>,
}

pub struct TitleReader {
    tx: Sender<(EntryId, String)>,
    rx: Receiver<TitleResult>,
    pending: HashSet<EntryId>,
    /// Show `_` as space; show `%20` as space (10 §5.4).
    pub fix_underscore: bool,
    pub fix_percent20: bool,
}

/// The title for an item from its tags (10 §5.1): title formatting, else `Artist - Title`,
/// else `None` (the caller keeps the file-name title).
pub fn title_from_tags(
    meta: &nulamp_meta::Metadata,
    format: &nulamp_tf::Formatter,
) -> Option<String> {
    let has_tags = meta.get("title").is_some() || meta.get("artist").is_some();
    if has_tags {
        let t = format.format(meta);
        if !t.trim().is_empty() {
            return Some(t);
        }
    }
    match (meta.field("artist"), meta.field("title")) {
        (Some(a), Some(t)) => Some(format!("{a} - {t}")),
        (None, Some(t)) => Some(t),
        _ => None,
    }
}

impl TitleReader {
    pub fn new(title_format: &str) -> Self {
        let (tx, req_rx) = channel::<(EntryId, String)>();
        let (res_tx, rx) = channel();
        let format = title_format.to_string();
        std::thread::Builder::new()
            .name("nulamp-titles".into())
            .spawn(move || {
                let formatter = nulamp_tf::Formatter::new(&format);
                while let Ok((id, location)) = req_rx.recv() {
                    let r = match nulamp_meta::read(&location) {
                        Ok(meta) => TitleResult {
                            id,
                            title: title_from_tags(&meta, &formatter),
                            length_ms: meta
                                .get("length")
                                .and_then(|l| l.parse().ok())
                                .filter(|l: &u64| *l > 0),
                        },
                        Err(_) => TitleResult {
                            id,
                            title: None,
                            length_ms: None,
                        },
                    };
                    if res_tx.send(r).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn title reader");
        Self {
            tx,
            rx,
            pending: HashSet::new(),
            fix_underscore: true,
            fix_percent20: true,
        }
    }

    /// Requests a (re-)read of one entry.
    pub fn request(&mut self, id: EntryId, location: &str) {
        if nulamp_core::playlist::is_url(location) || !self.pending.insert(id) {
            return;
        }
        let _ = self.tx.send((id, location.to_string()));
    }

    /// Applies finished reads and queues unread entries (background mode, 04 §10).
    pub fn tick(&mut self, engine: &mut Engine) -> bool {
        let mut changed = false;
        while let Ok(r) = self.rx.try_recv() {
            self.pending.remove(&r.id);
            if let Some(e) = engine.playlist.by_id_mut(r.id) {
                if let Some(t) = r.title {
                    e.title = nulamp_core::playlist::clean_title(
                        &t,
                        self.fix_underscore,
                        self.fix_percent20,
                    );
                }
                if r.length_ms.is_some() {
                    e.length_ms = r.length_ms;
                }
                e.metadata_read = true;
                changed = true;
            }
        }
        if self.pending.len() < 16 {
            let todo: Vec<(EntryId, String)> = engine
                .playlist
                .entries()
                .iter()
                .filter(|e| {
                    !e.metadata_read
                        && !self.pending.contains(&e.id)
                        && !nulamp_core::playlist::is_url(&e.location)
                })
                .take(32)
                .map(|e| (e.id, e.location.clone()))
                .collect();
            for (id, loc) in todo {
                self.request(id, &loc);
            }
        }
        changed
    }
}

/// Reads the ReplayGain values of a file (07 §5.3).
pub fn replaygain_tags(location: &str) -> nulamp_eq::ReplayGainTags {
    let mut tags = nulamp_eq::ReplayGainTags::default();
    if let Ok(meta) = nulamp_meta::read(location) {
        for k in [
            "replaygain_track_gain",
            "replaygain_track_peak",
            "replaygain_album_gain",
            "replaygain_album_peak",
        ] {
            if let Some(v) = meta.get(k) {
                tags.set_tag(k, &v);
            }
        }
    }
    tags
}

/// Album identity for "gapless for albums" (07 §6.4, decision XF4): the album tag together with
/// the album artist, or with the folder when there is no album artist. `None` without an album
/// tag (and for streams).
pub fn album_key(location: &str) -> Option<String> {
    let meta = nulamp_meta::read(location).ok()?;
    let album = meta.get("album")?.trim().to_lowercase();
    if album.is_empty() {
        return None;
    }
    let by = match meta.get("albumartist") {
        Some(a) if !a.trim().is_empty() => format!("artist:{}", a.trim().to_lowercase()),
        _ => {
            let dir = std::path::Path::new(location).parent()?;
            format!("dir:{}", dir.to_string_lossy())
        }
    };
    Some(format!("{album}\n{by}"))
}
