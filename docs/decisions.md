# Implementation decisions

This file records choices made where `spec/` is silent, where it offers options, or where the project deliberately departs from it. Add an entry whenever such a choice is made.

## Clean-room rule

The implementation is written only from `spec/`, public format standards and the documentation of third-party libraries. Contributors must not read Winamp source code from any place, and must not read the source of other re-implementations of classic skins.

## Project-level decisions

| # | Topic | Decision | Reason |
|---|---|---|---|
| D1 | Language | Rust, cargo workspace | Spec README §3 recommends a systems language with C interop |
| D2 | Text rendering | All text is 1-bit and hinted (FreeType mono mode) or uses skin bitmap fonts. No antialiasing, no subpixel rendering. Optional AA preferences from the spec default to off | Project owner requirement: small text must stay crisp and readable |
| D3 | Dialog toolkit | Own widget kit drawn with `genex.bmp` (05 §1.5). Native dialogs are used only for file pickers | Consistent skinned look; keeps D2 |
| D4 | Media library | In scope, although spec README §1 lists it as a non-goal. Local SQLite provider behind a provider trait; remote providers (Jellyfin, Subsonic) later | Project owner requirement. Designed from first principles, see `media-library.md` |
| D5 | Wayland | First run through XWayland; native compound-surface mode later (05 §6.5) | Wayland clients cannot position their own windows |
| D6 | Easter egg | The title-bar easter egg word is `NULAMP` | 02 §2.1 allows changing the word; avoids the original mark |
| D7 | Licence | MIT OR Apache-2.0. GPL or LGPL libraries only behind opt-in cargo features | Spec README §2.4 |
| D8 | `shufrep.bmp` layout (spec gap) | Repeat 28×15 at x 0, shuffle 47×15 at x 28; rows y 0 off, 15 off+pressed, 30 on, 45 on+pressed. Destinations: shuffle (164,89), repeat (210,89). EQ button 23×12 and PL button 23×12: off (0,61)/(23,61), off+pressed (46,61)/(69,61), on (0,73)/(23,73), on+pressed (46,73)/(69,73); destinations (219,58) and (242,58) | Spec 02 lists hit boxes but not sprites. Filled from the publicly known bitmap layout that real skins use (black-box interoperability fact, consistent with the 92×85 size in 01 §2.2). No source code consulted |
| D9 | Remaining time with 100+ minutes (02 §7.1) | The sign takes the x = 36 slot; the hundreds digit is not drawn | Spec gives one slot for "sign, or hundreds of minutes" |
| D10 | Large time digits, elapsed mode | With `numbers.bmp`, the minus line is cleared by copying (9,6) 5×1 to (40,32); with `nums_ex.bmp`, a blank cell is drawn at x = 36 when minutes < 100 | Spec defines the clear for the blank state only; same mechanism reused |
| D11 | kbps ≥ 10000, kHz ≥ 100 | kbps follows 02 §7.3 literally; kHz shows (value / 10) mod 10 and value mod 10 | Spec is literal for kbps; silent for kHz above 99 |
| D12 | Overlapping main-window sprites | Mono (212..239) and stereo (239..267) share column 239; shuffle (164..210) and repeat (210..237) share column 210. Mono and shuffle are drawn first | The right-hand sprite stays complete; matches the hit boxes (repeat starts at 210) |
| D13 | Presentation | `softbuffer` (CPU blit of each window's frame) instead of `wgpu`. Shapes use platform regions (XShape, `SetWindowRgn`). A GPU presenter can be added behind the same interface for platforms that need per-pixel alpha | Windows are small; CPU blits are pixel-exact and avoid a GPU dependency |
| D14 | Main-window title font | The skinned bitmap font (`text.bmp`) is the default (spec 02 §8.1 default is the system font). A title containing characters the bitmap font cannot draw falls back to the 1-bit system font automatically | Project owner preference: the bitmap font looks more authentic; the fallback keeps non-Latin titles readable |
| EQ1 | EQ marquee text (03 §1.3) | Band labels `70 Hz` … `1 kHz` … `16 kHz` (ISO `31.5 Hz`, `63 Hz`, …); text `EQ: 70 Hz: +12.0 dB` / `Preamp: -0.6 dB`; zero is `+0.0`. ISO centres are 31.5 and 63 Hz | Spec lists the label values and sign but not spacing, units or the sign of zero |
| EQ2 | EQ gain changes (03 §4.2) | The gain stage and every band gain glide to new values with a 10 ms one-pole curve. Each band runs its boost and cut band-pass as separate paths sharing input history, so a sign change cross-fades. A band whose gain reaches 0 stops; it restarts with cleared history and a gain rising from 0 | "No clicks" without a stated method; a click test bounds the sample-to-sample step |
| EQ3 | Limiter (03 §4.4) | One detector shared by all channels of a frame (linked), decayed once per frame. Order: detect = max(peak, detect), scale if above 0.930, then decay, so output never exceeds 0.930. Bypassed with the bands when the EQ is off or flat | Spec does not say per channel or linked; linking keeps the stereo image |
| EQ4 | EQ numerics (03 §4.2) | Coefficients designed in f64, stored and run in f32 (direct form I, b1 = 0). Filter states below 1e-15 are flushed to zero after each block instead of setting CPU flush-to-zero flags | Spec allows a DC offset or flushing; flushing needs no `unsafe` or platform code |
| EQ5 | Preset names (03 §6) | Read: UTF-8 if valid, else Windows-1252. Write: Windows-1252 when every character fits and the bytes would not read back as UTF-8, else UTF-8. NUL characters are dropped; names are cut at a character boundary to 256 bytes | Every name written reads back unchanged; ASCII and Latin-1 names stay compatible with the reference player |
| EQ6 | Auto-load key (03 §5.4) | The track's file name is the text after the last `/` or `\` | q2 files may come from Windows and paths may be URLs |
| EQ7 | ReplayGain details (07 §5) | Disabled → factor 1.0 (no no-tag gain). Gains parse as `[+-]digits[.digits]` plus optional `dB` (any case), no exponent; peaks missing, zero or invalid count as 1.0. `R128_*_GAIN` is used only when the `REPLAYGAIN_*_GAIN` of the same source is missing. No-tag gain and preamp are clamped to −14..+6 dB | Spec leaves these edge cases open |
| EQ8 | Constant-Q engine (03 §4.3) | Implemented as y = x + (G − 1) × band-pass(Q 1.41), which equals the bilinear transform of the given prototype. Both engines disable bands at ≥ 0.499 × sample rate (stricter than 2 × centre > rate) and design with the rate clamped to 4–384 kHz. Switching engines needs no restart | Algebraic identity; one code path for both engines |
| EQ9 | Gain stage (07 §4) | `Equalizer::set_pre_gain` (ReplayGain × its preamp) applies even when the EQ is off or flat; the EQ preamp applies only when the EQ is active | 07 §4 puts both in one stage but only the preamp depends on the EQ |
| EQ10 | Preset file writes (03 §6) | A new entry is appended after the last complete entry (a partial trailing entry is overwritten). EQF import matches `Entry1` ignoring case. Delete with no match leaves the file untouched; the temporary file is `<name>.tmp` next to it | Spec gives the rules but not these cases |
| EQ11 | EQ channels and settings | `Equalizer` takes 1..8 interleaved channels and panics otherwise; a trailing partial frame is left unchanged. `EqSettings` also carries the filter engine (03 §7) | 07 §4 requires up to 8 channels |
