//! HTTP access to a Jellyfin server: the `Authorization` header, JSON requests and errors.

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

/// Largest response body read (a full track listing of a big library is a few tens of MB).
const BODY_LIMIT: u64 = 256 * 1024 * 1024;

/// Client name sent to the server; shown in its dashboard next to the device.
const CLIENT: &str = "nulamp";

/// Makes a server address usable as a base URL: trims it, adds `http://` when no scheme is
/// given and drops trailing slashes. `None` when nothing is left.
pub fn normalize_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return None;
    }
    Some(if url.contains("://") {
        url.to_string()
    } else {
        format!("http://{url}")
    })
}

/// The device name shown by the server: the host name when known.
fn device_name() -> String {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| CLIENT.to_string())
}

/// A value inside the `MediaBrowser` header: quotes, commas and control characters removed.
fn header_value(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '"' | ',' | '\\') && !c.is_control())
        .collect()
}

/// The `Authorization` header value; `token` is empty before logging in.
pub fn authorization(device_id: &str, token: &str) -> String {
    let mut v = format!(
        "MediaBrowser Client=\"{CLIENT}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
        header_value(&device_name()),
        header_value(device_id),
        env!("CARGO_PKG_VERSION"),
    );
    if !token.is_empty() {
        v.push_str(&format!(", Token=\"{}\"", header_value(token)));
    }
    v
}

/// A new random device id (32 hex digits). Jellyfin keys sessions by device, so every login
/// uses its own.
pub fn new_device_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// One server, with or without an access token.
#[derive(Clone)]
pub(crate) struct Client {
    agent: ureq::Agent,
    base: String,
    auth: String,
}

impl Client {
    pub fn new(base: &str, device_id: &str, token: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            // A full listing of a large library can take a while to arrive.
            .timeout_global(Some(Duration::from_secs(300)))
            .http_status_as_error(false)
            .user_agent(concat!("nulamp/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
            base: base.to_string(),
            auth: authorization(device_id, token),
        }
    }

    pub fn authorization(&self) -> &str {
        &self.auth
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// GET `path` with `query`; the body of a 2xx answer, with its content type.
    pub fn get(&self, path: &str, query: &[(&str, String)]) -> Result<(String, Vec<u8>)> {
        let mut req = self
            .agent
            .get(self.url(path))
            .header("Authorization", &self.auth)
            .header("Accept", "application/json");
        for (k, v) in query {
            req = req.query(*k, v);
        }
        Self::finish(path, req.call())
    }

    /// POST `path` with `query` and an optional JSON body.
    pub fn post(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
    ) -> Result<(String, Vec<u8>)> {
        let mut req = self
            .agent
            .post(self.url(path))
            .header("Authorization", &self.auth)
            .header("Accept", "application/json");
        for (k, v) in query {
            req = req.query(*k, v);
        }
        let resp = match body {
            Some(b) => req
                .header("Content-Type", "application/json")
                .send(b.to_string().as_bytes()),
            None => req.send_empty(),
        };
        Self::finish(path, resp)
    }

    pub fn get_json<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let (_, body) = self.get(path, query)?;
        serde_json::from_slice(&body)
            .map_err(|e| Error::Provider(format!("unexpected answer from the server: {e}")))
    }

    fn finish(
        path: &str,
        resp: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<(String, Vec<u8>)> {
        let mut resp =
            resp.map_err(|e| Error::Provider(format!("cannot reach the server: {e}")))?;
        let status = resp.status().as_u16();
        match status {
            200..=299 => {}
            401 | 403 => {
                return Err(Error::Provider(
                    "the server refused the login (sign in again in the preferences)".into(),
                ));
            }
            404 => return Err(Error::NotFound(path.to_string())),
            _ => {
                return Err(Error::Provider(format!(
                    "the server answered {status} for {path}"
                )));
            }
        }
        let mime = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim().to_string())
            .unwrap_or_default();
        let body = resp
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_vec()
            .map_err(|e| Error::Provider(format!("reading the server's answer: {e}")))?;
        Ok((mime, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            normalize_url(" jf.lan:8096/ ").unwrap(),
            "http://jf.lan:8096"
        );
        assert_eq!(normalize_url("https://x/jf//").unwrap(), "https://x/jf");
        assert_eq!(normalize_url("  "), None);
    }

    #[test]
    fn header() {
        let h = authorization("dev\"1", "to,ken");
        assert!(h.starts_with("MediaBrowser Client=\"nulamp\""));
        assert!(h.contains("DeviceId=\"dev1\""));
        assert!(h.ends_with("Token=\"token\""));
        assert!(!authorization("d", "").contains("Token"));
    }

    #[test]
    fn device_ids() {
        let (a, b) = (new_device_id(), new_device_id());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
