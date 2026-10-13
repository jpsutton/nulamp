//! Browse tree and track listing of the local provider.
//!
//! Node ids: `""` (root); `artists`, `albums`, `genres`, `years`, `recent`, `most-played`,
//! `never-played`, `top-rated`, `playlists`; `artist/<name>`, `album/<artist>/<album>`,
//! `genre/<name>`, `year/<n>` (empty `n`: unknown), `playlist/<id>`. Segments are escaped
//! with [`crate::enc_segment`].

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params_from_iter};

use crate::error::{Error, Result};
use crate::model::{
    AlbumFacet, ArtistFacet, BrowseNode, NodeKind, Page, Track, TrackPage, TrackSummary,
};
use crate::query::{Query, Sort, SortKey, fold};
use crate::{enc_segment, split_id};

use super::db::{TRACK_COLS, row_to_track};

/// Artist the tree groups by: album artist, else artist (decision ML4).
const GROUP_ARTIST: &str = "COALESCE(NULLIF(t.albumartist, ''), t.artist, '')";

/// "Recently added" covers this many seconds before the newest addition.
const RECENT_WINDOW: i64 = 30 * 24 * 3600;

const UNKNOWN_ARTIST: &str = "Unknown artist";
const UNKNOWN_ALBUM: &str = "Unknown album";
const UNKNOWN_GENRE: &str = "Unknown genre";
const UNKNOWN_YEAR: &str = "Unknown year";

enum Node {
    Root,
    Artists,
    Albums,
    Genres,
    Years,
    Recent,
    MostPlayed,
    NeverPlayed,
    TopRated,
    Playlists,
    Artist(String),
    Album(String, String),
    Genre(String),
    Year(Option<i64>),
    Playlist(i64),
}

fn parse(id: &str) -> Result<Node> {
    let bad = || Error::NotFound(format!("browse node `{id}`"));
    Ok(match id {
        "" => Node::Root,
        "artists" => Node::Artists,
        "albums" => Node::Albums,
        "genres" => Node::Genres,
        "years" => Node::Years,
        "recent" => Node::Recent,
        "most-played" => Node::MostPlayed,
        "never-played" => Node::NeverPlayed,
        "top-rated" => Node::TopRated,
        "playlists" => Node::Playlists,
        _ => {
            let seg = split_id(id);
            match seg.iter().map(String::as_str).collect::<Vec<_>>()[..] {
                ["artist", a] => Node::Artist(a.to_string()),
                ["album", a, b] => Node::Album(a.to_string(), b.to_string()),
                ["genre", g] => Node::Genre(g.to_string()),
                ["year", ""] => Node::Year(None),
                ["year", y] => Node::Year(Some(y.parse().map_err(|_| bad())?)),
                ["playlist", p] => Node::Playlist(p.parse().map_err(|_| bad())?),
                _ => return Err(bad()),
            }
        }
    })
}

fn or_unknown(s: &str, unknown: &str) -> String {
    if s.is_empty() {
        unknown.to_string()
    } else {
        s.to_string()
    }
}

/// The fixed top level.
fn root_children() -> Vec<BrowseNode> {
    let cat = |id, label| BrowseNode::new(id, label, NodeKind::Category, true);
    let smart =
        |id, label, sort| BrowseNode::new(id, label, NodeKind::SmartList, false).with_sort(sort);
    vec![
        cat("artists", "Artists"),
        cat("albums", "Albums"),
        cat("genres", "Genres"),
        cat("years", "Years"),
        smart("recent", "Recently added", Sort::desc(SortKey::DateAdded)),
        smart("most-played", "Most played", Sort::desc(SortKey::PlayCount)),
        smart("never-played", "Never played", Sort::asc(SortKey::Artist)),
        smart("top-rated", "Top rated", Sort::desc(SortKey::Rating)),
        cat("playlists", "Playlists").with_sort(Sort::asc(SortKey::Position)),
    ]
}

fn album_node(artist: &str, album: &str, detail: Option<String>, count: u64) -> BrowseNode {
    BrowseNode::new(
        format!("album/{}/{}", enc_segment(artist), enc_segment(album)),
        or_unknown(album, UNKNOWN_ALBUM),
        NodeKind::Album,
        false,
    )
    .with_detail(detail)
    .with_count(count)
    .with_sort(Sort::asc(SortKey::Track))
}

fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

/// Children of a node.
pub(crate) fn children(conn: &Connection, node: &BrowseNode) -> Result<Vec<BrowseNode>> {
    let mut out = Vec::new();
    match parse(&node.id)? {
        Node::Root => out = root_children(),
        Node::Artists => {
            let mut st = conn.prepare(&format!(
                "SELECT MIN(g) AS name, COUNT(*) FROM (SELECT {GROUP_ARTIST} AS g FROM tracks t)
                 GROUP BY nl_fold(g) ORDER BY (name = '') ASC, name COLLATE nl_natural"
            ))?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (name, n) = row?;
                out.push(
                    BrowseNode::new(
                        format!("artist/{}", enc_segment(&name)),
                        or_unknown(&name, UNKNOWN_ARTIST),
                        NodeKind::Artist,
                        true,
                    )
                    .with_count(count(n)),
                );
            }
        }
        Node::Artist(artist) => {
            let mut st = conn.prepare(&format!(
                "SELECT MIN(COALESCE(t.album, '')) AS name, COUNT(*), MIN(t.year_num)
                 FROM tracks t WHERE nl_fold({GROUP_ARTIST}) = ?
                 GROUP BY nl_fold(COALESCE(t.album, ''))
                 ORDER BY (name = '') ASC, name COLLATE nl_natural"
            ))?;
            let rows = st.query_map([fold(&artist)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })?;
            for row in rows {
                let (album, n, year) = row?;
                out.push(album_node(
                    &artist,
                    &album,
                    year.map(|y| y.to_string()),
                    count(n),
                ));
            }
        }
        Node::Albums => {
            let mut st = conn.prepare(&format!(
                "SELECT MIN(COALESCE(t.album, '')) AS name, MIN({GROUP_ARTIST}) AS artist, COUNT(*)
                 FROM tracks t
                 GROUP BY nl_fold(COALESCE(t.album, '')), nl_fold({GROUP_ARTIST})
                 ORDER BY (name = '') ASC, name COLLATE nl_natural, artist COLLATE nl_natural"
            ))?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (album, artist, n) = row?;
                let detail = Some(or_unknown(&artist, UNKNOWN_ARTIST));
                out.push(album_node(&artist, &album, detail, count(n)));
            }
        }
        Node::Genres => {
            let mut st = conn.prepare(
                "SELECT MIN(COALESCE(t.genre, '')) AS name, COUNT(*) FROM tracks t
                 GROUP BY nl_fold(COALESCE(t.genre, ''))
                 ORDER BY (name = '') ASC, name COLLATE nl_natural",
            )?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (name, n) = row?;
                out.push(
                    BrowseNode::new(
                        format!("genre/{}", enc_segment(&name)),
                        or_unknown(&name, UNKNOWN_GENRE),
                        NodeKind::Genre,
                        false,
                    )
                    .with_count(count(n)),
                );
            }
        }
        Node::Years => {
            let mut st = conn.prepare(
                "SELECT t.year_num, COUNT(*) FROM tracks t GROUP BY t.year_num
                 ORDER BY (t.year_num IS NULL) ASC, t.year_num DESC",
            )?;
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (year, n) = row?;
                let (id, label) = match year {
                    Some(y) => (format!("year/{y}"), y.to_string()),
                    None => ("year/".to_string(), UNKNOWN_YEAR.to_string()),
                };
                out.push(BrowseNode::new(id, label, NodeKind::Year, false).with_count(count(n)));
            }
        }
        Node::Playlists => out.extend(playlists(conn)?.into_iter().map(playlist_node)),
        Node::Recent
        | Node::MostPlayed
        | Node::NeverPlayed
        | Node::TopRated
        | Node::Album(..)
        | Node::Genre(_)
        | Node::Year(_)
        | Node::Playlist(_) => {}
    }
    Ok(out)
}

/// The browse node of a stored playlist.
pub(crate) fn playlist_node(p: crate::PlaylistInfo) -> BrowseNode {
    BrowseNode::new(
        format!("playlist/{}", p.id),
        p.name,
        NodeKind::Playlist,
        false,
    )
    .with_count(p.track_count)
    .with_sort(Sort::asc(SortKey::Position))
}

/// Every stored playlist, sorted by name.
pub(crate) fn playlists(conn: &Connection) -> Result<Vec<crate::PlaylistInfo>> {
    let mut st = conn.prepare(
        "SELECT p.id, p.name, (SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id = p.id)
         FROM playlists p ORDER BY p.name COLLATE nl_natural, p.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok(crate::PlaylistInfo {
            id: r.get::<_, i64>(0)?.to_string(),
            name: r.get(1)?,
            track_count: count(r.get(2)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Which tracks a node lists.
enum Scope {
    All,
    Nothing,
    Where(String, Vec<Value>),
    Playlist(i64),
}

fn scope(node: &Node) -> Scope {
    let text = |s: &str| Value::Text(fold(s));
    match node {
        Node::Root | Node::Artists | Node::Albums | Node::Genres | Node::Years => Scope::All,
        Node::Playlists => Scope::Nothing,
        Node::Artist(a) => Scope::Where(format!("nl_fold({GROUP_ARTIST}) = ?"), vec![text(a)]),
        Node::Album(a, b) => Scope::Where(
            format!("nl_fold({GROUP_ARTIST}) = ? AND nl_fold(COALESCE(t.album, '')) = ?"),
            vec![text(a), text(b)],
        ),
        Node::Genre(g) => Scope::Where("nl_fold(COALESCE(t.genre, '')) = ?".into(), vec![text(g)]),
        Node::Year(Some(y)) => Scope::Where("t.year_num = ?".into(), vec![Value::Integer(*y)]),
        Node::Year(None) => Scope::Where("t.year_num IS NULL".into(), vec![]),
        Node::Recent => Scope::Where(
            "t.date_added >= (SELECT MAX(date_added) FROM tracks) - ?".into(),
            vec![Value::Integer(RECENT_WINDOW)],
        ),
        Node::MostPlayed => Scope::Where("t.play_count > 0".into(), vec![]),
        Node::NeverPlayed => Scope::Where("t.play_count = 0".into(), vec![]),
        Node::TopRated => Scope::Where("t.rating >= 4".into(), vec![]),
        Node::Playlist(id) => Scope::Playlist(*id),
    }
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The `FROM` clause, `WHERE` filter and parameters that select the tracks below a node
/// matching a query.
struct Selection {
    from: &'static str,
    filter: String,
    params: Vec<Value>,
    playlist: bool,
}

/// `None` when the node has no tracks.
fn select(node: &Node, query: &Query) -> Option<Selection> {
    let mut params = Vec::new();
    let mut clauses = Vec::new();
    let mut from = "tracks t";
    let mut playlist = false;
    match scope(node) {
        Scope::All => {}
        Scope::Nothing => return None,
        Scope::Where(sql, p) => {
            clauses.push(sql);
            params.extend(p);
        }
        Scope::Playlist(id) => {
            from = "playlist_items pi JOIN tracks t ON t.id = pi.track_id";
            clauses.push("pi.playlist_id = ?".into());
            params.push(Value::Integer(id));
            playlist = true;
        }
    }
    clauses.extend(query.to_sql(&mut params));
    let filter = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    Some(Selection {
        from,
        filter,
        params,
        playlist,
    })
}

/// Number and total length of the tracks below `node` matching `query`.
pub(crate) fn summary(conn: &Connection, node: &BrowseNode, query: &Query) -> Result<TrackSummary> {
    let node = parse(&node.id)?;
    let Some(sel) = select(&node, query) else {
        return Ok(TrackSummary::default());
    };
    let (n, ms): (i64, i64) = conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(t.length_ms), 0) FROM {}{}",
            sel.from, sel.filter
        ),
        params_from_iter(sel.params.iter()),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(TrackSummary {
        count: count(n),
        length_ms: count(ms),
    })
}

/// Artists of the tracks below `node` matching `query`.
pub(crate) fn artist_facets(
    conn: &Connection,
    node: &BrowseNode,
    query: &Query,
) -> Result<Vec<ArtistFacet>> {
    let node = parse(&node.id)?;
    let Some(sel) = select(&node, query) else {
        return Ok(Vec::new());
    };
    let mut st = conn.prepare(&format!(
        "SELECT MIN(COALESCE(t.artist, '')) AS name,
                COUNT(DISTINCT nl_fold(COALESCE(t.album, ''))), COUNT(*)
         FROM {}{} GROUP BY nl_fold(COALESCE(t.artist, ''))
         ORDER BY (name = '') ASC, name COLLATE nl_natural",
        sel.from, sel.filter
    ))?;
    let rows = st.query_map(params_from_iter(sel.params.iter()), |r| {
        Ok(ArtistFacet {
            name: r.get(0)?,
            albums: count(r.get(1)?),
            tracks: count(r.get(2)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Albums of the tracks below `node` matching `query`.
pub(crate) fn album_facets(
    conn: &Connection,
    node: &BrowseNode,
    query: &Query,
) -> Result<Vec<AlbumFacet>> {
    let node = parse(&node.id)?;
    let Some(sel) = select(&node, query) else {
        return Ok(Vec::new());
    };
    let mut st = conn.prepare(&format!(
        "SELECT MIN(COALESCE(t.album, '')) AS name, MIN(t.year_num), COUNT(*)
         FROM {}{} GROUP BY nl_fold(COALESCE(t.album, ''))
         ORDER BY (name = '') ASC, name COLLATE nl_natural",
        sel.from, sel.filter
    ))?;
    let rows = st.query_map(params_from_iter(sel.params.iter()), |r| {
        Ok(AlbumFacet {
            name: r.get(0)?,
            year: r
                .get::<_, Option<i64>>(1)?
                .and_then(|y| i32::try_from(y).ok()),
            tracks: count(r.get(2)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Tracks below `node` matching `query`.
pub(crate) fn tracks(
    conn: &Connection,
    node: &BrowseNode,
    query: &Query,
    sort: Sort,
    page: Page,
) -> Result<TrackPage> {
    let node = parse(&node.id)?;
    let Some(Selection {
        from,
        filter,
        mut params,
        playlist,
    }) = select(&node, query)
    else {
        return Ok(TrackPage {
            offset: page.offset,
            ..TrackPage::default()
        });
    };
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM {from}{filter}"),
        params_from_iter(params.iter()),
        |r| r.get(0),
    )?;
    let order = match (playlist, sort.key) {
        (true, SortKey::Position) if sort.descending => "pi.position DESC".to_string(),
        (true, SortKey::Position) => "pi.position ASC".to_string(),
        (true, _) => format!("{}, pi.position ASC", sort.to_sql()),
        (false, _) => sort.to_sql(),
    };
    params.push(Value::Integer(to_i64(page.limit)));
    params.push(Value::Integer(to_i64(page.offset)));
    let mut st = conn.prepare(&format!(
        "SELECT {TRACK_COLS} FROM {from}{filter} ORDER BY {order} LIMIT ? OFFSET ?"
    ))?;
    let tracks = st
        .query_map(params_from_iter(params.iter()), row_to_track)?
        .collect::<rusqlite::Result<Vec<Track>>>()?;
    Ok(TrackPage {
        tracks,
        total: count(total),
        offset: page.offset,
    })
}

/// One track by row id.
pub(crate) fn track(conn: &Connection, id: i64) -> Result<Option<Track>> {
    Ok(conn
        .query_row(
            &format!("SELECT {TRACK_COLS} FROM tracks t WHERE t.id = ?"),
            [id],
            row_to_track,
        )
        .optional()?)
}
