//! The compiler's public vocabulary: the program's node keys, the builds it asks the session for, the installed
//! table the session keeps (H14) and the result of a compilation.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use lm_render::{
    Install, MidiTiming, NodeIndex, ParamRange, ParamSlot, PortIndex, ProcessSpec, ProgramGen, SampleData,
};
use lm_state::{NodeId, Value};

use crate::catalog::PluginShape;
use crate::items::{Made, WaitingItem};
use crate::rack::Stage;
use crate::read::{same_except, same_rack_settings};

/// What a node of the program is, by the document's ids: stable across compilations, so a node keeps its index
/// (and its processor) while it stays in the project
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKey {
    /// The metronome's source (I-5.5)
    Metronome,
    /// The preview bus (I-7.3)
    Preview,
    /// A track's instrument (`p:`)
    Instrument(NodeId),
    /// A track's audio items (`t:`): a summing node; on a track without instrument it carries the track's volume
    TrackAudio(NodeId),
    /// A track's volume and mute for all its outputs, after the instrument (`t:`)
    TrackGain(NodeId),
    /// A track's output (`o:`), metered (M9)
    Output(NodeId),
    /// The monitored device input of an armed track (`t:`), outside the PDC (H5)
    Monitor(NodeId),
    /// A desk channel's input, where everything sent to it is summed in a fixed order (`m:`)
    ChannelIn(NodeId),
    /// A segment of a channel's rack, by the channel and its first stage
    Rack(NodeId, Stage),
    /// A channel's insert (`p:`)
    Insert(NodeId),
    /// A note effect of a track (`p:`), chained before its instrument: no audio
    NoteEffect(NodeId),
    /// A channel's fader (volume, and pan when no insert sits before the pan)
    Fader(NodeId),
    /// A channel's pan, after the `pre-pan` inserts
    Pan(NodeId),
    /// A send that repeats another route between the same two nodes, through its own summing node (`s:`)
    Hop(NodeId),
    /// An insert's wet/dry mix (`p:`): the insert's output at `wet`, the dry signal before it at `1 − wet`, delayed
    /// by the insert's latency (the PDC)
    InsertMix(NodeId),
    /// The old processor of a replaced node, sounding until its tail ends (U32), by a serial number
    Tail(u64),
}

impl NodeKey {
    /// The document node this program node belongs to (none for the metronome, the preview bus and the tails)
    pub fn owner(self) -> Option<NodeId> {
        match self {
            NodeKey::Metronome | NodeKey::Preview | NodeKey::Tail(_) => None,
            NodeKey::Instrument(id)
            | NodeKey::TrackAudio(id)
            | NodeKey::TrackGain(id)
            | NodeKey::Output(id)
            | NodeKey::Monitor(id)
            | NodeKey::ChannelIn(id)
            | NodeKey::Rack(id, _)
            | NodeKey::Insert(id)
            | NodeKey::NoteEffect(id)
            | NodeKey::Fader(id)
            | NodeKey::Pan(id)
            | NodeKey::Hop(id)
            | NodeKey::InsertMix(id) => Some(id),
        }
    }
}

/// What processor a node needs. Two builds with the same processor differ at most in parameter values, which reach
/// the processor through slots: no new build then
#[derive(Clone, Debug, PartialEq)]
pub enum Build {
    /// An instrument or an insert, built by the plugin layer from its state
    Plugin {
        node: NodeId,
        plugin: Arc<str>,
        version: u32,
        /// The whole state, parameters included (the build starts from the tree's values, S20)
        state: Value,
        /// The width it was asked to fit: the desk channel's for an insert, the track's for an instrument
        channels: u16,
    },
    /// A segment of a desk channel's rack (`rack.rs`)
    Rack {
        channel: NodeId,
        master: bool,
        /// The stages it runs, in order
        stages: Vec<Stage>,
        /// The channel's `rack` (its real fields are the parameters, indexed by `rack_params`)
        rack: Value,
        channels: u16,
    },
}

impl Build {
    /// The same processor: everything equal but the parameter values
    pub fn same_processor(&self, other: &Build) -> bool {
        match (self, other) {
            (
                Build::Plugin { node: a, plugin: pa, version: va, state: sa, channels: ca },
                Build::Plugin { node: b, plugin: pb, version: vb, state: sb, channels: cb },
            ) => {
                a == b
                    && pa == pb
                    && va == vb
                    && ca == cb
                    && same_except(sa, sb, &[lm_state::PARAMS_KEY, lm_plugin::CHUNK_KEY])
            }
            (
                Build::Rack { channel: a, master: ma, stages: sa, rack: ra, channels: ca },
                Build::Rack { channel: b, master: mb, stages: sb, rack: rb, channels: cb },
            ) => a == b && ma == mb && sa == sb && ca == cb && same_rack_settings(ra, rb),
            _ => false,
        }
    }
}

/// The number of a build request: `Compiler::ready` names it, and a stale one (superseded by a newer request for
/// the node, or for a node that is gone) is dropped
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticket(pub u64);

/// A processor the session must build in an `Operation` (T6, C11: once the resources its state cites are loaded),
/// then hand back with `Compiler::ready(ticket, NodeCore::new(processor, seed, display))`
#[derive(Clone, Debug)]
pub struct BuildRequest {
    pub ticket: Ticket,
    pub node: NodeKey,
    pub build: Build,
    /// The node's random seed: `Rng::seed_for(project.seed, id)` (C13; the rack's tolerances by the channel's id,
    /// P-21)
    pub seed: u64,
    pub spec: ProcessSpec,
    /// The shape the program gives the node once it is built
    pub shape: PluginShape,
}

/// When the metronome sounds (the preference `metronome.when`, F-REC-025)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MetronomeWhen {
    /// Playing and recording
    Always,
    /// Only while recording
    #[default]
    Recording,
    /// Never: the count-in is silent too
    Never,
}

impl MetronomeWhen {
    /// From the preference's text; anything unknown is the default
    pub fn parse(text: &str) -> Self {
        match text {
            "always" => Self::Always,
            "never" => Self::Never,
            _ => Self::Recording,
        }
    }
}

/// The editor's play of one MIDI or audio item (F-NOT-017): the song plays only that item, in a loop of its own when
/// `repeat`. Ticks of the song; never in the project and never in an export
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Audition {
    pub item: NodeId,
    pub from: i64,
    pub to: i64,
    pub repeat: bool,
}

/// What the machine adds to the tree for one compilation (preferences and devices, never in the project)
#[derive(Clone, Debug, Default)]
pub struct Environment {
    /// The metronome's sounds (resources the session loaded): the accent and the beat
    pub metronome_sounds: Option<(Arc<SampleData>, Arc<SampleData>)>,
    /// The device channels of the metronome (a preference, I-5.5); `None`: the master's output
    pub metronome_output: Option<Vec<u16>>,
    /// When the metronome sounds (`metronome.when`)
    pub metronome_when: MetronomeWhen,
    /// The live MIDI input ports of the instrument role: an armed track with an instrument and no ports of its own
    /// takes all of them (I-7.1)
    pub midi_ports: Vec<PortIndex>,
    /// Every port alias of the machine with its index (a track's `midiInput.ports` and `midiOutput.ports`, I-21.1)
    pub midi_aliases: Vec<(Arc<str>, PortIndex)>,
    /// The ports that are MPE zones (a preference of the port): their member channels' expressions go to the notes
    pub mpe_ports: Vec<lm_render::mpe::MpePort>,
    /// The selected track: a track whose input follows the selection plays when selected (F-REC-131)
    pub selected_track: Option<NodeId>,
    pub midi_timing: MidiTiming,
    /// The playhead in samples, for the informative tick of a relocation (H9)
    pub playhead: i64,
    /// The count-in (`transport.metronome.countIn`) also when playing, not only when recording (a preference)
    pub count_in_on_play: bool,
    /// Low-latency monitoring (P-10, a preference): on the output path of an armed track, the inserts that have
    /// latency are bypassed
    pub low_latency_monitoring: bool,
    /// The monitoring of the speakers (`preferences().monitor`: `level` dB, `dim`, `cut`, `mono`, `side`), run by
    /// the master's last segment; never in an export (whose Environment has none)
    pub monitor: Option<Value>,
    /// An editor playing one item alone: the other items are silent and the loop is the item's (see [`Audition`])
    pub audition: Option<Audition>,
}

/// A processor in the renderer's table, as the compiler knows it
#[derive(Clone, Debug)]
pub struct InstalledProcessor {
    pub node: NodeKey,
    pub build: Build,
    pub shape: PluginShape,
    pub latency: u32,
}

/// A tail node (U32)
#[derive(Clone, Debug)]
pub struct InstalledTail {
    pub serial: u64,
    pub index: NodeIndex,
    pub owner: NodeKey,
    pub shape: PluginShape,
    pub in_place: bool,
    pub latency: u32,
}

/// The tables of one install (H14): the session keeps the one of the last install it sent (installs are applied in
/// order, so it is what the renderer will have), uses it for the parameter slots (R12, I-6.3) and the telemetry
/// (by generation), and rebuilds a compiler from it after a panic
#[derive(Clone, Debug, Default)]
pub struct InstalledTable {
    pub generation: ProgramGen,
    /// Every node of the program
    pub nodes: BTreeMap<NodeKey, NodeIndex>,
    /// The slots of each parameter, by `(node id, key)` (M5: `("m:3", "rack.tube.drive")`). Usually one; a track's
    /// volume has a second one when its monitoring path is open (the session writes them all)
    pub slots: BTreeMap<(NodeId, Arc<str>), Vec<ParamSlot>>,
    pub processors: Vec<InstalledProcessor>,
    pub tails: Vec<InstalledTail>,
    /// Indices taken out by sent installs and not returned yet
    pub pending_nodes: Vec<NodeIndex>,
    /// Slots unbound by sent installs, by the install's generation
    pub pending_slots: Vec<(u64, ParamSlot)>,
    pub(crate) slot_keys: BTreeMap<(NodeId, Arc<str>, u8), ParamSlot>,
    /// The range of each parameter, by `(node id, key)`: what a MIDI control maps onto (I-21.2)
    pub ranges: BTreeMap<(NodeId, Arc<str>), ParamRange>,
}

impl InstalledTable {
    pub fn node(&self, key: NodeKey) -> Option<NodeIndex> {
        self.nodes.get(&key).copied()
    }

    /// The slots of a parameter (empty when it has none: not bound, or a node still unknown)
    pub fn slots_of(&self, node: NodeId, key: &str) -> &[ParamSlot] {
        self.slots.get(&(node, Arc::from(key))).map_or(&[], Vec::as_slice)
    }

    /// The node behind an index of this generation (for the telemetry)
    pub fn key_of(&self, index: NodeIndex) -> Option<NodeKey> {
        self.nodes
            .iter()
            .find(|(_, i)| **i == index)
            .map(|(k, _)| *k)
            .or_else(|| self.tails.iter().find(|t| t.index == index).map(|t| NodeKey::Tail(t.serial)))
    }

    /// The node whose meter shows a document node (M9): a track's gain, an output, a channel's last node, a
    /// plugin's node
    pub fn meter_of(&self, id: NodeId) -> Option<NodeIndex> {
        use lm_doc::schemas::{KIND_CHANNEL, KIND_OUTPUT, KIND_PLUGIN, KIND_TRACK};
        let keys: &[NodeKey] = match id.kind {
            KIND_TRACK => &[NodeKey::TrackGain(id), NodeKey::TrackAudio(id)],
            KIND_OUTPUT => &[NodeKey::Output(id)],
            KIND_CHANNEL => &[NodeKey::Pan(id), NodeKey::Fader(id)],
            KIND_PLUGIN => &[NodeKey::Instrument(id), NodeKey::Insert(id), NodeKey::NoteEffect(id)],
            _ => &[],
        };
        keys.iter().find_map(|k| self.node(*k))
    }
}

/// One compilation's result
#[derive(Debug)]
pub struct Compiled {
    /// For `EngineControl::install`, in the order the compilations came out
    pub install: Install,
    pub generation: ProgramGen,
    /// Processors to build now
    pub builds: Vec<BuildRequest>,
    /// Build requests no longer wanted (cancel their operations; a late `ready` for them is dropped)
    pub cancelled: Vec<Ticket>,
    /// The tables of this install
    pub table: Arc<InstalledTable>,
    /// New processor nodes that are silent until built (I-2.3)
    pub waiting: Vec<NodeKey>,
    /// Nodes that keep their old processor while the new one builds (T7)
    pub replacing: Vec<NodeKey>,
    /// Audio items whose resources are not loaded (silent until they are; the session recompiles then)
    pub waiting_items: Vec<WaitingItem>,
    /// The streams made by this compilation (P-32): hand each feeder to the disk reader with its resource's file.
    /// Streams of unchanged items are kept from compilation to compilation
    pub streams: Vec<crate::items::StreamRegistration>,
    /// The renderings of the lanes that follow the tempo (F-REC-090) that are not loaded: each lane plays its
    /// original until its rendering is (the session renders or loads them)
    pub stretches: Vec<crate::stretch::StretchPlan>,
    /// How many event lists and curves this compilation made new (V8)
    pub made: Made,
    pub elapsed: Duration,
    /// The routes of the program, in its summing order, with their PDC delays (for inspection and the routing
    /// view)
    pub routes: Vec<Route>,
    /// The nodes silenced by a mute or a solo, and the latency each node counts in the PDC
    pub nodes: Vec<NodeInfo>,
    /// The live MIDI routes (I-7.1): each input port to the armed instruments
    pub ports: Vec<(PortIndex, NodeKey)>,
    /// The inserts left out by the low-latency monitoring (P-10)
    pub low_latency_bypassed: Vec<NodeKey>,
}

/// One route of a compiled program
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub from: NodeKey,
    pub to: NodeKey,
    pub from_channel: u16,
    pub channels: u16,
    pub to_channel: u16,
    pub sidechain: bool,
    /// The monitoring path (H5): never delayed
    pub monitor: bool,
    /// Samples of PDC on this route
    pub delay: u32,
}

/// One node of a compiled program
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeInfo {
    pub key: NodeKey,
    pub muted: bool,
    pub latency: u32,
    pub channels: u16,
}

impl Compiled {
    /// Nothing waits: every processor is built and every resource loaded (an export starts only then, T8)
    pub fn complete(&self) -> bool {
        self.waiting.is_empty()
            && self.replacing.is_empty()
            && self.waiting_items.is_empty()
            && self.builds.is_empty()
            && self.stretches.is_empty()
    }
}
