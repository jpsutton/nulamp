//! XSPF, XML Shareable Playlist Format version 1 (10 §3.5, §3.6).

use crate::entry::{PlaylistEntry, ms_to_s};
use crate::location::{Resolver, is_url, path_to_file_uri};
use crate::text::{percent_decode, xml_escape};
use crate::{Error, WriteOptions, xml};

/// Parses `playlist/trackList/track`. Element names are case-sensitive; namespace
/// prefixes are ignored.
///
/// `location` is the first `location` child: `file:` URIs are decoded, other URLs kept,
/// and relative URI references are percent-decoded and resolved against the playlist.
/// Tracks without a location are skipped. The title is `creator - title` when both exist,
/// else `title`. `duration` (ms) is used only with a title. Returns
/// [`Error::NotAPlaylist`] when there is no `playlist` element.
pub fn parse(bytes: &[u8], resolver: &Resolver) -> Result<Vec<PlaylistEntry>, Error> {
    let doc = xml::parse(&xml::decode(bytes, None, |_| None), &[]);
    let playlist = doc.child("playlist", false).ok_or(Error::NotAPlaylist)?;
    let mut entries = Vec::new();
    for list in playlist.children_named("trackList", false) {
        for track in list.children_named("track", false) {
            let Some(raw) = track.child_text("location", false) else {
                continue;
            };
            let location = if is_url(&raw) || raw.to_ascii_lowercase().starts_with("file:") {
                resolver.resolve(&raw)
            } else {
                resolver.resolve(&percent_decode(&raw))
            };
            let Some(location) = location else {
                continue;
            };
            let mut e = PlaylistEntry::new(location);
            let title = track.child_text("title", false);
            e.artist = track.child_text("creator", false);
            e.album = track.child_text("album", false);
            e.track_number = track
                .child_text("trackNum", false)
                .and_then(|n| n.parse().ok());
            e.title = match (&e.artist, title) {
                (Some(a), Some(t)) => Some(format!("{a} - {t}")),
                (_, t) => t,
            };
            if e.title.is_some() {
                e.length_s = track
                    .child_text("duration", false)
                    .and_then(|d| d.parse::<u64>().ok())
                    .map(ms_to_s);
            }
            entries.push(e);
        }
    }
    Ok(entries)
}

/// Writes an XSPF document (UTF-8).
///
/// Local files become `file://` URIs, or relative URI references under
/// [`WriteOptions::relative_to`]. `title` and `duration` are written only for entries with
/// a title. `creator` is written only when the title starts with `<artist> - `, and that
/// prefix is then removed from `title`, so reading the file back gives the same title.
pub fn write(entries: &[PlaylistEntry], opts: &WriteOptions) -> Vec<u8> {
    let mut s = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<playlist version=\"1\" xmlns=\"http://xspf.org/ns/0/\">\n",
        "  <trackList>\n",
    ));
    let tag = |s: &mut String, name: &str, value: &str| {
        s.push_str(&format!("      <{name}>{}</{name}>\n", xml_escape(value)));
    };
    for e in entries {
        s.push_str("    <track>\n");
        let loc = opts.location(&e.location);
        let uri = if is_url(&loc) {
            loc
        } else {
            path_to_file_uri(&loc, opts.style)
        };
        tag(&mut s, "location", &uri);
        if let Some(title) = &e.title {
            let split = e
                .artist
                .as_deref()
                .and_then(|a| Some((a, title.strip_prefix(a)?.strip_prefix(" - ")?)));
            match split {
                Some((artist, rest)) => {
                    tag(&mut s, "title", rest);
                    tag(&mut s, "creator", artist);
                }
                None => tag(&mut s, "title", title),
            }
        }
        if let Some(album) = &e.album {
            tag(&mut s, "album", album);
        }
        if let Some(n) = e.track_number {
            tag(&mut s, "trackNum", &n.to_string());
        }
        if let (Some(_), Some(l)) = (&e.title, e.length_s) {
            tag(&mut s, "duration", &l.saturating_mul(1000).to_string());
        }
        s.push_str("    </track>\n");
    }
    s.push_str("  </trackList>\n</playlist>\n");
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{NullFs, PathStyle};

    fn read(text: &str) -> Result<Vec<PlaylistEntry>, Error> {
        parse(
            text.as_bytes(),
            &Resolver::new(Some("/m/l.xspf"), PathStyle::Unix, &NullFs),
        )
    }

    #[test]
    fn file_uri_with_escape() {
        let e = read(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><playlist version=\"1\" xmlns=\"http://xspf.org/ns/0/\">\
             <trackList><track><location>file:///home/u/My%20Music/a%C3%A9.mp3</location></track></trackList></playlist>",
        )
        .unwrap();
        assert_eq!(e[0].location, "/home/u/My Music/aé.mp3");
    }

    #[test]
    fn fields() {
        let e = read(
            "<playlist><trackList>\
             <track><location>sub/b%20c.ogg</location><title>T</title><creator>C</creator><album>Al</album>\
             <trackNum>4</trackNum><duration>200400</duration></track>\
             <track><title>No location</title></track>\
             <track><location>http://h/s?a=1&amp;b=2</location><creator>Only creator</creator><duration>5000</duration></track>\
             <track><Location>/ignored-case.mp3</Location><location>/x.mp3</location></track>\
             </trackList></playlist>",
        )
        .unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].location, "/m/sub/b c.ogg");
        assert_eq!(e[0].title.as_deref(), Some("C - T"));
        assert_eq!(
            (e[0].length_s, e[0].album.as_deref(), e[0].track_number),
            (Some(200), Some("Al"), Some(4))
        );
        assert_eq!(e[1].location, "http://h/s?a=1&b=2");
        assert_eq!((e[1].title.as_deref(), e[1].length_s), (None, None));
        assert_eq!(e[2].location, "/x.mp3");
        assert_eq!(read("<Playlist/>"), Err(Error::NotAPlaylist));
    }

    #[test]
    fn round_trip() {
        let mut a = PlaylistEntry::with_title("/m/a b/ü&.mp3", "Art - Song <1>", Some(42));
        a.artist = Some("Art".into());
        a.album = Some("Alb".into());
        a.track_number = Some(3);
        let mut b = PlaylistEntry::with_title("/m/sub/x.mp3", "Plain", None);
        b.artist = Some("Other".into());
        let entries = vec![
            a,
            b,
            PlaylistEntry::new("http://h/s?x=1&y=2"),
            PlaylistEntry::new("/elsewhere/q#1.mp3"),
        ];
        let opts = WriteOptions {
            style: PathStyle::Unix,
            relative_to: Some("/m".into()),
            ..Default::default()
        };
        let out = write(&entries, &opts);
        let text = String::from_utf8(out.clone()).unwrap();
        assert!(text.contains("<location>a%20b/%C3%BC%26.mp3</location>"));
        assert!(text.contains("<location>file:///elsewhere/q%231.mp3</location>"));
        assert!(text.contains("<title>Song &lt;1&gt;</title>\n      <creator>Art</creator>"));
        assert!(text.contains("<duration>42000</duration>"));
        let mut back = parse(
            &out,
            &Resolver::new(Some("/m/l.xspf"), PathStyle::Unix, &NullFs),
        )
        .unwrap();
        // `creator` is not written when it is not part of the title.
        back[1].artist = Some("Other".into());
        assert_eq!(back, entries);
    }

    #[test]
    fn windows_paths() {
        let opts = WriteOptions {
            style: PathStyle::Windows,
            ..Default::default()
        };
        let out = write(&[PlaylistEntry::new("C:\\M u\\a.mp3")], &opts);
        assert!(String::from_utf8_lossy(&out).contains("file:///C:/M%20u/a.mp3"));
        let back = parse(
            &out,
            &Resolver::new(Some("C:\\l.xspf"), PathStyle::Windows, &NullFs),
        )
        .unwrap();
        assert_eq!(back[0].location, "C:\\M u\\a.mp3");
    }
}
