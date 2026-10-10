//! ASX (`.asx`, `.wax`, `.wvx`, `.wmx`) and the INI-style ASX v2 `[Reference]` (10 §3.5).

use crate::entry::PlaylistEntry;
use crate::location::Resolver;
use crate::text::decode_auto;
use crate::xml::{self, Element};
use crate::{Error, ini};

/// Elements that never contain children, so unclosed ones do not swallow what follows.
const VOID: &[&str] = &[
    "REF",
    "ENTRYREF",
    "PARAM",
    "BASE",
    "MOREINFO",
    "STARTTIME",
    "DURATION",
    "LOGO",
    "BANNER",
];

/// Parses ASX. Content starting with `[Reference]` is read with [`parse_reference`].
///
/// Element and attribute names are case-insensitive; stray `&` and unquoted attributes are
/// accepted. Each `ENTRY` gives one entry from its first `REF@HREF` (further `REF`s are
/// fallbacks for the same item and are ignored); the title is `AUTHOR - TITLE`, or
/// `TITLE` alone. `ENTRYREF@HREF` gives an entry with [`PlaylistEntry::nested`] set.
/// Entries inside `REPEAT COUNT="n"` get `repeat = n` (the innermost `REPEAT` wins; a
/// missing or unreadable count means 0).
///
/// The text is Windows-1252 unless a byte-order mark, a `<PARAM NAME="encoding"
/// VALUE="…">` or an XML declaration says otherwise. Returns [`Error::NotAPlaylist`] when
/// there is no `ASX` element.
pub fn parse(bytes: &[u8], resolver: &Resolver) -> Result<Vec<PlaylistEntry>, Error> {
    if decode_auto(&bytes[..bytes.len().min(64)])
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("[reference]")
    {
        return parse_reference(bytes, resolver);
    }
    let text = xml::decode(bytes, Some(encoding_rs::WINDOWS_1252), param_encoding);
    let doc = xml::parse(&text, VOID);
    let asx = doc.find("ASX", true).ok_or(Error::NotAPlaylist)?;
    let mut entries = Vec::new();
    collect(asx, 0, resolver, &mut entries);
    Ok(entries)
}

fn collect(parent: &Element, repeat: u32, resolver: &Resolver, out: &mut Vec<PlaylistEntry>) {
    for el in parent.elements() {
        if el.is("ENTRY", true) {
            let Some(location) = el
                .child("REF", true)
                .and_then(|r| r.attr("HREF"))
                .and_then(|h| resolver.resolve(h))
            else {
                continue;
            };
            let mut e = PlaylistEntry::new(location);
            e.repeat = repeat;
            e.artist = el.child_text("AUTHOR", true);
            e.title = match (&e.artist, el.child_text("TITLE", true)) {
                (Some(a), Some(t)) => Some(format!("{a} - {t}")),
                (_, t) => t,
            };
            out.push(e);
        } else if el.is("ENTRYREF", true) {
            if let Some(location) = el.attr("HREF").and_then(|h| resolver.resolve(h)) {
                out.push(PlaylistEntry {
                    location,
                    repeat,
                    nested: true,
                    ..Default::default()
                });
            }
        } else if el.is("REPEAT", true) {
            let count = el
                .attr("COUNT")
                .and_then(|c| c.trim().parse().ok())
                .unwrap_or(0);
            collect(el, count, resolver, out);
        }
    }
}

/// Value of `<PARAM NAME="encoding" VALUE="…">`.
fn param_encoding(text: &str) -> Option<String> {
    let doc = xml::parse(text, VOID);
    fn find(e: &Element) -> Option<String> {
        if e.is("PARAM", true)
            && e.attr("NAME")
                .is_some_and(|n| n.trim().eq_ignore_ascii_case("encoding"))
        {
            return e.attr("VALUE").map(str::to_string);
        }
        e.elements().find_map(find)
    }
    find(&doc)
}

/// Parses ASX v2: section `[Reference]` with `Ref1`, `Ref2`, … until one is missing.
/// Returns [`Error::NotAPlaylist`] without that section.
pub fn parse_reference(bytes: &[u8], resolver: &Resolver) -> Result<Vec<PlaylistEntry>, Error> {
    let sec = ini::section(&decode_auto(bytes), "reference").ok_or(Error::NotAPlaylist)?;
    Ok((1..)
        .map_while(|n| sec.get(&format!("ref{n}")))
        .filter_map(|r| resolver.resolve(r))
        .map(PlaylistEntry::new)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{NullFs, PathStyle};

    fn read_bytes(bytes: &[u8]) -> Result<Vec<PlaylistEntry>, Error> {
        parse(
            bytes,
            &Resolver::new(Some("/m/l.asx"), PathStyle::Unix, &NullFs),
        )
    }

    fn read(text: &str) -> Result<Vec<PlaylistEntry>, Error> {
        read_bytes(text.as_bytes())
    }

    #[test]
    fn entries_titles_case_insensitive() {
        let e = read(
            "<asx VERSION=\"3.0\"><Title>List</Title>\
             <Entry><Title>Song</Title><Author>Band</Author><Ref href=\"a.wma\"/><REF HREF=\"fallback.wma\"/></Entry>\
             <ENTRY><TITLE>Only title</TITLE><REF HREF=\"http://h/s\"></REF></ENTRY>\
             <ENTRY><TITLE>No ref</TITLE></ENTRY>\
             </ASX>",
        )
        .unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[0].location.as_str(), e[0].title.as_deref()),
            ("/m/a.wma", Some("Band - Song"))
        );
        assert_eq!(e[0].artist.as_deref(), Some("Band"));
        assert_eq!(
            (e[1].location.as_str(), e[1].title.as_deref()),
            ("http://h/s", Some("Only title"))
        );
    }

    #[test]
    fn lenient_syntax() {
        let e = read(
            "<ASX version=3>\n<ENTRY><TITLE>Rock & Roll &amp; more</TITLE>\
             <REF HREF=http://h/a?x=1&y=2></ENTRY>\n<ENTRY><REF HREF='b.mp3'><TITLE>Unclosed ref</TITLE></ENTRY>",
        )
        .unwrap();
        assert_eq!(e[0].title.as_deref(), Some("Rock & Roll & more"));
        assert_eq!(e[0].location, "http://h/a?x=1&y=2");
        assert_eq!(
            (e[1].location.as_str(), e[1].title.as_deref()),
            ("/m/b.mp3", Some("Unclosed ref"))
        );
    }

    #[test]
    fn entryref_and_repeat() {
        let e = read(
            "<asx><entryref href=\"other.asx\"/><repeat count=\"2\"><entry><ref href=\"a.mp3\"/></entry>\
             <entryref HREF=\"http://h/x.asx\"/></repeat><entry><ref href=\"b.mp3\"/></entry></asx>",
        )
        .unwrap();
        assert_eq!(e.len(), 4);
        assert!(e[0].nested && e[0].location == "/m/other.asx" && e[0].repeat == 0);
        assert_eq!(
            (e[1].location.as_str(), e[1].repeat, e[1].nested),
            ("/m/a.mp3", 2, false)
        );
        assert!(e[2].nested && e[2].repeat == 2);
        assert_eq!(e[3].repeat, 0);
    }

    #[test]
    fn encodings() {
        let e = read_bytes(b"<asx><entry><title>Caf\xE9</title><ref href=\"a\"/></entry></asx>")
            .unwrap();
        assert_eq!(e[0].title.as_deref(), Some("Café"));
        let e = read_bytes(
            "<asx><param name=\"Encoding\" value=\"UTF-8\"/><entry><title>Café ☃</title><ref href=\"a\"/></entry></asx>"
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(e[0].title.as_deref(), Some("Café ☃"));
        let e = read_bytes(
            b"\xEF\xBB\xBF<asx><entry><title>\xC3\xA9</title><ref href=\"a\"/></entry></asx>",
        )
        .unwrap();
        assert_eq!(e[0].title.as_deref(), Some("é"));
    }

    #[test]
    fn not_asx() {
        assert_eq!(
            read_bytes(b"\x30\x26\xB2\x75\x8E\x66\xCF\x11 binary asf"),
            Err(Error::NotAPlaylist)
        );
    }

    #[test]
    fn reference_ini() {
        let t = "[Reference]\r\nRef1=http://h/a.wmv?MSWMExt=.asf\r\nRef2=mms://h/b\r\nRef4=http://h/skipped\r\n";
        let e = read(t).unwrap();
        assert_eq!(
            e.iter().map(|e| e.location.as_str()).collect::<Vec<_>>(),
            ["http://h/a.wmv?MSWMExt=.asf", "mms://h/b"]
        );
        let r = Resolver::new(None, PathStyle::Unix, &NullFs);
        assert_eq!(
            parse_reference(b"[Other]\nRef1=x", &r),
            Err(Error::NotAPlaylist)
        );
    }
}
