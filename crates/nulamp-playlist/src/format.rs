//! Playlist format detection: by extension, HTTP content type, or content (10 §2.4,
//! 07 §8.2).

use crate::location::is_url;
use crate::text::{UTF8_BOM, bom, decode_with, looks_utf16le};

/// A playlist file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// `.m3u`: 8-bit or UTF-8 text.
    M3u,
    /// `.m3u8`: UTF-8 text.
    M3u8,
    /// `.pls`: INI-style `[playlist]`.
    Pls,
    /// `.b4s`: Winamp 3 XML.
    B4s,
    /// `.xspf`: XML Shareable Playlist Format 1.
    Xspf,
    /// `.asx`, `.wax`, `.wvx`, `.wmx`: ASX XML. Content starting with `[Reference]` is read
    /// as [`Format::AsxReference`].
    Asx,
    /// ASX v2: INI-style `[Reference]` with `Ref1`, `Ref2`, ….
    AsxReference,
    /// `.wpl`, `.zpl`: SMIL-based Windows Media / Zune playlists.
    Wpl,
}

impl Format {
    /// Formats that [`crate::write`] supports.
    pub const WRITABLE: [Format; 5] = [
        Format::M3u8,
        Format::M3u,
        Format::Pls,
        Format::Xspf,
        Format::B4s,
    ];

    /// Format for a file extension (case-insensitive, without the dot).
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "m3u" => Format::M3u,
            "m3u8" => Format::M3u8,
            "pls" => Format::Pls,
            "b4s" => Format::B4s,
            "xspf" => Format::Xspf,
            "asx" | "wax" | "wvx" | "wmx" => Format::Asx,
            "wpl" | "zpl" => Format::Wpl,
            _ => return None,
        })
    }

    /// Format from the extension of a path or URL. For URLs the query string and fragment
    /// are ignored.
    pub fn from_path(location: &str) -> Option<Self> {
        extension(location).and_then(Self::from_extension)
    }

    /// Format for an HTTP `Content-Type` value (parameters such as `charset` are ignored).
    ///
    /// `video/x-ms-asf` is also used for real ASF streams; parsing such a stream as ASX
    /// fails with [`crate::Error::NotAPlaylist`] and the caller plays it as a stream.
    pub fn from_content_type(content_type: &str) -> Option<Self> {
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        Some(match mime.as_str() {
            "audio/x-scpls" => Format::Pls,
            "audio/x-mpegurl" | "audio/mpegurl" | "application/x-mpegurl" => Format::M3u,
            "application/vnd.apple.mpegurl" => Format::M3u8,
            "application/xspf+xml" => Format::Xspf,
            "video/x-ms-asf" => Format::Asx,
            _ => return None,
        })
    }

    /// Format from the first bytes of a file without an extension (10 §2.4). `None` means
    /// a media file. A UTF-8 byte-order mark, UTF-16LE text and leading whitespace are
    /// accepted.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        let head = &bytes[..bytes.len().min(256)];
        let text = match bom(head) {
            Some((enc, rest)) => decode_with(enc, rest),
            None if looks_utf16le(head) => decode_with(encoding_rs::UTF_16LE, head),
            None => head.iter().map(|&b| b as char).collect(),
        };
        let t = text.trim_start().to_ascii_lowercase();
        if t.starts_with("[reference]") {
            Some(Format::AsxReference)
        } else if t.starts_with("[playlist]") {
            Some(Format::Pls)
        } else if t.starts_with("#extm3u") {
            Some(Format::M3u)
        } else if t.starts_with("<asx") {
            Some(Format::Asx)
        } else if t.starts_with("<?wpl") || t.starts_with("<?zpl") {
            Some(Format::Wpl)
        } else {
            None
        }
    }

    /// Format by extension; for a location without an extension, by content.
    pub fn detect(location: &str, bytes: &[u8]) -> Option<Self> {
        match extension(location) {
            Some(ext) => Self::from_extension(ext),
            None => Self::sniff(bytes),
        }
    }

    /// Usual file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::M3u => "m3u",
            Format::M3u8 => "m3u8",
            Format::Pls => "pls",
            Format::B4s => "b4s",
            Format::Xspf => "xspf",
            Format::Asx | Format::AsxReference => "asx",
            Format::Wpl => "wpl",
        }
    }
}

/// Extension of the last path segment, without the dot. URL query strings and fragments
/// are ignored. A leading dot (hidden file) is not an extension.
pub fn extension(location: &str) -> Option<&str> {
    let path = if is_url(location) {
        let after_scheme = location.find("://").map_or(0, |i| i + 3);
        let end = location[after_scheme..]
            .find(['?', '#'])
            .map_or(location.len(), |n| after_scheme + n);
        let path = &location[after_scheme..end];
        path.find('/').map_or("", |n| &path[n..])
    } else {
        location
    };
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(0) | None => None,
        Some(i) => Some(&name[i + 1..]).filter(|e| !e.is_empty()),
    }
}

/// True for HLS playlists (`#EXT-X-TARGETDURATION` or `#EXT-X-STREAM-INF`): these are
/// streams, not playlists (07 §8.2).
pub fn is_hls(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    [b"#EXT-X-TARGETDURATION".as_slice(), b"#EXT-X-STREAM-INF"]
        .iter()
        .any(|tag| bytes.windows(tag.len()).any(|w| w == *tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_extension() {
        assert_eq!(Format::from_path("/a/b.M3U"), Some(Format::M3u));
        assert_eq!(Format::from_path("C:\\x\\list.m3u8"), Some(Format::M3u8));
        assert_eq!(Format::from_path("x.PLS"), Some(Format::Pls));
        assert_eq!(Format::from_path("x.wvx"), Some(Format::Asx));
        assert_eq!(Format::from_path("x.zpl"), Some(Format::Wpl));
        assert_eq!(Format::from_path("x.xspf"), Some(Format::Xspf));
        assert_eq!(Format::from_path("x.b4s"), Some(Format::B4s));
        assert_eq!(Format::from_path("x.mp3"), None);
        assert_eq!(Format::from_path("/a.m3u/song"), None);
        assert_eq!(Format::from_path("/a/.pls"), None);
    }

    #[test]
    fn urls_ignore_query() {
        assert_eq!(
            Format::from_path("http://h/listen.pls?sid=1"),
            Some(Format::Pls)
        );
        assert_eq!(
            Format::from_path("https://h/a/b.M3U8#frag"),
            Some(Format::M3u8)
        );
        assert_eq!(Format::from_path("http://h/stream?x=a.pls"), None);
        assert_eq!(Format::from_path("http://h.pls"), None);
        assert_eq!(extension("http://h/"), None);
    }

    #[test]
    fn by_content_type() {
        assert_eq!(
            Format::from_content_type("audio/x-scpls"),
            Some(Format::Pls)
        );
        assert_eq!(
            Format::from_content_type("Audio/X-MpegURL; charset=utf-8"),
            Some(Format::M3u)
        );
        assert_eq!(
            Format::from_content_type("audio/mpegurl"),
            Some(Format::M3u)
        );
        assert_eq!(
            Format::from_content_type("application/vnd.apple.mpegurl"),
            Some(Format::M3u8)
        );
        assert_eq!(
            Format::from_content_type("application/xspf+xml"),
            Some(Format::Xspf)
        );
        assert_eq!(
            Format::from_content_type("video/x-ms-asf"),
            Some(Format::Asx)
        );
        assert_eq!(Format::from_content_type("audio/mpeg"), None);
    }

    #[test]
    fn sniffing() {
        assert_eq!(
            Format::sniff(b"[Reference]\r\nRef1=x"),
            Some(Format::AsxReference)
        );
        assert_eq!(Format::sniff(b"[PLAYLIST]\nFile1=x"), Some(Format::Pls));
        assert_eq!(
            Format::sniff(b"\xFF\xFE[\0p\0l\0a\0y\0l\0i\0s\0t\0]\0"),
            Some(Format::Pls)
        );
        assert_eq!(
            Format::sniff(b"[\0p\0l\0a\0y\0l\0i\0s\0t\0]\0"),
            Some(Format::Pls)
        );
        assert_eq!(Format::sniff(b"\xEF\xBB\xBF#extm3u\n"), Some(Format::M3u));
        assert_eq!(Format::sniff(b"  <Asx version=\"3\">"), Some(Format::Asx));
        assert_eq!(Format::sniff(b"<?wpl version=\"1.0\"?>"), Some(Format::Wpl));
        assert_eq!(Format::sniff(b"<?ZPL version=\"2.0\"?>"), Some(Format::Wpl));
        assert_eq!(Format::sniff(b"ID3\x03\0\0\0"), None);
        assert_eq!(Format::sniff(b""), None);
    }

    #[test]
    fn detect_sniffs_only_without_extension() {
        assert_eq!(Format::detect("/x/noext", b"[playlist]"), Some(Format::Pls));
        assert_eq!(Format::detect("/x/a.mp3", b"[playlist]"), None);
    }

    #[test]
    fn hls() {
        assert!(is_hls(b"#EXTM3U\n#EXT-X-TARGETDURATION:10\nseg.ts"));
        assert!(is_hls(b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8"));
        assert!(!is_hls(b"#EXTM3U\n#EXTINF:1,a\na.mp3"));
    }
}
