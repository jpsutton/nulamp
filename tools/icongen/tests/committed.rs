use std::path::Path;

/// Decodes a PNG to `(width, height, RGBA)`.
fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .expect("valid PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("PNG decodes");
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

/// The ICO's images, PNG entries decoded so that the comparison does not depend on the
/// deflate backend (Cargo may unify `flate2` to a different one in other builds).
fn ico_images(ico: &[u8]) -> Vec<Vec<u8>> {
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    (0..count)
        .map(|i| {
            let e = &ico[6 + 16 * i..][..16];
            let len = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
            let off = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
            let data = &ico[off..off + len];
            if data.starts_with(b"\x89PNG") {
                decode(data).2
            } else {
                data.to_vec()
            }
        })
        .collect()
}

#[test]
fn committed_icons_match_generator() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (name, data) in nulamp_icongen::files() {
        let committed = std::fs::read(root.join(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let same = if name.ends_with(".png") {
            decode(&committed) == decode(&data)
        } else if name.ends_with(".ico") {
            ico_images(&committed) == ico_images(&data)
        } else {
            committed == data
        };
        assert!(same, "{name} is stale: run `cargo run -p nulamp-icongen`");
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
