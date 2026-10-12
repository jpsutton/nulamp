/*
 * Video side of nulamp's FFmpeg layer (crates/nulamp-decoders/src/video.rs, 09 §8).
 *
 * One handle opens a local file with libavformat, decodes one of its video streams with
 * libavcodec and converts frames to native-endian 0x00RRGGBB pixels with libswscale. Audio is
 * decoded separately (shim.c); the caller paces the frames against the audio clock. Input is a
 * local file (the "file" protocol only) or a byte source read through callbacks (io.h). A
 * handle is used by one thread at a time.
 *
 * Times are milliseconds from the start of the file's audio (or, without audio, its video), the
 * same zero the audio decoder counts from.
 */

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/dict.h>
#include <libavutil/error.h>
#include <libavutil/log.h>
#include <libavutil/mathematics.h>
#include <libavutil/mem.h>
#include <libavutil/pixfmt.h>
#include <libswscale/swscale.h>

#include "io.h"

/* Must match `NlfvInfo` in video.rs. */
typedef struct NlfvInfo {
    int32_t width;
    int32_t height;
    /* Display aspect ratio (width / height of the picture as shown), 0/0 when unknown. */
    int32_t dar_num;
    int32_t dar_den;
    /* Frame rate, 0/0 when unknown. */
    int32_t fps_num;
    int32_t fps_den;
    /* Video streams in the file and the one decoded (0-based among them). */
    int32_t streams;
    int32_t current;
    /* Audio streams in the file and the one the audio decoder picks by default (-1: none). */
    int32_t audio_streams;
    int32_t audio_best;
    /* Length in ms, -1 when unknown. */
    int64_t length_ms;
    char codec[128];
} NlfvInfo;

typedef struct Nlfv {
    AVFormatContext *fmt;
    AVCodecContext *dec;
    AVPacket *pkt;
    struct SwsContext *sws;
    /* Indexes of the video streams (attached pictures such as cover art are not video). */
    int *streams;
    int nstreams;
    /* Indexes of the audio streams, and the default one among them (-1: none). */
    int *audio;
    int naudio;
    int audio_best;
    int current;
    int stream;
    AVRational tb;
    /* Stream time of 0 ms, in AV_TIME_BASE units. */
    int64_t zero;
    int flushing;
    int eof;
} Nlfv;

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

void nlfv_close(Nlfv *h) {
    if (!h)
        return;
    sws_freeContext(h->sws);
    av_packet_free(&h->pkt);
    avcodec_free_context(&h->dec);
    nl_close_input(&h->fmt);
    av_free(h->streams);
    av_free(h->audio);
    av_free(h);
}

static void fill_info(Nlfv *h, NlfvInfo *info) {
    AVStream *st = h->fmt->streams[h->stream];
    AVCodecParameters *par = st->codecpar;
    const AVCodecDescriptor *desc = avcodec_descriptor_get(par->codec_id);
    AVRational sar = av_guess_sample_aspect_ratio(h->fmt, st, NULL);
    AVRational fps = av_guess_frame_rate(h->fmt, st, NULL);

    memset(info, 0, sizeof *info);
    info->width = par->width;
    info->height = par->height;
    if (sar.num <= 0 || sar.den <= 0)
        sar = (AVRational){1, 1};
    if (par->width > 0 && par->height > 0)
        av_reduce(&info->dar_num, &info->dar_den, (int64_t)par->width * sar.num,
                  (int64_t)par->height * sar.den, 1 << 20);
    if (fps.num > 0 && fps.den > 0) {
        info->fps_num = fps.num;
        info->fps_den = fps.den;
    }
    info->streams = h->nstreams;
    info->current = h->current;
    info->audio_streams = h->naudio;
    info->audio_best = h->audio_best;
    info->length_ms = -1;
    if (h->fmt->duration != AV_NOPTS_VALUE && h->fmt->duration > 0)
        info->length_ms = av_rescale(h->fmt->duration, 1000, AV_TIME_BASE);
    else if (st->duration != AV_NOPTS_VALUE && st->duration > 0)
        info->length_ms = av_rescale_q(st->duration, st->time_base, (AVRational){1, 1000});
    snprintf(info->codec, sizeof info->codec, "%s",
             desc && desc->long_name ? desc->long_name : avcodec_get_name(par->codec_id));
}

/* Opens the decoder for the `which`-th video stream. Returns 0 or a negative error. */
static int open_stream(Nlfv *h, int which) {
    const AVCodec *codec;
    AVStream *st;
    unsigned i;
    int r;

    avcodec_free_context(&h->dec);
    sws_freeContext(h->sws);
    h->sws = NULL;
    h->current = which;
    h->stream = h->streams[which];
    for (i = 0; i < h->fmt->nb_streams; i++)
        h->fmt->streams[i]->discard = (int)i == h->stream ? AVDISCARD_DEFAULT : AVDISCARD_ALL;
    st = h->fmt->streams[h->stream];
    h->tb = st->time_base;
    codec = avcodec_find_decoder(st->codecpar->codec_id);
    if (!codec)
        return AVERROR_DECODER_NOT_FOUND;
    h->dec = avcodec_alloc_context3(codec);
    if (!h->dec)
        return AVERROR(ENOMEM);
    r = avcodec_parameters_to_context(h->dec, st->codecpar);
    if (r < 0)
        return r;
    h->dec->pkt_timebase = st->time_base;
    /* Let libavcodec pick the number of threads. */
    h->dec->thread_count = 0;
    r = avcodec_open2(h->dec, codec, NULL);
    h->flushing = 0;
    h->eof = 0;
    return r;
}

/*
 * Opens `path` (UTF-8), or reads through `io` when it is not NULL (`path` is then only a
 * name), and its first video stream. Returns NULL and an error text when the file cannot be
 * opened or has no video.
 */
Nlfv *nlfv_open(const char *path, const NlIo *io, NlfvInfo *info, char *err, int err_len) {
    static int quiet = 0;
    unsigned i;
    int r, audio;
    Nlfv *h;

    if (!quiet) {
        av_log_set_level(AV_LOG_QUIET);
        quiet = 1;
    }
    h = av_mallocz(sizeof *h);
    if (!h) {
        set_error(err, err_len, "out of memory", 0);
        return NULL;
    }
    r = nl_open_input(&h->fmt, path, io);
    if (r < 0) {
        set_error(err, err_len, "cannot open", r);
        nlfv_close(h);
        return NULL;
    }
    r = avformat_find_stream_info(h->fmt, NULL);
    if (r < 0) {
        set_error(err, err_len, "no stream information", r);
        nlfv_close(h);
        return NULL;
    }
    h->streams = av_malloc_array(h->fmt->nb_streams ? h->fmt->nb_streams : 1, sizeof(int));
    h->audio = av_malloc_array(h->fmt->nb_streams ? h->fmt->nb_streams : 1, sizeof(int));
    h->pkt = av_packet_alloc();
    if (!h->streams || !h->audio || !h->pkt) {
        set_error(err, err_len, "out of memory", 0);
        nlfv_close(h);
        return NULL;
    }
    for (i = 0; i < h->fmt->nb_streams; i++) {
        AVStream *st = h->fmt->streams[i];
        if (st->codecpar->codec_type == AVMEDIA_TYPE_VIDEO &&
            !(st->disposition & AV_DISPOSITION_ATTACHED_PIC) && st->codecpar->width > 0)
            h->streams[h->nstreams++] = (int)i;
        if (st->codecpar->codec_type == AVMEDIA_TYPE_AUDIO)
            h->audio[h->naudio++] = (int)i;
    }
    if (h->nstreams == 0) {
        set_error(err, err_len, "no video stream", 0);
        nlfv_close(h);
        return NULL;
    }
    /* The audio decoder counts from the start of the best audio stream. */
    h->zero = 0;
    audio = av_find_best_stream(h->fmt, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0);
    h->audio_best = -1;
    for (i = 0; i < (unsigned)h->naudio; i++)
        if (h->audio[i] == audio)
            h->audio_best = (int)i;
    {
        AVStream *st = h->fmt->streams[audio >= 0 ? audio : h->streams[0]];
        if (st->start_time != AV_NOPTS_VALUE)
            h->zero = av_rescale_q(st->start_time, st->time_base, AV_TIME_BASE_Q);
    }
    /* The default stream first: the one the file marks as default, else the first. */
    r = 0;
    for (i = 0; i < (unsigned)h->nstreams; i++)
        if (h->fmt->streams[h->streams[i]]->disposition & AV_DISPOSITION_DEFAULT) {
            r = (int)i;
            break;
        }
    r = open_stream(h, r);
    if (r < 0) {
        set_error(err, err_len, r == AVERROR_DECODER_NOT_FOUND ? "no decoder for the video"
                                                              : "cannot open the decoder",
                  r == AVERROR_DECODER_NOT_FOUND ? 0 : r);
        nlfv_close(h);
        return NULL;
    }
    fill_info(h, info);
    return h;
}

/* Description of the `i`-th video stream for menus, for example "H.264 1920x1080 [eng]". */
int nlfv_stream_name(Nlfv *h, int i, char *out, int len) {
    AVStream *st;
    const AVDictionaryEntry *lang, *title;
    if (i < 0 || i >= h->nstreams || len <= 0)
        return -1;
    st = h->fmt->streams[h->streams[i]];
    lang = av_dict_get(st->metadata, "language", NULL, 0);
    title = av_dict_get(st->metadata, "title", NULL, 0);
    snprintf(out, (size_t)len, "%s %dx%d%s%s%s%s%s", avcodec_get_name(st->codecpar->codec_id),
             st->codecpar->width, st->codecpar->height, title ? " " : "",
             title ? title->value : "", lang ? " [" : "", lang ? lang->value : "",
             lang ? "]" : "");
    return 0;
}

/* Description of the `i`-th audio stream for menus, for example "aac stereo [eng]". */
int nlfv_audio_name(Nlfv *h, int i, char *out, int len) {
    AVStream *st;
    const AVDictionaryEntry *lang, *title;
    if (i < 0 || i >= h->naudio || len <= 0)
        return -1;
    st = h->fmt->streams[h->audio[i]];
    lang = av_dict_get(st->metadata, "language", NULL, 0);
    title = av_dict_get(st->metadata, "title", NULL, 0);
    snprintf(out, (size_t)len, "%s %dch%s%s%s%s%s", avcodec_get_name(st->codecpar->codec_id),
             st->codecpar->ch_layout.nb_channels, title ? " " : "", title ? title->value : "",
             lang ? " [" : "", lang ? lang->value : "", lang ? "]" : "");
    return 0;
}

/* Switches to the `which`-th video stream. The caller seeks afterwards. */
int nlfv_select(Nlfv *h, int which, NlfvInfo *info) {
    int r;
    if (which < 0 || which >= h->nstreams)
        return AVERROR(EINVAL);
    r = open_stream(h, which);
    if (r >= 0)
        fill_info(h, info);
    return r;
}

/* Goes to the key frame at or before `ms`; the next frames start there. */
int nlfv_seek(Nlfv *h, int64_t ms) {
    int64_t ts = av_rescale_q(h->zero + ms * 1000, AV_TIME_BASE_Q, h->tb);
    int r;
    nl_clear_error(h->fmt);
    r = avformat_seek_file(h->fmt, h->stream, INT64_MIN, ts, ts, 0);
    if (r < 0)
        r = av_seek_frame(h->fmt, h->stream, ts, AVSEEK_FLAG_BACKWARD);
    if (r < 0)
        return r;
    avcodec_flush_buffers(h->dec);
    h->flushing = 0;
    h->eof = 0;
    return 0;
}

/*
 * Decodes the next frame. Returns a new frame (free it with nlfv_frame_free) and its time in
 * `*pts_ms` (-1 when unknown), or NULL with `*status` 0 at the end or a negative error.
 */
void *nlfv_next(Nlfv *h, int64_t *pts_ms, int *status) {
    AVFrame *f = av_frame_alloc();
    int r;
    *status = 0;
    if (!f) {
        *status = AVERROR(ENOMEM);
        return NULL;
    }
    for (;;) {
        r = avcodec_receive_frame(h->dec, f);
        if (r == 0) {
            int64_t ts = f->best_effort_timestamp;
            if (ts == AV_NOPTS_VALUE)
                ts = f->pts;
            *pts_ms = ts == AV_NOPTS_VALUE
                          ? -1
                          : av_rescale_q(ts, h->tb, (AVRational){1, 1000}) - h->zero / 1000;
            return f;
        }
        if (r == AVERROR_EOF || (r == AVERROR(EAGAIN) && h->flushing)) {
            h->eof = 1;
            break;
        }
        if (r != AVERROR(EAGAIN) && r != AVERROR_INVALIDDATA) {
            *status = r;
            break;
        }
        if (r == AVERROR_INVALIDDATA)
            continue;
        r = av_read_frame(h->fmt, h->pkt);
        if (r < 0) {
            if (r != AVERROR_EOF && !(h->fmt->pb && avio_feof(h->fmt->pb))) {
                *status = r;
                break;
            }
            h->flushing = 1;
            avcodec_send_packet(h->dec, NULL);
            continue;
        }
        if (h->pkt->stream_index != h->stream) {
            av_packet_unref(h->pkt);
            continue;
        }
        r = avcodec_send_packet(h->dec, h->pkt);
        av_packet_unref(h->pkt);
        /* A damaged packet is skipped. */
        if (r < 0 && r != AVERROR(EAGAIN) && r != AVERROR_INVALIDDATA) {
            *status = r;
            break;
        }
    }
    av_frame_free(&f);
    return NULL;
}

void nlfv_frame_free(void *frame) {
    AVFrame *f = frame;
    av_frame_free(&f);
}

/* Size of a decoded frame. */
void nlfv_frame_size(const void *frame, int *w, int *h) {
    const AVFrame *f = frame;
    *w = f->width;
    *h = f->height;
}

/*
 * Scales `frame` to `dst_w`×`dst_h` native-endian 0x00RRGGBB pixels in `out` (rows of `dst_w`
 * pixels), upside down when `flip` is set. Returns 0 or a negative error.
 */
int nlfv_scale(Nlfv *h, const void *frame, uint32_t *out, int dst_w, int dst_h, int flip) {
    const AVFrame *f = frame;
    uint8_t *dst[4] = {0};
    int stride[4] = {0};
    int r;

    if (dst_w <= 0 || dst_h <= 0 || f->width <= 0 || f->height <= 0)
        return AVERROR(EINVAL);
    h->sws = sws_getCachedContext(h->sws, f->width, f->height, (enum AVPixelFormat)f->format,
                                  dst_w, dst_h, AV_PIX_FMT_0RGB32, SWS_BILINEAR, NULL, NULL,
                                  NULL);
    if (!h->sws)
        return AVERROR(EINVAL);
    {
        /* BT.709 for HD sources that say so (or say nothing, by size), BT.601 otherwise. */
        int cs = f->colorspace;
        int full = f->color_range == AVCOL_RANGE_JPEG;
        int *inv, *table, src_range, dst_range, brightness, contrast, saturation;
        if (cs == AVCOL_SPC_UNSPECIFIED)
            cs = f->height >= 720 ? AVCOL_SPC_BT709 : AVCOL_SPC_BT470BG;
        if (sws_getColorspaceDetails(h->sws, &inv, &src_range, &table, &dst_range, &brightness,
                                     &contrast, &saturation) >= 0)
            sws_setColorspaceDetails(h->sws, sws_getCoefficients(cs), full, table, 1,
                                     brightness, contrast, saturation);
    }
    if (flip) {
        dst[0] = (uint8_t *)(out + (size_t)(dst_h - 1) * (size_t)dst_w);
        stride[0] = -dst_w * 4;
    } else {
        dst[0] = (uint8_t *)out;
        stride[0] = dst_w * 4;
    }
    r = sws_scale(h->sws, (const uint8_t *const *)f->data, f->linesize, 0, f->height, dst,
                  stride);
    return r < 0 ? r : 0;
}
