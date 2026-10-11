//! State shared by the network thread, the decoding thread and the decoder handle: the byte
//! ring buffer (or the spool of a finite stream) with its buffering state (07 §8.6), the
//! decoded audio queue and the stream description.

use std::collections::VecDeque;
use std::net::{Shutdown, TcpStream};
use std::sync::{Condvar, Mutex, MutexGuard};

use super::spool::Spool;
use super::{StreamError, StreamFields, icy};
use crate::{DecodeError, StreamInfo};

/// Decoded frames queued ahead of the audio thread. Kept short so the network buffer, not
/// this queue, holds the backlog.
pub(crate) const MAX_QUEUED_FRAMES: usize = 16_384;

/// Needed prebuffer in percent of the buffer (07 §8.6): min(100, setting × clamp(kbps, 8,
/// 320) / 128), at least half the setting at the start. Unknown bit rates count as 128.
pub fn prebuffer_percent(setting: u32, kbps: Option<u32>, at_start: bool) -> u32 {
    let k = kbps.unwrap_or(128).clamp(8, 320);
    let p = (setting * k / 128).min(100);
    if at_start { p.max(setting / 2) } else { p }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Phase {
    Connecting,
    /// Waiting until the ring holds `target` bytes.
    Buffering {
        target: usize,
    },
    Playing,
    Failed(StreamError),
}

/// A decoded block.
pub(crate) struct Block {
    pub samples: Vec<f32>,
    pub frames: usize,
    pub rate: u32,
    pub channels: u16,
}

pub(crate) struct State {
    pub ring: VecDeque<u8>,
    pub capacity: usize,
    /// Every byte received, for a finite stream (NET13); the ring is then unused.
    pub spool: Option<Spool>,
    /// The location is a file, not a live stream, even without a `Content-Length`.
    pub finite: bool,
    /// Spool finite streams ([`super::StreamConfig::spool`]).
    pub spool_enabled: bool,
    /// Absolute byte offset of the decoder's next byte: the ring's first byte, or the read
    /// position in the spool.
    pub pos: u64,
    pub phase: Phase,
    pub rebuffer_percent: u32,
    /// Bumped when the decoder seeks to a byte offset; bytes of older connections are dropped.
    pub epoch: u64,
    /// Byte offset the network thread must reconnect at.
    pub restart: Option<u64>,
    /// No more bytes arrive (until a restart).
    pub net_done: bool,
    pub abort: bool,
    pub socket: Option<TcpStream>,
    /// In-band metadata waiting for the decoder to reach its byte offset.
    pub pending_meta: VecDeque<(u64, Vec<(String, String)>)>,

    pub headers_ready: bool,
    pub connect_url: String,
    /// Title without stream metadata: `Content-Disposition` file name or the URL's last part.
    pub fallback_title: String,
    pub show_station: bool,
    pub fields: StreamFields,
    /// Format hint from the content type, as a file extension.
    pub hint: Option<&'static str>,
    pub seekable: bool,
    pub total_len: Option<u64>,

    pub pcm: VecDeque<Block>,
    pub pcm_frames: usize,
    /// Bumped by every seek; blocks decoded for an older generation are dropped.
    pub generation: u64,
    pub seek_req: Option<u64>,
    pub decode_done: bool,
    pub decode_error: Option<DecodeError>,
    pub decoding: bool,
    pub info: StreamInfo,
    pub info_version: u64,
    pub decoded_kbps: Option<u32>,
    pub chain_title: Option<String>,
}

impl State {
    pub fn new(location: &str, capacity: usize, rebuffer_percent: u32, show_station: bool) -> Self {
        Self {
            ring: VecDeque::with_capacity(capacity),
            capacity,
            spool: None,
            finite: false,
            spool_enabled: true,
            pos: 0,
            phase: Phase::Connecting,
            rebuffer_percent,
            epoch: 0,
            restart: None,
            net_done: false,
            abort: false,
            socket: None,
            pending_meta: VecDeque::new(),
            headers_ready: false,
            connect_url: location.to_string(),
            fallback_title: super::url::location_title(location),
            show_station,
            fields: StreamFields {
                location: location.to_string(),
                ..StreamFields::default()
            },
            hint: None,
            seekable: false,
            total_len: None,
            pcm: VecDeque::new(),
            pcm_frames: 0,
            generation: 0,
            seek_req: None,
            decode_done: false,
            decode_error: None,
            decoding: false,
            info: StreamInfo {
                sample_rate: 44_100,
                channels: 2,
                ..StreamInfo::default()
            },
            info_version: 0,
            decoded_kbps: None,
            chain_title: None,
        }
    }

    /// Bytes received so far (from the start, or from the last Range offset).
    pub fn received(&self) -> u64 {
        match &self.spool {
            Some(s) => s.len(),
            None => self.pos + self.ring.len() as u64,
        }
    }

    /// Bytes received ahead of the decoder.
    pub fn ahead(&self) -> u64 {
        self.received().saturating_sub(self.pos)
    }

    /// The whole stream is in the spool.
    pub fn complete(&self) -> bool {
        self.spool.is_some() && self.net_done && !matches!(self.phase, Phase::Failed(_))
    }

    /// The bytes the decoder can reach without a new request, as `start..end` offsets: the
    /// spool from its start, or the ring of a Range-seekable file. `None` for live streams.
    pub fn local_bytes(&self) -> Option<(u64, u64)> {
        if self.spool.is_some() {
            Some((0, self.received()))
        } else if self.seekable {
            Some((self.pos, self.received()))
        } else {
            None
        }
    }

    /// Moves up to `buf.len()` bytes out of the ring (or reads them from the spool) and
    /// applies the metadata they reach.
    pub fn take(&mut self, buf: &mut [u8]) -> usize {
        if let Some(spool) = &mut self.spool {
            let n = match spool.read_at(self.pos, buf) {
                Ok(n) => n,
                Err(e) => {
                    // Ends the stream: the decoder sees no more bytes.
                    log::warn!("stream spool: {e}");
                    0
                }
            };
            self.pos += n as u64;
            return n;
        }
        let n = buf.len().min(self.ring.len());
        for (d, s) in buf.iter_mut().zip(self.ring.drain(..n)) {
            *d = s;
        }
        self.pos += n as u64;
        self.apply_meta(false);
        n
    }

    /// Drops `n` buffered bytes.
    pub fn skip(&mut self, n: usize) {
        self.ring.drain(..n.min(self.ring.len()));
        self.pos += n as u64;
        self.apply_meta(false);
    }

    /// Applies metadata whose offset was reached, or all of it.
    pub fn apply_meta(&mut self, all: bool) {
        while let Some((offset, _)) = self.pending_meta.front() {
            if *offset > self.pos && !all {
                break;
            }
            let (_, pairs) = self.pending_meta.pop_front().expect("pending metadata");
            for (k, v) in pairs {
                match k.as_str() {
                    "StreamTitle" => self.fields.streamtitle = Some(v),
                    "StreamUrl" => self.fields.streamlink = Some(v),
                    _ => {}
                }
            }
        }
    }

    /// Waits until `percent` of the buffer is filled before audio flows again.
    pub fn start_buffering(&mut self, percent: u32) {
        let target = (self.capacity as u64 * percent as u64 / 100).max(1) as usize;
        self.phase = Phase::Buffering {
            target: target.min(self.capacity),
        };
        self.check_buffered();
    }

    pub fn check_buffered(&mut self) {
        if let Phase::Buffering { target } = self.phase
            && (self.ahead() >= target as u64 || self.net_done)
        {
            self.phase = Phase::Playing;
        }
    }

    /// Percent of the buffering target reached, while buffering.
    pub fn buffer_percent(&self) -> Option<u32> {
        match self.phase {
            Phase::Buffering { target } => {
                Some((self.ahead() * 100 / target.max(1) as u64).min(100) as u32)
            }
            _ => None,
        }
    }

    /// Audio may leave the ring (not connecting or buffering).
    pub fn gate_open(&self) -> bool {
        matches!(self.phase, Phase::Playing | Phase::Failed(_))
    }

    pub fn shutdown_socket(&mut self) {
        if let Some(s) = self.socket.take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }

    /// The title without a status prefix.
    pub fn base_title(&self) -> String {
        if let Some(t) = &self.chain_title {
            return t.clone();
        }
        icy::display_title(
            self.fields.streamtitle.as_deref(),
            self.fields.streamname.as_deref(),
            self.show_station,
            &self.fallback_title,
        )
    }

    /// The displayed title (07 §8.6): `[Connecting] <url>`, `[Buffer: N%] <title>`, an error
    /// status, or the stream title.
    pub fn title(&self) -> String {
        match &self.phase {
            Phase::Connecting => format!("[Connecting] {}", self.connect_url),
            Phase::Buffering { .. } => format!(
                "[Buffer: {}%] {}",
                self.buffer_percent().unwrap_or(0),
                self.base_title()
            ),
            Phase::Failed(e) => format!("{} {}", e.status_text(), self.base_title()),
            Phase::Playing => self.base_title(),
        }
    }

    /// The stream fields, unless the stream was closed.
    pub fn clone_fields(&self) -> Option<StreamFields> {
        (!self.abort).then(|| self.fields.clone())
    }

    pub fn set_info(&mut self, info: StreamInfo) {
        self.info = info;
        self.info_version += 1;
    }
}

pub(crate) struct Shared {
    pub state: Mutex<State>,
    pub cv: Condvar,
}

impl Shared {
    pub fn new(state: State) -> Self {
        Self {
            state: Mutex::new(state),
            cv: Condvar::new(),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn wait<'a>(&self, g: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.cv.wait(g).unwrap_or_else(|e| e.into_inner())
    }

    pub fn wait_timeout<'a>(
        &self,
        g: MutexGuard<'a, State>,
        d: std::time::Duration,
    ) -> MutexGuard<'a, State> {
        self.cv
            .wait_timeout(g, d)
            .unwrap_or_else(|e| e.into_inner())
            .0
    }

    /// Appends network bytes, waiting for room in the ring. False when the connection should
    /// stop (abort, a seek started a new epoch, or the spool cannot be written).
    pub fn push(&self, mut bytes: &[u8], epoch: u64) -> bool {
        let mut st = self.lock();
        loop {
            if st.abort || st.epoch != epoch {
                return false;
            }
            if bytes.is_empty() {
                return true;
            }
            if let Some(spool) = &mut st.spool {
                // A spool takes everything at once: finite streams download as fast as they can.
                let res = spool.append(bytes);
                if let Err(e) = res {
                    log::warn!("stream spool: {e}");
                    st.phase = Phase::Failed(StreamError::Io(e.to_string()));
                    st.net_done = true;
                    self.cv.notify_all();
                    return false;
                }
                st.check_buffered();
                self.cv.notify_all();
                return true;
            }
            let room = st.capacity - st.ring.len();
            if room > 0 {
                let n = room.min(bytes.len());
                st.ring.extend(&bytes[..n]);
                bytes = &bytes[n..];
                st.check_buffered();
                self.cv.notify_all();
                continue;
            }
            st = self.wait(st);
        }
    }

    /// Stops all threads and unblocks their sockets.
    pub fn abort(&self) {
        let mut st = self.lock();
        st.abort = true;
        st.shutdown_socket();
        self.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prebuffer_formula() {
        assert_eq!(prebuffer_percent(40, Some(128), true), 40);
        assert_eq!(prebuffer_percent(40, Some(32), true), 20);
        assert_eq!(prebuffer_percent(40, Some(32), false), 10);
        assert_eq!(prebuffer_percent(40, Some(1), false), 2);
        assert_eq!(prebuffer_percent(40, Some(320), true), 100);
        assert_eq!(prebuffer_percent(40, Some(9999), true), 100);
        assert_eq!(prebuffer_percent(40, None, false), 40);
    }

    #[test]
    fn buffering_states() {
        let mut st = State::new("http://h/a.mp3", 1000, 10, true);
        assert_eq!(st.title(), "[Connecting] http://h/a.mp3");
        st.start_buffering(40);
        assert_eq!(st.phase, Phase::Buffering { target: 400 });
        st.ring.extend([0u8; 100]);
        assert_eq!(st.buffer_percent(), Some(25));
        assert_eq!(st.title(), "[Buffer: 25%] a.mp3");
        st.ring.extend([0u8; 300]);
        st.check_buffered();
        assert_eq!(st.phase, Phase::Playing);
        assert!(st.gate_open());
        // Underrun: rebuffer to 10%.
        st.ring.clear();
        st.start_buffering(st.rebuffer_percent);
        assert_eq!(st.phase, Phase::Buffering { target: 100 });
        st.net_done = true;
        st.check_buffered();
        assert_eq!(st.phase, Phase::Playing);
        st.phase = Phase::Failed(StreamError::Timeout);
        assert_eq!(st.title(), "[Timed out] a.mp3");
    }

    #[test]
    fn metadata_applies_at_its_byte_offset() {
        let mut st = State::new("http://h/", 1000, 10, true);
        st.fields.streamname = Some("R".into());
        st.ring.extend([0u8; 300]);
        let pairs = |t: &str| vec![("StreamTitle".to_string(), t.to_string())];
        st.pending_meta.push_back((100, pairs("A")));
        st.pending_meta.push_back((250, pairs("B")));
        let mut buf = [0u8; 99];
        assert_eq!(st.take(&mut buf), 99);
        assert_eq!(st.base_title(), "R");
        st.take(&mut buf[..1]);
        assert_eq!(st.base_title(), "A (R)");
        st.skip(150);
        assert_eq!(st.base_title(), "B (R)");
        assert_eq!((st.pos, st.ring.len()), (250, 50));
    }
}
