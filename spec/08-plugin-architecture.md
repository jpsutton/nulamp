# 08 — Plugin Architecture

Winamp 2's modular design is one of its major features. The new player must be modular in the same way: decoders, outputs, effects, visualizations and general extensions are separate modules behind stable interfaces.

This document defines:

1. The native module model (required).
2. An optional compatibility layer for original Winamp 2 binary plugins (Windows only, or through a bridge).

All interfaces here are described by behaviour. Choose the language binding (C ABI recommended for third-party modules) during design.

---

## 1. Module kinds

| Kind | Count active | Role |
|---|---|---|
| Input (decoder) | Many loaded; one per playing item | Read a file, stream or device; produce PCM; read/write metadata |
| Output | One selected | Play PCM on a device (or write to a file); report the heard position |
| DSP / effect | One selected (an ordered chain is a recommended extension) | Change PCM between the EQ and the output |
| Visualization | One running | Draw from PCM / spectrum data |
| General | Many | Anything else: global hotkeys, tray controls, remote control, scrobbling, media library, lyrics |
| Playlist format | Many | Read (and optionally write) a playlist file type (10) |
| Skin component | — | Not pluggable; the classic skin engine is core |

---

## 2. Common rules

- **Discovery:** modules are loaded from a system plugin directory and a user plugin directory. Each module reports a name, version, description and its interface version.
- **Life cycle:** `init` at start-up (or on first use for DSP and visualization), `quit` at exit. A module that fails `init` is not used.
- **Configuration:** each module may provide "configure" and "about" actions. Module settings are stored in the module's own section of the settings store (11).
- **Isolation:** a crash in a module must not lose the user's playlist or settings. Recommended: catch faults where the platform allows, and offer a "safe mode" start that loads no third-party modules.
- **Threading:** input decoding, visualization and DSP run off the user-interface thread. Modules must not call user-interface functions from those threads except through the host's message queue.
- **Unicode:** all strings are UTF-8 (or UTF-16 on Windows if chosen), never system code pages.
- **Host API:** modules can query and control the player through the same command set as remote control (11 §6): transport, playlist, volume, EQ, current title, settings paths, skin colours.

---

## 3. Input module

### 3.1 Claims

- A list of (extension list, description) pairs, for example ("mp3;mp2;mp1", "MPEG Audio Files"). Used by file dialogs and folder scanning.
- An optional "is this mine?" check for a location. It is called before extension matching (used for URL schemes and content sniffing).
- Optional MIME types for stream content types.

Selection order: 07 §7.

### 3.2 Playback

| Operation | Behaviour |
|---|---|
| Play(location) | Start decoding. Return quickly. Return "not found", "error" or "ok" |
| Pause / Unpause / IsPaused | |
| Stop | Stop decoding and release the output |
| GetLength | Milliseconds, or "unknown" |
| GetOutputTime | Heard position in ms (normally the output's heard position) |
| Seek(ms) | Request a seek. The decoder seeks, then flushes the output to the new time |
| SetVolume / SetBalance | Usually forwarded to the output |
| IsSeekable | May change during playback |
| Info callback | Bit rate, sample rate, channels, in-sync state |
| Title-changed callback | For streams |
| End-of-stream callback | Raised when all audio has been written and the output has played it |

### 3.3 Feeding audio

- The host gives the decoder a handle to the output stage. The decoder loops:
  1. Wait until the output can accept a block. When the DSP stage may enlarge the block, require room for twice the block.
  2. Decode one block (576 frames is the classic size; any size is allowed).
  3. Pass the block to the host processing chain (gain, EQ, DSP), which may change the frame count.
  4. Pass the processed block and its time stamp to the visualization feeds.
  5. Write the block to the output.
- At the end: tell the output to drain, wait until it has played everything, then raise end-of-stream.
- **Recommended native design:** the host owns the loop and pulls blocks from the decoder ("read N frames", "seek", "close"). Decoders then contain no threading code. Gapless playback and crossfade (07 §6) need the host to open the next decoder before the current one ends; a pull model makes this simple.

### 3.4 Metadata

- Get a field by name (10 §7): returns "found with value", "understood but empty", or "not understood". A field the decoder does not understand is disabled in the file-info dialog.
- Set a field by name (empty value deletes it), then Write to commit all pending changes.
- Optional: clear all tags; embedded album art get/set/delete.
- Technical fields: `length` (ms), `bitrate` (kbps), `samplerate`, `channels`, `bitspersample`, `lossless`, `type` (0 audio, 1 video), `family` (format description), `mime`, `formatinformation` (multi-line description), `streamtitle`, `streamname`, `streamurl`, `streamgenre`, `streaminformation` for streams.

### 3.5 Decode to PCM

Open a location and read PCM (with seeking when possible) without the playback path. Used by the format converter, CD ripper, ReplayGain scanner and "write to file" output. Optional request: preferred sample format (16/24/32-bit integer or float).

### 3.6 Info dialog

Either the decoder uses the host's standard file-info dialog (10 §8) with extra panes, or it provides its own dialog.

---

## 4. Output module

| Operation | Behaviour |
|---|---|
| Open(sample rate, channels, bits/format, buffer hint, prebuffer hint) | Returns the maximum latency in ms, or failure |
| Close | Stop. With a fade preference, fade out without blocking |
| Write(data) | Accept or refuse (non-blocking) |
| CanWrite | Bytes that can be written now |
| Drain / IsPlaying | After the last write, report when everything has been heard |
| Pause(on/off) | Returns the previous state |
| Flush(time) | Discard buffered audio; next audio starts at this time (seek) |
| SetVolume(0..255), SetBalance(−127..127) | |
| GetOutputTime | Heard position, ms |
| GetWrittenTime | Written position, ms (used to time-stamp visualization data) |

The output implements fades, crossfade and gapless behaviour (07 §6), or the host does it before the output (recommended native design: host-side mixing; outputs are simple device sinks).

---

## 5. DSP / effect module

- A module file may contain several effects. The user selects one (or a chain, as an extension).
- **Process(samples, frame count, bits, channels, sample rate)** → new frame count. The effect may return up to 2× the input frame count and as few as ½. The host provides a buffer large enough for 2×.
- Called on the decode thread (or the host processing thread).
- Recommended native interface: float samples, a declared latency, and a flush call on seek.
- **Scripted effects (optional):** a built-in DSP that runs user scripts in a small expression language per sample, with variables for the left and right samples, the sample rate, channel count and four user sliders, and three code sections (init, per sample, on slider change). This matches the "signal processing studio" presets popular with Winamp 2 users (for example vocal removal: left = left − right).

---

## 6. Visualization module

- A module file may contain several visualizations. The user selects one.
- The plugin requests 0–2 channels of spectrum data and 0–2 channels of waveform data (06 §7.1), a latency in ms (the host gives data for "now + latency"), and a delay between frames.
- The host calls **init**, then **render** repeatedly (render can request exit), then **quit**, all on one dedicated thread.
- Thread priority is a preference (default normal).
- Recommended native interface: the host provides a GPU drawing surface inside a generic skinned window (05 §1), float PCM and FFT data, and keyboard/mouse events, and asks for next/previous/random preset and full screen.

---

## 7. General module

- **init** at start-up, **config**, **quit**.
- Can observe player events: track started, track stopped, title changed, volume changed, EQ changed, playlist changed, skin changed, paused/unpaused, info (bit rate etc.) changed.
- Can send any remote-control command (11 §6).
- Can add items to menus and add pages to preferences.
- Can host a generic skinned window.
- Can provide the "next item" and "previous item" hooks (07 §3.4), for queue modules.
- Can provide a title-formatting override and extra metadata fields.

Built-in general features that must be supplied (as modules or core):

- **Global hotkeys** (11 §5).
- **Media keys and OS media controls** (MPRIS on Linux, Now Playing / remote command centre on macOS, system media transport controls on Windows).
- **Tray / status icon** (11 §7).

---

## 8. Winamp 2 binary plugin compatibility (optional)

Many original Winamp 2 plugins are Windows DLLs. A compatibility layer is optional. It can run on Windows natively, or on other platforms through a separate bridge process that loads the DLL with a Windows API emulation layer. If implemented, it must follow the publicly documented Winamp plugin SDK interfaces. This section lists what the layer must provide; it does not replace the SDK documentation.

### 8.1 Module discovery

| Kind | File pattern | Entry point (exported) |
|---|---|---|
| Input | `in_*.dll` | `winampGetInModule2` |
| Output | `out_*.dll` | `winampGetOutModule` |
| General | `gen_*.dll` | `winampGetGeneralPurposePlugin` |
| DSP | `dsp_*.dll` | `winampDSPGetHeader2` |
| Visualization | `vis_*.dll` | `winampVisGetHeader` |

Optional exports: an uninstall hook, extended metadata get/set/write (ANSI and wide-character forms), "decode to PCM" read functions, a unified-file-info flag and pane provider.

### 8.2 Input module record (in order)

Version; description; host window handle (host fills); module instance (host fills); extension list (pairs of NUL-terminated strings ending with an extra NUL); seekable flag; flags (uses output module = 1, does its own EQ = 2, applied ReplayGain = 8, applied ReplayGain preamp = 16); config(parent window); about(parent window); init; quit; get-file-info(location or NULL for current, title out, length-ms out); info-box(location, parent window); is-our-file(location); play(location); pause; unpause; is-paused; stop; get-length; get-output-time; set-output-time; set-volume; set-pan; then host-provided functions: visualization init(max latency, sample rate), visualization de-init, classic-vis add PCM data(data, channels, bits, time stamp), classic-vis get mode, classic-vis add precomputed data, plugin-vis add PCM data, plugin-vis get mode, plugin-vis add precomputed data, plugin-vis set info(sample rate, channels), DSP is-active, DSP do-samples(samples, frame count, bits, channels, sample rate); then plugin-provided EQ set(on, ten band bytes, preamp byte); host-provided set-info(bit rate kbps, sample rate kHz, channels, in-sync); output module pointer (host fills).

Version values: 0x100 (original) and 0x101 (wide-character description). A high-bit flag indicates wide-character file names; another indicates that init returns a status.

### 8.3 Output module record (in order)

Version (0x10, or 0x11 for wide-character description); description; id; host window handle; module instance; config; about; init; quit; open(sample rate, channels, bits per sample, buffer length ms, prebuffer ms) → max latency or < 0; close; write(buffer, length) → 0 accepted / 1 not now; can-write → bytes; is-playing; pause(on) → previous; set-volume(0..255, or −666 for "current"); set-pan(−128..128); flush(time ms); get-output-time; get-written-time.

### 8.4 DSP header and module records

Header: version (0x20, or 0x22 when the entry point takes the host window); description; get-module(index) → module or NULL; an optional key-check function.

Module: description; host window; module instance; config(module); init(module) → 0 OK; modify-samples(module, samples, frame count, bits, channels, sample rate) → new frame count; quit(module); user data.

### 8.5 Visualization header and module records

Header: version (0x101, or 0x102 when the entry point takes the host window); description; get-module(index).

Module: description; host window; module instance; sample rate; channels; latency ms; delay ms; spectrum channels (0–2); waveform channels (0–2); spectrum data [2][576] bytes; waveform data [2][576] bytes; config; init → 0 OK; render → 0 OK, 1 quit; quit; user data.

### 8.6 General plugin record

Version (0x10, or 0x11 wide); description; init → 0 OK; config; quit; host window; module instance.

### 8.7 Host messaging

General plugins and many others control the player by sending window messages to the host's main window and by subclassing it to observe notifications. A compatibility layer must:

- Provide a main window handle that accepts the documented command messages (the "WM_COMMAND" button and menu identifiers, and the "IPC" message numbers; see 11 §6.3 for the important ones).
- Forward the host's notification messages (title changed, playing file, stopped, EQ changed, etc.) to subclassed procedures.
- Expose a skinned-window embedding service and the `genex.bmp` colour query so plugin windows look skinned.

Practical scope: inputs, outputs and DSPs with no user interface are the most valuable. General plugins depend heavily on Windows behaviour and may not work through a bridge.
