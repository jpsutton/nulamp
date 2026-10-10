//! Playlist model (10 §1) and list operations (04 §9).

use std::cmp::Ordering;

/// Stable identity of an entry; survives reordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: EntryId,
    /// Absolute path, URL or `cda://` location.
    pub location: String,
    /// Display title (placeholder until metadata is read).
    pub title: String,
    /// Length in milliseconds; `None` when unknown.
    pub length_ms: Option<u64>,
    /// Title and length are final.
    pub metadata_read: bool,
    pub selected: bool,
    /// Extra plays requested by the playlist file (`Repeat<n>`).
    pub repeat: u32,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    /// `#EXTINF` attributes and other extras kept for saving.
    pub extras: Vec<(String, String)>,
}

/// Data for a new entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewEntry {
    pub location: String,
    pub title: Option<String>,
    pub length_ms: Option<u64>,
    pub repeat: u32,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub extras: Vec<(String, String)>,
}

impl NewEntry {
    pub fn new(location: impl Into<String>) -> Self {
        Self {
            location: location.into().trim().to_string(),
            ..Default::default()
        }
    }
}

/// Placeholder title before metadata is read (04 §10): file name without directories and
/// extension; whole URL for URLs.
pub fn placeholder_title(location: &str) -> String {
    if is_url(location) {
        return location.to_string();
    }
    let name = location.rsplit(['/', '\\']).next().unwrap_or(location);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
}

/// Whether a location is a URL (has a scheme of two or more letters).
pub fn is_url(location: &str) -> bool {
    match location.find("://") {
        Some(i) => {
            i >= 2
                && location[..i]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        }
        None => false,
    }
}

/// Title clean-up (10 §5.4).
pub fn clean_title(title: &str, underscore: bool, percent20: bool) -> String {
    let mut t = title.to_string();
    if percent20 {
        t = t.replace("%20", " ");
    }
    if underscore {
        t = t.replace('_', " ");
    }
    t
}

#[derive(Debug, Clone, Default)]
pub struct Playlist {
    entries: Vec<Entry>,
    next_id: u64,
    /// The playing/current entry, tracked by identity.
    current: Option<EntryId>,
    /// Index the current entry had; used after it was removed.
    current_index: usize,
    /// The current entry was removed while playing (04 §3.3).
    pub current_removed: bool,
    /// Changed since the last save.
    pub modified: bool,
    /// Bumped on every structural change (shuffle order rebuild trigger).
    pub generation: u64,
    /// Anchor row for shift-selection.
    pub anchor: Option<usize>,
}

impl Playlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, i: usize) -> Option<&Entry> {
        self.entries.get(i)
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut Entry> {
        self.entries.get_mut(i)
    }

    /// Selected entries.
    pub fn entries_mut_selected(&mut self) -> impl Iterator<Item = &mut Entry> {
        self.entries.iter_mut().filter(|e| e.selected)
    }

    /// Entries whose location equals `location`.
    pub fn entries_mut_by_location<'a>(
        &'a mut self,
        location: &'a str,
    ) -> impl Iterator<Item = &'a mut Entry> + 'a {
        self.entries
            .iter_mut()
            .filter(move |e| e.location == location)
    }

    pub fn index_of(&self, id: EntryId) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    pub fn by_id(&self, id: EntryId) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn by_id_mut(&mut self, id: EntryId) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    fn changed(&mut self) {
        self.modified = true;
        self.generation += 1;
    }

    /// Current position. When the current entry was removed, the index it had (clamped).
    pub fn current_index(&self) -> usize {
        match self.current.and_then(|id| self.index_of(id)) {
            Some(i) => i,
            None => self.current_index.min(self.entries.len().saturating_sub(1)),
        }
    }

    pub fn current_id(&self) -> Option<EntryId> {
        self.current.filter(|id| self.index_of(*id).is_some())
    }

    pub fn current(&self) -> Option<&Entry> {
        self.current_id().and_then(|id| self.by_id(id))
    }

    /// Makes entry `i` current.
    pub fn set_current(&mut self, i: usize) {
        if let Some(e) = self.entries.get(i) {
            self.current = Some(e.id);
            self.current_index = i;
            self.current_removed = false;
        } else if self.entries.is_empty() {
            self.current = None;
            self.current_index = 0;
        }
    }

    /// Inserts entries at `at` (clamped). Returns the new entries' ids.
    pub fn insert(&mut self, at: usize, items: impl IntoIterator<Item = NewEntry>) -> Vec<EntryId> {
        let at = at.min(self.entries.len());
        let new: Vec<Entry> = items
            .into_iter()
            .map(|n| {
                self.next_id += 1;
                Entry {
                    id: EntryId(self.next_id),
                    title: n
                        .title
                        .clone()
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| placeholder_title(&n.location)),
                    metadata_read: false,
                    length_ms: n.length_ms,
                    location: n.location,
                    selected: false,
                    repeat: n.repeat,
                    start_ms: n.start_ms,
                    end_ms: n.end_ms,
                    extras: n.extras,
                }
            })
            .collect();
        let ids = new.iter().map(|e| e.id).collect();
        let count = new.len();
        self.entries.splice(at..at, new);
        if self.current.is_none() {
            if self.current_removed {
                if at <= self.current_index && self.entries.len() > count {
                    self.current_index += count;
                }
            } else if !self.entries.is_empty() {
                let i = self.current_index.min(self.entries.len() - 1);
                self.current = Some(self.entries[i].id);
                self.current_index = i;
            }
        }
        self.changed();
        ids
    }

    /// Appends entries.
    pub fn append(&mut self, items: impl IntoIterator<Item = NewEntry>) -> Vec<EntryId> {
        let n = self.entries.len();
        self.insert(n, items)
    }

    /// Removes entries whose index satisfies `pred`. Returns how many were removed.
    pub fn remove_where(&mut self, mut pred: impl FnMut(usize, &Entry) -> bool) -> usize {
        let before = self.entries.len();
        let cur = self.current_index();
        let mut i = 0;
        let mut removed_before_cur = 0;
        let mut cur_removed = false;
        self.entries.retain(|e| {
            let remove = pred(i, e);
            if remove {
                if i < cur {
                    removed_before_cur += 1;
                } else if i == cur && Some(e.id) == self.current {
                    cur_removed = true;
                }
            }
            i += 1;
            !remove
        });
        let removed = before - self.entries.len();
        if removed > 0 {
            if cur_removed {
                self.current_removed = true;
                self.current = None;
            }
            self.current_index = cur - removed_before_cur;
            self.anchor = None;
            self.changed();
        }
        removed
    }

    pub fn remove_selected(&mut self) -> usize {
        self.remove_where(|_, e| e.selected)
    }

    /// Deletes every unselected entry when at least one is selected (04 §9).
    pub fn crop(&mut self) -> usize {
        if !self.entries.iter().any(|e| e.selected) {
            return 0;
        }
        self.remove_where(|_, e| !e.selected)
    }

    /// Deletes all entries. The playing track continues (marked removed).
    pub fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.current_removed = self.current.is_some();
        }
        self.entries.clear();
        self.current = None;
        self.current_index = 0;
        self.anchor = None;
        self.changed();
    }

    /// Removes later entries whose location matches an earlier one.
    pub fn remove_duplicates(&mut self, case_insensitive: bool) -> usize {
        let mut seen = std::collections::HashSet::new();
        self.remove_where(|_, e| {
            let key = if case_insensitive {
                e.location.to_lowercase()
            } else {
                e.location.clone()
            };
            !seen.insert(key)
        })
    }

    // ----- Selection -----

    pub fn select_all(&mut self, on: bool) {
        for e in &mut self.entries {
            e.selected = on;
        }
    }

    pub fn invert_selection(&mut self) {
        for e in &mut self.entries {
            e.selected = !e.selected;
        }
    }

    pub fn selected_count(&self) -> usize {
        self.entries.iter().filter(|e| e.selected).count()
    }

    pub fn first_selected(&self) -> Option<usize> {
        self.entries.iter().position(|e| e.selected)
    }

    /// Selects `a..=b` (order-independent).
    pub fn select_range(&mut self, a: usize, b: usize) {
        let (lo, hi) = (a.min(b), a.max(b));
        for (i, e) in self.entries.iter_mut().enumerate() {
            if i >= lo && i <= hi {
                e.selected = true;
            }
        }
    }

    // ----- Reordering -----

    /// Moves every selected entry by one position (`up` = towards index 0). Does nothing when
    /// the selection touches the edge. Returns whether anything moved.
    pub fn move_selected(&mut self, up: bool) -> bool {
        let n = self.entries.len();
        if n == 0 {
            return false;
        }
        if up {
            if self.entries[0].selected {
                return false;
            }
            for i in 1..n {
                if self.entries[i].selected {
                    self.entries.swap(i - 1, i);
                }
            }
        } else {
            if self.entries[n - 1].selected {
                return false;
            }
            for i in (0..n - 1).rev() {
                if self.entries[i].selected {
                    self.entries.swap(i, i + 1);
                }
            }
        }
        self.changed();
        true
    }

    fn reorder(&mut self, f: impl FnOnce(&mut Vec<Entry>)) {
        f(&mut self.entries);
        if let Some(i) = self.current.and_then(|id| self.index_of(id)) {
            self.current_index = i;
        }
        self.anchor = None;
        self.changed();
    }

    pub fn reverse(&mut self) {
        self.reorder(|v| v.reverse());
    }

    /// Uniform random permutation.
    pub fn randomize(&mut self, rng: &mut impl rand::Rng) {
        self.reorder(|v| {
            for i in (1..v.len()).rev() {
                let j = rng.random_range(0..=i);
                v.swap(i, j);
            }
        });
    }

    pub fn sort_by_title(&mut self) {
        self.reorder(|v| v.sort_by(|a, b| natural_cmp(&a.title, &b.title)));
    }

    pub fn sort_by_file_name(&mut self) {
        self.reorder(|v| {
            v.sort_by(|a, b| natural_cmp(file_part(&a.location), file_part(&b.location)))
        });
    }

    pub fn sort_by_path(&mut self) {
        self.reorder(|v| {
            v.sort_by(|a, b| {
                let (da, db) = (dir_part(&a.location), dir_part(&b.location));
                match (da.is_empty(), db.is_empty()) {
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => natural_cmp(da, db)
                        .then_with(|| natural_cmp(file_part(&a.location), file_part(&b.location))),
                }
            })
        });
    }
}

fn file_part(loc: &str) -> &str {
    loc.rsplit(['/', '\\']).next().unwrap_or(loc)
}

fn dir_part(loc: &str) -> &str {
    match loc.rfind(['/', '\\']) {
        Some(i) => &loc[..i],
        None => "",
    }
}

/// Natural ("logical") order (04 §9): digit runs compare numerically, letters compare
/// case-insensitively; where one string has a digit and the other does not, the digit sorts
/// first.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (ca, cb) = (a[i], b[j]);
        match (ca.is_ascii_digit(), cb.is_ascii_digit()) {
            (true, true) => {
                let si = i;
                while i < a.len() && a[i].is_ascii_digit() {
                    i += 1;
                }
                let sj = j;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                let na: String = a[si..i]
                    .iter()
                    .collect::<String>()
                    .trim_start_matches('0')
                    .to_string();
                let nb: String = b[sj..j]
                    .iter()
                    .collect::<String>()
                    .trim_start_matches('0')
                    .to_string();
                let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
                if ord != Ordering::Equal {
                    return ord;
                }
                let ord = (i - si).cmp(&(j - sj));
                if ord != Ordering::Equal {
                    return ord.reverse();
                }
            }
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {
                let la: String = ca.to_lowercase().collect();
                let lb: String = cb.to_lowercase().collect();
                let ord = la.cmp(&lb);
                if ord != Ordering::Equal {
                    return ord;
                }
                i += 1;
                j += 1;
            }
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(names: &[&str]) -> Playlist {
        let mut p = Playlist::new();
        p.append(
            names
                .iter()
                .map(|n| NewEntry::new(format!("/music/{n}.mp3"))),
        );
        p
    }

    #[test]
    fn placeholder_titles() {
        assert_eq!(placeholder_title("/a/b/Song Name.mp3"), "Song Name");
        assert_eq!(placeholder_title("C:\\x\\y.flac"), "y");
        assert_eq!(
            placeholder_title("http://radio/stream"),
            "http://radio/stream"
        );
        assert_eq!(clean_title("a_b%20c", true, true), "a b c");
    }

    #[test]
    fn current_follows_identity() {
        let mut p = list(&["c", "a", "b"]);
        p.set_current(0);
        p.sort_by_title();
        assert_eq!(p.current().unwrap().title, "c");
        assert_eq!(p.current_index(), 2);
    }

    #[test]
    fn removing_current_marks_removed() {
        let mut p = list(&["a", "b", "c"]);
        p.set_current(1);
        p.get_mut(1).unwrap().selected = true;
        p.remove_selected();
        assert!(p.current_removed);
        assert!(p.current_id().is_none());
        assert_eq!(p.current_index(), 1);
    }

    #[test]
    fn crop_and_move() {
        let mut p = list(&["a", "b", "c", "d"]);
        assert_eq!(p.crop(), 0);
        p.get_mut(1).unwrap().selected = true;
        p.get_mut(2).unwrap().selected = true;
        assert!(p.move_selected(true));
        let t: Vec<_> = p.entries().iter().map(|e| e.title.as_str()).collect();
        assert_eq!(t, ["b", "c", "a", "d"]);
        assert!(!p.move_selected(true));
        assert_eq!(p.crop(), 2);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["track10", "Track2", "track1", "2 intro", "alpha"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["2 intro", "alpha", "track1", "Track2", "track10"]);
    }

    #[test]
    fn sort_by_path_dirless_first() {
        let mut p = Playlist::new();
        p.append(["/b/x.mp3", "y.mp3", "/a/z.mp3"].map(NewEntry::new));
        p.sort_by_path();
        let l: Vec<_> = p.entries().iter().map(|e| e.location.as_str()).collect();
        assert_eq!(l, ["y.mp3", "/a/z.mp3", "/b/x.mp3"]);
    }
}
