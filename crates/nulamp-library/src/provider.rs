//! The provider trait and the registry of providers.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::error::{Error, Result};
use crate::model::{
    AlbumFacet, ArtistFacet, BrowseNode, Capabilities, LibraryEvent, Page, PlaybackLocation,
    PlaylistInfo, Track, TrackId, TrackPage, TrackSummary,
};
use crate::query::{Query, Sort};

/// A source of tracks: the local SQLite library, or a remote server (Jellyfin, Subsonic, …).
///
/// Every method takes `&self`; implementations synchronise internally and may block (the UI
/// calls them from a worker thread). Writes are optional: their default implementations
/// return [`Error::Unsupported`], and [`LibraryProvider::capabilities`] tells the UI which
/// ones work.
pub trait LibraryProvider: Send + Sync {
    /// Stable id, used in [`TrackId::provider`] (for example `local`).
    fn id(&self) -> &str;

    /// Display name.
    fn name(&self) -> &str;

    fn capabilities(&self) -> Capabilities;

    /// Children of `node`; [`BrowseNode::root`] gives the top level.
    fn browse(&self, node: &BrowseNode) -> Result<Vec<BrowseNode>>;

    /// Tracks below `node` that match `query`, in `sort` order.
    fn tracks(&self, node: &BrowseNode, query: &Query, sort: Sort, page: Page)
    -> Result<TrackPage>;

    /// Tracks of the whole library that match `query`.
    fn search(&self, query: &Query, sort: Sort, page: Page) -> Result<TrackPage> {
        self.tracks(&BrowseNode::root(), query, sort, page)
    }

    /// Number and total length of the tracks below `node` that match `query` (for a status
    /// line). The default lists every matching track; providers override it with a cheaper
    /// query.
    fn summary(&self, node: &BrowseNode, query: &Query) -> Result<TrackSummary> {
        let page = self.tracks(node, query, Sort::default(), Page::all())?;
        Ok(TrackSummary {
            count: page.total,
            length_ms: page.tracks.iter().map(|t| t.length_ms).sum(),
        })
    }

    /// The artists of the tracks below `node` that match `query`, sorted by name (untagged
    /// last), for the artist pane. The default lists every matching track; providers
    /// override it with a cheaper query.
    fn artist_facets(&self, node: &BrowseNode, query: &Query) -> Result<Vec<ArtistFacet>> {
        let page = self.tracks(node, query, Sort::default(), Page::all())?;
        Ok(crate::facets::artists(&page.tracks))
    }

    /// The albums of the tracks below `node` that match `query`, sorted by name (untagged
    /// last), for the album pane.
    fn album_facets(&self, node: &BrowseNode, query: &Query) -> Result<Vec<AlbumFacet>> {
        let page = self.tracks(node, query, Sort::default(), Page::all())?;
        Ok(crate::facets::albums(&page.tracks))
    }

    /// One track, `None` when it no longer exists.
    fn track(&self, id: &TrackId) -> Result<Option<Track>>;

    /// What the player should open for `id`.
    fn resolve_playback(&self, id: &TrackId) -> Result<PlaybackLocation>;

    /// Cover image as (MIME type, bytes).
    fn artwork(&self, _id: &TrackId) -> Result<Option<(String, Vec<u8>)>> {
        Ok(None)
    }

    /// Starts re-indexing in the background ([`Capabilities::SCAN`]); progress arrives as
    /// events.
    fn rescan(&self) -> Result<()> {
        Err(Error::Unsupported)
    }

    /// Sets the rating, 0 (unrated) to 5 ([`Capabilities::WRITE_RATING`]).
    fn set_rating(&self, _id: &TrackId, _stars: u8) -> Result<()> {
        Err(Error::Unsupported)
    }

    /// Whether playing `played_ms` of a track `length_ms` long counts as a play
    /// ([`Registry::finish_play`]). The default is [`crate::should_count_play`]; a provider
    /// with videos asks for more of a film than of a song.
    fn counts_play(&self, _id: &TrackId, played_ms: u64, length_ms: u64) -> bool {
        crate::should_count_play(played_ms, length_ms)
    }

    /// Records one finished play ([`Capabilities::RECORD_PLAY`]). The caller decides whether
    /// a play counts, normally with [`LibraryProvider::counts_play`]; `played_ms` is passed on
    /// to providers that report listening time.
    fn record_play(&self, _id: &TrackId, _played_ms: u64) -> Result<()> {
        Err(Error::Unsupported)
    }

    /// Removes tracks from the library without touching their files
    /// ([`Capabilities::REMOVE`]). Returns how many were removed; unknown ids are skipped.
    fn remove_tracks(&self, _ids: &[TrackId]) -> Result<u64> {
        Err(Error::Unsupported)
    }

    /// Changes tag fields (spec 10 §7.1 keys; `None` deletes) ([`Capabilities::EDIT_TAGS`]).
    fn edit_tags(&self, _id: &TrackId, _changes: &[(String, Option<String>)]) -> Result<()> {
        Err(Error::Unsupported)
    }

    /// Stored playlists ([`Capabilities::PLAYLISTS`]).
    fn playlists(&self) -> Result<Vec<PlaylistInfo>> {
        Err(Error::Unsupported)
    }

    /// The browse node that lists a stored playlist's tracks (in playlist order).
    fn playlist_node(&self, _id: &str) -> Result<BrowseNode> {
        Err(Error::Unsupported)
    }

    fn create_playlist(&self, _name: &str) -> Result<PlaylistInfo> {
        Err(Error::Unsupported)
    }

    fn rename_playlist(&self, _id: &str, _name: &str) -> Result<()> {
        Err(Error::Unsupported)
    }

    fn delete_playlist(&self, _id: &str) -> Result<()> {
        Err(Error::Unsupported)
    }

    /// Replaces a playlist's contents with `tracks`, in order (duplicates allowed).
    fn set_playlist_tracks(&self, _id: &str, _tracks: &[TrackId]) -> Result<()> {
        Err(Error::Unsupported)
    }

    /// A new channel of change notifications. Dropping the receiver unsubscribes.
    fn subscribe(&self) -> Receiver<LibraryEvent>;
}

/// Fan-out of [`LibraryEvent`]s to every subscriber; for provider implementations.
#[derive(Debug, Default)]
pub struct Subscribers(Mutex<Vec<Sender<LibraryEvent>>>);

impl Subscribers {
    pub fn subscribe(&self) -> Receiver<LibraryEvent> {
        let (tx, rx) = mpsc::channel();
        self.lock().push(tx);
        rx
    }

    /// Sends `event` to every live subscriber and forgets the dropped ones.
    pub fn send(&self, event: LibraryEvent) {
        self.lock().retain(|tx| tx.send(event.clone()).is_ok());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Sender<LibraryEvent>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The providers known to the app, in registration order. Providers can be added and removed
/// while the registry is shared (for example when the user connects a server).
#[derive(Default)]
pub struct Registry {
    providers: RwLock<Vec<Arc<dyn LibraryProvider>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> RwLockReadGuard<'_, Vec<Arc<dyn LibraryProvider>>> {
        self.providers.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Vec<Arc<dyn LibraryProvider>>> {
        self.providers.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a provider, replacing one with the same id.
    pub fn register(&self, provider: Arc<dyn LibraryProvider>) {
        let mut providers = self.write();
        providers.retain(|p| p.id() != provider.id());
        providers.push(provider);
    }

    /// Removes and returns the provider with `id`.
    pub fn unregister(&self, id: &str) -> Option<Arc<dyn LibraryProvider>> {
        let mut providers = self.write();
        let i = providers.iter().position(|p| p.id() == id)?;
        Some(providers.remove(i))
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn LibraryProvider>> {
        self.read().iter().find(|p| p.id() == id).cloned()
    }

    /// The registered providers, in registration order.
    pub fn providers(&self) -> Vec<Arc<dyn LibraryProvider>> {
        self.read().clone()
    }

    /// The provider that owns `track`.
    pub fn provider_for(&self, track: &TrackId) -> Result<Arc<dyn LibraryProvider>> {
        self.get(&track.provider)
            .ok_or_else(|| Error::NotFound(format!("provider `{}`", track.provider)))
    }

    /// Resolves a track through its provider.
    pub fn resolve_playback(&self, track: &TrackId) -> Result<PlaybackLocation> {
        self.provider_for(track)?.resolve_playback(track)
    }

    /// Records a play through the track's provider when [`LibraryProvider::counts_play`]
    /// says it counts. Returns whether a play was recorded.
    pub fn finish_play(&self, track: &TrackId, played_ms: u64, length_ms: u64) -> Result<bool> {
        let provider = self.provider_for(track)?;
        if !provider.capabilities().contains(Capabilities::RECORD_PLAY)
            || !provider.counts_play(track, played_ms, length_ms)
        {
            return Ok(false);
        }
        provider.record_play(track, played_ms)?;
        Ok(true)
    }
}
