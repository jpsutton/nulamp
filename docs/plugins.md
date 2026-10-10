# Writing nulamp plugins

nulamp loads native plugins: shared libraries (`.so` on Linux and BSD, `.dylib` on macOS, `.dll` on Windows) that export one C function. The interface is a plain C ABI, defined in [`include/nulamp_plugin.h`](../include/nulamp_plugin.h), so plugins can be written in C, C++, Rust, Zig or any language that can export a C function. This guide explains the rules the header comments only summarise. The design follows spec 08 §1–§7; the choices are listed in `decisions.md` (PLG1–PLG13).

Binary compatibility with original Winamp 2 plugins (08 §8) is a separate, optional layer and is not covered here.

## 1. One file, many modules

A plugin file can hold any number of modules of four kinds:

| Kind | Constant | Record | What it does |
|---|---|---|---|
| Input | `NULAMP_KIND_INPUT` | `NulampInputApi` | Decodes files or streams to float PCM; reads and writes metadata |
| DSP | `NULAMP_KIND_DSP` | `NulampDspApi` | Changes PCM between the equalizer and the output |
| Visualization | `NULAMP_KIND_VIS` | `NulampVisApi` | Draws from spectrum, waveform and PCM data |
| General | `NULAMP_KIND_GENERAL` | `NulampGeneralApi` | Observes player events and controls the player |

Output modules are not pluggable yet; the player has a built-in output stage.

## 2. The entry point

```c
NULAMP_EXPORT const NulampPluginDesc *nulamp_plugin_entry(const NulampHost *host);
```

The host calls it once, on the user-interface thread, right after loading the file. Return a `NulampPluginDesc` that lists your modules, or `NULL` to refuse loading (for example when `NULAMP_ABI_MAJOR_OF(host->abi_version)` is not the major version you were built for). Everything the description points to (module records, kind records, strings) must stay valid and unchanged until the file is unloaded; `static const` data is the simplest way.

Each module is a `NulampModule` (name, version, description, `init`, `quit`, optional `configure` and `about`, and a `user` pointer passed back to every module-level call) whose `api` points to the record of its kind.

## 3. Versioning

- `NULAMP_ABI_VERSION` is `major << 16 | minor`. This document describes 1.0.
- **Every struct starts with `abi_version` and `struct_size`.** Fill them with `NULAMP_ABI_VERSION` and `sizeof(the struct)` in every record you provide.
- A struct only ever grows by appending fields, in a new minor version. A reader never touches fields beyond `struct_size`. The host rejects a record whose major version differs from its own or whose `struct_size` is smaller than the 1.0 size.
- Output records (`NulampStatus`, `NulampStreamInfo`) are prepared by the caller: set `abi_version` and `struct_size` before passing them. The callee writes only the fields that fit and keeps your `struct_size`.
- A plugin built against a newer minor version must check `host->struct_size` before calling a host function that the older host might not have.

## 4. Life cycle

1. The host loads the file and calls `nulamp_plugin_entry`.
2. For each module, the host calls `init(user, host, module_id)`. Keep `module_id`: it identifies the module in host calls (log, settings, `post_to_ui`). Return `NULAMP_OK`, or an error code to disable the module. A disabled module gets no other calls, not even `quit`.
3. The player uses the module. DSP modules make instances with `create`/`destroy`, visualizations with `start`/`stop`, input modules open decoder handles with `open`/`close`.
4. When the player shuts down (or unloads the plugin), it destroys every instance and handle, then calls `quit(user)` for each initialised module, then unloads the file.

`init` runs for every kind at load time, also for DSP and visualization modules; do expensive set-up in `create` or `start`.

`configure` and `about` are optional. They open your own dialog; there is no parent window yet, so a module without a user interface leaves them `NULL`.

## 5. Threading

| Call | Thread |
|---|---|
| `nulamp_plugin_entry`, `init`, `quit`, `configure`, `about` | User-interface thread |
| General `on_event` | User-interface thread |
| Input `claims`, `open`, `get_field`, `set_field`, `write_tags` | Any thread, possibly several at once |
| Input decoder handle (`info`, `read`, `seek`, `kbps`, `in_sync`, `take_title`, `close`) | One thread at a time; the handle may move between threads |
| DSP instance (`configure`, `process`, `latency_frames`, `flush`, `destroy`) | One thread at a time (the audio thread) |
| Visualization instance (`start`, `render`, `stop`) | One dedicated thread for the whole life of the instance |
| Every `NulampHost` function | Any thread, including from inside a call the host made into the plugin |

Never touch user-interface functions from a decoding, DSP or visualization thread. Use `post_to_ui` to run code on the user-interface thread.

`send_command` only queues the command; it runs later on the user-interface thread. It is safe to call from `on_event`.

## 6. Strings and errors

- All strings are UTF-8 and NUL-terminated. Pointers you receive (event text, locations) are valid only during the call; copy what you keep.
- **String output convention.** A function that fills `(buf, len)` writes at most `len - 1` bytes plus a NUL, cut at a UTF-8 character boundary, and returns the full length in bytes (without the NUL). A result `>= len` means the text was cut; call again with a buffer of `result + 1` bytes. Host functions follow it, and so must your `take_title` and `get_field`.
- Error codes are negative: `NULAMP_ERR_FAILED`, `NULAMP_ERR_NOT_FOUND`, `NULAMP_ERR_UNSUPPORTED`, `NULAMP_ERR_INVALID`, `NULAMP_ERR_IO`. `NULAMP_OK` is 0.
- **Nothing may unwind across the interface.** Catch C++ exceptions in every exported function. In Rust, wrap every `extern "C"` body in `std::panic::catch_unwind` (a panic that reaches an `extern "C"` boundary aborts the player). The host does the same for its functions.

## 7. Module kinds

### 7.1 Input

- `extensions`: pairs of strings ending with `NULL`, for example `"mp3;mp2"`, `"MPEG Audio Files"`, `NULL`. Used by file dialogs and folder scanning.
- `claims(user, location)` (optional): return 1 to take a location that extension matching would miss (URL schemes, content sniffing). It is consulted before extension matching (07 §7).
- `open(user, location, &error)` returns a decoder handle, or `NULL` with `error` set (`NULAMP_ERR_NOT_FOUND` for a missing file, `NULAMP_ERR_UNSUPPORTED` when the file is not yours).
- `info(handle, &info)`: sample rate, channels (1–8), source bits, length in ms (−1 unknown), seekable, lossless, and a format description. The host calls it after `open` and **before every `read`**, so a stream can change its format between reads.
- `read(handle, out, max_frames)`: write up to `max_frames` interleaved float frames in the layout `info` just reported. Return the frame count, 0 at the end, `NULAMP_PENDING` when a stream has no data yet (the host polls again shortly and gives up after about 10 seconds), or an error code.
- `seek(handle, ms)` returns the position reached, or an error code.
- `kbps`, `in_sync`, `take_title` (optional): current bit rate (−1 unknown); 0 while a stream is buffering; a changed stream title, consumed only when it fit in the buffer.
- Metadata (optional, 08 §3.4): `get_field` returns a positive length when the field has a value, 0 when you understand the field but it is empty, and `NULAMP_ERR_UNSUPPORTED` for a field you do not understand. `set_field` queues a change (an empty value deletes the field); `write_tags` commits the queued changes.

The host pulls audio from the decoder; decoders contain no threading code and never talk to the output.

### 7.2 DSP

- `create(user)` makes an instance; `destroy` frees it.
- `configure(inst, sample_rate, channels)` is called before the first block and whenever the format changes. Return an error for a layout you cannot handle; the audio then passes through unchanged.
- `process(inst, in, frames, out, max_out_frames)` reads `frames` interleaved float frames from `in` and writes to `out` (the buffers never overlap; `max_out_frames` is at least `2 * frames`). Return the number of frames written. Time-stretching effects may return a different count (between half and twice the input is typical). A negative result makes the host pass the block through.
- `latency_frames` (optional): the delay your effect adds. `flush` (optional): drop internal state after a seek.

### 7.3 Visualization

- `spectrum_channels` and `waveform_channels` (0–2) say what data you want; `latency_ms` shifts the data to "now + latency"; `delay_ms` is the wanted time between frames.
- `start(user, width, height)` makes an instance; `stop` frees it.
- `render(inst, frame)` is called repeatedly. `NulampVisFrame` carries 2 × 576 unsigned 8-bit spectrum values (low to high frequency), 2 × 576 signed 8-bit waveform values, pointers to 576 float PCM frames and 256 float FFT magnitudes per channel, the sample rate and the number of rows that hold data (1 or 2). The data follow spec 06 §7.1 and `decisions.md` VIS10.
- The host owns the drawing surface: `pixels` is RGBA, 8 bits per channel in that byte order, `stride` bytes per row, `width` × `height`. Draw into it during `render`; the host shows it in its own window. The pixels are kept between frames (cleared when the size changes), and alpha is ignored. Every pointer in the frame is valid only during the call.
- Return `NULAMP_VIS_CONTINUE`, or `NULAMP_VIS_QUIT` to stop (an error code also stops).

### 7.4 General

`on_event(user, event)` receives player events on the user-interface thread:

| Event | `value` | `text` |
|---|---|---|
| `NULAMP_EVENT_TRACK_STARTED` | 0-based playlist position | Location |
| `NULAMP_EVENT_TRACK_STOPPED` | 1 the user stopped, 0 the playlist ended | |
| `NULAMP_EVENT_TITLE_CHANGED` | | New title |
| `NULAMP_EVENT_VOLUME_CHANGED` | 0–255 | |
| `NULAMP_EVENT_EQ_CHANGED` | | |
| `NULAMP_EVENT_PLAYLIST_CHANGED` | Playlist length | |
| `NULAMP_EVENT_SKIN_CHANGED` | | Skin name or path |
| `NULAMP_EVENT_PAUSED`, `NULAMP_EVENT_UNPAUSED` | | |
| `NULAMP_EVENT_INFO_CHANGED` | Bit rate in kbps, −1 unknown | |

`text` is never `NULL` (empty when unused). New event numbers may appear in later minor versions; ignore the ones you do not know.

## 8. The host API

`NulampHost` is passed to the entry point and to every `init`, and stays valid until the file is unloaded. Call its functions with the host pointer as first argument: `host->log(host, id, NULAMP_LOG_INFO, "hello")`.

| Function | Use |
|---|---|
| `send_command(host, command, arg)` | Queue a player command (below). `NULAMP_ERR_UNSUPPORTED` for an unknown number |
| `get_status(host, &status)` | State (0 stopped, 1 playing, 3 paused), volume, balance, position, length, bit rate, sample rate, channels, playlist position and length, shuffle, repeat, manual advance, stop after current |
| `get_title(host, buf, len)` | Current title (string output convention) |
| `log(host, module_id, level, msg)` | A line in the player's log, tagged with the module name |
| `settings_get(host, module_id, key, buf, len)` | A value from the module's own settings section, or `NULAMP_ERR_NOT_FOUND` |
| `settings_set(host, module_id, key, value)` | Store a value; `NULL` deletes the key |
| `post_to_ui(host, module_id, cb, data)` | Run `cb(data)` later on the user-interface thread. The file stays loaded until it has run |

Settings live in the player's settings file (11 §1), in the section `plugin:<file name without extension>:<module name>` (characters other than letters, digits, `-`, `_` and `.` become `_`).

### 8.1 Commands

Numbers below 50000 are the numbered commands of spec 11 §6.3 and ignore `arg`; the host also accepts the other §6.3 numbers (dialogs, windows, toggles) and handles them like the menu items. Numbers from 50000 take an argument.

| Number | Constant | `arg` |
|---|---|---|
| 40044 / 40045 / 40046 / 40047 / 40048 | `PREVIOUS` / `PLAY` / `PAUSE` / `STOP` / `NEXT` | |
| 40058 / 40059 | `VOLUME_UP` / `VOLUME_DOWN` (±4) | |
| 40060, 40148 / 40061, 40144 | `FORWARD_5S` / `REWIND_5S` | |
| 40147 | `STOP_WITH_FADE` | |
| 40154 / 40158 | `START_OF_LIST` / `END_OF_LIST` | |
| 40157 | `STOP_AFTER_CURRENT` (toggle) | |
| 40195 / 40197 | `FORWARD_10_TRACKS` / `BACK_10_TRACKS` | |
| 40022 / 40023 / 40395 | `TOGGLE_REPEAT` / `TOGGLE_SHUFFLE` / `TOGGLE_MANUAL_ADVANCE` | |
| 50001 | `SEEK_TO` | Position in ms |
| 50002 | `SEEK_RELATIVE` | Signed offset in ms |
| 50003 | `SET_VOLUME` | 0–255 (clamped) |
| 50004 | `VOLUME_BY` | Signed delta |
| 50005 | `SET_BALANCE` | −127..127 (clamped) |
| 50006 / 50007 / 50008 | `SET_SHUFFLE` / `SET_REPEAT` / `SET_MANUAL_ADVANCE` | 0 off, else on |
| 50009 | `PLAY_INDEX` | 0-based playlist position (negative is rejected) |
| 50010 | `JUMP_ENTRIES` | Signed number of entries |

Every constant has the `NULAMP_CMD_` prefix.

## 9. Installing

The player looks in a system plugin folder (read-only) and a user plugin folder (11 §1.1), not in subfolders. Only files with the platform's library extension are loaded, in file-name order; a file in the user folder replaces a system file with the same name. Starting the player in safe mode (`/SAFE`) loads no plugins. A file that fails to load, has no `nulamp_plugin_entry`, or has a different major ABI version is skipped and reported in the log.

## 10. Writing a plugin in Rust

Depend on the API crate without its host part and use only the `sys` module, which mirrors the header:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
nulamp-plugin-api = { path = "…/crates/nulamp-plugin-api", default-features = false }
```

All function pointers are `Option`s (C code may store `NULL`); fill the required ones with `Some`. `sys::StaticPtrs` makes the module and extension lists storable in `static`s, and `sys::write_str_out` implements the string output convention. Export the entry point with `#[unsafe(no_mangle)] pub unsafe extern "C" fn nulamp_plugin_entry(...)`.

`crates/nulamp-plugin-example` is a complete plugin: three DSP effects (gain from a setting, vocal remover, half speed) and a general module that logs events. Its tests (`crates/nulamp-plugin-api/tests/`) load it as a shared library; another test builds a small C plugin against the header.

## 11. A minimal C plugin

```c
#include "nulamp_plugin.h"

static int32_t init(void *user, const NulampHost *host, uint64_t id) {
    (void)user; (void)host; (void)id;
    return NULAMP_OK;
}
static void quit(void *user) { (void)user; }

static uint32_t g_channels;
static void *create(void *user) { (void)user; return &g_channels; }
static void destroy(void *inst) { (void)inst; }
static int32_t configure(void *inst, uint32_t rate, uint32_t channels) {
    (void)rate;
    *(uint32_t *)inst = channels;
    return NULAMP_OK;
}
static int64_t process(void *inst, const float *in, uint32_t frames, float *out,
                       uint32_t max_out) {
    (void)max_out;
    uint32_t n = frames * *(uint32_t *)inst;
    for (uint32_t i = 0; i < n; i++) out[i] = in[i] * 0.5f;
    return frames;
}

static const NulampDspApi api = {
    NULAMP_ABI_VERSION, sizeof(NulampDspApi), create, destroy, configure, process, NULL, NULL,
};
static const NulampModule half_volume = {
    NULAMP_ABI_VERSION, sizeof(NulampModule), NULAMP_KIND_DSP, 0, &api,
    "Half volume", "1.0", "Halves the volume", NULL, init, quit, NULL, NULL,
};
static const NulampModule *const modules[] = { &half_volume };
static const NulampPluginDesc desc = {
    NULAMP_ABI_VERSION, sizeof(NulampPluginDesc), "Half volume", "1.0", "", 1, 0, modules,
};

NULAMP_EXPORT const NulampPluginDesc *nulamp_plugin_entry(const NulampHost *host) {
    return NULAMP_ABI_MAJOR_OF(host->abi_version) == NULAMP_ABI_MAJOR ? &desc : NULL;
}
```

Build it with `cc -shared -fPIC -I include -o half_volume.so half_volume.c` (macOS: `-dynamiclib`, `.dylib`) and copy the file into the user plugin folder. This instance is a single static, which is fine because the player uses one instance at a time; allocate per instance when the effect can appear twice in a chain.
