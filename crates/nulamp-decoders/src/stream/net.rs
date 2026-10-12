//! The network thread: connects, resolves playlist URLs, strips in-band metadata into the
//! stream fields and fills the ring buffer or the spool. Reconnects live streams (07 §8.7),
//! resumes dropped files and serves Range requests for seeks.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nulamp_playlist::{Format, Resolver, is_hls};

use super::http::{Client, OriginHeaders, Response, io_error};
use super::icy::{IcyParser, parse_metadata};
use super::shared::{Phase, Shared, State, prebuffer_percent};
use super::spool::{self, Spool};
use super::url::{Url, is_stream_location};
use super::{CredentialStore, StreamConfig, StreamError};

/// Most bytes read from a playlist response.
const MAX_PLAYLIST_BYTES: u64 = 1 << 20;
/// Most playlists followed to reach a stream.
const MAX_PLAYLIST_DEPTH: usize = 4;

/// What a location turned out to be.
pub(crate) enum Fetched {
    Media(Response),
    Playlist(Vec<String>),
}

/// GETs `url` and, when it is a playlist (by URL extension or content type, 07 §8.2), reads
/// and parses it. HLS playlists are [`StreamError::Hls`].
pub(crate) fn fetch(client: &Client, url: &Url, range_from: u64) -> Result<Fetched, StreamError> {
    let mut resp = client.get(url, range_from)?;
    let content_type = resp.content_type();
    let by_type = content_type.as_deref().and_then(Format::from_content_type);
    let audio_type = content_type
        .as_deref()
        .is_some_and(|t| t.starts_with("audio/") && by_type.is_none());
    let by_ext = Format::from_path(&url.to_string()).filter(|_| !audio_type);
    let Some(format) = by_type.or(by_ext) else {
        return Ok(Fetched::Media(resp));
    };
    let bytes = resp.read_all(MAX_PLAYLIST_BYTES)?;
    if is_hls(&bytes) {
        return Err(StreamError::Hls);
    }
    let format = match Format::sniff(&bytes) {
        Some(Format::M3u) if format == Format::M3u8 => format,
        Some(sniffed) => sniffed,
        None => format,
    };
    let base = resp.url.to_string();
    let entries = nulamp_playlist::parse(&bytes, format, &Resolver::plain(Some(&base)))
        .map_err(|e| StreamError::Playlist(e.to_string()))?;
    Ok(Fetched::Playlist(
        entries.into_iter().map(|e| e.location).collect(),
    ))
}

/// Follows playlists from `location` to the first stream response.
fn open_media(client: &Client, location: &str) -> Result<(Url, Response), StreamError> {
    let mut url = Url::parse(location)?;
    for _ in 0..=MAX_PLAYLIST_DEPTH {
        match fetch(client, &url, 0)? {
            Fetched::Media(resp) => return Ok((url, resp)),
            Fetched::Playlist(entries) => {
                let next = entries
                    .iter()
                    .find(|e| is_stream_location(e))
                    .ok_or_else(|| StreamError::Playlist("no stream entries".into()))?;
                url = Url::parse(next)?;
            }
        }
    }
    Err(StreamError::Playlist("playlists nested too deeply".into()))
}

/// Format hint (as an extension) for a content type.
pub(crate) fn hint_for(content_type: &str) -> Option<&'static str> {
    Some(match content_type {
        "audio/mpeg" | "audio/mp3" | "audio/mpeg3" | "audio/x-mpeg" | "audio/mpg" => "mp3",
        "audio/aac" | "audio/aacp" | "audio/x-aac" | "audio/vnd.dlna.adts" => "aac",
        "audio/ogg" | "application/ogg" | "audio/vorbis" | "audio/opus" => "ogg",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/aiff" | "audio/x-aiff" => "aiff",
        "audio/x-matroska" | "audio/matroska" => "mkv",
        "audio/webm" => "webm",
        "audio/x-caf" => "caf",
        _ => return None,
    })
}

/// Leading number of `icy-br` (some servers send `128,128`).
fn parse_kbps(v: &str) -> Option<u32> {
    let digits: String = v.trim().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|&k| k > 0)
}

/// `filename=` of a `Content-Disposition` header.
fn disposition_filename(v: &str) -> Option<String> {
    let lower = v.to_ascii_lowercase();
    let i = lower.find("filename=")?;
    let rest = v[i + 9..].trim();
    let name = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split(';').next().unwrap_or("").trim(),
    };
    (!name.is_empty()).then(|| name.to_string())
}

fn metaint(resp: &Response) -> usize {
    resp.header("icy-metaint")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Copies the response headers into the stream description (07 §8.4).
fn apply_headers(st: &mut State, resp: &Response, url: &Url, first: bool) {
    let h = |n: &str| {
        resp.header(n)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(String::from)
    };
    let f = &mut st.fields;
    f.streamname = h("icy-name").or(f.streamname.take());
    f.streamgenre = h("icy-genre").or(f.streamgenre.take());
    f.streamurl = h("icy-url").or(f.streamurl.take());
    f.bitrate = h("icy-br").and_then(|v| parse_kbps(&v)).or(f.bitrate);
    f.metaint = metaint(resp);
    if let Some(n) = h("icy-notice2") {
        let lower = n.to_ascii_lowercase();
        let end = lower.find("<br").unwrap_or(n.len());
        f.notice = Some(n[..end].trim().to_string());
    }
    if !first {
        return;
    }
    f.content_type = resp.content_type();
    f.location = url.to_string();
    if let Some(name) = h("content-disposition").and_then(|v| disposition_filename(&v)) {
        st.fallback_title = name;
    }
    st.hint = st
        .fields
        .content_type
        .as_deref()
        .and_then(hint_for)
        .or_else(|| match crate::extension_of(&url.to_string())?.as_str() {
            "mp3" | "mp2" | "mpga" => Some("mp3"),
            "aac" | "aacp" => Some("aac"),
            "ogg" | "oga" | "opus" => Some("ogg"),
            "flac" => Some("flac"),
            "aiff" | "aif" => Some("aiff"),
            "mka" | "mkv" => Some("mkv"),
            "webm" => Some("webm"),
            "caf" => Some("caf"),
            _ => None,
        });
    st.total_len = resp.total_length();
    let live = st.fields.metaint > 0;
    let ranged = st.total_len.is_some() && resp.accepts_ranges() && !live;
    let finite = !live && (st.total_len.is_some() || st.finite);
    // Very large files with Range support seek with requests rather than fill the disk, and so
    // do video files: FFmpeg reads their index at the end before playing (NET15).
    let too_big = ranged && (st.video || st.total_len.is_some_and(|t| t > spool::MAX_RANGED));
    if finite && st.spool_enabled && !too_big {
        st.spool = Spool::new()
            .inspect_err(|e| log::warn!("stream: no spool file: {e}"))
            .ok();
    }
    st.seekable = ranged || st.spool.is_some();
    st.headers_ready = true;
}

/// How a connection ended.
enum End {
    Eof,
    Error(StreamError),
    /// Aborted, or a seek started a new epoch.
    Stopped,
}

/// Moves body bytes into the ring until the connection ends.
fn pump(shared: &Shared, resp: &mut Response, epoch: u64, skip: u64) -> (u64, End) {
    let mut icy = IcyParser::new(metaint(resp));
    let mut buf = vec![0u8; 8192];
    let (mut audio, mut blocks) = (Vec::new(), Vec::new());
    let mut skip = skip;
    let mut total = 0u64;
    loop {
        let n = match resp.body.read(&mut buf) {
            Ok(0) => return (total, End::Eof),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let st = shared.lock();
                if st.abort || st.epoch != epoch {
                    return (total, End::Stopped);
                }
                return (total, End::Error(io_error(e)));
            }
        };
        audio.clear();
        blocks.clear();
        icy.feed(&buf[..n], &mut audio, &mut blocks);
        // A server that ignored the Range request: drop the bytes before the offset.
        let drop = (skip as usize).min(audio.len());
        skip -= drop as u64;
        let mut from = drop;
        for (at, block) in &blocks {
            let at = (*at).max(from);
            if !shared.push(&audio[from..at], epoch) {
                return (total, End::Stopped);
            }
            from = at;
            let mut st = shared.lock();
            if st.epoch == epoch {
                // Takes effect when the decoder reaches this byte.
                let offset = st.received();
                st.pending_meta.push_back((offset, parse_metadata(block)));
            }
        }
        if !shared.push(&audio[from..], epoch) {
            return (total, End::Stopped);
        }
        total += (audio.len() - drop) as u64;
    }
}

/// Waits up to `d`; false when aborted or a seek arrived.
fn pause(shared: &Shared, d: Duration) -> bool {
    let deadline = Instant::now() + d;
    let mut st = shared.lock();
    loop {
        if st.abort || st.restart.is_some() {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        st = shared.wait_timeout(st, deadline - now);
    }
}

/// No more bytes will arrive: let the buffered rest play.
fn finish(shared: &Shared, st: &mut State) {
    st.net_done = true;
    if matches!(st.phase, Phase::Connecting) {
        st.phase = Phase::Playing;
    }
    st.check_buffered();
    shared.cv.notify_all();
}

/// Reconnects a dropped live stream with increasing delays. `None` when the tries are used
/// up or the stream was closed.
fn reconnect(
    shared: &Shared,
    client: &Client,
    location: &str,
    config: &StreamConfig,
    tries: &mut u32,
) -> Option<(Response, u64, u64)> {
    while *tries < config.reconnect_tries {
        *tries += 1;
        {
            let mut st = shared.lock();
            if st.ahead() == 0 {
                st.phase = Phase::Connecting;
                shared.cv.notify_all();
            }
        }
        log::info!("stream: reconnect {tries}");
        if !pause(shared, config.reconnect_delay * *tries) {
            return None;
        }
        let epoch = shared.lock().epoch;
        match open_media(client, location) {
            Ok((url, r)) => {
                let mut st = shared.lock();
                apply_headers(&mut st, &r, &url, false);
                if matches!(st.phase, Phase::Connecting) {
                    let p = st.rebuffer_percent;
                    st.start_buffering(p);
                }
                shared.cv.notify_all();
                return Some((r, epoch, 0));
            }
            Err(e) => log::info!("stream: reconnect failed: {e}"),
        }
    }
    None
}

fn fail(shared: &Shared, e: StreamError) {
    log::warn!("stream: {e}");
    let mut st = shared.lock();
    st.phase = Phase::Failed(e);
    st.net_done = true;
    shared.cv.notify_all();
}

pub(crate) fn run(
    shared: Arc<Shared>,
    config: StreamConfig,
    credentials: Arc<CredentialStore>,
    location: String,
    headers: Vec<(String, String)>,
) {
    // Bound to the location as given: playlist entries and redirects elsewhere do not get them.
    let origin = Url::parse(&location)
        .ok()
        .and_then(|u| OriginHeaders::new(&u, &headers));
    let on_connect = |s: &TcpStream| {
        if let Ok(c) = s.try_clone() {
            let mut st = shared.lock();
            if st.abort {
                let _ = c.shutdown(std::net::Shutdown::Both);
            }
            st.socket = Some(c);
        }
    };
    let client = Client {
        config: &config,
        credentials: &credentials,
        on_connect: &on_connect,
        headers: origin.as_ref(),
    };
    let (url, first) = match open_media(&client, &location) {
        Ok(r) => r,
        Err(e) => return fail(&shared, e),
    };
    let epoch = {
        let mut st = shared.lock();
        apply_headers(&mut st, &first, &url, true);
        st.connect_url = url.to_string();
        let percent = prebuffer_percent(config.prebuffer_percent, st.fields.bitrate, true);
        st.start_buffering(percent);
        shared.cv.notify_all();
        st.epoch
    };
    let mut resp = Some((first, epoch, 0u64));
    let mut tries = 0u32;
    loop {
        if let Some((mut r, epoch, skip)) = resp.take() {
            let (got, end) = pump(&shared, &mut r, epoch, skip);
            drop(r);
            if got >= config.buffer_bytes as u64 {
                tries = 0;
            }
            if let End::Error(e) = &end {
                log::info!("stream connection lost: {e}");
            }
            if !matches!(end, End::Stopped) {
                let mut st = shared.lock();
                let received = st.received();
                if st.seekable {
                    // A spooled file of unknown length is cut short when its connection failed.
                    let short = match st.total_len {
                        Some(t) => received < t,
                        None => matches!(end, End::Error(_)),
                    };
                    if short && tries < config.reconnect_tries {
                        // Dropped in the middle of a file: continue where it stopped.
                        tries += 1;
                        st.restart.get_or_insert(received);
                    } else {
                        // Done, but a seek may still need a new connection.
                        finish(&shared, &mut st);
                    }
                } else if st.total_len.is_some() || !config.reconnect {
                    finish(&shared, &mut st);
                    return;
                } else {
                    drop(st);
                    match reconnect(&shared, &client, &location, &config, &mut tries) {
                        Some(r) => {
                            resp = Some(r);
                            continue;
                        }
                        None => {
                            finish(&shared, &mut shared.lock());
                            return;
                        }
                    }
                }
            }
        }
        // Wait for a seek (new byte offset) or the end.
        let (offset, epoch) = {
            let mut st = shared.lock();
            loop {
                if st.abort {
                    return;
                }
                if let Some(o) = st.restart.take() {
                    break (o, st.epoch);
                }
                st = shared.wait(st);
            }
        };
        match client.get(&url, offset) {
            Ok(r) => {
                let skip = if r.status == 206 { 0 } else { offset };
                resp = Some((r, epoch, skip));
            }
            Err(e) => {
                if shared.lock().abort {
                    return;
                }
                fail(&shared, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_values() {
        assert_eq!(parse_kbps("128,128"), Some(128));
        assert_eq!(parse_kbps(" 64"), Some(64));
        assert_eq!(parse_kbps("x"), None);
        assert_eq!(
            disposition_filename("attachment; filename=\"My Song.mp3\"").as_deref(),
            Some("My Song.mp3")
        );
        assert_eq!(
            disposition_filename("inline; FILENAME=a.ogg; x=1").as_deref(),
            Some("a.ogg")
        );
        assert_eq!(hint_for("audio/aacp"), Some("aac"));
        assert_eq!(hint_for("application/ogg"), Some("ogg"));
        assert_eq!(hint_for("audio/x-aiff"), Some("aiff"));
        assert_eq!(hint_for("audio/x-matroska"), Some("mkv"));
        assert_eq!(hint_for("text/html"), None);
    }
}
