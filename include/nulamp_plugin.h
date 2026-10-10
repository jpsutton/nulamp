/*
 * nulamp_plugin.h - native plugin ABI of nulamp (spec 08 §1-§7).
 *
 * One plugin file (a shared library: .so, .dylib or .dll) exports one function,
 * nulamp_plugin_entry, that returns a description of the file and of any number of modules
 * inside it. Module kinds: input (decoder), DSP (effect), visualization and general.
 *
 * Conventions (see docs/plugins.md for the full guide):
 *
 * - Every struct starts with abi_version and struct_size. A struct is only ever extended by
 *   appending fields, and only in a new minor version. A reader must not touch fields that do
 *   not fit in struct_size.
 * - Structs the plugin provides (descriptions, modules, APIs) carry the values the plugin was
 *   built with: abi_version = NULAMP_ABI_VERSION, struct_size = sizeof(the struct).
 * - Structs passed as output (NulampStatus, NulampStreamInfo) are prepared by the caller: it
 *   sets abi_version and struct_size, and the callee writes only fields that fit.
 * - The host rejects a plugin whose major version differs from its own. A plugin should check
 *   host->struct_size before calling a host function added in a later minor version.
 * - Strings are UTF-8 and NUL-terminated. Function pointers marked "optional" may be NULL.
 * - String output: a function that writes into (buf, len) writes at most len - 1 bytes plus a
 *   NUL, cut at a UTF-8 character boundary, and returns the full length in bytes without the
 *   NUL. A result >= len means the text was truncated; call again with a larger buffer.
 * - No function may unwind (C++ exceptions, Rust panics) across this interface.
 */
#ifndef NULAMP_PLUGIN_H
#define NULAMP_PLUGIN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Versioning ---------------------------------------------------------------------- */

#define NULAMP_ABI_MAJOR 1
#define NULAMP_ABI_MINOR 0
#define NULAMP_ABI_VERSION ((uint32_t)((NULAMP_ABI_MAJOR << 16) | NULAMP_ABI_MINOR))
#define NULAMP_ABI_MAJOR_OF(v) ((uint32_t)(v) >> 16)
#define NULAMP_ABI_MINOR_OF(v) ((uint32_t)(v) & 0xFFFFu)

/* Name of the exported entry point. */
#define NULAMP_PLUGIN_ENTRY_NAME "nulamp_plugin_entry"

#if defined(_WIN32)
#define NULAMP_EXPORT __declspec(dllexport)
#else
#define NULAMP_EXPORT __attribute__((visibility("default")))
#endif

/* ---- Result codes -------------------------------------------------------------------- */

#define NULAMP_OK 0
#define NULAMP_ERR_FAILED (-1)
#define NULAMP_ERR_NOT_FOUND (-2)
#define NULAMP_ERR_UNSUPPORTED (-3)
#define NULAMP_ERR_INVALID (-4)
#define NULAMP_ERR_IO (-5)
/* Returned by NulampInputApi.read when a stream has no data yet; the host calls again. */
#define NULAMP_PENDING (-100)

/* ---- Module kinds -------------------------------------------------------------------- */

#define NULAMP_KIND_INPUT 1
#define NULAMP_KIND_DSP 2
#define NULAMP_KIND_VIS 3
#define NULAMP_KIND_GENERAL 4

/* ---- Log levels ---------------------------------------------------------------------- */

#define NULAMP_LOG_ERROR 1
#define NULAMP_LOG_WARN 2
#define NULAMP_LOG_INFO 3
#define NULAMP_LOG_DEBUG 4
#define NULAMP_LOG_TRACE 5

/* ---- Play states (NulampStatus.state, same values as remote control, 07 §1) ----------- */

#define NULAMP_STATE_STOPPED 0
#define NULAMP_STATE_PLAYING 1
#define NULAMP_STATE_PAUSED 3

/* ---- Commands (NulampHost.send_command) ---------------------------------------------- */
/*
 * Ids below 50000 are the numbered commands of spec 11 §6.3; the arg is ignored. The host also
 * accepts the other §6.3 ids (dialogs, windows) and handles them like the menu items.
 * Ids from 50000 up are nulamp commands that take an argument.
 */
#define NULAMP_CMD_TOGGLE_REPEAT 40022
#define NULAMP_CMD_TOGGLE_SHUFFLE 40023
#define NULAMP_CMD_PREVIOUS 40044
#define NULAMP_CMD_PLAY 40045
#define NULAMP_CMD_PAUSE 40046
#define NULAMP_CMD_STOP 40047
#define NULAMP_CMD_NEXT 40048
#define NULAMP_CMD_VOLUME_UP 40058
#define NULAMP_CMD_VOLUME_DOWN 40059
#define NULAMP_CMD_FORWARD_5S 40060
#define NULAMP_CMD_REWIND_5S 40061
#define NULAMP_CMD_REWIND_5S_ALT 40144
#define NULAMP_CMD_STOP_WITH_FADE 40147
#define NULAMP_CMD_FORWARD_5S_ALT 40148
#define NULAMP_CMD_START_OF_LIST 40154
#define NULAMP_CMD_STOP_AFTER_CURRENT 40157
#define NULAMP_CMD_END_OF_LIST 40158
#define NULAMP_CMD_FORWARD_10_TRACKS 40195
#define NULAMP_CMD_BACK_10_TRACKS 40197
#define NULAMP_CMD_TOGGLE_MANUAL_ADVANCE 40395

#define NULAMP_CMD_SEEK_TO 50001            /* arg: position in ms */
#define NULAMP_CMD_SEEK_RELATIVE 50002      /* arg: signed offset in ms */
#define NULAMP_CMD_SET_VOLUME 50003         /* arg: 0..255 */
#define NULAMP_CMD_VOLUME_BY 50004          /* arg: signed delta */
#define NULAMP_CMD_SET_BALANCE 50005        /* arg: -127..127 */
#define NULAMP_CMD_SET_SHUFFLE 50006        /* arg: 0 off, else on */
#define NULAMP_CMD_SET_REPEAT 50007         /* arg: 0 off, else on */
#define NULAMP_CMD_SET_MANUAL_ADVANCE 50008 /* arg: 0 off, else on */
#define NULAMP_CMD_PLAY_INDEX 50009         /* arg: 0-based playlist position */
#define NULAMP_CMD_JUMP_ENTRIES 50010       /* arg: signed number of entries */

/* ---- Events (NulampGeneralApi.on_event) ---------------------------------------------- */

#define NULAMP_EVENT_TRACK_STARTED 1    /* text: location; value: 0-based playlist position */
#define NULAMP_EVENT_TRACK_STOPPED 2    /* value: 1 the user stopped, 0 the playlist ended */
#define NULAMP_EVENT_TITLE_CHANGED 3    /* text: new title */
#define NULAMP_EVENT_VOLUME_CHANGED 4   /* value: 0..255 */
#define NULAMP_EVENT_EQ_CHANGED 5
#define NULAMP_EVENT_PLAYLIST_CHANGED 6 /* value: playlist length */
#define NULAMP_EVENT_SKIN_CHANGED 7     /* text: skin name or path */
#define NULAMP_EVENT_PAUSED 8
#define NULAMP_EVENT_UNPAUSED 9
#define NULAMP_EVENT_INFO_CHANGED 10    /* value: bit rate in kbps, -1 unknown */

/* ---- Visualization ------------------------------------------------------------------- */

#define NULAMP_VIS_SAMPLES 576  /* values per channel of spectrum, waveform and PCM data */
#define NULAMP_VIS_FFT_BINS 256 /* float FFT magnitudes per channel */
#define NULAMP_VIS_CONTINUE 0   /* render result: keep running */
#define NULAMP_VIS_QUIT 1       /* render result: stop this visualization */

/* ---- Host API ------------------------------------------------------------------------ */

typedef struct NulampHost NulampHost;

/* Player state for NulampHost.get_status. */
typedef struct NulampStatus {
    uint32_t abi_version;
    uint32_t struct_size;
    int32_t state;   /* NULAMP_STATE_* */
    int32_t volume;  /* 0..255 */
    int32_t balance; /* -127..127 */
    int32_t kbps;    /* -1 unknown */
    int64_t position_ms;
    int64_t length_ms;     /* -1 unknown (streams) */
    uint32_t sample_rate;  /* 0 unknown */
    uint32_t channels;     /* 0 unknown */
    uint32_t playlist_position; /* 0-based */
    uint32_t playlist_length;
    uint8_t seekable;
    uint8_t in_sync;
    uint8_t shuffle;
    uint8_t repeat;
    uint8_t manual_advance;
    uint8_t stop_after_current;
    uint8_t reserved[2];
} NulampStatus;

typedef void (*NulampUiCallback)(void *data);

/*
 * Services the host offers. Passed to nulamp_plugin_entry and to every module's init; valid
 * until the plugin file is unloaded. Every function is thread-safe and may be called from any
 * thread, including from inside a call the host made into the plugin. module_id is the id
 * passed to the module's init (0 when the call is not about one module).
 */
struct NulampHost {
    uint32_t abi_version;
    uint32_t struct_size;
    void *ctx; /* host private */

    /* Queues a command (NULAMP_CMD_*). Returns NULAMP_OK, or NULAMP_ERR_UNSUPPORTED for an
     * unknown id. The command runs later on the user-interface thread. */
    int32_t (*send_command)(const NulampHost *host, uint32_t command, int64_t arg);
    /* Fills the player state. Prepare out->abi_version and out->struct_size first. */
    int32_t (*get_status)(const NulampHost *host, NulampStatus *out);
    /* Current title (string output convention). */
    int32_t (*get_title)(const NulampHost *host, char *buf, uint32_t len);
    /* Writes one line to the host log. level: NULAMP_LOG_*. */
    void (*log)(const NulampHost *host, uint64_t module_id, int32_t level, const char *msg);
    /* Reads a setting from the module's own section (string output convention), or returns
     * NULAMP_ERR_NOT_FOUND. */
    int32_t (*settings_get)(const NulampHost *host, uint64_t module_id, const char *key,
                            char *buf, uint32_t len);
    /* Stores a setting in the module's own section; value NULL deletes the key. */
    int32_t (*settings_set)(const NulampHost *host, uint64_t module_id, const char *key,
                            const char *value);
    /* Runs cb(data) later on the user-interface thread. data must be safe to use there. */
    int32_t (*post_to_ui)(const NulampHost *host, uint64_t module_id, NulampUiCallback cb,
                          void *data);
};

/* ---- Common module record ------------------------------------------------------------ */

/*
 * One module. `api` points to the kind's record: NulampInputApi, NulampDspApi, NulampVisApi
 * or NulampGeneralApi. `user` is passed back to every module-level function.
 *
 * Threading: init, quit, configure and about run on the user-interface thread. The host calls
 * init once after loading (a module whose init fails is not used) and quit once before
 * unloading, only when init succeeded.
 */
typedef struct NulampModule {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t kind;  /* NULAMP_KIND_* */
    uint32_t flags; /* reserved, 0 */
    const void *api;
    const char *name;
    const char *version;
    const char *description;
    void *user;
    /* Returns NULAMP_OK, or an error code to disable the module. */
    int32_t (*init)(void *user, const NulampHost *host, uint64_t module_id);
    void (*quit)(void *user);
    void (*configure)(void *user); /* optional: show a settings dialog */
    void (*about)(void *user);     /* optional: show an about box */
} NulampModule;

/* What nulamp_plugin_entry returns. Must stay valid until the file is unloaded. */
typedef struct NulampPluginDesc {
    uint32_t abi_version;
    uint32_t struct_size;
    const char *name;
    const char *version;
    const char *description;
    uint32_t module_count;
    uint32_t reserved;
    const NulampModule *const *modules;
} NulampPluginDesc;

/* ---- Input (decoder) modules --------------------------------------------------------- */

typedef struct NulampStreamInfo {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t sample_rate;
    uint16_t channels; /* 1..8 */
    uint16_t bits;     /* bits per sample of the source, 0 when not meaningful */
    int64_t length_ms; /* -1 unknown */
    uint8_t seekable;
    uint8_t lossless;
    uint8_t reserved[6];
    char codec[64]; /* format description, for example "MPEG-1 Layer 3" */
} NulampStreamInfo;

/*
 * Threading: claims, open, get_field, set_field and write_tags may be called from any thread,
 * concurrently. A decoder handle is used by one thread at a time (it may move between
 * threads).
 */
typedef struct NulampInputApi {
    uint32_t abi_version;
    uint32_t struct_size;
    /* Pairs of strings, then NULL: "mp3;mp2", "MPEG Audio Files", "ogg", "Ogg Vorbis", NULL. */
    const char *const *extensions;
    /* optional: 1 when the module wants this location (URL scheme, content check), else 0.
     * Consulted before extension matching. */
    int32_t (*claims)(void *user, const char *location);
    /* Opens a decoder. On failure returns NULL and sets *error (NULAMP_ERR_NOT_FOUND, ...). */
    void *(*open)(void *user, const char *location, int32_t *error);
    /* Fills the stream format. The host calls it after open and before every read. */
    int32_t (*info)(void *dec, NulampStreamInfo *out);
    /* Decodes up to max_frames interleaved float frames, in the layout the last info call
     * reported. Returns frames written, 0 at the end, NULAMP_PENDING when a stream has no data
     * yet, or another negative error code. */
    int64_t (*read)(void *dec, float *out, uint32_t max_frames);
    /* Seeks; returns the position reached in ms or a negative error code. */
    int64_t (*seek)(void *dec, uint64_t ms);
    int32_t (*kbps)(void *dec);    /* optional: current bit rate, -1 unknown */
    int32_t (*in_sync)(void *dec); /* optional: 0 while a stream is buffering, else 1 */
    /* optional: a stream title changed since the last call. Returns 0 when there is none,
     * else the string output convention; the title is consumed only when it fit. */
    int32_t (*take_title)(void *dec, char *buf, uint32_t len);
    void (*close)(void *dec);
    /* optional metadata (10 §7). get_field: > 0 found (string output convention), 0 the field
     * is understood but empty, NULAMP_ERR_UNSUPPORTED the field is not understood. */
    int32_t (*get_field)(void *user, const char *location, const char *name, char *buf,
                         uint32_t len);
    /* optional: queues a change; an empty value deletes the field. */
    int32_t (*set_field)(void *user, const char *location, const char *name, const char *value);
    /* optional: commits the queued changes to the file. */
    int32_t (*write_tags)(void *user, const char *location);
} NulampInputApi;

/* ---- DSP (effect) modules ------------------------------------------------------------ */

/*
 * Threading: an instance is used by one thread at a time (the audio thread).
 * Samples are 32-bit float, interleaved.
 */
typedef struct NulampDspApi {
    uint32_t abi_version;
    uint32_t struct_size;
    void *(*create)(void *user); /* NULL on failure */
    void (*destroy)(void *inst);
    /* Called before the first block and whenever the format changes. */
    int32_t (*configure)(void *inst, uint32_t sample_rate, uint32_t channels);
    /* Processes `frames` frames from `in` into `out` (they never overlap). max_out_frames is
     * at least 2 * frames. Returns frames written (between frames / 2 and 2 * frames is
     * typical), or a negative error code (the host then passes the input through). */
    int64_t (*process)(void *inst, const float *in, uint32_t frames, float *out,
                       uint32_t max_out_frames);
    uint32_t (*latency_frames)(void *inst); /* optional */
    void (*flush)(void *inst);              /* optional: drop state after a seek */
} NulampDspApi;

/* ---- Visualization modules ----------------------------------------------------------- */

/*
 * One frame of data and the surface to draw on. The host owns every buffer; pointers are
 * valid only during the render call. pixels: RGBA, 8 bits per channel in that byte order,
 * `stride` bytes per row. The host keeps the pixels between frames and ignores alpha.
 */
typedef struct NulampVisFrame {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t sample_rate;
    uint32_t channels; /* rows that hold data: 1 or 2 */
    uint8_t spectrum[2][NULAMP_VIS_SAMPLES]; /* low to high frequency */
    int8_t waveform[2][NULAMP_VIS_SAMPLES];
    const float *pcm[2]; /* pcm_frames floats per row, in [-1, 1) */
    const float *fft[2]; /* fft_bins magnitudes per row */
    uint32_t pcm_frames;
    uint32_t fft_bins;
    uint8_t *pixels;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint32_t reserved;
} NulampVisFrame;

/*
 * Threading: start, render and stop of one instance run on one dedicated thread.
 */
typedef struct NulampVisApi {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t spectrum_channels; /* 0..2 */
    uint32_t waveform_channels; /* 0..2 */
    uint32_t latency_ms;        /* the host gives data for "now + latency" */
    uint32_t delay_ms;          /* wanted time between frames */
    void *(*start)(void *user, uint32_t width, uint32_t height); /* NULL on failure */
    /* NULAMP_VIS_CONTINUE, NULAMP_VIS_QUIT, or a negative error code (also stops). The
     * surface size may change between calls. */
    int32_t (*render)(void *inst, const NulampVisFrame *frame);
    void (*stop)(void *inst);
} NulampVisApi;

/* ---- General modules ----------------------------------------------------------------- */

typedef struct NulampEvent {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t kind; /* NULAMP_EVENT_* */
    uint32_t reserved;
    int64_t value;
    const char *text; /* never NULL; "" when the event has no text; valid during the call */
} NulampEvent;

/*
 * Threading: on_event runs on the user-interface thread. To control the player, use the
 * NulampHost passed to init.
 */
typedef struct NulampGeneralApi {
    uint32_t abi_version;
    uint32_t struct_size;
    void (*on_event)(void *user, const NulampEvent *event);
} NulampGeneralApi;

/* ---- Entry point --------------------------------------------------------------------- */

/*
 * Returns the plugin description, or NULL to refuse loading (for example when
 * NULAMP_ABI_MAJOR_OF(host->abi_version) differs). Called once, on the user-interface thread.
 */
typedef const NulampPluginDesc *(*NulampPluginEntryFn)(const NulampHost *host);

NULAMP_EXPORT const NulampPluginDesc *nulamp_plugin_entry(const NulampHost *host);

#ifdef __cplusplus
}
#endif

#endif /* NULAMP_PLUGIN_H */
