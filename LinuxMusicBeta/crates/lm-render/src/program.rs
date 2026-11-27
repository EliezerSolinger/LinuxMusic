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
//! - [`NodeKind::Sum`]: no processor; its output is the sum of its inputs (a desk channel's input, the master's);
//! - [`NodeKind::NoteEffect`]: a processor with no audio, in a chain before an instrument ([`ProgramBuilder::note_chain`]):
//!   what the sources push for the instrument goes through the chain's effects, in order, before the pass starts.
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
use crate::event::{EventList, NoteOut};
use crate::ids::{NodeIndex, ParamSlot, PortIndex, ProgramGen};
use crate::midimap::ParamRange;
use crate::modulation::{Envelope, MOD_PARAMS, ModKind, ModulatorSpec, PeakTracker};
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
    /// A note effect: a processor with no audio that turns the note stream of the instrument it is chained before
    /// into another one ([`crate::Processor::process_notes`]). It runs inside the pass of its chain, not in a level
    NoteEffect,
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

    /// A note effect: no audio, its own parameters and events (chain it with [`ProgramBuilder::note_chain`])
    pub fn note_effect(node: NodeIndex) -> Self {
        Self { kind: NodeKind::NoteEffect, ..Self::processor(node, 0, 0) }
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
    /// Fades, in samples, inside the item
    pub fade_in: u32,
    pub fade_out: u32,
    /// The fades' tension, -1..1: 0 is linear, above it the fade starts slow, below it fast (see `fade_shape`)
    pub fade_in_curve: f32,
    pub fade_out_curve: f32,
    /// The fade is the half of an automatic crossfade of overlapping items at equal power (sine, F-CLP-062); a
    /// tension other than 0 overrides it
    pub fade_in_equal: bool,
    pub fade_out_equal: bool,
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
    /// Plays only while recording (the preference "only recording", F-REC-025); the count-in always sounds
    pub only_recording: bool,
}

/// Live MIDI routing: the port table (I-7.1). `channel: None` takes all channels
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortRoute {
    pub port: PortIndex,
    pub channel: Option<u8>,
    pub node: NodeIndex,
    /// The legacy's "fixed input velocity" (F-IJN-017): every note-on takes this velocity (1..=127)
    pub fixed_velocity: Option<u8>,
}

/// How a track shapes its notes, from the items and from the live MIDI input (F-IJN-017/019/021): the project's
/// transposition, the keys it plays (checked on the transposed key, as the legacy's `isKeyMapped`), the base key
/// shift in semitones, the host velocity scale (`baseVelocity / 63`), and, with the microtuner on, each key's
/// frequency as semitones from its 12-TET pitch
#[derive(Clone, Debug, PartialEq)]
pub struct LiveKeys {
    /// The project's transposition for a track that follows it, added to every key first
    pub transpose: i8,
    pub first: u8,
    pub last: u8,
    /// Semitones added to the key (the legacy `DefaultKey - baseNote`); 0 with the microtuner on
    pub shift: i8,
    pub velocity_scale: f32,
    /// By the transposed key: the tuning offset in semitones, or `NaN` for a key the keymap does not map
    pub tuning: Option<Box<[f32; 128]>>,
}

impl Default for LiveKeys {
    fn default() -> Self {
        Self { transpose: 0, first: 0, last: 127, shift: 0, velocity_scale: 1.0, tuning: None }
    }
}

impl LiveKeys {
    /// The note a played key becomes: its key and tuning offset in semitones, or `None` when the track does not
    /// play it (out of its keys, unmapped by the keymap, or out of 0..=127 after the shifts)
    pub fn map(&self, key: u8) -> Option<(u8, f32)> {
        let tk = i16::from(key) + i16::from(self.transpose);
        if !(0..=127).contains(&tk) {
            return None;
        }
        let tk = tk as u8;
        if tk < self.first || tk > self.last {
            return None;
        }
        let mut offset = match &self.tuning {
            Some(t) => {
                let v = t[usize::from(tk)];
                if v.is_nan() {
                    return None;
                }
                v
            }
            None => 0.0,
        };
        let mut played = i16::from(tk) + i16::from(self.shift);
        if !(0..=127).contains(&played) {
            return None;
        }
        // A tuning far from the key (a keymap that is not an octave map) moves the key, so the offset stays small
        if offset.abs() > 24.0 {
            let moved = (played + offset.round() as i16).clamp(0, 127);
            offset = (offset - f32::from(moved - played)).clamp(-24.0, 24.0);
            played = moved;
        }
        Some((played as u8, offset))
    }

    /// A MIDI velocity (1..=127) through the host curve
    pub fn velocity(&self, vel: u8) -> u8 {
        ((f32::from(vel) * self.velocity_scale).round() as i32).clamp(1, 127) as u8
    }
}

/// A node's notes and controls out to an external device (P-38): on `channel` (0..16), with the legacy's fixed
/// velocity and fixed note (F-IJN-017). The same note-off ends the key its note-on was sent on
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiOutRoute {
    pub node: NodeIndex,
    pub port: PortIndex,
    pub channel: u8,
    pub fixed_velocity: Option<u8>,
    pub fixed_note: Option<u8>,
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
    /// Bars played before a recording starts (F-REC-023): the transport starts that many bars (by the tempo map)
    /// before the playhead, and recording begins when it gets back to it. 0: none
    pub pre_roll_bars: u32,
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
    /// The chain of note effects before this node (an index into `Program::chains`), or `NONE`
    pub(crate) chain: u32,
}

/// The events one chain holds at once (the capacity of each of its lists)
pub(crate) const CHAIN_EVENTS: usize = 2048;

/// The note effects of an instrument, in the order they run: the notes its sources make (items, live input) go
/// through them, and what comes out is what the instrument plays and what MIDI out sends
#[derive(Debug)]
pub(crate) struct Chain {
    pub(crate) target: u32,
    pub(crate) effects: Box<[u32]>,
    /// What the sources pushed for the target this pass
    pub(crate) source: EventList,
    /// A stage's input: the stream so far, with the effect's own parameter events
    pub(crate) merge: EventList,
    pub(crate) cur: NoteOut,
    pub(crate) next: NoteOut,
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
    Processor {
        node: u32,
        index: u32,
    },
    Host(u32),
    /// A controller sent to the node's processor as a MIDI message (the track's CC rack, F-IJN-025): the value is
    /// 0..=127, rounded; a message only when the rounded value changes
    Cc {
        node: u32,
        channel: u8,
        number: u8,
    },
    /// A modulator's own parameter (its speed, its macro value...): the modulator reads it
    Modulator {
        modulator: u32,
        index: u32,
    },
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
    /// A modulator drives this parameter: its modulator and the target in it. The automation and the document's
    /// value are the base the modulation is added to (the renderer emits the sum)
    pub(crate) modulated: Option<(u32, u32)>,
}

/// A compiled target of a modulator
#[derive(Debug)]
pub(crate) struct PTarget {
    pub(crate) slot: u32,
    pub(crate) range: ParamRange,
    pub(crate) amount: f32,
    pub(crate) min: f32,
    pub(crate) max: f32,
    /// The last value emitted (NaN: none yet, or a hand held the parameter)
    pub(crate) last: f32,
    /// A host parameter's modulation (`amount * b`) on each step of the pass's grid (see `PModulator::first_grid`)
    pub(crate) steps: Box<[f32]>,
}

/// A modulator with its running state
#[derive(Debug)]
pub(crate) struct PModulator {
    pub(crate) kind: ModKind,
    pub(crate) seed: u64,
    pub(crate) user_wave: Option<Arc<[f32]>>,
    pub(crate) params: [f32; MOD_PARAMS],
    /// The binding of each parameter, for its automation
    pub(crate) param_bindings: [u32; MOD_PARAMS],
    pub(crate) targets: Box<[PTarget]>,
    pub(crate) input: u32,
    pub(crate) trigger: u32,
    pub(crate) envelope: Envelope,
    pub(crate) peak: PeakTracker,
    /// A gate change of the pass: its offset and state (the last change wins; the first offset counts)
    pub(crate) gate: Option<(u32, bool)>,
    /// `b` on each step of the pass's grid
    pub(crate) values: Box<[f32]>,
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
    /// The live input shaping of the nodes that have one
    pub(crate) live_keys: Box<[(NodeIndex, LiveKeys)]>,
    /// The ports that are MPE zones, with the last expression values of their channels
    pub(crate) mpe: Box<[crate::mpe::MpePort]>,
    /// By MIDI out route: the MPE zone of its port, with the channels of the sounding notes
    pub(crate) midi_mpe: Box<[Option<crate::mpe::MpeOut>]>,
    /// MIDI out to devices, by the node's program index
    pub(crate) midi_out: Box<[(u32, MidiOutRoute)]>,
    /// For each route, the key each sounding note was sent on (−1: none), by the note's key: its note-off ends that
    /// key, though the fixed note changed since
    pub(crate) midi_sent: Box<[i8]>,
    pub(crate) outputs: Box<[OutputRoute]>,
    pub(crate) master: Option<u32>,
    pub(crate) preview: Option<u32>,
    pub(crate) metronome: Option<(u32, Metronome)>,
    pub(crate) midi: Box<[MidiSource]>,
    pub(crate) chains: Box<[Chain]>,
    /// The streamed audio items, by node and item (the renderer tells them where the playhead is)
    pub(crate) streams: Box<[(u32, u32)]>,
    /// The modulators (decisoes-autonomas, 26/11, item 1)
    pub(crate) modulators: Box<[PModulator]>,
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
    param_limits: Vec<(NodeIndex, u32, f32, f32)>,
    audio: Vec<(NodeIndex, Vec<AudioItem>)>,
    inputs: Vec<(NodeIndex, Vec<u16>)>,
    midi: Vec<(NodeIndex, Vec<Arc<MidiItem>>)>,
    ports: Vec<PortRoute>,
    live_keys: Vec<(NodeIndex, LiveKeys)>,
    mpe: Vec<crate::mpe::MpePort>,
    outputs: Vec<OutputRoute>,
    master: Option<NodeIndex>,
    preview: Option<NodeIndex>,
    metronome: Option<(NodeIndex, Metronome)>,
    modes: Vec<(ParamSlot, AutomationMode)>,
    midi_out: Vec<MidiOutRoute>,
    ccs: Vec<(ParamSlot, NodeIndex, u8, u8, f32, Option<Curve>)>,
    mods: Vec<ModulatorSpec>,
    mod_params: Vec<(ParamSlot, usize, usize, ParamInfo, Option<Curve>)>,
    chains: Vec<(NodeIndex, Vec<NodeIndex>)>,
}

/// The most parameters of one processor that are given a declared range
const MAX_LIMITED_PARAMS: u32 = 4096;

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
            param_limits: Vec::new(),
            audio: Vec::new(),
            inputs: Vec::new(),
            midi: Vec::new(),
            ports: Vec::new(),
            live_keys: Vec::new(),
            mpe: Vec::new(),
            outputs: Vec::new(),
            master: None,
            preview: None,
            metronome: None,
            modes: Vec::new(),
            midi_out: Vec::new(),
            ccs: Vec::new(),
            mods: Vec::new(),
            mod_params: Vec::new(),
            chains: Vec::new(),
        }
    }

    /// The note effects chained before `instrument`, in order: the stream its sources make runs through them and
    /// the instrument (and its MIDI out) gets the result. Each effect is a [`NodeDesc::note_effect`]; an instrument
    /// has at most one chain
    pub fn note_chain(&mut self, instrument: NodeIndex, effects: Vec<NodeIndex>) -> &mut Self {
        self.chains.push((instrument, effects));
        self
    }

    /// Adds a modulator; gives its index (for [`ProgramBuilder::modulator_param`])
    pub fn modulator(&mut self, spec: ModulatorSpec) -> usize {
        self.mods.push(spec);
        self.mods.len() - 1
    }

    /// Binds a slot to parameter `index` (`modulation::P_*`) of modulator `modulator`, with its automation
    pub fn modulator_param(
        &mut self,
        slot: ParamSlot,
        modulator: usize,
        index: usize,
        info: ParamInfo,
        automation: Option<Curve>,
    ) -> &mut Self {
        self.mod_params.push((slot, modulator, index, info, automation));
        self
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

    /// The declared range of parameter `index` of the processor in `node`: every value that reaches the processor
    /// (document, automation, modulation, MIDI, a hand) is limited to it, so no processor repeats the check. A
    /// reversed pair is read as (lo, hi); one that is not finite is ignored. Without it the parameter is not limited
    pub fn param_limits(&mut self, node: NodeIndex, index: u32, min: f32, max: f32) -> &mut Self {
        if min.is_finite() && max.is_finite() {
            self.param_limits.push((node, index, min.min(max), min.max(max)));
        }
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

    /// A port that is an MPE zone: its member channels' bend, pressure and CC 74 become per-note expressions
    pub fn mpe_port(&mut self, port: crate::mpe::MpePort) -> &mut Self {
        self.mpe.retain(|p| p.port != port.port);
        self.mpe.push(port);
        self
    }

    /// The shaping of a node's live MIDI input: key range, base key, velocity scale and tuning
    pub fn live_keys(&mut self, node: NodeIndex, keys: LiveKeys) -> &mut Self {
        self.live_keys.push((node, keys));
        self
    }

    /// A node's MIDI out to a device (P-38)
    pub fn midi_out(&mut self, route: MidiOutRoute) -> &mut Self {
        self.midi_out.push(route);
        self
    }

    /// A slot whose value goes to `node`'s processor as controller `number` on `channel` (the CC rack, F-IJN-025),
    /// with its automation
    pub fn cc_param(
        &mut self,
        slot: ParamSlot,
        node: NodeIndex,
        channel: u8,
        number: u8,
        initial: f32,
        automation: Option<Curve>,
    ) -> &mut Self {
        self.ccs.push((slot, node, channel & 0x0F, number & 0x7F, initial, automation));
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
            bindings.push(Binding {
                slot: slot.0,
                target,
                rate,
                discrete,
                initial,
                curve,
                mode: AutomationMode::Read,
                modulated: None,
            });
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
                chain: NONE,
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
            if pnodes[i as usize].desc.kind == NodeKind::Sum {
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
        for (slot, node, channel, number, initial, curve) in self.ccs {
            let i = find(node)?;
            if pnodes[i as usize].desc.kind != NodeKind::Processor {
                return Err(node_arg(invalid("render.program.midi_on_sum"), node));
            }
            bind(slot, Target::Cc { node: i, channel, number }, Rate::Block, false, initial, curve, &mut bindings)?;
        }
        // Modulators: their own parameters, then their targets mark the bindings they modulate
        let mut pmods: Vec<PModulator> = Vec::with_capacity(self.mods.len());
        let grid_steps = max_block / AUTOMATION_GRID as usize + 2;
        for spec in &self.mods {
            let input = match spec.input {
                Some(n) => find(n)?,
                None => NONE,
            };
            let trigger = match spec.trigger {
                Some(n) => find(n)?,
                None => NONE,
            };
            if !spec.initial.iter().all(|v| v.is_finite()) {
                return Err(invalid("render.program.initial_not_finite"));
            }
            pmods.push(PModulator {
                kind: spec.kind,
                seed: spec.seed,
                user_wave: spec.user_wave.clone(),
                params: spec.initial,
                param_bindings: [NONE; MOD_PARAMS],
                targets: Box::new([]),
                input,
                trigger,
                envelope: Envelope::new(),
                peak: PeakTracker::new(max_block),
                gate: None,
                values: vec![0.0; grid_steps].into_boxed_slice(),
            });
        }
        for (slot, m, index, info, curve) in self.mod_params {
            if m >= pmods.len() || index >= MOD_PARAMS {
                return Err(invalid("render.program.modulator_unknown").arg("slot", i64::from(slot.0)));
            }
            let bi = bind(
                slot,
                Target::Modulator { modulator: m as u32, index: index as u32 },
                info.rate,
                info.discrete,
                info.initial,
                curve,
                &mut bindings,
            )?;
            pmods[m].param_bindings[index] = bi;
            pmods[m].params[index] = info.initial;
        }
        for (m, spec) in self.mods.iter().enumerate() {
            let mut targets = Vec::with_capacity(spec.targets.len());
            for (k, t) in spec.targets.iter().enumerate() {
                if !(t.amount.is_finite() && t.min.is_finite() && t.max.is_finite()) {
                    return Err(invalid("render.program.modulator_target").arg("slot", i64::from(t.slot.0)));
                }
                let Some(&bi) = slot_map.get(t.slot.0 as usize).filter(|b| **b != NONE) else {
                    // A parameter the program does not bind (its node is not built): nothing to modulate
                    continue;
                };
                if bindings[bi as usize].modulated.is_some() {
                    // One modulator to a parameter: the first one wins
                    continue;
                }
                bindings[bi as usize].modulated = Some((m as u32, targets.len() as u32));
                if let Target::Processor { node, .. } | Target::Cc { node, .. } = bindings[bi as usize].target {
                    event_room[node as usize] += grid_steps + 2;
                }
                let _ = k;
                targets.push(PTarget {
                    slot: t.slot.0,
                    range: t.range,
                    amount: t.amount,
                    min: t.min.clamp(0.0, 1.0),
                    max: t.max.clamp(0.0, 1.0),
                    last: f32::NAN,
                    steps: vec![0.0; grid_steps].into_boxed_slice(),
                });
            }
            pmods[m].targets = targets.into_boxed_slice();
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
        let mut chains = Vec::with_capacity(self.chains.len());
        for (target, effects) in &self.chains {
            let t = find(*target)?;
            if pnodes[t as usize].desc.kind != NodeKind::Processor || pnodes[t as usize].chain != NONE {
                return Err(node_arg(invalid("render.program.chain_target"), *target));
            }
            let mut list = Vec::with_capacity(effects.len());
            for e in effects {
                let i = find(*e)?;
                if pnodes[i as usize].desc.kind != NodeKind::NoteEffect || list.contains(&i) {
                    return Err(node_arg(invalid("render.program.chain_effect"), *e));
                }
                list.push(i);
            }
            pnodes[t as usize].chain = chains.len() as u32;
            pnodes[t as usize].has_midi = true;
            chains.push(Chain {
                target: t,
                effects: list.into_boxed_slice(),
                source: EventList::with_capacity(CHAIN_EVENTS),
                merge: EventList::with_capacity(CHAIN_EVENTS + 256),
                cur: NoteOut::with_capacity(CHAIN_EVENTS),
                next: NoteOut::with_capacity(CHAIN_EVENTS),
            });
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
        let mut midi_out = Vec::with_capacity(self.midi_out.len());
        for r in &self.midi_out {
            midi_out.push((find(r.node)?, *r));
        }
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

        // The declared ranges, by node
        let mut node_limits: Vec<Vec<(f32, f32)>> = vec![Vec::new(); n];
        for &(node, index, lo, hi) in &self.param_limits {
            let limits = &mut node_limits[find(node)? as usize];
            if index < MAX_LIMITED_PARAMS {
                if limits.len() <= index as usize {
                    limits.resize(index as usize + 1, (f32::NEG_INFINITY, f32::INFINITY));
                }
                limits[index as usize] = (lo, hi);
            }
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
            let mut list = EventList::with_capacity(events);
            let limits = std::mem::take(&mut node_limits[i]);
            list.set_limits(limits.into_boxed_slice());
            p.io = Some(Box::new(NodeIo { layout, buffer: vec![0.0; layout.len()].into_boxed_slice(), events: list }));
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
            live_keys: self.live_keys.into_boxed_slice(),
            midi_mpe: midi_out
                .iter()
                .map(|(_, r)| self.mpe.iter().find(|p| p.port == r.port).map(crate::mpe::MpeOut::new))
                .collect(),
            mpe: self.mpe.into_boxed_slice(),
            midi_sent: vec![-1i8; midi_out.len() * 128].into_boxed_slice(),
            midi_out: midi_out.into_boxed_slice(),
            outputs: self.outputs.into_boxed_slice(),
            master,
            preview,
            metronome,
            midi: midi.into_boxed_slice(),
            chains: chains.into_boxed_slice(),
            streams: streams.into_boxed_slice(),
            modulators: pmods.into_boxed_slice(),
        }))
    }
}
