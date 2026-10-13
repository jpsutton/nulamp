//! Artist and album groups computed from track lists: the default for providers without a
//! cheaper query.

use std::collections::{BTreeMap, HashSet};

use crate::model::{AlbumFacet, ArtistFacet, Track};
use crate::query::fold;

/// Name order of the panes: natural order, untagged (`""`) last.
fn name_key(name: &str) -> (bool, String) {
    (name.is_empty(), fold(name))
}

/// Artists of `tracks` by their `artist` tag.
pub(crate) fn artists(tracks: &[Track]) -> Vec<ArtistFacet> {
    let mut groups: BTreeMap<(bool, String), (String, HashSet<String>, u64)> = BTreeMap::new();
    for t in tracks {
        let name = t.artist().unwrap_or("");
        let g = groups
            .entry(name_key(name))
            .or_insert_with(|| (name.to_string(), HashSet::new(), 0));
        g.1.insert(fold(t.album().unwrap_or("")));
        g.2 += 1;
    }
    groups
        .into_values()
        .map(|(name, albums, tracks)| ArtistFacet {
            name,
            albums: albums.len() as u64,
            tracks,
        })
        .collect()
}

/// Albums of `tracks` by their `album` tag.
pub(crate) fn albums(tracks: &[Track]) -> Vec<AlbumFacet> {
    let mut groups: BTreeMap<(bool, String), AlbumFacet> = BTreeMap::new();
    for t in tracks {
        let name = t.album().unwrap_or("");
        let g = groups.entry(name_key(name)).or_insert_with(|| AlbumFacet {
            name: name.to_string(),
            year: None,
            tracks: 0,
        });
        g.year = match (g.year, t.year()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        g.tracks += 1;
    }
    groups.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackId;

    fn t(artist: &str, album: &str, year: &str) -> Track {
        let mut t = Track {
            id: TrackId::new("x", "1"),
            ..Track::default()
        };
        for (k, v) in [("artist", artist), ("album", album), ("year", year)] {
            if !v.is_empty() {
                t.tags.insert(k.into(), v.into());
            }
        }
        t
    }

    #[test]
    fn groups_fold_case_and_put_untagged_last() {
        let tracks = [
            t("", "Z", ""),
            t("beta", "One", "2001"),
            t("Beta", "Two", "1999"),
            t("Alpha", "One", ""),
        ];
        let a = artists(&tracks);
        let names: Vec<_> = a
            .iter()
            .map(|f| (f.name.as_str(), f.albums, f.tracks))
            .collect();
        assert_eq!(names, [("Alpha", 1, 1), ("beta", 2, 2), ("", 1, 1)]);
        let b = albums(&tracks);
        let names: Vec<_> = b
            .iter()
            .map(|f| (f.name.as_str(), f.year, f.tracks))
            .collect();
        assert_eq!(
            names,
            [
                ("One", Some(2001), 2),
                ("Two", Some(1999), 1),
                ("Z", None, 1)
            ]
        );
    }
}
