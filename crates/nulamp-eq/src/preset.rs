//! Preset libraries and EQF files (03 §5, §6).
//!
//! The preset library (`winamp.q1`), the auto-load library (`winamp.q2`) and EQF exchange
//! files share one format: a 31-byte signature followed by 268-byte entries.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::value::{BANDS, FLAT, MAX_VALUE};

/// File signature: `Winamp EQ library file v1.1`, then `1A 21 2D 2D`.
pub const SIGNATURE: &[u8; 31] = b"Winamp EQ library file v1.1\x1a!--";

/// Size of one entry in bytes.
pub const ENTRY_LEN: usize = 268;

/// Size of the NUL-padded name field in bytes.
pub const NAME_LEN: usize = 257;

/// File name of the preset library in the config directory.
pub const PRESET_LIBRARY_FILE: &str = "winamp.q1";

/// File name of the auto-load library in the config directory.
pub const AUTOLOAD_LIBRARY_FILE: &str = "winamp.q2";

/// Entry name written by EQF export and preferred by EQF import.
pub const EQF_ENTRY_NAME: &str = "Entry1";

/// Name of the preset loaded by auto-load when no file-name entry exists.
pub const DEFAULT_PRESET_NAME: &str = "Default";

/// Errors from reading or writing preset files.
#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    /// File system error.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// The file does not start with [`SIGNATURE`].
    #[error("not an EQ preset file (bad signature)")]
    BadSignature,
    /// An EQF file without entries.
    #[error("EQ preset file has no entries")]
    Empty,
}

/// One named preset. Values use the 0..63 slider encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    /// Preset name, or a media file name in the auto-load library.
    pub name: String,
    /// Band values, low band first.
    pub bands: [u8; BANDS],
    /// Preamp value.
    pub preamp: u8,
}

impl Preset {
    /// Creates a preset; values above 63 are clamped.
    pub fn new(name: impl Into<String>, bands: [u8; BANDS], preamp: u8) -> Self {
        Self {
            name: name.into(),
            bands: bands.map(|b| b.min(MAX_VALUE)),
            preamp: preamp.min(MAX_VALUE),
        }
    }

    /// True when `name` matches this preset's name, ignoring case.
    pub fn is_named(&self, name: &str) -> bool {
        names_match(&self.name, name)
    }

    /// Decodes one 268-byte entry, clamping values to 0..63.
    pub fn from_entry(entry: &[u8; ENTRY_LEN]) -> Self {
        let field = &entry[..NAME_LEN];
        let end = field.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        let mut bands = [0; BANDS];
        bands.copy_from_slice(&entry[NAME_LEN..NAME_LEN + BANDS]);
        Self::new(decode_name(&field[..end]), bands, entry[ENTRY_LEN - 1])
    }

    /// Encodes this preset as a 268-byte entry.
    pub fn to_entry(&self) -> [u8; ENTRY_LEN] {
        let mut entry = [0u8; ENTRY_LEN];
        let name = encode_name(&self.name);
        entry[..name.len()].copy_from_slice(&name);
        for (dst, &v) in entry[NAME_LEN..].iter_mut().zip(&self.bands) {
            *dst = v.min(MAX_VALUE);
        }
        entry[ENTRY_LEN - 1] = self.preamp.min(MAX_VALUE);
        entry
    }
}

/// Case-insensitive name comparison (Unicode lower case).
pub fn names_match(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
}

/// Finds a preset by name, ignoring case. The first match wins.
pub fn find<'a>(presets: &'a [Preset], name: &str) -> Option<&'a Preset> {
    presets.iter().find(|p| p.is_named(name))
}

/// Decodes a name field: UTF-8 when valid, else Windows-1252 (decision EQ5).
fn decode_name(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

/// Encodes a name: Windows-1252 when every character fits, else UTF-8 (decision EQ5).
///
/// UTF-8 is also used when the Windows-1252 bytes would read back as valid UTF-8, so every
/// name reads back unchanged. The result is cut at a character boundary to at most 256 bytes
/// and contains no NUL.
fn encode_name(name: &str) -> Vec<u8> {
    let name: String = name.chars().filter(|&c| c != '\0').collect();
    let (bytes, _, unmappable) = encoding_rs::WINDOWS_1252.encode(&name);
    let utf8 = unmappable || (!name.is_ascii() && std::str::from_utf8(&bytes).is_ok());
    let mut out = if utf8 {
        name.into_bytes()
    } else {
        bytes.into_owned()
    };
    if out.len() >= NAME_LEN {
        let mut cut = NAME_LEN - 1;
        if utf8 {
            while cut > 0 && (out[cut] & 0xC0) == 0x80 {
                cut -= 1;
            }
        }
        out.truncate(cut);
    }
    out
}

/// Splits file bytes into raw entries. A partial trailing entry is ignored.
fn raw_entries(bytes: &[u8]) -> Result<impl Iterator<Item = &[u8; ENTRY_LEN]>, PresetError> {
    let body = bytes
        .strip_prefix(SIGNATURE.as_slice())
        .ok_or(PresetError::BadSignature)?;
    Ok(body.as_chunks::<ENTRY_LEN>().0.iter())
}

/// Parses a preset file.
pub fn parse_library(bytes: &[u8]) -> Result<Vec<Preset>, PresetError> {
    Ok(raw_entries(bytes)?.map(Preset::from_entry).collect())
}

/// Encodes a complete preset file.
pub fn encode_library(presets: &[Preset]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SIGNATURE.len() + presets.len() * ENTRY_LEN);
    out.extend_from_slice(SIGNATURE);
    for p in presets {
        out.extend_from_slice(&p.to_entry());
    }
    out
}

/// Reads a preset file.
pub fn read_library(path: &Path) -> Result<Vec<Preset>, PresetError> {
    parse_library(&fs::read(path)?)
}

/// Reads a library, returning an empty list when the file is missing or invalid.
pub fn read_library_or_empty(path: &Path) -> Vec<Preset> {
    read_library(path).unwrap_or_default()
}

/// Saves one preset into a library file (03 §6 write rules).
///
/// A missing file, or one with a bad signature, is (re)created with the signature. An entry
/// with the same name (ignoring case) is overwritten in place; otherwise the entry is appended
/// after the last complete entry. Other entries are left byte for byte as they were.
pub fn save_preset(path: &Path, preset: &Preset) -> Result<(), PresetError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let entry = preset.to_entry();
    let offset = match raw_entries(&bytes) {
        Ok(entries) => {
            let mut count = 0;
            let mut found = None;
            for (i, raw) in entries.enumerate() {
                count = i + 1;
                if found.is_none() && Preset::from_entry(raw).is_named(&preset.name) {
                    found = Some(i);
                }
            }
            SIGNATURE.len() + found.unwrap_or(count) * ENTRY_LEN
        }
        Err(_) => {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(SIGNATURE)?;
            SIGNATURE.len()
        }
    };
    file.seek(SeekFrom::Start(offset as u64))?;
    file.write_all(&entry)?;
    file.sync_all()?;
    Ok(())
}

/// Removes every entry named `name` (ignoring case) from a library file.
///
/// Writes the remaining entries to a temporary file, then renames it over the original.
/// Returns false, leaving the file untouched, when no entry matched.
pub fn delete_preset(path: &Path, name: &str) -> Result<bool, PresetError> {
    let bytes = fs::read(path)?;
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(SIGNATURE);
    let mut removed = false;
    for raw in raw_entries(&bytes)? {
        if Preset::from_entry(raw).is_named(name) {
            removed = true;
        } else {
            out.extend_from_slice(raw);
        }
    }
    if !removed {
        return Ok(false);
    }
    let tmp = temp_path(path);
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(&out)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(true)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// The 18 presets written to a new preset library (03 §5.2).
pub fn first_run_presets() -> Vec<Preset> {
    const TABLE: [(&str, [u8; BANDS]); 18] = [
        ("Classical", [31, 31, 31, 31, 31, 31, 44, 44, 44, 48]),
        ("Club", [31, 31, 26, 22, 22, 22, 26, 31, 31, 31]),
        ("Dance", [16, 20, 28, 32, 32, 42, 44, 44, 32, 32]),
        ("Flat", [31, 31, 31, 31, 31, 31, 31, 31, 31, 31]),
        (
            "Laptop speakers/headphones",
            [24, 14, 23, 38, 36, 29, 24, 16, 11, 8],
        ),
        ("Large hall", [15, 15, 22, 22, 31, 40, 40, 40, 31, 31]),
        ("Party", [20, 20, 31, 31, 31, 31, 31, 31, 20, 20]),
        ("Pop", [35, 24, 20, 19, 23, 34, 36, 36, 35, 35]),
        ("Reggae", [31, 31, 33, 42, 31, 21, 21, 31, 31, 31]),
        ("Rock", [19, 24, 41, 45, 38, 25, 17, 14, 14, 14]),
        ("Soft", [24, 29, 34, 36, 34, 25, 18, 16, 14, 12]),
        ("Ska", [36, 40, 39, 33, 25, 22, 17, 16, 14, 16]),
        ("Full Bass", [16, 16, 16, 22, 29, 39, 46, 49, 50, 50]),
        ("Soft Rock", [25, 25, 28, 33, 39, 41, 38, 33, 27, 17]),
        ("Full Treble", [48, 48, 48, 39, 27, 14, 6, 6, 6, 4]),
        (
            "Full Bass & Treble",
            [20, 22, 31, 44, 40, 29, 18, 14, 12, 12],
        ),
        ("Live", [40, 31, 25, 23, 22, 22, 25, 27, 27, 28]),
        ("Techno", [19, 22, 31, 41, 40, 31, 19, 16, 16, 17]),
    ];
    TABLE
        .iter()
        .map(|&(name, bands)| Preset::new(name, bands, FLAT))
        .collect()
}

/// Reads the preset library, creating it with [`first_run_presets`] when it does not exist.
pub fn load_or_create_library(path: &Path) -> Result<Vec<Preset>, PresetError> {
    if path.exists() {
        return read_library(path);
    }
    let presets = first_run_presets();
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, encode_library(&presets))?;
    Ok(presets)
}

/// Encodes an EQF file holding one entry named [`EQF_ENTRY_NAME`].
pub fn eqf_bytes(bands: [u8; BANDS], preamp: u8) -> Vec<u8> {
    encode_library(&[Preset::new(EQF_ENTRY_NAME, bands, preamp)])
}

/// Writes an EQF file, replacing any existing file.
pub fn export_eqf(path: &Path, bands: [u8; BANDS], preamp: u8) -> Result<(), PresetError> {
    fs::write(path, eqf_bytes(bands, preamp))?;
    Ok(())
}

/// Picks the EQF entry to load: `Entry1` (ignoring case) if present, else the first entry.
pub fn parse_eqf(bytes: &[u8]) -> Result<Preset, PresetError> {
    let presets = parse_library(bytes)?;
    let index = presets
        .iter()
        .position(|p| p.is_named(EQF_ENTRY_NAME))
        .unwrap_or(0);
    presets.into_iter().nth(index).ok_or(PresetError::Empty)
}

/// Reads an EQF file with [`parse_eqf`].
pub fn import_eqf(path: &Path) -> Result<Preset, PresetError> {
    parse_eqf(&fs::read(path)?)
}

/// The auto-load key of a track: its location without directories, with extension.
///
/// Splits on both `/` and `\` so that keys from either platform match (decision EQ6).
pub fn track_file_name(location: &str) -> &str {
    location.rsplit(['/', '\\']).next().unwrap_or(location)
}

/// Auto-load lookup (03 §5.4): an auto-load entry named after the file (ignoring case), else
/// `Default` from the preset library, else `None` (keep the current settings).
pub fn auto_load<'a>(
    autoload_library: &'a [Preset],
    preset_library: &'a [Preset],
    file_name: &str,
) -> Option<&'a Preset> {
    find(autoload_library, track_file_name(file_name))
        .or_else(|| find(preset_library, DEFAULT_PRESET_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "nulamp-eq-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample() -> Vec<Preset> {
        vec![
            Preset::new("Rock", [1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 20),
            Preset::new("Café", [63; BANDS], 0),
            Preset::new("日本", [31; BANDS], 40),
            Preset::new("Ã©", [31; BANDS], 31),
        ]
    }

    #[test]
    fn signature_bytes() {
        assert_eq!(&SIGNATURE[..27], b"Winamp EQ library file v1.1");
        assert_eq!(&SIGNATURE[27..], &[0x1A, 0x21, 0x2D, 0x2D]);
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = TempDir::new();
        let path = dir.file("lib.q1");
        for p in sample() {
            save_preset(&path, &p).unwrap();
        }
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), SIGNATURE.len() + 4 * ENTRY_LEN);
        assert_eq!(&bytes[..31], SIGNATURE);
        let presets = parse_library(&bytes).unwrap();
        assert_eq!(presets, sample());
        assert_eq!(encode_library(&presets), bytes);
        // Windows-1252 for names that fit, NUL padded.
        let cafe = &bytes[31 + ENTRY_LEN..31 + 2 * ENTRY_LEN];
        assert_eq!(&cafe[..5], b"Caf\xe9\0");
        assert!(cafe[5..NAME_LEN].iter().all(|&b| b == 0));
        assert_eq!(
            cafe[NAME_LEN..],
            [63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 0]
        );
    }

    #[test]
    fn eqf_round_trip() {
        let dir = TempDir::new();
        let path = dir.file("x.eqf");
        let bands = [0, 5, 10, 20, 31, 40, 50, 60, 63, 7];
        export_eqf(&path, bands, 12).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 31 + ENTRY_LEN);
        assert_eq!(&bytes[31..38], b"Entry1\0");
        let p = import_eqf(&path).unwrap();
        assert_eq!((p.name.as_str(), p.bands, p.preamp), ("Entry1", bands, 12));
        assert_eq!(eqf_bytes(p.bands, p.preamp), bytes);
    }

    #[test]
    fn eqf_without_entry1_loads_first() {
        let bytes = encode_library(&sample());
        assert_eq!(parse_eqf(&bytes).unwrap().name, "Rock");
        let mut with = sample();
        with.push(Preset::new("entry1", [9; BANDS], 9));
        assert_eq!(parse_eqf(&encode_library(&with)).unwrap().bands, [9; BANDS]);
        assert!(matches!(parse_eqf(SIGNATURE), Err(PresetError::Empty)));
    }

    #[test]
    fn rejects_bad_signature_and_ignores_partial_entry() {
        let mut bytes = encode_library(&sample());
        bytes.extend_from_slice(&[1, 2, 3]);
        assert_eq!(parse_library(&bytes).unwrap().len(), 4);
        bytes[0] = b'w';
        assert!(matches!(
            parse_library(&bytes),
            Err(PresetError::BadSignature)
        ));
        assert!(matches!(
            parse_library(&SIGNATURE[..30]),
            Err(PresetError::BadSignature)
        ));
    }

    #[test]
    fn clamps_on_load() {
        let mut entry = Preset::new("X", [0; BANDS], 0).to_entry();
        entry[NAME_LEN] = 200;
        entry[ENTRY_LEN - 1] = 64;
        let p = Preset::from_entry(&entry);
        assert_eq!((p.bands[0], p.preamp), (63, 63));
    }

    #[test]
    fn overwrite_in_place_and_append() {
        let dir = TempDir::new();
        let path = dir.file("lib.q1");
        fs::write(&path, encode_library(&sample())).unwrap();
        save_preset(&path, &Preset::new("ROCK", [50; BANDS], 50)).unwrap();
        let p = read_library(&path).unwrap();
        assert_eq!(p.len(), 4);
        assert_eq!((p[0].name.as_str(), p[0].bands), ("ROCK", [50; BANDS]));
        save_preset(&path, &Preset::new("New", [1; BANDS], 2)).unwrap();
        let p = read_library(&path).unwrap();
        assert_eq!(p.len(), 5);
        assert_eq!(p[4].name, "New");

        // Partial trailing data is replaced by the appended entry.
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&[7; 100]);
        fs::write(&path, &bytes).unwrap();
        save_preset(&path, &Preset::new("Tail", [2; BANDS], 2)).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 31 + 6 * ENTRY_LEN);
        assert_eq!(read_library(&path).unwrap()[5].name, "Tail");
    }

    #[test]
    fn save_replaces_invalid_file() {
        let dir = TempDir::new();
        let path = dir.file("lib.q2");
        fs::write(&path, b"garbage that is not a preset file at all").unwrap();
        save_preset(&path, &Preset::new("song.mp3", [3; BANDS], 4)).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 31 + ENTRY_LEN);
        assert_eq!(read_library(&path).unwrap()[0].name, "song.mp3");
    }

    #[test]
    fn delete_rewrites_without_entry() {
        let dir = TempDir::new();
        let path = dir.file("lib.q1");
        fs::write(&path, encode_library(&sample())).unwrap();
        assert!(delete_preset(&path, "café").unwrap());
        assert!(!delete_preset(&path, "missing").unwrap());
        let p = read_library(&path).unwrap();
        assert_eq!(p.len(), 3);
        assert!(find(&p, "Café").is_none());
        assert!(!temp_path(&path).exists());
    }

    #[test]
    fn first_run_library() {
        let dir = TempDir::new();
        let path = dir.file("cfg").join(PRESET_LIBRARY_FILE);
        let created = load_or_create_library(&path).unwrap();
        assert_eq!(created.len(), 18);
        let read = read_library(&path).unwrap();
        assert_eq!(read, created);
        assert!(read.iter().all(|p| p.preamp == 31));
        assert!(find(&read, DEFAULT_PRESET_NAME).is_none());
        assert_eq!(read[0].name, "Classical");
        assert_eq!(
            find(&read, "laptop speakers/headphones").unwrap().bands,
            [24, 14, 23, 38, 36, 29, 24, 16, 11, 8]
        );
        assert_eq!(
            find(&read, "Full Bass & Treble").unwrap().bands,
            [20, 22, 31, 44, 40, 29, 18, 14, 12, 12]
        );
        assert_eq!(
            find(&read, "techno").unwrap().bands,
            [19, 22, 31, 41, 40, 31, 19, 16, 16, 17]
        );
        save_preset(&path, &Preset::new("Rock", [0; BANDS], 0)).unwrap();
        assert_eq!(load_or_create_library(&path).unwrap()[9].bands, [0; BANDS]);
    }

    #[test]
    fn auto_load_lookup() {
        let q2 = vec![Preset::new("Song.MP3", [1; BANDS], 1)];
        let mut q1 = first_run_presets();
        assert!(auto_load(&q2, &q1, "/music/other.mp3").is_none());
        assert_eq!(
            auto_load(&q2, &q1, "/music/song.mp3").unwrap().bands,
            [1; BANDS]
        );
        assert_eq!(
            auto_load(&q2, &q1, r"C:\Music\SONG.mp3").unwrap().bands,
            [1; BANDS]
        );
        q1.push(Preset::new("default", [5; BANDS], 5));
        assert_eq!(auto_load(&q2, &q1, "other.mp3").unwrap().bands, [5; BANDS]);
    }

    #[test]
    fn long_names_are_truncated() {
        let long = "é".repeat(300);
        let entry = Preset::new(long.clone(), [31; BANDS], 31).to_entry();
        assert_eq!(entry[NAME_LEN - 1], 0);
        let p = Preset::from_entry(&entry);
        assert_eq!(p.name, "é".repeat(256));
        let wide = "日".repeat(100);
        let p = Preset::from_entry(&Preset::new(wide, [31; BANDS], 31).to_entry());
        assert_eq!(p.name, "日".repeat(85));
    }
}
