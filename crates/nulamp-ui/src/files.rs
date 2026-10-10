//! Adding files, folders, URLs and playlists (10 §2), and saving playlists (10 §3).

use std::path::Path;

use nulamp_core::{NewEntry, Playlist};
use nulamp_playlist::{Format, PlaylistEntry, WriteOptions};

/// Converts a playlist-file entry to a new playlist entry.
fn from_playlist_entry(e: PlaylistEntry) -> NewEntry {
    NewEntry {
        location: e.location.trim().to_string(),
        title: e.title,
        length_ms: e.length_s.map(|s| s * 1000),
        repeat: e.repeat,
        start_ms: e.start_ms,
        end_ms: e.end_ms,
        extras: e.extras,
    }
}

/// Whether a location names a playlist by extension (ignoring URL queries).
pub fn is_playlist_location(location: &str) -> bool {
    Format::from_path(location).is_some()
}

/// Reads a playlist file into entries (nested playlists expanded).
pub fn load_playlist(path: &str) -> Result<Vec<NewEntry>, nulamp_playlist::Error> {
    Ok(nulamp_playlist::read_file(path)?
        .into_iter()
        .map(from_playlist_entry)
        .collect())
}

/// Expands one dropped or opened item (the "add anything" rule, 10 §2.5).
pub fn add_anything(
    item: &str,
    recurse: bool,
    sort: bool,
    supported: &dyn Fn(&str) -> bool,
) -> Vec<NewEntry> {
    let item = item.trim();
    if item.contains("://") && !item.starts_with("file://") {
        // URL: playlists by extension are downloaded later by the stream input; add as is.
        return vec![NewEntry::new(item)];
    }
    let local = item
        .strip_prefix("file://")
        .map(percent_decode)
        .unwrap_or_else(|| item.to_string());
    let path = Path::new(&local);
    if path.is_dir() {
        let mut files = Vec::new();
        walk(path, recurse, supported, &mut files);
        if sort {
            files.sort_by(|a, b| nulamp_core::playlist::natural_cmp(a, b));
        } else {
            files.sort();
        }
        return files.into_iter().map(NewEntry::new).collect();
    }
    let abs = std::fs::canonicalize(path)
        .map(|p| strip_verbatim(&p.to_string_lossy()))
        .unwrap_or(local.clone());
    if path.is_file() {
        let is_list =
            is_playlist_location(&abs) || (path.extension().is_none() && sniff_playlist(path));
        if is_list && let Ok(entries) = load_playlist(&abs) {
            return entries;
        }
    }
    // Existing media files and paths that do not exist are added as given (they fail at play).
    vec![NewEntry::new(abs)]
}

/// Windows `canonicalize` gives verbatim paths (`\\?\C:\…`, `\\?\UNC\server\…`), which
/// users and other programs do not expect in playlists: back to the ordinary form.
fn strip_verbatim(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match p.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => p.to_string(),
    }
}

fn sniff_playlist(path: &Path) -> bool {
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .unwrap_or(0);
    Format::sniff(&head[..n]).is_some()
}

fn walk(dir: &Path, recurse: bool, supported: &dyn Fn(&str) -> bool, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if recurse {
                walk(&p, recurse, supported, out);
            }
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str())
            && supported(ext)
            && !is_playlist_location(&p.to_string_lossy())
        {
            out.push(p.to_string_lossy().into_owned());
        }
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Writes the playlist to `path`; the format comes from the extension (default M3U8).
pub fn save_playlist(p: &Playlist, path: &str, relative: bool) -> Result<(), String> {
    let format = Format::from_path(path).unwrap_or(Format::M3u8);
    let entries: Vec<PlaylistEntry> = p
        .entries()
        .iter()
        .map(|e| {
            let mut x = PlaylistEntry::new(e.location.clone());
            if e.metadata_read || e.length_ms.is_some() {
                x.title = Some(e.title.clone());
                x.length_s = e.length_ms.map(|ms| ms / 1000);
            }
            x.repeat = e.repeat;
            x.start_ms = e.start_ms;
            x.end_ms = e.end_ms;
            x.extras = e.extras.clone();
            x
        })
        .collect();
    let dir = Path::new(path)
        .parent()
        .map(|d| d.to_string_lossy().into_owned());
    let opts = WriteOptions {
        relative_to: if relative { dir } else { None },
        ..WriteOptions::default()
    };
    let bytes = nulamp_playlist::write(&entries, format, &opts).map_err(|e| e.to_string())?;
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_paths_are_simplified() {
        assert_eq!(strip_verbatim(r"\\?\C:\Music\a.mp3"), r"C:\Music\a.mp3");
        assert_eq!(
            strip_verbatim(r"\\?\UNC\nas\share\a.mp3"),
            r"\\nas\share\a.mp3"
        );
        assert_eq!(
            strip_verbatim(r"\\?\Volume{x}\a.mp3"),
            r"\\?\Volume{x}\a.mp3"
        );
        assert_eq!(strip_verbatim("/music/a.mp3"), "/music/a.mp3");
    }

    #[test]
    fn folders_playlists_and_files() {
        let dir = std::env::temp_dir().join(format!("nulamp-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["b.mp3", "a.flac", "notes.txt", "sub/c.ogg", "list.m3u"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        std::fs::write(dir.join("list.m3u"), "#EXTM3U\n#EXTINF:5,Five\nb.mp3\n").unwrap();
        let supported =
            |e: &str| matches!(e.to_lowercase().as_str(), "mp3" | "flac" | "ogg" | "m3u");
        let d = dir.to_string_lossy().into_owned();
        let all = add_anything(&d, true, true, &supported);
        let names: Vec<_> = all
            .iter()
            .map(|e| {
                let name = Path::new(&e.location).file_name().unwrap();
                name.to_string_lossy().into_owned()
            })
            .collect();
        assert_eq!(names, ["a.flac", "b.mp3", "c.ogg"]);
        assert_eq!(add_anything(&d, false, true, &supported).len(), 2);
        let list = add_anything(&format!("{d}/list.m3u"), true, false, &supported);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title.as_deref(), Some("Five"));
        assert_eq!(list[0].length_ms, Some(5000));
        assert_eq!(
            add_anything("http://radio/x", true, false, &supported)[0].location,
            "http://radio/x"
        );

        let mut p = Playlist::new();
        p.append(list);
        let out = format!("{d}/out.pls");
        save_playlist(&p, &out, true).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("File1=b.mp3"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
