//! The `CorePort` (interfaces.md, I-15): everything the graphical interface may do with the core. Arguments and
//! results are DATA — no closures, no references into the core — so the same interface works with the core in the
//! same process (`LocalPort`, in `lm-session`) and over the network (`RemotePort`, which keeps a mirror of the tree).
//!
//! What this crate holds: the trait and the types it names. Types that already exist in the core are re-exported,
//! not copied: `Label`, `Error`, `Result`, `OperationId` and `HostTime` (lm-base), `NodeId`, `Value`, `PathStep` and
//! `Resource` (lm-state), `Diff`, `Feed`, `GestureId` and `Topic` (lm-doc), `Meter` (lm-render). No GPUI, no
//! session details: `lm-session` implements the port later (step 9); `lm-port-fake` implements it for the
//! interface's tests.
//!
//! Additions to the I-15 text, recorded there and in decisoes-autonomas.md (20/11):
//! - `drain(&self, &mut Feed)`: a feed drains against the document, which the interface never holds;
//! - `preferences`/`set_preference` and `status` (the session's side that is not the project: the project's file
//!   and access, recent projects, interrupted takes, safe mode, the last export), and `devices`;
//! - the browser service (`browse`, `search`, `prepare_preview`, `preview_play`, `preview_stop`,
//!   `set_favorite`), because in the client the files are the server's (module `browse`).

#![forbid(unsafe_code)]

pub mod browse;
pub mod trees;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub use lm_base::{Error, HostTime, Label, OperationId, Result};
pub use lm_doc::{Diff, Feed, GestureId, Topic};
pub use lm_render::{METER_CHANNELS, Meter};
pub use lm_state::{NodeId, PathStep, ROOT, Resource, Value};

pub use browse::{BrowseEntry, BrowsePage, EntryId, EntryKind, Preview};

/// The number of a request whose answer has not arrived (the RemotePort's; the LocalPort never waits)
pub type RequestId = u64;

/// A result that may not be there yet: in-process it is ready at once; over the network it arrives in a later pump.
/// The interface must handle `Waiting` everywhere (it is the normal case in the client)
#[derive(Debug)]
#[must_use = "a Pending may be Waiting: look at it"]
pub enum Pending<T> {
    Ready(Result<T>),
    Waiting(RequestId),
}

impl<T> Pending<T> {
    pub fn is_waiting(&self) -> bool {
        matches!(self, Pending::Waiting(_))
    }

    /// The value, if ready and good
    pub fn ok(self) -> Option<T> {
        match self {
            Pending::Ready(Ok(v)) => Some(v),
            _ => None,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Pending<U> {
        match self {
            Pending::Ready(r) => Pending::Ready(r.map(f)),
            Pending::Waiting(id) => Pending::Waiting(id),
        }
    }
}

/// An action of the registry (I-19), by id: `"transport.play"`, `"notes.quantize"`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActionId(pub Arc<str>);

impl From<&str> for ActionId {
    fn from(s: &str) -> Self {
        ActionId(Arc::from(s))
    }
}

impl ActionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A node named by its id, or by the number given to a node created earlier in the same list of edits
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ref {
    Id(NodeId),
    Temp(u32),
}

/// A note named by its number in the item, or by the number given to a note added earlier in the same list
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoteRef {
    N(u64),
    Temp(u32),
}

/// One edit as data (the port's and the protocol's form); a list of them is one transaction, all or nothing
#[derive(Clone, Debug, PartialEq)]
pub enum EditOp {
    Set { id: Ref, path: Vec<PathStep>, value: Value },
    Insert { temp: u32, parent: Ref, list: String, index: usize, node: Value },
    Remove { id: Ref },
    Move { id: Ref, parent: Ref, list: String, index: usize },
    AddNote { temp: u32, item: Ref, note: Value },
    SetNote { item: Ref, n: NoteRef, path: Vec<PathStep>, value: Value },
    RemoveNote { item: Ref, n: NoteRef },
}

impl EditOp {
    /// The common case: a field of a node, by its key path (`"volume"`, `"rack.eq.hi"` as two steps...)
    pub fn set(id: NodeId, path: &[&str], value: Value) -> EditOp {
        EditOp::Set { id: Ref::Id(id), path: path.iter().map(|k| PathStep::key(k)).collect(), value }
    }
}

/// What an `apply` did: the diff, and the ids and note numbers given to the nodes and notes it created, by their
/// temporary numbers
#[derive(Debug, Default)]
pub struct Applied {
    pub diff: Option<Diff>,
    pub created: Vec<(u32, NodeId)>,
    pub notes: Vec<(u32, u64)>,
}

/// What a long operation's loading shows on (I-2.2)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Project,
    Node(NodeId),
    Resource(Resource),
    Device,
    Export(u64),
    Scan(ScanKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScanKind {
    MidiPorts,
    Plugins,
    Library,
}

/// The interface's view of one target (I-2.2)
#[derive(Clone, Debug, PartialEq)]
pub enum Loading {
    Ready,
    /// Started less than 200 ms ago: draw nothing different (no flicker on fast operations)
    Pending,
    /// Past 200 ms: draw the loading in place of the object
    Visible {
        progress: Option<f32>,
        stage: Option<Label>,
        cancellable: bool,
        since: Duration,
    },
    Failed(Error),
    /// A resource the node needs is not where the project says (H17): an action "locate file", not a loading
    Missing {
        resource: Resource,
    },
}

pub const LOADING_THRESHOLD: Duration = Duration::from_millis(200);
pub const CANCEL_AFTER: Duration = Duration::from_secs(2);

/// One running long operation, as the activity list shows it
#[derive(Clone, Debug, PartialEq)]
pub struct OperationInfo {
    pub id: OperationId,
    pub label: Label,
    pub target: Target,
    pub progress: Option<f32>,
    pub stage: Option<Label>,
    pub cancellable: bool,
    pub since: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A notice (I-14): the core never writes finished text; the interface translates `key` with `args`
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub key: &'static str,
    pub args: Vec<(&'static str, Value)>,
    pub severity: Severity,
    pub target: Option<Target>,
    /// What it offers: "try again", "recreate the plugin"... with the action's parameter
    pub actions: Vec<(ActionId, Value)>,
    /// The same notice again is counted, not stacked
    pub repeats: u32,
    pub read: bool,
}

/// The transport's position, with the host time of the telemetry frame it came in: the interface interpolates
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub samples: i64,
    pub host_time: HostTime,
    pub sample_rate: u32,
    pub playing: bool,
    pub recording: bool,
}

impl Position {
    /// The position at `now` (host time), moving if playing
    pub fn at(&self, now: HostTime) -> f64 {
        if self.playing {
            let ns = now.since(self.host_time);
            self.samples as f64 + ns as f64 * 1e-9 * self.sample_rate as f64
        } else {
            self.samples as f64
        }
    }
}

/// A take being recorded, for the waveform that grows (U3): its peaks come by `peaks` on `resource`
#[derive(Clone, Debug, PartialEq)]
pub struct RecordingTake {
    pub track: NodeId,
    pub resource: Resource,
    pub start: i64,
    pub length: i64,
}

/// The continuous values (I-8, H16), by node: what the interface draws each frame
#[derive(Clone, Debug)]
pub struct TelemetryView {
    pub position: Position,
    /// The meters of the track outputs and the desk's channels and master, by node (M9)
    pub meters: HashMap<NodeId, Meter>,
    /// The inputs' levels (linear peak), shown in an armed track's header even without monitoring (F-REC-009)
    pub inputs: Vec<f32>,
    pub recording: Vec<RecordingTake>,
    /// The count-in is running: recording armed, the cursor still before the take's start
    pub counting_in: bool,
    /// Xruns so far, always shown (P-14), and where they happened (F-CLP-063)
    pub xruns: u32,
    pub xrun_marks: Vec<i64>,
}

impl TelemetryView {
    /// A node's meter as one linear peak (the loudest channel); 0 when there is none
    pub fn peak(&self, node: NodeId) -> f32 {
        self.meters.get(&node).map_or(0.0, |m| {
            m.peak[..(m.channels as usize).clamp(1, METER_CHANNELS)].iter().fold(0.0, |a, b| a.max(*b))
        })
    }
}

/// A piece of a resource's peak pyramid (I-20.4): `pairs` are the (min, max) of `level`, from pair `from` on.
/// Immutable for a resource's hash: the interface keeps what it got
#[derive(Clone, Debug, PartialEq)]
pub struct PeakBlock {
    pub level: u8,
    pub from: u64,
    pub samples_per_pair: u64,
    pub pairs: Arc<[(i16, i16)]>,
    /// The resource's length in samples
    pub length: u64,
}

/// A plugin's live data for its window (I-9.5, F2): the newest frame only
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayView {
    pub kind: Arc<str>,
    pub frame: Arc<[f32]>,
}

/// An audio or MIDI device, as the settings list it (I-10)
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub id: Arc<str>,
    pub name: Arc<str>,
    pub inputs: u32,
    pub outputs: u32,
    pub rates: Vec<u32>,
    /// Anything else the backend says (block sizes, latency, the busy holder...), as data
    pub details: Value,
}

/// Everything the GUI may do with the core (I-15)
pub trait CorePort {
    /// On the 10 ms timer of the interface's event loop (H2), and each frame. True when something shown may have
    /// changed
    fn pump(&mut self) -> bool;

    // --- reading: the port's mirror of the tree (in-process: the document's own root)
    fn root(&self) -> &Value;
    fn find(&self, id: NodeId) -> Option<Value>;
    fn feed(&mut self, topic: Topic) -> Feed;
    /// The diffs since the last drain, merged (I-3.6)
    fn drain(&self, feed: &mut Feed) -> Option<Diff>;
    /// Whether the project changed since it was last saved (asking before losing changes)
    fn modified(&self) -> bool;
    fn can_undo(&self) -> bool;
    fn can_redo(&self) -> bool;

    // --- changing: edits as data, actions by id, gestures by id
    fn apply(&mut self, label: Label, ops: Vec<EditOp>) -> Pending<Applied>;
    fn run_action(&mut self, action: ActionId, params: Value) -> Pending<()>;
    fn undo(&mut self) -> Pending<()>;
    fn redo(&mut self) -> Pending<()>;
    fn begin_gesture(&mut self, label: Label) -> GestureId;
    fn gesture_apply(&mut self, g: &GestureId, ops: Vec<EditOp>);
    fn gesture_commit(&mut self, g: GestureId);
    fn gesture_cancel(&mut self, g: GestureId);
    /// Live, while dragging, without an edit (the value reaches the audio's slot at once)
    fn param_write(&mut self, node: NodeId, param: &str, value: f32);

    // --- long operations, notices, telemetry, heavy data
    fn loading(&self, target: &Target) -> Loading;
    fn activity(&self) -> Vec<OperationInfo>;
    fn cancel(&mut self, op: OperationId);
    fn notices(&mut self) -> Vec<Notice>;
    fn dismiss_notice(&mut self, id: u64);
    fn telemetry(&self) -> &TelemetryView;
    fn peaks(&mut self, resource: &Resource, level: u8, from: u64, to: u64) -> Pending<PeakBlock>;
    /// A plugin's live data (F2)
    fn display(&self, node: NodeId) -> Option<DisplayView>;

    // --- the session's side that is not the project document (additions of 20/11)
    /// The preferences tree (I-16), and changing one by its path
    fn preferences(&self) -> &Value;
    fn set_preference(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()>;
    /// The project's file and access (H10), recent projects, templates, interrupted takes (U5), safe mode (U21),
    /// the last export's report: as a tree whose keys the screens read
    fn status(&self) -> &Value;
    fn devices(&mut self) -> Pending<Vec<DeviceInfo>>;

    // --- the browser service (addition of 20/11): the files are the core's (the server's, in the client)
    fn browse(&mut self, page: BrowsePage, parent: Option<EntryId>, show_hidden: bool) -> Pending<Vec<BrowseEntry>>;
    /// A search in the background: a new one replaces the last; its loading shows on `Target::Scan(Library)`
    fn search(&mut self, query: &str) -> Pending<Vec<BrowseEntry>>;
    /// Prepares an entry's preview (I-7.3): its waveform comes by `peaks` on the preview's resource
    fn prepare_preview(&mut self, entry: EntryId) -> Pending<Preview>;
    fn preview_play(&mut self, entry: EntryId, from: i64);
    fn preview_stop(&mut self);
    fn set_favorite(&mut self, entry: EntryId, on: bool) -> Pending<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_builds_a_key_path() {
        let op = EditOp::set(ROOT, &["rack", "eq", "hi"], Value::bool(true));
        let EditOp::Set { path, .. } = op else { panic!() };
        assert_eq!(path, vec![PathStep::key("rack"), PathStep::key("eq"), PathStep::key("hi")]);
    }

    #[test]
    fn pending_maps_and_waits() {
        let p: Pending<i32> = Pending::Ready(Ok(2));
        assert_eq!(p.map(|v| v * 2).ok(), Some(4));
        let w: Pending<i32> = Pending::Waiting(7);
        assert!(w.is_waiting());
        assert!(w.ok().is_none());
    }

    #[test]
    fn position_interpolates_while_playing() {
        let p =
            Position { samples: 1000, host_time: HostTime(0), sample_rate: 48_000, playing: true, recording: false };
        assert_eq!(p.at(HostTime(500_000_000)), 1000.0 + 24_000.0);
        let stopped = Position { playing: false, ..p };
        assert_eq!(stopped.at(HostTime(500_000_000)), 1000.0);
    }

    #[test]
    fn telemetry_peak_is_the_loudest_channel() {
        let mut meters = HashMap::new();
        let mut peak = [0.0; METER_CHANNELS];
        peak[0] = 0.25;
        peak[1] = 0.5;
        let m = Meter { channels: 2, peak, ..Meter::default() };
        meters.insert(ROOT, m);
        let t = TelemetryView {
            position: Position {
                samples: 0,
                host_time: HostTime(0),
                sample_rate: 48_000,
                playing: false,
                recording: false,
            },
            meters,
            inputs: Vec::new(),
            recording: Vec::new(),
            counting_in: false,
            xruns: 0,
            xrun_marks: Vec::new(),
        };
        assert_eq!(t.peak(ROOT), 0.5);
        assert_eq!(t.peak(NodeId { kind: b't', number: 9 }), 0.0);
    }
}
