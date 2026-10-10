# 10 — Playlists, Title Formatting and Metadata

---

## 1. Playlist model

Each entry holds:

- **Location**: absolute file path, URL, or `cda://` location. Leading and trailing whitespace is trimmed.
- **Title**: display title (§5). May be a placeholder until metadata is read.
- **Length**: seconds, or unknown.
- **Metadata read** flag (title and length are final).
- **Selected** flag.
- Optional: per-entry repeat count (from playlist files), start/end range in ms, a stream extension hint, a queue position.

Global playlist state: current position, modified flag (saved on exit only when set), shuffle order (07 §3.2).

The playing entry is tracked by identity, so it stays current when the list is reordered, and is marked as removed when deleted (the track keeps playing).

---

## 2. Adding items

### 2.1 Open vs. add

| Action | Effect |
|---|---|
| Open file(s) (L, eject button) | Clear the list, add the files, play the first (or first shuffle slot) |
| Open folder (Shift+L) | Clear, add the folder, play |
| Open URL (Ctrl+L) | Clear, add the URL, play |
| Add file(s)/folder/URL (playlist ADD menu) | Append only |
| Drop on main window | Clear, add, play. With Shift: append only |
| Drop on playlist | Insert at the drop row (04 §7.4) |
| Command line, file arguments | Clear, add, play. With `/ADD`: append only (11 §4) |
| Open playlist (Ctrl+O) | Replace the list with the playlist's entries. Do not start playback |

The "open URL" dialog keeps a most-recently-used list. Input without a scheme gets `http://` added.

### 2.2 Folders

- Recurse into sub-folders by default. The folder dialog has a "recurse sub-folders" check box; its state is remembered.
- Add only files whose extension is claimed by some decoder (case-insensitive). Playlist files inside folders are not added.
- Option "sort files by name when adding several" (default off): sort each added batch by path and file name in natural order.
- Resolve shortcuts / symbolic links: a link to a file is added if its extension is accepted; a link to a folder is loaded without recursion.

### 2.3 Playlist entries in playlists

- An entry that is itself a playlist (by extension) is loaded recursively in its place. Guard against cycles (maximum depth 8 recommended; the reference has no guard).
- An entry that is a folder is expanded recursively (not for the default playlist).

### 2.4 Content sniffing for files without an extension

For a local file with **no** extension, read the first bytes:

| Starts with (case-insensitive) | Treat as |
|---|---|
| `[Reference]` | ASX v2 (INI style) |
| `[playlist]` (ASCII or UTF-16LE) | PLS |
| `#EXTM3U` | M3U |
| `<ASX` | ASX |
| `<?wpl`, `<?zpl` | WPL |
| anything else | Media file |

### 2.5 "Add anything" rule

Used for drag and drop, command line, remote control and bookmarks:

1. Resolve shortcuts. Drop unresolvable ones.
2. `cda://X` (whole drive) expands to one entry per track.
3. A URL: if it is a playlist (07 §8.2), load it; else add it as a stream.
4. A folder: add it recursively.
5. An existing file: if it is a playlist, load it; else sniff (§2.4); else add it.
6. A path that does not exist is added anyway (it fails at play time).

---

## 3. Playlist file formats

| Format | Read | Write |
|---|---|---|
| M3U (8-bit text) | Yes | Yes |
| M3U8 (UTF-8) | Yes | Yes |
| PLS | Yes | Yes |
| B4S (Winamp 3 XML) | Yes | Optional |
| XSPF | Yes | Recommended |
| ASX, WAX, WVX, WMX, and ASX v2 `[Reference]` | Yes | No |
| WPL, ZPL | Yes | No |

The save dialog offers M3U, M3U8 and PLS (plus XSPF if implemented). Default type: M3U8 (the reference default is M3U). Remember the last chosen type.

### 3.1 M3U / M3U8 — reading

**Encoding:**

- A byte-order mark decides (UTF-8 or UTF-16LE).
- Else UTF-8 when the extension is `.m3u8`, or after a `#UTF8` line.
- Else try strict UTF-8; if invalid, decode as Windows-1252 (or the system code page).

**Lines** (end at CR or LF):

- Empty lines are skipped. They do **not** reset a pending `#EXTINF`.
- `#EXTM3U` anywhere (as a substring of a line, before extended mode starts) enables extended mode. Before it, `#EXTINF` lines are ordinary comments.
- Leading spaces and tabs are ignored.
- **`#EXTINF:` line** (extended mode only, case-sensitive prefix):
  - Read an integer duration in seconds (may be negative).
  - If nothing follows the number: duration unknown, no title.
  - Otherwise find the first comma. Text after it is the title (later commas stay in the title).
  - If there is text between the number and the comma, it is a list of attributes `key="value"` separated by `"` + space (for example IPTV `tvg-id="x" tvg-name="y" group-title="g"`, or the stream hint `ext="mp3"`). Keep them as entry extras.
  - Truncate titles to a sane length (the reference keeps 260 characters).
- **Entry line** (does not start with `#`):
  - Strip a legacy `ASF ` prefix.
  - Resolve the location (§3.7).
  - If a pending title is **non-empty**, use it and the pending duration (negative = unknown). If the pending title is empty, ignore the duration too.
  - Clear the pending title and duration.
- **Any other `#` line**, or a whitespace-only line, **clears the pending title and duration**. Note: this means `#EXTGRP`, `#EXTVLCOPT`, `#EXTALB`, `#EXTART` between `#EXTINF` and its entry drop the title in the reference. A new implementation should **ignore** unknown `#EXT` directives instead (they must not clear the pending title). Optionally use `#EXTALB` / `#EXTART` / `#EXTGRP` as metadata.
- HLS playlists (`#EXT-X-...` tags) are streams, not playlists (07 §8.2).

### 3.2 M3U / M3U8 — writing

```
#EXTM3U
#EXTINF:<seconds>,<title>
<location>
<location of an entry without metadata>
```

- `.m3u8`: UTF-8 with a byte-order mark. `.m3u`: Windows-1252 in the reference (lossy for other scripts). Recommended: UTF-8 without BOM for `.m3u` (readers above handle both), with an option for legacy 8-bit output.
- `#EXTINF` only for entries whose metadata has been read. Seconds `-1` when unknown. The title is the raw entry title (no number prefix).
- URL entries with an extension hint: `#EXTINF:<seconds> ext="<ext>",<title>`. Keep IPTV attributes when present: `#EXTINF:<seconds> tvg-id="…" tvg-name="…" tvg-logo="…" group-title="…",<title>`.
- Locations are written as absolute paths in the reference. Recommended option (default on): relative paths for files under the playlist's folder (§3.7).
- Line ending: LF (CRLF on Windows is acceptable).

### 3.3 PLS — reading

- INI rules: section and key names are case-insensitive; whitespace around values is trimmed; matching surrounding quotes are removed; lines starting with `;` are comments.
- Section `[playlist]`. `NumberOfEntries` missing or 0 → not a valid playlist. `Version` defaults to 1.
- For n = 1..NumberOfEntries: `File<n>` (missing or empty → skip; gaps are allowed). If Version ≠ 0: `Title<n>` and `Length<n>` (seconds; −1 or missing = unknown). Length is used only when the title is non-empty.
- Optional per-entry keys: `Repeat<n>` (extra plays), `Start<n>` and `End<n>` (ms range).
- Encoding: UTF-8 if valid, else Windows-1252.

### 3.4 PLS — writing

```
[playlist]
File1=<location>
Title1=<title>
Length1=<seconds or -1>
Repeat1=<n>
File2=<location>
NumberOfEntries=<count>
Version=2
```

- `Title<n>` and `Length<n>` only for entries whose metadata has been read. `Repeat<n>` only when greater than 0.
- `NumberOfEntries` and `Version=2` come **at the end**.
- Locations are relative to the PLS file's folder when the file is under it; otherwise absolute (§3.7).
- Line ending CRLF. Encoding UTF-8 (the reference uses Windows-1252).

### 3.5 XML formats — reading

- **B4S:** root `WinampXML` (or `WasabiXML`) → `playlist` → `entry` elements. Location in attribute `Playstring` (strip a leading `file://` or `file:`). Title in child `Name`. Length in child `Length`, in **milliseconds**. Resolve relative locations (the reference does not). Element names are case-insensitive.
- **XSPF:** `playlist/trackList/track`. Use `location` (decode `file://` URIs; resolve relative URIs against the playlist), `title`, `creator` (title = `creator - title` when both exist), `duration` (ms), `album`, `trackNum`. Element names are case-sensitive. The reference falls back to a media-library search by creator/title when the location is missing; this is optional.
- **ASX:** case-insensitive XML-like format. Entries: `ENTRY/REF@HREF`. Titles from `ENTRY/TITLE` and `ENTRY/AUTHOR` (`AUTHOR - TITLE`). `ENTRYREF@HREF` is a nested playlist. `REPEAT COUNT=n` sets repeats. Default encoding Windows-1252, overridable by `<PARAM NAME="encoding" VALUE="…">`. Be lenient: escape stray `&`, accept unquoted attributes.
- **ASX v2:** INI section `[Reference]` with `Ref1`, `Ref2`, … until one is missing.
- **WPL / ZPL:** `smil/body/seq/media@src`. Resolve relative paths against the playlist folder. UTF-8.
- **B4S writing (optional):** write a well-formed document with XML declaration, `WinampXML` root, `playlist num_entries="N"`, entries with `Playstring="file:<path>"`, `Name`, `Length` in ms, all text XML-escaped.

### 3.6 XSPF — writing (recommended)

Standard XSPF 1: `location` as `file://` URIs (percent-encoded) or URLs, `title`, `creator`, `album`, `duration` (ms).

### 3.7 Locations: relative paths and portability

**Reading** accepts: absolute paths with `/` or `\`, Windows drive paths (`C:\…`), drive-relative paths (`\Music\a.mp3`, combined with the playlist's drive), UNC paths (`\\server\share\…`), relative paths (resolved against the playlist folder and normalized, including `..`), `file://` URIs (percent-decoded), and URLs.

**Portability rules for non-Windows platforms:**

- Convert `\` to `/` in relative paths.
- Windows absolute paths that do not exist locally stay as given (they fail at play time). Optional: a user-defined prefix mapping (for example `D:\Music` → `/home/user/Music`).
- When a relative path does not exist with the exact case, try a case-insensitive match (playlists made on Windows).

**Writing relative paths:** a file under the playlist's folder is written as a relative path. Otherwise absolute. Never produce `..` components (reference behaviour; keeps playlists safe to move as a folder). On Windows, a file on the same drive but outside the folder may be written drive-relative (`\path`); new implementations should write absolute paths instead.

---

## 4. Default playlist and resume

- On exit, if the playlist was modified, save it as `Winamp.m3u8` (UTF-8 with BOM, absolute paths) in the settings folder. Name it after the new player; also read `Winamp.m3u8` / `Winamp.m3u` when importing settings from an original installation.
- Save the current position in the settings.
- On start-up (when no files are given on the command line, or with `/ADD`), load the saved playlist and restore the position.
- Optional (recommended): also save and restore the playback position within the track and whether it was playing.

---

## 5. Titles

### 5.1 Title sources, in order

1. **Title formatting** (§5.2) with the user's format string, when "use advanced title formatting" is on (default on) and the decoder provides metadata for the item. For streams (http and similar), title formatting applies only when the stream provides tags (for example an MP3 stream with ID3); otherwise use the stream title (07 §8.5).
2. Else the decoder's own title (most decoders: `Artist - Title`, or the title alone).
3. Else the file name without directories and extension. For URLs: the whole URL.

If title formatting produces an empty string, fall back to step 2.

### 5.2 Default format

```
[%artist% - ]$if2(%title%,$filepart(%filename%))
```

Result: `Artist - Title`; `Title` when there is no artist; the file name (no extension) when there is no title, prefixed by `Artist - ` when the artist exists.

### 5.3 Title formatting language

#### Syntax

| Construct | Meaning |
|---|---|
| `%name%` | Value of metadata field `name` (case-insensitive). Missing or empty → nothing |
| `$func(arg1,arg2,…)` | Function call. Function names are case-insensitive. Each argument is itself a format string. Up to 64 arguments |
| `[ … ]` | Conditional section: output its content only if at least one field inside it (at any depth, including inside function arguments) was found non-empty |
| `'text'` | Literal text; `%`, `$`, `[`, `]`, `(`, `)` and `,` inside are not special |
| `''` | A literal apostrophe |
| `%%` | A literal `%` |
| `$$` | A literal `$` |
| any other character | Literal |

Rules:

- **Parentheses and `]` are special everywhere.** Literal `(`, `)`, `[`, `]` must be quoted: `%artist% '(live)'`. A literal comma inside a function argument must be quoted: `','`.
- **Whitespace is not trimmed** in arguments. `$if(%a%, x, y)` yields ` x` or ` y`.
- All arguments are evaluated before the function runs (no short-circuit), left to right. Side effects of `$put` in untaken branches still happen.
- **Truth:** an argument is "true" if it referenced at least one non-empty field (directly or nested), **not** if its output is non-empty. `$if(abc,yes,no)` gives `no`; `$if($len(%title%),y,n)` gives `y` when there is a title. `[abc]` and `[$add(1,2)]` never display.
- Function output counts toward its enclosing `[ ]` through its arguments' fields, even when the function prints nothing (`[x$null(%artist%)]` prints `x` if the artist exists).
- **Syntax errors** (unmatched `%`, `'`, `[`, `(`; stray `]` or `)`): the whole result becomes `[SYNTAX ERROR IN FORMATTING STRING]`. Inside a function argument or bracket section, only that part is replaced.
- **Unknown function:** outputs `[UNKNOWN FUNCTION]` in place of the call (not an error).
- An empty argument is an empty string. (The reference crashes on some empty arguments; never crash.)
- Newlines and carriage returns in the result are replaced with spaces.
- Characters are counted as Unicode code points (grapheme clusters are acceptable).

#### Functions

**Control**

| Function | Result |
|---|---|
| `$if(c,a,b)` | a if c is true, else b. Wrong argument count → `[INVALID $IF SYNTAX]` |
| `$if2(a,b)` | a if a is true, else b. Wrong count → `[INVALID $IF SYNTAX]` |
| `$if3(a1,…,an,else)` | The first true ai; else the last argument |
| `$decode(v,k1,r1,k2,r2,…[,default])` | The first ri whose ki equals v (case-insensitive); else default (an odd final argument); else nothing |
| `$select(n,a1,a2,…)` | an (1-based). n out of range → nothing. (Reference quirk: n = 0 outputs `0`) |
| `$iflonger(s,n,a,b)` | a if length(s) > n, else b. Wrong count → `[INVALID $IFLONGER SYNTAX]` |
| `$ifgreater(x,y,a,b)` | a if int(x) > int(y), else b. Wrong count → `[INVALID $IFGREATER SYNTAX]` |
| `$ifstrequal(a,b,t)` | t if a equals b (case-insensitive), else nothing |
| `$ifstrnotequal(a,b,t)` | t if a does not equal b |
| `$ifstrequal2(a,b,t,e)` | t if equal, else e |

String comparisons are case-insensitive and width-insensitive (full-width = half-width). Reference quirk: an empty string never equals anything, not even another empty string.

**Text**

| Function | Result |
|---|---|
| `$len(s)` | Length of s |
| `$longest(a,…)` / `$shortest(a,…)` | Longest / shortest argument; ties go to the earliest |
| `$left(s,n)`, `$cut(s,n)` | First n characters. Negative n → whole string |
| `$right(s,n)` | Last n characters. (Reference quirk: n > length gives nothing; new implementations should return the whole string) |
| `$pad(s,n[,c])` | s, right-padded with c (default space) to length n. Never truncates. With a multi-character c, characters cycle |
| `$lpad(s,n[,c])` | Left-padded |
| `$padcut(s,n[,c])` | Cut to n, then pad to n |
| `$repeat(s,n)` | s repeated n times |
| `$trim(s)` | Remove leading and trailing whitespace |
| `$upper(s)`, `$lower(s)` | Case conversion (locale-aware) |
| `$caps(s)` | Lower-case all, then upper-case the first letter of each word. A word starts after whitespace **or punctuation** (so `don't` → `Don'T`) |
| `$caps2(s)` | As `$caps` without lower-casing first |
| `$abbr(s[,n])` | Abbreviation: first letter of each word; whole words for Roman numerals (I V X L C D M only), all-digit words, and the word `RPG`. With n: return s unchanged when shorter than n. (Reference quirk: the last word is never kept whole) |
| `$strchr(s,c)`, `$strlchr(s,c)` | 1-based position of the first occurrence of the first character of c; 0 if absent |
| `$strrchr(s,c)` | Position of the last occurrence |
| `$strstr(s,t)` | 1-based position of substring t; 0 if absent |
| `$substr(s,a[,b])` | Characters a..b (1-based, inclusive; b defaults to a). (Reference quirk: returns the whole string when b ≥ max(a,1); new implementations should return the documented substring) |
| `$split(s,sep,n)` | n = 0: text before the first sep (whole s if absent). n ≠ 0: text after the first sep (nothing if absent) |
| `$replace(s,find,repl)` | Replace every non-overlapping occurrence (case-sensitive). Empty find → s unchanged |
| `$char(n)` | Character with code point n; 0 → nothing |
| `$crlf()` | A line break (becomes spaces in titles) |
| `$tab([n])` | n tab characters (default 1). (Reference quirk: `$tab()` gives none) |

**Numbers** (32-bit signed integers; int(x) reads an optional sign and leading digits, ignoring leading spaces; non-numeric → 0)

| Function | Result |
|---|---|
| `$num(x,w)`, `$dec(x,w)` | x zero-padded to width w. `$num(%tracknumber%,2)` turns `3/12` into `03` |
| `$hex(x,w)` | Lower-case hexadecimal, zero-padded |
| `$add(a,…)`, `$sub(a,b,…)`, `$mul(a,…)`, `$div(a,b,…)`, `$mod(a,b,…)` | Integer arithmetic; division truncates; division by zero skips that divisor |
| `$muldiv(a,b,c)` | a × b / c rounded to nearest, with 64-bit intermediate; c = 0 → −1 |
| `$min(a,…)`, `$max(a,…)` | Minimum / maximum |

**Paths** (accept `/` and `\`)

| Function | Result for `C:\a\b\c.mp3` |
|---|---|
| `$filepart(p)` | `c` (no folder, no extension) |
| `$filename(p)` | Same as `$filepart` in the reference |
| `$fileext(p)`, `$ext(p)` | `mp3` |
| `$directory(p[,n])` | `b` (n = 2: `a`) |
| `$pathltrim(p[,n])` | `a\b\c.mp3` (drops the first n components) |
| `$pathlpart(p[,n])` | n = 2: `C:\a` (keeps the first n) |
| `$pathrtrim(p[,n])` | `C:\a\b` (drops the last n) |
| `$pathrpart(p[,n])` | `c.mp3` (n = 2: `b\c.mp3`) |
| `$pathsafe(s)` | Make s safe as a file name: remove control characters; `? * \|` → `_`; `/ \ :` → `-`; `"` → `'`; `<` → `(`; `>` → `)` |

**Variables and time**

| Function | Result |
|---|---|
| `$put(name,v)` | Store v and output it. Variables live for one formatting call |
| `$puts(name,v)` | Store v silently |
| `$get(name)` | Stored value, or nothing |
| `$null(…)` | Nothing (arguments still count as found) |
| `$systime_year()` | Current local year |
| `$systime_month()`, `$systime_day()`, `$systime_hour()`, `$systime_minute()`, `$systime_second()` | Two digits, local time |

Functions that do **not** exist: `$and`, `$or`, `$not`, `$xor`, `$greater`, `$strcmp`, `$stricmp`, `$insert`. They output `[UNKNOWN FUNCTION]`. (New implementations may add them only as an opt-in extension.)

#### Fields

| Field | Meaning |
|---|---|
| `%filename%` | Full path or URL of the item |
| `%folder%` | Name of the immediate parent folder only |
| `%tracknumber%` | Same as `%track%` |
| Any metadata field (§7) | `%artist%`, `%title%`, `%album%`, `%albumartist%`, `%year%`, `%genre%`, `%comment%`, `%track%`, `%disc%`, `%composer%`, `%publisher%`, `%bpm%`, `%rating%`, `%length%` (ms), `%bitrate%` (kbps), `%type%` (0 audio, 1 video), `%family%`, `%streamtitle%`, … |

Unknown fields are empty. Field values are raw tag strings (`%track%` may be `3/12`).

### 5.4 Title clean-up option

Preference (both on by default): show `_` as a space; show `%20` as a space. Applied to every playlist title, including tag titles.

### 5.5 When titles are re-read

- When the title format changes, ask whether to refresh all titles.
- "Rebuild titles on selection" re-reads selected entries.
- When an entry starts playing, its metadata is always re-read.

---

## 6. Jump to file

A small dialog to find and play a playlist entry by typing.

- Open with J, Numpad `.`, or F3 from any player window, or from the menu.
- Single instance; reopening brings it to the front.
- Controls: a search box, a result list, buttons. Recommended extra buttons: Play, Enqueue (07 §3.4), Close.
- **Result list:** entry titles (no numbers, no lengths), **sorted alphabetically**. Initially all entries are listed and the playing entry is selected. (A preference may start with an empty list.)
- **Search:** on every change, rebuild the list. An entry matches when **every** query token is a case-insensitive substring of `"<location> <title>"`.
  - **Tokens:** the reference splits the query at every character that is not an ASCII letter or digit; double quotes make a phrase token in which all characters are kept. A new implementation should treat all Unicode letters and digits as token characters and compare with case folding (so `björk` matches). Keep the quote behaviour.
  - A query with no tokens matches everything.
  - After filtering, select the first row.
- **Keys:** Up, Down, Page Up and Page Down in the search box move the list selection. Enter or double-click plays the selected entry, unless it is already the current entry (then do nothing). Esc closes. Recommended: Q enqueues.

---

## 7. Metadata

### 7.1 Field model

Fields are named by lower-case keys. Required keys:

`title`, `artist`, `album`, `albumartist`, `year`, `genre`, `comment`, `track`, `disc`, `composer`, `publisher`, `conductor`, `bpm`, `rating` (0–5), `category` (grouping), `lyricist`, `key`, `mood`, `subtitle`, `isrc`, `remixing`, `originalartist`, `encoder`, `copyright`, `compilation`, `url`, `tool` (encoded by), `replaygain_track_gain`, `replaygain_track_peak`, `replaygain_album_gain`, `replaygain_album_peak`.

Technical keys: 08 §3.4.

`track` and `disc` hold the raw text (`3` or `3/12`). Total counts are not separate fields.

### 7.2 Mapping by tag format

| Key | ID3v1 | ID3v2 | APEv2 | Vorbis comment | MP4 | ASF/WMA |
|---|---|---|---|---|---|---|
| title | title (30) | TIT2 | Title | TITLE | ©nam | Title |
| artist | artist (30) | TPE1 | Artist | ARTIST | ©ART | Author |
| album | album (30) | TALB | Album | ALBUM | ©alb | WM/AlbumTitle |
| albumartist | — | TPE2; else TXXX `ALBUM ARTIST` / `ALBUMARTIST` / `Band` | Album Artist | ALBUMARTIST, ALBUM ARTIST, ENSEMBLE | aART | WM/AlbumArtist |
| year | year (4) | read TDRC then TYER; write TYER (v2.3) or TDRC (v2.4) | Year | DATE, YEAR | ©day | WM/Year |
| genre | genre byte (Appendix A) | TCON (decode `(n)` and bare numbers through Appendix A) | Genre | GENRE | ©gen, or gnre (1-based index into Appendix A) | WM/Genre |
| comment | comment (30, or 28 with track) | COMM with empty description (any language; write language `eng`) | Comment | COMMENT, DESCRIPTION | ©cmt | Description |
| track | byte 126 when byte 125 is 0 (v1.1) | TRCK | Track | TRACKNUMBER (+ TRACKTOTAL when present → `n/N`) | trkn → `n/N` | WM/TrackNumber |
| disc | — | TPOS | Disc | DISCNUMBER (+ DISCTOTAL) | disk → `n/N` | WM/PartOfSet |
| composer | — | TCOM | Composer | COMPOSER | ©wrt | WM/Composer |
| publisher | — | TPUB | Publisher | PUBLISHER, ORGANIZATION | freeform `publisher` | WM/Publisher |
| bpm | — | TBPM | BPM | BPM | tmpo | WM/BeatsPerMinute |
| conductor | — | TPE3 | Conductor | CONDUCTOR | — | WM/Conductor |
| lyricist | — | TEXT | Lyricist | LYRICIST | — | WM/Writer |
| key / mood | — | TKEY / TMOO | — | KEY / MOOD | — | WM/InitialKey / WM/Mood |
| subtitle | — | TIT3 | Subtitle | SUBTITLE, VERSION | — | WM/SubTitle |
| category | — | TIT1 | Grouping | GROUPING, CONTENTGROUP, CATEGORY | ©grp | WM/Category |
| isrc | — | TSRC | ISRC | ISRC | — | WM/ISRC |
| remixing | — | TPE4 | — | REMIXER, REMIXING | — | — |
| originalartist | — | TOPE | — | — | — | — |
| encoder | — | TSSE | — | ENCODER | ©too | WM/EncodingSettings |
| tool (encoded by) | — | TENC | — | ENCODED-BY | — | WM/ToolName |
| copyright | — | TCOP | Copyright | COPYRIGHT | cprt | Copyright |
| compilation | — | TCMP | — | COMPILATION | cpil | — |
| url | — | WXXX | — | — | — | — |
| rating (0–5) | — | POPM (§7.3) | Rating | RATING (0–100, ×20) | rate (×20) | WM/SharedUserRating (1, 25, 50, 75, 99) |
| replaygain_* | — | TXXX with the same lower-case name | same name | REPLAYGAIN_* | freeform `replaygain_*` | same name |
| cover art | — | APIC (type 3 "front cover" preferred, else any) | binary "Cover Art (Front)" | METADATA_BLOCK_PICTURE / FLAC PICTURE | covr | WM/Picture |

Reading: Vorbis comment and APEv2 keys are case-insensitive; the first matching entry wins.
Writing: an empty value deletes the field (and its aliases).

### 7.3 Rating in ID3v2 (POPM)

- Use the POPM frame whose e-mail matches the configured rating e-mail (configurable; leave empty to use any POPM frame).
- Write: 1★ = 1, 2★ = 64, 3★ = 128, 4★ = 196, 5★ = 255. No rating deletes the frame.
- Read: 224–255 = 5★, 160–223 = 4★, 96–159 = 3★, 32–95 = 2★, 1–31 = 1★, 0 = none.

### 7.4 ID3v1 details

- Layout of the last 128 bytes: `TAG`, title (30), artist (30), album (30), year (4), comment (30), genre (1). ID3v1.1: when byte 125 is 0 and byte 126 is not 0, byte 126 is the track number and the comment is 28 bytes.
- Read: trim trailing spaces and NULs. Genre 255 or out of range = none. Track 0 = none.
- Write: encode, truncate to the field size, fill the rest with NUL. Write the track byte only when 1–255. Genre: case-insensitive exact match against Appendix A, else 255.
- Optional: ID3v1 "enhanced" (`TAG+`) read support.

### 7.5 APEv2 details

- 32-byte footer (and optional 32-byte header), version 2000, items: value length (32-bit LE), flags (32-bit LE), NUL-terminated key, value. Text values are UTF-8 without terminator. Keep binary items when rewriting. Write the header and footer completely (all 32 bytes; reserved bytes zero) with correct sizes.
- Accept APEv1 tags when reading.

### 7.6 Recommended library

A mature tag library (for example TagLib) covers all of the above. The implementation must keep the mapping in §7.2 regardless of the library's own key names.

---

## 8. File info dialog

Opened with Alt+3, the clutterbar I button, double-click on the main-window title, or the playlist menus. For several selected entries, open one after another (or a multi-file editor: "Edit metadata for selection", Shift+E).

**Basic info tab** (files): artist, title, album, album artist, year, genre (editable combo with Appendix A), track, disc, comment, composer, publisher, BPM, rating. A field the decoder does not support is disabled. Read-only "format information" text (codec, bit rate, sample rate, channels, length, file size) and ReplayGain values (`+X.XX dB` or "not present").

**Stream info tab** (streams): stream title, station name, genre, home page URL, current stream URL, server information. Refresh every second.

**Artwork tab:** embedded and folder images (`cover.*`, `folder.*`, `front.*`, `albumart*.*`); load, save, delete, paste.

**Advanced tabs:** per-format raw tag views (for example separate ID3v1, ID3v2 and APEv2 panes for MP3) supplied by the decoder.

**OK:** write all changed fields, then commit. If the file is playing, writing must not interrupt playback (or must pause and resume transparently). Show an error when the file is read-only or the write fails. Update the playlist titles of every entry with that file.

---

## 9. Bookmarks

- Bookmarks are pairs of (location, title).
- File: text, pairs of lines: location, then title. UTF-8 (`Winamp.bm8` in the reference; also read the legacy 8-bit `Winamp.bm` when importing). A final line without a newline is accepted by new implementations (the reference drops it). Pairs with an empty part are skipped.
- Add: "Add current as bookmark" (Ctrl+Alt+B), "Bookmark item(s)" in the playlist (Alt+I), remote control, command line `/BOOKMARK <file>`.
- Main menu ▸ Bookmarks lists them (title as label). Choosing one: by default, clear the playlist, add the bookmark and play; with Shift, append instead. A preference swaps the default.
- "Edit bookmarks…" (Ctrl+Alt+I) opens an editor: list with title and location, reorder, edit, remove, play, enqueue.

---

## Appendix A — ID3v1 genre list

Index → name. 255 = no genre. Indices 0–79 are the original ID3v1 list; later indices are extensions widely used by tag software.

| | | | | | | | |
|---|---|---|---|---|---|---|---|
| 0 Blues | 1 Classic Rock | 2 Country | 3 Dance | 4 Disco | 5 Funk | 6 Grunge | 7 Hip-Hop |
| 8 Jazz | 9 Metal | 10 New Age | 11 Oldies | 12 Other | 13 Pop | 14 R&B | 15 Rap |
| 16 Reggae | 17 Rock | 18 Techno | 19 Industrial | 20 Alternative | 21 Ska | 22 Death Metal | 23 Pranks |
| 24 Soundtrack | 25 Euro-Techno | 26 Ambient | 27 Trip-Hop | 28 Vocal | 29 Jazz+Funk | 30 Fusion | 31 Trance |
| 32 Classical | 33 Instrumental | 34 Acid | 35 House | 36 Game | 37 Sound Clip | 38 Gospel | 39 Noise |
| 40 Alt Rock | 41 Bass | 42 Soul | 43 Punk | 44 Space | 45 Meditative | 46 Instrumental Pop | 47 Instrumental Rock |
| 48 Ethnic | 49 Gothic | 50 Darkwave | 51 Techno-Industrial | 52 Electronic | 53 Pop-Folk | 54 Eurodance | 55 Dream |
| 56 Southern Rock | 57 Comedy | 58 Cult | 59 Gangsta Rap | 60 Top 40 | 61 Christian Rap | 62 Pop/Funk | 63 Jungle |
| 64 Native American | 65 Cabaret | 66 New Wave | 67 Psychedelic | 68 Rave | 69 Showtunes | 70 Trailer | 71 Lo-Fi |
| 72 Tribal | 73 Acid Punk | 74 Acid Jazz | 75 Polka | 76 Retro | 77 Musical | 78 Rock & Roll | 79 Hard Rock |
| 80 Folk | 81 Folk-Rock | 82 National Folk | 83 Swing | 84 Fast-Fusion | 85 Bebop | 86 Latin | 87 Revival |
| 88 Celtic | 89 Bluegrass | 90 Avantgarde | 91 Gothic Rock | 92 Progressive Rock | 93 Psychedelic Rock | 94 Symphonic Rock | 95 Slow Rock |
| 96 Big Band | 97 Chorus | 98 Easy Listening | 99 Acoustic | 100 Humour | 101 Speech | 102 Chanson | 103 Opera |
| 104 Chamber Music | 105 Sonata | 106 Symphony | 107 Booty Bass | 108 Primus | 109 Porn Groove | 110 Satire | 111 Slow Jam |
| 112 Club | 113 Tango | 114 Samba | 115 Folklore | 116 Ballad | 117 Power Ballad | 118 Rhythmic Soul | 119 Freestyle |
| 120 Duet | 121 Punk Rock | 122 Drum Solo | 123 A Cappella | 124 Euro-House | 125 Dance Hall | 126 Goa | 127 Drum & Bass |
| 128 Club-House | 129 Hardcore | 130 Terror | 131 Indie | 132 BritPop | 133 Afro-Punk | 134 Polsk Punk | 135 Beat |
| 136 Christian Gangsta Rap | 137 Heavy Metal | 138 Black Metal | 139 Crossover | 140 Contemporary Christian | 141 Christian Rock | 142 Merengue | 143 Salsa |
| 144 Thrash Metal | 145 Anime | 146 JPop | 147 Synthpop | 148 Abstract | 149 Art Rock | 150 Baroque | 151 Bhangra |
| 152 Big Beat | 153 Breakbeat | 154 Chillout | 155 Downtempo | 156 Dub | 157 EBM | 158 Eclectic | 159 Electro |
| 160 Electroclash | 161 Emo | 162 Experimental | 163 Garage | 164 Global | 165 IDM | 166 Illbient | 167 Industro-Goth |
| 168 Jam Band | 169 Krautrock | 170 Leftfield | 171 Lounge | 172 Math Rock | 173 New Romantic | 174 Nu-Breakz | 175 Post-Punk |
| 176 Post-Rock | 177 Psytrance | 178 Shoegaze | 179 Space Rock | 180 Trop Rock | 181 World Music | 182 Neoclassical | 183 Audiobook |
| 184 Audio Theatre | 185 Neue Deutsche Welle | 186 Podcast | 187 Indie Rock | 188 G-Funk | 189 Dubstep | 190 Garage Rock | 191 Psybient |
| 192 Glam Rock | 193 Dream Pop | 194 Merseybeat | 195 K-Pop | 196 Chiptune | 197 Grime | 198 Grindcore | 199 Indietronic |
| 200 Indietronica | 201 Jazz Rock | 202 Jazz Fusion | 203 Post-Punk Revival | 204 Electronica | 205 Psychill | 206 Ethnotronic | 207 Americana |
| 208 Ambient Dub | 209 Digital Dub | 210 Chillwave | 211 Stoner Rock | 212 Slowcore | 213 Softcore | 214 Flamenco | 215 Hi-NRG |
| 216 Ethereal | 217 Drone | 218 Doom Metal | 219 Doom Jazz | 220 Mainstream | 221 Glitch | 222 Balearic | 223 Modern Classical |
| 224 Mod | 225 Contemporary Classical | 226 Psybreaks | 227 Psystep | 228 Psydub | 229 Chillstep | 230 Berlin School | 231 Future Jazz |
| 232 Djent | 233 Musique Concrète | 234 Electroacoustic | 235 Folktronica | 236 Texas Country | 237 Red Dirt | 238 Arabic | 239 Asian |
| 240 Bachata | 241 Bollywood | 242 Cajun | 243 Calypso | 244 Creole | 245 Darkstep | 246 Jewish | 247 Reggaeton |
| 248 Smooth Jazz | 249 Soca | 250 Spiritual | 251 Turntablism | 252 Zouk | 253 Neofolk | 254 Nu Jazz | |

When decoding ID3v2 `TCON`, also accept the special values `(RX)` = Remix and `(CR)` = Cover. The genre combo box in the file-info dialog may offer extra names (for example "Psychobilly", "Rockabilly", "Schlager & Volksmusik"); these are written as text and map to 255 in ID3v1.
