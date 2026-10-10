//! Tray / status icon (11 §7). Linux uses StatusNotifierItem over D-Bus; other platforms are
//! not implemented yet.

use std::sync::mpsc::Sender;

use nulamp_remote::{NumberedCommand, Request};

/// The tray icon's requests go to the shell like remote-control requests.
pub struct Tray {
    #[cfg(target_os = "linux")]
    handle: ksni::blocking::Handle<sni::NulTray>,
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
        #[cfg(not(target_os = "linux"))]
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
        #[cfg(not(target_os = "linux"))]
        let _ = text;
    }
}

/// 22×22 ARGB icon: a play triangle on a dark rounded square.
pub fn icon_argb() -> (i32, i32, Vec<u8>) {
    let (w, h) = (22i32, 22i32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let corner = (x < 2 || x > w - 3) && (y < 2 || y > h - 3);
            // Triangle pointing right: half-height shrinks from 7 at x = 7 to 0 at x = 16.
            let in_tri = (7..=16).contains(&x) && (y - 11).abs() <= 7 - (x - 7) * 7 / 9;
            let (a, r, g, b) = if corner {
                (0, 0, 0, 0)
            } else if in_tri {
                (255, 0x3C, 0xF0, 0x6E)
            } else {
                (255, 0x2C, 0x30, 0x39)
            };
            data.extend_from_slice(&[a, r, g, b]);
        }
    }
    (w, h, data)
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
            let (width, height, data) = icon_argb();
            vec![ksni::Icon {
                width,
                height,
                data,
            }]
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

#[cfg(test)]
mod tests {
    #[test]
    fn icon_size() {
        let (w, h, d) = super::icon_argb();
        assert_eq!(d.len(), (w * h * 4) as usize);
    }
}
