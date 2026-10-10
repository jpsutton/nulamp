//! Reading and writing real files (spec 12 §2.5).
//!
//! MP3 and WAV files are generated here; FLAC, Ogg, Opus, MP4 and WavPack files come from
//! `ffmpeg`, and those tests are skipped when it (or the encoder) is missing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lofty::TextEncoding;
use lofty::ape::{ApeItem, ApeTag};
use lofty::config::WriteOptions;
use lofty::file::AudioFile;
use lofty::id3::v2::{Frame, FrameId, Id3v2Tag, PopularimeterFrame, TextInformationFrame};
use lofty::iff::wav::RiffInfoList;
use lofty::mpeg::MpegFile;
use lofty::tag::{ItemValue, TagExt};
use nulamp_meta::{
    Cover, Error, FIELDS, Id3v1, Options, read, read_cover, read_with, supported_fields, write,
    write_cover, write_with,
};
use tempfile::TempDir;

/// MPEG-1 Layer III frames, 128 kbps, 44.1 kHz, stereo, silent.
fn mp3_audio(frames: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for f in 0..frames {
        let mut frame = vec![0u8; 417];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        for (i, b) in frame.iter_mut().enumerate().skip(36) {
            *b = ((i + f) % 251) as u8;
        }
        out.extend(frame);
    }
    out
}

/// 16-bit stereo PCM WAV file with `samples` frames of a ramp.
fn wav_bytes(samples: u32) -> Vec<u8> {
    let data_len = samples * 4;
    let mut out = b"RIFF".to_vec();
    out.extend((36 + data_len).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(44100u32.to_le_bytes());
    out.extend((44100u32 * 4).to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data_len.to_le_bytes());
    out.extend((0..data_len).map(|i| (i % 200) as u8));
    out
}

fn mp3_file(dir: &TempDir, name: &str) -> (PathBuf, Vec<u8>) {
    let audio = mp3_audio(40);
    let path = dir.path().join(name);
    fs::write(&path, &audio).unwrap();
    (path, audio)
}

/// Encodes one second of a sine tone with `ffmpeg`; `None` when that is not possible.
fn ffmpeg(dir: &TempDir, name: &str, codec: &[&str]) -> Option<PathBuf> {
    let out = dir.path().join(name);
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
        ])
        .args(codec)
        .args(["-map_metadata", "-1", "-fflags", "+bitexact"])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        eprintln!("skipping: ffmpeg could not create {name}");
        return None;
    }
    Some(out)
}

/// True when `ffmpeg` decodes the file without error; `None` when ffmpeg is missing.
fn decodes(path: &Path) -> Option<bool> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .ok()?;
    Some(out.status.success() && out.stderr.is_empty())
}

fn sample(field: &str) -> String {
    match field {
        "year" => "1999",
        "genre" => "Rock",
        "track" => "3/12",
        "disc" => "1/2",
        "bpm" => "120",
        "rating" => "4",
        "compilation" => "1",
        "url" => "https://example.com/a",
        "isrc" => "USRC17607839",
        "replaygain_track_gain" => "-6.50 dB",
        "replaygain_track_peak" => "0.988000",
        "replaygain_album_gain" => "-7.25 dB",
        "replaygain_album_peak" => "1.000000",
        "r128_track_gain" => "-512",
        "r128_album_gain" => "256",
        f => return format!("{f} välue Ω"),
    }
    .to_string()
}

fn set_all(fields: &[&str]) -> Vec<(String, Option<String>)> {
    fields
        .iter()
        .map(|f| (f.to_string(), Some(sample(f))))
        .collect()
}

fn clear_all(fields: &[&str]) -> Vec<(String, Option<String>)> {
    fields.iter().map(|f| (f.to_string(), None)).collect()
}

/// Writes every supported field, reads it back, deletes every field and reads again.
fn round_trip_all(path: &Path) {
    let fields = supported_fields(path);
    assert!(!fields.is_empty(), "{path:?}");
    write(path, &set_all(&fields)).unwrap();
    let meta = read(path).unwrap();
    for f in &fields {
        assert_eq!(meta.get(f), Some(sample(f)), "{f} in {path:?}");
    }
    assert_eq!(meta.fields().len(), fields.len(), "{:?}", meta.fields());
    write(path, &clear_all(&fields)).unwrap();
    let meta = read(path).unwrap();
    assert!(meta.fields().is_empty(), "{:?}", meta.fields());
    if let Some(ok) = decodes(path) {
        assert!(ok, "{path:?} no longer decodes");
    }
}

/// Size of the ID3v2 tag at the start of `bytes`.
fn id3v2_len(bytes: &[u8]) -> usize {
    if &bytes[..3] != b"ID3" {
        return 0;
    }
    10 + bytes[6..10]
        .iter()
        .fold(0usize, |acc, b| (acc << 7) | usize::from(*b))
}

fn read_mpeg(path: &Path) -> MpegFile {
    let mut file = fs::File::open(path).unwrap();
    MpegFile::read_from(&mut file, lofty::config::ParseOptions::new()).unwrap()
}

#[test]
fn mp3_every_field_round_trips() {
    let dir = TempDir::new().unwrap();
    let (path, audio) = mp3_file(&dir, "a.mp3");
    assert_eq!(supported_fields(&path), FIELDS);
    write(&path, &set_all(FIELDS)).unwrap();

    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], b"ID3\x03", "new tags are ID3v2.3");
    let front = id3v2_len(&bytes);
    assert_eq!(&bytes[front..front + audio.len()], &audio[..]);
    let id3v1 = Id3v1::parse(&bytes[bytes.len() - 128..]).expect("ID3v1 created");
    assert_eq!(
        id3v1.title,
        "title välue Ω"
            .chars()
            .take(30)
            .collect::<String>()
            .trim_end()
    );
    assert_eq!(id3v1.year, "1999");
    assert_eq!(id3v1.track, Some(3));
    assert_eq!(id3v1.genre, 17);

    let meta = read(&path).unwrap();
    for f in FIELDS {
        assert_eq!(meta.get(f), Some(sample(f)), "{f}");
    }

    write(&path, &clear_all(FIELDS)).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        audio,
        "deleting every field removes both tags"
    );
}

#[test]
fn mp3_keeps_id3v24_and_honours_always_v24() {
    let dir = TempDir::new().unwrap();
    let (path, _) = mp3_file(&dir, "a.mp3");
    let v24 = Options {
        id3v2_always_v24: true,
        ..Options::default()
    };
    write_with(&path, &set_all(FIELDS), &v24).unwrap();
    assert_eq!(fs::read(&path).unwrap()[3], 4);
    write(&path, &[("title".into(), Some("New".into()))]).unwrap();
    assert_eq!(fs::read(&path).unwrap()[3], 4, "existing 2.4 tag stays 2.4");
    let meta = read(&path).unwrap();
    assert_eq!(meta.get("title").as_deref(), Some("New"));
    for f in FIELDS.iter().filter(|f| **f != "title") {
        assert_eq!(meta.get(f), Some(sample(f)), "{f}");
    }
    let tag = read_mpeg(&path).id3v2().unwrap().clone();
    assert!(tag.get(&FrameId::Valid("TMOO".into())).is_some());
}

#[test]
fn mp3_smaller_tag_reuses_padding_and_bigger_tag_rewrites() {
    let dir = TempDir::new().unwrap();
    let (path, audio) = mp3_file(&dir, "a.mp3");
    write(&path, &[("comment".into(), Some("x".repeat(2000)))]).unwrap();
    let first = fs::read(&path).unwrap();
    let front = id3v2_len(&first);
    assert!(front > 2000 + 10);

    write(
        &path,
        &[
            ("comment".into(), Some("short".into())),
            ("title".into(), Some("T".into())),
        ],
    )
    .unwrap();
    let second = fs::read(&path).unwrap();
    assert_eq!(second.len(), first.len(), "smaller tag written in place");
    assert_eq!(id3v2_len(&second), front);
    assert_eq!(&second[front..front + audio.len()], &audio[..]);
    assert_eq!(
        read(&path).unwrap().get("comment").as_deref(),
        Some("short")
    );

    write(&path, &[("comment".into(), Some("y".repeat(front * 2)))]).unwrap();
    let third = fs::read(&path).unwrap();
    let new_front = id3v2_len(&third);
    assert!(new_front > front);
    assert_eq!(&third[new_front..new_front + audio.len()], &audio[..]);
    let meta = read(&path).unwrap();
    assert_eq!(meta.get("comment"), Some("y".repeat(front * 2)));
    assert_eq!(meta.get("title").as_deref(), Some("T"));
    if let Some(ok) = decodes(&path) {
        assert!(ok);
    }
    let leftovers: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temporary files left");
}

#[test]
fn popm_rating_both_directions() {
    let dir = TempDir::new().unwrap();
    let (path, _) = mp3_file(&dir, "a.mp3");
    for (stars, byte) in [(1, 1), (2, 64), (3, 128), (4, 196), (5, 255)] {
        write(&path, &[("rating".into(), Some(stars.to_string()))]).unwrap();
        let tag = read_mpeg(&path).id3v2().unwrap().clone();
        let popm: Vec<_> = tag
            .into_iter()
            .filter_map(|f| match f {
                Frame::Popularimeter(p) => Some(p.rating),
                _ => None,
            })
            .collect();
        assert_eq!(popm, [byte]);
    }
    write(&path, &[("rating".into(), None)]).unwrap();
    assert!(
        read_mpeg(&path)
            .id3v2()
            .is_none_or(|t| t.into_iter().all(|f| !matches!(f, Frame::Popularimeter(_))))
    );

    for (byte, stars) in [
        (0, None),
        (20, Some("1")),
        (50, Some("2")),
        (100, Some("3")),
        (200, Some("4")),
        (230, Some("5")),
    ] {
        let mut tag = Id3v2Tag::new();
        tag.insert(Frame::Popularimeter(PopularimeterFrame::new(
            "someone@example.com",
            byte,
            3,
        )));
        tag.save_to_path(&path, WriteOptions::default()).unwrap();
        assert_eq!(
            read(&path).unwrap().get("rating").as_deref(),
            stars,
            "{byte}"
        );
    }

    let other = Options {
        rating_email: "me@example.com".into(),
        ..Options::default()
    };
    assert_eq!(read_with(&path, &other).unwrap().get("rating"), None);
    write_with(&path, &[("rating".into(), Some("2".into()))], &other).unwrap();
    assert_eq!(
        read_with(&path, &other).unwrap().get("rating").as_deref(),
        Some("2")
    );
    let tag = read_mpeg(&path).id3v2().unwrap().clone();
    assert_eq!(
        tag.into_iter()
            .filter(|f| matches!(f, Frame::Popularimeter(_)))
            .count(),
        2
    );
}

#[test]
fn tcon_forms_read_as_rock() {
    let dir = TempDir::new().unwrap();
    let (path, _) = mp3_file(&dir, "a.mp3");
    for value in ["(17)", "17", "(17)Rock", "Rock"] {
        let mut tag = Id3v2Tag::new();
        tag.insert(Frame::Text(TextInformationFrame::new(
            FrameId::Valid("TCON".into()),
            TextEncoding::Latin1,
            value.to_string(),
        )));
        tag.save_to_path(&path, WriteOptions::default()).unwrap();
        assert_eq!(
            read(&path).unwrap().get("genre").as_deref(),
            Some("Rock"),
            "{value}"
        );
    }
}

#[test]
fn id3v1_track_byte_and_genres() {
    let dir = TempDir::new().unwrap();
    let (path, audio) = mp3_file(&dir, "a.mp3");
    let tag = Id3v1 {
        title: "Café".into(),
        artist: "Ωmega".into(),
        track: Some(9),
        genre: 255,
        ..Id3v1::default()
    };
    fs::write(&path, [audio.clone(), tag.to_bytes().to_vec()].concat()).unwrap();
    let meta = read(&path).unwrap();
    assert_eq!(meta.get("title").as_deref(), Some("Café"));
    assert_eq!(meta.get("artist").as_deref(), Some("Ωmega"));
    assert_eq!(meta.get("track").as_deref(), Some("9"));
    assert_eq!(meta.get("genre"), None, "genre 255 is no genre");

    write(&path, &[("genre".into(), Some("Psychobilly".into()))]).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes[bytes.len() - 1], 255, "unknown genre stored as 255");
    assert_eq!(
        read(&path).unwrap().get("genre").as_deref(),
        Some("Psychobilly")
    );

    write(
        &path,
        &[
            ("genre".into(), Some("hard rock".into())),
            ("track".into(), Some("7/10".into())),
        ],
    )
    .unwrap();
    let bytes = fs::read(&path).unwrap();
    let v1 = Id3v1::parse(&bytes[bytes.len() - 128..]).unwrap();
    assert_eq!((v1.genre, v1.track), (79, Some(7)));
    assert_eq!(bytes[bytes.len() - 3], 0);
    assert_eq!(v1.title, "Café");

    let no_v1 = Options {
        read_id3v1: false,
        ..Options::default()
    };
    write(&path, &[("album".into(), Some("A".into()))]).unwrap();
    let v1 = Id3v1::parse(&fs::read(&path).unwrap()[bytes.len() - 128..]).unwrap();
    assert_eq!(v1.album, "A");
    let mut file = fs::File::open(&path).unwrap();
    assert_eq!(Id3v1::read_from(&mut file).unwrap().unwrap().album, "A");
    assert!(
        read_with(&path, &no_v1).unwrap().get("title").is_some(),
        "ID3v2 has the title too"
    );
}

#[test]
fn ape_tagged_mp3_and_read_order() {
    let dir = TempDir::new().unwrap();
    let (path, audio) = mp3_file(&dir, "a.mp3");
    let mut ape = ApeTag::new();
    for (k, v) in [("TITLE", "Ape title"), ("Album", "Ape album")] {
        ape.insert(ApeItem::new(k.into(), ItemValue::Text(v.into())).unwrap());
    }
    ape.save_to_path(&path, WriteOptions::default()).unwrap();
    let v1 = Id3v1 {
        album: "V1 album".into(),
        artist: "V1 artist".into(),
        title: "V1 title".into(),
        ..Id3v1::default()
    };
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend(v1.to_bytes());
    fs::write(&path, &bytes).unwrap();

    let meta = read(&path).unwrap();
    assert_eq!(meta.get("title").as_deref(), Some("Ape title"));
    assert_eq!(meta.get("album").as_deref(), Some("Ape album"));
    assert_eq!(meta.get("artist").as_deref(), Some("V1 artist"));

    write(&path, &[("title".into(), Some("V2 title".into()))]).unwrap();
    let meta = read(&path).unwrap();
    assert_eq!(meta.get("title").as_deref(), Some("V2 title"));
    let no_v2 = Options {
        read_id3v2: false,
        ..Options::default()
    };
    assert_eq!(
        read_with(&path, &no_v2).unwrap().get("title").as_deref(),
        Some("V2 title"),
        "APE updated too"
    );

    let ape_fields: Vec<&str> = FIELDS
        .iter()
        .copied()
        .filter(|f| {
            ![
                "key",
                "mood",
                "remixing",
                "originalartist",
                "encoder",
                "tool",
                "compilation",
                "url",
            ]
            .contains(f)
        })
        .collect();
    write(&path, &set_all(FIELDS)).unwrap();
    let no_v2_v1 = Options {
        read_id3v2: false,
        read_id3v1: false,
        ..Options::default()
    };
    let meta = read_with(&path, &no_v2_v1).unwrap();
    for f in &ape_fields {
        assert_eq!(meta.get(f), Some(sample(f)), "APE {f}");
    }
    assert_eq!(meta.get("mood"), None);

    let bytes = fs::read(&path).unwrap();
    let footer = bytes.len() - 128 - 32;
    assert_eq!(&bytes[footer..footer + 8], b"APETAGEX");
    let flags = u32::from_le_bytes(bytes[footer + 20..footer + 24].try_into().unwrap());
    assert_eq!(
        flags & 0xE000_0000,
        0x8000_0000,
        "header present, footer present, this is the footer"
    );
    let size = u32::from_le_bytes(bytes[footer + 12..footer + 16].try_into().unwrap()) as usize;
    let header = footer + 32 - size - 32;
    assert_eq!(&bytes[header..header + 8], b"APETAGEX");
    let front = id3v2_len(&bytes);
    assert_eq!(&bytes[front..front + audio.len()], &audio[..]);

    write(&path, &[("album".into(), Some("a".repeat(500)))]).unwrap();
    let grown = fs::read(&path).unwrap();
    assert_eq!(id3v2_len(&grown), front, "ID3v2 rewritten in its padding");
    assert_eq!(grown.len(), bytes.len() + 500 - sample("album").len());
    assert_eq!(&grown[front..front + audio.len()], &audio[..]);
    let meta = read_with(&path, &no_v2_v1).unwrap();
    assert_eq!(meta.get("album"), Some("a".repeat(500)));
    assert_eq!(meta.get("title"), Some(sample("title")));

    write(&path, &clear_all(FIELDS)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), audio);
}

#[test]
fn wav_id3_chunk_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a.wav");
    fs::write(&path, wav_bytes(4410)).unwrap();
    round_trip_all(&path);
    let meta = read(&path).unwrap();
    assert_eq!(meta.get("samplerate").as_deref(), Some("44100"));
    assert_eq!(meta.get("bitspersample").as_deref(), Some("16"));
    assert_eq!(meta.get("lossless").as_deref(), Some("1"));

    let mut info = RiffInfoList::new();
    info.insert("INAM".into(), "Riff title".into());
    info.insert("IART".into(), "Riff artist".into());
    info.save_to_path(&path, WriteOptions::default()).unwrap();
    assert_eq!(
        read(&path).unwrap().get("title").as_deref(),
        Some("Riff title")
    );
    write(&path, &[("title".into(), Some("New".into()))]).unwrap();
    let meta = read(&path).unwrap();
    assert_eq!(meta.get("title").as_deref(), Some("New"));
    assert_eq!(meta.get("artist").as_deref(), Some("Riff artist"));
    write(&path, &[("artist".into(), None)]).unwrap();
    assert_eq!(
        read(&path).unwrap().get("artist"),
        None,
        "RIFF INFO updated too"
    );
}

#[test]
fn flac_round_trip() {
    let dir = TempDir::new().unwrap();
    let Some(path) = ffmpeg(&dir, "a.flac", &["-c:a", "flac"]) else {
        return;
    };
    round_trip_all(&path);
    assert!(!supported_fields(&path).contains(&"url"));
}

#[test]
fn ogg_vorbis_round_trip() {
    let dir = TempDir::new().unwrap();
    let Some(path) = ffmpeg(&dir, "a.ogg", &["-c:a", "libvorbis"]) else {
        return;
    };
    round_trip_all(&path);
    assert_eq!(
        read(&path).unwrap().get("family").as_deref(),
        Some("Ogg Vorbis")
    );
}

#[test]
fn opus_r128_fields() {
    let dir = TempDir::new().unwrap();
    let Some(path) = ffmpeg(&dir, "a.opus", &["-c:a", "libopus"]) else {
        return;
    };
    assert!(supported_fields(&path).contains(&"r128_track_gain"));
    round_trip_all(&path);
}

#[test]
fn mp4_round_trip() {
    let dir = TempDir::new().unwrap();
    let Some(path) = ffmpeg(&dir, "a.m4a", &["-c:a", "aac"]) else {
        return;
    };
    round_trip_all(&path);
    write(&path, &[("title".into(), Some("Again".into()))]).unwrap();
    assert_eq!(read(&path).unwrap().get("title").as_deref(), Some("Again"));
    write(&path, &[("title".into(), None)]).unwrap();
    assert_eq!(read(&path).unwrap().get("title"), None);
    let fields = supported_fields(&path);
    assert!(!fields.contains(&"conductor"));
    let err = write(&path, &[("conductor".into(), Some("x".into()))]).unwrap_err();
    assert!(matches!(err, Error::UnsupportedField(f) if f == "conductor"));
    write(&path, &[("conductor".into(), None)]).unwrap();
    assert_eq!(
        read(&path).unwrap().get("family").as_deref(),
        Some("MPEG-4 AAC")
    );
}

#[test]
fn wavpack_ape_round_trip() {
    let dir = TempDir::new().unwrap();
    let Some(path) = ffmpeg(&dir, "a.wv", &["-c:a", "wavpack"]) else {
        return;
    };
    round_trip_all(&path);
    assert!(!supported_fields(&path).contains(&"mood"));
}

#[test]
fn covers() {
    let dir = TempDir::new().unwrap();
    let png = Cover {
        mime: "image/png".into(),
        data: b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x02\0\0\0\x90wS\xde".to_vec(),
    };
    let (mp3, _) = mp3_file(&dir, "a.mp3");
    let mut paths = vec![mp3];
    paths.extend(ffmpeg(&dir, "a.flac", &["-c:a", "flac"]));
    paths.extend(ffmpeg(&dir, "a.m4a", &["-c:a", "aac"]));
    paths.extend(ffmpeg(&dir, "a.ogg", &["-c:a", "libvorbis"]));
    paths.extend(ffmpeg(&dir, "a.wv", &["-c:a", "wavpack"]));
    for path in &paths {
        assert_eq!(read_cover(path).unwrap(), None);
        write(path, &[("title".into(), Some("T".into()))]).unwrap();
        write_cover(path, Some(&png)).unwrap();
        assert_eq!(read_cover(path).unwrap().as_ref(), Some(&png), "{path:?}");
        write(path, &[("title".into(), Some("U".into()))]).unwrap();
        assert_eq!(
            read_cover(path).unwrap().as_ref(),
            Some(&png),
            "{path:?} cover kept by a field write"
        );
        write_cover(path, None).unwrap();
        assert_eq!(read_cover(path).unwrap(), None, "{path:?}");
        assert_eq!(read(path).unwrap().get("title").as_deref(), Some("U"));
    }
}

#[test]
fn technical_fields_and_title_formatting() {
    let dir = TempDir::new().unwrap();
    let (path, _) = mp3_file(&dir, "song.mp3");
    write(
        &path,
        &[
            ("artist".into(), Some("A".into())),
            ("title".into(), Some("T".into())),
        ],
    )
    .unwrap();
    let meta = read(&path).unwrap();
    assert_eq!(meta.get("samplerate").as_deref(), Some("44100"));
    assert_eq!(meta.get("channels").as_deref(), Some("2"));
    assert_eq!(meta.get("bitrate").as_deref(), Some("128"));
    assert_eq!(meta.get("family").as_deref(), Some("MPEG-1 Layer 3"));
    assert_eq!(meta.get("mime").as_deref(), Some("audio/mpeg"));
    assert_eq!(meta.get("type").as_deref(), Some("0"));
    assert_eq!(meta.get("lossless").as_deref(), Some("0"));
    let length: u64 = meta.get("length").unwrap().parse().unwrap();
    assert!((1000..1100).contains(&length), "{length}");
    let info = meta.get("formatinformation").unwrap();
    assert!(
        info.starts_with("Format: MPEG-1 Layer 3\nBit rate: 128 kbps\n"),
        "{info}"
    );
    assert!(meta.supports("conductor") && meta.supports("length"));

    let title = nulamp_tf::format("%artist% - %title% <%length%> $filepart(%filename%)", &meta);
    assert_eq!(title, format!("A - T <{length}> song"));
    let title = nulamp_tf::format(
        nulamp_tf::DEFAULT_FORMAT,
        &nulamp_meta::Metadata::new().with_path("/x/y.mp3"),
    );
    assert_eq!(title, "y");
}

#[test]
fn invalid_writes_leave_file_untouched() {
    let dir = TempDir::new().unwrap();
    let (path, audio) = mp3_file(&dir, "a.mp3");
    let err = write(&path, &[("rating".into(), Some("9".into()))]).unwrap_err();
    assert!(matches!(err, Error::InvalidValue { .. }));
    let err = write(&path, &[("length".into(), Some("5".into()))]).unwrap_err();
    assert!(matches!(err, Error::UnsupportedField(_)));
    assert_eq!(fs::read(&path).unwrap(), audio);

    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&path, perms).unwrap();
    let err = write(&path, &[("title".into(), Some("x".into()))]).unwrap_err();
    assert!(matches!(err, Error::ReadOnly));

    let text = dir.path().join("notes.txt");
    fs::write(&text, "hello").unwrap();
    assert!(matches!(read(&text), Err(Error::UnsupportedFormat)));
    assert!(supported_fields(&text).is_empty());
}
