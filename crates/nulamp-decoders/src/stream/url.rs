//! Stream locations (07 §8.1): `http://`, `https://`, and the SHOUTcast schemes that mean
//! HTTP to the same host and port.

use std::fmt;

use nulamp_playlist::text::percent_decode;

use super::StreamError;

/// URL schemes the stream input claims.
pub const SCHEMES: &[&str] = &["http", "https", "icy", "sc", "shoutcast", "uvox"];

/// True when `location` uses one of [`SCHEMES`].
pub fn is_stream_location(location: &str) -> bool {
    location
        .split_once("://")
        .is_some_and(|(s, _)| SCHEMES.iter().any(|k| s.eq_ignore_ascii_case(k)))
}

/// A parsed HTTP location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub tls: bool,
    /// Host name or address, without IPv6 brackets.
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
    /// `user:password@` from the URL.
    pub user: Option<(String, String)>,
}

impl Url {
    pub fn parse(location: &str) -> Result<Url, StreamError> {
        let bad = || StreamError::BadUrl(location.to_string());
        let (scheme, rest) = location.trim().split_once("://").ok_or_else(bad)?;
        let tls = match scheme.to_ascii_lowercase().as_str() {
            "https" => true,
            "http" | "icy" | "sc" | "shoutcast" | "uvox" => false,
            _ => return Err(bad()),
        };
        let rest = rest.split('#').next().unwrap_or("");
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let path = match path.strip_prefix('?') {
            Some(_) => format!("/{path}"),
            None => path.replace(' ', "%20"),
        };
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, authority),
        };
        let user = userinfo.map(|u| {
            let (name, pass) = u.split_once(':').unwrap_or((u, ""));
            (percent_decode(name), percent_decode(pass))
        });
        let default_port = if tls { 443 } else { 80 };
        let (host, port) = split_host_port(hostport, default_port).ok_or_else(bad)?;
        if host.is_empty() {
            return Err(bad());
        }
        Ok(Url {
            tls,
            host,
            port,
            path,
            user,
        })
    }

    fn default_port(&self) -> u16 {
        if self.tls { 443 } else { 80 }
    }

    /// `host[:port]` as sent in `Host` and `CONNECT`, port omitted when it is the default.
    pub fn authority(&self, always_port: bool) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if always_port || self.port != self.default_port() {
            format!("{host}:{}", self.port)
        } else {
            host
        }
    }

    /// Resolves a `Location` header against this URL.
    pub fn join(&self, location: &str) -> Result<Url, StreamError> {
        let location = location.trim();
        if location.contains("://") {
            return Url::parse(location);
        }
        let scheme = if self.tls { "https" } else { "http" };
        if let Some(rest) = location.strip_prefix("//") {
            return Url::parse(&format!("{scheme}://{rest}"));
        }
        let mut url = self.clone();
        url.user = None;
        url.path = if location.starts_with('/') {
            location.to_string()
        } else if location.starts_with('?') {
            let base = self.path.split('?').next().unwrap_or("/");
            format!("{base}{location}")
        } else {
            let base = self.path.split('?').next().unwrap_or("/");
            let dir = &base[..base.rfind('/').map_or(0, |i| i + 1)];
            format!("{dir}{location}")
        };
        url.path = url.path.replace(' ', "%20");
        Ok(url)
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.tls { "https" } else { "http" };
        write!(f, "{scheme}://{}{}", self.authority(false), self.path)
    }
}

/// Splits `host[:port]` or `[v6][:port]`.
pub(crate) fn split_host_port(s: &str, default_port: u16) -> Option<(String, u16)> {
    if let Some(rest) = s.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if after.is_empty() => default_port,
            None => return None,
        };
        return Some((host.to_string(), port));
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => Some((h.to_string(), p.parse().ok()?)),
        Some(_) => Some((s.to_string(), default_port)),
        None => Some((s.to_string(), default_port)),
    }
}

/// The title of a location before any header arrives: the last path part, or the whole URL
/// (07 §8.4).
pub fn location_title(location: &str) -> String {
    let after_scheme = location.split_once("://").map_or(location, |(_, r)| r);
    let path = after_scheme.split(['?', '#']).next().unwrap_or("");
    let part = path
        .find('/')
        .map(|i| &path[i..])
        .and_then(|p| p.rsplit('/').find(|s| !s.is_empty()));
    match part {
        Some(p) => percent_decode(p),
        None => location.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        let u = Url::parse("icy://u:p%40ss@Radio.example:8000/live?x=1#f").unwrap();
        assert!(!u.tls);
        assert_eq!(u.host, "Radio.example");
        assert_eq!(u.port, 8000);
        assert_eq!(u.path, "/live?x=1");
        assert_eq!(u.user, Some(("u".into(), "p@ss".into())));
        assert_eq!(u.to_string(), "http://Radio.example:8000/live?x=1");
        let u = Url::parse("https://h").unwrap();
        assert_eq!((u.port, u.path.as_str()), (443, "/"));
        assert_eq!(u.authority(false), "h");
        assert_eq!(u.authority(true), "h:443");
        let u = Url::parse("http://[::1]:81?q").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 81));
        assert_eq!(u.path, "/?q");
        assert_eq!(u.authority(false), "[::1]:81");
        assert!(Url::parse("ftp://h/x").is_err());
        assert!(Url::parse("http://:80/x").is_err());
        assert!(Url::parse("http://h:x/").is_err());
    }

    #[test]
    fn joins_redirects() {
        let u = Url::parse("http://h:81/a/b.pls?q=1").unwrap();
        assert_eq!(u.join("c.mp3").unwrap().to_string(), "http://h:81/a/c.mp3");
        assert_eq!(u.join("/s").unwrap().to_string(), "http://h:81/s");
        assert_eq!(u.join("?z").unwrap().to_string(), "http://h:81/a/b.pls?z");
        assert_eq!(u.join("//o/p").unwrap().to_string(), "http://o/p");
        assert_eq!(
            u.join("https://o:9/p").unwrap().to_string(),
            "https://o:9/p"
        );
    }

    #[test]
    fn schemes_and_titles() {
        assert!(is_stream_location("SHOUTCAST://x"));
        assert!(is_stream_location("uvox://x"));
        assert!(!is_stream_location("file:///x.mp3"));
        assert!(!is_stream_location("/x.mp3"));
        assert_eq!(
            location_title("http://h/a/My%20Song.mp3?x=1"),
            "My Song.mp3"
        );
        assert_eq!(location_title("http://h/a/"), "a");
        assert_eq!(location_title("http://h:8000"), "http://h:8000");
        assert_eq!(location_title("http://h/?sid=1"), "http://h/?sid=1");
    }
}
