# 02 — Main Window

The main window is 275×116 skin pixels in normal mode and 275×14 in windowshade mode. All coordinates are in 1x skin pixels (see 01 for conventions). Double size is described in §9.

---

## 1. Rendering model

- Keep an off-screen 275×116 back buffer per main window.
- On skin load and on every Stop, copy all of `main.bmp` `(0,0) 275x116` into the back buffer, then redraw the dynamic elements listed below.
- Every element is an opaque rectangular copy from a skin bitmap. No blending, no scaling (except double size, §9).
- Elements are redrawn only when their state changes. A full redraw on every frame is acceptable if the visible result is identical.
- The title bar strip from `titlebar.bmp` always covers the top 14 rows of `main.bmp`.
- When stopped, or when the current item is not seekable, the seek bar thumb is not drawn and the groove from `main.bmp` shows.

---

## 2. `titlebar.bmp`

### 2.1 Title bar strip

The strip is 275×14, taken from source x = 27, and drawn at destination (0,0).

| State | Source y |
|---|---|
| Normal mode, active | 0 |
| Normal mode, inactive | 15 |
| Windowshade mode, active | 29 |
| Windowshade mode, inactive | 42 (the two shade rows overlap by one pixel row; this is intended) |
| Easter-egg title, active (normal mode only) | 57 |
| Easter-egg title, inactive (normal mode only) | 72 |

- "Active" means the window has focus. A preference "dim inactive title bars" (default on) controls this. When the preference is off, always draw the active strip.
- Easter egg: typing the letters `N U L L S O F T` in order (case-insensitive) while the main window has focus toggles the egg title strip. A new implementation may keep this, change the word, or omit it.

### 2.2 Title bar buttons (9×9)

The unpressed look is part of the strip. Draw the sprite only for the pressed state, and restore the strip when released.

| Button | Destination | Source unpressed | Source pressed |
|---|---|---|---|
| Main menu | (6,3) | (0,0) | (0,9) |
| Minimize | (244,3) | (9,0) | (9,9) |
| Close | (264,3) | (18,0) | (18,9) |
| Windowshade toggle, normal mode | (254,3) | (0,18) | (9,18) |
| Windowshade toggle, shade mode | (254,3) | (0,27) | (9,27) |

### 2.3 Clutterbar (O A I D V)

Destination (10,22), size 8×43.

| State | Source |
|---|---|
| Hidden | (312,0) 8×43 |
| Shown, nothing pressed | (304,0) 8×43 |
| O pressed | (304,44) 8×43 |
| A pressed | (312,44) 8×43 |
| I pressed | (320,44) 8×43 |
| D pressed | (328,44) 8×43 |
| V pressed | (336,44) 8×43 |

- Preference "always show clutterbar" (default on). When off, the clutterbar shows only while the mouse is over it.
- Overlays when the clutterbar is shown:
  - When A is not being pressed: draw (11,33) 7×8 from source (313,55) if always-on-top is on, else from (305,11).
  - When D is not being pressed: draw (11,49) 7×6 from source (329,71) if double size is on, else from (305,27).

### 2.4 Windowshade-mode position bar

Only in windowshade mode.

- Background: source (0,36) 17×7 → destination (226,4).
- Thumb 3×7 at destination (226 + p, 4), where p = position × 12 / 256 + 1, clamped to 1..13, and position is 0..256 (fraction of track × 256).
- Thumb sprite: source (17,36) when p < 6, (20,36) when p < 9, else (23,36).

---

## 3. `cbuttons.bmp` — transport buttons

Unpressed row y = 0, pressed row y = 18. Height 18 (except eject).

| Button | Source x | Width | Destination |
|---|---|---|---|
| Previous | 0 | 23 | (16,88) |
| Play | 23 | 23 | (39,88) |
| Pause | 46 | 23 | (62,88) |
| Stop | 69 | 23 | (85,88) |
| Next | 92 | 22 | (108,88) |
| Eject | 114 (y 0 unpressed, y 16 pressed) | 22 (height 16) | (136,89) |

---

## 4. `playpaus.bmp` — status indicator

- Icon 9×9 at destination (26,28). Source x: play 0, pause 9, stop 18, "none" 27; y = 0.
- Then a small strip at destination (24,28):
  - Playing and in sync: 3×9 from source (36,0) (green "working" light). It overwrites column 26 of the icon.
  - Playing but not in sync (buffering, lost sync): 3×9 from source (39,0) (red light), with the play icon.
  - Any other state: 2×9 from source (27,0).
- "In sync" is a signal from the decoder. Decoders report it while they produce audio normally. Network decoders clear it while buffering.

---

## 5. `monoster.bmp`

| Indicator | Source x | Size | Lit source y | Unlit source y | Destination |
|---|---|---|---|---|---|
| Stereo | 0 | 29×12 | 0 | 12 | (239,41) |
| Mono | 29 | 28×12 | 0 | 12 | (212,41) |

- Mono is lit only when the stream has exactly 1 channel. Stereo is lit only when it has exactly 2 channels.
- Any other channel count (for example 6), and stopped, shows both unlit. A new implementation may offer an option to light "stereo" for more than 2 channels; the default must match the reference.

---

## 6. Sliders

### 6.1 `volume.bmp`

- Background: destination (107,57), 68×13. There are 28 frames (index 0–27). Frame n has its source at (0, n × 15). Only the top 13 rows of each 15-row frame are shown.
- Frame index = volume × 27 / 255 (integer). Volume is 0..255. Frame 27 is reached only at 255.
- Thumb 14×11 at destination (107 + x, 58), where x = volume × 51 / 255 (integer), clamped 0..54.
- Thumb source: (15,422) normal, (0,422) pressed.
- If the bitmap is too short to hold the thumb rows (some skins are only 420 pixels high), apply the out-of-bounds rule from 01 §2.1.

### 6.2 `balance.bmp`

- Background: destination (177,57), 38×13. Source (9, n × 15), frame n = |balance| × 27 / 127 (integer), 0..27. Frame 0 is centre; frame 27 is fully left or fully right (symmetric).
- Thumb 14×11 at destination (177 + x, 58), where x = balance × 12 / 127 + 12, clamped 0..24. Balance is −127..127.
- Thumb source: (15,422) normal, (0,422) pressed.
- **If the skin has no `balance.bmp`, read these same rectangles from `volume.bmp`.**

### 6.3 `posbar.bmp` — seek bar

- Bar destination (16,72), 248×10.
- Background source (0,0) 248×10.
- Thumb 29×10. Source (248,0) normal, (278,0) pressed.
- Thumb x offset = position × 219 / 256 (integer), clamped 0..219, where position is 0..256.
- Draw the bar only while playing a seekable item. Update on each display tick (§11).

---

## 7. Time display

### 7.1 Large digits (normal mode)

- `numbers.bmp`: cells 9×13. Digit d at source (d × 9, 0). Cell 10 (x = 90) is blank.
- `nums_ex.bmp`: same layout, plus cell 11 (x = 99) = minus sign.
- Destinations (y = 26):

| Position | Destination x |
|---|---|
| Sign, or hundreds of minutes | 36 |
| Minutes, tens | 48 |
| Minutes, ones | 60 |
| Seconds, tens | 78 |
| Seconds, ones | 90 |

- The colon is part of `main.bmp`.
- Minutes of 100 or more: the hundreds digit appears at x = 36.
- **Remaining-time mode** (only when the item is seekable and its length is known): show length − elapsed. Show `00:00` if the length is not positive.
  - With `nums_ex.bmp`: the minus cell at (36,26).
  - With `numbers.bmp`: a 5×1 line copied from source (20,6) (middle row of the "2" glyph) to destination (40,32).
- **Blank state** (used for the pause blink): blank cells at 36, 48, 60, 78, 90. Quirk: with `nums_ex.bmp` the first blank is drawn at x = 38. With `numbers.bmp` the minus line is cleared by copying 5×1 from source (9,6).
- **Pause blink**: while paused, the time alternates between digits and blank about every 1.2 s (on 1.2 s, off 1.2 s).
- Elapsed time display is the position heard at the speakers (output position), not the decode position.

### 7.2 Windowshade time (small digits)

Uses `text.bmp` digits (row y = 6, 5×6 cells). Destination y = 4.

| Element | Destination x |
|---|---|
| Blank (3×6 from text.bmp (142,0)) | 126 always; also 130 when minutes < 100 |
| Hundreds of minutes | 129 |
| Minutes tens / ones | 134 / 139 |
| Seconds tens / ones | 147 / 152 |

- The colon is part of the shade strip.
- Remaining mode minus: 3×6 from text.bmp (75,6), at x = 126 when minutes ≥ 100, else at x = 130.
- Blank state: blanks at 126, 130, 134, 139, 147, 152.

### 7.3 Bit rate and sample rate

Small digits from `text.bmp` row y = 6.

- kbps: three cells at (111,43), (116,43), (121,43).
- kHz: two cells at (156,43), (161,43).
- Blank cell: text.bmp (100,12).
- kbps formatting:
  - Bit rate ≥ 10000: [hundred-thousands digit or blank][ten-thousands digit][letter "C"] (C means hundreds).
  - 1000 ≤ bit rate < 10000: [thousands digit][hundreds digit][letter "H"].
  - Below 1000: right-aligned with leading blanks.
- kHz: the integer kHz value (for example 44 for 44.1 kHz) as [tens or blank][ones].
- Update when the decoder reports a new bit rate (VBR files update continuously unless the decoder reports an average). Cleared on stop.

---

## 8. Song title marquee

Text area x 111–264, height about 12. Width 154 pixels.

### 8.1 Font modes

Preference "use skinned bitmap font for the title" (**default off**).

- **Bitmap font mode**: 5×6 glyphs from `text.bmp` (01 §6.1), glyph row at y = 27. The first glyph may be partly scrolled off. A glyph that starts at x ≥ 260 is clipped at x = 265. Fill the remaining area up to x = 265 with column (4,0) 1×6 of text.bmp.
- **System font mode** (default): font face from the user's playlist font setting if set, else `pledit.txt` `Font`, else the default UI font. Pixel height 10. No anti-aliasing at 1x (a preference may enable anti-aliasing in double size). Colours from `text.bmp` (01 §6.2). Fill the box (111,24)–(265,36) with the background colour. Draw the text left-aligned, single line, at y = 23.

The system font mode is the default because it supports all Unicode text. The bitmap font only covers the characters in 01 §6.1.

### 8.2 Displayed string

| Situation | String |
|---|---|
| Normal, length known | `N. Title (M:SS)` when "show playlist position in title" is on (default on), else `Title (M:SS)` |
| Current item with unknown length (stream) | `N. Title` (the number is always shown) |
| Nothing loaded | Player name and version |
| Transient message | The message, verbatim (see §8.4) |

N is the 1-based playlist position. Minutes in the length are not padded and are not limited to 2 digits.

### 8.3 Scrolling

- If the string is wider than 146 pixels, the scroll buffer is `string + "  ***  " + string` and the position wraps modulo (width of string + width of `"  ***  "`). Otherwise the position is 0.
- In bitmap mode, width = character count × 5.
- Auto-scroll: every 200 ms, advance 5 pixels. Run only when the main window is visible, not minimized, not in windowshade, the "auto-scroll title" option is on (default on), and no drag is in progress.
- After any manual drag, and at the start of a new title, auto-scroll pauses for 2 s.
- Manual scroll: mouse-down in the title area (x 117–266, y 24–35) starts a drag. Each mouse move scrolls the text by the negative of the horizontal mouse delta.
- Turning auto-scroll off resets the position to 0.

### 8.4 Transient messages

Some actions show a message in the marquee for about 1 s, then the title returns. Messages:

| Action | Message |
|---|---|
| Volume change | `Volume: N%` (N = volume × 100 / 255) |
| Balance change | `Balance: N% Left`, `Balance: N% Right`, or `Balance: Center` (N = |balance| × 100 / 127) |
| Seek drag | `Seek to: M:SS/M:SS (P%)` |
| EQ slider drag | `EQ: <band label>: <+/->X.X dB` or `Preamp: ...` (see 03) |
| Clutterbar hover/press | `Options Menu`, `Enable/Disable Always-On-Top`, `File Info Box`, `Enable/Disable Doublesize Mode`, `Visualization Menu` |
| Toggle shuffle | `Shuffle: On` / `Shuffle: Off` |
| Toggle repeat | `Repeat: Off` / `Repeat: Playlist` / `Repeat: Track` (see 07 §3) |
| Toggle manual advance | `Manual Advance: On` / `Off` |

After a shuffle/repeat toggle, hold the message for about 5 auto-scroll steps instead of 1 s. While a drag message is visible (volume, balance, seek, EQ), keep it until the mouse is released.

Network decoders may set the title to status text such as `[Connecting] url`, `[Buffer: 37%] Title`. This is title text from the decoder, not a core message.

---

## 9. Double size

- The back buffer stays 275×116. The window shows it scaled 2× with nearest-neighbour pixel doubling (550×232; shade 550×28).
- Mouse coordinates are halved before hit testing.
- Region polygons, the visualizer rectangle and popup-menu anchor points are doubled.
- The system-font title is rendered at 1x and doubled (optionally anti-aliased, see §8.1). A new implementation may render it at 2x resolution if the result fits the same box.
- The visualizer is rendered at native 2x resolution, not doubled (see 06).
- Toggle: Ctrl+D, clutterbar D, Options menu.
- Preference: "apply double size to the equalizer too" (default on). The playlist editor and generic windows never double.
- Implementations should also support fractional scale for high-DPI displays (for example 1.5x, 3x) as an extension, with integer scale as the default.

---

## 10. Windowshade mode

- Window becomes 275×14. Toggle with the shade button, double-click on the title bar, or the menu.
- The whole shade bar comes from the `titlebar.bmp` shade row: menu button, mini visualizer well, time well, mini transport, eject, position-bar groove, minimize/unshade/close.
- Drawn on top: visualizer (79,5) 38×5, time (§7.2), position bar (§2.4), pressed title buttons.
- The mini transport buttons have no pressed sprites.
- The song title is not shown.
- Docked windows below the main window move up or down to stay attached (05 §6.3).

---

## 11. Timers

| Period | Purpose |
|---|---|
| 100 ms while playing | Update time display, seek bar, bit rate, sync light, pending title changes |
| 200 ms | Title auto-scroll |
| 1000 ms (one shot) | Restore title after a transient message |
| ~155 ms (one shot) | Menu-button press delay before the main menu opens |
| ~250 ms after skin/size change | Apply window shape (region) |

---

## 12. Mouse interaction (normal mode)

### 12.1 Dispatch order

1. While a window drag is in progress, nothing else reacts.
2. Drag controls: seek bar, volume, balance, song title. The first that claims the mouse-down owns the drag.
3. Buttons: transport, eject, shuffle, repeat, EQ, PL, about, clutterbar, time display, visualizer.
4. Title bar buttons.
5. Window move.

Mouse-down captures the mouse until mouse-up.

### 12.2 Hit areas and actions

| Control | Hit box | Action |
|---|---|---|
| Transport | x 15–130, y 87–106. Previous x < 38; Play 38–59; Pause 60–83; Stop 84–106; Next ≥ 107 | Pressed sprite shows on mouse-down and follows the pointer between buttons while held. Action on **mouse-up** over a button. Shift and Ctrl variants: see 07 §2 |
| Eject | 136–158 × 89–105 | Mouse-up: open files. Ctrl: open URL. Shift: open folder |
| Shuffle | 165–209 × 90–102 | Mouse-up: toggle |
| Repeat | 210–239 × 90–102 | Mouse-up: toggle |
| EQ toggle | 219–242 × 58–70 | Mouse-up: show/hide equalizer |
| PL toggle | 242–265 × 58–70 (x 242 belongs to EQ) | Mouse-up: show/hide playlist |
| About | 253–265 × 91–105 | Mouse-up: About dialog |
| Time display | 36–96 × 26–39 | **Mouse-down**: toggle elapsed/remaining |
| Visualizer | 27–99 × 40–61 | **Mouse-down, only while playing**: cycle off → spectrum → oscilloscope → off |
| Volume | 107–175 × 58–66 | Drag (§12.4) |
| Balance | 177–215 × 58–66 | Drag (§12.4) |
| Seek bar | 18–263 × 73–81 | Drag (§12.3) |
| Song title | 117–266 × 24–35 | Drag to scroll |
| Clutterbar | O 11–19 × 24–31; A 11–19 × 32–39; I 11–19 × 40–47; D 11–19 × 48–55; V 11–19 × 56–62 | Mouse-up: O = Options menu at (14,25); A = toggle always on top; I = file info for current item; D = toggle double size; V = Visualization menu at (14,56). Show the hint text (§8.4) while held |
| Menu button | 5–17 × 3–13 | Mouse-down: pressed sprite, then after ~155 ms the main menu opens at (6,13). Moving off cancels |
| Minimize | 244–253 × 3–12 | Mouse-up: minimize. Shift: minimize to tray |
| Shade toggle | 254–262 × 3–12 | Mouse-up: toggle windowshade |
| Close | 264–272 × 3–12 | Mouse-up: exit. Shift: hide main window only |
| Title bar | y < 14, or anywhere not claimed when "EasyMove" is on (default on) | Move window with docking (05 §6) |

Title buttons show the pressed sprite while the button is held over them. Leaving the box restores the sprite. The action fires only on mouse-up inside the box.

### 12.3 Seeking

- Active only while playing a seekable item.
- Mouse-down in the bar: if the pointer is on the thumb, keep the grab offset; otherwise centre the thumb on the pointer (offset 14).
- While dragging: a = pointer x − offset − 17, clamped 0..218. Target time = length × a / 218. Draw the thumb pressed at the target. Show the `Seek to:` message.
- The seek happens on **mouse-up**. While Shift is held, seek continuously during the drag.
- If the seek fails, stop playback.
- (The drawing uses 219 pixels of travel and the hit math uses 218. Reproduce this so the thumb lines up exactly.)

### 12.4 Volume and balance drag

- **Volume**: mouse-down in 107–175 × 58–66. If the pointer is on the thumb (thumb x to thumb x + 14), keep the grab offset; otherwise use offset 7. While dragging: volume = (pointer x − offset − 107) × 255 / 51, clamped 0..255. Apply immediately. Show `Volume: N%`.
- **Balance**: mouse-down in 177–215 × 58–66. Thumb x = 177 + 12 + balance × 12 / 127. Same grab rule. While dragging: v = pointer x − offset − 177, clamped 0..24, then v = v − 12. Unless Shift is held, |v| < 2 becomes 0. Balance = v × 127 / 12. Apply immediately. Show the balance message.
- **On release of balance**: if |balance| < 27, set it to 0 (centre snap). This happens even with Shift.

### 12.5 Mouse wheel

- Unless "mouse wheel does not change volume" is set: each wheel notch changes volume by 8 (two steps of 4), clamped 0..255. Show `Volume: N%`. Accumulate partial notches from high-resolution wheels; reset the remainder when the direction changes.
- With the middle button held: each notch seeks ±5 s.

### 12.6 Double-click

| Where | Action |
|---|---|
| Menu button (6–16 × 4–12) | Exit the player |
| Song title (118–265 × 25–34) | File info for current item |
| Visualizer (24–99 × 43–58) | Start/stop the external visualization plugin (if one is configured). Also undo the mode change from the first click |
| Title bar (y < 15, x < 244) | Toggle windowshade |
| Elsewhere | Treat as a second mouse-down |

### 12.7 Right-click menus

Ignored while the left button is held. The menu opens at the pointer. Test regions in this order:

| Region | Menu |
|---|---|
| Song title (106–265 × 25–34) | View file info; Jump to file; Jump to time; Auto-scroll song title (check); Rating ▸ (5 stars … 1 star, No rating) |
| Time (36–95 × 26–38) | Time elapsed / Time remaining (radio) |
| Visualizer (27–98 × 40–60) | Visualization menu (06 §6) |
| Previous (16–36 × 88–105) | Previous / Start of list (Ctrl+click) / Rewind 5 s (Shift+click) |
| Play (37–61) | Play/restart / Open URL (Ctrl) / Open file (Shift) |
| Pause (62–88) | Pause/unpause |
| Stop (89–106) | Stop / Stop with fade-out (Shift) / Stop after current (Ctrl) |
| Next (107–129) | Next / Fast-forward 5 s (Shift) / End of list (Ctrl) |
| Eject (136–157 × 89–104) | Open file / Open folder (Shift) / Open URL (Ctrl), plus audio-CD drives |
| Seek bar (18–262 × 73–80), only while playing a seekable item | Jump to time / Rewind 5 s / Forward 5 s |
| Shuffle (164–210 × 89–103) | Shuffle (check) |
| Repeat (211–238 × 89–103) | Repeat (check); Manual playlist advance (check) |
| EQ (219–241 × 58–69) | Graphical equalizer (check) |
| PL (242–264 × 58–69) | Playlist editor (check) |
| Anywhere else | Main menu (11 §3) |

The keyboard context-menu key opens the main menu at the menu-button position.

### 12.8 Tooltips

Shown when the "tooltips" preference is on (default on).

| Rectangle (normal mode) | Text |
|---|---|
| 254,3–262,12 | Toggle Windowshade Mode |
| 244,3–253,12 | Minimize |
| 264,3–272,12 | Close |
| 5,3–17,13 | Main Menu |
| 219,58–242,70 | Toggle Graphical Equalizer |
| 242,58–265,70 | Toggle Playlist Editor |
| 11,24–19,31 | Options Menu |
| 11,32–19,39 | Toggle Always-On-Top |
| 11,40–19,47 | File Info Box |
| 11,48–19,55 | Toggle Doublesize Mode |
| 11,56–19,62 | Visualization Menu |
| 107,58–175,66 | Volume Bar |
| 177,58–215,66 | Panning Bar |
| 18,73–263,81 | Seeking Bar |
| 165,90–209,102 | Toggle Shuffle |
| 210,90–239,102 | Toggle Repeat |
| 253,91–265,105 | About |
| 136,89–158,105 | Open File(s) |
| 15,87–38,106 | Previous Track |
| 39,87–60,106 | Play |
| 61,87–84,106 | Pause |
| 85,87–107,106 | Stop |
| 108,87–130,106 | Next Track |
| 36,26–96,39 | Time Display (click to toggle elapsed/remaining) |
| 105,24–266,35 | Song Title (right-click for options) |

Windowshade mode tooltips: 226,4–243,11 Seeking Bar; 216,2–224,11 Open File(s); 168,2–176,11 Previous; 177,2–186,11 Play; 187,2–195,11 Pause; 196,2–204,11 Stop; 205,2–215,11 Next; 125,4–157,10 Time Display.

---

## 13. Mouse interaction (windowshade mode)

| Control | Hit box | Action |
|---|---|---|
| Mini transport + eject | x 168–224 × y 2–11. Previous < 176; Play 176–185; Pause 186–194; Stop 195–203; Next 204–214; Eject 215–224 | Fires on **mouse-down** (no pressed sprite). Shift/Ctrl variants as in normal mode |
| Time | 129–157 × 3–9 | Mouse-down: toggle elapsed/remaining |
| Visualizer | 78–116 × 4–11 | Mouse-down: cycle mode (also while stopped) |
| Seek | 228–242 × 3–12 | Drag; a = (x − 228) × 218 / 13 |
| Title buttons | same as normal mode | same |
| Everything else | — | Move window |

Double-clicks on the time area or the mini transport count as an extra mouse-down. Double-click on the visualizer toggles between off and spectrum and starts/stops the external visualization plugin.

Right-click regions in shade mode: time 129–156 × 3–8; visualizer 78–115 × 4–10; Previous 167–175 × 3–11; Play 176–185; Pause 186–195; Stop 196–205; Next 206–215; Eject 215–224 × 3–11. Menus are the same as in normal mode.

---

## 14. Other behaviours

- **Always on top**: Ctrl+A, clutterbar A, or the menu. Applies to all player windows. Optional preference "drop always-on-top while a full-screen application is focused" (default on).
- **Keep on screen** (default on): when a window is more than 10 pixels off-screen after a size change or a display change, move it back.
- **Window caption** (task bar / window list): `N. Title - <player name>` (the number is controlled by a preference, default on). An option scrolls the caption text in the task bar.
- **Start-up drawing**: draw shuffle, EQ/PL, repeat, eject, transport, volume, balance, title (player name and version), stop icon, clutterbar, mono/stereo off, active title bar.
- **Stop drawing**: copy `main.bmp` again, then mono/stereo off, clutterbar, stop icon, shuffle, eject, EQ/PL, repeat, volume, balance, transport, title of the current playlist entry. Time, bit rate, sample rate and seek bar are left as `main.bmp` shows them.
