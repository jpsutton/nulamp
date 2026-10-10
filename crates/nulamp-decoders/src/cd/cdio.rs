//! CD drive access through the system libcdio (GPL; built only with the opt-in `cdio`
//! feature, docs/decisions.md CD1) using a small FFI layer.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr::NonNull;
use std::sync::Once;

use super::{CdBackend, CdDrive, CdText, CdTextEntry, SECTOR_BYTES, Toc, TocTrack};
use crate::DecodeError;

mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct CdIo {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct CdText {
        _private: [u8; 0],
    }

    pub type Track = u8;
    pub type Lsn = i32;
    /// `driver_id_t`
    pub type DriverId = c_int;
    /// `driver_return_code_t`
    pub type DriverReturn = c_int;
    /// `track_format_t`
    pub type TrackFormat = c_int;
    /// `cdtext_field_t`
    pub type CdTextField = c_int;
    pub type LogHandler = Option<unsafe extern "C" fn(level: c_int, message: *const c_char)>;

    pub const DRIVER_DEVICE: DriverId = 11;
    pub const DRIVER_OP_SUCCESS: DriverReturn = 0;
    pub const TRACK_FORMAT_AUDIO: TrackFormat = 0;
    pub const CDIO_INVALID_TRACK: Track = 0xFF;
    pub const CDIO_CDROM_LEADOUT_TRACK: Track = 0xAA;
    pub const CDIO_INVALID_LSN: Lsn = -45301;
    pub const CDTEXT_FIELD_TITLE: CdTextField = 0;
    pub const CDTEXT_FIELD_PERFORMER: CdTextField = 1;
    pub const CDIO_LOG_INFO: c_int = 2;
    pub const CDIO_LOG_WARN: c_int = 3;

    unsafe extern "C" {
        pub fn cdio_open(source: *const c_char, driver: DriverId) -> *mut CdIo;
        pub fn cdio_destroy(p: *mut CdIo);
        pub fn cdio_get_devices(driver: DriverId) -> *mut *mut c_char;
        pub fn cdio_free_device_list(list: *mut *mut c_char);
        pub fn cdio_get_first_track_num(p: *const CdIo) -> Track;
        pub fn cdio_get_num_tracks(p: *const CdIo) -> Track;
        pub fn cdio_get_track_lsn(p: *const CdIo, track: Track) -> Lsn;
        pub fn cdio_get_track_format(p: *const CdIo, track: Track) -> TrackFormat;
        pub fn cdio_get_last_session(p: *mut CdIo, lsn: *mut Lsn) -> DriverReturn;
        pub fn cdio_read_audio_sectors(
            p: *const CdIo,
            buf: *mut c_void,
            lsn: Lsn,
            blocks: u32,
        ) -> DriverReturn;
        pub fn cdio_get_cdtext(p: *mut CdIo) -> *mut CdText;
        pub fn cdtext_get_const(
            text: *const CdText,
            field: CdTextField,
            track: Track,
        ) -> *const c_char;
        pub fn cdio_log_set_handler(handler: LogHandler) -> LogHandler;
    }
}

unsafe extern "C" fn log_message(level: c_int, message: *const c_char) {
    if message.is_null() {
        return;
    }
    // SAFETY: libcdio passes a NUL-terminated message valid for this call.
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let text = text.trim_end();
    match level {
        l if l >= ffi::CDIO_LOG_WARN => log::info!("libcdio: {text}"),
        ffi::CDIO_LOG_INFO => log::debug!("libcdio: {text}"),
        _ => log::trace!("libcdio: {text}"),
    }
}

/// Routes libcdio messages (which go to stderr by default) to the log.
fn init_logging() {
    static ONCE: Once = Once::new();
    // SAFETY: the handler is a plain function valid for the whole program.
    ONCE.call_once(|| unsafe {
        ffi::cdio_log_set_handler(Some(log_message));
    });
}

/// The CD drives of the system through libcdio.
#[derive(Debug, Default, Clone, Copy)]
pub struct CdioBackend;

impl CdBackend for CdioBackend {
    fn name(&self) -> &str {
        "libcdio"
    }

    fn devices(&self) -> Vec<String> {
        init_logging();
        // SAFETY: returns a NULL-terminated list (or NULL) freed below.
        let list = unsafe { ffi::cdio_get_devices(ffi::DRIVER_DEVICE) };
        if list.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for i in 0.. {
            // SAFETY: the list is NULL-terminated; we stop at the terminator.
            let p = unsafe { *list.add(i) };
            if p.is_null() {
                break;
            }
            // SAFETY: each element is a NUL-terminated string owned by the list.
            out.push(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned());
        }
        // SAFETY: allocated by cdio_get_devices and freed once.
        unsafe { ffi::cdio_free_device_list(list) };
        out
    }

    fn open(&self, device: &str) -> Result<Box<dyn CdDrive>, DecodeError> {
        init_logging();
        let c = CString::new(device).map_err(|_| DecodeError::NotFound(device.into()))?;
        // SAFETY: valid NUL-terminated device name.
        let p = unsafe { ffi::cdio_open(c.as_ptr(), ffi::DRIVER_DEVICE) };
        NonNull::new(p)
            .map(|p| Box::new(CdioDrive(p)) as Box<dyn CdDrive>)
            .ok_or_else(|| DecodeError::Open(format!("cannot open CD drive {device}")))
    }
}

/// An opened libcdio device.
struct CdioDrive(NonNull<ffi::CdIo>);

// SAFETY: a handle is only used through `&mut self`; libcdio handles have no thread affinity.
unsafe impl Send for CdioDrive {}

impl Drop for CdioDrive {
    fn drop(&mut self) {
        // SAFETY: created by cdio_open and destroyed once.
        unsafe { ffi::cdio_destroy(self.0.as_ptr()) };
    }
}

impl CdioDrive {
    fn ptr(&self) -> *mut ffi::CdIo {
        self.0.as_ptr()
    }

    fn lsn(&self, track: ffi::Track) -> Option<u32> {
        // SAFETY: valid handle.
        let lsn = unsafe { ffi::cdio_get_track_lsn(self.ptr(), track) };
        (lsn != ffi::CDIO_INVALID_LSN)
            .then(|| u32::try_from(lsn).ok())
            .flatten()
    }
}

/// A CD-Text field as an owned string.
fn text_field(text: *const ffi::CdText, field: ffi::CdTextField, track: u8) -> Option<String> {
    // SAFETY: `text` is the handle's CD-Text object; the result (UTF-8, owned by it) is copied.
    let p = unsafe { ffi::cdtext_get_const(text, field, track) };
    if p.is_null() {
        return None;
    }
    // SAFETY: NUL-terminated string valid while the CD-Text object lives.
    let s = unsafe { CStr::from_ptr(p) }
        .to_string_lossy()
        .trim()
        .to_string();
    (!s.is_empty()).then_some(s)
}

impl CdDrive for CdioDrive {
    fn toc(&mut self) -> Result<Toc, DecodeError> {
        // SAFETY: valid handle.
        let (first, count) = unsafe {
            (
                ffi::cdio_get_first_track_num(self.ptr()),
                ffi::cdio_get_num_tracks(self.ptr()),
            )
        };
        if first == ffi::CDIO_INVALID_TRACK || count == ffi::CDIO_INVALID_TRACK || count == 0 {
            return Err(DecodeError::Open("no disc in the drive".into()));
        }
        let mut tracks = Vec::with_capacity(count as usize);
        for number in first..first.saturating_add(count) {
            let start = self
                .lsn(number)
                .ok_or_else(|| DecodeError::Open(format!("cannot read track {number}")))?;
            // SAFETY: valid handle.
            let format = unsafe { ffi::cdio_get_track_format(self.ptr(), number) };
            tracks.push(TocTrack {
                number,
                start,
                audio: format == ffi::TRACK_FORMAT_AUDIO,
            });
        }
        let leadout = self
            .lsn(ffi::CDIO_CDROM_LEADOUT_TRACK)
            .ok_or_else(|| DecodeError::Open("cannot read the lead-out".into()))?;
        let mut session: ffi::Lsn = 0;
        // SAFETY: valid handle and out pointer.
        let ok = unsafe { ffi::cdio_get_last_session(self.ptr(), &mut session) };
        let last_session_start =
            (ok == ffi::DRIVER_OP_SUCCESS && session > 0).then_some(session as u32);
        Ok(Toc {
            tracks,
            leadout,
            last_session_start,
        })
    }

    fn cd_text(&mut self) -> Option<CdText> {
        // SAFETY: valid handle; the object stays owned by (and is freed with) the handle.
        let text = unsafe { ffi::cdio_get_cdtext(self.ptr()) };
        if text.is_null() {
            return None;
        }
        let entry = |track: u8| CdTextEntry {
            title: text_field(text, ffi::CDTEXT_FIELD_TITLE, track),
            performer: text_field(text, ffi::CDTEXT_FIELD_PERFORMER, track),
        };
        let toc = self.toc().ok()?;
        Some(CdText {
            disc: entry(0),
            tracks: toc
                .tracks
                .iter()
                .map(|t| (t.number, entry(t.number)))
                .collect(),
        })
    }

    fn read_audio(&mut self, lsn: u32, buf: &mut [u8]) -> Result<(), DecodeError> {
        let count = (buf.len() / SECTOR_BYTES) as u32;
        let lsn_c = ffi::Lsn::try_from(lsn).map_err(|_| DecodeError::Seek(lsn.to_string()))?;
        // SAFETY: `buf` holds `count` raw sectors.
        let r = unsafe {
            ffi::cdio_read_audio_sectors(self.ptr(), buf.as_mut_ptr().cast(), lsn_c, count)
        };
        if r == ffi::DRIVER_OP_SUCCESS {
            Ok(())
        } else {
            Err(DecodeError::Decode(format!(
                "cannot read sectors {lsn}..{} (libcdio error {r})",
                lsn + count
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::DecoderFactory;
    use crate::cd::CdFactory;

    #[test]
    fn lists_drives_without_failing() {
        // There may be no drive at all; the call must still work.
        let drives = CdFactory::system().drives();
        assert!(drives.iter().all(|d| d.location.starts_with("cda://")));
    }

    /// Reads a real disc: `NULAMP_TEST_CD=1 cargo test -p nulamp-decoders --features cdio --
    /// --ignored real_disc`. Uses the first drive, which must hold an audio CD.
    #[test]
    #[ignore = "needs an audio CD; set NULAMP_TEST_CD=1"]
    fn real_disc() {
        if std::env::var("NULAMP_TEST_CD").as_deref() != Ok("1") {
            return;
        }
        let f = CdFactory::system();
        let drives = f.drives();
        assert!(!drives.is_empty(), "no CD drive");
        let entries = f.disc_entries(&drives[0].location).unwrap();
        assert!(!entries.is_empty());
        for e in &entries {
            eprintln!("{} {:?} {:?}", e.location, e.title, e.length_ms);
        }
        let mut d = f.open(&entries[0].location).unwrap();
        assert_eq!(d.info().length_ms, entries[0].length_ms);
        let mut out = Vec::new();
        while out.len() < 2 * 44_100 {
            assert!(d.read(&mut out).unwrap().frames() > 0);
        }
        assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
        let len = d.info().length_ms.unwrap();
        assert_eq!(d.seek(len / 2).unwrap(), len / 2);
        assert!(d.read(&mut out).unwrap().frames() > 0);
    }
}
