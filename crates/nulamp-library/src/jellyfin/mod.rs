//! A Jellyfin server as a library provider (`docs/media-library.md` §9).
//!
//! [`login`] signs in with a user name and password and returns an [`Account`]: the server
//! address, the access token and the server's libraries. The app stores the account (never
//! the password) and builds a [`JellyfinProvider`] from it. The provider shows the libraries
//! the user enabled; only music libraries are offered for now, but [`Library`] keeps the
//! collection type of every library so other kinds can be added later.
//!
//! Listings without a search are paged and sorted by the server. A search, a sort the server
//! cannot do, a playlist and the root of several libraries are listed in full once, kept for a
//! few minutes, and filtered with [`Query::matches`] and sorted with [`Sort::compare`] in
//! memory, so search behaves as in the local library.
//!
//! Tracks are located by [`TrackId::location`] (`library://<provider>/<item>`); playback
//! resolves that to the server's universal audio URL with the token in the `Authorization`
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
use dto::{AuthResult, Item, ItemList, PublicInfo};

/// Prefix of every Jellyfin provider id; the rest is the user's id on the server.
pub const PROVIDER_PREFIX: &str = "jellyfin-";

/// Collection type of music libraries.
pub const MUSIC: &str = "music";

/// Containers (with their audio codec where it matters) that nulamp's network input decodes
/// as they are; the server transcodes anything else to MP3.
const DIRECT_CONTAINERS: &str =
    "mp3,flac,ogg|vorbis,oga|vorbis,wav,m4a|aac,m4a|alac,mp4|aac,m4b|aac,aac";

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
    pub fn is_music(&self) -> bool {
        self.collection_type == MUSIC
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

    /// The enabled libraries the provider shows (music only for now).
    pub fn shown_libraries(&self) -> impl Iterator<Item = &Library> {
        self.libraries.iter().filter(|l| l.enabled && l.is_music())
    }

    fn client(&self) -> Client {
        Client::new(&self.server_url, &self.device_id, &self.token)
    }
}

/// Signs in to the server at `url` and lists the user's libraries (music libraries start
/// enabled). Blocks; call it off the user-interface thread.
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
/// `account.libraries` keep their `enabled` flag; new music libraries start enabled.
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
    /// No tracks (the playlist folder).
    Empty,
}

/// Server sort for a [`Sort`]: the `SortBy` list and the number of leading keys that follow
/// the direction (as [`Sort::compare`] does). `None` when the server cannot sort by the key.
fn server_sort(sort: Sort) -> Option<(&'static [&'static str], usize)> {
    const TRACK: [&str; 3] = ["ParentIndexNumber", "IndexNumber", "SortName"];
    let keys: &'static [&'static str] = match sort.key {
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

/// The tracks of an account's enabled music libraries.
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

    /// The category nodes of a library.
    fn categories(lib: &str) -> Vec<BrowseNode> {
        let l = enc_segment(lib);
        vec![
            BrowseNode::new(format!("artists/{l}"), "Artists", NodeKind::Category, true),
            BrowseNode::new(format!("albums/{l}"), "Albums", NodeKind::Category, true),
            BrowseNode::new(format!("genres/{l}"), "Genres", NodeKind::Category, true),
            BrowseNode::new(
                format!("favorites/{l}"),
                "Favorites",
                NodeKind::SmartList,
                false,
            ),
            BrowseNode::new(
                format!("played/{l}"),
                "Most played",
                NodeKind::SmartList,
                false,
            )
            .with_sort(Sort::desc(SortKey::PlayCount)),
        ]
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
        let parent = |l: &str| vec![("parentId", l.to_string())];
        let with = |l: &str, k: &'static str, v: &str| {
            let mut p = parent(l);
            p.push((k, v.to_string()));
            p
        };
        Ok(match s[..] {
            [""] => {
                let libs = self.libraries();
                match libs.len() {
                    0 => Scope::Empty,
                    1 => Scope::Items(parent(&libs[0].id)),
                    _ => Scope::Union(libs.iter().map(|l| parent(&l.id)).collect()),
                }
            }
            ["lib" | "artists" | "albums" | "genres", l] => Scope::Items(parent(l)),
            ["artist", l, a] => Scope::Items(with(l, "albumArtistIds", a)),
            ["album", b] => Scope::Items(parent(b)),
            ["genre", l, g] => Scope::Items(with(l, "genreIds", g)),
            ["favorites", l] => Scope::Items(with(l, "filters", "IsFavorite")),
            ["played", l] => Scope::Items(with(l, "filters", "IsPlayed")),
            ["playlists"] => Scope::Empty,
            ["playlist", p] => Scope::Playlist(p.to_string()),
            _ => return Err(Error::NotFound(format!("browse node `{}`", node.id))),
        })
    }

    /// Audio items of one query, `page` of them in server order `sort`.
    fn query_page(
        &self,
        params: &[(&'static str, String)],
        sort: Option<Sort>,
        page: Page,
    ) -> Result<(Vec<Track>, u64)> {
        let mut q = params.to_vec();
        q.extend([
            ("includeItemTypes", "Audio".to_string()),
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
        if let Some((keys, primary)) =
            sort.and_then(|s| server_sort(s).map(|k| (s, k)))
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
            out.extend(
                items
                    .iter()
                    .filter(|i| i.is_audio())
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
        let playlist_order = matches!(scope, Scope::Playlist(_)) && sort.key == SortKey::Position;
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
        matches!(scope, Scope::Items(_)) && query.is_empty() && server_sort(sort).is_some()
    }

    fn fetch_item(&self, id: &TrackId) -> Result<Option<Item>> {
        self.check(id)?;
        let q = [
            self.user(),
            ("fields", format!("{TRACK_FIELDS},MediaSources")),
        ];
        match self.client.get_json(&format!("/Items/{}", seg(&id.id)), &q) {
            Ok(item) => Ok(Some(item)),
            Err(Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
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
        let seg = split_id(&node.id);
        let s: Vec<&str> = seg.iter().map(String::as_str).collect();
        let named = |items: Vec<Item>, f: &dyn Fn(&Item) -> BrowseNode| -> Vec<BrowseNode> {
            items.iter().map(f).collect()
        };
        let by_name = ("sortBy", "SortName".to_string());
        Ok(match s[..] {
            [""] => {
                let libs = self.libraries();
                let mut out = match libs[..] {
                    [one] => Self::categories(&one.id),
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
            ["lib", l] => Self::categories(l),
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
                let items = self.items("/MusicGenres", vec![("parentId", l.into()), by_name])?;
                named(items, &|i| {
                    BrowseNode::new(
                        format!("genre/{}/{}", enc_segment(l), enc_segment(&i.id)),
                        i.name(),
                        NodeKind::Genre,
                        false,
                    )
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
                items
                    .iter()
                    .filter(|i| i.media_type.as_deref().is_none_or(|m| m == "Audio"))
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
            ["album", _] | ["genre", _, _] | ["favorites", _] | ["played", _] | ["playlist", _] => {
                Vec::new()
            }
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
    /// MP3 transcode. Needs no request, so opening a track never waits for the server.
    fn resolve_playback(&self, id: &TrackId) -> Result<PlaybackLocation> {
        self.check(id)?;
        let a = &self.account;
        let location = format!(
            "{}/Audio/{}/universal?userId={}&deviceId={}&container={}&transcodingContainer=mp3\
             &transcodingProtocol=http&audioCodec=mp3&audioBitRate=320000\
             &maxStreamingBitrate=1000000000&enableRedirection=false",
            a.server_url,
            seg(&id.id),
            seg(&a.user_id),
            seg(&a.device_id),
            DIRECT_CONTAINERS.replace('|', "%7C"),
        );
        Ok(PlaybackLocation {
            location,
            headers: vec![(
                "Authorization".to_string(),
                self.client.authorization().to_string(),
            )],
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

    /// Marks the track played (the server counts the play and sets the time).
    fn record_play(&self, id: &TrackId, _played_ms: u64) -> Result<()> {
        self.check(id)?;
        self.client.post(
            &format!("/UserPlayedItems/{}", seg(&id.id)),
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
