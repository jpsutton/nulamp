/*
 * A small C layer over FFmpeg for nulamp's FFmpeg decoder (crates/nulamp-decoders/src/
 * ffmpeg_decoder.rs). It is compiled against the installed FFmpeg headers, so the Rust side
 * never depends on the layout of FFmpeg's structures, which changes between major versions.
 *
 * One handle opens a local file with libavformat, decodes its best audio stream with
 * libavcodec and converts every frame to interleaved 32-bit float with libswresample, keeping
 * the source sample rate and channel count. A video file without audio plays as silence of
 * the video's length (video.c shows the picture). Only the "file" protocol is allowed. A handle
 * is used by one thread at a time.
 */

#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/channel_layout.h>
#include <libavutil/dict.h>
#include <libavutil/error.h>
#include <libavutil/log.h>
#include <libavutil/mathematics.h>
#include <libavutil/mem.h>
#include <libavutil/samplefmt.h>
#include <libswresample/swresample.h>

/* Must match `NlffInfo` in ffmpeg_decoder.rs. */
typedef struct NlffInfo {
    int32_t sample_rate;
    int32_t channels;
    int32_t bits;
    int32_t lossless;
    int32_t seekable;
    /* Length in sample frames, -1 when unknown. */
    int64_t length_frames;
    /* Average bit rate in bits per second, 0 when unknown. */
    int64_t bit_rate;
    char codec[128];
} NlffInfo;

typedef struct Nlff {
    AVFormatContext *fmt;
    AVCodecContext *dec;
    SwrContext *swr;
    AVPacket *pkt;
    AVFrame *frame;
    int stream;
    AVRational tb;
    int rate;
    /* Stream time of sample frame 0. */
    int64_t origin;
    int origin_known;
    /* Input format of `swr`. */
    int swr_format;
    int swr_rate;
    AVChannelLayout swr_layout;
    float *buf;
    int buf_frames;
    int channels;
    /* Sample frame position of the next decoded sample. */
    int64_t pos;
    /* Samples before this frame are dropped after a seek (-1: none). */
    int64_t discard_until;
    /* Packet bytes sent to the decoder since the last returned block. */
    int64_t bytes;
    int flushing;
    /* `frame` holds a decoded frame not yet returned (read ahead by a seek). */
    int have_frame;
    /* A video file without audio: silence of `silent_frames` frames. */
    int silent;
    int64_t silent_frames;
} Nlff;

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

void nlff_close(Nlff *h) {
    if (!h)
        return;
    swr_free(&h->swr);
    av_channel_layout_uninit(&h->swr_layout);
    av_frame_free(&h->frame);
    av_packet_free(&h->pkt);
    avcodec_free_context(&h->dec);
    avformat_close_input(&h->fmt);
    av_free(h->buf);
    av_free(h);
}

static int64_t frames_from_ts(const Nlff *h, int64_t ts) {
    return av_rescale_q(ts, h->tb, (AVRational){1, h->rate});
}

static int64_t ts_from_frames(const Nlff *h, int64_t frames) {
    return av_rescale_q(frames, (AVRational){1, h->rate}, h->tb);
}

/*
 * A video file without an audio stream plays as 48 kHz stereo silence of the video's length.
 * Returns 1 when `h` was set up that way.
 */
static int open_silent(Nlff *h, int decode, NlffInfo *info) {
    unsigned i;
    int video = 0;
    for (i = 0; i < h->fmt->nb_streams; i++) {
        AVStream *st = h->fmt->streams[i];
        if (st->codecpar->codec_type == AVMEDIA_TYPE_VIDEO &&
            !(st->disposition & AV_DISPOSITION_ATTACHED_PIC))
            video = 1;
    }
    if (!video || h->fmt->duration == AV_NOPTS_VALUE || h->fmt->duration <= 0)
        return 0;
    h->silent = 1;
    h->rate = 48000;
    h->channels = 2;
    h->silent_frames = av_rescale(h->fmt->duration, h->rate, AV_TIME_BASE);
    memset(info, 0, sizeof *info);
    info->sample_rate = h->rate;
    info->channels = h->channels;
    info->length_frames = h->silent_frames;
    info->seekable = 1;
    snprintf(info->codec, sizeof info->codec, "%s", "No audio");
    if (decode) {
        h->buf_frames = 4096;
        h->buf = av_calloc((size_t)h->buf_frames * 2, sizeof(float));
        if (!h->buf)
            return 0;
    }
    return 1;
}

/*
 * Opens `path` (UTF-8). With `decode` 0 only the container is read (tags and stream
 * information). Returns NULL and an error text on failure.
 */
Nlff *nlff_open(const char *path, int decode, NlffInfo *info, char *err, int err_len) {
    static int quiet = 0;
    const AVCodec *codec = NULL;
    const AVCodecDescriptor *desc;
    AVDictionary *opts = NULL;
    AVCodecParameters *par;
    AVStream *st;
    unsigned i;
    int r;
    Nlff *h;

    if (!quiet) {
        av_log_set_level(AV_LOG_QUIET);
        quiet = 1;
    }
    h = av_mallocz(sizeof *h);
    if (!h) {
        set_error(err, err_len, "out of memory", 0);
        return NULL;
    }
    h->discard_until = -1;
    av_dict_set(&opts, "protocol_whitelist", "file", 0);
    r = avformat_open_input(&h->fmt, path, NULL, &opts);
    av_dict_free(&opts);
    if (r < 0) {
        set_error(err, err_len, "cannot open", r);
        nlff_close(h);
        return NULL;
    }
    r = avformat_find_stream_info(h->fmt, NULL);
    if (r < 0) {
        set_error(err, err_len, "no stream information", r);
        nlff_close(h);
        return NULL;
    }
    r = av_find_best_stream(h->fmt, AVMEDIA_TYPE_AUDIO, -1, -1, &codec, 0);
    if (r < 0 && open_silent(h, decode, info))
        return h;
    if (r < 0) {
        set_error(err, err_len, r == AVERROR_DECODER_NOT_FOUND ? "no decoder for the audio"
                                                              : "no audio stream", 0);
        nlff_close(h);
        return NULL;
    }
    h->stream = r;
    for (i = 0; i < h->fmt->nb_streams; i++)
        if ((int)i != h->stream)
            h->fmt->streams[i]->discard = AVDISCARD_ALL;
    st = h->fmt->streams[h->stream];
    par = st->codecpar;
    h->tb = st->time_base;
    h->rate = par->sample_rate > 0 ? par->sample_rate : 44100;
    h->channels = par->ch_layout.nb_channels > 0 ? par->ch_layout.nb_channels : 2;
    h->origin = st->start_time != AV_NOPTS_VALUE ? st->start_time : 0;

    memset(info, 0, sizeof *info);
    info->sample_rate = h->rate;
    info->channels = h->channels;
    desc = avcodec_descriptor_get(par->codec_id);
    /* WavPack is marked both lossy and lossless (hybrid mode); other codecs with both flags
     * (DTS) are lossy in the common case. */
    info->lossless = desc && (desc->props & AV_CODEC_PROP_LOSSLESS) &&
                     (!(desc->props & AV_CODEC_PROP_LOSSY) || par->codec_id == AV_CODEC_ID_WAVPACK);
    if (info->lossless) {
        info->bits = par->bits_per_raw_sample;
        if (info->bits <= 0)
            info->bits = av_get_bytes_per_sample((enum AVSampleFormat)par->format) * 8;
    }
    snprintf(info->codec, sizeof info->codec, "%s",
             desc && desc->long_name ? desc->long_name : avcodec_get_name(par->codec_id));
    info->length_frames = -1;
    if (st->duration != AV_NOPTS_VALUE && st->duration > 0)
        info->length_frames = frames_from_ts(h, st->duration);
    else if (h->fmt->duration != AV_NOPTS_VALUE && h->fmt->duration > 0)
        info->length_frames = av_rescale(h->fmt->duration, h->rate, AV_TIME_BASE);
    info->bit_rate = par->bit_rate > 0 ? par->bit_rate : h->fmt->bit_rate;
    if (info->bit_rate < 0)
        info->bit_rate = 0;
    info->seekable = h->fmt->pb && (h->fmt->pb->seekable & AVIO_SEEKABLE_NORMAL) &&
                     info->length_frames > 0;
    if (!decode)
        return h;

    h->dec = avcodec_alloc_context3(codec);
    h->pkt = av_packet_alloc();
    h->frame = av_frame_alloc();
    if (!h->dec || !h->pkt || !h->frame) {
        set_error(err, err_len, "out of memory", 0);
        nlff_close(h);
        return NULL;
    }
    r = avcodec_parameters_to_context(h->dec, par);
    if (r >= 0) {
        /* Lets the decoder move timestamps when it trims encoder delay and padding. */
        h->dec->pkt_timebase = st->time_base;
        r = avcodec_open2(h->dec, codec, NULL);
    }
    if (r < 0) {
        set_error(err, err_len, "cannot open the decoder", r);
        nlff_close(h);
        return NULL;
    }
    return h;
}

/* Converts `h->frame` into `h->buf`. Returns the number of frames or a negative error. */
static int convert(Nlff *h) {
    AVFrame *f = h->frame;
    int ch = f->ch_layout.nb_channels;
    int n = f->nb_samples;
    uint8_t *out[1];
    int r;

    if (ch <= 0 || n <= 0)
        return 0;
    if (n > h->buf_frames || ch != h->channels) {
        av_free(h->buf);
        h->buf = av_malloc_array((size_t)n * (size_t)ch, sizeof(float));
        h->buf_frames = h->buf ? n : 0;
        if (!h->buf)
            return AVERROR(ENOMEM);
    }
    h->channels = ch;
    if (f->format == AV_SAMPLE_FMT_FLT) {
        memcpy(h->buf, f->data[0], (size_t)n * (size_t)ch * sizeof(float));
        return n;
    }
    if (!h->swr || h->swr_format != f->format || h->swr_rate != f->sample_rate ||
        av_channel_layout_compare(&h->swr_layout, &f->ch_layout) != 0) {
        swr_free(&h->swr);
        av_channel_layout_uninit(&h->swr_layout);
        r = av_channel_layout_copy(&h->swr_layout, &f->ch_layout);
        if (r < 0)
            return r;
        h->swr_format = f->format;
        h->swr_rate = f->sample_rate;
        r = swr_alloc_set_opts2(&h->swr, &f->ch_layout, AV_SAMPLE_FMT_FLT, f->sample_rate,
                                &f->ch_layout, (enum AVSampleFormat)f->format, f->sample_rate,
                                0, NULL);
        if (r >= 0)
            r = swr_init(h->swr);
        if (r < 0) {
            swr_free(&h->swr);
            return r;
        }
    }
    out[0] = (uint8_t *)h->buf;
    return swr_convert(h->swr, out, n, (const uint8_t **)f->extended_data, n);
}

/* Decodes the next frame into `h->frame`. Returns 1, 0 at the end, or a negative error. */
static int next_frame(Nlff *h) {
    int r;
    for (;;) {
        r = avcodec_receive_frame(h->dec, h->frame);
        if (r == 0)
            return 1;
        if (r == AVERROR_EOF)
            return 0;
        if (r == AVERROR_INVALIDDATA)
            continue;
        if (r != AVERROR(EAGAIN))
            return r;
        if (h->flushing)
            return 0;
        r = av_read_frame(h->fmt, h->pkt);
        if (r < 0) {
            if (r != AVERROR_EOF && !(h->fmt->pb && avio_feof(h->fmt->pb)))
                return r;
            h->flushing = 1;
            r = avcodec_send_packet(h->dec, NULL);
            if (r < 0 && r != AVERROR_EOF)
                return r;
            continue;
        }
        if (h->pkt->stream_index != h->stream) {
            av_packet_unref(h->pkt);
            continue;
        }
        h->bytes += h->pkt->size;
        r = avcodec_send_packet(h->dec, h->pkt);
        av_packet_unref(h->pkt);
        /* A damaged packet is skipped. */
        if (r < 0 && r != AVERROR(EAGAIN) && r != AVERROR_INVALIDDATA)
            return r;
    }
}

static int64_t frame_ts(const AVFrame *f) {
    return f->pts != AV_NOPTS_VALUE ? f->pts : f->best_effort_timestamp;
}

/*
 * Decodes the next block. On success `*data` points to `frames × *channels` interleaved
 * samples, valid until the next call, and `*bytes` is the packet data read for it. Returns
 * the frame count, 0 at the end, or a negative error.
 */
int nlff_read(Nlff *h, const float **data, int *channels, int64_t *bytes) {
    if (h->silent) {
        int n = (int)FFMIN((int64_t)h->buf_frames, h->silent_frames - h->pos);
        if (n <= 0)
            return 0;
        h->pos += n;
        *data = h->buf;
        *channels = h->channels;
        *bytes = 0;
        return n;
    }
    for (;;) {
        int64_t ts, pos = h->pos;
        int n, skip = 0;
        if (!h->have_frame) {
            int r = next_frame(h);
            if (r <= 0)
                return r;
        }
        h->have_frame = 0;
        ts = frame_ts(h->frame);
        if (ts != AV_NOPTS_VALUE) {
            if (!h->origin_known)
                h->origin = ts;
            pos = frames_from_ts(h, ts - h->origin);
        }
        h->origin_known = 1;
        n = convert(h);
        av_frame_unref(h->frame);
        if (n < 0)
            return n;
        h->pos = pos + n;
        if (h->discard_until > pos)
            skip = (int)FFMIN((int64_t)n, h->discard_until - pos);
        if (h->discard_until >= 0 && h->pos >= h->discard_until)
            h->discard_until = -1;
        if (n - skip <= 0)
            continue;
        *data = h->buf + (size_t)skip * (size_t)h->channels;
        *channels = h->channels;
        *bytes = h->bytes;
        h->bytes = 0;
        return n - skip;
    }
}

/*
 * Seeks so that the next block starts at sample frame `target`. The demuxer goes to a packet
 * some way before it, the decoder runs from there and the samples before `target` are
 * dropped. When the first decoded frame still starts after `target` (codecs that need
 * several packets after a flush, coarse demuxer seeking), the seek is repeated from further
 * back. Returns 0 or a negative error.
 */
int nlff_seek(Nlff *h, int64_t target) {
    AVCodecParameters *par;
    int64_t margin;
    int attempt, r = 0;

    if (h->silent) {
        h->pos = FFMIN(FFMAX(target, 0), h->silent_frames);
        return 0;
    }
    par = h->fmt->streams[h->stream]->codecpar;
    margin = FFMAX(FFMAX(par->seek_preroll, par->frame_size), h->rate / 20);
    av_frame_unref(h->frame);
    h->have_frame = 0;
    for (attempt = 0; attempt < 5; attempt++, margin *= 4) {
        int64_t start = FFMAX(target - margin, 0);
        int64_t ts = h->origin + ts_from_frames(h, start);
        int64_t first;
        r = avformat_seek_file(h->fmt, h->stream, INT64_MIN, ts, ts, 0);
        if (r < 0)
            r = av_seek_frame(h->fmt, h->stream, ts, AVSEEK_FLAG_BACKWARD);
        if (r < 0)
            return r;
        avcodec_flush_buffers(h->dec);
        h->flushing = 0;
        h->have_frame = 0;
        r = next_frame(h);
        if (r < 0)
            return r;
        if (r == 0)
            break;
        h->have_frame = 1;
        first = frame_ts(h->frame);
        if (first == AV_NOPTS_VALUE || start == 0 ||
            frames_from_ts(h, first - h->origin) <= target)
            break;
        av_frame_unref(h->frame);
        h->have_frame = 0;
    }
    h->origin_known = 1;
    h->bytes = 0;
    h->pos = target;
    h->discard_until = target;
    return 0;
}

/*
 * Iterates the container tags (`which` 0) and the audio stream's tags (`which` 1). Start with
 * `*iter` NULL; returns 1 with the next key and value, 0 at the end.
 */
int nlff_tag_next(Nlff *h, int which, const void **iter, const char **key, const char **value) {
    const AVDictionary *d =
        which == 0 ? h->fmt->metadata : h->silent ? NULL : h->fmt->streams[h->stream]->metadata;
    const AVDictionaryEntry *e =
        av_dict_get(d, "", (const AVDictionaryEntry *)*iter, AV_DICT_IGNORE_SUFFIX);
    if (!e)
        return 0;
    *iter = e;
    *key = e->key;
    *value = e->value;
    return 1;
}

/* The libavcodec licence text ("LGPL version 2.1 or later", "GPL version 2 or later", …). */
const char *nlff_license(void) { return avcodec_license(); }
