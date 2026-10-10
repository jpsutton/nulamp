//! Vorbis comment field mapping (spec 10 §7.2), for FLAC, Ogg Vorbis, Opus and Speex.
//!
//! Keys are matched without regard to case. When a field has several keys, the keys are
//! tried in the listed order and the first entry with that key wins.

use lofty::ogg::{OggPictureStorage, VorbisComments};
use lofty::picture::{Picture, PictureInformation, PictureType};

use crate::Cover;
use crate::rating::{parse_stars, percent_to_stars, stars_field, stars_to_percent};

/// Keys for `field`, the written key first; `None` when Vorbis comments have no mapping.
fn keys(field: &str) -> Option<&'static [&'static str]> {
    Some(match field {
        "title" => &["TITLE"],
        "artist" => &["ARTIST"],
        "album" => &["ALBUM"],
        "albumartist" => &["ALBUMARTIST", "ALBUM ARTIST", "ENSEMBLE"],
        "year" => &["DATE", "YEAR"],
        "genre" => &["GENRE"],
        "comment" => &["COMMENT", "DESCRIPTION"],
        "track" => &["TRACKNUMBER"],
        "disc" => &["DISCNUMBER"],
        "composer" => &["COMPOSER"],
        "publisher" => &["PUBLISHER", "ORGANIZATION"],
        "bpm" => &["BPM"],
        "conductor" => &["CONDUCTOR"],
        "lyricist" => &["LYRICIST"],
        "key" => &["KEY"],
        "mood" => &["MOOD"],
        "subtitle" => &["SUBTITLE", "VERSION"],
        "category" => &["GROUPING", "CONTENTGROUP", "CATEGORY"],
        "isrc" => &["ISRC"],
        "remixing" => &["REMIXER", "REMIXING"],
        "encoder" => &["ENCODER"],
        "tool" => &["ENCODED-BY"],
        "copyright" => &["COPYRIGHT"],
        "compilation" => &["COMPILATION"],
        "rating" => &["RATING"],
        "replaygain_track_gain" => &["REPLAYGAIN_TRACK_GAIN"],
        "replaygain_track_peak" => &["REPLAYGAIN_TRACK_PEAK"],
        "replaygain_album_gain" => &["REPLAYGAIN_ALBUM_GAIN"],
        "replaygain_album_peak" => &["REPLAYGAIN_ALBUM_PEAK"],
        "r128_track_gain" => &["R128_TRACK_GAIN"],
        "r128_album_gain" => &["R128_ALBUM_GAIN"],
        _ => return None,
    })
}

/// Key holding the total for `track` and `disc`.
fn total_key(field: &str) -> Option<&'static str> {
    match field {
        "track" => Some("TRACKTOTAL"),
        "disc" => Some("DISCTOTAL"),
        _ => None,
    }
}

/// Fields Vorbis comments can hold (without the Opus-only R128 gains).
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
    "conductor",
    "lyricist",
    "key",
    "mood",
    "subtitle",
    "category",
    "isrc",
    "remixing",
    "encoder",
    "tool",
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
/// `track` and `disc` add the total as `n/N` when the number has none. `RATING` (0–100)
/// becomes 0–5 stars.
pub(crate) fn get(tag: &VorbisComments, field: &str) -> Option<String> {
    let first = |key: &str| tag.get(key).filter(|v| !v.is_empty());
    let value = keys(field)?.iter().find_map(|k| first(k))?.to_string();
    if field == "rating" {
        return value
            .trim()
            .parse()
            .ok()
            .map(percent_to_stars)
            .and_then(stars_field);
    }
    if let Some(total) = total_key(field).and_then(first)
        && !value.contains('/')
    {
        return Some(format!("{value}/{total}"));
    }
    Some(value)
}

/// Sets `field` (`None` deletes it and its aliases). Returns false when unmapped.
///
/// `n/N` for `track` and `disc` writes the number and the total separately.
pub(crate) fn set(tag: &mut VorbisComments, field: &str, value: Option<&str>) -> bool {
    let Some(keys) = keys(field) else {
        return false;
    };
    for k in keys {
        let _ = tag.remove(k);
    }
    let total = total_key(field);
    if let Some(t) = total {
        let _ = tag.remove(t);
    }
    let Some(value) = value else {
        return true;
    };
    match (field, total, value.split_once('/')) {
        ("rating", ..) => {
            if let Some(stars) = parse_stars(value).filter(|s| *s > 0) {
                tag.push(keys[0].to_string(), stars_to_percent(stars).to_string());
            }
        }
        (_, Some(total_key), Some((n, total))) => {
            if !n.trim().is_empty() {
                tag.push(keys[0].to_string(), n.trim().to_string());
            }
            if !total.trim().is_empty() {
                tag.push(total_key.to_string(), total.trim().to_string());
            }
        }
        _ => tag.push(keys[0].to_string(), value.to_string()),
    }
    true
}

/// First front cover, else the first picture, from `pictures`.
pub(crate) fn cover_in(pictures: &[(Picture, PictureInformation)]) -> Option<Cover> {
    pictures
        .iter()
        .find(|(p, _)| p.pic_type() == PictureType::CoverFront)
        .or(pictures.first())
        .map(|(p, _)| Cover::from_picture(p))
}

/// Replaces the front covers in `storage` with `cover`, or removes them.
pub(crate) fn set_cover_in<S: OggPictureStorage>(storage: &mut S, cover: Option<&Cover>) {
    storage.remove_picture_type(PictureType::CoverFront);
    if let Some(cover) = cover {
        let picture = cover.to_picture();
        let info = PictureInformation::from_picture(&picture).unwrap_or_default();
        let _ = storage.insert_picture(picture, Some(info));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_and_aliases() {
        let mut tag = VorbisComments::new();
        tag.push("tracknumber".into(), "3".into());
        tag.push("TrackTotal".into(), "12".into());
        tag.push("Album Artist".into(), "AA".into());
        tag.push("ensemble".into(), "E".into());
        tag.push("ARTIST".into(), "First".into());
        tag.push("ARTIST".into(), "Second".into());
        assert_eq!(get(&tag, "track").as_deref(), Some("3/12"));
        assert_eq!(get(&tag, "albumartist").as_deref(), Some("AA"));
        assert_eq!(get(&tag, "artist").as_deref(), Some("First"));
        set(&mut tag, "albumartist", Some("New"));
        assert_eq!(tag.get("ENSEMBLE"), None);
        assert_eq!(tag.get("ALBUM ARTIST"), None);
        assert_eq!(tag.get("ALBUMARTIST"), Some("New"));
        set(&mut tag, "disc", Some("1/2"));
        assert_eq!(tag.get("DISCNUMBER"), Some("1"));
        assert_eq!(tag.get("DISCTOTAL"), Some("2"));
        set(&mut tag, "track", Some("4"));
        assert_eq!(tag.get("TRACKTOTAL"), None);
        assert_eq!(get(&tag, "track").as_deref(), Some("4"));
    }

    #[test]
    fn rating_scale() {
        let mut tag = VorbisComments::new();
        set(&mut tag, "rating", Some("3"));
        assert_eq!(tag.get("RATING"), Some("60"));
        assert_eq!(get(&tag, "rating").as_deref(), Some("3"));
        set(&mut tag, "rating", Some("0"));
        assert_eq!(tag.get("RATING"), None);
    }
}
