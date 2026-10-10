# 12 — Conformance and Acceptance Tests

This document lists what must be tested before the implementation can claim Winamp 2 skin compatibility and feature parity. Each test names the spec section it checks.

---

## 1. Test corpus

- **Skins:** at least 200 real classic skins, chosen to cover: upper-case file names, sub-folder archives, missing bitmaps, `nums_ex.bmp`, no `balance.bmp`, short `volume.bmp`, 8-bit RLE bitmaps, `region.txt` shapes (main and EQ), `viscolor.txt` with comments or no final newline, custom cursors, `pledit.txt` fonts, `genex.bmp` colours. Public skin archives (for example the community skin museum) are a good source. Do not redistribute skins without their authors' permission; keep the corpus outside the repository if needed.
- **Reference screenshots:** for each corpus skin, screenshots from an original Winamp 2.9x (or 5.x with a classic skin) of: main window normal and shade, double size, EQ normal and shade, playlist at 275×116 and at 450×290, generic window. Capture with fixed state (§3).
- **Media:** a set of short files per format in 09 §1, including gapless album sequences (MP3 with LAME tags, AAC with iTunSMPB, FLAC, Vorbis, Opus), VBR MP3 without headers, files with each tag type, mixed-case and Unicode file names, a 6-channel file, a 24-bit file, a 96 kHz file.
- **Playlists:** samples of every format in 10 §3, including the edge cases listed there.
- **Streams:** a local test server that serves ICY metadata, redirects, Basic authentication, `ICY 200 OK` status lines, and dropped connections.

---

## 2. Unit tests

### 2.1 Skin parsing (01)

| Test | Expected |
|---|---|
| Archive with `SKIN/MAIN.BMP` | Found as `main.bmp` |
| Two archive entries with the same name in different folders | The later entry is used |
| PNG files in a classic archive | Ignored |
| `nums_ex.bmp` and `numbers.bmp` both present | Only `nums_ex.bmp` used |
| No `balance.bmp` | Balance drawn from `volume.bmp` |
| `region.txt` with `NumPoints=4` and 8 numbers | Section valid |
| `region.txt` with sum mismatch | Section ignored, rectangular window |
| `region.txt` values separated by `, \t` mixes and trailing junk | Parsed up to the junk |
| `[Playlist]` section in `region.txt` | Ignored |
| `viscolor.txt` with `// comment` after each line | 24 colours read |
| `viscolor.txt` with a blank line 10 | Colours 0–8 from file, 9–23 default |
| `viscolor.txt` value 300 | Stored as 44 (wraps) |
| `viscolor.txt` last line without newline | That colour keeps its default |
| `pledit.txt` `Normal=#00ff00` and `Normal=00FF00` | Both green |
| `gen.bmp` font scan | 26 letter widths match the corpus reference for 5 skins |
| `genex.bmp` pixel `#00C6FF` | Fallback colour used |
| text.bmp foreground detection | Matches the reference pick for 5 skins |

### 2.2 EQ (03)

| Test | Expected |
|---|---|
| Value 0 / 31 / 63 → dB | +12.0 / 0.0 / −12.0 |
| Preamp value 0 / 31 / 63 → gain | 4.0 / 1.0 / 0.25 |
| Filter response at each centre, band at +12 dB, others flat | +12 dB ± 0.3 dB at 44.1 and 48 kHz |
| Band at 16 kHz with sample rate 22.05 kHz | Band disabled, no instability |
| Limiter: full-scale input, EQ +12 dB | Output peak ≤ 0.93 |
| EQF write then read | Byte-identical round trip; signature and 268-byte entries |
| Read an EQF whose entry is not named `Entry1` | Loads (new behaviour) |
| q1 first-run content | 18 presets as in 03 §5.2 |
| Slider drag math | Thumb y and value for every pixel row match 03 §1.3 |
| Graph | Pixel-exact against reference screenshots for 10 curves |

### 2.3 Title formatting (10 §5.3)

Fields for these tests: artist = `Daft Punk`, title = `One More Time`, album = (missing), track = `3/14`, filename = `/music/Daft Punk - One More Time.mp3`.

| Format | Expected |
|---|---|
| `[%artist% - ]$if2(%title%,$filepart(%filename%))` | `Daft Punk - One More Time` |
| `[%album% - ]%title%` | `One More Time` |
| `$if(%album%,yes,no)` | `no` |
| `$if(abc,yes,no)` | `no` |
| `[abc]` | (empty) |
| `[x$null(%artist%)]` | `x` |
| `$if(%artist%, a, b)` | ` a` |
| `%artist% (live)` | `[SYNTAX ERROR IN FORMATTING STRING]` |
| `%artist% '(live)'` | `Daft Punk (live)` |
| `$num(%track%,2)` | `03` |
| `$upper($left(%artist%,4))` | `DAFT` |
| `$caps(don't stop)` | `Don'T Stop` |
| `$abbr(Final Fantasy VI Advance)` | `FFVIA` |
| `$foo(x)` | `[UNKNOWN FUNCTION]` |
| `$if2(%title%)` | `[INVALID $IF SYNTAX]` |
| `$div(7,0)` | `7` |
| `$muldiv(7,1,2)` | `4` |
| `$replace(%title%,One,Two)` | `Two More Time` |
| `$directory(%filename%)` | `music` |
| `$put(a,x)$get(a)` | `xx` |
| `%%$$''` | `%$'` |
| `$pathsafe(a/b:c?)` | `a-b-c_` |
| `$select(2,a,b,c)` | `b` |
| `$decode(%artist%,daft punk,DP,other)` | `DP` |

### 2.4 Playlists (10 §3)

| Test | Expected |
|---|---|
| M3U with `#EXTINF:123,A, B` | Title `A, B`, length 123 s |
| M3U `#EXTINF` before `#EXTM3U` | Treated as comment |
| M3U `#EXTINF:-1,Radio` then URL | Title `Radio`, unknown length |
| M3U `#EXTINF:200,` then file | No title, unknown length |
| M3U `#EXTINF` then `#EXTVLCOPT:x` then file | Title kept (new behaviour) |
| M3U IPTV attributes | Attributes kept; title after the comma |
| M3U relative `../a.mp3`, `sub\b.mp3` | Resolved against the playlist folder |
| PLS with `File1`, `File3` (gap), `NumberOfEntries=3` | Two entries |
| PLS `NumberOfEntries=0` | Not a playlist |
| PLS written | `NumberOfEntries` and `Version=2` at the end, CRLF |
| M3U8 written | Starts with BOM, `#EXTM3U` |
| B4S lengths in ms | Correct seconds |
| XSPF with `file:///` URI containing `%20` | Decoded path |
| Extension-less file starting with `[playlist]` | Loaded as PLS |
| Nested playlist that includes itself | Loads once, no hang |

### 2.5 Metadata (10 §7)

- Read and write every key in 10 §7.2 for every tag format; round-trip through the file-info dialog.
- POPM rating mapping both directions.
- ID3v1.1 track byte; genre text ↔ index; unknown genre → 255.
- ID3v2 `TCON` values `(17)`, `17`, `(17)Rock`, `Rock` all read as `Rock`.
- Writing ID3v2 to a file with a smaller existing tag rewrites the file safely; writing a smaller tag reuses padding.
- A tag write during playback does not interrupt audio.

### 2.6 Shuffle and advance (07 §3)

- Every entry plays exactly once per shuffle pass (no repeat, list sizes 1, 2, 3, 100).
- After a restart with shuffle on, the first track is the saved current track.
- With repeat and morph rate 50, over 1000 passes of a 20-entry list, no entry plays twice within 9 tracks of a pass boundary in more than 5% of cases.
- Previous after 5 Next commands walks back through the same 5 entries.
- All 4 repeat/manual-advance combinations follow the table in 07 §3.1.
- Stop after current stops exactly once, then clears.
- A playlist of only unplayable files stops instead of looping.

---

## 3. Visual (golden image) tests

Fixed state for screenshots: a known track at 1:23 of 3:45, 128 kbps, 44 kHz, stereo, volume 200, balance 0, shuffle on, repeat off, EQ open with a fixed curve, playlist with 12 known entries, entry 3 playing, entries 5–6 selected, visualizer off (and separately: a frozen spectrum frame from a known input).

| Test | Pass condition |
|---|---|
| Main window, every corpus skin | Pixel-exact against the reference, except the song title area in system-font mode (compare with tolerance) |
| Main window shade, double size | Pixel-exact |
| Time display: 0:00, 9:59, 99:59, 100:00, remaining mode, pause blink frames | Pixel-exact for `numbers.bmp` and `nums_ex.bmp` skins |
| Volume 0, 1, 128, 254, 255; balance −127, −13, 0, 13, 127 | Correct frame and thumb position |
| kbps 8, 64, 128, 320, 1411, 9999, 10000 | Matches 02 §7.3 |
| Mono, stereo, 6-channel | 6-channel shows both off |
| EQ normal and shade | Pixel-exact |
| Playlist at several sizes, with smooth resizing on and off | Pixel-exact frame; text with tolerance |
| Playlist skinned pop-ups, each item highlighted | Pixel-exact |
| Generic window title with UseGenNums on and off | Pixel-exact |
| `region.txt` skins | Window shape matches (alpha mask comparison) |
| Cursor per hit area | Matches 01 §7 |

---

## 4. Behaviour tests

- **Docking:** move the main window with EQ and playlist docked below; both follow. Shade the main window; EQ and playlist move up 102 pixels. Double size; they move down. Undock by dragging the EQ away; it no longer follows. Snap at 10 pixels; Shift disables snapping.
- **Mouse:** for each hit area in 02 §12.2, the documented action fires on the documented event (down or up), and modifiers work.
- **Keyboard:** every shortcut in 11 §2, 04 §8 and 03 §1.7.
- **Seek bar:** release at each of 0, 109 and 218 pixels gives 0%, 50%, 100% of the length; Shift drag seeks live.
- **Marquee:** scroll timing (5 px per 200 ms), separator `  ***  `, pause after manual scroll, transient messages for 1 s.
- **Command line:** each switch in 11 §4.2 against a running instance. Multi-file launch from a file manager makes one playlist.
- **Remote control:** each native command and query; MPRIS conformance on Linux.

---

## 5. Visualizer tests (06)

- Feed a 1 kHz sine at −6 dBFS: the peak bar index and height match the reference screenshot.
- Feed white noise: bar levels match the reference within ±1 pixel after 2 s.
- Falloff: from full scale to silence, bar height over time matches the step sizes for each falloff setting.
- Peak: hold and accelerating fall match the factor for each setting.
- Thin vs thick, Normal/Fire/Line styles, oscilloscope Dots/Lines/Solid: pixel-exact against reference frames generated from the same input.
- **Open item:** confirm the 2-row baseline offset (06 §4.2) with reference screenshots of real Winamp 2.x. Adjust the spec if the screenshots differ.

---

## 6. Audio tests

| Test | Pass condition |
|---|---|
| Gapless album (MP3 LAME, AAC iTunSMPB, FLAC, Vorbis, Opus) | Concatenated output equals the decoded original WAV sample for sample |
| Crossfade 5 s | Overlap length 5 s ± 10 ms; no clipping with the limiter on |
| Pause/stop/seek fades | 333 ms ± 5 ms; no clicks (no step larger than −40 dB between samples at fade edges) |
| ReplayGain, all 4 modes, track and album, missing tags | Gain factor matches 07 §5.2 within 0.01 dB |
| Volume 200 | Output level = 200/255 of input (−2.11 dB) |
| Balance +64 | Left channel × 0.5, right unchanged |
| Position accuracy | Displayed time matches heard audio within 50 ms on each back end |
| 24-bit / 96 kHz file | Played without truncation when the device supports it; dithered otherwise |
| Device unplugged | Playback continues on the default device |

---

## 7. Streaming tests (07 §8)

- ICY metadata at intervals of 8192 and 16000 bytes; title shows `Song (Station)`.
- Metadata with UTF-8 and with Windows-1252 text.
- `ICY 200 OK` status line accepted.
- Redirect chain of 3; more than 10 is an error.
- 401 with Basic authentication prompts once, remembers the realm.
- Buffering shows `[Buffer: N%]`; underrun rebuffers to 10%.
- Connection drop: reconnect (when enabled) or advance.
- `.pls` URL with a query string is detected as a playlist.

---

## 8. Platform tests

- Windows, macOS and Linux (X11 and Wayland): window shapes, docking, always on top, task bar / tray options, global hotkeys, media keys, high-DPI scaling with integer and fractional factors.
- Right-to-left UI language and right-to-left playlist text.
- File names with non-Latin scripts in every dialog, playlist format and tag.

---

## 9. Known open items

| Item | Section | Action |
|---|---|---|
| Visualizer 2-row baseline offset | 06 §4.2 | Verify against reference screenshots |
| Rendering of skin bitmaps smaller than expected | 01 §2.1 | Spec says black; verify common skins look right |
| Minibrowser button semantics | 05 §4 | Not needed unless a minibrowser is built |
| System-font title rendering differences across platforms | 02 §8.1 | Accept tolerance in golden tests; choose metric-compatible fonts |
