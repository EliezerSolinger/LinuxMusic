//! The audio side of the engine (interfaces.md, I-5 to I-9; Parte 2, rules 1, 3, 5).
//!
//! The [`Renderer`] belongs to the device callback. Each [`Renderer::render`]:
//! 1. applies every install that arrived, in order (S5);
//! 2. takes the commands (transport at once, MIDI by time stamp with one block of fixed latency, R14);
//! 3. cuts the block into passes at `max_block` (C7), the loop end, the punch edges and the tempo segments, so each
//!    of them happens at its exact sample;
//! 4. renders each pass through the graph, level by level, the levels' processors in parallel;
//! 5. publishes telemetry at ~60 Hz and counts the period.
//!
//! Every render call and every worker runs with subnormals flushed (H6, `lm-sys-fpu`), so the device and an export
//! give the same bits.
//!
//! Nothing here allocates or frees (rule 5): every buffer, list and ring was sized when the engine or the program
//! was made, and whatever leaves the audio side goes back through a ring that has room for it

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use lm_base::HostTime;
use lm_base::ring::{Producer, PushError};

use crate::command::{Command, CommandReceiver, PreviewCommand};
use crate::event::{EventKind, EventList};
use crate::exec::{Job, Watch, Workers, run_job, scan};
use crate::handoff::HandoffReceiver;
use crate::ids::{NodeIndex, NoteId, ParamSlot, PortIndex, PreviewId, ProgramGen};
use crate::install::{Install, NodeCore, Retired};
use crate::midimap::{MapEvent, MapTarget, MidiMap, MidiOutEvent, RETURN_DIV, RecordedMidi};
use crate::modulation::ModKind;
use crate::notes::{ActiveNotes, NoteSource};
use crate::panic::{RenderThreadGuard, catching};
use crate::params::ParamSlots;
use crate::processor::{Adopted, BufferLayout, Buffers, HostOutbox, NoteContext, ProcessContext, TransportInfo};
use crate::program::{
    AUTOMATION_GRID, AutomationMode, ChannelMix, DelayLine, MidiOutRoute, MidiTiming, NONE, NodeKind, PFeed,
    PHostParam, PNode, Program, Rate, Target,
};
use crate::program::{AudioItem, AudioSource};
use crate::rng::Rng;
use crate::sample::SampleData;
use crate::spec::{Limits, ProcessSpec};
use crate::telemetry::{
    METER_CHANNELS, PanicReport, PreviewEnded, PreviewReturn, Telemetry, TelemetryFrame, TelemetryKind, Touched,
    TransportEvent,
};

/// What one MIDI message may do to the map at most (several mappings on one control, a 14-bit pair, a touch)
const MAP_HITS: usize = 256;

/// Host-side smoothing of a slot written by hand, for the host's gains (samples)
const SMOOTH: u32 = 64;
/// Fade of a mute and of the master protection (samples)
const MUTE_FADE: f32 = 64.0;
const PROTECTION_FADE: usize = 32;
/// +24 dBFS (U20)
pub const PROTECTION_THRESHOLD: f32 = 15.848_932;
/// A tail node lives at most this long (U32)
pub const TAIL_SECONDS: u64 = 2;
/// Below this peak a tail or a preset preview counts as silent (−120 dBFS)
const SILENT: f32 = 1e-6;
/// A node goes to a worker only if its calls take longer than this (moving it costs a few hundred nanoseconds and,
/// for a parked worker, a wake-up); cheaper ones run on the audio thread. Which thread runs a node never changes
/// its output
const DISPATCH_MIN_NS: f32 = 3000.0;
/// The cost a node is assumed to have before it was measured (so it is tried on a worker first)
const UNMEASURED_NS: f32 = 1.0e6;

/// The device's block, as the backend gives it to [`Renderer::render`]
#[derive(Debug)]
pub struct DeviceBlock<'a, 'b> {
    pub frames: usize,
    /// The host time of the block's first sample (C6)
    pub host_time: HostTime,
    /// Captured channels (each at least `frames` long)
    pub inputs: &'a [&'b [f32]],
    /// Playback channels (each at least `frames` long); the renderer writes all of them
    pub outputs: &'a mut [&'b mut [f32]],
    /// Capture frames lost in an xrun right before this block (C2)
    pub capture_lost: u32,
    /// Playback xrun: how late this block is, in frames (0: on time)
    pub late_by: u32,
}

/// Part of the device block that was recorded
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordSpan {
    /// First frame of the span in the device block
    pub offset: usize,
    pub frames: usize,
    /// Timeline sample of the span's first frame
    pub sample: i64,
}

/// What a block did that the capture side needs at once (the recorder runs in the same callback)
#[derive(Debug)]
pub struct BlockReport {
    spans: Vec<RecordSpan>,
    /// Capture frames lost before this block, and the timeline sample where the gap goes (C2)
    pub capture_gap: Option<(i64, u32)>,
}

impl BlockReport {
    /// The recorded parts of the block, in order (punch edges and loop wraps cut them at the exact sample)
    pub fn recorded(&self) -> &[RecordSpan] {
        &self.spans
    }
}

#[derive(Debug)]
struct Slot {
    generation: u32,
    core: Option<Box<NodeCore>>,
    dead: bool,
    peak: [f32; METER_CHANNELS],
    channels: u8,
    mute_gain: f32,
    tail_since: Option<u64>,
    tail_quiet: u64,
    tail_done: bool,
    /// Moving average of the processor's call time
    cost_ns: f32,
}

impl Slot {
    fn empty() -> Self {
        Self {
            generation: 0,
            core: None,
            dead: false,
            peak: [0.0; METER_CHANNELS],
            channels: 0,
            mute_gain: 1.0,
            tail_since: None,
            tail_quiet: 0,
            tail_done: false,
            cost_ns: UNMEASURED_NS,
        }
    }

    fn reset_for(&mut self, generation: u32) {
        let core = self.core.take();
        *self = Slot::empty();
        self.core = core;
        self.generation = generation;
    }
}

#[derive(Clone, Copy, Debug)]
struct SlotState {
    current: f32,
    target: f32,
    step: f32,
    left: u32,
    emitted: f32,
    resume: bool,
    touch_overflow: bool,
    /// A MIDI touch source holds the slot (I-21.3)
    midi_hold: bool,
    /// The MIDI gesture holds the slot until this engine frame (500 ms after its last message, I-21.2)
    midi_until: u64,
    /// In the renderer's list of slots with a MIDI gesture
    midi_listed: bool,
    /// Latch mode: touched while playing, the curve stays off until the transport stops (I-21.5)
    latched: bool,
    /// Touched by a gesture at the last look (to tell a release from a write)
    was_touched: bool,
    /// Back to the curve after a release: (start frame, value it starts from), over 50 ms (I-21.5)
    ret: Option<(u64, f32)>,
}

impl SlotState {
    fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            left: 0,
            emitted: f32::NAN,
            resume: true,
            touch_overflow: false,
            midi_hold: false,
            midi_until: 0,
            midi_listed: false,
            latched: false,
            was_touched: false,
            ret: None,
        }
    }

    fn set_target(&mut self, value: f32, discrete: bool) {
        self.target = value;
        if discrete {
            self.current = value;
            self.left = 0;
        } else {
            self.left = SMOOTH;
            self.step = (value - self.current) / SMOOTH as f32;
        }
    }

    /// A hand holds the slot: a gesture (the touched bit) or MIDI
    fn hand(&self, touched: bool, frame: u64) -> bool {
        touched || self.midi_hold || frame < self.midi_until
    }

    /// A value of the curve during the return after a release (the plain curve value when not returning)
    fn returning(&mut self, v: f32, at: u64, len: u64) -> f32 {
        match self.ret {
            Some((start, from)) => {
                let elapsed = at.saturating_sub(start);
                if elapsed >= len {
                    self.ret = None;
                    v
                } else {
                    from + (v - from) * (elapsed as f32 / len as f32)
                }
            }
            None => v,
        }
    }

    fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.current = if self.left == 0 { self.target } else { self.current + self.step };
        }
        self.current
    }
}

#[derive(Clone, Copy, Debug)]
struct LiveMidi {
    offset: u32,
    /// Arrival order, to keep equal offsets stable under an unstable (allocation-free) sort
    seq: u32,
    port: PortIndex,
    bytes: [u8; 3],
}

/// By offset, arrival order kept among equal offsets (producers in index order). The unstable sort works in place:
/// the stable one allocates for long slices, and this runs in the audio block
fn sort_live(live: &mut [LiveMidi]) {
    live.sort_unstable_by_key(|m| (m.offset, m.seq));
}

#[derive(Debug, Default)]
struct Transport {
    playing: bool,
    armed: bool,
    recording: bool,
    position: i64,
    take_frames: u64,
    /// Where the playhead goes back to when a recording started from a stop ends (F-REC-020)
    return_to: Option<i64>,
    /// The pre-roll is running: recording begins when the playhead gets here (F-REC-023)
    record_from: Option<i64>,
}

#[derive(Debug, Default)]
struct Protection {
    engaged: bool,
    gain: f32,
    quiet: u64,
}

enum PreviewPlay {
    Sample { data: Arc<SampleData>, gain: f32, pos: usize },
    Preset { processor: Box<dyn Processor>, key: u8, started: bool, off: bool, released_at: Option<u64>, quiet: u64 },
}

use crate::processor::Processor;

struct PreviewSlot {
    id: PreviewId,
    play: Option<PreviewPlay>,
    stopping: Option<u32>,
    ending: Option<PreviewEnded>,
    panic: Option<Box<dyn Any + Send>>,
    buffer: Box<[f32]>,
    events: EventList,
    rng: Rng,
    outbox: HostOutbox,
    started_at: u64,
}

impl std::fmt::Debug for PreviewSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewSlot").field("id", &self.id).field("playing", &self.play.is_some()).finish()
    }
}

#[derive(Debug, Default)]
struct MetronomeState {
    click: Option<(bool, usize)>,
}

/// A count-in running (I-5.5): the transport waits at its position while the metronome counts
#[derive(Clone, Copy, Debug)]
struct Counting {
    /// The tick where the music starts
    t0: i64,
    /// Samples counted so far, and in all
    elapsed: i64,
    total: i64,
    /// Ticks per beat, beats per bar, beats in all, the next beat to click
    beat: i64,
    per_bar: u32,
    beats: u32,
    next: u32,
}

/// The rings from the audio side to the main thread
pub(crate) struct Outbound {
    pub(crate) frame: triple_buffer::Input<TelemetryFrame>,
    pub(crate) events: Producer<Telemetry>,
    pub(crate) transport: Producer<TransportEvent>,
    pub(crate) touched: Producer<Touched>,
    pub(crate) map_events: Producer<MapEvent>,
    pub(crate) recorded_midi: Producer<RecordedMidi>,
    pub(crate) midi_out: Producer<MidiOutEvent>,
    pub(crate) previews: Producer<PreviewEnded>,
    pub(crate) panics: Producer<PanicReport>,
}

/// The audio half of the engine. Owned by the device callback (rule 1); only it calls [`Renderer::render`]
pub struct Renderer {
    spec: ProcessSpec,
    limits: Limits,
    program: Box<Program>,
    slots: Box<[Slot]>,
    installs: HandoffReceiver<Install>,
    receivers: Box<[Option<CommandReceiver>]>,
    params: ParamSlots,
    slot_state: Box<[SlotState]>,
    notes: ActiveNotes,
    pending: Vec<(NodeIndex, EventKind)>,
    live: Vec<LiveMidi>,
    transport: Transport,
    out: Outbound,
    dropped: [u32; 5],
    previews: Box<[PreviewSlot]>,
    preview_mix: Box<[f32]>,
    metronome: MetronomeState,
    counting: Option<Counting>,
    /// The peak of each device input channel since the last telemetry frame
    input_peaks: Box<[f32]>,
    workers: Workers,
    gain: Box<[f32]>,
    pan: Box<[f32]>,
    curve: Box<[f32]>,
    /// A streamed item's frames for one pass (64 channels at most)
    stream_scratch: Box<[f32]>,
    protection: Protection,
    watch: Arc<Watch>,
    frame: u64,
    since_publish: u64,
    load: f32,
    xruns: u64,
    jump: bool,
    report: BlockReport,
    /// The MIDI map (I-21.2) and what one message did to it
    map: Box<MidiMap>,
    map_hits: Vec<(lm_midimap::Event, MapTarget)>,
    /// Slots with a MIDI gesture running
    midi_active: Vec<u32>,
    /// Live MIDI stamped after this block, kept for the next one (so a later stamp never holds back the ring)
    held: Vec<(PortIndex, HostTime, [u8; 3])>,
    map_lost: u32,
    midi_record_stopped: bool,
    /// The block's host time and the current pass's offset (MIDI out leaves at its exact time, P-38)
    block_host_time: HostTime,
    pass_off: u32,
    midi_out_lost: u32,
    /// The offset in this pass of the first modulation grid step after the first (host parameters read it)
    mod_first_grid: usize,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("generation", &self.program.generation)
            .field("frame", &self.frame)
            .field("workers", &self.workers.count())
            .finish()
    }
}

/// The spans a block can hold: the loop is at least 50 ms long (program rule), so a block wraps at most
/// `max_block / min_loop + 1` times, each wrap cutting at most three spans (wrap, punch in, punch out)
pub(crate) fn span_capacity(spec: &ProcessSpec) -> usize {
    let min_loop = (spec.sample_rate as usize / 20).max(1);
    4 * (spec.max_block / min_loop + 2) + 8
}

impl Renderer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        spec: ProcessSpec,
        limits: Limits,
        installs: HandoffReceiver<Install>,
        main: CommandReceiver,
        params: ParamSlots,
        out: Outbound,
        workers: Workers,
        watch: Arc<Watch>,
    ) -> Self {
        let mut receivers: Vec<Option<CommandReceiver>> = (0..limits.max_command_producers).map(|_| None).collect();
        receivers[0] = Some(main);
        let previews = (0..limits.max_previews)
            .map(|i| PreviewSlot {
                id: PreviewId(0),
                play: None,
                stopping: None,
                ending: None,
                panic: None,
                buffer: vec![0.0; 2 * spec.max_block].into_boxed_slice(),
                events: EventList::with_capacity(16),
                rng: Rng::new(i as u64),
                outbox: HostOutbox::default(),
                started_at: 0,
            })
            .collect();
        Self {
            spec,
            limits,
            program: Program::empty(spec, &limits),
            slots: (0..limits.max_nodes).map(|_| Slot::empty()).collect(),
            installs,
            receivers: receivers.into_boxed_slice(),
            params,
            slot_state: vec![SlotState::new(0.0); limits.max_params].into_boxed_slice(),
            notes: ActiveNotes::new(limits.max_active_notes),
            pending: Vec::with_capacity(limits.max_active_notes + 2 * limits.max_nodes),
            live: Vec::with_capacity(limits.max_live_midi_per_block),
            transport: Transport::default(),
            out,
            dropped: [0; 5],
            previews,
            preview_mix: vec![0.0; 2 * spec.max_block].into_boxed_slice(),
            metronome: MetronomeState::default(),
            counting: None,
            input_peaks: vec![0.0; crate::telemetry::INPUT_METERS].into_boxed_slice(),
            workers,
            gain: vec![0.0; spec.max_block].into_boxed_slice(),
            pan: vec![0.0; spec.max_block].into_boxed_slice(),
            curve: vec![0.0; spec.max_block].into_boxed_slice(),
            stream_scratch: vec![0.0; 64 * spec.max_block].into_boxed_slice(),
            protection: Protection { engaged: false, gain: 1.0, quiet: 0 },
            watch,
            frame: 0,
            since_publish: 0,
            load: 0.0,
            xruns: 0,
            jump: true,
            report: BlockReport { spans: Vec::with_capacity(span_capacity(&spec)), capture_gap: None },
            map: MidiMap::empty(),
            map_hits: Vec::with_capacity(MAP_HITS),
            midi_active: Vec::with_capacity(limits.max_params),
            held: Vec::with_capacity(limits.max_live_midi_per_block),
            map_lost: 0,
            midi_record_stopped: false,
            block_host_time: HostTime(0),
            pass_off: 0,
            midi_out_lost: 0,
            mod_first_grid: 0,
        }
    }

    pub fn spec(&self) -> ProcessSpec {
        self.spec
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Notes sounding now, from ports and items (the active-notes table, U25)
    pub fn active_notes(&self) -> usize {
        self.notes.len()
    }

    /// The generation of the installed program
    pub fn generation(&self) -> ProgramGen {
        self.program.generation
    }

    /// The playhead
    pub fn position(&self) -> i64 {
        self.transport.position
    }

    pub fn is_playing(&self) -> bool {
        self.transport.playing
    }

    /// Renders one device block (any size; larger than `max_block` is split, C7)
    pub fn render(&mut self, block: DeviceBlock<'_, '_>) -> &BlockReport {
        let _render = RenderThreadGuard::enter();
        // H6: subnormals flushed for the whole call (a few register instructions; restored when the call returns,
        // so a thread lent to rendering gets its state back)
        let _flush = lm_sys_fpu::FlushDenormals::enable();
        let started = HostTime::now();
        let DeviceBlock { frames, host_time, inputs, outputs, capture_lost, late_by } = block;
        self.block_host_time = host_time;
        for out in outputs.iter_mut() {
            let n = frames.min(out.len());
            out[..n].fill(0.0);
        }
        self.report.spans.clear();
        self.report.capture_gap = None;
        if late_by > 0 {
            self.xruns += 1;
            self.telemetry(Telemetry::Xrun { late_by });
        }

        while let Some(install) = self.installs.receive() {
            self.apply(install);
        }
        self.take_commands(host_time, frames);
        for (peak, ch) in self.input_peaks.iter_mut().zip(inputs.iter()) {
            let n = frames.min(ch.len());
            *peak = ch[..n].iter().fold(*peak, |a, x| if x.is_finite() { a.max(x.abs()) } else { a });
        }

        if capture_lost > 0 {
            let at = self.transport.position;
            self.report.capture_gap = Some((at, capture_lost));
            if self.transport.recording {
                self.transport_event(TransportEvent::CaptureGap { sample: at, frames: capture_lost });
            }
            if self.transport.playing {
                // The device says that much time passed: the playhead keeps with it, so the take stays on the grid
                self.transport.position += i64::from(capture_lost);
                self.release_item_notes();
                self.jump = true;
            }
        }

        let mut off = 0usize;
        let mut first = true;
        let mut live_next = 0usize;
        while off < frames {
            let n = self.next_pass_len(frames - off);
            self.update_recording();
            self.pass(off, n, first, inputs, &mut live_next);
            if self.transport.recording && self.report.spans.len() < self.report.spans.capacity() {
                let sample = self.transport.position;
                match self.report.spans.last_mut() {
                    Some(s) if s.offset + s.frames == off && s.sample + s.frames as i64 == sample => s.frames += n,
                    _ => self.report.spans.push(RecordSpan { offset: off, frames: n, sample }),
                }
                self.transport.take_frames += n as u64;
            }
            self.write_outputs(outputs, off, n);
            first = false;
            off += n;
            self.frame += n as u64;
            if self.transport.playing {
                self.transport.position += n as i64;
                if let Some((a, b)) = self.program.transport.loop_range
                    && self.transport.position == b
                {
                    if self.transport.recording {
                        let take = self.transport.take_frames;
                        self.transport_event(TransportEvent::LoopWrapped { from: b, to: a, sample_in_take: take });
                    }
                    self.transport.position = a;
                    self.release_item_notes();
                    self.jump = true;
                }
            }
            if let Some(c) = self.counting.as_mut() {
                c.elapsed += n as i64;
                if c.elapsed >= c.total {
                    // The count-in is over: the music starts at the next sample, where it was asked to
                    self.counting = None;
                    self.transport.playing = true;
                    self.jump = true;
                }
            }
        }
        // MIDI not consumed (its time is ahead) stays in its ring; what was taken is all played
        self.live.clear();
        self.publish_streams();

        self.watch.periods.fetch_add(1, Ordering::Relaxed);
        let elapsed = HostTime::now().since(started) as f64;
        let budget = frames.max(1) as f64 * 1e9 / f64::from(self.spec.sample_rate);
        self.load = self.load.max((elapsed / budget) as f32);
        self.since_publish += frames as u64;
        if self.since_publish * 60 >= u64::from(self.spec.sample_rate) {
            self.publish(host_time);
        }
        self.flush_dropped();
        &self.report
    }

    // ------------------------------------------------------------------------------------------------------------
    // Installs

    fn apply(&mut self, mut install: Box<Install>) {
        let mut skipped = 0u32;
        let max = self.slots.len();
        // 1. add
        for (node, core) in install.add.drain(..) {
            if node.slot() >= max {
                skipped += 1;
                install.returned.processors.push(Retired { node, core });
                continue;
            }
            let slot = &mut self.slots[node.slot()];
            if let Some(old) = slot.core.take() {
                let old_node = NodeIndex::new(node.index, slot.generation);
                install.returned.processors.push(Retired { node: old_node, core: old });
            }
            slot.reset_for(node.generation);
            slot.core = Some(core);
            self.notes.take_where(|a| a.node.index == node.index, |_| {});
        }
        // 2. replace
        for r in install.replace.drain(..) {
            let crate::install::Replacement { node, mut core, tail } = r;
            if node.slot() >= max || tail.is_some_and(|t| t.slot() >= max || t.slot() == node.slot()) {
                skipped += 1;
                install.returned.processors.push(Retired { node, core });
                continue;
            }
            let (old, was_dead, old_generation) = {
                let slot = &mut self.slots[node.slot()];
                (slot.core.take(), slot.dead, slot.generation)
            };
            let mut adopted = Adopted::No;
            let mut adopt_panic = None;
            let mut old = old;
            if let Some(o) = old.as_mut()
                && !was_dead
            {
                let new = &mut core.processor;
                match catching(AssertUnwindSafe(|| new.adopt(o.processor.as_mut()))) {
                    Ok(a) => adopted = a,
                    Err(p) => adopt_panic = Some(p),
                }
            }
            {
                let slot = &mut self.slots[node.slot()];
                slot.reset_for(node.generation);
                slot.core = Some(core);
                slot.dead = adopt_panic.is_some();
            }
            if let Some(payload) = adopt_panic {
                self.report_panic(Some(node), None, payload);
            }
            let old_node = NodeIndex::new(node.index, old_generation);
            match (old, adopted, tail) {
                (Some(o), Adopted::No, Some(t)) if !was_dead => {
                    let tslot = &mut self.slots[t.slot()];
                    if let Some(prev) = tslot.core.take() {
                        install
                            .returned
                            .processors
                            .push(Retired { node: NodeIndex::new(t.index, tslot.generation), core: prev });
                    }
                    tslot.reset_for(t.generation);
                    tslot.core = Some(o);
                    tslot.tail_since = Some(self.frame);
                    // Its notes are all released now; the new processor never got them
                    self.notes.take_where(|a| a.node.index == node.index, |_| {});
                    self.pending.push((t, EventKind::AllNotesRelease));
                }
                (Some(o), Adopted::Yes, _) => {
                    // The notes carry on in the new processor: their entries stay as they are
                    install.returned.processors.push(Retired { node: old_node, core: o });
                }
                (Some(o), _, _) => {
                    install.returned.processors.push(Retired { node: old_node, core: o });
                    self.notes.take_where(|a| a.node.index == node.index, |_| {});
                }
                (None, _, _) => {
                    self.notes.take_where(|a| a.node.index == node.index, |_| {});
                }
            }
        }
        // 3. remove
        for node in install.remove.drain(..) {
            if node.slot() >= max {
                skipped += 1;
                continue;
            }
            let slot = &mut self.slots[node.slot()];
            if let Some(core) = slot.core.take() {
                install.returned.processors.push(Retired { node, core });
            }
            *slot = Slot::empty();
            install.returned.freed.push(node);
            // A node that leaves takes its notes with it: there is nothing left to end them in
            self.notes.take_where(|a| a.node.index == node.index, |_| {});
        }
        // 4. command rings
        for p in install.remove_producers.drain(..) {
            match self.receivers.get_mut(usize::from(p.0)) {
                Some(r) if p.0 != 0 => {
                    if let Some(rx) = r.take() {
                        install.returned.receivers.push(rx);
                    }
                }
                _ => skipped += 1,
            }
        }
        for rx in install.producers.drain(..) {
            let p = usize::from(rx.producer().0);
            match self.receivers.get_mut(p) {
                Some(r) if p != 0 => {
                    if let Some(old) = r.replace(rx) {
                        install.returned.receivers.push(old);
                    }
                }
                _ => {
                    skipped += 1;
                    install.returned.receivers.push(rx);
                }
            }
        }
        // 5. the program
        if let Some(mut program) = install.program.take() {
            if program.node_map.len() != self.slots.len()
                || program.slot_map.len() != self.slot_state.len()
                || program.spec != self.spec
            {
                // Built for other limits: refused whole (the main side checks this before sending)
                skipped += 1;
                install.returned.program = Some(program);
            } else {
                self.swap_program(&mut program, install.relocate.is_some());
                install.returned.program = Some(program);
            }
        }
        // 6. the MIDI map: its running state carries over (pickup, toggles, 14-bit halves)
        if let Some(mut map) = install.map.take() {
            map.table.inherit(&self.map.table);
            std::mem::swap(&mut self.map, &mut map);
            install.returned.map = Some(map);
        }
        install.returned.skipped = skipped;
        if skipped > 0 {
            self.telemetry(Telemetry::InstallInvalid { generation: install.generation });
        }
        self.installs.give_back(install);
    }

    /// Swaps `incoming` in; on return `incoming` holds the old program
    fn swap_program(&mut self, incoming: &mut Box<Program>, relocate: bool) {
        // Delay lines keep their history across programs (sorted by key in both)
        {
            let old = &self.program.delays;
            for d in incoming.delays.iter_mut() {
                let i = old.partition_point(|o| o.key < d.key);
                if let Some(o) = old.get(i)
                    && o.key == d.key
                {
                    d.inherit(o);
                }
            }
        }
        // New slots start from the slot's value (the main thread wrote the initial one, S20)
        for b in incoming.bindings.iter() {
            let s = b.slot as usize;
            if self.program.slot_map[s] == NONE {
                self.slot_state[s] = SlotState::new(self.params.value_at(s));
            } else {
                self.slot_state[s].resume = true;
            }
        }
        std::mem::swap(&mut self.program, incoming);
        let old = &**incoming;

        if relocate && self.program.tempo != old.tempo {
            let from = self.transport.position;
            let to = lm_time::relocate(&old.tempo, &self.program.tempo, from);
            if to != from {
                self.transport.position = to;
                self.transport_event(TransportEvent::Relocated { from, to });
            }
        }
        self.jump = true;

        // U25: notes whose node or item note is gone (or now ended) get their note-off now
        let tick = self.program.tempo.tick_at(self.transport.position);
        let program = &self.program;
        let pending = &mut self.pending;
        let slots = &self.slots;
        self.notes.take_where(
            |a| {
                let Some(ni) = program.node_of(a.node) else { return true };
                match a.source {
                    NoteSource::Port(p) => !program.ports.iter().any(|r| r.port == p && r.node == a.node),
                    NoteSource::Item { item, n } => !program.midi.iter().any(|src| {
                        src.node == ni
                            && src.items.iter().any(|it| it.key() == item && it.note_end(n).is_some_and(|e| e > tick))
                    }),
                }
            },
            |a| {
                if slots[a.node.slot()].core.is_some() && pending.len() < pending.capacity() {
                    pending.push((
                        a.node,
                        EventKind::NoteOff { note: a.note, channel: a.channel, key: a.key, velocity: 0.0 },
                    ));
                }
            },
        );
        // A chain that changed (an effect added, removed, bypassed or moved) ends what the old one had started: the
        // instrument releases its notes, through the new chain
        // (compared as iterators: this runs in the audio block, nothing is collected)
        fn effects_of(p: &Program, ci: usize) -> impl Iterator<Item = NodeIndex> + '_ {
            p.chains[ci].effects.iter().map(move |&e| p.nodes[e as usize].desc.node)
        }
        let chain_of = |p: &Program, target: NodeIndex| -> Option<usize> {
            p.chains.iter().position(|c| p.nodes[c.target as usize].desc.node == target)
        };
        for ci in 0..program.chains.len() {
            let target = program.nodes[program.chains[ci].target as usize].desc.node;
            let changed = match chain_of(old, target) {
                Some(oi) => !effects_of(old, oi).eq(effects_of(program, ci)),
                None => old.node_of(target).is_some(),
            };
            if changed && pending.len() < pending.capacity() {
                pending.push((target, EventKind::AllNotesRelease));
            }
        }
        for oi in 0..old.chains.len() {
            let target = old.nodes[old.chains[oi].target as usize].desc.node;
            if chain_of(program, target).is_none()
                && program.node_of(target).is_some()
                && pending.len() < pending.capacity()
            {
                pending.push((target, EventKind::AllNotesRelease));
            }
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // Commands

    fn take_commands(&mut self, host_time: HostTime, frames: usize) {
        let rate = self.spec.sample_rate;
        let latency = match self.program.transport.midi {
            MidiTiming::FixedBlock => self.spec.max_block as i64,
            MidiTiming::Immediate => 0,
        };
        let rate_i = i128::from(rate);
        let offset_of = |at: HostTime, timing: MidiTiming| -> i64 {
            match timing {
                MidiTiming::Immediate => 0,
                MidiTiming::FixedBlock => {
                    let dt = i128::from(at.0) - i128::from(host_time.0);
                    let samples = (dt * rate_i).div_euclid(1_000_000_000) as i64;
                    (samples + latency).max(0)
                }
            }
        };
        let timing = self.program.transport.midi;
        // What waited from the last block first (in its order)
        let mut keep = 0;
        for k in 0..self.held.len() {
            let (port, at, bytes) = self.held[k];
            let offset = offset_of(at, timing);
            if offset < frames as i64 && self.live.len() < self.live.capacity() {
                self.live.push(LiveMidi { offset: offset as u32, seq: self.live.len() as u32, port, bytes });
            } else {
                if offset < frames as i64 {
                    self.dropped[3] = self.dropped[3].saturating_add(1);
                }
                self.held[keep] = self.held[k];
                keep += 1;
            }
        }
        self.held.truncate(keep);
        enum Next {
            Midi(i64),
            Preview,
            Other,
        }
        for p in 0..self.receivers.len() {
            loop {
                let next = {
                    let Some(rx) = self.receivers[p].as_mut() else { break };
                    let Ok(head) = rx.rx.peek() else { break };
                    match head {
                        Command::Midi { at, .. } => Next::Midi(offset_of(*at, timing)),
                        Command::Preview(PreviewCommand::Sample { .. } | PreviewCommand::Preset { .. }) => {
                            Next::Preview
                        }
                        _ => Next::Other,
                    }
                };
                match next {
                    Next::Midi(offset) => {
                        if offset >= frames as i64 {
                            // Ahead of this block: kept for the next one, so what is behind it in the ring (another
                            // port with another latency) is not held back
                            if self.held.len() == self.held.capacity() {
                                break; // it waits in the ring
                            }
                            let Some(rx) = self.receivers[p].as_mut() else { break };
                            let Ok(Command::Midi { port, at, bytes }) = rx.rx.pop() else { break };
                            self.held.push((port, at, bytes));
                            continue;
                        }
                        if self.live.len() == self.live.capacity() {
                            self.dropped[3] = self.dropped[3].saturating_add(1);
                            break; // the rest waits for the next block
                        }
                        let Some(rx) = self.receivers[p].as_mut() else { break };
                        let Ok(Command::Midi { port, bytes, .. }) = rx.rx.pop() else { break };
                        self.live.push(LiveMidi { offset: offset as u32, seq: self.live.len() as u32, port, bytes });
                    }
                    Next::Preview => {
                        let Some(i) = self.free_preview() else { break };
                        let Some(rx) = self.receivers[p].as_mut() else { break };
                        let Ok(Command::Preview(cmd)) = rx.rx.pop() else { break };
                        self.start_preview(i, cmd);
                    }
                    Next::Other => {
                        let Some(rx) = self.receivers[p].as_mut() else { break };
                        let Ok(cmd) = rx.rx.pop() else { break };
                        self.command(cmd);
                    }
                }
            }
        }
        sort_live(&mut self.live);
    }

    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Play => {
                if !self.transport.playing && self.counting.is_none() {
                    self.transport.return_to = None;
                    self.transport.record_from = None;
                    if self.transport.armed {
                        // Recording from a stop: the playhead goes back where it started when it ends, and the
                        // pre-roll begins that many bars earlier (F-REC-020, F-REC-023)
                        self.transport.return_to = Some(self.transport.position);
                        if let Some(back) = self.pre_roll_start() {
                            self.transport.record_from = Some(self.transport.position);
                            self.transport.position = back;
                        }
                    }
                    match self.count_in() {
                        Some(c) => self.counting = Some(c),
                        None => {
                            self.transport.playing = true;
                            self.jump = true;
                        }
                    }
                }
            }
            Command::Stop => {
                self.counting = None;
                self.transport.record_from = None;
                if let Some(back) = self.transport.return_to.take()
                    && self.transport.armed
                {
                    self.transport.position = back;
                    self.release_item_notes();
                    self.jump = true;
                }
                self.unlatch();
                if self.transport.playing {
                    self.transport.playing = false;
                    self.release_item_notes();
                    self.jump = true;
                }
            }
            Command::Locate { sample } => {
                if self.transport.recording {
                    self.telemetry(Telemetry::LocateRefused);
                } else {
                    self.transport.position = sample;
                    self.release_item_notes();
                    self.unlatch();
                    self.jump = true;
                }
            }
            Command::Record { on } => self.transport.armed = on,
            Command::Panic => self.panic_all(),
            Command::Midi { .. } => {}
            Command::Preview(PreviewCommand::NoteOff { id }) => {
                for s in self.previews.iter_mut() {
                    if s.id == id
                        && let Some(PreviewPlay::Preset { off, .. }) = s.play.as_mut()
                    {
                        *off = true;
                    }
                }
            }
            Command::Preview(PreviewCommand::Stop { id }) => {
                for s in self.previews.iter_mut() {
                    if s.id == id && s.play.is_some() && s.stopping.is_none() {
                        s.stopping = Some(MUTE_FADE as u32);
                    }
                }
            }
            Command::Preview(_) => {}
        }
    }

    /// Tells every streamed item where the playhead will need it next, and where the loop comes back (P-32)
    fn publish_streams(&self) {
        let pos = self.transport.position;
        let lp = self.program.transport.loop_range;
        for &(ni, ii) in self.program.streams.iter() {
            let it = &self.program.nodes[ni as usize].audio[ii as usize];
            let AudioSource::Stream(s) = &it.source else { continue };
            let end = it.start + it.length;
            let map = |t: i64| u64::try_from(it.offset + (t.max(it.start) - it.start)).ok();
            s.set_playhead(if pos < end { map(pos) } else { None });
            s.set_cue(match lp {
                Some((a, b)) if a < end && b > it.start => map(a),
                _ => None,
            });
        }
    }

    /// Where the pre-roll starts: the program's bars before the playhead, by the tempo map (never before 0); `None`
    /// when there is no pre-roll or no room for one
    fn pre_roll_start(&self) -> Option<i64> {
        let bars = self.program.transport.pre_roll_bars;
        if bars == 0 {
            return None;
        }
        let tempo = &self.program.tempo;
        let t0 = tempo.tick_at(self.transport.position);
        let sig = tempo.time_signature_at(t0);
        let beat = sig.beat_ticks(tempo.ppq())?.max(1);
        let per_bar = i64::from(sig.numerator()).max(1);
        let back = (t0 - i64::from(bars) * per_bar * beat).max(0);
        let start = tempo.sample_at(back);
        (start < self.transport.position).then_some(start)
    }

    /// The count-in the program asks for at the playhead, if any
    fn count_in(&self) -> Option<Counting> {
        let ci = self.program.transport.count_in?;
        if ci.bars == 0 || !(ci.on_play || self.transport.armed) {
            return None;
        }
        let tempo = &self.program.tempo;
        let t0 = tempo.tick_at(self.transport.position);
        let sig = tempo.time_signature_at(t0);
        let beat = sig.beat_ticks(tempo.ppq())?.max(1);
        let per_bar = u32::from(sig.numerator()).max(1);
        let beats = ci.bars.saturating_mul(per_bar);
        let base = tempo.sample_at(t0);
        let total = tempo.sample_at(t0.saturating_add(i64::from(beats) * beat)) - base;
        (total > 0).then_some(Counting { t0, elapsed: 0, total, beat, per_bar, beats, next: 0 })
    }

    fn panic_all(&mut self) {
        let pending = &mut self.pending;
        self.notes.take_where(
            |_| true,
            |a| {
                if pending.len() < pending.capacity() {
                    pending.push((
                        a.node,
                        EventKind::NoteOff { note: a.note, channel: a.channel, key: a.key, velocity: 0.0 },
                    ));
                }
            },
        );
        for i in 0..self.program.nodes.len() {
            let node = self.program.nodes[i].desc.node;
            if self.program.nodes[i].desc.kind == NodeKind::Processor && self.pending.len() < self.pending.capacity() {
                self.pending.push((node, EventKind::AllSoundOff));
            }
        }
        for i in 0..self.slots.len() {
            let dead = self.slots[i].dead;
            if let Some(core) = self.slots[i].core.as_mut()
                && !dead
            {
                let p = &mut core.processor;
                if let Err(payload) = catching(AssertUnwindSafe(|| p.reset())) {
                    self.slots[i].dead = true;
                    let node = NodeIndex::new(i as u32, self.slots[i].generation);
                    self.report_panic(Some(node), None, payload);
                }
            }
        }
        for d in self.program.delays.iter_mut() {
            d.memory.fill(0.0);
        }
        for s in self.previews.iter_mut() {
            if s.play.is_some() && s.stopping.is_none() {
                s.stopping = Some(MUTE_FADE as u32);
            }
        }
        self.metronome.click = None;
    }

    fn release_item_notes(&mut self) {
        let pending = &mut self.pending;
        self.notes.take_where(
            |a| matches!(a.source, NoteSource::Item { .. }),
            |a| {
                if pending.len() < pending.capacity() {
                    pending.push((
                        a.node,
                        EventKind::NoteOff { note: a.note, channel: a.channel, key: a.key, velocity: 0.0 },
                    ));
                }
            },
        );
    }

    // ------------------------------------------------------------------------------------------------------------
    // Transport

    fn next_pass_len(&self, left: usize) -> usize {
        let mut n = left.min(self.spec.max_block) as i64;
        if let Some(c) = &self.counting {
            n = n.min(c.total - c.elapsed);
        }
        if self.transport.playing {
            let pos = self.transport.position;
            if let Some((_, b)) = self.program.transport.loop_range
                && pos < b
            {
                n = n.min(b - pos);
            }
            if let Some(from) = self.transport.record_from
                && pos < from
            {
                n = n.min(from - pos);
            }
            if self.transport.armed
                && let Some((a, b)) = self.program.transport.punch
            {
                if pos < a {
                    n = n.min(a - pos);
                } else if pos < b {
                    n = n.min(b - pos);
                }
            }
            let seg_end = self.program.tempo.segment_at_sample(pos).end_sample();
            if seg_end > pos {
                n = n.min(seg_end - pos);
            }
        }
        n.max(1) as usize
    }

    fn update_recording(&mut self) {
        let t = &self.transport;
        let inside = match self.program.transport.punch {
            Some((a, b)) => (a..b).contains(&t.position),
            None => true,
        };
        let reached = t.record_from.is_none_or(|from| t.position >= from);
        let should = t.playing && t.armed && inside && reached;
        if reached && t.playing {
            self.transport.record_from = None;
        }
        if should != self.transport.recording {
            let t = &self.transport;
            let (sample, frame) = (t.position, self.frame);
            self.transport.recording = should;
            if should {
                self.transport.take_frames = 0;
                self.midi_record_stopped = false;
                self.transport_event(TransportEvent::RecordStarted { sample, frame });
            } else {
                self.transport_event(TransportEvent::RecordStopped { sample, frame });
            }
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // A pass through the graph

    fn transport_info(&self) -> TransportInfo {
        let tempo = &self.program.tempo;
        let pos = self.transport.position;
        let tick = tempo.tick_at(pos);
        TransportInfo {
            sample_rate: self.spec.sample_rate,
            playing: self.transport.playing,
            recording: self.transport.recording,
            position: pos,
            tick,
            ppq: tempo.ppq().get(),
            bpm: tempo.bpm_at_sample(pos),
            time_signature: tempo.time_signature_at(tick),
            frame: self.frame,
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // MIDI mapping (I-21.2), touch and release (I-21.3, I-21.5), recording (I-21.4)

    /// A touched slot's value for automation recording (lossless; when full, that slot's recording stops, C3)
    fn touched_value(&mut self, s: usize, value: f32, sample: i64, touching: bool) {
        if self.slot_state[s].touch_overflow {
            return;
        }
        let t = Touched { slot: ParamSlot(s as u32), value, sample, touching };
        if self.out.touched.push(t).is_err() {
            self.slot_state[s].touch_overflow = true;
            self.telemetry(Telemetry::TouchedOverflow { slot: ParamSlot(s as u32) });
        }
    }

    fn map_event(&mut self, e: MapEvent) {
        if self.out.map_events.push(e).is_err() {
            self.map_lost = self.map_lost.saturating_add(1);
        }
    }

    /// The transport stopped or jumped: Latch lets go of every slot
    fn unlatch(&mut self) {
        for st in self.slot_state.iter_mut() {
            st.latched = false;
            st.ret = None;
        }
    }

    /// One live message through the MIDI map, at `offset` in the current pass. Returns whether a mapping consumed it
    fn map_message(&mut self, m: LiveMidi, offset: u32) -> bool {
        if self.map.table.is_empty() {
            return false;
        }
        let len = usize::from(lm_midimap::message::channel_message_len(m.bytes[0]));
        if len == 0 {
            return false;
        }
        let now = self.frame + u64::from(offset);
        let mut hits = std::mem::take(&mut self.map_hits);
        hits.clear();
        let slot_state = &self.slot_state;
        let consumed = self.map.table.feed(
            m.port.0,
            &m.bytes[..len],
            now,
            |t| match t {
                MapTarget::Param { slot, range } => {
                    slot_state.get(slot.0 as usize).map_or(0.0, |st| range.normalize(st.current))
                }
                MapTarget::Session => 0.0,
            },
            |ev, t| {
                if hits.len() < hits.capacity() {
                    hits.push((ev, *t));
                }
            },
        );
        for &(ev, target) in hits.iter() {
            match ev {
                lm_midimap::Event::Value { entry, out } => {
                    let mapping = self.map.table.entries()[entry].id;
                    match target {
                        MapTarget::Param { slot, range } => {
                            self.map_param(mapping, m.port, slot, range, out.value, offset, now)
                        }
                        MapTarget::Session => self.map_event(MapEvent::Session {
                            mapping,
                            port: m.port,
                            value: out.value,
                            raw: out.raw,
                            press: out.press,
                            frame: now,
                        }),
                    }
                }
                lm_midimap::Event::Touch { entry, on } => {
                    let mapping = self.map.table.entries()[entry].id;
                    let slot = match target {
                        MapTarget::Param { slot, .. } if (slot.0 as usize) < self.slot_state.len() => {
                            let s = slot.0 as usize;
                            let st = &mut self.slot_state[s];
                            st.midi_hold = on;
                            if !on {
                                st.midi_until = now; // released: the gesture ends at the next look
                            }
                            if on && !st.midi_listed && self.midi_active.len() < self.midi_active.capacity() {
                                st.midi_listed = true;
                                self.midi_active.push(s as u32);
                            }
                            Some(slot)
                        }
                        _ => None,
                    };
                    self.map_event(MapEvent::Touch { mapping, port: m.port, slot, on, frame: now });
                }
            }
        }
        self.map_hits = hits;
        consumed
    }

    /// A mapped parameter written at `offset`: the event at that sample, the slot's value, the gesture
    #[allow(clippy::too_many_arguments)]
    fn map_param(
        &mut self,
        mapping: u64,
        port: PortIndex,
        slot: ParamSlot,
        range: crate::midimap::ParamRange,
        normalized: f32,
        offset: u32,
        now: u64,
    ) {
        let s = slot.0 as usize;
        if s >= self.slot_state.len() {
            return;
        }
        let value = range.to_units(normalized);
        self.params.store_quiet(slot, value);
        let gesture = u64::from(self.spec.sample_rate) / crate::midimap::GESTURE_SECONDS_DIV;
        {
            let st = &mut self.slot_state[s];
            st.midi_until = st.midi_until.max(now + gesture);
            st.ret = None;
            if !st.midi_listed && self.midi_active.len() < self.midi_active.capacity() {
                st.midi_listed = true;
                self.midi_active.push(s as u32);
            }
        }
        let bi = self.program.slot_map[s];
        if bi == NONE {
            self.slot_state[s].current = value;
        } else {
            let (target, discrete) = {
                let b = &self.program.bindings[bi as usize];
                (b.target, b.discrete)
            };
            match target {
                Target::Processor { node, index } => {
                    self.slot_state[s].emitted = value;
                    self.slot_state[s].current = value;
                    self.push_event(node, offset, EventKind::Param { index, value });
                }
                Target::Host(_) => self.slot_state[s].set_target(value, discrete),
                Target::Cc { node, channel, number } => self.cc_value(s, node, offset, channel, number, value),
                Target::Modulator { modulator, index } => {
                    self.program.modulators[modulator as usize].params[index as usize] = value;
                    self.slot_state[s].current = value;
                }
            }
        }
        let sample = self.transport.position + i64::from(offset);
        if self.transport.playing {
            self.touched_value(s, value, sample, true);
        }
        self.map_event(MapEvent::Param { mapping, port, slot, value, normalized, sample, frame: now });
    }

    /// Ends the MIDI gestures whose time is up (500 ms after the last message, or the touch source released):
    /// the curve comes back in 50 ms, the session writes the value into the document
    fn expire_midi(&mut self) {
        let now = self.frame;
        let playing = self.transport.playing;
        let position = self.transport.position;
        let mut k = 0;
        while k < self.midi_active.len() {
            let s = self.midi_active[k] as usize;
            let st = &mut self.slot_state[s];
            if st.midi_hold || now < st.midi_until {
                k += 1;
                continue;
            }
            st.midi_listed = false;
            self.midi_active.swap_remove(k);
            let value = self.params.value_at(s);
            let bi = self.program.slot_map[s];
            let automated_continuous = bi != NONE
                && self.program.bindings[bi as usize].curve.is_some()
                && !self.program.bindings[bi as usize].discrete;
            if !self.params.touched_at(s) {
                let st = &mut self.slot_state[s];
                st.resume = true;
                if automated_continuous {
                    st.ret = Some((now, st.current));
                }
                if playing {
                    self.touched_value(s, value, position, false);
                }
            }
            self.map_event(MapEvent::Released { slot: ParamSlot(s as u32), value, frame: now });
        }
    }

    /// A live message that reached the instruments while recording (I-21.4)
    fn record_midi(&mut self, m: LiveMidi, offset: u32) {
        if self.midi_record_stopped {
            return;
        }
        let len = lm_midimap::message::channel_message_len(m.bytes[0]);
        if len == 0 {
            return;
        }
        let r = RecordedMidi {
            port: m.port,
            bytes: m.bytes,
            len,
            sample: self.transport.position + i64::from(offset),
            frame: self.frame + u64::from(offset),
        };
        if self.out.recorded_midi.push(r).is_err() {
            self.midi_record_stopped = true;
            self.telemetry(Telemetry::MidiRecordOverflow);
        }
    }

    fn push_event(&mut self, node: u32, offset: u32, kind: EventKind) {
        if let EventKind::NoteOn { .. } | EventKind::NoteOff { .. } = kind {
            let on = matches!(kind, EventKind::NoteOn { .. });
            for m in self.program.modulators.iter_mut() {
                if m.kind == ModKind::Envelope && m.trigger == node && m.gate.is_none() {
                    m.gate = Some((offset, on));
                }
            }
        }
        // The note stream of a node with note effects goes through them first (`run_chains`)
        let chain = self.program.nodes[node as usize].chain;
        if chain != NONE && kind.is_note_stream() {
            if !self.program.chains[chain as usize].source.push(offset, kind) {
                self.dropped[1] = self.dropped[1].saturating_add(1);
            }
            return;
        }
        self.deliver(node, offset, kind);
    }

    /// An event for a node's processor, and out to its devices
    fn deliver(&mut self, node: u32, offset: u32, kind: EventKind) {
        if let Some(io) = self.program.nodes[node as usize].io.as_mut()
            && !io.events.push(offset, kind)
        {
            self.dropped[1] = self.dropped[1].saturating_add(1);
        }
        if !self.program.midi_out.is_empty() {
            self.send_out(node, offset, kind);
        }
    }

    /// What a node plays goes out to its devices too (P-38): notes on the route's channel with its fixed velocity
    /// and fixed note, controllers and other channel messages, stamped with their exact time
    fn send_out(&mut self, node: u32, offset: u32, kind: EventKind) {
        let rate = u64::from(self.spec.sample_rate.max(1));
        let at = HostTime(self.block_host_time.0 + u64::from(self.pass_off + offset) * 1_000_000_000 / rate);
        for r in 0..self.program.midi_out.len() {
            let (n, route) = self.program.midi_out[r];
            if n != node {
                continue;
            }
            // An MPE port: every note on a channel of its own, its expressions on that channel
            if self.program.midi_mpe[r].is_some() {
                self.send_mpe(r, route, kind, at);
                continue;
            }
            let ch = route.channel & 0x0F;
            let sent = &mut self.program.midi_sent[r * 128..(r + 1) * 128];
            let (bytes, len) = match kind {
                EventKind::NoteOn { key, velocity, .. } => {
                    let k = route.fixed_note.unwrap_or(key) & 0x7F;
                    sent[usize::from(key & 0x7F)] = k as i8;
                    let v = route.fixed_velocity.unwrap_or(((velocity * 127.0).round() as u8).max(1)).clamp(1, 127);
                    ([0x90 | ch, k, v], 3)
                }
                EventKind::NoteOff { key, .. } => {
                    let slot = &mut sent[usize::from(key & 0x7F)];
                    let k = if *slot >= 0 { *slot as u8 } else { route.fixed_note.unwrap_or(key) & 0x7F };
                    *slot = -1;
                    ([0x80 | ch, k, 0], 3)
                }
                EventKind::Midi { bytes } if (0x80..0xF0).contains(&bytes[0]) => {
                    let len = lm_midimap::message::channel_message_len(bytes[0]);
                    ([(bytes[0] & 0xF0) | ch, bytes[1], bytes[2]], len)
                }
                // Stop, locate, a wrap: the device's notes end too (all notes off on the route's channel)
                EventKind::AllNotesRelease | EventKind::AllSoundOff => {
                    sent.fill(-1);
                    ([0xB0 | ch, 123, 0], 3)
                }
                _ => continue,
            };
            let e = MidiOutEvent { port: route.port, at, len, bytes };
            if self.out.midi_out.push(e).is_err() {
                self.midi_out_lost = self.midi_out_lost.saturating_add(1);
            }
        }
    }

    /// A node's event out through an MPE port (route `r`): a note-on takes a member channel (its bend centred first),
    /// the note's expressions follow on that channel, and what is not a note's goes to the zone's master channel
    fn send_mpe(&mut self, r: usize, route: MidiOutRoute, kind: EventKind, at: HostTime) {
        let Some(mpe) = self.program.midi_mpe[r].as_mut() else { return };
        let mut msgs = [([0u8; 3], 0u8); 17];
        let mut n = 0;
        let mut put = |bytes: [u8; 3], len: u8| {
            if n < msgs.len() {
                msgs[n] = (bytes, len);
                n += 1;
            }
        };
        match kind {
            EventKind::NoteOn { note, key, velocity, .. } => {
                let k = route.fixed_note.unwrap_or(key) & 0x7F;
                let (ch, taken) = mpe.start(note.0, k);
                if let Some(old_key) = taken {
                    put([0x80 | ch, old_key, 0], 3);
                }
                let v = route.fixed_velocity.unwrap_or(((velocity * 127.0).round() as u8).max(1)).clamp(1, 127);
                put(mpe.bend(ch, 0.0), 3);
                put([0x90 | ch, k, v], 3);
            }
            EventKind::NoteOff { note, .. } => {
                if let Some((ch, k)) = mpe.end(note.0) {
                    put([0x80 | ch, k, 0], 3);
                }
            }
            EventKind::NoteExpression { note, expression, value, .. } => {
                if let Some(ch) = mpe.channel(note.0)
                    && let Some((bytes, len)) = mpe.expression(ch, expression, value)
                {
                    put(bytes, len);
                }
            }
            EventKind::Midi { bytes } if (0x80..0xF0).contains(&bytes[0]) => {
                let len = lm_midimap::message::channel_message_len(bytes[0]);
                put([(bytes[0] & 0xF0) | mpe.master(), bytes[1], bytes[2]], len);
            }
            EventKind::AllNotesRelease | EventKind::AllSoundOff => {
                put([0xB0 | mpe.master(), 123, 0], 3);
                for ch in mpe.channels() {
                    put([0xB0 | ch, 123, 0], 3);
                }
                mpe.clear();
            }
            _ => {}
        }
        for &(bytes, len) in &msgs[..n] {
            let e = MidiOutEvent { port: route.port, at, len, bytes };
            if self.out.midi_out.push(e).is_err() {
                self.midi_out_lost = self.midi_out_lost.saturating_add(1);
            }
        }
    }

    fn pass(&mut self, off: usize, n: usize, first: bool, inputs: &[&[f32]], live_next: &mut usize) {
        self.pass_off = off as u32;
        let info = self.transport_info();
        for node in self.program.nodes.iter_mut() {
            if let Some(io) = node.io.as_mut() {
                io.events.clear();
            }
        }
        // Note-offs and releases owed from installs, stops, locates and wraps
        for k in 0..self.pending.len() {
            let (node, kind) = self.pending[k];
            if let Some(i) = self.program.node_of(node) {
                self.push_event(i, 0, kind);
            }
        }
        self.pending.clear();
        self.expire_midi();
        if first {
            self.take_dirty_params();
        }
        self.automation_events(n);
        if self.transport.playing {
            self.item_events(n);
        }
        self.live_events(off, n, live_next);
        self.modulation_events(n);
        if !self.program.chains.is_empty() {
            self.run_chains(n, &info);
        }
        for node in self.program.nodes.iter_mut() {
            if let Some(io) = node.io.as_mut() {
                io.events.finish();
            }
        }
        self.render_previews(n, &info);

        for level in 0..self.program.levels.len() {
            let count = self.program.levels[level].len();
            for k in 0..count {
                let i = self.program.levels[level][k];
                self.prepare(i, off, n, inputs);
            }
            self.run_level(level, n, &info);
            for k in 0..count {
                let i = self.program.levels[level][k];
                self.post(i, n);
            }
        }
        self.jump = false;
    }

    /// The note effects: for each chain, the stream the sources pushed goes through its effects in order, and what
    /// comes out is delivered to the instrument (and its MIDI out). An effect that is not built yet, or is dead,
    /// passes the stream through
    fn run_chains(&mut self, n: usize, info: &TransportInfo) {
        for c in 0..self.program.chains.len() {
            {
                let chain = &mut self.program.chains[c];
                chain.cur.clear();
                chain.source.finish();
                for e in chain.source.sorted() {
                    if !chain.cur.push(e.offset, e.kind) {
                        self.dropped[1] = self.dropped[1].saturating_add(1);
                    }
                }
                chain.source.clear();
            }
            for k in 0..self.program.chains[c].effects.len() {
                let ei = self.program.chains[c].effects[k];
                let node = self.program.nodes[ei as usize].desc.node;
                let slot = node.slot();
                let ready = self.slots[slot].core.is_some()
                    && !self.slots[slot].dead
                    && self.slots[slot].generation == node.generation;
                if !ready {
                    continue;
                }
                let Some(mut core) = self.slots[slot].core.take() else { continue };
                let Program { nodes, chains, .. } = &mut *self.program;
                let chain = &mut chains[c];
                let mut lost = 0u32;
                chain.merge.clear();
                if let Some(io) = nodes[ei as usize].io.as_mut() {
                    io.events.finish();
                    for e in io.events.sorted() {
                        lost += u32::from(!chain.merge.push(e.offset, e.kind));
                    }
                }
                for e in chain.cur.as_slice() {
                    lost += u32::from(!chain.merge.push(e.offset, e.kind));
                }
                self.dropped[1] = self.dropped[1].saturating_add(lost);
                chain.merge.finish();
                chain.next.clear();
                let started = std::time::Instant::now();
                self.watch.current[0].store(slot as u32 + 1, Ordering::Relaxed);
                let result = catching(AssertUnwindSafe(|| {
                    let mut cx = NoteContext {
                        frames: n,
                        events: chain.merge.sorted(),
                        transport: info,
                        rng: &mut core.rng,
                        out: &mut chain.next,
                        host: &mut core.outbox,
                    };
                    core.processor.process_notes(&mut cx);
                }));
                self.watch.current[0].store(0, Ordering::Relaxed);
                let measured = started.elapsed().as_nanos() as f32;
                let cost = &mut self.slots[slot].cost_ns;
                *cost = if *cost >= UNMEASURED_NS { measured } else { 0.9 * *cost + 0.1 * measured };
                let generation = self.program.generation;
                for request in core.outbox.drain() {
                    if self.out.events.push(Telemetry::PluginRequest { generation, node, request }).is_err() {
                        self.dropped[0] = self.dropped[0].saturating_add(1);
                    }
                }
                self.dropped[4] = self.dropped[4].saturating_add(core.outbox.take_dropped());
                self.slots[slot].core = Some(core);
                let chain = &mut self.program.chains[c];
                self.dropped[1] = self.dropped[1].saturating_add(chain.next.take_dropped());
                match result {
                    Ok(()) => std::mem::swap(&mut chain.cur, &mut chain.next),
                    Err(payload) => {
                        self.slots[slot].dead = true;
                        self.report_panic(Some(node), None, payload);
                    }
                }
            }
            let target = self.program.chains[c].target;
            for j in 0..self.program.chains[c].cur.len() {
                let e = self.program.chains[c].cur.as_slice()[j];
                self.deliver(target, e.offset, e.kind);
            }
        }
    }

    fn take_dirty_params(&mut self) {
        let playing = self.transport.playing;
        let position = self.transport.position;
        for w in 0..self.params.words() {
            let mut bits = self.params.take_dirty(w);
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let s = w * 64 + b;
                let value = self.params.value_at(s);
                let gesture = self.params.touched_at(s);
                let was = std::mem::replace(&mut self.slot_state[s].was_touched, gesture);
                let touched = self.slot_state[s].hand(gesture, self.frame);
                if !touched {
                    if was && playing {
                        self.touched_value(s, value, position, false);
                    }
                    self.slot_state[s].touch_overflow = false;
                } else if playing {
                    self.touched_value(s, value, position, true);
                }
                let bi = self.program.slot_map[s];
                if bi == NONE {
                    self.slot_state[s] = SlotState::new(value);
                    continue;
                }
                let (automated, target, discrete, modulated) = {
                    let b = &self.program.bindings[bi as usize];
                    (b.curve.is_some(), b.target, b.discrete, b.modulated.is_some())
                };
                if (automated || (modulated && !matches!(target, Target::Host(_)))) && !touched {
                    // A modulated parameter's value is the base the modulation is added to: the modulation pass
                    // emits the sum
                    // The automation has the parameter (again): emit its value at the next chance; after a release,
                    // back to the curve in 50 ms (I-21.5)
                    let st = &mut self.slot_state[s];
                    st.resume = true;
                    if was && !discrete {
                        st.ret = Some((self.frame, st.current));
                    }
                    continue;
                }
                match target {
                    Target::Processor { node, index } => {
                        self.slot_state[s].emitted = value;
                        self.slot_state[s].current = value;
                        self.push_event(node, 0, EventKind::Param { index, value });
                    }
                    Target::Host(_) => self.slot_state[s].set_target(value, discrete),
                    Target::Cc { node, channel, number } => self.cc_value(s, node, 0, channel, number, value),
                    Target::Modulator { modulator, index } => {
                        self.program.modulators[modulator as usize].params[index as usize] = value;
                        self.slot_state[s].current = value;
                    }
                }
            }
        }
    }

    /// A CC rack value (F-IJN-025): a control change to the processor when its rounded value changes
    fn cc_value(&mut self, s: usize, node: u32, offset: u32, channel: u8, number: u8, value: f32) {
        let st = &mut self.slot_state[s];
        st.current = value;
        let q = value.round().clamp(0.0, 127.0);
        if st.emitted.is_nan() || st.emitted.round() != q {
            st.emitted = value;
            self.push_event(node, offset, EventKind::Midi { bytes: [0xB0 | (channel & 0x0F), number & 0x7F, q as u8] });
        }
    }

    fn automation_events(&mut self, n: usize) {
        let pos0 = self.transport.position;
        let frame0 = self.frame;
        let jump = self.jump;
        let playing = self.transport.playing;
        let ret_len = u64::from(self.spec.sample_rate) / RETURN_DIV;
        let Program { automated, bindings, nodes, chains, .. } = &mut *self.program;
        let dropped = &mut self.dropped[1];
        for &bi in automated.iter() {
            let b = &bindings[bi as usize];
            // A processor's parameter, or a CC of the rack (a control change when the rounded value changes)
            let (node, index, cc) = match b.target {
                Target::Processor { node, index } => (node, index, None),
                Target::Cc { node, channel, number } => (node, 0, Some((channel, number))),
                Target::Host(_) | Target::Modulator { .. } => continue,
            };
            let Some(curve) = b.curve.as_ref() else { continue };
            let slot = b.slot as usize;
            let st = &mut self.slot_state[slot];
            let hand = st.hand(self.params.touched_at(slot), frame0);
            if playing && hand && b.mode == AutomationMode::Latch {
                st.latched = true;
            }
            let write = playing && b.mode == AutomationMode::Write;
            if hand || (playing && st.latched) || write {
                if write && jump && !hand && !st.touch_overflow {
                    // A Write pass starts: its first value (I-21.5)
                    let t = Touched { slot: ParamSlot(slot as u32), value: st.current, sample: pos0, touching: true };
                    if self.out.touched.push(t).is_err() {
                        st.touch_overflow = true;
                    }
                }
                continue;
            }
            if b.modulated.is_some() {
                // The modulation pass emits the curve's value plus the modulation
                continue;
            }
            if jump {
                st.ret = None;
            }
            let resume = jump || st.resume || st.emitted.is_nan();
            st.resume = false;
            let chain = nodes[node as usize].chain;
            let Some(io) = nodes[node as usize].io.as_mut() else { continue };
            let returning = st.ret.is_some();
            let mut emit = |offset: usize, v: f32, force: bool| {
                let v = st.returning(v, frame0 + offset as u64, ret_len);
                if let Some((channel, number)) = cc {
                    st.current = v;
                    let q = v.round().clamp(0.0, 127.0);
                    if force || st.emitted.is_nan() || st.emitted.round() != q {
                        st.emitted = v;
                        let kind = EventKind::Midi { bytes: [0xB0 | channel, number, q as u8] };
                        // (a controller of the rack goes through the note effects too)
                        let pushed = if chain != NONE {
                            chains[chain as usize].source.push(offset as u32, kind)
                        } else {
                            io.events.push(offset as u32, kind)
                        };
                        if !pushed {
                            *dropped = dropped.saturating_add(1);
                        }
                    }
                    return;
                }
                if force || v.to_bits() != st.emitted.to_bits() {
                    st.emitted = v;
                    st.current = v;
                    if !io.events.push(offset as u32, EventKind::Param { index, value: v }) {
                        *dropped = dropped.saturating_add(1);
                    }
                }
            };
            if b.discrete {
                let from = if resume {
                    emit(0, curve.step_at(pos0), false);
                    pos0 + 1
                } else {
                    pos0
                };
                for p in curve.points_in(from, pos0 + n as i64) {
                    emit((p.sample - pos0) as usize, p.value, true);
                }
                continue;
            }
            if !playing && !b.discrete {
                // Stopped, the playhead does not move: the curve holds its value there (the return ramp still
                // runs, on the grid)
                emit(0, curve.value_at(pos0), false);
                let mut f = frame0.div_ceil(AUTOMATION_GRID) * AUTOMATION_GRID;
                while returning && f < frame0 + n as u64 {
                    emit((f - frame0) as usize, curve.value_at(pos0), false);
                    f += AUTOMATION_GRID;
                }
                continue;
            }
            match b.rate {
                Rate::Block => {
                    if resume {
                        emit(0, curve.value_at(pos0), false);
                    }
                    let mut f = frame0.div_ceil(AUTOMATION_GRID) * AUTOMATION_GRID;
                    while f < frame0 + n as u64 {
                        let off = (f - frame0) as usize;
                        if !(resume && off == 0) {
                            emit(off, curve.value_at(pos0 + off as i64), false);
                        }
                        f += AUTOMATION_GRID;
                    }
                }
                Rate::Sample => {
                    let buf = &mut self.curve[..n];
                    curve.fill(pos0, buf);
                    for (i, &v) in buf.iter().enumerate() {
                        emit(i, v, false);
                    }
                }
            }
        }
    }

    /// The modulators of the program (decisoes-autonomas, 26/11, item 1): evaluated on the automation grid, then
    /// added to each target's base value (its automation or its document value). A hand on the parameter, or an
    /// automation being recorded, wins: the modulation leaves it alone
    fn modulation_events(&mut self, n: usize) {
        if self.program.modulators.is_empty() || n == 0 {
            return;
        }
        let grid = AUTOMATION_GRID as usize;
        let pos0 = self.transport.position;
        let frame0 = self.frame;
        let playing = self.transport.playing;
        let rate = self.spec.sample_rate;
        let ppq = self.program.tempo.ppq().get();
        let first = (frame0.div_ceil(AUTOMATION_GRID) * AUTOMATION_GRID - frame0) as usize;
        let g1 = if first == 0 { grid } else { first };
        self.mod_first_grid = g1;
        let start_of = |k: usize| if k == 0 { 0 } else { g1 + (k - 1) * grid };
        let mut steps = 0;
        while start_of(steps) < n {
            steps += 1;
        }
        for m in 0..self.program.modulators.len() {
            let steps = steps.min(self.program.modulators[m].values.len());
            for k in 0..steps {
                let off = start_of(k);
                let len = (start_of(k + 1).min(n) - off) as u32;
                let timeline = if playing { pos0 + off as i64 } else { frame0 as i64 + off as i64 };
                // The parameters at this step: an automated one follows its curve (unless a hand holds it)
                let mut p = self.program.modulators[m].params;
                for (i, v) in p.iter_mut().enumerate() {
                    let bi = self.program.modulators[m].param_bindings[i];
                    if bi == NONE {
                        continue;
                    }
                    let b = &self.program.bindings[bi as usize];
                    if let Some(curve) = &b.curve {
                        let s = b.slot as usize;
                        if !self.slot_state[s].hand(self.params.touched_at(s), frame0) {
                            *v = if b.discrete {
                                curve.step_at(pos0 + off as i64)
                            } else {
                                curve.value_at(pos0 + off as i64)
                            };
                        }
                    }
                }
                let tick = self.program.tempo.tick_at(timeline);
                let md = &mut self.program.modulators[m];
                let b = match md.kind {
                    ModKind::Lfo => {
                        let ph = crate::modulation::lfo_phase(&p, rate, timeline, tick, ppq);
                        crate::modulation::lfo_value(&p, ph, md.seed, md.user_wave.as_deref())
                    }
                    ModKind::Random => crate::modulation::random_value(&p, rate, timeline, md.seed),
                    ModKind::Macro => p[crate::modulation::P_VALUE].clamp(0.0, 1.0),
                    ModKind::Envelope => {
                        if let Some((at, on)) = md.gate
                            && at as usize <= off
                        {
                            md.envelope.gate(on);
                            md.gate = None;
                        }
                        let v = md.envelope.level;
                        md.envelope.advance(&p, rate, len);
                        v
                    }
                    ModKind::Peak => {
                        let cell = (frame0 + off as u64) / AUTOMATION_GRID;
                        md.peak.value_at(cell, &p, rate)
                    }
                };
                md.values[k] = if b.is_finite() { b } else { 0.0 };
            }
            md_gate_reset(&mut self.program.modulators[m]);
            // The targets
            for t in 0..self.program.modulators[m].targets.len() {
                let (slot, range, amount, lo, hi) = {
                    let x = &self.program.modulators[m].targets[t];
                    (x.slot as usize, x.range, x.amount, x.min.min(x.max), x.min.max(x.max))
                };
                let bi = self.program.slot_map[slot];
                if bi == NONE {
                    continue;
                }
                let (target, discrete, mode) = {
                    let b = &self.program.bindings[bi as usize];
                    (b.target, b.discrete, b.mode)
                };
                let hand = self.slot_state[slot].hand(self.params.touched_at(slot), frame0);
                let recording = playing && (self.slot_state[slot].latched || mode == AutomationMode::Write);
                if hand || recording {
                    self.program.modulators[m].targets[t].last = f32::NAN;
                    continue;
                }
                if let Target::Host(_) = target {
                    let md = &mut self.program.modulators[m];
                    for k in 0..steps {
                        let v = md.values[k] * amount;
                        md.targets[t].steps[k] = v;
                    }
                    continue;
                }
                if matches!(target, Target::Modulator { .. }) {
                    continue;
                }
                let mut force = std::mem::take(&mut self.slot_state[slot].resume) || self.jump;
                for k in 0..steps {
                    let off = start_of(k);
                    let at = if playing { pos0 + off as i64 } else { pos0 };
                    let base = match self.program.bindings[bi as usize].curve.as_ref() {
                        Some(c) if discrete => c.step_at(at),
                        Some(c) => c.value_at(at),
                        None => self.params.value_at(slot),
                    };
                    let b = self.program.modulators[m].values[k];
                    let nrm = (range.normalize(base) + amount * b).clamp(lo, hi);
                    let v = range.to_units(nrm);
                    let tg = &mut self.program.modulators[m].targets[t];
                    if force || tg.last.is_nan() || v.to_bits() != tg.last.to_bits() {
                        force = false;
                        tg.last = v;
                        self.slot_state[slot].current = v;
                        self.slot_state[slot].emitted = v;
                        match target {
                            Target::Processor { node, index } => {
                                self.push_event(node, off as u32, EventKind::Param { index, value: v })
                            }
                            Target::Cc { node, channel, number } => {
                                self.cc_value(slot, node, off as u32, channel, number, v)
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    fn item_events(&mut self, n: usize) {
        let pos0 = self.transport.position;
        let ticks = self.program.tempo.ticks_in(pos0, pos0 + n as i64);
        for s in 0..self.program.midi.len() {
            let node = self.program.midi[s].node;
            let node_index = self.program.nodes[node as usize].desc.node;
            for it in 0..self.program.midi[s].items.len() {
                let item = self.program.midi[s].items[it].key();
                let count = self.program.midi[s].items[it].events_in(ticks.clone()).len();
                for e in 0..count {
                    let ev = self.program.midi[s].items[it].events_in(ticks.clone())[e];
                    let offset = (self.program.tempo.sample_at(ev.tick) - pos0).clamp(0, n as i64 - 1) as u32;
                    match ev.kind {
                        crate::program::ItemEventKind::NoteOn { n: num, channel, key, velocity } => {
                            match self.notes.start(NoteSource::Item { item, n: num }, channel, key, node_index) {
                                Some(note) => {
                                    self.push_event(node, offset, EventKind::NoteOn { note, channel, key, velocity })
                                }
                                None => self.dropped[2] = self.dropped[2].saturating_add(1),
                            }
                        }
                        crate::program::ItemEventKind::NoteOff { n: num } => {
                            if let Some(a) = self.notes.take_item_note(item, num)
                                && let Some(target) = self.program.node_of(a.node)
                            {
                                let kind =
                                    EventKind::NoteOff { note: a.note, channel: a.channel, key: a.key, velocity: 0.0 };
                                self.push_event(target, offset, kind);
                            }
                        }
                        crate::program::ItemEventKind::Midi { bytes } => {
                            self.push_event(node, offset, EventKind::Midi { bytes });
                        }
                        crate::program::ItemEventKind::Expression { n: num, expression, value, until } => {
                            // Only to a note that sounds, on the node that has it
                            if let Some(a) = self.notes.item_note(item, num)
                                && let Some(target) = self.program.node_of(a.node)
                            {
                                let tempo = &self.program.tempo;
                                let ramp = (tempo.sample_at(until.max(ev.tick)) - tempo.sample_at(ev.tick)).max(0);
                                let kind = EventKind::NoteExpression {
                                    note: a.note,
                                    expression,
                                    value,
                                    ramp: u32::try_from(ramp).unwrap_or(u32::MAX),
                                };
                                self.push_event(target, offset, kind);
                            }
                        }
                    }
                }
            }
        }
    }

    fn live_events(&mut self, off: usize, n: usize, next: &mut usize) {
        while *next < self.live.len() {
            let m = self.live[*next];
            let at = m.offset as usize;
            if at >= off + n {
                break;
            }
            *next += 1;
            let offset = (at.saturating_sub(off)) as u32;
            let status = m.bytes[0] & 0xF0;
            let channel = m.bytes[0] & 0x0F;
            let (key, vel) = (m.bytes[1] & 0x7F, m.bytes[2] & 0x7F);
            let first = self.program.ports.partition_point(|r| r.port < m.port);
            let routes = first..self.program.ports.partition_point(|r| r.port <= m.port);
            let note_off = status == 0x80 || (status == 0x90 && vel == 0);
            // The interface's notes are not a device: no mapping takes them, and they are not recorded as input
            let consumed = !m.port.is_gui() && self.map_message(m, offset);
            if !consumed && !m.port.is_gui() && self.transport.recording {
                self.record_midi(m, offset);
            }
            if note_off {
                self.port_note_off(m.port, channel, key, offset);
                continue;
            }
            if consumed {
                continue; // a mapped control does not also play the instruments (I-21.2, `consume`)
            }
            // MPE: the bend, pressure and CC 74 of a member channel belong to the note on that channel
            if let Some(z) = self.program.mpe.iter().position(|p| p.port == m.port)
                && self.program.mpe[z].is_member(channel)
                && let Some((expression, value)) = self.program.mpe[z].expression(m.bytes)
            {
                self.program.mpe[z].remember(channel, expression, value);
                self.mpe_expression(m.port, channel, expression, value, offset);
                continue;
            }
            if status == 0xB0 && (key == 120 || key == 123) {
                // All sound off / all notes off on this channel: end the port's notes here, at this offset
                while let Some(a) =
                    self.notes.take_one(|a| a.source == NoteSource::Port(m.port) && a.channel == channel)
                {
                    if let Some(i) = self.program.node_of(a.node) {
                        let kind = EventKind::NoteOff { note: a.note, channel, key: a.key, velocity: 0.0 };
                        self.push_event(i, offset, kind);
                    }
                }
            }
            for r in routes {
                let route = self.program.ports[r];
                if route.channel.is_some_and(|c| c != channel) {
                    continue;
                }
                let Some(target) = self.program.node_of(route.node) else { continue };
                if status == 0x90 {
                    // A second note-on of a held key on the same node ends the first (no stranded voice)
                    let mut prev = None;
                    self.notes.take_where(
                        |a| {
                            a.source == NoteSource::Port(m.port)
                                && a.channel == channel
                                && a.key == key
                                && a.node == route.node
                        },
                        |a| prev = Some(a.note),
                    );
                    if let Some(note) = prev {
                        self.push_event(target, offset, EventKind::NoteOff { note, channel, key, velocity: 0.0 });
                    }
                    // The track's key range, base key, host velocity and microtuner (F-IJN-017/019/021)
                    let shaped = self.program.live_keys.iter().find(|(n, _)| *n == route.node).map(|(_, k)| k);
                    let (play_key, tuning, vel) = match shaped {
                        Some(k) => {
                            let Some((pk, t)) = k.map(key) else { continue };
                            (pk, t, k.velocity(vel))
                        }
                        None => (key, 0.0, vel),
                    };
                    match self.notes.start(NoteSource::Port(m.port), channel, key, route.node) {
                        Some(note) => {
                            let vel = route.fixed_velocity.map_or(vel, |f| f.clamp(1, 127));
                            let velocity = f32::from(vel) / 127.0;
                            self.push_event(
                                target,
                                offset,
                                EventKind::NoteOn { note, channel, key: play_key, velocity },
                            );
                            self.notes.set_base_tuning(note, tuning);
                            if tuning != 0.0 {
                                let kind = EventKind::NoteExpression {
                                    note,
                                    expression: crate::Expression::Tuning,
                                    value: tuning,
                                    ramp: 0,
                                };
                                self.push_event(target, offset, kind);
                            }
                            // An MPE note starts with what its channel sent before it (the bend comes first)
                            if let Some(z) = self.program.mpe.iter().position(|p| p.port == m.port)
                                && self.program.mpe[z].is_member(channel)
                            {
                                for (expression, value) in self.program.mpe[z].pending(channel).into_iter().flatten() {
                                    let value =
                                        if expression == crate::Expression::Tuning { tuning + value } else { value };
                                    let kind = EventKind::NoteExpression { note, expression, value, ramp: 0 };
                                    self.push_event(target, offset, kind);
                                }
                            }
                        }
                        None => self.dropped[2] = self.dropped[2].saturating_add(1),
                    }
                } else {
                    self.push_event(target, offset, EventKind::Midi { bytes: m.bytes });
                }
            }
        }
    }

    /// An MPE member channel's expression for the note it sounds (a bend adds to the note's own tuning offset)
    fn mpe_expression(&mut self, port: PortIndex, channel: u8, expression: crate::Expression, value: f32, offset: u32) {
        let Some(a) = self.notes.port_note(port, channel).copied() else { return };
        let Some(i) = self.program.node_of(a.node) else { return };
        let value = if expression == crate::Expression::Tuning { a.base_tuning + value } else { value };
        self.push_event(i, offset, EventKind::NoteExpression { note: a.note, expression, value, ramp: 0 });
    }

    fn port_note_off(&mut self, port: PortIndex, channel: u8, key: u8, offset: u32) {
        while let Some(a) =
            self.notes.take_one(|a| a.source == NoteSource::Port(port) && a.channel == channel && a.key == key)
        {
            if let Some(i) = self.program.node_of(a.node) {
                self.push_event(i, offset, EventKind::NoteOff { note: a.note, channel, key, velocity: 0.0 });
            }
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // Nodes

    fn prepare(&mut self, i: u32, off: usize, n: usize, inputs: &[&[f32]]) {
        let Some(mut io) = self.program.nodes[i as usize].io.take() else { return };
        let layout = io.layout;
        let in_channels = layout.inputs;
        for ch in 0..in_channels {
            let o = layout.input_offset(ch);
            io.buffer[o..o + n].fill(0.0);
        }
        for ch in 0..layout.sidechain {
            let o = layout.sidechain_offset(ch);
            io.buffer[o..o + n].fill(0.0);
        }
        let playing = self.transport.playing;
        let pos0 = self.transport.position;
        let mut missing = 0u64;
        {
            let node = &self.program.nodes[i as usize];
            // Device input (monitoring)
            for (ch, &dc) in node.input.iter().enumerate() {
                if let Some(src) = inputs.get(usize::from(dc)) {
                    let o = layout.input_offset(ch);
                    let end = (off + n).min(src.len());
                    if end > off {
                        io.buffer[o..o + (end - off)].copy_from_slice(&src[off..end]);
                    }
                }
            }
            if playing && !node.audio.is_empty() {
                missing = play_audio(node, &mut io.buffer, &layout, pos0, n, &mut self.stream_scratch);
            }
        }
        if missing > 0 {
            let (generation, node) = (self.program.generation, self.program.nodes[i as usize].desc.node);
            let frames = u32::try_from(missing).unwrap_or(u32::MAX);
            self.telemetry(Telemetry::StreamUnderrun { generation, node, frames });
        }
        if let Some((mi, _)) = &self.program.metronome
            && *mi == i
        {
            if playing {
                self.metronome_into(&mut io.buffer, &layout, n);
            } else if self.counting.is_some() {
                self.count_into(&mut io.buffer, &layout, n);
            }
        }
        if self.program.preview == Some(i) {
            for ch in 0..in_channels {
                let src = &self.preview_mix[(ch % 2) * self.spec.max_block..][..n];
                let o = layout.input_offset(ch);
                for (d, s) in io.buffer[o..o + n].iter_mut().zip(src) {
                    *d += *s;
                }
            }
        }
        // Feeds, in the program's order
        let range = self.program.nodes[i as usize].feeds.clone();
        for f in range {
            let feed_gain = self.program.feeds[f as usize].gain;
            match feed_gain {
                Some(h) => {
                    let mut g = std::mem::take(&mut self.gain);
                    self.host_values(h, &mut g[..n]);
                    self.gain = g;
                }
                None => self.gain[..n].fill(1.0),
            }
            let Program { nodes, feeds, delays, .. } = &mut *self.program;
            let pf = &feeds[f as usize];
            let Some(src) = nodes[pf.from as usize].io.as_ref() else { continue };
            let delay = pf.delay.map(|d| &mut delays[d as usize]);
            mix_feed(pf, src, &mut io, delay, &self.gain[..n], n);
        }
        self.program.nodes[i as usize].io = Some(io);
    }

    fn run_level(&mut self, level: usize, n: usize, info: &TransportInfo) {
        let count = self.program.levels[level].len();
        let mut next = 0usize;
        loop {
            while let Some(job) = self.workers.take_done() {
                self.finish(job);
            }
            // Next node that needs a processor call
            while next < count {
                let i = self.program.levels[level][next];
                if self.runnable(i) {
                    break;
                }
                self.silence_if_needed(i, n);
                next += 1;
            }
            if next < count {
                let i = self.program.levels[level][next];
                next += 1;
                let slot = self.program.nodes[i as usize].desc.node.slot();
                let worth = count > 1 && self.slots[slot].cost_ns >= DISPATCH_MIN_NS;
                let Some(mut job) = self.job(i, n, info) else {
                    self.silence_if_needed(i, n);
                    continue;
                };
                match self.workers.idle() {
                    Some(w) if worth => self.workers.give(w, job),
                    _ => {
                        run_job(&mut job, &self.watch.current[0]);
                        self.finish(job);
                    }
                }
                continue;
            }
            if !self.workers.any_busy() {
                break;
            }
            std::hint::spin_loop();
        }
    }

    fn runnable(&self, i: u32) -> bool {
        let node = &self.program.nodes[i as usize];
        if node.desc.kind != NodeKind::Processor {
            return false;
        }
        let slot = &self.slots[node.desc.node.slot()];
        slot.core.is_some() && !slot.dead && !slot.tail_done && slot.generation == node.desc.node.generation
    }

    /// A processor node that is not called (waiting, dead, a finished tail) outputs silence
    fn silence_if_needed(&mut self, i: u32, n: usize) {
        let node = &mut self.program.nodes[i as usize];
        if node.desc.kind != NodeKind::Processor {
            return;
        }
        if let Some(io) = node.io.as_mut() {
            let layout = io.layout;
            for ch in 0..layout.outputs {
                let o = layout.output_offset(ch);
                io.buffer[o..o + n].fill(0.0);
            }
        }
    }

    /// The job of a runnable node, or `None` when its processor or memory is not home (an invariant that should
    /// hold: the node then outputs silence for this pass instead of panicking in the audio block)
    fn job(&mut self, i: u32, n: usize, info: &TransportInfo) -> Option<Job> {
        let slot = self.program.nodes[i as usize].desc.node.slot();
        let core = self.slots[slot].core.take()?;
        let Some(io) = self.program.nodes[i as usize].io.take() else {
            self.slots[slot].core = Some(core);
            return None;
        };
        Some(Job {
            node: i,
            slot: slot as u32,
            core,
            io,
            frames: n,
            transport: *info,
            panic: None,
            non_finite: false,
            cost_ns: 0.0,
        })
    }

    fn finish(&mut self, mut job: Job) {
        let node = self.program.nodes[job.node as usize].desc.node;
        let generation = self.program.generation;
        for request in job.core.outbox.drain() {
            if self.out.events.push(Telemetry::PluginRequest { generation, node, request }).is_err() {
                self.dropped[0] = self.dropped[0].saturating_add(1);
            }
        }
        let lost = job.core.outbox.take_dropped();
        self.dropped[4] = self.dropped[4].saturating_add(lost);
        let slot = job.slot as usize;
        let measured = job.cost_ns;
        let cost = &mut self.slots[slot].cost_ns;
        *cost = if *cost >= UNMEASURED_NS { measured } else { 0.9 * *cost + 0.1 * measured };
        self.slots[slot].core = Some(job.core);
        self.program.nodes[job.node as usize].io = Some(job.io);
        if let Some(payload) = job.panic.take() {
            self.slots[slot].dead = true;
            self.report_panic(Some(node), None, payload);
        } else if job.non_finite {
            self.slots[slot].dead = true;
            self.telemetry(Telemetry::NodeNonFinite { generation, node });
        }
    }

    fn post(&mut self, i: u32, n: usize) {
        let (gain, pan, muted, node, kind, tail) = {
            let d = &self.program.nodes[i as usize];
            (d.gain, d.pan, d.desc.muted, d.desc.node, d.desc.kind, d.desc.tail)
        };
        if let Some(h) = gain {
            let mut g = std::mem::take(&mut self.gain);
            self.host_values(h, &mut g[..n]);
            self.gain = g;
        }
        if let Some(h) = pan {
            let mut p = std::mem::take(&mut self.pan);
            self.host_values(h, &mut p[..n]);
            self.pan = p;
        }
        let Some(mut io) = self.program.nodes[i as usize].io.take() else { return };
        let layout = io.layout;
        let slot = &mut self.slots[node.slot()];
        let outputs = layout.outputs;

        // Sources and sums can carry NaN too (a broken file): the node is silenced and marked like a processor
        if kind == NodeKind::Sum && !slot.dead {
            let bad = (0..outputs).any(|ch| {
                let o = layout.output_offset(ch);
                !scan(&io.buffer[o..o + n]).1
            });
            if bad {
                slot.dead = true;
                if self.out.events.push(Telemetry::NodeNonFinite { generation: self.program.generation, node }).is_err()
                {
                    self.dropped[0] = self.dropped[0].saturating_add(1);
                }
            }
        }
        // The hard cap falls on an exact frame, not on the end of the block that crosses it, so the audible tail
        // is the same whatever the block size
        if tail && !slot.tail_done {
            let since = slot.tail_since.unwrap_or(self.frame);
            let cap = since + TAIL_SECONDS * u64::from(self.spec.sample_rate);
            if self.frame + n as u64 > cap {
                let from = usize::try_from(cap.saturating_sub(self.frame)).unwrap_or(n).min(n);
                for ch in 0..outputs {
                    let o = layout.output_offset(ch);
                    io.buffer[o + from..o + n].fill(0.0);
                }
            }
        }
        if slot.dead || slot.tail_done {
            for ch in 0..outputs {
                let o = layout.output_offset(ch);
                io.buffer[o..o + n].fill(0.0);
            }
        }

        // Strip: gain, balance, mute (ramped). The per-sample gain is built once, then each channel is one
        // multiplication per sample
        let target_mute = if muted { 0.0 } else { 1.0 };
        let ramping = slot.mute_gain != target_mute;
        if gain.is_some() || pan.is_some() || ramping || muted {
            let g = &mut self.gain[..n];
            if gain.is_none() {
                g.fill(1.0);
            }
            if ramping {
                let mut m = slot.mute_gain;
                for x in g.iter_mut() {
                    if m != target_mute {
                        m = if target_mute > m {
                            (m + 1.0 / MUTE_FADE).min(1.0)
                        } else {
                            (m - 1.0 / MUTE_FADE).max(0.0)
                        };
                    }
                    *x *= m;
                }
                slot.mute_gain = m;
            } else if muted {
                g.fill(0.0);
            }
            let g = &self.gain[..n];
            if pan.is_some() && outputs == 2 {
                let p = &self.pan[..n];
                for ch in 0..2 {
                    let o = layout.output_offset(ch);
                    let side = if ch == 0 { -1.0 } else { 1.0 };
                    for ((x, g), p) in io.buffer[o..o + n].iter_mut().zip(g).zip(p) {
                        *x *= g * (1.0 + side * p.clamp(-1.0, 1.0)).min(1.0);
                    }
                }
            } else {
                for ch in 0..outputs {
                    let o = layout.output_offset(ch);
                    for (x, g) in io.buffer[o..o + n].iter_mut().zip(g) {
                        *x *= g;
                    }
                }
            }
        }

        // Master protection (U20)
        if self.program.master == Some(i) {
            let mut peak = 0.0f32;
            for ch in 0..outputs {
                let o = layout.output_offset(ch);
                peak = peak.max(scan(&io.buffer[o..o + n]).0);
            }
            let p = &mut self.protection;
            if peak > PROTECTION_THRESHOLD {
                p.quiet = 0;
                if !p.engaged {
                    p.engaged = true;
                    if self
                        .out
                        .events
                        .push(Telemetry::MasterProtection { generation: self.program.generation, peak })
                        .is_err()
                    {
                        self.dropped[0] = self.dropped[0].saturating_add(1);
                    }
                }
            } else if p.engaged {
                p.quiet += n as u64;
                if p.quiet >= u64::from(self.spec.sample_rate) {
                    p.engaged = false;
                }
            }
            let target = if p.engaged { 0.0 } else { 1.0 };
            if p.gain != target || p.engaged {
                let step = 1.0 / PROTECTION_FADE as f32;
                let start = p.gain;
                let mut g = start;
                for ch in 0..outputs {
                    let o = layout.output_offset(ch);
                    g = start;
                    for x in io.buffer[o..o + n].iter_mut() {
                        g = if target > g { (g + step).min(1.0) } else { (g - step).max(0.0) };
                        *x *= g;
                    }
                }
                p.gain = g;
            }
        }

        // Meter and tail
        let mut node_peak = 0.0f32;
        let metered = outputs.min(METER_CHANNELS);
        for ch in 0..outputs {
            let o = layout.output_offset(ch);
            let peak = scan(&io.buffer[o..o + n]).0;
            node_peak = node_peak.max(peak);
            if ch < metered
                && let Some(held) = slot.peak.get_mut(ch)
            {
                *held = held.max(peak);
            }
        }
        slot.channels = metered as u8;
        if tail && !slot.tail_done {
            let since = *slot.tail_since.get_or_insert(self.frame);
            slot.tail_quiet = if node_peak < SILENT { slot.tail_quiet + n as u64 } else { 0 };
            let rate = u64::from(self.spec.sample_rate);
            let age = self.frame + n as u64 - since;
            if age >= TAIL_SECONDS * rate || (age >= rate / 20 && slot.tail_quiet >= rate / 20) {
                slot.tail_done = true;
                if self.out.events.push(Telemetry::TailEnded { generation: self.program.generation, node }).is_err() {
                    self.dropped[0] = self.dropped[0].saturating_add(1);
                }
            }
        }
        // Peak followers measure this node's output (the legacy period's RMS)
        for m in self.program.modulators.iter_mut() {
            if m.kind == ModKind::Peak && m.input == i && outputs > 0 {
                let l = layout.output_offset(0);
                let r = layout.output_offset(1.min(outputs - 1));
                m.peak.accumulate(self.frame, &io.buffer[l..l + n], &io.buffer[r..r + n], &m.params);
            }
        }
        self.program.nodes[i as usize].io = Some(io);
    }

    /// Fills `out` with a host parameter's per-sample values (`1 − value` for a complement)
    fn host_values(&mut self, h: u32, out: &mut [f32]) {
        self.host_values_of(h, out);
        if self.program.host_params[h as usize].complement {
            for o in out.iter_mut() {
                *o = 1.0 - *o;
            }
        }
    }

    fn host_values_of(&mut self, h: u32, out: &mut [f32]) {
        self.host_base(h, out);
        let PHostParam { binding, .. } = self.program.host_params[h as usize];
        let Some(bi) = binding else { return };
        let Some((m, t)) = self.program.bindings[bi as usize].modulated else { return };
        let b = &self.program.bindings[bi as usize];
        let s = b.slot as usize;
        let hold = self.slot_state[s].hand(self.params.touched_at(s), self.frame)
            || (self.transport.playing && (self.slot_state[s].latched || b.mode == AutomationMode::Write));
        if hold {
            return;
        }
        let target = &self.program.modulators[m as usize].targets[t as usize];
        let g1 = self.mod_first_grid;
        let (range, lo, hi) = (target.range, target.min.min(target.max), target.min.max(target.max));
        for (i, o) in out.iter_mut().enumerate() {
            let k = if i < g1 { 0 } else { 1 + (i - g1) / AUTOMATION_GRID as usize };
            let k = k.min(target.steps.len() - 1);
            let n = (range.normalize(*o) + target.steps[k]).clamp(lo, hi);
            *o = range.to_units(n);
        }
    }

    fn host_base(&mut self, h: u32, out: &mut [f32]) {
        let PHostParam { value, binding, .. } = self.program.host_params[h as usize];
        let Some(bi) = binding else {
            out.fill(value);
            return;
        };
        let b = &self.program.bindings[bi as usize];
        let s = b.slot as usize;
        let state = &mut self.slot_state[s];
        let playing = self.transport.playing;
        let frame = self.frame;
        let hand = state.hand(self.params.touched_at(s), frame);
        if playing && hand && b.mode == AutomationMode::Latch {
            state.latched = true;
        }
        let hold = hand || (playing && (state.latched || b.mode == AutomationMode::Write));
        if let Some(curve) = &b.curve
            && !hold
        {
            if self.transport.playing {
                curve.fill(self.transport.position, out);
            } else {
                // Stopped: the value at the playhead, held
                out.fill(curve.value_at(self.transport.position));
            }
            if self.jump {
                state.ret = None;
            }
            if state.ret.is_some() {
                let len = u64::from(self.spec.sample_rate) / RETURN_DIV;
                for (i, o) in out.iter_mut().enumerate() {
                    *o = state.returning(*o, frame + i as u64, len);
                }
            }
            let last = out[out.len() - 1];
            state.current = last;
            state.target = last;
            state.left = 0;
            state.emitted = last;
            return;
        }
        for o in out.iter_mut() {
            *o = state.next();
        }
    }

    fn write_outputs(&mut self, outputs: &mut [&mut [f32]], off: usize, n: usize) {
        for r in self.program.outputs.iter() {
            let Some(dst) = outputs.get_mut(usize::from(r.device_channel)) else { continue };
            let Some(i) = self.program.node_of(r.node) else { continue };
            let Some(io) = self.program.nodes[i as usize].io.as_ref() else { continue };
            let o = io.layout.output_offset(usize::from(r.channel));
            let end = (off + n).min(dst.len());
            if end <= off {
                continue;
            }
            for (d, s) in dst[off..end].iter_mut().zip(&io.buffer[o..o + (end - off)]) {
                *d += *s;
            }
        }
    }

    fn metronome_into(&mut self, buffer: &mut [f32], layout: &BufferLayout, n: usize) {
        let Some((_, m)) = &self.program.metronome else { return };
        // The pre-roll counts even when the metronome is off (F-REC-024)
        let pre_roll = self.transport.record_from.is_some();
        if !pre_roll && (!m.enabled || (m.only_recording && !self.transport.recording)) {
            return;
        }
        let tempo = &self.program.tempo;
        let pos0 = self.transport.position;
        let ticks = tempo.ticks_in(pos0, pos0 + n as i64);
        let ppq = i64::from(tempo.ppq().get());
        let mut cursor = 0usize;
        let mut t = ticks.start;
        while t < ticks.end {
            let bbt = tempo.bar_beat_tick(t);
            let beat_ticks = tempo.time_signature_at(t).beat_ticks(tempo.ppq()).unwrap_or(ppq).max(1);
            if bbt.tick == 0 {
                let at = (tempo.sample_at(t) - pos0).clamp(0, n as i64) as usize;
                click(&mut self.metronome.click, m, buffer, layout, cursor, at);
                self.metronome.click = Some((bbt.beat == 1, 0));
                cursor = at;
            }
            let mut next = t + beat_ticks - i64::from(bbt.tick);
            if let Ok(bar_end) = tempo.bar_start(bbt.bar + 1) {
                next = next.min(bar_end);
            }
            t = next.max(t + 1);
        }
        click(&mut self.metronome.click, m, buffer, layout, cursor, n);
    }

    /// The count-in's clicks (whatever the metronome's own switch says): beat `k` at the sample where the `k`-th
    /// beat after the start falls, counted from the start, the accent on each bar's first beat
    fn count_into(&mut self, buffer: &mut [f32], layout: &BufferLayout, n: usize) {
        let Some((_, m)) = &self.program.metronome else { return };
        let Some(c) = self.counting.as_mut() else { return };
        let tempo = &self.program.tempo;
        let base = tempo.sample_at(c.t0);
        let mut cursor = 0usize;
        while c.next < c.beats {
            let d = tempo.sample_at(c.t0 + i64::from(c.next) * c.beat) - base;
            if d >= c.elapsed + n as i64 {
                break;
            }
            let at = (d - c.elapsed).clamp(0, n as i64) as usize;
            click(&mut self.metronome.click, m, buffer, layout, cursor, at);
            self.metronome.click = Some((c.next % c.per_bar == 0, 0));
            cursor = at;
            c.next += 1;
        }
        click(&mut self.metronome.click, m, buffer, layout, cursor, n);
    }

    // ------------------------------------------------------------------------------------------------------------
    // Previews

    fn free_preview(&mut self) -> Option<usize> {
        self.retry_preview_returns();
        if let Some(i) = self.previews.iter().position(|s| s.play.is_none() && s.ending.is_none()) {
            return Some(i);
        }
        // None free: the oldest playing one fades out, and the command waits for the next block
        if let Some(i) = (0..self.previews.len())
            .filter(|&i| self.previews[i].play.is_some() && self.previews[i].stopping.is_none())
            .min_by_key(|&i| self.previews[i].started_at)
        {
            self.previews[i].stopping = Some(MUTE_FADE as u32);
        }
        None
    }

    fn start_preview(&mut self, i: usize, cmd: PreviewCommand) {
        let s = &mut self.previews[i];
        s.stopping = None;
        s.started_at = self.frame;
        match cmd {
            PreviewCommand::Sample { id, data, gain } => {
                s.id = id;
                s.play = Some(PreviewPlay::Sample { data, gain, pos: 0 });
            }
            PreviewCommand::Preset { id, processor, key } => {
                s.id = id;
                s.play = Some(PreviewPlay::Preset {
                    processor,
                    key,
                    started: false,
                    off: false,
                    released_at: None,
                    quiet: 0,
                });
            }
            PreviewCommand::NoteOff { .. } | PreviewCommand::Stop { .. } => {}
        }
    }

    fn retry_preview_returns(&mut self) {
        for s in self.previews.iter_mut() {
            if let Some(ended) = s.ending.take()
                && let Err(PushError::Full(e)) = self.out.previews.push(ended)
            {
                s.ending = Some(e);
            }
        }
    }

    fn render_previews(&mut self, n: usize, info: &TransportInfo) {
        let max = self.spec.max_block;
        self.preview_mix.fill(0.0);
        let rate = u64::from(self.spec.sample_rate);
        for p in 0..self.previews.len() {
            let s = &mut self.previews[p];
            let Some(play) = s.play.as_mut() else { continue };
            let mut finished = false;
            let (left, right) = s.buffer.split_at_mut(max);
            left[..n].fill(0.0);
            right[..n].fill(0.0);
            match play {
                PreviewPlay::Sample { data, gain, pos } => {
                    let frames = data.frames();
                    let k = n.min(frames.saturating_sub(*pos));
                    for ch in 0..2 {
                        let src = data.channel(ch % data.channels());
                        let dst = if ch == 0 { &mut left[..k] } else { &mut right[..k] };
                        for (d, x) in dst.iter_mut().zip(&src[*pos..*pos + k]) {
                            *d = x * *gain;
                        }
                    }
                    *pos += k;
                    finished = *pos >= frames;
                }
                PreviewPlay::Preset { processor, key, started, off, released_at, quiet } => {
                    s.events.clear();
                    if !*started {
                        *started = true;
                        let on = EventKind::NoteOn { note: NoteId(1), channel: 0, key: *key, velocity: 0.8 };
                        s.events.push(0, on);
                    }
                    if *off && released_at.is_none() {
                        *released_at = Some(self.frame);
                        s.events.push(0, EventKind::NoteOff { note: NoteId(1), channel: 0, key: *key, velocity: 0.0 });
                    }
                    let events = s.events.finish();
                    let layout = BufferLayout { inputs: 0, outputs: 2, sidechain: 0, in_place: false, stride: max };
                    let rng = &mut s.rng;
                    let outbox = &mut s.outbox;
                    let buffer = &mut s.buffer;
                    let result = catching(AssertUnwindSafe(|| {
                        let mut cx = ProcessContext {
                            frames: n,
                            buffers: Buffers::new(buffer, layout, n),
                            events,
                            transport: info,
                            rng,
                            host: outbox,
                            display: None,
                        };
                        processor.process(&mut cx);
                    }));
                    for _ in s.outbox.drain() {}
                    let _ = s.outbox.take_dropped();
                    match result {
                        Err(payload) => {
                            s.panic = Some(payload);
                            s.buffer.fill(0.0);
                            finished = true;
                        }
                        Ok(()) => {
                            let peak = s.buffer[..n]
                                .iter()
                                .chain(&s.buffer[max..max + n])
                                .fold(0.0f32, |p, x| if x.is_finite() { p.max(x.abs()) } else { f32::INFINITY });
                            if !peak.is_finite() {
                                s.buffer.fill(0.0);
                                finished = true;
                            }
                            if let Some(at) = *released_at {
                                *quiet = if peak < SILENT { *quiet + n as u64 } else { 0 };
                                if *quiet >= rate / 10 || self.frame - at >= 5 * rate {
                                    finished = true;
                                }
                            }
                        }
                    }
                }
            }
            if let Some(left) = s.stopping.as_mut() {
                let mut g = *left as f32 / MUTE_FADE;
                for k in 0..n {
                    g = (g - 1.0 / MUTE_FADE).max(0.0);
                    s.buffer[k] *= g;
                    s.buffer[max + k] *= g;
                }
                *left = left.saturating_sub(n as u32);
                if *left == 0 {
                    finished = true;
                }
            }
            for k in 0..n {
                self.preview_mix[k] += s.buffer[k];
                self.preview_mix[max + k] += s.buffer[max + k];
            }
            if finished {
                self.end_preview(p);
            }
        }
    }

    fn end_preview(&mut self, p: usize) {
        let s = &mut self.previews[p];
        let Some(play) = s.play.take() else { return };
        s.stopping = None;
        let returned = match play {
            PreviewPlay::Sample { data, .. } => PreviewReturn::Sample(data),
            PreviewPlay::Preset { processor, .. } => PreviewReturn::Processor(processor),
        };
        let id = s.id;
        if let Some(payload) = s.panic.take() {
            self.report_panic(None, Some(id), payload);
        }
        let s = &mut self.previews[p];
        if let Err(PushError::Full(e)) = self.out.previews.push(PreviewEnded { id, returned }) {
            s.ending = Some(e);
        }
    }

    // ------------------------------------------------------------------------------------------------------------
    // Telemetry

    fn telemetry(&mut self, t: Telemetry) {
        if self.out.events.push(t).is_err() {
            self.dropped[0] = self.dropped[0].saturating_add(1);
        }
    }

    fn transport_event(&mut self, e: TransportEvent) {
        if self.out.transport.push(e).is_err() {
            // Lossless by sizing (C4); if it ever fills, recording stops explicitly instead of losing a cut (U1)
            self.transport.armed = false;
            self.transport.recording = false;
            self.telemetry(Telemetry::TransportEventsOverflow);
        }
    }

    fn report_panic(&mut self, node: Option<NodeIndex>, preview: Option<PreviewId>, payload: Box<dyn Any + Send>) {
        let generation = self.program.generation;
        if let Some(node) = node {
            self.telemetry(Telemetry::NodePanicked { generation, node });
        }
        if let Err(PushError::Full(r)) = self.out.panics.push(PanicReport { generation, node, preview, payload }) {
            // Sized for every node that can panic between two collects; never free on the audio thread
            std::mem::forget(r);
        }
    }

    fn flush_dropped(&mut self) {
        if self.midi_out_lost > 0
            && self.out.events.push(Telemetry::MidiOutDropped { count: self.midi_out_lost }).is_ok()
        {
            self.midi_out_lost = 0;
        }
        if self.map_lost > 0 && self.out.events.push(Telemetry::MapEventsOverflow { count: self.map_lost }).is_ok() {
            self.map_lost = 0;
        }
        const KINDS: [TelemetryKind; 5] = [
            TelemetryKind::Telemetry,
            TelemetryKind::NodeEvents,
            TelemetryKind::ActiveNotes,
            TelemetryKind::LiveMidi,
            TelemetryKind::PluginRequests,
        ];
        for (k, kind) in KINDS.iter().enumerate() {
            let count = self.dropped[k];
            if count > 0 && self.out.events.push(Telemetry::Dropped { kind: *kind, count }).is_ok() {
                self.dropped[k] = 0;
            }
        }
    }

    fn publish(&mut self, host_time: HostTime) {
        let tick = self.program.tempo.tick_at(self.transport.position);
        let f = self.out.frame.input_buffer_mut();
        f.generation = self.program.generation;
        f.position = self.transport.position;
        f.tick = tick;
        f.host_time = host_time;
        f.playing = self.transport.playing;
        f.recording = self.transport.recording;
        f.periods = self.watch.periods.load(Ordering::Relaxed);
        f.frames = self.frame;
        f.xruns = self.xruns;
        f.load = self.load;
        f.flushing_denormals = crate::spec::flushing_denormals();
        f.protection = self.protection.engaged;
        f.counting_in = self.counting.is_some() || self.transport.record_from.is_some();
        for (d, p) in f.inputs.iter_mut().zip(self.input_peaks.iter_mut()) {
            *d = *p;
            *p = 0.0;
        }
        for (m, s) in f.meters.iter_mut().zip(self.slots.iter_mut()) {
            m.generation = s.generation;
            m.channels = s.channels;
            m.peak = s.peak;
            m.dead = s.dead;
            s.peak = [0.0; METER_CHANNELS];
        }
        // Every bound parameter's value as it plays (automation, the hand, MIDI): the session's MIDI feedback reads
        // them (I-21.3), the interface shows automated values
        for b in self.program.bindings.iter() {
            let s = b.slot as usize;
            f.params[s] = self.slot_state[s].current;
        }
        self.out.frame.publish();
        self.since_publish = 0;
        self.load = 0.0;
    }
}

/// A gate change that fell after the last step of the pass (it cannot, but never carry one over a pass)
fn md_gate_reset(m: &mut crate::program::PModulator) {
    if let Some((_, on)) = m.gate.take()
        && m.kind == ModKind::Envelope
    {
        m.envelope.gate(on);
    }
}

fn click(
    state: &mut Option<(bool, usize)>,
    m: &crate::program::Metronome,
    buffer: &mut [f32],
    layout: &BufferLayout,
    from: usize,
    to: usize,
) {
    let Some((accent, pos)) = state.as_mut() else { return };
    let data = if *accent { &m.accent } else { &m.beat };
    let frames = data.frames();
    let k = (to - from).min(frames.saturating_sub(*pos));
    for ch in 0..layout.inputs {
        let src = &data.channel(ch % data.channels())[*pos..*pos + k];
        let o = layout.input_offset(ch) + from;
        for (d, s) in buffer[o..o + k].iter_mut().zip(src) {
            *d += s * m.gain;
        }
    }
    *pos += k;
    if *pos >= frames {
        *state = None;
    }
}

/// Audio items into a node's inputs, with the explicit channel conversions (mono to N at equal power, N to mono
/// averaged, otherwise channel to channel). A streamed item is read into `scratch` first (P-32); its frames not
/// loaded play as silence and are returned as missing
fn play_audio(
    node: &PNode,
    buffer: &mut [f32],
    layout: &BufferLayout,
    pos0: i64,
    n: usize,
    scratch: &mut [f32],
) -> u64 {
    let end = pos0 + n as i64;
    let first = node.audio.partition_point(|it| it.start + node.audio_max_len <= pos0);
    let mut missing = 0u64;
    for it in &node.audio[first..] {
        if it.start >= end {
            break;
        }
        let a = pos0.max(it.start);
        let b = end.min(it.start + it.length);
        if b <= a {
            continue;
        }
        match &it.source {
            AudioSource::Memory(data) => {
                let frames = data.frames() as i64;
                mix_item(it, buffer, layout, pos0, a, b, data.channels(), |ch, f| {
                    if f < 0 || f >= frames { None } else { Some(data.channel(ch)[f as usize]) }
                });
            }
            AudioSource::Stream(stream) => {
                let frame0 = it.offset + (a - it.start);
                let from = frame0.max(0);
                let count = (b - a - (from - frame0)).max(0) as usize;
                let channels = stream.channels();
                if count == 0 || scratch.len() < channels * count {
                    continue;
                }
                missing += stream.read(from as u64, count, scratch) as u64;
                let got = &scratch[..channels * count];
                mix_item(it, buffer, layout, pos0, a, b, channels, |ch, f| {
                    let k = f - from;
                    (k >= 0 && (k as usize) < count).then(|| got[ch * count + k as usize])
                });
            }
        }
    }
    missing
}

/// One item's samples `a..b` (timeline) into the buffer, with its gain and fades; `get(channel, source frame)`
#[allow(clippy::too_many_arguments)]
fn mix_item(
    it: &AudioItem,
    buffer: &mut [f32],
    layout: &BufferLayout,
    pos0: i64,
    a: i64,
    b: i64,
    src_ch: usize,
    get: impl Fn(usize, i64) -> Option<f32>,
) {
    let dst_ch = layout.inputs;
    for k in a..b {
        let in_item = k - it.start;
        let frame = it.offset + in_item;
        if get(0, frame).is_none() {
            continue;
        }
        let mut g = it.gain;
        if it.fade_in > 0 && in_item < i64::from(it.fade_in) {
            g *= in_item as f32 / it.fade_in as f32;
        }
        let to_end = it.length - in_item;
        if it.fade_out > 0 && to_end <= i64::from(it.fade_out) {
            g *= (to_end - 1).max(0) as f32 / it.fade_out as f32;
        }
        let at = (k - pos0) as usize;
        let s = |ch: usize| get(ch, frame).unwrap_or(0.0);
        if src_ch == dst_ch {
            for ch in 0..dst_ch {
                buffer[layout.input_offset(ch) + at] += s(ch) * g;
            }
        } else if src_ch == 1 {
            let x = s(0) * g / (dst_ch as f32).sqrt();
            for ch in 0..dst_ch {
                buffer[layout.input_offset(ch) + at] += x;
            }
        } else if dst_ch == 1 {
            let mut x = 0.0;
            for ch in 0..src_ch {
                x += s(ch);
            }
            buffer[layout.input_offset(0) + at] += x * g / src_ch as f32;
        } else {
            for ch in 0..dst_ch.min(src_ch) {
                buffer[layout.input_offset(ch) + at] += s(ch) * g;
            }
        }
    }
}

/// One feed into a node's inputs: gain, then the PDC delay, then the channel conversion, summed
fn mix_feed(
    pf: &PFeed,
    src: &crate::program::NodeIo,
    dst: &mut crate::program::NodeIo,
    mut delay: Option<&mut DelayLine>,
    gain: &[f32],
    n: usize,
) {
    let f = &pf.feed;
    let channels = usize::from(f.channels);
    let to_base = |ch: usize| {
        let c = usize::from(f.to_channel) + ch;
        if f.sidechain { dst.layout.sidechain_offset(c) } else { dst.layout.input_offset(c) }
    };
    let scale = match f.mix {
        ChannelMix::Direct => 1.0,
        ChannelMix::UpMix { to } => 1.0 / f32::from(to).sqrt(),
        ChannelMix::DownMix => 1.0 / channels as f32,
    };
    for c in 0..channels {
        let so = src.layout.output_offset(usize::from(f.from_channel) + c);
        let s = &src.buffer[so..so + n];
        let targets: (usize, usize) = match f.mix {
            ChannelMix::Direct => (c, c + 1),
            ChannelMix::UpMix { to } => (0, usize::from(to)),
            ChannelMix::DownMix => (0, 1),
        };
        match delay.as_deref_mut() {
            None => {
                for t in targets.0..targets.1 {
                    let o = to_base(t);
                    for ((d, s), g) in dst.buffer[o..o + n].iter_mut().zip(s).zip(gain) {
                        *d += s * g * scale;
                    }
                }
            }
            Some(d) => {
                let len = d.length;
                let base = c * len;
                for k in 0..n {
                    let idx = base + (d.pos + k) % len;
                    let y = d.memory[idx];
                    d.memory[idx] = s[k] * gain[k] * scale;
                    for t in targets.0..targets.1 {
                        dst.buffer[to_base(t) + k] += y;
                    }
                }
            }
        }
    }
    if let Some(d) = delay {
        d.pos = (d.pos + n) % d.length;
    }
}

#[cfg(test)]
mod live_sort_tests {
    use super::*;

    #[test]
    fn live_midi_sorts_by_offset_keeping_arrival_order_for_a_long_burst() {
        // Past the size where the stable sort would allocate
        let n = 1000u32;
        let mut live: Vec<LiveMidi> = (0..n)
            .map(|i| LiveMidi { offset: (n - i) % 7, seq: i, port: PortIndex(0), bytes: [0x90, (i % 128) as u8, 1] })
            .collect();
        sort_live(&mut live);
        assert!(live.windows(2).all(|w| (w[0].offset, w[0].seq) < (w[1].offset, w[1].seq)));
    }
}
