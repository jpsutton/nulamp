//! M3U and M3U8 (10 §3.1, §3.2).

use crate::entry::PlaylistEntry;
use crate::format::is_hls;
use crate::location::Resolver;
use crate::text::{UTF8_BOM, bom, decode_utf8_or_1252, decode_with, encode_1252, one_line};
use crate::{Error, WriteOptions};

/// Maximum title length in characters; longer titles are truncated.
pub const MAX_TITLE_CHARS: usize = 260;

/// Decodes M3U text (10 §3.1): a byte-order mark decides; else UTF-8 when `utf8` (the
/// `.m3u8` extension) or when a line is `#UTF8`; else strict UTF-8, then Windows-1252.
///
/// A `#UTF8` line switches the whole file, not only the lines after it (in practice it
/// follows `#EXTM3U`, which is ASCII).
pub fn decode(bytes: &[u8], utf8: bool) -> String {
    if let Some((enc, rest)) = bom(bytes) {
        return decode_with(enc, rest);
    }
    let utf8_line = bytes
        .split(|&b| b == b'\n' || b == b'\r')
        .any(|l| l.trim_ascii() == b"#UTF8");
    if utf8 || utf8_line {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    decode_utf8_or_1252(bytes)
}

#[derive(Default)]
struct Pending {
    title: String,
    length: i64,
    extras: Vec<(String, String)>,
    artist: Option<String>,
    album: Option<String>,
}

/// Parses M3U (`utf8 == false`) or M3U8 (`utf8 == true`). Returns [`Error::Stream`] for
/// HLS playlists.
///
/// Unknown `#EXT…` directives (`#EXTVLCOPT`, `#EXTGRP`, …) between `#EXTINF` and its
/// entry are ignored and keep the pending title; `#EXTART` and `#EXTALB` set the artist
/// and album of the next entry. Other `#` lines and whitespace-only lines clear the
/// pending `#EXTINF`; empty lines do not.
pub fn parse(bytes: &[u8], utf8: bool, resolver: &Resolver) -> Result<Vec<PlaylistEntry>, Error> {
    if is_hls(bytes) {
        return Err(Error::Stream);
    }
    let text = decode(bytes, utf8);
    let mut entries = Vec::new();
    let mut extended = false;
    let mut pending = Pending::default();
    for raw in text.split(['\r', '\n']) {
        if raw.is_empty() {
            continue;
        }
        let line = raw.trim_start_matches([' ', '\t', '\u{feff}']);
        if line.trim().is_empty() {
            pending = Pending::default();
            continue;
        }
        if !extended && line.contains("#EXTM3U") {
            extended = true;
            continue;
        }
        if line.starts_with('#') {
            if !extended {
                continue;
            }
            if let Some(rest) = line.strip_prefix("#EXTINF:") {
                let (length, title, extras) = parse_extinf(rest);
                pending = Pending {
                    title,
                    length,
                    extras,
                    ..pending
                };
            } else if let Some(v) = line.strip_prefix("#EXTART:") {
                pending.artist = Some(v.trim().to_string()).filter(|v| !v.is_empty());
            } else if let Some(v) = line.strip_prefix("#EXTALB:") {
                pending.album = Some(v.trim().to_string()).filter(|v| !v.is_empty());
            } else if !line.starts_with("#EXT") && line.trim_end() != "#UTF8" {
                pending = Pending::default();
            }
            continue;
        }
        let p = std::mem::take(&mut pending);
        let loc = line.strip_prefix("ASF ").unwrap_or(line);
        let Some(location) = resolver.resolve(loc) else {
            continue;
        };
        let mut e = PlaylistEntry {
            location,
            extras: p.extras,
            artist: p.artist,
            album: p.album,
            ..Default::default()
        };
        if !p.title.is_empty() {
            e.title = Some(truncate(&p.title));
            e.length_s = u64::try_from(p.length).ok();
        }
        entries.push(e);
    }
    Ok(entries)
}

/// Parses the text after `#EXTINF:` into (seconds or -1, title, attributes).
///
/// A fractional part of the duration is ignored. The title starts after the first comma
/// that is not inside a double-quoted attribute value (the first comma when quotes are
/// unbalanced), so `tvg-name="a, b"` stays intact.
fn parse_extinf(s: &str) -> (i64, String, Vec<(String, String)>) {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut i = usize::from(matches!(b.first(), Some(b'-' | b'+')));
    let digits = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let length = if i > digits {
        s[..i].parse().unwrap_or(-1)
    } else {
        -1
    };
    if i > digits && b.get(i) == Some(&b'.') {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    let rest = &s[i..];
    if rest.trim().is_empty() {
        return (-1, String::new(), Vec::new());
    }
    let Some(comma) = title_comma(rest) else {
        return (length, String::new(), parse_attrs(rest));
    };
    (
        length,
        rest[comma + 1..].to_string(),
        parse_attrs(&rest[..comma]),
    )
}

fn title_comma(s: &str) -> Option<usize> {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => return Some(i),
            _ => {}
        }
    }
    s.find(',')
}

/// Parses `key="value" key2="value 2"`; unquoted values end at whitespace.
fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let key_end = rest
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rest.len());
        if key_end == 0 {
            rest = rest[1..].trim_start();
            continue;
        }
        let key = &rest[..key_end];
        rest = &rest[key_end..];
        if let Some(after) = rest.strip_prefix('=') {
            let (value, next) = match after.strip_prefix('"') {
                Some(q) => match q.find('"') {
                    Some(n) => (&q[..n], &q[n + 1..]),
                    None => (q, ""),
                },
                None => {
                    let n = after.find(char::is_whitespace).unwrap_or(after.len());
                    (&after[..n], &after[n..])
                }
            };
            out.push((key.to_string(), value.to_string()));
            rest = next;
        }
        rest = rest.trim_start();
    }
    out
}

fn truncate(title: &str) -> String {
    match title.char_indices().nth(MAX_TITLE_CHARS) {
        Some((i, _)) => title[..i].to_string(),
        None => title.to_string(),
    }
}

/// Writes M3U (`utf8_bom == false`) or M3U8 (`utf8_bom == true`), LF line endings.
///
/// M3U8 is UTF-8 with a byte-order mark. M3U is UTF-8 without one, or Windows-1252 with
/// [`WriteOptions::legacy_8bit`]. `#EXTINF` (with the entry's attributes) is written only
/// for entries with a title.
pub fn write(entries: &[PlaylistEntry], utf8_bom: bool, opts: &WriteOptions) -> Vec<u8> {
    let mut s = String::from("#EXTM3U\n");
    for e in entries {
        if let Some(title) = &e.title {
            let length = e
                .length_s
                .map_or(-1, |l| i64::try_from(l).unwrap_or(i64::MAX));
            s.push_str(&format!("#EXTINF:{length}"));
            for (k, v) in &e.extras {
                s.push_str(&format!(
                    " {}=\"{}\"",
                    one_line(k),
                    one_line(v).replace('"', "'")
                ));
            }
            s.push_str(&format!(",{}\n", one_line(title)));
        }
        s.push_str(&one_line(&opts.location(&e.location)));
        s.push('\n');
    }
    if utf8_bom {
        let mut out = UTF8_BOM.to_vec();
        out.extend_from_slice(s.as_bytes());
        out
    } else if opts.legacy_8bit {
        encode_1252(&s)
    } else {
        s.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{NullFs, PathStyle};

    const BASE: &str = "/music/lists/x.m3u";

    fn read(text: &str) -> Vec<PlaylistEntry> {
        read_bytes(text.as_bytes(), false)
    }

    fn read_bytes(bytes: &[u8], utf8: bool) -> Vec<PlaylistEntry> {
        parse(
            bytes,
            utf8,
            &Resolver::new(Some(BASE), PathStyle::Unix, &NullFs),
        )
        .unwrap()
    }

    fn opts() -> WriteOptions {
        WriteOptions {
            style: PathStyle::Unix,
            ..Default::default()
        }
    }

    // Spec 12 §2.4 rows.

    #[test]
    fn extinf_title_with_commas() {
        let e = read("#EXTM3U\n#EXTINF:123,A, B\n/a.mp3\n");
        assert_eq!(e[0].title.as_deref(), Some("A, B"));
        assert_eq!(e[0].length_s, Some(123));
    }

    #[test]
    fn extinf_before_extm3u_is_a_comment() {
        let e = read("#EXTINF:5,Early\n/a.mp3\n#EXTM3U\n#EXTINF:6,Late\n/b.mp3\n");
        assert_eq!(e[0].title, None);
        assert_eq!(e[0].length_s, None);
        assert_eq!(e[1].title.as_deref(), Some("Late"));
    }

    #[test]
    fn negative_length_with_url() {
        let e = read("#EXTM3U\n#EXTINF:-1,Radio\nhttp://radio.example:8000/stream\n");
        assert_eq!(e[0].location, "http://radio.example:8000/stream");
        assert_eq!(e[0].title.as_deref(), Some("Radio"));
        assert_eq!(e[0].length_s, None);
    }

    #[test]
    fn empty_title_drops_length() {
        let e = read("#EXTM3U\n#EXTINF:200,\n/a.mp3\n");
        assert_eq!((e[0].title.clone(), e[0].length_s), (None, None));
    }

    #[test]
    fn unknown_ext_directive_keeps_title() {
        let e = read("#EXTM3U\n#EXTINF:10,T\n#EXTVLCOPT:x\n#EXTGRP:g\n/a.mp3\n");
        assert_eq!(e[0].title.as_deref(), Some("T"));
        assert_eq!(e[0].length_s, Some(10));
    }

    #[test]
    fn iptv_attributes() {
        let e = read(
            "#EXTM3U\n#EXTINF:-1 tvg-id=\"ch.1\" tvg-name=\"One, HD\" tvg-logo=\"http://l/x.png\" group-title=\"News\",Channel One, HD\nhttp://iptv/1\n",
        );
        assert_eq!(e[0].title.as_deref(), Some("Channel One, HD"));
        assert_eq!(e[0].extra("tvg-id"), Some("ch.1"));
        assert_eq!(e[0].extra("TVG-NAME"), Some("One, HD"));
        assert_eq!(e[0].extra("group-title"), Some("News"));
        assert_eq!(e[0].extras.len(), 4);
    }

    #[test]
    fn relative_locations() {
        let e = read("../a.mp3\nsub\\b.mp3\n");
        assert_eq!(e[0].location, "/music/a.mp3");
        assert_eq!(e[1].location, "/music/lists/sub/b.mp3");
    }

    #[test]
    fn m3u8_written_with_bom() {
        let out = write(&[PlaylistEntry::new("/a.mp3")], true, &opts());
        assert!(out.starts_with(b"\xEF\xBB\xBF#EXTM3U\n"));
    }

    // Further §3.1 edge cases.

    #[test]
    fn comment_and_whitespace_lines_clear_pending() {
        let e = read(
            "#EXTM3U\n#EXTINF:1,A\n# comment\n/a.mp3\n#EXTINF:2,B\n \t \n/b.mp3\n#EXTINF:3,C\n\n\n/c.mp3\n",
        );
        assert_eq!(e[0].title, None);
        assert_eq!(e[1].title, None);
        assert_eq!(e[2].title.as_deref(), Some("C"));
    }

    #[test]
    fn leading_whitespace_asf_prefix_and_crlf() {
        let e = read("  #EXTM3U\r\n\t#EXTINF:7,T\r\n  ASF http://h/a.asf\r\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].location, "http://h/a.asf");
        assert_eq!(e[0].title.as_deref(), Some("T"));
    }

    #[test]
    fn extm3u_as_substring() {
        let e = read("\u{feff}junk #EXTM3U\n#EXTINF:4,T\n/a.mp3\n");
        assert_eq!(e[0].title.as_deref(), Some("T"));
    }

    #[test]
    fn extinf_variants() {
        assert_eq!(parse_extinf("123"), (-1, String::new(), vec![]));
        assert_eq!(parse_extinf("12.5,T").0, 12);
        assert_eq!(parse_extinf(" 5,T").1, "T");
        assert_eq!(parse_extinf("abc,T"), (-1, "T".into(), vec![]));
        let (l, t, a) = parse_extinf("0 ext=\"mp3\",Song");
        assert_eq!((l, t.as_str()), (0, "Song"));
        assert_eq!(a, vec![("ext".to_string(), "mp3".to_string())]);
    }

    #[test]
    fn extinf_without_comma_has_no_title() {
        let e = read("#EXTM3U\n#EXTINF:9 Title\n/a.mp3\n");
        assert_eq!((e[0].title.clone(), e[0].length_s), (None, None));
    }

    #[test]
    fn title_truncated() {
        let long = "é".repeat(300);
        let e = read(&format!("#EXTM3U\n#EXTINF:1,{long}\n/a.mp3\n"));
        assert_eq!(
            e[0].title.as_ref().unwrap().chars().count(),
            MAX_TITLE_CHARS
        );
    }

    #[test]
    fn artist_album_directives() {
        let e = read("#EXTM3U\n#EXTART:Art\n#EXTINF:1,T\n#EXTALB:Alb\n/a.mp3\n/b.mp3\n");
        assert_eq!(e[0].artist.as_deref(), Some("Art"));
        assert_eq!(e[0].album.as_deref(), Some("Alb"));
        assert_eq!(e[0].title.as_deref(), Some("T"));
        assert_eq!(e[1].artist, None);
    }

    #[test]
    fn hls_is_a_stream() {
        let r = Resolver::new(Some(BASE), PathStyle::Unix, &NullFs);
        assert_eq!(
            parse(
                b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\na.ts\n",
                true,
                &r
            ),
            Err(Error::Stream)
        );
    }

    #[test]
    fn encodings() {
        assert_eq!(read_bytes(b"/caf\xE9.mp3", false)[0].location, "/café.mp3");
        assert_eq!(
            read_bytes("/café.mp3".as_bytes(), false)[0].location,
            "/café.mp3"
        );
        // `.m3u8` and `#UTF8` force UTF-8 (invalid bytes are replaced, not read as 1252).
        assert_eq!(
            read_bytes(b"/caf\xE9.mp3", true)[0].location,
            "/caf\u{fffd}.mp3"
        );
        assert_eq!(
            read_bytes(b"#UTF8\n/caf\xE9.mp3", false)[0].location,
            "/caf\u{fffd}.mp3"
        );
        assert_eq!(
            read_bytes(b"\xEF\xBB\xBF/caf\xC3\xA9.mp3", false)[0].location,
            "/café.mp3"
        );
        assert_eq!(
            read_bytes(b"\xFF\xFE/\0\xE9\0.\0m\0", false)[0].location,
            "/é.m"
        );
    }

    #[test]
    fn round_trip() {
        let mut a = PlaylistEntry::with_title("/m/a.mp3", "Artist - Song, Live", Some(321));
        a.extras = vec![("tvg-id".into(), "x".into()), ("ext".into(), "mp3".into())];
        let entries = vec![
            a,
            PlaylistEntry::new("/m/no meta.ogg"),
            PlaylistEntry::with_title("http://h/s", "Radio ☃", None),
        ];
        for bom in [true, false] {
            let out = write(&entries, bom, &opts());
            let back = read_bytes(&out, bom);
            assert_eq!(back, entries);
        }
        let text = String::from_utf8(write(&entries, false, &opts())).unwrap();
        assert_eq!(
            text,
            "#EXTM3U\n#EXTINF:321 tvg-id=\"x\" ext=\"mp3\",Artist - Song, Live\n/m/a.mp3\n/m/no meta.ogg\n#EXTINF:-1,Radio ☃\nhttp://h/s\n"
        );
    }

    #[test]
    fn legacy_8bit_round_trip() {
        let entries = vec![PlaylistEntry::with_title("/m/café.mp3", "Café", Some(1))];
        let o = WriteOptions {
            legacy_8bit: true,
            ..opts()
        };
        let out = write(&entries, false, &o);
        assert!(out.windows(4).any(|w| w == b"Caf\xE9"));
        assert_eq!(read_bytes(&out, false), entries);
    }

    #[test]
    fn relative_writing() {
        let o = WriteOptions {
            relative_to: Some("/m/lists".into()),
            ..opts()
        };
        let out = write(
            &[
                PlaylistEntry::new("/m/lists/s/a.mp3"),
                PlaylistEntry::new("/m/b.mp3"),
            ],
            false,
            &o,
        );
        assert_eq!(out, b"#EXTM3U\ns/a.mp3\n/m/b.mp3\n");
        let back = parse(
            &out,
            false,
            &Resolver::new(Some("/m/lists/l.m3u"), PathStyle::Unix, &NullFs),
        )
        .unwrap();
        assert_eq!(back[0].location, "/m/lists/s/a.mp3");
    }
}
