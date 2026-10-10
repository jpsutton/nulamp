use nulamp_skin::{SkinBitmap, builtin};

#[test]
fn committed_assets_match_generator() {
    for (name, data) in nulamp_skingen::files() {
        let committed = builtin::FILES.iter().find(|(n, _)| *n == name);
        let (_, bytes) = committed.unwrap_or_else(|| panic!("{name} missing from builtin::FILES"));
        assert!(
            *bytes == data.as_slice(),
            "{name} is stale: run `cargo run -p nulamp-skingen`"
        );
    }
}

#[test]
fn builtin_bitmaps_have_spec_sizes() {
    let skin = builtin::skin();
    for which in SkinBitmap::ALL {
        if matches!(which, SkinBitmap::NumsEx | SkinBitmap::Avs) {
            continue;
        }
        let b = skin.bitmap(which);
        assert_eq!((b.width(), b.height()), which.default_size(), "{which:?}");
        assert!(skin.has_own(which) || skin.is_builtin, "{which:?}");
    }
    assert!(!skin.nums_ex);
}

#[test]
fn builtin_fonts_and_colours() {
    let skin = builtin::skin();
    assert_eq!(skin.title_colors.bg, 0x0A0D10);
    assert_eq!(skin.title_colors.fg, 0x3CF06E);
    for (i, g) in skin.gen_font.letters.iter().enumerate() {
        let c = (b'A' + i as u8) as char;
        assert_eq!(g.width, nulamp_skingen::font::advance(c), "letter {c}");
    }
    for g in skin.gen_font.digits {
        assert!(g.width >= 3);
    }
    assert!(skin.pledit.use_gen_nums);
    assert_eq!(skin.pledit.normal, 0x3CF06E);
}
