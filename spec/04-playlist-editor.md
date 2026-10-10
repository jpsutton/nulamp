# 04 — Playlist Editor

The playlist editor (PL) is a resizable skinned window. This document covers its drawing, layout, input and list operations. Playlist file formats and title formatting are in 10. Playback order (shuffle, repeat) is in 07.

---

## 1. Geometry

- Default size 275×116. Default position directly below the equalizer (see 05 §6.4).
- Size snaps to steps: width = 275 + 25k, height = 116 + 29k (k ≥ 0). Minimum 275×116.
- Resize drag: new width rounds **up** to the next step (add 24, then remove the remainder modulo 25); same for height with 28 and 29.
- Optional preference "smooth resizing" (default off) disables the snapping. Then the leftover pixels of tiled parts are filled by stretching a tile (§2.2).
- **Resize grip:** in normal mode, the triangle in the bottom-right 20×20 corner where (w − x) + (h − y) ≤ 30. In windowshade mode, a horizontal-only grip at x w−29 .. w−20.
- **The playlist editor is never drawn at double size.**
- **Windowshade:** height 14. The previous height is remembered and restored on unshade (minimum 116). Width stays resizable in shade mode.

Layout in normal mode (w, h = window size):

| Area | Rectangle |
|---|---|
| Title bar | y 0–19 |
| List | x 12 .. w−20, y 20 .. h−38 |
| Right edge (scroll bar) | x w−20 .. w |
| Bottom bar | y h−38 .. h |

---

## 2. `pledit.bmp` sprites

### 2.1 Sprite table

Title pieces use source y = 0 when active and y = 21 when inactive.

| Element | Source | Destination |
|---|---|---|
| Title left corner | (0,y) 25×20 | (0,0) |
| Title centre (title text) | (26,y) 100×20 | centred (§2.2) |
| Title fill tile | (127,y) 25×20 | both sides of the centre |
| Title right corner (has shade and close buttons) | (153,y) 25×20 | (w−25,0) |
| Left side tile | (0,42) 12×29 | (0, 20 + 29k) |
| Right side, outer part 1 | (31,42) 5×29 | (w−20, 20 + 29k) |
| Scroll bar groove | (36,42) 8×29 | (w−15, 20 + 29k) |
| Right side, outer part 2 | (44,42) 7×29 | (w−7, 20 + 29k) |
| Scroll thumb normal / pressed | (52,53) / (61,53) 8×18 | (w−15, 20 + offset) |
| Bottom left (ADD, REM, SEL, MISC) | (0,72) 125×38 | (0, h−38) |
| Bottom right (info, mini transport, time, LIST, grip) | (126,72) 150×38 | (w−150, h−38) |
| Bottom fill tile | (179,0) 25×38 | between the two bottom parts |
| Bottom visualizer area | (205,0) 75×38 | just left of bottom-right, only when w ≥ 350 |
| Close button pressed | (52,42) 9×9 | (w−11,3) |
| Close button normal (normal and shade mode) | (167,3) 9×9 | (w−11,3) |
| Shade button normal (normal mode) | (158,3) 9×9 | (w−20,3) |
| Shade button pressed (normal mode) | (62,42) 9×9 | (w−20,3) |
| Unshade button normal (shade mode) | (128,45) 9×9 | (w−20,3) |
| Unshade button pressed (shade mode) | (150,42) 9×9 | (w−20,3) |
| Shade bar left | (72,42) 25×14 | (0,0) |
| Shade bar fill | (72,57) 25×14 | tiled |
| Shade bar right, active / inactive | (99,42) / (99,57) 50×14 | (w−50,0) |

### 2.2 Tiling rules

- **Title:** let n = (w − 150) / 25 fill tiles in total. Left of the centre piece: if n is odd, a 12-pixel-wide slice of the fill tile, then n/2 whole tiles. Right of the centre: if n is odd, a 13-pixel slice, then n/2 whole tiles. Then any remainder (w − 150) mod 25 (only with smooth resizing) is filled by stretching a fill tile. Then the right corner.
- **Sides:** (h − 58) / 29 whole tiles, then any remainder stretched.
- **Bottom:** k = (w − 275) / 25 fill tiles. If k ≥ 3, three of them are replaced by the 75-pixel visualizer piece, placed immediately left of the 150-pixel bottom-right piece.
- **Shade bar:** left piece, fill tiles, remainder stretched from a 24-pixel slice, right piece.

---

## 3. List text

### 3.1 Font and rows

- Font face: the user's playlist font if "use custom font" is on, else `pledit.txt` `Font`, else the default UI font. Default size 12 pixels. Not anti-aliased by default (an option may enable it).
- Ctrl+Numpad+ / Ctrl+Numpad− change the size (minimum 2). Ctrl+Enter resets it.
- Row height = the font's line height.
- Visible rows = (h − 38 − 20 − 2) / row height. Row i starts at y = 22 + i × row height.
- Text rectangle per row: x 12 .. w−20.
- Fill the 2-pixel strip y 20–22 and any space below the last row with `NormalBG`.

### 3.2 Row content

- Number prefix, when "show numbers" is on (default on): `N. ` where N is right-aligned to the digit count of the list length (1 digit below 10 entries, 2 below 100, and so on, up to 5). Pad with spaces, or with zeros when "zero-pad numbers" is on (default off).
- Then the entry title (10 §5).
- Duration on the right: `" M:SS "` (one space before and after; minutes not limited) right-aligned at the right edge, vertically centred. Drawn only when the length is known and it fits.
- The title is drawn left-aligned in the remaining space. Long titles are truncated with an ellipsis.
- Right-to-left text: an option (auto / left-to-right / right-to-left) mirrors the layout.

### 3.3 Colours

| Condition | Text colour | Background |
|---|---|---|
| Normal row | `Normal` | `NormalBG` |
| Playing entry | `Current` | `NormalBG` |
| Selected row | `Normal` | `SelectedBG` |
| Selected and playing | `Current` | `SelectedBG` |

If the playing entry was removed from the list, no row is "current".

### 3.4 Windowshade text

- One line at x 4 .. w−29, y 2, height 10.
- Font: same face at 10 pixels, **upper-cased** text.
- Colours: the `text.bmp` title colours (01 §6.2).
- Content: `N. TITLE` of the current entry, and its time right-aligned. With an empty list: `<no file>` (or an equivalent message).
- If the bitmap-font preference is on, use `text.bmp` glyphs instead; the time starts at x = w − 29 − 5 × (time string length); if the title is truncated, its last visible character becomes the ellipsis glyph.

---

## 4. Bottom bar

### 4.1 Info string

18 characters of `text.bmp` 5×6 glyphs at (w−143, h−28), padded with spaces or truncated. Content: `SELECTED/TOTAL`:

- Each part is `M:SS` below 1 hour, else `H:MM:SS`. Minutes and hours are not zero-padded. Empty set: `0:00`.
- TOTAL sums every entry with a known length. SELECTED sums the selected entries.
- Append `+` to a part when some entries in that set have unknown length and the known sum is above 0.
- Show `?` when all entries in the set have unknown length.

### 4.2 Mini transport (part of the bottom-right sprite)

Hit area x w−144 .. w−91, y h−15 .. h−8. With t = w − 144:

| Button | x range |
|---|---|
| Previous | t .. t+6 |
| Play | t+7 .. t+16 |
| Pause | t+17 .. t+26 |
| Stop | t+27 .. t+35 |
| Next | t+36 .. t+44 |
| Eject | t+45 .. t+53 |

Action on mouse-down. Shift and Ctrl variants as on the main window. Eject: Ctrl = open URL, Shift = open folder, else open files.

### 4.3 Mini time display

`text.bmp` digits at origin x = w − 86, y = h − 15:

| Element | x | Notes |
|---|---|---|
| Sign slot | x | minutes < 100: a 4×6 blank at x, sign glyph at x + 4. Minutes ≥ 100: sign glyph at x, hundreds digit at x + 4 |
| Minutes tens | x + 9 | |
| Minutes ones | x + 14 | |
| Seconds tens | x + 22 | the colon is part of the skin |
| Seconds ones | x + 27 | |

Sign glyph: minus `(75,6)` in remaining-time mode, else blank `(142,0)`. Clicking the time (w−87 .. w−53, h−18 .. h−9) toggles elapsed/remaining.

### 4.4 Scroll arrows

Small up/down buttons inside the bottom-right sprite: up at (w−15 .. w−7, h−36 .. h−32), down at (w−15 .. w−7, h−31 .. h−27). Each click scrolls by the "page scroll lines" setting (default 1).

### 4.5 Mini visualizer

When the window is at least 350 pixels wide and not shaded, a 72×16 visualizer sits at (w−223, h−26). **It shows the classic visualizer only when the main window is hidden.** Left-click while playing cycles the mode; right-click opens the visualization menu.

---

## 5. Scroll bar

- Track: x w−15, 8 pixels wide, y 20 .. h−38 (track height = h − 58).
- Thumb 18 pixels tall at y = 20 + (track height − 18) × first visible row / (entry count − visible rows). At the top when the list fits.
- Drag: if the pointer is on the thumb keep the grab offset, else centre (offset 9). First visible row = (pointer y − offset − 20) × (count − visible) / (h − 76), clamped.
- Pressed thumb sprite while dragging.

---

## 6. Bottom-button pop-up menus

### 6.1 Skinned pop-ups

Pressing ADD, REM, SEL, MISC or LIST shows a vertical stack of 22×18 skinned items above the button, plus a 3-pixel-wide bar to its left. Item rows from the bottom: row 0 at y = h−30, row 1 at h−48, row 2 at h−66, row 3 at h−84. The normal sprite of an item is at source x = X; the highlighted sprite is at x = X + 23 (same y).

| Menu | Bar source → destination | Items x | Items, bottom (row 0) to top: action — source normal / highlighted |
|---|---|---|---|
| ADD | (48,111) 3×54 → (11, h−66) | 14 | 0 Add file — (0,149)/(23,149); 1 Add folder — (0,130)/(23,130); 2 Add URL — (0,111)/(23,111) |
| REM | (100,111) 3×72 → (40, h−84) | 43 | 0 Remove selected — (54,149)/(77,149); 1 Crop — (54,130)/(77,130); 2 Remove all — (54,111)/(77,111); 3 Remove misc ▸ — (54,168)/(77,168) |
| SEL | (150,111) 3×54 → (69, h−66) | 72 | 0 Select all — (104,149)/(127,149); 1 Select none — (104,130)/(127,130); 2 Invert selection — (104,111)/(127,111) |
| MISC | (200,111) 3×54 → (98, h−66) | 101 | 0 Misc options ▸ — (154,149)/(177,149); 1 File info ▸ — (154,130)/(177,130); 2 Sort list ▸ — (154,111)/(177,111) |
| LIST | (250,111) 3×54 → (w−47, h−66) | w−44 | 0 Load list — (204,149)/(227,149); 1 Save list — (204,130)/(227,130); 2 New list — (204,111)/(227,111) |

Button hit boxes (the row-0 item position), y h−30 .. h−12: ADD x 14–35, REM 43–64, SEL 72–93, MISC 101–122, LIST w−44 .. w−22.

### 6.2 Pop-up behaviour

- Mouse-down on a button opens the pop-up and highlights the item under the pointer as it moves.
- **Default behaviour:** click to open, then click an item to choose it. Clicking outside cancels.
- **"Old-school playlist buttons" preference** (default off): press, drag to an item, release to choose. Releasing on the button itself chooses item 0.
- Items marked ▸ open a standard menu: "Remove misc" at (64, h−84), the MISC items at (122, item y).
- Losing window focus cancels the pop-up.
- **Right-click** on a button opens the same choices as a standard menu (§6.3).
- **Double-click** on a button: ADD = add file; REM = remove selected; SEL = select all; MISC = misc options menu; LIST = open playlist.

### 6.3 Standard menus

```
ADD:   Add file(s)                     L
       Add folder                      Shift+L
       Add URL                         Ctrl+L
REM:   Remove selected                 Del
       Crop selected                   Ctrl+Del
       Clear playlist                  Ctrl+Shift+Del
       Remove ▸  Remove missing files  Alt+Del
                 Physically remove selected file(s)
SEL:   Select all                      Ctrl+A
       Select none
       Invert selection                Ctrl+I
MISC:  File info ▸  File info...       Alt+3
                    Playlist entry...  Ctrl+E
       Sort ▸       Sort list by title                 Ctrl+Shift+1
                    Sort list by file name             Ctrl+Shift+2
                    Sort list by path and file name    Ctrl+Shift+3
                    ─────
                    Reverse list                       Ctrl+R
                    Randomize list                     Ctrl+Shift+R
       Misc ▸       Generate HTML playlist             Ctrl+Alt+G
                    ─────
                    Rebuild titles on selection        Ctrl+Alt+E
LIST:  Open playlist...                Ctrl+O
       Save playlist...                Ctrl+S
       New playlist (clear)            Ctrl+N
```

### 6.4 List context menu (right-click on a row)

```
Play item(s)                         Enter
─────
Remove item(s)                       Del
Crop files                           Ctrl+Del
─────
Edit metadata for selection...       Shift+E
View file info...                    Alt+3
Playlist entry...                    Ctrl+E
Bookmark item(s)                     Alt+I
─────
Rate items ▸  ★★★★★ / ★★★★ / ★★★ / ★★ / ★ / No rating
─────
Show item(s) in folder               Ctrl+F
```

Right-click first applies the selection rule from §7.1 (Ctrl toggles). Right-click outside the list and buttons opens the main menu. Right-click on the mini visualizer (w−225 .. w−150, h−26 .. h−8) opens the visualization menu.

---

## 7. Mouse behaviour in the list

### 7.1 Selection

- Row under pointer = (y − 22) / row height + first visible row.
- **Mouse-down** (x 12 .. w−20, y 20 .. h−38):
  - Without Ctrl, if the row is not already selected, clear the selection.
  - With Shift (and an anchor): unless Ctrl is held and the row is selected, clear the selection; select the range between the anchor and this row. The anchor does not move.
  - Without Shift: Ctrl+click on a selected row deselects it; otherwise select it. Set the anchor to this row.
- **Mouse-up** without a drag or Shift: without Ctrl, if the row is selected, make it the only selection. (Clicking inside a multi-selection collapses it, unless the user dragged.)
- **Double-click** on a row: play it.
- **Double-click** on the title bar (y ≤ 14, x < w−20): toggle windowshade.

### 7.2 Drag to reorder

- With the button held on a selected row, moving down by one row moves every selected entry down by one position (swap with the neighbour, iterating from the far end); moving up does the reverse.
- The block stops when a selected entry reaches the first or last position.
- The playing entry keeps playing; its index follows its new position.
- Auto-scroll one row per mouse event when the pointer is above y = 10 or below h − 28.

### 7.3 Mouse wheel

- Scroll by the "page scroll lines" setting per notch, doubled when "wheel scrolls double" is on (default on).
- With the middle button held: move the selection up/down.
- In windowshade mode: previous/next track.

### 7.4 Drag and drop from the desktop

- Insert at the row under the drop point ((y − 20) / row height + first visible row). Past the end, or in shade mode: append.
- Each dropped item goes through the "add anything" rule (10 §2.5): folders expand, playlists are read, shortcuts are resolved, URLs are added.
- If the insert point is at or before the playing entry, its index shifts by the number of added entries.
- Playback does not change.

Dropping on the **main window** is different: without Shift it clears the list, adds the items and starts playback from the start (or the first shuffle slot). With Shift it only appends.

Drag **out** of the playlist (to a file manager or another app) as a list of file URIs is recommended.

### 7.5 Cursors

Normal mode: shade button (w−21 .. w−13, 3–12) `PWinBut.cur`; close (w−11 .. w−3) `PClose.cur`; title (0,0)–(w−1,13) `PTBar.cur`; scroll bar (w−15,20)–(w−7,h−38) `PVScroll.cur`; resize (w−20,h−20)–(w−1,h−1) `PSize.cur`; else `PNormal.cur`. Shade mode: `PWinBut`, `PClose`, resize (w−29 .. w−20) `PWSSize.cur`, else `PWSNorm.cur`.

---

## 8. Keyboard (playlist focused)

| Key | Action |
|---|---|
| Up / Down | Move the selection cursor (single selection) and scroll it into view. Shift extends. In windowshade: previous / next track |
| Page Up / Page Down | Scroll by the "page scroll lines" setting |
| Home / End | Scroll to top / bottom (selection unchanged) |
| Space | Select and show the playing entry |
| Enter | Play the first selected entry |
| Delete | Remove selected |
| Ctrl+Delete | Crop |
| Ctrl+Shift+Delete, Ctrl+N | Clear playlist |
| Alt+Delete | Remove missing files |
| Alt+Up / Alt+Down | Move selected entries up / down |
| Ctrl+A | Select all |
| Ctrl+I | Invert selection |
| L, Numpad 0, Ctrl+Insert | Add files |
| Shift+L, Insert | Add folder |
| Ctrl+L, Ctrl+Numpad 0, Shift+Insert | Add URL |
| Ctrl+E | Edit playlist entry (file name/URL) for each selected entry |
| Shift+E | Edit metadata for selection |
| Alt+3 | File info for each selected entry |
| Ctrl+F | Show selected file in the file manager |
| Alt+I | Bookmark selected entries |
| Ctrl+R | Reverse list |
| Ctrl+Shift+R | Randomize list |
| Ctrl+Shift+1 / 2 / 3 | Sort by title / file name / path and file name |
| Ctrl+Alt+E | Rebuild titles of selection |
| Ctrl+Alt+G | Generate HTML playlist |
| Ctrl+Numpad+ / Ctrl+Numpad− / Ctrl+Enter | Font size bigger / smaller / reset |
| Ctrl+F4 | Close the playlist window |
| Z X C V B, Numpad 4 5 6 | Transport (with Shift and Ctrl variants as in the main window) |
| Left / Right, Numpad 7 / 9 | Seek −5 s / +5 s |
| Numpad 1 / 3 | Jump 10 entries back / forward |
| J, Numpad ., F3 | Jump to file |
| R / S | Toggle repeat / shuffle |
| Numpad 8 / 2 | Volume up / down (arrow keys are used for selection here) |
| F5 | Reload skin |
| Alt+S | Skin browser |
| Ctrl+Alt+A | Always on top |

---

## 9. List operations

| Operation | Behaviour |
|---|---|
| Select all / none / invert | As named |
| Remove selected | Delete selected entries |
| Crop | If at least one entry is selected, delete every unselected entry. Otherwise do nothing |
| Clear | Delete all entries and reset the shuffle order. The playing track continues |
| Remove missing files | Delete entries that are local files and cannot be opened for reading. URLs are kept |
| Physically remove selected files | Ask for confirmation, then delete the files (to the system trash when "use trash" is on, default on). Remove an entry only if its file was actually deleted. URLs are skipped |
| Remove duplicates (extension) | Not in the reference player. Recommended addition: remove later entries whose path matches an earlier entry (case-insensitive on case-insensitive file systems) |
| Move up / down | Swap each selected entry with its neighbour. Do nothing if the selection touches the edge |
| Sort by title | Natural ("logical") order of titles: digit runs compare numerically; letters compare case-insensitively using the user's locale; at a position where one string has a digit and the other does not, the digit sorts first |
| Sort by file name | Natural order of the file-name part only |
| Sort by path and file name | Natural order of the directory part; ties broken by file name. Entries with no directory part sort first |
| Reverse | Reverse the order |
| Randomize | Uniform random permutation |
| Rebuild titles | Re-read metadata for selected entries |
| Generate HTML playlist | Write an HTML page listing the entries (numbers, titles, lengths, total time) and open it in the browser. Recommended, not required |
| Edit playlist entry | Dialog per selected entry to edit its location. Cancel stops the loop. Re-read the title after an edit |

- Sort, reverse and randomize keep the playing entry as current (its index follows the move). After the operation, scroll so the playing entry is at the top of the view.
- Sorting need not be stable. A stable sort is acceptable.
- Every change marks the playlist as modified (for saving on exit, 10 §4).

---

## 10. Lazy title reading

Preference "when to read metadata":

| Value | Behaviour |
|---|---|
| On load | Read tags synchronously as entries are added |
| On display | Use a file-name title at first. Resolve visible rows in the background |
| Background (default) | Like "on display", but continue through the whole list: start at the first visible row, go to the end, then wrap to the start |
| Only when played | Read tags only when an entry starts playing |

- Background resolution: resolve one entry about every 100 ms (an implementation may go faster with a worker thread), only while the playlist window is visible and not shaded. Redraw after each resolved entry.
- When an entry starts playing, always re-read its metadata and update its title and length. (A title from a playlist file is replaced by the tag title at this point.)
- The file-name title before reading: for files, the name without directories and extension; for URLs, the whole URL. Then apply the title clean-up option (10 §5.4).
