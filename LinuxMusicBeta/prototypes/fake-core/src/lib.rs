//! A fake of the core's surface for the GUI (interfaces.md, I-15), small but with the same shape:
//! - a `!Send` `Session` the UI owns, with `pump()` meant for a 10 ms main-loop timer (H2);
//! - `transact`, gestures by `GestureId` (I-3.7), `Feed`s pulled after each pump (I-3.6);
//! - `loading(target)` with the 200 ms rule (I-2), fed by a real worker thread with an injectable delay;
//! - telemetry through a triple buffer from a fake "engine" thread publishing at ~60 Hz (I-8, H16), with the
//!   position and its host time so the UI interpolates the cursor (I-7.2);
//! - the i16 min/max peak pyramid (I-20.4);
//! - parameter slots as atomics (I-6): the volume a gesture writes reaches the fake engine's meters at once.
//!
//! The document is a plain struct, not the `Value` tree: the prototype measures the GUI, not the document.

pub mod bench;
pub mod peaks;
pub mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use peaks::Peaks;

pub const SAMPLE_RATE: u32 = 48_000;
pub const LOADING_THRESHOLD: Duration = Duration::from_millis(200);
pub const CANCEL_AFTER: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------------------------------------------
// Ids, values, the document

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    pub kind: u8,
    pub number: u64,
}
impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind as char, self.number)
    }
}

/// A recorded file's peaks, by content (a hash in the real core)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId(pub u32);

#[derive(Clone, Debug)]
pub struct Item {
    pub id: NodeId,
    pub start: i64,
    pub length: i64,
    pub resource: ResourceId,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: NodeId,
    pub name: String,
    pub color: u32,
    pub volume_db: f64,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub item: Item,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub id: NodeId,
    pub name: String,
    pub volume_db: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub tracks: Vec<Track>,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Volume,
    Mute,
    Solo,
    Armed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Real(f64),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    NoSuchNode(NodeId),
    WrongType,
    GestureClosed,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    User,
    Undo,
    Gesture,
    Loading,
}

/// What changed; merged on the final state (T13)
#[derive(Clone, Debug, Default)]
pub struct Diff {
    pub changed: Vec<(NodeId, Option<Field>)>,
}
impl Diff {
    fn merge(&mut self, other: &Diff) {
        for c in &other.changed {
            if !self.changed.contains(c) {
                self.changed.push(*c);
            }
        }
    }
    pub fn touches(&self, id: NodeId) -> bool {
        self.changed.iter().any(|(n, _)| *n == id)
    }
}

/// A transaction's or gesture's way into the document
pub struct Edit<'a> {
    doc: &'a mut Doc,
    diff: &'a mut Diff,
    touched: Option<&'a mut Vec<(NodeId, Field, Value)>>,
}

impl Edit<'_> {
    pub fn set(&mut self, id: NodeId, field: Field, v: Value) -> Result<()> {
        let old = self.get(id, field)?;
        if let Some(t) = self.touched.as_deref_mut() {
            if !t.iter().any(|(n, f, _)| *n == id && *f == field) {
                t.push((id, field, old));
            }
        }
        match (id.kind, field, v) {
            (b't', Field::Volume, Value::Real(x)) => self.track(id)?.volume_db = x,
            (b't', Field::Mute, Value::Bool(b)) => self.track(id)?.mute = b,
            (b't', Field::Solo, Value::Bool(b)) => self.track(id)?.solo = b,
            (b't', Field::Armed, Value::Bool(b)) => self.track(id)?.armed = b,
            (b'm', Field::Volume, Value::Real(x)) => self.channel(id)?.volume_db = x,
            _ => return Err(Error::WrongType),
        }
        self.diff.changed.push((id, Some(field)));
        Ok(())
    }
    fn get(&mut self, id: NodeId, field: Field) -> Result<Value> {
        Ok(match (id.kind, field) {
            (b't', Field::Volume) => Value::Real(self.track(id)?.volume_db),
            (b't', Field::Mute) => Value::Bool(self.track(id)?.mute),
            (b't', Field::Solo) => Value::Bool(self.track(id)?.solo),
            (b't', Field::Armed) => Value::Bool(self.track(id)?.armed),
            (b'm', Field::Volume) => Value::Real(self.channel(id)?.volume_db),
            _ => return Err(Error::WrongType),
        })
    }
    fn track(&mut self, id: NodeId) -> Result<&mut Track> {
        self.doc.tracks.iter_mut().find(|t| t.id == id).ok_or(Error::NoSuchNode(id))
    }
    fn channel(&mut self, id: NodeId) -> Result<&mut Channel> {
        self.doc.channels.iter_mut().find(|c| c.id == id).ok_or(Error::NoSuchNode(id))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Feeds and gestures

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Node(NodeId),
    All,
}

struct FeedShared {
    topic: Topic,
    pending: RefCell<Option<Diff>>,
}

/// A receiver of diffs for one topic (I-3.6). Dropping it makes the session stop accumulating for it
#[must_use]
pub struct Feed {
    shared: Rc<FeedShared>,
}
impl Feed {
    /// The diffs since the last drain, merged. Called by the UI in its frame, after `Session::pump`
    pub fn drain(&mut self, _session: &Session) -> Option<Diff> {
        self.shared.pending.borrow_mut().take()
    }
}

/// An open gesture (I-3.7). Dropped without commit or cancel: the session commits it on the next pump
#[must_use]
pub struct GestureId {
    id: u64,
    dropped: Rc<RefCell<Vec<u64>>>,
    live: bool,
}
impl Drop for GestureId {
    fn drop(&mut self) {
        if self.live {
            self.dropped.borrow_mut().push(self.id);
        }
    }
}
impl fmt::Debug for GestureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GestureId({})", self.id)
    }
}

struct OpenGesture {
    label: &'static str,
    before: Doc,
    touched: Vec<(NodeId, Field, Value)>,
}

// ---------------------------------------------------------------------------------------------------------------
// Long operations and loading (I-2)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Project,
    Node(NodeId),
    Resource(ResourceId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Loading {
    Ready,
    /// Started less than 200 ms ago: draw nothing different
    Pending,
    /// Past 200 ms: draw the loading in place of the object
    Visible { progress: Option<f32>, stage: &'static str, cancellable: bool, since: Duration },
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Cancel(Arc<AtomicBool>);
impl Cancel {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
#[derive(Debug, Clone)]
pub struct Progress(Arc<AtomicU32>);
impl Progress {
    pub fn set(&self, p: f32) {
        self.0.store(p.to_bits(), Ordering::Relaxed)
    }
    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

struct OpInfo {
    target: Target,
    started: Instant,
    stage: &'static str,
    cancellable: bool,
    progress: Progress,
    cancel: Cancel,
    finished: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// A long operation's result, polled from the pump
#[must_use]
pub struct OperationHandle<T> {
    slot: Arc<Mutex<Option<std::result::Result<T, String>>>>,
    pub id: u64,
}
impl<T> OperationHandle<T> {
    pub fn poll(&self) -> Option<std::result::Result<T, String>> {
        self.slot.lock().unwrap().take()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Telemetry from the fake engine (I-8)

pub const MAX_METERS: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct TelemetryFrame {
    pub position: i64,
    pub host_time: Instant,
    pub playing: bool,
    pub recording: bool,
    /// linear peak 0..1 by meter index: tracks 0..64, then channels 64..128
    pub meters: [f32; MAX_METERS],
    pub serial: u64,
}

/// The interpolated position the UI draws with
#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub samples: i64,
    pub host_time: Instant,
    pub playing: bool,
    pub recording: bool,
}
impl Position {
    pub fn at(&self, now: Instant) -> f64 {
        if self.playing {
            self.samples as f64 + now.saturating_duration_since(self.host_time).as_secs_f64() * SAMPLE_RATE as f64
        } else {
            self.samples as f64
        }
    }
}

/// Parameter slots (I-6): atomics the main thread writes and the engine reads
struct Slots {
    gain: Vec<AtomicU32>, // linear, by meter index
}

struct EngineShared {
    playing: AtomicBool,
    recording: AtomicBool,
    locate: AtomicI64, // i64::MIN = none
    quit: AtomicBool,
    slots: Slots,
}

fn engine_thread(
    shared: Arc<EngineShared>,
    mut input: triple_buffer::Input<TelemetryFrame>,
    sources: Vec<(Arc<Peaks>, i64)>,
) {
    let tick = Duration::from_secs_f64(1.0 / 60.0);
    let mut next = Instant::now();
    let mut pos: i64 = 0;
    let mut last = Instant::now();
    let mut serial = 0u64;
    let mut held = [0f32; MAX_METERS];
    let mut noise = peaks::Rng(0x1234_5678);
    while !shared.quit.load(Ordering::Relaxed) {
        next += tick;
        let now = Instant::now();
        if let Some(d) = next.checked_duration_since(now) {
            std::thread::sleep(d);
        } else {
            next = now;
        }
        let now = Instant::now();
        let l = shared.locate.swap(i64::MIN, Ordering::Relaxed);
        if l != i64::MIN {
            pos = l;
        }
        let playing = shared.playing.load(Ordering::Relaxed);
        if playing {
            pos += (now.duration_since(last).as_secs_f64() * SAMPLE_RATE as f64) as i64;
        }
        last = now;
        let span = (SAMPLE_RATE / 60) as u64;
        let mut meters = [0f32; MAX_METERS];
        let n = sources.len().min(MAX_METERS / 2);
        for (i, (peaks, start)) in sources.iter().enumerate().take(n) {
            let a = if playing { peaks.amplitude_at(pos - start, span) } else { 0.0 };
            let a = a * (0.85 + 0.15 * noise.unit() as f32);
            let track = a * f32::from_bits(shared.slots.gain[i].load(Ordering::Relaxed));
            let channel = track * f32::from_bits(shared.slots.gain[64 + i].load(Ordering::Relaxed));
            meters[i] = track;
            meters[64 + i] = channel;
        }
        for (h, m) in held.iter_mut().zip(meters.iter()) {
            *h = m.max(*h * 0.8); // the engine accumulates peaks per block; a short hold
            if *h < 1e-4 {
                *h = 0.0; // below -80 dB is silence: no endless tail of changing frames
            }
        }
        serial += 1;
        input.write(TelemetryFrame {
            position: pos,
            host_time: now,
            playing,
            recording: shared.recording.load(Ordering::Relaxed),
            meters: held,
            serial,
        });
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The session

/// Proof of the main thread; makes the session `!Send` (I-1)
struct Main {
    _not_send: PhantomData<*const ()>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub tracks: usize,
    pub channels: usize,
    /// The injected delay of the resource that loads slowly (I-2.4)
    pub slow_delay: Duration,
    pub slow_track: usize,
    pub item_seconds: f64,
}
impl Default for Config {
    fn default() -> Self {
        Config { tracks: 64, channels: 64, slow_delay: Duration::from_millis(1500), slow_track: 2, item_seconds: 180.0 }
    }
}

pub struct Session {
    doc: Doc,
    undo: Vec<Doc>,
    pending: Diff,
    feeds: Vec<Rc<FeedShared>>,
    gestures: HashMap<u64, OpenGesture>,
    dropped_gestures: Rc<RefCell<Vec<u64>>>,
    next_id: u64,
    ops: HashMap<u64, OpInfo>,
    resources: HashMap<ResourceId, Arc<Peaks>>,
    pending_resources: Vec<(ResourceId, OperationHandle<Arc<Peaks>>)>,
    engine: Arc<EngineShared>,
    telemetry: triple_buffer::Output<TelemetryFrame>,
    last_frame: TelemetryFrame,
    engine_thread: Option<JoinHandle<()>>,
    config: Config,
    pumps: u64,
    _main: Main,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("tracks", &self.doc.tracks.len()).finish_non_exhaustive()
    }
}

const NAMES: [&str; 8] = ["Bateria", "Voz principal", "Backing", "Guitarra", "Baixo", "Teclado", "Sopros", "Percussão"];

impl Session {
    pub fn create(config: Config) -> Session {
        let resources: HashMap<ResourceId, Arc<Peaks>> = (0..8u32)
            .map(|r| (ResourceId(r), Arc::new(Peaks::synthesize(r as u64 + 1, config.item_seconds, SAMPLE_RATE))))
            .collect();
        let mut doc = Doc::default();
        let mut sources = Vec::new();
        for i in 0..config.tracks {
            let resource = ResourceId((i % 8) as u32);
            let start = ((i % 5) as i64) * SAMPLE_RATE as i64 * 2;
            doc.tracks.push(Track {
                id: NodeId { kind: b't', number: i as u64 + 1 },
                name: format!("{} {}", NAMES[i % 8], i / 8 + 1),
                color: view::TRACK_COLORS[i % 8],
                volume_db: 0.0,
                mute: false,
                solo: false,
                armed: i == 1,
                item: Item {
                    id: NodeId { kind: b'i', number: 1000 + i as u64 },
                    start,
                    length: (config.item_seconds * SAMPLE_RATE as f64) as i64,
                    resource,
                },
            });
            sources.push((resources[&resource].clone(), start));
        }
        for i in 0..config.channels {
            doc.channels.push(Channel {
                id: NodeId { kind: b'm', number: i as u64 + 1 },
                name: format!("{}", i + 1),
                volume_db: 0.0,
            });
        }
        let initial = TelemetryFrame {
            position: 0,
            host_time: Instant::now(),
            playing: false,
            recording: false,
            meters: [0.0; MAX_METERS],
            serial: 0,
        };
        let (input, output) = triple_buffer::triple_buffer(&initial);
        let engine = Arc::new(EngineShared {
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            locate: AtomicI64::new(i64::MIN),
            quit: AtomicBool::new(false),
            slots: Slots { gain: (0..MAX_METERS).map(|_| AtomicU32::new(1f32.to_bits())).collect() },
        });
        let e = engine.clone();
        let engine_thread = std::thread::Builder::new()
            .name("fake-engine".into())
            .spawn(move || engine_thread(e, input, sources))
            .expect("spawn engine");
        let mut s = Session {
            doc,
            undo: Vec::new(),
            pending: Diff::default(),
            feeds: Vec::new(),
            gestures: HashMap::new(),
            dropped_gestures: Rc::new(RefCell::new(Vec::new())),
            next_id: 1,
            ops: HashMap::new(),
            resources,
            pending_resources: Vec::new(),
            engine,
            telemetry: output,
            last_frame: initial,
            engine_thread: Some(engine_thread),
            config,
            pumps: 0,
            _main: Main { _not_send: PhantomData },
        };
        // the slow take: its peaks are "generated" again, with the injected delay
        let slow = s.config.slow_track;
        s.reload_take(slow);
        s
    }

    // --- the frame, in the order of I-15 --------------------------------------------------------------------

    /// Collects what the engine and the workers produced and hands the diffs to the feeds. On a 10 ms timer (H2).
    /// Returns true when anything the UI shows may have changed (a new telemetry frame, a diff, a loading state).
    pub fn pump(&mut self) -> bool {
        self.pumps += 1;
        let mut changed = false;
        // telemetry
        // a new frame counts as a change only if what it shows changed (no redraws while stopped and silent)
        let f = *self.telemetry.read();
        if f.serial != self.last_frame.serial
            && (f.playing || f.playing != self.last_frame.playing || f.position != self.last_frame.position
                || f.meters != self.last_frame.meters)
        {
            changed = true;
        }
        self.last_frame = f;
        // operations
        let finished: Vec<u64> =
            self.ops.iter().filter(|(_, o)| o.finished.load(Ordering::Acquire)).map(|(k, _)| *k).collect();
        for id in finished {
            if let Some(mut op) = self.ops.remove(&id) {
                if let Some(t) = op.thread.take() {
                    let _ = t.join();
                }
                changed = true;
            }
        }
        let mut i = 0;
        while i < self.pending_resources.len() {
            if let Some(r) = self.pending_resources[i].1.poll() {
                let (res, _) = self.pending_resources.remove(i);
                if let Ok(peaks) = r {
                    self.resources.insert(res, peaks);
                }
                let items: Vec<NodeId> =
                    self.doc.tracks.iter().filter(|t| t.item.resource == res).map(|t| t.item.id).collect();
                for it in items {
                    self.pending.changed.push((it, None));
                }
            } else {
                i += 1;
            }
        }
        if self.ops.values().any(|o| o.started.elapsed() >= LOADING_THRESHOLD) {
            changed = true; // the loading's progress moves
        }
        // dropped gestures are committed
        let dropped: Vec<u64> = self.dropped_gestures.borrow_mut().drain(..).collect();
        for id in dropped {
            eprintln!("debug: gesture {id} dropped without commit; committed");
            self.commit_gesture_id(id);
        }
        // dropped feeds are forgotten; the diffs go to the live ones
        self.feeds.retain(|f| Rc::strong_count(f) > 1);
        if !self.pending.changed.is_empty() {
            changed = true;
            let diff = std::mem::take(&mut self.pending);
            for f in &self.feeds {
                let wants = match f.topic {
                    Topic::All => true,
                    Topic::Node(id) => diff.touches(id),
                };
                if wants {
                    let mut p = f.pending.borrow_mut();
                    match p.as_mut() {
                        Some(d) => d.merge(&diff),
                        None => *p = Some(diff.clone()),
                    }
                }
            }
        }
        changed
    }

    pub fn feed(&mut self, topic: Topic) -> Feed {
        let shared = Rc::new(FeedShared { topic, pending: RefCell::new(None) });
        self.feeds.push(shared.clone());
        Feed { shared }
    }

    // --- reading -------------------------------------------------------------------------------------------

    pub fn doc(&self) -> &Doc {
        &self.doc
    }
    pub fn tracks(&self) -> &[Track] {
        &self.doc.tracks
    }
    pub fn channels(&self) -> &[Channel] {
        &self.doc.channels
    }
    /// The latest telemetry meters, linear peak, by meter index (tracks, then channels)
    pub fn meters(&self) -> &[f32; MAX_METERS] {
        &self.last_frame.meters
    }
    pub fn position(&self) -> Position {
        Position {
            samples: self.last_frame.position,
            host_time: self.last_frame.host_time,
            playing: self.last_frame.playing,
            recording: self.last_frame.recording,
        }
    }
    /// The peaks of a resource, if loaded (immutable, shared: the UI may keep the Arc)
    pub fn peaks(&self, r: ResourceId) -> Option<&Arc<Peaks>> {
        if self.pending_resources.iter().any(|(p, _)| *p == r) {
            return None;
        }
        self.resources.get(&r)
    }
    pub fn pumps(&self) -> u64 {
        self.pumps
    }

    pub fn loading(&self, target: &Target) -> Loading {
        let mut worst: Option<&OpInfo> = None;
        for op in self.ops.values() {
            let hit = op.target == *target
                || match (target, op.target) {
                    // aggregation (T10): an item is loading while its resource is
                    (Target::Node(n), Target::Resource(r)) => {
                        self.doc.tracks.iter().any(|t| t.item.id == *n && t.item.resource == r)
                    }
                    _ => false,
                };
            if hit && worst.is_none_or(|w| op.started < w.started) {
                worst = Some(op);
            }
        }
        match worst {
            None => Loading::Ready,
            Some(op) => {
                let since = op.started.elapsed();
                if since < LOADING_THRESHOLD {
                    Loading::Pending
                } else {
                    Loading::Visible {
                        progress: Some(op.progress.get()),
                        stage: op.stage,
                        cancellable: op.cancellable && since >= CANCEL_AFTER,
                        since,
                    }
                }
            }
        }
    }

    // --- changing ------------------------------------------------------------------------------------------

    pub fn transact(&mut self, _label: &'static str, edit: impl FnOnce(&mut Edit) -> Result<()>) -> Result<Diff> {
        let before = self.doc.clone();
        let mut diff = Diff::default();
        let r = edit(&mut Edit { doc: &mut self.doc, diff: &mut diff, touched: None });
        match r {
            Ok(()) => {
                self.undo.push(before);
                self.after_change(&diff);
                Ok(diff)
            }
            Err(e) => {
                self.doc = before; // refused whole
                Err(e)
            }
        }
    }

    pub fn begin_gesture(&mut self, label: &'static str) -> GestureId {
        let id = self.next_id;
        self.next_id += 1;
        self.gestures.insert(id, OpenGesture { label, before: self.doc.clone(), touched: Vec::new() });
        GestureId { id, dropped: self.dropped_gestures.clone(), live: true }
    }

    /// Changes the current root without a history step (I-3.7)
    pub fn gesture_update(&mut self, g: &GestureId, edit: impl FnOnce(&mut Edit) -> Result<()>) -> Result<()> {
        let open = self.gestures.get_mut(&g.id).ok_or(Error::GestureClosed)?;
        let mut diff = Diff::default();
        edit(&mut Edit { doc: &mut self.doc, diff: &mut diff, touched: Some(&mut open.touched) })?;
        self.after_change(&diff);
        Ok(())
    }

    pub fn gesture_commit(&mut self, mut g: GestureId) -> Result<Diff> {
        g.live = false;
        self.commit_gesture_id(g.id).ok_or(Error::GestureClosed)
    }

    pub fn gesture_cancel(&mut self, mut g: GestureId) {
        g.live = false;
        if let Some(open) = self.gestures.remove(&g.id) {
            let mut diff = Diff::default();
            let mut e = Edit { doc: &mut self.doc, diff: &mut diff, touched: None };
            for (id, f, v) in open.touched {
                let _ = e.set(id, f, v); // only the paths this gesture touched (G1)
            }
            self.after_change(&diff);
        }
    }

    fn commit_gesture_id(&mut self, id: u64) -> Option<Diff> {
        let open = self.gestures.remove(&id)?;
        let _ = open.label;
        self.undo.push(open.before);
        Some(Diff { changed: open.touched.iter().map(|(n, f, _)| (*n, Some(*f))).collect() })
    }

    pub fn undo(&mut self) -> Result<Diff> {
        // G2: commit every open gesture first
        let open: Vec<u64> = self.gestures.keys().copied().collect();
        for id in open {
            self.commit_gesture_id(id);
        }
        let Some(prev) = self.undo.pop() else { return Ok(Diff::default()) };
        self.doc = prev;
        let diff = Diff {
            changed: self.doc.tracks.iter().map(|t| (t.id, None)).chain(self.doc.channels.iter().map(|c| (c.id, None))).collect(),
        };
        self.after_change(&diff);
        Ok(diff)
    }

    fn after_change(&mut self, diff: &Diff) {
        // parameter slots: the fake engine sees the new gains at once (I-6)
        for (id, f) in &diff.changed {
            if *f == Some(Field::Volume) || *f == Some(Field::Mute) {
                if let Some((i, t)) = self.doc.tracks.iter().enumerate().find(|(_, t)| t.id == *id) {
                    let g = if t.mute { 0.0 } else { db_gain(t.volume_db) };
                    self.engine.slots.gain[i].store(g.to_bits(), Ordering::Relaxed);
                }
                if let Some((i, c)) = self.doc.channels.iter().enumerate().find(|(_, c)| c.id == *id) {
                    self.engine.slots.gain[64 + i].store(db_gain(c.volume_db).to_bits(), Ordering::Relaxed);
                }
            }
        }
        self.pending.merge(diff);
    }

    // --- operations ----------------------------------------------------------------------------------------

    /// Starts `work` on a worker thread for `target` (I-2.2)
    pub fn start<T: Send + 'static>(
        &mut self,
        stage: &'static str,
        target: Target,
        cancellable: bool,
        work: impl FnOnce(&Cancel, &Progress) -> std::result::Result<T, String> + Send + 'static,
    ) -> OperationHandle<T> {
        let id = self.next_id;
        self.next_id += 1;
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let progress = Progress(Arc::new(AtomicU32::new(0f32.to_bits())));
        let finished = Arc::new(AtomicBool::new(false));
        let slot = Arc::new(Mutex::new(None));
        let (c, p, f, s) = (cancel.clone(), progress.clone(), finished.clone(), slot.clone());
        let thread = std::thread::Builder::new()
            .name("worker".into())
            .spawn(move || {
                let r = work(&c, &p);
                *s.lock().unwrap() = Some(r);
                f.store(true, Ordering::Release);
            })
            .expect("spawn worker");
        self.ops.insert(
            id,
            OpInfo { target, started: Instant::now(), stage, cancellable, progress, cancel, finished, thread: Some(thread) },
        );
        OperationHandle { slot, id }
    }

    pub fn cancel(&mut self, op: u64) {
        if let Some(o) = self.ops.get(&op) {
            o.cancel.0.store(true, Ordering::Relaxed);
        }
    }

    /// Cancels every operation the loading of `target` is waiting on (the loading component's "Cancel")
    pub fn cancel_target(&mut self, target: &Target) {
        let item_res: Option<ResourceId> = match target {
            Target::Node(n) => self.doc.tracks.iter().find(|t| t.item.id == *n).map(|t| t.item.resource),
            _ => None,
        };
        for o in self.ops.values() {
            if o.target == *target || matches!(o.target, Target::Resource(r) if Some(r) == item_res) {
                o.cancel.0.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Regenerates a track's take peaks with the injected delay: the loading component's test case
    pub fn reload_take(&mut self, track: usize) {
        let Some(t) = self.doc.tracks.get(track) else { return };
        let res = t.item.resource;
        if self.pending_resources.iter().any(|(r, _)| *r == res) {
            return;
        }
        let delay = self.config.slow_delay;
        let seconds = self.config.item_seconds;
        let h = self.start("Gerando picos", Target::Resource(res), true, move |cancel, progress| {
            let steps = 50;
            for i in 0..steps {
                if cancel.is_cancelled() {
                    return Err("cancelled".into());
                }
                std::thread::sleep(delay / steps);
                progress.set((i + 1) as f32 / steps as f32);
            }
            Ok(Arc::new(Peaks::synthesize(res.0 as u64 + 1, seconds, SAMPLE_RATE)))
        });
        self.pending_resources.push((res, h));
    }

    // --- transport (commands to the engine, I-7) -----------------------------------------------------------

    pub fn play(&mut self) {
        self.engine.playing.store(true, Ordering::Relaxed);
    }
    pub fn stop(&mut self) {
        if !self.engine.playing.swap(false, Ordering::Relaxed) {
            self.engine.locate.store(0, Ordering::Relaxed); // stop twice: back to the start
        }
        self.engine.recording.store(false, Ordering::Relaxed);
    }
    pub fn toggle_record(&mut self) {
        let r = !self.engine.recording.load(Ordering::Relaxed);
        self.engine.recording.store(r, Ordering::Relaxed);
        if r {
            self.play();
        }
    }
    pub fn locate(&mut self, sample: i64) {
        self.engine.locate.store(sample.max(0), Ordering::Relaxed);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.engine.quit.store(true, Ordering::Relaxed);
        if let Some(t) = self.engine_thread.take() {
            let _ = t.join();
        }
        for op in self.ops.values_mut() {
            op.cancel.0.store(true, Ordering::Relaxed);
            if let Some(t) = op.thread.take() {
                let _ = t.join();
            }
        }
    }
}

pub fn db_gain(db: f64) -> f32 {
    if db.is_infinite() || db <= -60.0 { 0.0 } else { 10f64.powf(db / 20.0) as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> Config {
        Config { tracks: 4, channels: 4, slow_delay: Duration::from_millis(400), slow_track: 0, item_seconds: 5.0 }
    }

    #[test]
    fn loading_follows_the_200_ms_rule() {
        let mut s = Session::create(small());
        let item = s.tracks()[0].item.id;
        let res = s.tracks()[0].item.resource;
        assert_eq!(s.loading(&Target::Node(item)), Loading::Pending);
        assert!(s.peaks(res).is_none());
        std::thread::sleep(Duration::from_millis(250));
        s.pump();
        assert!(matches!(s.loading(&Target::Node(item)), Loading::Visible { .. }));
        std::thread::sleep(Duration::from_millis(300));
        s.pump();
        assert_eq!(s.loading(&Target::Node(item)), Loading::Ready);
        assert!(s.peaks(res).is_some());
    }

    #[test]
    fn gesture_is_one_undo_step_and_feeds_see_it() {
        let mut s = Session::create(small());
        let t = s.tracks()[1].id;
        let mut feed = s.feed(Topic::Node(t));
        let g = s.begin_gesture("Volume");
        for db in [-1.0, -2.0, -3.0] {
            s.gesture_update(&g, |e| e.set(t, Field::Volume, Value::Real(db))).unwrap();
        }
        s.gesture_commit(g).unwrap();
        s.pump();
        assert!(feed.drain(&s).is_some());
        assert!(feed.drain(&s).is_none());
        assert_eq!(s.tracks()[1].volume_db, -3.0);
        s.undo().unwrap();
        assert_eq!(s.tracks()[1].volume_db, 0.0);
    }

    #[test]
    fn dropped_gesture_commits_on_pump() {
        let mut s = Session::create(small());
        let t = s.tracks()[0].id;
        {
            let g = s.begin_gesture("Volume");
            s.gesture_update(&g, |e| e.set(t, Field::Volume, Value::Real(-6.0))).unwrap();
        }
        s.pump();
        assert!(s.gestures.is_empty());
        s.undo().unwrap();
        assert_eq!(s.tracks()[0].volume_db, 0.0);
    }
}
