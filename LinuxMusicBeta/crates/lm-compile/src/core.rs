//! The translation (I-5.4, I-3.9, base spec items 1–3 and 9): the tree to a program, an install and the tables.
//!
//! # The graph
//!
//! Per track (folders make no audio; base item 2):
//! - its **instrument** (a processor node; MIDI items and, when armed, the live MIDI ports go to it), and/or its
//!   **audio** node (a summing node the audio items play into, in the track's own format, M6);
//! - its **gain** node: the track's volume and mute, once for all its outputs (a track without instrument carries
//!   them on its audio node). The track has no pan;
//! - one node per **output** (`o:`), fed from its port's channels (an instrument's port by key, or the next channels
//!   of an audio track), metered, and summed into the channel it sends to (the master if that is gone);
//! - when armed and monitored (`monitor` `"on"`, or `"auto"` and armed), a **monitor** node with the device input,
//!   the track's volume (a second slot of it) and mute, summed into the main output **outside the PDC** (H5).
//!
//! Per desk channel: its **input** (the sum, in a fixed order: the tracks' outputs in track and output order, then
//! the sends of the channels in desk order), the rack in segments and the inserts at their points (`rack.rs`), the
//! **fader** (volume, and pan unless inserts sit before the pan, then a **pan** node after them), sends taken before
//! the fader (pre) or after the last node (post), and the output to another channel or, for the master, to the
//! device. The master's order is DRIVE, its inserts, STEREO and BUS COMP, the fader, XFMR.
//!
//! Plus the **metronome** (its own source, to the device channels of the preference or the master's) and the
//! **preview bus** (into the master).
//!
//! # Mute and solo (base item 2)
//!
//! - A track's mute silences all its outputs (its gain node). Soloing tracks silences the other tracks (a folder's
//!   solo covers its children); the desk stays whole.
//! - Soloing channels keeps audible the soloed ones, the channels that feed them (their sources) and the ones they
//!   reach (their path to the master, through outputs and sends); the rest of the desk is silenced **before** its
//!   fader, so their pre-fader sends go quiet too. A channel that reaches an audible one only through a sidechain
//!   keeps feeding it and is silenced after its fader (F-MIX-014: the sidechain sources keep feeding).
//! - Both at once: the intersection (the track solo silences tracks, the channel solo silences channels).
//! - A channel's own mute is after its pan, so its pre-fader sends go on, as on a desk.
//!
//! # PDC (base item 3)
//!
//! Every node's input is aligned to its latest input: a feed is delayed by the difference. The latency of a
//! processor is the one its built processor reports (`Processor::latency`, read when it arrives, or set again by
//! `latency_changed`); a waiting node has none. The monitoring feed is never delayed nor counted (H5); for that, the
//! delay a track's output needs is moved before the output node. A tail node's feeds are aligned to its
//! destination without counting.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use lm_base::{Error, Result};
use lm_doc::schemas::{KIND_CHANNEL, KIND_PLUGIN, MASTER};
use lm_render::modulation::{MOD_DEFAULTS, MOD_KEYS, MOD_PARAMS, ModKind, ModTarget, ModulatorSpec};
use lm_render::{
    AudioItem, AutomationMode, ChannelMix, CountIn, Curve, Feed, HostParam, Install, Limits, Metronome, MidiItem,
    MidiOutRoute, NodeCore, NodeDesc, NodeIndex, OutputRoute, ParamInfo, ParamRange, ParamSlot, PortIndex, PortRoute,
    ProcessSpec, ProgramBuilder, ProgramGen, RangeScale, Rate, Relocate, Returned, Rng, Strip, TransportSettings,
};
use lm_state::{NodeId, ParamSpec, Scale, SchemaRegistry, Value};
use lm_time::TempoMap;

use crate::alloc::{NodeAllocator, SlotAllocator};
use crate::catalog::{PluginCatalog, PluginShape, Resources};
use crate::items::{Caches, StreamRegistration, WaitingItem};
use crate::rack::{InsertPoint, RackParam, Stage, rack_params};
use crate::read::{
    channels, device_channels, flag, id_number, id_of, int, items, key, real, ref_of, settings, tempo_map, text,
};
use crate::types::{
    Build, BuildRequest, Compiled, Environment, InstalledProcessor, InstalledTable, InstalledTail, NodeInfo, NodeKey,
    Route, Ticket,
};

/// A tail restored from a table, not yet stamped with the audio clock
const UNSTAMPED: u64 = u64::MAX;

/// The renderer ends every tail by itself within `lm_render::TAIL_SECONDS` of audio and says so
/// (`Telemetry::TailEnded`). If that event is lost (its ring was full) the compiler drops the tail anyway after this
/// many audio seconds (frames rendered, never wall time: C-3 of the render predictability round)
pub const TAIL_NET_SECONDS: u64 = lm_render::TAIL_SECONDS + 2;

/// What the compiler is made with
#[derive(Clone)]
pub struct CompilerConfig {
    /// The project's rate and the device's negotiated block (C7)
    pub spec: ProcessSpec,
    /// The engine's capacities (the same `Limits` given to `lm_render::engine`)
    pub limits: Limits,
    /// The project's schemas (the rack's parameters come from them)
    pub schemas: Arc<SchemaRegistry>,
    pub plugins: Arc<dyn PluginCatalog>,
}

impl std::fmt::Debug for CompilerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilerConfig").field("spec", &self.spec).field("limits", &self.limits).finish_non_exhaustive()
    }
}

/// How much of the engine a project needs (H12), as the compiler counts it: give it to the document's limits rule
/// (`lm_doc::Options::usage`) so the document refuses exactly what the compiler would
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub nodes: usize,
    pub params: usize,
}

struct ReadyCore {
    ticket: u64,
    build: Build,
    shape: PluginShape,
    core: Box<NodeCore>,
    latency: u32,
}

impl std::fmt::Debug for ReadyCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadyCore").field("ticket", &self.ticket).field("latency", &self.latency).finish()
    }
}

#[derive(Debug, Default)]
struct Proc {
    installed: Option<InstalledProcessor>,
    requested: Option<(u64, Build, PluginShape)>,
    ready: Option<ReadyCore>,
    /// The installed processor must be built again though its state did not change (a third-party plugin whose
    /// helper crashed or hung: "recreate"); it plays until the new one is ready
    renew: bool,
}

#[derive(Clone, Debug)]
struct Tail {
    serial: u64,
    owner: NodeKey,
    shape: PluginShape,
    in_place: bool,
    latency: u32,
    index: Option<NodeIndex>,
    /// The audio clock (`CompilerCore::frames`) when the tail was made
    created: u64,
    ended: bool,
}

// ---------------------------------------------------------------------------------------------------------------
// The plan: the graph by keys, before indices and slots

#[derive(Clone, Debug)]
enum HPlan {
    Fixed(f32),
    Param { owner: NodeId, key: Arc<str>, instance: u8, value: f32, complement: bool },
}

#[derive(Clone, Debug)]
struct PNode {
    key: NodeKey,
    sum: bool,
    inputs: u16,
    outputs: u16,
    sidechain: u16,
    in_place: bool,
    gain: Option<HPlan>,
    pan: Option<HPlan>,
    muted: bool,
    tail: bool,
    latency: u32,
    /// A note effect: no audio, chained before an instrument
    note_effect: bool,
}

impl PNode {
    fn sum(key: NodeKey, channels: u16) -> Self {
        Self {
            key,
            sum: true,
            inputs: channels,
            outputs: channels,
            sidechain: 0,
            in_place: true,
            gain: None,
            pan: None,
            muted: false,
            tail: false,
            latency: 0,
            note_effect: false,
        }
    }

    fn processor(key: NodeKey, shape: &PluginShape, latency: u32) -> Self {
        let outputs = shape.output_channels();
        let in_place = shape.inputs > 0 && shape.inputs == outputs;
        Self {
            key,
            sum: false,
            inputs: shape.inputs,
            outputs,
            sidechain: shape.sidechain_channels(),
            in_place,
            gain: None,
            pan: None,
            muted: false,
            tail: false,
            latency,
            note_effect: shape.note_effect,
        }
    }
}

#[derive(Clone, Debug)]
struct PFeed {
    from: usize,
    to: usize,
    from_channel: u16,
    channels: u16,
    to_channel: u16,
    mix: ChannelMix,
    sidechain: bool,
    gain: Option<HPlan>,
    /// The monitoring path: never delayed nor counted (H5)
    monitor: bool,
    /// From a tail node: aligned, not counted
    tail: bool,
    delay: u32,
}

#[derive(Clone, Debug)]
struct PParam {
    node: usize,
    index: u32,
    owner: NodeId,
    key: Arc<str>,
    rate: Rate,
    discrete: bool,
    value: f32,
    range: ParamRange,
}

/// What the plan decided for a processor node
#[derive(Debug)]
struct ProcPlan {
    key: NodeKey,
    /// What it should be (`None`: an unknown plugin, silent)
    wanted: Option<(Build, PluginShape)>,
    /// The ready core matches: it goes into this install
    use_ready: bool,
    /// A new build is needed
    request: bool,
    /// Whether it is waiting (silent) or keeping its old processor (T7)
    waiting: bool,
    replacing: bool,
    /// The node needs a fresh index (its processor must go: an unknown plugin now)
    fresh: bool,
    seed: u64,
}

/// A track's MIDI out (P-38): (node, port, channel, fixed velocity, fixed note)
type PMidiOut = (usize, PortIndex, u8, Option<u8>, Option<u8>);

#[derive(Debug, Default)]
struct Plan {
    nodes: Vec<PNode>,
    index: HashMap<NodeKey, usize>,
    feeds: Vec<PFeed>,
    feed_keys: HashSet<(usize, usize, u16, u16, bool)>,
    params: Vec<PParam>,
    audio: Vec<(usize, Vec<AudioItem>)>,
    midi: Vec<(usize, Vec<Arc<MidiItem>>)>,
    inputs: Vec<(usize, Vec<u16>)>,
    /// Live MIDI routes: (port, node, channel filter, fixed velocity)
    ports: Vec<(PortIndex, usize, Option<u8>, Option<u8>)>,
    /// The key range, base key, velocity and tuning of the nodes' live input
    live_keys: Vec<(usize, lm_render::LiveKeys)>,
    /// The MPE ports of the machine (a preference)
    mpe: Vec<lm_render::mpe::MpePort>,
    /// MIDI out to devices (P-38)
    midi_out: Vec<PMidiOut>,
    /// The note effects chained before an instrument: (instrument node, its effects in order)
    chains: Vec<(usize, Vec<usize>)>,
    /// The CC rack (F-IJN-025): (instrument node, track, controller, value)
    ccs: Vec<(usize, NodeId, u8, f32)>,
    outputs: Vec<(usize, u16, u16)>,
    master: Option<usize>,
    preview: Option<usize>,
    metronome: Option<(usize, Metronome)>,
    transport: TransportSettings,
    curves: HashMap<(NodeId, Arc<str>), Curve>,
    /// How each curve plays (I-21.5)
    modes: HashMap<(NodeId, Arc<str>), AutomationMode>,
    procs: Vec<ProcPlan>,
    /// Tails: (serial, owner's plan index, the tail's plan index)
    tails: Vec<(u64, usize)>,
    new_tails: Vec<(u64, NodeKey, PluginShape, bool, u32)>,
    waiting_items: Vec<WaitingItem>,
    streams: Vec<StreamRegistration>,
    stretches: Vec<crate::stretch::StretchPlan>,
    low_latency_bypassed: Vec<NodeKey>,
    /// The modulators of the project (decisoes-autonomas, 26/11, item 1)
    mods: Vec<PMod>,
}

/// A modulator as the plan read it
#[derive(Debug)]
struct PMod {
    id: NodeId,
    kind: ModKind,
    /// The document's kind name (`lm_doc::schemas::modulator_keys`)
    kind_name: &'static str,
    params: [f32; MOD_PARAMS],
    seed: u64,
    user_wave: Option<Arc<[f32]>>,
    /// Plan nodes: what a peak follower measures, what triggers an envelope
    input: Option<usize>,
    trigger: Option<usize>,
    /// (node, parameter key, amount, min, max)
    targets: Vec<(NodeId, Arc<str>, f32, f32, f32)>,
}

impl Plan {
    fn add(&mut self, node: PNode) -> usize {
        let i = self.nodes.len();
        self.index.insert(node.key, i);
        self.nodes.push(node);
        i
    }

    /// A feed converting `from_ch` channels (from `from_channel`) to `to_ch` channels (at `to_channel`): direct,
    /// mono to N at equal power, N to mono averaged, or the common channels (base item 3)
    #[allow(clippy::too_many_arguments)]
    fn feed(
        &mut self,
        from: usize,
        from_channel: u16,
        from_ch: u16,
        to: usize,
        to_channel: u16,
        to_ch: u16,
        sidechain: bool,
    ) -> &mut PFeed {
        let (channels, mix) = convert(from_ch, to_ch);
        self.feed_keys.insert((from, to, from_channel, to_channel, sidechain));
        self.feeds.push(PFeed {
            from,
            to,
            from_channel,
            channels,
            to_channel,
            mix,
            sidechain,
            gain: None,
            monitor: false,
            tail: false,
            delay: 0,
        });
        self.feeds.last_mut().expect("just pushed")
    }

    fn has_feed(&self, from: usize, to: usize, from_channel: u16, to_channel: u16, sidechain: bool) -> bool {
        self.feed_keys.contains(&(from, to, from_channel, to_channel, sidechain))
    }
}

fn convert(from: u16, to: u16) -> (u16, ChannelMix) {
    if from == to {
        (from, ChannelMix::Direct)
    } else if from == 1 {
        (1, ChannelMix::UpMix { to })
    } else if to == 1 {
        (from, ChannelMix::DownMix)
    } else {
        (from.min(to), ChannelMix::Direct)
    }
}

/// A desk channel (or the master) as read from the tree
struct ChannelView<'a> {
    id: NodeId,
    value: &'a Value,
    master: bool,
    width: u16,
}

/// The compiler's state and translation, synchronous (the `Compiler` runs it on the compile thread). Tests and
/// the export (I-13: its own compiler, no device) can use it directly
pub struct CompilerCore {
    config: CompilerConfig,
    rack_channel: Vec<RackParam>,
    rack_master: Vec<RackParam>,
    nodes: BTreeMap<NodeKey, NodeIndex>,
    node_alloc: NodeAllocator,
    slot_keys: BTreeMap<(NodeId, Arc<str>, u8), ParamSlot>,
    slot_alloc: SlotAllocator,
    procs: BTreeMap<NodeKey, Proc>,
    tails: Vec<Tail>,
    next_serial: u64,
    next_ticket: u64,
    generation: u64,
    tempo: Option<TempoMap>,
    caches: Caches,
    /// The automation curves of the last install, by parameter
    curves: HashMap<(NodeId, Arc<str>), Curve>,
    /// The ranges of the parameters of the last install (I-21.2)
    ranges: BTreeMap<(NodeId, Arc<str>), ParamRange>,
    /// Frames the engine has rendered, as the session last reported them (`observe_frames`)
    frames: u64,
}

impl std::fmt::Debug for CompilerCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilerCore")
            .field("generation", &self.generation)
            .field("nodes", &self.nodes.len())
            .field("slots", &self.slot_keys.len())
            .finish_non_exhaustive()
    }
}

impl CompilerCore {
    pub fn new(config: CompilerConfig) -> Self {
        let rack_channel = rack_params(&config.schemas, false);
        let rack_master = rack_params(&config.schemas, true);
        Self {
            node_alloc: NodeAllocator::new(config.limits.max_nodes),
            slot_alloc: SlotAllocator::new(config.limits.max_params),
            config,
            rack_channel,
            rack_master,
            nodes: BTreeMap::new(),
            slot_keys: BTreeMap::new(),
            procs: BTreeMap::new(),
            tails: Vec::new(),
            next_serial: 1,
            next_ticket: 1,
            generation: 0,
            tempo: None,
            caches: Caches::default(),
            curves: HashMap::new(),
            ranges: BTreeMap::new(),
            frames: 0,
        }
    }

    /// A compiler that continues from an installed table (H14: after a panic of the old one). The processors it
    /// had received but not installed are lost: their nodes are requested again
    pub fn from_table(config: CompilerConfig, table: &InstalledTable) -> Self {
        let mut c = Self::new(config);
        let mut used: Vec<NodeIndex> = table.nodes.values().copied().collect();
        used.extend(table.tails.iter().map(|t| t.index));
        c.node_alloc = NodeAllocator::restore(c.config.limits.max_nodes, &used, &table.pending_nodes);
        let used_slots: Vec<ParamSlot> = table.slot_keys.values().copied().collect();
        c.slot_alloc = SlotAllocator::restore(c.config.limits.max_params, &used_slots, &table.pending_slots);
        c.nodes = table.nodes.clone();
        c.slot_keys = table.slot_keys.clone();
        c.ranges = table.ranges.clone();
        for p in &table.processors {
            c.procs.insert(p.node, Proc { installed: Some(p.clone()), requested: None, ready: None, renew: false });
        }
        // Restored tails have no known birth on the audio clock: the first observation stamps them
        let now = UNSTAMPED;
        c.tails = table
            .tails
            .iter()
            .map(|t| Tail {
                serial: t.serial,
                owner: t.owner,
                shape: t.shape.clone(),
                in_place: t.in_place,
                latency: t.latency,
                index: Some(t.index),
                created: now,
                ended: false,
            })
            .collect();
        c.next_serial = table.tails.iter().map(|t| t.serial + 1).max().unwrap_or(1);
        c.generation = table.generation.0;
        c
    }

    pub fn config(&self) -> &CompilerConfig {
        &self.config
    }

    /// The generation of the last install made
    pub fn generation(&self) -> ProgramGen {
        ProgramGen(self.generation)
    }

    /// The parameters of a channel's rack (`master: false`) or of the master's, in their index order
    pub fn rack_params(&self, master: bool) -> &[RackParam] {
        if master { &self.rack_master } else { &self.rack_channel }
    }

    /// An install came back (S6): its removed indices and unbound slots are free from now on
    pub fn returned(&mut self, returned: &Returned) {
        self.returned_parts(returned.generation, &returned.freed);
    }

    pub(crate) fn returned_parts(&mut self, generation: ProgramGen, freed: &[NodeIndex]) {
        for n in freed {
            self.node_alloc.freed(*n);
        }
        self.slot_alloc.returned(generation.0);
    }

    /// A processor built for `ticket` (T6). `false`: the request is stale (superseded or its node gone), and the
    /// core is dropped here
    pub fn ready(&mut self, ticket: Ticket, mut core: Box<NodeCore>) -> bool {
        for proc in self.procs.values_mut() {
            if proc.requested.as_ref().is_some_and(|(t, _, _)| *t == ticket.0) {
                let (t, build, shape) = proc.requested.take().expect("checked");
                let latency = core.processor_mut().latency();
                proc.ready = Some(ReadyCore { ticket: t, build, shape, core, latency });
                return true;
            }
        }
        false
    }

    /// A processor reported a new latency (`PluginRequest::LatencyChanged`, I-9.4): the next compilation aligns
    /// with it. `false` if the node has no processor
    pub fn latency_changed(&mut self, node: NodeKey, latency: u32) -> bool {
        match self.procs.get_mut(&node).and_then(|p| p.installed.as_mut()) {
            Some(i) => {
                i.latency = latency;
                true
            }
            None => false,
        }
    }

    /// Builds the node's processor again at the next compilation, with the same state ("recreate" after a
    /// third-party plugin crashed or hung); the old one stays until the new one is ready. `false` if the node has no
    /// processor
    pub fn renew(&mut self, node: NodeKey) -> bool {
        match self.procs.get_mut(&node) {
            Some(p) if p.installed.is_some() => {
                p.renew = true;
                true
            }
            _ => false,
        }
    }

    /// The plugins and schemas grew (a plugin scan, I-9.6): what compiled before compiles the same. Only additions:
    /// a plugin of the project never leaves the catalog while the session runs
    pub fn set_catalog(&mut self, plugins: Arc<dyn PluginCatalog>, schemas: Arc<SchemaRegistry>) {
        self.rack_channel = rack_params(&schemas, false);
        self.rack_master = rack_params(&schemas, true);
        self.config.plugins = plugins;
        self.config.schemas = schemas;
    }

    /// The renderer reported the end of a tail (`Telemetry::TailEnded`): the next compilation removes it
    pub fn tail_ended(&mut self, node: NodeIndex) -> bool {
        match self.tails.iter_mut().find(|t| t.index == Some(node)) {
            Some(t) => {
                t.ended = true;
                true
            }
            None => false,
        }
    }

    /// The engine's audio clock moved (`TelemetryFrame::frames`). Only the audio clock ages a tail
    pub fn observe_frames(&mut self, frames: u64) {
        self.frames = self.frames.max(frames);
        for t in self.tails.iter_mut().filter(|t| t.created == UNSTAMPED) {
            t.created = self.frames;
        }
    }

    fn tail_net_frames(&self) -> u64 {
        TAIL_NET_SECONDS * u64::from(self.config.spec.sample_rate)
    }

    /// The audio clock value at which the oldest live tail is dropped even without `TailEnded` (the session wakes
    /// the compiler when the clock reaches it)
    pub fn next_tail_deadline(&self) -> Option<u64> {
        let net = self.tail_net_frames();
        self.tails.iter().filter(|t| !t.ended).map(|t| t.created.saturating_add(net)).min()
    }

    /// Whether a compilation would change something without a new root: a processor arrived, a tail ended or
    /// expired
    pub fn has_pending_work(&self) -> bool {
        let net = self.tail_net_frames();
        self.procs.values().any(|p| p.ready.is_some())
            || self.tails.iter().any(|t| t.ended || self.frames >= t.created.saturating_add(net))
    }

    /// How much of the engine `root` needs, counted exactly as the compilation does
    pub fn usage(&mut self, root: &Value) -> Result<Usage> {
        let plan = self.plan(root, &crate::catalog::NoResources, &Environment::default())?;
        let mut slots: BTreeSet<(NodeId, Arc<str>, u8)> = BTreeSet::new();
        for p in &plan.params {
            slots.insert((p.owner, p.key.clone(), 0));
        }
        for h in plan.host_params() {
            if let HPlan::Param { owner, key, instance, .. } = h {
                slots.insert((*owner, key.clone(), *instance));
            }
        }
        for (_, track, number, _) in &plan.ccs {
            slots.insert((*track, Arc::from(format!("cc.{number}")), 0));
        }
        for m in &plan.mods {
            for k in lm_doc::schemas::modulator_keys(m.kind_name) {
                slots.insert((m.id, key(k), 0));
            }
        }
        Ok(Usage { nodes: plan.nodes.len(), params: slots.len() })
    }

    // -----------------------------------------------------------------------------------------------------------
    // Compiling

    /// Compiles `root` into an install, the build requests and the tables. On an error nothing changes (the
    /// audio keeps the previous program)
    pub fn compile(&mut self, root: &Value, resources: &dyn Resources, env: &Environment) -> Result<Compiled> {
        let started = Instant::now();
        let s = settings(root);
        if s.sample_rate != self.config.spec.sample_rate {
            return Err(Error::new("compile.sample_rate_mismatch")
                .arg("project", i64::from(s.sample_rate))
                .arg("engine", i64::from(self.config.spec.sample_rate)));
        }
        let mut plan = self.plan(root, resources, env)?;
        let tempo = tempo_map(root, s)?;
        self.pdc(&mut plan)?;

        // Limits (H12)
        let limits = self.config.limits;
        if plan.nodes.len() > limits.max_nodes {
            return Err(Error::new("compile.limits.nodes")
                .arg("used", plan.nodes.len() as i64)
                .arg("max", limits.max_nodes as i64));
        }
        let mut wanted_slots: Vec<(NodeId, Arc<str>, u8)> = Vec::new();
        {
            let mut seen = HashSet::new();
            for p in &plan.params {
                let k = (p.owner, p.key.clone(), 0u8);
                if seen.insert(k.clone()) {
                    wanted_slots.push(k);
                }
            }
            for (_, track, number, _) in &plan.ccs {
                let k = (*track, Arc::from(format!("cc.{number}")), 0u8);
                if seen.insert(k.clone()) {
                    wanted_slots.push(k);
                }
            }
            for h in plan.host_params() {
                if let HPlan::Param { owner, key, instance, .. } = h {
                    let k = (*owner, key.clone(), *instance);
                    if seen.insert(k.clone()) {
                        wanted_slots.push(k);
                    }
                }
            }
            for m in &plan.mods {
                for k in lm_doc::schemas::modulator_keys(m.kind_name) {
                    let k = (m.id, key(k), 0u8);
                    if seen.insert(k.clone()) {
                        wanted_slots.push(k);
                    }
                }
            }
        }
        if wanted_slots.len() > limits.max_params {
            return Err(Error::new("compile.limits.params")
                .arg("used", wanted_slots.len() as i64)
                .arg("max", limits.max_params as i64));
        }

        // Indices: kept by key, new ones taken from a copy of the allocator (committed only if all goes well)
        let generation = self.generation + 1;
        let mut node_alloc = self.node_alloc.clone();
        let fresh: HashSet<NodeKey> = plan.procs.iter().filter(|p| p.fresh).map(|p| p.key).collect();
        let mut index_of: Vec<NodeIndex> = Vec::with_capacity(plan.nodes.len());
        let mut removed: Vec<NodeIndex> = Vec::new();
        let mut new_nodes: BTreeMap<NodeKey, NodeIndex> = BTreeMap::new();
        let mut tail_index: HashMap<u64, NodeIndex> = HashMap::new();
        for n in &plan.nodes {
            let existing = match n.key {
                NodeKey::Tail(serial) => self.tails.iter().find(|t| t.serial == serial).and_then(|t| t.index),
                k => self.nodes.get(&k).copied().filter(|_| !fresh.contains(&k)),
            };
            let idx = match existing {
                Some(i) => i,
                None => node_alloc.alloc()?,
            };
            index_of.push(idx);
            match n.key {
                NodeKey::Tail(serial) => {
                    tail_index.insert(serial, idx);
                }
                k => {
                    new_nodes.insert(k, idx);
                }
            }
        }
        for (k, i) in &self.nodes {
            if new_nodes.get(k) != Some(i) {
                node_alloc.release(*i);
                removed.push(*i);
            }
        }
        for t in &self.tails {
            if let Some(i) = t.index
                && !tail_index.contains_key(&t.serial)
            {
                node_alloc.release(i);
                removed.push(i);
            }
        }

        // Slots: kept by key, new ones from a copy
        let mut slot_alloc = self.slot_alloc.clone();
        let mut slot_keys: BTreeMap<(NodeId, Arc<str>, u8), ParamSlot> = BTreeMap::new();
        for k in wanted_slots {
            let slot = match self.slot_keys.get(&k) {
                Some(s) => *s,
                None => slot_alloc.alloc()?,
            };
            slot_keys.insert(k, slot);
        }
        for (k, s) in &self.slot_keys {
            if !slot_keys.contains_key(k) {
                slot_alloc.release(*s, generation);
            }
        }

        // The program
        let mut b = ProgramBuilder::new(ProgramGen(generation), self.config.spec, &limits, tempo.clone());
        let slot_of =
            |owner: NodeId, key: &Arc<str>, instance: u8| -> ParamSlot { slot_keys[&(owner, key.clone(), instance)] };
        let host = |h: &HPlan, curves: &HashMap<(NodeId, Arc<str>), Curve>| -> HostParam {
            match h {
                HPlan::Fixed(v) => HostParam::fixed(*v),
                HPlan::Param { owner, key, instance, value, complement } => {
                    let mut p = HostParam::slot(slot_of(*owner, key, *instance), *value);
                    if *complement {
                        p = p.complement();
                    }
                    match curves.get(&(*owner, key.clone())) {
                        Some(c) => p.automated(c.clone()),
                        None => p,
                    }
                }
            }
        };
        for (i, n) in plan.nodes.iter().enumerate() {
            let idx = index_of[i];
            let mut d = if n.sum {
                NodeDesc::sum(idx, n.inputs)
            } else if n.note_effect {
                NodeDesc::note_effect(idx)
            } else if n.in_place {
                NodeDesc::insert(idx, n.inputs)
            } else {
                NodeDesc::processor(idx, n.inputs, n.outputs)
            };
            if n.sidechain > 0 {
                d = d.with_sidechain(n.sidechain);
            }
            if let Some(g) = &n.gain {
                d = d.with_strip(Strip {
                    gain: host(g, &plan.curves),
                    pan: n.pan.as_ref().map(|p| host(p, &plan.curves)),
                });
            }
            d = d.muted(n.muted);
            if n.tail {
                d = d.tail();
            }
            b.node(d);
        }
        for f in &plan.feeds {
            let mut feed = Feed::new(index_of[f.from], index_of[f.to], f.channels)
                .channels_from(f.from_channel)
                .channels_to(f.to_channel)
                .mix(f.mix)
                .delay(f.delay);
            if f.sidechain {
                feed = feed.to_sidechain();
            }
            if let Some(g) = &f.gain {
                feed = feed.gain(host(g, &plan.curves));
            }
            b.feed(feed);
        }
        for p in &plan.params {
            let slot = slot_of(p.owner, &p.key, 0);
            let curve = plan.curves.get(&(p.owner, p.key.clone())).cloned();
            b.param(
                slot,
                index_of[p.node],
                p.index,
                ParamInfo { rate: p.rate, discrete: p.discrete, initial: p.value },
                curve,
            );
            b.param_limits(index_of[p.node], p.index, p.range.min, p.range.max);
        }
        for pm in &plan.mods {
            let mut spec = ModulatorSpec::new(pm.kind, pm.seed);
            spec.initial = pm.params;
            spec.user_wave = pm.user_wave.clone();
            spec.input = pm.input.map(|i| index_of[i]);
            spec.trigger = pm.trigger.map(|i| index_of[i]);
            for (node, param, amount, min, max) in &pm.targets {
                let k = (*node, param.clone(), 0u8);
                let Some(&slot) = slot_keys.get(&k) else { continue };
                let range = plan
                    .params
                    .iter()
                    .find(|p| p.owner == *node && p.key == *param)
                    .map(|p| p.range)
                    .or_else(|| {
                        plan.host_params()
                            .any(|h| matches!(h, HPlan::Param { owner, key, .. } if owner == node && key == param))
                            .then(|| host_range(param))
                    })
                    .or_else(|| param.strip_prefix("cc.").is_some().then(|| ParamRange::linear(0.0, 127.0)));
                let Some(range) = range else { continue };
                spec.targets.push(ModTarget { slot, range, amount: *amount, min: *min, max: *max });
            }
            let mi = b.modulator(spec);
            for k in lm_doc::schemas::modulator_keys(pm.kind_name) {
                let Some(index) = MOD_KEYS.iter().position(|x| x == k) else { continue };
                let slot = slot_of(pm.id, &key(k), 0);
                let stepped =
                    matches!(*k, "wave" | "multiplier" | "sync" | "syncNumerator" | "syncDenominator" | "abs");
                let curve = plan.curves.get(&(pm.id, key(k))).cloned();
                b.modulator_param(
                    slot,
                    mi,
                    index,
                    ParamInfo { rate: Rate::Block, discrete: stepped, initial: pm.params[index] },
                    curve,
                );
            }
        }
        for ((owner, key, _), slot) in &slot_keys {
            if let Some(mode) = plan.modes.get(&(*owner, key.clone()))
                && *mode != AutomationMode::Read
            {
                b.automation_mode(*slot, *mode);
            }
        }
        for (n, items) in std::mem::take(&mut plan.audio) {
            b.audio_items(index_of[n], items);
        }
        for (n, items) in std::mem::take(&mut plan.midi) {
            b.midi_items(index_of[n], items);
        }
        for (n, effects) in &plan.chains {
            b.note_chain(index_of[*n], effects.iter().map(|e| index_of[*e]).collect());
        }
        for (n, chans) in std::mem::take(&mut plan.inputs) {
            b.input(index_of[n], chans);
        }
        for (port, n, channel, fixed_velocity) in &plan.ports {
            b.port(PortRoute { port: *port, channel: *channel, node: index_of[*n], fixed_velocity: *fixed_velocity });
        }
        for port in std::mem::take(&mut plan.mpe) {
            b.mpe_port(port);
        }
        for (n, keys) in std::mem::take(&mut plan.live_keys) {
            b.live_keys(index_of[n], keys);
        }
        for (n, port, channel, fixed_velocity, fixed_note) in &plan.midi_out {
            b.midi_out(MidiOutRoute {
                node: index_of[*n],
                port: *port,
                channel: *channel,
                fixed_velocity: *fixed_velocity,
                fixed_note: *fixed_note,
            });
        }
        for (n, track, number, value) in &plan.ccs {
            let key: Arc<str> = Arc::from(format!("cc.{number}"));
            let curve = plan.curves.get(&(*track, key.clone())).cloned();
            b.cc_param(slot_of(*track, &key, 0), index_of[*n], 0, *number, *value, curve);
        }
        for (n, channel, device_channel) in &plan.outputs {
            b.output(OutputRoute { node: index_of[*n], channel: *channel, device_channel: *device_channel });
        }
        if let Some(m) = plan.master {
            b.master(index_of[m]);
        }
        if let Some(p) = plan.preview {
            b.preview_bus(index_of[p]);
        }
        if let Some((n, m)) = plan.metronome.take() {
            b.metronome(index_of[n], m);
        }
        let mut transport = plan.transport;
        transport.midi = env.midi_timing;
        b.transport(transport);
        let program = b.build()?;

        // ---- Nothing can fail from here: commit ----
        let mut install = Install::new(program);
        if let Some(old) = &self.tempo
            && *old != tempo
        {
            install.relocate(Relocate { tick: old.tick_at(env.playhead) });
        }
        let mut builds = Vec::new();
        let mut cancelled = Vec::new();
        let mut waiting = Vec::new();
        let mut replacing = Vec::new();
        let now = self.frames;
        let planned: HashSet<NodeKey> = plan.procs.iter().map(|p| p.key).collect();
        // Processors whose node is gone
        let gone: Vec<NodeKey> = self.procs.keys().filter(|k| !planned.contains(k)).copied().collect();
        for k in gone {
            if let Some(p) = self.procs.remove(&k) {
                if let Some((t, _, _)) = p.requested {
                    cancelled.push(Ticket(t));
                }
                if let Some(r) = p.ready {
                    cancelled.push(Ticket(r.ticket));
                }
            }
        }
        for pp in plan.procs.drain(..) {
            let idx = new_nodes[&pp.key];
            let proc = self.procs.entry(pp.key).or_default();
            if pp.fresh {
                proc.installed = None;
            }
            if pp.use_ready {
                let r = proc.ready.take().expect("planned with a ready core");
                proc.renew = false;
                let installed = InstalledProcessor { node: pp.key, build: r.build, shape: r.shape, latency: r.latency };
                match proc.installed.replace(installed) {
                    Some(old) => {
                        // The old one may go on in a tail (U32); a note effect has none planned (no sound to leave)
                        let tail = plan
                            .new_tails
                            .iter()
                            .find(|t| t.1 == pp.key)
                            .and_then(|t| tail_index.get(&t.0).copied());
                        install.replace(idx, r.core, tail);
                        let _ = old;
                    }
                    None => {
                        install.add(idx, r.core);
                    }
                }
            } else if let Some(r) = &proc.ready {
                // A core for something no longer wanted
                let stale = pp.wanted.as_ref().is_none_or(|(b, _)| !r.build.same_processor(b));
                if stale {
                    cancelled.push(Ticket(r.ticket));
                    proc.ready = None;
                }
            }
            match (&pp.wanted, pp.request) {
                (Some((build, shape)), true) => {
                    if let Some((t, _, _)) = proc.requested.take() {
                        cancelled.push(Ticket(t));
                    }
                    let ticket = self.next_ticket;
                    self.next_ticket += 1;
                    proc.requested = Some((ticket, build.clone(), shape.clone()));
                    builds.push(BuildRequest {
                        ticket: Ticket(ticket),
                        node: pp.key,
                        build: build.clone(),
                        seed: pp.seed,
                        spec: self.config.spec,
                        shape: shape.clone(),
                    });
                }
                (None, _) => {
                    if let Some((t, _, _)) = proc.requested.take() {
                        cancelled.push(Ticket(t));
                    }
                }
                _ => {}
            }
            if pp.waiting {
                waiting.push(pp.key);
            }
            if pp.replacing {
                replacing.push(pp.key);
            }
        }
        for n in &removed {
            install.remove(*n);
        }
        // Tails: the kept ones, then the new ones
        self.tails.retain(|t| tail_index.contains_key(&t.serial));
        for (serial, owner, shape, in_place, latency) in plan.new_tails.drain(..) {
            self.next_serial = self.next_serial.max(serial + 1);
            // A tail with nowhere to sound was not put in the program: the old processor just comes back
            if let Some(index) = tail_index.get(&serial).copied() {
                self.tails.push(Tail {
                    serial,
                    owner,
                    shape,
                    in_place,
                    latency,
                    index: Some(index),
                    created: now,
                    ended: false,
                });
            }
        }
        self.nodes = new_nodes;
        self.node_alloc = node_alloc;
        self.slot_keys = slot_keys;
        self.slot_alloc = slot_alloc;
        self.generation = generation;
        self.tempo = Some(tempo);
        self.caches.sweep();
        self.curves = std::mem::take(&mut plan.curves);
        let mut ranges = BTreeMap::new();
        for p in &plan.params {
            ranges.insert((p.owner, p.key.clone()), p.range);
        }
        for h in plan.host_params() {
            if let HPlan::Param { owner, key, .. } = h {
                ranges.insert((*owner, key.clone()), host_range(key));
            }
        }
        for (_, track, number, _) in &plan.ccs {
            ranges.insert(
                (*track, Arc::from(format!("cc.{number}"))),
                ParamRange { min: 0.0, max: 127.0, scale: RangeScale::Stepped },
            );
        }
        self.ranges = ranges;
        let made = self.caches.made;
        let table = Arc::new(self.table());
        let routes = plan
            .feeds
            .iter()
            .map(|f| Route {
                from: plan.nodes[f.from].key,
                to: plan.nodes[f.to].key,
                from_channel: f.from_channel,
                channels: f.channels,
                to_channel: f.to_channel,
                sidechain: f.sidechain,
                monitor: f.monitor,
                delay: f.delay,
            })
            .collect();
        let ports = plan.ports.iter().map(|(p, n, _, _)| (*p, plan.nodes[*n].key)).collect();
        let nodes = plan
            .nodes
            .iter()
            .map(|n| NodeInfo { key: n.key, muted: n.muted, latency: n.latency, channels: n.outputs })
            .collect();
        Ok(Compiled {
            install,
            generation: ProgramGen(generation),
            builds,
            cancelled,
            table,
            waiting,
            replacing,
            waiting_items: plan.waiting_items,
            routes,
            nodes,
            ports,
            low_latency_bypassed: plan.low_latency_bypassed,
            streams: plan.streams,
            stretches: plan.stretches,
            made,
            elapsed: started.elapsed(),
        })
    }

    /// The tables of the last install made
    pub fn table(&self) -> InstalledTable {
        let mut slots: BTreeMap<(NodeId, Arc<str>), Vec<ParamSlot>> = BTreeMap::new();
        for ((owner, key, _), slot) in &self.slot_keys {
            slots.entry((*owner, key.clone())).or_default().push(*slot);
        }
        InstalledTable {
            generation: ProgramGen(self.generation),
            nodes: self.nodes.clone(),
            slots,
            processors: self.procs.values().filter_map(|p| p.installed.clone()).collect(),
            tails: self
                .tails
                .iter()
                .filter_map(|t| {
                    Some(InstalledTail {
                        serial: t.serial,
                        index: t.index?,
                        owner: t.owner,
                        shape: t.shape.clone(),
                        in_place: t.in_place,
                        latency: t.latency,
                    })
                })
                .collect(),
            pending_nodes: self.node_alloc.pending(),
            pending_slots: self.slot_alloc.pending(),
            slot_keys: self.slot_keys.clone(),
            ranges: self.ranges.clone(),
        }
    }

    /// A streamed item still waits for the disk where the playhead needs it (the export waits, T8)
    pub fn streams_pending(&self) -> bool {
        self.caches.streams_pending()
    }

    /// The automation curve of a parameter in the last install (in samples)
    pub fn curve(&self, node: NodeId, key: &str) -> Option<&Curve> {
        self.curves.get(&(node, Arc::from(key)))
    }

    /// How many node indices and slots are free right now (not counting the pending ones)
    pub fn free(&self) -> (usize, usize) {
        (self.node_alloc.free_count(), self.slot_alloc.free_count())
    }

    // -----------------------------------------------------------------------------------------------------------
    // Planning

    /// Decides a processor node: what it should be, and what plays meanwhile. Returns the shape the program gives
    /// it and its latency
    fn decide(
        &self,
        plan: &mut Plan,
        key: NodeKey,
        wanted: Option<(Build, PluginShape)>,
        seed: u64,
        fallback: PluginShape,
    ) -> (PluginShape, u32) {
        let proc = self.procs.get(&key);
        let installed = proc.and_then(|p| p.installed.as_ref());
        let ready = proc.and_then(|p| p.ready.as_ref());
        let requested = proc.and_then(|p| p.requested.as_ref());
        let renew = proc.is_some_and(|p| p.renew);
        let mut pp = ProcPlan {
            key,
            wanted: None,
            use_ready: false,
            request: false,
            waiting: false,
            replacing: false,
            fresh: false,
            seed,
        };
        let out = match &wanted {
            None => {
                pp.fresh = installed.is_some();
                (fallback, 0)
            }
            Some((build, shape)) => {
                if let Some(r) = ready.filter(|r| r.build.same_processor(build)) {
                    pp.use_ready = true;
                    // (a note effect has no sound to leave in a tail)
                    if let Some(old) = installed.filter(|o| !o.shape.note_effect) {
                        let serial = self.next_serial + plan.new_tails.len() as u64;
                        let in_place = PNode::processor(key, &old.shape, 0).in_place;
                        plan.new_tails.push((serial, key, old.shape.clone(), in_place, old.latency));
                    }
                    (r.shape.clone(), r.latency)
                } else if let Some(i) = installed.filter(|i| !renew && i.build.same_processor(build)) {
                    (i.shape.clone(), i.latency)
                } else {
                    pp.request = !requested.is_some_and(|(_, b, _)| b.same_processor(build));
                    match installed {
                        Some(i) => {
                            pp.replacing = true;
                            (i.shape.clone(), i.latency)
                        }
                        None => {
                            pp.waiting = true;
                            (shape.clone(), 0)
                        }
                    }
                }
            }
        };
        pp.wanted = wanted;
        plan.procs.push(pp);
        out
    }

    /// The plugin build and shape of a plugin node of the tree
    fn plugin_build(&self, node: &Value, width: u16) -> Option<(Build, PluginShape)> {
        let id = id_of(node)?;
        let plugin = text(node, "plugin", "");
        let version = u32::try_from(int(node, "version", 1)).unwrap_or(1);
        let state = node.get("state").cloned().unwrap_or_else(Value::empty_object);
        let catalog = &self.config.plugins;
        let shape = catalog.shape(plugin, version, &state, width)?;
        let channels = if catalog.is_effect(plugin) { width } else { 0 };
        Some((Build::Plugin { node: id, plugin: Arc::from(plugin), version, state, channels }, shape))
    }

    fn plugin_params(&self, plan: &mut Plan, node_index: usize, node: &Value) {
        let Some(id) = id_of(node) else { return };
        let plugin = text(node, "plugin", "");
        let Some(params) = self.config.plugins.params(plugin) else { return };
        let values = node.get("state").and_then(|s| s.get(lm_state::PARAMS_KEY));
        for (i, spec) in params.iter().enumerate() {
            let value = values.and_then(|v| v.get(spec.key)).and_then(Value::as_real).unwrap_or(spec.def);
            plan.params.push(PParam {
                node: node_index,
                index: u32::try_from(i).unwrap_or(u32::MAX),
                owner: id,
                key: key(spec.key),
                rate: rate_of(spec),
                discrete: spec.scale == Scale::Stepped,
                value: value as f32,
                range: ParamRange {
                    min: spec.min as f32,
                    max: spec.max as f32,
                    scale: match spec.scale {
                        Scale::Linear => RangeScale::Linear,
                        Scale::Log => RangeScale::Log,
                        Scale::Stepped => RangeScale::Stepped,
                    },
                },
            });
        }
    }

    fn plan(&mut self, root: &Value, resources: &dyn Resources, env: &Environment) -> Result<Plan> {
        let s = settings(root);
        let tempo = tempo_map(root, s)?;
        if self.tempo.as_ref().is_some_and(|t| *t != tempo) {
            // The caches that depend on the map are made again (the version is bumped for good at the commit)
            self.caches.tempo_version += 1;
        }
        self.caches.begin();
        let mut plan = Plan { mpe: env.mpe_ports.clone(), ..Plan::default() };
        let mixer = root.get("mixer").cloned().unwrap_or_default();
        let analog = text(&mixer, "mode", "analog") != "digital";
        let transport = root.get("transport").cloned().unwrap_or_default();
        let project_transpose = int(&transport, "transpose", 0).clamp(-12, 12) as i32;

        // The desk, read first (the tracks send to it)
        let desk: Vec<ChannelView<'_>> = items(&mixer, "channels")
            .filter_map(|c| Some(ChannelView { id: id_of(c)?, value: c, master: false, width: channels(c, 2) }))
            .collect();
        let master_value = mixer.get("master");
        let master_view =
            master_value.map(|m| ChannelView { id: MASTER, value: m, master: true, width: channels(m, 2) });
        let by_id: HashMap<NodeId, usize> = desk.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let width_of = |id: NodeId| -> u16 {
            match by_id.get(&id) {
                Some(i) => desk[*i].width,
                None => master_view.as_ref().map_or(2, |m| m.width),
            }
        };
        let dest_of = |id: Option<NodeId>| -> NodeId {
            match id {
                Some(id) if by_id.contains_key(&id) => id,
                _ => MASTER,
            }
        };
        // Insert → its channel
        let mut insert_owner: HashMap<NodeId, NodeId> = HashMap::new();
        for c in desk.iter().chain(master_view.iter()) {
            for p in items(c.value, "inserts") {
                if let Some(pid) = id_of(p) {
                    insert_owner.insert(pid, c.id);
                }
            }
        }

        // Channel solo (base item 2)
        let (solo_muted, keyed) = channel_solo(&desk, &dest_of, &insert_owner);

        // Track solo: a soloed folder covers its children
        let tracks: Vec<&Value> = items(root, "tracks").collect();
        let parent_of: HashMap<NodeId, Option<NodeId>> =
            tracks.iter().filter_map(|t| Some((id_of(t)?, ref_of(t, "parent")))).collect();
        let soloed_tracks: HashSet<NodeId> =
            tracks.iter().filter(|t| flag(t, "solo", false)).filter_map(|t| id_of(t)).collect();
        let track_audible = |id: NodeId| -> bool {
            if soloed_tracks.is_empty() {
                return true;
            }
            let mut cur = Some(id);
            let mut depth = 0;
            while let Some(c) = cur {
                if soloed_tracks.contains(&c) {
                    return true;
                }
                depth += 1;
                if depth > tracks.len() {
                    break;
                }
                cur = parent_of.get(&c).copied().flatten();
            }
            false
        };

        // Automation lanes, by target (the first lane of a parameter wins)
        let mut lanes: Vec<(&Value, NodeId)> = Vec::new();
        for t in &tracks {
            for l in items(t, "automation") {
                if let Some(id) = id_of(l) {
                    lanes.push((l, id));
                }
            }
        }
        for c in desk.iter().chain(master_view.iter()) {
            for l in items(c.value, "automation") {
                if let Some(id) = id_of(l) {
                    lanes.push((l, id));
                }
            }
        }
        let plugin_of: HashMap<NodeId, String> = {
            let mut m = HashMap::new();
            for t in &tracks {
                if let Some(i) = t.get("instrument").filter(|i| !i.is_null())
                    && let Some(id) = id_of(i)
                {
                    m.insert(id, text(i, "plugin", "").to_owned());
                }
                for e in items(t, "noteEffects") {
                    if let Some(id) = id_of(e) {
                        m.insert(id, text(e, "plugin", "").to_owned());
                    }
                }
            }
            for c in desk.iter().chain(master_view.iter()) {
                for p in items(c.value, "inserts") {
                    if let Some(id) = id_of(p) {
                        m.insert(id, text(p, "plugin", "").to_owned());
                    }
                }
            }
            m
        };
        // The global automation button wins over each lane's mode (I-21.5)
        let global = root.get("transport").and_then(|t| t.get("automationMode")).and_then(Value::as_text);
        for (lane, lane_id) in lanes {
            let mode = match global.filter(|g| *g != "lanes") {
                Some(g) => g,
                None => lane.get("mode").and_then(Value::as_text).unwrap_or("read"),
            };
            let mode = match mode {
                "off" => continue,
                "touch" => AutomationMode::Touch,
                "latch" => AutomationMode::Latch,
                "write" => AutomationMode::Write,
                _ => AutomationMode::Read,
            };
            let Some(target) = lane.get("target") else { continue };
            let Some(node) = ref_of(target, "node") else { continue };
            let param = text(target, "param", "");
            let canonical: Arc<str> = if node.kind == KIND_PLUGIN {
                let Some(plugin) = plugin_of.get(&node) else { continue };
                match self.config.plugins.params(plugin).and_then(|ps| {
                    ps.iter().find(|p| p.key == param).or_else(|| ps.iter().find(|p| p.aliases.contains(&param)))
                }) {
                    Some(p) => key(p.key),
                    None => continue,
                }
            } else {
                key(param)
            };
            if plan.curves.contains_key(&(node, canonical.clone())) {
                continue;
            }
            if let Some(curve) = self.caches.curve(lane_id, lane, &tempo)? {
                plan.modes.insert((node, canonical.clone()), mode);
                plan.curves.insert((node, canonical), curve);
            }
        }

        // The metronome and the preview bus come first
        let master_devices = master_value.map(|m| device_channels(m.get("output").and_then(|o| o.get("channels"))));
        let metro = plan.add(PNode::sum(NodeKey::Metronome, 2));
        if let Some((accent, beat)) = &env.metronome_sounds
            && env.metronome_when != crate::types::MetronomeWhen::Never
            && accent.rate() == s.sample_rate
            && beat.rate() == s.sample_rate
        {
            let m = transport.get("metronome").cloned().unwrap_or_default();
            plan.metronome = Some((
                metro,
                Metronome {
                    accent: accent.clone(),
                    beat: beat.clone(),
                    gain: real(&m, "volume", 0.8) as f32,
                    enabled: flag(&m, "on", false),
                    only_recording: env.metronome_when == crate::types::MetronomeWhen::Recording,
                },
            ));
        }
        let metro_out = env.metronome_output.clone().or_else(|| master_devices.clone()).unwrap_or_default();
        for (i, dev) in metro_out.iter().enumerate() {
            plan.outputs.push((metro, (i as u16).min(1), *dev));
        }
        let preview = plan.add(PNode::sum(NodeKey::Preview, 2));
        plan.preview = Some(preview);

        // Channel inputs exist before anything sends to them
        for c in desk.iter().chain(master_view.iter()) {
            plan.add(PNode::sum(NodeKey::ChannelIn(c.id), c.width));
        }
        let channel_in = |plan: &Plan, id: NodeId| plan.index[&NodeKey::ChannelIn(id)];
        if let Some(m) = &master_view {
            let to = channel_in(&plan, MASTER);
            plan.feed(preview, 0, 2, to, 0, m.width, false);
        }

        // ---- Tracks
        for (ordinal, t) in tracks.iter().enumerate() {
            let Some(tid) = id_of(t) else { continue };
            if flag(t, "folder", false) {
                continue;
            }
            let width = channels(t, 2);
            let muted = flag(t, "mute", false) || !track_audible(tid);
            let volume = real(t, "volume", 1.0) as f32;
            let vol = |instance: u8| HPlan::Param {
                owner: tid,
                key: key("volume"),
                instance,
                value: volume,
                complement: false,
            };
            // A bypassed instrument is not built nor run (safe mode, U21): the track plays its audio items only
            let instrument = t.get("instrument").filter(|i| !i.is_null() && !flag(i, "bypass", false));
            // An item played alone by an editor (`env.audition`) silences every other one
            let audible = |i: &Value| env.audition.is_none_or(|a| id_of(i) == Some(a.item));
            let audio_items: Vec<&Value> = items(t, "items")
                .filter(|i| text(i, "kind", "midi") == "audio" && !flag(i, "mute", false) && audible(i))
                .collect();

            // The source and the gain node, and the port layout of the gain node
            let mut port_layout: Option<PluginShape> = None;
            let gain;
            if let Some(inst) = instrument {
                let pid = id_of(inst);
                let wanted = self.plugin_build(inst, width);
                let fallback = PluginShape::instrument(Vec::new());
                let seed = pid.map_or(0, |p| Rng::seed_for(s.seed, id_number(p)));
                let Some(pid) = pid else { continue };
                let k = NodeKey::Instrument(pid);
                let (shape, latency) = self.decide(&mut plan, k, wanted, seed, fallback);
                let p = plan.add(PNode::processor(k, &shape, latency));
                self.plugin_params(&mut plan, p, inst);
                let total = shape.output_channels().max(1);
                let mut g = PNode::sum(NodeKey::TrackGain(tid), total);
                g.gain = Some(vol(0));
                g.muted = muted;
                gain = plan.add(g);
                let out = shape.output_channels();
                if out > 0 {
                    plan.feed(p, 0, out, gain, 0, out, false);
                }
                // Notes
                if shape.notes {
                    let transpose = if flag(t, "transpose", true) { project_transpose } else { 0 };
                    // The keys, base key, base velocity and microtuner (F-IJN-017/019/021)
                    let keys = crate::keys::track_keys(t, resources, transpose);
                    let mut list = Vec::new();
                    for item in items(t, "items")
                        .filter(|i| text(i, "kind", "midi") == "midi" && !flag(i, "mute", false) && audible(i))
                    {
                        if let Some(iid) = id_of(item) {
                            list.push(self.caches.midi(iid, item, &keys, &tempo)?);
                        }
                    }
                    if !list.is_empty() {
                        plan.midi.push((p, list));
                    }
                    if keys != lm_render::LiveKeys::default() {
                        plan.live_keys.push((p, keys));
                    }
                    // The interface's own live notes (the instrument window's keyboard, the computer keyboard, the
                    // piano roll, I-15 `note_on`): one port per track, always routed, armed or not
                    plan.ports.push((lm_render::gui_port(ordinal), p, None, None));
                    // The live input (I-21.1, F-IJN-017): armed or monitored, or following the selection; its ports
                    // (the instrument ports when none), its channel and fixed velocity
                    let input = t.get("midiInput").filter(|i| !i.is_null());
                    let follows = input.is_some_and(|i| flag(i, "followSelection", false));
                    let listening = flag(t, "armed", false)
                        || text(t, "monitor", "auto") == "on"
                        || (follows && env.selected_track == Some(tid));
                    if listening {
                        let aliases: Vec<&str> =
                            input.map(|i| items(i, "ports").filter_map(Value::as_text).collect()).unwrap_or_default();
                        let ports: Vec<PortIndex> = if aliases.is_empty() {
                            env.midi_ports.clone()
                        } else {
                            aliases
                                .iter()
                                .filter_map(|a| env.midi_aliases.iter().find(|(x, _)| &**x == *a).map(|(_, p)| *p))
                                .collect()
                        };
                        let channel = input.map_or(-1, |i| int(i, "channel", -1));
                        let channel = u8::try_from(channel).ok().filter(|c| *c < 16);
                        let fixed = input.and_then(|i| i.get("fixedVelocity")).and_then(Value::as_int);
                        let fixed = fixed.and_then(|v| u8::try_from(v).ok()).filter(|v| (1..128).contains(v));
                        for port in ports {
                            plan.ports.push((port, p, channel, fixed));
                        }
                    }
                }
                // The note effects (before the instrument): what the sources push for it goes through them in order;
                // a bypassed one is not in the chain (and a chain that changes releases the notes: the renderer)
                if shape.notes {
                    let mut chain = Vec::new();
                    for e in items(t, "noteEffects").filter(|e| !flag(e, "bypass", false)) {
                        let Some(eid) = id_of(e) else { continue };
                        let ek = NodeKey::NoteEffect(eid);
                        let eseed = Rng::seed_for(s.seed, id_number(eid));
                        let wanted = self.plugin_build(e, width);
                        let (eshape, elatency) = self.decide(&mut plan, ek, wanted, eseed, PluginShape::note_effect());
                        let n = plan.add(PNode::processor(ek, &eshape, elatency));
                        self.plugin_params(&mut plan, n, e);
                        chain.push(n);
                    }
                    if !chain.is_empty() {
                        plan.chains.push((p, chain));
                    }
                }
                // MIDI out to devices (P-38): what the instrument plays, on the output's channel
                if let Some(o) = t.get("midiOutput").filter(|o| !o.is_null() && flag(o, "on", true)) {
                    let channel = int(o, "channel", 0).clamp(0, 15) as u8;
                    let seven = |k: &str| {
                        o.get(k).and_then(Value::as_int).and_then(|v| u8::try_from(v).ok()).filter(|v| *v < 128)
                    };
                    let (velocity, note) = (seven("fixedVelocity").filter(|v| *v > 0), seven("fixedNote"));
                    for alias in items(o, "ports").filter_map(Value::as_text) {
                        if let Some((_, port)) = env.midi_aliases.iter().find(|(x, _)| &**x == alias) {
                            plan.midi_out.push((p, *port, channel, velocity, note));
                        }
                    }
                }
                // The CC rack (F-IJN-025): every controller set, and every one with a lane, to the instrument
                if t.get("ccRack").is_some_and(|r| flag(r, "on", false)) {
                    let mut numbers: Vec<(u8, f32)> = t
                        .get("cc")
                        .map(|c| {
                            c.entries()
                                .filter_map(|(k, v)| {
                                    let n = k.strip_prefix('c')?.parse::<u8>().ok().filter(|n| *n < 128)?;
                                    Some((n, v.as_real()? as f32))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    for lane in items(t, "automation") {
                        let target = lane.get("target");
                        let on_this = target.and_then(|x| ref_of(x, "node")) == Some(tid);
                        let n = target
                            .and_then(|x| x.get("param"))
                            .and_then(Value::as_text)
                            .and_then(|k| k.strip_prefix("cc."))
                            .and_then(|n| n.parse::<u8>().ok())
                            .filter(|n| *n < 128);
                        if let (true, Some(n)) = (on_this, n)
                            && !numbers.iter().any(|(m, _)| *m == n)
                        {
                            numbers.push((n, 0.0));
                        }
                    }
                    numbers.sort_by_key(|(n, _)| *n);
                    for (n, v) in numbers {
                        plan.ccs.push((p, tid, n, v.clamp(0.0, 127.0)));
                    }
                }
                port_layout = Some(shape);
            } else {
                let mut a = PNode::sum(NodeKey::TrackAudio(tid), width);
                a.gain = Some(vol(0));
                a.muted = muted;
                gain = plan.add(a);
            }
            // Audio items
            if !audio_items.is_empty() || instrument.is_none() {
                let a = if instrument.is_some() {
                    let a = plan.add(PNode::sum(NodeKey::TrackAudio(tid), width));
                    let (start, chans) = port_layout
                        .as_ref()
                        .and_then(|l| l.outputs.first().map(|p| (0u16, crate::catalog::clamp_channels(p.channels))))
                        .unwrap_or((0, plan.nodes[gain].inputs));
                    plan.feed(a, 0, width, gain, start, chans, false);
                    a
                } else {
                    gain
                };
                let mut list = Vec::new();
                for item in &audio_items {
                    let Some(iid) = id_of(item) else { continue };
                    let (segments, waiting, streams, stretches) =
                        self.caches.audio(iid, item, &tempo, resources, s.sample_rate)?;
                    list.extend(segments.iter().cloned());
                    plan.waiting_items.extend(waiting);
                    plan.streams.extend(streams);
                    for st in stretches {
                        if !plan.stretches.contains(&st) {
                            plan.stretches.push(st);
                        }
                    }
                }
                if !list.is_empty() {
                    crate::items::auto_crossfade(&mut list);
                    plan.audio.push((a, list));
                }
            }

            // Outputs
            let gain_width = plan.nodes[gain].outputs;
            let mut next = 0u16;
            let mut main_output: Option<(usize, u16)> = None;
            for o in items(t, "outputs") {
                let Some(oid) = id_of(o) else { continue };
                let c = channels(o, 2);
                let node = plan.add(PNode::sum(NodeKey::Output(oid), c));
                if main_output.is_none() {
                    main_output = Some((node, c));
                }
                let range = match &port_layout {
                    Some(l) => l.output_range(text(o, "port", "main")),
                    None => {
                        let r = (next < gain_width).then(|| (next, c.min(gain_width - next)));
                        next = next.saturating_add(c);
                        r
                    }
                };
                if let Some((start, n)) = range
                    && n > 0
                {
                    plan.feed(gain, start, n, node, 0, c, false);
                }
                let dest = dest_of(ref_of(o, "to"));
                let to = channel_in(&plan, dest);
                plan.feed(node, 0, c, to, 0, width_of(dest), false);
            }

            // Monitoring (H5)
            let monitor = text(t, "monitor", "auto");
            let armed = flag(t, "armed", false);
            if let Some(input) = t.get("input").filter(|i| !i.is_null())
                && let Some((out, out_ch)) = main_output
                && (monitor == "on" || (monitor == "auto" && armed))
            {
                let chans = device_channels(input.get("channels"));
                if !chans.is_empty() {
                    let k = u16::try_from(chans.len().min(64)).unwrap_or(64);
                    let mut m = PNode::sum(NodeKey::Monitor(tid), k);
                    m.gain = Some(vol(1));
                    m.muted = muted;
                    let mi = plan.add(m);
                    plan.inputs.push((mi, chans.into_iter().take(usize::from(k)).collect()));
                    plan.feed(mi, 0, k, out, 0, out_ch, false).monitor = true;
                }
            }
        }

        // Low-latency monitoring (P-10): the channels from each armed track's main output to the master
        let mut low_latency_path: HashSet<NodeId> = HashSet::new();
        if env.low_latency_monitoring {
            for t in tracks.iter().filter(|t| flag(t, "armed", false) && !flag(t, "folder", false)) {
                let Some(main) = items(t, "outputs").next() else { continue };
                let mut at = dest_of(ref_of(main, "to"));
                while low_latency_path.insert(at) && at != MASTER {
                    let next = by_id.get(&at).map(|i| desk[*i].value).and_then(|c| ref_of(c, "output"));
                    at = dest_of(next);
                }
                low_latency_path.insert(MASTER);
            }
        }

        // ---- The desk
        let mut sends_later: Vec<(usize, usize, u16, &Value)> = Vec::new(); // (pre tap, post tap, width, channel)
        for c in desk.iter().chain(master_view.iter()) {
            let input = channel_in(&plan, c.id);
            let w = c.width;
            let rack = c.value.get("rack").cloned().unwrap_or_default();
            // The legacy strip's channel-level controls (optional fields) for the stages that run them, and the
            // machine's monitoring for the master's last segment
            let mut channel_controls = Value::empty_object();
            for k in ["trim", "mono", "width", "phaseInvert", "swap", "preLeft", "preRight"] {
                if let Some(v) = c.value.get(k) {
                    channel_controls = channel_controls.with(k, v.clone());
                }
            }
            let has = |k: &str| channel_controls.get(k).is_some();
            let on = |k: &str| channel_controls.get(k).and_then(Value::as_bool) == Some(true);
            let wants_input = !c.master && (has("trim") || has("width") || on("mono"));
            let wants_polarity = !c.master && (on("phaseInvert") || on("swap"));
            let wants_pre = c.master && (has("preLeft") || has("preRight"));
            let monitor = if c.master { env.monitor.clone() } else { None };
            let seed = Rng::seed_for(s.seed, id_number(c.id));
            let inserts: Vec<(&Value, InsertPoint)> = items(c.value, "inserts")
                .map(|p| (p, InsertPoint::parse(p.get("point").and_then(Value::as_text))))
                .collect();

            // The chain before the fader: (node, input width, output width)
            let mut prev = (input, w);
            let add_rack = |this: &Self, plan: &mut Plan, stages: Vec<Stage>, prev: &mut (usize, u16)| {
                // The digital desk has no rack, but the channel-level controls and the monitoring still run
                let digital_ok = stages.iter().all(|st| match st {
                    Stage::Input | Stage::Polarity => true,
                    Stage::MasterDrive => wants_pre,
                    Stage::MasterTransformer => monitor.is_some(),
                    _ => false,
                });
                if stages.is_empty() || (!analog && !digital_ok) {
                    return;
                }
                let k = NodeKey::Rack(c.id, stages[0]);
                let mut seg = rack.clone();
                if stages.iter().any(|st| matches!(st, Stage::Input | Stage::Polarity | Stage::MasterDrive)) {
                    seg = seg.with("channel", channel_controls.clone());
                }
                if stages.contains(&Stage::MasterTransformer)
                    && let Some(m) = &monitor
                {
                    seg = seg.with("monitor", m.clone());
                }
                if !analog {
                    seg = seg.with("digital", Value::bool(true));
                }
                let build = Build::Rack {
                    channel: c.id,
                    master: c.master,
                    stages: stages.clone(),
                    rack: seg.clone(),
                    channels: w,
                };
                let shape = PluginShape::effect(w);
                let (shape, latency) = this.decide(plan, k, Some((build, shape.clone())), seed, shape);
                let n = plan.add(PNode::processor(k, &shape, latency));
                let table = if c.master { &this.rack_master } else { &this.rack_channel };
                for (i, p) in table.iter().enumerate() {
                    if stages.contains(&p.stage) {
                        let v = seg
                            .get(p.section)
                            .and_then(|sec| sec.get(p.field))
                            .and_then(Value::as_real)
                            .unwrap_or(p.default);
                        plan.params.push(PParam {
                            node: n,
                            index: i as u32,
                            owner: c.id,
                            key: key(&p.key),
                            rate: Rate::Block,
                            discrete: false,
                            value: v as f32,
                            range: ParamRange::linear(p.min as f32, p.max as f32),
                        });
                    }
                }
                let ins = shape.inputs.max(1);
                plan.feed(prev.0, 0, prev.1, n, 0, ins, false);
                *prev = (n, shape.output_channels());
            };
            let add_insert = |this: &Self, plan: &mut Plan, p: &Value, prev: &mut (usize, u16)| -> Result<()> {
                let Some(pid) = id_of(p) else { return Ok(()) };
                // A bypassed insert is not built nor run: the chain goes past it (and safe mode's hung plugin, U21,
                // is never called again)
                if flag(p, "bypass", false) {
                    return Ok(());
                }
                let wanted = this.plugin_build(p, w);
                let k = NodeKey::Insert(pid);
                let iseed = Rng::seed_for(s.seed, id_number(pid));
                let (shape, latency) = this.decide(plan, k, wanted, iseed, PluginShape::effect(w));
                let n = plan.add(PNode::processor(k, &shape, latency));
                this.plugin_params(plan, n, p);
                // Low-latency monitoring (P-10): on an armed track's path, an insert with latency is left out
                let low_latency = latency > 0 && low_latency_path.contains(&c.id);
                if low_latency {
                    plan.low_latency_bypassed.push(k);
                }
                if !low_latency {
                    plan.feed(prev.0, 0, prev.1, n, 0, shape.inputs.max(1), false);
                    // The legacy EffectView's W/D: the insert's output at `wet`, the dry signal at `1 − wet`, summed
                    // in a node of their own; the PDC delays the dry side by the insert's latency
                    if let Some(wet) = p.get("wet").and_then(Value::as_real) {
                        let out = shape.output_channels().max(1);
                        let mut mix = PNode::sum(NodeKey::InsertMix(pid), out);
                        mix.gain = Some(HPlan::Fixed(1.0));
                        let m = plan.add(mix);
                        let wet_param = |instance: u8, complement: bool| HPlan::Param {
                            owner: pid,
                            key: key("insert.wet"),
                            instance,
                            value: wet as f32,
                            complement,
                        };
                        plan.feed(n, 0, out, m, 0, out, false).gain = Some(wet_param(0, false));
                        plan.feed(prev.0, 0, prev.1, m, 0, out, false).gain = Some(wet_param(1, true));
                        *prev = (m, out);
                    } else {
                        *prev = (n, shape.output_channels().max(1));
                    }
                }
                Ok(())
            };
            if wants_input {
                add_rack(self, &mut plan, vec![Stage::Input], &mut prev);
            }
            if c.master {
                add_rack(self, &mut plan, vec![Stage::MasterDrive], &mut prev);
                for (p, _) in &inserts {
                    add_insert(self, &mut plan, p, &mut prev)?;
                }
                add_rack(self, &mut plan, vec![Stage::MasterBus], &mut prev);
            } else {
                let mut run: Vec<Stage> = Vec::new();
                for (s_idx, stage) in Stage::CHANNEL.iter().enumerate() {
                    let here: Vec<&Value> = inserts
                        .iter()
                        .filter(|(_, pt)| *pt != InsertPoint::PrePan && pt.stages_before() == s_idx)
                        .map(|(p, _)| *p)
                        .collect();
                    if !here.is_empty() {
                        add_rack(self, &mut plan, std::mem::take(&mut run), &mut prev);
                        for p in here {
                            add_insert(self, &mut plan, p, &mut prev)?;
                        }
                    }
                    run.push(*stage);
                }
                add_rack(self, &mut plan, run, &mut prev);
                for (p, _) in inserts.iter().filter(|(_, pt)| *pt == InsertPoint::PostEq) {
                    add_insert(self, &mut plan, p, &mut prev)?;
                }
            }
            let pre_tap = prev;
            if wants_polarity {
                add_rack(self, &mut plan, vec![Stage::Polarity], &mut prev);
            }
            let volume = HPlan::Param {
                owner: c.id,
                key: key("volume"),
                instance: 0,
                value: real(c.value, "volume", 1.0) as f32,
                complement: false,
            };
            let pan = (!c.master).then(|| HPlan::Param {
                owner: c.id,
                key: key("pan"),
                instance: 0,
                value: real(c.value, "pan", 0.0) as f32,
                complement: false,
            });
            let pre_pan: Vec<&Value> =
                inserts.iter().filter(|(_, pt)| !c.master && *pt == InsertPoint::PrePan).map(|(p, _)| *p).collect();
            let mut fader = PNode::sum(NodeKey::Fader(c.id), w);
            fader.gain = Some(volume);
            if pre_pan.is_empty() {
                fader.pan = pan.clone();
            }
            let f = plan.add(fader);
            plan.feed(prev.0, 0, prev.1, f, 0, w, false);
            prev = (f, w);
            if !pre_pan.is_empty() {
                for p in pre_pan {
                    add_insert(self, &mut plan, p, &mut prev)?;
                }
                let mut pn = PNode::sum(NodeKey::Pan(c.id), w);
                pn.gain = Some(HPlan::Fixed(1.0));
                pn.pan = pan;
                let pi = plan.add(pn);
                plan.feed(prev.0, 0, prev.1, pi, 0, w, false);
                prev = (pi, w);
            }
            if c.master {
                add_rack(self, &mut plan, vec![Stage::MasterTransformer], &mut prev);
            }
            let last = prev.0;
            // Mutes: the channel's own after its last node; the solo's before the fader (base item 2)
            let own_mute = flag(c.value, "mute", false);
            if own_mute || keyed.contains(&c.id) {
                plan.nodes[last].muted = true;
            }
            if solo_muted.contains(&c.id) {
                plan.nodes[pre_tap.0].muted = true;
            }
            if c.master {
                plan.master = Some(last);
                for (i, dev) in master_devices.clone().unwrap_or_default().iter().enumerate() {
                    plan.outputs.push((last, (i as u16).min(prev.1.saturating_sub(1)), *dev));
                }
            } else {
                let dest = dest_of(ref_of(c.value, "output"));
                let to = channel_in(&plan, dest);
                plan.feed(last, 0, prev.1, to, 0, width_of(dest), false);
                sends_later.push((pre_tap.0, last, pre_tap.1.min(prev.1).max(1), c.value));
            }
        }

        // Sends, after every node exists (a sidechain goes to an insert of a later channel)
        for (pre, post, w, ch) in sends_later {
            for send in items(ch, "sends") {
                let Some(sid) = id_of(send) else { continue };
                let Some(to) = ref_of(send, "to") else { continue };
                let tap = if flag(send, "pre", false) { pre } else { post };
                let tap_w = plan.nodes[tap].outputs.max(1).min(w.max(plan.nodes[tap].outputs));
                let amount = real(send, "amount", 1.0) as f32;
                // A slot, so a hand or a MIDI control moves it without a compilation (I-21.2)
                let amount_param =
                    HPlan::Param { owner: sid, key: key("amount"), instance: 0, value: amount, complement: false };
                let (dest, to_channel, dest_w, sidechain) = if to.kind == KIND_CHANNEL {
                    let Some(&d) = plan.index.get(&NodeKey::ChannelIn(to)) else { continue };
                    (d, 0, plan.nodes[d].inputs, false)
                } else {
                    let Some(&d) = plan.index.get(&NodeKey::Insert(to)) else { continue };
                    let port = text(send, "port", "");
                    let shape = self.sidechain_shape(&plan, to);
                    let Some((start, n)) = shape.and_then(|sh| sh.sidechain_range(port)) else { continue };
                    if n == 0 {
                        continue;
                    }
                    (d, start, n, true)
                };
                if plan.has_feed(tap, dest, 0, to_channel, sidechain) {
                    // The same route again: through its own summing node, so each has its own delay line
                    let hop = plan.add(PNode::sum(NodeKey::Hop(sid), tap_w));
                    plan.feed(tap, 0, tap_w, hop, 0, tap_w, false).gain = Some(amount_param);
                    plan.feed(hop, 0, tap_w, dest, to_channel, dest_w, sidechain);
                } else {
                    plan.feed(tap, 0, tap_w, dest, to_channel, dest_w, sidechain).gain = Some(amount_param);
                }
            }
        }

        // Tails (U32): the kept ones and the ones this install makes, fed where their owner feeds
        let net = self.tail_net_frames();
        let live: Vec<(u64, NodeKey, PluginShape, bool, u32)> = self
            .tails
            .iter()
            .filter(|t| !t.ended && self.frames < t.created.saturating_add(net))
            .map(|t| (t.serial, t.owner, t.shape.clone(), t.in_place, t.latency))
            .chain(plan.new_tails.iter().cloned())
            .collect();
        for (serial, owner, shape, in_place, latency) in live {
            let Some(&o) = plan.index.get(&owner) else { continue };
            let outgoing: Vec<PFeed> = plan.feeds.iter().filter(|f| f.from == o && !f.monitor).cloned().collect();
            if outgoing.is_empty() {
                continue;
            }
            let mut node = PNode::processor(NodeKey::Tail(serial), &shape, latency);
            node.in_place = in_place && node.inputs == node.outputs;
            node.tail = true;
            node.sidechain = 0;
            let ti = plan.add(node);
            let outs = plan.nodes[ti].outputs;
            for f in outgoing {
                if f.from_channel >= outs {
                    continue;
                }
                let mut g = f.clone();
                g.from = ti;
                g.channels = g.channels.min(outs - f.from_channel);
                g.tail = true;
                plan.feeds.push(g);
            }
            plan.tails.push((serial, ti));
        }

        // Transport (I-7.2), in samples
        let range = |key: &str| -> Option<(i64, i64)> {
            let r = transport.get(key)?;
            flag(r, "on", false).then(|| (tempo.sample_at(int(r, "from", 0)), tempo.sample_at(int(r, "to", 0))))
        };
        let min_loop = i64::from((s.sample_rate / 20).max(1));
        plan.transport.loop_range = range("loop").filter(|(a, b)| b - a >= min_loop);
        plan.transport.punch = range("punch").filter(|(a, b)| b > a);
        if let Some(a) = env.audition {
            // the item's own loop in place of the project's; no punch while an editor plays an item
            plan.transport.loop_range =
                a.repeat.then(|| (tempo.sample_at(a.from), tempo.sample_at(a.to))).filter(|(x, y)| y - x >= min_loop);
            plan.transport.punch = None;
        }
        let bars = transport.get("metronome").map_or(0, |m| int(m, "countIn", 0)).clamp(0, 64) as u32;
        plan.transport.count_in = (bars > 0).then_some(CountIn { bars, on_play: env.count_in_on_play });
        plan.transport.pre_roll_bars = transport.get("preRoll").map_or(0, |p| int(p, "bars", 0)).clamp(0, 16) as u32;
        self.collect_modulators(root, resources, s.seed, &mut plan);
        Ok(plan)
    }

    /// Reads the modulators of every track, channel, master and plugin (a modulator whose node is not in the
    /// program is skipped)
    fn collect_modulators(&self, root: &Value, resources: &dyn Resources, seed: u64, plan: &mut Plan) {
        let mut owners: Vec<(&Value, bool)> = Vec::new();
        for t in items(root, "tracks") {
            owners.push((t, true));
            if let Some(i) = t.get("instrument").filter(|i| !i.is_null()) {
                owners.push((i, false));
            }
        }
        if let Some(mixer) = root.get("mixer") {
            for c in items(mixer, "channels").chain(mixer.get("master")) {
                owners.push((c, false));
                for p in items(c, "inserts") {
                    owners.push((p, false));
                }
            }
        }
        for (owner, is_track) in owners {
            let Some(owner_id) = id_of(owner) else { continue };
            let trigger = if is_track {
                owner.get("instrument").filter(|i| !i.is_null()).and_then(id_of).map(NodeKey::Instrument)
            } else if owner_id.kind == KIND_PLUGIN {
                Some(NodeKey::Instrument(owner_id))
            } else {
                None
            };
            let trigger = trigger.and_then(|k| plan.index.get(&k).copied());
            for m in items(owner, "modulators") {
                let Some(id) = id_of(m) else { continue };
                let kind_name: &'static str = match text(m, "kind", "lfo") {
                    "envelope" => "envelope",
                    "peak" => "peak",
                    "random" => "random",
                    "macro" => "macro",
                    _ => "lfo",
                };
                let kind = match kind_name {
                    "envelope" => ModKind::Envelope,
                    "peak" => ModKind::Peak,
                    "random" => ModKind::Random,
                    "macro" => ModKind::Macro,
                    _ => ModKind::Lfo,
                };
                let mut params = MOD_DEFAULTS;
                if let Some(ps) = m.get(lm_state::PARAMS_KEY) {
                    for (i, k) in MOD_KEYS.iter().enumerate() {
                        if let Some(v) = ps.get(k).and_then(Value::as_real) {
                            params[i] = v as f32;
                        }
                    }
                }
                let input = ref_of(m, "input").and_then(|n| {
                    let keys: &[NodeKey] = match n.kind {
                        lm_doc::schemas::KIND_TRACK => &[NodeKey::TrackGain(n), NodeKey::TrackAudio(n)],
                        KIND_CHANNEL => &[NodeKey::Pan(n), NodeKey::Fader(n)],
                        _ => &[],
                    };
                    keys.iter().find_map(|k| plan.index.get(k).copied())
                });
                let user_wave = m.get("userWave").and_then(Value::as_resource).and_then(|r| match resources.audio(r) {
                    crate::catalog::ResourceState::Ready(d) => Some(Arc::from(d.channel(0).to_vec())),
                    _ => None,
                });
                let targets = items(m, "targets")
                    .filter_map(|t| {
                        let node = ref_of(t, "node")?;
                        let param = text(t, "param", "");
                        if param.is_empty() {
                            return None;
                        }
                        Some((
                            node,
                            key(param),
                            real(t, "amount", 0.5) as f32,
                            real(t, "min", 0.0) as f32,
                            real(t, "max", 1.0) as f32,
                        ))
                    })
                    .collect();
                let _ = owner_id;
                plan.mods.push(PMod {
                    id,
                    kind,
                    kind_name,
                    params,
                    seed: Rng::seed_for(seed, id_number(id)),
                    user_wave,
                    input,
                    trigger: if kind == ModKind::Envelope { trigger } else { None },
                    targets,
                });
            }
        }
    }

    fn sidechain_shape(&self, plan: &Plan, insert: NodeId) -> Option<PluginShape> {
        let k = NodeKey::Insert(insert);
        let proc = self.procs.get(&k);
        // The shape the plan gave the node: the planned decision's
        let pp = plan.procs.iter().find(|p| p.key == k)?;
        if pp.use_ready {
            return proc.and_then(|p| p.ready.as_ref()).map(|r| r.shape.clone());
        }
        if let Some(i) = proc.and_then(|p| p.installed.as_ref())
            && !pp.fresh
        {
            return Some(i.shape.clone());
        }
        pp.wanted.as_ref().map(|(_, s)| s.clone())
    }

    /// The PDC (base item 3): delays on the feeds so that every node's inputs line up
    fn pdc(&self, plan: &mut Plan) -> Result<()> {
        let n = plan.nodes.len();
        let mut indegree = vec![0u32; n];
        let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_feeds: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (fi, f) in plan.feeds.iter().enumerate() {
            indegree[f.to] += 1;
            out_edges[f.from].push(f.to);
            in_feeds[f.to].push(fi);
        }
        let mut queue: VecDeque<usize> = (0..n).filter(|i| indegree[*i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = queue.pop_front() {
            order.push(i);
            for &t in &out_edges[i] {
                indegree[t] -= 1;
                if indegree[t] == 0 {
                    queue.push_back(t);
                }
            }
        }
        if order.len() != n {
            return Err(Error::new("compile.routing.cycle"));
        }
        let mut arrive = vec![0u64; n];
        let mut out = vec![0u64; n];
        for &i in &order {
            let a = in_feeds[i]
                .iter()
                .map(|&f| &plan.feeds[f])
                .filter(|f| !f.monitor && !f.tail)
                .map(|f| out[f.from])
                .max()
                .unwrap_or(0);
            arrive[i] = a;
            out[i] = a + u64::from(plan.nodes[i].latency);
        }
        for f in &mut plan.feeds {
            f.delay =
                if f.monitor { 0 } else { u32::try_from(arrive[f.to].saturating_sub(out[f.from])).unwrap_or(u32::MAX) };
        }
        // A track output's delay goes before the output node, so the monitoring that joins there is not delayed
        for fi in 0..plan.feeds.len() {
            let f = &plan.feeds[fi];
            if f.delay == 0 || !matches!(plan.nodes[f.from].key, NodeKey::Output(_)) {
                continue;
            }
            let from = f.from;
            let feeding: Vec<usize> =
                in_feeds[from].iter().copied().filter(|&g| !plan.feeds[g].monitor && !plan.feeds[g].tail).collect();
            let outgoing = plan.feeds.iter().filter(|g| g.from == from).count();
            if feeding.len() == 1 && outgoing == 1 {
                let d = plan.feeds[fi].delay;
                plan.feeds[feeding[0]].delay += d;
                plan.feeds[fi].delay = 0;
            }
        }
        Ok(())
    }
}

impl Plan {
    fn host_params(&self) -> impl Iterator<Item = &HPlan> + '_ {
        self.nodes
            .iter()
            .flat_map(|n| n.gain.iter().chain(n.pan.iter()))
            .chain(self.feeds.iter().filter_map(|f| f.gain.as_ref()))
    }
}

/// The range of a parameter the host applies: gains (a fader, a send) on the fader scale, the pan from −1 to 1
fn host_range(key: &str) -> ParamRange {
    match key {
        "pan" => ParamRange::linear(-1.0, 1.0),
        "insert.wet" => ParamRange::linear(0.0, 1.0),
        _ => ParamRange { min: 0.0, max: 4.0, scale: RangeScale::Gain },
    }
}

fn rate_of(spec: &ParamSpec) -> Rate {
    match spec.rate {
        lm_state::Rate::Block => Rate::Block,
        lm_state::Rate::Sample => Rate::Sample,
    }
}

/// The channel solo (base item 2): the channels silenced before their fader, and the sidechain-only sources
/// silenced after it
fn channel_solo(
    desk: &[ChannelView<'_>],
    dest_of: &dyn Fn(Option<NodeId>) -> NodeId,
    insert_owner: &HashMap<NodeId, NodeId>,
) -> (HashSet<NodeId>, HashSet<NodeId>) {
    let soloed: HashSet<NodeId> = desk.iter().filter(|c| flag(c.value, "solo", false)).map(|c| c.id).collect();
    if soloed.is_empty() {
        return (HashSet::new(), HashSet::new());
    }
    let mut audio: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut side: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for c in desk {
        let mut out = vec![dest_of(ref_of(c.value, "output"))];
        for send in items(c.value, "sends") {
            let Some(to) = ref_of(send, "to") else { continue };
            if to.kind == KIND_CHANNEL {
                out.push(to);
            } else if let Some(owner) = insert_owner.get(&to) {
                side.entry(c.id).or_default().push(*owner);
            }
        }
        audio.insert(c.id, out);
    }
    let reach = |start: &HashSet<NodeId>, edges: &dyn Fn(NodeId) -> Vec<NodeId>| -> HashSet<NodeId> {
        let mut seen: HashSet<NodeId> = start.clone();
        let mut stack: Vec<NodeId> = start.iter().copied().collect();
        while let Some(n) = stack.pop() {
            for m in edges(n) {
                if seen.insert(m) {
                    stack.push(m);
                }
            }
        }
        seen
    };
    let down = reach(&soloed, &|n| audio.get(&n).cloned().unwrap_or_default());
    let mut reverse: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for (from, tos) in &audio {
        for to in tos {
            reverse.entry(*to).or_default().push(*from);
        }
    }
    let up = reach(&soloed, &|n| reverse.get(&n).cloned().unwrap_or_default());
    let audible: HashSet<NodeId> = down.union(&up).copied().collect();
    let keyed: HashSet<NodeId> = desk
        .iter()
        .filter(|c| !audible.contains(&c.id))
        .filter(|c| side.get(&c.id).is_some_and(|t| t.iter().any(|o| audible.contains(o))))
        .map(|c| c.id)
        .collect();
    let muted = desk.iter().map(|c| c.id).filter(|id| !audible.contains(id) && !keyed.contains(id)).collect();
    (muted, keyed)
}
