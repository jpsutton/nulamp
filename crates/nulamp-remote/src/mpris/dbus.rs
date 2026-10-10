//! The D-Bus side of [`Mpris`], built on `zbus`'s blocking API.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use zbus::blocking::connection::Builder;
use zbus::fdo;
use zbus::names::InterfaceName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use super::{
    BUS_NAME, OBJECT_PATH, capabilities, changed_properties, is_seek, location_to_url, loop_status,
    loop_status_requests, playback_status, position_at, track_path, url_to_location,
    volume_from_mpris, volume_to_mpris,
};
use crate::{Error, Flag, PlayerSnapshot, Request, RequestSender, Result};

const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Self {
        Error::Bus(e.to_string())
    }
}

/// The last snapshot and when it arrived.
struct Shared {
    snapshot: PlayerSnapshot,
    at: Instant,
}

type SharedRef = Arc<Mutex<Shared>>;

fn lock(s: &SharedRef) -> MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// `Metadata` value (`a{sv}`).
pub(crate) fn metadata(s: &PlayerSnapshot) -> HashMap<String, OwnedValue> {
    let mut m = HashMap::new();
    let path = ObjectPath::try_from(track_path(s.track_id)).expect("valid object path");
    m.insert("mpris:trackid".into(), OwnedValue::from(path));
    if s.track_id.is_none() {
        return m;
    }
    if let Some(len) = s.length_ms {
        m.insert(
            "mpris:length".into(),
            OwnedValue::from(len.saturating_mul(1000) as i64),
        );
    }
    let text = |v: &str| OwnedValue::try_from(Value::from(v.to_string())).expect("string value");
    if !s.title.is_empty() {
        m.insert("xesam:title".into(), text(&s.title));
    }
    if !s.artist.is_empty() {
        let artists = Value::from(vec![s.artist.clone()]);
        m.insert(
            "xesam:artist".into(),
            OwnedValue::try_from(artists).expect("array value"),
        );
    }
    if !s.album.is_empty() {
        m.insert("xesam:album".into(), text(&s.album));
    }
    if !s.location.is_empty() {
        m.insert("xesam:url".into(), text(&location_to_url(&s.location)));
    }
    m
}

/// Value of a property named by [`changed_properties`].
fn property_value(name: &str, s: &PlayerSnapshot) -> Value<'static> {
    let caps = capabilities(s);
    match name {
        "PlaybackStatus" => Value::from(playback_status(s.state)),
        "LoopStatus" => Value::from(loop_status(s.repeat, s.manual_advance)),
        "Shuffle" => Value::from(s.shuffle),
        "Metadata" => Value::from(metadata(s)),
        "Volume" => Value::from(volume_to_mpris(s.volume)),
        "CanGoNext" => Value::from(caps[0]),
        "CanGoPrevious" => Value::from(caps[1]),
        "CanPlay" => Value::from(caps[2]),
        "CanPause" => Value::from(caps[3]),
        _ => Value::from(caps[4]),
    }
}

/// `org.mpris.MediaPlayer2`.
struct Root {
    requests: RequestSender,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        self.requests.post(Request::Raise);
    }

    fn quit(&self) {
        self.requests.post(Request::Quit);
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn identity(&self) -> &str {
        "nulamp"
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn desktop_entry(&self) -> &str {
        "nulamp"
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn supported_uri_schemes(&self) -> Vec<String> {
        ["file", "http", "https"].map(String::from).to_vec()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn supported_mime_types(&self) -> Vec<String> {
        [
            "audio/mpeg",
            "audio/flac",
            "audio/ogg",
            "audio/x-vorbis+ogg",
            "audio/opus",
            "audio/wav",
            "audio/x-wav",
            "audio/aac",
            "audio/mp4",
            "audio/x-mpegurl",
            "audio/x-scpls",
        ]
        .map(String::from)
        .to_vec()
    }
}

/// `org.mpris.MediaPlayer2.Player`.
struct Player {
    requests: RequestSender,
    shared: SharedRef,
}

impl Player {
    fn post(&self, r: Request) {
        self.requests.post(r);
    }

    fn snapshot(&self) -> PlayerSnapshot {
        lock(&self.shared).snapshot.clone()
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.post(Request::Next);
    }

    fn previous(&self) {
        self.post(Request::Previous);
    }

    fn pause(&self) {
        self.post(Request::Pause);
    }

    fn play_pause(&self) {
        self.post(Request::PlayPause);
    }

    fn stop(&self) {
        self.post(Request::Stop);
    }

    fn play(&self) {
        self.post(Request::Play);
    }

    /// Offset in microseconds.
    fn seek(&self, offset: i64) {
        self.post(Request::SeekRelative(offset / 1000));
    }

    /// Ignored unless `track_id` is the current track and the position is within it.
    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        let s = self.snapshot();
        let ms = position / 1000;
        if track_id.as_str() != track_path(s.track_id)
            || s.track_id.is_none()
            || ms < 0
            || s.length_ms.is_some_and(|l| ms as u64 > l)
        {
            return;
        }
        self.post(Request::SeekTo(ms as u64));
    }

    fn open_uri(&self, uri: String) {
        self.post(Request::Open {
            files: vec![url_to_location(&uri)],
            enqueue: false,
        });
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> &'static str {
        playback_status(lock(&self.shared).snapshot.state)
    }

    #[zbus(property)]
    fn loop_status(&self) -> &'static str {
        let s = lock(&self.shared);
        loop_status(s.snapshot.repeat, s.snapshot.manual_advance)
    }

    #[zbus(property)]
    fn set_loop_status(&self, value: String) -> fdo::Result<()> {
        let requests = loop_status_requests(&value)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("unknown loop status {value}")))?;
        {
            let mut s = lock(&self.shared);
            for r in &requests {
                match r {
                    Request::SetRepeat(f) => s.snapshot.repeat = f.apply(s.snapshot.repeat),
                    Request::SetManualAdvance(f) => {
                        s.snapshot.manual_advance = f.apply(s.snapshot.manual_advance)
                    }
                    _ => {}
                }
            }
        }
        requests.into_iter().for_each(|r| self.post(r));
        Ok(())
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    /// The rate is fixed; writes are ignored.
    #[zbus(property)]
    fn set_rate(&self, _rate: f64) {}

    #[zbus(property(emits_changed_signal = "const"))]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        lock(&self.shared).snapshot.shuffle
    }

    #[zbus(property)]
    fn set_shuffle(&self, on: bool) {
        lock(&self.shared).snapshot.shuffle = on;
        self.post(Request::SetShuffle(Flag::from(on)));
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&lock(&self.shared).snapshot)
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        volume_to_mpris(lock(&self.shared).snapshot.volume)
    }

    #[zbus(property)]
    fn set_volume(&self, volume: f64) {
        let v = volume_from_mpris(volume);
        lock(&self.shared).snapshot.volume = v;
        self.post(Request::SetVolume(v));
    }

    /// Microseconds.
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        let s = lock(&self.shared);
        position_at(&s.snapshot, s.at.elapsed()).saturating_mul(1000) as i64
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        capabilities(&lock(&self.shared).snapshot)[0]
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        capabilities(&lock(&self.shared).snapshot)[1]
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        capabilities(&lock(&self.shared).snapshot)[2]
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        capabilities(&lock(&self.shared).snapshot)[3]
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        capabilities(&lock(&self.shared).snapshot)[4]
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }
}

/// The exported MPRIS 2 player. Dropping it closes the bus connection.
pub struct Mpris {
    conn: zbus::blocking::Connection,
    shared: SharedRef,
    bus_name: String,
}

impl Mpris {
    /// Connects to the session bus and exports the player. Fails when there is no session bus.
    pub fn start(requests: RequestSender) -> Result<Self> {
        Self::start_with(Builder::session()?, requests)
    }

    /// Like [`start`](Self::start), on the bus at a D-Bus address.
    pub fn start_at(address: &str, requests: RequestSender) -> Result<Self> {
        Self::start_with(Builder::address(address)?, requests)
    }

    fn start_with(builder: Builder<'_>, requests: RequestSender) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            snapshot: PlayerSnapshot::default(),
            at: Instant::now(),
        }));
        let conn = builder
            .serve_at(
                OBJECT_PATH,
                Root {
                    requests: requests.clone(),
                },
            )?
            .serve_at(
                OBJECT_PATH,
                Player {
                    requests,
                    shared: shared.clone(),
                },
            )?
            .build()?;
        // Another player must not take the name over, and this one must not queue for it.
        let flags = || fdo::RequestNameFlags::DoNotQueue.into();
        let bus_name = match conn.request_name_with_flags(BUS_NAME, flags()) {
            Ok(_) => BUS_NAME.to_string(),
            Err(zbus::Error::NameTaken) => {
                let name = format!("{BUS_NAME}.instance{}", std::process::id());
                conn.request_name_with_flags(name.as_str(), flags())?;
                name
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            conn,
            shared,
            bus_name,
        })
    }

    /// The bus name the player is exported under.
    pub fn bus_name(&self) -> &str {
        &self.bus_name
    }

    /// Takes the player's state; emits `PropertiesChanged` for changed properties and `Seeked`
    /// when the position jumped.
    pub fn update(&self, snapshot: &PlayerSnapshot) -> Result<()> {
        let now = Instant::now();
        let (changed, seek) = {
            let mut s = lock(&self.shared);
            let seek = is_seek(&s.snapshot, now.saturating_duration_since(s.at), snapshot);
            let changed = changed_properties(&s.snapshot, snapshot);
            s.snapshot = snapshot.clone();
            s.at = now;
            (changed, seek)
        };
        let emitter = SignalEmitter::new(self.conn.inner(), OBJECT_PATH)?;
        if !changed.is_empty() {
            let values: HashMap<&str, Value<'_>> = changed
                .iter()
                .map(|&n| (n, property_value(n, snapshot)))
                .collect();
            zbus::block_on(fdo::Properties::properties_changed(
                &emitter,
                InterfaceName::from_static_str_unchecked(PLAYER_IFACE),
                values,
                Cow::Borrowed(&[]),
            ))?;
        }
        if seek {
            let us = snapshot.position_ms.saturating_mul(1000) as i64;
            zbus::block_on(Player::seeked(&emitter, us))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Response, channel};
    use nulamp_core::PlayState;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::Receiver;
    use std::time::Duration;
    use zbus::blocking::{Connection, MessageIterator, Proxy};

    fn snap() -> PlayerSnapshot {
        PlayerSnapshot {
            state: PlayState::Playing,
            position_ms: 2_000,
            length_ms: Some(180_000),
            seekable: true,
            title: "Song".into(),
            artist: "Band".into(),
            album: "Record".into(),
            location: "/m/a b.mp3".into(),
            volume: 51,
            repeat: true,
            playlist_len: 4,
            track_id: Some(7),
            ..Default::default()
        }
    }

    fn get<T: TryFrom<OwnedValue>>(m: &HashMap<String, OwnedValue>, k: &str) -> T
    where
        T::Error: std::fmt::Debug,
    {
        T::try_from(m[k].try_clone().unwrap()).unwrap()
    }

    #[test]
    fn metadata_fields() {
        let m = metadata(&snap());
        assert_eq!(
            get::<ObjectPath<'_>>(&m, "mpris:trackid").as_str(),
            "/org/nulamp/track/7"
        );
        assert_eq!(get::<i64>(&m, "mpris:length"), 180_000_000);
        assert_eq!(get::<String>(&m, "xesam:title"), "Song");
        assert_eq!(get::<Vec<String>>(&m, "xesam:artist"), vec!["Band"]);
        assert_eq!(get::<String>(&m, "xesam:album"), "Record");
        assert_eq!(get::<String>(&m, "xesam:url"), "file:///m/a%20b.mp3");
        assert_eq!(m.len(), 6);

        let stream = PlayerSnapshot {
            length_ms: None,
            artist: String::new(),
            ..snap()
        };
        let m = metadata(&stream);
        assert!(!m.contains_key("mpris:length") && !m.contains_key("xesam:artist"));

        let none = metadata(&PlayerSnapshot::default());
        assert_eq!(none.len(), 1);
        assert_eq!(
            get::<ObjectPath<'_>>(&none, "mpris:trackid").as_str(),
            super::super::NO_TRACK
        );
    }

    /// A private bus; `None` when `dbus-daemon` cannot be started.
    struct Bus {
        child: Child,
        address: String,
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn private_bus() -> Option<Bus> {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut line = String::new();
        let ok = BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .is_ok();
        let address = line.trim().to_string();
        if !ok || address.is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        Some(Bus { child, address })
    }

    fn next(rx: &Receiver<crate::Incoming>) -> Request {
        let (r, reply) = rx.recv_timeout(Duration::from_secs(5)).expect("request");
        reply.send(Response::Ok);
        r
    }

    #[test]
    fn exported_on_private_bus() {
        let Some(bus) = private_bus() else {
            eprintln!("dbus-daemon not available; skipping");
            return;
        };
        let (sender, rx) = channel();
        let mpris = Mpris::start_at(&bus.address, sender.clone()).unwrap();
        assert_eq!(mpris.bus_name(), BUS_NAME);
        let second = Mpris::start_at(&bus.address, sender).unwrap();
        assert_eq!(
            second.bus_name(),
            format!("{BUS_NAME}.instance{}", std::process::id())
        );
        drop(second);

        let client = Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .unwrap();
        let player = proxy(&client, PLAYER_IFACE);
        let root = proxy(&client, "org.mpris.MediaPlayer2");
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .path(OBJECT_PATH)
            .unwrap()
            .build();
        let mut signals = MessageIterator::for_match_rule(rule, &client, None).unwrap();

        assert_eq!(root.get_property::<String>("Identity").unwrap(), "nulamp");
        assert!(root.get_property::<bool>("CanRaise").unwrap());
        assert_eq!(
            player.get_property::<String>("PlaybackStatus").unwrap(),
            "Stopped"
        );
        assert!(!player.get_property::<bool>("CanGoNext").unwrap());

        mpris.update(&snap()).unwrap();
        let msg = signals.next().unwrap().unwrap();
        let (iface, changed, _): (String, HashMap<String, OwnedValue>, Vec<String>) =
            msg.body().deserialize().unwrap();
        assert_eq!(iface, PLAYER_IFACE);
        assert_eq!(get::<String>(&changed, "PlaybackStatus"), "Playing");
        assert_eq!(get::<String>(&changed, "LoopStatus"), "Playlist");
        assert!(changed.contains_key("Metadata") && !changed.contains_key("Shuffle"));

        assert_eq!(
            player.get_property::<String>("PlaybackStatus").unwrap(),
            "Playing"
        );
        assert_eq!(player.get_property::<f64>("Volume").unwrap(), 0.2);
        assert_eq!(player.get_property::<f64>("Rate").unwrap(), 1.0);
        assert!(player.get_property::<bool>("CanSeek").unwrap());
        assert!(player.get_property::<i64>("Position").unwrap() >= 2_000_000);
        let meta: HashMap<String, OwnedValue> = player.get_property("Metadata").unwrap();
        assert_eq!(get::<String>(&meta, "xesam:title"), "Song");

        mpris
            .update(&PlayerSnapshot {
                position_ms: 90_000,
                ..snap()
            })
            .unwrap();
        let msg = signals.next().unwrap().unwrap();
        assert_eq!(msg.header().member().unwrap().as_str(), "Seeked");
        assert_eq!(msg.body().deserialize::<i64>().unwrap(), 90_000_000);

        for (method, req) in [
            ("Next", Request::Next),
            ("Previous", Request::Previous),
            ("Pause", Request::Pause),
            ("PlayPause", Request::PlayPause),
            ("Stop", Request::Stop),
            ("Play", Request::Play),
        ] {
            player.call_method(method, &()).unwrap();
            assert_eq!(next(&rx), req, "{method}");
        }
        player.call_method("Seek", &(-5_000_000i64)).unwrap();
        assert_eq!(next(&rx), Request::SeekRelative(-5000));
        let track = ObjectPath::try_from("/org/nulamp/track/7").unwrap();
        player
            .call_method("SetPosition", &(&track, 30_000_000i64))
            .unwrap();
        assert_eq!(next(&rx), Request::SeekTo(30_000));
        let other = ObjectPath::try_from("/org/nulamp/track/8").unwrap();
        player.call_method("SetPosition", &(&other, 1i64)).unwrap();
        player
            .call_method("OpenUri", &("file:///m/x%20y.ogg",))
            .unwrap();
        assert_eq!(
            next(&rx),
            Request::Open {
                files: vec!["/m/x y.ogg".into()],
                enqueue: false
            }
        );
        root.call_method("Raise", &()).unwrap();
        assert_eq!(next(&rx), Request::Raise);
        root.call_method("Quit", &()).unwrap();
        assert_eq!(next(&rx), Request::Quit);

        player.set_property("Volume", 1.0f64).unwrap();
        assert_eq!(next(&rx), Request::SetVolume(255));
        assert_eq!(player.get_property::<f64>("Volume").unwrap(), 1.0);
        player.set_property("Shuffle", true).unwrap();
        assert_eq!(next(&rx), Request::SetShuffle(Flag::On));
        player.set_property("LoopStatus", "Track").unwrap();
        assert_eq!(next(&rx), Request::SetRepeat(Flag::On));
        assert_eq!(next(&rx), Request::SetManualAdvance(Flag::On));
        assert_eq!(
            player.get_property::<String>("LoopStatus").unwrap(),
            "Track"
        );
        assert!(player.set_property("LoopStatus", "Sometimes").is_err());
        assert!(rx.try_recv().is_err());
    }

    fn proxy<'a>(conn: &Connection, iface: &'static str) -> Proxy<'a> {
        zbus::blocking::proxy::Builder::new(conn)
            .destination(BUS_NAME)
            .unwrap()
            .path(OBJECT_PATH)
            .unwrap()
            .interface(iface)
            .unwrap()
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .unwrap()
    }
}
