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
//!   `set_favorite`), because in the client the files are the server's (module `browse`);
//! - `plugins` and `plugin_schema`: the plugins the core has and their schemas, which the generic plugin editor
//!   builds its controls from;
//! - MIDI mapping (I-21.2): `midi_learn`, `cancel_learn`, `learning`, `unmap`, the `MidiUiAction` feed
//!   (`drain_midi_ui`) and the raw MIDI activity (`midi_activity`); the mappings' types come from `lm-midimap`;
//!   `TelemetryView::hand_won`, the automation lanes the hand overrode (I-21.5);
//! - the interface document (I-3.8): `ui` and `set_ui`, no undo, not journaled; the GUI writes the selection there
//!   (`selection: { track, channel, plugin }`), which the selection-following MIDI targets read (I-21.2).

#![forbid(unsafe_code)]

pub mod browse;
pub mod tempo;
pub mod trees;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub use lm_base::{Arg, Error, HostTime, Label, OperationId, ParamRange, RangeScale, Result, intern};
pub use lm_doc::{Diff, Feed, GestureId, Topic};
pub use lm_render::{METER_CHANNELS, Meter};
pub use lm_state::{FieldKind, FieldSpec, NodeId, NodeSchema, ParamSpec, PathStep, ROOT, Resource, Scale, Value};

pub use lm_time::TempoMap;
pub use tempo::{TempoCache, tempo_map, tempo_map_or_default};
pub use browse::{BrowseEntry, BrowsePage, EntryId, EntryKind, PluginInfo, Preview};
pub use lm_midimap::{DocSource, MapScope, MapTarget, Mapping, Message as MidiMessage, NodeRef, SelRole};

/// A MIDI learn under way (I-21.2): its number, to cancel it
pub type LearnId = u64;

/// A mapped MIDI control reached an action of the interface (zoom, scroll, page...): the session sends it on, the
/// interface runs it (I-21.2, the `MidiUiAction` feed). `value` is the control's, normalized 0..1
#[derive(Clone, Debug, PartialEq)]
pub struct MidiUiAction {
    pub action: ActionId,
    pub params: Value,
    pub value: f32,
}

/// One raw MIDI message as it arrived (the activity monitor and learning, I-21.2): lossy, the last 256 kept
#[derive(Clone, Debug, PartialEq)]
pub struct MidiActivity {
    /// The port's alias, or its name when it has none
    pub port: Arc<str>,
    pub channel: u8,
    pub message: MidiMessage,
    /// 7 or 14 bits
    pub value: u16,
    /// whether a mapping took it
    pub mapped: bool,
    pub time: HostTime,
}

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

/// A result that arrived for a request that answered `Pending::Waiting` (collected with `CorePort::finished`)
#[derive(Debug)]
pub enum Finished {
    Applied(Result<Applied>),
    /// `run_action`, `undo`, `redo`, `set_preference`, `set_favorite`
    Done(Result<()>),
    Peaks(Result<PeakBlock>),
    Devices(Result<Vec<DeviceInfo>>),
    /// `browse`, `search`
    Entries(Result<Vec<BrowseEntry>>),
    Preview(Result<Preview>),
    /// `import`: the resource the file became in the project's store
    Imported(Result<Resource>),
    /// `midi_learn`: the learn's number (addition of 20/11, I-21.2)
    Learned(Result<LearnId>),
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
    /// The audio's load: render time over block time of the slowest block since the last frame (0 to 1 and beyond;
    /// F-JAN-050)
    pub load: f32,
    /// The automation lanes the hand overrode in Read or Touch (I-21.5): the lane's owner node and the parameter's
    /// key; shown as "the hand won" until re-armed (the action `automation.rearm`)
    pub hand_won: Vec<(NodeId, Arc<str>)>,
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

    /// The results of the requests that answered `Waiting`, collected each pump (the LocalPort answers everything
    /// `Ready`, so it has none)
    fn finished(&mut self) -> Vec<(RequestId, Finished)> {
        Vec::new()
    }

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
    /// A live note on the instrument of `track` (the test keyboard, the computer keyboard as a piano, the piano roll's
    /// keys and a note being drawn): a MIDI command to the audio thread, like live MIDI input. Data only, never
    /// waits, no answer; `key` 0..=127, `velocity` 1..=127. The interface releases every note it started (mouse-up,
    /// focus loss, window closed)
    fn note_on(&mut self, track: NodeId, key: u8, velocity: u8);
    /// Ends the live note `key` of `track`
    fn note_off(&mut self, track: NodeId, key: u8);
    /// Ends every live note of `track` (all notes off)
    fn all_notes_off(&mut self, track: NodeId);

    // --- long operations, notices, telemetry, heavy data
    fn loading(&self, target: &Target) -> Loading;
    fn activity(&self) -> Vec<OperationInfo>;
    fn cancel(&mut self, op: OperationId);
    fn notices(&mut self) -> Vec<Notice>;
    fn dismiss_notice(&mut self, id: u64);
    fn telemetry(&self) -> &TelemetryView;
    fn peaks(&mut self, resource: &Resource, level: u8, from: u64, to: u64) -> Pending<PeakBlock>;
    /// The project's tempo map (ticks <-> samples, bars and beats): the core's own, the one the render and the edits
    /// use, shared by revision: the interface asks it, it never rebuilds one from the tree. The provided method builds
    /// it again on every call; the ports keep it in a [`TempoCache`]
    fn tempo_map(&self) -> Arc<TempoMap> {
        Arc::new(tempo_map_or_default(self.root()))
    }
    /// A plugin's live data (F2)
    fn display(&self, node: NodeId) -> Option<DisplayView>;
    /// A parameter's value law (its range and curve: linear, log, stepped, gain), the one the core uses for mapped
    /// MIDI, automation and its edits; the interface draws and edits with it, never with a law of its own. `None`:
    /// no such parameter (or a port that cannot say yet)
    fn param_range(&self, node: NodeId, key: &str) -> Option<ParamRange> {
        let _ = (node, key);
        None
    }

    // --- the session's side that is not the project document (additions of 20/11)
    /// The preferences tree (I-16), and changing one by its path
    fn preferences(&self) -> &Value;
    fn set_preference(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()>;
    /// The folder of the user's own data beside the preferences (the keymap presets live in `keymaps/` under it);
    /// `None` where there is none (a core that keeps nothing on disk, a remote one)
    fn config_dir(&self) -> Option<std::path::PathBuf> {
        None
    }
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

    /// Imports an audio file into the project's resources (I-12; an operation, its loading on
    /// `Target::Scan(Library)`). In-process the path is the core's; over the network the RemotePort uploads the
    /// client's file first (chunked, resumable, hash-checked, I-20.3) and the server imports the upload
    fn import(&mut self, path: std::path::PathBuf) -> Pending<Resource> {
        let _ = path;
        Pending::Ready(Err(Error::new("port.import.unsupported")))
    }

    // --- the plugins (addition of 20/11): what the generic editor, "Swap" and "add effect" read
    /// The plugins the core has. The default is for the transition only: every real port answers (the fake from
    /// its registry, the LocalPort from `internal_plugins()`, the RemotePort from what it fetched at connect)
    fn plugins(&self) -> Vec<PluginInfo> {
        Vec::new()
    }
    /// A plugin's main schema (`plugin:<id>`): its parameters (range, scale, unit, automatable) and its fields
    /// (choices by key, U30). The default is for the transition only, as for `plugins`
    fn plugin_schema(&self, _plugin: &str) -> Option<NodeSchema> {
        None
    }

    // --- the interface document (I-3.8, addition of 20/11): windows, zoom, scroll, selection; no undo, out of the
    // journal, saved in interface.json. The defaults are for the transition only
    /// The interface document's root
    fn ui(&self) -> &Value {
        static NONE: Value = Value::NULL;
        &NONE
    }
    /// Sets a key path of the interface document (made when absent)
    fn set_ui(&mut self, path: Vec<PathStep>, value: Value) -> Pending<()> {
        let _ = (path, value);
        Pending::Ready(Err(Error::new("port.ui.unsupported")))
    }

    // --- MIDI mapping (I-21.2, addition of 20/11). The defaults are for the transition only: every real port
    // answers (the fake in its tests, the LocalPort over the session's mapper, the RemotePort through the server)
    /// Starts learning a mapping for `target`: the first control moved becomes its source (14 bits, NRPN and relative
    /// encoders detected in the next 300 ms); the mapping is made in a transaction labelled "MIDI: aprender", with a
    /// notice. A loop (the target controls its own source) is refused with a notice
    fn midi_learn(&mut self, target: MapTarget, scope: MapScope) -> Pending<LearnId> {
        let _ = (target, scope);
        Pending::Ready(Err(Error::new("port.midi.unsupported")))
    }
    fn cancel_learn(&mut self, id: LearnId) {
        let _ = id;
    }
    /// The learn under way, if any: the interface says "move a control"
    fn learning(&self) -> Option<LearnId> {
        None
    }
    /// Removes every mapping on `target` in `scope` ("Tirar o mapa")
    fn unmap(&mut self, target: &MapTarget, scope: MapScope) -> Pending<()> {
        let _ = (target, scope);
        Pending::Ready(Err(Error::new("port.midi.unsupported")))
    }
    /// The interface's actions mapped controls reached since the last call
    fn drain_midi_ui(&mut self, out: &mut Vec<MidiUiAction>) {
        let _ = out;
    }
    /// The raw MIDI activity after `since` (a counter): the new counter and the messages, at most the last 256
    fn midi_activity(&self, since: u64) -> (u64, Vec<MidiActivity>) {
        (since, Vec::new())
    }
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
            load: 0.0,
            hand_won: Vec::new(),
        };
        assert_eq!(t.peak(ROOT), 0.5);
        assert_eq!(t.peak(NodeId { kind: b't', number: 9 }), 0.0);
    }
}
