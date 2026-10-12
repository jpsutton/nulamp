//! A Jellyfin server as a library provider (`docs/media-library.md` §9).
//!
//! [`login`] signs in with a user name and password and returns an [`Account`]: the server
//! address, the access token and the server's libraries. The app stores the account (never
//! the password) and builds a [`JellyfinProvider`] from it. The provider shows the libraries
//! the user enabled: music, movies, TV shows, music videos and home videos ([`LibraryKind`]);
//! [`Library`] keeps the collection type of every library, shown or not.
//!
//! Listings without a search are paged and sorted by the server. A search, a sort the server
//! cannot do, a playlist and the root of several libraries are listed in full once, kept for a
//! few minutes, and filtered with [`Query::matches`] and sorted with [`Sort::compare`] in
//! memory, so search behaves as in the local library.
//!
//! Tracks are located by [`TrackId::location`] (`library://<provider>/<item>`, and
//! `library://<provider>/video:<item>` for videos); playback resolves that to the server's
//! universal audio URL, or a video's file as it is, with the token in the `Authorization`
//! header. Requires Jellyfin 10.9 or newer.

mod client;
mod dto;

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub use client::{authorization, new_device_id, normalize_url};

use crate::error::{Error, Result};
use crate::model::{
    BrowseNode, Capabilities, LibraryEvent, NodeKind, Page, PlaybackLocation, Track, TrackId,
    TrackPage, TrackSummary,
};
use crate::provider::{LibraryProvider, Subscribers};
use crate::query::{Query, Sort, SortKey};
use crate::{enc_segment, split_id};
use client::Client;
use dto::{AuthResult, Item, ItemList, PublicInfo, item_of};

/// Prefix of every Jellyfin provider id; the rest is the user's id on the server.
pub const PROVIDER_PREFIX: &str = "jellyfin-";

/// Collection type of music libraries.
pub const MUSIC: &str = "music";

/// What a library holds: it decides the library's browse tree and the items it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryKind {
    /// `music`: songs.
    Music,
    /// `movies`.
    Movies,
    /// `tvshows`: series, seasons and episodes.
    Shows,
    /// `musicvideos` and `homevideos`.
    Videos,
}

impl LibraryKind {
    /// The kind of a collection type; `None` for kinds the provider does not show (mixed
    /// folders, books, photos, …).
    pub fn of(collection_type: &str) -> Option<Self> {
        Some(match collection_type {
            MUSIC => Self::Music,
            "movies" => Self::Movies,
            "tvshows" => Self::Shows,
            "musicvideos" | "homevideos" => Self::Videos,
            _ => return None,
        })
    }

    pub fn is_video(self) -> bool {
        self != Self::Music
    }

    /// The `includeItemTypes` of the library's tracks.
    fn item_types(self) -> &'static str {
        match self {
            Self::Music => "Audio",
            Self::Movies => "Movie",
            Self::Shows => "Episode",
            Self::Videos => "Video,MusicVideo",
        }
    }
}

/// Share of a video, in percent, that must be played for it to count as watched (Jellyfin's
/// own default for marking an item played).
const WATCHED_PERCENT: u64 = 90;

/// Containers (with their audio codec where it matters) that nulamp's network input decodes
/// as they are; the server transcodes anything else to MP3. Files played as they are seek
/// exactly; a transcode seeks only in what has been downloaded (NET13).
const DIRECT_CONTAINERS: &str = "mp3,flac,ogg|vorbis,ogg|flac,ogg|opus,oga|vorbis,oga|flac,oga|opus,wav,aiff,aif,\
     m4a|aac,m4a|alac,mp4|aac,mp4|alac,m4b|aac,m4b|alac,aac,mka|vorbis,mka|flac,mka|aac,\
     mka|mp3,webm|vorbis,caf|alac,caf|aac";

/// Playlist entries fetched per request.
const CHUNK: u64 = 2000;

/// How long a full listing is reused.
const CACHE_TTL: Duration = Duration::from_secs(300);

/// Full listings kept.
const CACHE_SIZE: usize = 4;

/// Fields requested with every track.
const TRACK_FIELDS: &str = "Genres,DateCreated";

/// A library (a top-level "view") on the server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Library {
    pub id: String,
    pub name: String,
    /// `music`, `movies`, `tvshows`, … (empty for mixed folders).
    pub collection_type: String,
    /// Shown in the media library.
    pub enabled: bool,
}

impl Library {
    pub fn kind(&self) -> Option<LibraryKind> {
        LibraryKind::of(&self.collection_type)
    }

    pub fn is_music(&self) -> bool {
        self.kind() == Some(LibraryKind::Music)
    }

    pub fn is_video(&self) -> bool {
        self.kind().is_some_and(LibraryKind::is_video)
    }
}

/// A signed-in user on a server: everything the provider needs, without the password.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Account {
    /// Base URL without a trailing slash (`http://host:8096`).
    pub server_url: String,
    pub server_id: String,
    pub server_name: String,
    pub user_id: String,
    pub user_name: String,
    pub token: String,
    pub device_id: String,
    pub libraries: Vec<Library>,
}

/// Leaves the token out, so logs and panic messages never show it.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("server_url", &self.server_url)
            .field("server_id", &self.server_id)
            .field("server_name", &self.server_name)
            .field("user_id", &self.user_id)
            .field("user_name", &self.user_name)
            .field("device_id", &self.device_id)
            .field("libraries", &self.libraries)
            .finish_non_exhaustive()
    }
}

impl Account {
    /// The id of this account's provider.
    pub fn provider_id(&self) -> String {
        format!("{PROVIDER_PREFIX}{}", self.user_id)
    }

    /// The enabled libraries of a kind the provider shows.
    pub fn shown_libraries(&self) -> impl Iterator<Item = &Library> {
        self.libraries
            .iter()
            .filter(|l| l.enabled && l.kind().is_some())
    }

    fn client(&self) -> Client {
        Client::new(&self.server_url, &self.device_id, &self.token)
    }
}

/// Signs in to the server at `url` and lists the user's libraries (music libraries start
/// enabled, video libraries hidden). Blocks; call it off the user-interface thread.
pub fn login(url: &str, user: &str, password: &str) -> Result<Account> {
    let base = normalize_url(url)
        .ok_or_else(|| Error::InvalidArgument("no server address given".into()))?;
    let device_id = new_device_id();
    let anon = Client::new(&base, &device_id, "");
    let info: PublicInfo = anon
        .get_json("/System/Info/Public", &[])
        .map_err(|e| match e {
            Error::NotFound(_) => Error::Provider(format!("{base} is not a Jellyfin server")),
            e => e,
        })?;
    log::info!(
        "Jellyfin server {} version {}",
        info.server_name.as_deref().unwrap_or("?"),
        info.version.as_deref().unwrap_or("?")
    );
    let body = serde_json::json!({ "Username": user, "Pw": password });
    let (_, answer) = anon
        .post("/Users/AuthenticateByName", &[], Some(&body))
        .map_err(|e| match e {
            Error::Provider(m) if m.contains("refused") => {
                Error::Provider("wrong user name or password".into())
            }
            e => e,
        })?;
    let auth: AuthResult = serde_json::from_slice(&answer)
        .map_err(|e| Error::Provider(format!("unexpected login answer: {e}")))?;
    let (Some(u), Some(token)) = (auth.user, auth.access_token) else {
        return Err(Error::Provider("the server sent no access token".into()));
    };
    let mut account = Account {
        server_url: base,
        server_id: auth.server_id.or(info.id).unwrap_or_default(),
        server_name: info.server_name.unwrap_or_else(|| "Jellyfin".into()),
        user_name: u.name.unwrap_or_else(|| user.to_string()),
        user_id: u.id,
        token,
        device_id,
        libraries: Vec::new(),
    };
    match libraries(&account) {
        Ok(l) => account.libraries = l,
        Err(e) => {
            // Do not leave an unused session behind.
            let _ = logout(&account);
            return Err(e);
        }
    }
    Ok(account)
}

/// The account's libraries as the server lists them now. Libraries already in
/// `account.libraries` keep their `enabled` flag; new music libraries start enabled, new video
/// libraries hidden.
pub fn libraries(account: &Account) -> Result<Vec<Library>> {
    let views: ItemList = account
        .client()
        .get_json("/UserViews", &[("userId", account.user_id.clone())])?;
    Ok(views
        .into_items()
        .into_iter()
        .map(|v| {
            let collection_type = v.collection_type.clone().unwrap_or_default();
            let enabled = account
                .libraries
                .iter()
                .find(|l| l.id == v.id)
                .map_or(collection_type == MUSIC, |l| l.enabled);
            Library {
                name: v.name(),
                id: v.id,
                collection_type,
                enabled,
            }
        })
        .collect())
}

/// Ends the account's session on the server (the token stops working). Blocks.
pub fn logout(account: &Account) -> Result<()> {
    account.client().post("/Sessions/Logout", &[], None)?;
    Ok(())
}

/// Which tracks a browse node lists.
enum Scope {
    /// One items query (the parameters besides type, paging and sort).
    Items(Vec<(&'static str, String)>),
    /// Several items queries whose results are joined (the root over several libraries).
    Union(Vec<Vec<(&'static str, String)>>),
    /// A playlist, in its own order.
    Playlist(String),
    /// The next episode to watch of each series of a library, in the server's order.
    NextUp(String),
    /// No tracks (the playlist folder).
    Empty,
}

/// Server sort for a [`Sort`]: the `SortBy` list and the number of leading keys that follow
/// the direction (as [`Sort::compare`] does). `None` when the server cannot sort by the key.
/// Videos have no artist or album on the server: an episode's series and season stand in, as
/// in [`Item::to_track`].
fn server_sort(sort: Sort, video: bool) -> Option<(&'static [&'static str], usize)> {
    const TRACK: [&str; 3] = ["ParentIndexNumber", "IndexNumber", "SortName"];
    const SERIES: [&str; 4] = [
        "SeriesSortName",
        "ParentIndexNumber",
        "IndexNumber",
        "SortName",
    ];
    let keys: &'static [&'static str] = match sort.key {
        SortKey::Artist | SortKey::Album | SortKey::Position if video => &SERIES,
        SortKey::Title if video => &["SortName", "ProductionYear"],
        SortKey::Year if video => &["ProductionYear", "SortName"],
        SortKey::PlayCount if video => &["PlayCount", "SortName"],
        SortKey::Artist | SortKey::Position => &[
            "Artist",
            "Album",
            "ParentIndexNumber",
            "IndexNumber",
            "SortName",
        ],
        SortKey::Album => &[
            "Album",
            "Artist",
            "ParentIndexNumber",
            "IndexNumber",
            "SortName",
        ],
        SortKey::Title => &["SortName", "Artist"],
        SortKey::Track => &TRACK,
        SortKey::Year => &[
            "ProductionYear",
            "Artist",
            "Album",
            "ParentIndexNumber",
            "IndexNumber",
        ],
        SortKey::Length => &["Runtime", "SortName"],
        SortKey::PlayCount => &[
            "PlayCount",
            "Artist",
            "Album",
            "ParentIndexNumber",
            "IndexNumber",
        ],
        SortKey::LastPlayed => &["DatePlayed", "SortName"],
        SortKey::DateAdded => &["DateCreated", "SortName"],
        SortKey::Genre | SortKey::Rating | SortKey::Path => return None,
    };
    Some((keys, if sort.key == SortKey::Track { 2 } else { 1 }))
}

/// A full listing kept for searches and in-memory sorting.
struct Cached {
    node: String,
    at: Instant,
    tracks: Arc<Vec<Track>>,
}

/// The last filtered and sorted listing, so its pages are sliced rather than recomputed.
struct View {
    node: String,
    query: Query,
    sort: Sort,
    at: Instant,
    tracks: Arc<Vec<Track>>,
}

/// The tracks and videos of an account's enabled libraries.
pub struct JellyfinProvider {
    account: Account,
    id: String,
    name: String,
    client: Client,
    cache: Mutex<Vec<Cached>>,
    view: Mutex<Option<View>>,
    events: Subscribers,
}

impl JellyfinProvider {
    pub fn new(account: Account) -> Self {
        Self {
            id: account.provider_id(),
            name: format!("Jellyfin: {}", account.server_name),
            client: account.client(),
            account,
            cache: Mutex::new(Vec::new()),
            view: Mutex::new(None),
            events: Subscribers::default(),
        }
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    fn cache(&self) -> MutexGuard<'_, Vec<Cached>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn view(&self) -> MutexGuard<'_, Option<View>> {
        self.view.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Forgets every kept listing (the server's data changed).
    fn forget(&self) {
        self.cache().clear();
        *self.view() = None;
    }

    fn user(&self) -> (&'static str, String) {
        ("userId", self.account.user_id.clone())
    }

    fn check(&self, id: &TrackId) -> Result<()> {
        if id.provider == self.id {
            Ok(())
        } else {
            Err(Error::InvalidArgument(format!(
                "not a track of {}",
                self.name
            )))
        }
    }

    fn libraries(&self) -> Vec<&Library> {
        self.account.shown_libraries().collect()
    }

    /// The kind of the shown library `id` (music when unknown).
    fn kind(&self, id: &str) -> LibraryKind {
        self.account
            .shown_libraries()
            .find(|l| l.id == id)
            .and_then(Library::kind)
            .unwrap_or(LibraryKind::Music)
    }

    /// Whether a video library is shown (video playlists and playlist entries are then
    /// listed too).
    fn shows_video(&self) -> bool {
        self.account.shown_libraries().any(Library::is_video)
    }

    /// `parentId` and the item types of library `l`'s tracks.
    fn in_library(&self, l: &str) -> Vec<(&'static str, String)> {
        vec![
            ("parentId", l.to_string()),
            ("includeItemTypes", self.kind(l).item_types().to_string()),
        ]
    }

    /// The category nodes of a library.
    fn categories(lib: &Library) -> Vec<BrowseNode> {
        let l = enc_segment(&lib.id);
        let node = |id: &str, label: &str, kind, expandable| {
            BrowseNode::new(format!("{id}/{l}"), label, kind, expandable)
        };
        let favorites = || node("favorites", "Favorites", NodeKind::SmartList, false);
        let unwatched = || {
            node("unwatched", "Unwatched", NodeKind::SmartList, false)
                .with_sort(Sort::asc(SortKey::Artist))
        };
        let recent = || {
            node("recent", "Recently added", NodeKind::SmartList, false)
                .with_sort(Sort::desc(SortKey::DateAdded))
        };
        match lib.kind().unwrap_or(LibraryKind::Music) {
            LibraryKind::Music => vec![
                node("artists", "Artists", NodeKind::Category, true),
                node("albums", "Albums", NodeKind::Category, true),
                node("genres", "Genres", NodeKind::Category, true),
                favorites(),
                node("played", "Most played", NodeKind::SmartList, false)
                    .with_sort(Sort::desc(SortKey::PlayCount)),
            ],
            LibraryKind::Movies => vec![
                node("movies", "Movies", NodeKind::Category, false)
                    .with_sort(Sort::asc(SortKey::Title)),
                node("genres", "Genres", NodeKind::Category, true),
                favorites(),
                unwatched().with_sort(Sort::asc(SortKey::Title)),
                recent(),
            ],
            LibraryKind::Shows => vec![
                node("shows", "Shows", NodeKind::Category, true),
                node("nextup", "Next up", NodeKind::SmartList, false)
                    .with_sort(Sort::asc(SortKey::Position)),
                unwatched(),
                recent(),
            ],
            LibraryKind::Videos => vec![
                node("videos", "Videos", NodeKind::Category, false)
                    .with_sort(Sort::asc(SortKey::Title)),
                favorites(),
                unwatched().with_sort(Sort::asc(SortKey::Title)),
                recent(),
            ],
        }
    }

    fn album_node(item: &Item, detail: Option<String>) -> BrowseNode {
        let mut n = BrowseNode::new(
            format!("album/{}", enc_segment(&item.id)),
            item.name(),
            NodeKind::Album,
            false,
        )
        .with_detail(detail)
        .with_sort(Sort::asc(SortKey::Track));
        if let Some(c) = item.child_count.or(item.song_count) {
            n = n.with_count(c);
        }
        n
    }

    fn items(&self, path: &str, mut q: Vec<(&'static str, String)>) -> Result<Vec<Item>> {
        q.push(self.user());
        Ok(self.client.get_json::<ItemList>(path, &q)?.into_items())
    }

    /// The node's tracks and the parameters that select them.
    fn scope(&self, node: &BrowseNode) -> Result<Scope> {
        let seg = split_id(&node.id);
        let s: Vec<&str> = seg.iter().map(String::as_str).collect();
        let audio = |p: &str| {
            vec![
                ("parentId", p.to_string()),
                ("includeItemTypes", "Audio".to_string()),
            ]
        };
        let with = |l: &str, k: &'static str, v: &str| {
            let mut p = self.in_library(l);
            p.push((k, v.to_string()));
            p
        };
        Ok(match s[..] {
            [""] => {
                let libs = self.libraries();
                match libs.len() {
                    0 => Scope::Empty,
                    1 => Scope::Items(self.in_library(&libs[0].id)),
                    _ => Scope::Union(libs.iter().map(|l| self.in_library(&l.id)).collect()),
                }
            }
            [
                "lib" | "artists" | "albums" | "genres" | "movies" | "shows" | "videos" | "recent",
                l,
            ] => Scope::Items(self.in_library(l)),
            ["artist", l, a] => Scope::Items(with(l, "albumArtistIds", a)),
            ["album", b] => Scope::Items(audio(b)),
            ["genre", l, g] => Scope::Items(with(l, "genreIds", g)),
            ["favorites", l] => Scope::Items(with(l, "filters", "IsFavorite")),
            ["played", l] => Scope::Items(with(l, "filters", "IsPlayed")),
            ["unwatched", l] => Scope::Items(with(l, "filters", "IsUnplayed")),
            ["nextup", l] => Scope::NextUp(l.to_string()),
            ["series", _, p] | ["season", p] => Scope::Items(vec![
                ("parentId", p.to_string()),
                ("includeItemTypes", "Episode".to_string()),
            ]),
            ["playlists"] => Scope::Empty,
            ["playlist", p] => Scope::Playlist(p.to_string()),
            _ => return Err(Error::NotFound(format!("browse node `{}`", node.id))),
        })
    }

    /// Items of one query (its parameters name the item types), `page` of them in server
    /// order `sort`.
    fn query_page(
        &self,
        params: &[(&'static str, String)],
        sort: Option<Sort>,
        page: Page,
    ) -> Result<(Vec<Track>, u64)> {
        let video = is_video_query(params);
        let mut q = params.to_vec();
        q.extend([
            ("recursive", "true".to_string()),
            ("fields", TRACK_FIELDS.to_string()),
            ("enableImageTypes", "Primary".to_string()),
            ("imageTypeLimit", "1".to_string()),
            ("enableTotalRecordCount", "true".to_string()),
            ("startIndex", page.offset.to_string()),
        ]);
        if page.limit != u64::MAX {
            q.push(("limit", page.limit.to_string()));
        }
        if let Some((keys, primary)) = sort
            .and_then(|s| server_sort(s, video).map(|k| (s, k)))
            .map(|(s, (keys, primary))| {
                let dir = |i: usize| {
                    if s.descending && i < primary {
                        "Descending"
                    } else {
                        "Ascending"
                    }
                };
                (
                    keys.join(","),
                    (0..keys.len()).map(dir).collect::<Vec<_>>().join(","),
                )
            })
        {
            q.push(("sortBy", keys));
            q.push(("sortOrder", primary));
        }
        q.push(self.user());
        let list: ItemList = self.client.get_json("/Items", &q)?;
        let total = list.total_record_count;
        let tracks = list
            .into_items()
            .iter()
            .map(|i| i.to_track(&self.id))
            .collect();
        Ok((tracks, total))
    }

    /// Every track of a query, in one request: paging over sort keys with ties could skip or
    /// repeat tracks between requests.
    fn query_all(&self, params: &[(&'static str, String)]) -> Result<Vec<Track>> {
        Ok(self
            .query_page(params, Some(Sort::default()), Page::all())?
            .0)
    }

    /// Every track of a playlist, in playlist order.
    fn playlist_all(&self, id: &str) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        let mut offset = 0u64;
        loop {
            let q = vec![
                self.user(),
                ("fields", TRACK_FIELDS.to_string()),
                ("enableTotalRecordCount", "true".to_string()),
                ("startIndex", offset.to_string()),
                ("limit", CHUNK.to_string()),
            ];
            let list: ItemList = self
                .client
                .get_json(&format!("/Playlists/{}/Items", seg(id)), &q)?;
            let total = list.total_record_count;
            let items = list.into_items();
            offset += items.len() as u64;
            let done = (items.len() as u64) < CHUNK || offset >= total;
            let video = self.shows_video();
            out.extend(
                items
                    .iter()
                    .filter(|i| i.is_audio() || (video && i.is_video()))
                    .map(|i| i.to_track(&self.id)),
            );
            if done {
                return Ok(out);
            }
        }
    }

    /// The node's full listing, from the cache while it is fresh.
    fn all_tracks(&self, node: &BrowseNode, scope: &Scope) -> Result<Arc<Vec<Track>>> {
        if let Some(c) = self
            .cache()
            .iter()
            .find(|c| c.node == node.id && c.at.elapsed() < CACHE_TTL)
        {
            return Ok(c.tracks.clone());
        }
        let tracks = Arc::new(match scope {
            Scope::Items(p) => self.query_all(p)?,
            Scope::Union(list) => {
                let mut all = Vec::new();
                for p in list {
                    all.extend(self.query_all(p)?);
                }
                all
            }
            Scope::Playlist(id) => self.playlist_all(id)?,
            Scope::NextUp(l) => {
                let q = vec![
                    self.user(),
                    ("parentId", l.clone()),
                    ("fields", TRACK_FIELDS.to_string()),
                    ("enableImageTypes", "Primary".to_string()),
                    ("imageTypeLimit", "1".to_string()),
                    ("limit", CHUNK.to_string()),
                ];
                let list: ItemList = self.client.get_json("/Shows/NextUp", &q)?;
                list.into_items()
                    .iter()
                    .map(|i| i.to_track(&self.id))
                    .collect()
            }
            Scope::Empty => Vec::new(),
        });
        let mut cache = self.cache();
        cache.retain(|c| c.node != node.id && c.at.elapsed() < CACHE_TTL);
        if cache.len() >= CACHE_SIZE {
            cache.remove(0);
        }
        cache.push(Cached {
            node: node.id.clone(),
            at: Instant::now(),
            tracks: tracks.clone(),
        });
        Ok(tracks)
    }

    /// Matching tracks of a full listing, sorted.
    fn filtered(
        &self,
        node: &BrowseNode,
        scope: &Scope,
        query: &Query,
        sort: Sort,
    ) -> Result<Arc<Vec<Track>>> {
        if let Some(v) = self.view().as_ref()
            && v.node == node.id
            && v.query == *query
            && v.sort == sort
            && v.at.elapsed() < CACHE_TTL
        {
            return Ok(v.tracks.clone());
        }
        let all = self.all_tracks(node, scope)?;
        let mut list: Vec<Track> = all.iter().filter(|t| query.matches(t)).cloned().collect();
        let playlist_order =
            matches!(scope, Scope::Playlist(_) | Scope::NextUp(_)) && sort.key == SortKey::Position;
        if playlist_order {
            if sort.descending {
                list.reverse();
            }
        } else {
            list.sort_by(|a, b| sort.compare(a, b));
        }
        let list = Arc::new(list);
        *self.view() = Some(View {
            node: node.id.clone(),
            query: query.clone(),
            sort,
            at: Instant::now(),
            tracks: list.clone(),
        });
        Ok(list)
    }

    /// Whether the server can list this page itself.
    fn server_side(scope: &Scope, query: &Query, sort: Sort) -> bool {
        match scope {
            Scope::Items(p) => query.is_empty() && server_sort(sort, is_video_query(p)).is_some(),
            _ => false,
        }
    }

    fn fetch_item(&self, id: &TrackId) -> Result<Option<Item>> {
        self.check(id)?;
        let q = [
            self.user(),
            ("fields", format!("{TRACK_FIELDS},MediaSources")),
        ];
        match self
            .client
            .get_json(&format!("/Items/{}", seg(item_of(&id.id).0)), &q)
        {
            Ok(item) => Ok(Some(item)),
            Err(Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Whether a query lists videos (by its item types).
fn is_video_query(params: &[(&'static str, String)]) -> bool {
    params
        .iter()
        .any(|(k, v)| *k == "includeItemTypes" && v != "Audio")
}

/// A server id as one URL path segment.
fn seg(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

impl LibraryProvider for JellyfinProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::REMOTE | Capabilities::RECORD_PLAY | Capabilities::ARTWORK
    }

    fn browse(&self, node: &BrowseNode) -> Result<Vec<BrowseNode>> {
        let parts = split_id(&node.id);
        let s: Vec<&str> = parts.iter().map(String::as_str).collect();
        let named = |items: Vec<Item>, f: &dyn Fn(&Item) -> BrowseNode| -> Vec<BrowseNode> {
            items.iter().map(f).collect()
        };
        let by_name = ("sortBy", "SortName".to_string());
        Ok(match s[..] {
            [""] => {
                let libs = self.libraries();
                let mut out = match libs[..] {
                    [one] => Self::categories(one),
                    _ => libs
                        .iter()
                        .map(|l| {
                            BrowseNode::new(
                                format!("lib/{}", enc_segment(&l.id)),
                                l.name.clone(),
                                NodeKind::Category,
                                true,
                            )
                        })
                        .collect(),
                };
                out.push(BrowseNode::new(
                    "playlists",
                    "Playlists",
                    NodeKind::Category,
                    true,
                ));
                out
            }
            ["lib", l] => match self.libraries().into_iter().find(|x| x.id == l) {
                Some(lib) => Self::categories(lib),
                None => Vec::new(),
            },
            ["artists", l] => {
                let items = self.items(
                    "/Artists/AlbumArtists",
                    vec![("parentId", l.into()), by_name],
                )?;
                named(items, &|i| {
                    BrowseNode::new(
                        format!("artist/{}/{}", enc_segment(l), enc_segment(&i.id)),
                        i.name(),
                        NodeKind::Artist,
                        true,
                    )
                })
            }
            ["artist", l, a] => {
                let items = self.items(
                    "/Items",
                    vec![
                        ("parentId", l.into()),
                        ("albumArtistIds", a.into()),
                        ("includeItemTypes", "MusicAlbum".into()),
                        ("recursive", "true".into()),
                        ("sortBy", "ProductionYear,SortName".into()),
                    ],
                )?;
                named(items, &|i| {
                    Self::album_node(i, i.production_year.map(|y| y.to_string()))
                })
            }
            ["albums", l] => {
                let items = self.items(
                    "/Items",
                    vec![
                        ("parentId", l.into()),
                        ("includeItemTypes", "MusicAlbum".into()),
                        ("recursive", "true".into()),
                        by_name,
                    ],
                )?;
                named(items, &|i| Self::album_node(i, i.album_artist.clone()))
            }
            ["genres", l] => {
                let kind = self.kind(l);
                let items = if kind == LibraryKind::Music {
                    self.items("/MusicGenres", vec![("parentId", l.into()), by_name])?
                } else {
                    let types = ("includeItemTypes", kind.item_types().to_string());
                    self.items("/Genres", vec![("parentId", l.into()), types, by_name])?
                };
                let sort = if kind.is_video() {
                    Sort::asc(SortKey::Title)
                } else {
                    Sort::asc(SortKey::Artist)
                };
                named(items, &|i| {
                    BrowseNode::new(
                        format!("genre/{}/{}", enc_segment(l), enc_segment(&i.id)),
                        i.name(),
                        NodeKind::Genre,
                        false,
                    )
                    .with_sort(sort)
                })
            }
            ["shows", l] => {
                let items = self.items(
                    "/Items",
                    vec![
                        ("parentId", l.into()),
                        ("includeItemTypes", "Series".into()),
                        ("recursive", "true".into()),
                        by_name,
                    ],
                )?;
                named(items, &|i| {
                    let mut n = BrowseNode::new(
                        format!("series/{}/{}", enc_segment(l), enc_segment(&i.id)),
                        i.name(),
                        NodeKind::Series,
                        true,
                    )
                    .with_detail(i.production_year.map(|y| y.to_string()))
                    .with_sort(Sort::asc(SortKey::Track));
                    if let Some(c) = i.child_count {
                        n = n.with_count(c);
                    }
                    n
                })
            }
            ["series", _, p] => {
                let items = self.items(&format!("/Shows/{}/Seasons", seg(p)), Vec::new())?;
                named(items, &|i| {
                    let mut n = BrowseNode::new(
                        format!("season/{}", enc_segment(&i.id)),
                        i.name(),
                        NodeKind::Season,
                        false,
                    )
                    .with_sort(Sort::asc(SortKey::Track));
                    if let Some(c) = i.child_count {
                        n = n.with_count(c);
                    }
                    n
                })
            }
            ["playlists"] => {
                let items = self.items(
                    "/Items",
                    vec![
                        ("includeItemTypes", "Playlist".into()),
                        ("recursive", "true".into()),
                        by_name,
                    ],
                )?;
                let video = self.shows_video();
                items
                    .iter()
                    .filter(|i| match i.media_type.as_deref() {
                        None | Some("Audio") => true,
                        Some("Video") => video,
                        Some(_) => false,
                    })
                    .map(|i| {
                        let mut n = BrowseNode::new(
                            format!("playlist/{}", enc_segment(&i.id)),
                            i.name(),
                            NodeKind::Playlist,
                            false,
                        )
                        .with_sort(Sort::asc(SortKey::Position));
                        if let Some(c) = i.child_count {
                            n = n.with_count(c);
                        }
                        n
                    })
                    .collect()
            }
            ["album", _]
            | ["genre", _, _]
            | ["favorites", _]
            | ["played", _]
            | ["unwatched", _]
            | ["recent", _]
            | ["nextup", _]
            | ["movies", _]
            | ["videos", _]
            | ["season", _]
            | ["playlist", _] => Vec::new(),
            _ => return Err(Error::NotFound(format!("browse node `{}`", node.id))),
        })
    }

    fn tracks(
        &self,
        node: &BrowseNode,
        query: &Query,
        sort: Sort,
        page: Page,
    ) -> Result<TrackPage> {
        let scope = self.scope(node)?;
        if let Scope::Items(params) = &scope
            && Self::server_side(&scope, query, sort)
        {
            let (tracks, total) = self.query_page(params, Some(sort), page)?;
            return Ok(TrackPage {
                tracks,
                total,
                offset: page.offset,
            });
        }
        let list = self.filtered(node, &scope, query, sort)?;
        let total = list.len() as u64;
        let tracks = list
            .iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(usize::MAX))
            .cloned()
            .collect();
        Ok(TrackPage {
            tracks,
            total,
            offset: page.offset,
        })
    }

    /// Count and length from a full listing when one is at hand or needed; otherwise only
    /// the count (one cheap request), with a length of 0.
    fn summary(&self, node: &BrowseNode, query: &Query) -> Result<TrackSummary> {
        let scope = self.scope(node)?;
        let cached = self
            .cache()
            .iter()
            .any(|c| c.node == node.id && c.at.elapsed() < CACHE_TTL);
        if let Scope::Items(params) = &scope
            && !cached
            && query.is_empty()
        {
            let (_, total) = self.query_page(params, None, Page::new(0, 1))?;
            return Ok(TrackSummary {
                count: total,
                length_ms: 0,
            });
        }
        let list = self.filtered(node, &scope, query, Sort::default())?;
        Ok(TrackSummary {
            count: list.len() as u64,
            length_ms: list.iter().map(|t| t.length_ms).sum(),
        })
    }

    fn track(&self, id: &TrackId) -> Result<Option<Track>> {
        Ok(self.fetch_item(id)?.map(|i| i.to_track(&self.id)))
    }

    /// The universal audio URL: the file as it is when nulamp decodes its format, else an
    /// MP3 transcode. A video's file as it is (nulamp decodes it with FFmpeg). Needs no
    /// request, so opening a track never waits for the server.
    fn resolve_playback(&self, id: &TrackId) -> Result<PlaybackLocation> {
        self.check(id)?;
        let a = &self.account;
        let (item, video) = item_of(&id.id);
        let location = if video {
            format!(
                "{}/Videos/{}/stream?static=true&mediaSourceId={}&deviceId={}",
                a.server_url,
                seg(item),
                seg(item),
                seg(&a.device_id),
            )
        } else {
            format!(
                "{}/Audio/{}/universal?userId={}&deviceId={}&container={}&transcodingContainer=mp3\
                 &transcodingProtocol=http&audioCodec=mp3&audioBitRate=320000\
                 &maxStreamingBitrate=1000000000&enableRedirection=false",
                a.server_url,
                seg(item),
                seg(&a.user_id),
                seg(&a.device_id),
                DIRECT_CONTAINERS.replace('|', "%7C"),
            )
        };
        Ok(PlaybackLocation {
            location,
            headers: vec![(
                "Authorization".to_string(),
                self.client.authorization().to_string(),
            )],
            video,
        })
    }

    fn artwork(&self, id: &TrackId) -> Result<Option<(String, Vec<u8>)>> {
        let Some(item) = self.fetch_item(id)? else {
            return Ok(None);
        };
        let Some(art) = item.to_track(&self.id).artwork else {
            return Ok(None);
        };
        match self.client.get(
            &format!("/Items/{}/Images/Primary", seg(&art)),
            &[("maxWidth", "600".to_string())],
        ) {
            Ok((mime, bytes)) => Ok(Some((mime, bytes))),
            Err(Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A song counts by the usual rule; a video once [`WATCHED_PERCENT`] of it was played.
    fn counts_play(&self, id: &TrackId, played_ms: u64, length_ms: u64) -> bool {
        if item_of(&id.id).1 {
            length_ms > 0 && played_ms.saturating_mul(100) >= length_ms * WATCHED_PERCENT
        } else {
            crate::should_count_play(played_ms, length_ms)
        }
    }

    /// Marks the track or video played (the server counts the play and sets the time).
    fn record_play(&self, id: &TrackId, _played_ms: u64) -> Result<()> {
        self.check(id)?;
        self.client.post(
            &format!("/UserPlayedItems/{}", seg(item_of(&id.id).0)),
            &[self.user()],
            None,
        )?;
        self.forget();
        self.events.send(LibraryEvent::TracksChanged);
        Ok(())
    }

    fn subscribe(&self) -> Receiver<LibraryEvent> {
        self.events.subscribe()
    }
}
