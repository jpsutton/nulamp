//! Tag positions in files with tags at both ends, and safe file replacement.
//!
//! Layout (spec 09 §2): `[ID3v2][audio][APEv2][Lyrics3v2][ID3v1]`. A changed ID3v2 tag that
//! fits in the old tag and its padding is written in place, as are tags at the end when the
//! front is unchanged or fits. Otherwise the file is copied to a temporary file next to it,
//! which then replaces the original with a rename.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::id3v1::TAG_LEN;

/// Padding added after an ID3v2 tag when the file has to be rewritten.
pub(crate) const ID3V2_PADDING: usize = 1024;

/// Byte ranges of the tags in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Size of the leading ID3v2 tag including padding and footer; 0 when there is none.
    pub front_len: u64,
    /// End of the audio data: start of the first tag at the end, or the file length.
    pub audio_end: u64,
    pub ape: Option<Range<u64>>,
    pub lyrics3: Option<Range<u64>>,
    pub id3v1: Option<Range<u64>>,
    pub len: u64,
}

impl Layout {
    /// Finds the tags in `file`.
    pub(crate) fn scan<R: Read + Seek>(file: &mut R) -> io::Result<Self> {
        let len = file.seek(SeekFrom::End(0))?;
        let front_len = front_id3v2_len(file, len)?;
        let mut end = len;

        let id3v1 = (end >= front_len + TAG_LEN as u64 && read_at(file, end - 128, 3)? == b"TAG")
            .then(|| end - TAG_LEN as u64..end);
        if let Some(r) = &id3v1 {
            end = r.start;
        }

        let mut lyrics3 = None;
        if end >= front_len + 15 {
            let trailer = read_at(file, end - 15, 15)?;
            if &trailer[6..] == b"LYRICS200" && trailer[..6].iter().all(u8::is_ascii_digit) {
                let size: u64 = std::str::from_utf8(&trailer[..6])
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);
                if let Some(start) = (end - 15).checked_sub(size).filter(|s| *s >= front_len)
                    && read_at(file, start, 11)? == b"LYRICSBEGIN"
                {
                    lyrics3 = Some(start..end);
                    end = start;
                }
            }
        }

        let mut ape = None;
        if end >= front_len + 32 {
            let footer = read_at(file, end - 32, 32)?;
            if &footer[..8] == b"APETAGEX" {
                let size = u64::from(u32::from_le_bytes([
                    footer[12], footer[13], footer[14], footer[15],
                ]));
                let flags = u32::from_le_bytes([footer[20], footer[21], footer[22], footer[23]]);
                let header = if flags & (1 << 31) != 0 { 32 } else { 0 };
                if let Some(start) = end.checked_sub(size + header).filter(|s| *s >= front_len) {
                    ape = Some(start..end);
                    end = start;
                }
            }
        }

        Ok(Self {
            front_len,
            audio_end: end,
            ape,
            lyrics3,
            id3v1,
            len,
        })
    }
}

/// Size of the ID3v2 tag at the start of the file, or 0.
fn front_id3v2_len<R: Read + Seek>(file: &mut R, len: u64) -> io::Result<u64> {
    if len < 10 {
        return Ok(0);
    }
    let h = read_at(file, 0, 10)?;
    if &h[..3] != b"ID3" || h[3] == 0xFF || h[4] == 0xFF || h[6..].iter().any(|b| b & 0x80 != 0) {
        return Ok(0);
    }
    let size = h[6..]
        .iter()
        .fold(0u64, |acc, b| (acc << 7) | u64::from(*b));
    let footer = if h[3] == 4 && h[5] & 0x10 != 0 { 10 } else { 0 };
    Ok((10 + size + footer).min(len))
}

fn read_at<R: Read + Seek>(file: &mut R, pos: u64, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0; n];
    file.seek(SeekFrom::Start(pos))?;
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// New content for one tag position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Part {
    /// Leave the existing bytes (or absence) as they are.
    Keep,
    /// Replace with these bytes; empty removes the tag.
    Replace(Vec<u8>),
}

/// Changes to a file with tags at both ends. `front` is an ID3v2 tag without padding.
#[derive(Debug, Clone)]
pub(crate) struct EndTagsEdit {
    pub front: Part,
    pub ape: Part,
    pub id3v1: Part,
}

/// Writes `edit` to the file at `path` whose tags are at `layout`.
///
/// Returns true when the file was changed in place, false when it was rewritten.
pub(crate) fn write_end_tags(path: &Path, layout: &Layout, edit: &EndTagsEdit) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let old_tail = read_at(
        &mut file,
        layout.audio_end,
        (layout.len - layout.audio_end) as usize,
    )?;
    let tail_part = |part: &Part, range: &Option<Range<u64>>| match part {
        Part::Replace(bytes) => bytes.clone(),
        Part::Keep => range
            .as_ref()
            .map(|r| {
                old_tail[(r.start - layout.audio_end) as usize..(r.end - layout.audio_end) as usize]
                    .to_vec()
            })
            .unwrap_or_default(),
    };
    let mut new_tail = tail_part(&edit.ape, &layout.ape);
    new_tail.extend(tail_part(&Part::Keep, &layout.lyrics3));
    new_tail.extend(tail_part(&edit.id3v1, &layout.id3v1));

    let front = match &edit.front {
        Part::Keep => Some(None),
        Part::Replace(tag) if tag.is_empty() && layout.front_len == 0 => Some(None),
        Part::Replace(tag) if !tag.is_empty() && tag.len() as u64 <= layout.front_len => {
            Some(Some(pad_id3v2(tag, layout.front_len as usize)))
        }
        Part::Replace(_) => None,
    };

    if let Some(front) = front {
        drop(file);
        let mut file = OpenOptions::new().write(true).open(path)?;
        if let Some(front) = front {
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&front)?;
        }
        if new_tail != old_tail {
            if new_tail.len() != old_tail.len() {
                file.set_len(layout.audio_end)?;
            }
            file.seek(SeekFrom::Start(layout.audio_end))?;
            file.write_all(&new_tail)?;
        }
        file.sync_all()?;
        return Ok(true);
    }

    let Part::Replace(tag) = &edit.front else {
        unreachable!("kept front is always written in place")
    };
    let new_front = if tag.is_empty() {
        Vec::new()
    } else {
        pad_id3v2(tag, tag.len() + ID3V2_PADDING)
    };
    replace_file(path, |tmp| {
        tmp.write_all(&new_front)?;
        file.seek(SeekFrom::Start(layout.front_len))?;
        io::copy(
            &mut (&mut file).take(layout.audio_end - layout.front_len),
            tmp,
        )?;
        tmp.write_all(&new_tail)
    })?;
    Ok(false)
}

/// `tag` (an ID3v2 tag without padding or footer) padded with zeros to `total` bytes, with
/// the size in its header updated.
fn pad_id3v2(tag: &[u8], total: usize) -> Vec<u8> {
    let mut out = tag.to_vec();
    out.resize(total, 0);
    let size = (total - 10) as u32;
    for (i, shift) in [21, 14, 7, 0].into_iter().enumerate() {
        out[6 + i] = ((size >> shift) & 0x7F) as u8;
    }
    out
}

/// Builds a replacement for `path` in a temporary file next to it, then renames it over
/// `path`. The temporary file gets the original's permissions and is removed on failure.
pub(crate) fn replace_file(
    path: &Path,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let tmp = temp_path(path);
    let result = (|| {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        fill(&mut file)?;
        file.sync_all()?;
        fs::set_permissions(&tmp, fs::metadata(path)?.permissions())?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Copies `path` to a temporary file, lets `edit` change the copy, then renames the copy
/// over `path`.
pub(crate) fn edit_copy<E: From<io::Error>>(
    path: &Path,
    edit: impl FnOnce(&mut File) -> Result<(), E>,
) -> Result<(), E> {
    let mut outcome = Ok(());
    let io_result = replace_file(path, |tmp| {
        io::copy(&mut File::open(path)?, tmp)?;
        tmp.seek(SeekFrom::Start(0))?;
        outcome = edit(tmp);
        match &outcome {
            Ok(()) => Ok(()),
            Err(_) => Err(io::Error::other("tag write failed")),
        }
    });
    outcome?;
    io_result.map_err(E::from)
}

fn temp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.nulamp-{}-{n}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn id3v2(size: u32) -> Vec<u8> {
        let mut v = b"ID3\x03\x00\x00".to_vec();
        v.extend([
            (size >> 21) as u8 & 0x7F,
            (size >> 14) as u8 & 0x7F,
            (size >> 7) as u8 & 0x7F,
            size as u8 & 0x7F,
        ]);
        v.resize(10 + size as usize, 0);
        v
    }

    fn ape(items: &[u8]) -> Vec<u8> {
        let mut footer = b"APETAGEX".to_vec();
        footer.extend(2000u32.to_le_bytes());
        footer.extend((items.len() as u32 + 32).to_le_bytes());
        footer.extend(0u32.to_le_bytes());
        let mut header = footer.clone();
        header.extend((0xA000_0000u32).to_le_bytes());
        header.extend([0; 8]);
        footer.extend((0x8000_0000u32).to_le_bytes());
        footer.extend([0; 8]);
        [header, items.to_vec(), footer].concat()
    }

    #[test]
    fn scan_all_tags() {
        let lyrics = b"LYRICSBEGININD00002110000011LYRICS200".to_vec();
        let lyrics = {
            let body = &lyrics[..lyrics.len() - 15];
            [
                body.to_vec(),
                format!("{:06}", body.len()).into_bytes(),
                b"LYRICS200".to_vec(),
            ]
            .concat()
        };
        let mut id3v1 = vec![0u8; 128];
        id3v1[..3].copy_from_slice(b"TAG");
        let audio = vec![0xAAu8; 500];
        let file = [
            id3v2(100),
            audio.clone(),
            ape(b"xyz"),
            lyrics.clone(),
            id3v1,
        ]
        .concat();
        let layout = Layout::scan(&mut Cursor::new(&file)).unwrap();
        assert_eq!(layout.front_len, 110);
        assert_eq!(layout.audio_end, 610);
        assert_eq!(layout.ape, Some(610..610 + 67));
        assert_eq!(layout.lyrics3, Some(677..677 + lyrics.len() as u64));
        assert_eq!(
            layout.id3v1,
            Some(file.len() as u64 - 128..file.len() as u64)
        );

        let bare = Layout::scan(&mut Cursor::new(&audio)).unwrap();
        assert_eq!((bare.front_len, bare.audio_end), (0, 500));
        assert_eq!((bare.ape, bare.lyrics3, bare.id3v1), (None, None, None));
    }

    #[test]
    fn padding_updates_size() {
        let tag = id3v2(20);
        let padded = pad_id3v2(&tag, 300);
        assert_eq!(padded.len(), 300);
        assert_eq!(
            front_id3v2_len(&mut Cursor::new(&padded), 300).unwrap(),
            300
        );
    }
}
