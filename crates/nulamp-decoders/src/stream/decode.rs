//! The decoding thread: feeds the ring buffer to Symphonia through a [`MediaSource`] and
//! queues decoded blocks for the audio thread.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

use super::shared::{Block, MAX_QUEUED_FRAMES, Phase, Shared};
use crate::symphonia_decoder::SymphoniaDecoder;
use crate::{DecodeError, Decoder, ReadStatus, StreamInfo};

/// Most bytes handed to Symphonia per read, so the ring keeps the backlog.
const MAX_READ: usize = 4096;

/// The ring buffer as a byte source. Reads wait while the stream connects or buffers.
struct RingSource {
    shared: Arc<Shared>,
    seekable: bool,
    len: Option<u64>,
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
            if st.seek_req.is_some() {
                return Err(interrupted());
            }
            if !st.ring.is_empty() && st.gate_open() {
                let n = buf.len().min(MAX_READ);
                let n = st.take(&mut buf[..n]);
                self.shared.cv.notify_all();
                return Ok(n);
            }
            if st.ring.is_empty() {
                if st.net_done {
                    st.apply_meta(true);
                    return Ok(0);
                }
                if st.phase == Phase::Playing {
                    // Underrun.
                    let p = st.rebuffer_percent;
                    st.start_buffering(p);
                    self.shared.cv.notify_all();
                }
            }
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
            SeekFrom::End(d) => self.len.and_then(|l| l.checked_add_signed(d)),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad seek"))?;
        if target == st.pos {
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

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
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

pub(crate) fn run(shared: Arc<Shared>) {
    let (hint, seekable, len) = {
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
        (st.hint, st.seekable, st.total_len)
    };
    let source = RingSource {
        shared: shared.clone(),
        seekable,
        len,
    };
    let mss = MediaSourceStream::new(Box::new(source), Default::default());
    let mut h = Hint::new();
    if let Some(ext) = hint {
        h.with_extension(ext);
    }
    let format = match symphonia::default::get_probe().probe(
        &h,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(f) => f,
        Err(e) => {
            if shared.lock().abort {
                return;
            }
            return done(
                &shared,
                Some(DecodeError::Unsupported(format!("stream format: {e}"))),
            );
        }
    };
    let mut dec = match SymphoniaDecoder::new(format) {
        Ok(d) => d.with_coarse_seek(),
        Err(e) => return done(&shared, Some(e)),
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
