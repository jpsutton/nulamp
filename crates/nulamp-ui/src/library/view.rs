//! State of the media-library window, independent of drawing and input: the provider tree,
//! the paged track listing, the debounced search and the status line. Provider calls are
//! queued as [`Request`]s for the worker; responses come back through the `on_*` methods.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use nulamp_library::{
    BrowseNode, Capabilities, LibraryEvent, Page, Sort, SortKey, Track, TrackPage, TrackSummary,
};
use nulamp_skinui::TreeRow;

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

pub const COLUMNS: [ColumnDef; 9] = [
    col("#", 26, true, SortKey::Track),
    col("Artist", 100, false, SortKey::Artist),
    col("Title", 140, false, SortKey::Title),
    col("Album", 100, false, SortKey::Album),
    col("Length", 44, true, SortKey::Length),
    col("Year", 36, true, SortKey::Year),
    col("Genre", 64, false, SortKey::Genre),
    col("Rating", 50, false, SortKey::Rating),
    col("Plays", 38, true, SortKey::PlayCount),
];

/// The column whose header shows a sort key.
pub fn column_for_sort(key: SortKey) -> Option<usize> {
    COLUMNS.iter().position(|c| c.sort == key)
}

/// Text of a list cell.
pub fn cell(t: &Track, column: usize) -> String {
    let opt = |v: Option<&str>| v.unwrap_or("").to_string();
    match column {
        0 => t.track_number().map(|n| n.to_string()).unwrap_or_default(),
        1 => opt(t.artist()),
        2 => t.display_title(),
        3 => opt(t.album()),
        4 if t.length_ms > 0 => crate::marquee::format_length(t.length_ms / 1000),
        5 => t.year().map(|y| y.to_string()).unwrap_or_default(),
        6 => opt(t.genre()),
        7 => "★".repeat(t.rating.min(5) as usize),
        8 => t.play_count.to_string(),
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

fn flatten(nodes: &[TreeNode], depth: usize, rows: &mut Vec<TreeRow>, keys: &mut Vec<NodeKey>) {
    for n in nodes {
        rows.push(TreeRow {
            depth,
            label: n.label(),
            expandable: n.node.expandable,
            expanded: n.expanded,
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
        let spec = ListSpec {
            provider: n.provider.clone(),
            node: n.node.clone(),
            query: self.search.clone(),
            sort: n.node.default_sort,
        };
        self.selected = Some(key.clone());
        self.spec = Some(spec);
        self.tree_dirty = true;
        self.reset_list();
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
        self.request_summary();
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
        }
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
        let mut s = match (count, self.summary) {
            (None, _) => "Loading…".to_string(),
            (Some(n), Some(sum)) => format!(
                "{n} {}, {}",
                if n == 1 { "track" } else { "tracks" },
                crate::playlist::format_total(sum.length_ms / 1000)
            ),
            (Some(n), None) => format!("{n} {}", if n == 1 { "track" } else { "tracks" }),
        };
        if let Some(m) = &self.message {
            s.push_str("  ·  ");
            s.push_str(m);
        }
        s
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
        assert_eq!(v.status_text(), "1000 tracks, 1:02:05");
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
        assert_eq!(column_for_sort(SortKey::Year), Some(5));
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
        assert_eq!(cell(&t, 0), "3");
        assert_eq!(cell(&t, 2), "1.mp3");
        assert_eq!(cell(&t, 4), "1:01");
        assert_eq!(cell(&t, 7), "★★★★");
    }
}
