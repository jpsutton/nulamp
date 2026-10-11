//! Network stream tests against a local server (12 §7). Nothing leaves 127.0.0.1.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::shared::{Shared, State};
use super::*;

/// A silent MPEG-1 Layer III frame: 128 kbps, 44.1 kHz, mono, no padding.
const FRAME_LEN: usize = 417;
const FRAME_SAMPLES: usize = 1152;

fn mp3(frames: usize) -> Vec<u8> {
    let mut f = vec![0u8; FRAME_LEN];
    f[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
    f.repeat(frames)
}

/// Inserts a metadata block after every `metaint` audio bytes: `first` at the first
/// interval, then empty blocks (no change).
fn with_icy(audio: &[u8], metaint: usize, first: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, chunk) in audio.chunks(metaint).enumerate() {
        out.extend_from_slice(chunk);
        if chunk.len() < metaint {
            break;
        }
        if i == 0 {
            let mut block = first.to_vec();
            block.resize(first.len().div_ceil(16) * 16, 0);
            out.push((block.len() / 16) as u8);
            out.extend_from_slice(&block);
        } else {
            out.push(0);
        }
    }
    out
}

#[derive(Clone)]
struct Req {
    path: String,
    headers: Vec<(String, String)>,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

struct Server {
    base: String,
    hits: Arc<AtomicUsize>,
    log: Arc<Mutex<Vec<Req>>>,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

fn read_req(s: &TcpStream) -> Option<Req> {
    let mut r = BufReader::new(s.try_clone().ok()?);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_string();
    let mut headers = Vec::new();
    loop {
        let mut l = String::new();
        if r.read_line(&mut l).ok()? == 0 || l.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Some(Req { path, headers })
}

/// Serves each connection on its own thread; the handler gets the connection index.
fn serve(handler: impl Fn(&Req, &mut TcpStream, usize) + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let hits = Arc::new(AtomicUsize::new(0));
    let log = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(handler);
    let (h, l) = (hits.clone(), log.clone());
    thread::spawn(move || {
        for s in listener.incoming() {
            let Ok(mut s) = s else { continue };
            let n = h.fetch_add(1, Ordering::SeqCst);
            let (handler, l) = (handler.clone(), l.clone());
            thread::spawn(move || {
                if let Some(req) = read_req(&s) {
                    l.lock().unwrap().push(req.clone());
                    handler(&req, &mut s, n);
                }
            });
        }
    });
    Server { base, hits, log }
}

fn send(s: &mut TcpStream, head: &str, body: &[u8]) {
    let _ = s.write_all(format!("{head}\r\n").as_bytes());
    let _ = s.write_all(body);
}

/// Test settings: test servers close after their body, so reconnecting is off by default.
fn config() -> StreamConfig {
    StreamConfig {
        reconnect: false,
        reconnect_delay: Duration::from_millis(20),
        read_timeout: Duration::from_secs(5),
        ..StreamConfig::default()
    }
}

/// Runs the network thread alone and collects the audio bytes it puts in the ring.
fn fetch_audio(location: &str, config: StreamConfig) -> (Vec<u8>, State) {
    let shared = Arc::new(Shared::new(State::new(location, 64 * 1024, 10, true)));
    let s = shared.clone();
    let loc = location.to_string();
    let net = thread::spawn(move || net::run(s, config, Arc::default(), loc, Vec::new()));
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut st = shared.lock();
    let mut buf = vec![0u8; 1000];
    loop {
        let n = st.take(&mut buf);
        out.extend_from_slice(&buf[..n]);
        shared.cv.notify_all();
        if st.net_done && st.ring.is_empty() {
            st.apply_meta(true);
            break;
        }
        assert!(Instant::now() < deadline, "timed out");
        if st.ring.is_empty() {
            st = shared.wait_timeout(st, Duration::from_millis(50));
        }
    }
    let state = std::mem::replace(&mut *st, State::new("", 1, 10, true));
    st.abort = true;
    drop(st);
    shared.cv.notify_all();
    net.join().unwrap();
    (out, state)
}

#[derive(Default)]
struct Run {
    frames: usize,
    samples: usize,
    titles: Vec<String>,
    error: Option<DecodeError>,
    peak: f32,
    synced: bool,
}

/// Reads a decoder to its end, the way the audio engine does.
fn drain(d: &mut dyn Decoder, mut on_frames: impl FnMut(&mut dyn Decoder, usize)) -> Run {
    let mut run = Run::default();
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "decoder did not finish");
        out.clear();
        let status = d.read(&mut out);
        if let Some(t) = d.take_title() {
            run.titles.push(t);
        }
        run.synced |= d.in_sync();
        match status {
            Ok(ReadStatus::Frames(n)) => {
                run.frames += n;
                run.samples += out.len();
                run.peak = out.iter().fold(run.peak, |p, s| p.max(s.abs()));
                on_frames(d, run.frames);
            }
            Ok(ReadStatus::Pending) => thread::sleep(Duration::from_millis(1)),
            Ok(ReadStatus::End) => break,
            Err(e) => {
                run.error = Some(e);
                break;
            }
        }
    }
    run
}

fn open(factory: &StreamFactory, location: &str) -> StreamDecoder {
    factory.open_stream(location).unwrap()
}

fn icy_server(metaint: usize, frames: usize, title: &'static [u8]) -> Server {
    serve(move |_, s, _| {
        let audio = mp3(frames);
        let head = format!(
            "HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\nicy-name: Station\r\n\
             icy-genre: Rock\r\nicy-url: http://station.example/\r\nicy-br: 128\r\n\
             icy-notice2: Hello<BR>\r\nicy-metaint: {metaint}\r\n"
        );
        send(s, &head, &with_icy(&audio, metaint, title));
    })
}

fn icy_metadata(metaint: usize) {
    let frames = 300;
    let server = icy_server(
        metaint,
        frames,
        b"StreamTitle='Song';StreamUrl='http://x/';",
    );
    let (audio, st) = fetch_audio(&server.url("/live"), config());
    assert_eq!(audio, mp3(frames), "metadata must be stripped exactly");
    assert_eq!(st.fields.streamtitle.as_deref(), Some("Song"));
    assert_eq!(st.fields.streamlink.as_deref(), Some("http://x/"));
    assert_eq!(st.fields.metaint, metaint);
    assert_eq!(st.title(), "Song (Station)");

    let factory = StreamFactory::new(config());
    let mut d = open(&factory, &server.url("/live"));
    let run = drain(&mut d, |_, _| {});
    assert!(run.error.is_none(), "{:?}", run.error);
    assert!(
        run.frames >= (frames - 2) * FRAME_SAMPLES && run.frames <= frames * FRAME_SAMPLES,
        "{}",
        run.frames
    );
    assert_eq!(run.samples, run.frames, "mono");
    assert_eq!(
        run.titles.last().map(String::as_str),
        Some("Song (Station)")
    );
    assert!(run.synced);
    let f = d.fields();
    assert_eq!(f.get("streamname"), Some("Station"));
    assert_eq!(f.get("streamgenre"), Some("Rock"));
    assert_eq!(f.get("streamurl"), Some("http://station.example/"));
    assert_eq!(f.get("streamtitle"), Some("Song"));
    assert_eq!(f.notice.as_deref(), Some("Hello"));
    assert_eq!(factory.fields(&server.url("/live")), Some(f));
    assert_eq!(d.info().sample_rate, 44_100);
    assert_eq!(d.info().channels, 1);
    assert!(!d.info().seekable);
    assert!(d.kbps().is_some_and(|k| (120..=130).contains(&k)));
    drop(d);
    assert_eq!(factory.fields(&server.url("/live")), None);
}

#[test]
fn icy_metadata_every_8192_bytes() {
    icy_metadata(8192);
}

#[test]
fn icy_metadata_every_16000_bytes() {
    icy_metadata(16000);
}

#[test]
fn station_name_preference() {
    let server = icy_server(8192, 60, b"StreamTitle='Song';");
    let factory = StreamFactory::new(StreamConfig {
        show_station_name: false,
        ..config()
    });
    let run = drain(&mut open(&factory, &server.url("/")), |_, _| {});
    assert_eq!(run.titles.last().map(String::as_str), Some("Song"));
    let server = icy_server(8192, 60, b"StreamTitle='';");
    let run = drain(&mut open(&factory, &server.url("/")), |_, _| {});
    assert_eq!(run.titles.last().map(String::as_str), Some("(Station)"));
}

#[test]
fn utf8_and_windows_1252_titles() {
    let utf8 = icy_server(8192, 40, "StreamTitle='Café – Bar';".as_bytes());
    let cp1252 = icy_server(8192, 40, b"StreamTitle='Caf\xe9 \x96 Bar';");
    for server in [utf8, cp1252] {
        let (_, st) = fetch_audio(&server.url("/"), config());
        assert_eq!(st.fields.streamtitle.as_deref(), Some("Café – Bar"));
    }
}

#[test]
fn icy_status_line() {
    let frames = 50;
    let server = serve(move |_, s, _| {
        let body = with_icy(&mp3(frames), 8192, b"StreamTitle='T';");
        send(
            s,
            "ICY 200 OK\r\nicy-name: Old\r\nicy-metaint: 8192\r\n",
            &body,
        );
    });
    let (audio, st) = fetch_audio(&server.url("/"), config());
    assert_eq!(audio, mp3(frames));
    assert_eq!(st.fields.streamname.as_deref(), Some("Old"));
    let request = &server.log.lock().unwrap()[0];
    assert_eq!(request.header("Icy-MetaData"), Some("1"));
    assert_eq!(request.header("Accept"), Some("*/*"));
    assert!(request.header("User-Agent").unwrap().starts_with("nulamp/"));
    assert_eq!(
        request.header("Host"),
        Some(&server.base["http://".len()..])
    );
}

#[test]
fn chunked_transfer_encoding() {
    let frames = 120;
    let server = serve(move |_, s, _| {
        let body = with_icy(&mp3(frames), 8192, b"StreamTitle='Chunky';");
        send(
            s,
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: audio/mpeg\r\n\
             icy-metaint: 8192\r\n",
            b"",
        );
        let mut sizes = [1usize, 7, 1000, 4096, 333].iter().cycle();
        let mut rest = &body[..];
        while !rest.is_empty() {
            let n = (*sizes.next().unwrap()).min(rest.len());
            let _ = write!(s, "{n:x};ext=1\r\n");
            let _ = s.write_all(&rest[..n]);
            let _ = s.write_all(b"\r\n");
            rest = &rest[n..];
        }
        let _ = s.write_all(b"0\r\nX-Trailer: y\r\n\r\n");
    });
    let (audio, st) = fetch_audio(&server.url("/"), config());
    assert_eq!(audio, mp3(frames));
    assert_eq!(st.fields.streamtitle.as_deref(), Some("Chunky"));
}

fn redirect_server() -> Server {
    serve(|req, s, _| {
        let n: u32 = req.path.trim_start_matches("/r/").parse().unwrap_or(0);
        if n > 0 {
            // Alternate absolute and relative locations.
            let loc = if n.is_multiple_of(2) {
                format!("/r/{}", n - 1)
            } else {
                format!("{}", n - 1)
            };
            send(
                s,
                &format!("HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\n"),
                b"",
            );
        } else {
            send(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 4170\r\n",
                &mp3(10),
            );
        }
    })
}

#[test]
fn redirect_chains() {
    let server = redirect_server();
    let factory = StreamFactory::new(config());
    for n in [3, 10] {
        let url = server.url(&format!("/r/{n}"));
        assert_eq!(factory.resolve_playlist_url(&url), Ok(vec![url.clone()]));
        let run = drain(&mut open(&factory, &url), |_, _| {});
        assert!(run.error.is_none(), "{:?}", run.error);
        assert_eq!(run.frames, 10 * FRAME_SAMPLES);
    }
    let url = server.url("/r/11");
    assert_eq!(
        factory.resolve_playlist_url(&url),
        Err(StreamError::TooManyRedirects)
    );
    let run = drain(&mut open(&factory, &url), |_, _| {});
    assert!(matches!(
        run.error,
        Some(DecodeError::Stream(StreamError::TooManyRedirects))
    ));
    assert_eq!(
        run.titles.last().map(String::as_str),
        Some("[Too many redirects] 11")
    );
}

struct CountingPrompt {
    calls: AtomicUsize,
    answer: (&'static str, &'static str),
    realms: Mutex<Vec<String>>,
}

impl CredentialPrompt for CountingPrompt {
    fn credentials(&self, _url: &str, realm: &str) -> Option<(String, String)> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.realms.lock().unwrap().push(realm.to_string());
        Some((self.answer.0.into(), self.answer.1.into()))
    }
}

fn auth_server() -> Server {
    serve(|req, s, _| {
        let realm = if req.path.starts_with("/other") {
            "Other"
        } else {
            "Radio"
        };
        let expected = format!("Basic {}", http::base64("user:secret"));
        if req.header("Authorization") == Some(expected.as_str()) {
            send(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 4170\r\n",
                &mp3(10),
            );
        } else {
            send(
                s,
                &format!(
                    "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"{realm}\"\r\n\
                     Content-Length: 0\r\n"
                ),
                b"",
            );
        }
    })
}

#[test]
fn basic_authentication_prompts_once_per_realm() {
    let server = auth_server();
    let factory = StreamFactory::new(config());
    let url = server.url("/a");
    assert_eq!(
        factory.resolve_playlist_url(&url),
        Err(StreamError::AuthRequired {
            realm: "Radio".into()
        })
    );
    let run = drain(&mut open(&factory, &url), |_, _| {});
    assert!(matches!(
        run.error,
        Some(DecodeError::Stream(StreamError::AuthRequired { .. }))
    ));
    assert_eq!(
        run.titles.last().map(String::as_str),
        Some("[Access denied] a")
    );

    let prompt = Arc::new(CountingPrompt {
        calls: AtomicUsize::new(0),
        answer: ("user", "secret"),
        realms: Mutex::new(Vec::new()),
    });
    factory.set_credential_prompt(prompt.clone());
    for _ in 0..2 {
        let run = drain(&mut open(&factory, &url), |_, _| {});
        assert!(run.error.is_none(), "{:?}", run.error);
        assert_eq!(run.frames, 10 * FRAME_SAMPLES);
    }
    assert_eq!(factory.resolve_playlist_url(&url), Ok(vec![url.clone()]));
    assert_eq!(prompt.calls.load(Ordering::SeqCst), 1);
    assert!(factory.resolve_playlist_url(&server.url("/other")).is_ok());
    assert_eq!(prompt.calls.load(Ordering::SeqCst), 2);
    assert_eq!(*prompt.realms.lock().unwrap(), ["Radio", "Other"]);

    // Wrong credentials: asked once more, then refused.
    let wrong = Arc::new(CountingPrompt {
        calls: AtomicUsize::new(0),
        answer: ("user", "nope"),
        realms: Mutex::new(Vec::new()),
    });
    let factory = StreamFactory::new(config());
    factory.set_credential_prompt(wrong.clone());
    factory.credentials().remember("Radio", "old", "stale");
    assert!(matches!(
        factory.resolve_playlist_url(&url),
        Err(StreamError::AuthRequired { .. })
    ));
    assert_eq!(wrong.calls.load(Ordering::SeqCst), 1);
    // Credentials in the URL are used directly.
    let with_user = url.replace("http://", "http://user:secret@");
    assert!(
        StreamFactory::new(config())
            .resolve_playlist_url(&with_user)
            .is_ok()
    );
}

#[test]
fn error_statuses() {
    let server = serve(|req, s, _| match req.path.as_str() {
        "/full" => send(s, "HTTP/1.0 503 Service Unavailable\r\n", b""),
        "/icyfull" => send(s, "ICY 400 Server Full\r\n", b""),
        "/html" => send(
            s,
            "HTTP/1.0 200 OK\r\nContent-Type: text/html\r\n",
            &b"<html><body>no audio here</body></html>".repeat(200),
        ),
        _ => send(s, "HTTP/1.0 404 Not Found\r\n", b""),
    });
    let factory = StreamFactory::new(config());
    for (path, text) in [
        ("/full", "[Server full] full"),
        ("/icyfull", "[Server full] icyfull"),
        ("/gone", "[Not found] gone"),
    ] {
        let run = drain(&mut open(&factory, &server.url(path)), |_, _| {});
        assert!(run.error.is_some(), "{path}");
        assert_eq!(run.titles.last().map(String::as_str), Some(text));
    }
    let run = drain(&mut open(&factory, &server.url("/html")), |_, _| {});
    assert!(matches!(run.error, Some(DecodeError::Unsupported(_))));
    // Nothing listens on this port any more.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let run = drain(
        &mut open(&factory, &format!("http://127.0.0.1:{port}/x")),
        |_, _| {},
    );
    assert!(matches!(
        run.error,
        Some(DecodeError::Stream(StreamError::Connect(_)))
    ));
}

#[test]
fn tls_handshake_failure() {
    // A server that closes every connection during the handshake.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for s in listener.incoming() {
            drop(s);
        }
    });
    let url = format!("https://127.0.0.1:{port}/live");
    let run = drain(&mut open(&StreamFactory::new(config()), &url), |_, _| {});
    assert!(
        matches!(run.error, Some(DecodeError::Stream(StreamError::Tls(_)))),
        "{:?}",
        run.error
    );
}

fn dropping_server() -> Server {
    serve(|_, s, n| {
        if n < 2 {
            send(
                s,
                "HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\nicy-name: Drop\r\n",
                &mp3(100),
            );
        } else {
            send(s, "HTTP/1.0 404 Not Found\r\n", b"");
        }
    })
}

#[test]
fn reconnects_after_a_drop() {
    let server = dropping_server();
    let factory = StreamFactory::new(StreamConfig {
        reconnect: true,
        ..config()
    });
    let run = drain(&mut open(&factory, &server.url("/live")), |_, _| {});
    assert!(run.error.is_none(), "{:?}", run.error);
    // Two connections with audio, then the other four of five tries fail.
    assert_eq!(server.hits(), 6);
    assert!(run.frames >= 198 * FRAME_SAMPLES, "{}", run.frames);
}

#[test]
fn ends_after_a_drop_without_reconnect() {
    let server = dropping_server();
    let factory = StreamFactory::new(StreamConfig {
        reconnect: false,
        ..config()
    });
    let run = drain(&mut open(&factory, &server.url("/live")), |_, _| {});
    assert!(run.error.is_none(), "{:?}", run.error);
    assert_eq!(server.hits(), 1);
    assert!(
        (99 * FRAME_SAMPLES..=100 * FRAME_SAMPLES).contains(&run.frames),
        "{}",
        run.frames
    );
}

fn playlist_server() -> Server {
    serve(|req, s, _| {
        let host = req.header("Host").unwrap_or_default().to_string();
        match req.path.as_str() {
            "/list.pls?x=1" => send(
                s,
                "HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n",
                format!("[playlist]\nNumberOfEntries=2\nFile1=http://{host}/live\nFile2=/b.mp3\n")
                    .as_bytes(),
            ),
            "/listen" => send(
                s,
                "HTTP/1.0 200 OK\r\nContent-Type: audio/x-mpegurl\r\n",
                format!("#EXTM3U\n#EXTINF:-1,Live\nhttp://{host}/live\n").as_bytes(),
            ),
            "/hls.m3u8" => send(
                s,
                "HTTP/1.0 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\n",
                b"#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nseg1.ts\n",
            ),
            _ => send(
                s,
                "HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\nicy-name: Live\r\n",
                &mp3(30),
            ),
        }
    })
}

#[test]
fn playlist_urls() {
    let server = playlist_server();
    let factory = StreamFactory::new(config());
    let live = server.url("/live");
    assert_eq!(
        factory.resolve_playlist_url(&server.url("/list.pls?x=1")),
        Ok(vec![live.clone(), server.url("/b.mp3")])
    );
    assert_eq!(
        factory.resolve_playlist_url(&server.url("/listen")),
        Ok(vec![live.clone()])
    );
    assert_eq!(factory.resolve_playlist_url(&live), Ok(vec![live.clone()]));
    assert_eq!(
        factory.resolve_playlist_url(&server.url("/hls.m3u8")),
        Err(StreamError::Hls)
    );
    for path in ["/list.pls?x=1", "/listen"] {
        let mut d = open(&factory, &server.url(path));
        let run = drain(&mut d, |_, _| {});
        assert!(run.error.is_none(), "{:?}", run.error);
        assert_eq!(run.frames, 30 * FRAME_SAMPLES);
        assert_eq!(d.fields().location, live);
        assert_eq!(run.titles.last().map(String::as_str), Some("Live"));
    }
    let run = drain(&mut open(&factory, &server.url("/hls.m3u8")), |_, _| {});
    assert!(matches!(
        run.error,
        Some(DecodeError::Stream(StreamError::Hls))
    ));
}

#[test]
fn short_gaps_do_not_rebuffer() {
    // A server sending in real time leaves the buffer empty between packets; that alone
    // must not show buffering again.
    let server = serve(|_, s, _| {
        send(
            s,
            "HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\nicy-name: Live\r\nicy-br: 128\r\n",
            b"",
        );
        let audio = mp3(120);
        let (burst, rest) = audio.split_at(audio.len() / 2);
        if s.write_all(burst).is_err() {
            return;
        }
        for chunk in rest.chunks(rest.len() / 4 + 1) {
            thread::sleep(Duration::from_millis(300));
            if s.write_all(chunk).is_err() {
                return;
            }
        }
    });
    let factory = StreamFactory::new(config());
    let mut d = open(&factory, &server.url("/live"));
    let run = drain(&mut d, |_, _| {});
    assert!(run.error.is_none(), "{:?}", run.error);
    let first_play = run.titles.iter().position(|t| t == "Live").unwrap();
    assert!(
        !run.titles[first_play..]
            .iter()
            .any(|t| t.starts_with("[Buffer: ")),
        "{:?}",
        run.titles
    );
}

#[test]
fn buffering_progress_and_rebuffering() {
    let server = serve(|_, s, _| {
        thread::sleep(Duration::from_millis(200));
        send(
            s,
            "HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\nicy-name: Slow\r\nicy-br: 128\r\n",
            b"",
        );
        let audio = mp3(240);
        for (i, chunk) in audio.chunks(2048).enumerate() {
            if s.write_all(chunk).is_err() {
                return;
            }
            // A stall half way through, longer than `UNDERRUN_AFTER`.
            let pause = if i == 24 { 1200 } else { 8 };
            thread::sleep(Duration::from_millis(pause));
        }
    });
    let factory = StreamFactory::new(config());
    let mut d = open(&factory, &server.url("/live"));
    assert!(!d.in_sync());
    let run = drain(&mut d, |_, _| {});
    assert!(run.error.is_none(), "{:?}", run.error);
    assert!(run.frames >= 238 * FRAME_SAMPLES, "{}", run.frames);
    assert_eq!(
        run.titles.first().map(String::as_str),
        Some(format!("[Connecting] {}", server.url("/live")).as_str())
    );
    assert!(run.synced);
    let buffer_titles: Vec<u32> = run
        .titles
        .iter()
        .filter_map(|t| {
            t.strip_prefix("[Buffer: ")?
                .strip_suffix("%] Slow")?
                .parse()
                .ok()
        })
        .collect();
    let first_play = run.titles.iter().position(|t| t == "Slow").unwrap();
    // Initial buffering counts up to the target.
    let initial: Vec<u32> = run.titles[..first_play]
        .iter()
        .filter_map(|t| {
            t.strip_prefix("[Buffer: ")?
                .strip_suffix("%] Slow")?
                .parse()
                .ok()
        })
        .collect();
    assert!(initial.len() >= 3, "{:?}", run.titles);
    assert!(initial.windows(2).all(|w| w[0] <= w[1]), "{initial:?}");
    // The stall underruns the buffer and shows buffering again.
    assert!(
        run.titles[first_play..]
            .iter()
            .any(|t| t.starts_with("[Buffer: ")),
        "{:?}",
        run.titles
    );
    assert!(buffer_titles.iter().all(|&p| p <= 100));
}

fn file_server(frames: usize) -> Server {
    serve(move |req, s, _| {
        let body = mp3(frames);
        let total = body.len();
        let start = req
            .header("Range")
            .and_then(|r| r.strip_prefix("bytes=")?.trim_end_matches('-').parse().ok())
            .unwrap_or(0usize);
        let head = if start > 0 {
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{total}\r\n\
                 Content-Length: {}\r\n",
                total - 1,
                total - start
            )
        } else {
            format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n")
        };
        let head = format!(
            "{head}Accept-Ranges: bytes\r\nContent-Type: audio/mpeg\r\n\
             Content-Disposition: attachment; filename=\"tone.mp3\"\r\n"
        );
        send(s, &head, &body[start.min(total)..]);
    })
}

#[test]
fn range_requests_seek_in_files() {
    let frames = 400;
    let server = file_server(frames);
    let factory = StreamFactory::new(config());
    let mut d = open(&factory, &server.url("/files/x?id=1"));
    let mut seeked = None;
    let run = drain(&mut d, |d, total| {
        if seeked.is_none() && total >= 20 * FRAME_SAMPLES {
            assert!(d.info().seekable);
            seeked = Some((total, d.info().length_ms, d.seek(8000).unwrap()));
        }
    });
    assert!(run.error.is_none(), "{:?}", run.error);
    let (before, length, pos) = seeked.unwrap();
    assert_eq!(pos, 8000);
    let length = length.unwrap();
    let expected = frames as u64 * FRAME_SAMPLES as u64 * 1000 / 44_100;
    assert!(length.abs_diff(expected) < 300, "{length} vs {expected}");
    let after = run.frames - before;
    let rest = (frames * FRAME_SAMPLES) as f64 - 8.0 * 44_100.0;
    assert!(
        (after as f64 - rest).abs() < 44_100.0 * 0.5,
        "{after} frames after the seek, expected about {rest}"
    );
    assert_eq!(run.titles.last().map(String::as_str), Some("tone.mp3"));
    let log = server.log.lock().unwrap();
    let range = log
        .iter()
        .filter_map(|r| r.header("Range"))
        .filter_map(|r| {
            r.strip_prefix("bytes=")?
                .trim_end_matches('-')
                .parse::<usize>()
                .ok()
        })
        .next_back()
        .expect("a Range request");
    // The byte offset of 8 s, less a few frames the coarse seek steps back.
    let byte = frames * FRAME_LEN * 8000 / expected as usize;
    assert!(
        range <= byte && byte - range < 16 * FRAME_LEN,
        "{range} vs {byte}"
    );
}

#[test]
fn live_streams_do_not_seek() {
    let server = icy_server(8192, 30, b"StreamTitle='x';");
    let mut d = open(&StreamFactory::new(config()), &server.url("/"));
    assert!(d.seek(1000).is_err());
}

#[test]
fn closing_stops_the_threads() {
    let server = serve(|_, s, _| {
        send(s, "HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\n", b"");
        // Endless live stream.
        let frame = mp3(1);
        while s.write_all(&frame).is_ok() {}
    });
    let factory = StreamFactory::new(config());
    let mut d = open(&factory, &server.url("/"));
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !matches!(d.read(&mut out), Ok(ReadStatus::Frames(_))) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let shared = d.shared.clone();
    drop(d);
    let deadline = Instant::now() + Duration::from_secs(5);
    // Both threads let go of the shared state.
    while Arc::strong_count(&shared) > 1 {
        assert!(Instant::now() < deadline, "threads still running");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn proxies() {
    assert_eq!(
        Proxy::parse("u:p@proxy:3128"),
        Some(Proxy {
            host: "proxy".into(),
            port: 3128,
            user: Some(("u".into(), "p".into()))
        })
    );
    assert_eq!(Proxy::parse("http://proxy/").map(|p| p.port), Some(80));
    assert_eq!(Proxy::parse(""), None);

    // A plain HTTP proxy gets absolute request URIs and the proxy credentials.
    let proxy = serve(|_, s, _| {
        send(
            s,
            "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 4170\r\n",
            &mp3(10),
        );
    });
    let port: u16 = proxy.base.rsplit(':').next().unwrap().parse().unwrap();
    let mut cfg = config();
    cfg.proxy = Proxy::parse(&format!("u:p@127.0.0.1:{port}"));
    let factory = StreamFactory::new(cfg.clone());
    let run = drain(
        &mut open(&factory, "http://radio.invalid:8000/live"),
        |_, _| {},
    );
    assert!(run.error.is_none(), "{:?}", run.error);
    {
        let log = proxy.log.lock().unwrap();
        assert_eq!(log[0].path, "http://radio.invalid:8000/live");
        let auth = format!("Basic {}", http::base64("u:p"));
        assert_eq!(log[0].header("Proxy-Authorization"), Some(auth.as_str()));
    }
    // "Only for port 80": other ports connect directly.
    cfg.proxy_only_port_80 = true;
    let direct = file_server(10);
    let factory = StreamFactory::new(cfg);
    let run = drain(&mut open(&factory, &direct.url("/a.mp3")), |_, _| {});
    assert!(run.error.is_none(), "{:?}", run.error);
    assert_eq!(proxy.hits(), 1);
}

fn header_options(headers: &[(&str, &str)]) -> StreamOptions {
    StreamOptions {
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ..StreamOptions::default()
    }
}

/// Values of every request header named `name`.
fn header_values(req: &Req, name: &str) -> Vec<String> {
    req.headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
        .collect()
}

#[test]
fn extra_headers_reach_the_origin() {
    let server = file_server(10);
    let factory = StreamFactory::new(config());
    let options = header_options(&[
        ("X-Emby-Token", "abc"),
        ("Authorization", "MediaBrowser Token=\"abc\""),
    ]);
    // Credentials in the URL are not sent as well: the extra `Authorization` replaces them.
    let url = server
        .url("/a.mp3")
        .replace("http://", "http://user:secret@");
    let run = drain(
        &mut factory.open_stream_with(&url, &options).unwrap(),
        |_, _| {},
    );
    assert!(run.error.is_none(), "{:?}", run.error);
    let log = server.log.lock().unwrap();
    assert_eq!(log[0].header("X-Emby-Token"), Some("abc"));
    assert_eq!(
        header_values(&log[0], "Authorization"),
        ["MediaBrowser Token=\"abc\""]
    );
    drop(log);

    // A refused `Authorization` header fails at once instead of asking for Basic credentials.
    let server = auth_server();
    let prompt = Arc::new(CountingPrompt {
        calls: AtomicUsize::new(0),
        answer: ("user", "secret"),
        realms: Mutex::new(Vec::new()),
    });
    factory.set_credential_prompt(prompt.clone());
    let options = header_options(&[("Authorization", "Token wrong")]);
    let run = drain(
        &mut factory
            .open_stream_with(&server.url("/a"), &options)
            .unwrap(),
        |_, _| {},
    );
    assert!(matches!(
        run.error,
        Some(DecodeError::Stream(StreamError::AuthRequired { .. }))
    ));
    assert_eq!(prompt.calls.load(Ordering::SeqCst), 0);
    assert_eq!(server.hits(), 1);
}

#[test]
fn extra_headers_stay_on_their_origin() {
    let other = file_server(10);
    let target = other.url("/b.mp3");
    // Redirects once on its own origin, then to another port.
    let origin = serve(move |req, s, _| {
        let loc = if req.path == "/start" {
            "/next".to_string()
        } else {
            target.clone()
        };
        send(
            s,
            &format!("HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\n"),
            b"",
        );
    });
    let factory = StreamFactory::new(config());
    let options = header_options(&[("X-Emby-Token", "abc"), ("Authorization", "Token t")]);
    let run = drain(
        &mut factory
            .open_stream_with(&origin.url("/start"), &options)
            .unwrap(),
        |_, _| {},
    );
    assert!(run.error.is_none(), "{:?}", run.error);
    assert_eq!(run.frames, 10 * FRAME_SAMPLES);
    // Range requests after the first connection follow the same redirects.
    let log = origin.log.lock().unwrap();
    assert!(log.len() >= 2);
    assert!(log.iter().all(|r| r.header("X-Emby-Token") == Some("abc")));
    let log = other.log.lock().unwrap();
    assert!(!log.is_empty());
    assert!(
        log.iter()
            .all(|r| r.header("X-Emby-Token").is_none() && r.header("Authorization").is_none())
    );
}

#[test]
fn header_injection_is_refused() {
    let server = file_server(10);
    let options = header_options(&[
        ("X-Good", "1"),
        ("X-Bad", "a\r\nX-Injected: yes"),
        ("X-Lf", "a\nX-Injected: yes"),
        ("X-Evil\r\nX-Injected", "v"),
        ("Bad Name", "v"),
        ("Host", "evil.example"),
        ("Proxy-Authorization", "Basic x"),
    ]);
    let run = drain(
        &mut StreamFactory::new(config())
            .open_stream_with(&server.url("/a.mp3"), &options)
            .unwrap(),
        |_, _| {},
    );
    assert!(run.error.is_none(), "{:?}", run.error);
    let log = server.log.lock().unwrap();
    let req = &log[0];
    assert_eq!(req.header("X-Good"), Some("1"));
    for name in [
        "X-Injected",
        "X-Bad",
        "X-Lf",
        "X-Evil",
        "Bad Name",
        "Proxy-Authorization",
    ] {
        assert_eq!(req.header(name), None, "{name}");
    }
    assert_eq!(
        header_values(req, "Host"),
        [&server.base["http://".len()..]]
    );
}

#[test]
fn kept_titles_are_not_reported() {
    let server = icy_server(8192, 60, b"StreamTitle='Song';");
    let options = StreamOptions {
        keep_title: true,
        ..StreamOptions::default()
    };
    let mut d = StreamFactory::new(config())
        .open_stream_with(&server.url("/live?api_key=secret"), &options)
        .unwrap();
    let run = drain(&mut d, |_, _| {});
    assert!(run.error.is_none(), "{:?}", run.error);
    assert!(run.frames > 0);
    assert!(run.synced);
    assert!(run.titles.is_empty(), "{:?}", run.titles);
    assert_eq!(d.take_title(), None);
    // The stream fields are still there.
    assert_eq!(d.fields().streamtitle.as_deref(), Some("Song"));
}

/// Encodes 440 Hz with ffmpeg; `None` (test skipped) when ffmpeg or the encoder is missing.
fn ffmpeg_tone(name: &str, args: &[&str]) -> Option<Vec<u8>> {
    let ffmpeg = std::path::Path::new("/usr/bin/ffmpeg");
    if !ffmpeg.exists() {
        eprintln!("ffmpeg not found; skipping");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("nulamp-stream-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let ok = std::process::Command::new(ffmpeg)
        .args(["-v", "quiet", "-y", "-f", "lavfi", "-i"])
        .arg("sine=frequency=440:duration=3:sample_rate=44100")
        .args(args)
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    let data = std::fs::read(&path).ok().filter(|_| ok);
    std::fs::remove_dir_all(&dir).unwrap();
    if data.is_none() {
        eprintln!("ffmpeg cannot encode {name}; skipping");
    }
    data
}

#[test]
fn chained_ogg_titles() {
    let tagged = |title: &str| {
        let meta = format!("title={title}");
        ffmpeg_tone(
            &format!("{title}.ogg"),
            &[
                "-ac",
                "2",
                "-c:a",
                "libvorbis",
                "-metadata",
                "artist=Band",
                "-metadata",
                &meta,
            ],
        )
    };
    let (Some(one), Some(two)) = (tagged("One"), tagged("Two")) else {
        return;
    };
    let body = [one, two].concat();
    let server = serve(move |_, s, _| {
        send(
            s,
            "HTTP/1.0 200 OK\r\nContent-Type: application/ogg\r\n",
            &body,
        );
    });
    let run = drain(
        &mut open(&StreamFactory::new(config()), &server.url("/")),
        |_, _| {},
    );
    assert!(run.error.is_none(), "{:?}", run.error);
    assert!(run.frames.abs_diff(6 * 44_100) < 4410, "{}", run.frames);
    assert_eq!(run.titles.last().map(String::as_str), Some("Band - Two"));
}

/// A real encoded MP3 when ffmpeg is installed (else skipped).
#[test]
fn decodes_an_encoded_mp3() {
    let Some(data) = ffmpeg_tone(
        "tone.mp3",
        &["-ac", "2", "-c:a", "libmp3lame", "-b:a", "128k"],
    ) else {
        return;
    };
    let server = serve(move |_, s, _| {
        send(
            s,
            "HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\nicy-name: Tone\r\nicy-metaint: 16000\r\n",
            &with_icy(&data, 16000, b"StreamTitle='A440';"),
        );
    });
    let mut d = open(&StreamFactory::new(config()), &server.url("/"));
    let run = drain(&mut d, |_, _| {});
    assert!(run.error.is_none(), "{:?}", run.error);
    assert_eq!(d.info().channels, 2);
    // The lavfi sine source has an amplitude of about -21.5 dBFS.
    assert!(run.peak > 0.05, "{}", run.peak);
    assert!(run.frames.abs_diff(3 * 44_100) < 4410, "{}", run.frames);
    assert_eq!(run.titles.last().map(String::as_str), Some("A440 (Tone)"));
}
