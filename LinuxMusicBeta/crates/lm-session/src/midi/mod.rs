//! MIDI and automation in the session (interfaces.md, I-21): the map compiled into the engine (and recompiled
//! alone when the selection or a bank moves), the fields and actions a control drives, the end of a MIDI gesture as
//! one undo step, learning, feedback, controller profiles and the Mackie Control translator (`mcu`), MIDI recording
//! (`record`), automation recording (`automation`) and the actions of I-21 (`actions`).
//!
//! A child module of `session`, so it works on the session's own fields. The devices (`lm-io`'s `MidiDevices`)
//! call in through [`Session::midi_input`] (the raw activity: learn, the monitor) and [`Session::midi_ports`];
//! the feedback leaves through [`Session::take_midi_feedback`] and [`Session::take_midi_sysex`].

pub(crate) mod actions;
pub(crate) mod audition;
pub(crate) mod automation;
pub(crate) mod devices;
pub(crate) mod mcu;
pub(crate) mod mpe;
pub(crate) mod mpe_record;
pub(crate) mod record;
pub(crate) mod step;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use lm_base::ring::{Consumer, Producer, ring};
use lm_base::{Error, HostTime, Label, Result};
use lm_compile::{CompiledMap, MapInput, PortAlias, Resolved, Selection, compile_map};
use lm_doc::Origin;
use lm_midimap::{Detector, Feedback, Limiter, MapScope, MapTarget, Mapping, Message, Mode, Raw, Ring, Transform};
use lm_port::{MidiActivity, MidiUiAction, Severity};
use lm_render::{Install, MapEvent, MidiOutEvent, ParamRange, ParamSlot, PortIndex, RangeScale};
use lm_state::{FieldKind, NodeId, PathStep, ROOT, Value};

use super::Session;

/// The ids of learning requests
pub type LearnId = u64;

/// The feedback ring to the devices: 10 s of feedback at the rate limit for 256 controls (C4: one value per control
/// every 10 ms), lossy (a lost feedback value is sent again with the next change, and on every resend)
const FEEDBACK_RING: usize = 256 * 100 * 10;
/// The activity monitor keeps this many messages
const ACTIVITY_KEEP: usize = 256;
const MS: u64 = 1_000_000;

/// A learning request (I-21.2): the first movement sets the source, then 300 ms to tell 14 bits, NRPN, relative
#[derive(Debug)]
struct Learning {
    id: LearnId,
    target: MapTarget,
    scope: MapScope,
    detector: Detector,
}

/// A MIDI gesture running on a parameter (its end writes the document)
#[derive(Debug)]
struct Gesture {
    node: NodeId,
    key: Arc<str>,
    /// The document's gesture (I-3.7, T4: a controller's gesture beside the mouse's): the tree follows the control
    /// at each pump, so a program compiled meanwhile (a rebuild, a new node) starts from the moved value (S20), and
    /// the commit is the one undo step. `None`: a parameter the document does not keep
    doc: Option<lm_doc::GestureId>,
    /// The last value (the parameter's unit), and whether the tree has it yet
    value: f32,
    dirty: bool,
}

/// One feedback destination
#[derive(Debug)]
struct Fb {
    port: PortIndex,
    spec: Feedback,
    transform: Transform,
    limiter: Limiter,
}

/// The session's MIDI state
pub(crate) struct MidiCore {
    /// The project's mappings, then the machine's and the profiles' (from `project_count` on)
    mappings: Vec<Mapping>,
    project_count: usize,
    resolved: Vec<Resolved>,
    by_id: HashMap<u64, usize>,
    pub(crate) selection: Selection,
    /// The port of each alias, kept for the whole session (a port that comes back keeps its index)
    aliases: BTreeMap<Arc<str>, (PortIndex, bool)>,
    connected: BTreeSet<u16>,
    map_dirty: bool,
    resend: bool,
    gestures: BTreeMap<u32, Gesture>,
    /// The last field/action event of each mapping in this pump (a knob on a field makes one transaction per pump)
    pending: Vec<MapEvent>,
    ui: Vec<MidiUiAction>,
    activity: VecDeque<(u64, MidiActivity)>,
    activity_seq: u64,
    learning: Option<Learning>,
    learn_serial: u64,
    feedback: HashMap<u64, Fb>,
    out: Producer<MidiOutEvent>,
    out_consumer: Option<Consumer<MidiOutEvent>>,
    pub(crate) sysex: Vec<(PortIndex, Vec<u8>)>,
    pub(crate) mcu: Vec<mcu::Mcu>,
    pub(crate) recorder: record::Recorder,
    pub(crate) automation: automation::Recorder,
    pub(crate) hand_won: BTreeSet<(NodeId, Arc<str>)>,
    /// The banks of the control surfaces (tracks, channels)
    pub(crate) bank: (u32, u32),
    /// Step recording's cursor (F-PAS-001)
    pub(crate) step: Option<step::Step>,
    /// The machine's MIDI devices (`lm_io::MidiDevices`, I-21.1)
    pub(crate) devices: devices::Devices,
    /// Ports connected at some point in this session (a later connection is a reconnection)
    ever_connected: BTreeSet<u16>,
    /// The machine's ports as last compiled
    ports_seen: Value,
    /// The program each track's MIDI output was sent, by track: (ports, channel, program)
    programs_sent: HashMap<NodeId, (Vec<PortIndex>, u8, u8)>,
    /// "Test the control": the actions a control would run are listed instead of run
    dry_run: Option<Vec<Arc<str>>>,
    /// An editor playing one item alone (F-NOT-017, see `audition.rs`)
    pub(crate) audition: Option<lm_compile::Audition>,
    /// The sample the transport starts from once the program of the audition is installed
    pub(crate) audition_start: Option<i64>,
}

impl std::fmt::Debug for MidiCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiCore").field("mappings", &self.mappings.len()).finish_non_exhaustive()
    }
}

impl MidiCore {
    pub(crate) fn new() -> MidiCore {
        let (out, consumer) = ring(FEEDBACK_RING);
        MidiCore {
            audition: None,
            audition_start: None,
            mappings: Vec::new(),
            project_count: 0,
            resolved: Vec::new(),
            by_id: HashMap::new(),
            selection: Selection::default(),
            aliases: BTreeMap::new(),
            connected: BTreeSet::new(),
            map_dirty: true,
            resend: true,
            gestures: BTreeMap::new(),
            pending: Vec::new(),
            ui: Vec::new(),
            activity: VecDeque::new(),
            activity_seq: 0,
            learning: None,
            learn_serial: 0,
            feedback: HashMap::new(),
            out,
            out_consumer: Some(consumer),
            sysex: Vec::new(),
            mcu: Vec::new(),
            recorder: record::Recorder::default(),
            automation: automation::Recorder::default(),
            hand_won: BTreeSet::new(),
            bank: (0, 0),
            step: None,
            devices: devices::Devices::default(),
            ever_connected: BTreeSet::new(),
            ports_seen: Value::NULL,
            programs_sent: HashMap::new(),
            dry_run: None,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.map_dirty = true;
    }

    pub(crate) fn connected_ports(&self) -> Vec<PortIndex> {
        self.connected.iter().map(|p| PortIndex(*p)).collect()
    }

    /// Whether a port was connected before in this session
    pub(crate) fn was_connected(&self, port: PortIndex) -> bool {
        self.ever_connected.contains(&port.0)
    }

    fn mapping_of(&self, id: u64) -> Option<(&Mapping, &Resolved)> {
        let r = &self.resolved[*self.by_id.get(&id)?];
        Some((&self.mappings[r.mapping], r))
    }

    fn port_alias(&self, port: PortIndex) -> Arc<str> {
        self.aliases
            .iter()
            .find(|(_, (p, _))| *p == port)
            .map_or_else(|| Arc::from(format!("{}", port.0)), |(a, _)| a.clone())
    }
}

/// The mappings of a `midiMap` list (project or preferences); unreadable ones are skipped with a notice key
fn read_mappings(list: Option<&Value>, bad: &mut usize) -> Vec<Mapping> {
    let mut out = Vec::new();
    for v in list.map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        // An OSC alias is the OSC server's, not an unreadable MIDI mapping
        if lm_midimap::doc::osc_address(&v).is_some() {
            continue;
        }
        match Mapping::from_value(&v) {
            Ok(m) => out.push(m),
            Err(_) => *bad += 1,
        }
    }
    out
}

/// The interface document's selection (I-3.8): `selection: { track, channel, plugin }`
pub(crate) fn read_selection(ui: &Value) -> (Option<NodeId>, Option<NodeId>, Option<NodeId>) {
    let s = ui.get("selection");
    let id = |k: &str| s.and_then(|s| s.get(k)).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
    (id("track"), id("channel"), id("plugin"))
}

/// An `f32` from the engine as the `f64` the document keeps: its shortest decimal (0.05f32 is 0.05, not
/// 0.0500000007), so a value at the top of a range is not refused as above it
pub(crate) fn doc_real(v: f32) -> f64 {
    v.to_string().parse::<f64>().unwrap_or(f64::from(v))
}

/// A normalized value of a document field (bool, choice, int, real by their schema ranges)
pub(crate) fn field_norm(kind: &FieldKind, v: &Value) -> Option<f32> {
    Some(match kind {
        FieldKind::Bool => {
            if v.as_bool()? {
                1.0
            } else {
                0.0
            }
        }
        FieldKind::Choice(options) => {
            let i = options.iter().position(|o| Some(*o) == v.as_text())?;
            if options.len() < 2 { 0.0 } else { i as f32 / (options.len() - 1) as f32 }
        }
        FieldKind::Int { min, max } => ((v.as_int()? - min) as f64 / (*max - *min).max(1) as f64) as f32,
        FieldKind::Real { min, max } => ((v.as_real()? - min) / (max - min).max(f64::MIN_POSITIVE)) as f32,
        _ => return None,
    })
}

impl Session {
    /// The field kind at `path` in node `id` (following nested objects)
    pub(crate) fn field_kind(&self, id: NodeId, path: &[PathStep]) -> Option<FieldKind> {
        let mut ty = if id == ROOT { lm_doc::PROJECT } else { self.document.type_of(id)? };
        let mut kind = None;
        for (i, step) in path.iter().enumerate() {
            let PathStep::Key(k) = step else { return None };
            let f = self.schemas.find(ty)?.find_field(k)?;
            if i + 1 < path.len() {
                match &f.kind {
                    FieldKind::Object(t) => ty = t,
                    _ => return None,
                }
            } else {
                kind = Some(f.kind.clone());
            }
        }
        kind
    }

    /// A parameter's range: the compiled one when it has a slot, else its schema's (the one value law the core and the
    /// interface share for MIDI, automation and the editors)
    pub fn param_range(&self, node: NodeId, key: &str) -> ParamRange {
        if let Some(r) = self.table.ranges.get(&(node, Arc::from(key))) {
            return *r;
        }
        let ty = self.document.type_of(node).unwrap_or("");
        if ty == lm_doc::schemas::types::PLUGIN {
            let plugin =
                self.document.find(node).and_then(|n| n.get("plugin").and_then(Value::as_text).map(str::to_owned));
            if let Some(spec) =
                plugin.and_then(|p| self.schemas.resolve_param(&lm_doc::schemas::plugin_type(&p), key).cloned())
            {
                return ParamRange { min: spec.min as f32, max: spec.max as f32, scale: RangeScale::Linear };
            }
        }
        let path = lm_doc::param_path(ty, key);
        match self.field_kind(node, &path) {
            Some(FieldKind::Real { min, max }) => ParamRange::linear(min as f32, max as f32),
            Some(FieldKind::Int { min, max }) => {
                ParamRange { min: min as f32, max: max as f32, scale: RangeScale::Stepped }
            }
            _ => ParamRange::linear(0.0, 1.0),
        }
    }

    // ---- the map ----

    /// Reads the mappings of the project, the machine and the enabled controller profiles, and the port aliases
    fn midi_read_mappings(&mut self) {
        let mut bad = 0;
        let mut mappings = read_mappings(self.document.root().get("midiMap"), &mut bad);
        let project_count = mappings.len();
        mappings.extend(read_mappings(self.preferences.get(&["midiMap"]), &mut bad));
        let profiles = mcu::profile_mappings(&self.preferences, &mut bad);
        mappings.extend(profiles);
        let ports = self.preferences.get(&["midi", "ports"]).cloned().unwrap_or_else(Value::empty_list);
        for p in ports.items() {
            let Some(alias) = p.get("alias").and_then(Value::as_shared_text).filter(|a| !a.is_empty()) else {
                continue;
            };
            let controller = p.get("role").and_then(Value::as_text) != Some("instrument");
            let next = PortIndex(u16::try_from(self.midi.aliases.len()).unwrap_or(u16::MAX));
            let e = self.midi.aliases.entry(alias.clone()).or_insert((next, controller));
            e.1 = controller;
        }
        if bad > 0 {
            self.notices.warn("midi.map.unreadable", vec![("count", Value::int(bad as i64))]);
        }
        let aliases: Vec<Arc<str>> = mappings[..project_count]
            .iter()
            .flat_map(|m| std::iter::once(m.source.port.clone()).chain(m.touch.as_ref().map(|t| t.port.clone())))
            .collect();
        self.midi.mappings = mappings;
        self.midi.project_count = project_count;
        self.midi_unknown_aliases(aliases);
    }

    /// Compiles the map against the last install's slots and sends it alone (I-21.2: cheap)
    fn midi_send_map(&mut self) {
        let ui = self.ui_document.root().clone();
        let (track, channel, plugin) = read_selection(&ui);
        self.midi.selection =
            Selection { track, channel, plugin, track_bank: self.midi.bank.0, channel_bank: self.midi.bank.1 };
        let input = MapInput {
            mappings: self.midi.mappings.clone(),
            project_count: self.midi.project_count,
            ports: self
                .midi
                .aliases
                .iter()
                .map(|(a, (i, c))| PortAlias { alias: a.clone(), index: *i, controller: *c })
                .collect(),
            port_count: u16::try_from(self.midi.aliases.len().max(1)).unwrap_or(u16::MAX),
            selection: self.midi.selection.clone(),
        };
        let CompiledMap { map, resolved, .. } =
            compile_map(self.document.root(), &|id| self.document.find(id), &self.table, &input, self.spec);
        if let Err(e) = self.engine.install(Install::map_only(self.table.generation, map)) {
            self.notices.error(&e, None, None);
            return;
        }
        self.midi.by_id = resolved.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        self.midi.resolved = resolved;
        // Feedback destinations: every mapping with feedback on a known port
        let mut fb = HashMap::new();
        for r in &self.midi.resolved {
            let m = &self.midi.mappings[r.mapping];
            let Some(port) = r.port else { continue };
            if !m.feedback || matches!(m.target, MapTarget::Action { .. }) {
                continue;
            }
            let (message, ring) = m.feedback_to.unwrap_or((m.source.message, Ring::None));
            let spec = Feedback { message, channel: m.source.channel.unwrap_or(0), ring };
            let limiter = self.midi.feedback.remove(&r.id).map(|f| f.limiter).unwrap_or_default();
            fb.insert(r.id, Fb { port, spec, transform: m.transform, limiter });
        }
        self.midi.feedback = fb;
        self.midi.map_dirty = false;
        self.midi.resend = true;
    }

    /// Each pump (I-16 step 2 and after): the rings of the map, the gestures, fields and actions, the recordings,
    /// learning, the map when it must change, feedback and the control surfaces
    pub(crate) fn midi_pump(&mut self) -> bool {
        let mut changed = false;
        let now0 = self.clock.now();
        self.midi_devices_in(now0);
        let mut events = Vec::new();
        self.engine.map_events(|e| events.push(e));
        changed |= !events.is_empty();
        let now = self.clock.now();
        for e in events {
            self.midi_event(e, now);
        }
        self.midi_apply_pending();
        self.midi_gestures_update();
        let mut touched = Vec::new();
        self.engine.touched(|t| touched.push(t));
        changed |= !touched.is_empty();
        self.automation_touched(&touched);
        let mut recorded = Vec::new();
        self.engine.recorded_midi(|r| recorded.push(r));
        changed |= !recorded.is_empty();
        self.record_midi(recorded);
        self.midi_learn_poll(now);
        // The selection the targets follow
        let (track, channel, plugin) = read_selection(self.ui_document.root());
        let s = &self.midi.selection;
        if (s.track, s.channel, s.plugin) != (track, channel, plugin) {
            self.midi.map_dirty = true;
            // A track whose input follows the selection: the live routes change (F-REC-131)
            let follows = self.document.root().get("tracks").is_some_and(|l| {
                l.items().any(|t| {
                    t.get("midiInput").and_then(|i| i.get("followSelection")).and_then(Value::as_bool) == Some(true)
                })
            });
            if follows && s.track != track {
                self.midi.selection.track = track;
                self.needs_compile = true;
            }
        }
        // The ports of the machine changed: the tracks' routes by alias are compiled again
        let ports = self.preferences.get(&["midi", "ports"]).cloned().unwrap_or(Value::NULL);
        if ports != self.midi.ports_seen {
            self.midi.ports_seen = ports;
            self.midi_read_mappings();
            self.needs_compile = true;
            self.midi.map_dirty = true;
        }
        if self.midi.map_dirty {
            self.midi_read_mappings();
            self.midi_send_map();
        }
        self.midi_feedback(now);
        self.mcu_pump(now);
        self.midi_programs();
        self.midi_devices_out(now);
        self.automation_pump();
        self.record_pump(now);
        changed
    }

    /// The document changed (I-16 step 7): the map follows mapping edits and removed targets
    pub(crate) fn midi_diff(&mut self, diff: &lm_doc::Diff) {
        let touches_map = diff
            .changed
            .iter()
            .any(|c| c.node == ROOT && c.paths.iter().any(|p| p.first() == Some(&PathStep::key("midiMap"))))
            || diff.created.iter().chain(diff.removed.iter()).any(|id| id.kind == lm_doc::schemas::KIND_MAPPING)
            || diff.structural;
        if touches_map {
            self.midi.map_dirty = true;
        }
    }

    fn midi_event(&mut self, e: MapEvent, now: HostTime) {
        match e {
            MapEvent::Param { mapping, port, slot, normalized, .. } => {
                if let Some((m, r)) = self.midi.mapping_of(mapping)
                    && let (MapTarget::Param { key, .. }, Some(node)) = (&m.target, r.node)
                {
                    let key: Arc<str> = Arc::from(key.as_str());
                    self.automation_hand(node, &key);
                    let value = match e {
                        MapEvent::Param { value, .. } => value,
                        _ => 0.0,
                    };
                    if let Some(g) = self.midi.gestures.get_mut(&slot.0) {
                        g.value = value;
                        g.dirty = true;
                    } else {
                        let doc = self.midi_doc_path(node, &key).map(|_| {
                            let label = Label::new("midi.gesture").arg("target", format!("{node} {key}"));
                            self.document.begin_gesture(label)
                        });
                        self.midi.gestures.insert(slot.0, Gesture { node, key, doc, value, dirty: true });
                    }
                }
                self.midi_echo(mapping, normalized, now);
                self.midi_activity_mapped(port, mapping);
            }
            MapEvent::Session { mapping, port, value, .. } => {
                self.midi_echo(mapping, value, now);
                self.midi_activity_mapped(port, mapping);
                // Keep the press and release of buttons; for knobs only the last value of this pump
                let knob = matches!(e, MapEvent::Session { press: None, .. });
                if knob
                    && let Some(i) =
                        self.midi.pending.iter().position(
                            |p| matches!(p, MapEvent::Session { mapping: m, press: None, .. } if *m == mapping),
                        )
                {
                    let relative =
                        self.midi.mapping_of(mapping).is_some_and(|(m, _)| m.transform.mode != Mode::Absolute);
                    if relative {
                        // Steps add up
                        if let (MapEvent::Session { raw: a, .. }, MapEvent::Session { raw: b, .. }) =
                            (&mut self.midi.pending[i], &e)
                        {
                            *a += *b;
                        }
                    } else {
                        self.midi.pending[i] = e;
                    }
                } else {
                    self.midi.pending.push(e);
                }
            }
            MapEvent::Touch { .. } => {}
            MapEvent::Released { slot, value, .. } => self.midi_gesture_end(slot, value),
        }
    }

    /// Where a parameter lives in the tree, if the tree keeps it (the monitor's level is only in its slot)
    pub(crate) fn midi_doc_path(&self, node: NodeId, key: &str) -> Option<Vec<PathStep>> {
        let ty = self.document.type_of(node)?;
        let path = lm_doc::param_path(ty, key);
        let kept = self.document.find(node).is_some_and(|n| n.at_path(&path).is_some());
        (kept || ty == lm_doc::schemas::types::PLUGIN).then_some(path)
    }

    /// A value for the tree: the shortest decimal, inside the schema's range
    pub(crate) fn midi_doc_value(&self, node: NodeId, key: &str, path: &[PathStep], value: f32) -> Option<Value> {
        let mut x = doc_real(value);
        if let Some(FieldKind::Real { min, max }) = self.field_kind(node, path) {
            x = x.clamp(min, max);
        } else if self.document.type_of(node) == Some(lm_doc::schemas::types::PLUGIN) {
            let r = self.param_range(node, key);
            x = x.clamp(f64::from(r.min.min(r.max)), f64::from(r.min.max(r.max)));
        }
        Value::real(x).ok()
    }

    /// The tree follows each open MIDI gesture (once per pump, the last value)
    fn midi_gestures_update(&mut self) {
        let slots: Vec<u32> = self.midi.gestures.iter().filter(|(_, g)| g.dirty).map(|(s, _)| *s).collect();
        for s in slots {
            self.midi_gesture_write(s);
        }
    }

    fn midi_gesture_write(&mut self, slot: u32) {
        let Some(g) = self.midi.gestures.get_mut(&slot) else { return };
        g.dirty = false;
        let (node, key, value) = (g.node, g.key.clone(), g.value);
        let Some(path) = self.midi_doc_path(node, &key) else { return };
        let Some(v) = self.midi_doc_value(node, &key, &path, value) else { return };
        let Some(g) = self.midi.gestures.get_mut(&slot) else { return };
        let Some(id) = g.doc.as_ref() else { return };
        if let Err(e) = self.document.gesture_update(id, |e| e.set(node, &path, v)) {
            // The target went away (G3), or an undo closed the gesture (G2): the control lets go of the tree
            if let Some(g) = self.midi.gestures.get_mut(&slot)
                && let Some(id) = g.doc.take()
            {
                let _ = self.document.gesture_cancel(id);
            }
            if e.key() != "doc.gesture.target_gone" && e.key() != "doc.gesture.closed" {
                self.notices.error(&e, None, None);
            }
        }
    }

    /// Whether a slot is held by a MIDI gesture (the session does not write it from the tree meanwhile: the renderer
    /// has newer values than the tree)
    pub(crate) fn midi_holds(&self, slot: ParamSlot) -> bool {
        self.midi.gestures.contains_key(&slot.0)
    }

    /// The end of a MIDI gesture (I-21.2): the final value into the tree, and the commit is one undo step
    fn midi_gesture_end(&mut self, slot: ParamSlot, value: f32) {
        let Some(g) = self.midi.gestures.get_mut(&slot.0) else { return };
        g.value = value;
        self.midi_gesture_write(slot.0);
        let Some(g) = self.midi.gestures.remove(&slot.0) else { return };
        if let Some(id) = g.doc
            && let Err(e) = self.document.gesture_commit(id)
            && e.key() != "doc.gesture.closed"
        {
            self.notices.error(&e, None, None);
        }
    }

    /// Fields and actions (I-21.2): a field is a transaction, an action runs through the registry
    fn midi_apply_pending(&mut self) {
        let pending = std::mem::take(&mut self.midi.pending);
        for e in pending {
            let MapEvent::Session { mapping, value, raw, press, .. } = e else { continue };
            let Some((m, r)) = self.midi.mapping_of(mapping) else { continue };
            let (m, node) = (m.clone(), r.node);
            let result = match &m.target {
                MapTarget::Field { path, .. } => match node {
                    Some(n) => self.midi_set_field(n, path, &m, value, raw, press),
                    None => Ok(()),
                },
                MapTarget::Param { key, .. } => match node {
                    Some(n) => {
                        let ty = self.document.type_of(n).unwrap_or("");
                        let path = lm_doc::param_path(ty, key);
                        let range = self.param_range(n, key);
                        let v = if m.transform.mode == Mode::Absolute {
                            range.to_units(value)
                        } else {
                            let cur = self.document.find(n).and_then(|x| x.at_path(&path).and_then(Value::as_real));
                            let y = range.normalize(cur.unwrap_or(0.0) as f32) + raw as f32 * m.transform.step;
                            range.to_units(y)
                        };
                        let label = Label::new("midi.gesture").arg("target", format!("{n} {key}"));
                        Value::real(doc_real(v)).and_then(|v| {
                            self.document.transact_as(label, Origin::Gesture, |e| e.set(n, &path, v)).map(|_| ())
                        })
                    }
                    None => Ok(()),
                },
                MapTarget::Action { id, params, value_into } => {
                    // Buttons act on the press; knobs on every value
                    if press == Some(false) {
                        continue;
                    }
                    if let Some(list) = self.midi.dry_run.as_mut() {
                        list.push(id.clone());
                        continue;
                    }
                    let mut p = params.clone();
                    if let Some(path) = value_into {
                        p = p.with_path(path, Value::int(i64::from(raw))).unwrap_or(p);
                    }
                    if crate::actions::find(id).is_some() {
                        self.run_action(id, &p)
                    } else {
                        // An action of the interface (zoom, scroll, a page): the GUI runs it (`MidiUiAction`)
                        self.midi.ui.push(MidiUiAction { action: lm_port::ActionId(id.clone()), params: p, value });
                        Ok(())
                    }
                }
            };
            if let Err(err) = result {
                self.notices.error(&err, None, None);
            }
        }
    }

    fn midi_set_field(
        &mut self,
        node: NodeId,
        path: &[PathStep],
        m: &Mapping,
        value: f32,
        raw: i32,
        press: Option<bool>,
    ) -> Result<()> {
        let Some(kind) = self.field_kind(node, path) else {
            return Err(Error::new("midi.field.unknown").arg("node", node.to_string()));
        };
        let cur = self.document.find(node).and_then(|n| n.at_path(path).cloned()).unwrap_or(Value::NULL);
        let relative = m.transform.mode != Mode::Absolute;
        let new = match &kind {
            FieldKind::Bool => {
                let now = cur.as_bool().unwrap_or(false);
                let v = match (press, m.transform.button) {
                    (Some(true), lm_midimap::Button::Toggle) => !now,
                    (Some(down), lm_midimap::Button::Momentary) => down,
                    (Some(true), lm_midimap::Button::Trigger) => true,
                    (Some(_), _) => return Ok(()),
                    (None, _) if relative => raw > 0,
                    (None, _) => value >= 0.5,
                };
                Value::bool(v)
            }
            FieldKind::Choice(options) => {
                let n = options.len().max(1) as i64;
                let i = options.iter().position(|o| Some(*o) == cur.as_text()).unwrap_or(0) as i64;
                let j = if relative {
                    i + i64::from(raw)
                } else if press == Some(true) && m.transform.button == lm_midimap::Button::Toggle {
                    i + 1 // a button steps through the choices
                } else {
                    (f64::from(value) * (n - 1) as f64).round() as i64
                };
                let j = if press.is_some() && m.transform.button == lm_midimap::Button::Toggle {
                    j.rem_euclid(n)
                } else {
                    j.clamp(0, n - 1)
                };
                Value::text(options[j as usize])
            }
            FieldKind::Int { min, max } => {
                let v = if relative {
                    cur.as_int().unwrap_or(*min) + i64::from(raw)
                } else {
                    min + (f64::from(value) * (max - min) as f64).round() as i64
                };
                Value::int(v.clamp(*min, *max))
            }
            FieldKind::Real { min, max } => {
                let v = if relative {
                    cur.as_real().unwrap_or(*min) + f64::from(raw) * f64::from(m.transform.step) * (max - min)
                } else {
                    min + f64::from(value) * (max - min)
                };
                Value::real(v.clamp(*min, *max))?
            }
            _ => return Err(Error::new("midi.field.not_mappable")),
        };
        if new == cur {
            return Ok(());
        }
        let label = Label::new("midi.gesture").arg("target", format!("{node} {}", lm_state::path_text(path)));
        self.document.transact(label, |e| e.set(node, path, new)).map(|_| ())
    }

    // ---- the activity monitor and learning ----

    /// A raw message from a device (every one, mapped or not): the activity monitor and learning
    pub fn midi_input(&mut self, port: PortIndex, at: HostTime, bytes: &[u8]) {
        self.step_key(port, bytes);
        if let Some(l) = self.midi.learning.as_mut() {
            l.detector.feed(port.0, at.0, bytes);
        }
        let Some(raw) = Raw::parse(bytes) else { return };
        let (message, value) = match raw {
            Raw::NoteOn { key, velocity, .. } => (Message::Note { key }, u16::from(velocity)),
            Raw::NoteOff { key, .. } => (Message::Note { key }, 0),
            Raw::PolyPressure { key, value, .. } => (Message::PolyPressure { key }, u16::from(value)),
            Raw::Cc { number, value, .. } => (Message::Cc { number }, u16::from(value)),
            Raw::ProgramChange { program, .. } => (Message::ProgramChange, u16::from(program)),
            Raw::ChannelPressure { value, .. } => (Message::ChannelPressure, u16::from(value)),
            Raw::PitchBend { value, .. } => (Message::PitchBend, value),
        };
        let a = MidiActivity {
            port: self.midi.port_alias(port),
            channel: raw.channel(),
            message,
            value,
            mapped: false,
            time: at,
        };
        self.midi_push_activity(a);
    }

    fn midi_push_activity(&mut self, a: MidiActivity) {
        self.midi.activity_seq += 1;
        self.midi.activity.push_back((self.midi.activity_seq, a));
        while self.midi.activity.len() > ACTIVITY_KEEP {
            self.midi.activity.pop_front();
        }
    }

    /// The monitor marks the messages a mapping took
    fn midi_activity_mapped(&mut self, port: PortIndex, mapping: u64) {
        let alias = self.midi.port_alias(port);
        let message = self.midi.mapping_of(mapping).map(|(m, _)| m.source.message);
        for (_, a) in self.midi.activity.iter_mut().rev().take(8) {
            if a.port == alias && Some(a.message) == message {
                a.mapped = true;
                return;
            }
        }
    }

    /// The activity since `since` (lossy: the last 256)
    pub fn midi_activity(&self, since: u64) -> (u64, Vec<MidiActivity>) {
        let list = self.midi.activity.iter().filter(|(s, _)| *s > since).map(|(_, a)| a.clone()).collect();
        (self.midi.activity_seq, list)
    }

    /// The interface's actions a control asked for (I-21.2, the `MidiUiAction` feed)
    pub fn drain_midi_ui(&mut self, out: &mut Vec<MidiUiAction>) {
        out.append(&mut self.midi.ui);
    }

    /// Starts learning a source for `target` (I-21.2). A new request replaces the one running
    pub fn midi_learn(&mut self, target: MapTarget, scope: MapScope) -> LearnId {
        self.midi.learn_serial += 1;
        let id = self.midi.learn_serial;
        self.midi.learning = Some(Learning { id, target, scope, detector: Detector::default() });
        id
    }

    pub fn cancel_learn(&mut self, id: LearnId) {
        if self.midi.learning.as_ref().is_some_and(|l| l.id == id) {
            self.midi.learning = None;
        }
    }

    pub fn learning(&self) -> Option<LearnId> {
        self.midi.learning.as_ref().map(|l| l.id)
    }

    fn midi_learn_poll(&mut self, now: HostTime) {
        let Some(learned) = self.midi.learning.as_ref().and_then(|l| l.detector.result(now.0)) else { return };
        let Some(l) = self.midi.learning.take() else { return };
        let alias = self.midi.port_alias(PortIndex(learned.port));
        // A loop (the target drives its own source) is refused: the mapping's feedback would answer the control
        let loops = match &l.target {
            MapTarget::Action { id, .. } => id.starts_with("midi."),
            _ => false,
        };
        if loops {
            self.notices.warn("midi.learn_refused", Vec::new());
            return;
        }
        let mut transform = Transform { mode: learned.mode, ..Transform::default() };
        if learned.switch || learned.message.is_note() {
            transform.button = lm_midimap::Button::Toggle;
        }
        let mapping = Mapping {
            id: None,
            source: lm_midimap::DocSource { port: alias, channel: Some(learned.channel), message: learned.message },
            touch: None,
            target: l.target,
            transform,
            feedback: true,
            consume: true,
            feedback_to: None,
        };
        let r = self.midi_add_mapping(&mapping, l.scope, Label::new("midi.learn"));
        match r {
            Ok(()) => self.notices.push(
                "midi.learned",
                vec![("source", Value::text(format!("{:?}", learned.message)))],
                Severity::Info,
                None,
                Vec::new(),
            ),
            Err(e) => self.notices.error(&e, None, None),
        }
    }

    /// Adds a mapping to the project (one transaction) or to the machine's preferences
    pub fn midi_add_mapping(&mut self, mapping: &Mapping, scope: MapScope, label: Label) -> Result<()> {
        let v = mapping.to_value().without("id");
        match scope {
            MapScope::Project => {
                self.document
                    .transact(label, |e| {
                        let n = e.root().get("midiMap").map_or(0, Value::len);
                        e.insert(ROOT, "midiMap", n, v).map(|_| ())
                    })
                    .map(|_| ())?;
            }
            MapScope::Machine => {
                let list = self.preferences.get(&["midiMap"]).cloned().unwrap_or_else(Value::empty_list);
                let list = list.with_pushed(v)?;
                self.set_preference(&[PathStep::key("midiMap")], list)?;
            }
        }
        self.midi.map_dirty = true;
        Ok(())
    }

    /// Removes every mapping on `target` ("Tirar o mapa")
    pub fn midi_unmap(&mut self, target: &MapTarget, scope: MapScope) -> Result<()> {
        let same = |v: &Value| Mapping::from_value(v).is_ok_and(|m| &m.target == target);
        match scope {
            MapScope::Project => {
                let ids: Vec<NodeId> = self
                    .document
                    .root()
                    .get("midiMap")
                    .map(|l| {
                        l.items()
                            .filter(|v| same(v))
                            .filter_map(|v| v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()))
                            .collect()
                    })
                    .unwrap_or_default();
                if !ids.is_empty() {
                    self.document.transact(Label::new("midi.unmap"), |e| {
                        for id in &ids {
                            e.remove(*id)?;
                        }
                        Ok(())
                    })?;
                }
            }
            MapScope::Machine => {
                let list = self.preferences.get(&["midiMap"]).cloned().unwrap_or_else(Value::empty_list);
                let kept = Value::list(list.items().filter(|v| !same(v)).cloned());
                self.set_preference(&[PathStep::key("midiMap")], kept)?;
            }
        }
        self.midi.map_dirty = true;
        Ok(())
    }

    // ---- feedback (I-21.3) ----

    /// The consumer of the feedback ring, for the device's output thread (taken once)
    pub fn take_midi_feedback(&mut self) -> Option<Consumer<MidiOutEvent>> {
        self.midi.out_consumer.take()
    }

    /// The SysEx to send (the Mackie Control display), for the device's output
    pub fn take_midi_sysex(&mut self) -> Vec<(PortIndex, Vec<u8>)> {
        std::mem::take(&mut self.midi.sysex)
    }

    /// The ports that are connected now (`lm-io`'s hotplug events): a port that comes back gets every value again
    pub fn midi_ports(&mut self, connected: &[PortIndex]) {
        let now: BTreeSet<u16> = connected.iter().map(|p| p.0).collect();
        self.midi.ever_connected.extend(now.iter().copied());
        let back: Vec<u16> = now.difference(&self.midi.connected).copied().collect();
        if !back.is_empty() {
            for f in self.midi.feedback.values_mut() {
                if back.contains(&f.port.0) {
                    f.limiter.reset();
                    f.limiter.resend();
                }
            }
            // The output programs go again to a port that came back
            self.midi.programs_sent.retain(|_, (ports, _, _)| !ports.iter().any(|p| back.contains(&p.0)));
            for m in &mut self.midi.mcu {
                if back.contains(&m.port.0) {
                    m.reset();
                }
            }
        }
        self.midi.connected = now;
    }

    /// A value that came from a control: no feedback back to it for 50 ms
    fn midi_echo(&mut self, mapping: u64, value: f32, now: HostTime) {
        if let Some(f) = self.midi.feedback.get_mut(&mapping) {
            let q = f.spec.quantize(f.transform.inverse(value));
            f.limiter.input(now.0, q, 50 * MS);
        }
    }

    /// The target value of each fed-back mapping, from the renderer's frame (parameters) or the document (fields)
    fn midi_feedback(&mut self, now: HostTime) {
        if self.midi.feedback.is_empty() {
            return;
        }
        let params: Vec<(u64, f32)> = {
            let frame = self.engine.frame();
            let mut v = Vec::new();
            for id in self.midi.feedback.keys() {
                let Some(i) = self.midi.by_id.get(id) else { continue };
                let r = &self.midi.resolved[*i];
                let m = &self.midi.mappings[r.mapping];
                if let (MapTarget::Param { key, .. }, Some(node), Some(slot)) = (&m.target, r.node, r.slots.first())
                    && let Some(x) = frame.params.get(slot.0 as usize)
                    && let Some(range) = self.table.ranges.get(&(node, Arc::from(key.as_str())))
                {
                    v.push((*id, range.normalize(*x)));
                }
            }
            v
        };
        let mut values: HashMap<u64, f32> = params.into_iter().collect();
        for (id, i) in &self.midi.by_id {
            if values.contains_key(id) || !self.midi.feedback.contains_key(id) {
                continue;
            }
            let r = &self.midi.resolved[*i];
            let m = &self.midi.mappings[r.mapping];
            let Some(node) = r.node else { continue };
            let path = match &m.target {
                MapTarget::Field { path, .. } => path.clone(),
                MapTarget::Param { key, .. } => lm_doc::param_path(self.document.type_of(node).unwrap_or(""), key),
                MapTarget::Action { .. } => continue,
            };
            let Some(v) = self.document.find(node).and_then(|n| n.at_path(&path).cloned()) else { continue };
            let norm = match &m.target {
                MapTarget::Param { key, .. } => v.as_real().map(|x| self.param_range(node, key).normalize(x as f32)),
                _ => self.field_kind(node, &path).and_then(|k| field_norm(&k, &v)),
            };
            if let Some(n) = norm {
                values.insert(*id, n);
            }
        }
        let resend = std::mem::take(&mut self.midi.resend);
        let mut out = Vec::new();
        for (id, f) in self.midi.feedback.iter_mut() {
            if let Some(y) = values.get(id) {
                f.limiter.offer(f.spec.quantize(f.transform.inverse(*y)));
            }
            if resend {
                f.limiter.resend();
            }
            if let Some(q) = f.limiter.poll(now.0, 10 * MS) {
                let port = f.port;
                f.spec.encode(q, |bytes, len| out.push(MidiOutEvent { port, at: now, len, bytes }));
            }
        }
        for e in out {
            let _ = self.midi.out.push(e); // lossy: the next change or resend sends it again
        }
    }

    /// The output program of each track's MIDI output (P-38, the legacy "Output MIDI program"): sent when it changes,
    /// when the project opens and when its port comes back
    fn midi_programs(&mut self) {
        let mut wanted: HashMap<NodeId, (Vec<PortIndex>, u8, u8)> = HashMap::new();
        for t in self.document.root().get("tracks").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default()
        {
            let Some(id) = t.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else { continue };
            let Some(o) = t.get("midiOutput").filter(|o| !o.is_null()) else { continue };
            if o.get("on").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let Some(program) = o.get("program").and_then(Value::as_int).and_then(|p| u8::try_from(p).ok()) else {
                continue;
            };
            let channel = o.get("channel").and_then(Value::as_int).unwrap_or(0).clamp(0, 15) as u8;
            let ports: Vec<PortIndex> = o
                .get("ports")
                .map(|l| l.items().filter_map(Value::as_text).filter_map(|a| self.midi_port_of(a)).collect())
                .unwrap_or_default();
            wanted.insert(id, (ports, channel, program.min(127)));
        }
        for (id, w) in &wanted {
            if self.midi.programs_sent.get(id) != Some(w) {
                for port in &w.0 {
                    self.midi_send(*port, &[0xC0 | w.1, w.2]);
                }
            }
        }
        self.midi.programs_sent = wanted;
    }

    /// Sends a raw message to a port through the feedback ring (the MCU module's meters and LEDs)
    pub(crate) fn midi_send(&mut self, port: PortIndex, bytes: &[u8]) {
        let mut b = [0u8; 3];
        let len = bytes.len().min(3);
        b[..len].copy_from_slice(&bytes[..len]);
        let at = self.clock.now();
        let _ = self.midi.out.push(MidiOutEvent { port, at, len: len as u8, bytes: b });
    }

    /// The MIDI target a node's control resolves to now (the binding window shows it)
    pub fn midi_mappings_of(&self, node: NodeId) -> Vec<Mapping> {
        self.midi
            .resolved
            .iter()
            .filter(|r| r.node == Some(node))
            .map(|r| self.midi.mappings[r.mapping].clone())
            .collect()
    }

    /// The port index of an alias (the device layer opens ports by alias)
    pub fn midi_port_of(&self, alias: &str) -> Option<PortIndex> {
        self.midi.aliases.get(alias).map(|(p, _)| *p)
    }

    /// Recompiles the map now (tests; the session does it by itself when something changes)
    pub fn midi_refresh(&mut self) {
        self.midi.map_dirty = true;
        self.midi_read_mappings();
        self.midi_send_map();
    }

    /// Sets a key path of the interface document (I-3.8: no undo, not journaled); the selection the MIDI targets
    /// follow lives there (`selection: {track, channel, plugin}`)
    pub fn set_ui(&mut self, path: &[PathStep], value: Value) -> Result<()> {
        self.ui_document.transact(Label::new("ui.set"), |e| e.set(ROOT, path, value)).map(|_| ())
    }

    /// "Test the control" (the binding window): while on, the actions mapped controls would run are only listed
    /// (`midi_dispatched`); parameters and fields still move
    pub fn midi_dry_run(&mut self, on: bool) {
        self.midi.dry_run = on.then(Vec::new);
    }

    /// The actions listed while testing controls
    pub fn midi_dispatched(&mut self) -> Vec<Arc<str>> {
        self.midi.dry_run.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// The hand overrode these automation lanes (shown until re-armed, I-21.5)
    pub fn hand_won(&self) -> Vec<(NodeId, Arc<str>)> {
        self.midi.hand_won.iter().cloned().collect()
    }
}
