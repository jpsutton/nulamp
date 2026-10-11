//! Keeps the screen saver and display sleep away while a video plays (05 §2 video
//! preferences).
//!
//! A helper thread holds the request, so the user interface never waits for the desktop:
//! on Linux and the BSDs it asks `org.freedesktop.ScreenSaver` over the session bus, on Windows
//! it sets the thread execution state. Other systems are not supported (the request does
//! nothing).

use std::sync::mpsc::{Receiver, Sender, channel};

/// While this value lives, the screen saver stays off.
pub struct ScreenSaverInhibit {
    release: Option<Sender<()>>,
}

impl ScreenSaverInhibit {
    /// Asks the system to keep the screen on. `app` and `reason` are shown by desktops that
    /// list inhibitors.
    pub fn new(app: &str, reason: &str) -> Self {
        let (tx, rx) = channel();
        let (app, reason) = (app.to_string(), reason.to_string());
        let spawned = std::thread::Builder::new()
            .name("nulamp-inhibit".into())
            .spawn(move || hold(&app, &reason, rx));
        if let Err(e) = spawned {
            log::debug!("cannot inhibit the screen saver: {e}");
        }
        Self { release: Some(tx) }
    }
}

impl Drop for ScreenSaverInhibit {
    fn drop(&mut self) {
        if let Some(tx) = self.release.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn hold(app: &str, reason: &str, release: Receiver<()>) {
    let inhibit = || -> zbus::Result<(zbus::blocking::Connection, u32)> {
        let conn = zbus::blocking::Connection::session()?;
        let cookie: u32 = conn
            .call_method(
                Some("org.freedesktop.ScreenSaver"),
                "/org/freedesktop/ScreenSaver",
                Some("org.freedesktop.ScreenSaver"),
                "Inhibit",
                &(app, reason),
            )?
            .body()
            .deserialize()?;
        Ok((conn, cookie))
    };
    match inhibit() {
        Ok((conn, cookie)) => {
            let _ = release.recv();
            let _ = conn.call_method(
                Some("org.freedesktop.ScreenSaver"),
                "/org/freedesktop/ScreenSaver",
                Some("org.freedesktop.ScreenSaver"),
                "UnInhibit",
                &(cookie,),
            );
        }
        Err(e) => log::debug!("cannot inhibit the screen saver: {e}"),
    }
}

#[cfg(windows)]
fn hold(_app: &str, _reason: &str, release: Receiver<()>) {
    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };
    // SAFETY: plain Win32 call; the state belongs to this thread and ends with it.
    unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED) };
    let _ = release.recv();
    // SAFETY: as above.
    unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)))]
fn hold(_app: &str, _reason: &str, release: Receiver<()>) {
    let _ = release.recv();
}
