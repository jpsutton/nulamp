//! PLS (10 §3.3, §3.4).

use crate::entry::{PlaylistEntry, non_empty};
use crate::ini;
use crate::location::Resolver;
use crate::text::{decode_auto, one_line};
use crate::{Error, WriteOptions};

/// Parses a PLS file. Returns [`Error::NotAPlaylist`] when there is no `[playlist]`
/// section or `NumberOfEntries` is missing, unreadable or 0.
///
/// Only indices `1..=NumberOfEntries` are read; missing or empty `File<n>` keys are
/// skipped. With `Version=0`, `Title<n>` and `Length<n>` are ignored. An unreadable
/// `Version` counts as 1.
pub fn parse(bytes: &[u8], resolver: &Resolver) -> Result<Vec<PlaylistEntry>, Error> {
    let text = decode_auto(bytes);
    let sec = ini::section(&text, "playlist").ok_or(Error::NotAPlaylist)?;
    let count: u64 = sec
        .get("numberofentries")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if count == 0 {
        return Err(Error::NotAPlaylist);
    }
    let version: i64 = sec.get("version").and_then(|v| v.parse().ok()).unwrap_or(1);
    let mut indices: Vec<u64> = sec
        .keys()
        .filter_map(|k| k.strip_prefix("file")?.parse().ok())
        .filter(|n| (1..=count).contains(n))
        .collect();
    indices.sort_unstable();
    let num = |key: &str, n: u64| {
        sec.get(&format!("{key}{n}"))
            .and_then(|v| v.parse::<i64>().ok())
    };
    let mut entries = Vec::new();
    for n in indices {
        let Some(location) = sec
            .get(&format!("file{n}"))
            .and_then(|f| resolver.resolve(f))
        else {
            continue;
        };
        let mut e = PlaylistEntry::new(location);
        if version != 0
            && let Some(title) = sec.get(&format!("title{n}")).and_then(|t| non_empty(t))
        {
            e.title = Some(title);
            e.length_s = num("length", n).and_then(|l| u64::try_from(l).ok());
        }
        e.repeat = num("repeat", n)
            .and_then(|r| u32::try_from(r).ok())
            .unwrap_or(0);
        e.start_ms = num("start", n).and_then(|v| u64::try_from(v).ok());
        e.end_ms = num("end", n).and_then(|v| u64::try_from(v).ok());
        entries.push(e);
    }
    Ok(entries)
}

/// Writes a PLS file: UTF-8, CRLF, `NumberOfEntries` and `Version=2` at the end.
///
/// `Title<n>`/`Length<n>` only for entries with a title; `Repeat<n>` only when greater
/// than 0; `Start<n>`/`End<n>` only when set.
pub fn write(entries: &[PlaylistEntry], opts: &WriteOptions) -> Vec<u8> {
    let mut s = String::from("[playlist]\r\n");
    for (i, e) in entries.iter().enumerate() {
        let n = i + 1;
        s.push_str(&format!(
            "File{n}={}\r\n",
            one_line(&opts.location(&e.location))
        ));
        if let Some(title) = &e.title {
            let length = e
                .length_s
                .map_or(-1, |l| i64::try_from(l).unwrap_or(i64::MAX));
            s.push_str(&format!(
                "Title{n}={}\r\nLength{n}={length}\r\n",
                one_line(title)
            ));
        }
        if e.repeat > 0 {
            s.push_str(&format!("Repeat{n}={}\r\n", e.repeat));
        }
        if let Some(v) = e.start_ms {
            s.push_str(&format!("Start{n}={v}\r\n"));
        }
        if let Some(v) = e.end_ms {
            s.push_str(&format!("End{n}={v}\r\n"));
        }
    }
    s.push_str(&format!(
        "NumberOfEntries={}\r\nVersion=2\r\n",
        entries.len()
    ));
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{NullFs, PathStyle};

    fn read(text: &str) -> Result<Vec<PlaylistEntry>, Error> {
        parse(
            text.as_bytes(),
            &Resolver::new(Some("/m/l.pls"), PathStyle::Unix, &NullFs),
        )
    }

    fn opts() -> WriteOptions {
        WriteOptions {
            style: PathStyle::Unix,
            ..Default::default()
        }
    }

    #[test]
    fn gaps_are_allowed() {
        let e = read("[playlist]\nFile1=a.mp3\nFile3=c.mp3\nNumberOfEntries=3\n").unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].location, "/m/a.mp3");
        assert_eq!(e[1].location, "/m/c.mp3");
    }

    #[test]
    fn zero_entries_is_not_a_playlist() {
        assert_eq!(
            read("[playlist]\nFile1=a.mp3\nNumberOfEntries=0\n"),
            Err(Error::NotAPlaylist)
        );
        assert_eq!(read("[playlist]\nFile1=a.mp3\n"), Err(Error::NotAPlaylist));
        assert_eq!(
            read("File1=a.mp3\nNumberOfEntries=1\n"),
            Err(Error::NotAPlaylist)
        );
    }

    #[test]
    fn written_layout() {
        let mut a = PlaylistEntry::with_title("/m/a.mp3", "A", Some(10));
        a.repeat = 2;
        let entries = [
            a,
            PlaylistEntry::new("http://h/s"),
            PlaylistEntry::with_title("/b.mp3", "B", None),
        ];
        let out = String::from_utf8(write(&entries, &opts())).unwrap();
        assert_eq!(
            out,
            "[playlist]\r\nFile1=/m/a.mp3\r\nTitle1=A\r\nLength1=10\r\nRepeat1=2\r\nFile2=http://h/s\r\n\
             File3=/b.mp3\r\nTitle3=B\r\nLength3=-1\r\nNumberOfEntries=3\r\nVersion=2\r\n"
        );
        assert!(out.ends_with("NumberOfEntries=3\r\nVersion=2\r\n"));
        assert!(!out.replace("\r\n", "").contains(['\r', '\n']));
    }

    #[test]
    fn round_trip() {
        let mut a = PlaylistEntry::with_title("/m/a b.mp3", "Ä, title = x", Some(10));
        a.repeat = 1;
        a.start_ms = Some(1500);
        a.end_ms = Some(9000);
        let entries = vec![
            a,
            PlaylistEntry::new("/m/sub/c.ogg"),
            PlaylistEntry::with_title("http://h/s", "R", None),
        ];
        let o = WriteOptions {
            relative_to: Some("/m".into()),
            ..opts()
        };
        let out = write(&entries, &o);
        assert!(String::from_utf8_lossy(&out).contains("File2=sub/c.ogg\r\n"));
        assert_eq!(
            parse(
                &out,
                &Resolver::new(Some("/m/l.pls"), PathStyle::Unix, &NullFs)
            )
            .unwrap(),
            entries
        );
    }

    #[test]
    fn version_title_and_length_rules() {
        let t = "[Playlist]\nfile1=a\ntitle1=T\nlength1=5\nFile2=b\nTitle2=\nLength2=7\nFile3=c\nLength3=-1\nTitle3=X\nNumberOfEntries=3\n";
        let e = read(t).unwrap();
        assert_eq!((e[0].title.as_deref(), e[0].length_s), (Some("T"), Some(5)));
        assert_eq!((e[1].title.as_deref(), e[1].length_s), (None, None));
        assert_eq!((e[2].title.as_deref(), e[2].length_s), (Some("X"), None));
        let e = read(&format!("{t}Version=0\n")).unwrap();
        assert!(e.iter().all(|e| e.title.is_none() && e.length_s.is_none()));
    }

    #[test]
    fn ini_rules_and_encoding() {
        let e = read(
            "; c\n[playlist]\n  FILE1 = \"x y.mp3\" \n;File2=z\nFile2=\nNumberOfEntries = 2\n",
        )
        .unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].location, "/m/x y.mp3");
        let r = Resolver::new(Some("/m/l.pls"), PathStyle::Unix, &NullFs);
        let e = parse(b"[playlist]\nFile1=caf\xE9\nNumberOfEntries=1", &r).unwrap();
        assert_eq!(e[0].location, "/m/café");
        let utf16: Vec<u8> = "[playlist]\r\nFile1=é\r\nNumberOfEntries=1\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(parse(&utf16, &r).unwrap()[0].location, "/m/é");
    }

    #[test]
    fn huge_entry_count_is_cheap() {
        let e =
            read("[playlist]\nFile1=a\nFile5000000000=b\nNumberOfEntries=4294967295\n").unwrap();
        assert_eq!(e.len(), 1);
    }
}
