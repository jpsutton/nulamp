# 03 — Equalizer

The equalizer (EQ) window is 275×116 in normal mode and 275×14 in windowshade mode. This document covers the window, the value model, the audio filter, presets and the preset file formats.

---

## 1. Window rendering

### 1.1 Bitmaps

- Use `eqmain.bmp` for the window. If the user selected ISO band labels (§4.1) and the skin has `eqmain_iso.bmp`, use that instead. If neither is in the skin, use the default skin's bitmap.
- Use `eq_ex.bmp` for windowshade mode and for the pressed shade button.
- The window background is `eqmain.bmp` `(0,0) 275x116`. All dynamic sprites come from the lower part of the same bitmap.
- **Shade-button quirk:** if the skin has `eqmain.bmp` but no `eq_ex.bmp`, the pressed state of the shade button is not drawn in normal mode. The button still works. (Old skins pre-date `eq_ex.bmp` and would show the default skin's sprite otherwise.)

### 1.2 `eqmain.bmp` sprites

| Element | Source | Destination |
|---|---|---|
| Background | (0,0) 275×116 | (0,0) |
| Title bar, active | (0,134) 275×14 | (0,0) |
| Title bar, inactive | (0,149) 275×14 | (0,0) |
| Close button, normal | (0,116) 9×9 | (264,3) |
| Close button, pressed | (0,125) 9×9 | (264,3) |
| Shade button, normal | (254,137) 9×9 (same pixels as in the active title bar) | (254,3) |
| Shade button, pressed | `eq_ex.bmp` (1,38) 9×9 | (254,3) |
| ON button: off / on / off+pressed / on+pressed | (10,119) / (69,119) / (128,119) / (187,119), 25×12 | (14,18) |
| AUTO button: off / on / off+pressed / on+pressed | (35,119) / (94,119) / (153,119) / (212,119), 33×12 | (39,18) |
| PRESETS button, normal / pressed | (224,164) / (224,176), 44×12 | (217,18) |
| Slider thumb, normal / pressed | (0,164) / (0,176), 11×11 | see §1.3 |
| Slider background frames 0–13 | (13 + n × 15, 164) 14×63 | slider x, y = 38 |
| Slider background frames 14–27 | (13 + (n − 14) × 15, 229) 14×63 | slider x, y = 38 |
| Graph background | (0,294) 113×19 | (86,17) |
| Preamp line in graph | (0,314) 113×1 | (86, y), see §1.4 |
| Graph curve colours | column (115, 294 + row) 1×1, row 0–18 | per curve pixel |

The ON and AUTO source x values follow a pattern: base x + 59 when on, + 118 when pressed.

### 1.3 Sliders

- 11 sliders. Slider 0 is the preamp at x = 21. Bands 1–10 are at x = 78 + (i − 1) × 18, that is 78, 96, 114, 132, 150, 168, 186, 204, 222, 240.
- Value range 0–63 (encoding in §3).
- Background frame n = 27 − (value × 28 / 64) (integer). Value 0 (maximum boost) shows frame 27; value 63 (maximum cut) shows frame 0. Frames 0–13 come from row y = 164, frames 14–27 from row y = 229.
- Thumb 11×11 at (x + 1, 38 + 63 − 12 − ((63 − value) × 52 / 64)), that is y = 89 − ((63 − value) × 52 / 64). Value 0 puts the thumb at the top (y = 38); value 63 at the bottom (y = 89).
- Hit box per slider: x .. x + 12, y 39–98.
- **Drag:**
  - Let p = 51 − ((63 − value) × 52 / 64). On mouse-down, if the pointer is on the thumb (pointer y − 39 is in p − 1 .. p + 10), remember the grab offset (pointer y − 39 − p). Otherwise use offset 5 (the thumb centres on the pointer).
  - New value = ((pointer y − offset − 39) × 64) / 52, clamped 0..63.
  - Centre detent: unless Shift is held, values 30–32 snap to 31 (0 dB).
- **Paint across bands:** while dragging a band slider (not preamp), if the pointer moves more than 3 pixels sideways out of the slider but stays in the band area (x 78–258, y 39–98), release that slider. The next mouse move with the button held captures the slider under the pointer. Result: sweeping the mouse sideways with the button held draws a curve. There is no other multi-band coupling.
- While dragging, the main-window marquee shows `EQ: <label>: <sign><X.X> dB` (or `Preamp: ...`). The dB value is −12 × (value − 31.5) / 31.5, shown as 0.0 if its magnitude is ≤ 0.32, with one decimal.
- Band labels for the marquee:
  - Winamp style: 70, 180, 320, 600 Hz; 1, 3, 6, 12, 14, 16 kHz.
  - ISO style: 31.5, 63, 125, 250, 500 Hz; 1, 2, 4, 8, 16 kHz.
- Every change updates the audio filter immediately and notifies remote-control clients.

### 1.4 EQ graph

The graph shows the band curve in a 113×19 box at destination (86,17).

1. Copy the graph background.
2. **Preamp line:** copy (0,314) 113×1 to y = 35 − (preamp value × 19 / 64) (integer, using real division then truncating). **Note the direction:** preamp value 0 (+12 dB) puts the line at the bottom row, value 63 at the top row. This is the opposite direction to the band curve. A faithful implementation reproduces this.
3. **Curve:**
   - Build 12 keys: key[1..10] = band value × 19 / 64 (real numbers). key[0] = key[1]; key[11] = key[10].
   - Interpolate with a **Kochanek–Bartels (TCB) spline**: tension 0.1, continuity 0, bias 0, key spacing 1. Use the standard TCB tangents with neighbouring keys wrapping modulo 12.
   - For x = 0..108, evaluate at parameter t = 1 + x / 12. y = integer part, clamped 0..18.
   - Plot column = 86 + 2 + x. If y equals the previous column's y (or this is the first column), plot one pixel. Otherwise plot a vertical run between the previous y and this y, inclusive.
   - Each pixel's colour = `eqmain.bmp` pixel at (115, 294 + row), where row is that pixel's own row 0–18.

Default skin curve colours, top to bottom: red at +12 dB through orange and yellow to green at −12 dB. The new default skin may choose its own.

### 1.5 Window controls

| Control | Hit box | Action |
|---|---|---|
| Title bar | y < 14, or anywhere unclaimed with EasyMove | Move (with snapping, 05 §6) |
| Double-click title bar | y ≤ 14, x < 252 | Toggle EQ windowshade |
| Shade / close | x 253–273 × y 3–12; x < 264 is shade, else close | Action on mouse-up inside. Close hides the EQ window |
| ON / AUTO | x 14–86 × y 18–30; x ≥ 39 is AUTO | Pressed sprite while held. Toggle on mouse-up |
| PRESETS | 217–261 × 18–30 | Pressed while held. On release, open the Presets menu at (218,19) |
| "+12 dB" label | x 42–67 × y 33–42 | Mouse-up: set all 10 bands to value 0 (+12 dB) |
| "0 dB" label | x 42–67 × y 65–74 | Mouse-up: set all 10 bands to 31 (flat) |
| "−12 dB" label | x 42–67 × y 92–101 | Mouse-up: set all 10 bands to 63 (−12 dB) |
| Right-click on ON | 14–39 × 18–30 | Menu: "EQ enabled" (check) |
| Right-click on AUTO | 39–72 × 18–30 | Menu: "EQ auto-loading enabled" (check) |
| Right-click elsewhere | — | Main menu |
| Mouse wheel | — | Volume (same as main window) |

The quick-set labels do not change the preamp.

### 1.6 Windowshade mode (`eq_ex.bmp`)

- Window 275×14.
- Title bar: `eq_ex.bmp` (0,0) 275×14 active, (0,15) 275×14 inactive.
- **Volume mini slider:** thumb 3×7 at destination (r − 2, 4), r = 63 + 94 × volume / 255. Sprite index k = volume × 3 / 256 (0, 1 or 2). Source (1 + 3k, 30).
- **Balance mini slider:** r = 166 + 39 × (balance + 128) / 255. k = (balance + 128) × 3 / 256. Source (11 + 3k, 30). Destination (r − 2, 4).
- Unshade button: normal is part of the title strip at (254,3); pressed `eq_ex.bmp` (1,47) 9×9.
- Close button: normal `eq_ex.bmp` (11,38); pressed (11,47); 9×9 at (264,3).
- Volume drag: hit 61–162 × 3–11. t = clamp(pointer x − 61, 0, 96). Volume = t × 255 / 96.
- Balance drag: hit 163–206 × 3–11. t = clamp(pointer x − 164, 0, 42). Balance = t × 255 / 42 − 127, maximum 127. Unless Shift is held, |balance| < 9 snaps to 0.
- Both update the main-window sliders and show the marquee message.

### 1.7 Keyboard (EQ window focused)

| Key | Action |
|---|---|
| 1 2 3 4 5 6 7 8 9 0 | Raise band 1–10 by one step (value − 2) |
| Q W E R T Y U I O P | Lower band 1–10 by one step (value + 2) |
| ` (backtick) | Raise preamp |
| Tab | Lower preamp |
| N | Toggle EQ on/off |
| A | Toggle auto-load |
| S | Open Presets menu |
| Ctrl+Alt+S | Load preset dialog |
| Left / Right | Balance −4 / +4; snaps to 0 when crossing within 20 of centre |
| Up / Down | Volume |
| Ctrl+F4 | Close EQ |
| Z X C V B (with Shift/Ctrl variants), J, Ctrl+A, Ctrl+E | Same as main window (11 §2) |

Values clamp to 0..63. Use physical key positions for the band rows so the layout works on non-QWERTY keyboards.

---

## 2. Double size

When global double size is on and "apply double size to the equalizer" is on (default on), the EQ window is drawn at 2× with pixel doubling, like the main window. Mouse coordinates are halved.

---

## 3. Value encoding

- Band values and the preamp value are integers 0..63.
- **0 = +12 dB (top), 31 = 0 dB, 63 = −12 dB (bottom).** The scale is inverted relative to dB.
- Default: all bands 31, preamp 31, EQ off, auto-load off.

### 3.1 Band gain in dB

Let v = value − 31, clamped to −31..32.

- v > 0: dB = −12 × v / 32.
- v < 0: dB = −12 × v / 31.
- v = 0: dB = 0.

So value 0 → +12.0 dB and value 63 → −12.0 dB. The step size differs slightly above and below the centre.

### 3.2 Preamp gain

The preamp is a linear gain multiplier from a fixed 64-entry table. Endpoints: value 0 → ×4.0 (+12.04 dB), value 31 → ×1.0, value 63 → ×0.25 (−12.04 dB). The table is not linear in dB. Reference values (gain multiplier by index):

| Index | Gain | Index | Gain | Index | Gain | Index | Gain |
|---|---|---|---|---|---|---|---|
| 0 | 4.000000 | 16 | 1.696653 | 32 | 0.983296 | 48 | 0.547435 |
| 1 | 3.610166 | 17 | 1.636363 | 33 | 0.950604 | 49 | 0.525008 |
| 2 | 3.320019 | 18 | 1.579094 | 34 | 0.918821 | 50 | 0.503013 |
| 3 | 3.088821 | 19 | 1.524558 | 35 | 0.887898 | 51 | 0.481433 |
| 4 | 2.896617 | 20 | 1.472507 | 36 | 0.857789 | 52 | 0.460253 |
| 5 | 2.732131 | 21 | 1.422724 | 37 | 0.828454 | 53 | 0.439458 |
| 6 | 2.588368 | 22 | 1.375019 | 38 | 0.799853 | 54 | 0.419035 |
| 7 | 2.460685 | 23 | 1.329225 | 39 | 0.771950 | 55 | 0.398970 |
| 8 | 2.345845 | 24 | 1.285197 | 40 | 0.744712 | 56 | 0.379252 |
| 9 | 2.241498 | 25 | 1.242801 | 41 | 0.718108 | 57 | 0.359868 |
| 10 | 2.145887 | 26 | 1.201923 | 42 | 0.692110 | 58 | 0.340807 |
| 11 | 2.057660 | 27 | 1.162456 | 43 | 0.666689 | 59 | 0.322060 |
| 12 | 1.975760 | 28 | 1.124306 | 44 | 0.641822 | 60 | 0.303614 |
| 13 | 1.899338 | 29 | 1.087389 | 45 | 0.617485 | 61 | 0.285462 |
| 14 | 1.827707 | 30 | 1.051628 | 46 | 0.593655 | 62 | 0.267593 |
| 15 | 1.760303 | 31 | 1.000000 | 47 | 0.570311 | 63 | 0.250000 |

An implementation may instead use the band dB formula (§3.1) for the preamp. The audible difference is under 0.5 dB. The table is the reference behaviour.

---

## 4. Audio filter

### 4.1 Band centre frequencies

| Mode | Centres (Hz) | Labels drawn in the skin |
|---|---|---|
| Winamp (default) | 70, 180, 320, 600, 1000, 3000, 6000, 12000, 14000, 16000 | 60, 170, 310, 600, 1K, 3K, 6K, 12K, 14K, 16K (in `eqmain.bmp`) |
| ISO | 31.5 (or 31), 63 (or 62), 125, 250, 500, 1000, 2000, 4000, 8000, 16000 | `eqmain_iso.bmp` |

The mode is a preference. Changing it takes effect immediately and switches the skin bitmap.

### 4.2 Default filter design

Per channel, process 10 sections in series, in floating point. Each section adds a band-pass component to the dry signal:

- Base Q = 1.41.
- For each band, compute two band-pass designs (constant 0 dB peak gain band-pass from the standard audio-EQ cookbook formulas): a **boost** design with Q × 2 (2.82), and a **cut** design with Q × 0.5 (0.705).
- Linear band gain g = 10^(dB / 20) − 1.
- Section output = input + g × bandpass(input), where the boost design is used when g > 0 and the cut design when g < 0. At the centre frequency the gain is 10^(dB / 20).
- Clamp the sample rate used for design to 4000..384000 Hz. Clamp band frequency to ≥ 20 Hz. Disable a band whose centre is ≥ 0.499 × sample rate.
- Rebuild filter coefficients when the sample rate changes. Update gains on every EQ change without resetting filter state (no clicks).
- Add a tiny DC offset or flush denormals to avoid CPU slow-down on silence.

### 4.3 Alternative design (optional)

A "constant-Q" engine may be offered: each band is a peaking filter from the analog prototype H(s) = (s² + (G/Q)s + 1) / (s² + (1/Q)s + 1), Q = 1.41, G = 10^(dB/20), converted with the bilinear transform with frequency pre-warping. Bypass a band when |G − 1| < 0.02. Drop bands with 2 × centre > sample rate. The engine choice may require restarting playback.

### 4.4 Limiter

Preference "use limiter" (default on). After the bands:

- Track a peak level: detect = max(|x|, detect). Then detect is multiplied by a decay factor each sample, decay = 0.001^(1 / (sample rate × 0.7)) (0.7 s release).
- If detect > 0.930 (about −0.6 dB full scale), scale the sample by 0.930 / detect.

### 4.5 Placement

- Preamp gain (and ReplayGain, see 07 §5) are applied as one combined gain before the bands.
- The filter is bypassed completely when EQ is off, or when the preamp and all bands are 31.
- See 07 §4 for the full processing chain order.

---

## 5. Presets

### 5.1 Libraries

- **Preset library** (file name `winamp.q1` for compatibility; store in the config directory): named presets.
- **Auto-load library** (`winamp.q2`): presets named after media file names.
- **EQF file**: a single preset for exchange between users.
- All three use the same binary format (§6).

### 5.2 First-run presets

If the preset library does not exist, create it with these entries. All have preamp 31. Band values are in the 0..63 encoding, low band first.

| Name | Bands |
|---|---|
| Classical | 31 31 31 31 31 31 44 44 44 48 |
| Club | 31 31 26 22 22 22 26 31 31 31 |
| Dance | 16 20 28 32 32 42 44 44 32 32 |
| Flat | 31 31 31 31 31 31 31 31 31 31 |
| Laptop speakers/headphones | 24 14 23 38 36 29 24 16 11 8 |
| Large hall | 15 15 22 22 31 40 40 40 31 31 |
| Party | 20 20 31 31 31 31 31 31 20 20 |
| Pop | 35 24 20 19 23 34 36 36 35 35 |
| Reggae | 31 31 33 42 31 21 21 31 31 31 |
| Rock | 19 24 41 45 38 25 17 14 14 14 |
| Soft | 24 29 34 36 34 25 18 16 14 12 |
| Ska | 36 40 39 33 25 22 17 16 14 16 |
| Full Bass | 16 16 16 22 29 39 46 49 50 50 |
| Soft Rock | 25 25 28 33 39 41 38 33 27 17 |
| Full Treble | 48 48 48 39 27 14 6 6 6 4 |
| Full Bass & Treble | 20 22 31 44 40 29 18 14 12 12 |
| Live | 40 31 25 23 22 22 25 27 27 28 |
| Techno | 19 22 31 41 40 31 19 16 16 17 |

A preset named `Default` is not created at first run. It exists only after the user saves one.

### 5.3 Presets menu

```
Load ▸   Preset...            (list of library presets)
         Auto-load preset...  (list of auto-load presets; current file name preselected)
         Default
         ─────
         From EQF...
Save ▸   Preset...            (name box + list)
         Auto-load preset...  (prefilled with the current file name)
         Default
         ─────
         To EQF...
Delete ▸ Preset...
         Auto-load preset...
```

- **Load preset dialog:** selecting a list item applies it live (preview). Cancel restores the previous values. Double-click means OK. The initial selection is the last loaded preset.
- **Save preset dialog:** OK is disabled while the name is empty. Saving with an existing name overwrites it.
- Loading a preset changes bands and preamp only. It does not change the EQ on/off state.
- Name matching is case-insensitive.

### 5.4 Auto-load

When auto-load is on, every time a track starts:

1. Look up an entry in the auto-load library whose name equals the file name of the track (without directories, with extension).
2. If none, load `Default` from the preset library.
3. If neither exists, keep the current settings.

---

## 6. Preset file format (EQF / q1 / q2)

Binary, no byte-order issues (all single bytes).

| Offset | Size | Content |
|---|---|---|
| 0 | 31 | Signature: the 27 ASCII characters `Winamp EQ library file v1.1`, then bytes `1A 21 2D 2D` (that is, 0x1A followed by `!--`). No NUL terminator. |
| 31 | 268 × N | N entries, back to back. No count field. |

Each 268-byte entry:

| Entry offset | Size | Content |
|---|---|---|
| 0 | 257 | Name, 8-bit text (Windows-1252 for compatibility; implementations may read UTF-8 if valid), NUL-terminated, padded with NUL |
| 257 | 10 | Band values, low to high, each 0..63 |
| 267 | 1 | Preamp value, 0..63 |

Rules:

- **Read:** the signature must match exactly, else the file is rejected. Read entries until fewer than 268 bytes remain (a partial trailing entry is ignored).
- **Write:** if the file does not exist or its signature does not match, create or truncate it and write the signature. If an entry with the same name (case-insensitive) exists, overwrite it in place; otherwise append.
- **Delete:** rewrite the file without the matching entry (write to a temporary file, then rename).
- **EQF export:** write a one-entry file. The reference player names the entry `Entry1`.
- **EQF import:** the reference player loads only an entry named `Entry1`. A new implementation should load `Entry1` if present, else the first entry, so that EQF files from other tools also load.
- Values outside 0..63 are clamped on load.

---

## 7. Persistence

Store in the settings (see 11):

- 10 band values and the preamp value.
- EQ on/off, auto-load on/off.
- Band frequency mode (Winamp or ISO), limiter on/off, filter engine.
- Window position, windowshade state, visibility.
