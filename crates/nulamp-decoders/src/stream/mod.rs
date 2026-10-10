//! Network stream input (07 §8): HTTP and SHOUTcast/Icecast streams, playlist URLs, in-band
//! metadata, buffering and reconnection.
//!
//! [`StreamFactory::open`] returns at once. A network thread connects and fills a byte ring
//! buffer; a decoding thread feeds it to Symphonia and queues decoded blocks. The decoder
//! handle only takes queued blocks, so [`Decoder::read`] never waits for the network: it
//! answers [`ReadStatus::Pending`] while the stream connects or buffers.
//!
//! Choices where the spec is silent are recorded as `NET1`… in `docs/decisions.md`.

mod decode;
mod http;
pub mod icy;
mod net;
mod shared;
pub mod url;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

pub use http::MAX_REDIRECTS;
pub use shared::prebuffer_percent;
pub use url::{SCHEMES, is_stream_location, location_title};

use crate::{DecodeError, Decoder, DecoderFactory, ReadStatus, StreamInfo};
use shared::{Phase, Shared, State};

/// Network errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    #[error("invalid stream URL: {0}")]
    BadUrl(String),
    #[error("cannot connect to {0}")]
    Connect(String),
    #[error("timed out")]
    Timeout,
    /// A 401 answer for which no credentials are known or given.
    #[error("authentication required for realm \"{realm}\"")]
    AuthRequired { realm: String },
    #[error("proxy authentication required")]
    ProxyAuth,
    #[error("more than {MAX_REDIRECTS} redirects")]
    TooManyRedirects,
    #[error("server answered {code} {reason}")]
    Status { code: u16, reason: String },
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("{0}")]
    Io(String),
    /// HLS (`#EXT-X-TARGETDURATION`) is not supported yet.
    #[error("HLS streams are not supported")]
    Hls,
    #[error("playlist: {0}")]
    Playlist(String),
}

impl StreamError {
    /// Short status text shown in the title (07 §8.6).
    pub fn status_text(&self) -> &'static str {
        match self {
            StreamError::AuthRequired { .. } | StreamError::ProxyAuth => "[Access denied]",
            StreamError::Status {
                code: 401 | 403, ..
            } => "[Access denied]",
            StreamError::Status { code: 503, .. } => "[Server full]",
            StreamError::Status { reason, .. } if reason.to_ascii_lowercase().contains("full") => {
                "[Server full]"
            }
            StreamError::Status { code: 404, .. } => "[Not found]",
            StreamError::Timeout => "[Timed out]",
            StreamError::TooManyRedirects => "[Too many redirects]",
            StreamError::Hls | StreamError::Playlist(_) => "[Unsupported]",
            _ => "[Connection failed]",
        }
    }
}

/// An HTTP proxy, `[user:pass@]host:port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    pub host: String,
    pub port: u16,
    pub user: Option<(String, String)>,
}

impl Proxy {
    /// Parses `[user:pass@]host:port` (an `http://` prefix and a trailing `/` are accepted).
    /// The port defaults to 80.
    pub fn parse(s: &str) -> Option<Proxy> {
        let s = s.trim();
        let s = s.strip_prefix("http://").unwrap_or(s).trim_end_matches('/');
        let (user, hostport) = match s.rsplit_once('@') {
            Some((u, h)) => {
                let (name, pass) = u.split_once(':').unwrap_or((u, ""));
                (Some((name.to_string(), pass.to_string())), h)
            }
            None => (None, s),
        };
        let (host, port) = url::split_host_port(hostport, 80)?;
        (!host.is_empty()).then_some(Proxy { host, port, user })
    }

    /// The system proxy from `http_proxy` / `HTTP_PROXY`.
    pub fn from_env() -> Option<Proxy> {
        ["http_proxy", "HTTP_PROXY"]
            .iter()
            .find_map(|k| std::env::var(k).ok())
            .and_then(|v| Proxy::parse(&v))
    }
}

/// Stream preferences (07 §8.3, §8.6, §8.7).
#[derive(Debug, Clone, PartialEq)]
pub struct StreamConfig {
    pub user_agent: String,
    pub proxy: Option<Proxy>,
    /// Use the proxy only for servers on port 80.
    pub proxy_only_port_80: bool,
    /// Network buffer in bytes (at least [`StreamConfig::MIN_BUFFER`]).
    pub buffer_bytes: usize,
    /// Prebuffer setting in percent of the buffer, scaled by bit rate.
    pub prebuffer_percent: u32,
    /// Buffer level to reach again after an underrun or seek, in percent.
    pub rebuffer_percent: u32,
    /// Show `StreamTitle (Station)` instead of `StreamTitle`.
    pub show_station_name: bool,
    /// Reconnect dropped live streams.
    pub reconnect: bool,
    pub reconnect_tries: u32,
    /// Delay before reconnect try n is n × this.
    pub reconnect_delay: Duration,
    pub connect_timeout: Duration,
    /// A connection with no data for this long counts as dropped.
    pub read_timeout: Duration,
}

impl StreamConfig {
    pub const MIN_BUFFER: usize = 32 * 1024;
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            user_agent: concat!("nulamp/", env!("CARGO_PKG_VERSION")).into(),
            proxy: None,
            proxy_only_port_80: false,
            buffer_bytes: 64 * 1024,
            prebuffer_percent: 40,
            rebuffer_percent: 10,
            show_station_name: true,
            reconnect: true,
            reconnect_tries: 5,
            reconnect_delay: Duration::from_secs(1),
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
        }
    }
}

/// Asks the user for credentials when a server answers 401 (prompting is the app's job).
/// Called on a network thread; it may block until the user answers.
pub trait CredentialPrompt: Send + Sync {
    /// User name and password for `realm` at `url`, or `None` to give up.
    fn credentials(&self, url: &str, realm: &str) -> Option<(String, String)>;
}

/// Basic-authentication credentials remembered per realm, and the prompt for new ones.
#[derive(Default)]
pub struct CredentialStore {
    prompt: Mutex<Option<Arc<dyn CredentialPrompt>>>,
    remembered: Mutex<HashMap<String, (String, String)>>,
    /// Serialises prompts so one realm is asked for once.
    prompting: Mutex<()>,
}

impl std::fmt::Debug for CredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let realms: Vec<String> = self
            .remembered
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("CredentialStore")
            .field("realms", &realms)
            .finish()
    }
}

impl CredentialStore {
    pub fn set_prompt(&self, prompt: Option<Arc<dyn CredentialPrompt>>) {
        *self.prompt.lock().unwrap_or_else(|e| e.into_inner()) = prompt;
    }

    pub fn remember(&self, realm: &str, user: &str, password: &str) {
        self.remembered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(realm.into(), (user.into(), password.into()));
    }

    pub fn forget(&self, realm: &str) {
        self.remembered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(realm);
    }

    fn lookup(&self, realm: &str) -> Option<(String, String)> {
        self.remembered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(realm)
            .cloned()
    }

    /// Remembered credentials for `realm`, else the prompt's answer (remembered). The flag is
    /// true when the prompt was asked.
    fn get(&self, url: &str, realm: &str) -> Option<((String, String), bool)> {
        if let Some(c) = self.lookup(realm) {
            return Some((c, false));
        }
        let _guard = self.prompting.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = self.lookup(realm) {
            return Some((c, false));
        }
        let prompt = self
            .prompt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        let c = prompt.credentials(url, realm)?;
        self.remember(realm, &c.0, &c.1);
        Some((c, true))
    }
}

/// Station and song fields of a stream (07 §8.4, §8.5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamFields {
    /// In-band `StreamTitle` (`streamtitle`).
    pub streamtitle: Option<String>,
    /// `icy-name` (`streamname`).
    pub streamname: Option<String>,
    /// `icy-url`, the station home page (`streamurl`).
    pub streamurl: Option<String>,
    /// `icy-genre` (`streamgenre`).
    pub streamgenre: Option<String>,
    /// In-band `StreamUrl`.
    pub streamlink: Option<String>,
    /// `icy-notice2` up to `<BR>`.
    pub notice: Option<String>,
    /// `icy-br` in kbps.
    pub bitrate: Option<u32>,
    pub metaint: usize,
    pub content_type: Option<String>,
    /// The stream URL after playlist resolution (before redirects).
    pub location: String,
}

impl StreamFields {
    /// A field by its metadata name (`streamtitle`, `streamname`, `streamurl`, `streamgenre`).
    pub fn get(&self, name: &str) -> Option<&str> {
        match name.to_ascii_lowercase().as_str() {
            "streamtitle" => self.streamtitle.as_deref(),
            "streamname" => self.streamname.as_deref(),
            "streamurl" => self.streamurl.as_deref(),
            "streamgenre" => self.streamgenre.as_deref(),
            _ => None,
        }
    }
}

struct FactoryInner {
    config: RwLock<StreamConfig>,
    credentials: Arc<CredentialStore>,
    open: Mutex<Vec<(String, Weak<Shared>)>>,
}

/// Opens network streams. Clones share settings, credentials and the open-stream list.
#[derive(Clone)]
pub struct StreamFactory {
    inner: Arc<FactoryInner>,
}

impl Default for StreamFactory {
    fn default() -> Self {
        Self::new(StreamConfig::default())
    }
}

impl std::fmt::Debug for StreamFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamFactory")
            .field("config", &self.config())
            .finish()
    }
}

impl StreamFactory {
    pub fn new(config: StreamConfig) -> Self {
        Self {
            inner: Arc::new(FactoryInner {
                config: RwLock::new(config),
                credentials: Arc::new(CredentialStore::default()),
                open: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn config(&self) -> StreamConfig {
        self.inner
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// New settings apply to streams opened afterwards.
    pub fn set_config(&self, config: StreamConfig) {
        *self.inner.config.write().unwrap_or_else(|e| e.into_inner()) = config;
    }

    pub fn credentials(&self) -> &CredentialStore {
        &self.inner.credentials
    }

    /// Sets the hook asked for user name and password on a 401 answer.
    pub fn set_credential_prompt(&self, prompt: Arc<dyn CredentialPrompt>) {
        self.inner.credentials.set_prompt(Some(prompt));
    }

    /// Fields of the most recently opened stream for `location` that is still open.
    pub fn fields(&self, location: &str) -> Option<StreamFields> {
        let open = self.inner.open.lock().unwrap_or_else(|e| e.into_inner());
        open.iter()
            .rev()
            .filter(|(l, _)| l == location)
            .filter_map(|(_, w)| w.upgrade())
            .map(|s| s.lock().clone_fields())
            .find_map(|f| f)
    }

    /// Expands a URL into playlist entries (07 §8.2, 10 §2.5): a playlist URL (by extension
    /// or content type) gives its entries, any other URL gives itself. Blocks on the network:
    /// call it off the user-interface thread.
    pub fn resolve_playlist_url(&self, location: &str) -> Result<Vec<String>, StreamError> {
        let config = self.config();
        let on_connect = |_: &std::net::TcpStream| {};
        let client = http::Client {
            config: &config,
            credentials: &self.inner.credentials,
            on_connect: &on_connect,
        };
        let url = url::Url::parse(location)?;
        match net::fetch(&client, &url, 0)? {
            net::Fetched::Media(_) => Ok(vec![location.to_string()]),
            net::Fetched::Playlist(entries) => Ok(entries),
        }
    }

    /// Starts connecting to `location` and returns at once.
    pub fn open_stream(&self, location: &str) -> Result<StreamDecoder, DecodeError> {
        let config = self.config();
        let capacity = config.buffer_bytes.max(StreamConfig::MIN_BUFFER);
        let state = State::new(
            location,
            capacity,
            config.rebuffer_percent,
            config.show_station_name,
        );
        let shared = Arc::new(Shared::new(state));
        {
            let mut open = self.inner.open.lock().unwrap_or_else(|e| e.into_inner());
            open.retain(|(_, w)| w.strong_count() > 0);
            open.push((location.to_string(), Arc::downgrade(&shared)));
        }
        let spawn_err = |e: std::io::Error| DecodeError::Open(e.to_string());
        let (s, creds, loc) = (
            shared.clone(),
            self.inner.credentials.clone(),
            location.to_string(),
        );
        std::thread::Builder::new()
            .name("nulamp-net".into())
            .spawn(move || net::run(s, config, creds, loc))
            .map_err(spawn_err)?;
        let s = shared.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("nulamp-stream".into())
            .spawn(move || decode::run(s))
        {
            shared.abort();
            return Err(spawn_err(e));
        }
        let info = shared.lock().info.clone();
        Ok(StreamDecoder {
            shared,
            info,
            info_version: 0,
            last_title: None,
            starved_since: None,
        })
    }
}

impl DecoderFactory for StreamFactory {
    fn name(&self) -> &str {
        "Network stream"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    fn claims(&self, location: &str) -> bool {
        is_stream_location(location)
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        Ok(Box::new(self.open_stream(location)?))
    }
}

/// [`StreamFactory::resolve_playlist_url`] with default settings.
pub fn resolve_playlist_url(location: &str) -> Result<Vec<String>, StreamError> {
    StreamFactory::default().resolve_playlist_url(location)
}

/// An open network stream. Dropping it closes the connection.
pub struct StreamDecoder {
    shared: Arc<Shared>,
    info: StreamInfo,
    info_version: u64,
    last_title: Option<String>,
    /// When the reader first found no decoded audio, for underrun detection.
    starved_since: Option<std::time::Instant>,
}

/// How long the reader may find no decoded audio before the stream rebuffers (NET7). Shorter
/// gaps are normal for servers that send in real time; the output buffer covers them.
pub const UNDERRUN_AFTER: Duration = Duration::from_millis(750);

impl StreamDecoder {
    /// The current title, with any `[Connecting]` / `[Buffer: N%]` / error prefix.
    pub fn title(&self) -> String {
        self.shared.lock().title()
    }

    /// Percent of the buffering target reached, while buffering.
    pub fn buffer_percent(&self) -> Option<u32> {
        self.shared.lock().buffer_percent()
    }

    pub fn fields(&self) -> StreamFields {
        self.shared.lock().fields.clone()
    }
}

impl Drop for StreamDecoder {
    fn drop(&mut self) {
        self.shared.abort();
    }
}

impl Decoder for StreamDecoder {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError> {
        let mut st = self.shared.lock();
        if st.info_version != self.info_version {
            self.info_version = st.info_version;
            self.info = st.info.clone();
        }
        if let Some(b) = st.pcm.pop_front() {
            st.pcm_frames -= b.frames;
            self.shared.cv.notify_all();
            drop(st);
            self.starved_since = None;
            self.info.sample_rate = b.rate;
            self.info.channels = b.channels;
            out.extend_from_slice(&b.samples);
            return Ok(ReadStatus::Frames(b.frames));
        }
        if st.decode_done {
            return match st.decode_error.take() {
                Some(e) => Err(e),
                None => Ok(ReadStatus::End),
            };
        }
        if st.phase == Phase::Playing && st.ring.is_empty() && !st.net_done {
            let since = *self
                .starved_since
                .get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= UNDERRUN_AFTER {
                let p = st.rebuffer_percent;
                st.start_buffering(p);
                self.shared.cv.notify_all();
                self.starved_since = None;
            }
        } else {
            self.starved_since = None;
        }
        Ok(ReadStatus::Pending)
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        if !self.info.seekable {
            return Err(DecodeError::Seek("live stream".into()));
        }
        let mut st = self.shared.lock();
        st.seek_req = Some(ms);
        st.generation += 1;
        st.pcm.clear();
        st.pcm_frames = 0;
        st.decode_done = false;
        st.decode_error = None;
        self.shared.cv.notify_all();
        Ok(ms)
    }

    fn kbps(&self) -> Option<u32> {
        let st = self.shared.lock();
        st.decoded_kbps.or(st.fields.bitrate)
    }

    fn in_sync(&self) -> bool {
        let st = self.shared.lock();
        st.phase == Phase::Playing && st.decoding
    }

    fn take_title(&mut self) -> Option<String> {
        let title = self.shared.lock().title();
        if self.last_title.as_deref() == Some(title.as_str()) {
            return None;
        }
        self.last_title = Some(title.clone());
        Some(title)
    }

    fn stream_fields(&self) -> Option<StreamFields> {
        Some(self.fields())
    }
}
