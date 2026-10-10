//! Sends synthetic pointer and key events through the X11 XTEST extension.
//!
//! Usage:
//!   xdrive move X Y
//!   xdrive click X Y [BUTTON]       (1 left, 2 middle, 3 right)
//!   xdrive drag X1 Y1 X2 Y2
//!   xdrive key KEYCODE              (X keycode, e.g. 9 = Escape)
//!   xdrive where NAME               (prints X Y W H of the first window whose title contains NAME)

#[cfg(all(unix, not(target_os = "macos")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{thread::sleep, time::Duration};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
        MOTION_NOTIFY_EVENT,
    };
    use x11rb::protocol::xtest::ConnectionExt as _;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize| -> i32 { args.get(i).and_then(|a| a.parse().ok()).unwrap_or(0) };
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let pause = || sleep(Duration::from_millis(60));
    let motion = |x: i32, y: i32| {
        conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x as i16, y as i16, 0)
    };
    match args.first().map(String::as_str) {
        Some("move") => {
            motion(num(1), num(2))?;
        }
        Some("click") => {
            let b = if args.len() > 3 { num(3) as u8 } else { 1 };
            motion(num(1), num(2))?;
            conn.flush()?;
            pause();
            conn.xtest_fake_input(BUTTON_PRESS_EVENT, b, 0, root, 0, 0, 0)?;
            conn.flush()?;
            pause();
            conn.xtest_fake_input(BUTTON_RELEASE_EVENT, b, 0, root, 0, 0, 0)?;
        }
        Some("drag") => {
            motion(num(1), num(2))?;
            conn.flush()?;
            pause();
            conn.xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)?;
            for k in 1..=10 {
                let x = num(1) + (num(3) - num(1)) * k / 10;
                let y = num(2) + (num(4) - num(2)) * k / 10;
                conn.flush()?;
                pause();
                motion(x, y)?;
            }
            conn.flush()?;
            pause();
            conn.xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)?;
        }
        Some("key") => {
            let k = num(1) as u8;
            conn.xtest_fake_input(KEY_PRESS_EVENT, k, 0, root, 0, 0, 0)?;
            conn.flush()?;
            pause();
            conn.xtest_fake_input(KEY_RELEASE_EVENT, k, 0, root, 0, 0, 0)?;
        }
        Some("where") => {
            use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
            let name = args.get(1).cloned().unwrap_or_default();
            let list_atom = conn.intern_atom(false, b"_NET_CLIENT_LIST")?.reply()?.atom;
            let utf8 = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
            let net_name = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;
            let list = conn
                .get_property(false, root, list_atom, AtomEnum::WINDOW, 0, 4096)?
                .reply()?;
            for w in list.value32().into_iter().flatten() {
                let mut title = conn
                    .get_property(false, w, net_name, utf8, 0, 1024)?
                    .reply()?
                    .value;
                if title.is_empty() {
                    title = conn
                        .get_property(false, w, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)?
                        .reply()?
                        .value;
                }
                let title = String::from_utf8_lossy(&title).into_owned();
                if title.contains(&name) {
                    let g = conn.get_geometry(w)?.reply()?;
                    let t = conn.translate_coordinates(w, root, 0, 0)?.reply()?;
                    println!("{} {} {} {}", t.dst_x, t.dst_y, g.width, g.height);
                    break;
                }
            }
        }
        _ => eprintln!("usage: xdrive move|click|drag|key|where ..."),
    }
    conn.flush()?;
    sleep(Duration::from_millis(30));
    Ok(())
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn main() {
    eprintln!("xdrive needs X11");
}
