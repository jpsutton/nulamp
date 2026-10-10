//! An in-memory provider shaped like a remote server.
//!
//! [`MockProvider`] answers playback with `http://mock/stream/<id>` URLs plus an
//! `Authorization` header, filters with [`Query::matches`] and sorts with [`Sort::compare`]
//! the way a network provider without server-side search would. Tests use it to check that
//! the provider trait fits non-local sources; it also serves as a template for Jellyfin or
//! Subsonic providers.

use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, MutexGuard};

use crate::error::{Error, Result};
use crate::model::{
    BrowseNode, Capabilities, LibraryEvent, NodeKind, Page, PlaybackLocation, Track, TrackId,
    TrackPage,
};
use crate::provider::{LibraryProvider, Subscribers};
use crate::query::{Query, Sort, SortKey, fold};
use crate::{enc_segment, split_id};

/// A minimal PNG signature returned as artwork.
const ARTWORK: &[u8] = b"\x89PNG\r\n\x1a\nmock";

/// In-memory remote-style provider.
pub struct MockProvider {
    tracks: Mutex<Vec<Track>>,
    events: Subscribers,
    token: String,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl MockProvider {
    /// Provider id, used in [`TrackId::provider`].
    pub const ID: &'static str = "mock";

    /// A provider serving `tracks`; their ids are rewritten to this provider's.
    pub fn new(tracks: Vec<Track>) -> Self {
        let tracks = tracks
            .into_iter()
            .enumerate()
            .map(|(i, mut t)| {
                if t.id.id.is_empty() {
                    t.id.id = (i + 1).to_string();
                }
                t.id.provider = Self::ID.to_string();
                if t.location.is_empty() {
                    t.location = format!("http://mock/stream/{}", t.id.id);
                }
                t
            })
            .collect();
        Self {
            tracks: Mutex::new(tracks),
            events: Subscribers::default(),
            token: "mock-token".to_string(),
        }
    }

    /// A small fixed catalogue: two artists, three albums, five tracks.
    pub fn sample() -> Self {
        let mk = |artist: &str, album: &str, n: u32, title: &str, year: &str| {
            let mut tags = BTreeMap::new();
            for (k, v) in [
                ("artist", artist),
                ("album", album),
                ("title", title),
                ("year", year),
                ("genre", "Test"),
            ] {
                tags.insert(k.to_string(), v.to_string());
            }
            tags.insert("track".to_string(), n.to_string());
            Track {
                tags,
                length_ms: 180_000 + u64::from(n) * 1000,
                codec: Some("FLAC".into()),
                ..Track::default()
            }
        };
        Self::new(vec![
            mk("Alpha", "First", 1, "Opening", "2001"),
            mk("Alpha", "First", 2, "Middle", "2001"),
            mk("Alpha", "Second", 1, "Return", "2004"),
            mk("Beta", "Gamma Rays", 1, "Ray One", "1999"),
            mk("Beta", "Gamma Rays", 10, "Ray Ten", "1999"),
        ])
    }

    /// The bearer token sent with playback requests.
    pub fn token(&self) -> &str {
        &self.token
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Track>> {
        self.tracks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with_track<T>(&self, id: &TrackId, f: impl FnOnce(&mut Track) -> T) -> Result<T> {
        if id.provider != Self::ID {
            return Err(Error::InvalidArgument(format!("not a {} track", Self::ID)));
        }
        let mut tracks = self.lock();
        let t = tracks
            .iter_mut()
            .find(|t| t.id == *id)
            .ok_or_else(|| Error::NotFound(format!("track {}", id.id)))?;
        Ok(f(t))
    }

    fn group(t: &Track) -> String {
        t.group_artist().unwrap_or("").to_string()
    }

    /// Whether `t` is below the node with segments `seg`.
    fn in_node(seg: &[String], t: &Track) -> Result<bool> {
        let s: Vec<&str> = seg.iter().map(String::as_str).collect();
        Ok(match s[..] {
            [""] | ["artists"] => true,
            ["most-played"] => t.play_count > 0,
            ["artist", a] => fold(&Self::group(t)) == fold(a),
            ["album", a, b] => {
                fold(&Self::group(t)) == fold(a) && fold(t.album().unwrap_or("")) == fold(b)
            }
            _ => return Err(Error::NotFound(format!("browse node `{}`", seg.join("/")))),
        })
    }
}

impl LibraryProvider for MockProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn name(&self) -> &str {
        "Mock server"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::REMOTE
            | Capabilities::WRITE_RATING
            | Capabilities::RECORD_PLAY
            | Capabilities::ARTWORK
    }

    fn browse(&self, node: &BrowseNode) -> Result<Vec<BrowseNode>> {
        let seg = split_id(&node.id);
        let s: Vec<&str> = seg.iter().map(String::as_str).collect();
        let tracks = self.lock();
        let mut groups: BTreeMap<String, (String, u64)> = BTreeMap::new();
        let out = match s[..] {
            [""] => vec![
                BrowseNode::new("artists", "Artists", NodeKind::Category, true),
                BrowseNode::new("most-played", "Most played", NodeKind::SmartList, false)
                    .with_sort(Sort::desc(SortKey::PlayCount)),
            ],
            ["artists"] => {
                for t in tracks.iter() {
                    let g = Self::group(t);
                    groups.entry(fold(&g)).or_insert((g, 0)).1 += 1;
                }
                groups
                    .into_values()
                    .map(|(g, n)| {
                        BrowseNode::new(
                            format!("artist/{}", enc_segment(&g)),
                            g,
                            NodeKind::Artist,
                            true,
                        )
                        .with_count(n)
                    })
                    .collect()
            }
            ["artist", a] => {
                for t in tracks.iter().filter(|t| fold(&Self::group(t)) == fold(a)) {
                    let album = t.album().unwrap_or("").to_string();
                    groups.entry(fold(&album)).or_insert((album, 0)).1 += 1;
                }
                groups
                    .into_values()
                    .map(|(b, n)| {
                        let id = format!("album/{}/{}", enc_segment(a), enc_segment(&b));
                        BrowseNode::new(id, b, NodeKind::Album, false)
                            .with_count(n)
                            .with_sort(Sort::asc(SortKey::Track))
                    })
                    .collect()
            }
            ["most-played"] | ["album", _, _] => Vec::new(),
            _ => return Err(Error::NotFound(format!("browse node `{}`", node.id))),
        };
        Ok(out)
    }

    fn tracks(
        &self,
        node: &BrowseNode,
        query: &Query,
        sort: Sort,
        page: Page,
    ) -> Result<TrackPage> {
        let seg = split_id(&node.id);
        let mut list = Vec::new();
        for t in self.lock().iter() {
            if Self::in_node(&seg, t)? && query.matches(t) {
                list.push(t.clone());
            }
        }
        list.sort_by(|a, b| sort.compare(a, b));
        let total = list.len() as u64;
        let tracks = list
            .into_iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(usize::MAX))
            .collect();
        Ok(TrackPage {
            tracks,
            total,
            offset: page.offset,
        })
    }

    fn track(&self, id: &TrackId) -> Result<Option<Track>> {
        match self.with_track(id, |t| t.clone()) {
            Ok(t) => Ok(Some(t)),
            Err(Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn resolve_playback(&self, id: &TrackId) -> Result<PlaybackLocation> {
        self.with_track(id, |t| PlaybackLocation {
            location: format!("http://mock/stream/{}", t.id.id),
            headers: vec![(
                "Authorization".to_string(),
                format!("Bearer {}", self.token),
            )],
        })
    }

    fn artwork(&self, id: &TrackId) -> Result<Option<(String, Vec<u8>)>> {
        self.with_track(id, |_| Some(("image/png".to_string(), ARTWORK.to_vec())))
    }

    fn set_rating(&self, id: &TrackId, stars: u8) -> Result<()> {
        if stars > 5 {
            return Err(Error::InvalidArgument(format!("rating {stars} is not 0–5")));
        }
        self.with_track(id, |t| t.rating = stars)?;
        self.events.send(LibraryEvent::TracksChanged);
        Ok(())
    }

    fn record_play(&self, id: &TrackId, _played_ms: u64) -> Result<()> {
        self.with_track(id, |t| {
            t.play_count += 1;
            t.last_played = Some(crate::now_secs());
        })?;
        self.events.send(LibraryEvent::TracksChanged);
        Ok(())
    }

    fn subscribe(&self) -> Receiver<LibraryEvent> {
        self.events.subscribe()
    }
}
