//! RIFF `INFO` mapping for WAV files (spec 09 §1), read after the `ID3 ` chunk.

use lofty::iff::wav::RiffInfoList;

/// Chunk IDs for `field`, the written one first.
fn keys(field: &str) -> Option<&'static [&'static str]> {
    Some(match field {
        "title" => &["INAM"],
        "artist" => &["IART"],
        "album" => &["IPRD"],
        "comment" => &["ICMT"],
        "genre" => &["IGNR"],
        "year" => &["ICRD"],
        "track" => &["ITRK", "IPRT"],
        "copyright" => &["ICOP"],
        "encoder" => &["ISFT"],
        _ => return None,
    })
}

/// Value of `field`, or `None` when missing, empty or unmapped.
pub(crate) fn get(tag: &RiffInfoList, field: &str) -> Option<String> {
    keys(field)?
        .iter()
        .find_map(|k| tag.get(k).map(|v| v.trim_end_matches('\0').to_string()))
        .filter(|v| !v.is_empty())
}

/// Sets `field` (`None` deletes it); unmapped fields are ignored.
pub(crate) fn set(tag: &mut RiffInfoList, field: &str, value: Option<&str>) {
    let Some(keys) = keys(field) else { return };
    for k in keys {
        let _ = tag.remove(k);
    }
    if let Some(v) = value {
        tag.insert(keys[0].to_string(), v.to_string());
    }
}
