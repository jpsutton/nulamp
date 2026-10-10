//! Rendering of system-font text in skinned windows (02 §8.1).

use nulamp_render::Frame;
use nulamp_skin::Skin;
use nulamp_text::{Direction, FontSpec, FontSystem};

/// Font for the main-window title: the user's playlist font if set, else `pledit.txt` `Font`,
/// else the default UI font. Pixel height 10.
pub fn title_font(skin: &Skin, user_font: Option<&str>) -> FontSpec {
    let family = user_font
        .filter(|f| !f.is_empty())
        .unwrap_or(&skin.pledit.font);
    FontSpec::new(family, 10)
}

/// Renders the 154×12 title box (111,24)–(265,36) with the scroll buffer at `offset`.
pub fn title_image(
    fonts: &mut FontSystem,
    skin: &Skin,
    spec: &FontSpec,
    text: &str,
    offset: i32,
) -> Frame {
    let (fg, bg) = (skin.title_colors.fg, skin.title_colors.bg);
    let mut img = Frame::new(154, 12, bg);
    let layout = fonts.layout(text, spec, Direction::Ltr);
    // Text top at window y = 23, i.e. one row above the box.
    let baseline = -1 + layout.metrics.ascent;
    fonts.draw(&mut img, &layout, -offset, baseline, fg, (0, 0, 154, 12));
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_has_no_grey_pixels() {
        let mut fonts = FontSystem::new();
        if !fonts.has_fonts() {
            return;
        }
        let skin = nulamp_skin::builtin::skin();
        let spec = title_font(&skin, None);
        let img = title_image(&mut fonts, &skin, &spec, "1. Artist - Title (3:45) ÄÖÜ", 0);
        let (fg, bg) = (skin.title_colors.fg, skin.title_colors.bg);
        assert!(img.pixels().iter().all(|&p| p == fg || p == bg));
        assert!(img.pixels().contains(&fg));
    }
}
