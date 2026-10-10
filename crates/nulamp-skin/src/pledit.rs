//! `pledit.txt` playlist colours and font (01 §6.3).

use crate::ini::Ini;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlEdit {
    pub normal: u32,
    pub current: u32,
    pub normal_bg: u32,
    pub selected_bg: u32,
    pub mb_fg: u32,
    pub mb_bg: u32,
    /// Font face name; empty means the player's default UI font.
    pub font: String,
    /// `[Misc] UseGenNums`.
    pub use_gen_nums: bool,
}

impl Default for PlEdit {
    fn default() -> Self {
        Self {
            normal: 0x00FF00,
            current: 0xFFFFFF,
            normal_bg: 0x000000,
            selected_bg: 0x0000C6,
            mb_fg: 0x00FF00,
            mb_bg: 0x000000,
            font: String::new(),
            use_gen_nums: false,
        }
    }
}

/// Parses a colour: optional `#`, then hex digits up to the first non-hex character.
pub fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim();
    let v = v.strip_prefix('#').unwrap_or(v);
    let digits: String = v
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect();
    if digits.is_empty() {
        return None;
    }
    u32::from_str_radix(&digits, 16).ok().map(|c| c & 0xFF_FFFF)
}

impl PlEdit {
    pub fn parse(text: &str) -> Self {
        let ini = Ini::parse(text);
        let mut p = PlEdit::default();
        let color = |key: &str, slot: &mut u32| {
            if let Some(c) = ini.get("Text", key).and_then(parse_color) {
                *slot = c;
            }
        };
        color("Normal", &mut p.normal);
        color("Current", &mut p.current);
        color("NormalBG", &mut p.normal_bg);
        color("SelectedBG", &mut p.selected_bg);
        color("mbFG", &mut p.mb_fg);
        color("mbBG", &mut p.mb_bg);
        if let Some(f) = ini.get("Text", "Font") {
            p.font = f.to_string();
        }
        if let Some(v) = ini.get("Misc", "UseGenNums") {
            p.use_gen_nums = v.trim().starts_with('1');
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_optional() {
        let a = PlEdit::parse("[Text]\nNormal=#00ff00\nCurrent=FF0000\n");
        assert_eq!(a.normal, 0x00FF00);
        assert_eq!(a.current, 0xFF0000);
        assert_eq!(PlEdit::parse("[Text]\nNormal=00FF00").normal, 0x00FF00);
    }

    #[test]
    fn lenient_and_defaults() {
        let p = PlEdit::parse(
            "[text]\nnormalbg=#123456xyz\nSelectedBG=zz\nFont=Tahoma\n[Misc]\nUseGenNums=1",
        );
        assert_eq!(p.normal_bg, 0x123456);
        assert_eq!(p.selected_bg, 0x0000C6);
        assert_eq!(p.font, "Tahoma");
        assert!(p.use_gen_nums);
    }
}
