//! Amp & IR (legacy `plugins/AmpAndIr`, the fork's own): the input, through its gain, into an amp model (Neural Amp
//! Modeler, mono: the two sides' mid), then convolved with an impulse response (a guitar cabinet, an acoustic body, a
//! room), then its output gain. Either may be absent.
//!
//! Its state, version 1 (the legacy keys):
//! - parameters `input` and `output` (−24–24 dB, 0): the gains;
//! - fields `ir` (an `audio` resource, or null: the impulse response, decoded at the project's rate; a mono IR serves
//!   both sides, a stereo one each its own; normalized in energy, so that changing it keeps about the same level)
//!   and `nam` (a `nam` resource, or null: the bytes of a `.nam` file, the amp model).
//!
//! Wet only: the legacy W/D knob is the host's dry/wet, which delays the dry signal by the latency the processor
//! reports (the IR's block, 256 frames, when there is an IR; the amp's resampling when its model runs at another
//! rate). The effect works in place on its channel's channels, one amp and one pair of convolvers for each pair (a
//! last odd channel is a pair of itself).
//!
//! The block is cut on a grid of [`GRID`] frames of the engine's frame count and at every parameter event, so the
//! audio is the same for any block size that is a multiple of the grid (V3). A change of IR or model while it plays
//! is a new processor that takes the old one's sound over: the same IR and model go on where they were; another
//! crossfades from the old chain to the new over [`CROSSFADE_MS`].

#![forbid(unsafe_code)]

pub mod amp;
pub mod convolver;
pub mod fft;

use std::any::Any;

use lm_base::{Error, Result};
use lm_dsp::math::dbfs_to_amp;
use lm_plugin::instrument::GRID;
use lm_plugin::{BuildContext, Descriptor, PluginKind, Ports, StateView, TestHints, Velocity};
use lm_render::{Adopted, EventKind, ProcessContext, Processor};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Resource, Value};

use crate::amp::Amp;
use crate::convolver::{BLOCK, Convolver};

pub const ID: &str = "ampandir";
/// The crossfade from the old chain to the new when the IR or the model changes while it plays, ms
pub const CROSSFADE_MS: f32 = 10.0;
const INPUT: usize = 0;
const OUTPUT: usize = 1;

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.ampandir.name",
    kind: PluginKind::Effect,
    schemas,
    ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:ampandir", 1)
            .param(ParamSpec::new("input", -24.0, 24.0, 0.0).unit("dB"))
            .param(ParamSpec::new("output", -24.0, 24.0, 0.0).unit("dB"))
            .field(FieldSpec::new("ir", FieldKind::Resource("audio")).nullable().default(Value::NULL))
            .field(FieldSpec::new("nam", FieldKind::Resource("nam")).nullable().default(Value::NULL)),
    ]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: Vec::new(), sidechains: Vec::new() }
}

/// The IR's two sides, normalized together to unit energy per side (the legacy `loadImpulseResponse`, after its
/// resampling: here the resource is decoded at the project's rate). Keys: `plugin.ampandir.ir_empty`,
/// `plugin.ampandir.ir_silent`
pub fn impulse_sides(channels: &[&[f32]]) -> Result<[Vec<f32>; 2]> {
    let frames = channels.first().map_or(0, |c| c.len());
    if frames == 0 {
        return Err(Error::new("plugin.ampandir.ir_empty"));
    }
    let mut sides: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    let mut energy = 0.0f64;
    for (side, out) in sides.iter_mut().enumerate() {
        let channel = channels[side.min(channels.len() - 1)];
        out.extend_from_slice(&channel[..frames]);
        for &x in out.iter() {
            energy += f64::from(x * x);
        }
    }
    if energy <= 0.0 {
        return Err(Error::new("plugin.ampandir.ir_silent"));
    }
    let scale = (1.0 / (energy / 2.0).sqrt()) as f32;
    for side in &mut sides {
        for x in side.iter_mut() {
            *x *= scale;
        }
    }
    Ok(sides)
}

/// One pair's amp and convolvers
#[derive(Debug, Default)]
struct Chain {
    amp: Option<Amp>,
    ir: Option<Box<[Convolver; 2]>>,
}

impl Chain {
    fn latency(&self) -> u32 {
        self.ir.as_ref().map_or(0, |_| BLOCK as u32) + self.amp.as_ref().map_or(0, Amp::latency)
    }

    /// The legacy `process`, on a piece of a pair
    fn run(&mut self, left: &mut [f32], right: &mut [f32], input: f32, output: f32) {
        if let Some(amp) = self.amp.as_mut() {
            // The amp is mono: the sides' mid, to both sides
            for (l, r) in left.iter_mut().zip(right.iter()) {
                *l = (*l + *r) * 0.5 * input;
            }
            amp.process(left);
            right.copy_from_slice(left);
        } else {
            for x in left.iter_mut().chain(right.iter_mut()) {
                *x *= input;
            }
        }
        if let Some(ir) = self.ir.as_mut() {
            ir[0].process(left);
            ir[1].process(right);
        }
        for x in left.iter_mut().chain(right.iter_mut()) {
            *x *= output;
        }
    }

    fn reset(&mut self) {
        if let Some(amp) = self.amp.as_mut() {
            amp.reset();
        }
        if let Some(ir) = self.ir.as_mut() {
            ir[0].reset();
            ir[1].reset();
        }
    }
}

#[derive(Debug)]
struct Pair {
    chain: Chain,
    /// The chain this one took over from, playing out under a crossfade (kept until the processor is dropped: never
    /// freed on the audio thread)
    fading: Chain,
}

/// The processor
#[derive(Debug)]
pub struct AmpAndIr {
    pairs: Vec<Pair>,
    channels: usize,
    /// The resources this was built with: the same ones are taken over as they are, other ones crossfaded
    ir_key: Option<Resource>,
    nam_key: Option<Resource>,
    /// The gains (amplitude) and their knobs (dB)
    knobs: [f32; 2],
    gains: [f32; 2],
    /// Frames of the crossfade left, and its length
    fade_left: usize,
    fade_frames: usize,
    piece: [[f32; GRID]; 2],
    old: [[f32; GRID]; 2],
}

impl AmpAndIr {
    fn set(&mut self, index: usize, value: f32) {
        if let Some(knob) = self.knobs.get_mut(index) {
            *knob = value;
            self.gains[index] = dbfs_to_amp(value);
        }
    }

    fn run_piece(&mut self, cx: &mut ProcessContext<'_>, at: usize, n: usize) {
        let channels = self.channels;
        let [input, output] = self.gains;
        for (p, pair) in self.pairs.iter_mut().enumerate() {
            let (l, r) = (2 * p, (2 * p + 1).min(channels - 1));
            let [left, right] = &mut self.piece;
            let (left, right) = (&mut left[..n], &mut right[..n]);
            left.copy_from_slice(&cx.buffers.in_out(l)[at..at + n]);
            right.copy_from_slice(&cx.buffers.in_out(r)[at..at + n]);
            let fading = self.fade_left > 0;
            if fading {
                self.old[0][..n].copy_from_slice(left);
                self.old[1][..n].copy_from_slice(right);
            }
            pair.chain.run(left, right, input, output);
            if fading {
                let [ol, or] = &mut self.old;
                let (ol, or) = (&mut ol[..n], &mut or[..n]);
                pair.fading.run(ol, or, input, output);
                let total = self.fade_frames as f32;
                for i in 0..n {
                    let new = (self.fade_frames - self.fade_left.saturating_sub(i)) as f32 / total;
                    left[i] = left[i] * new + ol[i] * (1.0 - new);
                    right[i] = right[i] * new + or[i] * (1.0 - new);
                }
            }
            cx.buffers.in_out(l)[at..at + n].copy_from_slice(left);
            if r != l {
                cx.buffers.in_out(r)[at..at + n].copy_from_slice(right);
            }
        }
        self.fade_left = self.fade_left.saturating_sub(n);
    }
}

impl Processor for AmpAndIr {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let events = cx.events;
        let origin = cx.transport.frame;
        let mut e = 0;
        let mut at = 0;
        while at < frames {
            while e < events.len() && (events[e].offset as usize) <= at {
                self.event(events[e].kind);
                e += 1;
            }
            let grid = GRID - ((origin + at as u64) % GRID as u64) as usize;
            let mut end = (at + grid).min(frames);
            if e < events.len() {
                end = end.min(events[e].offset as usize).max(at + 1);
            }
            self.run_piece(cx, at, end - at);
            at = end;
        }
        for ev in &events[e..] {
            self.event(ev.kind);
        }
    }

    fn reset(&mut self) {
        for pair in &mut self.pairs {
            pair.chain.reset();
        }
        self.fade_left = 0;
    }

    fn latency(&self) -> u32 {
        self.pairs.first().map_or(0, |p| p.chain.latency())
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if old.pairs.len() != self.pairs.len() {
            return Adopted::No;
        }
        if old.ir_key == self.ir_key && old.nam_key == self.nam_key {
            // The same chain: it goes on where it was (and so does a crossfade it was in)
            for (new, old) in self.pairs.iter_mut().zip(old.pairs.iter_mut()) {
                std::mem::swap(&mut new.chain, &mut old.chain);
                std::mem::swap(&mut new.fading, &mut old.fading);
            }
            self.fade_left = old.fade_left;
            self.fade_frames = old.fade_frames;
        } else {
            // Another IR or model: the old chain plays out under a crossfade to the new one
            for (new, old) in self.pairs.iter_mut().zip(old.pairs.iter_mut()) {
                std::mem::swap(&mut new.fading, &mut old.chain);
            }
            self.fade_left = self.fade_frames;
        }
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl AmpAndIr {
    fn event(&mut self, kind: EventKind) {
        match kind {
            EventKind::Param { index, value } => self.set(index as usize, value),
            EventKind::AllSoundOff => Processor::reset(self),
            _ => {}
        }
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let rate = f64::from(cx.spec.sample_rate);
    let knobs = [s.param_f32("input")?, s.param_f32("output")?];
    let ir_key = s.resource("ir")?.cloned();
    let nam_key = s.resource("nam")?.cloned();
    let sides = match &ir_key {
        Some(r) => {
            let data = cx.resources.audio(r)?;
            let channels: Vec<&[f32]> = (0..data.channels()).map(|c| data.channel(c)).collect();
            Some(impulse_sides(&channels)?)
        }
        None => None,
    };
    let model = match &nam_key {
        Some(r) => Some(cx.resources.bytes(r)?),
        None => None,
    };
    let channels = usize::from(cx.channels).max(1);
    let mut pairs = Vec::new();
    for _ in 0..channels.div_ceil(2) {
        let amp = match &model {
            Some(bytes) => Some(Amp::new(bytes, rate)?),
            None => None,
        };
        let ir = sides.as_ref().map(|[l, r]| Box::new([Convolver::new(l), Convolver::new(r)]));
        pairs.push(Pair { chain: Chain { amp, ir }, fading: Chain::default() });
    }
    let fade_frames = ((CROSSFADE_MS / 1000.0) * cx.spec.sample_rate as f32).round().max(1.0) as usize;
    let mut processor = AmpAndIr {
        pairs,
        channels,
        ir_key,
        nam_key,
        knobs,
        gains: [1.0; 2],
        fade_left: 0,
        fade_frames,
        piece: [[0.0; GRID]; 2],
        old: [[0.0; GRID]; 2],
    };
    processor.set(INPUT, knobs[INPUT]);
    processor.set(OUTPUT, knobs[OUTPUT]);
    Ok(Box::new(processor))
}
