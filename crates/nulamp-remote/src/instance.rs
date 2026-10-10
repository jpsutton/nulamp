//! Single instance (11 §4.1): the local-socket server of the primary instance and the client
//! a second launch uses to reach it.
//!
//! Socket names (RC2): an abstract-namespace socket `nulamp-<uid>-<class>` on Linux, a named
//! pipe `\\.\pipe\nulamp-<user>-<class>` on Windows, and a socket file
//! `nulamp-<user>-<class>.sock` in `$XDG_RUNTIME_DIR` (else the temporary folder) elsewhere.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, mpsc};
use std::time::Duration;

use interprocess::ConnectWaitMode;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{
    ConnectOptions, GenericFilePath, GenericNamespaced, ListenerOptions, Name,
};

use crate::{Error, Request, Response, Result, VERSION};

/// Callback that wakes the user-interface event loop.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// A request delivered to the application, with the sender for its answer.
pub type Incoming = (Request, ReplySender);

/// Longest accepted message line.
const MAX_LINE: u64 = 16 << 20;
/// How long a connection waits for the application to answer a request.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// A server connection with no request for this long is closed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Default client timeout for connecting.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// Default client timeout for one request; longer than the server's reply timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// After [`Request::Kill`], the server exits the process if it still runs after this long.
pub const KILL_TIMEOUT: Duration = Duration::from_secs(3);

/// Where the answer to one request goes. Dropping it without sending answers [`Response::Ok`].
pub struct ReplySender(Option<mpsc::SyncSender<Response>>);

impl std::fmt::Debug for ReplySender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReplySender")
    }
}

impl ReplySender {
    /// A sender whose answer is discarded.
    pub fn none() -> Self {
        Self(None)
    }

    pub fn send(mut self, response: Response) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(response);
        }
    }
}

/// Queues requests for the application and wakes its event loop. Clones share the waker.
#[derive(Clone)]
pub struct RequestSender {
    tx: mpsc::Sender<Incoming>,
    waker: Arc<RwLock<Option<Waker>>>,
}

/// A request queue: the sender for producers (socket, MPRIS) and the receiver for the
/// application.
pub fn channel() -> (RequestSender, mpsc::Receiver<Incoming>) {
    let (tx, rx) = mpsc::channel();
    (
        RequestSender {
            tx,
            waker: Arc::default(),
        },
        rx,
    )
}

impl RequestSender {
    /// Sets the callback called after each queued request, and calls it once so requests
    /// queued before it was set are noticed.
    pub fn set_waker(&self, waker: impl Fn() + Send + Sync + 'static) {
        let waker: Waker = Arc::new(waker);
        *self.waker.write().unwrap_or_else(|e| e.into_inner()) = Some(waker.clone());
        waker();
    }

    /// Queues a request and returns the receiver of its answer, or `None` when the
    /// application no longer receives requests.
    pub fn submit(&self, request: Request) -> Option<mpsc::Receiver<Response>> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx.send((request, ReplySender(Some(tx)))).ok()?;
        self.wake();
        Some(rx)
    }

    /// Queues a request whose answer is not needed. Returns false when the application no
    /// longer receives requests.
    pub fn post(&self, request: Request) -> bool {
        let ok = self.tx.send((request, ReplySender::none())).is_ok();
        if ok {
            self.wake();
        }
        ok
    }

    fn wake(&self) {
        let waker = self.waker.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(w) = waker {
            w();
        }
    }
}

/// The outcome of [`connect_or_serve`].
pub enum Role {
    /// No instance was running: this process serves requests.
    Primary(Server),
    /// Another instance runs: send it the launch's requests, then exit.
    Secondary(Client),
}

/// Becomes the primary instance of `class`, or connects to the running one.
///
/// A socket left behind by a crashed instance is replaced.
pub fn connect_or_serve(class: &str) -> Result<Role> {
    connect_or_serve_at(&SocketName::for_class(class)?)
}

pub(crate) fn connect_or_serve_at(name: &SocketName) -> Result<Role> {
    const ATTEMPTS: u32 = 5;
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(50 * u64::from(attempt)));
        }
        let stale = match Client::connect_at(name, CONNECT_TIMEOUT) {
            Ok(c) => return Ok(Role::Secondary(c)),
            Err(Error::Io(e)) => {
                matches!(name, SocketName::File(_)) && e.kind() == io::ErrorKind::ConnectionRefused
            }
            Err(e) => return Err(e),
        };
        match Server::bind_at(name, stale) {
            Ok(s) => return Ok(Role::Primary(s)),
            // Another instance may have bound the name since; connect to it.
            Err(e) => last = Some(e),
        }
    }
    Err(last.map_or(Error::Closed, Error::Io))
}

/// Local-socket name of a class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SocketName {
    /// Abstract socket (Linux) or named pipe (Windows).
    Namespaced(String),
    /// Socket file.
    #[cfg_attr(
        any(target_os = "linux", target_os = "android", windows),
        allow(dead_code)
    )]
    File(PathBuf),
}

/// Keeps `[A-Za-z0-9._-]`, replaces other characters with `_`, at most 40 characters.
fn sanitize(s: &str) -> String {
    s.chars()
        .take(40)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

fn user_name() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    if user.is_empty() {
        "user".into()
    } else {
        sanitize(&user)
    }
}

impl SocketName {
    pub(crate) fn for_class(class: &str) -> Result<Self> {
        let class = sanitize(class);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let user = own_uid().map_or_else(user_name, |u| u.to_string());
            Ok(Self::Namespaced(format!("nulamp-{user}-{class}")))
        }
        #[cfg(windows)]
        {
            Ok(Self::Namespaced(format!("nulamp-{}-{class}", user_name())))
        }
        #[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
        {
            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .filter(|d| d.is_dir())
                .unwrap_or_else(std::env::temp_dir);
            Ok(Self::File(
                dir.join(format!("nulamp-{}-{class}.sock", user_name())),
            ))
        }
    }

    fn to_name(&self) -> Result<Name<'_>> {
        let name = match self {
            Self::Namespaced(s) => s.as_str().to_ns_name::<GenericNamespaced>(),
            Self::File(p) => p.as_path().to_fs_name::<GenericFilePath>(),
        };
        name.map_err(|e| Error::Name(e.to_string()))
    }
}

/// Fails with [`Error::ForeignPeer`] when the peer runs as another user. Abstract sockets have
/// no file permissions, so this check guards them; socket files and pipes rely on their
/// per-user folder and name.
fn check_peer(stream: &LocalSocketStream) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let peer = stream.peer_creds()?.euid();
        if own_uid().is_some_and(|u| peer != Some(u)) {
            return Err(Error::ForeignPeer);
        }
    }
    let _ = stream;
    Ok(())
}

/// Reads one `\n`-terminated line into `buf`. Returns false at end of stream.
fn read_line(r: &mut impl BufRead, buf: &mut Vec<u8>) -> io::Result<bool> {
    buf.clear();
    let n = r.by_ref().take(MAX_LINE).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(false);
    }
    if buf.last() != Some(&b'\n') && n as u64 == MAX_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too long",
        ));
    }
    Ok(true)
}

fn write_message(mut w: impl Write, msg: &impl serde::Serialize) -> Result<()> {
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()?;
    Ok(())
}

/// The primary instance's request server. An accept thread serves each connection on its own
/// thread and queues the requests; the application takes them from [`requests`](Self::requests).
pub struct Server {
    requests: mpsc::Receiver<Incoming>,
    sender: RequestSender,
    name: SocketName,
    shared: Arc<ServerShared>,
}

struct ServerShared {
    stop: AtomicBool,
    kill_timeout: RwLock<Option<Duration>>,
}

impl Server {
    fn bind_at(name: &SocketName, replace_stale: bool) -> io::Result<Self> {
        let n = name
            .to_name()
            .map_err(|e| io::Error::other(e.to_string()))?;
        let mut opts = ListenerOptions::new().name(n);
        if replace_stale {
            opts = opts
                .try_overwrite(true)
                .max_spin_time(Duration::from_millis(500));
        }
        let listener = opts.create_sync()?;
        let (sender, requests) = channel();
        let shared = Arc::new(ServerShared {
            stop: AtomicBool::new(false),
            kill_timeout: RwLock::new(Some(KILL_TIMEOUT)),
        });
        let (s, sh) = (sender.clone(), shared.clone());
        std::thread::Builder::new()
            .name("nulamp-remote-accept".into())
            .spawn(move || accept_loop(listener, s, sh))?;
        Ok(Self {
            requests,
            sender,
            name: name.clone(),
            shared,
        })
    }

    /// The queue's sender, for other producers such as [`crate::Mpris`].
    pub fn sender(&self) -> RequestSender {
        self.sender.clone()
    }

    /// Shortcut for [`RequestSender::set_waker`].
    pub fn set_waker(&self, waker: impl Fn() + Send + Sync + 'static) {
        self.sender.set_waker(waker);
    }

    /// The queued requests.
    pub fn requests(&self) -> &mpsc::Receiver<Incoming> {
        &self.requests
    }

    /// Takes the next queued request, if any.
    pub fn try_recv(&self) -> Option<Incoming> {
        self.requests.try_recv().ok()
    }

    /// Sets how long after [`Request::Kill`] the process is force-exited; `None` disables it.
    pub fn set_kill_timeout(&self, timeout: Option<Duration>) {
        *self
            .shared
            .kill_timeout
            .write()
            .unwrap_or_else(|e| e.into_inner()) = timeout;
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        // Wake the accept thread so it drops the listener and frees the name.
        if let Ok(n) = self.name.to_name() {
            let _ = ConnectOptions::new()
                .name(n)
                .wait_mode(ConnectWaitMode::Timeout(Duration::from_millis(200)))
                .connect_sync();
        }
    }
}

fn accept_loop(listener: LocalSocketListener, sender: RequestSender, shared: Arc<ServerShared>) {
    let mut errors = 0u32;
    for conn in listener.incoming() {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        match conn {
            Ok(stream) => {
                errors = 0;
                let (s, sh) = (sender.clone(), shared.clone());
                let spawned = std::thread::Builder::new()
                    .name("nulamp-remote-conn".into())
                    .spawn(move || serve_connection(stream, s, sh));
                if let Err(e) = spawned {
                    log::warn!("remote control: cannot start a connection thread: {e}");
                }
            }
            Err(e) => {
                log::warn!("remote control: accept failed: {e}");
                errors += 1;
                std::thread::sleep(Duration::from_millis(50 * u64::from(errors.min(20))));
            }
        }
    }
}

fn serve_connection(stream: LocalSocketStream, sender: RequestSender, shared: Arc<ServerShared>) {
    if let Err(e) = check_peer(&stream) {
        log::warn!("remote control: connection refused: {e}");
        return;
    }
    let _ = stream.set_recv_timeout(Some(IDLE_TIMEOUT));
    let _ = stream.set_send_timeout(Some(REPLY_TIMEOUT));
    let mut reader = BufReader::new(&stream);
    let mut line = Vec::new();
    loop {
        match read_line(&mut reader, &mut line) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                log::debug!("remote control: read failed: {e}");
                return;
            }
        }
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let response = match serde_json::from_slice::<Request>(&line) {
            Ok(request) => answer(request, &sender, &shared),
            Err(e) => Response::Error(format!("bad request: {e}")),
        };
        if let Err(e) = write_message(&stream, &response) {
            log::debug!("remote control: write failed: {e}");
            return;
        }
    }
}

fn answer(request: Request, sender: &RequestSender, shared: &ServerShared) -> Response {
    match request {
        Request::GetVersion => {
            return Response::Version {
                version: VERSION.into(),
                pid: std::process::id(),
            };
        }
        Request::Kill => {
            let timeout = *shared
                .kill_timeout
                .read()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(t) = timeout {
                std::thread::spawn(move || {
                    std::thread::sleep(t);
                    log::warn!("remote control: still running {t:?} after kill; exiting");
                    std::process::exit(1);
                });
            }
        }
        _ => {}
    }
    let Some(reply) = sender.submit(request) else {
        return Response::Error("the player is shutting down".into());
    };
    match reply.recv_timeout(REPLY_TIMEOUT) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Disconnected) => Response::Ok,
        Err(mpsc::RecvTimeoutError::Timeout) => Response::Error("the player did not answer".into()),
    }
}

/// A connection to the primary instance.
pub struct Client {
    /// `None` after a failed exchange.
    stream: Option<BufReader<LocalSocketStream>>,
    timeout: Duration,
}

impl Client {
    /// Connects to the primary instance of `class`.
    pub fn connect(class: &str, timeout: Duration) -> Result<Self> {
        Self::connect_at(&SocketName::for_class(class)?, timeout)
    }

    pub(crate) fn connect_at(name: &SocketName, timeout: Duration) -> Result<Self> {
        let stream = ConnectOptions::new()
            .name(name.to_name()?)
            .wait_mode(ConnectWaitMode::Timeout(timeout))
            .connect_sync()?;
        check_peer(&stream)?;
        Ok(Self {
            stream: Some(BufReader::new(stream)),
            timeout: REQUEST_TIMEOUT,
        })
    }

    /// Sets the time limit for one request and its answer ([`REQUEST_TIMEOUT`] by default).
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Sends a request and waits for its answer. After an i/o error or a timeout the
    /// connection is closed and later calls fail with [`Error::Closed`].
    pub fn send(&mut self, request: &Request) -> Result<Response> {
        let mut stream = self.stream.take().ok_or(Error::Closed)?;
        let mut line = serde_json::to_vec(request)?;
        line.push(b'\n');
        // Named pipes have no i/o timeouts, so the exchange runs on its own thread.
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("nulamp-remote-client".into())
            .spawn(move || {
                let r = exchange(&mut stream, &line);
                let _ = tx.send((stream, r));
            })?;
        let (stream, result) = rx.recv_timeout(self.timeout).map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "the running instance did not answer",
            )
        })?;
        if matches!(result, Ok(_) | Err(Error::Protocol(_))) {
            self.stream = Some(stream);
        }
        result
    }
}

fn exchange(stream: &mut BufReader<LocalSocketStream>, line: &[u8]) -> Result<Response> {
    let w = stream.get_mut();
    w.write_all(line)?;
    w.flush()?;
    let mut answer = Vec::new();
    if !read_line(stream, &mut answer)? {
        return Err(Error::Closed);
    }
    Ok(serde_json::from_slice(&answer)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn unique_class() -> String {
        static N: AtomicUsize = AtomicUsize::new(0);
        format!(
            "test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }

    fn primary(class: &str) -> Server {
        match connect_or_serve(class).unwrap() {
            Role::Primary(s) => s,
            Role::Secondary(_) => panic!("expected primary"),
        }
    }

    fn secondary(class: &str) -> Client {
        match connect_or_serve(class).unwrap() {
            Role::Secondary(c) => c,
            Role::Primary(_) => panic!("expected secondary"),
        }
    }

    /// Answers requests on a thread like the application's event loop would.
    fn run_app(server: Server, count: usize) -> std::thread::JoinHandle<Vec<Request>> {
        std::thread::spawn(move || {
            let mut seen = Vec::new();
            while seen.len() < count {
                let (req, reply) = server.requests().recv().unwrap();
                match &req {
                    Request::GetCurrentTitle => reply.send(Response::Title("Song".into())),
                    Request::GetEntry(9) => reply.send(Response::Error("no entry".into())),
                    _ => drop(reply),
                }
                seen.push(req);
            }
            seen
        })
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("a b/c\\d.e_f-g"), "a_b_c_d.e_f-g");
        assert_eq!(sanitize(&"x".repeat(100)).len(), 40);
    }

    #[test]
    fn class_names_differ() {
        let a = SocketName::for_class("a").unwrap();
        let b = SocketName::for_class("b").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn primary_then_secondary() {
        let class = unique_class();
        let server = primary(&class);
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        server.set_waker(move || {
            w.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        let app = run_app(server, 3);

        let mut client = secondary(&class);
        let open = Request::Open {
            files: vec!["/a b.mp3".into()],
            enqueue: false,
        };
        assert_eq!(client.send(&open).unwrap(), Response::Ok);
        assert_eq!(
            client.send(&Request::GetCurrentTitle).unwrap(),
            Response::Title("Song".into())
        );
        match client.send(&Request::GetVersion).unwrap() {
            Response::Version { version, pid } => {
                assert_eq!(version, VERSION);
                assert_eq!(pid, std::process::id());
            }
            r => panic!("{r:?}"),
        }
        let mut other = Client::connect(&class, CONNECT_TIMEOUT).unwrap();
        assert_eq!(
            other.send(&Request::GetEntry(9)).unwrap(),
            Response::Error("no entry".into())
        );

        let seen = app.join().unwrap();
        assert_eq!(
            seen,
            vec![open, Request::GetCurrentTitle, Request::GetEntry(9)]
        );
        assert!(woken.load(Ordering::SeqCst) >= 4);
    }

    #[test]
    fn bad_request_line() {
        let class = unique_class();
        let _server = primary(&class);
        let mut client = secondary(&class);
        let stream = client.stream.as_mut().unwrap();
        let r = exchange(stream, b"{\"cmd\":\"nope\"}\n").unwrap();
        assert!(matches!(r, Response::Error(_)), "{r:?}");
        // The connection stays usable.
        assert!(matches!(
            client.send(&Request::GetVersion).unwrap(),
            Response::Version { .. }
        ));
    }

    #[test]
    fn app_gone() {
        let (sender, rx) = channel();
        drop(rx);
        assert!(!sender.post(Request::Play));
        assert!(sender.submit(Request::Play).is_none());
        let shared = ServerShared {
            stop: AtomicBool::new(false),
            kill_timeout: RwLock::new(None),
        };
        assert!(matches!(
            answer(Request::Play, &sender, &shared),
            Response::Error(_)
        ));
    }

    #[test]
    fn name_is_released_on_drop() {
        let class = unique_class();
        drop(primary(&class));
        let mut ok = false;
        for _ in 0..20 {
            match connect_or_serve(&class).unwrap() {
                Role::Primary(_) => {
                    ok = true;
                    break;
                }
                Role::Secondary(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert!(ok);
    }

    #[test]
    fn post_and_reply_none() {
        let (sender, rx) = channel();
        assert!(sender.post(Request::Stop));
        let (req, reply) = rx.try_recv().unwrap();
        assert_eq!(req, Request::Stop);
        reply.send(Response::Ok);
        let answer = sender.submit(Request::GetStatus).unwrap();
        let (_, reply) = rx.try_recv().unwrap();
        drop(reply);
        assert!(answer.recv().is_err());
    }

    #[test]
    fn client_times_out() {
        let class = unique_class();
        let _server = primary(&class);
        let mut client = secondary(&class);
        client.set_timeout(Duration::from_millis(100));
        match client.send(&Request::Play) {
            Err(Error::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::TimedOut),
            r => panic!("{r:?}"),
        }
        assert!(matches!(client.send(&Request::Play), Err(Error::Closed)));
    }

    #[test]
    fn kill_is_forwarded() {
        let class = unique_class();
        let server = primary(&class);
        server.set_kill_timeout(None);
        let app = run_app(server, 1);
        let mut client = secondary(&class);
        assert_eq!(client.send(&Request::Kill).unwrap(), Response::Ok);
        assert_eq!(app.join().unwrap(), vec![Request::Kill]);
    }

    #[cfg(unix)]
    #[test]
    fn stale_socket_file_is_replaced() {
        let dir = std::env::temp_dir().join(format!("nulamp-remote-{}", unique_class()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.sock");
        // A listener that goes away without removing its file, as after a crash.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let name = SocketName::File(path.clone());
        let server = match connect_or_serve_at(&name).unwrap() {
            Role::Primary(s) => s,
            Role::Secondary(_) => panic!("stale socket accepted a connection"),
        };
        let app = run_app(server, 1);
        let mut client = match connect_or_serve_at(&name).unwrap() {
            Role::Secondary(c) => c,
            Role::Primary(_) => panic!("expected secondary"),
        };
        assert_eq!(client.send(&Request::Play).unwrap(), Response::Ok);
        assert_eq!(app.join().unwrap(), vec![Request::Play]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn request_line_limit() {
        let data = [b'x'; 10];
        let mut r = BufReader::new(&data[..]);
        let mut line = Vec::new();
        assert!(read_line(&mut r, &mut line).unwrap());
        assert_eq!(line.len(), 10);
        assert!(!read_line(&mut r, &mut line).unwrap());
    }
}
