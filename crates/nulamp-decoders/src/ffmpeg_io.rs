//! Byte sources for FFmpeg (`ffmpeg/io.c`): FFmpeg reads a [`ByteSource`] through callbacks
//! instead of opening a file itself. Media-server files reach the FFmpeg decoder and the video
//! window this way, over nulamp's own HTTP client, so FFmpeg's network protocols (absent from
//! the AppImage's FFmpeg) are never used.

#[cfg(nulamp_ffmpeg)]
use std::ffi::{c_int, c_void};
#[cfg(nulamp_ffmpeg)]
use std::io::SeekFrom;
use std::io::{Read, Seek};

/// Something FFmpeg can read and seek like a file.
#[cfg_attr(not(nulamp_ffmpeg), allow(dead_code))]
pub(crate) trait ByteSource: Read + Seek + Send {
    /// Total size in bytes, when known.
    fn byte_len(&mut self) -> Option<u64>;

    /// Whether [`Seek`] works.
    fn seekable(&self) -> bool {
        true
    }
}

/// Mirrors `NlIo` in `io.h`.
#[cfg(nulamp_ffmpeg)]
#[repr(C)]
pub(crate) struct NlIo {
    opaque: *mut c_void,
    read: unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int,
    seek: Option<unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64>,
}

/// FFmpeg's `AVSEEK_SIZE`.
#[cfg(nulamp_ffmpeg)]
const AVSEEK_SIZE: c_int = 0x10000;

/// A byte source kept at a fixed address for the callbacks. Keep it alive (and unmoved inside
/// its box) until the FFmpeg handle that reads it is closed.
#[cfg(nulamp_ffmpeg)]
pub(crate) struct Io(Box<Box<dyn ByteSource>>);

#[cfg(nulamp_ffmpeg)]
impl Io {
    pub fn new(source: Box<dyn ByteSource>) -> Self {
        Self(Box::new(source))
    }

    /// The callback table for `nl_open_input`; valid while `self` lives.
    pub fn ffi(&mut self) -> NlIo {
        let seekable = self.0.seekable();
        NlIo {
            opaque: (&mut *self.0 as *mut Box<dyn ByteSource>).cast(),
            read: read_cb,
            seek: seekable.then_some(seek_cb as unsafe extern "C" fn(_, _, _) -> _),
        }
    }
}

/// # Safety
/// `opaque` is the pointer from [`Io::ffi`] and `buf` holds `size` bytes.
#[cfg(nulamp_ffmpeg)]
unsafe extern "C" fn read_cb(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    // SAFETY: see the function's contract; FFmpeg calls back on the thread using the handle.
    let source = unsafe { &mut *opaque.cast::<Box<dyn ByteSource>>() };
    let Ok(len) = usize::try_from(size) else {
        return -1;
    };
    // SAFETY: as above.
    let buf = unsafe { std::slice::from_raw_parts_mut(buf, len) };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        loop {
            match source.read(buf) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                r => break r,
            }
        }
    }));
    match r {
        Ok(Ok(n)) => c_int::try_from(n).unwrap_or(-1),
        Ok(Err(e)) => {
            log::debug!("FFmpeg input: {e}");
            -1
        }
        Err(_) => -1,
    }
}

/// # Safety
/// `opaque` is the pointer from [`Io::ffi`].
#[cfg(nulamp_ffmpeg)]
unsafe extern "C" fn seek_cb(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    // SAFETY: see the function's contract.
    let source = unsafe { &mut *opaque.cast::<Box<dyn ByteSource>>() };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if whence & AVSEEK_SIZE != 0 {
            return source.byte_len().and_then(|l| i64::try_from(l).ok());
        }
        let to = match whence {
            0 => SeekFrom::Start(u64::try_from(offset).ok()?),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return None,
        };
        match source.seek(to) {
            Ok(p) => i64::try_from(p).ok(),
            Err(e) => {
                log::debug!("FFmpeg input seek: {e}");
                None
            }
        }
    }));
    r.ok().flatten().unwrap_or(-1)
}

#[cfg(all(test, nulamp_ffmpeg))]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Mem(Cursor<Vec<u8>>);

    impl Read for Mem {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Seek for Mem {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(to)
        }
    }

    impl ByteSource for Mem {
        fn byte_len(&mut self) -> Option<u64> {
            Some(self.0.get_ref().len() as u64)
        }
    }

    #[test]
    fn callbacks() {
        let mut io = Io::new(Box::new(Mem(Cursor::new((0..100u8).collect()))));
        let t = io.ffi();
        let seek = t.seek.unwrap();
        let mut buf = [0u8; 8];
        // SAFETY: `t` belongs to `io`, which outlives the calls; `buf` holds 8 bytes.
        unsafe {
            assert_eq!(seek(t.opaque, 0, AVSEEK_SIZE), 100);
            assert_eq!(seek(t.opaque, 90, 0), 90);
            assert_eq!((t.read)(t.opaque, buf.as_mut_ptr(), 8), 8);
            assert_eq!(buf[0], 90);
            assert_eq!(seek(t.opaque, -4, 2), 96);
            assert_eq!((t.read)(t.opaque, buf.as_mut_ptr(), 8), 4);
            assert_eq!((t.read)(t.opaque, buf.as_mut_ptr(), 8), 0);
            assert_eq!(seek(t.opaque, -200, 1), -1);
        }
    }
}
