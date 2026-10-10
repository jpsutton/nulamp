//! Local provider: scanning, queries, browse tree, statistics, playlists, migrations.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Fixture, retag, scan, tagged, touch_later};
use nulamp_library::{
    BrowseNode, Capabilities, Error, LibraryEvent, LibraryProvider, LocalLibrary, NodeKind, Page,
    Query, ScanMode, Sort, SortKey, Track, TrackId,
};

fn search(lib: &LocalLibrary, q: &str) -> Vec<Track> {
    lib.search(&Query::parse(q), Sort::asc(SortKey::Path), Page::all())
        .unwrap()
        .tracks
}

fn titles(tracks: &[Track]) -> Vec<String> {
    tracks.iter().map(Track::display_title).collect()
}

fn find(lib: &LocalLibrary, title: &str) -> Track {
    let t = search(lib, &format!("title=\"{title}\""));
    assert_eq!(t.len(), 1, "{title}");
    t.into_iter().next().unwrap()
}

fn child(lib: &LocalLibrary, node: &BrowseNode, label: &str) -> BrowseNode {
    lib.browse(node)
        .unwrap()
        .into_iter()
        .find(|n| n.label == label)
        .unwrap_or_else(|| panic!("no child `{label}` under `{}`", node.label))
}

fn node_titles(lib: &LocalLibrary, node: &BrowseNode) -> Vec<String> {
    titles(
        &lib.tracks(node, &Query::all(), node.default_sort, Page::all())
            .unwrap()
            .tracks,
    )
}

#[test]
fn initial_scan_indexes_audio_files() {
    let fx = Fixture::new();
    fx.populate();
    let lib = fx.open();
    let events = lib.subscribe();
    lib.add_folder(fx.music(), true).unwrap();
    let s = scan(&lib, ScanMode::Incremental);
    assert_eq!((s.added, s.updated, s.removed, s.unchanged), (7, 0, 0, 0));
    assert!(!s.cancelled);
    assert_eq!(lib.track_count().unwrap(), 7);

    let hunter = find(&lib, "Hunter");
    assert_eq!(hunter.artist(), Some("Björk"));
    assert_eq!(hunter.year(), Some(1997));
    assert_eq!(hunter.track_number(), Some(1));
    assert_eq!(hunter.tag("comment"), Some("Iceland"));
    assert_eq!(hunter.length_ms, 1500);
    assert_eq!(hunter.samplerate, Some(8000));
    assert_eq!(hunter.channels, Some(1));
    assert!(hunter.file_size.unwrap() > 24_000);
    assert!(hunter.date_added.is_some());
    assert_eq!(hunter.play_count, 0);
    assert!(hunter.location.ends_with("hunter.wav"));
    assert_eq!(hunter.id.provider, "local");

    let untagged = search(&lib, "untagged");
    assert_eq!(titles(&untagged), ["untagged.wav"]);

    let got: Vec<LibraryEvent> = events.try_iter().collect();
    assert!(got.contains(&LibraryEvent::ScanProgress { done: 0, total: 7 }));
    assert!(got.contains(&LibraryEvent::ScanProgress { done: 7, total: 7 }));
    assert!(got.contains(&LibraryEvent::TracksChanged));
    assert!(matches!(got.last(), Some(LibraryEvent::ScanFinished(s)) if s.added == 7));

    // A second incremental scan reads nothing.
    let s = scan(&lib, ScanMode::Incremental);
    assert_eq!((s.added, s.updated, s.unchanged), (0, 0, 7));
    let s = scan(&lib, ScanMode::Full);
    assert_eq!((s.added, s.updated, s.unchanged), (0, 7, 0));
}

#[test]
fn non_recursive_folder_skips_subfolders() {
    let fx = Fixture::new();
    fx.populate();
    let lib = fx.open();
    lib.add_folder(fx.music(), false).unwrap();
    scan(&lib, ScanMode::Incremental);
    assert_eq!(titles(&search(&lib, "")), ["untagged.wav"]);
    // Making it recursive picks up the rest.
    lib.add_folder(fx.music(), true).unwrap();
    assert_eq!(scan(&lib, ScanMode::Incremental).added, 6);
    assert_eq!(lib.folders().unwrap().len(), 1);
}

#[test]
fn rescan_after_tag_edit() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    let before = find(&lib, "Something");
    lib.record_play(&before.id, 3000).unwrap();

    let path = fx.music().join("Beatles/Abbey Road/02.wav");
    retag(
        &path,
        &[("title", "Something (Remastered)"), ("bpm", "120")],
    );
    touch_later(&path, 10);
    let s = scan(&lib, ScanMode::Incremental);
    assert_eq!((s.added, s.updated, s.removed, s.unchanged), (0, 1, 0, 6));

    let after = lib.track(&before.id).unwrap().unwrap();
    assert_eq!(after.title(), Some("Something (Remastered)"));
    assert_eq!(after.tag("bpm"), Some("120"));
    assert_eq!(after.play_count, 1, "statistics survive a rescan");
    assert_eq!(after.date_added, before.date_added);
    assert!(
        search(&lib, "remastered").len() == 1,
        "full-text index updated"
    );
}

#[test]
fn removed_files_and_offline_folders() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    fs::remove_file(fx.music().join("Beatles/Hey Jude.wav")).unwrap();
    let s = scan(&lib, ScanMode::Incremental);
    assert_eq!((s.removed, s.unchanged), (1, 6));
    assert!(search(&lib, "jude").is_empty());

    // A second, offline folder keeps its tracks until "remove missing".
    let other = fx.dir.path().join("usb");
    tagged(&other, "a.wav", 100, &[("title", "On USB")]);
    lib.add_folder(&other, true).unwrap();
    scan(&lib, ScanMode::Incremental);
    assert_eq!(search(&lib, "usb").len(), 1);
    fs::rename(&other, fx.dir.path().join("usb-unplugged")).unwrap();
    let s = scan(&lib, ScanMode::Incremental);
    assert_eq!(s.removed, 0);
    assert_eq!(search(&lib, "usb").len(), 1);
    let s = scan(&lib, ScanMode::RemoveMissing);
    assert_eq!((s.removed, s.unchanged), (1, 6));
    assert!(search(&lib, "usb").is_empty());

    // Removing a watch folder drops its tracks.
    assert_eq!(lib.remove_folder(fx.music()).unwrap(), 6);
    assert_eq!(lib.track_count().unwrap(), 0);
    assert!(matches!(
        lib.remove_folder(fx.music()),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn search_tokens() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    assert_eq!(search(&lib, "").len(), 7, "no tokens: everything");
    assert_eq!(titles(&search(&lib, "beat majesty")), ["Her Majesty"]);
    assert_eq!(search(&lib, "beat").len(), 4, "substring of artist");
    assert_eq!(
        titles(&search(&lib, "BJÖRK")),
        ["Hunter"],
        "Unicode case folding"
    );
    assert_eq!(titles(&search(&lib, "ö")), ["Hunter"], "short tokens");
    assert_eq!(titles(&search(&lib, "ce")).len(), 1, "Iceland comment");
    assert_eq!(titles(&search(&lib, "\"hey jude\"")), ["Hey Jude"]);
    assert!(
        search(&lib, "\"jude hey\"").is_empty(),
        "phrase keeps order"
    );
    assert_eq!(
        titles(&search(&lib, "jude, hey!")),
        ["Hey Jude"],
        "tokens in any order"
    );
    assert_eq!(titles(&search(&lib, "comp")), ["Track X"], "file name");
    assert!(
        search(&lib, "beatles hunter").is_empty(),
        "tokens are AND-ed"
    );
}

#[test]
fn search_fields() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    let n = |q: &str| search(&lib, q).len();
    assert_eq!(n("artist:beatles"), 4);
    assert_eq!(n("artist=beatles"), 0, "= is the whole value");
    assert_eq!(n("artist=\"the beatles\""), 4);
    assert_eq!(n("album:\"abbey road\""), 3);
    assert_eq!(n("album:\"road abbey\""), 0);
    assert_eq!(n("genre:rock"), 4);
    assert_eq!(n("genre:oc"), 4, "short value");
    assert_eq!(n("albumartist:various"), 1);
    assert_eq!(n("year>2000"), 1);
    assert_eq!(n("year<=1968"), 1);
    assert_eq!(n("year=1969 track>=2"), 2);
    assert_eq!(n("year:1997"), 1, "year from a full date");
    assert_eq!(n("length>=2"), 3, "whole seconds");
    assert_eq!(n("track=10"), 1);
    assert_eq!(n("path:homogenic"), 1);
    assert_eq!(n("composer:x"), 0);
    assert_eq!(n("rating>=4"), 0);
    assert_eq!(n("plays>0"), 0);
    let t = find(&lib, "Hunter");
    lib.set_rating(&t.id, 4).unwrap();
    lib.record_play(&t.id, 0).unwrap();
    assert_eq!(n("rating>=4"), 1);
    assert_eq!(n("plays>0"), 1);
    assert_eq!(n("plays>0 artist:beatles"), 0);
    assert_eq!(n("rating:0"), 6);
}

#[test]
fn sql_like_input_is_plain_text() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    for q in [
        "'; DROP TABLE tracks; --",
        "artist:\"x' OR 1=1 --\"",
        "artist=\"' OR ''='\"",
        "\" OR 1=1 --",
        "title:\"a\"\" OR \"\"b\"",
        "year>1 OR 1=1",
        "genre:%",
        "NEAR(a b)",
        "title:*",
    ] {
        let r = lib.search(&Query::parse(q), Sort::default(), Page::all());
        assert!(r.is_ok(), "{q}: {r:?}");
        assert_eq!(r.unwrap().total, 0, "{q}");
    }
    assert_eq!(lib.track_count().unwrap(), 7);
    assert_eq!(search(&lib, "\"").len(), 7, "a lone quote has no tokens");
    // Characters with meaning in SQL or FTS5 are matched literally.
    tagged(
        &fx.music(),
        "odd.wav",
        100,
        &[("title", "100% \"Pure\" O'Brien*")],
    );
    scan(&lib, ScanMode::Incremental);
    assert_eq!(
        titles(&search(&lib, "title:\"100%\"")),
        ["100% \"Pure\" O'Brien*"]
    );
    assert_eq!(search(&lib, "title:o'brien*").len(), 1);
    assert_eq!(search(&lib, "title:%").len(), 1);
    assert_eq!(search(&lib, "\"o'brien\"").len(), 1);
}

#[test]
fn sorting_and_paging() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    let sorted = |sort: Sort, q: &str| {
        titles(
            &lib.search(&Query::parse(q), sort, Page::all())
                .unwrap()
                .tracks,
        )
    };
    let abbey = "album:abbey";
    assert_eq!(
        sorted(Sort::asc(SortKey::Track), abbey),
        ["Come Together", "Something", "Her Majesty"],
        "track numbers compare as numbers"
    );
    assert_eq!(
        sorted(Sort::desc(SortKey::Track), abbey),
        ["Her Majesty", "Something", "Come Together"]
    );
    assert_eq!(
        sorted(Sort::asc(SortKey::Title), abbey),
        ["Come Together", "Her Majesty", "Something"]
    );
    assert_eq!(
        sorted(Sort::desc(SortKey::Length), abbey),
        ["Something", "Come Together", "Her Majesty"]
    );
    let by_year = sorted(Sort::asc(SortKey::Year), "");
    assert_eq!(by_year[0], "Hey Jude");
    assert_eq!(by_year[4], "Hunter");
    assert_eq!(by_year[5], "Track X");
    assert_eq!(by_year[6], "untagged.wav", "missing values last");
    let by_year_desc = sorted(Sort::desc(SortKey::Year), "");
    assert_eq!(by_year_desc[0], "Track X");
    assert_eq!(
        by_year_desc[6], "untagged.wav",
        "missing values last in both directions"
    );
    let by_artist = sorted(Sort::asc(SortKey::Artist), "");
    assert_eq!(by_artist[0], "Hunter", "Björk");
    assert_eq!(
        &by_artist[2..5],
        ["Come Together", "Something", "Her Majesty"]
    );
    assert_eq!(sorted(Sort::asc(SortKey::Genre), "")[0], "Hunter");

    let page = lib
        .search(&Query::all(), Sort::asc(SortKey::Title), Page::new(2, 3))
        .unwrap();
    assert_eq!(page.total, 7);
    assert_eq!(page.offset, 2);
    let all = sorted(Sort::asc(SortKey::Title), "");
    assert_eq!(titles(&page.tracks), all[2..5]);

    let t = find(&lib, "Hunter");
    lib.record_play(&t.id, 0).unwrap();
    lib.record_play(&t.id, 0).unwrap();
    lib.record_play(&find(&lib, "Something").id, 0).unwrap();
    let by_plays = sorted(Sort::desc(SortKey::PlayCount), "");
    assert_eq!(&by_plays[..2], ["Hunter", "Something"]);
    let mut recent = sorted(Sort::desc(SortKey::LastPlayed), "")[..2].to_vec();
    recent.sort();
    assert_eq!(recent, ["Hunter", "Something"], "never played last");
    assert_eq!(sorted(Sort::desc(SortKey::DateAdded), "").len(), 7);
}

#[test]
fn browse_tree() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    let root = BrowseNode::root();
    let top: Vec<String> = lib
        .browse(&root)
        .unwrap()
        .into_iter()
        .map(|n| n.label)
        .collect();
    assert_eq!(
        top,
        [
            "Artists",
            "Albums",
            "Genres",
            "Years",
            "Recently added",
            "Most played",
            "Never played",
            "Top rated",
            "Playlists"
        ]
    );

    let artists = child(&lib, &root, "Artists");
    let names: Vec<(String, Option<u64>)> = lib
        .browse(&artists)
        .unwrap()
        .into_iter()
        .map(|n| (n.label, n.track_count))
        .collect();
    assert_eq!(
        names,
        [
            ("Björk".to_string(), Some(1)),
            ("The Beatles".to_string(), Some(4)),
            ("Various Artists".to_string(), Some(1)),
            ("Unknown artist".to_string(), Some(1)),
        ]
    );
    let beatles = child(&lib, &artists, "The Beatles");
    assert_eq!(beatles.kind, NodeKind::Artist);
    assert_eq!(node_titles(&lib, &beatles).len(), 4);
    let albums: Vec<String> = lib
        .browse(&beatles)
        .unwrap()
        .into_iter()
        .map(|n| n.label)
        .collect();
    assert_eq!(albums, ["Abbey Road", "Hey Jude"]);
    let abbey = child(&lib, &beatles, "Abbey Road");
    assert_eq!(abbey.kind, NodeKind::Album);
    assert_eq!(abbey.detail.as_deref(), Some("1969"));
    assert!(!abbey.expandable);
    assert_eq!(
        node_titles(&lib, &abbey),
        ["Come Together", "Something", "Her Majesty"]
    );
    assert!(lib.browse(&abbey).unwrap().is_empty());

    let albums = child(&lib, &root, "Albums");
    let list: Vec<(String, Option<String>)> = lib
        .browse(&albums)
        .unwrap()
        .into_iter()
        .map(|n| (n.label, n.detail))
        .collect();
    assert_eq!(
        list[0],
        ("Abbey Road".to_string(), Some("The Beatles".to_string()))
    );
    assert_eq!(
        list[2],
        ("Hits 2005".to_string(), Some("Various Artists".to_string()))
    );
    assert_eq!(list.last().unwrap().0, "Unknown album");
    let hits = child(&lib, &albums, "Hits 2005");
    assert_eq!(node_titles(&lib, &hits), ["Track X"]);

    let genres = child(&lib, &root, "Genres");
    let rock = child(&lib, &genres, "Rock");
    assert_eq!(rock.track_count, Some(4));
    let q = Query::parse("jude");
    let page = lib.tracks(&rock, &q, Sort::default(), Page::all()).unwrap();
    assert_eq!(titles(&page.tracks), ["Hey Jude"], "query inside a node");

    let years: Vec<String> = lib
        .browse(&child(&lib, &root, "Years"))
        .unwrap()
        .into_iter()
        .map(|n| n.label)
        .collect();
    assert_eq!(years, ["2005", "1997", "1969", "1968", "Unknown year"]);
    let unknown = child(&lib, &child(&lib, &root, "Years"), "Unknown year");
    assert_eq!(node_titles(&lib, &unknown), ["untagged.wav"]);

    assert_eq!(
        node_titles(&lib, &child(&lib, &root, "Recently added")).len(),
        7
    );
    assert!(node_titles(&lib, &child(&lib, &root, "Playlists")).is_empty());
    assert!(matches!(
        lib.browse(&BrowseNode::new("nope/x", "", NodeKind::Other, false)),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn play_statistics() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    let root = BrowseNode::root();
    let most = child(&lib, &root, "Most played");
    let never = child(&lib, &root, "Never played");
    assert!(node_titles(&lib, &most).is_empty());
    assert_eq!(node_titles(&lib, &never).len(), 7);

    let hunter = find(&lib, "Hunter");
    let jude = find(&lib, "Hey Jude");
    let events = lib.subscribe();
    lib.record_play(&jude.id, 2000).unwrap();
    for _ in 0..3 {
        lib.record_play(&hunter.id, 1500).unwrap();
    }
    assert_eq!(events.try_iter().count(), 4);
    let h = lib.track(&hunter.id).unwrap().unwrap();
    assert_eq!(h.play_count, 3);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!((now - h.last_played.unwrap()).abs() < 60);
    assert_eq!(node_titles(&lib, &most), ["Hunter", "Hey Jude"]);
    assert_eq!(node_titles(&lib, &never).len(), 5);

    assert!(nulamp_library::should_count_play(1000, 2000));
    assert!(!nulamp_library::should_count_play(999, 2000));
    let missing = TrackId::new("local", "99999");
    assert!(matches!(
        lib.record_play(&missing, 0),
        Err(Error::NotFound(_))
    ));
    let foreign = TrackId::new("mock", "1");
    assert!(matches!(
        lib.record_play(&foreign, 0),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn ratings() {
    let fx = Fixture::new();
    fx.populate();
    let path = tagged(
        &fx.music(),
        "rated.wav",
        100,
        &[("title", "Rated"), ("rating", "3")],
    );
    let lib = fx.open();
    lib.add_folder(fx.music(), true).unwrap();
    scan(&lib, ScanMode::Incremental);
    assert!(
        lib.capabilities()
            .contains(Capabilities::WRITE_RATING | Capabilities::RATING_DB_ONLY)
    );

    let rated = find(&lib, "Rated");
    assert_eq!(rated.rating, 3, "initial rating from the tag");
    lib.set_rating(&rated.id, 5).unwrap();
    assert_eq!(lib.track(&rated.id).unwrap().unwrap().rating, 5);
    assert!(matches!(
        lib.set_rating(&rated.id, 6),
        Err(Error::InvalidArgument(_))
    ));

    // An unchanged tag does not override the library rating…
    scan(&lib, ScanMode::Full);
    assert_eq!(lib.track(&rated.id).unwrap().unwrap().rating, 5);
    // …a changed tag does.
    retag(&path, &[("rating", "1")]);
    touch_later(&path, 10);
    scan(&lib, ScanMode::Incremental);
    assert_eq!(lib.track(&rated.id).unwrap().unwrap().rating, 1);

    let hunter = find(&lib, "Hunter");
    lib.set_rating(&hunter.id, 4).unwrap();
    let top = child(&lib, &BrowseNode::root(), "Top rated");
    assert_eq!(node_titles(&lib, &top), ["Hunter"]);
    lib.set_rating(&rated.id, 5).unwrap();
    assert_eq!(node_titles(&lib, &top), ["Rated", "Hunter"]);
}

#[test]
fn playlists_crud() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    let events = lib.subscribe();
    let a = lib.create_playlist("Road trip").unwrap();
    let b = lib.create_playlist("  Chill  ").unwrap();
    assert_eq!(b.name, "Chill");
    assert!(matches!(
        lib.create_playlist(" "),
        Err(Error::InvalidArgument(_))
    ));
    let names: Vec<String> = lib
        .playlists()
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Chill", "Road trip"]);

    let ids: Vec<TrackId> = ["Something", "Hunter", "Something", "Come Together"]
        .iter()
        .map(|t| find(&lib, t).id)
        .collect();
    lib.set_playlist_tracks(&a.id, &ids).unwrap();
    lib.rename_playlist(&a.id, "Long drive").unwrap();

    let node = child(
        &lib,
        &child(&lib, &BrowseNode::root(), "Playlists"),
        "Long drive",
    );
    assert_eq!(node.kind, NodeKind::Playlist);
    assert_eq!(node.track_count, Some(4));
    assert_eq!(
        node_titles(&lib, &node),
        ["Something", "Hunter", "Something", "Come Together"],
        "playlist order, duplicates kept"
    );
    let desc = lib
        .tracks(
            &node,
            &Query::all(),
            Sort::desc(SortKey::Position),
            Page::all(),
        )
        .unwrap();
    assert_eq!(titles(&desc.tracks)[0], "Come Together");
    let by_title = lib
        .tracks(
            &node,
            &Query::parse("some"),
            Sort::asc(SortKey::Title),
            Page::new(1, 10),
        )
        .unwrap();
    assert_eq!(
        (by_title.total, titles(&by_title.tracks)),
        (2, vec!["Something".to_string()])
    );

    // Replacing contents; unknown tracks are rejected without changes.
    lib.set_playlist_tracks(&a.id, &ids[1..2]).unwrap();
    assert_eq!(node_titles(&lib, &node), ["Hunter"]);
    let bad = [ids[0].clone(), TrackId::new("local", "424242")];
    assert!(matches!(
        lib.set_playlist_tracks(&a.id, &bad),
        Err(Error::NotFound(_))
    ));
    assert_eq!(node_titles(&lib, &node), ["Hunter"]);

    // Deleting a file removes it from playlists.
    fs::remove_file(fx.music().join("Bjork/Homogenic/hunter.wav")).unwrap();
    scan(&lib, ScanMode::Incremental);
    assert!(node_titles(&lib, &node).is_empty());

    lib.delete_playlist(&b.id).unwrap();
    assert!(matches!(
        lib.delete_playlist(&b.id),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        lib.rename_playlist("x", "y"),
        Err(Error::NotFound(_))
    ));
    assert_eq!(lib.playlists().unwrap().len(), 1);
    let got: Vec<LibraryEvent> = events.try_iter().collect();
    assert!(
        got.iter()
            .filter(|e| **e == LibraryEvent::PlaylistsChanged)
            .count()
            >= 5
    );
}

#[test]
fn migrations_are_idempotent_on_reopen() {
    let fx = Fixture::new();
    let id;
    {
        let lib = fx.scanned();
        assert_eq!(
            lib.schema_version().unwrap(),
            LocalLibrary::supported_schema_version()
        );
        let t = find(&lib, "Hunter");
        lib.record_play(&t.id, 0).unwrap();
        lib.create_playlist("Keep").unwrap();
        id = t.id;
    }
    for _ in 0..2 {
        let lib = fx.open();
        assert_eq!(
            lib.schema_version().unwrap(),
            LocalLibrary::supported_schema_version()
        );
        assert_eq!(lib.track_count().unwrap(), 7);
        assert_eq!(lib.track(&id).unwrap().unwrap().play_count, 1);
        assert_eq!(lib.playlists().unwrap().len(), 1);
        assert_eq!(lib.folders().unwrap().len(), 1);
        assert_eq!(search(&lib, "hunter").len(), 1);
    }
    let wal = fx.db().with_extension("sqlite-wal");
    let lib = fx.open();
    lib.record_play(&id, 0).unwrap();
    assert!(wal.exists(), "WAL journal in use");
}

#[test]
fn newer_schema_is_refused() {
    let fx = Fixture::new();
    drop(fx.open());
    let conn = rusqlite_set_version(&fx.db(), 999);
    drop(conn);
    assert!(matches!(
        LocalLibrary::open(fx.db(), common::audio),
        Err(Error::SchemaTooNew { found: 999, .. })
    ));
}

/// Writes a schema version through a second library-independent connection.
fn rusqlite_set_version(db: &Path, v: u32) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute(
        "UPDATE meta SET value = ? WHERE key = 'schema_version'",
        [v.to_string()],
    )
    .unwrap();
    conn
}

#[test]
fn edit_tags_and_playback() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    let t = find(&lib, "Track X");
    let loc = lib.resolve_playback(&t.id).unwrap();
    assert_eq!(loc.location, t.location);
    assert!(loc.headers.is_empty());
    assert_eq!(
        lib.track_id_for_path(&t.location).unwrap(),
        Some(t.id.clone())
    );

    lib.edit_tags(
        &t.id,
        &[
            ("title".to_string(), Some("Track Y".to_string())),
            ("genre".to_string(), None),
        ],
    )
    .unwrap();
    let after = lib.track(&t.id).unwrap().unwrap();
    assert_eq!(after.title(), Some("Track Y"));
    assert_eq!(after.genre(), None);
    assert_eq!(
        nulamp_meta::read(&t.location)
            .unwrap()
            .get("title")
            .as_deref(),
        Some("Track Y")
    );
    assert_eq!(
        scan(&lib, ScanMode::Incremental).updated,
        0,
        "index already current"
    );
}

#[test]
fn artwork_embedded_and_folder() {
    let fx = Fixture::new();
    let m = fx.music();
    let png = b"\x89PNG\r\n\x1a\n0000000000000000".to_vec();
    let with_cover = tagged(&m, "a/embedded.wav", 100, &[("title", "Embedded")]);
    let cover = nulamp_meta::Cover {
        mime: "image/png".into(),
        data: png.clone(),
    };
    nulamp_meta::write_cover(&with_cover, Some(&cover)).unwrap();
    tagged(&m, "b/plain.wav", 100, &[("title", "Plain")]);
    fs::write(m.join("b/Folder.JPG"), b"\xff\xd8\xffjpeg").unwrap();
    tagged(&m, "c/none.wav", 100, &[("title", "None")]);
    let lib = fx.open();
    lib.add_folder(&m, true).unwrap();
    scan(&lib, ScanMode::Incremental);

    let e = find(&lib, "Embedded");
    assert_eq!(
        lib.artwork(&e.id).unwrap(),
        Some(("image/png".to_string(), png.clone()))
    );
    assert_eq!(lib.artwork(&e.id).unwrap().unwrap().1, png, "cached");
    let p = find(&lib, "Plain");
    assert!(p.artwork.as_deref().unwrap().ends_with("Folder.JPG"));
    let (mime, data) = lib.artwork(&p.id).unwrap().unwrap();
    assert_eq!(
        (mime.as_str(), &data[..3]),
        ("image/jpeg", &b"\xff\xd8\xff"[..])
    );
    assert_eq!(lib.artwork(&find(&lib, "None").id).unwrap(), None);
}

#[test]
fn scan_busy_and_cancel() {
    let fx = Fixture::new();
    let m = fx.music();
    for i in 0..300 {
        tagged(&m, &format!("bulk/{i:03}.wav"), 10, &[]);
    }
    let lib = fx.open();
    lib.add_folder(&m, true).unwrap();
    let handle = lib.scan(ScanMode::Incremental).unwrap();
    assert!(matches!(lib.scan(ScanMode::Full), Err(Error::Busy)) || handle.is_finished());
    handle.cancel();
    let s = handle.wait().unwrap();
    assert!(s.cancelled || s.added == 300);
    assert!(!lib.is_scanning());
    let s = scan(&lib, ScanMode::Incremental);
    assert!(!s.cancelled);
    assert_eq!(lib.track_count().unwrap(), 300);
}

#[test]
fn watcher_picks_up_changes() {
    let fx = Fixture::new();
    let lib = fx.scanned();
    if let Err(e) = lib.set_watching(true) {
        eprintln!("skipping: platform watcher unavailable: {e}");
        return;
    }
    assert!(lib.is_watching());
    let events = lib.subscribe();
    tagged(
        &fx.music(),
        "new/fresh.wav",
        100,
        &[("title", "Fresh Arrival")],
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while search(&lib, "fresh arrival").is_empty() {
        assert!(
            Instant::now() < deadline,
            "watcher did not index the new file"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(events.try_iter().any(|e| e == LibraryEvent::TracksChanged));

    fs::remove_dir_all(fx.music().join("Beatles")).unwrap();
    while !search(&lib, "beatles").is_empty() {
        assert!(
            Instant::now() < deadline,
            "watcher did not drop removed files"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    lib.set_watching(false).unwrap();
    assert!(!lib.is_watching());
}

#[test]
fn ffmpeg_encoded_formats() {
    let ffmpeg = Path::new("/usr/bin/ffmpeg");
    if !ffmpeg.exists() {
        eprintln!("skipping: /usr/bin/ffmpeg not found");
        return;
    }
    let fx = Fixture::new();
    let m = fx.music();
    for (name, codec) in [("tone.mp3", "libmp3lame"), ("tone.flac", "flac")] {
        let status = Command::new(ffmpeg)
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:sample_rate=44100:duration=2")
            .args(["-ac", "2", "-c:a", codec])
            .args(["-metadata", &format!("title=Tone {name}")])
            .args([
                "-metadata",
                "artist=Generator",
                "-metadata",
                "album=Signals",
            ])
            .args(["-metadata", "date=2020", "-metadata", "track=3/9"])
            .arg(m.join(name))
            .status();
        match status {
            Ok(s) if s.success() => {}
            other => {
                eprintln!("skipping: ffmpeg could not encode {name}: {other:?}");
                return;
            }
        }
    }
    let lib = fx.open();
    lib.add_folder(&m, true).unwrap();
    assert_eq!(scan(&lib, ScanMode::Incremental).added, 2);
    for (q, codec) in [("tone mp3", "MPEG"), ("tone flac", "FLAC")] {
        let t = search(&lib, q);
        assert_eq!(t.len(), 1, "{q}");
        let t = &t[0];
        assert_eq!(t.artist(), Some("Generator"));
        assert_eq!(t.year(), Some(2020));
        assert_eq!(t.track_number(), Some(3));
        assert_eq!(t.samplerate, Some(44100));
        assert_eq!(t.channels, Some(2));
        assert!((1900..=2100).contains(&t.length_ms), "{q}: {}", t.length_ms);
        assert!(t.bitrate.is_some());
        assert!(t.codec.as_deref().unwrap().contains(codec), "{:?}", t.codec);
    }
    assert_eq!(search(&lib, "album:signals track=3").len(), 2);
}
