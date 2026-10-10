//! B4S, the Winamp 3 XML playlist (10 §3.5).

use crate::entry::{PlaylistEntry, ms_to_s};
use crate::location::{Resolver, is_url, strip_prefix_ci};
use crate::text::xml_escape;
use crate::{Error, WriteOptions, xml};

/// Parses a B4S document: `WinampXML` (or `WasabiXML`) → `playlist` → `entry`. Element
/// and attribute names are case-insensitive.
///
/// `Playstring` loses a leading `file://` or `file:` and is then resolved like a plain
/// path (not percent-decoded: B4S writers store raw paths). `Length` is in milliseconds
/// and is rounded to the nearest second. Returns [`Error::NotAPlaylist`] when there is no
/// `playlist` element.
pub fn parse(bytes: &[u8], resolver: &Resolver) -> Result<Vec<PlaylistEntry>, Error> {
    let doc = xml::parse(&xml::decode(bytes, None, |_| None), &[]);
    let root = doc
        .elements()
        .find(|e| e.is("WinampXML", true) || e.is("WasabiXML", true))
        .unwrap_or(&doc);
    let playlist = root.find("playlist", true).ok_or(Error::NotAPlaylist)?;
    let mut entries = Vec::new();
    for item in playlist.children_named("entry", true) {
        let Some(raw) = item.attr("Playstring").map(str::trim) else {
            continue;
        };
        let raw = strip_prefix_ci(raw, "file://")
            .or_else(|| strip_prefix_ci(raw, "file:"))
            .unwrap_or(raw);
        if raw.trim().is_empty() {
            continue;
        }
        let location = if is_url(raw) {
            raw.trim().to_string()
        } else {
            resolver.resolve_path(raw.trim())
        };
        let mut e = PlaylistEntry::new(location);
        if let Some(name) = item.child_text("Name", true) {
            e.title = Some(name);
            e.length_s = item
                .child_text("Length", true)
                .and_then(|l| l.parse::<u64>().ok())
                .map(ms_to_s);
        }
        entries.push(e);
    }
    Ok(entries)
}

/// Writes a B4S document (UTF-8). Local files get `Playstring="file:<path>"`; `Name` and
/// `Length` (ms) only for entries with a title.
pub fn write(entries: &[PlaylistEntry], opts: &WriteOptions) -> Vec<u8> {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<WinampXML>\n",
    );
    s.push_str(&format!(
        "<playlist num_entries=\"{}\" label=\"Playlist\">\n",
        entries.len()
    ));
    for e in entries {
        let loc = opts.location(&e.location);
        let play = if is_url(&loc) {
            loc
        } else {
            format!("file:{loc}")
        };
        s.push_str(&format!("<entry Playstring=\"{}\">\n", xml_escape(&play)));
        if let Some(title) = &e.title {
            s.push_str(&format!("<Name>{}</Name>\n", xml_escape(title)));
            if let Some(l) = e.length_s {
                s.push_str(&format!("<Length>{}</Length>\n", l.saturating_mul(1000)));
            }
        }
        s.push_str("</entry>\n");
    }
    s.push_str("</playlist>\n</WinampXML>\n");
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{NullFs, PathStyle};

    fn read(text: &str) -> Result<Vec<PlaylistEntry>, Error> {
        parse(
            text.as_bytes(),
            &Resolver::new(Some("/m/l.b4s"), PathStyle::Unix, &NullFs),
        )
    }

    #[test]
    fn lengths_in_ms() {
        let e = read(
            "<?xml version=\"1.0\"?><WinampXML><playlist num_entries=\"3\">\
             <entry Playstring=\"file:/m/a.mp3\"><Name>A</Name><Length>123456</Length></entry>\
             <ENTRY PLAYSTRING=\"file:///m/b.mp3\"><name>B</name><length>59600</length></ENTRY>\
             <entry Playstring=\"c.mp3\"><Length>1000</Length></entry>\
             </playlist></WinampXML>",
        )
        .unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(
            (e[0].location.as_str(), e[0].title.as_deref(), e[0].length_s),
            ("/m/a.mp3", Some("A"), Some(123))
        );
        assert_eq!(
            (e[1].location.as_str(), e[1].length_s),
            ("/m/b.mp3", Some(60))
        );
        assert_eq!(
            (e[2].location.as_str(), e[2].title.as_deref(), e[2].length_s),
            ("/m/c.mp3", None, None)
        );
    }

    #[test]
    fn wasabi_root_and_errors() {
        let e =
            read("<WasabiXML><playlist><entry Playstring=\"http://h/s\"/></playlist></WasabiXML>")
                .unwrap();
        assert_eq!(e[0].location, "http://h/s");
        assert_eq!(read("<WinampXML></WinampXML>"), Err(Error::NotAPlaylist));
    }

    #[test]
    fn round_trip() {
        let entries = vec![
            PlaylistEntry::with_title("/m/a & <b>.mp3", "T \"q\" & é", Some(61)),
            PlaylistEntry::new("/m/sub/x.mp3"),
            PlaylistEntry::with_title("http://h/s?a=1&b=2", "R", None),
        ];
        let opts = WriteOptions {
            style: PathStyle::Unix,
            relative_to: Some("/m".into()),
            ..Default::default()
        };
        let out = write(&entries, &opts);
        let text = String::from_utf8(out.clone()).unwrap();
        assert!(text.contains("num_entries=\"3\""));
        assert!(text.contains("Playstring=\"file:sub/x.mp3\""));
        assert!(text.contains("<Length>61000</Length>"));
        assert_eq!(
            parse(
                &out,
                &Resolver::new(Some("/m/l.b4s"), PathStyle::Unix, &NullFs)
            )
            .unwrap(),
            entries
        );
    }
}
