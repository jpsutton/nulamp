//! MP4 `ilst` field mapping (spec 10 §7.2).
//!
//! Freeform keys are written as `----:com.apple.iTunes:<name>` and read under any mean,
//! matching the name without regard to case.

use std::borrow::Cow;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;

use lofty::mp4::{Atom, AtomData, AtomIdent, DataType, Ilst};

use crate::genre::genre_name;
use crate::rating::{parse_stars, percent_to_stars, stars_field, stars_to_percent};
use crate::{Cover, join_values};

/// Mean written for freeform atoms.
const ITUNES_MEAN: &str = "com.apple.iTunes";

/// Where a field lives in the `ilst`.
enum Slot {
    Text([u8; 4]),
    Freeform(&'static str),
}

fn slot(field: &str) -> Option<Slot> {
    Some(match field {
        "title" => Slot::Text(*b"\xa9nam"),
        "artist" => Slot::Text(*b"\xa9ART"),
        "album" => Slot::Text(*b"\xa9alb"),
        "albumartist" => Slot::Text(*b"aART"),
        "year" => Slot::Text(*b"\xa9day"),
        "genre" => Slot::Text(*b"\xa9gen"),
        "comment" => Slot::Text(*b"\xa9cmt"),
        "composer" => Slot::Text(*b"\xa9wrt"),
        "category" => Slot::Text(*b"\xa9grp"),
        "encoder" => Slot::Text(*b"\xa9too"),
        "copyright" => Slot::Text(*b"cprt"),
        "track" => Slot::Text(*b"trkn"),
        "disc" => Slot::Text(*b"disk"),
        "bpm" => Slot::Text(*b"tmpo"),
        "compilation" => Slot::Text(*b"cpil"),
        "rating" => Slot::Text(*b"rate"),
        "publisher" => Slot::Freeform("publisher"),
        "replaygain_track_gain" => Slot::Freeform("replaygain_track_gain"),
        "replaygain_track_peak" => Slot::Freeform("replaygain_track_peak"),
        "replaygain_album_gain" => Slot::Freeform("replaygain_album_gain"),
        "replaygain_album_peak" => Slot::Freeform("replaygain_album_peak"),
        _ => return None,
    })
}

/// Fields an `ilst` can hold.
pub(crate) const FIELDS: &[&str] = &[
    "title",
    "artist",
    "album",
    "albumartist",
    "year",
    "genre",
    "comment",
    "track",
    "disc",
    "composer",
    "publisher",
    "bpm",
    "category",
    "encoder",
    "copyright",
    "compilation",
    "rating",
    "replaygain_track_gain",
    "replaygain_track_peak",
    "replaygain_album_gain",
    "replaygain_album_peak",
];

/// Value of `field`, or `None` when missing or empty.
///
/// `trkn`/`disk` give `n` or `n/N`; `gnre` (a 1-based genre index) is used when `©gen` is
/// missing; `rate` (0–100) becomes 0–5 stars; `cpil` gives `1` or `0`.
pub(crate) fn get(tag: &Ilst, field: &str) -> Option<String> {
    let value = match (field, slot(field)?) {
        ("track" | "disc", Slot::Text(fourcc)) => number_pair(tag, fourcc),
        ("genre", Slot::Text(fourcc)) => atom_text(tag, fourcc).or_else(|| gnre(tag)),
        ("rating", Slot::Text(fourcc)) => atom_text(tag, fourcc)
            .and_then(|v| v.trim().parse().ok())
            .map(percent_to_stars)
            .and_then(stars_field),
        (_, Slot::Text(fourcc)) => atom_text(tag, fourcc),
        (_, Slot::Freeform(name)) => freeform(tag, name),
    };
    value.filter(|v| !v.is_empty())
}

/// Sets `field` (`None` deletes it and its aliases). Returns false when unmapped.
pub(crate) fn set(tag: &mut Ilst, field: &str, value: Option<&str>) -> bool {
    let Some(slot) = slot(field) else {
        return false;
    };
    match slot {
        Slot::Text(fourcc) => {
            let _ = tag.remove(&AtomIdent::Fourcc(fourcc));
            if field == "genre" {
                let _ = tag.remove(&AtomIdent::Fourcc(*b"gnre"));
            }
            if let Some(data) = value.and_then(|v| encode(field, v)) {
                tag.insert(Atom::new(AtomIdent::Fourcc(fourcc), data));
            }
        }
        Slot::Freeform(name) => {
            tag.retain(|a| !matches!(a.ident(), AtomIdent::Freeform { name: n, .. } if n.eq_ignore_ascii_case(name)));
            if let Some(v) = value {
                let ident = AtomIdent::Freeform {
                    mean: Cow::Borrowed(ITUNES_MEAN),
                    name: Cow::Borrowed(name),
                };
                tag.insert(Atom::new(ident, AtomData::UTF8(v.to_string())));
            }
        }
    }
    true
}

/// First `covr` image.
pub(crate) fn cover(tag: &Ilst) -> Option<Cover> {
    tag.pictures()?.next().map(Cover::from_picture)
}

/// Replaces the `covr` images with `cover`, or removes them.
pub(crate) fn set_cover(tag: &mut Ilst, cover: Option<&Cover>) {
    tag.remove_pictures();
    if let Some(cover) = cover {
        tag.insert_picture(cover.to_picture());
    }
}

/// Atom data for a value; `None` when the value cannot be stored (deletes the field).
fn encode(field: &str, value: &str) -> Option<AtomData> {
    Some(match field {
        "track" | "disc" => {
            let (n, total) = value.split_once('/').unwrap_or((value, ""));
            let n: u16 = n.trim().parse().unwrap_or(0);
            let total: u16 = total.trim().parse().unwrap_or(0);
            if n == 0 && total == 0 {
                return None;
            }
            let [n0, n1] = n.to_be_bytes();
            let [t0, t1] = total.to_be_bytes();
            let mut data = vec![0, 0, n0, n1, t0, t1];
            if field == "track" {
                data.extend([0, 0]);
            }
            AtomData::Unknown {
                code: DataType::Reserved,
                data,
            }
        }
        "bpm" => AtomData::Unknown {
            code: DataType::BeSignedInteger,
            data: value.trim().parse::<u16>().ok()?.to_be_bytes().to_vec(),
        },
        "compilation" => AtomData::Bool(!matches!(value.trim(), "0" | "")),
        "rating" => {
            let stars = parse_stars(value).filter(|s| *s > 0)?;
            AtomData::UTF8(stars_to_percent(stars).to_string())
        }
        _ => AtomData::UTF8(value.to_string()),
    })
}

/// Text of the atom `fourcc`; several values are joined, numbers and flags are formatted.
fn atom_text(tag: &Ilst, fourcc: [u8; 4]) -> Option<String> {
    data_text(tag.get(&AtomIdent::Fourcc(fourcc))?)
}

fn data_text(atom: &Atom<'_>) -> Option<String> {
    let values: Vec<String> = atom
        .data()
        .filter_map(|d| match d {
            AtomData::UTF8(s) | AtomData::UTF16(s) => Some(s.clone()),
            AtomData::SignedInteger(i) => Some(i.to_string()),
            AtomData::UnsignedInteger(u) => Some(u.to_string()),
            AtomData::Bool(b) => Some(if *b { "1" } else { "0" }.to_string()),
            AtomData::Unknown { code, data }
                if matches!(
                    code,
                    DataType::BeSignedInteger | DataType::BeUnsignedInteger
                ) && !data.is_empty()
                    && data.len() <= 8 =>
            {
                Some(
                    data.iter()
                        .fold(0u64, |acc, b| acc << 8 | u64::from(*b))
                        .to_string(),
                )
            }
            _ => None,
        })
        .collect();
    (!values.is_empty()).then(|| join_values(&values.join("\0")))
}

/// `n` or `n/N` from a `trkn`/`disk` atom.
fn number_pair(tag: &Ilst, fourcc: [u8; 4]) -> Option<String> {
    let atom = tag.get(&AtomIdent::Fourcc(fourcc))?;
    atom.data().find_map(|d| match d {
        AtomData::Unknown { data, .. } if data.len() >= 6 => {
            let n = u16::from_be_bytes([data[2], data[3]]);
            let total = u16::from_be_bytes([data[4], data[5]]);
            match (n, total) {
                (0, 0) => None,
                (n, 0) => Some(n.to_string()),
                (n, total) => Some(format!("{n}/{total}")),
            }
        }
        _ => None,
    })
}

/// Genre from a `gnre` atom (1-based index into the genre list).
fn gnre(tag: &Ilst) -> Option<String> {
    let atom = tag.get(&AtomIdent::Fourcc(*b"gnre"))?;
    atom.data().find_map(|d| {
        let index = match d {
            AtomData::Unknown { data, .. } if data.len() == 2 => {
                u16::from_be_bytes([data[0], data[1]])
            }
            AtomData::SignedInteger(i) => u16::try_from(*i).ok()?,
            AtomData::UnsignedInteger(u) => u16::try_from(*u).ok()?,
            _ => return None,
        };
        let index = u8::try_from(index.checked_sub(1)?).ok()?;
        genre_name(index).map(str::to_string)
    })
}

/// First freeform atom called `name` (any mean, case-insensitive).
fn freeform(tag: &Ilst, name: &str) -> Option<String> {
    tag.into_iter()
        .filter(|a| matches!(a.ident(), AtomIdent::Freeform { name: n, .. } if n.eq_ignore_ascii_case(name)))
        .find_map(data_text)
}

/// Empties the tag of an MP4 file in place by renaming `moov.udta.meta.ilst` to `free`.
///
/// Used instead of `lofty`'s tag removal, which can leave a `meta` atom that `lofty`
/// itself cannot read back. Atom sizes do not change, so no offsets move. Returns whether
/// an `ilst` atom was found.
pub(crate) fn blank_ilst<F: Read + Write + Seek>(file: &mut F) -> io::Result<bool> {
    let len = file.seek(SeekFrom::End(0))?;
    let Some(moov) = child(file, 0..len, b"moov")? else {
        return Ok(false);
    };
    let Some(udta) = child(file, moov, b"udta")? else {
        return Ok(false);
    };
    let Some(mut meta) = child(file, udta, b"meta")? else {
        return Ok(false);
    };
    let mut probe = [0; 8];
    file.seek(SeekFrom::Start(meta.start))?;
    if meta.end - meta.start >= 8 {
        file.read_exact(&mut probe)?;
    }
    if !matches!(&probe[4..], b"hdlr" | b"ilst" | b"mhdr" | b"ctry" | b"lang") {
        meta.start += 4;
    }
    let Some(ilst) = child(file, meta, b"ilst")? else {
        return Ok(false);
    };
    file.seek(SeekFrom::Start(ilst.start - 4))?;
    file.write_all(b"free")?;
    Ok(true)
}

/// Content range (after the header) of the first child atom `fourcc` within `range`.
fn child<F: Read + Seek>(
    file: &mut F,
    range: Range<u64>,
    fourcc: &[u8; 4],
) -> io::Result<Option<Range<u64>>> {
    let mut pos = range.start;
    while pos + 8 <= range.end {
        let mut header = [0; 8];
        file.seek(SeekFrom::Start(pos))?;
        file.read_exact(&mut header)?;
        let (len, header_len) =
            match u32::from_be_bytes([header[0], header[1], header[2], header[3]]) {
                0 => (range.end - pos, 8),
                1 => {
                    let mut large = [0; 8];
                    file.read_exact(&mut large)?;
                    (u64::from_be_bytes(large), 16)
                }
                n => (u64::from(n), 8),
            };
        if len < header_len || pos + len > range.end {
            return Ok(None);
        }
        if &header[4..] == fourcc {
            return Ok(Some(pos + header_len..pos + len));
        }
        pos += len;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_in_memory() {
        let mut tag = Ilst::new();
        for f in FIELDS {
            let v = match *f {
                "track" => "3/12",
                "disc" => "1/2",
                "bpm" => "120",
                "compilation" => "1",
                "rating" => "4",
                f => f,
            };
            assert!(set(&mut tag, f, Some(v)), "{f}");
            assert_eq!(get(&tag, f).as_deref(), Some(v), "{f}");
        }
        for f in FIELDS {
            set(&mut tag, f, None);
            assert_eq!(get(&tag, f), None, "{f}");
        }
        assert!(!set(&mut tag, "conductor", Some("x")));
    }

    #[test]
    fn gnre_index_and_freeform_mean() {
        let mut tag = Ilst::new();
        tag.insert(Atom::new(
            AtomIdent::Fourcc(*b"gnre"),
            AtomData::Unknown {
                code: DataType::Reserved,
                data: vec![0, 18],
            },
        ));
        assert_eq!(get(&tag, "genre").as_deref(), Some("Rock"));
        set(&mut tag, "genre", Some("Jazz"));
        assert!(tag.get(&AtomIdent::Fourcc(*b"gnre")).is_none());
        tag.insert(Atom::new(
            AtomIdent::Freeform {
                mean: Cow::Borrowed("org.example"),
                name: Cow::Borrowed("REPLAYGAIN_TRACK_GAIN"),
            },
            AtomData::UTF8("-6.5 dB".into()),
        ));
        assert_eq!(
            get(&tag, "replaygain_track_gain").as_deref(),
            Some("-6.5 dB")
        );
    }
}
