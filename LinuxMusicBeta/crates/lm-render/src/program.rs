//! The render program (interfaces.md, I-5; arquitetura-nova.md, piece 3 and base items 1–3, 9): flat lists of nodes,
//! routes, PDC delays, item events, automation curves, the port table and the parameter table. No `Value` reaches it
//! (R1).
//!
//! The compiler describes the program with a [`ProgramBuilder`]; [`ProgramBuilder::build`] checks it, orders the
//! graph into levels, and allocates everything the renderer will need while the program is installed: each node's
//! contiguous buffer block (S4) and event list, the delay lines, the lookup tables. The finished [`Program`] moves to
//! the audio thread inside an [`crate::Install`] and comes back whole when the next one replaces it.
//!
//! # The graph
//!
//! Every node has a slot in the renderer's table ([`NodeIndex`]) and is one of:
//! - [`NodeKind::Processor`]: the processor in its table slot runs (an instrument, an insert, the desk rack, a host
//!   processor). A processor node whose slot holds nothing yet (a plugin still being built, I-5.4) is silent;
//! - [`NodeKind::Sum`]: no processor; its output is the sum of its inputs (a desk channel's input, the master's).
//!
//! What a node receives is, in this fixed order: its sources (audio items, a device input, the metronome, the preview
//! bus) and then its [`Feed`]s in the order they were given — the summing order is the compiler's, never the
//! workers' (base item 9). After it runs, its optional [`Strip`] applies gain, pan and the solo-resolved mute.
//! A pre-fader send is a feed from the node before the strip node; a post-fader send, from the strip node.
//!
//! The builder puts each node one level after the latest node that feeds it; the nodes of a level run in parallel.

use std::ops::Range;
use std::sync::Arc;

use lm_base::{Error, Result};
use lm_time::TempoMap;

use crate::automation::Curve;
use crate::event::EventList;
use crate::ids::{NodeIndex, ParamSlot, PortIndex, ProgramGen};
use crate::processor::BufferLayout;
use crate::sample::SampleData;
use crate::spec::{Limits, ProcessSpec};
use crate::stream::Stream;

/// What runs in a node
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// The processor in the node's table slot
    Processor,
    /// No processor: the output is the sum of the inputs
    Sum,
}

/// A node of the program
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDesc {
    pub node: NodeIndex,
    pub kind: NodeKind,
    pub inputs: u16,
    pub outputs: u16,
    pub sidechain: u16,
    /// Inputs and outputs are the same channels (`inputs == outputs`): an insert
    pub in_place: bool,
    /// Gain, pan and mute applied by the host after the node runs
    pub strip: Option<Strip>,
    /// The solo-resolved mute mask (the compiler resolves solo; the renderer only executes, base item 2). Applied
    /// with a short fade
    pub muted: bool,
    /// A tail node (U32): it is silenced 2 s after it started, or earlier once quiet, and reported
    pub tail: bool,
}

impl NodeDesc {
    /// A processor with separate inputs and outputs (an instrument: `inputs` 0)
    pub fn processor(node: NodeIndex, inputs: u16, outputs: u16) -> Self {
        Self {
            node,
            kind: NodeKind::Processor,
            inputs,
            outputs,
            sidechain: 0,
            in_place: false,
            strip: None,
            muted: false,
            tail: false,
        }
    }

    /// A processor working in place on `channels` channels (an insert, the rack)
    pub fn insert(node: NodeIndex, channels: u16) -> Self {
        Self { in_place: true, ..Self::processor(node, channels, channels) }
    }

    /// A summing node of `channels` channels (a desk channel's input, the master's)
    pub fn sum(node: NodeIndex, channels: u16) -> Self {
        Self { kind: NodeKind::Sum, in_place: true, ..Self::processor(node, channels, channels) }
    }

    #[must_use]
    pub fn with_strip(mut self, strip: Strip) -> Self {
        self.strip = Some(strip);
        self
    }

    #[must_use]
    pub fn with_sidechain(mut self, channels: u16) -> Self {
        self.sidechain = channels;
        self
    }

    #[must_use]
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    #[must_use]
    pub fn tail(mut self) -> Self {
        self.tail = true;
        self
    }
}

/// A gain the host applies: fixed, or driven by a parameter slot (smoothed, sample-accurate) and its automation
#[derive(Clone, Debug, PartialEq)]
pub struct HostParam {
    /// The value, or the initial value of the slot (S20)
    pub value: f32,
    pub slot: Option<ParamSlot>,
    pub automation: Option<Curve>,
    /// The gain is `1 − value` (the dry side of an insert's wet/dry mix, driven by its own slot that gets the same
    /// writes and the same automation as the wet side's)
    pub complement: bool,
}

impl HostParam {
    pub fn fixed(value: f32) -> Self {
        Self { value, slot: None, automation: None, complement: false }
    }

    pub fn slot(slot: ParamSlot, initial: f32) -> Self {
        Self { value: initial, slot: Some(slot), automation: None, complement: false }
    }

    /// The same parameter, applied as `1 − value`
    #[must_use]
    pub fn complement(mut self) -> Self {
        self.complement = true;
        self
    }

    #[must_use]
    pub fn automated(mut self, curve: Curve) -> Self {
        self.automation = Some(curve);
        self
    }
}

/// Gain (linear) and pan (−1 left to 1 right) after a node. The pan of a stereo node is a balance: the far side is
/// attenuated linearly and the near side kept (`left = min(1, 1 − pan)`, `right = min(1, 1 + pan)`); a node that is
/// not stereo ignores it. The compiler converts mono to stereo explicitly before a stereo channel ([`ChannelMix`])
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub gain: HostParam,
    pub pan: Option<HostParam>,
}

/// How a feed maps the source's channels to the destination's (the explicit conversions of base item 3)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelMix {
    /// Channel `i` to channel `i`
    Direct,
    /// One channel to `to` channels, each at `1/√to` (equal power: −3 dB in each side of a stereo pair)
    UpMix { to: u16 },
    /// All channels to one, averaged
    DownMix,
}

/// A route: channels of one node's output summed into another node's input (or sidechain)
#[derive(Clone, Debug, PartialEq)]
pub struct Feed {
    pub from: NodeIndex,
    pub from_channel: u16,
    pub channels: u16,
    pub to: NodeIndex,
    pub to_channel: u16,
    pub mix: ChannelMix,
    pub sidechain: bool,
    /// A send amount or a fixed gain; `None` is unity
    pub gain: Option<HostParam>,
    /// PDC: samples of delay on this path, computed by the compiler (base item 3)
    pub delay: u32,
}

impl Feed {
    /// `channels` channels from the start of `from`'s output to the start of `to`'s input, unity gain
    pub fn new(from: NodeIndex, to: NodeIndex, channels: u16) -> Self {
        Self {
            from,
            from_channel: 0,
            channels,
            to,
            to_channel: 0,
            mix: ChannelMix::Direct,
            sidechain: false,
            gain: None,
            delay: 0,
        }
    }

    #[must_use]
    pub fn channels_from(mut self, first: u16) -> Self {
        self.from_channel = first;
        self
    }

    #[must_use]
    pub fn channels_to(mut self, first: u16) -> Self {
        self.to_channel = first;
        self
    }

    #[must_use]
    pub fn mix(mut self, mix: ChannelMix) -> Self {
        self.mix = mix;
        self
    }

    #[must_use]
    pub fn to_sidechain(mut self) -> Self {
        self.sidechain = true;
        self
    }

    #[must_use]
    pub fn gain(mut self, gain: HostParam) -> Self {
        self.gain = Some(gain);
        self
    }

    #[must_use]
    pub fn delay(mut self, samples: u32) -> Self {
        self.delay = samples;
        self
    }

    fn dest_channels(&self) -> u16 {
        match self.mix {
            ChannelMix::Direct => self.channels,
            ChannelMix::UpMix { to } => to,
            ChannelMix::DownMix => 1,
        }
    }
}

/// How often a processor parameter's automation reaches the processor
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    /// On a grid of [`AUTOMATION_GRID`] samples of the engine's running frame count (so the events fall on the same
    /// samples whatever the block size), when the value changed
    Block,
    /// Every sample where the value changed
    Sample,
}

/// The automation grid of [`Rate::Block`] parameters
pub const AUTOMATION_GRID: u64 = 32;

/// A processor parameter's binding to a slot
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamInfo {
    pub rate: Rate,
    /// `Choice`, `Stepped`, `Bool`: never interpolated nor smoothed, changes at the exact sample of a point
    pub discrete: bool,
    /// The tree's value (S20): `EngineControl::install` writes it to the slot when the slot is new
    pub initial: f32,
}

/// Where an audio item's samples come from
#[derive(Clone, Debug, PartialEq)]
pub enum AudioSource {
    /// Decoded in memory
    Memory(Arc<SampleData>),
    /// Read from the disk ahead of the playhead (P-32, `stream.rs`)
    Stream(Stream),
}

impl AudioSource {
    pub fn rate(&self) -> u32 {
        match self {
            AudioSource::Memory(d) => d.rate(),
            AudioSource::Stream(s) => s.rate(),
        }
    }

    pub fn channels(&self) -> usize {
        match self {
            AudioSource::Memory(d) => d.channels(),
            AudioSource::Stream(s) => s.channels(),
        }
    }
}

/// An audio item's playback, resolved to samples by the compiler (comping and crossfades are flattened into items)
#[derive(Clone, Debug, PartialEq)]
pub struct AudioItem {
    /// Timeline sample where the item starts
    pub start: i64,
    pub length: i64,
    pub source: AudioSource,
    /// Frame of the source played at `start`
    pub offset: i64,
    pub gain: f32,
    /// Linear fades, in samples, inside the item
    pub fade_in: u32,
    pub fade_out: u32,
}

/// One event of a MIDI item, at an absolute tick
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemEvent {
    pub tick: i64,
    pub kind: ItemEventKind,
}

/// The events of a MIDI item. Notes are numbered by the item's note map (`n`, C1)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ItemEventKind {
    NoteOn {
        n: u32,
        channel: u8,
        key: u8,
        velocity: f32,
    },
    NoteOff {
        n: u32,
    },
    Midi {
        bytes: [u8; 3],
    },
    /// Note `n`'s expression goes to `value`, arriving at tick `until` (`until == tick`: at once); the renderer turns
    /// the ticks into a ramp in samples through the tempo map
    Expression {
        n: u32,
        expression: crate::event::Expression,
        value: f32,
        until: i64,
    },
}

/// A MIDI item's events in time order, with a table of its notes by number (to find, after a program change, whether
/// a sounding note still exists and still lasts, U25)
#[derive(Debug, PartialEq)]
pub struct MidiItem {
    key: u64,
    events: Box<[ItemEvent]>,
    notes: Box<[(u32, i64)]>,
}

impl MidiItem {
    /// `key` is stable for the item across programs (the compiler hashes its `NodeId`). Events are ordered by tick,
    /// a note-off before a note-on at the same tick. Keys: `render.item.note_twice`, `render.item.off_without_on`
    pub fn new(key: u64, mut events: Vec<ItemEvent>) -> Result<Self> {
        let rank = |k: &ItemEventKind| match k {
            ItemEventKind::NoteOff { .. } => 0,
            ItemEventKind::Midi { .. } => 1,
            ItemEventKind::NoteOn { .. } => 2,
            ItemEventKind::Expression { .. } => 3,
        };
        events.sort_by(|a, b| a.tick.cmp(&b.tick).then(rank(&a.kind).cmp(&rank(&b.kind))));
        // Linear in the events (V8: one item of 10,000 notes is made again when one note moves)
        let mut notes: Vec<(u32, i64)> = Vec::with_capacity(events.len() / 2);
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::with_capacity(events.len() / 2);
        let mut open: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for e in &events {
            match e.kind {
                ItemEventKind::NoteOn { n, .. } => {
                    if !seen.insert(n) {
                        return Err(Error::new("render.item.note_twice").arg("n", i64::from(n)));
                    }
                    open.insert(n, notes.len());
                    notes.push((n, i64::MAX));
                }
                ItemEventKind::NoteOff { n } => {
                    let Some(i) = open.remove(&n) else {
                        return Err(Error::new("render.item.off_without_on").arg("n", i64::from(n)));
                    };
                    notes[i].1 = e.tick;
                }
                ItemEventKind::Midi { .. } | ItemEventKind::Expression { .. } => {}
            }
        }
        notes.sort_unstable_by_key(|(n, _)| *n);
        Ok(Self { key, events: events.into_boxed_slice(), notes: notes.into_boxed_slice() })
    }

    pub fn key(&self) -> u64 {
        self.key
    }

    pub fn events(&self) -> &[ItemEvent] {
        &self.events
    }

    /// The tick where note `n` ends, if the item has it
    pub(crate) fn note_end(&self, n: u32) -> Option<i64> {
        self.notes.binary_search_by_key(&n, |(m, _)| *m).ok().map(|i| self.notes[i].1)
    }

    /// The events whose tick is in `ticks`
    pub(crate) fn events_in(&self, ticks: Range<i64>) -> &[ItemEvent] {
        let a = self.events.partition_point(|e| e.tick < ticks.start);
        let b = self.events.partition_point(|e| e.tick < ticks.end);
        &self.events[a..b]
    }
}

/// The metronome source (I-5.5): a click per beat while playing, the accent on the first beat of a bar
#[derive(Clone, Debug, PartialEq)]
pub struct Metronome {
    pub accent: Arc<SampleData>,
    pub beat: Arc<SampleData>,
    pub gain: f32,
    pub enabled: bool,
}

/// Live MIDI routing: the port table (I-7.1). `channel: None` takes all channels
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortRoute {
    pub port: PortIndex,
    pub channel: Option<u8>,
    pub node: NodeIndex,
}

/// A node's output channel summed into a device output channel
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputRoute {
    pub node: NodeIndex,
    pub channel: u16,
    pub device_channel: u16,
}

/// When a live MIDI message plays (R14)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MidiTiming {
    /// One block of fixed latency (`max_block` samples): no jitter. The default
    #[default]
    FixedBlock,
    /// As soon as possible (offset 0 of the block it arrives in): less latency, with jitter (a preference)
    Immediate,
}

/// The count-in (I-5.5): `bars` bars of the metronome before the music starts, at the meter and tempo where it
/// starts. The clicks fall where the beats of the bars that follow the start fall, so the music starts exactly one
/// count-in after the start command
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountIn {
    pub bars: u32,
    /// Also when playing; otherwise only when the transport starts with recording armed
    pub on_play: bool,
}

/// The persistent transport settings of the tree (I-7.2), in samples
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransportSettings {
    /// `from..to`, at least 50 ms long; playing past `to` jumps to `from` at the exact sample
    pub loop_range: Option<(i64, i64)>,
    /// Recording happens only inside `from..to` (U4: the handles outside are the recorder's business)
    pub punch: Option<(i64, i64)>,
    pub midi: MidiTiming,
    /// The count-in before the music starts (I-5.5)
    pub count_in: Option<CountIn>,
}

// ---------------------------------------------------------------------------------------------------------------
// The built program

/// A node's own memory while the program is installed: its contiguous buffer block and its event list
#[derive(Debug)]
pub(crate) struct NodeIo {
    pub(crate) layout: BufferLayout,
    pub(crate) buffer: Box<[f32]>,
    pub(crate) events: EventList,
}

#[derive(Debug)]
pub(crate) struct PNode {
    pub(crate) desc: NodeDesc,
    pub(crate) io: Option<Box<NodeIo>>,
    pub(crate) feeds: Range<u32>,
    pub(crate) audio: Box<[AudioItem]>,
    pub(crate) audio_max_len: i64,
    pub(crate) input: Box<[u16]>,
    pub(crate) gain: Option<u32>,
    pub(crate) pan: Option<u32>,
    pub(crate) has_midi: bool,
}

#[derive(Debug)]
pub(crate) struct PFeed {
    pub(crate) from: u32,
    pub(crate) to: u32,
    pub(crate) feed: Feed,
    pub(crate) gain: Option<u32>,
    pub(crate) delay: Option<u32>,
}

/// The identity of a delay line across programs: the history of a line moves to the new program's line with the
/// same key at install time, so a recompilation does not click
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DelayKey {
    from: NodeIndex,
    to: NodeIndex,
    from_channel: u16,
    to_channel: u16,
    sidechain: bool,
}

#[derive(Debug)]
pub(crate) struct DelayLine {
    pub(crate) key: DelayKey,
    pub(crate) channels: usize,
    pub(crate) length: usize,
    /// `channels × length`, channel after channel
    pub(crate) memory: Box<[f32]>,
    /// The next slot to read and overwrite
    pub(crate) pos: usize,
}

impl DelayLine {
    /// Takes over the newest part of `old`'s history (no allocation)
    pub(crate) fn inherit(&mut self, old: &DelayLine) {
        let keep = self.length.min(old.length);
        let channels = self.channels.min(old.channels);
        for ch in 0..channels {
            for k in 0..keep {
                // k = 1 is the most recent sample of each line
                let src = (old.pos + old.length - 1 - k) % old.length;
                let dst = (self.pos + self.length - 1 - k) % self.length;
                self.memory[ch * self.length + dst] = old.memory[ch * old.length + src];
            }
        }
    }
}

/// How a parameter's automation plays (I-21.5). `Off` is not here: an automation lane that is off is not compiled
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutomationMode {
    /// The curve plays; a touch wins while it lasts, then back to the curve in 50 ms
    #[default]
    Read,
    /// As `Read`; the session records while touched
    Touch,
    /// From the first touch while playing, the curve stops and the last value holds until the transport stops
    Latch,
    /// While playing, the curve never plays: the control's value is what plays (and is recorded)
    Write,
}

/// What a parameter slot drives
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Processor { node: u32, index: u32 },
    Host(u32),
}

#[derive(Debug)]
pub(crate) struct Binding {
    pub(crate) slot: u32,
    pub(crate) target: Target,
    pub(crate) rate: Rate,
    pub(crate) discrete: bool,
    pub(crate) initial: f32,
    pub(crate) curve: Option<Curve>,
    pub(crate) mode: AutomationMode,
}

#[derive(Debug)]
pub(crate) struct PHostParam {
    pub(crate) value: f32,
    pub(crate) binding: Option<u32>,
    pub(crate) complement: bool,
}

#[derive(Debug)]
pub(crate) struct MidiSource {
    pub(crate) node: u32,
    pub(crate) items: Box<[Arc<MidiItem>]>,
}

pub(crate) const NONE: u32 = u32::MAX;

/// A compiled program. Made by [`ProgramBuilder::build`]; owned by the renderer while installed
#[derive(Debug)]
pub struct Program {
    pub(crate) generation: ProgramGen,
    pub(crate) spec: ProcessSpec,
    pub(crate) tempo: TempoMap,
    pub(crate) transport: TransportSettings,
    pub(crate) nodes: Box<[PNode]>,
    pub(crate) levels: Box<[Box<[u32]>]>,
    pub(crate) feeds: Box<[PFeed]>,
    pub(crate) delays: Box<[DelayLine]>,
    pub(crate) host_params: Box<[PHostParam]>,
    pub(crate) bindings: Box<[Binding]>,
    pub(crate) automated: Box<[u32]>,
    pub(crate) slot_map: Box<[u32]>,
    pub(crate) node_map: Box<[u32]>,
    pub(crate) ports: Box<[PortRoute]>,
    pub(crate) outputs: Box<[OutputRoute]>,
    pub(crate) master: Option<u32>,
    pub(crate) preview: Option<u32>,
    pub(crate) metronome: Option<(u32, Metronome)>,
    pub(crate) midi: Box<[MidiSource]>,
    /// The streamed audio items, by node and item (the renderer tells them where the playhead is)
    pub(crate) streams: Box<[(u32, u32)]>,
}

impl Program {
    /// The empty program `engine()` starts with (generation 0, 120 BPM)
    pub fn empty(spec: ProcessSpec, limits: &Limits) -> Box<Program> {
        let tempo = TempoMap::constant(
            spec.sample_rate.clamp(1, lm_time::MAX_SAMPLE_RATE),
            lm_time::Ppq::DEFAULT,
            lm_time::Bpm::from_integer(120).expect("120 BPM"),
            lm_time::TimeSignature::COMMON,
        )
        .expect("a constant map");
        ProgramBuilder::new(ProgramGen(0), spec, limits, tempo).build().expect("the empty program")
    }

    pub fn generation(&self) -> ProgramGen {
        self.generation
    }

    pub fn tempo(&self) -> &TempoMap {
        &self.tempo
    }

    pub fn transport(&self) -> &TransportSettings {
        &self.transport
    }

    /// The nodes, in the order they were given
    pub fn nodes(&self) -> impl Iterator<Item = &NodeDesc> {
        self.nodes.iter().map(|n| &n.desc)
    }

    /// How many levels the graph has (the longest chain)
    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// The slots that are new in this program's parameter table, with their initial values (S20)
    pub fn param_initials(&self) -> impl Iterator<Item = (ParamSlot, f32)> + '_ {
        self.bindings.iter().map(|b| (ParamSlot(b.slot), b.initial))
    }

    pub(crate) fn node_of(&self, node: NodeIndex) -> Option<u32> {
        let i = *self.node_map.get(node.slot())?;
        (i != NONE && self.nodes[i as usize].desc.node.generation == node.generation).then_some(i)
    }
}

/// Builds and checks a [`Program`]. The compiler's only way to make one
#[derive(Debug)]
pub struct ProgramBuilder {
    generation: ProgramGen,
    spec: ProcessSpec,
    limits: Limits,
    tempo: TempoMap,
    transport: TransportSettings,
    nodes: Vec<NodeDesc>,
    feeds: Vec<Feed>,
    params: Vec<(ParamSlot, NodeIndex, u32, ParamInfo, Option<Curve>)>,
    audio: Vec<(NodeIndex, Vec<AudioItem>)>,
    inputs: Vec<(NodeIndex, Vec<u16>)>,
    midi: Vec<(NodeIndex, Vec<Arc<MidiItem>>)>,
    ports: Vec<PortRoute>,
    outputs: Vec<OutputRoute>,
    master: Option<NodeIndex>,
    preview: Option<NodeIndex>,
    metronome: Option<(NodeIndex, Metronome)>,
    modes: Vec<(ParamSlot, AutomationMode)>,
}

fn invalid(key: &'static str) -> Error {
    Error::new(key)
}

fn node_arg(e: Error, node: NodeIndex) -> Error {
    e.arg("node", i64::from(node.index)).arg("generation", i64::from(node.generation))
}

impl ProgramBuilder {
    pub fn new(generation: ProgramGen, spec: ProcessSpec, limits: &Limits, tempo: TempoMap) -> Self {
        Self {
            generation,
            spec,
            limits: *limits,
            tempo,
            transport: TransportSettings::default(),
            nodes: Vec::new(),
            feeds: Vec::new(),
            params: Vec::new(),
            audio: Vec::new(),
            inputs: Vec::new(),
            midi: Vec::new(),
            ports: Vec::new(),
            outputs: Vec::new(),
            master: None,
            preview: None,
            metronome: None,
            modes: Vec::new(),
        }
    }

    /// The automation mode of a slot's curve (default `Read`)
    pub fn automation_mode(&mut self, slot: ParamSlot, mode: AutomationMode) -> &mut Self {
        self.modes.push((slot, mode));
        self
    }

    pub fn node(&mut self, desc: NodeDesc) -> &mut Self {
        self.nodes.push(desc);
        self
    }

    pub fn feed(&mut self, feed: Feed) -> &mut Self {
        self.feeds.push(feed);
        self
    }

    /// Binds a slot to parameter `index` of the processor in `node`, with its automation
    pub fn param(
        &mut self,
        slot: ParamSlot,
        node: NodeIndex,
        index: u32,
        info: ParamInfo,
        automation: Option<Curve>,
    ) -> &mut Self {
        self.params.push((slot, node, index, info, automation));
        self
    }

    /// The audio items played into `node`'s input
    pub fn audio_items(&mut self, node: NodeIndex, items: Vec<AudioItem>) -> &mut Self {
        self.audio.push((node, items));
        self
    }

    /// Device input channels written into `node`'s input channels, in order (monitoring, outside the PDC, H5)
    pub fn input(&mut self, node: NodeIndex, device_channels: Vec<u16>) -> &mut Self {
        self.inputs.push((node, device_channels));
        self
    }

    /// The MIDI items whose notes play `node`'s processor
    pub fn midi_items(&mut self, node: NodeIndex, items: Vec<Arc<MidiItem>>) -> &mut Self {
        self.midi.push((node, items));
        self
    }

    pub fn port(&mut self, route: PortRoute) -> &mut Self {
        self.ports.push(route);
        self
    }

    pub fn output(&mut self, route: OutputRoute) -> &mut Self {
        self.outputs.push(route);
        self
    }

    /// The master: its output gets the ear protection (U20)
    pub fn master(&mut self, node: NodeIndex) -> &mut Self {
        self.master = Some(node);
        self
    }

    /// Where the preview bus is summed (I-7.3)
    pub fn preview_bus(&mut self, node: NodeIndex) -> &mut Self {
        self.preview = Some(node);
        self
    }

    pub fn metronome(&mut self, node: NodeIndex, metronome: Metronome) -> &mut Self {
        self.metronome = Some((node, metronome));
        self
    }

    pub fn transport(&mut self, settings: TransportSettings) -> &mut Self {
        self.transport = settings;
        self
    }

    /// Checks everything and allocates the program. Error keys `render.program.*` with the node or slot in the
    /// arguments
    pub fn build(self) -> Result<Box<Program>> {
        let limits = self.limits;
        let max_block = self.spec.max_block;
        if max_block == 0 || self.spec.sample_rate == 0 {
            return Err(invalid("render.program.spec_invalid"));
        }
        if self.tempo.sample_rate() != self.spec.sample_rate {
            return Err(invalid("render.program.tempo_rate_mismatch"));
        }

        // Nodes
        let mut node_map = vec![NONE; limits.max_nodes].into_boxed_slice();
        for (i, d) in self.nodes.iter().enumerate() {
            if d.node.slot() >= limits.max_nodes {
                return Err(node_arg(invalid("render.program.node_out_of_range"), d.node));
            }
            if node_map[d.node.slot()] != NONE {
                return Err(node_arg(invalid("render.program.node_twice"), d.node));
            }
            if d.in_place && d.inputs != d.outputs {
                return Err(node_arg(invalid("render.program.in_place_channels"), d.node));
            }
            if d.kind == NodeKind::Sum && (!d.in_place || d.sidechain != 0) {
                return Err(node_arg(invalid("render.program.sum_shape"), d.node));
            }
            if d.tail && d.kind != NodeKind::Processor {
                return Err(node_arg(invalid("render.program.tail_not_processor"), d.node));
            }
            node_map[d.node.slot()] = i as u32;
        }
        let find = |n: NodeIndex| -> Result<u32> {
            match node_map.get(n.slot()) {
                Some(&i) if i != NONE && self.nodes[i as usize].node == n => Ok(i),
                _ => Err(node_arg(invalid("render.program.unknown_node"), n)),
            }
        };

        // Host parameters and bindings
        let mut host_params: Vec<PHostParam> = Vec::new();
        let mut bindings: Vec<Binding> = Vec::new();
        let mut slot_map = vec![NONE; limits.max_params].into_boxed_slice();
        let mut bind = |slot: ParamSlot,
                        target: Target,
                        rate: Rate,
                        discrete: bool,
                        initial: f32,
                        curve: Option<Curve>,
                        bindings: &mut Vec<Binding>|
         -> Result<u32> {
            let s = slot.0 as usize;
            if s >= limits.max_params {
                return Err(invalid("render.program.slot_out_of_range").arg("slot", i64::from(slot.0)));
            }
            if slot_map[s] != NONE {
                return Err(invalid("render.program.slot_twice").arg("slot", i64::from(slot.0)));
            }
            if !initial.is_finite() {
                return Err(invalid("render.program.initial_not_finite").arg("slot", i64::from(slot.0)));
            }
            slot_map[s] = bindings.len() as u32;
            bindings.push(Binding { slot: slot.0, target, rate, discrete, initial, curve, mode: AutomationMode::Read });
            Ok(bindings.len() as u32 - 1)
        };
        let mut host = |p: &HostParam, bindings: &mut Vec<Binding>, host_params: &mut Vec<PHostParam>| -> Result<u32> {
            if !p.value.is_finite() {
                return Err(invalid("render.program.gain_not_finite"));
            }
            let index = host_params.len() as u32;
            let binding = match p.slot {
                Some(slot) => {
                    Some(bind(slot, Target::Host(index), Rate::Sample, false, p.value, p.automation.clone(), bindings)?)
                }
                None if p.automation.is_some() => return Err(invalid("render.program.automation_without_slot")),
                None => None,
            };
            host_params.push(PHostParam { value: p.value, binding, complement: p.complement });
            Ok(index)
        };

        let mut pnodes: Vec<PNode> = Vec::with_capacity(self.nodes.len());
        for d in &self.nodes {
            let (gain, pan) = match &d.strip {
                Some(s) => {
                    let g = host(&s.gain, &mut bindings, &mut host_params)?;
                    let p = match &s.pan {
                        Some(p) => Some(host(p, &mut bindings, &mut host_params)?),
                        None => None,
                    };
                    (Some(g), p)
                }
                None => (None, None),
            };
            pnodes.push(PNode {
                desc: d.clone(),
                io: None,
                feeds: 0..0,
                audio: Box::new([]),
                audio_max_len: 0,
                input: Box::new([]),
                gain,
                pan,
                has_midi: false,
            });
        }

        // Feeds, grouped by destination in the order given
        let mut pfeeds: Vec<PFeed> = Vec::with_capacity(self.feeds.len());
        let mut edges: Vec<(u32, u32)> = Vec::with_capacity(self.feeds.len());
        for f in &self.feeds {
            let from = find(f.from)?;
            let to = find(f.to)?;
            let src = &self.nodes[from as usize];
            let dst = &self.nodes[to as usize];
            let bad = |key| node_arg(invalid(key), f.to);
            if f.channels == 0 || u32::from(f.from_channel) + u32::from(f.channels) > u32::from(src.outputs) {
                return Err(bad("render.program.feed_source_channels"));
            }
            let room = if f.sidechain { dst.sidechain } else { dst.inputs };
            if u32::from(f.to_channel) + u32::from(f.dest_channels()) > u32::from(room) {
                return Err(bad("render.program.feed_dest_channels"));
            }
            match f.mix {
                ChannelMix::UpMix { to } if f.channels != 1 || to == 0 => {
                    return Err(bad("render.program.upmix_shape"));
                }
                _ => {}
            }
            if from == to {
                return Err(bad("render.program.cycle"));
            }
            edges.push((from, to));
            let gain = match &f.gain {
                Some(g) => Some(host(g, &mut bindings, &mut host_params)?),
                None => None,
            };
            pfeeds.push(PFeed { from, to, feed: f.clone(), gain, delay: None });
        }
        pfeeds.sort_by_key(|f| f.to); // stable: keeps the given order inside a destination
        let mut start = 0usize;
        while start < pfeeds.len() {
            let to = pfeeds[start].to;
            let mut end = start;
            while end < pfeeds.len() && pfeeds[end].to == to {
                end += 1;
            }
            pnodes[to as usize].feeds = start as u32..end as u32;
            start = end;
        }

        // Levels (Kahn), each node one level after its latest predecessor; order inside a level is the given order
        let n = pnodes.len();
        let mut indegree = vec![0u32; n];
        let mut succ: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &(a, b) in &edges {
            indegree[b as usize] += 1;
            succ[a as usize].push(b);
        }
        let mut level = vec![0u32; n];
        let mut ready: Vec<u32> = (0..n as u32).filter(|&i| indegree[i as usize] == 0).collect();
        let mut done = 0usize;
        while let Some(i) = ready.pop() {
            done += 1;
            for &s in &succ[i as usize] {
                level[s as usize] = level[s as usize].max(level[i as usize] + 1);
                indegree[s as usize] -= 1;
                if indegree[s as usize] == 0 {
                    ready.push(s);
                }
            }
        }
        if done != n {
            return Err(invalid("render.program.cycle"));
        }
        let level_count = level.iter().copied().max().map_or(0, |m| m as usize + 1);
        let mut levels: Vec<Vec<u32>> = vec![Vec::new(); level_count];
        for (i, &l) in level.iter().enumerate() {
            levels[l as usize].push(i as u32);
        }

        // Delay lines (PDC), keyed for inheritance
        let mut delays: Vec<DelayLine> = Vec::new();
        for f in &pfeeds {
            if f.feed.delay > 0 {
                let key = DelayKey {
                    from: f.feed.from,
                    to: f.feed.to,
                    from_channel: f.feed.from_channel,
                    to_channel: f.feed.to_channel,
                    sidechain: f.feed.sidechain,
                };
                let channels = usize::from(f.feed.channels);
                let length = f.feed.delay as usize;
                delays.push(DelayLine {
                    key,
                    channels,
                    length,
                    memory: vec![0.0; channels * length].into_boxed_slice(),
                    pos: 0,
                });
            }
        }
        delays.sort_by_key(|d| d.key);
        for f in &mut pfeeds {
            if f.feed.delay > 0 {
                let key = DelayKey {
                    from: f.feed.from,
                    to: f.feed.to,
                    from_channel: f.feed.from_channel,
                    to_channel: f.feed.to_channel,
                    sidechain: f.feed.sidechain,
                };
                let i = delays.partition_point(|d| d.key < key);
                if delays.get(i).is_some_and(|d| d.key == key) {
                    if delays.iter().filter(|d| d.key == key).count() > 1 {
                        return Err(node_arg(invalid("render.program.feed_twice"), f.feed.to));
                    }
                    f.delay = Some(i as u32);
                }
            }
        }

        // Processor parameters
        let mut event_room = vec![0usize; n];
        for (slot, node, index, info, curve) in self.params {
            let i = find(node)?;
            if pnodes[i as usize].desc.kind != NodeKind::Processor {
                return Err(node_arg(invalid("render.program.param_on_sum"), node));
            }
            bind(
                slot,
                Target::Processor { node: i, index },
                info.rate,
                info.discrete,
                info.initial,
                curve,
                &mut bindings,
            )?;
            event_room[i as usize] += match info.rate {
                Rate::Block => max_block / AUTOMATION_GRID as usize + 2,
                Rate::Sample => max_block + 1,
            } + 2;
        }
        for (slot, mode) in &self.modes {
            if let Some(&bi) = slot_map.get(slot.0 as usize)
                && bi != NONE
            {
                bindings[bi as usize].mode = *mode;
            }
        }
        let automated: Vec<u32> =
            bindings.iter().enumerate().filter(|(_, b)| b.curve.is_some()).map(|(i, _)| i as u32).collect();

        // Sources
        let rate = self.spec.sample_rate;
        let mut streams: Vec<(u32, u32)> = Vec::new();
        for (node, mut items) in self.audio {
            let i = find(node)? as usize;
            if pnodes[i].desc.inputs == 0 {
                return Err(node_arg(invalid("render.program.source_without_inputs"), node));
            }
            for it in &items {
                if it.source.rate() != rate {
                    return Err(node_arg(invalid("render.program.sample_rate_mismatch"), node));
                }
                if it.length < 0 || it.offset < 0 || !it.gain.is_finite() {
                    return Err(node_arg(invalid("render.program.audio_item_invalid"), node));
                }
            }
            items.sort_by_key(|it| it.start);
            for (k, it) in items.iter().enumerate() {
                if matches!(it.source, AudioSource::Stream(_)) {
                    streams.push((i as u32, k as u32));
                }
            }
            pnodes[i].audio_max_len = items.iter().map(|it| it.length).max().unwrap_or(0);
            pnodes[i].audio = items.into_boxed_slice();
        }
        for (node, chans) in self.inputs {
            let i = find(node)? as usize;
            if chans.len() > usize::from(pnodes[i].desc.inputs) {
                return Err(node_arg(invalid("render.program.input_channels"), node));
            }
            pnodes[i].input = chans.into_boxed_slice();
        }
        let mut midi = Vec::with_capacity(self.midi.len());
        for (node, items) in self.midi {
            let i = find(node)?;
            if pnodes[i as usize].desc.kind != NodeKind::Processor {
                return Err(node_arg(invalid("render.program.midi_on_sum"), node));
            }
            pnodes[i as usize].has_midi = true;
            midi.push(MidiSource { node: i, items: items.into_boxed_slice() });
        }
        let mut ports = self.ports;
        for r in &ports {
            let i = find(r.node)?;
            if pnodes[i as usize].desc.kind != NodeKind::Processor {
                return Err(node_arg(invalid("render.program.midi_on_sum"), r.node));
            }
            pnodes[i as usize].has_midi = true;
        }
        ports.sort_by_key(|r| r.port);
        for o in &self.outputs {
            let i = find(o.node)?;
            if o.channel >= pnodes[i as usize].desc.outputs {
                return Err(node_arg(invalid("render.program.output_channel"), o.node));
            }
        }
        let master = self.master.map(find).transpose()?;
        let preview = self.preview.map(find).transpose()?;
        if let Some(p) = preview
            && pnodes[p as usize].desc.inputs == 0
        {
            return Err(invalid("render.program.source_without_inputs"));
        }
        let metronome = match self.metronome {
            Some((node, m)) => {
                let i = find(node)?;
                if pnodes[i as usize].desc.inputs == 0 {
                    return Err(node_arg(invalid("render.program.source_without_inputs"), node));
                }
                if m.accent.rate() != rate || m.beat.rate() != rate {
                    return Err(node_arg(invalid("render.program.sample_rate_mismatch"), node));
                }
                Some((i, m))
            }
            None => None,
        };

        // Transport
        // At least 50 ms: the record spans of a block and the lossless transport ring are sized on it (C4)
        let min_loop = (self.spec.sample_rate / 20).max(1) as i64;
        if let Some((a, b)) = self.transport.loop_range
            && b - a < min_loop
        {
            return Err(invalid("render.program.loop_too_short").arg("min", min_loop));
        }
        if let Some((a, b)) = self.transport.punch
            && b <= a
        {
            return Err(invalid("render.program.punch_empty"));
        }

        // Memory: one block and one event list per node
        for (i, p) in pnodes.iter_mut().enumerate() {
            let d = &p.desc;
            let layout = BufferLayout {
                inputs: usize::from(d.inputs),
                outputs: usize::from(d.outputs),
                sidechain: usize::from(d.sidechain),
                in_place: d.in_place,
                stride: max_block,
            };
            let events = 64 + event_room[i] + if p.has_midi { 1024 } else { 0 };
            p.io = Some(Box::new(NodeIo {
                layout,
                buffer: vec![0.0; layout.len()].into_boxed_slice(),
                events: EventList::with_capacity(events),
            }));
        }

        Ok(Box::new(Program {
            generation: self.generation,
            spec: self.spec,
            tempo: self.tempo,
            transport: self.transport,
            nodes: pnodes.into_boxed_slice(),
            levels: levels.into_iter().map(Vec::into_boxed_slice).collect(),
            feeds: pfeeds.into_boxed_slice(),
            delays: delays.into_boxed_slice(),
            host_params: host_params.into_boxed_slice(),
            bindings: bindings.into_boxed_slice(),
            automated: automated.into_boxed_slice(),
            slot_map,
            node_map,
            ports: ports.into_boxed_slice(),
            outputs: self.outputs.into_boxed_slice(),
            master,
            preview,
            metronome,
            midi: midi.into_boxed_slice(),
            streams: streams.into_boxed_slice(),
        }))
    }
}
