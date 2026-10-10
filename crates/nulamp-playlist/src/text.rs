//! Text decoding and encoding helpers shared by the readers and writers.

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

/// UTF-8 byte-order mark.
pub const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Encoding given by a byte-order mark, and the data after it.
pub fn bom(bytes: &[u8]) -> Option<(&'static Encoding, &[u8])> {
    if let Some(rest) = bytes.strip_prefix(UTF8_BOM) {
        Some((UTF_8, rest))
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        Some((UTF_16LE, rest))
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        Some((UTF_16BE, rest))
    } else {
        None
    }
}

/// True for UTF-16LE text without a byte-order mark that starts with an ASCII character.
pub fn looks_utf16le(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0] != 0 && bytes[1] == 0 && bytes[2] != 0 && bytes[3] == 0
}

/// Decodes with `encoding`, replacing malformed sequences.
pub fn decode_with(encoding: &'static Encoding, bytes: &[u8]) -> String {
    encoding.decode_without_bom_handling(bytes).0.into_owned()
}

/// Strict UTF-8, else Windows-1252.
pub fn decode_utf8_or_1252(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => decode_with(WINDOWS_1252, bytes),
    }
}

/// Byte-order mark, then BOM-less UTF-16LE, then strict UTF-8, then Windows-1252.
pub fn decode_auto(bytes: &[u8]) -> String {
    if let Some((enc, rest)) = bom(bytes) {
        return decode_with(enc, rest);
    }
    if looks_utf16le(bytes) {
        return decode_with(UTF_16LE, bytes);
    }
    decode_utf8_or_1252(bytes)
}

/// Encodes as Windows-1252; characters outside the code page become `?`.
pub fn encode_1252(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut buf = [0u8; 4];
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c as u8);
            continue;
        }
        let (bytes, _, unmappable) = WINDOWS_1252.encode(c.encode_utf8(&mut buf));
        if unmappable {
            out.push(b'?')
        } else {
            out.extend_from_slice(&bytes)
        }
    }
    out
}

/// Decodes `%XX` escapes. Invalid escapes are kept; the bytes are read as UTF-8, or as
/// Windows-1252 when they are not valid UTF-8.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    decode_utf8_or_1252(&out)
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// Percent-encodes everything except unreserved characters, `/` and `:`.
pub fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// Escapes text for XML content and attribute values.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// Replaces line breaks so a value stays on one line.
pub fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding() {
        assert_eq!(decode_auto(b"\xEF\xBB\xBFhi"), "hi");
        assert_eq!(decode_auto(b"\xFF\xFEh\0i\0"), "hi");
        assert_eq!(decode_auto(b"[\0p\0l\0"), "[pl");
        assert_eq!(decode_auto(b"caf\xE9"), "café");
        assert_eq!(decode_auto("café".as_bytes()), "café");
    }

    #[test]
    fn percent() {
        assert_eq!(percent_decode("a%20b%C3%A9%zz%"), "a bé%zz%");
        assert_eq!(percent_encode_path("/a b/é"), "/a%20b/%C3%A9");
        assert_eq!(percent_decode(&percent_encode_path("/x y/ü#?")), "/x y/ü#?");
    }

    #[test]
    fn windows_1252_output() {
        assert_eq!(encode_1252("café €日"), b"caf\xE9 \x80?");
    }

    #[test]
    fn xml() {
        assert_eq!(xml_escape("a<b>&\"'\x01"), "a&lt;b&gt;&amp;&quot;&apos;");
    }
}
