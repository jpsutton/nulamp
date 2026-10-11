//! The media-library worker. Provider calls may block (disk, network), so the user interface
//! sends [`Request`]s to one worker thread and polls [`Response`]s on its tick. Provider
//! change events are forwarded to the same response channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use nulamp_library::{
    BrowseNode, Capabilities, LibraryEvent, LibraryProvider, LocalLibrary, Page, PlaybackLocation,
    PlaylistInfo, Query, Registry, ScanMode, Sort, Track, TrackId, TrackPage, TrackSummary,
    WatchFolder, jellyfin,
};

/// The library as handed to the shell by the application.
#[derive(Clone)]
pub struct MediaLibrary {
    /// Shared with the player, which resolves `library://` locations through it.
    pub registry: Arc<Registry>,
    /// The local provider, for watch folders, scans and mapping files to tracks.
    pub local: Option<Arc<LocalLibrary>>,
}

impl MediaLibrary {
    /// Replaces the Jellyfin provider with one for `account` (none for `None`, or when the
    /// account shows no library).
    pub fn set_jellyfin(&self, account: Option<&jellyfin::Account>) {
        for p in self.registry.providers() {
            if p.id().starts_with(jellyfin::PROVIDER_PREFIX) {
                self.registry.unregister(p.id());
            }
        }
        if let Some(a) = account.filter(|a| a.shown_libraries().next().is_some()) {
            self.registry
                .register(Arc::new(nulamp_library::JellyfinProvider::new(a.clone())));
        }
    }
}

/// A track listing: provider, node, search text and order.
#[derive(Debug, Clone, PartialEq)]
pub struct ListSpec {
    pub provider: String,
    pub node: BrowseNode,
    pub query: String,
    pub sort: Sort,
}

/// What collected tracks are for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    /// Replace the playlist and play.
    Play,
    /// Append to the playlist.
    Enqueue,
    /// Append and put at the front of the play queue.
    PlayNext,
    /// Insert into the playlist at a row (drag and drop).
    Insert(usize),
    Rate(u8),
    Remove,
    FileInfo,
    ShowInFolder,
    /// Add to a stored playlist of the tracks' provider; `None` asks for a new one.
    AddToPlaylist(Option<String>),
}

#[derive(Debug, Clone)]
pub enum Request {
    Browse {
        provider: String,
        node: BrowseNode,
    },
    Tracks {
        spec: ListSpec,
        page: Page,
        generation: u64,
    },
    Summary {
        spec: ListSpec,
        generation: u64,
    },
    /// Fetches the rows `start..end` of a listing and resolves their playback locations.
    Collect {
        spec: ListSpec,
        ranges: Vec<(usize, usize)>,
        purpose: Purpose,
    },
    SetRating {
        tracks: Vec<Track>,
        stars: u8,
    },
    Remove {
        ids: Vec<TrackId>,
    },
    Rescan {
        full: bool,
    },
    /// Adds the watch folders missing from the local library (and with `remove_others`
    /// removes the ones not listed), then scans when something changed.
    SyncFolders {
        folders: Vec<(String, bool)>,
        remove_others: bool,
    },
    SetWatching(bool),
    /// Playback of a file ended: record a play when it is a library track.
    FinishPlay {
        location: String,
        played_ms: u64,
        length_ms: u64,
    },
    Playlists,
    AddToPlaylist {
        provider: String,
        /// `None` creates a playlist called `name`.
        playlist: Option<String>,
        name: String,
        ids: Vec<TrackId>,
    },
    /// The tags of a file were written outside the library.
    FileChanged(String),
}

#[derive(Debug, Clone)]
pub enum Response {
    Children {
        provider: String,
        parent: String,
        result: Result<Vec<BrowseNode>, String>,
    },
    Page {
        generation: u64,
        offset: u64,
        result: Result<TrackPage, String>,
    },
    Summary {
        generation: u64,
        summary: TrackSummary,
    },
    Collected {
        purpose: Purpose,
        items: Vec<(Track, PlaybackLocation)>,
    },
    Event {
        provider: String,
        event: LibraryEvent,
    },
    /// Stored playlists of every provider that has them.
    Playlists(Vec<(String, PlaylistInfo)>),
    /// The local library's watch folders after a change.
    Folders(Vec<WatchFolder>),
    Error(String),
}

/// Handle of the running worker.
pub struct LibraryService {
    tx: Sender<Request>,
    rx: Receiver<Response>,
    generation: Arc<AtomicU64>,
    /// (id, name, capabilities) of every provider, in registration order.
    pub providers: Vec<(String, String, Capabilities)>,
}

impl LibraryService {
    pub fn start(lib: MediaLibrary) -> Self {
        let (tx, worker_rx) = mpsc::channel::<Request>();
        let (worker_tx, rx) = mpsc::channel::<Response>();
        let generation = Arc::new(AtomicU64::new(0));
        let providers = lib
            .registry
            .providers()
            .iter()
            .map(|p| (p.id().to_string(), p.name().to_string(), p.capabilities()))
            .collect();
        for p in lib.registry.providers() {
            let events = p.subscribe();
            let (out, id) = (worker_tx.clone(), p.id().to_string());
            let _ = std::thread::Builder::new()
                .name("nulamp-library-events".into())
                .spawn(move || {
                    for event in events {
                        let r = Response::Event {
                            provider: id.clone(),
                            event,
                        };
                        if out.send(r).is_err() {
                            break;
                        }
                    }
                });
        }
        let worker = Worker {
            lib,
            out: worker_tx,
            generation: generation.clone(),
        };
        let _ = std::thread::Builder::new()
            .name("nulamp-library".into())
            .spawn(move || {
                for r in worker_rx {
                    worker.handle(r);
                }
            });
        Self {
            tx,
            rx,
            generation,
            providers,
        }
    }

    pub fn send(&self, r: Request) {
        let _ = self.tx.send(r);
    }

    /// Requests for older listing generations are skipped by the worker.
    pub fn set_generation(&self, g: u64) {
        self.generation.store(g, Ordering::Relaxed);
    }

    /// Every response that arrived.
    pub fn poll(&self) -> Vec<Response> {
        self.rx.try_iter().collect()
    }

    /// Waits for one response (tests).
    pub fn recv_timeout(&self, d: std::time::Duration) -> Option<Response> {
        self.rx.recv_timeout(d).ok()
    }

    pub fn capabilities(&self, provider: &str) -> Capabilities {
        self.providers
            .iter()
            .find(|p| p.0 == provider)
            .map(|p| p.2)
            .unwrap_or_default()
    }
}

struct Worker {
    lib: MediaLibrary,
    out: Sender<Response>,
    generation: Arc<AtomicU64>,
}

/// Rows fetched per provider call while collecting.
const COLLECT_CHUNK: usize = 1000;

impl Worker {
    fn send(&self, r: Response) {
        let _ = self.out.send(r);
    }

    fn provider(&self, id: &str) -> Option<Arc<dyn LibraryProvider>> {
        self.lib.registry.get(id)
    }

    fn stale(&self, generation: u64) -> bool {
        generation < self.generation.load(Ordering::Relaxed)
    }

    fn error(&self, what: &str, e: impl std::fmt::Display) {
        log::warn!("library: {what}: {e}");
        self.send(Response::Error(format!("{what}: {e}")));
    }

    fn handle(&self, r: Request) {
        match r {
            Request::Browse { provider, node } => {
                let Some(p) = self.provider(&provider) else {
                    return;
                };
                let result = p.browse(&node).map_err(|e| e.to_string());
                self.send(Response::Children {
                    provider,
                    parent: node.id,
                    result,
                });
            }
            Request::Tracks {
                spec,
                page,
                generation,
            } => {
                if self.stale(generation) {
                    return;
                }
                let Some(p) = self.provider(&spec.provider) else {
                    return;
                };
                let q = Query::parse(&spec.query);
                let result = p
                    .tracks(&spec.node, &q, spec.sort, page)
                    .map_err(|e| e.to_string());
                self.send(Response::Page {
                    generation,
                    offset: page.offset,
                    result,
                });
            }
            Request::Summary { spec, generation } => {
                if self.stale(generation) {
                    return;
                }
                let Some(p) = self.provider(&spec.provider) else {
                    return;
                };
                match p.summary(&spec.node, &Query::parse(&spec.query)) {
                    Ok(summary) => self.send(Response::Summary {
                        generation,
                        summary,
                    }),
                    Err(e) => self.error("summary", e),
                }
            }
            Request::Collect {
                spec,
                ranges,
                purpose,
            } => self.collect(&spec, &ranges, purpose),
            Request::SetRating { tracks, stars } => {
                for t in &tracks {
                    let Ok(p) = self.lib.registry.provider_for(&t.id) else {
                        continue;
                    };
                    if let Err(e) = p.set_rating(&t.id, stars) {
                        self.error("rating", e);
                        continue;
                    }
                    // Ratings kept only in the library are also written to the file's tag.
                    if p.capabilities().contains(Capabilities::RATING_DB_ONLY)
                        && !nulamp_core::playlist::is_url(&t.location)
                    {
                        let value = (stars > 0).then(|| stars.to_string());
                        if let Err(e) = nulamp_meta::write(&t.location, &[("rating".into(), value)])
                        {
                            log::warn!("cannot write the rating of {}: {e}", t.location);
                        }
                    }
                }
            }
            Request::Remove { ids } => {
                let mut by_provider: Vec<(String, Vec<TrackId>)> = Vec::new();
                for id in ids {
                    match by_provider.iter_mut().find(|(p, _)| *p == id.provider) {
                        Some((_, v)) => v.push(id),
                        None => by_provider.push((id.provider.clone(), vec![id])),
                    }
                }
                for (provider, ids) in by_provider {
                    if let Some(p) = self.provider(&provider)
                        && let Err(e) = p.remove_tracks(&ids)
                    {
                        self.error("remove", e);
                    }
                }
            }
            Request::Rescan { full } => {
                if let Some(local) = &self.lib.local {
                    let mode = if full {
                        ScanMode::Full
                    } else {
                        ScanMode::Incremental
                    };
                    match local.scan(mode) {
                        // A running scan already covers the request.
                        Ok(_) | Err(nulamp_library::Error::Busy) => {}
                        Err(e) => self.error("scan", e),
                    }
                }
                for p in self.lib.registry.providers() {
                    if p.id() != LocalLibrary::ID
                        && p.capabilities().contains(Capabilities::SCAN)
                        && let Err(e) = p.rescan()
                    {
                        self.error("rescan", e);
                    }
                }
            }
            Request::SyncFolders {
                folders,
                remove_others,
            } => self.sync_folders(&folders, remove_others),
            Request::SetWatching(on) => {
                if let Some(local) = &self.lib.local
                    && let Err(e) = local.set_watching(on)
                {
                    self.error("watching folders", e);
                }
            }
            Request::FinishPlay {
                location,
                played_ms,
                length_ms,
            } => {
                // A remote track by its library location, else a local file.
                let id = match TrackId::from_location(&location) {
                    Some(id) => id,
                    None => {
                        let Some(local) = &self.lib.local else {
                            return;
                        };
                        let Ok(Some(id)) = local.track_id_for_path(&location) else {
                            return;
                        };
                        id
                    }
                };
                let length = match length_ms {
                    0 => self
                        .lib
                        .registry
                        .provider_for(&id)
                        .ok()
                        .and_then(|p| p.track(&id).ok().flatten())
                        .map(|t| t.length_ms)
                        .unwrap_or(0),
                    l => l,
                };
                match self.lib.registry.finish_play(&id, played_ms, length) {
                    Ok(counted) => log::debug!("play of {location}: counted {counted}"),
                    Err(e) => self.error("recording a play", e),
                }
            }
            Request::Playlists => {
                let mut all = Vec::new();
                for p in self.lib.registry.providers() {
                    if p.capabilities().contains(Capabilities::PLAYLISTS)
                        && let Ok(list) = p.playlists()
                    {
                        all.extend(list.into_iter().map(|i| (p.id().to_string(), i)));
                    }
                }
                self.send(Response::Playlists(all));
            }
            Request::AddToPlaylist {
                provider,
                playlist,
                name,
                ids,
            } => {
                if let Err(e) = self.add_to_playlist(&provider, playlist, &name, ids) {
                    self.error("adding to the playlist", e);
                }
            }
            Request::FileChanged(location) => {
                if let Some(local) = &self.lib.local
                    && !local.is_watching()
                    && matches!(local.track_id_for_path(&location), Ok(Some(_)))
                {
                    // Re-read changed files; a running scan already covers it.
                    let _ = local.scan(ScanMode::Incremental);
                }
            }
        }
    }

    fn collect(&self, spec: &ListSpec, ranges: &[(usize, usize)], purpose: Purpose) {
        let Some(p) = self.provider(&spec.provider) else {
            return;
        };
        let q = Query::parse(&spec.query);
        let remote = p.capabilities().contains(Capabilities::REMOTE);
        let mut items = Vec::new();
        for &(start, end) in ranges {
            let mut at = start;
            while at < end {
                let n = (end - at).min(COLLECT_CHUNK);
                let page = Page::new(at as u64, n as u64);
                let tracks = match p.tracks(&spec.node, &q, spec.sort, page) {
                    Ok(page) => page.tracks,
                    Err(e) => {
                        self.error("listing tracks", e);
                        return;
                    }
                };
                if tracks.is_empty() {
                    break;
                }
                at += tracks.len();
                for t in tracks {
                    // Remote tracks go into the playlist by their library location, resolved
                    // when they play.
                    if remote {
                        let loc = PlaybackLocation {
                            location: t.id.location(),
                            headers: Vec::new(),
                        };
                        items.push((t, loc));
                        continue;
                    }
                    match p.resolve_playback(&t.id) {
                        Ok(loc) => items.push((t, loc)),
                        Err(e) => log::warn!("cannot resolve {}: {e}", t.location),
                    }
                }
            }
        }
        self.send(Response::Collected { purpose, items });
    }

    fn sync_folders(&self, folders: &[(String, bool)], remove_others: bool) {
        let Some(local) = &self.lib.local else {
            return;
        };
        let current = local.folders().unwrap_or_default();
        let key = |p: &str| {
            std::path::absolute(p)
                .map(|a| {
                    a.to_string_lossy()
                        .trim_end_matches(std::path::MAIN_SEPARATOR)
                        .to_string()
                })
                .unwrap_or_else(|_| p.to_string())
        };
        let mut changed = false;
        for (path, recursive) in folders {
            let k = key(path);
            if !current
                .iter()
                .any(|f| f.path == k && f.recursive == *recursive)
            {
                match local.add_folder(path, *recursive) {
                    Ok(()) => changed = true,
                    Err(e) => self.error("adding a watch folder", e),
                }
            }
        }
        if remove_others {
            for f in &current {
                if !folders.iter().any(|(p, _)| key(p) == f.path) {
                    match local.remove_folder(&f.path) {
                        Ok(_) => changed = true,
                        Err(e) => self.error("removing a watch folder", e),
                    }
                }
            }
        }
        if changed && let Err(e) = local.scan(ScanMode::Incremental) {
            log::info!("library scan not started: {e}");
        }
        self.send(Response::Folders(local.folders().unwrap_or_default()));
    }

    fn add_to_playlist(
        &self,
        provider: &str,
        playlist: Option<String>,
        name: &str,
        ids: Vec<TrackId>,
    ) -> nulamp_library::Result<()> {
        let Some(p) = self.provider(provider) else {
            return Ok(());
        };
        let id = match playlist {
            Some(id) => id,
            None => p.create_playlist(name)?.id,
        };
        let node = p.playlist_node(&id)?;
        let mut all: Vec<TrackId> = p
            .tracks(&node, &Query::all(), node.default_sort, Page::all())?
            .tracks
            .into_iter()
            .map(|t| t.id)
            .collect();
        all.extend(ids.into_iter().filter(|t| t.provider == provider));
        p.set_playlist_tracks(&id, &all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nulamp_library::MockProvider;
    use std::time::Duration;

    fn service() -> LibraryService {
        let registry = Registry::new();
        registry.register(Arc::new(MockProvider::sample()));
        LibraryService::start(MediaLibrary {
            registry: Arc::new(registry),
            local: None,
        })
    }

    fn next(s: &LibraryService) -> Response {
        s.recv_timeout(Duration::from_secs(5)).expect("response")
    }

    #[test]
    fn browse_page_and_collect() {
        let s = service();
        assert_eq!(s.providers[0].0, "mock");
        s.send(Request::Browse {
            provider: "mock".into(),
            node: BrowseNode::root(),
        });
        match next(&s) {
            Response::Children {
                provider, result, ..
            } => {
                assert_eq!(provider, "mock");
                assert!(!result.unwrap().is_empty());
            }
            other => panic!("{other:?}"),
        }
        let spec = ListSpec {
            provider: "mock".into(),
            node: BrowseNode::root(),
            query: String::new(),
            sort: Sort::default(),
        };
        s.send(Request::Tracks {
            spec: spec.clone(),
            page: Page::new(0, 2),
            generation: 3,
        });
        let total = match next(&s) {
            Response::Page {
                generation: 3,
                result: Ok(page),
                ..
            } => {
                assert_eq!(page.tracks.len(), 2);
                page.total
            }
            other => panic!("{other:?}"),
        };
        // Older generations are skipped.
        s.set_generation(5);
        s.send(Request::Tracks {
            spec: spec.clone(),
            page: Page::new(0, 2),
            generation: 4,
        });
        s.send(Request::Summary {
            spec: spec.clone(),
            generation: 5,
        });
        match next(&s) {
            Response::Summary {
                generation: 5,
                summary,
            } => assert_eq!(summary.count, total),
            other => panic!("{other:?}"),
        }
        // An open-ended range reads to the end of the listing.
        s.send(Request::Collect {
            spec: spec.clone(),
            ranges: vec![(0, usize::MAX)],
            purpose: Purpose::Play,
        });
        match next(&s) {
            Response::Collected { items, .. } => assert_eq!(items.len() as u64, total),
            other => panic!("{other:?}"),
        }
        s.send(Request::Collect {
            spec,
            ranges: vec![(0, 1), (2, 3)],
            purpose: Purpose::Enqueue,
        });
        match next(&s) {
            Response::Collected { purpose, items } => {
                assert_eq!(purpose, Purpose::Enqueue);
                assert_eq!(items.len(), 2);
                // Remote tracks are queued by library location, resolved when they play.
                assert_eq!(items[0].1.location, items[0].0.id.location());
                assert!(items[0].1.location.starts_with("library://mock/"));
                assert!(items[0].1.headers.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn plays_of_library_locations_are_recorded() {
        let mock = Arc::new(MockProvider::sample());
        let registry = Registry::new();
        registry.register(mock.clone());
        let s = LibraryService::start(MediaLibrary {
            registry: Arc::new(registry),
            local: None,
        });
        let id = TrackId::new("mock", "2");
        s.send(Request::FinishPlay {
            location: id.location(),
            played_ms: 1000,
            length_ms: 0,
        });
        s.send(Request::FinishPlay {
            location: id.location(),
            played_ms: 120_000,
            length_ms: 0,
        });
        // Wait for the worker: the mock announces the recorded play.
        match next(&s) {
            Response::Event { event, .. } => assert_eq!(event, LibraryEvent::TracksChanged),
            other => panic!("{other:?}"),
        }
        assert_eq!(mock.track(&id).unwrap().unwrap().play_count, 1);
    }

    #[test]
    fn jellyfin_provider_follows_the_account() {
        let lib = MediaLibrary {
            registry: Arc::new(Registry::new()),
            local: None,
        };
        let mut account = jellyfin::Account {
            server_url: "http://127.0.0.1:9".into(),
            user_id: "u1".into(),
            token: "t".into(),
            libraries: vec![jellyfin::Library {
                id: "l".into(),
                name: "Music".into(),
                collection_type: jellyfin::MUSIC.into(),
                enabled: true,
            }],
            ..Default::default()
        };
        lib.set_jellyfin(Some(&account));
        let ids = |l: &MediaLibrary| {
            l.registry
                .providers()
                .iter()
                .map(|p| p.id().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&lib), ["jellyfin-u1"]);
        account.user_id = "u2".into();
        lib.set_jellyfin(Some(&account));
        assert_eq!(ids(&lib), ["jellyfin-u2"], "replaced, not added");
        account.libraries[0].enabled = false;
        lib.set_jellyfin(Some(&account));
        assert!(ids(&lib).is_empty(), "no library shown");
        account.libraries[0].enabled = true;
        lib.set_jellyfin(Some(&account));
        lib.set_jellyfin(None);
        assert!(ids(&lib).is_empty());
    }
}
