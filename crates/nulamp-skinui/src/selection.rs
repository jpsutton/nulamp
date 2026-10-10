//! Multi-selection of list rows, stored as ranges so "select all" stays cheap on long lists.

/// Selected rows as sorted, disjoint, half-open ranges, plus the anchor of Shift ranges and
/// the cursor (the row keyboard moves start from).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    ranges: Vec<(usize, usize)>,
    /// Fixed end of a Shift range.
    pub anchor: Option<usize>,
    /// Focused row.
    pub cursor: Option<usize>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deselects everything and forgets anchor and cursor.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Number of selected rows.
    pub fn count(&self) -> usize {
        self.ranges.iter().map(|(a, b)| b - a).sum()
    }

    pub fn contains(&self, row: usize) -> bool {
        let i = self.ranges.partition_point(|&(_, end)| end <= row);
        self.ranges.get(i).is_some_and(|&(start, _)| start <= row)
    }

    /// The selected ranges, sorted and disjoint (`start..end`).
    pub fn ranges(&self) -> &[(usize, usize)] {
        &self.ranges
    }

    /// The first selected row.
    pub fn first(&self) -> Option<usize> {
        self.ranges.first().map(|r| r.0)
    }

    /// Selected rows in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.ranges.iter().flat_map(|&(a, b)| a..b)
    }

    /// Selects `start..end` in addition to the current selection.
    pub fn add_range(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let (mut s, mut e) = (start, end);
        let mut out = Vec::with_capacity(self.ranges.len() + 1);
        let mut placed = false;
        for &(a, b) in &self.ranges {
            if b < s {
                out.push((a, b));
            } else if a > e {
                if !placed {
                    out.push((s, e));
                    placed = true;
                }
                out.push((a, b));
            } else {
                s = s.min(a);
                e = e.max(b);
            }
        }
        if !placed {
            out.push((s, e));
        }
        self.ranges = out;
    }

    /// Deselects `start..end`.
    pub fn remove_range(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let mut out = Vec::with_capacity(self.ranges.len() + 1);
        for &(a, b) in &self.ranges {
            if b <= start || a >= end {
                out.push((a, b));
                continue;
            }
            if a < start {
                out.push((a, start));
            }
            if b > end {
                out.push((end, b));
            }
        }
        self.ranges = out;
    }

    /// Selects only `row`, which becomes anchor and cursor.
    pub fn set_only(&mut self, row: usize) {
        self.ranges = vec![(row, row + 1)];
        self.anchor = Some(row);
        self.cursor = Some(row);
    }

    /// Flips one row; it becomes anchor and cursor.
    pub fn toggle(&mut self, row: usize) {
        if self.contains(row) {
            self.remove_range(row, row + 1);
        } else {
            self.add_range(row, row + 1);
        }
        self.anchor = Some(row);
        self.cursor = Some(row);
    }

    /// Selects every row of a list of `n` rows.
    pub fn select_all(&mut self, n: usize) {
        self.ranges.clear();
        self.add_range(0, n);
        if n > 0 && self.cursor.is_none() {
            self.cursor = Some(0);
            self.anchor = Some(0);
        }
    }

    /// A click on `row`: plain selects only it, Ctrl toggles it, Shift selects the range
    /// from the anchor (Ctrl+Shift adds the range to the selection).
    pub fn click(&mut self, row: usize, ctrl: bool, shift: bool) {
        match (ctrl, shift) {
            (false, false) => self.set_only(row),
            (true, false) => self.toggle(row),
            (_, true) => {
                let anchor = self.anchor.unwrap_or(row);
                if !ctrl {
                    self.ranges.clear();
                }
                self.add_range(anchor.min(row), anchor.max(row) + 1);
                self.anchor = Some(anchor);
                self.cursor = Some(row);
            }
        }
    }

    /// Keyboard move of the cursor to `row`: selects only it, or with Shift the range from
    /// the anchor.
    pub fn move_cursor(&mut self, row: usize, shift: bool) {
        self.click(row, false, shift);
    }

    /// Drops rows at or after `n` (the list got shorter).
    pub fn clamp(&mut self, n: usize) {
        self.remove_range(n, usize::MAX);
        if self.anchor.is_some_and(|a| a >= n) {
            self.anchor = n.checked_sub(1);
        }
        if self.cursor.is_some_and(|c| c >= n) {
            self.cursor = n.checked_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_merge_and_split() {
        let mut s = Selection::new();
        s.add_range(5, 8);
        s.add_range(1, 3);
        s.add_range(3, 5);
        assert_eq!(s.ranges(), [(1, 8)]);
        s.remove_range(4, 6);
        assert_eq!(s.ranges(), [(1, 4), (6, 8)]);
        assert!(s.contains(3) && !s.contains(4) && s.contains(6) && !s.contains(8));
        assert_eq!(s.count(), 5);
        assert_eq!(s.iter().collect::<Vec<_>>(), [1, 2, 3, 6, 7]);
        s.add_range(10, 12);
        s.add_range(0, 11);
        assert_eq!(s.ranges(), [(0, 12)]);
    }

    #[test]
    fn clicks_with_modifiers() {
        let mut s = Selection::new();
        s.click(3, false, false);
        assert_eq!(s.ranges(), [(3, 4)]);
        s.click(6, false, true);
        assert_eq!(s.ranges(), [(3, 7)]);
        assert_eq!((s.anchor, s.cursor), (Some(3), Some(6)));
        // Shift from the same anchor replaces the range.
        s.click(1, false, true);
        assert_eq!(s.ranges(), [(1, 4)]);
        s.click(10, true, false);
        assert_eq!(s.ranges(), [(1, 4), (10, 11)]);
        s.click(2, true, false);
        assert_eq!(s.ranges(), [(1, 2), (3, 4), (10, 11)]);
        // Ctrl+Shift adds the range from the new anchor.
        s.click(12, true, true);
        assert_eq!(s.ranges(), [(1, 13)]);
        assert_eq!((s.anchor, s.cursor), (Some(2), Some(12)));
    }

    #[test]
    fn select_all_and_clamp() {
        let mut s = Selection::new();
        s.select_all(1_000_000);
        assert_eq!(s.count(), 1_000_000);
        assert_eq!(s.ranges().len(), 1);
        s.clamp(10);
        assert_eq!(s.ranges(), [(0, 10)]);
        s.set_only(9);
        s.clamp(5);
        assert!(s.is_empty());
        assert_eq!(s.cursor, Some(4));
    }
}
