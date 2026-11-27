//! I-15 and I-20: the same interface script against the LocalPort and against the RemotePort (through a local
//! socket) gives the same tree and the same diffs; a dropped connection reconnects and resyncs; a version mismatch is
//! a clear error; roles; the decoders and the server survive garbage.

// The decoder fuzz reports its seed and rounds on the standard error: the nightly job's log needs them to reproduce a
// failure (I-20.8)
#![allow(clippy::print_stderr)]

use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use lm_base::{Label, Result};
use lm_doc::{Diff, Topic};
use lm_port::{ActionId, Applied, CorePort, EditOp, Finished, NodeId, Pending, Ref, RequestId, Target, Value};
use lm_port_fake::{Config, FakePort};
use lm_remote::{ClientConfig, Connection, Endpoint, RemotePort, Role, Server, ServerConfig, wire};
use lm_state::{PathStep, ROOT};

fn fake() -> FakePort {
    FakePort::new(Config { tracks: 4, slow_delay: None, slow_track: 0 }).expect("fake")
}

fn schemas() -> Arc<lm_state::SchemaRegistry> {
    Arc::new(lm_port_fake::demo::registry().expect("registry"))
}

fn socket_path() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!("lmr-{}-{}.sock", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// A server serving a fake core, and its client, pumped together on this thread
struct Remote {
    core: FakePort,
    server: Server,
    client: RemotePort,
    done: Vec<(RequestId, Finished)>,
}

impl Remote {
    fn unix() -> Remote {
        let path = socket_path();
        let mut cfg = ServerConfig::new("test");
        cfg.socket = Some(path.clone());
        Self::with(cfg, move |_| Endpoint::Unix(path), None)
    }

    fn with(cfg: ServerConfig, endpoint: impl FnOnce(&Server) -> Endpoint, token: Option<&str>) -> Remote {
        let mut core = fake();
        let mut server = Server::start(cfg).expect("server");
        let endpoint = endpoint(&server);
        // The client's handshake waits for the server's pump: run it on a thread meanwhile
        let mut ccfg = ClientConfig::new(endpoint, schemas());
        ccfg.token = token.map(str::to_owned);
        let mut connecting = RemotePort::start(ccfg);
        let client = loop {
            core.pump();
            server.pump(&mut core);
            if let Some(r) = connecting.poll() {
                break r.expect("connect");
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        Remote { core, server, client, done: Vec::new() }
    }

    fn step(&mut self) {
        self.core.pump();
        self.server.pump(&mut self.core);
        self.client.pump();
        self.done.extend(self.client.finished());
    }

    fn wait(&mut self, id: RequestId) -> Finished {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(i) = self.done.iter().position(|(r, _)| *r == id) {
                return self.done.remove(i).1;
            }
            assert!(Instant::now() < end, "no answer to request {id}");
            self.step();
            std::thread::sleep(Duration::from_micros(200));
        }
    }
}

/// What the script needs besides the port: waiting for results, and a point where everything sent has been seen
trait Drive {
    fn port(&mut self) -> &mut dyn CorePort;
    /// The core's outside world (MIDI, the engine's reports), as the fake has it
    fn outside(&self) -> lm_port_fake::Outside;
    fn learned(&mut self, p: Pending<lm_port::LearnId>) -> Result<lm_port::LearnId>;
    fn applied(&mut self, p: Pending<Applied>) -> Result<Applied>;
    fn unit(&mut self, p: Pending<()>) -> Result<()>;
    fn barrier(&mut self);
}

struct Local(FakePort);

impl Drive for Local {
    fn port(&mut self) -> &mut dyn CorePort {
        &mut self.0
    }
    fn outside(&self) -> lm_port_fake::Outside {
        self.0.outside()
    }
    fn learned(&mut self, p: Pending<lm_port::LearnId>) -> Result<lm_port::LearnId> {
        match p {
            Pending::Ready(r) => r,
            Pending::Waiting(_) => panic!("the LocalPort never waits"),
        }
    }
    fn applied(&mut self, p: Pending<Applied>) -> Result<Applied> {
        match p {
            Pending::Ready(r) => r,
            Pending::Waiting(_) => panic!("the LocalPort never waits"),
        }
    }
    fn unit(&mut self, p: Pending<()>) -> Result<()> {
        match p {
            Pending::Ready(r) => r,
            Pending::Waiting(_) => panic!("the LocalPort never waits"),
        }
    }
    fn barrier(&mut self) {
        self.0.pump();
    }
}

impl Drive for Remote {
    fn port(&mut self) -> &mut dyn CorePort {
        &mut self.client
    }
    fn outside(&self) -> lm_port_fake::Outside {
        self.core.outside()
    }
    fn learned(&mut self, p: Pending<lm_port::LearnId>) -> Result<lm_port::LearnId> {
        match p {
            Pending::Ready(r) => r,
            Pending::Waiting(id) => match self.wait(id) {
                Finished::Learned(r) => r,
                other => panic!("{other:?}"),
            },
        }
    }
    fn applied(&mut self, p: Pending<Applied>) -> Result<Applied> {
        match p {
            Pending::Ready(r) => r,
            Pending::Waiting(id) => match self.wait(id) {
                Finished::Applied(r) => r,
                other => panic!("{other:?}"),
            },
        }
    }
    fn unit(&mut self, p: Pending<()>) -> Result<()> {
        match p {
            Pending::Ready(r) => r,
            Pending::Waiting(id) => match self.wait(id) {
                Finished::Done(r) => r,
                other => panic!("{other:?}"),
            },
        }
    }
    fn barrier(&mut self) {
        let p = self.client.ping();
        self.unit(p).expect("ping");
    }
}

/// What a script saw
#[derive(Debug, PartialEq)]
struct Seen {
    diffs: Vec<(&'static str, Option<Diff>, Option<Diff>)>,
    results: Vec<String>,
    root: Value,
    flags: (bool, bool, bool),
    /// the interface document (I-3.8)
    ui: Value,
}

fn label(k: &'static str) -> Label {
    Label::new(k)
}

/// The interface's script: edits, a gesture, parameter writes, undo, redo, an action, a refused edit
fn script(d: &mut dyn Drive) -> Seen {
    let mut all = d.port().feed(Topic::All);
    let mut tracks = d.port().feed(Topic::Type("track"));
    let mut seen = Seen {
        diffs: Vec::new(),
        results: Vec::new(),
        root: Value::NULL,
        flags: (false, false, false),
        ui: Value::NULL,
    };
    let mut look = |d: &mut dyn Drive, step: &'static str, seen: &mut Seen| {
        d.barrier();
        let a = d.port().drain(&mut all);
        let t = d.port().drain(&mut tracks);
        seen.diffs.push((step, a, t));
    };
    let first =
        d.port().root().get("tracks").and_then(|t| t.at(0)).and_then(|t| t.get("id")).cloned().expect("a track");
    let first = NodeId::parse(first.as_text().unwrap()).unwrap();

    // A new track with an item and two notes, by temporary numbers
    let ops = vec![
        EditOp::Insert {
            temp: 1,
            parent: Ref::Id(ROOT),
            list: "tracks".into(),
            index: 1,
            node: Value::object([("name", Value::text("new"))]),
        },
        EditOp::Insert {
            temp: 2,
            parent: Ref::Temp(1),
            list: "items".into(),
            index: 0,
            node: Value::object([("kind", Value::text("midi")), ("start", Value::int(960))]),
        },
        EditOp::AddNote {
            temp: 3,
            item: Ref::Temp(2),
            note: Value::object([("key", Value::int(60)), ("start", Value::int(0))]),
        },
        EditOp::AddNote {
            temp: 4,
            item: Ref::Temp(2),
            note: Value::object([("key", Value::int(64)), ("start", Value::int(240))]),
        },
    ];
    let p = d.port().apply(label("test.add"), ops);
    let a = d.applied(p).expect("apply");
    seen.results.push(format!("{:?} {:?}", a.created, a.notes));
    look(d, "add", &mut seen);
    let item = a.created.iter().find(|(t, _)| *t == 2).unwrap().1;
    let n = a.notes[0].1;

    // A note moved, a name set
    let p = d.port().apply(
        label("test.move"),
        vec![
            EditOp::SetNote {
                item: Ref::Id(item),
                n: lm_port::NoteRef::N(n),
                path: vec![PathStep::key("start")],
                value: Value::int(480),
            },
            EditOp::set(first, &["name"], Value::text("renamed")),
        ],
    );
    d.applied(p).expect("move");
    look(d, "move", &mut seen);

    // A gesture: three updates, committed: one step
    let g = d.port().begin_gesture(label("test.drag"));
    for v in [0.3, 0.5, 0.7] {
        d.port().gesture_apply(&g, vec![EditOp::set(first, &["volume"], Value::real(v).unwrap())]);
        d.port().param_write(first, "volume", v as f32);
    }
    d.port().gesture_commit(g);
    look(d, "gesture", &mut seen);

    // A cancelled gesture leaves nothing
    let g = d.port().begin_gesture(label("test.drag2"));
    d.port().gesture_apply(&g, vec![EditOp::set(first, &["volume"], Value::real(0.1).unwrap())]);
    d.port().gesture_cancel(g);
    look(d, "cancelled", &mut seen);

    // Undo, redo, undo
    for (k, undo) in [("undo", true), ("redo", false), ("undo2", true)] {
        let p = if undo { d.port().undo() } else { d.port().redo() };
        d.unit(p).expect("undo/redo");
        look(d, k, &mut seen);
    }

    // A refused edit: nothing changes, the error comes back
    let p =
        d.port().apply(label("test.bad"), vec![EditOp::Remove { id: Ref::Id(NodeId { kind: b't', number: 999_999 }) }]);
    seen.results.push(format!("{:?}", d.applied(p).map(|_| ()).map_err(|e| e.key())));
    look(d, "refused", &mut seen);

    // A domain action
    let p = d.port().run_action(ActionId::from("transport.locate"), Value::int(4800));
    seen.results.push(format!("{:?}", d.unit(p).map_err(|e| e.key())));
    look(d, "action", &mut seen);

    // The interface document (I-3.8): the selection
    let sel =
        Value::object([("track", Value::text(first.to_string())), ("channel", Value::NULL), ("plugin", Value::NULL)]);
    let p = d.port().set_ui(vec![PathStep::key("selection")], sel);
    d.unit(p).expect("set_ui");
    d.barrier();
    seen.ui = d.port().ui().clone();

    // MIDI learn (I-21.2): the first control moved makes the mapping; the activity shows it
    let out = d.outside();
    let volume = lm_port::MapTarget::Param { node: lm_port::NodeRef::Id(first), key: "volume".into() };
    let p = d.port().midi_learn(volume.clone(), lm_port::MapScope::Project);
    let learn = d.learned(p).expect("learn");
    d.barrier();
    seen.results.push(format!("learning {:?}", d.port().learning() == Some(learn)));
    out.midi("Controlador 1", 0, lm_port::MidiMessage::Cc { number: 7 }, 64);
    d.barrier();
    d.barrier();
    seen.results.push(format!("learned {:?}", d.port().learning()));
    let (_, act) = d.port().midi_activity(0);
    seen.results.push(format!("activity {:?}", act.iter().map(|a| (a.channel, a.value, a.mapped)).collect::<Vec<_>>()));
    look(d, "learned", &mut seen);

    // A mapped interface action (machine scope) reaches the interface's feed
    let zoom = lm_port::MapTarget::Action { id: "arrange.zoom_in".into(), params: Value::NULL, value_into: None };
    let p = d.port().midi_learn(zoom, lm_port::MapScope::Machine);
    d.learned(p).expect("learn");
    d.barrier();
    for _ in 0..2 {
        out.midi("Controlador 1", 0, lm_port::MidiMessage::Note { key: 36 }, 127);
        d.barrier();
        d.barrier();
    }
    let mut ui = Vec::new();
    d.port().drain_midi_ui(&mut ui);
    seen.results.push(format!("ui {:?}", ui.iter().map(|a| a.action.as_str().to_owned()).collect::<Vec<_>>()));

    // The hand won on a lane (I-21.5), until re-armed
    out.hand_wins(first, "volume");
    d.barrier();
    d.barrier();
    seen.results.push(format!("hand {:?}", d.port().telemetry().hand_won));
    let p = d.port().run_action(ActionId::from("automation.rearm"), Value::empty_object());
    d.unit(p).expect("rearm");
    d.barrier();
    d.barrier();
    seen.results.push(format!("rearmed {:?}", d.port().telemetry().hand_won));

    // Unmapping
    let p = d.port().unmap(&volume, lm_port::MapScope::Project);
    d.unit(p).expect("unmap");
    look(d, "unmapped", &mut seen);

    seen.root = d.port().root().without("meta");
    seen.flags = (d.port().modified(), d.port().can_undo(), d.port().can_redo());
    seen
}

/// Where two trees first differ, for a readable failure
fn first_difference(a: &Value, b: &Value, at: String) -> Option<String> {
    if a == b {
        return None;
    }
    if a.is_object() && b.is_object() {
        for k in a.keys().chain(b.keys()) {
            match (a.get(k), b.get(k)) {
                (Some(x), Some(y)) => {
                    if let Some(d) = first_difference(x, y, format!("{at}/{k}")) {
                        return Some(d);
                    }
                }
                _ => return Some(format!("{at}/{k}: {:?} vs {:?}", a.get(k), b.get(k))),
            }
        }
    }
    if a.is_list() && b.is_list() && a.len() == b.len() {
        for (i, (x, y)) in a.items().zip(b.items()).enumerate() {
            if let Some(d) = first_difference(x, y, format!("{at}/{i}")) {
                return Some(d);
            }
        }
    }
    Some(format!("{at}: {a:?} vs {b:?}"))
}

#[test]
fn the_same_script_gives_the_same_tree_and_diffs() {
    let mut local = Local(fake());
    let a = script(&mut local);
    let mut remote = Remote::unix();
    let b = script(&mut remote);
    assert_eq!(a.results, b.results);
    for (x, y) in a.diffs.iter().zip(&b.diffs) {
        assert_eq!(x, y, "after {}", x.0);
    }
    assert_eq!(a.diffs.len(), b.diffs.len());
    // The steps did change things (the comparison is not of empty diffs)
    assert!(a.diffs.iter().filter(|d| d.1.is_some() && d.2.is_some()).count() >= 6, "{:?}", a.diffs);
    // Two cores differ only in what is drawn at random when a project is made (its uuid and seed)
    let no_seed = |v: &Value| v.with("project", v.get("project").unwrap().without("seed"));
    assert_eq!(first_difference(&no_seed(&a.root), &no_seed(&b.root), String::new()), None);
    assert_eq!(a.flags, b.flags);
    assert_eq!(a.ui, b.ui, "the interface document");
    assert!(a.ui.get("selection").is_some());
    // the I-21 steps did something on both sides
    assert!(a.results.iter().any(|r| r.contains("arrange.zoom_in")), "{:?}", a.results);
    assert!(
        a.results.iter().any(|r| r.starts_with("hand [")) && !a.results.iter().any(|r| r == "hand []"),
        "{:?}",
        a.results
    );
    // The server's core has the same tree as the mirror
    assert_eq!(remote.core.root().without("meta"), b.root);
}

#[test]
fn a_dropped_connection_reconnects_and_resyncs() {
    let mut r = Remote::unix();
    let mut feed = r.client.feed(Topic::All);
    r.client.disconnect();
    // While it is down, the core changes (another client, the session itself)
    let first =
        NodeId::parse(r.core.root().get("tracks").unwrap().at(0).unwrap().get("id").unwrap().as_text().unwrap())
            .unwrap();
    let p = r.core.apply(label("test.offline"), vec![EditOp::set(first, &["name"], Value::text("changed meanwhile"))]);
    assert!(matches!(p, Pending::Ready(Ok(_))));
    let end = Instant::now() + Duration::from_secs(10);
    let mut saw_reconnecting = false;
    loop {
        r.step();
        match r.client.connection() {
            Connection::Reconnecting { .. } => saw_reconnecting = true,
            Connection::Connected { .. } if saw_reconnecting => break,
            Connection::Failed(e) => panic!("{e}"),
            _ => {}
        }
        assert!(Instant::now() < end, "did not reconnect");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(r.client.root().without("meta"), r.core.root().without("meta"));
    let d = r.client.drain(&mut feed).expect("the resync's diff");
    assert!(d.changed.iter().any(|c| c.node == first));
    // And it works as before
    let p = r.client.apply(label("test.after"), vec![EditOp::set(first, &["name"], Value::text("again"))]);
    r.applied(p).expect("apply after the reconnection");
    assert_eq!(r.client.find(first).unwrap().get("name").unwrap().as_text(), Some("again"));
}

fn raw_hello(addr: std::net::SocketAddr, hello: &Value) -> Value {
    let mut s = TcpStream::connect(addr).unwrap();
    wire::write_frame(&mut s, wire::KIND_JSON, &wire::json(hello).unwrap()).unwrap();
    s.flush().unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let f = wire::read_frame(&mut s, 1 << 20).unwrap();
    wire::parse_json(&f.payload).unwrap()
}

fn tcp_server(tokens: Vec<(String, Role)>) -> (FakePort, Server, std::net::SocketAddr) {
    let mut cfg = ServerConfig::new("tcp");
    cfg.tcp = Some("127.0.0.1:0".parse().unwrap());
    cfg.tokens = tokens;
    let server = Server::start(cfg).unwrap();
    let addr = server.tcp_addr().unwrap();
    (fake(), server, addr)
}

#[test]
fn a_version_mismatch_is_a_clear_error() {
    let (mut core, mut server, addr) = tcp_server(vec![("secret".into(), Role::Admin)]);
    let hello = Value::object([
        ("t", Value::text("hello")),
        ("protocol", Value::int(lm_remote::PROTOCOL)),
        ("program", Value::text("0.0.1-other")),
        ("mode", Value::text("gui")),
        ("token", Value::text("secret")),
    ]);
    let t = std::thread::spawn(move || raw_hello(addr, &hello));
    while !t.is_finished() {
        server.pump(&mut core);
        std::thread::sleep(Duration::from_millis(1));
    }
    let answer = t.join().unwrap();
    assert_eq!(answer.get("t").unwrap().as_text(), Some("refused"));
    let e = lm_remote::codec::error_from(answer.get("error").unwrap()).unwrap();
    assert_eq!(e.key(), "remote.version_mismatch");
    let args: Vec<_> = e.label.args.iter().map(|(k, _)| *k).collect();
    assert!(args.contains(&"server") && args.contains(&"client"));
    // A wrong token is refused too
    let hello = Value::object([
        ("t", Value::text("hello")),
        ("protocol", Value::int(lm_remote::PROTOCOL)),
        ("program", Value::text(lm_remote::PROGRAM)),
        ("mode", Value::text("gui")),
        ("token", Value::text("wrong")),
    ]);
    let t = std::thread::spawn(move || raw_hello(addr, &hello));
    while !t.is_finished() {
        server.pump(&mut core);
        std::thread::sleep(Duration::from_millis(1));
    }
    let e = lm_remote::codec::error_from(t.join().unwrap().get("error").unwrap()).unwrap();
    assert_eq!(e.key(), "remote.auth.token");
}

#[test]
fn an_observer_changes_nothing() {
    let mut cfg = ServerConfig::new("tcp");
    cfg.tcp = Some("127.0.0.1:0".parse().unwrap());
    cfg.tokens = vec![("look".into(), Role::Observe)];
    let mut r = Remote::with(cfg, |s| Endpoint::Tcp(s.tcp_addr().unwrap().to_string()), Some("look"));
    let before = r.core.root().clone();
    let first =
        NodeId::parse(before.get("tracks").unwrap().at(0).unwrap().get("id").unwrap().as_text().unwrap()).unwrap();
    let p = r.client.apply(label("x"), vec![EditOp::set(first, &["name"], Value::text("no"))]);
    assert_eq!(r.applied(p).unwrap_err().key(), "remote.forbidden");
    let p = r.client.undo();
    assert_eq!(r.unit(p).unwrap_err().key(), "remote.forbidden");
    let p = r.client.run_action(ActionId::from("project.save"), Value::NULL);
    assert_eq!(r.unit(p).unwrap_err().key(), "remote.forbidden");
    let g = r.client.begin_gesture(label("g"));
    r.client.gesture_apply(&g, vec![EditOp::set(first, &["name"], Value::text("no"))]);
    r.client.gesture_commit(g);
    r.client.param_write(first, "volume", 0.1);
    r.barrier();
    assert!(r.core.root().same_node(&before) || *r.core.root() == before);
    // But it reads and watches
    assert_eq!(r.client.loading(&Target::Project), lm_port::Loading::Ready);
    assert!(r.client.find(first).is_some());
}

#[test]
fn peaks_come_binary_and_are_kept() {
    let mut r = Remote::unix();
    // A resource of the fake's project: the first lane of the first item that has one
    let res = r
        .core
        .root()
        .get("tracks")
        .unwrap()
        .items()
        .filter_map(|t| t.get("items")?.at(0)?.get("lanes")?.at(0)?.get("source")?.as_resource().cloned())
        .next()
        .expect("a resource");
    let local = match r.core.peaks(&res, 2, 0, 500) {
        Pending::Ready(Ok(b)) => b,
        other => panic!("{other:?}"),
    };
    let p = r.client.peaks(&res, 2, 0, 500);
    let got = match p {
        Pending::Waiting(id) => match r.wait(id) {
            Finished::Peaks(Ok(b)) => b,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert_eq!(got, local);
    // Asked again: from the client's cache, at once
    assert!(matches!(r.client.peaks(&res, 2, 0, 500), Pending::Ready(Ok(_))));
}

/// A block asked again while its first request is in flight shares that request
#[test]
fn peaks_asked_twice_in_flight_share_one_request() {
    let mut r = Remote::unix();
    let res = r
        .core
        .root()
        .get("tracks")
        .unwrap()
        .items()
        .filter_map(|t| t.get("items")?.at(0)?.get("lanes")?.at(0)?.get("source")?.as_resource().cloned())
        .next()
        .expect("a resource");
    let (Pending::Waiting(a), Pending::Waiting(b)) = (r.client.peaks(&res, 2, 0, 500), r.client.peaks(&res, 2, 0, 500))
    else {
        panic!("both wait")
    };
    assert_eq!(a, b);
    assert!(matches!(r.wait(a), Finished::Peaks(Ok(_))));
}

#[test]
fn telemetry_and_session_state_reach_the_mirror() {
    let mut r = Remote::unix();
    let p = r.client.run_action(ActionId::from("transport.play"), Value::NULL);
    r.unit(p).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while !r.client.telemetry().position.playing {
        r.step();
        assert!(Instant::now() < end, "no telemetry");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(r.client.telemetry().position.sample_rate, 48_000);
    assert_eq!(r.client.preferences(), r.core.preferences());
    assert_eq!(r.client.status(), r.core.status());
    assert_eq!(r.client.notices(), r.core.notices());
}

// ---------------------------------------------------------------------------------------------------------------
// Garbage

#[derive(Debug)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Valid messages of every kind, to mutate
fn samples() -> Vec<Vec<u8>> {
    let first = Value::text("t:1");
    let msgs = [
        Value::object([
            ("t", Value::text("apply")),
            ("id", Value::int(1)),
            ("label", lm_remote::codec::label_to(&label("x"))),
            (
                "ops",
                Value::list([Value::object([
                    ("op", Value::text("set")),
                    ("id", Value::object([("id", first.clone())])),
                    ("path", Value::list([Value::text("name")])),
                    ("value", Value::text("y")),
                ])]),
            ),
        ]),
        Value::object([
            ("t", Value::text("gesture_begin")),
            ("g", Value::int(1)),
            ("label", lm_remote::codec::label_to(&label("g"))),
        ]),
        Value::object([("t", Value::text("gesture_apply")), ("g", Value::int(1)), ("ops", Value::list([]))]),
        Value::object([
            ("t", Value::text("param")),
            ("node", first.clone()),
            ("param", Value::text("volume")),
            ("value", Value::real(0.5).unwrap()),
        ]),
        Value::object([
            ("t", Value::text("peaks")),
            ("id", Value::int(2)),
            ("resource", Value::text("x")),
            ("level", Value::int(1)),
            ("from", Value::int(0)),
            ("to", Value::int(9)),
        ]),
        Value::object([
            ("t", Value::text("watch")),
            ("target", Value::object([("kind", Value::text("node")), ("id", first)])),
            ("on", Value::bool(true)),
        ]),
        Value::object([("t", Value::text("telemetry")), ("hz", Value::int(1000))]),
        Value::object([("t", Value::text("undo")), ("id", Value::int(3))]),
        Value::object([
            ("t", Value::text("browse")),
            ("id", Value::int(4)),
            ("page", Value::text("samples")),
            ("parent", Value::NULL),
            ("hidden", Value::bool(false)),
        ]),
    ];
    msgs.iter().map(|m| wire::json(m).unwrap()).collect()
}

fn mutate(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
    let mut b = base.to_vec();
    for _ in 0..1 + rng.below(6) {
        match rng.below(4) {
            0 if !b.is_empty() => {
                let i = rng.below(b.len());
                b[i] = rng.next() as u8;
            }
            1 if !b.is_empty() => {
                let i = rng.below(b.len());
                b.truncate(i);
            }
            2 => {
                let i = rng.below(b.len() + 1);
                b.insert(i, b"{}[]\":,0-e9tf"[rng.below(13)]);
            }
            _ => {
                let i = rng.below(b.len() + 1);
                let j = rng.below(b.len() + 1);
                let (a, z) = (i.min(j), i.max(j));
                let piece = b[a..z].to_vec();
                b.splice(a..a, piece);
            }
        }
    }
    b
}

/// A random value built from the protocol's own words, so the decoders get past their first checks
fn random_value(rng: &mut Rng, depth: usize) -> Value {
    const KEYS: &[&str] = &[
        "t",
        "id",
        "op",
        "path",
        "value",
        "node",
        "ops",
        "label",
        "key",
        "args",
        "diff",
        "created",
        "removed",
        "changed",
        "paths",
        "notes",
        "n",
        "temp",
        "parent",
        "list",
        "index",
        "item",
        "note",
        "kind",
        "target",
        "state",
        "progress",
        "stage",
        "since",
        "error",
        "resource",
        "severity",
        "actions",
        "repeats",
        "read",
        "origin",
        "structural",
        "params",
        "items",
        "keep",
        "entry",
        "length",
        "rate",
        "name",
        "factory",
        "hidden",
        "favorite",
        "info",
        "ok",
        "err",
        "cancellable",
        "scan",
        "level",
        "from",
        "to",
        "g",
        "hz",
        "nodes",
        "on",
    ];
    const TEXTS: &[&str] = &[
        "set",
        "insert",
        "remove",
        "move",
        "add_note",
        "set_note",
        "remove_note",
        "list",
        "t:1",
        "m:0",
        "p:3",
        "user",
        "undo",
        "ready",
        "visible",
        "failed",
        "missing",
        "node",
        "project",
        "scan",
        "library",
        "info",
        "warning",
        "audio",
        "folder",
        "",
        "x",
    ];
    match if depth > 4 { rng.below(5) } else { rng.below(8) } {
        0 => Value::NULL,
        1 => Value::bool(rng.below(2) == 0),
        2 => Value::int([0, 1, -1, 7, 1 << 40, i64::MAX >> 11, -(1 << 52)][rng.below(7)]),
        3 => Value::real([0.0, -0.5, 1.5, 1e300, -1e-300][rng.below(5)]).unwrap(),
        4 => Value::text(TEXTS[rng.below(TEXTS.len())]),
        5 | 6 => Value::object((0..rng.below(6)).map(|_| (KEYS[rng.below(KEYS.len())], random_value(rng, depth + 1)))),
        _ => Value::list((0..rng.below(5)).map(|_| random_value(rng, depth + 1))),
    }
}

#[test]
fn the_decoders_never_panic() {
    let seed = std::env::var("LM_FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(42u64);
    let rounds: u64 = std::env::var("LM_FUZZ_ROUNDS").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000);
    // Run for a duration instead (the nightly hour, I-20.8)
    let seconds: Option<u64> = std::env::var("LM_FUZZ_SECONDS").ok().and_then(|s| s.parse().ok());
    eprintln!("decoder fuzz: seed {seed}, {}", seconds.map_or(format!("{rounds} rounds"), |s| format!("{s} s")));
    let mut rng = Rng(seed);
    let samples = samples();
    let core = lm_port_fake::FakePort::new(Config { tracks: 3, slow_delay: None, slow_track: 0 }).unwrap();
    let mut telemetry = core.telemetry().clone();
    let tree = core.root().clone();
    let start = Instant::now();
    let mut round = 0u64;
    loop {
        match seconds {
            Some(s) if start.elapsed() >= Duration::from_secs(s) => break,
            None if round >= rounds => break,
            _ => {}
        }
        round += 1;
        let bytes = match rng.below(4) {
            0 => (0..rng.below(64)).map(|_| rng.next() as u8).collect(),
            1 => wire::json(&random_value(&mut rng, 0)).unwrap_or_default(),
            _ => {
                let base = &samples[rng.below(samples.len())];
                mutate(&mut rng, base)
            }
        };
        let result = std::panic::catch_unwind(|| {
            // Every decoder of the protocol, on whatever comes
            if let Ok(v) = wire::parse_json(&bytes) {
                let _ = lm_remote::codec::ops_from(&v);
                let _ = lm_remote::codec::diff_from(&v);
                let _ = lm_remote::codec::label_from(&v);
                let _ = lm_remote::codec::error_from(&v);
                let _ = lm_remote::codec::target_from(&v);
                let _ = lm_remote::codec::loading_from(&v);
                let _ = lm_remote::codec::operation_from(&v);
                let _ = lm_remote::codec::notice_from(&v);
                let _ = lm_remote::codec::applied_from(&v);
                let _ = lm_remote::codec::entries_from(&v);
                let _ = lm_remote::codec::device_from(&v);
                let _ = lm_remote::codec::preview_from(&v);
                let _ = lm_remote::codec::path_from(&v);
                if let Ok(ops) = lm_remote::delta::from_value(&v) {
                    let _ = lm_remote::delta::apply(&v, &ops);
                    let _ = lm_remote::delta::apply(&tree, &ops);
                }
            }
            let mut t = telemetry.clone();
            let _ = wire::read_frame(&mut bytes.as_slice(), 1 << 16);
            let _ = wire::telemetry_from(&bytes, &mut t);
            let _ = wire::peaks_from(&bytes);
            let _ = wire::display_from(&bytes);
            t
        });
        match result {
            Ok(t) => telemetry = t,
            Err(p) => {
                eprintln!("decoder fuzz: panic at round {round} (seed {seed}) on {bytes:?}");
                std::panic::resume_unwind(p);
            }
        }
    }
    eprintln!("decoder fuzz: {round} rounds in {:?}, no panic", start.elapsed());
}

#[test]
fn the_server_survives_garbage() {
    let mut r = Remote::unix();
    let mut rng = Rng(7);
    let samples = samples();
    // A raw client of another server on the same core: after a good hello, a stream of broken frames and messages
    let mut cfg = ServerConfig::new("garbage");
    cfg.tcp = Some("127.0.0.1:0".parse().unwrap());
    cfg.tokens = vec![("t".into(), Role::Admin)];
    let mut server = Server::start(cfg).unwrap();
    let addr = server.tcp_addr().unwrap();
    let mut raw = TcpStream::connect(addr).unwrap();
    let hello = Value::object([
        ("t", Value::text("hello")),
        ("protocol", Value::int(lm_remote::PROTOCOL)),
        ("program", Value::text(lm_remote::PROGRAM)),
        ("mode", Value::text("gui")),
        ("token", Value::text("t")),
    ]);
    wire::write_frame(&mut raw, wire::KIND_JSON, &wire::json(&hello).unwrap()).unwrap();
    for i in 0..3000 {
        let base = &samples[rng.below(samples.len())];
        let bytes = mutate(&mut rng, base);
        let kind = if rng.below(10) == 0 { rng.below(256) as u8 } else { wire::KIND_JSON };
        if wire::write_frame(&mut raw, kind, &bytes).is_err() {
            break;
        }
        if i % 50 == 0 {
            server.pump(&mut r.core);
            r.step();
        }
    }
    // A frame that claims to be huge: the connection is dropped, nothing else
    let _ = raw.write_all(&[0xff, 0xff, 0xff, 0x7f, 0]);
    for _ in 0..50 {
        server.pump(&mut r.core);
        r.step();
        std::thread::sleep(Duration::from_millis(1));
    }
    // The good client was never disturbed
    let p = r.client.ping();
    r.unit(p).expect("still served");
    assert!(matches!(r.client.connection(), Connection::Connected { .. }));
}

fn client_named(server: &mut Server, core: &mut FakePort, endpoint: Endpoint, name: &str) -> RemotePort {
    let mut cfg = ClientConfig::new(endpoint, schemas());
    cfg.name = name.into();
    let mut connecting = RemotePort::start(cfg);
    loop {
        core.pump();
        server.pump(core);
        if let Some(r) = connecting.poll() {
            return r.expect("connect");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn two_clients_editing_at_once_keep_one_coherent_history() {
    let path = socket_path();
    let mut cfg = ServerConfig::new("shared");
    cfg.socket = Some(path.clone());
    let mut core = fake();
    let mut server = Server::start(cfg).unwrap();
    let mut alice = client_named(&mut server, &mut core, Endpoint::Unix(path.clone()), "alice");
    let mut bob = client_named(&mut server, &mut core, Endpoint::Unix(path), "bob");
    let original = core.root().clone();
    let steps_before = core.document().history_len();
    let ids: Vec<NodeId> = original
        .get("tracks")
        .unwrap()
        .items()
        .map(|t| NodeId::parse(t.get("id").unwrap().as_text().unwrap()).unwrap())
        .collect();
    // Both send their edits at once, before the server sees any
    let mut waiting = Vec::new();
    for round in 0..5 {
        for (who, port, track) in [("alice", &mut alice, ids[0]), ("bob", &mut bob, ids[1])] {
            let name = format!("{who}-{round}");
            let p = port.apply(label("test.rename"), vec![EditOp::set(track, &["name"], Value::text(name.as_str()))]);
            let Pending::Waiting(id) = p else { panic!("remote") };
            waiting.push((who, id));
        }
    }
    // An open gesture of alice's, and bob's edit in the middle of it
    let g = alice.begin_gesture(label("test.drag"));
    alice.gesture_apply(&g, vec![EditOp::set(ids[0], &["volume"], Value::real(0.25).unwrap())]);
    let Pending::Waiting(bob_last) =
        bob.apply(label("test.solo"), vec![EditOp::set(ids[1], &["solo"], Value::bool(true))])
    else {
        panic!()
    };
    // Everything answered
    let mut done: Vec<(&str, RequestId)> = Vec::new();
    let end = Instant::now() + Duration::from_secs(10);
    let mut bob_done = false;
    while done.len() < waiting.len() || !bob_done {
        core.pump();
        server.pump(&mut core);
        for (who, port) in [("alice", &mut alice), ("bob", &mut bob)] {
            port.pump();
            for (id, f) in port.finished() {
                if let Finished::Applied(r) = f {
                    r.expect("each edit applies");
                }
                if who == "bob" && id == bob_last {
                    bob_done = true;
                } else {
                    done.push((who, id));
                }
            }
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    // Alice drops in the middle of her gesture: only her gesture goes back (I-20.5, G1)
    drop(alice);
    let end = Instant::now() + Duration::from_secs(5);
    while server.clients() > 1 {
        core.pump();
        server.pump(&mut core);
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    core.pump();
    server.pump(&mut core);
    let p = bob.ping();
    let Pending::Waiting(ping) = p else { panic!() };
    while !bob.finished().iter().any(|(id, _)| *id == ping) {
        core.pump();
        server.pump(&mut core);
        bob.pump();
        std::thread::sleep(Duration::from_millis(1));
    }
    let root = core.root().clone();
    let track = |i: usize| root.get("tracks").unwrap().at(i).unwrap().clone();
    assert_eq!(
        track(0).get("volume"),
        original.get("tracks").unwrap().at(0).unwrap().get("volume"),
        "the cancelled gesture went back"
    );
    assert_eq!(track(1).get("solo").and_then(Value::as_bool), Some(true), "bob's edit stays");
    assert_eq!(track(0).get("name").unwrap().as_text(), Some("alice-4"));
    assert_eq!(track(1).get("name").unwrap().as_text(), Some("bob-4"));
    // The mirror that stayed sees the same tree
    assert_eq!(bob.root(), &root);
    // One history: eleven steps, each with its client's name, each client's in its own order
    // One history: eleven steps, each with its client's name. When bob's edit landed inside alice's gesture, the
    // gesture's cancel is a step of its own (G1: only the gesture's paths go back, bob's edit stays)
    let mut labels = Vec::new();
    while core.document().history_len() > steps_before && core.document().can_undo() {
        labels.push(core.document().undo_label().cloned().expect("a label"));
        assert!(matches!(core.undo(), Pending::Ready(Ok(()))));
    }
    let named: Vec<&Label> = labels.iter().filter(|l| l.key != "doc.gesture.cancel").collect();
    assert!(labels.len() - named.len() <= 1, "{labels:?}");
    let client = |l: &Label| l.args.iter().find(|(k, _)| *k == "client").map(|(_, v)| format!("{v:?}")).expect("named");
    let alice_steps = named.iter().filter(|l| client(l).contains("alice")).count();
    let bob_steps = named.iter().filter(|l| client(l).contains("bob")).count();
    assert_eq!((alice_steps, bob_steps), (5, 6), "{labels:?}");
    // Undoing all of them gives the tree from before (no step lost or doubled)
    assert_eq!(core.root().without("meta"), original.without("meta"));
}
