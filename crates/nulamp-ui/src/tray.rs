//! Tray / status icon (11 §7). Linux uses StatusNotifierItem over D-Bus; other platforms are
//! not implemented yet.

use std::sync::mpsc::Sender;

#[cfg(target_os = "linux")]
use nulamp_remote::NumberedCommand;
use nulamp_remote::Request;

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
