//! ID3v2 field mapping (spec 10 §7.2, §7.3).

use std::borrow::Cow;

use lofty::TextEncoding;
use lofty::id3::v2::{
    AttachedPictureFrame, CommentFrame, ExtendedTextFrame, ExtendedUrlFrame, Frame, FrameId,
    Id3v2Tag, PopularimeterFrame, TextInformationFrame,
};
use lofty::picture::{Picture, PictureType};

use crate::genre::decode_tcon;
use crate::rating::{parse_stars, popm_to_stars, stars_field, stars_to_popm};
use crate::{Cover, join_values};

/// `TXXX` descriptions read for `albumartist` when `TPE2` is missing.
const ALBUM_ARTIST_TXXX: [&str; 3] = ["ALBUM ARTIST", "ALBUMARTIST", "Band"];

/// `TXXX` description holding `mood` in ID3v2.3 tags, which have no `TMOO`.
const MOOD_TXXX: &str = "MOOD";

/// Plain text frame for `field`.
fn text_frame_id(field: &str) -> Option<&'static str> {
    Some(match field {
        "title" => "TIT2",
        "artist" => "TPE1",
        "album" => "TALB",
        "albumartist" => "TPE2",
        "genre" => "TCON",
        "track" => "TRCK",
        "disc" => "TPOS",
        "composer" => "TCOM",
        "publisher" => "TPUB",
        "bpm" => "TBPM",
        "conductor" => "TPE3",
        "lyricist" => "TEXT",
        "key" => "TKEY",
        "mood" => "TMOO",
        "subtitle" => "TIT3",
        "category" => "TIT1",
        "isrc" => "TSRC",
        "remixing" => "TPE4",
        "originalartist" => "TOPE",
        "encoder" => "TSSE",
        "tool" => "TENC",
        "copyright" => "TCOP",
        "compilation" => "TCMP",
        _ => return None,
    })
}

/// Value of `field`, or `None` when missing or empty.
///
/// `rating_email` selects the `POPM` frame; empty uses the first one.
pub(crate) fn get(tag: &Id3v2Tag, field: &str, rating_email: &str) -> Option<String> {
    let value = match field {
        "albumartist" => text(tag, "TPE2").or_else(|| user_text(tag, &ALBUM_ARTIST_TXXX)),
        "year" => text(tag, "TDRC").or_else(|| text(tag, "TYER")),
        "genre" => raw_text(tag, "TCON").map(|v| decode_tcon(&v)),
        "mood" => text(tag, "TMOO").or_else(|| user_text(tag, &[MOOD_TXXX])),
        "comment" => comment(tag),
        "rating" => popm(tag, rating_email).and_then(|f| stars_field(popm_to_stars(f.rating))),
        "url" => url(tag),
        f if f.starts_with("replaygain_") => user_text(tag, &[f]),
        f => text_frame_id(f).and_then(|id| text(tag, id)),
    };
    value.filter(|v| !v.is_empty())
}

/// Sets `field` (`None` deletes it and its aliases). Returns false for an unknown field.
///
/// `v24` selects the frames of the version that will be written: `year` goes to `TDRC`
/// (2.4) or `TYER` (2.3), `mood` to `TMOO` (2.4) or `TXXX:MOOD` (2.3). New text frames are
/// UTF-8 (written as UTF-16 in 2.3); `WXXX` uses Latin-1, as `lofty` encodes the URL with
/// the frame's encoding.
pub(crate) fn set(
    tag: &mut Id3v2Tag,
    field: &str,
    value: Option<&str>,
    v24: bool,
    rating_email: &str,
) -> bool {
    match field {
        "albumartist" => {
            remove_user_text(tag, &ALBUM_ARTIST_TXXX);
            set_text(tag, "TPE2", value);
        }
        "year" => {
            tag.retain(|f| f.id_str() != "TDRC" && f.id_str() != "TYER");
            set_text(tag, if v24 { "TDRC" } else { "TYER" }, value);
        }
        "mood" => {
            remove_user_text(tag, &[MOOD_TXXX]);
            if v24 {
                set_text(tag, "TMOO", value);
            } else {
                tag.retain(|f| f.id_str() != "TMOO");
                set_user_text(tag, MOOD_TXXX, value);
            }
        }
        "comment" => {
            tag.retain(|f| !matches!(f, Frame::Comment(c) if c.description.is_empty()));
            if let Some(v) = value {
                tag.insert(Frame::Comment(CommentFrame::new(
                    TextEncoding::UTF8,
                    *b"eng",
                    "",
                    v.to_string(),
                )));
            }
        }
        "rating" => set_popm(tag, value.and_then(parse_stars).unwrap_or(0), rating_email),
        "url" => {
            tag.retain(|f| !matches!(f, Frame::UserUrl(u) if u.description.is_empty()));
            if let Some(v) = value {
                tag.insert(Frame::UserUrl(ExtendedUrlFrame::new(
                    TextEncoding::Latin1,
                    "",
                    v.to_string(),
                )));
            }
        }
        f if f.starts_with("replaygain_") => {
            remove_user_text(tag, &[f]);
            set_user_text(tag, f, value);
        }
        f => match text_frame_id(f) {
            Some(id) => set_text(tag, id, value),
            None => return false,
        },
    }
    true
}

/// First front cover (`APIC` type 3), else the first picture.
pub(crate) fn cover(tag: &Id3v2Tag) -> Option<Cover> {
    let pictures: Vec<&Picture> = tag
        .into_iter()
        .filter_map(|f| match f {
            Frame::Picture(p) => Some(p.picture.as_ref()),
            _ => None,
        })
        .collect();
    pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or(pictures.first())
        .map(|p| Cover::from_picture(p))
}

/// Replaces the front covers with `cover`, or removes them.
///
/// The `APIC` frame uses Latin-1: `lofty` ends a missing UTF-16 description with one byte,
/// which it then cannot read back.
pub(crate) fn set_cover(tag: &mut Id3v2Tag, cover: Option<&Cover>) {
    tag.retain(
        |f| !matches!(f, Frame::Picture(p) if p.picture.pic_type() == PictureType::CoverFront),
    );
    if let Some(cover) = cover {
        tag.insert(Frame::Picture(AttachedPictureFrame::new(
            TextEncoding::Latin1,
            cover.to_picture(),
        )));
    }
}

/// Raw text of the first frame `id`, Latin-1 text re-read as UTF-8 when it is valid UTF-8.
fn raw_text(tag: &Id3v2Tag, id: &str) -> Option<String> {
    tag.into_iter()
        .find(|f| f.id_str() == id)
        .and_then(|f| match f {
            Frame::Text(t) => Some(fix_latin1(t.encoding, &t.value)),
            Frame::Timestamp(t) => Some(t.timestamp.to_string()),
            _ => None,
        })
}

fn text(tag: &Id3v2Tag, id: &str) -> Option<String> {
    raw_text(tag, id).map(|v| join_values(&v))
}

/// First `TXXX` whose description matches one of `names` (case-insensitive, in that order).
fn user_text(tag: &Id3v2Tag, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        tag.into_iter().find_map(|f| match f {
            Frame::UserText(t)
                if t.description.eq_ignore_ascii_case(name) && !t.content.is_empty() =>
            {
                Some(join_values(&fix_latin1(t.encoding, &t.content)))
            }
            _ => None,
        })
    })
}

fn comment(tag: &Id3v2Tag) -> Option<String> {
    tag.into_iter().find_map(|f| match f {
        Frame::Comment(c) if c.description.is_empty() => Some(fix_latin1(c.encoding, &c.content)),
        _ => None,
    })
}

fn url(tag: &Id3v2Tag) -> Option<String> {
    let urls = || {
        tag.into_iter().filter_map(|f| match f {
            Frame::UserUrl(u) => Some(u),
            _ => None,
        })
    };
    urls()
        .find(|u| u.description.is_empty())
        .or_else(|| urls().next())
        .map(|u| u.content.to_string())
}

fn popm<'a>(tag: &'a Id3v2Tag, email: &str) -> Option<&'a PopularimeterFrame<'static>> {
    tag.into_iter().find_map(|f| match f {
        Frame::Popularimeter(p) if email.is_empty() || p.email == email => Some(p),
        _ => None,
    })
}

/// Sets the rating; 0 stars deletes the matching `POPM` frames.
///
/// An existing frame keeps its e-mail and play counter; a new one uses `email`.
fn set_popm(tag: &mut Id3v2Tag, stars: u8, email: &str) {
    let existing = popm(tag, email).cloned();
    tag.retain(|f| match f {
        Frame::Popularimeter(p) => !(email.is_empty() || p.email == email),
        _ => true,
    });
    if stars == 0 {
        return;
    }
    let frame = match existing {
        Some(mut p) => {
            p.rating = stars_to_popm(stars);
            p
        }
        None => PopularimeterFrame::new(email.to_string(), stars_to_popm(stars), 0),
    };
    tag.insert(Frame::Popularimeter(frame));
}

fn set_text(tag: &mut Id3v2Tag, id: &'static str, value: Option<&str>) {
    tag.retain(|f| f.id_str() != id);
    if let Some(v) = value {
        tag.insert(Frame::Text(TextInformationFrame::new(
            FrameId::Valid(Cow::Borrowed(id)),
            TextEncoding::UTF8,
            v.to_string(),
        )));
    }
}

fn set_user_text(tag: &mut Id3v2Tag, description: &str, value: Option<&str>) {
    if let Some(v) = value {
        tag.insert(Frame::UserText(ExtendedTextFrame::new(
            TextEncoding::UTF8,
            description.to_string(),
            v.to_string(),
        )));
    }
}

fn remove_user_text(tag: &mut Id3v2Tag, names: &[&str]) {
    tag.retain(|f| {
        !matches!(f, Frame::UserText(t) if names.iter().any(|n| t.description.eq_ignore_ascii_case(n)))
    });
}

/// Text from a Latin-1 frame whose bytes are valid UTF-8 is re-read as UTF-8 (spec 09 §2).
fn fix_latin1(encoding: TextEncoding, text: &str) -> String {
    if encoding == TextEncoding::Latin1
        && !text.is_ascii()
        && text.chars().all(|c| u32::from(c) <= 0xFF)
    {
        let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
        if let Ok(s) = String::from_utf8(bytes) {
            return s;
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[&str] = crate::FIELDS;

    #[test]
    fn every_field_round_trips_in_memory() {
        for v24 in [false, true] {
            let mut tag = Id3v2Tag::new();
            for f in ALL {
                let value = if *f == "rating" { "4" } else { f };
                assert!(set(&mut tag, f, Some(value), v24, ""), "{f}");
            }
            for f in ALL {
                let want = if *f == "rating" { "4" } else { f };
                assert_eq!(get(&tag, f, "").as_deref(), Some(want), "{f} v24={v24}");
            }
            for f in ALL {
                set(&mut tag, f, None, v24, "");
                assert_eq!(get(&tag, f, ""), None, "{f}");
            }
            assert!(tag.into_iter().next().is_none());
        }
    }

    #[test]
    fn album_artist_fallbacks() {
        let mut tag = Id3v2Tag::new();
        tag.insert_user_text("Band".into(), "B".into());
        assert_eq!(get(&tag, "albumartist", "").as_deref(), Some("B"));
        tag.insert_user_text("album artist".into(), "AA".into());
        assert_eq!(get(&tag, "albumartist", "").as_deref(), Some("AA"));
        set(&mut tag, "albumartist", None, false, "");
        assert!(tag.into_iter().next().is_none());
    }

    #[test]
    fn popm_email_selection() {
        let mut tag = Id3v2Tag::new();
        tag.insert(Frame::Popularimeter(PopularimeterFrame::new("a@x", 64, 5)));
        tag.insert(Frame::Popularimeter(PopularimeterFrame::new("b@x", 255, 0)));
        assert_eq!(get(&tag, "rating", "").as_deref(), Some("2"));
        assert_eq!(get(&tag, "rating", "b@x").as_deref(), Some("5"));
        assert_eq!(get(&tag, "rating", "c@x"), None);
        set(&mut tag, "rating", Some("3"), false, "a@x");
        let p = popm(&tag, "a@x").unwrap();
        assert_eq!((p.rating, p.counter), (128, 5));
        set(&mut tag, "rating", Some("1"), false, "c@x");
        assert_eq!(popm(&tag, "c@x").unwrap().rating, 1);
        set(&mut tag, "rating", None, false, "b@x");
        assert!(popm(&tag, "b@x").is_none());
        set(&mut tag, "rating", None, false, "");
        assert!(popm(&tag, "").is_none());
    }

    #[test]
    fn latin1_utf8_fix() {
        assert_eq!(fix_latin1(TextEncoding::Latin1, "CafÃ©"), "Café");
        assert_eq!(fix_latin1(TextEncoding::Latin1, "Café"), "Café");
        assert_eq!(fix_latin1(TextEncoding::UTF16, "CafÃ©"), "CafÃ©");
    }
}
