# 11 — Settings, Commands, Keyboard, Menus and Remote Control

---

## 1. Settings storage

### 1.1 Locations

| Item | Location |
|---|---|
| Settings file | Per user: `$XDG_CONFIG_HOME/<app>/` on Linux and BSD, `~/Library/Application Support/<app>/` on macOS, `%APPDATA%\<app>\` on Windows |
| Portable mode | If a file `paths.ini` (or a `portable` marker) exists next to the executable, use the folder it names (or the executable folder) for everything |
| Skins, plugins, language packs | A system folder (read-only) plus a user folder (writable). The skins folder is configurable |
| Default playlist | `<settings>/Winamp.m3u8` equivalent (10 §4) |
| EQ libraries | `<settings>/winamp.q1`, `<settings>/winamp.q2` equivalents (03 §5) |
| Bookmarks | `<settings>/Winamp.bm8` equivalent (10 §9) |

`paths.ini` (section `[Winamp]`) keys: `inidir` (settings folder), `m3udir` (default playlist folder), `m3ubase` (base for relative paths in the default playlist), `inifile`, `cwd`. Values may contain `%VAR%` environment variables. Command-line switches override them (§4).

### 1.2 Format

A text INI-style file (or another human-editable format) in UTF-8. Numbers are decimal. Booleans are 0/1. Write atomically (temporary file, then rename). Write on exit, and also after the preferences dialog closes.

### 1.3 Settings list

Default values are the reference defaults. Settings in **bold** are visible in the classic UI menus.

**Windows and layout**

| Setting | Default | Notes |
|---|---|---|
| Main window position | 26, 29 | |
| Main window visible | on | |
| Main window state | normal | normal / minimized / minimized to tray |
| **Windowshade (main)** | off | |
| **Double size** | off | |
| Double size applies to equalizer | on | |
| **Always on top** | off | |
| **EasyMove** (drag windows from anywhere) | on | |
| EQ window position / visible / windowshade | 26,145 / on / off | |
| Playlist position / size / visible | 26,261 / 275×116 / on | Windowshade = height 14, previous height kept |
| Video window position / size / visible | 26,145 / 275×232 / off | |
| Snap windows / snap distance | on / 10 px | |
| Keep windows on screen | on | |
| Smooth resizing | off | |
| Dim inactive title bars | on | |
| Always show clutterbar | on | |
| Tooltips | on | |
| Skinned cursors | on | |
| Bitmap font for main title | off | |
| Anti-aliased title font in double size | off | |
| Show playlist position in title | on | |
| **Auto-scroll song title** | on | Separate option: scroll title in task bar (off) |
| Show playlist number in task bar caption | on | |
| Old-school playlist buttons | off | |
| Drop always-on-top for full-screen apps | on | |
| Mouse wheel does not change volume | off | |
| Show in: task bar / tray | task bar only | task bar, tray, both, neither |
| Allow multiple instances | off | |
| Show splash screen | off | |

**Playback**

| Setting | Default |
|---|---|
| Volume | 200 (0–255) |
| Balance | 0 (−127..127) |
| **Shuffle** / **Repeat** / **Manual advance** | off / off / off |
| Shuffle morph rate | 50 (0–50) |
| **Time display** | elapsed (elapsed / remaining) |
| Current playlist position | 0 |
| Output bit depth | 16 (24 when "allow 24-bit") |
| Allow surround / force mono / dither | on / off / on |
| Decode thread priority | high |
| ReplayGain settings | 07 §5.1 |
| Output module and its settings | 07 §6 |
| Assumed extension of unknown files | `mp3` |

**Equalizer:** band values, preamp, on/off, auto-load, band mode (Winamp/ISO), limiter, engine (03 §7).

**Classic visualizer:** 06 §6.3.

**Playlist and titles**

| Setting | Default |
|---|---|
| Playlist font size | 12 px |
| Use custom playlist font / font face | on / a sans-serif face |
| Text direction | left-to-right (auto / LTR / RTL) |
| Show numbers / zero-pad numbers | on / off |
| Page scroll lines | 1 (1–16) |
| Wheel scrolls double | on |
| Title format | 10 §5.2 |
| Use advanced title formatting | on |
| When to read metadata | background |
| Show `_` as space / show `%20` as space | on / on |
| Sort files by name when adding several | off |
| Recurse sub-folders when adding a folder | on |
| Use trash for physically removed files | on |
| Jump-to-file starts with an empty list | off |

**Skin:** current skin (empty = default), random skin on play (off), skins folder, prompt before installing a dropped skin (on).

**Network:** HTTP proxy (empty), proxy only for port 80 (off), stream buffer settings (07 §8.6), show station name in stream titles (on), custom user agent (empty).

**Plugins:** selected output, DSP and visualization modules and their indices; visualization auto-start (off); visualization thread priority; disable built-in visualizer while a plugin runs (on); safe mode (off).

**Misc:** recent URL list; last directory; dialog positions; last preferences page; language.

### 1.4 Importing an original `winamp.ini` (recommended)

Offer a one-time import of settings, presets, bookmarks and the default playlist from an existing Winamp installation. The original file is INI with section `[Winamp]`; strings may be UTF-8 (`utf8=1`). Relevant keys:

| Key | Meaning |
|---|---|
| `wx`, `wy` / `eq_wx`, `eq_wy` / `pe_wx`, `pe_wy` / `video_wx`, `video_wy` | Window positions |
| `mw_open`, `eq_open`, `pe_open`, `video_open` | Visibility |
| `pe_width`, `pe_height`, `pe_height_ws`, `video_width`, `video_height` | Sizes (playlist height 14 = shaded) |
| `windowshade`, `eq_ws`, `dsize`, `eqdsize`, `aot`, `easymove` | Window modes |
| `volume`, `pan`, `shuffle`, `repeat`, `pladv` (1 = automatic advance), `pilp` (position), `timeleftmode` | Playback state |
| `eq_data` (`n,n,…` ten values), `preamp`, `use_eq`, `autoload_eq`, `eq_frequencies`, `eq_limiter`, `eq_type` | Equalizer |
| `sa` (0 off, 1 spectrum, 2 oscilloscope), `safire` (bits 0–1 analyzer style; bits 2–3 scope style; bit 5 thin bands), `saref`, `safalloff`, `sa_peaks`, `sa_peak_falloff` | Classic visualizer |
| `skin`, `randskin`, `SkinDir` | Skin |
| `snap`, `snaplen`, `hilite`, `ascb_new`, `ttips`, `usecursors`, `bifont`, `bifont_alt`, `dotitlenum`, `dotasknum`, `ospb`, `keeponscreen`, `autoscrollname`, `embedwnd_freesize` | Classic UI options |
| `pe_fontsize`, `custom_plfont`, `playlist_custom_font`, `pe_direction`, `shownumsinpl`, `zeropadplnum`, `plscrollsize`, `plmw2xscroll` | Playlist view |
| `titlefmt`, `useexttitles`, `riol` (1 on load, 0 on display, 2 on play, 4 background), `fixtitles` (bit 0 `%20`, bit 1 `_`) | Titles |
| `rofiob` (bit 0 sort on add, bit 1 do **not** recurse), `defext`, `shuffle_morph_rate`, `playlist_recyclebin` | Playlist behaviour |
| `replaygain`, `replaygain_mode`, `replaygain_source`, `replaygain_preferred_only`, `replaygain_non_rg_gain` and `replaygain_preamp` (both dB × 1000) | ReplayGain |
| `audio_bits`, `audio_mono`, `audio_surround`, `audio_dither` | Output format |
| `proxy`, `proxyonly80` | Network |
| `minst`, `taskbar` (0 task bar, 1 tray, 2 both, 3 neither), `nomwheel`, `dropaotfs` | General |

---

## 2. Keyboard shortcuts

Use physical key positions for letter shortcuts so they work with any keyboard layout. On macOS, Ctrl shortcuts may map to Cmd as an option; the default must keep the classic keys working.

### 2.1 Global (active in every player window)

| Key | Command |
|---|---|
| Ctrl+P | Preferences |
| Ctrl+K, Alt+O | Preferences ▸ Visualization plugins |
| Alt+K | Configure visualization plugin |
| Ctrl+Shift+K | Start/stop visualization plugin |
| Ctrl+D | Double size |
| Ctrl+W | Windowshade the focused window (main, EQ or playlist) |
| Ctrl+T | Toggle elapsed/remaining time |
| Ctrl+J | Jump to time |
| Ctrl+O / Ctrl+S | Open playlist / Save playlist |
| Ctrl+Tab | Focus next player window (main, EQ, playlist, video, generic windows) |
| Ctrl+Alt+B | Bookmark current track |
| Ctrl+Alt+N | Start a new instance |
| Alt+W | Show/hide main window |
| Alt+G | Show/hide equalizer |
| Alt+E | Show/hide playlist |
| Alt+V | Show/hide video window |
| Alt+F | Open main menu at the menu button |
| Alt+M | Minimize (Shift: to tray) |
| Alt+Enter | Video full screen |
| Ctrl+F1 | About |
| F1 | Help |
| Alt+F4 | Exit |

### 2.2 Main window

| Key | Command |
|---|---|
| Z, Numpad 4 | Previous |
| X, Numpad 5 | Play |
| C | Pause |
| V | Stop |
| B, Numpad 6 | Next |
| Shift+Z / Shift+B | Rewind / fast-forward 5 s |
| Ctrl+Z / Ctrl+B | Start / end of list |
| Shift+V | Stop with fade-out |
| Ctrl+V | Stop after current |
| Left, Numpad 7 / Right, Numpad 9 | Seek −5 s / +5 s |
| Up, Numpad 8 / Down, Numpad 2 | Volume up / down |
| Numpad 1 / Numpad 3 | 10 tracks back / forward |
| L, Numpad 0, Shift+X | Open file(s) |
| Shift+L, Insert | Open folder |
| Ctrl+L, Ctrl+Numpad 0, Ctrl+X | Open URL |
| R / S / Shift+R | Toggle repeat / shuffle / manual advance |
| J, Numpad ., F3 | Jump to file |
| Ctrl+A | Always on top |
| Ctrl+E | EasyMove |
| Alt+3 | File info for current track |
| Alt+S | Skin browser |
| F5 | Reload skin |
| Ctrl+Alt+I | Edit bookmarks |
| Menu key, Shift+F10 | Main menu |

### 2.3 Playlist and equalizer

See 04 §8 and 03 §1.7.

### 2.4 Precedence

The focused window's shortcuts win, then the global shortcuts. Commands from the playlist and EQ windows that they do not handle themselves go to the main window.

---

## 3. Menus

### 3.1 Main menu

Opened by the main-window menu button, Alt+F, right-click on unclaimed areas, and the tray icon.

```
About <player>...                    Ctrl+F1
─────
Play ▸        File...                L
              URL...                 Ctrl+L
              Folder...              Shift+L
              ─────
              Audio CD <drive>       (one item per CD drive)
              Bookmark ▸             (bookmark list)
View file info...                    Alt+3
Bookmarks ▸   Edit bookmarks...      Ctrl+Alt+I
              Add current as bookmark  Ctrl+Alt+B
              ─────
              (bookmark list, or "(no bookmarks)" greyed)
─────
Main Window                          Alt+W     (check)
Playlist Editor                      Alt+E     (check)
Equalizer                            Alt+G     (check)
Video                                Alt+V     (check)
─────
Options ▸     Preferences...         Ctrl+P
              Skins ▸                Skin Browser...  Alt+S
                                     ─────
                                     <default skin>
                                     (installed skins)
              ─────
              Time elapsed           Ctrl+T toggles   (radio)
              Time remaining         Ctrl+T toggles   (radio)
              ─────
              Always On Top          Ctrl+A    (check)
              Double Size            Ctrl+D    (check)
              EasyMove               Ctrl+E    (check)
              ─────
              Repeat                 R         (check)
              Shuffle                S         (check)
Playback ▸    Previous               Z
              Play                   X
              Pause                  C
              Stop                   V
              Next                   B
              ─────
              Stop with fade-out     Shift+V
              Stop after current     Ctrl+V    (check)
              Back 5 seconds         Left
              Forward 5 seconds      Right
              Start of list          Ctrl+Z
              End of list            Ctrl+B
              10 tracks back         Num 1
              10 tracks forward      Num 3
              ─────
              Jump to time           Ctrl+J
              Jump to file           J
Visualization ▸ (06 §6.2)
Skins ▸       (same as Options ▸ Skins)
─────
Help                                 F1
─────
Exit                                 Alt+F4
```

The clutterbar O button opens **Options ▸** at (14,25). The V button opens **Visualization ▸** at (14,56).

### 3.2 Other menus

- Main-window right-click menus: 02 §12.7.
- Playlist menus: 04 §6.
- EQ presets menu: 03 §5.3.
- Video menu: 05 §2.

---

## 4. Command line

Syntax: `<player> [switches] [files, folders, URLs]`. Switches are case-insensitive and must come before file arguments. Quoted arguments may contain spaces.

### 4.1 Single instance

- By default only one instance runs. A second launch sends its request to the running instance and exits.
- With file arguments: the running instance clears its playlist, adds the files and plays (`/ADD`: appends only). When the file manager launches one process per selected file, the running instance must collect all of them into one playlist (accept requests during a short window, about 1 s, and start playback once).
- Skin archives (`.wsz`) passed as arguments are installed into the skins folder and applied, after a confirmation (preference, default on).
- "Allow multiple instances" lets a launch without files start a new instance. `/NEW` always starts one.

### 4.2 Switches

| Switch | Action |
|---|---|
| `/ADD` | Append the files; do not clear, do not play |
| `/NEW` | Start a new instance |
| `/PLAY`, `/PAUSE`, `/STOP`, `/PREV`, `/NEXT` | Transport (sent to the running instance) |
| `/PLAYPAUSE` | Pause if playing, else play |
| `/STOPFADE`, `/STOPAFTER` | Stop with fade-out; toggle stop after current |
| `/FWD`, `/REV` | Seek ±5 s |
| `/VOLUP`, `/VOLDOWN`, `/VOL=<0..100>` | Volume (±4 of 255; or percent) |
| `/PANLEFT`, `/PANRIGHT`, `/PAN=<-127..127>` | Balance (±12) |
| `/SHUFFLE`, `/SHUFFLE=0\|1`, `/RANDOM`, `/RANDOM=0\|1` | Toggle or set shuffle |
| `/REPEAT`, `/REPEAT=0\|1` | Toggle or set repeat |
| `/CLEAR` | Clear the playlist |
| `/JUMPTO` | Open Jump to file |
| `/CD[0-3]` | Play audio CD in drive n |
| `/BOOKMARK <file>` | Add a bookmark, then exit |
| `/QUIT` (also `/EXIT`, `/CLOSE`) | Close the running instance |
| `/KILL` | Close the running instance; force-quit it after 3 s |
| `/STARTMIN`, `/STARTMAX` | Start minimized / restored |
| `/CONFIG=<file>`, `/INIDIR=<dir>`, `/M3UDIR=<dir>` | Settings file, settings folder, playlist folder |
| `/CLASS=<name>` | Instance identity (separate single-instance groups) |
| `/SAFE` | Safe mode (no third-party plugins) |
| `/DELM3U` | Delete the saved default playlist, then exit |
| `/COMMAND=<id>` | Send a numbered command (§6.3) |
| `/?` | Show command-line help |

Only the last transport-type switch is used. When files are also given, the files are processed and the command is ignored.

---

## 5. Global hotkeys

Global hotkeys work while other applications have focus. **Off by default**; enabled in preferences.

Default bindings (when enabled):

| Hotkey | Action |
|---|---|
| Ctrl+Alt+Insert | Play |
| Ctrl+Alt+Home | Pause |
| Ctrl+Alt+End | Stop |
| Ctrl+Alt+Page Up | Previous |
| Ctrl+Alt+Page Down | Next |
| Ctrl+Alt+Up / Down | Volume up / down |
| Ctrl+Alt+Right / Left | Forward / rewind 5 s |
| Ctrl+Alt+J | Jump to file (bring player to front) |
| Ctrl+Alt+L | Open file |
| Media Play/Pause | Play/pause |
| Media Stop / Previous / Next | Stop / previous / next |

Assignable actions: every transport command and variant; volume; seek; repeat on/off/toggle; shuffle toggle; manual advance; start/end of list; jump to file; open file/URL/folder; show/hide EQ, playlist, main window; always on top; double size; windowshade (main, playlist); preferences; skin browser; about; visualization start/stop, configure, next/previous/random preset, full screen; copy current title or file path to the clipboard; show current file in folder; file info; rating 0–5, rating up/down; bring to front / hide; quit.

Media keys and OS media controls (play/pause, next, previous, stop, seek, now-playing title and art) must work even with global hotkeys off, through the platform's media-session interface.

---

## 6. Remote control

### 6.1 Native interface (required)

A local IPC interface (D-Bus on Linux/BSD, a local socket or named pipe elsewhere) and the platform media-session interface (MPRIS 2 on Linux) expose:

**Commands:** play, pause, play/pause, stop, stop with fade, stop after current, next, previous, start/end of list, seek absolute (ms) and relative, set volume (0–255) and balance (−127..127), set position in the playlist, enqueue locations, clear playlist, start playback at the beginning (respecting shuffle), set shuffle/repeat/manual advance, set EQ band/preamp/on/auto-load, load skin, save default playlist, show/hide windows, any numbered command (§6.3).

**Queries:** version; state (0 stopped, 1 playing, 3 paused); position (ms) and length (ms, or unknown); current playlist position and length; next position (prediction); sample rate, bit rate, channels; EQ values; shuffle/repeat/manual advance; location and title of any entry; current title and location; time display mode; metadata field of any location; settings folder; whether a visualization plugin runs.

**Events:** track started (location), stopped (with whether the user stopped or the playlist ended), title changed, volume changed, EQ changed, status changed (play/pause/stop), info changed (bit rate etc.), playlist changed, skin changed, output started.

### 6.2 Windows legacy interface (optional)

On Windows, many tools control Winamp by window messages. An optional compatibility mode registers a window with the class name `Winamp v1.x` that accepts the classic message numbers (`WM_USER` "IPC" messages with an id in lParam, `WM_COMMAND` ids, and `WM_COPYDATA`). Main IPC ids:

| Id | Meaning |
|---|---|
| 0 | Get version (return as 0x20yx for 2.yx compatibility, or the clone's own version) |
| 100 | Enqueue file (WM_COPYDATA with the path; wide version 1100) |
| 101 | Clear playlist |
| 102 | Start playback from the start of the list |
| 104 | Is playing: 0 stopped, 1 playing, 3 paused |
| 105 | wParam 0: position ms (−1 stopped); 1: length s; 2: length ms |
| 106 | Jump to time (ms): 0 OK, −1 not playing, 1 past end |
| 120 | Save default playlist; returns current position |
| 121 | Set playlist position |
| 122 | Set volume 0–255 (wParam −666 = get) |
| 123 | Set balance −127..127 (−666 = get) |
| 124 | Get playlist length |
| 125 | Get playlist position |
| 126 | Get info: 0 sample rate kHz, 1 bit rate kbps, 2 channels, 5 sample rate Hz |
| 127 | Get EQ: 0–9 bands (0..63), 10 preamp, 11 enabled, 12 auto-load |
| 128 | Set EQ value for the position last queried with 127 |
| 129 | Add bookmark (WM_COPYDATA) |
| 136 | Predicted next playlist position |
| 211 / 212 | Get entry file name / title (in-process only) |
| 243 | Refresh title |
| 250 / 252 | Get / set shuffle |
| 251 / 253 | Get / set repeat |
| 610 | Get time display mode |
| 634 / 635 | Get / set manual advance |
| 639 / 640 | Set / get rating of current track |

### 6.3 Numbered commands

These numbers are used by the `WM_COMMAND` interface, the `/COMMAND=` switch and the native "numbered command" call:

| Id | Command | Id | Command |
|---|---|---|---|
| 40044 | Previous | 40144 | Rewind 5 s |
| 40045 | Play | 40145 | Open file dialog |
| 40046 | Pause | 40147 | Stop with fade-out |
| 40047 | Stop | 40148 | Fast-forward 5 s |
| 40048 | Next | 40154 | Start of list |
| 40058 | Volume up | 40155 | Open URL dialog |
| 40059 | Volume down | 40157 | Stop after current |
| 40060 | Forward 5 s | 40158 | End of list |
| 40061 | Rewind 5 s | 40165 | Double size |
| 40001 | Exit | 40185 | Open URL |
| 40012 | Preferences | 40186 | EasyMove |
| 40019 | Always on top | 40187 | Open folder |
| 40022 | Repeat | 40188 | File info |
| 40023 | Shuffle | 40192 | Start/stop visualization plugin |
| 40029 | Open file(s) | 40193 | Jump to time |
| 40036 | Show/hide EQ | 40194 | Jump to file |
| 40037 | Time elapsed | 40195 / 40197 | 10 tracks forward / back |
| 40038 | Time remaining | 40219 | Skin browser |
| 40040 | Show/hide playlist | 40242 | Toggle time display |
| 40041 | About | 40258 | Show/hide main window |
| 40043 | Main menu | 40266 | Playlist windowshade |
| 40063 | Next window | 40291 | Reload skin |
| 40064 | Main windowshade | 40321 | Bookmark current |
| 40065 | Windowshade focused window | 40328 | Show/hide video |
| | | 40395 | Manual advance |

---

## 7. Tray / status icon

- Shown when the "show in tray" preference is on, or when the window is minimized with Shift (minimize to tray).
- Tooltip: the window caption (`N. Title - <player>`).
- Left click: show and raise the player (or bring it to front). Ctrl+click: open URL. Shift+click: open files.
- Right click: the main menu.
- Optional extra tray buttons (previous, play/pause, stop, next, open, volume) as separate icons where the platform supports several icons.

---

## 8. Preferences dialog

A dialog with a tree of pages. Remember the last page and the dialog position. Pages:

| Page | Contents |
|---|---|
| General | Show in task bar/tray; tray icon; allow multiple instances; splash screen; scroll title in task bar; show playlist number in task bar; use trash; mouse wheel volume; drop always-on-top for full screen; proxy |
| File types | Associate file extensions and playlist types (where the platform allows); enqueue on double-click instead of play; folder context-menu actions |
| Playlist | Numbers, zero-pad, font and size, text direction, page scroll lines, wheel double scroll; manual advance; sort on add; assumed extension; shuffle morph rate |
| Titles | When to read metadata; advanced title formatting on/off; format string with Default and Help buttons; `_` and `%20` as spaces |
| Playback | Thread priority; output bit depth; surround; dither; mono. Tabs: Equalizer (engine, band mode, limiter); ReplayGain (07 §5.1) |
| Video | 05 §2 options |
| Skins | Skin list, install/rename/uninstall, skins folder, random skin, prompt on install, skin info (readme) |
| Classic skins | Dim inactive title bars; snap and distance; smooth resizing; tooltips; skinned cursors; playlist position in title; always show clutterbar; double size for EQ; bitmap title font; anti-aliased title in double size; old-school playlist buttons; keep on screen. Tab "Classic visualization": 06 §6.3 |
| Plugins | Input, Output, Visualization, DSP/Effect, General pages: list, configure, about, uninstall; plugin folders; visualization priority and auto-start; disable built-in visualizer while a plugin runs; safe mode |
| Global hotkeys | §5 |
| Language | Language selection (translations) |

---

## 9. About dialog

Tabs: the player (name, version, licence, link); credits; keyboard shortcuts (text summary of §2); version history. Do not reuse the original's logo, artwork or credits text.
