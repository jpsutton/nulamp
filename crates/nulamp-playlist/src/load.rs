//! Format dispatch and recursive loading of nested playlists (10 §2.3).

use crate::entry::PlaylistEntry;
use crate::format::Format;
use crate::location::{FileSystem, NullFs, PathStyle, Resolver, StdFs, is_url};
use crate::{Error, asx, b4s, m3u, pls, wpl, xspf};

/// Parses `bytes` as `format`. Nested playlists are not expanded (see [`Loader`]).
pub fn parse(
    bytes: &[u8],
    format: Format,
    resolver: &Resolver,
) -> Result<Vec<PlaylistEntry>, Error> {
    match format {
        Format::M3u => m3u::parse(bytes, false, resolver),
        Format::M3u8 => m3u::parse(bytes, true, resolver),
        Format::Pls => pls::parse(bytes, resolver),
        Format::B4s => b4s::parse(bytes, resolver),
        Format::Xspf => xspf::parse(bytes, resolver),
        Format::Asx => asx::parse(bytes, resolver),
        Format::AsxReference => asx::parse_reference(bytes, resolver),
        Format::Wpl => wpl::parse(bytes, resolver),
    }
}

/// Loads a playlist and expands nested playlists in place.
///
/// An entry is expanded when it is an ASX `ENTRYREF` or its location has a playlist
/// extension. Expansion is skipped (and the entry dropped) when that playlist is already
/// being loaded further up the chain, so a playlist that includes itself loads once, or
/// when [`Loader::max_depth`] playlists are already open. A nested playlist that cannot be
/// fetched, has an unknown format, is not a valid playlist, or is an HLS stream is kept as
/// an ordinary entry.
pub struct Loader<'a> {
    style: PathStyle,
    fs: &'a dyn FileSystem,
    max_depth: usize,
}

impl<'a> Loader<'a> {
    /// Maximum chain of open playlists, including the top-level one.
    pub const DEFAULT_MAX_DEPTH: usize = 8;

    pub fn new(style: PathStyle, fs: &'a dyn FileSystem) -> Self {
        Self {
            style,
            fs,
            max_depth: Self::DEFAULT_MAX_DEPTH,
        }
    }

    /// Native style and the real file system.
    pub fn native() -> Loader<'static> {
        Loader::new(PathStyle::native(), &StdFs)
    }

    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth.max(1);
        self
    }

    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Loads the playlist at `location`. `fetch` returns the bytes of a location (file or
    /// URL), or `None` when it cannot be read. The format comes from the extension, or
    /// from the content when there is none.
    pub fn load<F>(&self, location: &str, mut fetch: F) -> Result<Vec<PlaylistEntry>, Error>
    where
        F: FnMut(&str) -> Option<Vec<u8>>,
    {
        let mut active = Vec::new();
        self.expand(location, &mut fetch, &mut active)
    }

    fn expand(
        &self,
        location: &str,
        fetch: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
        active: &mut Vec<String>,
    ) -> Result<Vec<PlaylistEntry>, Error> {
        let bytes = fetch(location).ok_or_else(|| Error::Unreadable(location.to_string()))?;
        let format = Format::detect(location, &bytes).ok_or(Error::UnknownFormat)?;
        let entries = parse(
            &bytes,
            format,
            &Resolver::new(Some(location), self.style, self.fs),
        )?;
        active.push(self.key(location));
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            if !e.nested && Format::from_path(&e.location).is_none() {
                out.push(e);
                continue;
            }
            if active.len() >= self.max_depth || active.contains(&self.key(&e.location)) {
                continue;
            }
            match self.expand(&e.location, fetch, active) {
                Ok(children) => out.extend(children.into_iter().map(|mut c| {
                    c.repeat = c.repeat.max(e.repeat);
                    c
                })),
                Err(_) => out.push(PlaylistEntry { nested: false, ..e }),
            }
        }
        active.pop();
        Ok(out)
    }

    /// Identity of a playlist location for cycle detection.
    fn key(&self, location: &str) -> String {
        let normalized = Resolver::new(None, self.style, &NullFs)
            .resolve(location)
            .unwrap_or_else(|| location.to_string());
        match self.style {
            PathStyle::Windows if !is_url(location) => normalized.to_lowercase(),
            _ => normalized,
        }
    }
}

/// Reads a local playlist file with the native path style and the real file system,
/// expanding nested local playlists. Format by extension, or by content when the file has
/// none (10 §2.4).
pub fn read_file(path: &str) -> Result<Vec<PlaylistEntry>, Error> {
    Loader::native().load(path, |p| {
        if is_url(p) {
            None
        } else {
            std::fs::read(p).ok()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::collections::HashMap;

    fn loader() -> Loader<'static> {
        Loader::new(PathStyle::Unix, &NullFs)
    }

    fn mem(files: &[(&str, &str)]) -> HashMap<String, Vec<u8>> {
        files
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    fn locations(e: &[PlaylistEntry]) -> Vec<&str> {
        e.iter().map(|e| e.location.as_str()).collect()
    }

    #[test]
    fn self_including_playlist_loads_once() {
        let files = mem(&[("/p/a.m3u", "x.mp3\na.m3u\n/p/./a.m3u\ny.mp3\n")]);
        let mut fetches = 0;
        let e = loader()
            .load("/p/a.m3u", |p| {
                fetches += 1;
                files.get(p).cloned()
            })
            .unwrap();
        assert_eq!(locations(&e), ["/p/x.mp3", "/p/y.mp3"]);
        assert_eq!(fetches, 1);
    }

    #[test]
    fn mutual_recursion_and_nesting() {
        let files = mem(&[
            ("/p/a.m3u", "1.mp3\nb.pls\n4.mp3\n"),
            (
                "/p/b.pls",
                "[playlist]\nFile1=2.mp3\nFile2=sub/c.xspf\nFile3=a.m3u\nNumberOfEntries=3\n",
            ),
            (
                "/p/sub/c.xspf",
                "<playlist><trackList><track><location>3.mp3</location></track></trackList></playlist>",
            ),
        ]);
        let e = loader()
            .load("/p/a.m3u", |p| files.get(p).cloned())
            .unwrap();
        assert_eq!(
            locations(&e),
            ["/p/1.mp3", "/p/2.mp3", "/p/sub/3.mp3", "/p/4.mp3"]
        );
    }

    #[test]
    fn depth_limit() {
        let files: HashMap<String, Vec<u8>> = (0..20)
            .map(|i| {
                (
                    format!("/p/{i}.m3u"),
                    format!("{i}.mp3\n{}.m3u\n", i + 1).into_bytes(),
                )
            })
            .collect();
        let e = loader()
            .load("/p/0.m3u", |p| files.get(p).cloned())
            .unwrap();
        assert_eq!(e.len(), Loader::DEFAULT_MAX_DEPTH);
        let e = loader()
            .with_max_depth(2)
            .load("/p/0.m3u", |p| files.get(p).cloned())
            .unwrap();
        assert_eq!(locations(&e), ["/p/0.mp3", "/p/1.mp3"]);
    }

    #[test]
    fn unreadable_or_stream_nested_entries_are_kept() {
        let files = mem(&[
            (
                "/p/a.m3u8",
                "missing.pls\nhttp://h/live.m3u8\n#EXTM3U\n#EXTINF:1,T\nbad.pls\n",
            ),
            (
                "http://h/live.m3u8",
                "#EXTM3U\n#EXT-X-TARGETDURATION:4\nseg.ts\n",
            ),
            ("/p/bad.pls", "[playlist]\nNumberOfEntries=0\n"),
        ]);
        let e = loader()
            .load("/p/a.m3u8", |p| files.get(p).cloned())
            .unwrap();
        assert_eq!(
            locations(&e),
            ["/p/missing.pls", "http://h/live.m3u8", "/p/bad.pls"]
        );
        assert_eq!(e[2].title.as_deref(), Some("T"));
    }

    #[test]
    fn asx_entryref_is_expanded() {
        let files = mem(&[
            (
                "/p/a.asx",
                "<asx><repeat count=3><entryref href=\"inner\"/></repeat></asx>",
            ),
            ("/p/inner", "[Reference]\nRef1=http://h/x\n"),
        ]);
        let e = loader()
            .load("/p/a.asx", |p| files.get(p).cloned())
            .unwrap();
        assert_eq!(locations(&e), ["http://h/x"]);
        assert_eq!(e[0].repeat, 3);
        assert!(!e[0].nested);
    }

    #[test]
    fn top_level_errors() {
        let files = mem(&[
            ("/p/x.mp3", "ID3"),
            ("/p/hls.m3u8", "#EXT-X-TARGETDURATION:1"),
        ]);
        let l = loader();
        assert_eq!(
            l.load("/p/none.m3u", |p| files.get(p).cloned()),
            Err(Error::Unreadable("/p/none.m3u".into()))
        );
        assert_eq!(
            l.load("/p/x.mp3", |p| files.get(p).cloned()),
            Err(Error::UnknownFormat)
        );
        assert_eq!(
            l.load("/p/hls.m3u8", |p| files.get(p).cloned()),
            Err(Error::Stream)
        );
    }

    #[test]
    fn extensionless_pls_file_on_disk() {
        let t = TempDir::new("sniff");
        let path = t.write(
            "lists/radio",
            b"[playlist]\r\nFile1=a.mp3\r\nTitle1=A\r\nNumberOfEntries=1\r\n",
        );
        let e = read_file(&path).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].location, t.path("lists/a.mp3"));
        assert_eq!(e[0].title.as_deref(), Some("A"));
        let utf16: Vec<u8> = "\u{feff}[playlist]\r\nFile1=b.mp3\r\nNumberOfEntries=1\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let path = t.write("lists/radio16", &utf16);
        assert_eq!(read_file(&path).unwrap()[0].location, t.path("lists/b.mp3"));
        let path = t.write("lists/song", b"ID3\x04\0");
        assert_eq!(read_file(&path), Err(Error::UnknownFormat));
    }

    #[test]
    fn nested_on_disk_with_case_fallback() {
        let t = TempDir::new("nested");
        t.write("Music/Song.MP3", b"x");
        t.write("Lists/Inner.M3U", b"../music/song.mp3\n../top/outer.m3u\n");
        let outer = t.write("top/outer.m3u", b"#EXTM3U\n../LISTS/inner.m3u\n");
        let e = Loader::new(PathStyle::Unix, &StdFs)
            .load(&outer, |p| std::fs::read(p).ok())
            .unwrap();
        assert_eq!(locations(&e), [t.path("Music/Song.MP3")]);
    }
}
