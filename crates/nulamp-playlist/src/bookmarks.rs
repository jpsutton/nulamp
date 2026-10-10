//! Bookmarks file: pairs of lines, location then title (10 §9).

use crate::text::{UTF8_BOM, decode_with, one_line};

/// One bookmark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub location: String,
    pub title: String,
}

impl Bookmark {
    pub fn new(location: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            title: title.into(),
        }
    }
}

/// Reads a UTF-8 bookmarks file (a byte-order mark is ignored; invalid bytes are
/// replaced).
///
/// Lines end at LF, CRLF or CR. A final line without a newline is read. Pairs where the
/// location or the title is empty or whitespace-only are skipped; a final location
/// without a title line is dropped.
pub fn read_bookmarks(bytes: &[u8]) -> Vec<Bookmark> {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    pairs(&String::from_utf8_lossy(bytes))
}

/// Reads the legacy 8-bit bookmarks file (`Winamp.bm`) as Windows-1252.
pub fn read_legacy_bookmarks(bytes: &[u8]) -> Vec<Bookmark> {
    pairs(&decode_with(encoding_rs::WINDOWS_1252, bytes))
}

fn pairs(text: &str) -> Vec<Bookmark> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|p| !p[0].trim().is_empty() && !p[1].trim().is_empty())
        .map(|p| Bookmark::new(p[0], p[1]))
        .collect()
}

/// Writes a bookmarks file: UTF-8 without a byte-order mark, LF line endings, a newline
/// after the last title. Bookmarks with an empty part are skipped; line breaks inside a
/// value become spaces.
pub fn write_bookmarks(bookmarks: &[Bookmark]) -> Vec<u8> {
    let mut s = String::new();
    for b in bookmarks {
        let (loc, title) = (one_line(&b.location), one_line(&b.title));
        if loc.trim().is_empty() || title.trim().is_empty() {
            continue;
        }
        s.push_str(&format!("{loc}\n{title}\n"));
    }
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_and_final_line() {
        let b = read_bookmarks(b"/a.mp3\r\nA\r\nhttp://h/s\nRadio \xE2\x98\x83");
        assert_eq!(
            b,
            [
                Bookmark::new("/a.mp3", "A"),
                Bookmark::new("http://h/s", "Radio ☃")
            ]
        );
    }

    #[test]
    fn empty_parts_are_skipped() {
        let b = read_bookmarks(b"/a.mp3\n\n\nTitle\n/c.mp3\nC\n/dangling.mp3\n");
        assert_eq!(b, [Bookmark::new("/c.mp3", "C")]);
    }

    #[test]
    fn legacy_8bit() {
        assert_eq!(
            read_legacy_bookmarks(b"C:\\caf\xE9.mp3\r\nCaf\xE9\r\n"),
            [Bookmark::new("C:\\café.mp3", "Café")]
        );
    }

    #[test]
    fn round_trip() {
        let b = vec![
            Bookmark::new("/m/é.mp3", "É"),
            Bookmark::new("http://h/s", "R\nx"),
            Bookmark::new("", "skip"),
        ];
        let out = write_bookmarks(&b);
        assert_eq!(out, "/m/é.mp3\nÉ\nhttp://h/s\nR x\n".as_bytes());
        assert_eq!(
            read_bookmarks(&out),
            [
                Bookmark::new("/m/é.mp3", "É"),
                Bookmark::new("http://h/s", "R x")
            ]
        );
    }
}
