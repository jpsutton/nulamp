//! State of the media-library window, independent of drawing and input: the provider tree,
//! the paged track listing, the debounced search and the status line. Provider calls are
//! queued as [`Request`]s for the worker; responses come back through the `on_*` methods.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use nulamp_library::{
    AlbumFacet, ArtistFacet, BrowseNode, Capabilities, LibraryEvent, NodeKind, Page, Sort, SortKey,
    Track, TrackPage, TrackSummary,
};
use nulamp_skinui::{TreeIcon, TreeRow};

use super::service::{ListSpec, Purpose, Request};

/// Rows fetched per page.
pub const PAGE_ROWS: usize = 200;
/// Typing pause before the search applies.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
/// Pages kept in memory; the ones farthest from the view are dropped first.
const MAX_PAGES: usize = 24;
/// Provider change events refresh the views at most this often (scans send many).
pub const REFRESH_INTERVAL: Duration = Duration::from_millis(1000);
/// How long a status-line message stays.
pub const MESSAGE_TIME: Duration = Duration::from_secs(8);

/// A list column: title, default width, alignment and the sort key its header selects.
#[derive(Debug, Clone, Copy)]
pub struct ColumnDef {
    pub title: &'static str,
    pub width: i32,
    pub right: bool,
    pub sort: SortKey,
}

const fn col(title: &'static str, width: i32, right: bool, sort: SortKey) -> ColumnDef {
    ColumnDef {
        title,
        width,
        right,
        sort,
    }
}

pub const COLUMNS: [ColumnDef; 10] = [
    col("Artist", 100, false, SortKey::Artist),
    col("Album", 100, false, SortKey::Album),
    col("Track #", 50, true, SortKey::Track),
    col("Title", 130, false, SortKey::Title),
    col("Length", 46, true, SortKey::Length),
    col("Genre", 64, false, SortKey::Genre),
    col("Rating", 52, false, SortKey::Rating),
    col("Play Count", 66, true, SortKey::PlayCount),
    col("Played Last", 104, false, SortKey::LastPlayed),
    col("Year", 38, true, SortKey::Year),
];

/// The column whose header shows a sort key.
pub fn column_for_sort(key: SortKey) -> Option<usize> {
    COLUMNS.iter().position(|c| c.sort == key)
}

/// Text of a list cell.
pub fn cell(t: &Track, column: usize) -> String {
    let opt = |v: Option<&str>| v.unwrap_or("").to_string();
    match column {
        0 => opt(t.artist()),
        1 => opt(t.album()),
        2 => t.track_number().map(|n| n.to_string()).unwrap_or_default(),
        3 => t.display_title(),
        4 if t.length_ms > 0 => crate::marquee::format_length(t.length_ms / 1000),
        5 => opt(t.genre()),
        6 => "★".repeat(t.rating.min(5) as usize),
        7 => t.play_count.to_string(),
        8 => t
            .last_played
            .map(super::clock::format_local)
            .unwrap_or_default(),
        9 => t.year().map(|y| y.to_string()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Identifies a tree node: provider id and node id.
pub type NodeKey = (String, String);

/// A node of the browse tree.
#[derive(Debug, Clone)]
pub struct TreeNode {
    pub provider: String,
    pub node: BrowseNode,
    pub expanded: bool,
    /// `None` until loaded.
    pub children: Option<Vec<TreeNode>>,
    pub loading: bool,
}

impl TreeNode {
    fn new(provider: &str, node: BrowseNode) -> Self {
        Self {
            provider: provider.to_string(),
            node,
            expanded: false,
            children: None,
            loading: false,
        }
    }

    fn key(&self) -> NodeKey {
        (self.provider.clone(), self.node.id.clone())
    }

    fn label(&self) -> String {
        let mut s = self.node.label.clone();
        if let Some(d) = self.node.detail.as_deref().filter(|d| !d.is_empty()) {
            s.push_str(" – ");
            s.push_str(d);
        }
        if let Some(n) = self.node.track_count {
            s.push_str(&format!(" ({n})"));
        }
        s
    }
}

fn find_mut<'a>(nodes: &'a mut [TreeNode], key: &NodeKey) -> Option<&'a mut TreeNode> {
    for n in nodes {
        if n.provider == key.0 && n.node.id == key.1 {
            return Some(n);
        }
        if let Some(children) = n.children.as_mut()
            && let Some(found) = find_mut(children, key)
        {
            return Some(found);
        }
    }
    None
}

fn find<'a>(nodes: &'a [TreeNode], key: &NodeKey) -> Option<&'a TreeNode> {
    for n in nodes {
        if n.provider == key.0 && n.node.id == key.1 {
            return Some(n);
        }
        if let Some(found) = n.children.as_deref().and_then(|c| find(c, key)) {
            return Some(found);
        }
    }
    None
}

fn contains(node: &TreeNode, key: &NodeKey) -> bool {
    node.children
        .as_deref()
        .is_some_and(|c| c.iter().any(|n| n.key() == *key || contains(n, key)))
}

/// New children merged with the old ones: nodes that stay keep their expansion and loaded
/// children.
fn merge(provider: &str, old: Option<Vec<TreeNode>>, new: Vec<BrowseNode>) -> Vec<TreeNode> {
    let mut old: HashMap<String, TreeNode> = old
        .unwrap_or_default()
        .into_iter()
        .map(|n| (n.node.id.clone(), n))
        .collect();
    new.into_iter()
        .map(|b| match old.remove(&b.id) {
            Some(mut n) => {
                n.node = b;
                n
            }
            None => TreeNode::new(provider, b),
        })
        .collect()
}

fn icon_of(n: &TreeNode) -> TreeIcon {
    match n.node.kind {
        NodeKind::Root => TreeIcon::Folder,
        NodeKind::Category | NodeKind::Other => TreeIcon::Folder,
        NodeKind::Artist => TreeIcon::Person,
        NodeKind::Album => TreeIcon::Disc,
        NodeKind::Genre | NodeKind::Year => TreeIcon::Tag,
        NodeKind::SmartList => TreeIcon::Star,
        NodeKind::Playlist => TreeIcon::List,
    }
}

fn flatten(nodes: &[TreeNode], depth: usize, rows: &mut Vec<TreeRow>, keys: &mut Vec<NodeKey>) {
    for n in nodes {
        rows.push(TreeRow {
            depth,
            label: n.label(),
            expandable: n.node.expandable,
            expanded: n.expanded,
            icon: icon_of(n),
        });
        keys.push(n.key());
        if n.expanded
            && let Some(c) = &n.children
        {
            flatten(c, depth + 1, rows, keys);
        }
    }
}

/// The library window's state.
pub struct LibraryView {
    pub roots: Vec<TreeNode>,
    pub selected: Option<NodeKey>,
    /// The listing shown, with the applied search text.
    pub spec: Option<ListSpec>,
    /// Changes whenever the listing is reloaded; responses for older ones are dropped.
    pub generation: u64,
    /// Matching rows, once the first page arrived.
    pub total: Option<usize>,
    pages: HashMap<usize, (u64, Vec<Track>)>,
    requested: HashSet<(u64, usize)>,
    pub summary: Option<TrackSummary>,
    /// When the listing was last (re)loaded and how long its summary took to arrive.
    list_started: Option<Instant>,
    pub list_secs: Option<f64>,
    /// The artist pane's rows (without the "All" row), in the pane's order.
    pub artists: Vec<ArtistFacet>,
    /// The album pane's rows.
    pub albums: Vec<AlbumFacet>,
    /// Pane sort: column and direction (descending).
    pub artist_sort: (usize, bool),
    pub album_sort: (usize, bool),
    artists_token: u64,
    albums_token: u64,
    /// The artist and album panes changed.
    pub panes_dirty: bool,
    /// Text of the search box (applied after [`SEARCH_DEBOUNCE`]).
    pub search: String,
    search_edited: Option<Instant>,
    /// Scan progress (done, total).
    pub scan: Option<(u64, u64)>,
    /// A note for the status line (scan result, error); it goes away after
    /// [`MESSAGE_TIME`].
    pub message: Option<String>,
    message_shown: Option<(String, Instant)>,
    refresh_due: Option<Instant>,
    last_refresh: Option<Instant>,
    /// The tree rows changed.
    pub tree_dirty: bool,
    /// The listing was replaced: the window clears its selection and scrolls to the top.
    pub list_reset: bool,
    capabilities: HashMap<String, Capabilities>,
    out: Vec<Request>,
}

impl LibraryView {
    /// A tree with one top node per provider (id, name, capabilities); the first is
    /// expanded and listed.
    pub fn new(providers: &[(String, String, Capabilities)]) -> Self {
        let roots = providers
            .iter()
            .map(|(id, name, _)| {
                let mut root = BrowseNode::root();
                root.label = name.clone();
                TreeNode::new(id, root)
            })
            .collect();
        let mut v = Self {
            roots,
            selected: None,
            spec: None,
            generation: 0,
            total: None,
            pages: HashMap::new(),
            requested: HashSet::new(),
            summary: None,
            list_started: None,
            list_secs: None,
            artists: Vec::new(),
            albums: Vec::new(),
            artist_sort: (0, false),
            album_sort: (0, false),
            artists_token: 0,
            albums_token: 0,
            panes_dirty: true,
            search: String::new(),
            search_edited: None,
            scan: None,
            message: None,
            message_shown: None,
            refresh_due: None,
            last_refresh: None,
            tree_dirty: true,
            list_reset: true,
            capabilities: providers
                .iter()
                .map(|(id, _, c)| (id.clone(), *c))
                .collect(),
            out: Vec::new(),
        };
        if let Some(first) = v.roots.first().map(TreeNode::key) {
            v.expand(&first);
            v.select(&first);
        }
        v
    }

    /// Requests queued for the worker.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.out)
    }

    pub fn capabilities(&self, provider: &str) -> Capabilities {
        self.capabilities.get(provider).copied().unwrap_or_default()
    }

    // ----- Tree -----

    /// Visible tree rows, their node keys and the selected row.
    pub fn tree_rows(&self) -> (Vec<TreeRow>, Vec<NodeKey>, Option<usize>) {
        let (mut rows, mut keys) = (Vec::new(), Vec::new());
        flatten(&self.roots, 0, &mut rows, &mut keys);
        let sel = self
            .selected
            .as_ref()
            .and_then(|s| keys.iter().position(|k| k == s));
        (rows, keys, sel)
    }

    pub fn node(&self, key: &NodeKey) -> Option<&TreeNode> {
        find(&self.roots, key)
    }

    pub fn expand(&mut self, key: &NodeKey) {
        let Some(n) = find_mut(&mut self.roots, key) else {
            return;
        };
        if !n.node.expandable || n.expanded {
            return;
        }
        n.expanded = true;
        if n.children.is_none() && !n.loading {
            n.loading = true;
            self.out.push(Request::Browse {
                provider: n.provider.clone(),
                node: n.node.clone(),
            });
        }
        self.tree_dirty = true;
    }

    /// Collapses a node; a selected node inside it passes the selection to it.
    pub fn collapse(&mut self, key: &NodeKey) {
        let Some(n) = find_mut(&mut self.roots, key) else {
            return;
        };
        if !n.expanded {
            return;
        }
        n.expanded = false;
        let inside = self.selected.as_ref().is_some_and(|s| contains(n, s));
        self.tree_dirty = true;
        if inside {
            self.select(key);
        }
    }

    pub fn toggle(&mut self, key: &NodeKey) {
        match self.node(key).map(|n| n.expanded) {
            Some(true) => self.collapse(key),
            Some(false) => self.expand(key),
            None => {}
        }
    }

    /// Selects a node and lists its tracks in the node's default order.
    pub fn select(&mut self, key: &NodeKey) {
        let Some(n) = self.node(key) else { return };
        if self.selected.as_ref() == Some(key) && self.spec.is_some() {
            return;
        }
        let spec = ListSpec::new(
            &n.provider,
            n.node.clone(),
            &self.search,
            n.node.default_sort,
        );
        self.selected = Some(key.clone());
        self.spec = Some(spec);
        self.tree_dirty = true;
        self.artists.clear();
        self.albums.clear();
        self.panes_dirty = true;
        self.reset_list();
        self.request_artists();
        self.request_albums();
    }

    pub fn on_children(
        &mut self,
        provider: &str,
        parent: &str,
        result: Result<Vec<BrowseNode>, String>,
    ) {
        let key = (provider.to_string(), parent.to_string());
        let Some(n) = find_mut(&mut self.roots, &key) else {
            return;
        };
        n.loading = false;
        match result {
            Ok(children) => {
                let old = n.children.take();
                n.children = Some(merge(provider, old, children));
            }
            Err(e) => {
                n.children.get_or_insert_with(Vec::new);
                self.message = Some(e);
            }
        }
        self.tree_dirty = true;
    }

    // ----- Listing -----

    fn reset_list(&mut self) {
        self.generation += 1;
        self.pages.clear();
        self.requested.clear();
        self.total = None;
        self.summary = None;
        self.list_started = Some(Instant::now());
        self.list_secs = None;
        self.list_reset = true;
        self.request_summary();
        self.want_rows(0, 1);
    }

    fn request_summary(&mut self) {
        if let Some(spec) = &self.spec {
            self.out.push(Request::Summary {
                spec: spec.clone(),
                generation: self.generation,
            });
        }
    }

    /// Reloads the listing in place (the library changed): rows stay on screen until the
    /// new pages arrive, and the selection and scroll position are kept.
    pub fn reload_list(&mut self) {
        self.generation += 1;
        self.requested.clear();
        self.list_started = Some(Instant::now());
        self.request_summary();
        self.request_artists();
        self.request_albums();
    }

    /// Asks for the pages covering `count` rows from `first` (plus one page ahead) that are
    /// missing or out of date.
    pub fn want_rows(&mut self, first: usize, count: usize) {
        let Some(spec) = &self.spec else { return };
        let last = first + count.max(1) + PAGE_ROWS / 2;
        let (p0, p1) = (first / PAGE_ROWS, last / PAGE_ROWS);
        for p in p0..=p1 {
            if self.total.is_some_and(|t| p * PAGE_ROWS >= t && p > 0) {
                break;
            }
            let fresh = self
                .pages
                .get(&p)
                .is_some_and(|(g, _)| *g == self.generation);
            if fresh || !self.requested.insert((self.generation, p)) {
                continue;
            }
            self.out.push(Request::Tracks {
                spec: spec.clone(),
                page: Page::new((p * PAGE_ROWS) as u64, PAGE_ROWS as u64),
                generation: self.generation,
            });
        }
        self.evict(p0);
    }

    fn evict(&mut self, near: usize) {
        while self.pages.len() > MAX_PAGES {
            let Some(&far) = self.pages.keys().max_by_key(|&&p| p.abs_diff(near)) else {
                break;
            };
            self.pages.remove(&far);
        }
    }

    pub fn on_page(&mut self, generation: u64, offset: u64, result: Result<TrackPage, String>) {
        if generation != self.generation {
            return;
        }
        match result {
            Ok(page) => {
                let total = usize::try_from(page.total).unwrap_or(usize::MAX);
                if self.total != Some(total) {
                    self.total = Some(total);
                    // Pages past the new end are gone.
                    let last = total.div_ceil(PAGE_ROWS);
                    self.pages.retain(|p, _| *p < last.max(1));
                }
                let p = offset as usize / PAGE_ROWS;
                self.pages.insert(p, (generation, page.tracks));
            }
            Err(e) => {
                self.requested
                    .remove(&(generation, offset as usize / PAGE_ROWS));
                self.message = Some(e);
            }
        }
    }

    pub fn on_summary(&mut self, generation: u64, summary: TrackSummary) {
        if generation == self.generation {
            self.summary = Some(summary);
            self.list_secs = self.list_started.map(|t| t.elapsed().as_secs_f64());
        }
    }

    // ----- Artist and album panes -----

    fn request_artists(&mut self) {
        if let Some(spec) = &self.spec {
            self.artists_token += 1;
            self.out.push(Request::Artists {
                spec: spec.clone(),
                token: self.artists_token,
            });
        }
    }

    fn request_albums(&mut self) {
        if let Some(spec) = &self.spec {
            self.albums_token += 1;
            self.out.push(Request::Albums {
                spec: spec.clone(),
                token: self.albums_token,
            });
        }
    }

    pub fn on_artists(&mut self, token: u64, result: Result<Vec<ArtistFacet>, String>) {
        if token != self.artists_token {
            return;
        }
        match result {
            Ok(list) => self.artists = list,
            Err(e) => {
                self.artists.clear();
                self.message = Some(e);
            }
        }
        sort_artists(&mut self.artists, self.artist_sort);
        self.panes_dirty = true;
        // The picked artist is gone (a new search or a changed library): show everything.
        let gone = self
            .spec
            .as_ref()
            .and_then(|s| s.artist.as_deref())
            .is_some_and(|a| !self.artists.iter().any(|f| same_name(&f.name, a)));
        if gone {
            self.select_artist(None);
        }
    }

    pub fn on_albums(&mut self, token: u64, result: Result<Vec<AlbumFacet>, String>) {
        if token != self.albums_token {
            return;
        }
        match result {
            Ok(list) => self.albums = list,
            Err(e) => {
                self.albums.clear();
                self.message = Some(e);
            }
        }
        sort_albums(&mut self.albums, self.album_sort);
        self.panes_dirty = true;
        let gone = self
            .spec
            .as_ref()
            .and_then(|s| s.album.as_deref())
            .is_some_and(|a| !self.albums.iter().any(|f| same_name(&f.name, a)));
        if gone {
            self.select_album(None);
        }
    }

    /// Picks an artist (`None`: all). The albums and the track list follow.
    pub fn select_artist(&mut self, artist: Option<String>) {
        let Some(spec) = &mut self.spec else { return };
        if spec.artist == artist && spec.album.is_none() {
            return;
        }
        let changed = spec.artist != artist;
        spec.artist = artist;
        spec.album = None;
        self.panes_dirty = true;
        self.reset_list();
        if changed {
            self.albums.clear();
            self.request_albums();
        }
    }

    /// Picks an album (`None`: all of the artist's). The track list follows.
    pub fn select_album(&mut self, album: Option<String>) {
        let Some(spec) = &mut self.spec else { return };
        if spec.album == album {
            return;
        }
        spec.album = album;
        self.panes_dirty = true;
        self.reset_list();
    }

    /// Orders the artist pane by a column (0 name, 1 albums, 2 tracks).
    pub fn set_artist_sort(&mut self, column: usize, descending: bool) {
        self.artist_sort = (column, descending);
        sort_artists(&mut self.artists, self.artist_sort);
        self.panes_dirty = true;
    }

    /// Orders the album pane by a column (0 name, 1 year, 2 tracks).
    pub fn set_album_sort(&mut self, column: usize, descending: bool) {
        self.album_sort = (column, descending);
        sort_albums(&mut self.albums, self.album_sort);
        self.panes_dirty = true;
    }

    /// Row of the picked artist in the artist pane (row 0 is "All").
    pub fn artist_row(&self) -> usize {
        match self.spec.as_ref().and_then(|s| s.artist.as_deref()) {
            Some(a) => self
                .artists
                .iter()
                .position(|f| same_name(&f.name, a))
                .map_or(0, |i| i + 1),
            None => 0,
        }
    }

    pub fn album_row(&self) -> usize {
        match self.spec.as_ref().and_then(|s| s.album.as_deref()) {
            Some(a) => self
                .albums
                .iter()
                .position(|f| same_name(&f.name, a))
                .map_or(0, |i| i + 1),
            None => 0,
        }
    }

    /// Text of an artist-pane cell (row 0 is "All").
    pub fn artist_cell(&self, row: usize, col: usize) -> Option<String> {
        if row == 0 {
            let n = self.artists.len();
            return Some(match col {
                0 => format!("All ({n} {})", if n == 1 { "artist" } else { "artists" }),
                1 => self
                    .artists
                    .iter()
                    .map(|a| a.albums)
                    .sum::<u64>()
                    .to_string(),
                _ => self
                    .artists
                    .iter()
                    .map(|a| a.tracks)
                    .sum::<u64>()
                    .to_string(),
            });
        }
        let f = self.artists.get(row - 1)?;
        Some(match col {
            0 if f.name.is_empty() => UNKNOWN_ARTIST.to_string(),
            0 => f.name.clone(),
            1 => f.albums.to_string(),
            _ => f.tracks.to_string(),
        })
    }

    pub fn album_cell(&self, row: usize, col: usize) -> Option<String> {
        if row == 0 {
            let n = self.albums.len();
            return Some(match col {
                0 => format!("All ({n} {})", if n == 1 { "album" } else { "albums" }),
                1 => String::new(),
                _ => self
                    .albums
                    .iter()
                    .map(|a| a.tracks)
                    .sum::<u64>()
                    .to_string(),
            });
        }
        let f = self.albums.get(row - 1)?;
        Some(match col {
            0 if f.name.is_empty() => UNKNOWN_ALBUM.to_string(),
            0 => f.name.clone(),
            1 => f.year.map(|y| y.to_string()).unwrap_or_default(),
            _ => f.tracks.to_string(),
        })
    }

    /// The artist an artist-pane row stands for (`None` for "All").
    pub fn artist_at(&self, row: usize) -> Option<String> {
        self.artists
            .get(row.checked_sub(1)?)
            .map(|f| f.name.clone())
    }

    pub fn album_at(&self, row: usize) -> Option<String> {
        self.albums.get(row.checked_sub(1)?).map(|f| f.name.clone())
    }

    /// The track at a row, when its page is loaded.
    pub fn track(&self, row: usize) -> Option<&Track> {
        self.pages
            .get(&(row / PAGE_ROWS))
            .and_then(|(_, t)| t.get(row % PAGE_ROWS))
    }

    /// The tracks of selected rows (`start..end` ranges) when all of them are loaded.
    pub fn loaded_tracks(&self, ranges: &[(usize, usize)]) -> Option<Vec<Track>> {
        let mut out = Vec::new();
        for &(a, b) in ranges {
            for row in a..b {
                out.push(self.track(row)?.clone());
            }
        }
        Some(out)
    }

    /// Queues fetching the selected rows for an action.
    pub fn collect(&mut self, ranges: &[(usize, usize)], purpose: Purpose) {
        let Some(spec) = &self.spec else { return };
        if ranges.is_empty() {
            return;
        }
        self.out.push(Request::Collect {
            spec: spec.clone(),
            ranges: ranges.to_vec(),
            purpose,
        });
    }

    /// Sorts the listing (a header click).
    pub fn set_sort(&mut self, sort: Sort) {
        if let Some(spec) = &mut self.spec
            && spec.sort != sort
        {
            spec.sort = sort;
            self.reset_list();
        }
    }

    pub fn sort(&self) -> Option<Sort> {
        self.spec.as_ref().map(|s| s.sort)
    }

    // ----- Search -----

    /// The search box changed; the search applies after [`SEARCH_DEBOUNCE`] without typing.
    pub fn set_search(&mut self, text: &str, now: Instant) {
        if self.search != text {
            self.search = text.to_string();
            self.search_edited = Some(now);
        }
    }

    /// Applies the search box now.
    pub fn apply_search(&mut self) -> bool {
        self.search_edited = None;
        match &mut self.spec {
            Some(spec) if spec.query != self.search => {
                spec.query = self.search.clone();
                self.reset_list();
                self.request_artists();
                self.request_albums();
                true
            }
            _ => false,
        }
    }

    /// Time-based work: the debounced search and throttled refreshes. Returns true when the
    /// view changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self
            .search_edited
            .is_some_and(|t| now.duration_since(t) >= SEARCH_DEBOUNCE)
        {
            changed |= self.apply_search();
        }
        if self.refresh_due.is_some_and(|t| now >= t) {
            self.refresh_due = None;
            self.refresh(now);
            changed = true;
        }
        match (&self.message, &self.message_shown) {
            (Some(m), Some((shown, since))) if m == shown => {
                if now.duration_since(*since) >= MESSAGE_TIME {
                    self.message = None;
                    self.message_shown = None;
                    changed = true;
                }
            }
            (Some(m), _) => self.message_shown = Some((m.clone(), now)),
            (None, _) => self.message_shown = None,
        }
        changed
    }

    // ----- Provider events -----

    /// A provider event. Returns true when the status line changed.
    pub fn on_event(&mut self, event: LibraryEvent, now: Instant) -> bool {
        match event {
            LibraryEvent::TracksChanged | LibraryEvent::PlaylistsChanged => {
                self.schedule_refresh(now);
                false
            }
            LibraryEvent::ScanProgress { done, total } => {
                self.scan = Some((done, total));
                true
            }
            LibraryEvent::ScanFinished(s) => {
                self.scan = None;
                self.message = Some(match &s.error {
                    Some(e) => format!("Scan failed: {e}"),
                    None if s.cancelled => "Scan cancelled".into(),
                    None => format!(
                        "Scan done: {} added, {} updated, {} removed",
                        s.added, s.updated, s.removed
                    ),
                });
                self.schedule_refresh(now);
                true
            }
        }
    }

    fn schedule_refresh(&mut self, now: Instant) {
        match self.last_refresh {
            Some(t) if now.duration_since(t) < REFRESH_INTERVAL => {
                self.refresh_due.get_or_insert(t + REFRESH_INTERVAL);
            }
            _ => self.refresh(now),
        }
    }

    /// Reloads the listing and the loaded parts of the tree. Collapsed nodes forget their
    /// children and load them again when expanded.
    pub fn refresh(&mut self, now: Instant) {
        self.last_refresh = Some(now);
        self.refresh_due = None;
        fn walk(nodes: &mut [TreeNode], top: bool, out: &mut Vec<Request>) {
            for n in nodes {
                if n.children.is_none() || n.loading {
                    continue;
                }
                if n.expanded || top {
                    n.loading = true;
                    out.push(Request::Browse {
                        provider: n.provider.clone(),
                        node: n.node.clone(),
                    });
                    if let Some(c) = n.children.as_mut() {
                        walk(c, false, out);
                    }
                } else {
                    n.children = None;
                }
            }
        }
        walk(&mut self.roots, true, &mut self.out);
        self.reload_list();
    }

    // ----- Status line -----

    pub fn status_text(&self) -> String {
        if let Some((done, total)) = self.scan {
            return format!("Scanning: {done}/{total} files");
        }
        let count = self
            .summary
            .map(|s| s.count)
            .or(self.total.map(|t| t as u64));
        let mut s = match count {
            None => "Loading…".to_string(),
            Some(n) => {
                let items = format!("{n} {}", if n == 1 { "item" } else { "items" });
                let length = self
                    .summary
                    .filter(|s| s.length_ms >= 1000)
                    .map(|s| format!(" [{}]", crate::playlist::format_total(s.length_ms / 1000)))
                    .unwrap_or_default();
                let took = self
                    .list_secs
                    .map(|t| format!(" in {t:.3} sec."))
                    .unwrap_or_default();
                format!("{items}{length}{took}")
            }
        };
        if let Some(m) = &self.message {
            s.push_str("  ·  ");
            s.push_str(m);
        }
        s
    }
}

const UNKNOWN_ARTIST: &str = "Unknown artist";
const UNKNOWN_ALBUM: &str = "Unknown album";

/// Pane names compare without case, like the providers group them.
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Orders a pane by name; untagged entries stay last whatever the direction.
fn sort_artists(list: &mut [ArtistFacet], (column, desc): (usize, bool)) {
    list.sort_by(|a, b| {
        let by = match column {
            1 => a.albums.cmp(&b.albums),
            2 => a.tracks.cmp(&b.tracks),
            _ => nulamp_core::playlist::natural_cmp(&a.name, &b.name),
        };
        untagged_last(&a.name, &b.name, if desc { by.reverse() } else { by })
    });
}

fn sort_albums(list: &mut [AlbumFacet], (column, desc): (usize, bool)) {
    use std::cmp::Ordering::{Equal, Greater, Less};
    list.sort_by(|a, b| {
        let by = match column {
            1 => match (a.year, b.year) {
                // An unknown year is last in both directions.
                (None, Some(_)) => return Greater,
                (Some(_), None) => return Less,
                (Some(x), Some(y)) => x.cmp(&y),
                (None, None) => Equal,
            },
            2 => a.tracks.cmp(&b.tracks),
            _ => nulamp_core::playlist::natural_cmp(&a.name, &b.name),
        };
        untagged_last(&a.name, &b.name, if desc { by.reverse() } else { by })
    });
}

fn untagged_last(a: &str, b: &str, by: std::cmp::Ordering) -> std::cmp::Ordering {
    use std::cmp::Ordering::{Greater, Less};
    match (a.is_empty(), b.is_empty()) {
        (true, false) => Greater,
        (false, true) => Less,
        _ => by,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nulamp_library::{NodeKind, TrackId};

    fn view() -> LibraryView {
        LibraryView::new(&[("local".into(), "Local library".into(), Capabilities::SCAN)])
    }

    fn track(i: usize) -> Track {
        Track {
            id: TrackId::new("local", i.to_string()),
            location: format!("/m/{i}.mp3"),
            length_ms: 61_000,
            ..Track::default()
        }
    }

    fn page(offset: usize, n: usize, total: u64) -> TrackPage {
        TrackPage {
            tracks: (offset..offset + n).map(track).collect(),
            total,
            offset: offset as u64,
        }
    }

    fn tracks_requests(reqs: &[Request]) -> Vec<(u64, u64)> {
        reqs.iter()
            .filter_map(|r| match r {
                Request::Tracks {
                    page, generation, ..
                } => Some((*generation, page.offset)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn starts_with_the_first_provider_listed() {
        let mut v = view();
        let reqs = v.take_requests();
        assert!(
            matches!(&reqs[0], Request::Browse { provider, node } if provider == "local" && node.id.is_empty())
        );
        assert!(matches!(&reqs[1], Request::Summary { generation: 1, .. }));
        assert_eq!(tracks_requests(&reqs), [(1, 0)]);
        let (rows, keys, sel) = v.tree_rows();
        assert_eq!(rows[0].label, "Local library");
        assert!(rows[0].expanded);
        assert_eq!(sel, Some(0));
        assert_eq!(keys[0], ("local".to_string(), String::new()));
        assert_eq!(v.status_text(), "Loading…");
    }

    #[test]
    fn paging_requests_only_missing_pages() {
        let mut v = view();
        v.take_requests();
        v.on_page(1, 0, Ok(page(0, PAGE_ROWS, 1000)));
        assert_eq!(v.total, Some(1000));
        assert_eq!(v.track(5).unwrap().location, "/m/5.mp3");
        assert!(v.track(PAGE_ROWS).is_none());
        // Rows 150..180 need page 0 (loaded) and page 1 (ahead).
        v.want_rows(150, 30);
        assert_eq!(tracks_requests(&v.take_requests()), [(1, 200)]);
        v.want_rows(150, 30);
        assert!(v.take_requests().is_empty(), "already requested");
        // Near the end: nothing past the total.
        v.want_rows(950, 40);
        assert_eq!(tracks_requests(&v.take_requests()), [(1, 800)]);
        // Stale responses are dropped.
        v.on_page(0, 400, Ok(page(400, PAGE_ROWS, 1000)));
        assert!(v.track(400).is_none());
        // Loaded rows of a selection.
        assert_eq!(v.loaded_tracks(&[(1, 3)]).unwrap().len(), 2);
        assert!(v.loaded_tracks(&[(1, 3), (500, 501)]).is_none());
        v.on_summary(
            1,
            TrackSummary {
                count: 1000,
                length_ms: 3_725_000,
            },
        );
        v.list_secs = Some(0.0123);
        assert_eq!(v.status_text(), "1000 items [1:02:05] in 0.012 sec.");
    }

    #[test]
    fn many_pages_are_evicted_far_from_the_view() {
        let mut v = view();
        v.take_requests();
        for p in 0..40 {
            v.on_page(
                1,
                (p * PAGE_ROWS) as u64,
                Ok(page(p * PAGE_ROWS, PAGE_ROWS, 8000)),
            );
        }
        v.want_rows(0, 10);
        assert!(v.pages.len() <= MAX_PAGES);
        assert!(v.track(0).is_some());
        assert!(v.track(39 * PAGE_ROWS).is_none());
    }

    #[test]
    fn search_is_debounced() {
        let mut v = view();
        v.take_requests();
        let t0 = Instant::now();
        v.set_search("beat", t0);
        assert!(!v.tick(t0 + Duration::from_millis(100)));
        v.set_search("beatles", t0 + Duration::from_millis(200));
        assert!(
            !v.tick(t0 + Duration::from_millis(400)),
            "typing restarts the wait"
        );
        assert!(v.tick(t0 + Duration::from_millis(450)));
        let reqs = v.take_requests();
        assert!(
            matches!(&reqs[0], Request::Summary { spec, generation: 2 } if spec.query == "beatles")
        );
        assert_eq!(tracks_requests(&reqs), [(2, 0)]);
        assert!(v.list_reset);
        assert!(!v.tick(t0 + Duration::from_secs(2)), "applied once");
        // The same text again does nothing.
        v.set_search("beatles", t0 + Duration::from_secs(3));
        assert!(!v.apply_search());
    }

    #[test]
    fn tree_expand_collapse_and_select() {
        let mut v = view();
        v.take_requests();
        let artists = BrowseNode::new("artists", "Artists", NodeKind::Category, true);
        let recent = BrowseNode::new("recent", "Recently added", NodeKind::SmartList, false)
            .with_sort(Sort::desc(SortKey::DateAdded));
        v.on_children("local", "", Ok(vec![artists.clone(), recent]));
        let (rows, keys, _) = v.tree_rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].depth, 1);
        let a = keys[1].clone();
        v.expand(&a);
        let reqs = v.take_requests();
        assert!(matches!(&reqs[..], [Request::Browse { node, .. }] if node.id == "artists"));
        v.on_children(
            "local",
            "artists",
            Ok(vec![
                BrowseNode::new("artist/x", "X", NodeKind::Artist, true).with_count(3),
            ]),
        );
        let (rows, keys, _) = v.tree_rows();
        assert_eq!(rows[2].label, "X (3)");
        v.select(&keys[2]);
        assert_eq!(v.spec.as_ref().unwrap().node.id, "artist/x");
        let g = v.generation;
        // Collapsing the parent of the selection selects the parent.
        v.collapse(&a);
        assert_eq!(v.selected, Some(a.clone()));
        assert_eq!(v.generation, g + 1);
        assert_eq!(v.tree_rows().0.len(), 3);
        // Re-expanding uses the loaded children.
        v.take_requests();
        v.toggle(&a);
        assert!(v.take_requests().is_empty());
        // Selecting a smart list starts with its own order.
        let recent_key = ("local".to_string(), "recent".to_string());
        v.select(&recent_key);
        assert_eq!(v.sort(), Some(Sort::desc(SortKey::DateAdded)));
    }

    #[test]
    fn events_refresh_throttled_and_keep_rows() {
        let mut v = view();
        v.take_requests();
        v.on_children("local", "", Ok(vec![]));
        v.on_page(1, 0, Ok(page(0, 10, 10)));
        let t0 = Instant::now();
        v.on_event(LibraryEvent::TracksChanged, t0);
        let reqs = v.take_requests();
        assert!(reqs.iter().any(|r| matches!(r, Request::Browse { .. })));
        assert!(v.track(0).is_some(), "rows stay until reloaded");
        v.want_rows(0, 10);
        assert_eq!(tracks_requests(&v.take_requests()), [(2, 0)]);
        // A second event within the interval waits.
        v.on_children("local", "", Ok(vec![]));
        v.on_event(LibraryEvent::TracksChanged, t0 + Duration::from_millis(100));
        assert!(v.take_requests().is_empty());
        assert!(!v.tick(t0 + Duration::from_millis(500)));
        assert!(v.tick(t0 + REFRESH_INTERVAL));
        assert!(!v.take_requests().is_empty());
        assert!(v.on_event(LibraryEvent::ScanProgress { done: 3, total: 9 }, t0));
        assert_eq!(v.status_text(), "Scanning: 3/9 files");
        // The scan result shows for a while.
        let done = nulamp_library::ScanSummary {
            added: 2,
            ..Default::default()
        };
        v.on_event(LibraryEvent::ScanFinished(done), t0);
        assert!(
            v.status_text()
                .ends_with("Scan done: 2 added, 0 updated, 0 removed")
        );
        let t1 = t0 + Duration::from_secs(5);
        v.tick(t1);
        assert!(!v.tick(t1 + MESSAGE_TIME / 2));
        assert!(v.tick(t1 + MESSAGE_TIME));
        assert!(v.message.is_none());
    }

    #[test]
    fn sorting_and_collecting() {
        let mut v = view();
        v.take_requests();
        v.set_sort(Sort::desc(SortKey::Year));
        let reqs = v.take_requests();
        assert!(
            matches!(&reqs[0], Request::Summary { spec, .. } if spec.sort == Sort::desc(SortKey::Year))
        );
        assert_eq!(column_for_sort(SortKey::Year), Some(9));
        assert_eq!(column_for_sort(SortKey::DateAdded), None);
        v.collect(&[(0, 2), (5, 6)], Purpose::Play);
        match &v.take_requests()[..] {
            [
                Request::Collect {
                    ranges, purpose, ..
                },
            ] => {
                assert_eq!(ranges, &[(0, 2), (5, 6)]);
                assert_eq!(*purpose, Purpose::Play);
            }
            other => panic!("{other:?}"),
        }
        let mut t = track(1);
        t.tags.insert("track".into(), "3/12".into());
        t.rating = 4;
        t.play_count = 7;
        t.last_played = Some(1_760_227_200);
        assert_eq!(cell(&t, 2), "3");
        assert_eq!(cell(&t, 3), "1.mp3");
        assert_eq!(cell(&t, 4), "1:01");
        assert_eq!(cell(&t, 6), "★★★★");
        assert_eq!(cell(&t, 7), "7");
        assert!(cell(&t, 8).starts_with("2025-10-1"), "{}", cell(&t, 8));
        assert_eq!(cell(&track(2), 8), "", "never played");
    }

    fn artist(name: &str, albums: u64, tracks: u64) -> ArtistFacet {
        ArtistFacet {
            name: name.into(),
            albums,
            tracks,
        }
    }

    fn album(name: &str, year: Option<i32>, tracks: u64) -> AlbumFacet {
        AlbumFacet {
            name: name.into(),
            year,
            tracks,
        }
    }

    /// Tokens of the facet requests in `reqs`.
    fn facet_tokens(reqs: &[Request]) -> (Option<u64>, Option<u64>) {
        let mut out = (None, None);
        for r in reqs {
            match r {
                Request::Artists { token, .. } => out.0 = Some(*token),
                Request::Albums { token, .. } => out.1 = Some(*token),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn panes_load_with_the_node_and_filter_the_list() {
        let mut v = view();
        let (a, b) = facet_tokens(&v.take_requests());
        let (a, b) = (a.unwrap(), b.unwrap());
        // Answers to older requests are dropped.
        v.on_artists(a + 5, Ok(vec![artist("Stale", 1, 1)]));
        assert!(v.artists.is_empty());
        v.on_artists(
            a,
            Ok(vec![
                artist("The Band", 2, 9),
                artist("", 1, 1),
                artist("Abba", 1, 3),
            ]),
        );
        v.on_albums(
            b,
            Ok(vec![album("One", Some(1999), 4), album("Two", None, 1)]),
        );
        let names: Vec<_> = v.artists.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            ["Abba", "The Band", ""],
            "natural order, untagged last"
        );
        assert_eq!(v.artist_cell(0, 0).unwrap(), "All (3 artists)");
        assert_eq!(v.artist_cell(0, 1).unwrap(), "4");
        assert_eq!(v.artist_cell(0, 2).unwrap(), "13");
        assert_eq!(v.artist_cell(3, 0).unwrap(), "Unknown artist");
        assert_eq!(v.album_cell(0, 0).unwrap(), "All (2 albums)");
        assert_eq!(v.album_cell(1, 1).unwrap(), "1999");
        assert_eq!(v.album_cell(2, 1).unwrap(), "");
        assert!(v.artist_cell(4, 0).is_none());
        assert_eq!((v.artist_row(), v.album_row()), (0, 0));
        // Picking an artist reloads the albums and the list, not the artists.
        v.select_artist(Some("The Band".into()));
        let reqs = v.take_requests();
        let (a2, b2) = facet_tokens(&reqs);
        assert!(a2.is_none());
        assert!(matches!(
            reqs.iter().find(|r| matches!(r, Request::Albums { .. })),
            Some(Request::Albums { spec, .. }) if spec.artist.as_deref() == Some("The Band")
        ));
        assert_eq!(tracks_requests(&reqs), [(v.generation, 0)]);
        assert_eq!(v.artist_row(), 2);
        assert!(
            v.albums.is_empty(),
            "the old albums are not shown for the new artist"
        );
        // The old album answer is stale now.
        v.on_albums(b, Ok(vec![album("Old", None, 1)]));
        assert!(v.albums.is_empty());
        v.on_albums(b2.unwrap(), Ok(vec![album("One", Some(1999), 4)]));
        v.select_album(Some("one".into()));
        assert_eq!(v.album_row(), 1, "names match without regard to case");
        let spec = v.spec.as_ref().unwrap();
        assert_eq!(spec.album.as_deref(), Some("one"));
        // Changing the artist forgets the album; the same pick again does nothing.
        v.take_requests();
        v.select_artist(Some("Abba".into()));
        assert_eq!(v.spec.as_ref().unwrap().album, None);
        v.take_requests();
        v.select_artist(Some("Abba".into()));
        assert!(v.take_requests().is_empty());
        // "All" shows everything again.
        v.select_artist(None);
        assert_eq!(v.spec.as_ref().unwrap().artist, None);
        assert_eq!(v.artist_row(), 0);
    }

    #[test]
    fn a_vanished_pick_falls_back_to_all() {
        let mut v = view();
        let (a, b) = facet_tokens(&v.take_requests());
        v.on_artists(
            a.unwrap(),
            Ok(vec![artist("Abba", 1, 3), artist("Cher", 1, 2)]),
        );
        v.on_albums(b.unwrap(), Ok(vec![album("One", None, 5)]));
        v.select_artist(Some("Cher".into()));
        let (_, b) = facet_tokens(&v.take_requests());
        v.on_albums(b.unwrap(), Ok(vec![album("Believe", Some(1998), 2)]));
        v.select_album(Some("Believe".into()));
        v.take_requests();
        // A search removes Cher: the pick is dropped and the list reloads.
        v.set_search("abba", Instant::now());
        v.apply_search();
        let (a, _) = facet_tokens(&v.take_requests());
        let g = v.generation;
        v.on_artists(a.unwrap(), Ok(vec![artist("Abba", 1, 3)]));
        let spec = v.spec.as_ref().unwrap();
        assert_eq!(
            (spec.artist.as_deref(), spec.album.as_deref()),
            (None, None)
        );
        assert_eq!(v.generation, g + 1);
        assert_eq!(v.artist_row(), 0);
        // A refresh keeps a pick that still exists.
        v.select_artist(Some("Abba".into()));
        v.refresh(Instant::now());
        let (a, _) = facet_tokens(&v.take_requests());
        v.on_artists(a.unwrap(), Ok(vec![artist("Abba", 1, 4)]));
        assert_eq!(v.spec.as_ref().unwrap().artist.as_deref(), Some("Abba"));
    }

    #[test]
    fn panes_sort_by_their_columns() {
        let mut v = view();
        let (a, b) = facet_tokens(&v.take_requests());
        v.on_artists(
            a.unwrap(),
            Ok(vec![artist("B", 1, 2), artist("", 1, 9), artist("A", 3, 1)]),
        );
        v.on_albums(
            b.unwrap(),
            Ok(vec![
                album("x", None, 1),
                album("y", Some(2000), 5),
                album("z", Some(1990), 2),
            ]),
        );
        v.set_artist_sort(1, true);
        let names: Vec<_> = v.artists.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            ["A", "B", ""],
            "most albums first, untagged still last"
        );
        v.set_artist_sort(2, false);
        let names: Vec<_> = v.artists.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["A", "B", ""]);
        v.set_album_sort(1, false);
        let names: Vec<_> = v.albums.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["z", "y", "x"], "unknown years last");
        v.set_album_sort(1, true);
        let names: Vec<_> = v.albums.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["y", "z", "x"]);
        v.set_album_sort(2, true);
        assert_eq!(v.albums[0].name, "y");
        assert!(v.panes_dirty);
    }

    #[test]
    fn status_shows_items_length_and_time() {
        let mut v = view();
        assert_eq!(v.status_text(), "Loading…");
        v.on_page(1, 0, Ok(page(0, 1, 1)));
        assert_eq!(v.status_text(), "1 item");
        v.on_summary(
            1,
            TrackSummary {
                count: 1,
                length_ms: 500,
            },
        );
        v.list_secs = Some(0.0);
        assert_eq!(
            v.status_text(),
            "1 item in 0.000 sec.",
            "under a second: no length"
        );
        v.on_summary(
            1,
            TrackSummary {
                count: 2,
                length_ms: 125_000,
            },
        );
        v.list_secs = Some(1.5);
        assert_eq!(v.status_text(), "2 items [2:05] in 1.500 sec.");
    }
}
