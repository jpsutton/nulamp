/*
 * Opening inputs for shim.c and video.c: a local file through the "file" protocol, or any
 * byte source the Rust side reads (a media server's file over nulamp's own HTTP client,
 * crates/nulamp-decoders/src/ffmpeg_io.rs). FFmpeg's own network protocols are never used.
 */

#ifndef NULAMP_FFMPEG_IO_H
#define NULAMP_FFMPEG_IO_H

#include <stdint.h>

#include <libavformat/avformat.h>

/*
 * A byte source. Must match `NlIo` in ffmpeg_io.rs. `read` returns the bytes read (> 0), 0 at
 * the end or a negative value on error. `seek` takes SEEK_SET (0), SEEK_CUR (1), SEEK_END (2)
 * or AVSEEK_SIZE and returns the new position (the size for AVSEEK_SIZE) or a negative value;
 * NULL when the source cannot seek.
 */
typedef struct NlIo {
    void *opaque;
    int (*read)(void *opaque, uint8_t *buf, int size);
    int64_t (*seek)(void *opaque, int64_t offset, int whence);
} NlIo;

/*
 * Opens `path` (UTF-8) with the "file" protocol only, or, when `io` is not NULL, reads through
 * `io` (`path` is then only a name). Returns 0 or a negative error; `*fmt` is NULL on failure.
 */
int nl_open_input(AVFormatContext **fmt, const char *path, const NlIo *io);

/* Closes an input from nl_open_input, with its byte-source context. */
void nl_close_input(AVFormatContext **fmt);

/* Clears a read error of the byte source so reading can go on after a seek. */
void nl_clear_error(AVFormatContext *fmt);

#endif
