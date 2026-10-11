use std::path::Path;

#[test]
fn committed_icons_match_generator() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (name, data) in nulamp_icongen::files() {
        let committed = std::fs::read(root.join(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            committed == data,
            "{name} is stale: run `cargo run -p nulamp-icongen`"
        );
    }
}

#[test]
fn ico_directory() {
    let ico = nulamp_icongen::encode_ico(&nulamp_icongen::ICO_SIZES);
    assert_eq!(&ico[..6], &[0, 0, 1, 0, 5, 0]);
    let dims: Vec<u8> = (0..5).map(|i| ico[6 + 16 * i]).collect();
    assert_eq!(dims, [16, 24, 32, 48, 0]);
    // The last entry is a PNG.
    let e = 6 + 16 * 4;
    let offset = u32::from_le_bytes(ico[e + 12..e + 16].try_into().unwrap()) as usize;
    assert_eq!(&ico[offset..offset + 8], b"\x89PNG\r\n\x1a\n");
}
