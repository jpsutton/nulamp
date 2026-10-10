//! APEv2 field mapping (spec 10 §7.2, §7.5).
//!
//! Keys are matched without regard to case; the first matching item wins. `Rating` holds
//! the 0–5 star value itself.

use lofty::ape::{ApeItem, ApeTag};
use lofty::picture::Picture;
use lofty::tag::ItemValue;

use crate::{Cover, join_values};

/// Item key written for front covers.
const FRONT_COVER: &str = "Cover Art (Front)";

/// Item key for `field`; `None` when APEv2 has no mapping.
fn key(field: &str) -> Option<&str> {
    Some(match field {
        "title" => "Title",
        "artist" => "Artist",
        "album" => "Album",
        "albumartist" => "Album Artist",
        "year" => "Year",
        "genre" => "Genre",
        "comment" => "Comment",
        "track" => "Track",
        "disc" => "Disc",
        "composer" => "Composer",
        "publisher" => "Publisher",
        "bpm" => "BPM",
        "conductor" => "Conductor",
        "lyricist" => "Lyricist",
        "subtitle" => "Subtitle",
        "category" => "Grouping",
        "isrc" => "ISRC",
        "copyright" => "Copyright",
        "rating" => "Rating",
        f if f.starts_with("replaygain_") => f,
        _ => return None,
    })
}

/// Fields APEv2 can hold.
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
    "subtitle",
    "category",
    "isrc",
    "copyright",
    "rating",
    "replaygain_track_gain",
    "replaygain_track_peak",
    "replaygain_album_gain",
    "replaygain_album_peak",
];

/// Value of `field`, or `None` when missing, empty or binary.
pub(crate) fn get(tag: &ApeTag, field: &str) -> Option<String> {
    let item = tag.get(key(field)?)?;
    let value = match item.value() {
        ItemValue::Text(s) | ItemValue::Locator(s) => join_values(s),
        ItemValue::Binary(_) => return None,
    };
    if field == "rating" {
        return crate::rating::parse_stars(&value).and_then(crate::rating::stars_field);
    }
    (!value.is_empty()).then_some(value)
}

/// Sets `field` (`None` deletes it). Returns false when APEv2 has no mapping.
pub(crate) fn set(tag: &mut ApeTag, field: &str, value: Option<&str>) -> bool {
    let Some(key) = key(field) else {
        return false;
    };
    tag.remove(key);
    if let Some(v) = value
        && let Ok(item) = ApeItem::new(key.to_string(), ItemValue::Text(v.to_string()))
    {
        tag.insert(item);
    }
    true
}

/// `Cover Art (Front)`, else the first other binary `Cover Art (…)` item.
pub(crate) fn cover(tag: &ApeTag) -> Option<Cover> {
    let picture = |item: &ApeItem| match item.value() {
        ItemValue::Binary(bytes) => Picture::from_ape_bytes(item.key(), bytes).ok(),
        _ => None,
    };
    tag.get(FRONT_COVER)
        .and_then(picture)
        .or_else(|| {
            tag.into_iter()
                .filter(|i| i.key().to_ascii_lowercase().starts_with("cover art"))
                .find_map(picture)
        })
        .map(|p| Cover::from_picture(&p))
}

/// Replaces or removes `Cover Art (Front)`.
pub(crate) fn set_cover(tag: &mut ApeTag, cover: Option<&Cover>) {
    tag.remove(FRONT_COVER);
    if let Some(cover) = cover {
        let bytes = cover.to_picture().as_ape_bytes();
        if let Ok(item) = ApeItem::new(FRONT_COVER.to_string(), ItemValue::Binary(bytes)) {
            tag.insert(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_and_round_trip() {
        let mut tag = ApeTag::new();
        tag.insert(ApeItem::new("TITLE".into(), ItemValue::Text("T".into())).unwrap());
        assert_eq!(get(&tag, "title").as_deref(), Some("T"));
        for f in FIELDS {
            let v = if *f == "rating" { "5" } else { f };
            assert!(set(&mut tag, f, Some(v)));
            assert_eq!(get(&tag, f).as_deref(), Some(v), "{f}");
        }
        for f in FIELDS {
            set(&mut tag, f, None);
            assert_eq!(get(&tag, f), None);
        }
        assert!(!set(&mut tag, "mood", Some("x")));
    }
}
