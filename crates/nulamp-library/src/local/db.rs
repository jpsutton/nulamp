//! SQLite connection setup, schema migrations and track rows.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use nulamp_core::playlist::natural_cmp;
use rusqlite::functions::FunctionFlags;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Row, named_params, params};

use crate::error::{Error, Result};
use crate::model::{ReplayGain, Track, TrackId, leading_number};
use crate::query::fold;

use super::LocalLibrary;

/// Schema migrations; entry `n` moves the database from version `n` to `n + 1`.
const MIGRATIONS: &[&str] = &[V1];

/// Schema version this build writes.
pub(crate) const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

const V1: &str = "
CREATE TABLE folders (
    id        INTEGER PRIMARY KEY,
    path      TEXT NOT NULL UNIQUE,
    recursive INTEGER NOT NULL DEFAULT 1,
    added     INTEGER NOT NULL
);
CREATE TABLE tracks (
    id          INTEGER PRIMARY KEY,
    path        TEXT NOT NULL UNIQUE,
    filename    TEXT NOT NULL,
    size        INTEGER NOT NULL,
    mtime       INTEGER NOT NULL,
    title       TEXT,
    artist      TEXT,
    album       TEXT,
    albumartist TEXT,
    year        TEXT,
    genre       TEXT,
    comment     TEXT,
    track       TEXT,
    disc        TEXT,
    composer    TEXT,
    publisher   TEXT,
    extra       TEXT NOT NULL DEFAULT '',
    year_num    INTEGER,
    track_num   INTEGER,
    disc_num    INTEGER,
    rating      INTEGER NOT NULL DEFAULT 0,
    tag_rating  INTEGER,
    length_ms   INTEGER NOT NULL DEFAULT 0,
    bitrate     INTEGER,
    samplerate  INTEGER,
    channels    INTEGER,
    codec       TEXT,
    artwork     TEXT,
    date_added  INTEGER NOT NULL,
    play_count  INTEGER NOT NULL DEFAULT 0,
    last_played INTEGER,
    scanned     INTEGER NOT NULL
);
CREATE INDEX tracks_artist ON tracks(artist);
CREATE INDEX tracks_album ON tracks(album);
CREATE INDEX tracks_genre ON tracks(genre);
CREATE INDEX tracks_year ON tracks(year_num);
CREATE INDEX tracks_added ON tracks(date_added);
CREATE INDEX tracks_plays ON tracks(play_count);
CREATE INDEX tracks_rating ON tracks(rating);
CREATE TABLE playlists (
    id       INTEGER PRIMARY KEY,
    name     TEXT NOT NULL,
    created  INTEGER NOT NULL,
    modified INTEGER NOT NULL
);
CREATE TABLE playlist_items (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    PRIMARY KEY (playlist_id, position)
);
CREATE INDEX playlist_items_track ON playlist_items(track_id);
CREATE TABLE artwork (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    mtime    INTEGER NOT NULL,
    mime     TEXT,
    data     BLOB
);
CREATE VIRTUAL TABLE tracks_fts USING fts5(
    title, artist, album, albumartist, genre, comment, filename,
    tokenize = 'trigram case_sensitive 1'
);
";

/// Opens a connection with the pragmas and SQL functions every connection needs:
/// WAL journal, foreign keys, a busy timeout, `nl_fold(text)` and the `nl_natural` collation.
pub(crate) fn open_connection(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(10))?;
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(Error::Provider(format!(
            "could not enable WAL (journal mode {mode})"
        )));
    }
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.create_scalar_function(
        "nl_fold",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            Ok(match ctx.get_raw(0) {
                ValueRef::Text(b) => Some(fold(&String::from_utf8_lossy(b))),
                ValueRef::Integer(i) => Some(i.to_string()),
                ValueRef::Real(f) => Some(f.to_string()),
                ValueRef::Null | ValueRef::Blob(_) => None,
            })
        },
    )?;
    conn.create_collation("nl_natural", natural_cmp)?;
    Ok(conn)
}

/// Current schema version (0 for a new database).
pub(crate) fn schema_version(conn: &Connection) -> Result<u32> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    )?;
    let v: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// Applies the missing migrations, each in its own transaction. Returns the version.
pub(crate) fn migrate(conn: &mut Connection) -> Result<u32> {
    let mut version = schema_version(conn)?;
    if version > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    while version < SCHEMA_VERSION {
        let tx = conn.transaction()?;
        tx.execute_batch(MIGRATIONS[version as usize])?;
        version += 1;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [version.to_string()],
        )?;
        tx.commit()?;
    }
    Ok(version)
}

/// Tag keys stored in their own columns; the other spec 10 §7.1 keys go to `extra`.
const CORE_TAGS: [&str; 11] = [
    "title",
    "artist",
    "album",
    "albumartist",
    "year",
    "genre",
    "comment",
    "track",
    "disc",
    "composer",
    "publisher",
];

/// Columns read by [`row_to_track`], for `tracks t`.
pub(crate) const TRACK_COLS: &str = "t.id, t.path, t.title, t.artist, t.album, t.albumartist, \
    t.year, t.genre, t.comment, t.track, t.disc, t.composer, t.publisher, t.extra, t.rating, \
    t.length_ms, t.bitrate, t.samplerate, t.channels, t.codec, t.size, t.mtime, t.date_added, \
    t.play_count, t.last_played, t.artwork";

pub(crate) fn row_to_track(row: &Row<'_>) -> rusqlite::Result<Track> {
    let id: i64 = row.get(0)?;
    let mut tags = decode_extra(&row.get::<_, String>(13)?);
    for (i, key) in CORE_TAGS.iter().enumerate() {
        if let Some(v) = row.get::<_, Option<String>>(2 + i)? {
            tags.insert((*key).to_string(), v);
        }
    }
    let replaygain = ReplayGain::from_tags(&tags);
    let u = |v: Option<i64>| v.and_then(|v| u32::try_from(v).ok());
    Ok(Track {
        id: TrackId::new(LocalLibrary::ID, id.to_string()),
        location: row.get(1)?,
        tags,
        rating: u8::try_from(row.get::<_, i64>(14)?.clamp(0, 5)).unwrap_or(0),
        length_ms: u64::try_from(row.get::<_, i64>(15)?).unwrap_or(0),
        bitrate: u(row.get(16)?),
        samplerate: u(row.get(17)?),
        channels: row
            .get::<_, Option<i64>>(18)?
            .and_then(|v| u8::try_from(v).ok()),
        codec: row.get(19)?,
        file_size: u64::try_from(row.get::<_, i64>(20)?).ok(),
        modified: Some(row.get::<_, i64>(21)?.div_euclid(1_000_000_000)),
        date_added: Some(row.get(22)?),
        play_count: u64::try_from(row.get::<_, i64>(23)?).unwrap_or(0),
        last_played: row.get(24)?,
        replaygain,
        artwork: row.get(25)?,
    })
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

/// `key=value` lines (values escaped) for the tags not stored in columns.
fn encode_extra(tags: &BTreeMap<String, String>) -> String {
    tags.iter()
        .filter(|(k, v)| !CORE_TAGS.contains(&k.as_str()) && !v.is_empty())
        .map(|(k, v)| format!("{k}={}", escape(v)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_extra(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), unescape(v)))
        .collect()
}

/// Size and modification time of a file, as stored for change detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileStat {
    pub path: String,
    pub size: i64,
    /// Nanoseconds since the unix epoch.
    pub mtime: i64,
}

impl FileStat {
    pub(crate) fn of(path: &Path, md: &fs::Metadata) -> Option<Self> {
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
        Some(Self {
            path: path.to_str()?.to_string(),
            size: i64::try_from(md.len()).unwrap_or(i64::MAX),
            mtime,
        })
    }
}

/// Everything read from one file.
pub(crate) struct Scanned {
    stat: FileStat,
    tags: BTreeMap<String, String>,
    tag_rating: Option<i64>,
    length_ms: i64,
    bitrate: Option<u32>,
    samplerate: Option<u32>,
    channels: Option<u8>,
    codec: Option<String>,
    artwork: Option<String>,
}

impl Scanned {
    /// Reads tags and stream properties; an unreadable file gives empty tags and `false`.
    pub(crate) fn read(stat: FileStat, artwork: Option<String>) -> (Self, bool) {
        let mut s = Self {
            stat,
            tags: BTreeMap::new(),
            tag_rating: None,
            length_ms: 0,
            bitrate: None,
            samplerate: None,
            channels: None,
            codec: None,
            artwork,
        };
        let Ok(meta) = nulamp_meta::read(&s.stat.path) else {
            return (s, false);
        };
        s.tags = meta
            .fields()
            .iter()
            .filter(|(k, v)| nulamp_meta::FIELDS.contains(&k.as_str()) && !v.is_empty())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        s.tag_rating = s
            .tags
            .remove("rating")
            .and_then(|r| r.trim().parse::<i64>().ok())
            .filter(|r| (0..=5).contains(r));
        let tech = meta.tech();
        s.length_ms = i64::try_from(tech.length_ms).unwrap_or(i64::MAX);
        s.bitrate = tech.bitrate;
        s.samplerate = tech.sample_rate;
        s.channels = tech.channels;
        s.codec = (!tech.family.is_empty()).then(|| tech.family.clone());
        (s, true)
    }

    fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key).map(String::as_str)
    }
}

/// Inserts or updates the row for `s.path` and its full-text entry. Returns the id and
/// whether the row is new. Play statistics and the date added are kept on update; the rating
/// follows the tag only when the tag's rating changed (decision ML5).
pub(crate) fn upsert(conn: &Connection, s: &Scanned, now: i64) -> Result<(i64, bool)> {
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, mtime FROM tracks WHERE path = ?",
            [&s.stat.path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let filename = Path::new(&s.stat.path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let num = |k: &str| s.tag(k).and_then(leading_number);
    let (extra, year_num, track_num, disc_num) = (
        encode_extra(&s.tags),
        num("year"),
        num("track"),
        num("disc"),
    );
    let (title, artist, album, albumartist) = (
        s.tag("title"),
        s.tag("artist"),
        s.tag("album"),
        s.tag("albumartist"),
    );
    let (year, genre, comment, track) = (
        s.tag("year"),
        s.tag("genre"),
        s.tag("comment"),
        s.tag("track"),
    );
    let (disc, composer, publisher) = (s.tag("disc"), s.tag("composer"), s.tag("publisher"));
    let args = named_params! {
        ":path": s.stat.path,
        ":filename": filename,
        ":size": s.stat.size,
        ":mtime": s.stat.mtime,
        ":title": title,
        ":artist": artist,
        ":album": album,
        ":albumartist": albumartist,
        ":year": year,
        ":genre": genre,
        ":comment": comment,
        ":track": track,
        ":disc": disc,
        ":composer": composer,
        ":publisher": publisher,
        ":extra": extra,
        ":year_num": year_num,
        ":track_num": track_num,
        ":disc_num": disc_num,
        ":tag_rating": s.tag_rating,
        ":length_ms": s.length_ms,
        ":bitrate": s.bitrate,
        ":samplerate": s.samplerate,
        ":channels": s.channels,
        ":codec": s.codec,
        ":artwork": s.artwork,
        ":now": now,
    };
    let (id, inserted) = match existing {
        None => {
            conn.execute(
                "INSERT INTO tracks (path, filename, size, mtime, title, artist, album,
                    albumartist, year, genre, comment, track, disc, composer, publisher, extra,
                    year_num, track_num, disc_num, rating, tag_rating, length_ms, bitrate,
                    samplerate, channels, codec, artwork, date_added, scanned)
                 VALUES (:path, :filename, :size, :mtime, :title, :artist, :album,
                    :albumartist, :year, :genre, :comment, :track, :disc, :composer, :publisher,
                    :extra, :year_num, :track_num, :disc_num, COALESCE(:tag_rating, 0),
                    :tag_rating, :length_ms, :bitrate, :samplerate, :channels, :codec, :artwork,
                    :now, :now)",
                args,
            )?;
            (conn.last_insert_rowid(), true)
        }
        Some((id, old_mtime)) => {
            conn.execute(
                "UPDATE tracks SET filename = :filename, size = :size, mtime = :mtime,
                    title = :title, artist = :artist, album = :album, albumartist = :albumartist,
                    year = :year, genre = :genre, comment = :comment, track = :track,
                    disc = :disc, composer = :composer, publisher = :publisher, extra = :extra,
                    year_num = :year_num, track_num = :track_num, disc_num = :disc_num,
                    rating = CASE WHEN :tag_rating IS NOT NULL AND :tag_rating IS NOT tag_rating
                                  THEN :tag_rating ELSE rating END,
                    tag_rating = :tag_rating, length_ms = :length_ms, bitrate = :bitrate,
                    samplerate = :samplerate, channels = :channels, codec = :codec,
                    artwork = :artwork, scanned = :now
                 WHERE path = :path",
                args,
            )?;
            if old_mtime != s.stat.mtime {
                conn.execute("DELETE FROM artwork WHERE track_id = ?", [id])?;
            }
            (id, false)
        }
    };
    let f = |k: &str| fold(s.tag(k).unwrap_or(""));
    conn.execute("DELETE FROM tracks_fts WHERE rowid = ?", [id])?;
    conn.execute(
        "INSERT INTO tracks_fts (rowid, title, artist, album, albumartist, genre, comment,
            filename)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            f("title"),
            f("artist"),
            f("album"),
            f("albumartist"),
            f("genre"),
            f("comment"),
            fold(&filename)
        ],
    )?;
    Ok((id, inserted))
}

/// Deletes a track, its full-text entry, playlist items and cached artwork.
pub(crate) fn delete_track(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM tracks_fts WHERE rowid = ?", [id])?;
    conn.execute("DELETE FROM tracks WHERE id = ?", [id])?;
    Ok(())
}

/// Stored size and mtime of `path`, when indexed.
pub(crate) fn stored_stat(conn: &Connection, path: &str) -> Result<Option<(i64, i64)>> {
    Ok(conn
        .query_row(
            "SELECT size, mtime FROM tracks WHERE path = ?",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_round_trip() {
        let mut tags = BTreeMap::new();
        tags.insert("title".to_string(), "core".to_string());
        tags.insert("mood".to_string(), "a\\b\nc=d\re".to_string());
        tags.insert("bpm".to_string(), "120".to_string());
        let enc = encode_extra(&tags);
        let dec = decode_extra(&enc);
        assert_eq!(dec.get("mood").unwrap(), "a\\b\nc=d\re");
        assert_eq!(dec.get("bpm").unwrap(), "120");
        assert!(!dec.contains_key("title"));
    }
}
