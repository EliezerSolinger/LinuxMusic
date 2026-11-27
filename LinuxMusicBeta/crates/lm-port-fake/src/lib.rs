//! A `CorePort` over a real lm-doc `Document` with a fake engine: for the interface's tests and for running the
//! interface before `lm-session` exists. Only `lm-gui`'s dev-dependencies may name this crate.
//!
//! Real: the document (the project's schemas, transactions, the domain rules, undo and redo, gestures, feeds) — the
//! interface meets the same tree and the same refusals it will meet with the session.
//! Fake: the transport (a clock), the meters (from the fake recordings' peaks at the cursor, through the outputs, the
//! channels' volumes and the routing), the peaks (synthesized per resource), the long operations (threads with an
//! injectable delay), the notices, the preferences, the session's status, the devices and the browser's library.

#![forbid(unsafe_code)]

pub mod demo;
mod desk;
mod field_load;
mod library;
mod midi;
mod modulators;
pub use midi::Outside;
mod arrange_b;
pub mod peaks;
mod ranges;
mod services;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lm_base::{Error, HostTime, Label, OperationId, Result};
use lm_doc::{Document, Feed, GestureId, Topic};
use lm_port::{
    ActionId, Applied, BrowseEntry, BrowsePage, CorePort, DeviceInfo, DisplayView, EditOp, EntryId, EntryKind,
    LOADING_THRESHOLD, Loading, Meter, NoteRef, Notice, OperationInfo, PeakBlock, Pending, Position, Preview,
    RecordingTake, Ref, ScanKind, Severity, Target, TelemetryView,
};
use lm_state::{NodeId, NodeSchema, PathStep, Resource, Value};

use crate::demo::BAR;
use crate::peaks::Peaks;

pub const SAMPLE_RATE: u32 = 48_000;

/// What the fake's tests and the demo choose
#[derive(Clone, Debug)]
pub struct Config {
    pub tracks: usize,
    /// The injected delay of the resource that loads slowly (I-2.4); `None`: everything is there at once
    pub slow_delay: Option<Duration>,
    /// The resource that loads slowly: the first lane of this track's first item
    pub slow_track: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config { tracks: 64, slow_delay: Some(Duration::from_millis(1500)), slow_track: 2 }
    }
}

/// What an operation does when it ends well
type Then = Box<dyn FnOnce(&mut FakePort)>;

struct Op {
    id: OperationId,
    label: Label,
    target: Target,
    started: Instant,
    cancellable: bool,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// What to do when it ends well
    then: Option<Then>,
}

/// The transport's fake clock
#[derive(Clone, Copy, Debug)]
struct Transport {
    playing: bool,
    recording: bool,
    /// the position at `since`
    at: i64,
    since: Instant,
    /// recording: where the take begins, and the count-in before it
    take_start: i64,
}

/// A live note the interface sent to the fake (I-15 `note_on`, `note_off`, `all_notes_off`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveNote {
    On { track: NodeId, key: u8, velocity: u8 },
    Off { track: NodeId, key: u8 },
    AllOff { track: NodeId },
}

/// The log of the live notes a [`FakePort`] got, shared with whoever keeps a handle to it
#[derive(Clone, Debug, Default)]
pub struct LiveLog(Arc<std::sync::Mutex<Vec<LiveNote>>>);

impl LiveLog {
    fn push(&self, n: LiveNote) {
        if let Ok(mut v) = self.0.lock() {
            v.push(n);
        }
    }

    /// Every live note, in the order it was sent
    pub fn all(&self) -> Vec<LiveNote> {
        self.0.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// The keys sounding now (a note-on not yet ended by its note-off or an all-notes-off), as `(track, key)`
    pub fn held(&self) -> Vec<(NodeId, u8)> {
        let mut held: Vec<(NodeId, u8)> = Vec::new();
        for n in self.all() {
            match n {
                LiveNote::On { track, key, .. } => {
                    held.retain(|h| *h != (track, key));
                    held.push((track, key));
                }
                LiveNote::Off { track, key } => held.retain(|h| *h != (track, key)),
                LiveNote::AllOff { track } => held.retain(|h| h.0 != track),
            }
        }
        held
    }
}

/// The fake port
pub struct FakePort {
    doc: Document,
    /// the arrangement's clipboard (arrange_b.rs)
    clipboard: Vec<arrange_b::Copied>,
    resources: HashMap<Resource, Arc<Peaks>>,
    loading_resources: Vec<Resource>,
    ops: Vec<Op>,
    notices: Vec<Notice>,
    next_notice: u64,
    transport: Transport,
    telemetry: TelemetryView,
    preferences: Value,
    tempo: lm_port::TempoCache,
    status: Value,
    saved_serial: u64,
    serial: u64,
    pumps: u64,
    last_pump: Instant,
    library: library::Library,
    preview: Option<(EntryId, Instant)>,
    /// Writes that arrived by `param_write` (live values while dragging), by node and key
    live: HashMap<(NodeId, String), f32>,
    /// The fake's own feed: any change makes the project modified
    own_feed: Feed,
    midi: midi::Midi,
    /// Real plugins given by `with_plugins` (their schemas replace the fake ones of the same id): id, instrument
    real_plugins: Vec<(Arc<str>, bool)>,
    /// The last `param_write` (not cleared at the gesture's end, for the plugin panels' tests)
    last_write: Option<(NodeId, String, f32)>,
    /// Every live note the interface sent, in order, and the keys still held
    notes_log: LiveLog,
    /// The interface document (I-3.8)
    ui: Value,
    outside: midi::Outside,
    /// Blocks each desk channel clipped so far (the rack display's CLIP)
    clipped: HashMap<NodeId, u64>,
}

/// A real plugin's schema for the fake (lm-gui-plugins' tests: the panels move the real parameters). The fake
/// cannot depend on lm-plugin, so the caller passes the descriptor's main schema
#[derive(Clone, Debug)]
pub struct RealPlugin {
    pub schema: NodeSchema,
    pub instrument: bool,
}

impl std::fmt::Debug for FakePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakePort").field("pumps", &self.pumps).finish_non_exhaustive()
    }
}

/// A node's id, read from its `id` field
fn lm_id(v: &Value) -> Option<NodeId> {
    v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())
}

fn err(key: &'static str) -> Error {
    Error::new(key)
}

impl FakePort {
    pub fn new(config: Config) -> Result<FakePort> {
        Self::with_plugins(config, Vec::new())
    }

    /// The demo with real plugins: each schema replaces the fake plugin of the same id (or adds one), and every
    /// node of a replaced plugin in the demo starts at the real plugin's defaults (an empty state)
    pub fn with_plugins(config: Config, real: Vec<RealPlugin>) -> Result<FakePort> {
        let registry = Arc::new(demo::registry()?);
        let mut doc = Document::new_project(registry, demo::options())?;
        demo::project(&mut doc, config.tracks.max(3))?;
        // the demo is the opening state: nothing to undo
        let doc = if real.is_empty() {
            Document::open(Arc::clone(doc.registry()), doc.saved(), demo::options())?.document
        } else {
            let ids: Vec<String> =
                real.iter().map(|p| p.schema.type_name.trim_start_matches("plugin:").to_string()).collect();
            let registry = Arc::new(demo::registry_with(real.iter().map(|p| p.schema.clone()).collect())?);
            let saved = demo::reset_plugins(&doc.saved(), &ids, &registry)?;
            Document::open(registry, saved, demo::options_with(ids))?.document
        };
        let real_plugins =
            real.iter().map(|p| (Arc::from(p.schema.type_name.trim_start_matches("plugin:")), p.instrument)).collect();
        let now = Instant::now();
        let mut doc = doc;
        let own_feed = doc.feed(Topic::All);
        let mut port = FakePort {
            tempo: Default::default(),
            clipboard: Vec::new(),
            own_feed,
            midi: Default::default(),
            ui: Value::empty_object(),
            outside: Default::default(),
            clipped: HashMap::new(),
            doc,
            resources: HashMap::new(),
            loading_resources: Vec::new(),
            ops: Vec::new(),
            notices: Vec::new(),
            next_notice: 1,
            transport: Transport { playing: false, recording: false, at: 0, since: now, take_start: 0 },
            telemetry: TelemetryView {
                position: Position {
                    samples: 0,
                    host_time: HostTime::now(),
                    sample_rate: SAMPLE_RATE,
                    playing: false,
                    recording: false,
                },
                meters: HashMap::new(),
                inputs: vec![0.0; 8],
                recording: Vec::new(),
                counting_in: false,
                xruns: 0,
                xrun_marks: Vec::new(),
                load: 0.0,
                hand_won: Vec::new(),
            },
            preferences: library::preferences(),
            status: library::status(),
            saved_serial: 0,
            serial: 0,
            pumps: 0,
            last_pump: now,
            library: library::Library::new(),
            preview: None,
            live: HashMap::new(),
            real_plugins,
            last_write: None,
            notes_log: LiveLog::default(),
        };
        port.push_notice("Autosaved {0}", vec![("time", Value::text("12:04"))], Severity::Info, None, Vec::new());
        if let Some(delay) = config.slow_delay
            && let Some(res) = port.first_lane_of_track(config.slow_track)
        {
            port.regenerate(res, delay);
        }
        Ok(port)
    }

    /// The document (for the fake's own tests; the interface never sees it)
    pub fn document(&self) -> &Document {
        &self.doc
    }

    fn first_lane_of_track(&self, t: usize) -> Option<Resource> {
        let track = self.doc.root().get("tracks")?.at(t)?;
        let item = track.get("items")?.at(0)?;
        item.get("lanes")?.at(0)?.get("source")?.as_resource().cloned()
    }

    /// Makes a resource's peaks again, with a delay: the loading component's test case (I-2.4)
    pub fn regenerate(&mut self, res: Resource, delay: Duration) {
        if self.loading_resources.contains(&res) {
            return;
        }
        self.loading_resources.push(res.clone());
        let r2 = res.clone();
        self.start(Label::new("Generating peaks"), Target::Resource(res.clone()), true, delay, move |port| {
            port.loading_resources.retain(|x| *x != r2);
            port.resources.remove(&r2);
        });
    }

    fn start(
        &mut self,
        label: Label,
        target: Target,
        cancellable: bool,
        delay: Duration,
        then: impl FnOnce(&mut FakePort) + 'static,
    ) -> OperationId {
        let id = OperationId(1_000_000 + self.ops.len() as u64 + self.pumps * 7);
        let progress = Arc::new(AtomicU32::new(0f32.to_bits()));
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (p, c, d) = (Arc::clone(&progress), Arc::clone(&cancel), Arc::clone(&done));
        let thread = std::thread::Builder::new()
            .name("fake-op".into())
            .spawn(move || {
                let steps = 40u32;
                for i in 0..steps {
                    if c.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(delay / steps);
                    p.store(((i + 1) as f32 / steps as f32).to_bits(), Ordering::Relaxed);
                }
                d.store(true, Ordering::Release);
            })
            .ok();
        self.ops.push(Op {
            id,
            label,
            target,
            started: Instant::now(),
            cancellable,
            progress,
            cancel,
            done,
            thread,
            then: Some(Box::new(then)),
        });
        id
    }

    fn push_notice(
        &mut self,
        key: &'static str,
        args: Vec<(&'static str, Value)>,
        severity: Severity,
        target: Option<Target>,
        actions: Vec<(ActionId, Value)>,
    ) {
        if let Some(n) = self.notices.iter_mut().find(|n| n.key == key && n.args == args) {
            n.repeats += 1;
            n.read = false;
            return;
        }
        let id = self.next_notice;
        self.next_notice += 1;
        self.notices.push(Notice { id, key, args, severity, target, actions, repeats: 0, read: false });
    }

    fn peaks_of(&mut self, res: &Resource) -> Option<Arc<Peaks>> {
        if self.loading_resources.contains(res) {
            return None;
        }
        Some(Arc::clone(self.resources.entry(res.clone()).or_insert_with(|| {
            let seed = u64::from_str_radix(&res.key()[..12], 16).unwrap_or(1);
            Arc::new(Peaks::synthesize(seed, 60.0, SAMPLE_RATE))
        })))
    }

    fn ticks_to_samples(&self, ticks: i64) -> i64 {
        // the fake engine plays at the first tempo point (the real one follows the map, lm-time)
        let bpm = self.bpm();
        (ticks as f64 * 60.0 * SAMPLE_RATE as f64 / (bpm * demo::PPQ as f64)) as i64
    }

    fn samples_to_ticks(&self, samples: i64) -> i64 {
        (samples as f64 * self.bpm() * demo::PPQ as f64 / (60.0 * SAMPLE_RATE as f64)) as i64
    }

    fn bpm(&self) -> f64 {
        self.doc
            .root()
            .get("tempoMap")
            .and_then(|m| m.at(0))
            .and_then(|p| p.get("bpm"))
            .and_then(Value::as_text)
            .and_then(|t| t.parse().ok())
            .unwrap_or(120.0)
    }

    fn position(&self) -> i64 {
        let t = self.transport;
        if t.playing { t.at + (t.since.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as i64 } else { t.at }
    }

    fn locate(&mut self, samples: i64) {
        self.transport.at = samples.max(0);
        self.transport.since = Instant::now();
    }

    fn play(&mut self) {
        if !self.transport.playing {
            self.transport.at = self.position();
            self.transport.since = Instant::now();
            self.transport.playing = true;
        }
    }

    fn stop(&mut self) {
        if self.transport.recording {
            self.finish_recording();
        }
        if self.transport.playing {
            self.transport.at = self.position();
            self.transport.playing = false;
        } else {
            self.locate(0); // stop twice: back to the start
        }
    }

    fn record(&mut self, fallback: Option<NodeId>) {
        if self.transport.recording {
            self.finish_recording();
            return;
        }
        let armed = self.tracks_where(|t| t.get("armed").and_then(Value::as_bool) == Some(true));
        if armed.is_empty()
            && let Some(t) = fallback
        {
            let _ = self.doc.transact(Label::new("Arm"), |e| e.set(t, &[PathStep::key("armed")], Value::bool(true)));
        }
        let at = self.position();
        self.transport.take_start = at;
        self.transport.recording = true;
        if !self.transport.playing {
            // count-in and pre-roll: the cursor goes back the pre-roll's bars (U4)
            let bars = self
                .doc
                .root()
                .get("transport")
                .and_then(|t| t.get("preRoll"))
                .and_then(|p| p.get("bars"))
                .and_then(Value::as_int)
                .unwrap_or(1);
            let back = self.ticks_to_samples(bars * BAR);
            self.locate(at - back);
            self.play();
        }
    }

    fn tracks_where(&self, f: impl Fn(&Value) -> bool) -> Vec<NodeId> {
        self.doc
            .root()
            .get("tracks")
            .map(|l| {
                l.items()
                    .filter(|t| f(t))
                    .filter_map(|t| t.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The takes become items: one audio item per armed track, its lane on a new recording (U1)
    fn finish_recording(&mut self) {
        self.transport.recording = false;
        let start = self.transport.take_start;
        let len = self.position() - start;
        if len <= 0 {
            return;
        }
        let (start_t, len_t) = (self.samples_to_ticks(start), self.samples_to_ticks(len).max(1));
        let armed = self.tracks_where(|t| t.get("armed").and_then(Value::as_bool) == Some(true));
        let serial = self.serial;
        let _ = self.doc.transact(Label::new("Record"), |e| {
            for (n, t) in armed.iter().enumerate() {
                let count = e.find(*t).and_then(|v| v.get("items")).map_or(0, Value::len);
                let it = e.insert(
                    *t,
                    "items",
                    count,
                    Value::object([
                        ("kind", Value::text("audio")),
                        ("name", Value::text("Take")),
                        ("start", Value::int(start_t)),
                        ("length", Value::int(len_t)),
                        ("timebase", Value::text("musical")),
                    ]),
                )?;
                let lane = e.insert(
                    it,
                    "lanes",
                    0,
                    Value::object([
                        ("source", Value::resource(demo::audio(&format!("rec-{serial}-{n}")))),
                        ("name", Value::text("Take 1")),
                    ]),
                )?;
                let seg = Value::object([
                    ("from", Value::int(0)),
                    ("to", Value::int(len_t)),
                    ("lane", Value::text(lane.to_string())),
                ]);
                e.set(it, &[PathStep::key("comp")], Value::list([seg]))?;
            }
            Ok(())
        });
    }

    /// The meters: each track's outputs from its recordings (or its notes) at the cursor, then the channels through
    /// the routing, with the volumes (M9)
    fn meters(&mut self, pos: i64) -> HashMap<NodeId, Meter> {
        let root = self.doc.root().clone();
        let tick = self.samples_to_ticks(pos);
        let mut out: HashMap<NodeId, f32> = HashMap::new();
        let playing = self.transport.playing;
        let tracks: Vec<Value> = root.get("tracks").map(|l| l.items().cloned().collect()).unwrap_or_default();
        for t in &tracks {
            if t.get("mute").and_then(Value::as_bool) == Some(true) || !playing {
                continue;
            }
            let vol = t.get("volume").and_then(Value::as_real).unwrap_or(1.0) as f32;
            let mut level = 0f32;
            for it in t.get("items").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                let start = it.get("start").and_then(Value::as_int).unwrap_or(0);
                let length = it.get("length").and_then(Value::as_int).unwrap_or(0);
                if tick < start || tick >= start + length {
                    continue;
                }
                if it.get("kind").and_then(Value::as_text) == Some("audio") {
                    if let Some(res) = it
                        .get("lanes")
                        .and_then(|l| l.at(0))
                        .and_then(|l| l.get("source"))
                        .and_then(Value::as_resource)
                        .cloned()
                        && let Some(p) = self.peaks_of(&res)
                    {
                        level =
                            level.max(p.amplitude_at(self.ticks_to_samples(tick - start), (SAMPLE_RATE / 60) as u64));
                    }
                } else {
                    level = level.max(0.55);
                }
            }
            for (k, o) in
                t.get("outputs").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default().iter().enumerate()
            {
                let Some(id) = o.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else {
                    continue;
                };
                let v = level * vol * if k == 0 { 1.0 } else { 0.8 };
                out.insert(id, v);
                if let Some(to) = o.get("to").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) {
                    let e = out.entry(to).or_insert(0.0);
                    *e = e.max(v);
                }
            }
        }
        // the channels: their volume, then where they output and send (sources before, by depth)
        let channels: Vec<Value> =
            root.get("mixer").and_then(|m| m.get("channels")).map(|l| l.items().cloned().collect()).unwrap_or_default();
        let id_of = |c: &Value| c.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok());
        let mut order: Vec<(usize, Value)> = channels
            .iter()
            .map(|c| {
                let mut depth = 0;
                let mut at = c.get("output").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok());
                while let Some(n) = at {
                    depth += 1;
                    if depth > channels.len() {
                        break;
                    }
                    at = channels
                        .iter()
                        .find(|x| id_of(x) == Some(n))
                        .and_then(|x| x.get("output"))
                        .and_then(Value::as_text)
                        .and_then(|x| NodeId::parse(x).ok());
                }
                (depth, c.clone())
            })
            .collect();
        order.sort_by_key(|(d, _)| std::cmp::Reverse(*d));
        for (_, c) in &order {
            let Some(id) = id_of(c) else { continue };
            let muted = c.get("mute").and_then(Value::as_bool) == Some(true);
            let vol = self
                .live
                .get(&(id, "volume".into()))
                .copied()
                .unwrap_or(c.get("volume").and_then(Value::as_real).unwrap_or(1.0) as f32);
            let v = if muted { 0.0 } else { out.get(&id).copied().unwrap_or(0.0) * vol };
            out.insert(id, v);
            if let Some(to) = c.get("output").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) {
                let e = out.entry(to).or_insert(0.0);
                *e = e.max(v * 0.9);
            }
            for s in c.get("sends").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                if let Some(to) = s.get("to").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) {
                    let amount = s.get("amount").and_then(Value::as_real).unwrap_or(0.0) as f32;
                    let e = out.entry(to).or_insert(0.0);
                    *e = e.max(v * amount);
                }
            }
        }
        let master = lm_doc::schemas::MASTER;
        let mv = root
            .get("mixer")
            .and_then(|m| m.get("master"))
            .and_then(|m| m.get("volume"))
            .and_then(Value::as_real)
            .unwrap_or(1.0) as f32;
        if let Some(v) = out.get_mut(&master) {
            *v = (*v * mv).min(1.2);
        }
        out.into_iter()
            .map(|(id, v)| {
                let mut m = Meter { channels: 2, ..Meter::default() };
                m.peak[0] = v;
                m.peak[1] = v * 0.93;
                (id, m)
            })
            .collect()
    }

    fn apply_ops(e: &mut lm_doc::Edit<'_>, ops: &[EditOp], applied: &mut Applied) -> Result<()> {
        let id = |r: Ref, created: &[(u32, NodeId)]| -> Result<NodeId> {
            match r {
                Ref::Id(id) => Ok(id),
                Ref::Temp(t) => {
                    created.iter().find(|(k, _)| *k == t).map(|(_, id)| *id).ok_or_else(|| err("port.ref.unknown_temp"))
                }
            }
        };
        let note = |n: NoteRef, notes: &[(u32, u64)]| -> Result<u64> {
            match n {
                NoteRef::N(n) => Ok(n),
                NoteRef::Temp(t) => {
                    notes.iter().find(|(k, _)| *k == t).map(|(_, n)| *n).ok_or_else(|| err("port.ref.unknown_temp"))
                }
            }
        };
        for op in ops {
            match op {
                EditOp::Set { id: r, path, value } => {
                    let n = id(*r, &applied.created)?;
                    if path.len() >= 2
                        && path[0] == PathStep::key("state")
                        && path.get(1) == Some(&PathStep::key("params"))
                    {
                        // a plugin parameter: through set_param, so the slot hears it (S20)
                        if let (Some(PathStep::Key(key)), Some(v)) = (path.get(2), value.as_real()) {
                            e.set_param(n, key, v)?;
                            continue;
                        }
                    }
                    e.set(n, path, value.clone())?;
                }
                EditOp::Insert { temp, parent, list, index, node } => {
                    let p = id(*parent, &applied.created)?;
                    let new = e.insert(p, list, *index, node.clone())?;
                    applied.created.push((*temp, new));
                }
                EditOp::Remove { id: r } => e.remove(id(*r, &applied.created)?)?,
                EditOp::Move { id: r, parent, list, index } => {
                    let (n, p) = (id(*r, &applied.created)?, id(*parent, &applied.created)?);
                    e.move_to(n, p, list, *index)?;
                }
                EditOp::AddNote { temp, item, note: v } => {
                    let n = e.add_note(id(*item, &applied.created)?, v.clone())?;
                    applied.notes.push((*temp, n));
                }
                EditOp::SetNote { item, n, path, value } => {
                    let it = id(*item, &applied.created)?;
                    e.set_note(it, note(*n, &applied.notes)?, path, value.clone())?;
                }
                EditOp::RemoveNote { item, n } => {
                    let it = id(*item, &applied.created)?;
                    e.remove_note(it, note(*n, &applied.notes)?)?;
                }
            }
        }
        Ok(())
    }
}

impl FakePort {
    fn item_param(params: &Value) -> Result<NodeId> {
        params
            .get("item")
            .and_then(Value::as_text)
            .and_then(|t| NodeId::parse(t).ok())
            .ok_or_else(|| err("port.params"))
    }

    /// `item` or `items` (the session's rule; for the split, `items: []` means every item)
    fn items_param(params: &Value) -> Result<Vec<NodeId>> {
        match params.get("items") {
            Some(l) => l
                .items()
                .map(|v| v.as_text().and_then(|t| NodeId::parse(t).ok()).ok_or_else(|| err("port.params")))
                .collect(),
            None => Ok(vec![Self::item_param(params)?]),
        }
    }

    /// Splits items at `tick` (S, F-ARR-005), one undo step: the right part is a pasted copy; each side keeps its
    /// notes and its comp segments, shifted to its own start. The items the tick is not inside stay
    fn split(&mut self, params: &Value) -> Result<()> {
        let mut items = Self::items_param(params)?;
        if items.is_empty() {
            items = self
                .doc
                .root()
                .get("tracks")
                .map(|l| {
                    l.items()
                        .flat_map(|t| t.get("items").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default())
                        .filter_map(|i| NodeId::parse(i.get("id")?.as_text()?).ok())
                        .collect()
                })
                .unwrap_or_default();
        }
        let at = params.get("tick").and_then(Value::as_int).ok_or_else(|| err("port.params"))?;
        let mut plans = Vec::new();
        for item in items {
            let v = self.doc.find(item).ok_or_else(|| err("port.no_such_node"))?;
            let (start, length) = (
                v.get("start").and_then(Value::as_int).unwrap_or(0),
                v.get("length").and_then(Value::as_int).unwrap_or(1),
            );
            let cut = at - start;
            if cut <= 0 || cut >= length {
                continue;
            }
            let loc = self.doc.loc(item).cloned().ok_or_else(|| err("port.no_such_node"))?;
            plans.push((item, cut, length, loc, self.doc.copy(item)?));
        }
        if plans.is_empty() {
            return Err(err("arrange.split.outside"));
        }
        self.doc
            .transact(Label::new("Split"), |e| {
                for (item, cut, length, loc, clip) in &plans {
                    let (item, cut, length) = (*item, *cut, *length);
                    let index = e.find(loc.parent).and_then(|t| t.get("items")).map_or(0, |l| {
                        l.items()
                            .position(|x| x.get("id").and_then(Value::as_text) == Some(&item.to_string()))
                            .unwrap_or(0)
                            + 1
                    });
                    let right =
                        e.paste(loc.parent, "items", index, clip)?.ok_or_else(|| err("arrange.split.refused"))?;
                    let k = PathStep::key;
                    e.set(item, &[k("length")], Value::int(cut))?;
                    e.set(item, &[k("fadeOut")], Value::int(0))?;
                    e.set(right, &[k("start")], Value::int(at))?;
                    e.set(right, &[k("length")], Value::int(length - cut))?;
                    e.set(right, &[k("fadeIn")], Value::int(0))?;
                    // notes: each side keeps its own, the right's moved to its start
                    for (side, keep_left) in [(item, true), (right, false)] {
                        let notes: Vec<(u64, i64)> = e
                            .find(side)
                            .and_then(|v| v.get("notes"))
                            .map(|m| {
                                m.entries()
                                    .filter_map(|(n, v)| {
                                        Some((n.parse().ok()?, v.get("start").and_then(Value::as_int)?))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        for (n, s0) in notes {
                            if (keep_left && s0 >= cut) || (!keep_left && s0 < cut) {
                                e.remove_note(side, n)?;
                            } else if !keep_left {
                                e.set_note(side, n, &[k("start")], Value::int(s0 - cut))?;
                            }
                        }
                    }
                    // the comp: clipped and shifted per side
                    for (side, from, to, shift) in [(item, 0, cut, 0), (right, cut, length, cut)] {
                        let comp: Vec<Value> = e
                            .find(side)
                            .and_then(|v| v.get("comp"))
                            .map(|l| l.items().cloned().collect())
                            .unwrap_or_default();
                        let kept = comp.into_iter().filter_map(|seg| {
                            let (a, b) = (seg.get("from")?.as_int()?.max(from), seg.get("to")?.as_int()?.min(to));
                            (b > a).then(|| seg.with("from", Value::int(a - shift)).with("to", Value::int(b - shift)))
                        });
                        e.set(side, &[k("comp")], Value::list(kept.collect::<Vec<_>>()))?;
                    }
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// "Create channels for the outputs" (I-3.9, F-MIX-012): a group channel, one channel per output of the track
    /// sending to it, each output routed to its channel; one transaction
    fn create_channels_for_outputs(&mut self, params: &Value) -> Result<()> {
        let track = params
            .get("track")
            .and_then(Value::as_text)
            .and_then(|t| NodeId::parse(t).ok())
            .ok_or_else(|| err("port.params"))?;
        let t = self.doc.find(track).ok_or_else(|| err("port.no_such_node"))?;
        let name = t.get("name").and_then(Value::as_text).unwrap_or("").to_string();
        let outputs: Vec<(NodeId, String, i64)> = t
            .get("outputs")
            .map(|l| {
                l.items()
                    .filter_map(|o| {
                        let id = NodeId::parse(o.get("id")?.as_text()?).ok()?;
                        Some((id, o.get("port")?.as_text()?.to_string(), o.get("channels")?.as_int()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.doc
            .transact(Label::new("Create channels for the outputs"), |e| {
                let k = PathStep::key;
                let n = e.root().get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len);
                let group = e.insert(
                    lm_state::ROOT,
                    "mixer.channels",
                    n,
                    Value::object([("name", Value::text(format!("{name} (peças)")))]),
                )?;
                for (i, (out, port, channels)) in outputs.iter().enumerate() {
                    let ch = e.insert(
                        lm_state::ROOT,
                        "mixer.channels",
                        n + 1 + i,
                        Value::object([
                            ("name", Value::text(format!("{name} {port}"))),
                            ("channels", Value::int(*channels)),
                        ]),
                    )?;
                    e.set(ch, &[k("output")], Value::text(group.to_string()))?;
                    e.set(*out, &[k("to")], Value::text(ch.to_string()))?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// Ctrl+D: copies of the items as a group, moved by whole bars to start at or after the group's end (F-ARR-006);
    /// one undo step
    fn duplicate(&mut self, params: &Value) -> Result<()> {
        let mut group = Vec::new();
        let (mut first, mut end) = (i64::MAX, i64::MIN);
        for item in Self::items_param(params)? {
            let v = self.doc.find(item).ok_or_else(|| err("port.no_such_node"))?;
            let start = v.get("start").and_then(Value::as_int).unwrap_or(0);
            first = first.min(start);
            end = end.max(start + v.get("length").and_then(Value::as_int).unwrap_or(1));
            let loc = self.doc.loc(item).cloned().ok_or_else(|| err("port.no_such_node"))?;
            group.push((start, loc, self.doc.copy(item)?));
        }
        if group.is_empty() {
            return Err(err("arrange.copy.empty"));
        }
        // whole bars from the group's start, at least one (the legacy's Ctrl+D)
        let offset = ((end - first + BAR - 1) / BAR).max(1) * BAR;
        self.doc
            .transact(Label::new("Duplicate"), |e| {
                for (start, loc, clip) in &group {
                    let n = e.find(loc.parent).and_then(|t| t.get("items")).map_or(0, Value::len);
                    let copy =
                        e.paste(loc.parent, "items", n, clip)?.ok_or_else(|| err("arrange.duplicate.refused"))?;
                    e.set(copy, &[PathStep::key("start")], Value::int(start + offset))?;
                }
                Ok(())
            })
            .map(|_| ())
    }
}

impl Drop for FakePort {
    fn drop(&mut self) {
        for op in &mut self.ops {
            op.cancel.store(true, Ordering::Relaxed);
            if let Some(t) = op.thread.take() {
                let _ = t.join();
            }
        }
    }
}

impl CorePort for FakePort {
    fn pump(&mut self) -> bool {
        self.pumps += 1;
        let outside = self.pump_outside();
        let now = Instant::now();
        // a pump far too late: an xrun in a real engine's terms (P-14)
        if self.transport.playing && now.duration_since(self.last_pump) > Duration::from_millis(250) {
            self.telemetry.xruns += 1;
            let p = self.position();
            self.telemetry.xrun_marks.push(p);
        }
        self.last_pump = now;
        let mut changed = outside;
        // operations that ended: their effect, applied here (an operation never touches the session, I-2.2)
        let mut i = 0;
        while i < self.ops.len() {
            if self.ops[i].done.load(Ordering::Acquire) {
                let mut op = self.ops.remove(i);
                if let Some(t) = op.thread.take() {
                    let _ = t.join();
                }
                let cancelled = op.cancel.load(Ordering::Relaxed);
                if let Some(then) = op.then.take() {
                    then(self);
                }
                if cancelled {
                    self.push_notice(
                        "Cancelled: {0}",
                        vec![("what", Value::text(op.label.key))],
                        Severity::Info,
                        Some(op.target.clone()),
                        Vec::new(),
                    );
                }
                changed = true;
            } else {
                if self.ops[i].started.elapsed() >= LOADING_THRESHOLD {
                    changed = true;
                }
                i += 1;
            }
        }
        // the feeds
        let _ = self.doc.deliver();
        if self.own_feed.drain(&self.doc).is_some() {
            self.serial += 1;
            changed = true;
        }
        // the telemetry
        let pos = self.position();
        let before = self.telemetry.position.samples;
        let host = HostTime::now();
        let meters = self.meters(pos);
        let rec = self.transport.recording;
        let armed = self.tracks_where(|t| t.get("armed").and_then(Value::as_bool) == Some(true));
        let take_start = self.transport.take_start;
        self.telemetry.position = Position {
            samples: pos,
            host_time: host,
            sample_rate: SAMPLE_RATE,
            playing: self.transport.playing,
            recording: rec,
        };
        self.telemetry.counting_in = rec && pos < take_start;
        self.telemetry.recording = if rec && pos > take_start {
            armed
                .iter()
                .enumerate()
                .map(|(n, t)| RecordingTake {
                    track: *t,
                    resource: demo::audio(&format!("rec-{}-{n}", self.serial)),
                    start: take_start,
                    length: pos - take_start,
                })
                .collect()
        } else {
            Vec::new()
        };
        let t = self.pumps as f32 * 0.01;
        self.telemetry.inputs = (0..8)
            .map(|k| (0.35 + 0.3 * (t * (1.3 + k as f32 * 0.4)).sin().abs()) * if k % 3 == 2 { 0.2 } else { 1.0 })
            .collect();
        let had = self.telemetry.meters.values().any(|m| m.peak[0] > 0.0);
        let has = meters.values().any(|m| m.peak[0] > 0.0);
        for (id, m) in &meters {
            if m.peak.iter().any(|p| *p >= 1.0) {
                *self.clipped.entry(*id).or_insert(0) += 1;
            }
        }
        self.telemetry.meters = meters;
        if self.transport.playing || pos != before || had || has {
            changed = true;
        }
        if let Some((_, since)) = self.preview
            && since.elapsed() > Duration::from_secs(4)
        {
            self.preview = None;
        }
        changed
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

    fn drain(&self, feed: &mut Feed) -> Option<lm_doc::Diff> {
        feed.drain(&self.doc)
    }

    fn modified(&self) -> bool {
        self.serial != self.saved_serial
    }

    fn can_undo(&self) -> bool {
        self.doc.can_undo()
    }

    fn can_redo(&self) -> bool {
        self.doc.can_redo()
    }

    fn apply(&mut self, label: Label, ops: Vec<EditOp>) -> Pending<Applied> {
        let mut applied = Applied::default();
        let r = self.doc.transact(label, |e| Self::apply_ops(e, &ops, &mut applied));
        Pending::Ready(r.map(|diff| Applied { diff: Some(diff), ..applied }))
    }

    fn run_action(&mut self, action: ActionId, params: Value) -> Pending<()> {
        let id_param = || params.as_text().and_then(|t| NodeId::parse(t).ok());
        let r: Result<()> = match action.as_str() {
            "transport.play" => {
                self.play();
                Ok(())
            }
            "transport.stop" => {
                self.stop();
                Ok(())
            }
            "transport.toggle" => {
                let _: () = if self.transport.playing { self.stop() } else { self.play() };
                Ok(())
            }
            "transport.record" => {
                self.record(id_param());
                Ok(())
            }
            "transport.locate" => match params.as_int() {
                Some(ticks) => {
                    self.locate(self.ticks_to_samples(ticks));
                    Ok(())
                }
                None => Err(err("port.params")),
            },
            "project.save" => {
                self.saved_serial = self.serial;
                Ok(())
            }
            "take.reload" => match id_param().and(None::<Resource>).or_else(|| self.first_lane_of_track(2)) {
                Some(res) => {
                    self.regenerate(res, Duration::from_millis(1500));
                    Ok(())
                }
                None => Err(err("port.params")),
            },
            "modulator.attach" => self.modulator_attach(&params),
            "modulator.remove" => self.modulator_remove(&params),
            // the arrangement's domain edits: copy and paste keep the lanes, the comp and the notes coherent
            "arrange.split" => self.split(&params),
            "arrange.duplicate" => self.duplicate(&params),
            "arrange.copy" | "arrange.cut" | "arrange.paste" | "arrange.insert_bar" | "arrange.remove_bar" => {
                self.arrange_b(action.as_str(), &params)
            }
            "mixer.create_channels_for_outputs" => self.create_channels_for_outputs(&params),
            "automation.rearm" => self.rearm(&params),
            "mixer.group" => self.mixer_group(&params),
            // the fake's loudness follows its meter: nothing to start again
            "loudness.reset" => Ok(()),
            "items.move" => self.move_items(&params),
            "automation.thin" => self.thin(&params),
            "items.rerender" => {
                self.start(
                    Label::new("Re-rendering the recorded items"),
                    Target::Project,
                    true,
                    Duration::from_secs(2),
                    |_| {},
                );
                Ok(())
            }
            // a preset is a plugin's state in a file (piece 7a): loading it is one transaction, saving it a file
            "plugin.load_preset" => self.load_preset(&params),
            "plugin.save_preset" => self.save_preset(&params),
            // the track's tuning files (F-IJN-020): a Scala scale or keymap cited by the track, one edit
            "tuning.set_scale" | "tuning.set_keymap" | "tuning.clear" => self.set_tuning(action.as_str(), &params),
            // a file into a plugin field (a patch, a wave): one undo step
            "plugin.load_into_field" => field_load::load_into_field(self, &params),
            // the list of the presets of a bank file (the real core reads the file; the fake has none to read)
            "sampler.list_presets" => Ok(()),
            // the entry's file shown in the system's file manager (F-BRW-008): the fake says where it is
            "browser.reveal" => self.reveal(&params),
            // a project of the browser opened (F-BRW-006): an operation on the project
            "project.open_entry" => self.open_entry(&params),
            "plugins.rescan" => {
                self.start(
                    Label::new("Scanning plugins"),
                    Target::Scan(ScanKind::Plugins),
                    true,
                    Duration::from_secs(2),
                    |_| {},
                );
                Ok(())
            }
            other if let Some(r) = services::run(self, other, &params) => r,
            other => {
                self.push_notice(
                    "Not in the fake core: {0}",
                    vec![("action", Value::text(other))],
                    Severity::Warning,
                    None,
                    Vec::new(),
                );
                Err(err("port.action.unknown"))
            }
        };
        Pending::Ready(r)
    }

    fn undo(&mut self) -> Pending<()> {
        Pending::Ready(self.doc.undo().map(|_| ()))
    }

    fn redo(&mut self) -> Pending<()> {
        Pending::Ready(self.doc.redo().map(|_| ()))
    }

    fn begin_gesture(&mut self, label: Label) -> GestureId {
        self.doc.begin_gesture(label)
    }

    fn gesture_apply(&mut self, g: &GestureId, ops: Vec<EditOp>) {
        let mut applied = Applied::default();
        if let Err(e) = self.doc.gesture_update(g, |e| Self::apply_ops(e, &ops, &mut applied))
            && e.label.key != "doc.gesture.closed"
        {
            self.push_notice("{0}", vec![("error", Value::text(e.label.key))], Severity::Warning, None, Vec::new());
        }
    }

    fn gesture_commit(&mut self, g: GestureId) {
        let _ = self.doc.gesture_commit(g);
        self.live.clear();
    }

    fn gesture_cancel(&mut self, g: GestureId) {
        let _ = self.doc.gesture_cancel(g);
        self.live.clear();
    }

    fn param_write(&mut self, node: NodeId, param: &str, value: f32) {
        self.live.insert((node, param.to_string()), value);
        self.last_write = Some((node, param.to_string(), value));
    }

    fn note_on(&mut self, track: NodeId, key: u8, velocity: u8) {
        self.notes_log.push(LiveNote::On { track, key, velocity });
    }

    fn note_off(&mut self, track: NodeId, key: u8) {
        self.notes_log.push(LiveNote::Off { track, key });
    }

    fn all_notes_off(&mut self, track: NodeId) {
        self.notes_log.push(LiveNote::AllOff { track });
    }

    fn loading(&self, target: &Target) -> Loading {
        // aggregation (T10): an item is loading while one of its lanes' resources is
        let item_resources = |id: NodeId| -> Vec<Resource> {
            self.doc
                .find(id)
                .and_then(|v| {
                    v.get("lanes").map(|l| {
                        l.items().filter_map(|x| x.get("source").and_then(Value::as_resource).cloned()).collect()
                    })
                })
                .unwrap_or_default()
        };
        let hit = |op: &Op| match (target, &op.target) {
            (a, b) if a == b => true,
            (Target::Node(n), Target::Resource(r)) => item_resources(*n).contains(r),
            _ => false,
        };
        match self.ops.iter().filter(|o| hit(o)).min_by_key(|o| o.started) {
            None => Loading::Ready,
            Some(op) => {
                let since = op.started.elapsed();
                if since < LOADING_THRESHOLD {
                    Loading::Pending
                } else {
                    Loading::Visible {
                        progress: Some(f32::from_bits(op.progress.load(Ordering::Relaxed))),
                        stage: Some(op.label.clone()),
                        cancellable: op.cancellable && since >= lm_port::CANCEL_AFTER,
                        since,
                    }
                }
            }
        }
    }

    fn activity(&self) -> Vec<OperationInfo> {
        self.ops
            .iter()
            .filter(|o| o.started.elapsed() >= LOADING_THRESHOLD)
            .map(|o| OperationInfo {
                id: o.id,
                label: o.label.clone(),
                target: o.target.clone(),
                progress: Some(f32::from_bits(o.progress.load(Ordering::Relaxed))),
                stage: None,
                cancellable: o.cancellable && o.started.elapsed() >= lm_port::CANCEL_AFTER,
                since: o.started.elapsed(),
            })
            .collect()
    }

    fn cancel(&mut self, op: OperationId) {
        for o in &self.ops {
            if o.id == op {
                o.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    fn notices(&mut self) -> Vec<Notice> {
        self.notices.clone()
    }

    fn dismiss_notice(&mut self, id: u64) {
        self.notices.retain(|n| n.id != id);
    }

    fn telemetry(&self) -> &TelemetryView {
        &self.telemetry
    }

    fn peaks(&mut self, resource: &Resource, level: u8, from: u64, to: u64) -> Pending<PeakBlock> {
        let Some(p) = self.peaks_of(resource) else { return Pending::Waiting(from) };
        let l = (level as usize).min(p.levels.len() - 1);
        let pairs = &p.levels[l];
        let a = (from as usize).min(pairs.len());
        let z = (to as usize).clamp(a, pairs.len());
        Pending::Ready(Ok(PeakBlock {
            level: l as u8,
            from: a as u64,
            samples_per_pair: Peaks::samples_per_pair(l),
            pairs: pairs[a..z].into(),
            length: p.length,
        }))
    }

    fn param_range(&self, node: NodeId, key: &str) -> Option<lm_port::ParamRange> {
        ranges::param_range(self.doc.find(node).as_ref(), node, key)
    }

    fn tempo_map(&self) -> std::sync::Arc<lm_port::TempoMap> {
        self.tempo.get(self.root())
    }

    fn display(&self, node: NodeId) -> Option<DisplayView> {
        self.rack_display(node)
    }

    fn preferences(&self) -> &Value {
        &self.preferences
    }

    fn set_preference(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        Pending::Ready(self.preferences.with_path(&path, value).map(|p| self.preferences = p))
    }

    fn status(&self) -> &Value {
        &self.status
    }

    fn devices(&mut self) -> Pending<Vec<DeviceInfo>> {
        Pending::Ready(Ok(library::devices()))
    }

    fn import(&mut self, path: std::path::PathBuf) -> Pending<Resource> {
        services::import(self, &path)
    }

    fn browse(&mut self, page: BrowsePage, parent: Option<EntryId>, show_hidden: bool) -> Pending<Vec<BrowseEntry>> {
        Pending::Ready(Ok(self.library.children(page, parent, show_hidden)))
    }

    fn search(&mut self, query: &str) -> Pending<Vec<BrowseEntry>> {
        Pending::Ready(Ok(self.library.search(query)))
    }

    fn prepare_preview(&mut self, entry: EntryId) -> Pending<Preview> {
        match self.library.entry(entry) {
            Some(e) => {
                let res = demo::audio(&format!("preview-{}", entry.0));
                let seconds = e.info.get("seconds").and_then(Value::as_real).unwrap_or(2.0);
                Pending::Ready(Ok(Preview {
                    entry,
                    resource: res,
                    length: (seconds * SAMPLE_RATE as f64) as i64,
                    sample_rate: SAMPLE_RATE,
                }))
            }
            None => Pending::Ready(Err(err("port.browse.unknown_entry"))),
        }
    }

    fn preview_play(&mut self, entry: EntryId, _from: i64) {
        self.preview = Some((entry, Instant::now()));
    }

    fn preview_stop(&mut self) {
        self.preview = None;
    }

    fn set_favorite(&mut self, entry: EntryId, on: bool) -> Pending<()> {
        Pending::Ready(self.library.set_favorite(entry, on))
    }

    fn plugins(&self) -> Vec<lm_port::PluginInfo> {
        let fake = demo::PLUGINS.iter().filter(|(id, ..)| !self.real_plugins.iter().any(|(r, _)| &**r == *id));
        fake.map(|(id, instrument, _)| lm_port::PluginInfo {
            id: Arc::from(*id),
            name: Arc::from(*id),
            instrument: *instrument,
            category: Arc::from(demo::category(id)),
            third_party: false,
            latency: demo::latency(id),
            sidechain: demo::ports(id, &Value::empty_object()).is_some_and(|p| !p.sidechains.is_empty()),
        })
        .chain(self.real_plugins.iter().map(|(id, instrument)| lm_port::PluginInfo {
            id: Arc::clone(id),
            name: Arc::clone(id),
            instrument: *instrument,
            category: Arc::from(demo::category(id)),
            third_party: false,
            latency: 0,
            // the real plugins' ports are the core's (the LocalPort answers them); the fake does not run them
            sidechain: false,
        }))
        .collect()
    }

    fn plugin_schema(&self, plugin: &str) -> Option<lm_port::NodeSchema> {
        self.doc.registry().find(&format!("plugin:{plugin}")).cloned()
    }

    fn ui(&self) -> &Value {
        &self.ui
    }

    fn set_ui(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        Pending::Ready(self.ui.with_path(&path, value).map(|u| self.ui = u))
    }

    fn midi_learn(&mut self, target: lm_port::MapTarget, scope: lm_port::MapScope) -> Pending<lm_port::LearnId> {
        Pending::Ready(self.midi_learn_impl(target, scope))
    }

    fn cancel_learn(&mut self, id: lm_port::LearnId) {
        self.cancel_learn_impl(id);
    }

    fn learning(&self) -> Option<lm_port::LearnId> {
        self.learning_impl()
    }

    fn unmap(&mut self, target: &lm_port::MapTarget, scope: lm_port::MapScope) -> Pending<()> {
        Pending::Ready(self.unmap_impl(target, scope))
    }

    fn drain_midi_ui(&mut self, out: &mut Vec<lm_port::MidiUiAction>) {
        self.drain_midi_ui_impl(out);
    }

    fn midi_activity(&self, since: u64) -> (u64, Vec<lm_port::MidiActivity>) {
        self.midi_activity_impl(since)
    }
}

impl FakePort {
    /// `{node, entry}`: the preset entry's state on the plugin node (the fake's presets are deterministic variations
    /// of the defaults, by the entry's number)
    fn load_preset(&mut self, params: &Value) -> Result<()> {
        let node = params.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
        let entry = params.get("entry").and_then(Value::as_int);
        let (Some(node), Some(entry)) = (node, entry) else { return Err(err("port.params")) };
        let preset = self.library.entry(EntryId(entry as u64)).ok_or_else(|| err("port.browse.unknown_entry"))?;
        let plugin_node = CorePort::find(self, node).ok_or_else(|| err("port.params"))?;
        let plugin = plugin_node.get("plugin").and_then(Value::as_text).unwrap_or("").to_string();
        if preset.info.get("plugin").and_then(Value::as_text) != Some(plugin.as_str()) {
            return Err(err("plugin.preset.other_plugin"));
        }
        let schema = self.plugin_schema(&plugin).ok_or_else(|| err("port.params"))?;
        let ops: Vec<EditOp> = schema
            .params
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let f = ((entry as usize * 7 + k * 13) % 10) as f64 / 10.0;
                let v = p.min + (p.max - p.min) * f;
                let v = if p.scale == lm_state::Scale::Stepped { v.round() } else { v };
                EditOp::set(node, &["state", "params", p.key], Value::real(v).unwrap_or_default())
            })
            .collect();
        match CorePort::apply(self, Label::new("Load a preset"), ops) {
            Pending::Ready(r) => r.map(|_| ()),
            Pending::Waiting(_) => Ok(()),
        }
    }

    /// `{track, path}`: reads a `.scl` or `.kbm` and cites it from the track's tuning (the fake keeps no project
    /// folder: the resource is named by the content); `tuning.clear {track}` removes the tuning (one undo step)
    fn set_tuning(&mut self, action: &str, params: &Value) -> Result<()> {
        let track = params.get("track").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
        let Some(track) = track else { return Err(err("port.params")) };
        let Some(node) = self.doc.find(track) else { return Err(err("port.params")) };
        let current = node.get("tuning").filter(|t| !t.is_null()).cloned();
        let (label, tuning) = if action == "tuning.clear" {
            ("Clear the tuning", Value::NULL)
        } else {
            let path = params.get("path").and_then(Value::as_text).ok_or_else(|| err("port.params"))?;
            let bytes = std::fs::read(path).map_err(|_| err("tuning.file.unreadable"))?;
            let text = lm_doc::tuning::text_of(&bytes);
            let keymap = action == "tuning.set_keymap";
            if keymap {
                lm_doc::tuning::parse_kbm(&text)?;
            } else {
                lm_doc::tuning::parse_scl(&text)?;
            }
            let resource = lm_state::Resource::of_content(lm_doc::schemas::TUNING_KIND, &bytes)?;
            let base = current
                .unwrap_or_else(|| Value::object([("on", Value::bool(true)), ("importRanges", Value::bool(true))]));
            (
                if keymap { "Set the keymap" } else { "Set the scale" },
                base.with(if keymap { "keymap" } else { "scale" }, Value::resource(resource)),
            )
        };
        match CorePort::apply(self, Label::new(label), vec![EditOp::set(track, &["tuning"], tuning)]) {
            Pending::Ready(r) => r.map(|_| ()),
            Pending::Waiting(_) => Ok(()),
        }
    }

    /// `entry`: a notice with the file's place (the fake has no file manager to open)
    fn reveal(&mut self, params: &Value) -> Result<()> {
        let entry = params
            .as_int()
            .and_then(|e| self.library.entry(EntryId(e as u64)))
            .ok_or_else(|| err("port.browse.unknown_entry"))?;
        let path = entry.info.get("path").and_then(Value::as_text).unwrap_or(&entry.name).to_string();
        self.push_notice(
            "Shown in the file manager: {0}",
            vec![("path", Value::text(path.as_str()))],
            Severity::Info,
            None,
            Vec::new(),
        );
        Ok(())
    }

    /// `entry`: opens a project of the browser (the fake runs the opening's operation on the project)
    fn open_entry(&mut self, params: &Value) -> Result<()> {
        let entry = params
            .as_int()
            .and_then(|e| self.library.entry(EntryId(e as u64)))
            .ok_or_else(|| err("port.browse.unknown_entry"))?;
        if entry.kind != EntryKind::Project {
            return Err(err("port.params"));
        }
        self.start(Label::new("Opening the project"), Target::Project, true, Duration::from_millis(800), |_| {});
        Ok(())
    }

    /// `{node, name}`: a new user preset of the node's plugin, on the Presets page
    fn save_preset(&mut self, params: &Value) -> Result<()> {
        let node = params.get("node").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
        let name = params.get("name").and_then(Value::as_text).filter(|n| !n.trim().is_empty());
        let (Some(node), Some(name)) = (node, name) else { return Err(err("port.params")) };
        let plugin_node = CorePort::find(self, node).ok_or_else(|| err("port.params"))?;
        let plugin = plugin_node.get("plugin").and_then(Value::as_text).unwrap_or("").to_string();
        self.library.add_preset(&plugin, name);
        Ok(())
    }
}

/// The preview playing now, if any (the fake has no sound; the browser shows its cursor)
impl FakePort {
    pub fn previewing(&self) -> Option<(EntryId, Duration)> {
        self.preview.map(|(e, t)| (e, t.elapsed()))
    }

    /// The xruns the fake counts: tests make a pump late on purpose
    /// The last value written by `param_write`: node, key, value
    pub fn last_param_write(&self) -> Option<(NodeId, &str, f32)> {
        self.last_write.as_ref().map(|(n, k, v)| (*n, k.as_str(), *v))
    }

    /// The live notes the interface sent (a handle that outlives the boxed port, for tests)
    pub fn live_log(&self) -> LiveLog {
        self.notes_log.clone()
    }

    pub fn pumps(&self) -> u64 {
        self.pumps
    }

    /// Writes the document's saved form (what `project.save` would write)
    pub fn saved(&self) -> Value {
        self.doc.saved()
    }
}

#[cfg(test)]
mod tests;
