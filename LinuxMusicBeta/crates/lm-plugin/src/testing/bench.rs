//! A host for one processor, outside the renderer: it lays the node's block out as the renderer does, sorts each
//! block's events in the renderer's order (offset, then [`EventKind::rank`], then arrival: U26), keeps the node's
//! random generator across a replacement (as the renderer keeps the node), and runs every `process` and `adopt`
//! under the allocation trap when asked.

use lm_render::{
    Adopted, BlockShape, Buffers, Event, EventKind, HostOutbox, ProcessContext, ProcessSpec, Processor, Rng,
    TransportInfo,
};
use lm_time::TimeSignature;

/// An event at an absolute sample of the bench's time line
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timed {
    pub at: u64,
    pub kind: EventKind,
}

impl Timed {
    pub fn new(at: u64, kind: EventKind) -> Self {
        Self { at, kind }
    }
}

/// The input an effect hears: a deterministic test signal, per absolute sample and channel
pub type InputFn = fn(u64, usize, u32) -> f32;

/// One processor, driven block by block
pub struct Bench {
    processor: Box<dyn Processor>,
    spec: ProcessSpec,
    shape: BlockShape,
    data: Vec<f32>,
    keyed: Vec<(u64, Event)>,
    events: Vec<Event>,
    rng: Rng,
    outbox: HostOutbox,
    frame: u64,
    bpm: f64,
    ppq: u32,
    input: Option<InputFn>,
    /// The keys of the notes played, mapped (an instrument that sounds on some keys only: a drum kit)
    key_map: Option<fn(u8) -> u8>,
}

impl std::fmt::Debug for Bench {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bench").field("spec", &self.spec).field("frame", &self.frame).finish()
    }
}

/// The trap of `assert_no_alloc` around `f`, or `f` alone
pub(crate) fn trapped<T>(trap: bool, f: impl FnOnce() -> T) -> T {
    if trap { assert_no_alloc::assert_no_alloc(f) } else { f() }
}

impl Bench {
    /// Every note's key mapped by `f` (see [`super::Setup::key_map`])
    #[must_use]
    pub fn with_key_map(mut self, f: Option<fn(u8) -> u8>) -> Self {
        self.key_map = f;
        self
    }

    /// An instrument with `outputs` output channels
    pub fn instrument(processor: Box<dyn Processor>, spec: ProcessSpec, outputs: usize, seed: u64) -> Self {
        Self::new(processor, spec, BlockShape { inputs: 0, outputs, sidechain: 0, in_place: false }, seed, None)
    }

    /// An effect working in place on `channels` channels, hearing `input`
    pub fn effect(
        processor: Box<dyn Processor>,
        spec: ProcessSpec,
        channels: usize,
        seed: u64,
        input: InputFn,
    ) -> Self {
        let shape = BlockShape { inputs: channels, outputs: channels, sidechain: 0, in_place: true };
        Self::new(processor, spec, shape, seed, Some(input))
    }

    fn new(
        processor: Box<dyn Processor>,
        spec: ProcessSpec,
        shape: BlockShape,
        seed: u64,
        input: Option<InputFn>,
    ) -> Self {
        Self {
            processor,
            spec,
            shape,
            data: vec![0.0; shape.len(spec.max_block)],
            keyed: Vec::with_capacity(8192),
            events: Vec::with_capacity(8192),
            rng: Rng::new(seed),
            outbox: HostOutbox::default(),
            frame: 0,
            bpm: 120.0,
            ppq: 960,
            input,
            key_map: None,
        }
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn outputs(&self) -> usize {
        self.shape.outputs
    }

    pub fn processor_mut(&mut self) -> &mut dyn Processor {
        self.processor.as_mut()
    }

    /// Replaces the processor as an install's `replace` does: the new one adopts the old one (under the trap when
    /// asked); the old one comes back
    pub fn replace(&mut self, mut new: Box<dyn Processor>, trap: bool) -> (Adopted, Box<dyn Processor>) {
        let old = &mut self.processor;
        let adopted = trapped(trap, || new.adopt(old.as_mut()));
        let old = std::mem::replace(&mut self.processor, new);
        (adopted, old)
    }

    /// Renders `total` frames in blocks of `block` (at most the spec's), with the events of `events` that fall in
    /// them (the rest are left for a later call), handing each block's output channels to `sink`
    pub fn run(
        &mut self,
        total: usize,
        block: usize,
        events: &[Timed],
        trap: bool,
        sink: &mut dyn FnMut(u64, &[&[f32]]),
    ) {
        let block = block.clamp(1, self.spec.max_block);
        let stride = self.spec.max_block;
        let mut done = 0;
        while done < total {
            let n = block.min(total - done);
            let start = self.frame;
            // The block's events, in the renderer's order
            self.keyed.clear();
            for (i, t) in events.iter().enumerate() {
                if t.at >= start && t.at < start + n as u64 {
                    let offset = (t.at - start) as u32;
                    let kind = match (self.key_map, t.kind) {
                        (Some(f), EventKind::NoteOn { note, channel, key, velocity }) => {
                            EventKind::NoteOn { note, channel, key: f(key), velocity }
                        }
                        (Some(f), EventKind::NoteOff { note, channel, key, velocity }) => {
                            EventKind::NoteOff { note, channel, key: f(key), velocity }
                        }
                        (_, k) => k,
                    };
                    let key = (u64::from(offset) << 32) | (u64::from(t.kind.rank()) << 28) | (i as u64 & 0x0FFF_FFFF);
                    self.keyed.push((key, Event { offset, kind }));
                }
            }
            self.keyed.sort_unstable_by_key(|(k, _)| *k);
            self.events.clear();
            self.events.extend(self.keyed.iter().map(|(_, e)| *e));
            // The outputs zeroed, or holding the input (U23)
            self.data.fill(0.0);
            if let Some(input) = self.input {
                for ch in 0..self.shape.inputs {
                    let o = self.shape.input_offset(ch, stride);
                    for k in 0..n {
                        self.data[o + k] = input(start + k as u64, ch, self.spec.sample_rate);
                    }
                }
            }
            let position = start as i64;
            let ticks_per_sample = self.bpm * f64::from(self.ppq) / (60.0 * f64::from(self.spec.sample_rate));
            let transport = TransportInfo {
                sample_rate: self.spec.sample_rate,
                playing: true,
                recording: false,
                position,
                tick: (position as f64 * ticks_per_sample).floor() as i64,
                ppq: self.ppq,
                bpm: self.bpm,
                time_signature: TimeSignature::COMMON,
                frame: start,
            };
            {
                let Self { processor, data, events, rng, outbox, shape, .. } = self;
                let buffers = Buffers::from_block(data, stride, n, *shape);
                let mut cx = ProcessContext {
                    frames: n,
                    buffers,
                    events,
                    transport: &transport,
                    rng,
                    host: outbox,
                    display: None,
                };
                trapped(trap, || processor.process(&mut cx));
            }
            let mut refs: Vec<&[f32]> = Vec::with_capacity(self.shape.outputs);
            for ch in 0..self.shape.outputs {
                let o = self.shape.output_offset(ch, stride);
                refs.push(&self.data[o..o + n]);
            }
            sink(start, &refs);
            self.frame += n as u64;
            done += n;
        }
    }

    /// [`Bench::run`], keeping every output channel whole
    pub fn render(&mut self, total: usize, block: usize, events: &[Timed], trap: bool) -> Vec<Vec<f32>> {
        let mut out = vec![Vec::with_capacity(total); self.shape.outputs];
        self.run(total, block, events, trap, &mut |_, chans| {
            for (o, c) in out.iter_mut().zip(chans) {
                o.extend_from_slice(c);
            }
        });
        out
    }
}
