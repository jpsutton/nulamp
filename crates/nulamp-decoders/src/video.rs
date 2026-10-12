//! Video playback (09 §8): the picture of a video file, decoded by FFmpeg on a worker thread and
//! paced against the audio clock.
//!
//! The sound and the picture of a video file share one demuxer ([`crate::demux`], decision
//! V10), so the file is read once, from disk or from a server. The FFmpeg decoder opens it for
//! the sound and registers it under the playlist location; [`VideoPlayer::open`] finds it there
//! and decodes the picture from the same packets. The user interface tells it the heard
//! position ([`VideoPlayer::set_clock`]) and the picture size it wants
//! ([`VideoPlayer::set_output`]); the worker decodes a few frames ahead, picks the frame for the
//! position, scales it and publishes it ([`VideoPlayer::frame`]). Seeks come from the sound;
//! the picture follows. Decoding the picture never holds up the sound.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Instant;

/// Video file extensions (09 §1). Their sound plays through the FFmpeg decoder; their picture
/// through [`VideoPlayer`].
pub const EXTENSIONS: &str = "avi;mkv;webm;mp4;m4v;mov;flv;nsv;wmv;asf;mpg;mpeg;m2v;ogv";

/// Whether nulamp was built with video support (the FFmpeg libraries).
pub fn available() -> bool {
    cfg!(nulamp_ffmpeg)
}

/// Whether a location is a local file with a video extension. Such files may still have no
/// picture (an audio-only MP4); [`VideoPlayer`] finds out.
pub fn is_video_location(location: &str) -> bool {
    if crate::symphonia_decoder::local_path(location).is_none() {
        return false;
    }
    crate::extension_of(location)
        .is_some_and(|ext| EXTENSIONS.split(';').any(|e| e.eq_ignore_ascii_case(&ext)))
}

/// What the video stream is.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    /// Coded picture size in pixels.
    pub width: u32,
    pub height: u32,
    /// Display aspect ratio (width / height of the picture as shown).
    pub aspect: f64,
    pub fps: Option<f64>,
    pub length_ms: Option<u64>,
    /// Codec description, for example "H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10".
    pub codec: String,
    /// Description of every video stream, for the video track menu.
    pub streams: Vec<String>,
    /// The stream shown (index into `streams`).
    pub current: usize,
    /// Description of every audio stream, for the audio track menu.
    pub audio_streams: Vec<String>,
    /// The audio stream heard: the chosen one, else the file's default.
    pub audio_current: Option<usize>,
}

/// A scaled picture ready to show: `width × height` pixels of `0x00RRGGBB`.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    /// Time of the frame in ms from the start of the file.
    pub pts_ms: u64,
}

/// State of a [`VideoPlayer`].
#[derive(Debug, Clone, PartialEq)]
pub enum VideoState {
    /// The worker is opening the file.
    Opening,
    Ready(VideoInfo),
    /// The file has no picture or cannot be decoded (the message says why).
    Failed(String),
}

/// Settings and results shared with the worker.
struct Ctl {
    quit: bool,
    /// Heard position at `clock_at`, and whether it advances.
    clock_ms: u64,
    clock_at: Instant,
    playing: bool,
    /// Wanted picture size (0 × 0: none) and vertical flip.
    out: (u32, u32),
    flip: bool,
    /// Video stream to switch to.
    select: Option<usize>,
    state: VideoState,
    frame: Option<Arc<VideoFrame>>,
    /// Incremented with every published frame.
    seq: u64,
}

impl Ctl {
    /// The position the picture should show now.
    fn target(&self) -> u64 {
        if self.playing {
            self.clock_ms + self.clock_at.elapsed().as_millis() as u64
        } else {
            self.clock_ms
        }
    }
}

struct Shared {
    ctl: Mutex<Ctl>,
    wake: Condvar,
}

impl Shared {
    fn new() -> Self {
        Self {
            ctl: Mutex::new(Ctl {
                quit: false,
                clock_ms: 0,
                clock_at: Instant::now(),
                playing: false,
                out: (0, 0),
                flip: false,
                select: None,
                state: VideoState::Opening,
                frame: None,
                seq: 0,
            }),
            wake: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Ctl> {
        self.ctl.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The picture of one video file. Dropping it stops the worker.
pub struct VideoPlayer {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

/// The clock is re-anchored when the reported position and the running clock differ by more
/// than this (the engine reports the heard position in output-block steps).
const CLOCK_SLACK_MS: u64 = 60;

impl VideoPlayer {
    /// Starts showing the picture of the playlist entry `location` on a worker thread: a local
    /// video file, or a media library video (`library://…`). The worker waits for the sound
    /// decoder of that location to open the file and then shares its demuxer, following it when
    /// the sound is reopened (another audio stream). Check [`Self::state`] for the outcome.
    pub fn open(location: &str) -> Self {
        let shared = Arc::new(Shared::new());
        let worker = start_worker(location.to_string(), shared.clone());
        Self { shared, worker }
    }

    pub fn state(&self) -> VideoState {
        self.shared.lock().state.clone()
    }

    /// The heard position of the file and whether it advances. Call it often; small
    /// differences from the running clock are ignored so the picture moves smoothly.
    pub fn set_clock(&self, position_ms: u64, playing: bool) {
        let mut c = self.shared.lock();
        let running = c.target();
        if c.playing == playing && running.abs_diff(position_ms) <= CLOCK_SLACK_MS {
            return;
        }
        c.clock_ms = position_ms;
        c.clock_at = Instant::now();
        c.playing = playing;
        drop(c);
        self.shared.wake.notify_all();
    }

    /// The picture size to produce (the caller keeps the aspect ratio), and vertical flip.
    pub fn set_output(&self, width: u32, height: u32, flip: bool) {
        let mut c = self.shared.lock();
        if c.out == (width, height) && c.flip == flip {
            return;
        }
        c.out = (width, height);
        c.flip = flip;
        drop(c);
        self.shared.wake.notify_all();
    }

    /// Shows the `index`-th video stream of the file, from its next key frame.
    pub fn select_stream(&self, index: usize) {
        self.shared.lock().select = Some(index);
        self.shared.wake.notify_all();
    }

    /// The latest picture and its sequence number (changes with every new picture).
    pub fn frame(&self) -> (u64, Option<Arc<VideoFrame>>) {
        let c = self.shared.lock();
        (c.seq, c.frame.clone())
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.shared.lock().quit = true;
        self.shared.wake.notify_all();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

#[cfg(not(nulamp_ffmpeg))]
fn start_worker(_key: String, shared: Arc<Shared>) -> Option<JoinHandle<()>> {
    shared.lock().state = VideoState::Failed("nulamp was built without FFmpeg".into());
    None
}

#[cfg(nulamp_ffmpeg)]
fn start_worker(key: String, shared: Arc<Shared>) -> Option<JoinHandle<()>> {
    let s = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("nulamp-video".into())
        .spawn(move || ffmpeg::run(&key, &s));
    match spawned {
        Ok(h) => Some(h),
        Err(e) => {
            shared.lock().state = VideoState::Failed(format!("cannot start: {e}"));
            None
        }
    }
}

#[cfg(nulamp_ffmpeg)]
mod ffmpeg {
    use std::collections::VecDeque;
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::ptr::NonNull;
    use std::sync::Arc;
    use std::time::Duration;

    use super::{Shared, VideoFrame, VideoInfo, VideoState};
    use crate::demux::{self, Session};

    mod ffi {
        use std::ffi::{c_char, c_int, c_void};

        #[repr(C)]
        pub struct Nlfv {
            _private: [u8; 0],
        }

        /// Mirrors `NlfvInfo` in `video.c`.
        #[repr(C)]
        pub struct NlfvInfo {
            pub width: i32,
            pub height: i32,
            pub dar_num: i32,
            pub dar_den: i32,
            pub fps_num: i32,
            pub fps_den: i32,
            pub streams: i32,
            pub current: i32,
            pub audio_streams: i32,
            pub audio_best: i32,
            pub length_ms: i64,
            pub codec: [c_char; 128],
        }

        unsafe extern "C" {
            pub fn nlfv_open_demux(
                d: *mut crate::demux::NlDemux,
                pull: crate::demux::NlPull,
                opaque: *mut c_void,
                info: *mut NlfvInfo,
                err: *mut c_char,
                err_len: c_int,
            ) -> *mut Nlfv;
            pub fn nlfv_close(h: *mut Nlfv);
            pub fn nlfv_stream_name(h: *mut Nlfv, i: c_int, out: *mut c_char, len: c_int) -> c_int;
            pub fn nlfv_audio_name(h: *mut Nlfv, i: c_int, out: *mut c_char, len: c_int) -> c_int;
            pub fn nlfv_select(h: *mut Nlfv, which: c_int, info: *mut NlfvInfo) -> c_int;
            pub fn nlfv_reset(h: *mut Nlfv);
            pub fn nlfv_stream(h: *const Nlfv) -> c_int;
            pub fn nlfv_next(h: *mut Nlfv, pts_ms: *mut i64, status: *mut c_int) -> *mut c_void;
            pub fn nlfv_frame_free(frame: *mut c_void);
            pub fn nlfv_frame_size(frame: *const c_void, w: *mut c_int, h: *mut c_int);
            pub fn nlfv_scale(
                h: *mut Nlfv,
                frame: *const c_void,
                out: *mut u32,
                dst_w: c_int,
                dst_h: c_int,
                flip: c_int,
            ) -> c_int;
        }
    }

    /// Owned shim handle.
    struct Handle(NonNull<ffi::Nlfv>);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: created by nlfv_open_demux and closed once, before its session.
            unsafe { ffi::nlfv_close(self.0.as_ptr()) };
        }
    }

    /// A decoded frame (FFmpeg's), not scaled yet.
    struct Raw {
        frame: NonNull<c_void>,
        pts: u64,
    }

    impl Drop for Raw {
        fn drop(&mut self) {
            // SAFETY: returned by nlfv_next and freed once.
            unsafe { ffi::nlfv_frame_free(self.frame.as_ptr()) };
        }
    }

    /// Frames decoded ahead of the position.
    const AHEAD_FRAMES: usize = 6;
    /// How far past the position frames are decoded.
    const AHEAD_MS: u64 = 120;

    fn info_of(h: &Handle, i: &ffi::NlfvInfo, location: &str) -> VideoInfo {
        // SAFETY: `codec` is NUL-terminated by the shim's snprintf.
        let codec = unsafe { CStr::from_ptr(i.codec.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let names = |count: i32,
                     f: unsafe extern "C" fn(
            *mut ffi::Nlfv,
            c_int,
            *mut c_char,
            c_int,
        ) -> c_int| {
            (0..count.max(0))
                .map(|n| {
                    let mut buf = [0 as c_char; 160];
                    // SAFETY: valid handle and buffer of the given size.
                    unsafe { f(h.0.as_ptr(), n, buf.as_mut_ptr(), 160) };
                    // SAFETY: NUL-terminated by snprintf (the buffer starts zeroed).
                    let name = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
                    format!("{}: {name}", n + 1)
                })
                .collect::<Vec<_>>()
        };
        let streams = names(i.streams, ffi::nlfv_stream_name);
        let audio_streams = names(i.audio_streams, ffi::nlfv_audio_name);
        let audio_current = crate::ffmpeg_decoder::audio_track(location)
            .filter(|n| *n < audio_streams.len())
            .or_else(|| usize::try_from(i.audio_best).ok());
        let aspect = if i.dar_num > 0 && i.dar_den > 0 {
            i.dar_num as f64 / i.dar_den as f64
        } else if i.height > 0 {
            i.width as f64 / i.height as f64
        } else {
            0.0
        };
        VideoInfo {
            width: i.width.max(0) as u32,
            height: i.height.max(0) as u32,
            aspect,
            fps: (i.fps_num > 0 && i.fps_den > 0).then(|| i.fps_num as f64 / i.fps_den as f64),
            length_ms: u64::try_from(i.length_ms).ok(),
            codec,
            streams,
            current: i.current.max(0) as usize,
            audio_streams,
            audio_current,
        }
    }

    /// Opens the picture decoder on `session`'s demuxer.
    fn open(session: &Arc<Session>, location: &str) -> Result<(Handle, VideoInfo), String> {
        // SAFETY: plain-data struct; the shim fills it in.
        let mut info: ffi::NlfvInfo = unsafe { std::mem::zeroed() };
        let mut err = [0 as c_char; 256];
        let d = session.demux();
        // SAFETY: valid demuxer under its lock; the pull callback's session outlives the
        // handle (the worker holds it); out buffers of the given sizes.
        let h = unsafe {
            ffi::nlfv_open_demux(
                d.as_ptr(),
                demux::pull_video,
                demux::opaque(session),
                &mut info,
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        drop(d);
        let Some(h) = NonNull::new(h) else {
            // SAFETY: the shim writes a NUL-terminated message (the buffer starts zeroed).
            let msg = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy();
            return Err(msg.into_owned());
        };
        let h = Handle(h);
        // SAFETY: valid handle.
        session.set_video_stream(unsafe { ffi::nlfv_stream(h.0.as_ptr()) });
        let info = info_of(&h, &info, location);
        Ok((h, info))
    }

    /// Outcome of decoding one frame.
    enum Step {
        Frame,
        /// No packet yet (the sound side is reading, or is far behind).
        Pending,
        End,
    }

    /// The decoding side of the worker.
    struct Worker {
        /// Declared before `session`: the handle reads the session's packets.
        h: Handle,
        session: Arc<Session>,
        location: String,
        info: VideoInfo,
        queue: VecDeque<Raw>,
        current: Option<Raw>,
        eof: bool,
        /// The session's seek count the frames belong to.
        epoch: u64,
        /// Time of the last decoded frame, for frames without a time stamp.
        last_pts: Option<u64>,
        /// (frame time, size, flip) of the published picture.
        shown: Option<(u64, (u32, u32), bool)>,
    }

    impl Worker {
        fn frame_ms(&self) -> u64 {
            self.info
                .fps
                .filter(|f| *f > 0.0)
                .map(|f| (1000.0 / f).round() as u64)
                .unwrap_or(40)
        }

        /// Forgets the decoded frames (the sound seeked, or another stream was chosen).
        fn reset(&mut self) {
            // SAFETY: valid handle.
            unsafe { ffi::nlfv_reset(self.h.0.as_ptr()) };
            self.queue.clear();
            self.current = None;
            self.last_pts = None;
            self.eof = false;
            self.shown = None;
        }

        fn decode(&mut self) -> Step {
            let mut pts = -1i64;
            let mut status: c_int = 0;
            // SAFETY: valid handle and out pointers.
            let f = unsafe { ffi::nlfv_next(self.h.0.as_ptr(), &mut pts, &mut status) };
            let Some(frame) = NonNull::new(f) else {
                if status == 1 {
                    return Step::Pending;
                }
                if status < 0 {
                    log::debug!("video decoding stopped: FFmpeg error {status}");
                }
                self.eof = true;
                return Step::End;
            };
            let mut raw = Raw { frame, pts: 0 };
            if self.session.epoch() != self.epoch {
                // Decoded from packets before a seek.
                return Step::Pending;
            }
            let pts = u64::try_from(pts)
                .ok()
                .or_else(|| self.last_pts.map(|p| p + self.frame_ms()))
                .unwrap_or(0);
            self.last_pts = Some(pts);
            raw.pts = pts;
            self.queue.push_back(raw);
            Step::Frame
        }

        fn newest(&self) -> Option<u64> {
            self.queue.back().or(self.current.as_ref()).map(|f| f.pts)
        }

        /// Brings the decoded frames up to `target` and picks the frame to show.
        fn advance(&mut self, target: u64) {
            let epoch = self.session.epoch();
            if epoch != self.epoch {
                self.reset();
                self.epoch = epoch;
            }
            while !self.eof
                && self.queue.len() < AHEAD_FRAMES
                && self.newest().is_none_or(|n| n <= target + AHEAD_MS)
            {
                if !matches!(self.decode(), Step::Frame) {
                    break;
                }
                // Frames that are already late are dropped as they come.
                while self.queue.len() > 1 && self.queue[1].pts <= target {
                    self.queue.pop_front();
                }
            }
            while self.queue.front().is_some_and(|f| f.pts <= target) {
                self.current = self.queue.pop_front();
            }
            // Before the first frame's time (or right after a seek), show the first frame.
            if self.current.is_none() {
                self.current = self.queue.pop_front();
            }
        }

        /// Scales the current frame when it, the size or the flip changed.
        fn picture(&mut self, out: (u32, u32), flip: bool) -> Option<VideoFrame> {
            let cur = self.current.as_ref()?;
            if out.0 == 0 || out.1 == 0 || self.shown == Some((cur.pts, out, flip)) {
                return None;
            }
            let mut pixels = vec![0u32; out.0 as usize * out.1 as usize];
            // SAFETY: valid handle and frame; `pixels` holds out.0 × out.1 pixels.
            let r = unsafe {
                ffi::nlfv_scale(
                    self.h.0.as_ptr(),
                    cur.frame.as_ptr(),
                    pixels.as_mut_ptr(),
                    out.0 as c_int,
                    out.1 as c_int,
                    flip as c_int,
                )
            };
            if r < 0 {
                let (mut w, mut h) = (0, 0);
                // SAFETY: valid frame and out pointers.
                unsafe { ffi::nlfv_frame_size(cur.frame.as_ptr(), &mut w, &mut h) };
                log::debug!("cannot scale a {w}x{h} frame to {out:?}: FFmpeg error {r}");
                return None;
            }
            self.shown = Some((cur.pts, out, flip));
            Some(VideoFrame {
                width: out.0,
                height: out.1,
                pixels,
                pts_ms: cur.pts,
            })
        }

        /// Switches to another video stream; its picture starts at its next key frame.
        fn select(&mut self, index: usize) -> bool {
            // SAFETY: plain-data struct; the shim fills it in.
            let mut i: ffi::NlfvInfo = unsafe { std::mem::zeroed() };
            // SAFETY: valid handle and out pointer.
            let r = unsafe { ffi::nlfv_select(self.h.0.as_ptr(), index as c_int, &mut i) };
            if r < 0 {
                log::debug!("cannot switch to video stream {index}: FFmpeg error {r}");
                return false;
            }
            // SAFETY: valid handle.
            self.session
                .set_video_stream(unsafe { ffi::nlfv_stream(self.h.0.as_ptr()) });
            self.info = info_of(&self.h, &i, &self.location);
            self.queue.clear();
            self.last_pts = None;
            self.eof = false;
            self.shown = None;
            true
        }

        /// Shows pictures until the worker quits (true) or the sound decoder lets go of the
        /// session (false).
        fn play(&mut self, shared: &Shared) -> bool {
            loop {
                let (target, out, flip, select) = {
                    let mut c = shared.lock();
                    if c.quit {
                        return true;
                    }
                    (c.target(), c.out, c.flip, c.select.take())
                };
                if self.session.closed() {
                    return false;
                }
                if let Some(i) = select
                    && i != self.info.current
                    && self.select(i)
                {
                    shared.lock().state = VideoState::Ready(self.info.clone());
                }
                self.advance(target);
                let picture = self.picture(out, flip);
                let mut c = shared.lock();
                if c.quit {
                    return true;
                }
                if let Some(p) = picture {
                    c.frame = Some(Arc::new(p));
                    c.seq += 1;
                }
                // Sleep until the next frame is due, or until something changes.
                let wait = match (c.playing, self.queue.front()) {
                    (true, Some(next)) => {
                        Duration::from_millis(next.pts.saturating_sub(c.target()))
                            .clamp(Duration::from_millis(1), Duration::from_millis(20))
                    }
                    (_, None) if !self.eof => Duration::from_millis(5),
                    _ => Duration::from_millis(100),
                };
                let _unused = shared
                    .wake
                    .wait_timeout(c, wait)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }
    }

    /// Waits up to `ms`; true when the worker should quit.
    fn pause(shared: &Shared, ms: u64) -> bool {
        let c = shared.lock();
        if c.quit {
            return true;
        }
        let (c, _) = shared
            .wake
            .wait_timeout(c, Duration::from_millis(ms))
            .unwrap_or_else(|e| e.into_inner());
        c.quit
    }

    pub(super) fn run(location: &str, shared: &Arc<Shared>) {
        loop {
            // The sound decoder opens the file; a reopened sound (another audio stream) opens
            // it again.
            let Some(session) = Session::find(location) else {
                if pause(shared, 20) {
                    return;
                }
                continue;
            };
            let (h, info) = match open(&session, location) {
                Ok(x) => x,
                Err(e) => {
                    shared.lock().state = VideoState::Failed(e);
                    while !session.closed() {
                        if pause(shared, 50) {
                            return;
                        }
                    }
                    continue;
                }
            };
            shared.lock().state = VideoState::Ready(info.clone());
            session.attach();
            let epoch = session.epoch();
            let mut w = Worker {
                h,
                session: session.clone(),
                location: location.to_string(),
                info,
                queue: VecDeque::new(),
                current: None,
                eof: false,
                epoch,
                last_pts: None,
                shown: None,
            };
            let quit = w.play(shared);
            session.detach();
            if quit {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_locations() {
        assert!(is_video_location("/a/b.MKV"));
        assert!(is_video_location("file:///a/b.mp4"));
        assert!(!is_video_location("/a/b.mp3"));
        assert!(!is_video_location("http://x/y.mp4"));
    }

    /// Video fixtures made with the `ffmpeg` tool (MPEG-4 part 2, 25 fps, 320×240): the first
    /// second is red over blue, the second second green. Tests are skipped without the tool.
    #[cfg(nulamp_ffmpeg)]
    mod decoding {
        use super::super::*;
        use crate::{Decoder, ReadStatus, Registry};
        use std::path::PathBuf;
        use std::process::Command;
        use std::time::{Duration, Instant};

        struct Fixture(PathBuf);

        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }

        impl Fixture {
            fn location(&self) -> &str {
                self.0.to_str().unwrap()
            }
        }

        /// `audio`: add a 440 Hz sine. `video`: add the picture.
        fn fixture(name: &str, video: bool, audio: bool) -> Option<Fixture> {
            let path =
                std::env::temp_dir().join(format!("nulamp-video-{}-{name}", std::process::id()));
            let mut cmd = Command::new("ffmpeg");
            cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
            if video {
                cmd.args([
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=blue:s=320x240:r=25:d=1,drawbox=x=0:y=0:w=320:h=120:color=red:t=fill",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=green:s=320x240:r=25:d=1",
                ]);
            }
            if audio {
                cmd.args([
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=44100:duration=2",
                ]);
            }
            if video {
                cmd.args([
                    "-filter_complex",
                    "[0:v][1:v]concat=n=2:v=1:a=0[v]",
                    "-map",
                    "[v]",
                    "-c:v",
                    "mpeg4",
                    "-q:v",
                    "2",
                ]);
            }
            if audio {
                cmd.args(["-map", if video { "2:a" } else { "0:a" }, "-c:a", "mp2"]);
            }
            let ok = cmd.arg(&path).status().is_ok_and(|s| s.success());
            if !ok {
                eprintln!("skipping: ffmpeg cannot write {name}");
                return None;
            }
            Some(Fixture(path))
        }

        fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> T {
            let until = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(v) = f() {
                    return v;
                }
                assert!(Instant::now() < until, "timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn ready(v: &VideoPlayer) -> VideoInfo {
            wait_for(|| match v.state() {
                VideoState::Opening => None,
                VideoState::Ready(i) => Some(i),
                VideoState::Failed(e) => panic!("{e}"),
            })
        }

        /// The first picture showing the frame for `ms`.
        fn picture_at(v: &VideoPlayer, ms: u64) -> Arc<VideoFrame> {
            v.set_clock(ms, false);
            wait_for(|| v.frame().1.filter(|f| f.pts_ms <= ms && f.pts_ms + 40 > ms))
        }

        fn rgb(p: u32) -> (u8, u8, u8) {
            ((p >> 16) as u8, (p >> 8) as u8, p as u8)
        }

        fn is_red(p: u32) -> bool {
            let (r, g, b) = rgb(p);
            r > 200 && g < 60 && b < 60
        }

        fn is_blue(p: u32) -> bool {
            let (r, g, b) = rgb(p);
            b > 200 && r < 60 && g < 60
        }

        fn is_green(p: u32) -> bool {
            let (r, g, b) = rgb(p);
            g > 100 && r < 60 && b < 60
        }

        #[test]
        fn pictures_follow_the_clock() {
            let Some(f) = fixture("clock.avi", true, true) else {
                return;
            };
            // The sound decoder opens the file; the picture shares its demuxer.
            let mut d = Registry::with_defaults().open(f.location()).unwrap();
            let v = VideoPlayer::open(f.location());
            let info = ready(&v);
            assert_eq!((info.width, info.height), (320, 240));
            assert!((info.aspect - 4.0 / 3.0).abs() < 1e-6);
            assert_eq!(info.fps, Some(25.0));
            assert!(info.length_ms.unwrap().abs_diff(2000) < 100);
            assert_eq!(info.streams.len(), 1);
            assert!(info.codec.contains("MPEG-4"), "{}", info.codec);

            v.set_output(160, 120, false);
            let p = picture_at(&v, 500);
            assert_eq!((p.width, p.height), (160, 120));
            assert!(is_red(p.pixels[10 * 160 + 80]));
            assert!(is_blue(p.pixels[110 * 160 + 80]));
            // Forward the picture follows the clock; back it follows the sound's seek.
            assert!(is_green(picture_at(&v, 1500).pixels[60 * 160 + 80]));
            d.seek(200).unwrap();
            let p = picture_at(&v, 200);
            assert!(is_red(p.pixels[10 * 160 + 80]));
            // Flipped.
            v.set_output(160, 120, true);
            let p = wait_for(|| {
                v.frame()
                    .1
                    .filter(|f| f.pts_ms <= 200 && is_blue(f.pixels[10 * 160 + 80]))
            });
            assert!(is_red(p.pixels[110 * 160 + 80]));
            // A new size without a new frame.
            v.set_output(80, 60, false);
            let p = wait_for(|| v.frame().1.filter(|f| f.width == 80));
            assert_eq!(p.pixels.len(), 80 * 60);

            // Playing: the picture advances with the clock.
            d.seek(0).unwrap();
            v.set_clock(0, true);
            std::thread::sleep(Duration::from_millis(400));
            let (_, p) = v.frame();
            let pts = p.unwrap().pts_ms;
            assert!((200..=800).contains(&pts), "{pts}");
        }

        #[test]
        fn video_without_audio_plays_silence() {
            let Some(f) = fixture("silent.mkv", true, false) else {
                return;
            };
            let r = Registry::with_defaults();
            let mut d = r.open(f.location()).unwrap();
            let info = d.info().clone();
            assert_eq!((info.sample_rate, info.channels), (48_000, 2));
            assert!(info.length_ms.unwrap().abs_diff(2000) < 100);
            let mut out = Vec::new();
            while let ReadStatus::Frames(_) = d.read(&mut out).unwrap() {}
            assert!((out.len() / 2).abs_diff(96_000) < 4800);
            assert!(out.iter().all(|s| *s == 0.0));
            assert_eq!(d.seek(1000).unwrap(), 1000);
            let mut rest = Vec::new();
            while let ReadStatus::Frames(_) = d.read(&mut rest).unwrap() {}
            assert!((rest.len() / 2).abs_diff(48_000) < 4800);
            // Its picture opens as usual.
            let v = VideoPlayer::open(f.location());
            assert_eq!(ready(&v).width, 320);
        }

        #[test]
        fn video_files_play_their_audio() {
            let Some(f) = fixture("sound.avi", true, true) else {
                return;
            };
            let r = Registry::with_defaults();
            assert!(r.is_supported_extension("avi"));
            assert!(r.is_supported_extension("mkv"));
            let mut d: Box<dyn Decoder> = r.open(f.location()).unwrap();
            assert_eq!(d.info().sample_rate, 44_100);
            let mut out = Vec::new();
            while let ReadStatus::Frames(_) = d.read(&mut out).unwrap() {}
            assert!(out.iter().any(|s| s.abs() > 0.1));
        }

        #[test]
        fn audio_tracks_can_be_chosen() {
            let path = std::env::temp_dir()
                .join(format!("nulamp-video-{}-tracks.mkv", std::process::id()));
            let ok = Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                ])
                .arg("color=c=blue:s=64x48:r=10:d=1")
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=44100:duration=1",
                ])
                .args(["-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono", "-t", "1"])
                .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-c:v", "mpeg4"])
                .args(["-c:a", "mp2", "-metadata:s:a:1", "language=fra"])
                .arg(&path)
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                eprintln!("skipping: ffmpeg cannot write tracks.mkv");
                return;
            }
            let f = Fixture(path);
            let loc = f.location().to_string();
            let sound = Registry::with_defaults().open(&loc).unwrap();
            let v = VideoPlayer::open(&loc);
            let info = ready(&v);
            assert_eq!(info.audio_streams.len(), 2);
            assert!(
                info.audio_streams[1].contains("[fra]"),
                "{:?}",
                info.audio_streams
            );
            assert_eq!(info.audio_current, Some(0));
            let peak = |loc: &str| {
                let mut d = Registry::with_defaults().open(loc).unwrap();
                let mut out = Vec::new();
                while let ReadStatus::Frames(_) = d.read(&mut out).unwrap() {}
                out.iter().fold(0f32, |m, s| m.max(s.abs()))
            };
            assert!(peak(&loc) > 0.1);
            crate::ffmpeg_decoder::set_audio_track(&loc, Some(1));
            assert_eq!(peak(&loc), 0.0);
            // The sound reopens with the other stream; the picture follows it.
            drop(sound);
            let _sound = Registry::with_defaults().open(&loc).unwrap();
            wait_for(|| match v.state() {
                VideoState::Ready(i) if i.audio_current == Some(1) => Some(()),
                _ => None,
            });
            crate::ffmpeg_decoder::set_audio_track(&loc, None);
            assert!(peak(&loc) > 0.1);
        }

        /// Requests seen by [`range_server`]: (path, Authorization, Range).
        type Seen = Arc<std::sync::Mutex<Vec<(String, Option<String>, Option<String>)>>>;

        /// Serves `body` at every path with Range support (`stall`: send the headers and
        /// nothing else). Nothing leaves 127.0.0.1.
        fn range_server(body: Vec<u8>, stall: bool) -> (String, Seen) {
            use std::io::{BufRead, BufReader, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
            let seen: Seen = Arc::default();
            let (body, log) = (Arc::new(body), seen.clone());
            std::thread::spawn(move || {
                for s in listener.incoming() {
                    let Ok(mut s) = s else { continue };
                    let (body, log) = (body.clone(), log.clone());
                    std::thread::spawn(move || {
                        let mut r = BufReader::new(s.try_clone().unwrap());
                        let mut line = String::new();
                        r.read_line(&mut line).unwrap();
                        let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                        let (mut auth, mut range) = (None, None);
                        loop {
                            let mut l = String::new();
                            if r.read_line(&mut l).unwrap_or(0) == 0 || l.trim().is_empty() {
                                break;
                            }
                            if let Some((k, v)) = l.split_once(':') {
                                match k.trim().to_ascii_lowercase().as_str() {
                                    "authorization" => auth = Some(v.trim().to_string()),
                                    "range" => range = Some(v.trim().to_string()),
                                    _ => {}
                                }
                            }
                        }
                        let start = range
                            .as_deref()
                            .and_then(|r| {
                                r.strip_prefix("bytes=")?.trim_end_matches('-').parse().ok()
                            })
                            .unwrap_or(0usize)
                            .min(body.len());
                        log.lock().unwrap().push((path, auth, range));
                        let total = body.len();
                        let head = if start > 0 {
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes \
                                 {start}-{}/{total}\r\n",
                                total - 1
                            )
                        } else {
                            "HTTP/1.1 200 OK\r\n".to_string()
                        };
                        let _ = s.write_all(
                            format!(
                                "{head}Content-Length: {}\r\nAccept-Ranges: bytes\r\n\
                                 Content-Type: video/x-matroska\r\n\r\n",
                                total - start
                            )
                            .as_bytes(),
                        );
                        if stall {
                            std::thread::sleep(Duration::from_secs(30));
                        } else {
                            let _ = s.write_all(&body[start..]);
                        }
                    });
                }
            });
            (base, seen)
        }

        /// Every frame of a network stream, waiting while it buffers.
        fn drain_stream(d: &mut dyn Decoder) -> Vec<f32> {
            let mut out = Vec::new();
            let until = Instant::now() + Duration::from_secs(20);
            loop {
                match d.read(&mut out).unwrap() {
                    ReadStatus::Frames(_) => {}
                    ReadStatus::Pending => std::thread::sleep(Duration::from_millis(2)),
                    ReadStatus::End => return out,
                }
                assert!(Instant::now() < until, "timed out");
            }
        }

        /// A media server's video: one download feeds the sound (the stream input, FFmpeg) and
        /// the picture, with the authorization header.
        #[test]
        fn remote_videos_play_over_http() {
            use crate::stream::{StreamFactory, StreamOptions};
            // Matroska keeps its index at the end; this MP4 too (no `faststart`).
            for name in ["remote.mkv", "remote.mp4"] {
                let Some(f) = fixture(name, true, true) else {
                    return;
                };
                let (base, seen) = range_server(std::fs::read(f.location()).unwrap(), false);
                let url = format!("{base}/Videos/1/stream?static=true");
                let key = format!("library://p/{name}");
                let headers = vec![(
                    "Authorization".to_string(),
                    "MediaBrowser Token=\"t\"".to_string(),
                )];
                let factory = StreamFactory::default();
                let options = StreamOptions {
                    headers,
                    keep_title: true,
                    finite: true,
                    video: Some(key.clone()),
                    audio_track: None,
                };
                let mut d = factory.open_stream_with(&url, &options).unwrap();
                let v = VideoPlayer::open(&key);
                let vi = ready(&v);
                assert_eq!((vi.width, vi.height), (320, 240), "{name}");
                v.set_output(160, 120, false);
                assert!(is_red(picture_at(&v, 500).pixels[10 * 160 + 80]), "{name}");
                assert!(
                    is_green(picture_at(&v, 1500).pixels[60 * 160 + 80]),
                    "{name}"
                );
                d.seek(200).unwrap();
                assert!(is_red(picture_at(&v, 200).pixels[10 * 160 + 80]), "{name}");

                let rest = drain_stream(&mut d).len();
                let ch = d.info().channels as usize;
                let rest = rest / ch;
                assert_eq!(d.info().sample_rate, 44_100, "{name}");
                assert!(d.info().seekable, "{name}");
                assert!(
                    rest.abs_diff(79_380) < 4410,
                    "{name}: {rest} frames from 200 ms"
                );
                assert_eq!(d.seek(1000).unwrap(), 1000);
                let all = drain_stream(&mut d);
                assert!(
                    (all.len() / ch).abs_diff(44_100) < 4410,
                    "{name}: after a seek"
                );
                assert!(all.iter().any(|s| s.abs() > 0.1), "{name}");
                drop(v);
                drop(d);

                // One download from the start; later requests only for seeks.
                let seen = seen.lock().unwrap();
                assert!(
                    seen.iter()
                        .all(|(_, a, _)| a.as_deref() == Some("MediaBrowser Token=\"t\""))
                );
                let from_start = seen
                    .iter()
                    .filter(|(_, _, r)| r.as_deref().is_none_or(|r| r == "bytes=0-"))
                    .count();
                assert!(from_start <= 2, "{name}: {seen:?}");
                assert!(seen.len() <= 6, "{name}: {seen:?}");
            }
        }

        /// A picture whose sound never opens waits, and closes at once.
        #[test]
        fn pictures_wait_for_their_sound() {
            let v = VideoPlayer::open("/no/such/video.mkv");
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(v.state(), VideoState::Opening);
            let t = Instant::now();
            drop(v);
            assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
        }

        #[test]
        fn audio_only_files_have_no_picture() {
            let Some(f) = fixture("audio.mkv", false, true) else {
                return;
            };
            let _sound = Registry::with_defaults().open(f.location()).unwrap();
            let v = VideoPlayer::open(f.location());
            let state = wait_for(|| (v.state() != VideoState::Opening).then(|| v.state()));
            assert_eq!(state, VideoState::Failed("no video stream".into()));
            assert_eq!(v.frame(), (0, None));
        }
    }
}
