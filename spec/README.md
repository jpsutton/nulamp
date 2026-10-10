# Classic Skinnable Audio Player — Implementation Specification

This specification describes a new, cross-platform audio player that is compatible with Winamp 2.x classic skins (`.wsz`) and reproduces the major features of Winamp 2: the main window, equalizer, playlist editor, classic visualizer, modular decoders and plugins, playlist formats, title formatting, metadata, streaming and remote control.

The documents are written so that a team can build the player from scratch without access to any original source code.

---

## 1. Goals

1. **Skin compatibility.** Any classic Winamp 2.x skin renders and behaves as it did in the original player, pixel for pixel where that is defined.
2. **Feature parity** with Winamp 2.x, plus the most-used features that later 5.x versions added to the classic interface (ReplayGain, gapless playback, more formats, title formatting functions).
3. **Cross-platform:** Windows, macOS and Linux (X11 and Wayland) as first-class targets; BSD desirable.
4. **Modular:** decoders, output, effects, visualizations and general extensions are plugins behind stable interfaces.
5. **Data compatibility:** read original presets (`.eqf`, `winamp.q1`), playlists, bookmarks and settings, so existing users can migrate.

### Non-goals

- Modern (XML/"Wasabi") skins (`.wal`, `skin.xml`).
- The Winamp 5 media library, portable-device sync, online services, CD burning, DRM.
- Binary compatibility with original plugins is optional (08 §8).

---

## 2. Clean-room rules

The original Winamp source code is available to read under a licence that **forbids distributing modified versions**. This specification was written from observed behaviour, file formats and interface facts. To keep the new implementation independent:

1. **Implementers must not read or copy the original source code.** This specification is the only reference needed. Where something is not specified, test against the original player's behaviour (black-box), not its code.
2. **No original assets.** Ship an original default skin, icons, cursors, fonts, about-box artwork and help texts. The default skin must follow the bitmap sizes and layouts in 01–05 so that skin authors' expectations hold.
3. **No trademarks.** Do not name the product "Winamp" or use its logos, the "llama" slogan or similar marks. Describing compatibility ("plays Winamp 2 skins") is fine. File names that skins and users depend on (for example `main.bmp`, `region.txt`, `winamp.q1` when importing) are interoperability facts and may be used.
4. **Third-party libraries** are allowed under their own licences (see 09). Check licence compatibility for the chosen distribution model (for example fdk-aac is not GPL-compatible).
5. Numbers, coordinates, file formats, protocol details and algorithm descriptions in this spec are functional facts needed for compatibility.

---

## 3. Architecture overview

```
 ┌─────────────────────── User interface (skin engine) ─────────────────────────┐
 │ Main window │ Equalizer │ Playlist editor │ Generic windows │ Video window   │
 │   02        │   03      │   04            │   05            │   05           │
 │ Skin loader (01) · docking/snapping (05 §6) · menus, keys, dialogs (11)      │
 └───────────────┬──────────────────────────────────────────────┬───────────────┘
                 │ commands / events                            │ draw data
 ┌───────────────▼──────────────────┐            ┌──────────────▼──────────────┐
 │ Player core (07)                 │            │ Classic visualizer (06)     │
 │ state machine · playlist (10)    │            │ + visualization plugins     │
 │ shuffle · title formatting (10)  │            └──────────────▲──────────────┘
 │ settings (11) · remote control   │                           │ PCM/FFT tap
 └───────────────┬──────────────────┘                           │
                 │                                              │
 ┌───────────────▼───────────────────────────────────────────────┴─────────────┐
 │ Audio pipeline (07 §4)                                                       │
 │ Input module → gain (preamp × ReplayGain) → 10-band EQ → DSP plugin → tap →  │
 │ output stage (resample, fades, crossfade, gapless, volume/balance) → device  │
 └──────────────────────────────────────────────────────────────────────────────┘
       ▲ decoders, tag readers (09, 10 §7)          ▲ output back ends (07 §6.7)
```

Recommended technology choices (not mandatory):

| Concern | Suggestion |
|---|---|
| Language | A systems language with good C interop (C++, Rust, Zig) |
| Windowing | Per-window native surfaces with shape support; a thin portable layer (for example SDL3, or native per platform). Classic skins need pixel-exact, unscaled blitting and per-window input shapes |
| Audio output | miniaudio, PortAudio, or native back ends |
| Decoding | mpg123, libFLAC, libvorbis, libopus, libsndfile, libopenmpt, FluidSynth, libwavpack, FFmpeg for the rest |
| Tags | TagLib |
| CD | libcdio + paranoia, libdiscid |
| Visualization plugins | libprojectM for MilkDrop presets |
| Archives | minizip or libzip |
| Remote control | D-Bus/MPRIS (Linux), platform media sessions |

---

## 4. Document map

| File | Contents |
|---|---|
| [01-skin-format.md](01-skin-format.md) | Skin containers, file list, fallbacks, BMP decoding, `region.txt`, `viscolor.txt`, `pledit.txt`, `text.bmp` font, cursors |
| [02-main-window.md](02-main-window.md) | Main window sprites and positions, time and bit-rate display, marquee, double size, windowshade, mouse, menus, tooltips |
| [03-equalizer.md](03-equalizer.md) | EQ window, value encoding, filter design, limiter, presets, EQF/q1/q2 format |
| [04-playlist-editor.md](04-playlist-editor.md) | Playlist window, tiling, text, pop-up menus, selection, keyboard, list operations |
| [05-aux-windows-docking.md](05-aux-windows-docking.md) | Generic skinned window (`gen.bmp`, `genex.bmp`), video window, legacy frames, docking and snapping |
| [06-visualization.md](06-visualization.md) | Classic spectrum analyzer and oscilloscope algorithms, colours, visualization plugin data |
| [07-playback-engine.md](07-playback-engine.md) | Transport, end-of-track logic, shuffle, processing chain, ReplayGain, output, gapless, crossfade, streaming |
| [08-plugin-architecture.md](08-plugin-architecture.md) | Module kinds and interfaces; optional binary compatibility with original plugins |
| [09-formats.md](09-formats.md) | Decoders, encoders, CD, MIDI, tracker modules, per-format behaviour |
| [10-playlists-metadata.md](10-playlists-metadata.md) | Playlist model and file formats, title formatting language, metadata mapping, file info, bookmarks, jump to file, ID3v1 genres |
| [11-config-commands.md](11-config-commands.md) | Settings, import of original settings, keyboard shortcuts, menus, command line, global hotkeys, remote control, tray, preferences |
| [12-conformance.md](12-conformance.md) | Test corpus, unit tests, golden-image tests, audio and streaming tests, open items |

---

## 5. Conventions

- "Must" means required for compatibility. "Should" and "recommended" mean strongly advised. "Optional" means the implementer decides.
- "Reference player" or "reference" means the original Winamp. Where the reference has a defect, the spec says so and gives the behaviour a new implementation should have. If the defect is visible in skins or files that users depend on, the spec asks to keep it.
- Skin coordinates are 1x skin pixels; rectangles are `(x,y) WxH`; hit boxes `x1..x2 × y1..y2` are inclusive (01 conventions).
- Times are milliseconds unless stated. Volume is 0–255; balance is −127..127; EQ values are 0–63 with 0 = +12 dB.

---

## 6. Suggested milestones

| Milestone | Scope |
|---|---|
| M1 — Skinned shell | Skin loader (01), main window drawing and mouse (02) with a fake player, docking (05 §6), default skin |
| M2 — Playback | Core state machine and playlist (07 §1–3, 10 §1–2), MP3/WAV/Vorbis/FLAC decoders, output with buffering and fades, time display, seek, volume |
| M3 — EQ and playlist editor | 03 complete, 04 complete, M3U/PLS read and write, title formatting, metadata read |
| M4 — Visualization and streaming | 06 complete, ICY streaming, MIDI, tracker modules, CD playback |
| M5 — Plugins and integration | Module interfaces (08), ReplayGain, gapless, crossfade, tag writing and file info, settings import, command line, remote control, global hotkeys, tray |
| M6 — Conformance | Golden-image and audio test suites (12) passing on all platforms |
