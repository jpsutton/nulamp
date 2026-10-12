//! The decoding thread: feeds the ring buffer to Symphonia through a [`MediaSource`] (or, for
//! a video file, to FFmpeg through a [`ByteSource`]) and queues decoded blocks for the audio
//! thread.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

use super::shared::{Block, MAX_QUEUED_FRAMES, Phase, Shared, State};
use crate::ffmpeg_io::ByteSource;
use crate::opus_decoder::OpusDecoder;
use crate::symphonia_decoder::SymphoniaDecoder;
use crate::{DecodeError, Decoder, ReadStatus, StreamInfo};

/// Most bytes handed to Symphonia per read, so the ring keeps the backlog.
const MAX_READ: usize = 4096;

/// The ring buffer (or the spool) as a byte source. Reads wait while the stream connects or
/// buffers.
struct RingSource {
    shared: Arc<Shared>,
    seekable: bool,
}

fn interrupted() -> io::Error {
    io::Error::other("seek requested")
}

impl Read for RingSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut st = self.shared.lock();
        loop {
            if st.abort {
                return Ok(0);
            }
            // A seek asked for before decoding started waits for the probe to finish.
            if st.seek_req.is_some() && st.decoding {
                return Err(interrupted());
            }
            if st.ahead() > 0 && st.gate_open() {
                let n = buf.len().min(MAX_READ);
                let n = st.take(&mut buf[..n]);
                self.shared.cv.notify_all();
                return Ok(n);
            }
            if st.ahead() == 0 && st.net_done {
                st.apply_meta(true);
                return Ok(0);
            }
            // An empty ring is not an underrun by itself: a server sending in real time
            // keeps it near empty. The reader decides (see `StreamDecoder::read`).
            st = self.shared.wait(st);
        }
    }
}

impl Seek for RingSource {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let mut st = self.shared.lock();
        let target = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => st.pos.checked_add_signed(d),
            SeekFrom::End(d) => byte_len(&st).and_then(|l| l.checked_add_signed(d)),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad seek"))?;
        if target == st.pos {
            return Ok(target);
        }
        if st.spool.is_some() {
            // Reads past the received bytes wait for them.
            st.pos = target;
            self.shared.cv.notify_all();
            return Ok(target);
        }
        if target > st.pos && target - st.pos <= st.ring.len() as u64 {
            let n = (target - st.pos) as usize;
            st.skip(n);
            self.shared.cv.notify_all();
            return Ok(target);
        }
        if !self.seekable {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "live stream is not seekable",
            ));
        }
        // Reconnect with a Range request at the new offset.
        st.epoch += 1;
        st.restart = Some(target);
        st.ring.clear();
        st.pending_meta.clear();
        st.pos = target;
        st.net_done = false;
        if !matches!(st.phase, Phase::Failed(_)) {
            let p = st.rebuffer_percent;
            st.start_buffering(p);
        }
        st.shutdown_socket();
        self.shared.cv.notify_all();
        Ok(target)
    }
}

impl MediaSource for RingSource {
    fn is_seekable(&self) -> bool {
        self.seekable
    }

    /// Hidden from the probe while a spool fills: Symphonia then reads no trailing tags,
    /// which would wait for the whole download before playback starts.
    fn byte_len(&self) -> Option<u64> {
        let st = self.shared.lock();
        if st.spool.is_some() && !st.decoding && !st.complete() {
            return None;
        }
        byte_len(&st)
    }
}

/// FFmpeg gets the length at once: it needs it to find the index at the end of some files.
impl ByteSource for RingSource {
    fn byte_len(&mut self) -> Option<u64> {
        byte_len(&self.shared.lock())
    }

    fn seekable(&self) -> bool {
        self.seekable
    }
}

/// The stream's length: from the headers, or a complete spool's size.
fn byte_len(st: &State) -> Option<u64> {
    st.total_len
        .or_else(|| st.complete().then(|| st.received()))
}

/// Marks decoding as finished, with an error when the stream failed or `error` is given.
fn done(shared: &Shared, error: Option<DecodeError>) {
    let mut st = shared.lock();
    st.decode_done = true;
    st.decode_error = match &st.phase {
        Phase::Failed(e) => Some(DecodeError::Stream(e.clone())),
        _ => error,
    };
    shared.cv.notify_all();
}

/// Whether the stream is Ogg Opus, which Symphonia cannot decode: its first page names the
/// codec. Leaves the source at the start.
fn is_opus(source: &mut RingSource) -> bool {
    let mut head = [0u8; 128];
    let mut n = 0;
    while n < head.len() {
        match source.read(&mut head[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    let opus = head[..n].starts_with(b"OggS") && head[..n].windows(8).any(|w| w == b"OpusHead");
    opus && source.seek(SeekFrom::Start(0)).is_ok()
}

/// Opens an Ogg Opus stream with the Opus decoder, from the spool.
fn open_opus(
    source: RingSource,
    len: u64,
    complete: bool,
) -> Result<Box<dyn Decoder>, DecodeError> {
    Ok(Box::new(OpusDecoder::open_source(
        Box::new(source),
        len,
        complete,
    )?))
}

/// Probes the stream with Symphonia.
fn open_symphonia(
    source: RingSource,
    hint: Option<&str>,
    spooled: bool,
) -> Result<Box<dyn Decoder>, DecodeError> {
    let mss = MediaSourceStream::new(Box::new(source), Default::default());
    let mut h = Hint::new();
    if let Some(ext) = hint {
        h.with_extension(ext);
    }
    let format = symphonia::default::get_probe()
        .probe(
            &h,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| DecodeError::Unsupported(format!("stream format: {e}")))?;
    // A spool reads like a local file: seeks are exact. Range seeks guess the byte offset
    // instead of reading from the start.
    let dec = SymphoniaDecoder::new(format)?;
    Ok(Box::new(if spooled { dec } else { dec.with_coarse_seek() }))
}

/// Opens the sound of a video file with FFmpeg, through a demuxer the video window shares
/// under `key` (Symphonia without FFmpeg).
fn open_video(
    source: RingSource,
    key: &str,
    name: &str,
    audio: Option<usize>,
    hint: Option<&str>,
    spooled: bool,
) -> Result<Box<dyn Decoder>, DecodeError> {
    #[cfg(nulamp_ffmpeg)]
    {
        let _ = (hint, spooled);
        let io = crate::ffmpeg_io::Io::new(Box::new(source));
        Ok(Box::new(crate::ffmpeg_decoder::open_video(
            key,
            name,
            Some(io),
            audio,
        )?))
    }
    #[cfg(not(nulamp_ffmpeg))]
    {
        let _ = (key, name, audio);
        open_symphonia(source, hint, spooled)
    }
}

pub(crate) fn run(shared: Arc<Shared>) {
    let (hint, seekable, len, spooled, video, name) = {
        let mut st = shared.lock();
        while !st.headers_ready && !st.net_done && !st.abort {
            st = shared.wait(st);
        }
        if st.abort {
            return;
        }
        if !st.headers_ready {
            drop(st);
            return done(&shared, None);
        }
        let video = st.video.clone().map(|key| (key, st.audio_track));
        (
            st.hint,
            st.seekable,
            st.total_len,
            st.spool.is_some(),
            video,
            st.fields.location.clone(),
        )
    };
    let mut source = RingSource {
        shared: shared.clone(),
        seekable,
    };
    // Only a spool seeks back to the start for free, and the Opus decoder needs the length.
    let opus_len = len.filter(|_| video.is_none() && spooled && hint == Some("ogg"));
    let opened = match (video, opus_len) {
        (Some((key, audio)), _) => open_video(source, &key, &name, audio, hint, spooled),
        (None, Some(len)) if is_opus(&mut source) => {
            let complete = shared.lock().complete();
            open_opus(source, len, complete)
        }
        (None, _) => open_symphonia(source, hint, spooled),
    };
    let mut dec = match opened {
        Ok(d) => d,
        Err(e) => {
            if shared.lock().abort {
                return;
            }
            return done(&shared, Some(e));
        }
    };
    let mut info = StreamInfo {
        seekable,
        ..dec.info().clone()
    };
    if !seekable {
        info.length_ms = None;
    }
    {
        let mut st = shared.lock();
        st.set_info(info.clone());
        st.decoding = true;
        shared.cv.notify_all();
    }
    let mut buf = Vec::new();
    loop {
        let (generation, seek) = {
            let mut st = shared.lock();
            loop {
                if st.abort {
                    return;
                }
                if let Some(ms) = st.seek_req.take() {
                    break (st.generation, Some(ms));
                }
                if !st.decode_done && st.pcm_frames < MAX_QUEUED_FRAMES {
                    break (st.generation, None);
                }
                st = shared.wait(st);
            }
        };
        if let Some(ms) = seek {
            if let Err(e) = dec.seek(ms) {
                log::warn!("stream seek failed: {e}");
            }
            continue;
        }
        buf.clear();
        let status = dec.read(&mut buf);
        let mut st = shared.lock();
        if st.abort {
            return;
        }
        if st.seek_req.is_some() || st.generation != generation {
            continue;
        }
        match status {
            Ok(ReadStatus::Frames(frames)) => {
                let (rate, channels) = (dec.info().sample_rate, dec.info().channels);
                st.pcm.push_back(Block {
                    samples: std::mem::take(&mut buf),
                    frames,
                    rate,
                    channels,
                });
                st.pcm_frames += frames;
                st.decoded_kbps = dec.kbps();
                if let Some(t) = dec.take_title() {
                    st.chain_title = Some(t);
                }
                if info.length_ms.is_none()
                    && let (Some(total), Some(kbps)) = (len.filter(|_| seekable), st.decoded_kbps)
                    && kbps > 0
                {
                    // Length estimate from the decoded bit rate.
                    info.length_ms = Some(total * 8 / kbps as u64);
                    st.set_info(info.clone());
                }
                shared.cv.notify_all();
            }
            Ok(ReadStatus::Pending) => {}
            Ok(ReadStatus::End) | Err(_) => {
                if let Err(e) = &status {
                    log::info!("stream decoding stopped: {e}");
                }
                drop(st);
                done(&shared, None);
                if !seekable {
                    return;
                }
            }
        }
    }
}
