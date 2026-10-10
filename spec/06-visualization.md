# 06 — Visualization

This document defines the built-in ("classic") spectrum analyzer and oscilloscope drawn in the skin's visualizer area, and the host interface for external visualization plugins.

---

## 1. Visualizer areas

| Where | Rectangle (1x) | Notes |
|---|---|---|
| Main window, normal | (24,43) 76×16 | 152×32 in double size, rendered natively (not pixel-doubled) |
| Main window, windowshade | (79,5) 38×5 | 76×10 in double size, rendered natively |
| Playlist editor bottom bar | (w−223, h−26) 72×16 | Only when the playlist is ≥ 350 wide, not shaded, **and the main window is hidden** |

Mouse events in the visualizer area belong to the window under it (02 §12, 04 §4.5).

---

## 2. Data pipeline

### 2.1 Source samples

- The visualizer uses the audio **after** EQ and DSP effects and **before** volume and balance (07 §4). So the display does not change with the volume slider.
- If ReplayGain applies the "no ReplayGain tag" gain to untagged files, divide that gain back out for the spectrum, so untagged files do not look quieter.
- Samples are fed in blocks of 576 frames, each block tagged with the time (ms) at which it will be heard.
- At draw time, use the block whose timestamp is closest to the current **output** position (what is heard now). If no block is within 400 ms, show the idle animation (§2.5).

### 2.2 Oscilloscope values

For each block, produce 75 values:

- For x = 0..74, take the sample at index floor(x × 576 / 75) (nearest sample, no interpolation).
- Convert each channel's sample to signed 8-bit (keep only the most significant byte), average across channels, divide by 8 and round. The result is about −16..+15.

### 2.3 Spectrum values

1. Accumulate samples as floating point in [−1, 1). Keep a left buffer and a right buffer of 512 samples (mono: same samples in both).
2. When 512 samples are available, form w[i] = (L[i] + R[i]) × hann[i], where hann[i] = 0.5 − 0.5 cos(2π i / 511), i = 0..511.
3. Compute a 512-point real FFT, unnormalized.
4. Magnitude for bins i = 0..255: m[i] = sqrt(re² + im²) × 0.5. (Bin 0 may combine DC and Nyquist; it is not used by any bar.)
5. Map 256 bins to **75 bars** on a semitone-like scale:
   - Let S = 255 / 2^(75/12). Define W(x) = (2^(x/12) − 1) × S.
   - Bar x covers the fractional bin range [W(x) + 1, W(x + 1) + 1). Bar 0 starts at bin 1.0; bar 74 ends near bin 253.6.
   - The bar value is the **sum** (not the mean) of the bins it covers. Whole bins inside the range add their full magnitude. The partial first and last bins add a value interpolated with a 4-point cubic Hermite (Catmull-Rom) through bins b−1, b, b+1, b+2, at the fractional position, multiplied by the fraction of the bin covered.
   - Clamp each bar to 0..255 and round to an integer.
6. Advance the buffers by **256 samples** (50% overlap) and add 256 / sample rate seconds to the block time stamp. At 44.1 kHz this gives about 172 spectra per second.

Because high bars cover many bins and are summed, high frequencies read larger than a plain average would show. This is the reference look.

### 2.4 Refresh timing

- The visualizer runs on a timer of about 16 ms (≈ 60 Hz). Bar falloff and peak motion advance **every** tick.
- Pixels are redrawn only every Nth tick, set by the "refresh rate" preference: N = 1 ("70 fps"), 2 ("35 fps", default), 3 ("18 fps"), 4 ("9 fps").
- The visualizer runs only while: the mode is not "off", playback is running and not paused, the player is not minimized, the main window is visible (or the playlist mini area applies), and no external visualization plugin is running with "disable built-in visualizer while a plugin runs" on (default on).
- On stop, draw one empty frame (background and grid only).

### 2.5 Idle animation

When no data is available within 400 ms of the current time (for example during buffering, or with a decoder that feeds no data):

- Spectrum: all bars 0 except one bar of value 15, at a column that moves 0 → 74 → 0, one step per tick.
- Oscilloscope: value at column x = 7 × sin((phase + x) × 0.1), with phase increasing each tick (a scrolling sine).

Decoders may also replace the data to show a buffering progress bar (07 §8).

---

## 3. Colours

Colours come from the skin's `viscolor.txt` (01 §5):

| Index | Use |
|---|---|
| 0 | Background |
| 1 | Grid dots |
| 2–17 | Analyzer bars, 2 = top, 17 = bottom |
| 18–22 | Oscilloscope, 18 = centre (brightest) to 22 = extremes |
| 23 | Analyzer peak dots |

---

## 4. Drawing — normal size, main window

The drawing surface is 76 wide by 16 high. Row 0 is the top. "Level" counts up from the bottom.

### 4.1 Background

Each drawn frame: fill with colour 0, then put colour 1 dots at every even x (0, 2, …, 74) on every odd row (1, 3, …, 15).

### 4.2 Reference baseline offset

In the reference implementation, the analyzer and oscilloscope are drawn with their baseline **two rows below the visible area**: level 0 and level 1 fall outside the 16-row box. As a result:

- A bar of level v shows v − 2 visible pixels (0 if v ≤ 2). The maximum bar (v = 15) covers rows 3–15.
- A peak at level p appears at row 17 − p, only when p ≥ 2.
- Oscilloscope value v (0..15) appears at row 17 − v, only when v ≥ 2.

Conformance must be checked against screenshots of the reference player (12 §5). If screenshots show otherwise, the screenshot wins.

### 4.3 Spectrum analyzer

State per column x (0..74): bar height `bh[x]` in 1/16 pixel units, peak height `ph[x]` in 1/256 pixel units, and peak speed `pv[x]`.

For each tick and each column x:

1. **Input level:**
   - Thick bands (default): t = x rounded down to a multiple of 4; v = integer average of bars t, t+1, t+2, t+3 (bar 75 counts as 0).
   - Thin bands: v = bar x.
   - Clamp v to at most 15.
2. **Bar falloff:** if v × 16 < bh[x], then bh[x] decreases by the falloff step and v = bh[x] / 16 (integer). Else bh[x] = v × 16. Clamp bh and v to ≥ 0. Falloff step by preference (slowest → fastest): 3, 6, **12** (default), 16, 32 (that is 0.1875 to 2 pixels per tick).
3. **Colour base c:** Normal style: c = 17. Fire style: c = v + 2. Line style: c = 17 − v.
4. **Peak:** if ph[x] ≤ v × 256, set ph[x] = v × 256 and pv[x] = 3.0.
5. **Draw** (on drawing ticks only), unless thick bands are on and x mod 4 = 3 (the 1-pixel gap between 3-pixel-wide bars; so thick mode shows 19 bars):
   - For y = 0..v−1 (level from the bottom), plot colour c − y (Normal and Fire styles) or colour c (Line style).
     - Normal: the gradient is fixed to the window (bottom pixel colour 17 at level 0).
     - Fire: every bar's top pixel has colour 3; the gradient moves with the bar.
     - Line: each bar is one flat colour chosen by its height.
   - If peaks are on (default on) and ph[x] / 256 is in 0..15, plot colour 23 at level ph[x] / 256.
6. **Peak motion:** ph[x] decreases by floor(pv[x]); then pv[x] is multiplied by the peak falloff factor; clamp ph[x] ≥ 0. Factor by preference (slowest → fastest): 1.05, **1.1** (default), 1.2, 1.4, 1.6. The peak holds its height, then falls with increasing speed.

### 4.4 Oscilloscope

For each column x (0..74) with value s from §2.2:

1. v = s + 8, clamped 0..15.
2. Colour c = 18 + |floor(v / 2) − 4|. The centre (v = 8, 9) is colour 18; the extremes are colours 21–22.
3. Style:
   - **Dots:** plot level v.
   - **Lines** (default): plot a vertical run between the previous column's v and this v (the first column uses its own v).
   - **Solid:** fill from level v to the centre: levels 8..v when v > 7, else levels v..7.

---

## 5. Drawing — other sizes

### 5.1 Double size, main window (152×32)

- Rendered at full resolution. No baseline offset: level 0 is the bottom row of the 32-row area.
- Background: colour-1 dots as 2×2 blocks at x ≡ 0,1 (mod 4) and y ≡ 2,3 (mod 4) (the 1x pattern scaled up).
- Spectrum: each column is 2 pixels wide. A bar of level v is 2v rows high; the pixel at row level y has colour c − floor(y / 2) (Normal/Fire) or c (Line). Peaks are 2×2 at rows 2p, 2p + 1, drawn only when p > 0.
- Oscilloscope: each sample is a 2×2 block at rows 2v, 2v + 1.

### 5.2 Windowshade, single size (38×5)

- Background: colour 0, no grid.
- Spectrum: 37 columns (x = 0..36).
  - Thick: average of bars 2 × (x rounded down to a multiple of 4) .. +3. Thin: average of bars 2x and 2x + 1.
  - Falloff and peaks use the same state machine, indexed by 2x.
  - Display height = min(v × 5 / 15, 5). Pixel colour at row level y: c − (y × 15 / 5) (Normal/Fire) or c (Line).
  - Peak row = ph / 256 × 5 / 15 (0..4).
  - Thick gap rule: skip columns where x mod 4 = 3.
  - Baseline: the bottom visible row (no offset).
- Oscilloscope: 38 columns using the **first 38** of the 75 values (not resampled). v = (s + 8) × 5 / 16, clamped 0..4. Colour always 18. Dots/lines/solid as in §4.4, with centre row 2.

### 5.3 Windowshade, double size (76×10)

Rendered natively at 75×10: spectrum height v × 10 / 15, colours c − (y × 15 / 10), peaks 0..9. Oscilloscope v = (s + 8) × 10 / 16, clamped 0..9, solid centre row 4.

### 5.4 Playlist mini area

Copies the left 72×16 of the single-size main-window image. Shown only when the main window is hidden (§1).

---

## 6. Interaction and preferences

### 6.1 Mouse

- Left-click in the visualizer area while playing (normal mode) cycles: off → spectrum → oscilloscope → off. In windowshade mode the click works while stopped too.
- Double-click undoes the click's mode change and starts or stops the external visualization plugin. In windowshade mode, double-click toggles between off and spectrum.
- Right-click opens the visualization menu.

### 6.2 Visualization menu

The reference 5.x player shows only plugin items here. A Winamp 2.x-style implementation must offer:

```
Visualization mode ▸   Spectrum analyzer / Oscilloscope / Off   (radio)
Analyzer options ▸     Normal style / Fire style / Line style   (radio)
                       ─────
                       Thin bands / Thick bands                 (radio)
                       ─────
                       Show peaks                               (check)
Oscilloscope options ▸ Dot scope / Line scope / Solid scope     (radio)
Refresh rate ▸         Full (70 fps) / Half (35 fps) / Quarter (18 fps) / Eighth (9 fps)
Analyzer falloff ▸     Slowest / Slow / Moderate / Fast / Fastest
Peak falloff ▸         Slowest / Slow / Moderate / Fast / Fastest
─────
Start/stop plugin                 Ctrl+Shift+K
Configure plugin...               Alt+K
Select plugin...                  Ctrl+K
```

### 6.3 Preferences and defaults

| Setting | Default |
|---|---|
| Mode | Spectrum analyzer |
| Analyzer style | Normal |
| Band width | Thick |
| Show peaks | On |
| Analyzer falloff | Moderate (step 12) |
| Peak falloff | Slow (factor 1.1) |
| Oscilloscope style | Lines |
| Refresh rate | Half (35 fps) |

---

## 7. External visualization plugins

### 7.1 Data contract

The host gives each visualization plugin, at each render call:

- Up to 2 channels of **waveform** data: 576 signed 8-bit samples per channel.
- Up to 2 channels of **spectrum** data: 576 unsigned 8-bit magnitudes per channel, low to high frequency.
- The sample rate and channel count of the stream.
- Data aligned to the current output position plus the plugin's requested latency.

Spectrum data for plugins is computed per channel (or from a mono downmix when the plugin asks for one channel): take 512 samples (8-bit precision is sufficient), remove DC with a one-pole high-pass (y = x − x₋₁ + 0.99 y₋₁), apply a 512-point Hann window, FFT, magnitude / 16 clamped to 255. Each of the 256 bins produces two values (average of this bin and the previous one, then this bin), giving 512 values; pad to 576 with values that halve at each step.

A modern host should **also** offer full-resolution floating-point PCM and FFT data to plugins that request it.

### 7.2 Host behaviour

- Plugins run on their own thread (or render on a GPU context owned by the host).
- One visualization plugin at a time. Optional "auto-start on play" (default off).
- "Disable built-in visualizer while a plugin runs" (default on).
- The plugin draws in its own window. Prefer hosting it in a generic skinned window (05 §1), with a full-screen option.
- Actions: start/stop, configure, select, next preset, previous preset, random preset, full screen.

### 7.3 Preset-compatible engines

| Original | Preset format | Cross-platform approach |
|---|---|---|
| MilkDrop 2 | `.milk` text presets (version 1.x and 2.x: per-frame/per-vertex equations plus optional shader code) | Embed **libprojectM** (OpenGL/GLES). Feed it PCM aligned to the output time. Ship a preset collection with compatible licences |
| Advanced Visualization Studio (AVS) | `.avs` binary presets starting with `Nullsoft AVS Preset 0.2` followed by 0x1A | No drop-in library. Optional: integrate an open-source AVS reimplementation. Low priority |
| Simple full-screen modes (scrolling voiceprint, spectrum + voiceprint, spectrum + oscilloscope) | none | Easy to implement natively. Optional |

See 08 §6 for the plugin interface.
