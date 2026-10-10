//! Renders the main menu and a playlist menu to PPM files for inspection.

use std::io::Write;

use nulamp_ui::menu;
use nulamp_ui::menus::{self, MenuState};

fn write(path: &str, f: &nulamp_render::Frame) {
    let mut o = std::fs::File::create(path).expect("create");
    write!(o, "P6\n{} {}\n255\n", f.width(), f.height()).expect("write");
    let b: Vec<u8> = f
        .pixels()
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    o.write_all(&b).expect("write");
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut fonts = nulamp_text::FontSystem::new();
    let skin = nulamp_skin::builtin::skin();
    let state = MenuState {
        repeat: true,
        app_name: "nulamp".into(),
        eq_visible: true,
        ..MenuState::default()
    };
    for (name, items) in [
        ("main", menus::main_menu(&state)),
        ("vis", menus::vis_menu(&state)),
        ("ctx", menus::list_context()),
    ] {
        let l = menu::layout(&mut fonts, &items);
        let f = menu::draw(&mut fonts, &skin, &items, &l, Some(2));
        write(&format!("{dir}/menu-{name}.ppm"), &f);
    }
}
