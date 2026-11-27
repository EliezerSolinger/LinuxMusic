//! The `RemotePort` (I-15): the interface's `CorePort` over the network. It keeps a mirror of the tree (an lm-doc
//! `Document` in mirror mode, updated by the server's differences and diffs), so reading and the feeds are local and
//! immediate; changes answer `Pending::Waiting` and their results come in later pumps (`finished`); gestures and
//! parameter writes never wait for anyone.
//!
//! A dropped connection is retried in the background, with a growing pause (100 ms to 2 s); the mirror is resynced
//! from the new snapshot and the feeds get the difference as one diff. A version mismatch is a clear, final error
//! (`remote.version_mismatch`, with both versions).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lm_base::{Error, Label, OperationId, Result};
use lm_doc::{Diff, Document, Feed, GestureId, Options, Topic};
use lm_port::{
    ActionId, Applied, BrowseEntry, BrowsePage, CorePort, DeviceInfo, DisplayView, EditOp, EntryId, Finished, Loading,
    Notice, OperationInfo, PeakBlock, Pending, Position, Preview, RecordingTake, RequestId, Target, TelemetryView,
};
use lm_state::{NodeId, PathStep, Resource, SchemaRegistry, Value};

use crate::codec::{self as c, bad, get, text};
use crate::delta;
use crate::proto::{PROGRAM, PROTOCOL, Role, Stream};
use crate::wire::{self, Frame, KIND_DISPLAY, KIND_JSON, KIND_PEAKS, KIND_TELEMETRY};

/// Where the server is
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    /// `host:port` (through an SSH tunnel to reach another machine, decisoes-autonomas.md)
    Tcp(String),
}

/// How the RemotePort connects
#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub endpoint: Endpoint,
    pub token: Option<String>,
    pub name: String,
    /// The role asked for (the server grants at most the token's)
    pub role: Role,
    /// The project's schemas (the same program on both ends, so the same registry)
    pub schemas: Arc<SchemaRegistry>,
    /// Telemetry frames per second asked (up to 60)
    pub telemetry_hz: f64,
    /// Where peak blocks are kept forever, by resource hash (I-20.4); `None`: in memory only
    pub peaks_cache: Option<PathBuf>,
    /// How long a handshake may take
    pub timeout: Duration,
}

impl ClientConfig {
    pub fn new(endpoint: Endpoint, schemas: Arc<SchemaRegistry>) -> Self {
        Self {
            endpoint,
            token: None,
            name: "lm-gui".into(),
            role: Role::Admin,
            schemas,
            telemetry_hz: 30.0,
            peaks_cache: None,
            timeout: Duration::from_secs(10),
        }
    }
}

/// The connection's state, for the interface's indicator
#[derive(Clone, Debug, PartialEq)]
pub enum Connection {
    Connected {
        role: Role,
        server: String,
    },
    /// Lost; retrying in the background
    Reconnecting {
        since: Instant,
        attempts: u32,
        last: Error,
    },
    /// Given up: a refusal that retrying cannot change (version, authentication)
    Failed(Error),
}

enum Incoming {
    Frame(Frame),
    Closed(Error),
}

struct Live {
    out: Sender<(u8, Vec<u8>)>,
    incoming: Receiver<Incoming>,
    stream: Stream,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stream.shutdown();
    }
}

struct Welcome {
    live: Live,
    role: Role,
    server: String,
    snapshot: Value,
    /// The handshake's and the snapshot's bytes
    bytes: u64,
}

/// The bytes received, by kind (I-20.8's measures), frames' headers included
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Traffic {
    /// The handshakes and the snapshots (one per connection)
    pub snapshots: u64,
    /// The other JSON messages: tree updates, answers, session state
    pub messages: u64,
    pub peaks: u64,
    pub telemetry: u64,
    pub displays: u64,
}

impl Traffic {
    pub fn total(&self) -> u64 {
        self.snapshots + self.messages + self.peaks + self.telemetry + self.displays
    }
}

/// What a request was, to make its result
#[derive(Clone, Copy, Debug)]
enum Kind {
    Applied,
    Done,
    Peaks,
    Devices,
    Entries,
    Preview,
    Ping,
    Imported,
    /// `midi_learn`: the learn's number
    Learned,
    /// A plain value (the project list): collected by `values`
    Value,
}

/// An upload for an import (I-20.3): its thread reads the file, sends it in chunks within a window of what the
/// server acknowledged, and asks for the import at the end. A reconnection starts a new thread from what the
/// server already has
struct UploadJob {
    path: PathBuf,
    ext: String,
    /// The file's BLAKE3 (hex) and size, worked out once
    hash: Mutex<Option<(String, u64)>>,
    acked: AtomicU64,
    known: AtomicBool,
    /// One number per attempt (M6): a thread runs only while the job's generation is the one it was started with, so
    /// a thread that has not yet seen a disconnection can never overlap the one a reconnection starts
    generation: AtomicU64,
    error: Mutex<Option<Error>>,
}

impl UploadJob {
    /// Ends the running thread, and any older one, for good
    fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    fn stopped(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) != generation
    }
}

/// Bytes of an upload in flight beyond what the server acknowledged
const UPLOAD_WINDOW: u64 = 8 << 20;
const UPLOAD_CHUNK: usize = 256 << 10;

fn upload_thread(id: u64, job: Arc<UploadJob>, generation: u64, out: Sender<(u8, Vec<u8>)>) {
    let fail = |e: Error| {
        *job.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
    };
    let io = |e: std::io::Error| Error::new("remote.upload.read").arg("error", e.to_string());
    // The hash first (once)
    let known = job.hash.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let (hash, size) = match known {
        Some(h) => h,
        None => {
            let r = (|| -> Result<(String, u64)> {
                let mut f = File::open(&job.path).map_err(io)?;
                let mut h = blake3::Hasher::new();
                let mut buf = vec![0u8; 1 << 20];
                let mut size = 0u64;
                loop {
                    if job.stopped(generation) {
                        return Err(Error::new("remote.disconnected"));
                    }
                    let n = f.read(&mut buf).map_err(io)?;
                    if n == 0 {
                        break;
                    }
                    size += n as u64;
                    h.update(&buf[..n]);
                }
                Ok((h.finalize().to_hex().to_string(), size))
            })();
            match r {
                Ok(h) => {
                    *job.hash.lock().unwrap_or_else(PoisonError::into_inner) = Some(h.clone());
                    h
                }
                Err(e) if e.key() == "remote.disconnected" => return,
                Err(e) => return fail(e),
            }
        }
    };
    let begin = Value::object([
        ("t", Value::text("upload_begin")),
        // The request id too: a refusal (quota, limits) answers the import instead of leaving it waiting
        ("id", Value::int(id as i64)),
        ("upload", Value::int(id as i64)),
        ("hash", Value::text(hash.as_str())),
        ("size", Value::int(i64::try_from(size).unwrap_or(i64::MAX))),
        ("ext", Value::text(job.ext.as_str())),
    ]);
    let Ok(p) = wire::json(&begin) else { return };
    if out.send((KIND_JSON, p)).is_err() {
        return;
    }
    while !job.known.load(Ordering::Acquire) {
        if job.stopped(generation) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut f = match File::open(&job.path) {
        Ok(f) => f,
        Err(e) => return fail(io(e)),
    };
    let mut offset = job.acked.load(Ordering::Acquire);
    if let Err(e) = f.seek(SeekFrom::Start(offset)) {
        return fail(io(e));
    }
    let mut buf = vec![0u8; UPLOAD_CHUNK];
    while offset < size {
        if job.stopped(generation) {
            return;
        }
        if offset - job.acked.load(Ordering::Acquire).min(offset) >= UPLOAD_WINDOW {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        let want = UPLOAD_CHUNK.min((size - offset) as usize);
        if let Err(e) = f.read_exact(&mut buf[..want]) {
            return fail(io(e));
        }
        if out.send((wire::KIND_UPLOAD, wire::upload_to(id, offset, &buf[..want]))).is_err() {
            return;
        }
        offset += want as u64;
    }
    while job.acked.load(Ordering::Acquire) < size {
        if job.stopped(generation) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let end = Value::object([
        ("t", Value::text("upload_end")),
        ("id", Value::int(id as i64)),
        ("upload", Value::int(id as i64)),
    ]);
    if let Ok(p) = wire::json(&end) {
        let _ = out.send((KIND_JSON, p));
    }
}

fn open(endpoint: &Endpoint, timeout: Duration) -> Result<Stream> {
    let e = |e: std::io::Error| Error::new("remote.connect").arg("error", e.to_string());
    match endpoint {
        #[cfg(unix)]
        Endpoint::Unix(p) => Ok(Stream::Unix(UnixStream::connect(p).map_err(e)?)),
        Endpoint::Tcp(addr) => {
            let addrs: Vec<SocketAddr> = addr.to_socket_addrs().map_err(e)?.collect();
            let mut last = Error::new("remote.connect").arg("error", "no address");
            for a in addrs {
                match TcpStream::connect_timeout(&a, timeout) {
                    Ok(s) => {
                        let _ = s.set_nodelay(true);
                        return Ok(Stream::Tcp(s));
                    }
                    Err(err) => last = e(err),
                }
            }
            Err(last)
        }
    }
}

/// Connects and shakes hands (blocking, bounded by the config's timeout)
fn handshake(cfg: &ClientConfig) -> Result<Welcome> {
    let mut stream = open(&cfg.endpoint, cfg.timeout)?;
    stream.set_read_timeout(Some(cfg.timeout));
    let mut hello = Value::object([
        ("t", Value::text("hello")),
        ("protocol", Value::int(PROTOCOL)),
        ("program", Value::text(PROGRAM)),
        ("mode", Value::text("gui")),
        ("name", Value::text(cfg.name.as_str())),
        ("role", Value::text(cfg.role.key())),
    ]);
    if let Some(t) = &cfg.token {
        hello = hello.with("token", Value::text(t.as_str()));
    }
    wire::write_frame(&mut stream, KIND_JSON, &wire::json(&hello)?)?;
    let mut bytes = 0u64;
    let mut read = |s: &mut Stream| -> Result<Value> {
        let f = wire::read_frame(s, wire::MAX_FRAME_FROM_SERVER)?;
        bytes += f.payload.len() as u64 + 5;
        if f.kind != KIND_JSON {
            return Err(bad("handshake"));
        }
        wire::parse_json(&f.payload)
    };
    let w = read(&mut stream)?;
    match text(&w, "t")? {
        "welcome" => {}
        "refused" => return Err(c::error_from(get(&w, "error")?)?),
        _ => return Err(bad("handshake")),
    }
    let role = Role::parse(text(&w, "role")?).ok_or_else(|| bad("role"))?;
    let server = text(&w, "server")?.to_owned();
    let snapshot = read(&mut stream)?;
    if text(&snapshot, "t")? != "snapshot" {
        return Err(bad("snapshot"));
    }
    stream.set_read_timeout(None);
    // The threads
    let (in_tx, in_rx) = channel::<Incoming>();
    let (out_tx, out_rx) = channel::<(u8, Vec<u8>)>();
    let mut reader = stream.try_clone()?;
    let mut writer = stream.try_clone()?;
    let spawn = |name: &str, f: Box<dyn FnOnce() + Send>| {
        std::thread::Builder::new()
            .name(name.into())
            .spawn(f)
            .map_err(|e| Error::new("remote.io").arg("error", e.to_string()))
    };
    spawn(
        "lm-remote-client-in",
        Box::new(move || {
            loop {
                match wire::read_frame(&mut reader, wire::MAX_FRAME_FROM_SERVER) {
                    Ok(f) => {
                        if in_tx.send(Incoming::Frame(f)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = in_tx.send(Incoming::Closed(e));
                        return;
                    }
                }
            }
        }),
    )?;
    spawn(
        "lm-remote-client-out",
        Box::new(move || {
            while let Ok((kind, payload)) = out_rx.recv() {
                if wire::write_frame(&mut writer, kind, &payload).is_err() {
                    writer.shutdown();
                    return;
                }
            }
        }),
    )?;
    Ok(Welcome { live: Live { out: out_tx, incoming: in_rx, stream }, role, server, snapshot, bytes })
}

fn final_error(e: &Error) -> bool {
    e.key() == "remote.version_mismatch" || e.key().starts_with("remote.auth")
}

/// The port over the network
pub struct RemotePort {
    cfg: ClientConfig,
    live: Option<Live>,
    connection: Connection,
    retry: Option<(Instant, Receiver<Result<Welcome>>)>,
    /// When the reconnect thread could not be spawned: try again then (M7), the connection stays `Reconnecting`
    respawn_at: Option<Instant>,
    doc: Document,
    modified: bool,
    can_undo: bool,
    can_redo: bool,
    prefs: Value,
    status: Value,
    /// The interface document (I-3.8) and the learn under way (I-21.2), as the server last sent them
    ui: Value,
    learning: Option<lm_port::LearnId>,
    /// The interface's mapped actions not yet drained, and the raw MIDI activity with its own counter
    midi_ui: Vec<lm_port::MidiUiAction>,
    midi_activity: std::collections::VecDeque<(u64, lm_port::MidiActivity)>,
    midi_counter: u64,
    notices: Vec<Notice>,
    activity: Vec<OperationInfo>,
    telemetry: TelemetryView,
    loadings: RefCell<HashMap<Target, Loading>>,
    watch_new: RefCell<Vec<Target>>,
    displays: RefCell<HashMap<NodeId, Option<DisplayView>>>,
    display_new: RefCell<Vec<NodeId>>,
    next_request: u64,
    requests: HashMap<RequestId, Kind>,
    finished: Vec<(RequestId, Finished)>,
    next_gesture: u64,
    uploads: HashMap<RequestId, Arc<UploadJob>>,
    values: Vec<(RequestId, Result<Value>)>,
    peaks: HashMap<(String, u8, u64, u64), PeakBlock>,
    peaks_asked: HashMap<RequestId, (String, u8, u64, u64)>,
    tempo: lm_port::TempoCache,
    /// Updates applied since connecting (for tests and the indicator)
    updates: u64,
    traffic: Traffic,
}

impl std::fmt::Debug for RemotePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemotePort").field("connection", &self.connection).finish_non_exhaustive()
    }
}

fn mirror_options() -> Options {
    Options { history_limit: 0, journal: false, rules: Vec::new(), ..Options::default() }
}

/// A connection being made in the background (the interface never waits on the network: I-2.1); the port is built
/// on the thread that finishes it
pub struct Connecting {
    cfg: ClientConfig,
    rx: Receiver<Result<Welcome>>,
}

impl std::fmt::Debug for Connecting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connecting").field("endpoint", &self.cfg.endpoint).finish()
    }
}

impl Connecting {
    /// The port, once the handshake ended (well or not)
    pub fn poll(&mut self) -> Option<Result<RemotePort>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r.and_then(|w| RemotePort::from_welcome(self.cfg.clone(), w))),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(Error::new("remote.connect"))),
        }
    }

    /// Waits for it (tests, tools)
    pub fn wait(mut self) -> Result<RemotePort> {
        loop {
            if let Some(r) = self.poll() {
                return r;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl RemotePort {
    /// Starts connecting in the background
    pub fn start(cfg: ClientConfig) -> Connecting {
        let (tx, rx) = channel();
        let c = cfg.clone();
        let spawned = std::thread::Builder::new().name("lm-remote-connect".into()).spawn(move || {
            let _ = tx.send(handshake(&c));
        });
        if let Err(e) = spawned {
            let (tx, rx2) = channel();
            let _ = tx.send(Err(Error::new("remote.io").arg("error", e.to_string())));
            return Connecting { cfg, rx: rx2 };
        }
        Connecting { cfg, rx }
    }

    /// Connects, shakes hands and loads the snapshot (blocking, bounded by the timeout)
    pub fn connect(cfg: ClientConfig) -> Result<RemotePort> {
        Self::start(cfg).wait()
    }

    fn from_welcome(cfg: ClientConfig, w: Welcome) -> Result<RemotePort> {
        let root = get(&w.snapshot, "root")?.clone();
        let doc = Document::from_root(Arc::clone(&cfg.schemas), lm_doc::PROJECT, root, mirror_options())?.document;
        let telemetry = TelemetryView {
            position: Position {
                samples: 0,
                host_time: lm_base::HostTime(0),
                sample_rate: 0,
                playing: false,
                recording: false,
            },
            meters: HashMap::new(),
            inputs: Vec::new(),
            recording: Vec::new(),
            counting_in: false,
            xruns: 0,
            xrun_marks: Vec::new(),
            load: 0.0,
            hand_won: Vec::new(),
        };
        let mut port = RemotePort {
            tempo: Default::default(),
            connection: Connection::Connected { role: w.role, server: w.server.clone() },
            cfg,
            live: None,
            retry: None,
            respawn_at: None,
            doc,
            modified: false,
            can_undo: false,
            can_redo: false,
            prefs: Value::NULL,
            status: Value::NULL,
            ui: Value::NULL,
            learning: None,
            midi_ui: Vec::new(),
            midi_activity: std::collections::VecDeque::new(),
            midi_counter: 0,
            notices: Vec::new(),
            activity: Vec::new(),
            telemetry,
            loadings: RefCell::new(HashMap::new()),
            watch_new: RefCell::new(Vec::new()),
            displays: RefCell::new(HashMap::new()),
            display_new: RefCell::new(Vec::new()),
            next_request: 1,
            requests: HashMap::new(),
            finished: Vec::new(),
            next_gesture: 1,
            uploads: HashMap::new(),
            values: Vec::new(),
            peaks: HashMap::new(),
            peaks_asked: HashMap::new(),
            updates: 0,
            traffic: Traffic { snapshots: w.bytes, ..Traffic::default() },
        };
        port.session_state(&w.snapshot)?;
        port.live = Some(w.live);
        port.subscribe_all();
        Ok(port)
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// The bytes received so far, by kind
    pub fn traffic(&self) -> Traffic {
        self.traffic
    }

    /// The tree updates applied since the port was made
    pub fn updates(&self) -> u64 {
        self.updates
    }

    /// Cuts the connection (tests; it reconnects at the next pumps)
    pub fn disconnect(&mut self) {
        if let Some(l) = &self.live {
            l.stream.shutdown();
        }
    }

    fn session_state(&mut self, s: &Value) -> Result<()> {
        self.prefs = get(s, "prefs")?.clone();
        self.status = get(s, "status")?.clone();
        // the additions of 20/11 (absent from an older server)
        self.ui = s.get("ui").cloned().unwrap_or(Value::NULL);
        self.learning = s.get("learning").and_then(Value::as_int).map(|l| l as u64);
        self.telemetry.hand_won = s.get("hand_won").map(c::hand_won_from).transpose()?.unwrap_or_default();
        self.flags(s)?;
        self.notices = c::list(s, "notices")?.map(c::notice_from).collect::<Result<_>>()?;
        self.activity = c::list(s, "activity")?.map(c::operation_from).collect::<Result<_>>()?;
        Ok(())
    }

    fn flags(&mut self, v: &Value) -> Result<()> {
        self.modified = c::flag(v, "modified")?;
        self.can_undo = c::flag(v, "can_undo")?;
        self.can_redo = c::flag(v, "can_redo")?;
        Ok(())
    }

    fn send(&self, v: &Value) -> bool {
        match (&self.live, wire::json(v)) {
            (Some(l), Ok(p)) => l.out.send((KIND_JSON, p)).is_ok(),
            _ => false,
        }
    }

    fn subscribe_all(&mut self) {
        self.send(&Value::object([("t", Value::text("telemetry")), ("hz", c::real_value(self.cfg.telemetry_hz))]));
        let targets: Vec<Target> = self.loadings.borrow().keys().cloned().collect();
        self.watch_new.borrow_mut().extend(targets);
        let nodes: Vec<NodeId> = self.displays.borrow().keys().copied().collect();
        self.display_new.borrow_mut().extend(nodes);
    }

    /// Sends a request; `Waiting(id)`, or a ready error when not connected
    fn request<T>(&mut self, kind: Kind, msg: Value) -> Pending<T> {
        let id = self.next_request;
        self.next_request += 1;
        let msg = msg.with("id", Value::int(id as i64));
        if self.send(&msg) {
            self.requests.insert(id, kind);
            Pending::Waiting(id)
        } else {
            Pending::Ready(Err(Error::new("remote.disconnected")))
        }
    }

    fn message(t: &str, fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        Value::object(std::iter::once(("t", Value::text(t))).chain(fields))
    }

    fn on_disconnect(&mut self, e: Error) {
        self.live = None;
        let final_now = final_error(&e);
        for (id, kind) in std::mem::take(&mut self.requests) {
            // An upload goes on after the reconnection, from what the server has (unless it never will)
            if let Some(job) = self.uploads.get(&id) {
                job.cancel();
                job.known.store(false, Ordering::Release);
                if !final_now {
                    self.requests.insert(id, kind);
                    continue;
                }
                self.uploads.remove(&id);
            }
            if matches!(kind, Kind::Value) {
                self.values.push((id, Err(Error::new("remote.disconnected"))));
            } else {
                self.finished.push((id, failed(kind, Error::new("remote.disconnected"))));
            }
        }
        self.peaks_asked.clear();
        if final_error(&e) {
            self.connection = Connection::Failed(e);
            return;
        }
        let since = match &self.connection {
            Connection::Reconnecting { since, .. } => *since,
            _ => Instant::now(),
        };
        self.connection = Connection::Reconnecting { since, attempts: 0, last: e };
        self.start_retry(Duration::from_millis(100));
    }

    fn start_retry(&mut self, after: Duration) {
        let (tx, rx) = channel();
        let cfg = self.cfg.clone();
        let started = std::thread::Builder::new().name("lm-remote-reconnect".into()).spawn(move || {
            std::thread::sleep(after);
            let _ = tx.send(handshake(&cfg));
        });
        match started {
            Ok(_) => self.retry = Some((Instant::now(), rx)),
            Err(e) => {
                // The next attempt is the pump's job: a failed spawn must not end the reconnection for good
                if let Connection::Reconnecting { last, .. } = &mut self.connection {
                    *last = Error::new("remote.connect").arg("error", e.to_string());
                }
                self.respawn_at = Some(Instant::now() + Duration::from_secs(1));
            }
        }
    }

    fn poll_retry(&mut self) -> bool {
        let Some((_, rx)) = &self.retry else {
            if self.respawn_at.is_some_and(|at| Instant::now() >= at) {
                self.respawn_at = None;
                self.start_retry(Duration::ZERO);
            }
            return false;
        };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err(Error::new("remote.connect")),
        };
        self.retry = None;
        match r {
            Ok(w) => match self.resync(w) {
                Ok(()) => true,
                Err(e) => {
                    self.on_disconnect(e);
                    true
                }
            },
            Err(e) => {
                if final_error(&e) {
                    self.connection = Connection::Failed(e);
                    return true;
                }
                let (since, attempts) = match &self.connection {
                    Connection::Reconnecting { since, attempts, .. } => (*since, attempts + 1),
                    _ => (Instant::now(), 1),
                };
                self.connection = Connection::Reconnecting { since, attempts, last: e };
                let pause =
                    Duration::from_millis(100u64.saturating_mul(1 << attempts.min(5))).min(Duration::from_secs(2));
                self.start_retry(pause);
                true
            }
        }
    }

    fn resync(&mut self, w: Welcome) -> Result<()> {
        self.traffic.snapshots += w.bytes;
        let root = get(&w.snapshot, "root")?.clone();
        self.doc.mirror(root, None)?;
        self.session_state(&w.snapshot)?;
        self.connection = Connection::Connected { role: w.role, server: w.server };
        self.live = Some(w.live);
        self.subscribe_all();
        self.updates += 1;
        let ids: Vec<RequestId> = self.uploads.keys().copied().collect();
        for id in ids {
            self.start_upload(id);
        }
        Ok(())
    }

    fn start_upload(&mut self, id: RequestId) {
        let (Some(job), Some(live)) = (self.uploads.get(&id), &self.live) else { return };
        let (generation, job, out) = (job.generation.load(Ordering::Acquire), Arc::clone(job), live.out.clone());
        let spawned =
            std::thread::Builder::new().name("lm-remote-upload".into()).spawn(move || upload_thread(id, job, generation, out));
        if let Err(e) = spawned {
            self.uploads.remove(&id);
            self.requests.remove(&id);
            self.finished.push((id, Finished::Imported(Err(Error::new("remote.io").arg("error", e.to_string())))));
        }
    }

    /// The uploads whose thread failed (the file could not be read)
    fn upload_errors(&mut self) {
        let failed: Vec<(RequestId, Error)> = self
            .uploads
            .iter()
            .filter_map(|(id, j)| j.error.lock().unwrap_or_else(PoisonError::into_inner).take().map(|e| (*id, e)))
            .collect();
        for (id, e) in failed {
            self.uploads.remove(&id);
            self.requests.remove(&id);
            self.finished.push((id, Finished::Imported(Err(e))));
        }
    }

    // ---- the projects (I-20.3: the server's projects folder, by name; each one an operation of the session) ----

    /// Opens the project `name` of the server's projects folder
    pub fn open_project(&mut self, name: &str) -> Pending<()> {
        self.request(Kind::Done, Self::message("project_open", [("name", Value::text(name))]))
    }

    /// Makes a new project `name` there and goes on in it
    pub fn new_project(&mut self, name: &str) -> Pending<()> {
        self.request(Kind::Done, Self::message("project_new", [("name", Value::text(name))]))
    }

    pub fn save_project(&mut self) -> Pending<()> {
        self.request(Kind::Done, Self::message("project_save", []))
    }

    pub fn save_project_as(&mut self, name: &str) -> Pending<()> {
        self.request(Kind::Done, Self::message("project_save_as", [("name", Value::text(name))]))
    }

    /// Closes the project (the server goes on with a new untitled one)
    pub fn close_project(&mut self) -> Pending<()> {
        self.request(Kind::Done, Self::message("project_close", []))
    }

    /// The projects of the server's folder and the recent ones: `{projects: [name], recent: [...]}`, collected by
    /// `values`
    pub fn list_projects(&mut self) -> Pending<Value> {
        self.request(Kind::Value, Self::message("project_list", []))
    }

    /// The answers of `list_projects`
    pub fn values(&mut self) -> Vec<(RequestId, Result<Value>)> {
        std::mem::take(&mut self.values)
    }

    /// The bytes of an upload the server has acknowledged (for a progress bar), and the file's size once known
    pub fn upload_progress(&self, id: RequestId) -> Option<(u64, Option<u64>)> {
        let j = self.uploads.get(&id)?;
        let size = j.hash.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(|(_, s)| *s);
        Some((j.acked.load(Ordering::Acquire), size))
    }

    fn incoming(&mut self, f: Frame) -> Result<()> {
        let n = f.payload.len() as u64 + 5;
        match f.kind {
            KIND_JSON => self.traffic.messages += n,
            KIND_TELEMETRY => self.traffic.telemetry += n,
            KIND_PEAKS => self.traffic.peaks += n,
            KIND_DISPLAY => self.traffic.displays += n,
            _ => {}
        }
        match f.kind {
            KIND_JSON => self.json(&wire::parse_json(&f.payload)?),
            KIND_TELEMETRY => wire::telemetry_from(&f.payload, &mut self.telemetry),
            KIND_PEAKS => {
                let (id, block) = wire::peaks_from(&f.payload)?;
                self.requests.remove(&id);
                if let Some(key) = self.peaks_asked.remove(&id) {
                    self.store_peaks(&key, &block);
                    self.peaks.insert(key, block.clone());
                }
                self.finished.push((id, Finished::Peaks(Ok(block))));
                Ok(())
            }
            KIND_DISPLAY => {
                let (node, view) = wire::display_from(&f.payload)?;
                self.displays.borrow_mut().insert(node, Some(view));
                Ok(())
            }
            _ => Err(bad("frame kind")),
        }
    }

    fn json(&mut self, m: &Value) -> Result<()> {
        match text(m, "t")? {
            "update" => {
                let ops = delta::from_value(get(m, "ops")?)?;
                let diff: Diff = c::diff_from(get(m, "diff")?)?;
                let root = delta::apply(self.doc.root(), &ops)?;
                self.doc.mirror(root, Some(&diff))?;
                self.flags(m)?;
                self.updates += 1;
            }
            "state" => self.flags(m)?,
            "prefs" => self.prefs = get(m, "value")?.clone(),
            "status" => self.status = get(m, "value")?.clone(),
            "ui" => self.ui = get(m, "value")?.clone(),
            "learning" => self.learning = get(m, "value")?.as_int().map(|l| l as u64),
            "hand_won" => self.telemetry.hand_won = c::hand_won_from(get(m, "value")?)?,
            "midi_ui" => {
                for a in c::list(m, "value")? {
                    self.midi_ui.push(c::ui_action_from(a)?);
                }
            }
            "midi_activity" => {
                for a in c::list(m, "value")? {
                    self.midi_counter += 1;
                    self.midi_activity.push_back((self.midi_counter, c::activity_from(a)?));
                }
                while self.midi_activity.len() > 256 {
                    self.midi_activity.pop_front();
                }
            }
            "notices" => self.notices = c::list(m, "value")?.map(c::notice_from).collect::<Result<_>>()?,
            "activity" => self.activity = c::list(m, "value")?.map(c::operation_from).collect::<Result<_>>()?,
            "loading" => {
                let t = c::target_from(get(m, "target")?)?;
                let l = c::loading_from(get(m, "value")?)?;
                self.loadings.borrow_mut().insert(t, l);
            }
            "recording" => {
                self.telemetry.recording = c::list(m, "value")?
                    .map(|r| {
                        Ok(RecordingTake {
                            track: c::id_from(get(r, "track")?)?,
                            resource: c::resource_from(get(r, "resource")?)?,
                            start: c::int(r, "start")?,
                            length: c::int(r, "length")?,
                        })
                    })
                    .collect::<Result<_>>()?;
            }
            "done" => {
                let id = c::uint(m, "id")?;
                let r: Result<Value> = match m.get("err") {
                    Some(e) => Err(c::error_from(e)?),
                    None => Ok(get(m, "ok")?.clone()),
                };
                if let Some(Kind::Value) = self.requests.get(&id) {
                    self.requests.remove(&id);
                    self.values.push((id, r));
                } else if let Some(kind) = self.requests.remove(&id) {
                    self.peaks_asked.remove(&id);
                    if let Some(job) = self.uploads.remove(&id) {
                        job.cancel();
                    }
                    self.finished.push((id, finish(kind, r)));
                }
            }
            "upload_have" | "upload_ack" => {
                let id = c::uint(m, "upload")?;
                let have = c::uint(m, "have")?;
                if let Some(job) = self.uploads.get(&id) {
                    job.acked.store(have, Ordering::Release);
                    job.known.store(true, Ordering::Release);
                }
            }
            "error" => {
                // A message of ours the server could not take: shown as a notice-like activity would be noise; kept
                // only in the connection state for debugging
            }
            _ => return Err(bad("t")),
        }
        Ok(())
    }

    fn peaks_file(&self, key: &(String, u8, u64, u64)) -> Option<PathBuf> {
        let dir = self.cfg.peaks_cache.as_ref()?;
        // The key is a validated 64-hex hash (lm-state, C9): safe as a file name
        if !lm_state::is_valid_resource_key(&key.0) {
            return None;
        }
        Some(dir.join(format!("{}-{}-{}-{}.peaks", key.0, key.1, key.2, key.3)))
    }

    fn store_peaks(&self, key: &(String, u8, u64, u64), b: &PeakBlock) {
        if let Some(path) = self.peaks_file(key) {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
            let _ = std::fs::write(&path, wire::peaks_to(0, b));
        }
    }

    fn cached_peaks(&mut self, key: &(String, u8, u64, u64)) -> Option<PeakBlock> {
        if let Some(b) = self.peaks.get(key) {
            return Some(b.clone());
        }
        let bytes = std::fs::read(self.peaks_file(key)?).ok()?;
        let (_, b) = wire::peaks_from(&bytes).ok()?;
        self.peaks.insert(key.clone(), b.clone());
        Some(b)
    }

    /// Asks the server for a round trip (tests and the latency indicator)
    pub fn ping(&mut self) -> Pending<()> {
        self.request(Kind::Ping, Self::message("ping", []))
    }
}

fn failed(kind: Kind, e: Error) -> Finished {
    match kind {
        Kind::Applied => Finished::Applied(Err(e)),
        Kind::Done | Kind::Ping => Finished::Done(Err(e)),
        Kind::Peaks => Finished::Peaks(Err(e)),
        Kind::Devices => Finished::Devices(Err(e)),
        Kind::Entries => Finished::Entries(Err(e)),
        Kind::Preview => Finished::Preview(Err(e)),
        Kind::Imported => Finished::Imported(Err(e)),
        Kind::Learned => Finished::Learned(Err(e)),
        Kind::Value => Finished::Done(Err(e)),
    }
}

fn finish(kind: Kind, r: Result<Value>) -> Finished {
    match kind {
        Kind::Applied => Finished::Applied(r.and_then(|v| c::applied_from(&v))),
        Kind::Done | Kind::Ping => Finished::Done(r.map(|_| ())),
        Kind::Peaks => Finished::Peaks(r.and_then(|_| Err(bad("peaks")))),
        Kind::Devices => Finished::Devices(
            r.and_then(|v| if v.is_list() { v.items().map(c::device_from).collect() } else { Err(bad("devices")) }),
        ),
        Kind::Entries => Finished::Entries(r.and_then(|v| c::entries_from(&v))),
        Kind::Preview => Finished::Preview(r.and_then(|v| c::preview_from(&v))),
        Kind::Imported => Finished::Imported(r.and_then(|v| c::resource_from(&v))),
        Kind::Learned => Finished::Learned(r.and_then(|v| {
            v.as_int().map(|i| i as u64).ok_or_else(|| Error::new("remote.bad_message").arg("field", "learn"))
        })),
        Kind::Value => Finished::Done(r.map(|_| ())),
    }
}

impl CorePort for RemotePort {
    fn pump(&mut self) -> bool {
        let mut changed = self.poll_retry();
        self.upload_errors();
        // Everything that arrived
        let mut closed = None;
        if let Some(l) = &self.live {
            let mut frames = Vec::new();
            loop {
                match l.incoming.try_recv() {
                    Ok(Incoming::Frame(f)) => frames.push(f),
                    Ok(Incoming::Closed(e)) => {
                        closed = Some(e);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        closed = Some(Error::new("remote.disconnected"));
                        break;
                    }
                }
            }
            for f in frames {
                changed = true;
                if let Err(e) = self.incoming(f) {
                    // A message that does not decode means the two ends disagree: start over from a snapshot
                    closed = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = closed {
            self.on_disconnect(e);
            changed = true;
        }
        // Subscriptions asked by the reading calls (which take &self)
        let watches: Vec<Target> = std::mem::take(&mut *self.watch_new.borrow_mut());
        for t in watches {
            if let Ok(tv) = c::target_to(&t) {
                self.send(&Self::message("watch", [("target", tv), ("on", Value::bool(true))]));
            }
        }
        let displays: Vec<NodeId> = std::mem::take(&mut *self.display_new.borrow_mut());
        for n in displays {
            self.send(&Self::message("display", [("node", c::id_to(n)), ("on", Value::bool(true))]));
        }
        changed
    }

    fn finished(&mut self) -> Vec<(RequestId, Finished)> {
        std::mem::take(&mut self.finished)
    }

    fn root(&self) -> &Value {
        self.doc.root()
    }

    fn find(&self, id: NodeId) -> Option<Value> {
        self.doc.find(id)
    }

    fn feed(&mut self, topic: Topic) -> Feed {
        self.doc.feed(topic)
    }

    fn drain(&self, feed: &mut Feed) -> Option<Diff> {
        feed.drain(&self.doc)
    }

    fn modified(&self) -> bool {
        self.modified
    }

    fn can_undo(&self) -> bool {
        self.can_undo
    }

    fn can_redo(&self) -> bool {
        self.can_redo
    }

    fn apply(&mut self, label: Label, ops: Vec<EditOp>) -> Pending<Applied> {
        match c::ops_to(&ops) {
            Ok(o) => self.request(Kind::Applied, Self::message("apply", [("label", c::label_to(&label)), ("ops", o)])),
            Err(e) => Pending::Ready(Err(e)),
        }
    }

    fn run_action(&mut self, action: ActionId, params: Value) -> Pending<()> {
        self.request(
            Kind::Done,
            Self::message("run_action", [("action", Value::text(action.as_str())), ("params", params)]),
        )
    }

    fn undo(&mut self) -> Pending<()> {
        self.request(Kind::Done, Self::message("undo", []))
    }

    fn redo(&mut self) -> Pending<()> {
        self.request(Kind::Done, Self::message("redo", []))
    }

    fn begin_gesture(&mut self, label: Label) -> GestureId {
        let g = self.next_gesture;
        self.next_gesture += 1;
        self.send(&Self::message("gesture_begin", [("g", Value::int(g as i64)), ("label", c::label_to(&label))]));
        GestureId::from_raw(g)
    }

    fn gesture_apply(&mut self, g: &GestureId, ops: Vec<EditOp>) {
        if let Ok(o) = c::ops_to(&ops) {
            self.send(&Self::message("gesture_apply", [("g", Value::int(g.raw() as i64)), ("ops", o)]));
        }
    }

    fn gesture_commit(&mut self, g: GestureId) {
        self.send(&Self::message("gesture_commit", [("g", Value::int(g.raw() as i64))]));
    }

    fn gesture_cancel(&mut self, g: GestureId) {
        self.send(&Self::message("gesture_cancel", [("g", Value::int(g.raw() as i64))]));
    }

    fn param_write(&mut self, node: NodeId, param: &str, value: f32) {
        self.send(&Self::message(
            "param",
            [("node", c::id_to(node)), ("param", Value::text(param)), ("value", c::real_value(f64::from(value)))],
        ));
    }

    fn note_on(&mut self, track: NodeId, key: u8, velocity: u8) {
        self.send(&Self::message(
            "note_on",
            [
                ("track", c::id_to(track)),
                ("key", Value::int(i64::from(key))),
                ("velocity", Value::int(i64::from(velocity))),
            ],
        ));
    }

    fn note_off(&mut self, track: NodeId, key: u8) {
        self.send(&Self::message("note_off", [("track", c::id_to(track)), ("key", Value::int(i64::from(key)))]));
    }

    fn all_notes_off(&mut self, track: NodeId) {
        self.send(&Self::message("all_notes_off", [("track", c::id_to(track))]));
    }

    fn loading(&self, target: &Target) -> Loading {
        if let Some(l) = self.loadings.borrow().get(target) {
            return l.clone();
        }
        // First asked: watched from now on; ready until the server says otherwise
        self.loadings.borrow_mut().insert(target.clone(), Loading::Ready);
        self.watch_new.borrow_mut().push(target.clone());
        Loading::Ready
    }

    fn activity(&self) -> Vec<OperationInfo> {
        self.activity.clone()
    }

    fn cancel(&mut self, op: OperationId) {
        self.send(&Self::message("cancel", [("op", Value::int(op.0 as i64))]));
    }

    fn notices(&mut self) -> Vec<Notice> {
        self.notices.clone()
    }

    fn dismiss_notice(&mut self, id: u64) {
        self.notices.retain(|n| n.id != id);
        self.send(&Self::message("dismiss", [("notice", Value::int(id as i64))]));
    }

    fn telemetry(&self) -> &TelemetryView {
        &self.telemetry
    }

    fn peaks(&mut self, resource: &Resource, level: u8, from: u64, to: u64) -> Pending<PeakBlock> {
        let key = (resource.key().to_owned(), level, from, to);
        if let Some(b) = self.cached_peaks(&key) {
            return Pending::Ready(Ok(b));
        }
        // The same block already asked and not yet answered: the caller waits for that answer, no second request
        if let Some((id, _)) = self.peaks_asked.iter().find(|(_, k)| **k == key) {
            return Pending::Waiting(*id);
        }
        let (Ok(f), Ok(t)) = (c::int_value(from), c::int_value(to)) else {
            return Pending::Ready(Err(bad("peaks range")));
        };
        let p = self.request(
            Kind::Peaks,
            Self::message(
                "peaks",
                [
                    ("resource", c::resource_to(resource)),
                    ("level", Value::int(i64::from(level))),
                    ("from", f),
                    ("to", t),
                ],
            ),
        );
        if let Pending::Waiting(id) = &p {
            self.peaks_asked.insert(*id, key);
        }
        p
    }

    fn tempo_map(&self) -> std::sync::Arc<lm_port::TempoMap> {
        self.tempo.get(self.root())
    }

    fn display(&self, node: NodeId) -> Option<DisplayView> {
        if let Some(d) = self.displays.borrow().get(&node) {
            return d.clone();
        }
        self.displays.borrow_mut().insert(node, None);
        self.display_new.borrow_mut().push(node);
        None
    }

    fn preferences(&self) -> &Value {
        &self.prefs
    }

    fn set_preference(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        match c::path_to(&path) {
            Ok(p) => self.request(Kind::Done, Self::message("set_preference", [("path", p), ("value", value)])),
            Err(e) => Pending::Ready(Err(e)),
        }
    }

    fn status(&self) -> &Value {
        &self.status
    }

    fn ui(&self) -> &Value {
        &self.ui
    }

    fn set_ui(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        // shown at once (the server's answer confirms it, or its next `ui` corrects it)
        if let Ok(v) = self.ui.with_path(&path, value.clone()) {
            self.ui = v;
        }
        match c::path_to(&path) {
            Ok(p) => self.request(Kind::Done, Self::message("set_ui", [("path", p), ("value", value)])),
            Err(e) => Pending::Ready(Err(e)),
        }
    }

    fn midi_learn(&mut self, target: lm_port::MapTarget, scope: lm_port::MapScope) -> Pending<lm_port::LearnId> {
        let msg = Self::message("midi_learn", [("target", c::map_target_to(&target)), ("scope", c::scope_to(scope))]);
        self.request(Kind::Learned, msg)
    }

    fn cancel_learn(&mut self, id: lm_port::LearnId) {
        if self.learning == Some(id) {
            self.learning = None;
        }
        self.send(&Self::message("cancel_learn", [("learn", Value::int(id as i64))]));
    }

    fn learning(&self) -> Option<lm_port::LearnId> {
        self.learning
    }

    fn unmap(&mut self, target: &lm_port::MapTarget, scope: lm_port::MapScope) -> Pending<()> {
        let msg = Self::message("unmap", [("target", c::map_target_to(target)), ("scope", c::scope_to(scope))]);
        self.request(Kind::Done, msg)
    }

    fn drain_midi_ui(&mut self, out: &mut Vec<lm_port::MidiUiAction>) {
        out.append(&mut self.midi_ui);
    }

    fn midi_activity(&self, since: u64) -> (u64, Vec<lm_port::MidiActivity>) {
        (self.midi_counter, self.midi_activity.iter().filter(|(n, _)| *n > since).map(|(_, a)| a.clone()).collect())
    }

    fn devices(&mut self) -> Pending<Vec<DeviceInfo>> {
        self.request(Kind::Devices, Self::message("devices", []))
    }

    fn browse(&mut self, page: BrowsePage, parent: Option<EntryId>, show_hidden: bool) -> Pending<Vec<BrowseEntry>> {
        let parent = parent.map_or(Value::NULL, |p| Value::int(p.0 as i64));
        self.request(
            Kind::Entries,
            Self::message(
                "browse",
                [("page", Value::text(c::page_key(page))), ("parent", parent), ("hidden", Value::bool(show_hidden))],
            ),
        )
    }

    fn search(&mut self, query: &str) -> Pending<Vec<BrowseEntry>> {
        self.request(Kind::Entries, Self::message("search", [("query", Value::text(query))]))
    }

    fn prepare_preview(&mut self, entry: EntryId) -> Pending<Preview> {
        self.request(Kind::Preview, Self::message("prepare_preview", [("entry", Value::int(entry.0 as i64))]))
    }

    fn preview_play(&mut self, entry: EntryId, from: i64) {
        self.send(&Self::message("preview_play", [("entry", Value::int(entry.0 as i64)), ("from", Value::int(from))]));
    }

    fn preview_stop(&mut self) {
        self.send(&Self::message("preview_stop", []));
    }

    fn import(&mut self, path: PathBuf) -> Pending<Resource> {
        let id = self.next_request;
        self.next_request += 1;
        if self.live.is_none() {
            return Pending::Ready(Err(Error::new("remote.disconnected")));
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|e| (1..=8).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_alphanumeric()))
            .unwrap_or_else(|| "audio".to_owned());
        let job = Arc::new(UploadJob {
            path,
            ext,
            hash: Mutex::new(None),
            acked: AtomicU64::new(0),
            known: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            error: Mutex::new(None),
        });
        self.uploads.insert(id, job);
        self.requests.insert(id, Kind::Imported);
        self.start_upload(id);
        Pending::Waiting(id)
    }

    fn set_favorite(&mut self, entry: EntryId, on: bool) -> Pending<()> {
        self.request(
            Kind::Done,
            Self::message("set_favorite", [("entry", Value::int(entry.0 as i64)), ("on", Value::bool(on))]),
        )
    }
}

#[cfg(test)]
mod upload_generation_tests {
    use super::*;

    /// M6: a cancelled attempt stays stopped even after a newer one starts from the same job
    #[test]
    fn an_old_upload_thread_never_resumes() {
        let job = UploadJob {
            path: PathBuf::new(),
            ext: String::new(),
            hash: Mutex::new(None),
            acked: AtomicU64::new(0),
            known: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            error: Mutex::new(None),
        };
        let old = job.generation.load(Ordering::Acquire);
        assert!(!job.stopped(old));
        job.cancel();
        let new = job.generation.load(Ordering::Acquire);
        assert!(job.stopped(old));
        assert!(!job.stopped(new));
    }
}
