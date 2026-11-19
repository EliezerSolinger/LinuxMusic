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

pub mod arrange;
pub mod bench;
pub mod mixer;
pub mod peaks;
pub mod view;
// ONE LINE PER AREA MODULE: the other screens' fake data (owners in docs/prototipo-interface.md)
pub mod actions;
pub mod browser;
pub mod notes;
pub mod notices;
pub mod plugins;
pub mod project;

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

pub use arrange::{Item, ItemKind};

#[derive(Clone, Debug)]
pub struct Track {
    pub id: NodeId,
    pub name: String,
    pub color: u32,
    pub kind: arrange::TrackKind,
    /// the folder it is in (spec item 2: folders organize, they do not sum audio)
    pub parent: Option<NodeId>,
    /// a folder's tracks hidden
    pub collapsed: bool,
    pub volume_db: f64,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub monitor: bool,
    /// the input, as the user named it ("Entrada 1 (mono)")
    pub input: String,
    pub outputs: Vec<arrange::Output>,
    pub items: Vec<Item>,
    pub lanes: Vec<arrange::Lane>,
    pub show_lanes: bool,
    pub show_takes: bool,
}

impl Track {
    /// The first audio item's resource (what the fake engine meters)
    pub fn main_resource(&self) -> Option<ResourceId> {
        self.items.iter().find_map(|i| i.resource())
    }
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub id: NodeId,
    pub name: String,
    pub kind: mixer::ChannelKind,
    pub volume_db: f64,
    /// -1 (left) .. +1 (right)
    pub pan: f64,
    pub mute: bool,
    pub solo: bool,
    /// None: the master (the master itself has None and goes to the device)
    pub output: Option<NodeId>,
    pub inserts: Vec<mixer::Insert>,
    pub sends: Vec<mixer::SendInfo>,
    /// the rack's parameters by permanent key (`rack.eq.low`...), and `rack.<section>.on` as 0/1
    pub params: std::collections::BTreeMap<&'static str, f64>,
    /// in the linked faders' relative group (P-18)
    pub linked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub tracks: Vec<Track>,
    pub channels: Vec<Channel>,
    pub song: arrange::Song,
    /// the instruments, their functions and the inserts' states (owner: agent C, `plugins.rs`)
    pub plugins: plugins::Plugins,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Volume,
    Mute,
    Solo,
    Armed,
    Pan,
    /// a parameter by permanent key (the rack's, a plugin's)
    Param(&'static str),
    /// a send's level, by the send's id
    SendLevel(NodeId),
    Monitor,
    Collapsed,
    ShowLanes,
    ShowTakes,
    /// a track output's destination channel, by the output's id (Value::Id)
    OutputTo(NodeId),
    /// a channel's output (Value::Id; the master has none)
    ChannelOutput,
    // the song (on ROOT)
    LoopOn,
    PunchOn,
    Metronome,
    Tempo,
    CountIn,
    PreRoll,
    // the desk (mixer.rs): an insert's point among the rack's sections and its bypass, the fader link, the mode
    InsertPoint(NodeId),
    Bypass(NodeId),
    FaderLink,
    DeskAnalog,
}

/// The document's root: the song-wide fields live on it
pub const ROOT: NodeId = NodeId { kind: b'r', number: 0 };

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Real(f64),
    Bool(bool),
    Id(NodeId),
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
            (b't', Field::Monitor, Value::Bool(b)) => self.track(id)?.monitor = b,
            (b't', Field::Collapsed, Value::Bool(b)) => self.track(id)?.collapsed = b,
            (b't', Field::ShowLanes, Value::Bool(b)) => self.track(id)?.show_lanes = b,
            (b't', Field::ShowTakes, Value::Bool(b)) => self.track(id)?.show_takes = b,
            (b't', Field::OutputTo(o), Value::Id(ch)) => {
                if !self.doc.channels.iter().any(|c| c.id == ch) {
                    return Err(Error::NoSuchNode(ch));
                }
                self.track(id)?.outputs.iter_mut().find(|x| x.id == o).ok_or(Error::NoSuchNode(o))?.to = ch;
            }
            (b'm', Field::ChannelOutput, Value::Id(to)) => {
                // no cycles in the routing: refused whole (I-3.5)
                let mut at = Some(to);
                while let Some(n) = at {
                    if n == id {
                        return Err(Error::WrongType);
                    }
                    at = self.doc.channels.iter().find(|c| c.id == n).and_then(|c| c.output);
                }
                self.channel(id)?.output = Some(to);
            }
            (b'r', Field::LoopOn, Value::Bool(b)) => self.doc.song.loop_range.on = b,
            (b'r', Field::PunchOn, Value::Bool(b)) => self.doc.song.punch.on = b,
            (b'r', Field::Metronome, Value::Bool(b)) => self.doc.song.metronome = b,
            (b'r', Field::Tempo, Value::Real(x)) => self.doc.song.tempo[0].bpm = x.clamp(20.0, 400.0),
            // a plugin parameter: the instrument functions (on the track), an instrument, an insert (plugins.rs)
            (b't' | b'p' | b'x', Field::Param(k), Value::Real(x)) => plugins::set_param(self.doc, id, k, x)?,
            (b'm', f @ (Field::InsertPoint(_) | Field::Bypass(_) | Field::FaderLink), v) => mixer::set(self.doc, id, f, v)?,
            (b'r', Field::DeskAnalog, Value::Bool(b)) => self.doc.song.desk_analog = b,
            (b'r', Field::CountIn, Value::Real(x)) => self.doc.song.count_in_bars = x.clamp(0.0, 4.0) as u8,
            (b'r', Field::PreRoll, Value::Real(x)) => self.doc.song.pre_roll_bars = x.clamp(0.0, 4.0) as u8,
            (b'm', Field::Volume, Value::Real(x)) => self.channel(id)?.volume_db = x,
            (b'm', Field::Pan, Value::Real(x)) => self.channel(id)?.pan = x.clamp(-1.0, 1.0),
            (b'm', Field::Mute, Value::Bool(b)) => self.channel(id)?.mute = b,
            (b'm', Field::Solo, Value::Bool(b)) => self.channel(id)?.solo = b,
            (b'm', Field::Param(k), Value::Real(x)) => {
                let x = mixer::spec(k).map_or(x, |p| x.clamp(p.min, p.max));
                self.channel(id)?.params.insert(k, x);
            }
            (b'm', Field::SendLevel(s), Value::Real(x)) => {
                let ch = self.channel(id)?;
                ch.sends.iter_mut().find(|x| x.id == s).ok_or(Error::NoSuchNode(s))?.level_db = x;
            }
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
            (b't', Field::Monitor) => Value::Bool(self.track(id)?.monitor),
            (b't', Field::Collapsed) => Value::Bool(self.track(id)?.collapsed),
            (b't', Field::ShowLanes) => Value::Bool(self.track(id)?.show_lanes),
            (b't', Field::ShowTakes) => Value::Bool(self.track(id)?.show_takes),
            (b't', Field::OutputTo(o)) => {
                Value::Id(self.track(id)?.outputs.iter().find(|x| x.id == o).ok_or(Error::NoSuchNode(o))?.to)
            }
            (b'm', Field::ChannelOutput) => Value::Id(self.channel(id)?.output.unwrap_or(ROOT)),
            (b'r', Field::LoopOn) => Value::Bool(self.doc.song.loop_range.on),
            (b'r', Field::PunchOn) => Value::Bool(self.doc.song.punch.on),
            (b'r', Field::Metronome) => Value::Bool(self.doc.song.metronome),
            (b'r', Field::Tempo) => Value::Real(self.doc.song.tempo[0].bpm),
            (b't' | b'p' | b'x', Field::Param(k)) => Value::Real(plugins::get_param(self.doc, id, k)?),
            (b'm', f @ (Field::InsertPoint(_) | Field::Bypass(_) | Field::FaderLink)) => mixer::get(self.doc, id, f)?,
            (b'r', Field::DeskAnalog) => Value::Bool(self.doc.song.desk_analog),
            (b'r', Field::CountIn) => Value::Real(self.doc.song.count_in_bars as f64),
            (b'r', Field::PreRoll) => Value::Real(self.doc.song.pre_roll_bars as f64),
            (b'm', Field::Volume) => Value::Real(self.channel(id)?.volume_db),
            (b'm', Field::Pan) => Value::Real(self.channel(id)?.pan),
            (b'm', Field::Mute) => Value::Bool(self.channel(id)?.mute),
            (b'm', Field::Solo) => Value::Bool(self.channel(id)?.solo),
            (b'm', Field::Param(k)) => Value::Real(*self.channel(id)?.params.get(k).ok_or(Error::WrongType)?),
            (b'm', Field::SendLevel(s)) => {
                let ch = self.channel(id)?;
                Value::Real(ch.sends.iter().find(|x| x.id == s).ok_or(Error::NoSuchNode(s))?.level_db)
            }
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
    /// opening or switching the audio device, calibrating latency (I-2 list)
    Device,
    /// an export, by its number (`project::export`)
    Export(u64),
    /// listing the audio devices and the MIDI ports (U17)
    Scan,
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

/// Meters by index: tracks 0..64, then the channels in the document's order (normal, returns, master)
pub const MAX_METERS: usize = 192;
pub const CHANNEL_METERS: usize = 64;

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
    /// loop: start, end (samples), on
    loop_start: AtomicI64,
    loop_end: AtomicI64,
    loop_on: AtomicBool,
    locate: AtomicI64, // i64::MIN = none
    quit: AtomicBool,
    slots: Slots,
}

fn engine_thread(
    shared: Arc<EngineShared>,
    mut input: triple_buffer::Input<TelemetryFrame>,
    sources: Vec<(Arc<Peaks>, i64)>,
    routes: mixer::Routes,
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
            let (ls, le) = (shared.loop_start.load(Ordering::Relaxed), shared.loop_end.load(Ordering::Relaxed));
            if shared.loop_on.load(Ordering::Relaxed) && le > ls && pos >= le {
                pos = ls + (pos - le) % (le - ls);
            }
        }
        last = now;
        let span = (SAMPLE_RATE / 60) as u64;
        let mut meters = [0f32; MAX_METERS];
        let n = sources.len().min(MAX_METERS / 2);
        for (i, (peaks, start)) in sources.iter().enumerate().take(n) {
            let a = if playing { peaks.amplitude_at(pos - start, span) } else { 0.0 };
            let a = a * (0.85 + 0.15 * noise.unit() as f32);
            let track = a * f32::from_bits(shared.slots.gain[i].load(Ordering::Relaxed));
            meters[i] = track;
            meters[CHANNEL_METERS + i] = track;
        }
        // through the desk: each channel's fader, then into where it sends and outputs (sources first)
        if let Ok(routes) = routes.try_lock() {
            let mut done = [false; MAX_METERS];
            for &(from, to, g) in routes.iter() {
                if !done[from] {
                    meters[from] = (meters[from] * f32::from_bits(shared.slots.gain[from].load(Ordering::Relaxed))).min(1.5);
                    done[from] = true;
                }
                meters[to] = meters[to].max(meters[from] * g);
            }
            for (k, d) in done.iter().enumerate().skip(CHANNEL_METERS) {
                if !d {
                    meters[k] = (meters[k] * f32::from_bits(shared.slots.gain[k].load(Ordering::Relaxed))).min(1.5);
                }
            }
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

struct RecState {
    /// where the take begins
    start: i64,
    /// where the pre-roll began (the handle before the take, U4)
    pre_roll_from: i64,
    tracks: Vec<usize>,
}

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
    routes: mixer::Routes,
    telemetry: triple_buffer::Output<TelemetryFrame>,
    last_frame: TelemetryFrame,
    engine_thread: Option<JoinHandle<()>>,
    config: Config,
    pumps: u64,
    rec: Option<RecState>,
    /// The project folder, preferences, devices, export, notices (owner: agent D, `project.rs`)
    pub services: project::Services,
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
        let (tracks, srcs) = arrange_tracks(&config, &resources);
        doc.tracks = tracks;
        sources.extend(srcs);
        // the desk: groups and normal channels, the returns, the master (mixer.rs)
        doc.channels = mixer::make_channels(&config, &doc.tracks);
        plugins::populate(&mut doc);
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
            loop_start: AtomicI64::new(doc.song.loop_range.start),
            loop_end: AtomicI64::new(doc.song.loop_range.end),
            loop_on: AtomicBool::new(doc.song.loop_range.on),
            locate: AtomicI64::new(i64::MIN),
            quit: AtomicBool::new(false),
            slots: Slots { gain: (0..MAX_METERS).map(|_| AtomicU32::new(1f32.to_bits())).collect() },
        });
        let e = engine.clone();
        let routes: mixer::Routes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let engine_routes = routes.clone();
        let engine_thread = std::thread::Builder::new()
            .name("fake-engine".into())
            .spawn(move || engine_thread(e, input, sources, engine_routes))
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
            routes,
            telemetry: output,
            last_frame: initial,
            engine_thread: Some(engine_thread),
            config,
            pumps: 0,
            rec: None,
            services: project::Services::default(),
            _main: Main { _not_send: PhantomData },
        };
        // the slow take: its peaks are "generated" again, with the injected delay
        let all = Diff {
            changed: s.doc.tracks.iter().map(|t| (t.id, Some(Field::Volume)))
                .chain(s.doc.channels.iter().map(|c| (c.id, Some(Field::Volume)))).collect(),
        };
        s.after_change(&all);
        *s.routes.lock().unwrap() = mixer::routes(&s.doc, |i| (CHANNEL_METERS + i).min(MAX_METERS - 1));
        s.pending = Diff::default();
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
                let items: Vec<NodeId> = self
                    .doc
                    .tracks
                    .iter()
                    .flat_map(|t| t.items.iter())
                    .filter(|i| i.resource() == Some(res))
                    .map(|i| i.id)
                    .collect();
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
        // the services' operations: their results are applied here, never by the workers (I-2.2)
        changed |= self.services.pump();
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
    /// The meter index of the channel at `i` in `channels()`
    pub fn channel_meter(&self, i: usize) -> usize {
        (CHANNEL_METERS + i).min(MAX_METERS - 1)
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

    /// Every long operation running past the 200 ms rule, oldest first (I-2.2): the activity list
    pub fn activity(&self) -> Vec<notices::Activity> {
        let mut v: Vec<notices::Activity> = self
            .ops
            .iter()
            .filter(|(_, o)| !o.finished.load(Ordering::Acquire) && o.started.elapsed() >= LOADING_THRESHOLD)
            .map(|(id, o)| notices::Activity {
                op: *id,
                stage: o.stage,
                target: o.target,
                progress: Some(o.progress.get()),
                cancellable: o.cancellable && o.started.elapsed() >= CANCEL_AFTER,
                since: o.started.elapsed(),
            })
            .collect();
        v.sort_by(|a, b| b.since.cmp(&a.since));
        v
    }

    pub fn loading(&self, target: &Target) -> Loading {
        let mut worst: Option<&OpInfo> = None;
        for op in self.ops.values() {
            let hit = op.target == *target
                || match (target, op.target) {
                    // aggregation (T10): an item is loading while its resource is
                    (Target::Node(n), Target::Resource(r)) => {
                        self.doc.tracks.iter().flat_map(|t| t.items.iter()).any(|i| i.id == *n && i.resource() == Some(r))
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
        // the fake engine's routes follow the desk (outputs, sends, channels added)
        if diff.changed.iter().any(|(id, f)| {
            id.kind == b'm' && matches!(f, None | Some(Field::ChannelOutput) | Some(Field::SendLevel(_)))
        }) {
            let r = mixer::routes(&self.doc, |i| (CHANNEL_METERS + i).min(MAX_METERS - 1));
            *self.routes.lock().unwrap() = r;
        }
        // parameter slots: the fake engine sees the new gains at once (I-6)
        for (id, f) in &diff.changed {
            if *f == Some(Field::Volume) || *f == Some(Field::Mute) {
                if let Some((i, t)) = self.doc.tracks.iter().enumerate().find(|(_, t)| t.id == *id) {
                    let g = if t.mute { 0.0 } else { db_gain(t.volume_db) };
                    self.engine.slots.gain[i].store(g.to_bits(), Ordering::Relaxed);
                }
                if let Some((i, c)) = self.doc.channels.iter().enumerate().find(|(_, c)| c.id == *id) {
                    let g = if c.mute { 0.0 } else { db_gain(c.volume_db) };
                    self.engine.slots.gain[CHANNEL_METERS + i].store(g.to_bits(), Ordering::Relaxed);
                }
            }
        }
        if diff.changed.iter().any(|(_, f)| *f == Some(Field::LoopOn)) {
            let l = self.doc.song.loop_range;
            self.engine.loop_start.store(l.start, Ordering::Relaxed);
            self.engine.loop_end.store(l.end, Ordering::Relaxed);
            self.engine.loop_on.store(l.on, Ordering::Relaxed);
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
            Target::Node(n) => {
                self.doc.tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.id == *n).and_then(|i| i.resource())
            }
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
        let Some(res) = t.main_resource() else { return };
        if self.pending_resources.iter().any(|(r, _)| *r == res) {
            return;
        }
        let delay = self.config.slow_delay;
        let seconds = self.config.item_seconds;
        let h = self.start("Generating peaks", Target::Resource(res), true, move |cancel, progress| {
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
        self.finish_recording();
    }
    /// Recording starts at the cursor, after the count-in (pre-roll bars played before it, U4): the cursor goes
    /// back that much and the take begins at the old position. Pressed again: the takes become items.
    pub fn toggle_record(&mut self) {
        if self.rec.is_some() {
            self.finish_recording();
            return;
        }
        let at = self.last_frame.position;
        let pre = self.doc.song.pre_roll_bars as i64 * arrange::BAR;
        let tracks: Vec<usize> = (0..self.doc.tracks.len()).filter(|&i| self.doc.tracks[i].armed).collect();
        self.rec = Some(RecState { start: at, pre_roll_from: (at - pre).max(0), tracks });
        self.engine.locate.store((at - pre).max(0), Ordering::Relaxed);
        self.engine.recording.store(true, Ordering::Relaxed);
        self.play();
    }
    /// The takes being recorded, for the waveform that grows (U3): (track index, start, recorded length, peaks).
    /// The peaks are the fake DiskWriter's, published as they are computed.
    pub fn recording_takes(&self) -> Vec<(usize, i64, i64, Arc<Peaks>)> {
        let Some(r) = &self.rec else { return Vec::new() };
        let len = (self.last_frame.position - r.start).max(0);
        r.tracks
            .iter()
            .filter_map(|&t| {
                let res = self.doc.tracks[t].main_resource().unwrap_or(ResourceId(0));
                self.resources.get(&res).map(|p| (t, r.start, len, p.clone()))
            })
            .collect()
    }
    /// The count-in is running: recording armed, cursor still before the take's start
    pub fn counting_in(&self) -> bool {
        self.rec.as_ref().is_some_and(|r| self.last_frame.position < r.start)
    }
    fn finish_recording(&mut self) {
        self.engine.recording.store(false, Ordering::Relaxed);
        let Some(r) = self.rec.take() else { return };
        let len = self.last_frame.position - r.start;
        if len <= 0 {
            return;
        }
        let handle = r.start - r.pre_roll_from;
        let mut next = self.next_id;
        let _ = self.transact("Record", |e| {
            for &t in &r.tracks {
                let res = e.doc.tracks[t].main_resource().unwrap_or(ResourceId(0));
                next += 1;
                let id = NodeId { kind: b'i', number: 100_000 + next };
                e.doc.tracks[t].items.push(Item {
                    id,
                    name: "Take".into(),
                    start: r.start,
                    length: len,
                    kind: ItemKind::Audio { resource: res, takes: Vec::new(), comp: Vec::new(), handle_before: handle, handle_after: 0 },
                });
                e.diff.changed.push((id, None));
                e.diff.changed.push((e.doc.tracks[t].id, None));
            }
            Ok(())
        });
        self.next_id = next + 1;
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

/// The fake arrangement: folders, a multi-output drum instrument, audio tracks with two items (one with three
/// takes and a comp, one with pre-roll and punch handles), instrument tracks with MIDI items and patterns, and
/// automation lanes. Returns the tracks and, per track, what the fake engine meters.
fn arrange_tracks(config: &Config, resources: &HashMap<ResourceId, Arc<Peaks>>) -> (Vec<Track>, Vec<(Arc<Peaks>, i64)>) {
    use arrange::*;
    let ch = |n: usize| NodeId { kind: b'm', number: (n % config.channels.max(1)) as u64 + 1 };
    let mut tracks = Vec::new();
    let mut sources = Vec::new();
    let mut folder: Option<(NodeId, usize)> = None;
    let mut sub = 20_000u64;
    let mut next = || {
        sub += 1;
        sub
    };
    let item_len = (config.item_seconds / 3.0 * SAMPLE_RATE as f64) as i64;
    for i in 0..config.tracks {
        let id = NodeId { kind: b't', number: i as u64 + 1 };
        let resource = ResourceId((i % 8) as u32);
        let start = ((i % 5) as i64) * BAR / 2;
        let kind = if i % 10 == 0 { TrackKind::Folder } else if i % 3 == 0 || i == 1 { TrackKind::Instrument } else { TrackKind::Audio };
        let parent = match folder {
            Some((f, left)) if left > 0 && kind != TrackKind::Folder => {
                folder = Some((f, left - 1));
                Some(f)
            }
            _ => None,
        };
        let name = match (i, kind) {
            (0, _) => "Bateria".to_string(),
            (1, _) => "Bateria AVL".to_string(),
            (2, _) => "Voz principal".to_string(),
            (_, TrackKind::Folder) => format!("Grupo {}", i / 10 + 1),
            _ => format!("{} {}", NAMES[i % 8], i / 8 + 1),
        };
        let mut outputs = vec![Output { id: NodeId { kind: b'o', number: next() }, port: "main", channels: 2, to: ch(i) }];
        if i == 1 {
            // the drum kit's ports, each to its channel (I-3.9)
            outputs = [("kick", 1u8), ("snare", 1), ("oh", 2), ("room", 2)]
                .iter()
                .enumerate()
                .map(|(k, (port, n))| Output { id: NodeId { kind: b'o', number: next() }, port, channels: *n, to: ch(1 + k) })
                .collect();
        }
        let mut items = Vec::new();
        match kind {
            TrackKind::Folder => {}
            TrackKind::Audio => {
                let (takes, comp) = if i == 2 {
                    let takes: Vec<Take> = [1u32, 4, 6]
                        .iter()
                        .map(|r| Take { id: NodeId { kind: b'x', number: next() }, resource: ResourceId(*r) })
                        .collect();
                    let comp = vec![
                        CompSegment { start: 0, take: takes[0].id, crossfade_ms: 5.0 },
                        CompSegment { start: 6 * BAR, take: takes[1].id, crossfade_ms: 5.0 },
                        CompSegment { start: 14 * BAR, take: takes[2].id, crossfade_ms: 12.0 },
                    ];
                    (takes, comp)
                } else {
                    (Vec::new(), Vec::new())
                };
                items.push(Item {
                    id: NodeId { kind: b'i', number: 1000 + i as u64 },
                    name: if i == 2 { "Voz (3 takes)".into() } else { format!("Take {}", i) },
                    start,
                    length: item_len,
                    kind: ItemKind::Audio {
                        resource,
                        takes,
                        comp,
                        handle_before: if i % 4 == 2 { BAR } else { 0 },
                        handle_after: if i % 4 == 2 { BAR / 2 } else { 0 },
                    },
                });
                items.push(Item {
                    id: NodeId { kind: b'i', number: 3000 + i as u64 },
                    name: format!("Take {}b", i),
                    start: start + item_len + 2 * BAR,
                    length: item_len,
                    kind: ItemKind::Audio { resource, takes: Vec::new(), comp: Vec::new(), handle_before: 0, handle_after: 0 },
                });
            }
            TrackKind::Instrument => {
                for k in 0..3 {
                    let len = 4 * BAR;
                    items.push(Item {
                        id: NodeId { kind: b'i', number: 5000 + (i * 3 + k) as u64 },
                        name: if k == 0 { "Padrão A".into() } else { format!("Frase {}", k) },
                        start: start + k as i64 * 12 * BAR,
                        length: len,
                        kind: ItemKind::Midi { notes: fake_notes(i as u64 + k as u64, len), repeats: if k == 0 { 2 } else { 0 } },
                    });
                }
            }
        }
        let lanes = if i == 2 || i == 4 {
            vec![Lane {
                id: NodeId { kind: b'a', number: next() },
                param: if i == 2 { "Volume" } else { "Filter cutoff" },
                min: 0.0,
                max: 1.0,
                points: (0..24).map(|k| (k as i64 * BAR, 0.5 + 0.4 * ((k as f64) * 0.7).sin(), if k % 3 == 0 { 0.6 } else { 0.0 })).collect(),
            }]
        } else {
            Vec::new()
        };
        if kind == TrackKind::Folder {
            folder = Some((id, 3));
        }
        let meter_res = items.iter().find_map(|it| it.resource()).unwrap_or(resource);
        sources.push((resources[&meter_res].clone(), if kind == TrackKind::Folder { i64::MAX / 2 } else { start }));
        tracks.push(Track {
            id,
            name,
            color: view::TRACK_COLORS[i % 8],
            kind,
            parent,
            collapsed: false,
            volume_db: 0.0,
            mute: false,
            solo: false,
            armed: i == 2,
            monitor: i == 2,
            input: if kind == TrackKind::Audio { format!("Entrada {} (mono)", i % 8 + 1) } else { "MIDI: todas".into() },
            outputs,
            items,
            lanes,
            show_lanes: i == 2,
            show_takes: i == 2,
        });
    }
    (tracks, sources)
}

pub fn db_gain(db: f64) -> f32 {
    if db.is_infinite() || db <= -60.0 { 0.0 } else { 10f64.powf(db / 20.0) as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> Config {
        Config { tracks: 4, channels: 4, slow_delay: Duration::from_millis(400), slow_track: 2, item_seconds: 5.0 }
    }

    #[test]
    fn loading_follows_the_200_ms_rule() {
        let mut s = Session::create(small());
        let t = &s.tracks()[2]; // an audio track: track 0 is a folder, 1 and 3 instruments
        let (item, res) = (t.items[0].id, t.items[0].resource().unwrap());
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
