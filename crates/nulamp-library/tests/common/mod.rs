//! Test fixtures: tagged WAV files written at test time.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use nulamp_library::{LocalLibrary, ScanMode, ScanSummary};
use tempfile::TempDir;

/// Writes a silent 8 kHz mono 16-bit WAV file of `ms` milliseconds.
pub fn write_wav(path: &Path, ms: u32) {
    let rate = 8000u32;
    let data_len = rate * ms / 1000 * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    b.resize(b.len() + data_len as usize, 0);
    fs::write(path, b).unwrap();
}

/// Writes `root/rel` as a WAV file with the given tags.
pub fn tagged(root: &Path, rel: &str, ms: u32, tags: &[(&str, &str)]) -> PathBuf {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    write_wav(&p, ms);
    if !tags.is_empty() {
        retag(&p, tags);
    }
    p
}

/// Changes tags of an existing file and moves its modification time forward.
pub fn retag(p: &Path, tags: &[(&str, &str)]) {
    let changes: Vec<(String, Option<String>)> = tags
        .iter()
        .map(|(k, v)| (k.to_string(), Some(v.to_string())))
        .collect();
    nulamp_meta::write(p, &changes).unwrap();
}

/// Sets the modification time `secs` seconds into the future, so change detection sees it.
pub fn touch_later(p: &Path, secs: u64) {
    let f = fs::File::options().write(true).open(p).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(secs))
        .unwrap();
}

pub fn audio(ext: &str) -> bool {
    matches!(ext, "wav" | "mp3" | "flac")
}

/// A temporary directory with `music/` and the database under `db/`.
pub struct Fixture {
    pub dir: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nulamp-library-")
            .tempdir()
            .unwrap();
        fs::create_dir_all(dir.path().join("music")).unwrap();
        Self { dir }
    }

    pub fn music(&self) -> PathBuf {
        self.dir.path().join("music")
    }

    pub fn db(&self) -> PathBuf {
        self.dir.path().join("db").join("library.sqlite")
    }

    pub fn open(&self) -> LocalLibrary {
        LocalLibrary::open(self.db(), audio).unwrap()
    }

    /// The standard catalogue (7 audio files, one text file).
    pub fn populate(&self) {
        let m = self.music();
        let beatles = |title, album, year, track| {
            [
                ("title", title),
                ("artist", "The Beatles"),
                ("album", album),
                ("year", year),
                ("genre", "Rock"),
                ("track", track),
            ]
        };
        tagged(
            &m,
            "Beatles/Abbey Road/01.wav",
            1000,
            &beatles("Come Together", "Abbey Road", "1969", "1"),
        );
        tagged(
            &m,
            "Beatles/Abbey Road/02.wav",
            3000,
            &beatles("Something", "Abbey Road", "1969", "2"),
        );
        tagged(
            &m,
            "Beatles/Abbey Road/10.wav",
            500,
            &beatles("Her Majesty", "Abbey Road", "1969", "10"),
        );
        tagged(
            &m,
            "Beatles/Hey Jude.wav",
            2000,
            &beatles("Hey Jude", "Hey Jude", "1968", "1"),
        );
        tagged(
            &m,
            "Bjork/Homogenic/hunter.wav",
            1500,
            &[
                ("title", "Hunter"),
                ("artist", "Björk"),
                ("album", "Homogenic"),
                ("year", "1997-09-22"),
                ("genre", "Electronic"),
                ("track", "1/10"),
                ("comment", "Iceland"),
            ],
        );
        tagged(
            &m,
            "Various/comp.wav",
            2500,
            &[
                ("title", "Track X"),
                ("artist", "Someone"),
                ("albumartist", "Various Artists"),
                ("album", "Hits 2005"),
                ("year", "2005"),
                ("genre", "Pop"),
            ],
        );
        tagged(&m, "untagged.wav", 700, &[]);
        fs::write(m.join("notes.txt"), "not audio").unwrap();
    }

    /// Opens the library with `music/` as a recursive watch folder, scanned once.
    pub fn scanned(&self) -> LocalLibrary {
        self.populate();
        let lib = self.open();
        lib.add_folder(self.music(), true).unwrap();
        scan(&lib, ScanMode::Incremental);
        lib
    }
}

pub fn scan(lib: &LocalLibrary, mode: ScanMode) -> ScanSummary {
    let s = lib.scan(mode).unwrap().wait().unwrap();
    assert_eq!(s.error, None);
    s
}
