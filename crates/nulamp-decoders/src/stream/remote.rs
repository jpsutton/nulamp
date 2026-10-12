//! A file on a web server read like a local file, with Range requests: the picture of a media
//! server's video (NET15). Unlike [`super::StreamDecoder`] it blocks: its reader (the video
//! window's worker thread) waits for the network, and [`Abort`] ends a wait at once.

use std::io::{self, Read, Seek, SeekFrom};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::http::{Body, Client, OriginHeaders};
use super::url::Url;
use super::{CredentialStore, StreamConfig, StreamError};
use crate::ffmpeg_io::ByteSource;

/// Forward seeks up to this far read on instead of making a new request.
const SKIP_LIMIT: u64 = 256 * 1024;

/// Stops a [`RemoteFile`] from another thread: the current request fails and no new one is
/// made.
#[derive(Debug, Default)]
pub(crate) struct Abort {
    aborted: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
}

impl Abort {
    pub fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
        let socket = self.socket.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = socket.as_ref() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }

    fn aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }

    /// Keeps the new connection's socket so [`Abort::abort`] can shut it down.
    fn watch(&self, s: &TcpStream) {
        if let Ok(c) = s.try_clone() {
            let mut socket = self.socket.lock().unwrap_or_else(|e| e.into_inner());
            if self.aborted() {
                let _ = c.shutdown(std::net::Shutdown::Both);
            }
            *socket = Some(c);
        }
    }
}

fn closed() -> io::Error {
    io::Error::other("closed")
}

/// One file on a server, read and seeked on demand. Extra headers go only to the location's
/// origin, as for streams (NET12).
pub(crate) struct RemoteFile {
    config: StreamConfig,
    credentials: Arc<CredentialStore>,
    url: Url,
    origin: Option<OriginHeaders>,
    abort: Arc<Abort>,
    body: Option<Body>,
    pos: u64,
    len: Option<u64>,
}

impl RemoteFile {
    pub fn new(
        location: &str,
        headers: &[(String, String)],
        config: StreamConfig,
        credentials: Arc<CredentialStore>,
    ) -> Result<Self, StreamError> {
        let url = Url::parse(location)?;
        let origin = OriginHeaders::new(&url, headers);
        Ok(Self {
            config,
            credentials,
            url,
            origin,
            abort: Arc::default(),
            body: None,
            pos: 0,
            len: None,
        })
    }

    /// The handle that stops this file's requests.
    pub fn abort_handle(&self) -> Arc<Abort> {
        self.abort.clone()
    }

    /// Requests the file from byte `at`.
    fn connect(&mut self, at: u64) -> io::Result<()> {
        if self.abort.aborted() {
            return Err(closed());
        }
        let abort = self.abort.clone();
        let on_connect = move |s: &TcpStream| abort.watch(s);
        let client = Client {
            config: &self.config,
            credentials: &self.credentials,
            on_connect: &on_connect,
            headers: self.origin.as_ref(),
        };
        let resp = client.get(&self.url, at).map_err(io::Error::other)?;
        if self.len.is_none() && (at == 0 || resp.status == 206) {
            self.len = resp.total_length();
        }
        let ranged = resp.status == 206;
        let mut body = resp.body;
        if at > 0 && !ranged {
            // The server ignored the Range request: read up to the offset.
            let skipped = io::copy(&mut (&mut body).take(at), &mut io::sink())?;
            if skipped < at {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
        self.body = Some(body);
        Ok(())
    }

    /// The length, asking the server when no request was made yet.
    fn len(&mut self) -> Option<u64> {
        if self.len.is_none() && self.body.is_none() {
            let _ = self.connect(self.pos);
        }
        self.len
    }
}

impl Read for RemoteFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.len.is_some_and(|l| self.pos >= l) {
            return Ok(0);
        }
        // A connection that drops is resumed once at the same offset.
        for retry in [false, true] {
            if self.body.is_none() {
                self.connect(self.pos)?;
            }
            let Some(body) = self.body.as_mut() else {
                return Err(closed());
            };
            match body.read(buf) {
                Ok(0) if !retry && self.len.is_some_and(|l| self.pos < l) => self.body = None,
                Ok(n) => {
                    self.pos += n as u64;
                    return Ok(n);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(e),
                Err(_) if self.abort.aborted() => return Err(closed()),
                Err(e) if retry => return Err(e),
                Err(e) => {
                    log::debug!("remote file: connection lost: {e}");
                    self.body = None;
                }
            }
        }
        Ok(0)
    }
}

impl Seek for RemoteFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => self.len().and_then(|l| l.checked_add_signed(d)),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad seek"))?;
        if target == self.pos {
            return Ok(target);
        }
        if let Some(body) = self.body.as_mut()
            && target > self.pos
            && target - self.pos <= SKIP_LIMIT
        {
            let want = target - self.pos;
            let skipped = io::copy(&mut body.take(want), &mut io::sink()).unwrap_or(0);
            self.pos += skipped;
            if skipped == want {
                return Ok(target);
            }
        }
        self.body = None;
        self.pos = target;
        Ok(target)
    }
}

impl ByteSource for RemoteFile {
    fn byte_len(&mut self) -> Option<u64> {
        self.len()
    }
}
