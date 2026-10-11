//! Minimal HTTP/1.1 client for stream requests (07 §8.3): `ICY 200 OK` status lines,
//! redirects, Basic authentication, proxies, chunked bodies and Range requests.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};

use super::url::Url;
use super::{CredentialStore, StreamConfig, StreamError};

/// Most redirects followed for one request (07 §8.2).
pub const MAX_REDIRECTS: u32 = 10;

const MAX_HEADER_LINE: u64 = 16 * 1024;
const MAX_HEADERS: usize = 200;

/// A plain or TLS connection.
pub(crate) enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => match s.read(buf) {
                // Servers often close without a TLS close_notify.
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
                r => r,
            },
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum BodyMode {
    Length(u64),
    Chunked { left: u64, done: bool },
    UntilClose,
}

/// A response body: de-chunked and limited to `Content-Length`.
pub(crate) struct Body {
    reader: BufReader<Conn>,
    mode: BodyMode,
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        match &mut self.mode {
            BodyMode::Length(0) => Ok(0),
            BodyMode::Length(left) => {
                let max = (*left).min(buf.len() as u64) as usize;
                let n = self.reader.read(&mut buf[..max])?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                *left -= n as u64;
                Ok(n)
            }
            BodyMode::UntilClose => self.reader.read(buf),
            BodyMode::Chunked { left, done } => {
                if *done {
                    return Ok(0);
                }
                if *left == 0 {
                    let size = read_chunk_size(&mut self.reader)?;
                    if size == 0 {
                        // Trailer fields end with an empty line.
                        while !read_line(&mut self.reader)?.trim().is_empty() {}
                        *done = true;
                        return Ok(0);
                    }
                    *left = size;
                }
                let max = (*left).min(buf.len() as u64) as usize;
                let n = self.reader.read(&mut buf[..max])?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                *left -= n as u64;
                if *left == 0 {
                    read_line(&mut self.reader)?;
                }
                Ok(n)
            }
        }
    }
}

fn read_chunk_size(r: &mut impl BufRead) -> io::Result<u64> {
    loop {
        let line = read_line(r)?;
        let hex = line.split(';').next().unwrap_or("").trim();
        if hex.is_empty() {
            continue;
        }
        return u64::from_str_radix(hex, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"));
    }
}

/// Reads one line (LF or CRLF), without the line ending. EOF before any byte is an error.
fn read_line(r: &mut impl BufRead) -> io::Result<String> {
    let mut buf = Vec::new();
    r.take(MAX_HEADER_LINE).read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    Ok(nulamp_playlist::text::decode_utf8_or_1252(&buf))
}

/// A response with a 2xx status.
pub(crate) struct Response {
    /// The URL that answered, after redirects.
    pub url: Url,
    pub status: u16,
    /// Header names in lower case, in order.
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn content_type(&self) -> Option<String> {
        self.header("content-type").map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
    }

    /// Total length of the resource: the `Content-Range` total of a 206, else
    /// `Content-Length` (not for chunked bodies).
    pub fn total_length(&self) -> Option<u64> {
        if self.status == 206 {
            return self
                .header("content-range")
                .and_then(|v| v.rsplit('/').next())
                .and_then(|t| t.trim().parse().ok());
        }
        match self.body.mode {
            BodyMode::Length(_) => self.header("content-length")?.trim().parse().ok(),
            _ => None,
        }
    }

    /// Byte ranges can be requested (`Accept-Ranges: bytes`, or a 206 answer).
    pub fn accepts_ranges(&self) -> bool {
        self.status == 206
            || self
                .header("accept-ranges")
                .is_some_and(|v| v.to_ascii_lowercase().contains("bytes"))
    }

    /// Reads the whole body, up to `limit` bytes.
    pub fn read_all(&mut self, limit: u64) -> Result<Vec<u8>, StreamError> {
        let mut out = Vec::new();
        (&mut self.body)
            .take(limit)
            .read_to_end(&mut out)
            .map_err(io_error)?;
        Ok(out)
    }
}

pub(crate) fn io_error(e: io::Error) -> StreamError {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => StreamError::Timeout,
        _ => StreamError::Io(e.to_string()),
    }
}

/// Header names the client writes itself; extra headers may not use them (nor `Proxy-` names).
const RESERVED_HEADERS: &[&str] = &[
    "host",
    "connection",
    "range",
    "content-length",
    "transfer-encoding",
];

/// Extra request headers bound to one origin (scheme, host and port).
#[derive(Debug)]
pub(crate) struct OriginHeaders {
    tls: bool,
    host: String,
    port: u16,
    headers: Vec<(String, String)>,
}

impl OriginHeaders {
    /// Headers for requests to `origin`'s scheme, host and port. Headers with a name that is
    /// not an HTTP token, a reserved or `Proxy-` name, or a value with CR, LF or NUL are
    /// dropped. `None` when no header is left.
    pub fn new(origin: &Url, headers: &[(String, String)]) -> Option<OriginHeaders> {
        let headers: Vec<(String, String)> = headers
            .iter()
            .filter(|(k, v)| {
                let ok = valid_header(k, v);
                if !ok {
                    log::warn!("stream: dropped request header {:?}", k.trim());
                }
                ok
            })
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        (!headers.is_empty()).then(|| OriginHeaders {
            tls: origin.tls,
            host: origin.host.to_ascii_lowercase(),
            port: origin.port,
            headers,
        })
    }

    /// True when `url` is on the origin.
    fn applies_to(&self, url: &Url) -> bool {
        url.tls == self.tls && url.port == self.port && url.host.eq_ignore_ascii_case(&self.host)
    }

    fn has(&self, name: &str) -> bool {
        self.headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(name))
    }
}

fn valid_header(name: &str, value: &str) -> bool {
    let name = name.trim();
    let token = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
    let lower = name.to_ascii_lowercase();
    token
        && !RESERVED_HEADERS.contains(&lower.as_str())
        && !lower.starts_with("proxy-")
        && !value.contains(['\r', '\n', '\0'])
}

/// Request settings and the socket hook used to abort blocking reads.
pub(crate) struct Client<'a> {
    pub config: &'a StreamConfig,
    pub credentials: &'a CredentialStore,
    /// Called with each new socket, so another thread can shut it down.
    pub on_connect: &'a dyn Fn(&TcpStream),
    /// Extra headers, sent only to their origin.
    pub headers: Option<&'a OriginHeaders>,
}

impl Client<'_> {
    /// The extra headers for a request to `url`.
    fn headers_for(&self, url: &Url) -> Option<&OriginHeaders> {
        self.headers.filter(|h| h.applies_to(url))
    }

    /// GET `url`, following redirects and answering Basic authentication challenges.
    /// `range_from` > 0 asks for the bytes from that offset.
    pub fn get(&self, url: &Url, range_from: u64) -> Result<Response, StreamError> {
        let mut url = url.clone();
        let mut redirects = 0;
        // Realm and whether its credentials were just asked for.
        let mut auth: Option<(String, (String, String))> = None;
        let mut asked = Vec::<String>::new();
        loop {
            let basic = match (&auth, &url.user) {
                (Some((_, c)), _) => Some(c.clone()),
                (None, Some(c)) => Some(c.clone()),
                (None, None) => None,
            };
            let (status, reason, headers, reader) = self.send(&url, range_from, basic.as_ref())?;
            let header = |name: &str| {
                headers
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v): &(String, String)| v.clone())
            };
            match status {
                200..=299 => {
                    let mode = body_mode(status, &headers);
                    return Ok(Response {
                        url,
                        status,
                        headers,
                        body: Body { reader, mode },
                    });
                }
                301 | 302 | 303 | 307 | 308 => {
                    let location = header("location").ok_or_else(|| {
                        StreamError::BadResponse(format!("{status} without Location"))
                    })?;
                    redirects += 1;
                    if redirects > MAX_REDIRECTS {
                        return Err(StreamError::TooManyRedirects);
                    }
                    url = url.join(&location)?;
                    auth = None;
                }
                401 => {
                    let realm = header("www-authenticate")
                        .and_then(|v| basic_realm(&v))
                        .unwrap_or_default();
                    if self
                        .headers_for(&url)
                        .is_some_and(|h| h.has("authorization"))
                    {
                        // The caller's own authorization was refused; Basic would not be sent.
                        return Err(StreamError::AuthRequired { realm });
                    }
                    if let Some((r, _)) = &auth
                        && *r == realm
                    {
                        // The credentials were refused: forget them and ask once more.
                        self.credentials.forget(&realm);
                    }
                    if asked.contains(&realm) {
                        return Err(StreamError::AuthRequired { realm });
                    }
                    let (creds, prompted) = self
                        .credentials
                        .get(&url.to_string(), &realm)
                        .ok_or_else(|| StreamError::AuthRequired {
                            realm: realm.clone(),
                        })?;
                    if prompted {
                        asked.push(realm.clone());
                    }
                    auth = Some((realm, creds));
                }
                407 => return Err(StreamError::ProxyAuth),
                _ => {
                    return Err(StreamError::Status {
                        code: status,
                        reason,
                    });
                }
            }
        }
    }

    fn proxy_for(&self, url: &Url) -> Option<&super::Proxy> {
        let p = self.config.proxy.as_ref()?;
        (!self.config.proxy_only_port_80 || url.port == 80).then_some(p)
    }

    fn connect(&self, host: &str, port: u16) -> Result<TcpStream, StreamError> {
        let addrs = (host, port)
            .to_socket_addrs()
            .map_err(|e| StreamError::Connect(format!("{host}: {e}")))?;
        let mut last = StreamError::Connect(format!("{host}: no address"));
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, self.config.connect_timeout) {
                Ok(s) => {
                    s.set_read_timeout(Some(self.config.read_timeout)).ok();
                    s.set_write_timeout(Some(self.config.read_timeout)).ok();
                    s.set_nodelay(true).ok();
                    (self.on_connect)(&s);
                    return Ok(s);
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => last = StreamError::Timeout,
                Err(e) => last = StreamError::Connect(format!("{host}: {e}")),
            }
        }
        Err(last)
    }

    /// Sends one request and reads the status line and headers.
    fn send(
        &self,
        url: &Url,
        range_from: u64,
        basic: Option<&(String, String)>,
    ) -> Result<(u16, String, Vec<(String, String)>, BufReader<Conn>), StreamError> {
        let proxy = self.proxy_for(url);
        let mut extra = String::new();
        let (tcp, target) = match proxy {
            Some(p) => {
                let tcp = self.connect(&p.host, p.port)?;
                let auth = p.user.as_ref().map(|(u, pw)| {
                    format!(
                        "Proxy-Authorization: Basic {}\r\n",
                        base64(&format!("{u}:{pw}"))
                    )
                });
                if url.tls {
                    tunnel(&tcp, url, auth.as_deref().unwrap_or(""))?;
                    (tcp, url.path.clone())
                } else {
                    extra.push_str(auth.as_deref().unwrap_or(""));
                    (tcp, url.to_string())
                }
            }
            None => (self.connect(&url.host, url.port)?, url.path.clone()),
        };
        let mut conn = if url.tls {
            Conn::Tls(Box::new(tls_wrap(tcp, &url.host)?))
        } else {
            Conn::Plain(tcp)
        };
        let mut req = format!(
            "GET {target} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: */*\r\n\
             Icy-MetaData: 1\r\nConnection: close\r\n",
            url.authority(false),
            self.config.user_agent
        );
        if range_from > 0 {
            req.push_str(&format!("Range: bytes={range_from}-\r\n"));
        }
        let origin = self.headers_for(url);
        if let Some((u, p)) = basic.filter(|_| !origin.is_some_and(|h| h.has("authorization"))) {
            req.push_str(&format!(
                "Authorization: Basic {}\r\n",
                base64(&format!("{u}:{p}"))
            ));
        }
        for (k, v) in origin.iter().flat_map(|h| &h.headers) {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str(&extra);
        req.push_str("\r\n");
        conn.write_all(req.as_bytes()).map_err(io_error)?;
        conn.flush().map_err(io_error)?;
        let mut reader = BufReader::with_capacity(16 * 1024, conn);
        let (status, reason) = read_status(&mut reader)?;
        let headers = read_headers(&mut reader)?;
        Ok((status, reason, headers, reader))
    }
}

fn body_mode(status: u16, headers: &[(String, String)]) -> BodyMode {
    let get = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v);
    if status == 204 {
        return BodyMode::Length(0);
    }
    if get("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        return BodyMode::Chunked {
            left: 0,
            done: false,
        };
    }
    match get("content-length").and_then(|v| v.trim().parse().ok()) {
        Some(n) => BodyMode::Length(n),
        None => BodyMode::UntilClose,
    }
}

/// Parses `HTTP/1.x NNN reason` or `ICY NNN reason`.
fn read_status(r: &mut impl BufRead) -> Result<(u16, String), StreamError> {
    let mut line = String::new();
    for _ in 0..8 {
        line = read_line(r).map_err(|e| match e.kind() {
            io::ErrorKind::UnexpectedEof => StreamError::BadResponse("no response".into()),
            _ => io_error(e),
        })?;
        if !line.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.trim().splitn(3, ' ');
    let proto = parts.next().unwrap_or("");
    let code = parts.next().and_then(|c| c.parse::<u16>().ok());
    let reason = parts.next().unwrap_or("").trim().to_string();
    match code {
        Some(code) if proto.starts_with("HTTP/1.") || proto.eq_ignore_ascii_case("ICY") => {
            Ok((code, reason))
        }
        _ => Err(StreamError::BadResponse(format!("status line {line:?}"))),
    }
}

fn read_headers(r: &mut impl BufRead) -> Result<Vec<(String, String)>, StreamError> {
    let mut headers = Vec::new();
    loop {
        let line = match read_line(r) {
            Ok(l) => l,
            // Old servers may close right after the headers.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(io_error(e)),
        };
        if line.trim().is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(StreamError::BadResponse("too many headers".into()));
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    Ok(headers)
}

/// Opens a `CONNECT` tunnel through an HTTP proxy.
fn tunnel(tcp: &TcpStream, url: &Url, auth: &str) -> Result<(), StreamError> {
    let mut s = tcp;
    let target = url.authority(true);
    write!(
        s,
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{auth}\r\n"
    )
    .map_err(io_error)?;
    // Read byte by byte so nothing after the headers is consumed.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && !head.ends_with(b"\n\n") {
        if s.read(&mut byte).map_err(io_error)? == 0 || head.len() > 64 * 1024 {
            return Err(StreamError::BadResponse("proxy closed the tunnel".into()));
        }
        head.push(byte[0]);
    }
    let mut r = &head[..];
    let (status, reason) = read_status(&mut r)?;
    match status {
        200..=299 => Ok(()),
        407 => Err(StreamError::ProxyAuth),
        _ => Err(StreamError::Status {
            code: status,
            reason,
        }),
    }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("default TLS versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

fn tls_wrap(
    tcp: TcpStream,
    host: &str,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, StreamError> {
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| StreamError::Tls(e.to_string()))?;
    let conn = rustls::ClientConnection::new(tls_config(), name)
        .map_err(|e| StreamError::Tls(e.to_string()))?;
    let mut stream = rustls::StreamOwned::new(conn, tcp);
    // Finish the handshake now so certificate errors are reported as TLS errors.
    while stream.conn.is_handshaking() {
        stream
            .conn
            .complete_io(&mut stream.sock)
            .map_err(|e| match e.kind() {
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => StreamError::Timeout,
                _ => StreamError::Tls(e.to_string()),
            })?;
    }
    Ok(stream)
}

/// The realm of a `WWW-Authenticate: Basic realm="…"` challenge.
fn basic_realm(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let i = lower.find("realm=")?;
    let rest = &value[i + 6..];
    Some(match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or("").to_string(),
        None => rest.split([',', ' ']).next().unwrap_or("").to_string(),
    })
}

/// Standard Base64 with padding.
pub(crate) fn base64(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encodes() {
        assert_eq!(base64(""), "");
        assert_eq!(base64("f"), "Zg==");
        assert_eq!(base64("fo"), "Zm8=");
        assert_eq!(base64("foo"), "Zm9v");
        assert_eq!(
            base64("Aladdin:open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn realms() {
        assert_eq!(
            basic_realm("Basic realm=\"My Radio\"").as_deref(),
            Some("My Radio")
        );
        assert_eq!(
            basic_realm("Basic Realm=x, charset=y").as_deref(),
            Some("x")
        );
        assert_eq!(basic_realm("Basic"), None);
    }

    #[test]
    fn status_lines() {
        let mut r = &b"ICY 200 OK\r\nicy-name: R\r\n\r\n"[..];
        assert_eq!(read_status(&mut r).unwrap(), (200, "OK".into()));
        assert_eq!(
            read_headers(&mut r).unwrap(),
            vec![("icy-name".into(), "R".into())]
        );
        let mut r = &b"\r\nHTTP/1.0 302 Found\n"[..];
        assert_eq!(read_status(&mut r).unwrap(), (302, "Found".into()));
        let mut r = &b"SIP/2.0 200 OK\r\n"[..];
        assert!(read_status(&mut r).is_err());
    }

    #[test]
    fn chunked_body() {
        let data = b"4;x=1\r\nabcd\r\n2\r\nef\r\n0\r\nT: v\r\n\r\n";
        let mut body = Body {
            reader: BufReader::new(Conn::Plain(loopback_with(data))),
            mode: BodyMode::Chunked {
                left: 0,
                done: false,
            },
        };
        let mut out = Vec::new();
        body.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"abcdef");
    }

    /// A socket whose peer sends `data` and closes.
    fn loopback_with(data: &[u8]) -> TcpStream {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (s, _) = l.accept().unwrap();
        c.write_all(data).unwrap();
        drop(c);
        s
    }
}
