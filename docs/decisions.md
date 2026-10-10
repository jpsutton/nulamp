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
