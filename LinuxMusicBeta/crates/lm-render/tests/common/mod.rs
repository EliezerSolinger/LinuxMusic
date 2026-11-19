//! Test processors, programs and helpers shared by the integration tests
#![allow(dead_code)]

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use lm_render::*;
use lm_time::{Bpm, Ppq, TempoMap, TimeSignature};

pub const RATE: u32 = 48_000;
pub const MAX_BLOCK: usize = 1024;

pub fn spec() -> ProcessSpec {
    ProcessSpec { sample_rate: RATE, max_block: MAX_BLOCK }
}

pub fn limits(workers: usize) -> Limits {
    Limits { workers, max_nodes: 512, max_params: 1024, ..Limits::default() }
}

pub fn tempo(bpm: u32) -> TempoMap {
    TempoMap::constant(RATE, Ppq::DEFAULT, Bpm::from_integer(bpm).expect("bpm"), TimeSignature::COMMON).expect("map")
}

/// Render threads whose whole life runs under the allocation trap
pub fn checked_threads() -> RenderThreads {
    RenderThreads::new(Arc::new(|_, body: &mut dyn FnMut()| assert_no_alloc::assert_no_alloc(body)))
}

pub fn make(workers: usize) -> (EngineControl, Dummy) {
    install_panic_hook();
    let (control, renderer) = engine_with(spec(), limits(workers), checked_threads());
    (control, Dummy::new(renderer, 2, 2))
}

/// One block under the allocation trap
pub fn block(d: &mut Dummy, n: usize) {
    assert_no_alloc::assert_no_alloc(|| {
        d.block(n);
    });
}

/// `total` frames in blocks of `size`, under the trap, appending the outputs
pub fn render(d: &mut Dummy, total: usize, size: usize) -> Vec<Vec<f32>> {
    let channels = d.outputs().len();
    let mut out = vec![Vec::with_capacity(total); channels];
    let mut done = 0;
    while done < total {
        let n = size.min(total - done);
        block(d, n);
        for (o, c) in out.iter_mut().zip(d.outputs()) {
            o.extend_from_slice(&c[..n]);
        }
        done += n;
    }
    out
}

pub fn install(control: &mut EngineControl, install: Install) {
    control.install(install).expect("install");
}

pub fn collect(control: &mut EngineControl) -> Vec<Returned> {
    let mut v = Vec::new();
    control.collect(|r| v.push(r));
    v
}

pub fn core(p: impl Processor + 'static) -> Box<NodeCore> {
    NodeCore::new(Box::new(p), 1, None)
}

pub fn n(index: u32) -> NodeIndex {
    NodeIndex::new(index, 1)
}

// -------------------------------------------------------------------------------------------------------------
// Processors

/// Walks the events in order: `step(Some(event), 0, 0)` for each event, `step(None, from, to)` for each stretch
/// of samples between them
pub fn segments(events: &[Event], frames: usize, mut step: impl FnMut(Option<&Event>, usize, usize)) {
    let mut at = 0;
    for e in events {
        let o = (e.offset as usize).min(frames);
        if o > at {
            step(None, at, o);
            at = o;
        }
        step(Some(e), 0, 0);
    }
    if at < frames {
        step(None, at, frames);
    }
}

/// A sine on every output; parameter 0 is its gain, applied at the event's exact sample
pub struct Sine {
    pub freq: f64,
    pub phase: f64,
    pub gain: f32,
}

impl Sine {
    pub fn new(freq: f64, gain: f32) -> Self {
        Self { freq, phase: 0.0, gain }
    }
}

impl Processor for Sine {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let step = self.freq / f64::from(cx.transport.sample_rate);
        let frames = cx.frames;
        let outputs = cx.buffers.outputs();
        let mut phase = self.phase;
        let mut gain = self.gain;
        let (_, mut outs, _) = cx.buffers.split();
        segments(cx.events, frames, |e, a, b| {
            if let Some(e) = e {
                if let EventKind::Param { index: 0, value } = e.kind {
                    gain = value;
                }
            } else {
                for k in a..b {
                    let v = (libm::sin(phase * std::f64::consts::TAU) as f32) * gain;
                    for ch in 0..outputs {
                        outs.get_mut(ch)[k] = v;
                    }
                    phase = (phase + step).fract();
                }
            }
        });
        self.phase = phase;
        self.gain = gain;
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// White noise from the node's generator (deterministic per seed)
pub struct Noise(pub f32);

impl Processor for Noise {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        for ch in 0..cx.buffers.outputs() {
            for x in cx.buffers.output(ch).iter_mut() {
                *x = cx.rng.bipolar() * self.0;
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// In place: multiplies by parameter 0
pub struct Gain(pub f32);

impl Processor for Gain {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let mut g = self.0;
        let channels = cx.buffers.outputs();
        let (_, mut io, _) = cx.buffers.split();
        segments(cx.events, frames, |e, a, b| {
            if let Some(e) = e {
                if let EventKind::Param { index: 0, value } = e.kind {
                    g = value;
                }
            } else {
                for ch in 0..channels {
                    for x in &mut io.get_mut(ch)[a..b] {
                        *x *= g;
                    }
                }
            }
        });
        self.0 = g;
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// In place: a delay of `latency` samples (reports it), the memory allocated here
pub struct Latency {
    pub latency: usize,
    memory: Vec<Vec<f32>>,
    pos: usize,
}

impl Latency {
    pub fn new(latency: usize, channels: usize) -> Self {
        Self { latency, memory: vec![vec![0.0; latency.max(1)]; channels], pos: 0 }
    }
}

impl Processor for Latency {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let len = self.latency.max(1);
        let mut pos = self.pos;
        for ch in 0..cx.buffers.outputs() {
            pos = self.pos;
            let mem = &mut self.memory[ch];
            for x in cx.buffers.in_out(ch).iter_mut() {
                std::mem::swap(&mut mem[pos], x);
                pos = (pos + 1) % len;
            }
        }
        self.pos = pos;
    }

    fn latency(&self) -> u32 {
        self.latency as u32
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// 1.0 on every output at engine frame `at`
pub struct Impulse {
    pub at: u64,
}

impl Processor for Impulse {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let f0 = cx.transport.frame;
        if self.at >= f0 && self.at < f0 + cx.frames as u64 {
            let k = (self.at - f0) as usize;
            for ch in 0..cx.buffers.outputs() {
                cx.buffers.output(ch)[k] = 1.0;
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Writes the timeline position of each sample (to check loops and locates), scaled down
pub struct Position;

impl Processor for Position {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let p = cx.transport.position;
        for ch in 0..cx.buffers.outputs() {
            for (k, x) in cx.buffers.output(ch).iter_mut().enumerate() {
                *x = (p + k as i64) as f32;
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Outputs a constant, then NaN from call `bad_from` on
pub struct GoesNan {
    pub calls: u32,
    pub bad_from: u32,
}

impl Processor for GoesNan {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        self.calls += 1;
        let v = if self.calls >= self.bad_from { f32::NAN } else { 0.5 };
        for ch in 0..cx.buffers.outputs() {
            cx.buffers.output(ch).fill(v);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Outputs a constant, then panics on call `panic_at`
pub struct Panics {
    pub calls: u32,
    pub panic_at: u32,
}

impl Processor for Panics {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        self.calls += 1;
        assert!(self.calls < self.panic_at, "test processor panics on purpose");
        for ch in 0..cx.buffers.outputs() {
            cx.buffers.output(ch).fill(0.25);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Outputs `value` on every sample
pub struct Constant(pub f32);

impl Processor for Constant {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        for ch in 0..cx.buffers.outputs() {
            cx.buffers.output(ch).fill(self.0);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Counts notes; `open` is the number of notes it has heard start and not end (shared with the test)
pub struct NoteCounter {
    pub open: Arc<AtomicI64>,
    pub total: Arc<AtomicU64>,
    notes: Vec<NoteId>,
    pub adopts: bool,
}

impl NoteCounter {
    pub fn new(open: Arc<AtomicI64>, total: Arc<AtomicU64>, adopts: bool) -> Self {
        Self { open, total, notes: Vec::with_capacity(4096), adopts }
    }

    fn set_open(&self) {
        self.open.store(self.notes.len() as i64, Ordering::Relaxed);
    }
}

impl Processor for NoteCounter {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let outputs = cx.buffers.outputs();
        let (_, mut outs, _) = cx.buffers.split();
        let notes = &mut self.notes;
        let total = &self.total;
        segments(cx.events, frames, |e, a, b| match e.map(|e| e.kind) {
            Some(kind) => match kind {
                EventKind::NoteOn { note, .. } => {
                    assert!(!notes.contains(&note), "the same note started twice");
                    if notes.len() < notes.capacity() {
                        notes.push(note);
                    }
                    total.fetch_add(1, Ordering::Relaxed);
                }
                EventKind::NoteOff { note, .. } => {
                    if let Some(i) = notes.iter().position(|n| *n == note) {
                        notes.swap_remove(i);
                    }
                }
                EventKind::AllNotesRelease | EventKind::AllSoundOff => notes.clear(),
                _ => {}
            },
            None => {
                let v = notes.len() as f32 * 0.01;
                for ch in 0..outputs {
                    outs.get_mut(ch)[a..b].fill(v);
                }
            }
        });
        self.set_open();
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        if !self.adopts {
            return Adopted::No;
        }
        match old.as_any_mut().downcast_mut::<NoteCounter>() {
            Some(o) => {
                self.notes.clear();
                self.notes.append(&mut o.notes);
                o.set_open();
                self.set_open();
                Adopted::Yes
            }
            None => Adopted::No,
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------------------
// Programs

/// One track: instrument (stereo, with the track's strip) → a desk channel (sum with gain and pan) → master
pub struct Desk {
    pub master: NodeIndex,
}

/// Master sum at index 0, stereo, to device outputs 0 and 1
pub fn master(b: &mut ProgramBuilder) -> NodeIndex {
    let m = n(0);
    b.node(NodeDesc::sum(m, 2));
    b.master(m);
    b.output(OutputRoute { node: m, channel: 0, device_channel: 0 });
    b.output(OutputRoute { node: m, channel: 1, device_channel: 1 });
    m
}

/// `tracks` stereo sources into their own desk channel each, every eighth with a send to a shared bus; returns
/// the processors to add (index, core)
pub fn big_program(
    generation: u64,
    tracks: usize,
    seed_noise: bool,
) -> (Box<Program>, Vec<(NodeIndex, Box<NodeCore>)>) {
    let mut b = ProgramBuilder::new(ProgramGen(generation), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    let bus = n(1);
    let bus_fx = n(2);
    b.node(NodeDesc::sum(bus, 2));
    b.node(NodeDesc::insert(bus_fx, 2).with_strip(Strip { gain: HostParam::fixed(0.5), pan: None }));
    b.feed(Feed::new(bus, bus_fx, 2));
    b.feed(Feed::new(bus_fx, m, 2));
    let mut cores: Vec<(NodeIndex, Box<NodeCore>)> = vec![(bus_fx, core(Gain(0.8)))];
    for t in 0..tracks {
        let inst = n(10 + 3 * t as u32);
        let chan = n(11 + 3 * t as u32);
        let strip = n(12 + 3 * t as u32);
        b.node(NodeDesc::processor(inst, 0, 2).with_strip(Strip { gain: HostParam::fixed(0.7), pan: None }));
        b.node(NodeDesc::sum(chan, 2));
        b.node(NodeDesc::insert(strip, 2).with_strip(Strip {
            gain: HostParam::fixed(0.9),
            pan: Some(HostParam::fixed((t as f32 / tracks as f32) * 2.0 - 1.0)),
        }));
        b.feed(Feed::new(inst, chan, 2));
        b.feed(Feed::new(chan, strip, 2));
        b.feed(Feed::new(strip, m, 2));
        if t % 8 == 0 {
            b.feed(Feed::new(strip, bus, 2).gain(HostParam::fixed(0.3)));
        }
        let p: Box<NodeCore> = if seed_noise && t % 2 == 1 {
            NodeCore::new(Box::new(Noise(0.1)), Rng::seed_for(271_828, t as u64), None)
        } else {
            core(Sine::new(110.0 * (1.0 + t as f64 * 0.25), 0.1))
        };
        cores.push((inst, p));
        cores.push((strip, core(Gain(1.0))));
    }
    (b.build().expect("program"), cores)
}

pub fn install_new(control: &mut EngineControl, program: Box<Program>, cores: Vec<(NodeIndex, Box<NodeCore>)>) {
    let mut i = Install::new(program);
    for (node, c) in cores {
        i.add(node, c);
    }
    install(control, i);
}
