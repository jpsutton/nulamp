# 01 — Classic Skin Format

This document defines how the player finds, loads, and interprets a classic (Winamp 2.x style) skin. Window-specific sprite tables are in documents 02–05. Everything here is an interoperability requirement: real-world skins in the wild depend on this exact behaviour.

Conventions used in all skin documents:

- All coordinates are in **skin pixels at 1x scale**. Double-size mode scales the result (see 02 §9).
- A rectangle written `(x,y) WxH` has its top-left corner at x,y and is W pixels wide and H pixels high.
- A hit box written `x1..x2 × y1..y2` is **inclusive** on all four edges.
- "Source" means a rectangle in a skin bitmap. "Destination" means a position in a window's client area.
- All sprites are copied opaquely. There is no colour-key transparency and no alpha in classic skins. Window shape comes only from `region.txt`.

---

## 1. Skin containers

### 1.1 Archive skins

- A skin archive is a ZIP file. The extensions `.wsz` and `.zip` both mean "classic skin archive". The extension check is case-insensitive.
- A `.wal` file is a "modern" (XML-based) skin. Modern skins are out of scope. The player must reject them with a clear message, or fall back to the default skin.
- An archive that contains an entry named `skin.xml` (at the root, or inside a top-level folder) is a modern skin, whatever its extension.
- For a classic archive, the player uses only entries whose extension is `.bmp`, `.txt`, `.cur`, or `.ini` (case-insensitive). Other entries are ignored. **PNG, GIF, JPEG and `.ani` files in a classic skin are not used.**
- Folder structure inside the archive is **flattened**: only the final path component of each entry matters. A skin whose files are all inside one sub-folder must still work.
- If two entries have the same final name (in different folders), the **later entry in archive order wins**.
- Implementation may extract to a temporary directory or read entries directly from memory. Either way, the lookup rules in §1.3 apply.
- Archives with non-ASCII names, CP437 names, or UTF-8 names must all be readable. Entry names are matched after case folding (§1.3).

### 1.2 Directory skins

- A directory inside the skins folder is a classic skin used in place. Lookup rules are the same as for archives (§1.3), but only the top level of the directory is searched.
- `file_id.diz` is usable as skin info only for directory skins (because archives keep only `.txt`, `.bmp`, `.cur`, `.ini`).

### 1.3 File-name matching

The original player ran on a case-insensitive file system. Real skins name files in any case (`MAIN.BMP`, `Main.bmp`, `main.bmp`). **The player must match every skin file name case-insensitively**, on every platform, for archive entries and directory files.

### 1.4 Skin locations and selection

- The player has a user skins directory (configurable). The skin browser lists:
  - the built-in default skin (always first),
  - every sub-directory of the skins directory,
  - every `.wsz` and `.zip` file in the skins directory.
- The display name of a skin is its file or directory name without the extension.
- The current skin setting stores a name relative to the skins directory, or an absolute path. Empty means the built-in default skin.
- Optional feature: "random skin on every play". When enabled, each time playback starts, a random entry from the list (built-in default included, equal weight) is loaded.
- Dropping a `.wsz` file on any player window installs it (copies it into the skins directory) and applies it. This is recommended, not required.

### 1.5 Skin info (readme)

The skin browser shows a text description of the selected skin. Search order (case-insensitive):

1. `readme.txt`
2. `read me.txt`
3. `file_id.diz` (directory skins only)
4. The first other `.txt` file that is not `pledit.txt`, `viscolor.txt` or `region.txt`. (The original had a defect that skipped this step. A new implementation should perform it.)

Text decoding: a UTF-8 byte-order mark means UTF-8. Bytes `FF FE` mean UTF-16LE. Otherwise decode as Windows-1252. Truncate the display at 32 KiB.

---

## 2. Files a classic skin may contain

Every file is optional. **Each bitmap falls back independently** to the matching bitmap of the built-in default skin. A partial skin is therefore drawn as a composite of the skin and the default skin.

| File | Used by | Missing-file behaviour |
|---|---|---|
| `main.bmp` | Main window background | Default |
| `titlebar.bmp` | Main window title bar, clutterbar, shade mode | Default |
| `cbuttons.bmp` | Transport buttons + eject | Default |
| `playpaus.bmp` | Play/pause/stop indicator | Default |
| `monoster.bmp` | Mono / stereo indicators | Default |
| `shufrep.bmp` | Shuffle, repeat, EQ toggle, PL toggle | Default |
| `numbers.bmp` | Large time digits | Default (only used when `nums_ex.bmp` is absent) |
| `nums_ex.bmp` | Large time digits with minus sign | **If present, `numbers.bmp` is ignored completely.** No default exists. |
| `volume.bmp` | Volume slider | Default |
| `balance.bmp` | Balance slider | **If absent in a skin, the balance slider is drawn from the skin's `volume.bmp`** (or the default `volume.bmp` if that is also missing). See 02 §6. |
| `posbar.bmp` | Seek bar | Default |
| `text.bmp` | Bitmap font, small digits, title-text colours | Default |
| `eqmain.bmp` | Equalizer window | Default |
| `eqmain_iso.bmp` | Equalizer window when ISO band labels are selected | Falls back to `eqmain.bmp`, then default |
| `eq_ex.bmp` | Equalizer windowshade mode, pressed shade button | Default (see 03 §1.1 for a side effect) |
| `pledit.bmp` | Playlist editor | Default |
| `gen.bmp` | Generic ("embedded") window frame and its title font | Default |
| `genex.bmp` | Colours and widgets for generic window contents | Default |
| `video.bmp` | Video window frame | Default |
| `mb.bmp` | Minibrowser frame (legacy; see 05 §4) | Default |
| `avs.bmp` | Legacy AVS frame (see 05 §5) | Not needed |
| `viscolor.txt` | Visualizer colours | Default palette |
| `pledit.txt` | Playlist colours and font | Defaults |
| `region.txt` | Non-rectangular window shapes | Rectangular windows |
| `*.cur` | Mouse cursors (§7) | Default cursors |
| `readme.txt` etc. | Skin info | — |

### 2.1 Bitmap decoding

- Only the Windows BMP format is used. The decoder must support 1, 4, 8, 16, 24 and 32 bits per pixel, top-down and bottom-up rows, palettes, RLE4 and RLE8 compression, and BITFIELDS masks. Many skins use 8-bit palettised or RLE files.
- The 32-bit alpha channel, if present, is ignored (classic skins are opaque).
- Bitmaps are not size-checked. A bitmap may be larger than expected; the extra area is never shown. If a bitmap is **smaller** than a sprite rectangle requires, pixels outside the bitmap must be drawn as black. (The original behaviour depended on the OS graphics layer. Black is the recommended consistent choice.)
- A corrupt or undecodable bitmap is treated as missing (fallback to the default).

### 2.2 Default (built-in) bitmap sizes

The built-in default skin must provide every bitmap with these sizes. These sizes also tell skin authors and the conformance tests the expected layout:

| Bitmap | Size |
|---|---|
| main | 275×116 |
| titlebar | 344×87 |
| cbuttons | 136×36 |
| playpaus | 42×9 |
| monoster | 58×24 |
| shufrep | 92×85 |
| numbers | 99×13 |
| nums_ex | 108×13 (no default; size given for skins) |
| volume | 68×433 |
| balance | 68×433 |
| posbar | 307×10 |
| text | 155×74 (only rows 0–17 are used) |
| eqmain / eqmain_iso | 275×315 |
| eq_ex | 275×82 |
| pledit | 280×186 |
| gen | 194×109 |
| genex | 130×75 |
| video | 234×119 |
| mb | 234×119 |

**Copyright note.** The original default skin artwork is not free to redistribute. The new implementation must ship its own original default skin that follows these sizes and layouts.

---

## 3. Skin load sequence

When a skin is applied (at start-up, from the skin browser, from a menu, from a hotkey, or from remote control):

1. Release the previous skin's resources and delete any temporary extraction.
2. Open the archive or directory.
3. Reset these to defaults: region polygons, `pledit.txt` colours and font, visualizer palette, cursors.
4. Parse `region.txt`, `pledit.txt`, `viscolor.txt`.
5. Load all bitmaps (each with independent fallback).
6. Derive the title-text colours from `text.bmp` (§6.2) and the generic-window font metrics from `gen.bmp` (05 §1.3).
7. Load cursors.
8. Redraw every window. Re-apply window shapes after the windows have their final size.
9. Notify plugins and remote-control clients that the skin changed.

A skin change must not interrupt playback.

---

## 4. `region.txt` — window shapes

### 4.1 Syntax

- INI file. Section names and key names are case-insensitive. Lines that start with `;` are comments. Values can be very long (support at least 64 KiB per value).
- Sections used:
  - `[Normal]` — main window, normal mode
  - `[WindowShade]` — main window, windowshade mode
  - `[Equalizer]` — equalizer window, normal mode
  - `[EqualizerWS]` — equalizer window, windowshade mode
- Sections `[Playlist]` and `[PlaylistWS]` appear in some skins. **They must be ignored** (the reference player ignores them; honouring them would break those skins).
- Keys in each section:
  - `NumPoints` — a list of integers. Each integer is the vertex count of one polygon.
  - `PointList` — a flat list of integers: x,y pairs for all polygons, concatenated in order.

### 4.2 Number parsing

- Separators between numbers: any mix of spaces, tabs and commas.
- A number is an optional leading `-` followed by decimal digits.
- Parsing of a value stops at the first token that is not a number. The rest of the value is ignored.

### 4.3 Validation

- The sum of `NumPoints` must equal half the count of `PointList` numbers, and the `PointList` count must be even.
- If validation fails, that section is treated as absent.

### 4.4 Shape semantics

- The window shape is the **union of all polygons**, filled with the non-zero winding rule.
- Polygon coordinates are pixel-edge positions: the polygon `0,0 275,0 275,116 0,116` covers pixels x 0–274 and y 0–115. Right and bottom edges are exclusive.
- In double-size mode, every coordinate is multiplied by 2.
- Main window: use `[WindowShade]` in windowshade mode, else `[Normal]`. If the needed section is absent, the window is a full rectangle of its current size. (A skin with `[Normal]` but without `[WindowShade]` gets a rectangular shade bar.)
- Equalizer: same rule with `[EqualizerWS]` and `[Equalizer]`. Note: the reference player doubles equalizer points whenever global double size is on, even if the equalizer itself is not doubled. Reproducing this quirk is optional; a clone should double points only when the equalizer is actually drawn at 2x.
- Regions are applied only when a skin (not the built-in default) is active.
- Pixels outside the shape are fully transparent and do not receive mouse input. On platforms without per-window shape support, implement shape by per-pixel alpha with input pass-through.

---

## 5. `viscolor.txt` — visualizer palette

### 5.1 Meaning of the 24 colours

| Index | Use | Default R,G,B |
|---|---|---|
| 0 | Background | 0,0,0 |
| 1 | Background grid dots | 24,24,41 |
| 2 | Spectrum, top (loudest) | 239,49,16 |
| 3 | Spectrum | 206,41,16 |
| 4 | Spectrum | 214,90,0 |
| 5 | Spectrum | 214,102,0 |
| 6 | Spectrum | 214,115,0 |
| 7 | Spectrum | 198,123,8 |
| 8 | Spectrum | 222,165,24 |
| 9 | Spectrum | 214,181,33 |
| 10 | Spectrum | 189,222,41 |
| 11 | Spectrum | 148,222,33 |
| 12 | Spectrum | 41,206,16 |
| 13 | Spectrum | 50,190,16 |
| 14 | Spectrum | 57,181,16 |
| 15 | Spectrum | 49,156,8 |
| 16 | Spectrum | 41,148,0 |
| 17 | Spectrum, bottom | 24,132,8 |
| 18 | Oscilloscope, centre (brightest) | 255,255,255 |
| 19 | Oscilloscope | 214,214,222 |
| 20 | Oscilloscope | 181,189,189 |
| 21 | Oscilloscope | 160,170,175 |
| 22 | Oscilloscope, extremes | 148,156,165 |
| 23 | Spectrum peak dots | 150,150,150 |

How these colours are used is defined in 06.

### 5.2 Parsing rules

- One colour per line, in index order 0–23.
- For each of R, G, B: skip any run of spaces, commas and tabs, then read decimal digits.
- If a component has **no digits** (blank line, a line starting with text, a `-` sign), **parsing of the whole file stops**. Colours not yet read keep their defaults. Components of the current line that were already read keep their new values.
- Text after the third number (for example `// background`) is ignored.
- Values above 255 wrap modulo 256 (no clamping).
- Fewer than 24 lines: remaining colours keep defaults.
- Quirk: in the reference player, **a final line with no trailing newline is ignored**, and lines longer than 89 characters are split, with the remainder parsed as the next colour. Implementations should reproduce the final-line quirk (some skins look different without it). The long-line quirk is optional.
- `viscolor.txt` is read only for real skins. The built-in skin always uses the default palette (or its own file).

---

## 6. Text rendering resources

### 6.1 `text.bmp` bitmap font

`text.bmp` holds 5×6 pixel glyph cells. A cell at column c and row r has its source rectangle at `(c*5, r*6) 5x6`. Three rows are used.

**Row 0 (y = 0):**

| Column | Glyph |
|---|---|
| 0–25 | `A`–`Z` (lower-case letters map to the same cells) |
| 26 | `"` |
| 27 | `@` |
| 28, 29 | (unused) |
| 30 | space — **also the fallback for every unmapped character** |

**Row 1 (y = 6):**

| Column | Glyph | Column | Glyph |
|---|---|---|---|
| 0–9 | `0`–`9` | 20 | `\` |
| 10 | `…` (ellipsis) | 21 | `/` |
| 11 | `.` | 22 | `[` `{` `<` |
| 12 | `:` | 23 | `]` `}` `>` |
| 13 | `(` | 24 | `~` `^` |
| 14 | `)` | 25 | `&` |
| 15 | `-` | 26 | `%` |
| 16 | `'` and `` ` `` | 27 | `,` |
| 17 | `!` | 28 | `=` |
| 18 | `_` | 29 | `$` |
| 19 | `+` | 30 | `#` |

**Row 2 (y = 12):**

| Column | Glyph |
|---|---|
| 0 | `Å` `å` |
| 1 | `Ö` `ö` |
| 2 | `Ä` `ä` |
| 3 | `?` |
| 4 | `*` |
| 20 | blank (used by the bit-rate display) |

**Accent folding** before lookup: `°`→`0`; `Ç`→`C`; `É`→`E`; `Æ`→`A`; `Ü`→`U`; `ü ú ù û`→`u`; `è ë ê é`→`e`; `á à â æ`→`a`; `ç`→`c`; `í ì î ï`→`i`; `ó ò ô`→`o`; `ÿ`→`y`; `ƒ`→`f`; `Ñ ñ`→`n`. A new implementation should also fold other Latin accented letters with Unicode decomposition before falling back to space.

Other small glyphs taken from `text.bmp` by fixed coordinates:

- Small blank: `(142,0)` 3×6 or 4×6 (inside the space cell).
- Small minus: `(75,6)` 3×6 (part of the `-` cell).

### 6.2 System-font title colours

When the main-window title, the playlist windowshade line, or video info text uses a system (vector) font instead of the bitmap font, its colours come from `text.bmp`:

- **Background** = the pixel at `(150,4)` (inside the row-0 space cell).
- **Foreground** = among the pixels in the rectangle x 0–19, y 0–5 (the glyphs A–D), the pixel with the largest squared RGB distance from the background. Scan row by row, left to right. On a tie, the first pixel found wins.

### 6.3 `pledit.txt`

INI file. Section `[Text]`:

| Key | Meaning | Default |
|---|---|---|
| `Normal` | Playlist text colour | `#00FF00` |
| `Current` | Text colour of the playing entry | `#FFFFFF` |
| `NormalBG` | Playlist background | `#000000` |
| `SelectedBG` | Background of selected entries | `#0000C6` |
| `mbFG` | Info text in video / minibrowser windows | `#00FF00` |
| `mbBG` | Info text background in video / minibrowser windows | `#000000` |
| `Font` | Font face name for playlist and main-window title text (UTF-8) | empty (use the player's default UI font) |

- Colour values are hexadecimal `RRGGBB`. A leading `#` is optional. Parse leniently: read hex digits until the first non-hex character. Missing or invalid keys keep defaults.
- Section `[Misc]`, key `UseGenNums` (0/1, default 0 for skins; the built-in skin behaves as 1): enables the digit, `-` and `:` glyphs of `gen.bmp` in generic-window titles (05 §1.3).
- **Font substitution.** Skins name Windows fonts (`Arial`, `Tahoma`, `Verdana`, `MS Sans Serif`, ...). The player must map these to metric-compatible fonts when the named font is not installed (for example Arial → Liberation Sans / Arimo). The fallback when `Font` is empty or unresolvable is a sans-serif UI font that covers as much of Unicode as possible.

### 6.4 `colors.ini` (optional)

Some classic skins include `colors.ini` to colour skinned popup menus and tooltips. Format: first line `[colors]`, then `name=value` lines, where value is `#rrggbb` or `r,g,b`. Support is optional. If the player draws its own skinned menus, it may use these colours; otherwise ignore the file.

---

## 7. Cursors

- Cursor files are Windows `.cur` files. Some skins put animated cursor data (RIFF `ACON`) in a `.cur` file; the player should accept that and may show only the first frame.
- Skinned cursors are used only when the user preference "use skinned cursors" is on (default on).
- Each cursor falls back independently to a built-in cursor of the given kind.

| File | Kind | Where it is shown |
|---|---|---|
| `Normal.cur` | arrow | Main window: any area not listed below |
| `TitleBar.cur` | move | Main window title bar (x 0–275, y 0–13) |
| `MainMenu.cur` | arrow | Main window menu button (x 5–17, y 3–13) |
| `Min.cur` | arrow | Main window minimize button (normal and shade mode) |
| `WinBut.cur` | arrow | Main window shade/unshade button; also while a main-window popup menu is open |
| `Close.cur` | close/"danger" | Main window close button (normal and shade mode) |
| `VolBal.cur` | horizontal resize | Volume and balance sliders (x 107–215, y 58–66) |
| `Posbar.cur` | horizontal resize | Seek bar (x 18–263, y 73–81) |
| `SongName.cur` | horizontal resize | Song title area (x 105–266, y 24–35) |
| `MMenu.cur` | arrow | Main window shade mode: menu button |
| `WSPosbar.cur` | horizontal resize | Main window shade mode: mini seek bar (x 228–242, y 3–12) |
| `WSNormal.cur` | arrow | Main window shade mode: everything else |
| `EQSlid.cur` | vertical resize | Equalizer sliders |
| `EQClose.cur` | close | Equalizer close button |
| `EQTitle.cur` | move | Equalizer title bar |
| `EQNormal.cur` | arrow | Equalizer: everything else |
| `PTBar.cur` | move | Playlist title bar; video and generic window title bars |
| `PClose.cur` | close | Playlist close button (normal and shade); video and generic close buttons |
| `PWinBut.cur` | arrow | Playlist shade / unshade button |
| `PVScroll.cur` | vertical resize | Playlist scroll bar |
| `PSize.cur` | diagonal resize | Playlist resize corner; video and generic resize corners |
| `PNormal.cur` | arrow | Playlist, video and generic windows: everything else |
| `PWSSize.cur` | horizontal resize | Playlist shade mode: resize handle |
| `PWSNorm.cur` | arrow | Playlist shade mode: everything else |

Cursor hit areas for the main window are tested in the order of the table. The first match wins.

---

## 8. Skin authoring compatibility checklist

A conformant implementation renders these real-world cases correctly (see 12 for tests):

1. Skin files with upper-case or mixed-case names.
2. Archive with all files inside one sub-folder.
3. Skin with only `main.bmp` (everything else from the default skin).
4. Skin with `nums_ex.bmp` and a different `numbers.bmp` (only `nums_ex.bmp` is used).
5. Skin without `balance.bmp` (balance drawn from `volume.bmp`).
6. Skin whose `volume.bmp` is only 420 pixels high (no thumb sprites: thumbs draw black or nothing, see 02 §6).
7. 8-bit RLE bitmaps.
8. `viscolor.txt` with comments after each line, and one with no trailing newline.
9. `region.txt` with `[Normal]` only.
10. Skin without `eq_ex.bmp` (no pressed shade-button sprite on the equalizer; see 03).
