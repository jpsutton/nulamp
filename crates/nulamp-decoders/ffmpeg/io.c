/* See io.h. */

#include "io.h"

#include <libavutil/dict.h>
#include <libavutil/error.h>
#include <libavutil/mem.h>

/* Size of the buffer between the byte source and the demuxer. */
#define IO_BUFFER 65536

static int io_read(void *opaque, uint8_t *buf, int size) {
    const NlIo *io = opaque;
    int n = io->read(io->opaque, buf, size);
    if (n == 0)
        return AVERROR_EOF;
    return n < 0 ? AVERROR(EIO) : n;
}

static int64_t io_seek(void *opaque, int64_t offset, int whence) {
    const NlIo *io = opaque;
    int64_t r;
    whence &= ~AVSEEK_FORCE;
    r = io->seek(io->opaque, offset, whence);
    return r < 0 ? AVERROR(EIO) : r;
}

static void free_io(AVIOContext **pb) {
    if (!*pb)
        return;
    av_freep(&(*pb)->opaque);
    av_freep(&(*pb)->buffer);
    avio_context_free(pb);
}

int nl_open_input(AVFormatContext **fmt, const char *path, const NlIo *io) {
    AVDictionary *opts = NULL;
    AVIOContext *pb = NULL;
    NlIo *copy;
    uint8_t *buffer;
    int r;

    *fmt = NULL;
    if (!io) {
        av_dict_set(&opts, "protocol_whitelist", "file", 0);
        r = avformat_open_input(fmt, path, NULL, &opts);
        av_dict_free(&opts);
        return r;
    }
    copy = av_malloc(sizeof *copy);
    buffer = av_malloc(IO_BUFFER);
    if (copy && buffer) {
        *copy = *io;
        pb = avio_alloc_context(buffer, IO_BUFFER, 0, copy, io_read, NULL,
                                io->seek ? io_seek : NULL);
    }
    *fmt = avformat_alloc_context();
    if (!pb || !*fmt) {
        if (pb) {
            free_io(&pb);
        } else {
            av_free(copy);
            av_free(buffer);
        }
        avformat_free_context(*fmt);
        *fmt = NULL;
        return AVERROR(ENOMEM);
    }
    (*fmt)->pb = pb;
    (*fmt)->flags |= AVFMT_FLAG_CUSTOM_IO;
    /* Nothing the file names (playlists, references) may be opened. */
    av_dict_set(&opts, "protocol_whitelist", "", 0);
    r = avformat_open_input(fmt, path, NULL, &opts);
    av_dict_free(&opts);
    if (r < 0)
        free_io(&pb);
    return r;
}

void nl_close_input(AVFormatContext **fmt) {
    AVIOContext *pb = NULL;
    if (!*fmt)
        return;
    if ((*fmt)->flags & AVFMT_FLAG_CUSTOM_IO)
        pb = (*fmt)->pb;
    avformat_close_input(fmt);
    free_io(&pb);
}

void nl_clear_error(AVFormatContext *fmt) {
    if (fmt && fmt->pb && (fmt->flags & AVFMT_FLAG_CUSTOM_IO)) {
        fmt->pb->error = 0;
        fmt->pb->eof_reached = 0;
    }
}
