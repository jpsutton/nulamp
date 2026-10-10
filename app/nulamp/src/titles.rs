//! Background reading of tags for playlist titles and lengths (04 §10, 10 §5).

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, channel};

use nulamp_core::{Engine, EntryId};
use nulamp_tf::FieldSource;

/// The result of reading one entry.
struct TitleResult {
    id: EntryId,
    title: Option<String>,
    length_ms: Option<u64>,
}

pub struct TitleReader {
    tx: Sender<(EntryId, String)>,
    rx: Receiver<TitleResult>,
    pending: HashSet<EntryId>,
    /// Show `_` as space; show `%20` as space (10 §5.4).
    pub fix_underscore: bool,
    pub fix_percent20: bool,
}

/// Tags of files the tag library cannot read (ASF/WMA, TTA, …), through the FFmpeg decoder;
/// registered with [`nulamp_meta::set_external_reader`].
pub fn external_tags(
    path: &std::path::Path,
) -> Option<(Vec<(String, String)>, nulamp_meta::TechInfo)> {
    let tags = nulamp_decoders::ffmpeg_tags(path.to_str()?)?;
    let info = &tags.info;
    let mime = match nulamp_decoders::extension_of(&path.to_string_lossy()).as_deref() {
        Some("wma" | "asf") => "audio/x-ms-wma",
        Some("wv") => "audio/x-wavpack",
        Some("ape") => "audio/x-ape",
        Some("mpc" | "mp+" | "mpp") => "audio/x-musepack",
        Some("tta") => "audio/x-tta",
        _ => "",
    };
    let tech = nulamp_meta::TechInfo {
        length_ms: info.length_ms.unwrap_or(0),
        bitrate: tags.kbps,
        sample_rate: Some(info.sample_rate).filter(|r| *r > 0),
        channels: u8::try_from(info.channels).ok().filter(|c| *c > 0),
        bits_per_sample: u8::try_from(info.bits).ok().filter(|b| *b > 0),
        lossless: info.lossless,
        family: info.codec.clone(),
        mime: mime.to_string(),
        mode: None,
        file_size: std::fs::metadata(path).ok().map(|m| m.len()),
    };
    Some((tags.fields, tech))
}

/// The title for an item from its tags (10 §5.1): title formatting, else `Artist - Title`,
/// else `None` (the caller keeps the file-name title).
pub fn title_from_tags(
    meta: &nulamp_meta::Metadata,
    format: &nulamp_tf::Formatter,
) -> Option<String> {
    let has_tags = meta.get("title").is_some() || meta.get("artist").is_some();
    if has_tags {
        let t = format.format(meta);
        if !t.trim().is_empty() {
            return Some(t);
        }
    }
    match (meta.field("artist"), meta.field("title")) {
        (Some(a), Some(t)) => Some(format!("{a} - {t}")),
        (None, Some(t)) => Some(t),
        _ => None,
    }
}

impl TitleReader {
    pub fn new(title_format: &str) -> Self {
        let (tx, req_rx) = channel::<(EntryId, String)>();
        let (res_tx, rx) = channel();
        let format = title_format.to_string();
        std::thread::Builder::new()
            .name("nulamp-titles".into())
            .spawn(move || {
                let formatter = nulamp_tf::Formatter::new(&format);
                while let Ok((id, location)) = req_rx.recv() {
                    let r = match nulamp_meta::read(&location) {
                        Ok(meta) => TitleResult {
                            id,
                            title: title_from_tags(&meta, &formatter),
                            length_ms: meta
                                .get("length")
                                .and_then(|l| l.parse().ok())
                                .filter(|l: &u64| *l > 0),
                        },
                        Err(_) => TitleResult {
                            id,
                            title: None,
                            length_ms: None,
                        },
                    };
                    if res_tx.send(r).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn title reader");
        Self {
            tx,
            rx,
            pending: HashSet::new(),
            fix_underscore: true,
            fix_percent20: true,
        }
    }

    /// Requests a (re-)read of one entry.
    pub fn request(&mut self, id: EntryId, location: &str) {
        if nulamp_core::playlist::is_url(location) || !self.pending.insert(id) {
            return;
        }
        let _ = self.tx.send((id, location.to_string()));
    }

    /// Applies finished reads and queues unread entries (background mode, 04 §10).
    pub fn tick(&mut self, engine: &mut Engine) -> bool {
        let mut changed = false;
        while let Ok(r) = self.rx.try_recv() {
            self.pending.remove(&r.id);
            if let Some(e) = engine.playlist.by_id_mut(r.id) {
                if let Some(t) = r.title {
                    e.title = nulamp_core::playlist::clean_title(
                        &t,
                        self.fix_underscore,
                        self.fix_percent20,
                    );
                }
                if r.length_ms.is_some() {
                    e.length_ms = r.length_ms;
                }
                e.metadata_read = true;
                changed = true;
            }
        }
        if self.pending.len() < 16 {
            let todo: Vec<(EntryId, String)> = engine
                .playlist
                .entries()
                .iter()
                .filter(|e| {
                    !e.metadata_read
                        && !self.pending.contains(&e.id)
                        && !nulamp_core::playlist::is_url(&e.location)
                })
                .take(32)
                .map(|e| (e.id, e.location.clone()))
                .collect();
            for (id, loc) in todo {
                self.request(id, &loc);
            }
        }
        changed
    }
}

/// Reads the ReplayGain values of a file (07 §5.3).
pub fn replaygain_tags(location: &str) -> nulamp_eq::ReplayGainTags {
    let mut tags = nulamp_eq::ReplayGainTags::default();
    if let Ok(meta) = nulamp_meta::read(location) {
        for k in [
            "replaygain_track_gain",
            "replaygain_track_peak",
            "replaygain_album_gain",
            "replaygain_album_peak",
        ] {
            if let Some(v) = meta.get(k) {
                tags.set_tag(k, &v);
            }
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WMA file's ASF attributes reach `nulamp_meta::read` through the FFmpeg reader.
    #[test]
    fn wma_tags_through_ffmpeg() {
        let path = std::env::temp_dir().join(format!("nulamp-titles-{}.wma", std::process::id()));
        let made = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "sine=duration=1", "-c:a", "wmav2"])
            .args(["-metadata", "title=Tone", "-metadata", "album_artist=Band"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !made || nulamp_decoders::ffmpeg_tags(path.to_str().unwrap()).is_none() {
            eprintln!("skipping: no ffmpeg tool or FFmpeg decoder");
            let _ = std::fs::remove_file(&path);
            return;
        }
        nulamp_meta::set_external_reader(external_tags);
        let meta = nulamp_meta::read(&path).unwrap();
        assert_eq!(meta.get("title").as_deref(), Some("Tone"));
        assert_eq!(meta.get("albumartist").as_deref(), Some("Band"));
        assert_eq!(meta.get("mime").as_deref(), Some("audio/x-ms-wma"));
        assert!(meta.get("family").unwrap().contains("Windows Media Audio"));
        assert!(meta.tech().length_ms.abs_diff(1000) < 100);
        assert!(meta.supported_fields().is_empty());
        assert!(matches!(
            nulamp_meta::write(&path, &[("title".into(), Some("X".into()))]),
            Err(nulamp_meta::Error::UnsupportedFormat)
        ));
        std::fs::remove_file(&path).unwrap();
    }
}
