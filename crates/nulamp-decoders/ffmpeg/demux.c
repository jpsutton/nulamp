/* See demux.h. */

#include "demux.h"

#include <stdio.h>

#include <libavutil/error.h>
#include <libavutil/log.h>
#include <libavutil/mathematics.h>
#include <libavutil/mem.h>

struct NlDemux {
    AVFormatContext *fmt;
};

static void set_error(char *err, int len, const char *what, int code) {
    char text[AV_ERROR_MAX_STRING_SIZE] = {0};
    if (!err || len <= 0)
        return;
    if (code < 0) {
        av_strerror(code, text, sizeof text);
        snprintf(err, (size_t)len, "%s: %s", what, text);
    } else {
        snprintf(err, (size_t)len, "%s", what);
    }
}

NlDemux *nld_open(const char *path, const NlIo *io, char *err, int err_len) {
    NlDemux *d;
    int r;

    av_log_set_level(AV_LOG_QUIET);
    d = av_mallocz(sizeof *d);
    if (!d) {
        set_error(err, err_len, "out of memory", 0);
        return NULL;
    }
    r = nl_open_input(&d->fmt, path, io);
    if (r < 0) {
        set_error(err, err_len, "cannot open", r);
        av_free(d);
        return NULL;
    }
    r = avformat_find_stream_info(d->fmt, NULL);
    if (r < 0) {
        set_error(err, err_len, "no stream information", r);
        nld_close(d);
        return NULL;
    }
    return d;
}

void nld_close(NlDemux *d) {
    if (!d)
        return;
    nl_close_input(&d->fmt);
    av_free(d);
}

AVFormatContext *nld_fmt(NlDemux *d) { return d->fmt; }

int nld_read(NlDemux *d, void **pkt, int *stream, int *key, int *size) {
    AVPacket *p = av_packet_alloc();
    int r;
    *pkt = NULL;
    if (!p)
        return AVERROR(ENOMEM);
    r = av_read_frame(d->fmt, p);
    if (r < 0) {
        av_packet_free(&p);
        if (r == AVERROR_EOF || (d->fmt->pb && avio_feof(d->fmt->pb)))
            return 0;
        return r;
    }
    *pkt = p;
    *stream = p->stream_index;
    *key = (p->flags & AV_PKT_FLAG_KEY) != 0;
    *size = p->size;
    return 1;
}

void nld_packet_free(void *pkt) {
    AVPacket *p = pkt;
    av_packet_free(&p);
}

void nld_packet_move(AVPacket *dst, void *src) {
    AVPacket *p = src;
    av_packet_unref(dst);
    av_packet_move_ref(dst, p);
    av_packet_free(&p);
}

int nld_video_stream(NlDemux *d) {
    unsigned i;
    for (i = 0; i < d->fmt->nb_streams; i++) {
        AVStream *st = d->fmt->streams[i];
        if (st->codecpar->codec_type == AVMEDIA_TYPE_VIDEO &&
            !(st->disposition & AV_DISPOSITION_ATTACHED_PIC) && st->codecpar->width > 0)
            return (int)i;
    }
    return -1;
}

void nld_route(NlDemux *d, int audio, int video) {
    unsigned i;
    for (i = 0; i < d->fmt->nb_streams; i++)
        d->fmt->streams[i]->discard =
            ((int)i == audio || (int)i == video) ? AVDISCARD_DEFAULT : AVDISCARD_ALL;
}

int nld_seek(NlDemux *d, int stream, int64_t ms) {
    AVStream *st, *zero_st;
    int64_t zero = 0, ts;
    int audio, r;

    if (stream < 0 || stream >= (int)d->fmt->nb_streams)
        return AVERROR(EINVAL);
    st = d->fmt->streams[stream];
    audio = av_find_best_stream(d->fmt, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0);
    zero_st = audio >= 0 ? d->fmt->streams[audio] : st;
    if (zero_st->start_time != AV_NOPTS_VALUE)
        zero = av_rescale_q(zero_st->start_time, zero_st->time_base, AV_TIME_BASE_Q);
    ts = av_rescale_q(zero + ms * 1000, AV_TIME_BASE_Q, st->time_base);
    nl_clear_error(d->fmt);
    r = avformat_seek_file(d->fmt, stream, INT64_MIN, ts, ts, 0);
    if (r < 0)
        r = av_seek_frame(d->fmt, stream, ts, AVSEEK_FLAG_BACKWARD);
    return r < 0 ? r : 0;
}
