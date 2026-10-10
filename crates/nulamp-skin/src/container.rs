//! Skin containers: archives and directories (01 §1).

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::ini::decode_text;

#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("modern (XML) skins are not supported")]
    ModernSkin,
    #[error("cannot read skin: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a valid skin archive: {0}")]
    Archive(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    Archive,
    Directory,
    BuiltIn,
}

/// The files of one skin, keyed by lower-case final file name.
#[derive(Debug, Clone)]
pub struct SkinFiles {
    kind: ContainerKind,
    /// (lower-case name, bytes), in archive order (directories: name order).
    files: Vec<(String, Vec<u8>)>,
}

/// Largest file read from a directory skin.
const MAX_FILE: u64 = 16 * 1024 * 1024;

fn final_component(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn ext_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, e)| e)
}

impl SkinFiles {
    /// Builds a file set from (name, bytes) pairs. Names are flattened and case-folded;
    /// a later duplicate replaces an earlier one.
    pub fn from_entries(
        kind: ContainerKind,
        entries: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Self {
        let mut s = Self {
            kind,
            files: Vec::new(),
        };
        for (name, data) in entries {
            s.insert(final_component(&name).to_lowercase(), data);
        }
        s
    }

    fn insert(&mut self, name: String, data: Vec<u8>) {
        self.files.retain(|(n, _)| *n != name);
        self.files.push((name, data));
    }

    /// Opens a `.wsz`/`.zip` archive or a skin directory.
    pub fn open(path: &Path) -> Result<Self, ContainerError> {
        if path.is_dir() {
            return Self::open_dir(path);
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext == "wal" {
            return Err(ContainerError::ModernSkin);
        }
        Self::from_archive(&std::fs::read(path)?)
    }

    /// Reads a classic skin archive from memory.
    pub fn from_archive(data: &[u8]) -> Result<Self, ContainerError> {
        let mut zip = zip::ZipArchive::new(Cursor::new(data))
            .map_err(|e| ContainerError::Archive(e.to_string()))?;
        let mut s = Self {
            kind: ContainerKind::Archive,
            files: Vec::new(),
        };
        for i in 0..zip.len() {
            let mut f = match zip.by_index(i) {
                Ok(f) => f,
                Err(_) => continue,
            };
            if f.is_dir() {
                continue;
            }
            let full = match f.name() {
                Ok(n) => n.replace('\\', "/"),
                Err(_) => String::from_utf8_lossy(f.name_raw()).replace('\\', "/"),
            };
            let lower = final_component(&full).to_lowercase();
            // `skin.xml` at the root or inside one top-level folder marks a modern skin.
            if lower == "skin.xml" && full.trim_start_matches('/').matches('/').count() <= 1 {
                return Err(ContainerError::ModernSkin);
            }
            if !matches!(ext_of(&lower), Some("bmp" | "txt" | "cur" | "ini")) {
                continue;
            }
            if f.size() > MAX_FILE {
                continue;
            }
            let mut data = Vec::with_capacity(f.size() as usize);
            if f.read_to_end(&mut data).is_err() {
                continue;
            }
            s.insert(lower, data);
        }
        Ok(s)
    }

    fn open_dir(dir: &Path) -> Result<Self, ContainerError> {
        let mut names: Vec<(String, PathBuf)> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter(|e| e.metadata().map(|m| m.len() <= MAX_FILE).unwrap_or(false))
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
        names.sort();
        let mut s = Self {
            kind: ContainerKind::Directory,
            files: Vec::new(),
        };
        for (name, path) in names {
            if let Ok(data) = std::fs::read(&path) {
                s.insert(name.to_lowercase(), data);
            }
        }
        Ok(s)
    }

    pub fn kind(&self) -> ContainerKind {
        self.kind
    }

    /// File contents by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let name = name.to_lowercase();
        self.files
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, d)| d.as_slice())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|(n, _)| n.as_str())
    }

    /// Skin description text (01 §1.5), truncated to 32 KiB.
    pub fn readme(&self) -> Option<String> {
        let mut candidates = vec!["readme.txt", "read me.txt"];
        if self.kind == ContainerKind::Directory {
            candidates.push("file_id.diz");
        }
        let data = candidates.iter().find_map(|n| self.get(n)).or_else(|| {
            self.files
                .iter()
                .find(|(n, _)| {
                    n.ends_with(".txt")
                        && !matches!(n.as_str(), "pledit.txt" | "viscolor.txt" | "region.txt")
                })
                .map(|(_, d)| d.as_slice())
        })?;
        let mut text = decode_text(data);
        if text.len() > 32 * 1024 {
            let mut cut = 32 * 1024;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        Some(text)
    }
}

/// One entry of the skin browser list (01 §1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinEntry {
    /// Display name: file or directory name without extension.
    pub name: String,
    /// `None` for the built-in default skin.
    pub path: Option<PathBuf>,
}

/// Lists the built-in skin, then sub-directories and `.wsz`/`.zip` files of `dir`, by name.
pub fn list_skins(dir: &Path) -> Vec<SkinEntry> {
    let mut out = vec![SkinEntry {
        name: String::new(),
        path: None,
    }];
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut found: Vec<SkinEntry> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                return Some(SkinEntry {
                    name,
                    path: Some(path),
                });
            }
            let (stem, ext) = name.rsplit_once('.')?;
            matches!(ext.to_lowercase().as_str(), "wsz" | "zip").then(|| SkinEntry {
                name: stem.to_string(),
                path: Some(path.clone()),
            })
        })
        .collect();
    found.sort_by_key(|e| e.name.to_lowercase());
    out.extend(found);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in entries {
                z.start_file(*name, opts).unwrap();
                z.write_all(data).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn upper_case_in_subfolder() {
        let z = make_zip(&[("SKIN/MAIN.BMP", b"x"), ("SKIN/Foo.png", b"p")]);
        let s = SkinFiles::from_archive(&z).unwrap();
        assert_eq!(s.get("main.bmp"), Some(&b"x"[..]));
        assert!(s.get("foo.png").is_none());
    }

    #[test]
    fn later_duplicate_wins() {
        let z = make_zip(&[("a/main.bmp", b"1"), ("b/MAIN.bmp", b"2")]);
        let s = SkinFiles::from_archive(&z).unwrap();
        assert_eq!(s.get("Main.Bmp"), Some(&b"2"[..]));
    }

    #[test]
    fn modern_skin_rejected() {
        let z = make_zip(&[("skin.xml", b"<x/>")]);
        assert!(matches!(
            SkinFiles::from_archive(&z),
            Err(ContainerError::ModernSkin)
        ));
        let z = make_zip(&[("MySkin/skin.xml", b"<x/>")]);
        assert!(matches!(
            SkinFiles::from_archive(&z),
            Err(ContainerError::ModernSkin)
        ));
        let z = make_zip(&[("a/b/skin.xml", b"<x/>")]);
        assert!(SkinFiles::from_archive(&z).is_ok());
    }

    #[test]
    fn readme_order() {
        let s = SkinFiles::from_entries(
            ContainerKind::Archive,
            vec![
                ("pledit.txt".into(), b"p".to_vec()),
                ("notes.txt".into(), b"n".to_vec()),
            ],
        );
        assert_eq!(s.readme().as_deref(), Some("n"));
        let s = SkinFiles::from_entries(
            ContainerKind::Archive,
            vec![
                ("notes.txt".into(), b"n".to_vec()),
                ("Read Me.txt".into(), b"r".to_vec()),
            ],
        );
        assert_eq!(s.readme().as_deref(), Some("r"));
    }

    #[test]
    fn directory_skin_and_listing() {
        let tmp = std::env::temp_dir().join(format!("nulamp-skin-test-{}", std::process::id()));
        let skin_dir = tmp.join("Cool");
        std::fs::create_dir_all(&skin_dir).unwrap();
        std::fs::write(skin_dir.join("MAIN.BMP"), b"m").unwrap();
        std::fs::write(skin_dir.join("FILE_ID.DIZ"), b"diz").unwrap();
        std::fs::write(tmp.join("b.WSZ"), make_zip(&[])).unwrap();
        std::fs::write(tmp.join("c.txt"), b"").unwrap();
        let s = SkinFiles::open(&skin_dir).unwrap();
        assert_eq!(s.get("main.bmp"), Some(&b"m"[..]));
        assert_eq!(s.readme().as_deref(), Some("diz"));
        let names: Vec<_> = list_skins(&tmp).into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["", "b", "Cool"]);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
