/*
 * One demuxer shared by the sound decoder (shim.c) and the picture decoder (video.c) of a
 * video file, so the file is read once (crates/nulamp-decoders/src/demux.rs). The Rust side
 * reads packets with nld_read, queues them per stream and hands them to the decoders through
 * NlPull callbacks.
 */

#ifndef NULAMP_FFMPEG_DEMUX_H
#define NULAMP_FFMPEG_DEMUX_H

#include <stdint.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>

#include "io.h"

typedef struct NlDemux NlDemux;

/*
 * Gives a decoder its next packet: moves it into `dst` (with nld_packet_move) and returns 1,
 * or returns 0 at the end, 2 when no packet is ready yet (try again later), or a negative
 * value on error.
 */
typedef int (*NlPull)(void *opaque, AVPacket *dst);

/* Opens `path`, or reads through `io` when it is not NULL. NULL and an error text on failure. */
NlDemux *nld_open(const char *path, const NlIo *io, char *err, int err_len);
void nld_close(NlDemux *d);
AVFormatContext *nld_fmt(NlDemux *d);

/*
 * Reads the next packet of any stream not discarded. Returns 1 with a new packet in `*pkt`
 * (free it with nld_packet_free or hand it over with nld_packet_move), its stream index, key
 * frame flag and size; 0 at the end; a negative error.
 */
int nld_read(NlDemux *d, void **pkt, int *stream, int *key, int *size);
void nld_packet_free(void *pkt);
/* Moves packet `src` (from nld_read) into `dst` and frees `src`. */
void nld_packet_move(AVPacket *dst, void *src);

/* Index of the first video stream (not cover art), -1 when there is none. */
int nld_video_stream(NlDemux *d);

/* Reads only the streams `audio` and `video` (either may be -1); the others are discarded. */
void nld_route(NlDemux *d, int audio, int video);

/*
 * Goes to the key frame of stream `stream` at or before `ms` (from the start of the best audio
 * stream, as the decoders count). Returns 0 or a negative error.
 */
int nld_seek(NlDemux *d, int stream, int64_t ms);

#endif
