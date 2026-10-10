//! WPL and ZPL, the SMIL-based Windows Media and Zune playlists (10 §3.5).

use crate::entry::PlaylistEntry;
use crate::location::Resolver;
use crate::{Error, xml};

/// Parses `smil/body/seq/media@src` (names case-insensitive), resolving relative paths
/// against the playlist folder. Text is UTF-8 unless a byte-order mark or XML declaration
/// says otherwise. Returns [`Error::NotAPlaylist`] when there is no `smil` element.
pub fn parse(bytes: &[u8], resolver: &Resolver) -> Result<Vec<PlaylistEntry>, Error> {
    let doc = xml::parse(&xml::decode(bytes, None, |_| None), &["media"]);
    let smil = doc.find("smil", true).ok_or(Error::NotAPlaylist)?;
    Ok(smil
        .children_named("body", true)
        .flat_map(|b| b.children_named("seq", true))
        .flat_map(|s| s.children_named("media", true))
        .filter_map(|m| resolver.resolve(m.attr("src")?))
        .map(PlaylistEntry::new)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{NullFs, PathStyle};

    #[test]
    fn media_sources() {
        let t = "<?wpl version=\"1.0\"?>\r\n<smil><head><title>L</title></head><body><seq>\
                 <media src=\"..\\Music\\A &amp; B.mp3\" tid=\"{x}\"/>\
                 <MEDIA SRC=\"sub/c.wma\"></MEDIA>\
                 <media src=\"\"/>\
                 <media src=\"http://h/s\"/>\
                 </seq></body></smil>";
        let r = Resolver::new(Some("/u/Playlists/l.wpl"), PathStyle::Unix, &NullFs);
        let e = parse(t.as_bytes(), &r).unwrap();
        let locs: Vec<_> = e.iter().map(|e| e.location.as_str()).collect();
        assert_eq!(
            locs,
            ["/u/Music/A & B.mp3", "/u/Playlists/sub/c.wma", "http://h/s"]
        );
        assert_eq!(parse(b"<html/>", &r), Err(Error::NotAPlaylist));
    }

    #[test]
    fn zpl_windows() {
        let t = "<?zpl version=\"2.0\"?><smil><body><seq><media src=\"D:\\M\\x.mp3\"/></seq></body></smil>";
        let r = Resolver::new(Some("C:\\L\\l.zpl"), PathStyle::Windows, &NullFs);
        assert_eq!(parse(t.as_bytes(), &r).unwrap()[0].location, "D:\\M\\x.mp3");
    }
}
