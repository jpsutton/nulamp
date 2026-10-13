//! Tray / status icon (11 §7). Linux uses StatusNotifierItem over D-Bus and Windows a
//! notification area icon (`Shell_NotifyIconW`); other platforms are not implemented yet.

use std::sync::mpsc::Sender;

#[cfg(any(target_os = "linux", windows))]
use nulamp_remote::NumberedCommand;
use nulamp_remote::Request;

/// The tray icon's requests go to the shell like remote-control requests.
pub struct Tray {
    #[cfg(target_os = "linux")]
    handle: ksni::blocking::Handle<sni::NulTray>,
    #[cfg(windows)]
    hwnd: isize,
}

impl Tray {
    /// Starts the tray icon. Returns `None` when no tray host is available.
    pub fn start(tx: Sender<Request>, tooltip: &str) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            use ksni::blocking::TrayMethods;
            let t = sni::NulTray {
                tx,
                tooltip: tooltip.to_string(),
            };
            match t.spawn() {
                Ok(handle) => Some(Self { handle }),
                Err(e) => {
                    log::info!("tray icon not available: {e}");
                    None
                }
            }
        }
        #[cfg(windows)]
        {
            win::start(tx, tooltip).map(|hwnd| Self { hwnd })
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = (tx, tooltip);
            None
        }
    }

    /// Updates the tooltip (the window caption, 11 §7).
    pub fn set_tooltip(&self, text: &str) {
        #[cfg(target_os = "linux")]
        {
            let text = text.to_string();
            self.handle.update(move |t| t.tooltip = text);
        }
        #[cfg(windows)]
        win::set_tooltip(self.hwnd, text);
        #[cfg(not(any(target_os = "linux", windows)))]
        let _ = text;
    }
}

#[cfg(windows)]
impl Drop for Tray {
    fn drop(&mut self) {
        win::stop(self.hwnd);
    }
}

/// The application icon at each hand-drawn pixel size as `(width, height, ARGB32
/// big-endian)`; the tray host picks the closest size.
pub fn icons_argb() -> Vec<(i32, i32, Vec<u8>)> {
    nulamp_icon::PIXEL_SIZES
        .iter()
        .map(|&s| {
            let img = nulamp_icon::icon(s);
            let data = img
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[r, g, b, a]| [a, r, g, b])
                .collect();
            (img.width as i32, img.height as i32, data)
        })
        .collect()
}

#[cfg(target_os = "linux")]
mod sni {
    use super::*;

    pub struct NulTray {
        pub tx: Sender<Request>,
        pub tooltip: String,
    }

    impl NulTray {
        fn send(&self, r: Request) {
            let _ = self.tx.send(r);
        }
    }

    impl ksni::Tray for NulTray {
        fn id(&self) -> String {
            "nulamp".into()
        }

        fn title(&self) -> String {
            "nulamp".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            icons_argb()
                .into_iter()
                .map(|(width, height, data)| ksni::Icon {
                    width,
                    height,
                    data,
                })
                .collect()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: self.tooltip.clone(),
                ..Default::default()
            }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(Request::Raise);
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            let item = |label: &str, r: Request| -> ksni::MenuItem<Self> {
                StandardItem {
                    label: label.into(),
                    activate: Box::new(move |t: &mut Self| t.send(r.clone())),
                    ..Default::default()
                }
                .into()
            };
            vec![
                item("Show nulamp", Request::Raise),
                ksni::MenuItem::Separator,
                item("Previous", Request::Previous),
                item("Play", Request::Play),
                item("Pause", Request::Pause),
                item("Stop", Request::Stop),
                item("Next", Request::Next),
                ksni::MenuItem::Separator,
                item(
                    "Open file(s)...",
                    Request::Command(NumberedCommand::OpenFiles),
                ),
                item("Open URL...", Request::Command(NumberedCommand::OpenUrl)),
                ksni::MenuItem::Separator,
                item("Exit", Request::Quit),
            ]
        }
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
        Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    /// Mouse messages for the icon.
    const WM_TRAY: u32 = WM_APP + 1;
    /// The tooltip text in `TIP` changed.
    const WM_TIP: u32 = WM_APP + 2;

    static TIP: Mutex<String> = Mutex::new(String::new());

    thread_local! {
        static TX: RefCell<Option<Sender<Request>>> = const { RefCell::new(None) };
        static ICON: Cell<HICON> = const { Cell::new(std::ptr::null_mut()) };
        static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn menu_entries() -> Vec<Option<(&'static str, Request)>> {
        vec![
            Some(("Show nulamp", Request::Raise)),
            None,
            Some(("Previous", Request::Previous)),
            Some(("Play", Request::Play)),
            Some(("Pause", Request::Pause)),
            Some(("Stop", Request::Stop)),
            Some(("Next", Request::Next)),
            None,
            Some((
                "Open file(s)...",
                Request::Command(NumberedCommand::OpenFiles),
            )),
            Some(("Open URL...", Request::Command(NumberedCommand::OpenUrl))),
            None,
            Some(("Exit", Request::Quit)),
        ]
    }

    /// The icon master closest to the notification area's small icon size.
    unsafe fn make_icon() -> HICON {
        // SAFETY: plain metric query.
        let want = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        let icons = icons_argb();
        let Some((w, h, argb)) = icons.iter().min_by_key(|(w, _, _)| (w - want).abs()) else {
            return std::ptr::null_mut();
        };
        let bgra: Vec<u8> = argb
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[a, r, g, b]| [b, g, r, a])
            .collect();
        // SAFETY: the bitmaps are created from live buffers (copied by the system), used
        // for the icon (which copies them) and deleted.
        unsafe {
            let color = CreateBitmap(*w, *h, 1, 32, bgra.as_ptr().cast());
            let mask = CreateBitmap(*w, *h, 1, 1, std::ptr::null());
            let info = ICONINFO {
                fIcon: 1,
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: color,
            };
            let icon = CreateIconIndirect(&info);
            DeleteObject(color);
            DeleteObject(mask);
            icon
        }
    }

    unsafe fn data(hwnd: HWND) -> NOTIFYICONDATAW {
        // SAFETY: an all-zero NOTIFYICONDATAW is valid; the fields used are set below.
        let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        d.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = hwnd;
        d.uID = 1;
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        d.uCallbackMessage = WM_TRAY;
        d.hIcon = ICON.with(|i| i.get());
        let tip = TIP.lock().map(|t| t.clone()).unwrap_or_default();
        for (dst, c) in d.szTip.iter_mut().take(127).zip(tip.encode_utf16()) {
            *dst = c;
        }
        d
    }

    unsafe fn show_menu(hwnd: HWND) {
        // SAFETY: Win32 menu calls on this thread's window; the menu is destroyed after use.
        unsafe {
            let menu = CreatePopupMenu();
            let entries = menu_entries();
            for (i, e) in entries.iter().enumerate() {
                match e {
                    Some((label, _)) => {
                        AppendMenuW(menu, MF_STRING, i + 1, wide(label).as_ptr());
                    }
                    None => {
                        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                    }
                }
            }
            let mut pt = std::mem::zeroed();
            GetCursorPos(&mut pt);
            // Without the foreground window the menu does not close on a click elsewhere.
            SetForegroundWindow(hwnd);
            let id = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD,
                pt.x,
                pt.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
            if let Some(Some((_, r))) = (id as usize).checked_sub(1).and_then(|i| entries.get(i)) {
                send(r.clone());
            }
        }
    }

    fn send(r: Request) {
        TX.with(|t| {
            if let Some(tx) = &*t.borrow() {
                let _ = tx.send(r);
            }
        });
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: called by the system on the tray thread with a live window.
        unsafe {
            match msg {
                WM_TRAY => match lparam as u32 {
                    WM_LBUTTONUP => send(Request::Raise),
                    WM_RBUTTONUP => show_menu(hwnd),
                    _ => {}
                },
                WM_TIP => {
                    Shell_NotifyIconW(NIM_MODIFY, &data(hwnd));
                }
                WM_CLOSE => {
                    Shell_NotifyIconW(NIM_DELETE, &data(hwnd));
                    DestroyWindow(hwnd);
                }
                WM_DESTROY => PostQuitMessage(0),
                m if m != 0 && m == TASKBAR_CREATED.with(|t| t.get()) => {
                    // Explorer restarted: the icon is gone.
                    Shell_NotifyIconW(NIM_ADD, &data(hwnd));
                }
                _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
        0
    }

    /// Starts the icon on its own thread (it needs a window and a message loop). Returns the
    /// window handle.
    pub fn start(tx: Sender<Request>, tooltip: &str) -> Option<isize> {
        if let Ok(mut t) = TIP.lock() {
            *t = tooltip.to_string();
        }
        let (ready, wait) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || {
                TX.with(|t| *t.borrow_mut() = Some(tx));
                // SAFETY: Win32 window setup and message loop owned by this thread.
                unsafe {
                    let class = wide("nulamp-tray");
                    let hinst = GetModuleHandleW(std::ptr::null());
                    let mut wc: WNDCLASSW = std::mem::zeroed();
                    wc.lpfnWndProc = Some(wndproc);
                    wc.hInstance = hinst;
                    wc.lpszClassName = class.as_ptr();
                    RegisterClassW(&wc);
                    // A message-only window: never shown.
                    let hwnd = CreateWindowExW(
                        0,
                        class.as_ptr(),
                        class.as_ptr(),
                        0,
                        0,
                        0,
                        0,
                        0,
                        HWND_MESSAGE,
                        std::ptr::null_mut(),
                        hinst,
                        std::ptr::null(),
                    );
                    if hwnd.is_null() {
                        let _ = ready.send(None);
                        return;
                    }
                    TASKBAR_CREATED
                        .with(|t| t.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr())));
                    ICON.with(|i| i.set(make_icon()));
                    if Shell_NotifyIconW(NIM_ADD, &data(hwnd)) == 0 {
                        DestroyWindow(hwnd);
                        let _ = ready.send(None);
                        return;
                    }
                    let _ = ready.send(Some(hwnd as isize));
                    let mut m = std::mem::zeroed();
                    while GetMessageW(&mut m, std::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&m);
                        DispatchMessageW(&m);
                    }
                    let icon = ICON.with(|i| i.get());
                    if !icon.is_null() {
                        DestroyIcon(icon);
                    }
                }
            })
            .ok()?;
        let hwnd = wait.recv().ok().flatten();
        if hwnd.is_none() {
            log::info!("tray icon not available");
        }
        hwnd
    }

    pub fn set_tooltip(hwnd: isize, text: &str) {
        if let Ok(mut t) = TIP.lock() {
            *t = text.to_string();
        }
        // SAFETY: posting to a window from another thread is allowed; a stale handle fails.
        unsafe { PostMessageW(hwnd as HWND, WM_TIP, 0, 0) };
    }

    pub fn stop(hwnd: isize) {
        // SAFETY: as above.
        unsafe { PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0) };
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_sizes() {
        let icons = super::icons_argb();
        let sizes: Vec<_> = icons.iter().map(|(w, _, _)| *w).collect();
        assert_eq!(sizes, [16, 22, 24, 32, 48]);
        for (w, h, d) in &icons {
            assert_eq!(d.len(), (w * h * 4) as usize);
        }
    }
}
