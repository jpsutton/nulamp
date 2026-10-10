//! ID3v1 and ID3v1.1 tags (spec 10 §7.4): the last 128 bytes of a file.
//!
//! Layout: `TAG`, title (30), artist (30), album (30), year (4), comment (30), genre (1).
//! ID3v1.1 stores a track number in byte 126 when byte 125 is 0, leaving 28 bytes of comment.

use std::io::{self, Read, Seek, SeekFrom};

use crate::genre::{NO_GENRE, genre_index, genre_name};

/// Size of a tag in bytes.
pub const TAG_LEN: usize = 128;

/// Fields an ID3v1 tag can hold.
pub const FIELDS: [&str; 7] = [
    "title", "artist", "album", "year", "comment", "track", "genre",
];

/// One ID3v1 tag. Text is stored decoded; [`Id3v1::to_bytes`] truncates it to the field sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Id3v1 {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub comment: String,
    /// Track number 1–255 (ID3v1.1); `None` writes a 30-byte comment.
    pub track: Option<u8>,
    /// Index into the genre list; [`NO_GENRE`] (255) for none.
    pub genre: u8,
}

impl Default for Id3v1 {
    fn default() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            year: String::new(),
            comment: String::new(),
            track: None,
            genre: NO_GENRE,
        }
    }
}

impl Id3v1 {
    /// Parses a 128-byte block; `None` when it does not start with `TAG`.
    pub fn parse(block: &[u8]) -> Option<Self> {
        if block.len() != TAG_LEN || &block[..3] != b"TAG" {
            return None;
        }
        let v11 = block[125] == 0 && block[126] != 0;
        let comment_end = if v11 { 125 } else { 127 };
        Some(Self {
            title: decode_text(&block[3..33]),
            artist: decode_text(&block[33..63]),
            album: decode_text(&block[63..93]),
            year: decode_text(&block[93..97]),
            comment: decode_text(&block[97..comment_end]),
            track: v11.then_some(block[126]),
            genre: block[127],
        })
    }

    /// Reads the tag at the end of `file`, if any.
    pub fn read_from<R: Read + Seek>(file: &mut R) -> io::Result<Option<Self>> {
        let len = file.seek(SeekFrom::End(0))?;
        if len < TAG_LEN as u64 {
            return Ok(None);
        }
        let mut block = [0; TAG_LEN];
        file.seek(SeekFrom::End(-(TAG_LEN as i64)))?;
        file.read_exact(&mut block)?;
        Ok(Self::parse(&block))
    }

    /// Encodes the tag: text is truncated to the field size and padded with NUL.
    pub fn to_bytes(&self) -> [u8; TAG_LEN] {
        let mut out = [0; TAG_LEN];
        out[..3].copy_from_slice(b"TAG");
        put_text(&mut out[3..33], &self.title);
        put_text(&mut out[33..63], &self.artist);
        put_text(&mut out[63..93], &self.album);
        put_text(&mut out[93..97], &self.year);
        match self.track.filter(|t| *t != 0) {
            Some(track) => {
                put_text(&mut out[97..125], &self.comment);
                out[126] = track;
            }
            None => put_text(&mut out[97..127], &self.comment),
        }
        out[127] = self.genre;
        out
    }

    /// Value of `field` (a name from [`FIELDS`]); `None` when empty or not an ID3v1 field.
    pub fn get(&self, field: &str) -> Option<String> {
        let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
        match field {
            "title" => text(&self.title),
            "artist" => text(&self.artist),
            "album" => text(&self.album),
            "year" => text(&self.year),
            "comment" => text(&self.comment),
            "track" => self.track.filter(|t| *t != 0).map(|t| t.to_string()),
            "genre" => genre_name(self.genre).map(str::to_string),
            _ => None,
        }
    }

    /// Sets or clears (`None` or empty) `field`. Returns false when ID3v1 has no such field.
    ///
    /// `track` takes the leading number of the value (`3/12` → 3) and is dropped outside
    /// 1–255. `genre` is matched against the genre list; unknown names store 255.
    pub fn set(&mut self, field: &str, value: Option<&str>) -> bool {
        let value = value.unwrap_or("").to_string();
        match field {
            "title" => self.title = value,
            "artist" => self.artist = value,
            "album" => self.album = value,
            "year" => self.year = value,
            "comment" => self.comment = value,
            "track" => self.track = leading_number(&value).filter(|t| *t != 0),
            "genre" => self.genre = genre_index(value.trim()),
            _ => return false,
        }
        true
    }

    /// True when every field is empty.
    pub fn is_empty(&self) -> bool {
        FIELDS.iter().all(|f| self.get(f).is_none())
    }
}

/// Leading decimal number of `value` as a byte (`"3/12"` → 3).
fn leading_number(value: &str) -> Option<u8> {
    let digits: String = value
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Decodes an 8-bit text field: cut at the first NUL, trailing spaces removed, UTF-8 when
/// the bytes are valid UTF-8, else Latin-1.
pub fn decode_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let mut bytes = &bytes[..end];
    while let [rest @ .., b' '] = bytes {
        bytes = rest;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// Encodes `text` into at most `max` bytes so that [`decode_text`] gives it back (up to
/// truncation).
///
/// Latin-1 is used when every character fits and the bytes do not read back as different
/// UTF-8 text; otherwise UTF-8, cut at a character boundary. NUL characters are dropped.
pub fn encode_text(text: &str, max: usize) -> Vec<u8> {
    let text: String = text.chars().filter(|&c| c != '\0').collect();
    if text.chars().all(|c| u32::from(c) <= 0xFF) {
        let mut latin1: Vec<u8> = text.chars().map(|c| c as u8).collect();
        latin1.truncate(max);
        if latin1.is_ascii() || std::str::from_utf8(&latin1).is_err() {
            return latin1;
        }
    }
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.as_bytes()[..end].to_vec()
}

fn put_text(field: &mut [u8], text: &str) {
    let bytes = encode_text(text, field.len());
    field[..bytes.len()].copy_from_slice(&bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Id3v1 {
        Id3v1 {
            title: "Title".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            year: "1999".into(),
            comment: "Comment".into(),
            track: Some(7),
            genre: 17,
        }
    }

    #[test]
    fn v11_track_byte() {
        let bytes = sample().to_bytes();
        assert_eq!(&bytes[..3], b"TAG");
        assert_eq!(bytes[125], 0);
        assert_eq!(bytes[126], 7);
        assert_eq!(bytes[127], 17);
        let tag = Id3v1::parse(&bytes).unwrap();
        assert_eq!(tag, sample());
        assert_eq!(tag.get("track").as_deref(), Some("7"));
        assert_eq!(tag.get("genre").as_deref(), Some("Rock"));
    }

    #[test]
    fn v10_comment_uses_30_bytes() {
        let mut tag = sample();
        tag.track = None;
        tag.comment = "c".repeat(40);
        let bytes = tag.to_bytes();
        assert_eq!(&bytes[97..127], "c".repeat(30).as_bytes());
        let back = Id3v1::parse(&bytes).unwrap();
        assert_eq!(back.track, None);
        assert_eq!(back.comment.len(), 30);

        let mut tag = sample();
        tag.comment = "c".repeat(40);
        let back = Id3v1::parse(&tag.to_bytes()).unwrap();
        assert_eq!(back.comment.len(), 28);
        assert_eq!(back.track, Some(7));
    }

    #[test]
    fn trimming_and_padding() {
        let mut block = [0u8; TAG_LEN];
        block[..3].copy_from_slice(b"TAG");
        block[3..33].copy_from_slice(b"Hello   \0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
        block[33..63].copy_from_slice(&[b' '; 30]);
        block[127] = 255;
        let tag = Id3v1::parse(&block).unwrap();
        assert_eq!(tag.title, "Hello");
        assert_eq!(tag.artist, "");
        assert_eq!(tag.get("genre"), None);
        assert_eq!(tag.get("track"), None);
        assert!(Id3v1::parse(&[0; TAG_LEN]).is_none());
        let out = sample().to_bytes();
        assert!(out[8..33].iter().all(|&b| b == 0));
    }

    #[test]
    fn set_rules() {
        let mut tag = Id3v1::default();
        assert!(tag.set("track", Some("3/12")));
        assert_eq!(tag.track, Some(3));
        tag.set("track", Some("0"));
        assert_eq!(tag.track, None);
        tag.set("track", Some("300"));
        assert_eq!(tag.track, None);
        tag.set("genre", Some("hard rock"));
        assert_eq!(tag.genre, 79);
        tag.set("genre", Some("Psychobilly"));
        assert_eq!(tag.genre, NO_GENRE);
        assert!(!tag.set("composer", Some("x")));
        assert!(tag.is_empty());
    }

    #[test]
    fn text_encoding() {
        assert_eq!(decode_text(&[0x43, 0x61, 0x66, 0xE9]), "Café");
        assert_eq!(decode_text("Café".as_bytes()), "Café");
        assert_eq!(encode_text("Café", 30), vec![0x43, 0x61, 0x66, 0xE9]);
        assert_eq!(encode_text("Ωmega", 30), "Ωmega".as_bytes());
        assert_eq!(decode_text(&encode_text("Ωmega", 30)), "Ωmega");
        // Latin-1 bytes that would read back as UTF-8 are written as UTF-8 instead.
        assert_eq!(decode_text(&encode_text("Ã©", 30)), "Ã©");
        // Truncation stays on a character boundary.
        assert_eq!(encode_text("aΩ", 2), b"a");
        assert_eq!(encode_text("abcdef", 4), b"abcd");
    }
}
