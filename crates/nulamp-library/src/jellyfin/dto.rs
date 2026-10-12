//! The parts of Jellyfin's JSON answers that the provider reads, and their conversion to
//! [`Track`]s.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::model::{Track, TrackId};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicInfo {
    pub server_name: Option<String>,
    pub id: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthResult {
    pub user: Option<User>,
    pub access_token: Option<String>,
    pub server_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct User {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemList {
    pub items: Option<Vec<Item>>,
    #[serde(default)]
    pub total_record_count: u64,
}

impl ItemList {
    pub fn into_items(self) -> Vec<Item> {
        self.items.unwrap_or_default()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserData {
    #[serde(default)]
    pub play_count: u64,
    pub last_played_date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaStream {
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSource {
    pub size: Option<u64>,
    pub bitrate: Option<u64>,
    pub media_streams: Option<Vec<MediaStream>>,
}

/// Prefix of the track ids of video items (`video:<item id>`): playback then needs no request
/// to know the item is a video.
pub const VIDEO_ID: &str = "video:";

/// Item types that play as videos.
const VIDEO_TYPES: [&str; 4] = ["Movie", "Episode", "Video", "MusicVideo"];

/// The server item of a track id, and whether it is a video.
pub fn item_of(id: &str) -> (&str, bool) {
    match id.strip_prefix(VIDEO_ID) {
        Some(item) => (item, true),
        None => (id, false),
    }
}

/// A `BaseItemDto`: a library, artist, album, genre, playlist, series, season, track or video.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub collection_type: Option<String>,
    pub media_type: Option<String>,
    pub run_time_ticks: Option<i64>,
    pub production_year: Option<i32>,
    pub index_number: Option<i32>,
    pub parent_index_number: Option<i32>,
    pub artists: Option<Vec<String>>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub album_artist: Option<String>,
    pub series_name: Option<String>,
    pub series_id: Option<String>,
    pub series_primary_image_tag: Option<String>,
    pub season_name: Option<String>,
    pub genres: Option<Vec<String>>,
    pub container: Option<String>,
    pub date_created: Option<String>,
    pub user_data: Option<UserData>,
    pub image_tags: Option<HashMap<String, String>>,
    pub album_primary_image_tag: Option<String>,
    pub normalization_gain: Option<f32>,
    pub child_count: Option<u64>,
    pub song_count: Option<u64>,
    pub media_sources: Option<Vec<MediaSource>>,
}

impl Item {
    pub fn name(&self) -> String {
        self.name.clone().unwrap_or_default()
    }

    pub fn is_audio(&self) -> bool {
        self.kind.as_deref() == Some("Audio")
    }

    /// A movie, episode, music video or other video.
    pub fn is_video(&self) -> bool {
        self.kind
            .as_deref()
            .is_some_and(|k| VIDEO_TYPES.contains(&k))
    }

    /// The item whose primary image shows this track: its own, else its album's or its
    /// series'.
    fn artwork_item(&self) -> Option<String> {
        if self
            .image_tags
            .as_ref()
            .is_some_and(|t| t.contains_key("Primary"))
        {
            return Some(self.id.clone());
        }
        self.album_primary_image_tag
            .as_ref()
            .and(self.album_id.clone())
            .or_else(|| {
                self.series_primary_image_tag
                    .as_ref()
                    .and(self.series_id.clone())
            })
    }

    /// The track this item describes, owned by `provider`. An episode's series is its artist
    /// and album artist and its season its album, so lists group and sort it as music; its
    /// season and episode numbers are the disc and track numbers.
    pub fn to_track(&self, provider: &str) -> Track {
        let episode = self.kind.as_deref() == Some("Episode");
        let mut tags = BTreeMap::new();
        let mut put = |k: &str, v: Option<String>| {
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                tags.insert(k.to_string(), v);
            }
        };
        put("title", self.name.clone());
        if episode {
            put("artist", self.series_name.clone());
            put("album", self.season_name.clone());
            put("albumartist", self.series_name.clone());
        } else {
            put("artist", self.artists.as_ref().map(|a| a.join("; ")));
            put("album", self.album.clone());
            put("albumartist", self.album_artist.clone());
        }
        put("year", self.production_year.map(|y| y.to_string()));
        put("genre", self.genres.as_ref().map(|g| g.join("; ")));
        let number = |n: Option<i32>| n.filter(|n| *n >= 0).map(|n| n.to_string());
        put("track", number(self.index_number));
        put("disc", number(self.parent_index_number));
        let id = if self.is_video() {
            TrackId::new(provider, format!("{VIDEO_ID}{}", self.id))
        } else {
            TrackId::new(provider, self.id.clone())
        };
        let user = self.user_data.as_ref();
        let source = self.media_sources.as_ref().and_then(|s| s.first());
        let audio = source
            .and_then(|s| s.media_streams.as_ref())
            .and_then(|s| s.iter().find(|m| m.kind.as_deref() == Some("Audio")));
        let mut t = Track {
            location: id.location(),
            id,
            tags,
            length_ms: self
                .run_time_ticks
                .map_or(0, |t| u64::try_from(t / 10_000).unwrap_or(0)),
            bitrate: source
                .and_then(|s| s.bitrate)
                .and_then(|b| u32::try_from(b / 1000).ok()),
            samplerate: audio.and_then(|a| a.sample_rate),
            channels: audio.and_then(|a| a.channels),
            codec: audio
                .and_then(|a| a.codec.clone())
                .or_else(|| self.container.clone())
                .map(|c| c.to_ascii_uppercase()),
            file_size: source.and_then(|s| s.size),
            date_added: self.date_created.as_deref().and_then(parse_time),
            play_count: user.map_or(0, |u| u.play_count),
            last_played: user
                .and_then(|u| u.last_played_date.as_deref())
                .and_then(parse_time),
            artwork: self.artwork_item(),
            ..Track::default()
        };
        t.replaygain.track_gain = self.normalization_gain;
        t
    }
}

/// Unix seconds of an ISO 8601 time as Jellyfin writes it (`2024-05-01T12:34:56.1234567Z`,
/// optionally with a `+hh:mm` offset).
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b).and_then(|v| v.parse::<i64>().ok());
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = if s.len() >= 19 {
        (num(11, 13)?, num(14, 16)?, num(17, 19)?)
    } else {
        (0, 0, 0)
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut secs = days * 86_400 + h * 3600 + mi * 60 + sec;
    // A numeric offset after the seconds (and fraction).
    if let Some(tail) = s.get(19..) {
        let tail = tail.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
        if let Some(sign) = tail.chars().next().filter(|c| *c == '+' || *c == '-') {
            let oh = tail
                .get(1..3)
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);
            let om = tail
                .get(4..6)
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);
            let off = oh * 3600 + om * 60;
            secs += if sign == '+' { -off } else { off };
        }
    }
    Some(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_time("2024-05-01T12:34:56.1234567Z"),
            Some(1_714_566_896)
        );
        assert_eq!(parse_time("2024-05-01T14:34:56+02:00"), Some(1_714_566_896));
        assert_eq!(parse_time("2000-02-29"), Some(951_782_400));
        assert_eq!(parse_time("garbage"), None);
    }

    #[test]
    fn track_from_item() {
        let item: Item = serde_json::from_str(
            r#"{"Id":"abc","Name":"Song","Type":"Audio","RunTimeTicks":1835000000,
                "ProductionYear":1999,"IndexNumber":3,"ParentIndexNumber":1,
                "Artists":["A","B"],"Album":"Al","AlbumId":"alb","AlbumArtist":"A",
                "Genres":["Rock"],"Container":"flac","DateCreated":"1970-01-02T00:00:00Z",
                "UserData":{"PlayCount":4,"LastPlayedDate":"1970-01-01T00:01:00Z"},
                "ImageTags":{},"AlbumPrimaryImageTag":"t","NormalizationGain":-6.5,
                "MediaSources":[{"Size":1000,"Bitrate":900000,"MediaStreams":[
                    {"Type":"Audio","Codec":"flac","SampleRate":44100,"Channels":2}]}]}"#,
        )
        .unwrap();
        let t = item.to_track("jf");
        assert_eq!(t.id, TrackId::new("jf", "abc"));
        assert_eq!(t.location, "library://jf/abc");
        assert_eq!(t.title(), Some("Song"));
        assert_eq!(t.artist(), Some("A; B"));
        assert_eq!(t.year(), Some(1999));
        assert_eq!(t.track_number(), Some(3));
        assert_eq!(t.disc_number(), Some(1));
        assert_eq!(t.length_ms, 183_500);
        assert_eq!(t.bitrate, Some(900));
        assert_eq!(t.samplerate, Some(44100));
        assert_eq!(t.codec.as_deref(), Some("FLAC"));
        assert_eq!(t.date_added, Some(86_400));
        assert_eq!(t.play_count, 4);
        assert_eq!(t.last_played, Some(60));
        assert_eq!(t.artwork.as_deref(), Some("alb"));
        assert_eq!(t.replaygain.track_gain, Some(-6.5));

        let bare: Item =
            serde_json::from_str(r#"{"Id":"x","Artists":null,"IndexNumber":-1}"#).unwrap();
        let t = bare.to_track("jf");
        assert!(t.tags.is_empty());
        assert_eq!(t.artwork, None);
    }

    #[test]
    fn videos_from_items() {
        let ep: Item = serde_json::from_str(
            r#"{"Id":"e1","Name":"Pilot","Type":"Episode","RunTimeTicks":26000000000,
                "SeriesName":"Show","SeriesId":"s1","SeasonName":"Season 1",
                "IndexNumber":1,"ParentIndexNumber":1,"ProductionYear":2010,
                "SeriesPrimaryImageTag":"x","ImageTags":{},
                "MediaSources":[{"Bitrate":4000000,"MediaStreams":[
                    {"Type":"Video","Codec":"h264"},
                    {"Type":"Audio","Codec":"ac3","SampleRate":48000,"Channels":6}]}]}"#,
        )
        .unwrap();
        let t = ep.to_track("jf");
        assert_eq!(t.id, TrackId::new("jf", "video:e1"));
        assert_eq!(t.location, "library://jf/video:e1");
        assert_eq!(t.title(), Some("Pilot"));
        assert_eq!(t.artist(), Some("Show"));
        assert_eq!(t.albumartist(), Some("Show"));
        assert_eq!(t.album(), Some("Season 1"));
        assert_eq!((t.disc_number(), t.track_number()), (Some(1), Some(1)));
        assert_eq!(t.length_ms, 2_600_000);
        assert_eq!(t.codec.as_deref(), Some("AC3"));
        assert_eq!(t.channels, Some(6));
        assert_eq!(t.artwork.as_deref(), Some("s1"));

        let movie: Item = serde_json::from_str(
            r#"{"Id":"m1","Name":"Film","Type":"Movie","ProductionYear":1999,
                "Genres":["Drama"],"Container":"mkv","ImageTags":{"Primary":"p"}}"#,
        )
        .unwrap();
        let t = movie.to_track("jf");
        assert_eq!(t.id.id, "video:m1");
        assert_eq!(t.artist(), None);
        assert_eq!(t.genre(), Some("Drama"));
        assert_eq!(t.codec.as_deref(), Some("MKV"));
        assert_eq!(t.artwork.as_deref(), Some("m1"));
        assert_eq!(item_of("video:m1"), ("m1", true));
        assert_eq!(item_of("a1"), ("a1", false));
    }
}
