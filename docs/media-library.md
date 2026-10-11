# Media library

The media library is in scope although the spec lists it as a non-goal (decision D4). It is designed from first principles: no reference code or plugin was consulted. The code lives in `crates/nulamp-library`, which depends on `nulamp-meta` (tags), `nulamp-core` (natural sort order), `nulamp-tf` (title formatting fields), `ureq` and `serde_json` (Jellyfin), and on no UI crate. Choices are recorded as `ML*` rows in `decisions.md`.

## 1. Overview

```
UI ──► Registry ──► Arc<dyn LibraryProvider>
                     ├─ LocalLibrary      (SQLite index of watch folders)
                     ├─ JellyfinProvider  (music libraries of a Jellyfin server)
                     ├─ MockProvider      (in-memory, remote-style; tests)
                     └─ later: Subsonic, …
```

Every provider offers the same browse tree, track listing, search, playback resolution and optional writes. The UI never needs to know which kind of source it talks to; it checks `capabilities()` to decide which commands to show.

## 2. Provider trait

`LibraryProvider` is object-safe and `Send + Sync`. All methods take `&self` and may block (disk or network), so the UI calls them from a worker thread.

| Method | Purpose |
|---|---|
| `id()`, `name()` | Stable id (used in `TrackId::provider`) and display name |
| `capabilities()` | Flags: `SCAN`, `WRITE_RATING`, `RECORD_PLAY`, `EDIT_TAGS`, `PLAYLISTS`, `ARTWORK`, `REMOTE`, `RATING_DB_ONLY`, `REMOVE` |
| `browse(&BrowseNode)` | Children of a node; `BrowseNode::root()` gives the top level |
| `tracks(&BrowseNode, &Query, Sort, Page)` | Tracks below a node that match a query, sorted and paged |
| `search(&Query, Sort, Page)` | `tracks` on the root |
| `summary(&BrowseNode, &Query)` | `TrackSummary { count, length_ms }` of a listing over all pages (status lines). The default lists every track; the local provider runs one `COUNT`/`SUM` query |
| `track(&TrackId)` | One track, `None` when gone |
| `resolve_playback(&TrackId)` | `PlaybackLocation { location, headers }`: a file path, or a URL plus HTTP headers such as `Authorization` |
| `artwork(&TrackId)` | `(mime, bytes)` of the cover |
| `rescan()` | Start re-indexing (`SCAN`) |
| `set_rating`, `record_play`, `edit_tags` | Optional writes |
| `remove_tracks(&[TrackId])` | Optional (`REMOVE`): drop tracks from the index; files stay |
| `playlists`, `playlist_node`, `create_playlist`, `rename_playlist`, `delete_playlist`, `set_playlist_tracks` | Optional playlist storage; `playlist_node(id)` gives the node that lists a playlist in order |
| `subscribe()` | A `Receiver<LibraryEvent>`: `TracksChanged`, `PlaylistsChanged`, `ScanProgress { done, total }`, `ScanFinished(ScanSummary)` |

Optional methods return `Error::Unsupported` by default. `Subscribers` is a small helper that fans events out to every receiver and forgets dropped ones.

`Registry` holds the providers in registration order (`register` replaces a provider with the same id). It can be shared: `register` and `unregister` take `&self`, so providers can come and go while the app runs (signing in to a server). `provider_for(&TrackId)` and `resolve_playback(&TrackId)` route by the track's provider id, and `finish_play(&TrackId, played_ms, length_ms)` applies the play rule (§8) and records the play.

## 3. Data model

- `TrackId { provider, id }`: the provider's own id as a string.
- `Track`: `location` (for display), `tags` (the spec 10 §7.1 keys except `rating`), `rating` 0–5, `length_ms`, `bitrate` (kbps), `samplerate`, `channels`, `codec` (format family), `file_size`, `modified`, `date_added`, `play_count`, `last_played` (unix seconds), `replaygain` (parsed `replaygain_*` tags) and `artwork` (a provider-specific reference). Accessors: `title()`, `artist()`, `year()`, `track_number()`, `display_title()`, `get(name)`. `Track` implements the title formatter's `FieldSource`.
- `BrowseNode { id, label, detail, kind, track_count, expandable, default_sort }`: ids are opaque to the UI and only meaningful to the provider that returned them. `kind` (`Category`, `Artist`, `Album`, `Genre`, `Year`, `SmartList`, `Playlist`, `Other`) is for icons.
- `Page { offset, limit }` (`Page::all()`), `TrackPage { tracks, total, offset }`.

## 4. Browse tree

Root children of the local provider:

| Node | Children | Tracks |
|---|---|---|
| Artists | artists (album artist, else artist) → albums | all |
| Albums | albums with their artist as `detail` | all |
| Genres | genres | all |
| Years | years, newest first | all |
| Recently added | — | added within 30 days of the newest addition |
| Most played | — | play count > 0, most played first |
| Never played | — | play count 0 |
| Top rated | — | 4 stars or more |
| Playlists | stored playlists | none |

Grouping ignores case; empty values are shown last as "Unknown artist" (album, genre, year). Every node carries the sort a view should start with (album: track number; most played: play count, descending; playlist: playlist order).

## 5. Query syntax

A search string is a list of terms that must all match:

- **Free text** is split into tokens exactly as in jump-to-file (spec 10 §6): runs of Unicode letters and digits, or `"quoted phrases"` kept whole. Each token must be a case-insensitive substring of the title, artist, album, album artist, genre, comment or file name. No tokens: everything matches.
- **Text fields** `title`, `artist`, `album`, `albumartist`, `genre`, `comment`, `composer`, `publisher`, `filename`, `path`: `artist:beat` (substring), `artist="the beatles"` (whole value). Values may be quoted: `album:"abbey road"`.
- **Number fields** `year`, `rating`, `plays`, `length` (seconds), `bitrate`, `track`, `disc` with `:` or `=`, `!=`, `<`, `<=`, `>`, `>=`: `year>2000`, `year<=1999`, `rating>=4`, `plays>0`. A track whose value is unknown never matches.

A field term must start a word. Anything else (an unknown field, an empty value, a non-number) is free text, so `'; DROP TABLE tracks; --` simply searches for `drop`, `table` and `tracks`. `Query::parse` builds a `Query { terms }`; `Query::matches(&Track)` evaluates it in memory for providers that filter themselves.

The local provider compiles each term to a fixed SQL fragment; every value is a bound parameter. Text is folded with Rust's `to_lowercase` both when indexed and when queried. Tokens of three or more characters use the FTS5 trigram index (`tracks_fts MATCH '"token"'`, or `artist : "token"` for a field); shorter ones scan the same table with `instr`.

## 6. Sorting

`Sort { key, descending }` with keys `Artist`, `Album`, `Title`, `Track`, `Year`, `Genre`, `Length`, `Rating`, `PlayCount`, `LastPlayed`, `DateAdded`, `Path` and `Position` (playlist order; location elsewhere). Text uses nulamp's natural order (`natural_cmp`, digits as numbers, letters ignoring case), registered as the SQLite collation `nl_natural`. Each key has fixed tie-breakers (artist → album, disc, track, title, path); only the key itself is reversed when descending, and missing values sort last either way. `Sort::compare` gives the same order in memory.

## 7. Local provider

### 7.1 Database

`LocalLibrary::open(path, filter)` opens or creates the file the app passes (`<settings>/library.sqlite`). Every connection enables WAL, foreign keys, `synchronous = NORMAL`, a 10-second busy timeout, the `nl_fold(text)` function and the `nl_natural` collation. The API, the scan thread and the watch thread each use their own connection.

Migrations are SQL scripts applied in order; the version is stored in `meta(key = 'schema_version')`. Reopening an up-to-date database applies nothing; a database from a newer build fails with `SchemaTooNew`.

| Table | Contents |
|---|---|
| `meta` | `schema_version` |
| `folders` | Watch folders: `path` (absolute, no trailing separator), `recursive` |
| `tracks` | `path` (unique), `filename`, `size`, `mtime` (ns), common tags (`title` … `publisher`), `extra` (other §7.1 keys as escaped `key=value` lines), `year_num`, `track_num`, `disc_num`, `rating`, `tag_rating`, `length_ms`, `bitrate`, `samplerate`, `channels`, `codec`, `artwork` (folder image), `date_added`, `play_count`, `last_played`, `scanned` |
| `tracks_fts` | FTS5, trigram tokenizer: folded `title`, `artist`, `album`, `albumartist`, `genre`, `comment`, `filename`; `rowid` = track id |
| `playlists` | `name`, `created`, `modified` |
| `playlist_items` | `(playlist_id, position)` → `track_id`; cascades on playlist or track deletion |
| `artwork` | Cache of embedded covers per track and `mtime` (a row with no data means "no embedded cover") |

The FTS row is rewritten by the same statements that write the track row, inside the same transaction.

### 7.2 Scanner

- `add_folder(path, recursive)` / `remove_folder(path)` manage watch folders; removing one deletes the tracks no remaining folder covers.
- `scan(mode)` starts a background thread and returns a `ScanHandle` (`wait`, `cancel`, `is_finished`). Only one scan runs at a time (`Error::Busy`).
  - `Incremental`: walk every reachable folder; read new files and files whose size or modification time changed; drop indexed files that are gone. Tracks under an unreachable folder (unmounted drive) are kept.
  - `Full`: the same, but every file is re-read.
  - `RemoveMissing`: drop every track whose file no longer exists.
- The caller's extension filter (`Fn(&str) -> bool`, lower-case extension without the dot) selects the files.
- Tags and stream properties come from `nulamp_meta::read`. A file that cannot be read is indexed with empty tags.
- Files are read in batches of 64 and each batch is written in one transaction, followed by `ScanProgress` and `TracksChanged`. Cancellation is checked between batches; committed work stays. `ScanFinished` carries the counts (`added`, `updated`, `removed`, `unchanged`, `unreadable`, `cancelled`, `error`).
- Play counts, ratings and the date added survive a re-read.

### 7.3 Watcher

`set_watching(true)` starts a `notify` watcher on every folder. Events are collected and applied once no event arrived for 500 ms (at most 3 s after the first): changed or new files are re-read when their size or time differs, new directories are walked, and removed paths drop the tracks at and below them. `TracksChanged` follows. Adding or removing a folder restarts the watcher. `set_watching` fails with `Error::Watch` where the platform watcher is unavailable; scanning still works.

### 7.4 Statistics, ratings, tags, artwork

- `record_play` adds one to the play count and sets the last-played time. The rule for whether a play counts is `should_count_play(played_ms, length_ms)`: at least half the track or at least 240 seconds (only the 240-second rule when the length is unknown).
- `set_rating` stores 0–5 stars in the database only (`RATING_DB_ONLY`); the app may also write the tag. A new track takes the rating from its tag; a rescan replaces the stored rating only when the tag's rating changed.
- `edit_tags` writes the file with `nulamp_meta::write` and re-reads it into the index.
- `remove_tracks` deletes index rows and their playlist entries; the files are not touched, so a later scan adds a file again while a watch folder covers it.
- `artwork` returns the embedded front cover, else a folder image named `cover`, `folder` or `front` (`.jpg`, `.jpeg`, `.png`).
- `track_id_for_path` maps a playlist entry's file to its library track, for example to record a play.

## 8. Using it from the app

```rust
let local = Arc::new(LocalLibrary::open(settings_dir.join("library.sqlite"), |ext| {
    decoders.handles_extension(ext)
})?);
local.add_folder(music_dir, true)?;
local.scan(ScanMode::Incremental)?;          // progress arrives on local.subscribe()
local.set_watching(true).ok();               // optional
let registry = Arc::new(Registry::new());
registry.register(local.clone());

// Tree view: start at the root, expand on demand.
let top = provider.browse(&BrowseNode::root())?;
let children = provider.browse(&top[0])?;
// List view for the selected node, with the search box applied.
let page = provider.tracks(&node, &Query::parse(&search_text), node.default_sort, Page::new(0, 200))?;
// Play: hand location and headers to the playback engine.
let loc = registry.resolve_playback(&page.tracks[0].id)?;
// When playback of a library track ends or is skipped:
registry.finish_play(&track.id, played_ms, track.length_ms)?;
```

## 9. Remote providers

The trait was shaped so that streaming servers map onto it directly; `MockProvider` (in-memory, `REMOTE`, `http://mock/stream/<id>` with an `Authorization: Bearer` header) proves the shape in the tests, and `JellyfinProvider` is the first real one.

### 9.1 Track locations

A remote track is put into a playlist as `library://<provider>/<track id>` (`TrackId::location`; `TrackId::from_location` reverses it), never as the server's stream URL. The app registers a "Media library" input in the decoder registry that claims these locations: when the track opens it calls `Registry::resolve_playback` and plays the returned URL with the returned headers through the network stream input (`StreamFactory::open_stream_with`, decision NET12), keeping the entry's title. So tokens never reach saved playlists, titles or the file-info dialog, and entries keep working after signing in again. Remote providers must resolve without blocking (no request), because the player opens tracks on its own thread.

### 9.2 Jellyfin

`nulamp_library::jellyfin` (Jellyfin 10.9 or newer):

- `login(url, user, password)` reads `/System/Info/Public`, signs in with `/Users/AuthenticateByName` under a new random device id and lists the user's libraries (`/UserViews`). It returns an `Account`: server URL, server id and name, user id and name, access token, device id and every library with its collection type and an `enabled` flag (music libraries start enabled). The password is not kept. `libraries(account)` lists the libraries again and keeps the user's choices; `logout(account)` ends the session.
- Requests carry `Authorization: MediaBrowser Client="nulamp", Device=…, DeviceId=…, Version=…, Token=…`.
- `JellyfinProvider::new(account)` has the id `jellyfin-<user id>` and the name "Jellyfin: <server name>". It shows the enabled `music` libraries; other collection types are kept in the account for later (video).

| Trait | Jellyfin |
|---|---|
| `browse` | One node per shown library (or the categories directly when one is shown): Artists (`/Artists/AlbumArtists`) → albums, Albums, Genres (`/MusicGenres`), Favorites, Most played; and Playlists (audio playlists of the user) |
| `tracks`, `search` | `/Items` with `includeItemTypes=Audio`, `recursive`, the node's filter (`parentId`, `albumArtistIds`, `genreIds`, `filters=IsFavorite`/`IsPlayed`) and server paging and sorting, when there is no search and the server knows the sort (all keys except genre, rating and path; ties as in §6). Otherwise the node is listed in full (one request; playlists from `/Playlists/{id}/Items`, 2000 entries per request), kept for 5 minutes, and filtered with `Query::matches` and sorted with `Sort::compare`, so search behaves as in the local library |
| `summary` | From a kept full listing; otherwise one counting request, with a length of 0 (the status line then shows only the count) |
| `track` | `/Items/{id}` with media sources (bit rate, sample rate, channels, size) |
| `resolve_playback` | `/Audio/{id}/universal`: the file as it is when its container is one the network input decodes (MP3, FLAC, Ogg Vorbis/FLAC, WAV, AIFF, AAC/ALAC in MP4, Vorbis/FLAC/AAC/MP3 in Matroska, Vorbis in WebM, ALAC/AAC in CAF), else an MP3 transcode at 320 kbit/s. The token goes in the `Authorization` header. The track opens as a finite stream, so the file or transcode is spooled and seekable (NET13) |
| `artwork` | The primary image of the track, else of its album |
| `record_play` | `POST /UserPlayedItems/{id}` (the server counts the play and sets the time); `TracksChanged` follows |
| `set_rating`, `edit_tags`, playlists, `rescan` | Not offered |
| `subscribe` | Only the event after a recorded play |

### 9.3 Subsonic (not built)

| Trait | Subsonic / OpenSubsonic |
|---|---|
| `browse` | `getArtists` → `getArtist` → `getAlbum`; `getGenres`; `getAlbumList2` types `newest`, `frequent`, `highest`; `getPlaylists` |
| `tracks`, `search` | `search3` with count and offset; album and playlist contents |
| `resolve_playback` | `stream` URL; authentication parameters (user, salted token) are part of the URL, so `headers` stays empty |
| `artwork` | `getCoverArt` |
| `set_rating` | `setRating` (1–5, 0 clears) |
| `record_play` | `scrobble` with `submission=true` |
| playlists | `createPlaylist`, `updatePlaylist`, `deletePlaylist` |
| `subscribe` | Polling |

Track ids are the server's item ids; `TrackId::provider` names the server account so several servers can be registered side by side. Credentials and tokens belong to the provider's configuration and never appear in `Track::location`, only in `PlaybackLocation`.

## 10. User interface

The window and its wiring live in `crates/nulamp-ui/src/library/` and `shell/library_host.rs`; choices are the `MLUI*` rows in `decisions.md`.

```
app ── MediaLibrary { registry, local } ──► Shell::with_library
                                             │
          LibraryWindow ◄──► LibraryView ──► LibraryService (worker thread) ──► providers
          (widgets, input)   (tree, pages,      ▲ requests / responses polled every tick
                              search, status)   └── provider events (one forwarding thread each)
```

- **Worker.** `LibraryService` runs every provider call on one thread. The view queues `Request`s (browse, a page of tracks, the summary, collecting the selected rows for an action, ratings, removal, scans, watch folders, plays) and the shell feeds `Response`s back on each tick. Listing requests carry a generation; the worker skips those older than the current one.
- **View.** `LibraryView` keeps the provider tree (children load on first expansion), the listing of the selected node in 200-row pages fetched around the visible rows (24 pages kept), the search box (applied 250 ms after typing stops) and the status line (track count and total time from `summary`, scan progress, messages). Provider change events reload the tree and the listing in place, at most once a second.
- **Window.** `LibraryWindow` is a generic skinned window (05 §1): search box, tree (`nulamp_skinui::TreeView`) and track list (`nulamp_skinui::Table`: sortable, resizable columns, multi-selection, vertical and horizontal `ScrollBar`s) with a movable splitter, Play/Enqueue buttons and the status line. All text is 1-bit (`PixelText`).
- **Actions.** Play (replace the playlist), Enqueue (append), Play next (append and queue first), Insert (drag onto the playlist window), rate, file info, show in folder, add to a library playlist and remove from the library collect the selected rows through the worker first, so they work on rows that were never loaded.
- **Plays.** `PlayTracker` follows the heard entry and its listening time; when it ends or is skipped the worker maps the file with `track_id_for_path` and calls `Registry::finish_play`.
- **Remote tracks.** Actions on a remote provider's rows queue `library://` locations (§9.1) titled "artist - title"; their plays are recorded through `Registry::finish_play` like local ones.
- **Changing providers.** `MediaLibrary::set_jellyfin` replaces the Jellyfin provider in the shared registry; the shell then restarts the worker and the tree (the window keeps its layout).
- **Settings.** Window geometry (`lib_*`) and the options of the preferences page "Media Library" (`library_folders`, `library_rescan_on_start`, `library_watch`) are stored in `nulamp.ini`. The page "Jellyfin" signs in and out on a helper thread (dialogs poll for the answer with `Dialog::poll`) and chooses the shown libraries; the account is stored as `jellyfin_*` keys (the access token, never the password), and the settings file is written readable by the user only.
