# 05 — Generic Windows, Video Window, Docking

This document covers the generic skinned window frame (`gen.bmp`, `genex.bmp`), the video window (`video.bmp`), legacy frames (`mb.bmp`, `avs.bmp`), and how all skinned windows dock and snap.

---

## 1. Generic ("embedded") window

The generic window is a skinned frame that hosts other content: a visualization plugin, a media library, a lyrics view, or any plugin panel. Classic skins style it with `gen.bmp` and `genex.bmp`.

### 1.1 Frame sprites (`gen.bmp`)

Title pieces are 25×20. Source y = 0 when active, y = 21 when inactive.

| Piece | Source | Placement |
|---|---|---|
| Left corner | (0,y) | x = 0 |
| Title left end | (26,y) | left of the title text |
| Title middle (behind text) | (52,y) | repeated under the text |
| Title right end | (78,y) | right of the title text |
| Fill | (104,y) | both sides of the title |
| Right corner (contains close button) | (130,y) | x = w − 25 |
| Close button, normal | (144,3) 9×9 when active, (144,24) 9×9 when inactive | (w−11, 3) |
| Close button, pressed | (148,42) 9×9 | (w−11, 3) |
| Left side tile | (127,42) 11×29 | (0, 20 + 29k) |
| Right side tile | (139,42) 8×29 | (w−8, 20 + 29k) |
| Left side, lower piece | (158,42) 11×24 | just above the bottom bar |
| Right side, lower piece | (170,42) 8×24 | just above the bottom bar |
| Bottom left | (0,42) 125×14 | (0, h−14) |
| Bottom right (contains resize grip) | (0,57) 125×14 | (w−125, h−14) |
| Bottom fill | (127,72) 25×14 | tiled between; remainder stretched |
| "Not resizable" marker | (118,72) 7×7 | drawn at (w−125+112, h−14+2) when the hosted content has a fixed size |

Vertical layout: (h − 20 − 38) / 29 whole side tiles, then any remainder stretched, then the 24-pixel lower side pieces, then the 14-pixel bottom bar.

**Client area** for hosted content: (11, 20), size (w − 19) × (h − 34). Fill the client background with black before the content draws.

### 1.2 Title layout

- Title text: the hosted window's title, at most 31 characters, upper-cased.
- Measure the exact text width with the `gen.bmp` font (§1.3). Unknown characters advance 5 pixels as a blank.
- Title block width = exact width rounded up to a multiple of 25, capped at w − 100.
- Lay out: left corner, fill tiles, title left end, (block width / 25) middle tiles, title right end, fill tiles, right corner. The fill count is (w − 100 − block width) / 25, split evenly; odd counts use a 12-pixel slice on the left and a 13-pixel slice on the right, as in the playlist title (04 §2.2).
- Text x = start of the middle tiles + (block width − exact width) / 2 (+1 if not exact). Text y = 4.

### 1.3 `gen.bmp` variable-width font

- Letters A–Z: row y = 88 (active) and y = 96 (inactive), height 7.
- Digits 0–9, `-` (index 10) and `:` (index 11): row y = 72 (active) and y = 80 (inactive). Used only when `pledit.txt` `[Misc] UseGenNums=1` (01 §6.3). Otherwise digits are drawn as blanks.
- **Glyph boundaries are found at skin load time:**
  - Letters: take the colour of pixel (0, 90) as the delimiter colour. Scan row y = 90 from left to right. Skip delimiter-coloured pixels; then count non-delimiter pixels until the next delimiter pixel. That count is the glyph width, and its start is the glyph x. Repeat for 26 letters.
  - Digits: the same with row y = 74 and delimiter pixel (0, 74), for 12 glyphs.
- The inactive glyph has the same x and width as the active glyph.

### 1.4 Behaviour

- Size snaps to 275 + 25k by 116 + 29k unless smooth resizing is on. Minimum 275×116.
- Resize grip: the bottom-right triangle where (w − x) + (h − y) ≤ 30 inside the 20×20 corner.
- Move by dragging the title bar (y < 14) or anywhere with EasyMove. Snapping applies (§6).
- Close button hit: x w−10 .. w−1, y 3–12. Action on mouse-up.
- Ctrl+F4 closes the window.
- Generic windows are never drawn at double size.
- Cursors: title `PTBar.cur`, close `PClose.cur`, resize `PSize.cur`, else `PNormal.cur`.

### 1.5 `genex.bmp` — colours and widgets for hosted content

Hosted content (lists, buttons, edit fields, scroll bars) should take its colours from `genex.bmp` so it matches the skin.

**Colours:** read one pixel per colour at y = 0, x = 48 + 2i. A pixel equal to `#00C6FF`, or equal to the pixel at (111,0), means "not set": use the fallback.

| x | Colour | Fallback |
|---|---|---|
| 48 | Item background (lists, edit fields) | `#000000` |
| 50 | Item text | `#00FF00` |
| 52 | Window background | `#24243C` |
| 54 | Button text | `#393842` |
| 56 | Window text | `#FFFFFF` |
| 58 | Dividers, sunken borders | `#8494A5` |
| 60 | List selection | `#0000C6` |
| 62 | List header background | `#484878` |
| 64 | List header text | `#FFFFFF` |
| 66 | List header frame, top | `#6C6CB4` |
| 68 | List header frame, middle | `#24243C` |
| 70 | List header frame, bottom | `#12121E` |
| 72 | Empty list header background | `#24243C` |
| 74 | Scroll bar foreground | `#24243C` |
| 76 | Scroll bar background | `#24243C` |
| 78 | Scroll bar foreground, inverted | `#798296` |
| 80 | Scroll bar background, inverted | `#4E586E` |
| 82 | Scroll bar dead area | `#24243C` |
| 84 | Selection bar text, active | window text colour |
| 86 | Selection bar background, active | list selection colour |
| 88 | Selection bar text, inactive | window text colour |
| 90 | Selection bar background, inactive | average of list selection and window background |
| 92 | Alternate row background | item background |
| 94 | Alternate row text | item text |

**Widgets:**

- Push button 47×15: normal at (0,0), pressed at (0,15). Draw as a 9-slice: 4-pixel corners and edges are not stretched; the centre is stretched.
- Scroll bar arrow buttons 14×14: up (0,31), down (14,31), left (0,45), right (14,45). Pressed = same y, x + 28.
- Vertical scroll thumb 14 wide at x = 56 (normal) or x = 70 (pressed), from y = 31, 28 tall, in five parts top to bottom: 4 fixed, 5 stretched, 10 fixed (grip), 5 stretched, 4 fixed.
- Horizontal scroll thumb at x = 84, y = 31 (normal) or y = 45 (pressed), with the same 4/5/10/5/4 split horizontally.

---

## 2. Video window (`video.bmp`)

Optional component (see 09 for video formats). Default size 275×232.

| Element | Source | Destination |
|---|---|---|
| Title left corner | (0,y) 25×20 | tiled like the playlist title (04 §2.2); y = 0 active, 21 inactive |
| Title centre | (26,y) 100×20 | |
| Title fill | (127,y) 25×20 | |
| Title right corner | (153,y) 25×20 | |
| Left side | (127,42) 11×29 | (0, 20 + 29k) |
| Right side | (139,42) 8×29 | (w−8, 20 + 29k) |
| Bottom left (with buttons) | (0,42) 125×38 | (0, h−38) |
| Bottom right | (0,81) 125×38 | (w−125, h−38) |
| Bottom fill | (127,81) 25×38 | tiled, remainder stretched |
| Five buttons, normal | (9 + 15i, 51) 15×18 | (9 + 15i, h−29) |
| Five buttons, pressed | (158 + 15i, 42) 15×18 | same |
| Close normal / pressed | (167,3) / (148,42) 9×9 | (w−11, 3) |

- Video area: (11, 20), size (w − 19) × (h − 58).
- Buttons, hit x 9–89, y h−29 .. h−11, index (x − 9) / 15: 0 full screen; 1 zoom 100%; 2 zoom 200%; 3 misc/"TV" (optional, may open video options); 4 video menu.
- Info text in (92, h−27) – (w−25, h−13): 10-pixel font, colours `mbFG` on `mbBG` from `pledit.txt`. Shows the title and video information.
- Resize, move and snap rules are the same as the playlist editor.
- Keys in the video window: Esc leaves full screen; 1 = normal size, 2 = double, 3 or ` = half; Space = pause; Alt+Enter = full screen; Shift+F = flip vertically. Main-window shortcuts also work.
- Video context menu: Full screen; Normal size; Half size; Double size; Vertically flip; Audio track ▸; Video track ▸; Video options...
- Video preferences (recommended): open window when video starts; close window when video ends; stop playback when window is closed; resize window to video; disable screen saver while playing; full screen on start; leave full screen on stop; lock aspect ratio; custom screen aspect ratio; show logo when idle.

---

## 3. Shade and docking of the video window

The video window has no windowshade mode. It docks and snaps like other windows (§6).

---

## 4. Minibrowser (`mb.bmp`) — legacy

Winamp 2.x had a "minibrowser" web panel. The 5.x reference has removed it. `mb.bmp` (234×119) uses the same frame layout as `video.bmp`, with different button glyphs. Implementations do not need a minibrowser. If one is added (for example to show a stream's home page or a lyrics page), frame it with `mb.bmp` using the video-window layout, and use `mbFG`/`mbBG` for its info text.

---

## 5. `avs.bmp` — legacy

Some very old skins include `avs.bmp`, a frame for the Advanced Visualization Studio window. Modern hosts put visualizations in a generic window (§1). Support for `avs.bmp` is optional. If supported, its layout is: top-left (15,0) 50×15; top middle (66,0) 14×15 stretched; top right (81,0) 16×15; left (0,16) 7×172 stretched; right (8,16) 6×172 stretched; bottom-left (15,16) 50×5; bottom middle (66,16) 14×5 stretched; bottom right (81,16) 16×5.

---

## 6. Docking and snapping

### 6.1 Snap rule

- Preference "snap windows" (default on) with a distance in pixels (default 10). Holding **Shift** while dragging inverts the preference for that drag.
- "Close" means |a − b| ≤ snap distance.
- When moving window S near window D:
  1. If S and D overlap horizontally: if S's top is close to D's bottom, stick S below D; else if S's bottom is close to D's top, stick S above D.
  2. If S and D overlap vertically: if S's right is close to D's left, stick S to the left of D; else if S's left is close to D's right, stick S to the right of D.
  3. If S now touches D side by side: align tops if close, else align bottoms if close.
  4. If S now touches D top/bottom: align left edges if close, else right edges if close.
- Snap to screen first: if "keep on screen" is on (default on), snap each edge to the work area of the monitor within the snap distance.
- Then snap against each visible player window in turn: playlist, equalizer, main window, video window, every generic window.
- Snapping applies while dragging any skinned window by its title bar.

### 6.2 Moving the main window moves its docked group

- When the user starts dragging the main window (and snapping is active), find every window "attached" to it.
- Two windows are attached when an edge of one exactly equals the opposite edge of the other and they overlap along that edge (ranges inclusive). Example: A's bottom = B's top and their horizontal ranges overlap.
- The attached set is transitive: repeat until no new windows join. Consider the equalizer, playlist, video window and visible generic windows.
- While dragging, move the main window by the pointer delta, snap it to the screen and to windows that are **not** in the group, and move every window in the group by the same delta. If "keep on screen" is on, also snap each group window to the screen and shift the whole group to match.
- Dragging any other window moves only that window (with snapping).
- Without snapping (or with Shift), the main window moves alone.

### 6.3 Size changes keep docked windows attached

When a window changes size (windowshade toggle, double-size toggle, playlist resize):

- For the bottom edge: every window whose top edge equals the old bottom edge of the resized window (with horizontal overlap) moves to the new bottom edge. Apply recursively, so a stack Main → EQ → PL collapses and expands together.
- For the right edge: the same with left edges and the old right edge.
- Windows that were flush with the bottom or right edge of the screen stay flush after the resize, and windows docked above or left of them follow.
- For these comparisons, the main window's rectangle is its logical size: 275×116 or 275×14, doubled in double size. The EQ window doubles only when double size applies to it. The playlist and generic windows never double.

### 6.4 Default positions

The default layout is a vertical stack: main window at (26,29), equalizer at (26,145), playlist at (26,261). (29 + 116 = 145; 145 + 116 = 261.) All three visible by default. Video window at (26,145), hidden.

### 6.5 Platform notes

- On platforms where applications cannot position their own windows (for example some Wayland compositors), implement the windows as one transparent, input-shaped surface that contains all docked windows, or use a compositor protocol that allows positioning. Docking behaviour must be preserved.
- Windows must appear as one application in the task bar. Optional preference: show in task bar, tray, both, or neither.
