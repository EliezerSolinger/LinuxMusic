//! The server (I-20.1): network threads that never touch the session, and a `pump` on the main thread that turns
//! each request into calls of the served `CorePort` (the session's `LocalPort`), so the behaviour is the same as
//! in-process. After every change it sends the mirrors the tree's differences and the diff the core published.
//!
//! Per connection: a reader thread (frames in, bounded) and a writer thread (frames out, from a channel). Continuous
//! data (telemetry, displays) is sent only when the previous frame of it has left, so a slow network lowers the rate
//! instead of growing a queue (I-20.4). A connection that drops cancels its open gestures (I-20.5) and nothing else:
//! playback and recording go on.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lm_base::{Error, Label, OperationId, Result};
use lm_doc::{Feed, GestureId, Topic};
use lm_port::{ActionId, CorePort, EntryId, Finished, Loading, Notice, OperationInfo, Pending, RequestId, Target};
use lm_state::{NodeId, Value};

use crate::codec::{self as c, bad, get, int, real, text, uint};
use crate::delta;
use crate::proto::{self, Mode, PROGRAM, PROTOCOL, Role, Stream, TOOL_APIS};
use crate::wire::{self, Frame, KIND_DISPLAY, KIND_JSON, KIND_PEAKS, KIND_TELEMETRY};

/// How the server listens and whom it lets in
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// A Unix socket (mode 0600: only the same user connects); its users are `Admin`
    pub socket: Option<PathBuf>,
    /// TCP (off by default; bind 127.0.0.1 and reach it through an SSH tunnel, decisoes-autonomas.md)
    pub tcp: Option<SocketAddr>,
    /// The tokens a TCP client may give, with the role each grants (pairing, I-20.6)
    pub tokens: Vec<(String, Role)>,
    /// The server's name, shown by the clients
    pub name: String,
    pub limits: ClientLimits,
}

/// Limits per client (I-20.6)
#[derive(Clone, Copy, Debug)]
pub struct ClientLimits {
    /// The largest frame accepted
    pub max_frame: usize,
    /// Messages per second; past it requests are refused (`remote.rate_limited`) and parameter writes dropped
    pub messages_per_second: u32,
    /// Loading watches and display subscriptions
    pub max_watches: usize,
}

impl Default for ClientLimits {
    fn default() -> Self {
        Self { max_frame: wire::MAX_FRAME_FROM_CLIENT, messages_per_second: 4000, max_watches: 4096 }
    }
}

impl ServerConfig {
    pub fn new(name: &str) -> Self {
        Self { socket: None, tcp: None, tokens: Vec::new(), name: name.to_owned(), limits: ClientLimits::default() }
    }
}

enum Event {
    Open { conn: u64, out: Sender<Out>, stream: Stream, local: bool, busy: Arc<AtomicBool> },
    Frame { conn: u64, frame: Frame },
    Closed { conn: u64 },
}

/// A frame to send; `continuous` frames clear the connection's busy flag once written
struct Out {
    kind: u8,
    payload: Vec<u8>,
    continuous: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Hello,
    Ready,
}

struct Conn {
    out: Sender<Out>,
    stream: Stream,
    local: bool,
    stage: Stage,
    role: Role,
    mode: Mode,
    name: String,
    gestures: HashMap<u64, GestureId>,
    watches: Vec<(Target, Loading)>,
    displays: Vec<NodeId>,
    telemetry: Option<(Duration, Option<Vec<NodeId>>)>,
    last_telemetry: Instant,
    last_display: Instant,
    busy: Arc<AtomicBool>,
    window: (Instant, u32),
    closing: bool,
}

impl Conn {
    fn send_json(&self, v: &Value) {
        match wire::json(v) {
            Ok(p) => {
                let _ = self.out.send(Out { kind: KIND_JSON, payload: p, continuous: false });
            }
            Err(e) => self.send_raw_error(&e),
        }
    }

    fn send_raw_error(&self, e: &Error) {
        let v = Value::object([("t", Value::text("error")), ("error", c::error_to(e))]);
        if let Ok(p) = wire::json(&v) {
            let _ = self.out.send(Out { kind: KIND_JSON, payload: p, continuous: false });
        }
    }

    fn reply(&self, id: u64, r: Result<Value>) {
        let mut v = c::result_to(&r);
        v = v.with("t", Value::text("done"));
        v = v.with("id", Value::int(i64::try_from(id).unwrap_or(-1)));
        if wire::json(&v).is_ok() {
            self.send_json(&v);
        } else {
            // The value would not encode (an ambiguous resource-like object): an error instead
            let e = Error::new("remote.encode");
            self.send_json(&c::result_to(&Err(e)).with("t", Value::text("done")).with("id", Value::int(id as i64)));
        }
    }
}

/// What a request that the port answered `Waiting` was, to forward its result
struct Forward {
    conn: u64,
    id: u64,
}

/// The protocol's server
pub struct Server {
    config: ServerConfig,
    events: Receiver<Event>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    conns: HashMap<u64, Conn>,
    feed: Option<Feed>,
    root: Value,
    state: (bool, bool, bool),
    prefs: Value,
    status: Value,
    notices: Vec<Notice>,
    activity: (Vec<OperationInfo>, Instant),
    recording: Vec<lm_port::RecordingTake>,
    forwards: HashMap<RequestId, Forward>,
    params: Vec<(NodeId, String, f32)>,
    local_addr: Option<SocketAddr>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").field("name", &self.config.name).field("clients", &self.conns.len()).finish()
    }
}

fn io_error(e: &std::io::Error) -> Error {
    Error::new("remote.io").arg("error", e.to_string())
}

fn spawn_conn(conn: u64, stream: Stream, local: bool, max_frame: usize, events: Sender<Event>) -> Result<()> {
    let (out_tx, out_rx) = channel::<Out>();
    let busy = Arc::new(AtomicBool::new(false));
    let mut reader = stream.try_clone()?;
    let mut writer = stream.try_clone()?;
    let _ = events.send(Event::Open { conn, out: out_tx, stream, local, busy: busy.clone() });
    let ev = events.clone();
    std::thread::Builder::new()
        .name("lm-remote-in".into())
        .spawn(move || {
            while let Ok(frame) = wire::read_frame(&mut reader, max_frame) {
                if ev.send(Event::Frame { conn, frame }).is_err() {
                    break;
                }
            }
            let _ = ev.send(Event::Closed { conn });
        })
        .map_err(|e| io_error(&e))?;
    std::thread::Builder::new()
        .name("lm-remote-out".into())
        .spawn(move || {
            while let Ok(o) = out_rx.recv() {
                if wire::write_frame(&mut writer, o.kind, &o.payload).is_err() {
                    writer.shutdown();
                    break;
                }
                if o.continuous {
                    busy.store(false, Ordering::Release);
                }
            }
        })
        .map_err(|e| io_error(&e))?;
    Ok(())
}

impl Server {
    /// Starts listening. The network threads only move frames; the session is touched in `pump` alone
    pub fn start(config: ServerConfig) -> Result<Server> {
        let (tx, rx) = channel::<Event>();
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        let next = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let max_frame = config.limits.max_frame;
        let mut local_addr = None;
        #[cfg(unix)]
        if let Some(path) = &config.socket {
            if path.exists() {
                // A socket left by a server that died; a live one answers, and then this one must not take it
                if std::os::unix::net::UnixStream::connect(path).is_ok() {
                    return Err(Error::new("remote.socket_in_use").arg("path", path.display().to_string()));
                }
                let _ = std::fs::remove_file(path);
            }
            let listener = UnixListener::bind(path).map_err(|e| io_error(&e))?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| io_error(&e))?;
            listener.set_nonblocking(true).map_err(|e| io_error(&e))?;
            let (tx, stop, next) = (tx.clone(), stop.clone(), next.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("lm-remote-unix".into())
                    .spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            match listener.accept() {
                                Ok((s, _)) => {
                                    let _ = s.set_nonblocking(false);
                                    let conn = next.fetch_add(1, Ordering::Relaxed);
                                    let _ = spawn_conn(conn, Stream::Unix(s), true, max_frame, tx.clone());
                                }
                                Err(_) => std::thread::sleep(Duration::from_millis(20)),
                            }
                        }
                    })
                    .map_err(|e| io_error(&e))?,
            );
        }
        if let Some(addr) = config.tcp {
            let listener = TcpListener::bind(addr).map_err(|e| io_error(&e))?;
            local_addr = listener.local_addr().ok();
            listener.set_nonblocking(true).map_err(|e| io_error(&e))?;
            let (tx, stop, next) = (tx.clone(), stop.clone(), next.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("lm-remote-tcp".into())
                    .spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            match listener.accept() {
                                Ok((s, _)) => {
                                    let _ = s.set_nonblocking(false);
                                    let _ = s.set_nodelay(true);
                                    let conn = next.fetch_add(1, Ordering::Relaxed);
                                    let _ = spawn_conn(conn, Stream::Tcp(s), false, max_frame, tx.clone());
                                }
                                Err(_) => std::thread::sleep(Duration::from_millis(20)),
                            }
                        }
                    })
                    .map_err(|e| io_error(&e))?,
            );
        }
        Ok(Server {
            config,
            events: rx,
            stop,
            threads,
            conns: HashMap::new(),
            feed: None,
            root: Value::NULL,
            state: (false, false, false),
            prefs: Value::NULL,
            status: Value::NULL,
            notices: Vec::new(),
            activity: (Vec::new(), Instant::now()),
            recording: Vec::new(),
            forwards: HashMap::new(),
            params: Vec::new(),
            local_addr,
        })
    }

    /// The TCP address it listens on (with the port the system chose for `:0`)
    pub fn tcp_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    /// The clients connected and past the handshake
    pub fn clients(&self) -> usize {
        self.conns.values().filter(|c| c.stage == Stage::Ready).count()
    }

    /// On the main thread, after the port's own `pump`: answers the requests and sends what changed. True if
    /// something was done
    pub fn pump(&mut self, port: &mut dyn CorePort) -> bool {
        if self.feed.is_none() {
            self.feed = Some(port.feed(Topic::All));
            self.root = port.root().clone();
        }
        let mut did = false;
        while let Ok(ev) = self.events.try_recv() {
            did = true;
            self.event(port, ev);
        }
        // Parameter writes of this pump: the last value of each wins (I-20.1: rate limited)
        for (node, param, value) in std::mem::take(&mut self.params) {
            port.param_write(node, &param, value);
        }
        // Results of requests the port answered later
        for (rid, fin) in port.finished() {
            if let Some(f) = self.forwards.remove(&rid) {
                self.forward(f, fin);
            }
        }
        self.sync(port);
        self.broadcast_state(port);
        self.continuous(port);
        self.conns.retain(|_, c| !c.closing);
        did
    }

    fn event(&mut self, port: &mut dyn CorePort, ev: Event) {
        match ev {
            Event::Open { conn, out, stream, local, busy } => {
                let now = Instant::now();
                self.conns.insert(
                    conn,
                    Conn {
                        out,
                        stream,
                        local,
                        stage: Stage::Hello,
                        role: Role::Observe,
                        mode: Mode::Tool,
                        name: String::new(),
                        gestures: HashMap::new(),
                        watches: Vec::new(),
                        displays: Vec::new(),
                        telemetry: None,
                        last_telemetry: now,
                        last_display: now,
                        busy,
                        window: (now, 0),
                        closing: false,
                    },
                );
            }
            Event::Closed { conn } => {
                if let Some(c) = self.conns.remove(&conn) {
                    // I-20.5: its open gestures are cancelled (only their paths go back, G1)
                    for (_, g) in c.gestures {
                        port.gesture_cancel(g);
                    }
                    c.stream.shutdown();
                    self.forwards.retain(|_, f| f.conn != conn);
                    self.sync(port);
                }
            }
            Event::Frame { conn, frame } => {
                if frame.kind != KIND_JSON {
                    if let Some(c) = self.conns.get(&conn) {
                        c.send_raw_error(&bad("frame kind"));
                    }
                    return;
                }
                let msg = match wire::parse_json(&frame.payload) {
                    Ok(v) => v,
                    Err(e) => {
                        if let Some(c) = self.conns.get(&conn) {
                            c.send_raw_error(&e);
                        }
                        return;
                    }
                };
                let stage = match self.conns.get(&conn) {
                    Some(c) => c.stage,
                    None => return,
                };
                if stage == Stage::Hello {
                    self.hello(port, conn, &msg);
                } else {
                    self.message(port, conn, &msg);
                }
            }
        }
    }

    fn hello(&mut self, port: &mut dyn CorePort, conn: u64, msg: &Value) {
        let check = || -> Result<(Mode, Role, String)> {
            if text(msg, "t")? != "hello" {
                return Err(Error::new("remote.handshake.expected_hello"));
            }
            let protocol = int(msg, "protocol")?;
            let program = text(msg, "program")?;
            let mode = match text(msg, "mode")? {
                "gui" => Mode::Gui,
                "tool" => Mode::Tool,
                _ => return Err(bad("mode")),
            };
            match mode {
                Mode::Gui if protocol != PROTOCOL || program != PROGRAM => {
                    return Err(proto::mismatch(PROGRAM, program, PROTOCOL, protocol));
                }
                Mode::Tool => {
                    let api = int(msg, "tool_api")?;
                    if protocol != PROTOCOL || !TOOL_APIS.contains(&api) {
                        return Err(proto::mismatch(PROGRAM, program, PROTOCOL, protocol).arg("tool_api", api));
                    }
                }
                Mode::Gui => {}
            }
            let c = &self.conns[&conn];
            let role = if c.local {
                Role::Admin
            } else {
                let token = msg.get("token").and_then(Value::as_text).ok_or_else(|| Error::new("remote.auth.token"))?;
                // Every token is compared whole, whatever the first difference (no timing hint)
                let mut found = None;
                for (t, r) in &self.config.tokens {
                    let same =
                        t.len() == token.len() && t.bytes().zip(token.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0;
                    if same {
                        found = Some(*r);
                    }
                }
                found.ok_or_else(|| Error::new("remote.auth.token"))?
            };
            let wanted = msg.get("role").and_then(Value::as_text).and_then(Role::parse).unwrap_or(role);
            let name = msg.get("name").and_then(Value::as_text).unwrap_or("").chars().take(64).collect();
            Ok((mode, wanted.min(role), name))
        };
        match check() {
            Err(e) => {
                let c = self.conns.get_mut(&conn).expect("present");
                c.send_json(&Value::object([("t", Value::text("refused")), ("error", c::error_to(&e))]));
                c.closing = true;
                // The writer sends the refusal; the reader's end closes the connection
                let out = c.out.clone();
                drop(out);
            }
            Ok((mode, role, name)) => {
                self.sync(port);
                let snapshot = self.snapshot(port);
                let c = self.conns.get_mut(&conn).expect("present");
                c.stage = Stage::Ready;
                c.mode = mode;
                c.role = role;
                c.name = name;
                c.send_json(&Value::object([
                    ("t", Value::text("welcome")),
                    ("protocol", Value::int(PROTOCOL)),
                    ("program", Value::text(PROGRAM)),
                    ("role", Value::text(role.key())),
                    ("server", Value::text(self.config.name.as_str())),
                ]));
                if mode == Mode::Gui {
                    c.send_json(&snapshot);
                }
            }
        }
    }

    fn snapshot(&self, port: &mut dyn CorePort) -> Value {
        let notices = Value::list(port.notices().iter().filter_map(|n| c::notice_to(n).ok()));
        let activity = Value::list(port.activity().iter().filter_map(|o| c::operation_to(o).ok()));
        Value::object([
            ("t", Value::text("snapshot")),
            ("root", self.root.clone()),
            ("prefs", port.preferences().clone()),
            ("status", port.status().clone()),
            ("modified", Value::bool(port.modified())),
            ("can_undo", Value::bool(port.can_undo())),
            ("can_redo", Value::bool(port.can_redo())),
            ("notices", notices),
            ("activity", activity),
        ])
    }

    /// The tree's differences and the diff, to every mirror, after a change
    fn sync(&mut self, port: &mut dyn CorePort) {
        let Some(feed) = self.feed.as_mut() else { return };
        let Some(diff) = port.drain(feed) else { return };
        let new = port.root().clone();
        let ops = delta::delta(&self.root, &new);
        self.root = new;
        let msg = (|| -> Result<Value> {
            Ok(Value::object([
                ("t", Value::text("update")),
                ("ops", delta::to_value(&ops)?),
                ("diff", c::diff_to(&diff)?),
                ("modified", Value::bool(port.modified())),
                ("can_undo", Value::bool(port.can_undo())),
                ("can_redo", Value::bool(port.can_redo())),
            ]))
        })();
        self.state = (port.modified(), port.can_undo(), port.can_redo());
        match msg.and_then(|m| wire::json(&m)) {
            Ok(payload) => {
                for c in self.conns.values().filter(|c| c.stage == Stage::Ready && c.mode == Mode::Gui) {
                    let _ = c.out.send(Out { kind: KIND_JSON, payload: payload.clone(), continuous: false });
                }
            }
            Err(e) => {
                // A tree that does not encode cannot be mirrored: the mirrors are dropped and resync
                for c in self.conns.values_mut() {
                    c.send_raw_error(&e);
                    c.closing = true;
                    c.stream.shutdown();
                }
            }
        }
    }

    fn gui(&self) -> impl Iterator<Item = &Conn> + '_ {
        self.conns.values().filter(|c| c.stage == Stage::Ready && c.mode == Mode::Gui)
    }

    fn broadcast_state(&mut self, port: &mut dyn CorePort) {
        let state = (port.modified(), port.can_undo(), port.can_redo());
        if state != self.state {
            self.state = state;
            let v = Value::object([
                ("t", Value::text("state")),
                ("modified", Value::bool(state.0)),
                ("can_undo", Value::bool(state.1)),
                ("can_redo", Value::bool(state.2)),
            ]);
            self.gui().for_each(|c| c.send_json(&v));
        }
        let prefs = port.preferences();
        if !(prefs.same_node(&self.prefs) || *prefs == self.prefs) {
            self.prefs = prefs.clone();
            let v = Value::object([("t", Value::text("prefs")), ("value", self.prefs.clone())]);
            self.gui().for_each(|c| c.send_json(&v));
        }
        let status = port.status();
        if !(status.same_node(&self.status) || *status == self.status) {
            self.status = status.clone();
            let v = Value::object([("t", Value::text("status")), ("value", self.status.clone())]);
            self.gui().for_each(|c| c.send_json(&v));
        }
        let notices = port.notices();
        if notices != self.notices {
            let v = Value::object([
                ("t", Value::text("notices")),
                ("value", Value::list(notices.iter().filter_map(|n| c::notice_to(n).ok()))),
            ]);
            self.notices = notices;
            self.gui().for_each(|c| c.send_json(&v));
        }
        // The activity moves all the time (its `since`): at most ten times a second
        let activity = port.activity();
        let changed_set = activity.len() != self.activity.0.len()
            || activity.iter().zip(&self.activity.0).any(|(a, b)| a.id != b.id || a.stage != b.stage);
        if changed_set || (!activity.is_empty() && self.activity.1.elapsed() >= Duration::from_millis(100)) {
            let v = Value::object([
                ("t", Value::text("activity")),
                ("value", Value::list(activity.iter().filter_map(|o| c::operation_to(o).ok()))),
            ]);
            self.activity = (activity, Instant::now());
            self.gui().for_each(|c| c.send_json(&v));
        }
        let recording = port.telemetry().recording.clone();
        if recording != self.recording {
            let v = Value::object([
                ("t", Value::text("recording")),
                (
                    "value",
                    Value::list(recording.iter().map(|r| {
                        Value::object([
                            ("track", c::id_to(r.track)),
                            ("resource", c::resource_to(&r.resource)),
                            ("start", Value::int(r.start)),
                            ("length", Value::int(r.length)),
                        ])
                    })),
                ),
            ]);
            self.recording = recording;
            self.gui().for_each(|c| c.send_json(&v));
        }
        // Loading watches: sent when they change (a visible loading at most ten times a second)
        for c in self.conns.values_mut() {
            let mut msgs = Vec::new();
            for (t, last) in c.watches.iter_mut() {
                let now = port.loading(t);
                let moving =
                    matches!(now, Loading::Visible { .. }) && c.last_display.elapsed() >= Duration::from_millis(100);
                if now != *last
                    && (!matches!((&now, &*last), (Loading::Visible { .. }, Loading::Visible { .. })) || moving)
                {
                    if let Ok(tv) = c::target_to(t) {
                        msgs.push(Value::object([
                            ("t", Value::text("loading")),
                            ("target", tv),
                            ("value", c::loading_to(&now)),
                        ]));
                    }
                    *last = now;
                }
            }
            for m in msgs {
                c.send_json(&m);
            }
        }
    }

    /// Telemetry and displays, at each client's rate, never queued (I-20.4)
    fn continuous(&mut self, port: &mut dyn CorePort) {
        for c in self.conns.values_mut().filter(|c| c.stage == Stage::Ready) {
            if let Some((every, only)) = &c.telemetry
                && c.last_telemetry.elapsed() >= *every
                && !c.busy.load(Ordering::Acquire)
            {
                c.busy.store(true, Ordering::Release);
                c.last_telemetry = Instant::now();
                let payload = wire::telemetry_to(port.telemetry(), only.as_deref());
                let _ = c.out.send(Out { kind: KIND_TELEMETRY, payload, continuous: true });
            }
            if !c.displays.is_empty() && c.last_display.elapsed() >= Duration::from_millis(33) {
                c.last_display = Instant::now();
                for n in &c.displays {
                    if let Some(d) = port.display(*n) {
                        let _ = c.out.send(Out {
                            kind: KIND_DISPLAY,
                            payload: wire::display_to(*n, &d),
                            continuous: false,
                        });
                    }
                }
            }
        }
    }

    fn forward(&mut self, f: Forward, fin: Finished) {
        let Some(c) = self.conns.get(&f.conn) else { return };
        match fin {
            Finished::Peaks(Ok(p)) => {
                let _ = c.out.send(Out { kind: KIND_PEAKS, payload: wire::peaks_to(f.id, &p), continuous: false });
            }
            other => c.reply(f.id, finished_value(other)),
        }
    }

    fn message(&mut self, port: &mut dyn CorePort, conn: u64, msg: &Value) {
        // Rate limit (I-20.6)
        let limit = self.config.limits.messages_per_second;
        let (role, mode) = {
            let c = self.conns.get_mut(&conn).expect("present");
            if c.window.0.elapsed() >= Duration::from_secs(1) {
                c.window = (Instant::now(), 0);
            }
            c.window.1 += 1;
            if c.window.1 > limit {
                if let Some(id) = msg.get("id").and_then(Value::as_int).and_then(|i| u64::try_from(i).ok()) {
                    c.reply(id, Err(Error::new("remote.rate_limited")));
                }
                return;
            }
            (c.role, c.mode)
        };
        let id = msg.get("id").and_then(Value::as_int).and_then(|i| u64::try_from(i).ok());
        let r = self.dispatch(port, conn, role, mode, id, msg);
        if let Err(e) = r {
            let c = &self.conns[&conn];
            match id {
                Some(id) => c.reply(id, Err(e)),
                None => c.send_raw_error(&e),
            }
        }
    }

    fn need(role: Role, needed: Role) -> Result<()> {
        if role >= needed { Ok(()) } else { Err(Error::new("remote.forbidden").arg("role", needed.key())) }
    }

    /// One message. `Err` is answered to the request (or as an `error` message)
    fn dispatch(
        &mut self,
        port: &mut dyn CorePort,
        conn: u64,
        role: Role,
        mode: Mode,
        id: Option<u64>,
        msg: &Value,
    ) -> Result<()> {
        let t = text(msg, "t")?;
        let rid = || id.ok_or_else(|| bad("id"));
        let gui_only = |m: Mode| {
            if m == Mode::Gui { Ok(()) } else { Err(Error::new("remote.tool.unsupported").arg("message", t)) }
        };
        match t {
            "ping" => self.answer(conn, rid()?, Ok(Value::NULL)),
            "apply" => {
                Self::need(role, Role::Control)?;
                let label = label(msg)?;
                let ops = c::ops_from(get(msg, "ops")?)?;
                let p = port.apply(label, ops);
                self.pending(port, conn, rid()?, p, |a| c::applied_to(&a))
            }
            "run_action" => {
                let action = text(msg, "action")?;
                Self::need(role, if proto::admin_action(action) { Role::Admin } else { Role::Control })?;
                let p = port.run_action(ActionId::from(action), get(msg, "params")?.clone());
                self.pending(port, conn, rid()?, p, |()| Ok(Value::NULL))
            }
            "undo" | "redo" => {
                Self::need(role, Role::Control)?;
                let p = if t == "undo" { port.undo() } else { port.redo() };
                self.pending(port, conn, rid()?, p, |()| Ok(Value::NULL))
            }
            "set_preference" => {
                Self::need(role, Role::Admin)?;
                gui_only(mode)?;
                let p = port.set_preference(c::path_from(get(msg, "path")?)?, get(msg, "value")?.clone());
                self.pending(port, conn, rid()?, p, |()| Ok(Value::NULL))
            }
            "devices" => {
                gui_only(mode)?;
                let p = port.devices();
                self.pending(port, conn, rid()?, p, |d| Ok(Value::list(d.iter().map(c::device_to))))
            }
            "browse" => {
                gui_only(mode)?;
                let page = c::page_from(text(msg, "page")?)?;
                let parent = c::opt(msg, "parent").map(|_| uint(msg, "parent").map(EntryId)).transpose()?;
                let p = port.browse(page, parent, c::flag(msg, "hidden")?);
                self.pending(port, conn, rid()?, p, |e| c::entries_to(&e))
            }
            "search" => {
                gui_only(mode)?;
                let p = port.search(text(msg, "query")?);
                self.pending(port, conn, rid()?, p, |e| c::entries_to(&e))
            }
            "prepare_preview" => {
                Self::need(role, Role::Control)?;
                gui_only(mode)?;
                let p = port.prepare_preview(EntryId(uint(msg, "entry")?));
                self.pending(port, conn, rid()?, p, |p| c::preview_to(&p))
            }
            "set_favorite" => {
                Self::need(role, Role::Control)?;
                gui_only(mode)?;
                let p = port.set_favorite(EntryId(uint(msg, "entry")?), c::flag(msg, "on")?);
                self.pending(port, conn, rid()?, p, |()| Ok(Value::NULL))
            }
            "peaks" => {
                gui_only(mode)?;
                let r = c::resource_from(get(msg, "resource")?)?;
                let level = u8::try_from(int(msg, "level")?).map_err(|_| bad("level"))?;
                let request = rid()?;
                match port.peaks(&r, level, uint(msg, "from")?, uint(msg, "to")?) {
                    Pending::Ready(Ok(p)) => {
                        let c = &self.conns[&conn];
                        let _ = c.out.send(Out {
                            kind: KIND_PEAKS,
                            payload: wire::peaks_to(request, &p),
                            continuous: false,
                        });
                    }
                    Pending::Ready(Err(e)) => self.conns[&conn].reply(request, Err(e)),
                    Pending::Waiting(w) => {
                        self.forwards.insert(w, Forward { conn, id: request });
                    }
                }
                Ok(())
            }
            "gesture_begin" => {
                Self::need(role, Role::Control)?;
                let g = uint(msg, "g")?;
                let gid = port.begin_gesture(label(msg)?);
                let c = self.conns.get_mut(&conn).expect("present");
                if let Some(old) = c.gestures.insert(g, gid) {
                    port.gesture_cancel(old);
                }
                Ok(())
            }
            "gesture_apply" => {
                Self::need(role, Role::Control)?;
                let ops = c::ops_from(get(msg, "ops")?)?;
                let g = uint(msg, "g")?;
                let gid = self.conns[&conn].gestures.get(&g).ok_or_else(|| Error::new("doc.gesture.closed"))?;
                port.gesture_apply(gid, ops);
                self.sync(port);
                Ok(())
            }
            "gesture_commit" | "gesture_cancel" => {
                Self::need(role, Role::Control)?;
                let g = uint(msg, "g")?;
                let gid = self
                    .conns
                    .get_mut(&conn)
                    .expect("present")
                    .gestures
                    .remove(&g)
                    .ok_or_else(|| Error::new("doc.gesture.closed"))?;
                if t == "gesture_commit" {
                    port.gesture_commit(gid);
                } else {
                    port.gesture_cancel(gid);
                }
                self.sync(port);
                Ok(())
            }
            "param" => {
                Self::need(role, Role::Control)?;
                let node = c::id_from(get(msg, "node")?)?;
                let param = text(msg, "param")?.to_owned();
                let value = real(msg, "value")? as f32;
                if value.is_finite() {
                    self.params.retain(|(n, p, _)| !(*n == node && *p == param));
                    self.params.push((node, param, value));
                }
                Ok(())
            }
            "cancel" => {
                Self::need(role, Role::Control)?;
                port.cancel(OperationId(uint(msg, "op")?));
                Ok(())
            }
            "dismiss" => {
                gui_only(mode)?;
                port.dismiss_notice(uint(msg, "notice")?);
                Ok(())
            }
            "preview_play" => {
                Self::need(role, Role::Control)?;
                port.preview_play(EntryId(uint(msg, "entry")?), int(msg, "from")?);
                Ok(())
            }
            "preview_stop" => {
                Self::need(role, Role::Control)?;
                port.preview_stop();
                Ok(())
            }
            "watch" => {
                gui_only(mode)?;
                let target = c::target_from(get(msg, "target")?)?;
                let on = c::flag(msg, "on")?;
                let max = self.config.limits.max_watches;
                let c = self.conns.get_mut(&conn).expect("present");
                c.watches.retain(|(t, _)| *t != target);
                if on {
                    if c.watches.len() >= max {
                        return Err(Error::new("remote.limit.watches"));
                    }
                    let now = port.loading(&target);
                    if let Ok(tv) = c::target_to(&target) {
                        c.send_json(&Value::object([
                            ("t", Value::text("loading")),
                            ("target", tv),
                            ("value", c::loading_to(&now)),
                        ]));
                    }
                    c.watches.push((target, now));
                }
                Ok(())
            }
            "display" => {
                gui_only(mode)?;
                let node = c::id_from(get(msg, "node")?)?;
                let on = c::flag(msg, "on")?;
                let max = self.config.limits.max_watches;
                let c = self.conns.get_mut(&conn).expect("present");
                c.displays.retain(|n| *n != node);
                if on {
                    if c.displays.len() >= max {
                        return Err(Error::new("remote.limit.watches"));
                    }
                    c.displays.push(node);
                }
                Ok(())
            }
            "telemetry" => {
                // Up to 60 Hz (I-20.3); tools get at most 10 Hz (the stable subset's reduced rate)
                let hz = real(msg, "hz")?;
                let max = if mode == Mode::Gui { 60.0 } else { 10.0 };
                let only = match c::opt(msg, "nodes") {
                    Some(l) if l.is_list() => Some(l.items().map(c::id_from).collect::<Result<Vec<_>>>()?),
                    Some(_) => return Err(bad("nodes")),
                    None => None,
                };
                let c = self.conns.get_mut(&conn).expect("present");
                c.telemetry = (hz > 0.0).then(|| (Duration::from_secs_f64(1.0 / hz.min(max)), only));
                Ok(())
            }
            // The stable subset's reading (tools have no mirror)
            "find" => {
                let node = c::id_from(get(msg, "node")?)?;
                let v = port.find(node).unwrap_or(Value::NULL);
                self.answer(conn, rid()?, Ok(v))
            }
            "get" => {
                let path = c::path_from(get(msg, "path")?)?;
                let v = port.root().at_path(&path).cloned().unwrap_or(Value::NULL);
                self.answer(conn, rid()?, Ok(v))
            }
            "health" => {
                let tel = port.telemetry();
                let v = Value::object([
                    ("program", Value::text(PROGRAM)),
                    ("protocol", Value::int(PROTOCOL)),
                    ("server", Value::text(self.config.name.as_str())),
                    ("clients", Value::int(self.clients() as i64)),
                    ("modified", Value::bool(port.modified())),
                    ("playing", Value::bool(tel.position.playing)),
                    ("recording", Value::bool(tel.position.recording)),
                    ("xruns", Value::int(i64::from(tel.xruns))),
                ]);
                self.answer(conn, rid()?, Ok(v))
            }
            _ => Err(Error::new("remote.unknown_message").arg("t", t)),
        }
    }

    fn answer(&mut self, conn: u64, id: u64, r: Result<Value>) -> Result<()> {
        self.conns[&conn].reply(id, r);
        Ok(())
    }

    /// Answers a port result now, or later when the port finishes it. A change is mirrored before its answer
    fn pending<T>(
        &mut self,
        port: &mut dyn CorePort,
        conn: u64,
        id: u64,
        p: Pending<T>,
        to: impl FnOnce(T) -> Result<Value>,
    ) -> Result<()> {
        match p {
            Pending::Ready(r) => {
                self.sync(port);
                let v = r.and_then(to);
                self.conns[&conn].reply(id, v);
            }
            Pending::Waiting(w) => {
                self.forwards.insert(w, Forward { conn, id });
            }
        }
        Ok(())
    }
}

fn label(msg: &Value) -> Result<Label> {
    c::label_from(get(msg, "label")?)
}

fn finished_value(f: Finished) -> Result<Value> {
    match f {
        Finished::Applied(r) => r.and_then(|a| c::applied_to(&a)),
        Finished::Done(r) => r.map(|()| Value::NULL),
        Finished::Peaks(r) => r.map(|_| Value::NULL),
        Finished::Devices(r) => r.map(|d| Value::list(d.iter().map(c::device_to))),
        Finished::Entries(r) => r.and_then(|e| c::entries_to(&e)),
        Finished::Preview(r) => r.and_then(|p| c::preview_to(&p)),
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for c in self.conns.values() {
            c.stream.shutdown();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        if let Some(p) = &self.config.socket {
            let _ = std::fs::remove_file(p);
        }
    }
}
