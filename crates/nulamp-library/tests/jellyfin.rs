//! The Jellyfin provider against a small fake server that answers the requests the provider
//! makes with canned JSON.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use nulamp_library::jellyfin::{self, Account, Library};
use nulamp_library::{
    BrowseNode, Error, JellyfinProvider, LibraryEvent, LibraryProvider, NodeKind, Page, Query,
    Sort, SortKey, TrackId,
};
use serde_json::{Value, json};

/// One request as the fake server saw it.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    query: HashMap<String, String>,
    auth: String,
}

struct Server {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.seen.lock().unwrap().clear();
    }

    fn count(&self, path: &str) -> usize {
        self.requests().iter().filter(|s| s.path == path).count()
    }

    fn last(&self, path: &str) -> Seen {
        self.requests()
            .into_iter()
            .rev()
            .find(|s| s.path == path)
            .unwrap_or_else(|| panic!("no request for {path}"))
    }
}

fn decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 3 <= b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

fn track(id: &str, name: &str, artist: &str, album: &str, n: u32, secs: i64) -> Value {
    json!({
        "Id": id, "Name": name, "Type": "Audio", "MediaType": "Audio",
        "Artists": [artist], "AlbumArtist": artist, "Album": album, "AlbumId": format!("alb-{album}"),
        "IndexNumber": n, "RunTimeTicks": secs * 10_000_000, "ProductionYear": 2001,
        "Genres": ["Rock"], "Container": "flac", "UserData": {"PlayCount": n},
        "AlbumPrimaryImageTag": "tag"
    })
}

fn movie(id: &str, name: &str, year: i32) -> Value {
    json!({
        "Id": id, "Name": name, "Type": "Movie", "MediaType": "Video", "ProductionYear": year,
        "RunTimeTicks": 60_000_000_000i64, "Genres": ["Drama"], "Container": "mkv",
        "ImageTags": {"Primary": "p"}
    })
}

fn episode(id: &str, name: &str, season: u32, n: u32) -> Value {
    json!({
        "Id": id, "Name": name, "Type": "Episode", "MediaType": "Video",
        "SeriesName": "Show", "SeriesId": "s1", "SeasonName": format!("Season {season}"),
        "ParentIndexNumber": season, "IndexNumber": n, "RunTimeTicks": 26_000_000_000i64,
        "SeriesPrimaryImageTag": "x"
    })
}

/// Tracks per library: lib1 has t1–t3, lib2 has t4–t5.
fn library_tracks(lib: &str) -> Vec<Value> {
    match lib {
        "lib1" => vec![
            track("t1", "Opening", "Alpha", "First", 1, 100),
            track("t2", "Middle", "Alpha", "First", 2, 200),
            track("t3", "Ray", "Beta", "Rays", 1, 300),
        ],
        "lib2" => vec![
            track("t4", "Four", "Gamma", "Fourth", 1, 400),
            track("t5", "Five", "Gamma", "Fourth", 2, 500),
        ],
        _ => Vec::new(),
    }
}

fn list(items: Vec<Value>, total: usize) -> Value {
    json!({ "Items": items, "TotalRecordCount": total, "StartIndex": 0 })
}

/// Paging as the server does it.
fn paged(items: Vec<Value>, q: &HashMap<String, String>) -> Value {
    let total = items.len();
    let start: usize = q
        .get("startIndex")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let limit: usize = q
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    list(items.into_iter().skip(start).take(limit).collect(), total)
}

fn route(
    method: &str,
    path: &str,
    q: &HashMap<String, String>,
    body: &str,
) -> (u16, &'static str, Vec<u8>) {
    let ok = |v: Value| (200, "application/json", v.to_string().into_bytes());
    match (method, path) {
        ("GET", "/System/Info/Public") => {
            ok(json!({"ServerName": "Test server", "Id": "srv", "Version": "10.10.7"}))
        }
        ("POST", "/Users/AuthenticateByName") => {
            let b: Value = serde_json::from_str(body).unwrap();
            if b["Username"] == "user" && b["Pw"] == "secret" {
                ok(
                    json!({"User": {"Id": "u1", "Name": "User"}, "AccessToken": "tok", "ServerId": "srv"}),
                )
            } else {
                (401, "text/plain", b"no".to_vec())
            }
        }
        ("GET", "/UserViews") => ok(list(
            vec![
                json!({"Id": "lib1", "Name": "Music", "CollectionType": "music"}),
                json!({"Id": "lib2", "Name": "More music", "CollectionType": "music"}),
                json!({"Id": "mov", "Name": "Movies", "CollectionType": "movies"}),
                json!({"Id": "tv", "Name": "TV", "CollectionType": "tvshows"}),
                json!({"Id": "books", "Name": "Books", "CollectionType": "books"}),
            ],
            5,
        )),
        ("GET", "/Items") => {
            let types = q.get("includeItemTypes").map(String::as_str).unwrap_or("");
            let parent = q.get("parentId").map(String::as_str).unwrap_or("");
            match types {
                "Audio" => {
                    let mut items = match parent {
                        "alb-First" => library_tracks("lib1").into_iter().take(2).collect(),
                        p => library_tracks(p),
                    };
                    if q.get("filters").map(String::as_str) == Some("IsFavorite") {
                        items.truncate(1);
                    }
                    ok(paged(items, q))
                }
                "Movie" => {
                    let mut items = vec![movie("m1", "Film", 1999), movie("m2", "Another", 2005)];
                    if q.get("filters").map(String::as_str) == Some("IsUnplayed") {
                        items.truncate(1);
                    }
                    ok(paged(items, q))
                }
                "Episode" => {
                    let mut items = vec![
                        episode("e1", "Pilot", 1, 1),
                        episode("e2", "Second", 1, 2),
                        episode("e3", "Return", 2, 1),
                    ];
                    if parent == "se1" {
                        items.truncate(2);
                    }
                    ok(paged(items, q))
                }
                "Series" => ok(list(
                    vec![json!({"Id": "s1", "Name": "Show", "ProductionYear": 2010,
                                "ChildCount": 2})],
                    1,
                )),
                "MusicAlbum" => ok(list(
                    vec![
                        json!({"Id": "alb-First", "Name": "First", "AlbumArtist": "Alpha",
                                "ProductionYear": 2001, "ChildCount": 2}),
                    ],
                    1,
                )),
                "Playlist" => ok(list(
                    vec![
                        json!({"Id": "pl1", "Name": "Mix", "MediaType": "Audio", "ChildCount": 3}),
                        json!({"Id": "pl2", "Name": "Clips", "MediaType": "Video"}),
                    ],
                    2,
                )),
                _ => (400, "text/plain", Vec::new()),
            }
        }
        ("GET", "/Artists/AlbumArtists") => ok(list(
            vec![
                json!({"Id": "ar1", "Name": "Alpha"}),
                json!({"Id": "ar2", "Name": "Beta"}),
            ],
            2,
        )),
        ("GET", "/MusicGenres") => ok(list(vec![json!({"Id": "g1", "Name": "Rock"})], 1)),
        ("GET", "/Genres") => ok(list(vec![json!({"Id": "gd", "Name": "Drama"})], 1)),
        ("GET", "/Shows/s1/Seasons") => ok(list(
            vec![
                json!({"Id": "se1", "Name": "Season 1", "ChildCount": 2}),
                json!({"Id": "se2", "Name": "Season 2", "ChildCount": 1}),
            ],
            2,
        )),
        ("GET", "/Shows/NextUp") => ok(list(vec![episode("e2", "Second", 1, 2)], 1)),
        ("GET", "/Items/m1") => ok(movie("m1", "Film", 1999)),
        ("POST", "/UserPlayedItems/m1") => ok(json!({"Played": true})),
        ("GET", "/Playlists/pl1/Items") => {
            let t = library_tracks("lib1");
            let video = json!({"Id": "v1", "Name": "Clip", "Type": "MusicVideo"});
            ok(paged(vec![t[2].clone(), video, t[0].clone()], q))
        }
        ("GET", "/Items/t1") => ok(track("t1", "Opening", "Alpha", "First", 1, 100)),
        ("POST", "/UserPlayedItems/t1") => ok(json!({"Played": true})),
        ("GET", "/Items/alb-First/Images/Primary" | "/Items/m1/Images/Primary") => {
            (200, "image/jpeg", b"\xff\xd8jpeg".to_vec())
        }
        ("POST", "/Sessions/Logout") => (204, "text/plain", Vec::new()),
        _ => (404, "text/plain", Vec::new()),
    }
}

fn serve() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let target = parts.next().unwrap_or("").to_string();
            let mut headers = HashMap::new();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                }
            }
            let len: usize = headers
                .get("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let mut body = vec![0; len];
            let _ = reader.read_exact(&mut body);
            let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
            let query: HashMap<String, String> = qs
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (decode(k), decode(v)))
                .collect();
            let auth = headers.get("authorization").cloned().unwrap_or_default();
            log.lock().unwrap().push(Seen {
                method: method.clone(),
                path: path.to_string(),
                query: query.clone(),
                auth: auth.clone(),
            });
            let public = matches!(path, "/System/Info/Public" | "/Users/AuthenticateByName");
            let (status, mime, out) = if !public && !auth.contains("Token=\"tok\"") {
                (401, "text/plain", Vec::new())
            } else {
                route(&method, path, &query, &String::from_utf8_lossy(&body))
            };
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                out.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&out);
        }
    });
    Server { url, seen }
}

fn account(server: &Server, libs: &[&str]) -> Account {
    let lib = |id: &str, name: &str| Library {
        id: id.into(),
        name: name.into(),
        collection_type: "music".into(),
        enabled: libs.contains(&id),
    };
    Account {
        server_url: server.url.clone(),
        server_id: "srv".into(),
        server_name: "Test server".into(),
        user_id: "u1".into(),
        user_name: "User".into(),
        token: "tok".into(),
        device_id: "dev".into(),
        libraries: vec![
            lib("lib1", "Music"),
            lib("lib2", "More music"),
            Library {
                id: "mov".into(),
                name: "Movies".into(),
                collection_type: "movies".into(),
                enabled: libs.contains(&"mov"),
            },
            Library {
                id: "tv".into(),
                name: "TV".into(),
                collection_type: "tvshows".into(),
                enabled: libs.contains(&"tv"),
            },
        ],
    }
}

fn node(id: &str) -> BrowseNode {
    BrowseNode::new(id, id, NodeKind::Other, false)
}

fn ids(page: &nulamp_library::TrackPage) -> Vec<String> {
    page.tracks.iter().map(|t| t.id.id.clone()).collect()
}

#[test]
fn login_lists_libraries() {
    let server = serve();
    let e = jellyfin::login(&server.url, "user", "wrong").unwrap_err();
    assert!(e.to_string().contains("wrong user name or password"), "{e}");

    let a = jellyfin::login(&format!("{}/", server.url), "user", "secret").unwrap();
    assert_eq!(a.server_url, server.url);
    assert_eq!(
        (a.server_id.as_str(), a.server_name.as_str()),
        ("srv", "Test server")
    );
    assert_eq!((a.user_id.as_str(), a.user_name.as_str()), ("u1", "User"));
    assert_eq!(a.token, "tok");
    assert_eq!(a.device_id.len(), 32);
    assert_eq!(a.provider_id(), "jellyfin-u1");
    let flags: Vec<_> = a
        .libraries
        .iter()
        .map(|l| (l.id.as_str(), l.enabled, l.is_music(), l.is_video()))
        .collect();
    // Video libraries start hidden; books are never shown.
    assert_eq!(
        flags,
        [
            ("lib1", true, true, false),
            ("lib2", true, true, false),
            ("mov", false, false, true),
            ("tv", false, false, true),
            ("books", false, false, false)
        ]
    );
    let auth = server.last("/UserViews").auth;
    assert!(
        auth.contains("Client=\"nulamp\"") && auth.contains(&a.device_id),
        "{auth}"
    );

    // Refreshing keeps the user's choices.
    let mut b = a.clone();
    b.libraries[0].enabled = false;
    let libs = jellyfin::libraries(&b).unwrap();
    assert!(!libs[0].enabled && libs[1].enabled);

    jellyfin::logout(&a).unwrap();
    assert_eq!(server.last("/Sessions/Logout").method, "POST");

    let mut bad = a.clone();
    bad.token = "old".into();
    let e = jellyfin::libraries(&bad).unwrap_err();
    assert!(e.to_string().contains("sign in again"), "{e}");
}

#[test]
fn browse_tree() {
    let server = serve();
    let one = JellyfinProvider::new(account(&server, &["lib1"]));
    assert_eq!(one.id(), "jellyfin-u1");
    assert_eq!(one.name(), "Jellyfin: Test server");
    let root = one.browse(&BrowseNode::root()).unwrap();
    let labels: Vec<_> = root.iter().map(|n| n.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Artists",
            "Albums",
            "Genres",
            "Favorites",
            "Most played",
            "Playlists"
        ]
    );
    assert_eq!(root[4].default_sort, Sort::desc(SortKey::PlayCount));

    let artists = one.browse(&root[0]).unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0].label, "Alpha");
    assert_eq!(
        server.last("/Artists/AlbumArtists").query["parentId"],
        "lib1"
    );
    let albums = one.browse(&artists[0]).unwrap();
    assert_eq!(albums[0].label, "First");
    assert_eq!(albums[0].detail.as_deref(), Some("2001"));
    assert_eq!(albums[0].track_count, Some(2));
    assert_eq!(albums[0].default_sort, Sort::asc(SortKey::Track));
    let q = server.last("/Items").query;
    assert_eq!(
        (q["albumArtistIds"].as_str(), q["parentId"].as_str()),
        ("ar1", "lib1")
    );
    assert_eq!(
        one.browse(&root[1]).unwrap()[0].detail.as_deref(),
        Some("Alpha")
    );
    assert_eq!(one.browse(&root[2]).unwrap()[0].label, "Rock");
    let playlists = one.browse(&root[5]).unwrap();
    assert_eq!(playlists.len(), 1, "video playlists are left out");
    assert_eq!(playlists[0].kind, NodeKind::Playlist);
    assert!(one.browse(&albums[0]).unwrap().is_empty());
    assert!(matches!(
        one.browse(&node("bogus/1")),
        Err(Error::NotFound(_))
    ));

    let two = JellyfinProvider::new(account(&server, &["lib1", "lib2"]));
    let root = two.browse(&BrowseNode::root()).unwrap();
    let labels: Vec<_> = root.iter().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, ["Music", "More music", "Playlists"]);
    assert_eq!(two.browse(&root[1]).unwrap().len(), 5);
}

#[test]
fn server_side_listing() {
    let server = serve();
    let p = JellyfinProvider::new(account(&server, &["lib1"]));
    let root = BrowseNode::root();
    let page = p
        .tracks(&root, &Query::all(), Sort::default(), Page::new(1, 1))
        .unwrap();
    assert_eq!((page.total, page.offset), (3, 1));
    assert_eq!(ids(&page), ["t2"]);
    let q = server.last("/Items").query;
    assert_eq!(q["parentId"], "lib1");
    assert_eq!(q["includeItemTypes"], "Audio");
    assert_eq!(q["recursive"], "true");
    assert_eq!((q["startIndex"].as_str(), q["limit"].as_str()), ("1", "1"));
    assert_eq!(
        q["sortBy"],
        "Artist,Album,ParentIndexNumber,IndexNumber,SortName"
    );
    assert_eq!(
        q["sortOrder"],
        "Ascending,Ascending,Ascending,Ascending,Ascending"
    );
    assert_eq!(q["userId"], "u1");

    p.tracks(
        &root,
        &Query::all(),
        Sort::desc(SortKey::Track),
        Page::new(0, 10),
    )
    .unwrap();
    let q = server.last("/Items").query;
    assert_eq!(q["sortBy"], "ParentIndexNumber,IndexNumber,SortName");
    assert_eq!(q["sortOrder"], "Descending,Descending,Ascending");

    let fav = p
        .tracks(
            &node("favorites/lib1"),
            &Query::all(),
            Sort::default(),
            Page::all(),
        )
        .unwrap();
    assert_eq!(ids(&fav), ["t1"]);
    let q = server.last("/Items").query;
    assert_eq!(q["filters"], "IsFavorite");
    assert!(!q.contains_key("limit"));

    let album = p
        .tracks(
            &node("album/alb-First"),
            &Query::all(),
            Sort::asc(SortKey::Track),
            Page::all(),
        )
        .unwrap();
    assert_eq!(ids(&album), ["t1", "t2"]);

    // Summary without a full listing: one count request, no length.
    let s = p.summary(&root, &Query::all()).unwrap();
    assert_eq!((s.count, s.length_ms), (3, 0));
    assert_eq!(server.last("/Items").query["limit"], "1");
}

#[test]
fn searches_filter_a_cached_full_listing() {
    let server = serve();
    let p = JellyfinProvider::new(account(&server, &["lib1"]));
    let root = BrowseNode::root();
    let q = Query::parse("artist:alpha");
    let page = p
        .tracks(&root, &q, Sort::desc(SortKey::Title), Page::all())
        .unwrap();
    assert_eq!(
        ids(&page),
        ["t1", "t2"],
        "Opening before Middle, descending"
    );
    assert_eq!(page.total, 2);
    let s = p.summary(&root, &q).unwrap();
    assert_eq!((s.count, s.length_ms), (2, 300_000));
    let s = p.summary(&root, &Query::all()).unwrap();
    assert_eq!(
        (s.count, s.length_ms),
        (3, 600_000),
        "from the cached listing"
    );
    // Sorting by a key the server lacks also uses the listing.
    let by_genre = p
        .tracks(
            &root,
            &Query::all(),
            Sort::asc(SortKey::Genre),
            Page::new(2, 5),
        )
        .unwrap();
    assert_eq!((by_genre.total, by_genre.tracks.len()), (3, 1));
    assert_eq!(
        server.count("/Items"),
        1,
        "one full listing, then the cache"
    );
}

#[test]
fn several_libraries_and_playlists() {
    let server = serve();
    let p = JellyfinProvider::new(account(&server, &["lib1", "lib2"]));
    let all = p
        .tracks(
            &BrowseNode::root(),
            &Query::all(),
            Sort::asc(SortKey::Length),
            Page::new(0, 4),
        )
        .unwrap();
    assert_eq!(all.total, 5);
    assert_eq!(ids(&all), ["t1", "t2", "t3", "t4"]);
    let only_two = p
        .tracks(
            &node("lib/lib2"),
            &Query::all(),
            Sort::default(),
            Page::all(),
        )
        .unwrap();
    assert_eq!(ids(&only_two), ["t4", "t5"]);

    let pl = node("playlist/pl1");
    let order = p
        .tracks(
            &pl,
            &Query::all(),
            Sort::asc(SortKey::Position),
            Page::all(),
        )
        .unwrap();
    assert_eq!(ids(&order), ["t3", "t1"], "playlist order, videos left out");
    let rev = p
        .tracks(
            &pl,
            &Query::all(),
            Sort::desc(SortKey::Position),
            Page::all(),
        )
        .unwrap();
    assert_eq!(ids(&rev), ["t1", "t3"]);
    let by_title = p
        .tracks(&pl, &Query::all(), Sort::asc(SortKey::Title), Page::all())
        .unwrap();
    assert_eq!(ids(&by_title), ["t1", "t3"]);
    assert_eq!(server.last("/Playlists/pl1/Items").query["userId"], "u1");
    assert!(
        p.tracks(
            &node("playlists"),
            &Query::all(),
            Sort::default(),
            Page::all()
        )
        .unwrap()
        .tracks
        .is_empty()
    );
}

#[test]
fn playback_plays_and_artwork() {
    let server = serve();
    let p = JellyfinProvider::new(account(&server, &["lib1"]));
    let id = TrackId::new(p.id(), "t1");

    let loc = p.resolve_playback(&id).unwrap();
    assert!(
        loc.location.starts_with(&format!(
            "{}/Audio/t1/universal?userId=u1&deviceId=dev",
            server.url
        )),
        "{}",
        loc.location
    );
    assert!(!loc.location.contains("tok"), "no token in the URL");
    assert!(loc.location.contains("container=mp3,flac,ogg%7Cvorbis"));
    assert!(loc.location.contains(",mka%7Cflac,"));
    assert_eq!(loc.headers.len(), 1);
    assert_eq!(loc.headers[0].0, "Authorization");
    assert!(loc.headers[0].1.contains("Token=\"tok\""));
    assert!(matches!(
        p.resolve_playback(&TrackId::new("local", "t1")),
        Err(Error::InvalidArgument(_))
    ));

    let t = p.track(&id).unwrap().unwrap();
    assert_eq!(t.title(), Some("Opening"));
    assert_eq!(t.location, "library://jellyfin-u1/t1");
    assert_eq!(
        server.last("/Items/t1").query["fields"],
        "Genres,DateCreated,MediaSources"
    );
    assert_eq!(p.track(&TrackId::new(p.id(), "gone")).unwrap(), None);

    let events = p.subscribe();
    p.record_play(&id, 200_000).unwrap();
    let seen = server.last("/UserPlayedItems/t1");
    assert_eq!(
        (seen.method.as_str(), seen.query["userId"].as_str()),
        ("POST", "u1")
    );
    assert_eq!(events.try_recv(), Ok(LibraryEvent::TracksChanged));

    let (mime, bytes) = p.artwork(&id).unwrap().unwrap();
    assert_eq!(mime, "image/jpeg");
    assert_eq!(bytes, b"\xff\xd8jpeg");
    server.clear();
    assert_eq!(p.artwork(&TrackId::new(p.id(), "gone")).unwrap(), None);
}

#[test]
fn video_libraries() {
    let server = serve();
    let p = JellyfinProvider::new(account(&server, &["lib1", "mov", "tv"]));
    let labels = |n: &str| -> Vec<String> {
        p.browse(&node(n))
            .unwrap()
            .into_iter()
            .map(|n| format!("{} {}", n.id, n.label))
            .collect()
    };
    assert_eq!(
        labels(""),
        [
            "lib/lib1 Music",
            "lib/mov Movies",
            "lib/tv TV",
            "playlists Playlists"
        ]
    );
    assert_eq!(
        labels("lib/mov"),
        [
            "movies/mov Movies",
            "genres/mov Genres",
            "favorites/mov Favorites",
            "unwatched/mov Unwatched",
            "recent/mov Recently added"
        ]
    );
    assert_eq!(
        labels("lib/tv"),
        [
            "shows/tv Shows",
            "nextup/tv Next up",
            "unwatched/tv Unwatched",
            "recent/tv Recently added"
        ]
    );

    // Movies: listed and sorted by the server.
    let all = |n: &BrowseNode| {
        p.tracks(n, &Query::all(), n.default_sort, Page::all())
            .unwrap()
    };
    let movies = &p.browse(&node("lib/mov")).unwrap()[0];
    assert_eq!(ids(&all(movies)), ["video:m1", "video:m2"]);
    let q = server.last("/Items").query;
    assert_eq!(q["includeItemTypes"], "Movie");
    assert_eq!(q["parentId"], "mov");
    assert_eq!(q["sortBy"], "SortName,ProductionYear");
    let unwatched = &p.browse(&node("lib/mov")).unwrap()[3];
    assert_eq!(ids(&all(unwatched)), ["video:m1"]);
    assert_eq!(server.last("/Items").query["filters"], "IsUnplayed");
    let genres = p.browse(&node("genres/mov")).unwrap();
    assert_eq!(genres[0].id, "genre/mov/gd");
    assert_eq!(server.last("/Genres").query["includeItemTypes"], "Movie");
    all(&genres[0]);
    let q = server.last("/Items").query;
    assert_eq!(
        (q["genreIds"].as_str(), q["includeItemTypes"].as_str()),
        ("gd", "Movie")
    );

    // Shows → seasons → episodes.
    let shows = p.browse(&node("shows/tv")).unwrap();
    assert_eq!(shows.len(), 1);
    assert_eq!(
        (shows[0].id.as_str(), shows[0].kind),
        ("series/tv/s1", NodeKind::Series)
    );
    let seasons = p.browse(&shows[0]).unwrap();
    assert_eq!(
        seasons
            .iter()
            .map(|n| (n.id.as_str(), n.kind))
            .collect::<Vec<_>>(),
        [
            ("season/se1", NodeKind::Season),
            ("season/se2", NodeKind::Season)
        ]
    );
    let eps = all(&seasons[0]);
    assert_eq!(ids(&eps), ["video:e1", "video:e2"]);
    let q = server.last("/Items").query;
    assert_eq!(
        (q["parentId"].as_str(), q["includeItemTypes"].as_str()),
        ("se1", "Episode")
    );
    assert_eq!(q["sortBy"], "ParentIndexNumber,IndexNumber,SortName");
    assert_eq!(eps.tracks[1].artist(), Some("Show"));
    assert_eq!(eps.tracks[1].album(), Some("Season 1"));
    assert_eq!(ids(&all(&shows[0])), ["video:e1", "video:e2", "video:e3"]);
    assert_eq!(server.last("/Items").query["parentId"], "s1");
    let all_eps = p
        .tracks(
            &node("lib/tv"),
            &Query::all(),
            Sort::asc(SortKey::Artist),
            Page::all(),
        )
        .unwrap();
    assert_eq!(all_eps.total, 3);
    assert_eq!(
        server.last("/Items").query["sortBy"],
        "SeriesSortName,ParentIndexNumber,IndexNumber,SortName"
    );
    let next = &p.browse(&node("lib/tv")).unwrap()[1];
    assert_eq!(ids(&all(next)), ["video:e2"]);
    assert_eq!(server.last("/Shows/NextUp").query["parentId"], "tv");

    // A search of the whole library finds songs and videos.
    let found = p
        .search(&Query::parse("Film"), Sort::default(), Page::all())
        .unwrap();
    assert_eq!(ids(&found), ["video:m1"]);

    // Video playlists and playlist entries show with a video library.
    let pls: Vec<String> = p
        .browse(&node("playlists"))
        .unwrap()
        .into_iter()
        .map(|n| n.label)
        .collect();
    assert_eq!(pls, ["Mix", "Clips"]);
    let order = p
        .tracks(
            &node("playlist/pl1"),
            &Query::all(),
            Sort::asc(SortKey::Position),
            Page::all(),
        )
        .unwrap();
    assert_eq!(ids(&order), ["t3", "video:v1", "t1"]);
}

#[test]
fn video_playback_and_watched() {
    let server = serve();
    let p = Arc::new(JellyfinProvider::new(account(&server, &["mov"])));
    let id = TrackId::new(p.id(), "video:m1");
    assert_eq!(id.location(), "library://jellyfin-u1/video:m1");

    let loc = p.resolve_playback(&id).unwrap();
    assert_eq!(
        loc.location,
        format!(
            "{}/Videos/m1/stream?static=true&mediaSourceId=m1&deviceId=dev",
            server.url
        )
    );
    assert!(loc.video);
    assert!(loc.headers[0].1.contains("Token=\"tok\""));
    assert!(
        !p.resolve_playback(&TrackId::new(p.id(), "t1"))
            .unwrap()
            .video
    );

    let t = p.track(&id).unwrap().unwrap();
    assert_eq!((t.title(), t.length_ms), (Some("Film"), 6_000_000));
    assert_eq!(server.last("/Items/m1").method, "GET");

    // A film counts as watched near its end, a song after half of it.
    assert!(!p.counts_play(&id, 5_399_000, 6_000_000));
    assert!(p.counts_play(&id, 5_400_000, 6_000_000));
    assert!(!p.counts_play(&id, 600_000, 0));
    assert!(p.counts_play(&TrackId::new(p.id(), "t1"), 50_000, 100_000));
    let reg = nulamp_library::Registry::new();
    reg.register(p.clone());
    assert!(!reg.finish_play(&id, 1_000_000, 6_000_000).unwrap());
    assert_eq!(server.count("/UserPlayedItems/m1"), 0);
    assert!(reg.finish_play(&id, 5_900_000, 6_000_000).unwrap());
    assert_eq!(server.last("/UserPlayedItems/m1").method, "POST");

    let (mime, _) = p.artwork(&id).unwrap().unwrap();
    assert_eq!(mime, "image/jpeg");
    assert_eq!(server.last("/Items/m1/Images/Primary").method, "GET");
}
