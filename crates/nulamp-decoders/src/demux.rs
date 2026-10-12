//! One FFmpeg demuxer per open video file, shared by its sound and its picture (decision V10):
//! the file is read once, local or from a server.
//!
//! The sound decoder ([`crate::ffmpeg_decoder`]) opens a [`Session`] and registers it under the
//! playlist location; the video window's worker ([`crate::video::VideoPlayer`]) finds it there.
//! Packets are queued per stream. Whichever side needs a packet and finds its queue empty reads
//! the next one from the demuxer and queues it for the other side. Decoding happens outside the
//! locks, so the sound never waits for the picture to decode: when the picture falls behind
//! and its queue is full, its packets are dropped up to the next key frame. Seeks come from the
//! sound side; the picture follows the [`Session::epoch`].

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::NonNull;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError, Weak};
use std::time::Duration;

use crate::ffmpeg_io::{Io, NlIo};

mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct NlDemux {
        _private: [u8; 0],
    }

    unsafe extern "C" {
        pub fn nld_open(
            path: *const c_char,
            io: *const crate::ffmpeg_io::NlIo,
            err: *mut c_char,
            err_len: c_int,
        ) -> *mut NlDemux;
        pub fn nld_close(d: *mut NlDemux);
        pub fn nld_read(
            d: *mut NlDemux,
            pkt: *mut *mut c_void,
            stream: *mut c_int,
            key: *mut c_int,
            size: *mut c_int,
        ) -> c_int;
        pub fn nld_packet_free(pkt: *mut c_void);
        pub fn nld_packet_move(dst: *mut c_void, src: *mut c_void);
        pub fn nld_video_stream(d: *mut NlDemux) -> c_int;
        pub fn nld_route(d: *mut NlDemux, audio: c_int, video: c_int);
        pub fn nld_seek(d: *mut NlDemux, stream: c_int, ms: i64) -> c_int;
    }
}

pub(crate) use ffi::NlDemux;

/// The demuxer seeks this far before a target, so the sound has packets from before it (the
/// samples up to the target are dropped).
const SEEK_MARGIN_MS: u64 = 250;
/// Most bytes of picture packets queued while a picture is shown; more means it fell behind.
const VIDEO_CAP: usize = 96 << 20;
/// Most bytes of picture packets kept (from the last key frame) while no picture is shown.
const IDLE_VIDEO_CAP: usize = 32 << 20;
/// The picture stops reading ahead while this much sound is queued (the sound is behind).
const AUDIO_CAP: usize = 8 << 20;

/// A demuxed packet (an FFmpeg `AVPacket`), freed when dropped.
pub(crate) struct Packet {
    ptr: NonNull<c_void>,
    size: usize,
    key: bool,
}

// SAFETY: a packet is owned by one queue or one thread at a time; FFmpeg packets have no thread
// affinity.
unsafe impl Send for Packet {}

impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: from nld_read, freed once.
        unsafe { ffi::nld_packet_free(self.ptr.as_ptr()) };
    }
}

impl Packet {
    /// Moves the packet into the decoder's `AVPacket`.
    ///
    /// # Safety
    /// `dst` is a valid `AVPacket`.
    unsafe fn move_into(self, dst: *mut c_void) {
        let src = self.ptr.as_ptr();
        std::mem::forget(self);
        // SAFETY: see the function's contract; `src` is from nld_read and freed here.
        unsafe { ffi::nld_packet_move(dst, src) };
    }
}

/// The demuxer and the byte source it reads.
pub(crate) struct Demux {
    ptr: NonNull<NlDemux>,
    _io: Option<Io>,
}

// SAFETY: used under the session's lock only; FFmpeg contexts have no thread affinity.
unsafe impl Send for Demux {}

impl Drop for Demux {
    fn drop(&mut self) {
        // SAFETY: from nld_open, closed once, before the byte source is dropped.
        unsafe { ffi::nld_close(self.ptr.as_ptr()) };
    }
}

impl Demux {
    pub fn as_ptr(&self) -> *mut NlDemux {
        self.ptr.as_ptr()
    }
}

#[derive(Default)]
struct Queues {
    audio: VecDeque<Packet>,
    video: VecDeque<Packet>,
    audio_bytes: usize,
    video_bytes: usize,
    audio_stream: c_int,
    video_stream: c_int,
    /// Pictures attached ([`Session::attach`]).
    viewers: usize,
    /// Drop picture packets up to the next key frame.
    skip_to_key: bool,
    /// The demuxer reached the end (or failed); cleared by a seek.
    end: bool,
    /// Bumped by every seek and stream change.
    epoch: u64,
    /// Bumped by every packet read, to wake waiting readers.
    reads: u64,
    /// The sound decoder is gone.
    closed: bool,
}

impl Queues {
    fn clear_video(&mut self) {
        self.video.clear();
        self.video_bytes = 0;
    }

    fn route(&mut self, stream: c_int, p: Packet) {
        if stream == self.audio_stream {
            self.audio_bytes += p.size;
            self.audio.push_back(p);
            return;
        }
        if stream != self.video_stream {
            return;
        }
        if self.skip_to_key && !p.key {
            return;
        }
        self.skip_to_key = false;
        if self.viewers == 0 && p.key {
            // Nobody watches yet: keep only what a picture needs to start from.
            self.clear_video();
        }
        self.video_bytes += p.size;
        self.video.push_back(p);
        let cap = if self.viewers == 0 {
            IDLE_VIDEO_CAP
        } else {
            VIDEO_CAP
        };
        if self.video_bytes > cap {
            log::debug!("video: picture packets dropped up to the next key frame");
            self.clear_video();
            self.skip_to_key = true;
        }
    }
}

/// What [`Session::pull`] got.
pub(crate) enum Pulled {
    Packet(Packet),
    End,
    /// Nothing for the picture yet (the demuxer is busy, or the sound is far behind).
    Pending,
}

/// The demuxer of one video file and its packet queues.
pub(crate) struct Session {
    key: String,
    demux: Mutex<Demux>,
    q: Mutex<Queues>,
    cv: Condvar,
}

/// Open sessions, newest last.
static SESSIONS: Mutex<Vec<Weak<Session>>> = Mutex::new(Vec::new());

fn sessions() -> MutexGuard<'static, Vec<Weak<Session>>> {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

impl Session {
    /// Opens `path`, or reads `io` when given (`path` is then only a name), and registers the
    /// session under `key` (the playlist location). Blocks while FFmpeg reads the headers.
    pub fn open(key: &str, path: &str, mut io: Option<Io>) -> Result<Arc<Self>, String> {
        let c_path = CString::new(path).map_err(|e| e.to_string())?;
        let table: Option<NlIo> = io.as_mut().map(Io::ffi);
        let mut err = [0 as c_char; 256];
        // SAFETY: valid NUL-terminated path, callback table (or null) whose source lives in
        // `io`, and an error buffer of the given size.
        let d = unsafe {
            ffi::nld_open(
                c_path.as_ptr(),
                table.as_ref().map_or(std::ptr::null(), |t| t as *const _),
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        let Some(ptr) = NonNull::new(d) else {
            // SAFETY: the shim writes a NUL-terminated message (the buffer starts zeroed).
            let msg = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy();
            return Err(msg.into_owned());
        };
        let demux = Demux { ptr, _io: io };
        // SAFETY: valid demuxer.
        let video = unsafe { ffi::nld_video_stream(demux.as_ptr()) };
        // SAFETY: as above.
        unsafe { ffi::nld_route(demux.as_ptr(), -1, video) };
        let session = Arc::new(Self {
            key: key.to_string(),
            demux: Mutex::new(demux),
            q: Mutex::new(Queues {
                audio_stream: -1,
                video_stream: video,
                ..Queues::default()
            }),
            cv: Condvar::new(),
        });
        let mut all = sessions();
        all.retain(|s| s.strong_count() > 0);
        all.push(Arc::downgrade(&session));
        Ok(session)
    }

    /// The newest open session of `key`.
    pub fn find(key: &str) -> Option<Arc<Self>> {
        sessions()
            .iter()
            .rev()
            .filter_map(Weak::upgrade)
            .find(|s| s.key == key && !s.closed())
    }

    fn lock_q(&self) -> MutexGuard<'_, Queues> {
        self.q.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The demuxer, for opening a decoder on it.
    pub fn demux(&self) -> MutexGuard<'_, Demux> {
        self.demux.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn route_streams(&self, d: &Demux, q: &Queues) {
        // SAFETY: valid demuxer, used under its lock.
        unsafe { ffi::nld_route(d.as_ptr(), q.audio_stream, q.video_stream) };
    }

    /// The sound decoder reads stream `stream` (-1: none).
    pub fn set_audio_stream(&self, stream: c_int) {
        let d = self.demux();
        let mut q = self.lock_q();
        q.audio_stream = stream;
        q.audio.clear();
        q.audio_bytes = 0;
        self.route_streams(&d, &q);
    }

    /// The picture shows stream `stream` from its next key frame.
    pub fn set_video_stream(&self, stream: c_int) {
        let d = self.demux();
        let mut q = self.lock_q();
        if q.video_stream == stream {
            return;
        }
        q.video_stream = stream;
        q.clear_video();
        q.skip_to_key = true;
        self.route_streams(&d, &q);
    }

    pub fn attach(&self) {
        self.lock_q().viewers += 1;
    }

    pub fn detach(&self) {
        let mut q = self.lock_q();
        q.viewers = q.viewers.saturating_sub(1);
    }

    /// Bumped by every seek: picture frames decoded before it are stale.
    pub fn epoch(&self) -> u64 {
        self.lock_q().epoch
    }

    /// The sound decoder is gone (the track was closed or reopened).
    pub fn close(&self) {
        self.lock_q().closed = true;
        self.cv.notify_all();
    }

    pub fn closed(&self) -> bool {
        self.lock_q().closed
    }

    /// Reads one packet into the queues.
    fn read_one(&self, d: &Demux) {
        let mut pkt = std::ptr::null_mut();
        let (mut stream, mut key, mut size) = (0, 0, 0);
        // SAFETY: valid demuxer under its lock; valid out pointers.
        let r = unsafe { ffi::nld_read(d.as_ptr(), &mut pkt, &mut stream, &mut key, &mut size) };
        let mut q = self.lock_q();
        q.reads += 1;
        match NonNull::new(pkt) {
            Some(ptr) if r == 1 => q.route(
                stream,
                Packet {
                    ptr,
                    size: usize::try_from(size).unwrap_or(0),
                    key: key != 0,
                },
            ),
            _ => {
                if r < 0 {
                    log::debug!("demuxer stopped: FFmpeg error {r}");
                }
                q.end = true;
            }
        }
        self.cv.notify_all();
    }

    /// The next packet for the sound (`video` false; waits) or the picture (does not wait for
    /// the other side).
    pub fn pull(&self, video: bool) -> Pulled {
        loop {
            let seen = {
                let mut q = self.lock_q();
                let p = if video {
                    q.video.pop_front()
                } else {
                    q.audio.pop_front()
                };
                if let Some(p) = p {
                    if video {
                        q.video_bytes -= p.size;
                    } else {
                        q.audio_bytes -= p.size;
                    }
                    return Pulled::Packet(p);
                }
                if q.end || (video && q.closed) {
                    return Pulled::End;
                }
                if video && q.audio_bytes > AUDIO_CAP {
                    return Pulled::Pending;
                }
                q.reads
            };
            match self.demux.try_lock() {
                Ok(d) => self.read_one(&d),
                Err(TryLockError::Poisoned(e)) => self.read_one(&e.into_inner()),
                Err(TryLockError::WouldBlock) if video => return Pulled::Pending,
                Err(TryLockError::WouldBlock) => {
                    // The picture is reading: wait for its packet to be queued.
                    let mut q = self.lock_q();
                    while q.reads == seen && !q.end {
                        let (g, t) = self
                            .cv
                            .wait_timeout(q, Duration::from_millis(20))
                            .unwrap_or_else(|e| e.into_inner());
                        q = g;
                        if t.timed_out() {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Moves the demuxer to a key frame before `ms` and empties the queues. The sound decoder
    /// then drops the samples before `ms` itself.
    pub fn seek(&self, ms: u64) -> Result<(), c_int> {
        let d = self.demux();
        let stream = {
            let q = self.lock_q();
            if q.video_stream >= 0 {
                q.video_stream
            } else {
                q.audio_stream
            }
        };
        let to = ms.saturating_sub(SEEK_MARGIN_MS) as i64;
        // SAFETY: valid demuxer under its lock.
        let r = if stream >= 0 {
            unsafe { ffi::nld_seek(d.as_ptr(), stream, to) }
        } else {
            0
        };
        let mut q = self.lock_q();
        q.audio.clear();
        q.audio_bytes = 0;
        q.clear_video();
        q.skip_to_key = true;
        q.end = false;
        q.epoch += 1;
        self.cv.notify_all();
        if r < 0 { Err(r) } else { Ok(()) }
    }
}

/// `NlPull` of the sound decoder; `opaque` is a [`Session`] the decoder keeps alive.
///
/// # Safety
/// `opaque` points to a live `Session` and `dst` to an `AVPacket`.
pub(crate) unsafe extern "C" fn pull_audio(opaque: *mut c_void, dst: *mut c_void) -> c_int {
    // SAFETY: see the function's contract.
    let s = unsafe { &*opaque.cast::<Session>() };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.pull(false))) {
        Ok(Pulled::Packet(p)) => {
            // SAFETY: `dst` is the decoder's packet.
            unsafe { p.move_into(dst) };
            1
        }
        Ok(Pulled::End | Pulled::Pending) => 0,
        Err(_) => -1,
    }
}

/// `NlPull` of the picture decoder; `opaque` is a [`Session`] the worker keeps alive.
///
/// # Safety
/// As [`pull_audio`].
pub(crate) unsafe extern "C" fn pull_video(opaque: *mut c_void, dst: *mut c_void) -> c_int {
    // SAFETY: see the function's contract.
    let s = unsafe { &*opaque.cast::<Session>() };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.pull(true))) {
        Ok(Pulled::Packet(p)) => {
            // SAFETY: `dst` is the decoder's packet.
            unsafe { p.move_into(dst) };
            1
        }
        Ok(Pulled::End) => 0,
        Ok(Pulled::Pending) => 2,
        Err(_) => -1,
    }
}

/// The C callback type `NlPull`.
pub(crate) type NlPull = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;

/// `opaque` for a pull callback: the session behind an `Arc` its user keeps.
pub(crate) fn opaque(s: &Arc<Session>) -> *mut c_void {
    Arc::as_ptr(s).cast_mut().cast()
}
