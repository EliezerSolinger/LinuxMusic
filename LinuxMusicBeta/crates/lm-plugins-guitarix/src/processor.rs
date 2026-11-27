//! The processor of a Guitarix unit.
//!
//! - **Instances:** for each pair of the channel's channels ([`BuildContext::channels`]), one instance of a stereo
//!   unit, or one of a mono unit for each channel (as the legacy plugin: two for its two channels). A last odd
//!   channel is played as a pair of itself (a stereo unit gets it on both sides; its left is kept).
//! - **The unit's rate is always 48 kHz** ([`UNIT_RATE`]). Guitarix's DSP sounds different at each rate (filters,
//!   delays and nonlinearities tuned in samples here and there: a reverb's level moved 1.6 dB from 48 to 96 kHz, a
//!   fuzz 37 dB at 192 kHz), which a project must never hear when its rate changes (U36). At 48 kHz the unit runs
//!   directly, as the legacy plugin ran it (the A/B is there); at any other rate the pair's signal is converted to 48
//!   kHz and back around it (libsamplerate, medium sinc, time-aligned), with a fixed latency ([`Processor::latency`]).
//! - **Calls of fixed sizes:** at 48 kHz the block is cut on a grid of [`GRID`] samples of the engine's frame count
//!   and at every event, as the other effects do, so every unit gets calls of the same sizes for any engine block
//!   (V3). The units whose sound depends on the size of each call ([`crate::fixed_block`]: the convolving units, the
//!   amp head, the Detune) run in blocks of that size through a FIFO, a latency of one block. Converted, every unit
//!   runs in blocks of its size (32 frames for the others) as the 48 kHz signal comes.
//! - **Parameters** take effect at the event's sample (at the start of the block that holds it, for a unit in
//!   blocks). Logarithmic ones (frequencies, bandwidths) glide there in the log domain with a time constant of
//!   [`GLIDE_MS`]: Guitarix's crossovers and peak filters, in direct form, released their state as a burst of up to
//!   +56 dB when one jumped (a crossover from 20 Hz to 20 kHz).
//! - **Fields** (selectors, and the settings read only at the start, [`crate::rebuilds`]): a new processor is built
//!   with the new value, and takes over the old one's instances only to crossfade from them over
//!   [`lm_plugin::instrument::CROSSFADE_MS`] (a module switched in Guitarix is cleared: a click; a cabinet's new
//!   response starts fresh). The same tree rebuilt takes the old instances over and goes on bit for bit.
//! - **Guard:** a non-finite sample clears the unit (its state would stay poisoned) and plays silence for that call.

use std::any::Any;

use lm_base::{Error, Result};
use lm_plugin::instrument::{CROSSFADE_MS, GRID};
use lm_plugin::{BuildContext, StateView};
use lm_render::{Adopted, EventKind, ProcessContext, Processor};
use lm_state::{Scale, Value};
use lm_sys_guitarix::{Instance, MAX_BLOCK};
use lm_sys_samplerate::{Quality, Resampler};

use crate::schema::{DISPLAY_BYTES, Plan, plan};

/// The rate every unit runs at
pub const UNIT_RATE: u32 = 48_000;
/// The glide of a logarithmic parameter: its time constant
pub const GLIDE_MS: f64 = 5.0;
/// A unit's calls when nothing else asks for a size
const UNIT_BLOCK: usize = GRID;

/// A FIFO of a fixed block for a pair of channels: what comes in waits for the block to fill, what goes out is the
/// last block's output (a latency of one block)
struct Fifo {
    block: usize,
    input: [Vec<f32>; 2],
    output: [Vec<f32>; 2],
    pos: usize,
}

impl Fifo {
    fn new(block: usize) -> Self {
        Self {
            block,
            input: [vec![0.0; block], vec![0.0; block]],
            output: [vec![0.0; block], vec![0.0; block]],
            pos: 0,
        }
    }

    fn clear(&mut self) {
        for b in self.input.iter_mut().chain(self.output.iter_mut()) {
            b.fill(0.0);
        }
        self.pos = 0;
    }
}

/// The conversion to the unit's rate and back, for a pair of channels
struct Converter {
    up: Resampler,
    down: Resampler,
    /// The unit's block
    block: usize,
    /// The engine's piece, interleaved
    staged: Vec<f32>,
    /// The signal at the unit's rate waiting for a block, interleaved
    pending: Vec<f32>,
    pending_frames: usize,
    /// The unit's block in and out, by channel
    input: [Vec<f32>; 2],
    output: [Vec<f32>; 2],
    interleaved: Vec<f32>,
    /// What the down converter gave in one call
    converted: Vec<f32>,
    /// The output at the engine's rate, a ring of interleaved frames
    ring: Vec<f32>,
    read: usize,
    filled: usize,
    /// The latency: frames of silence the ring starts with
    latency: usize,
}

impl Converter {
    fn new(rate: u32, block: usize) -> Result<Self> {
        let up_ratio = f64::from(UNIT_RATE) / f64::from(rate);
        let up = Resampler::new(Quality::Medium, 2, up_ratio)?;
        let down = Resampler::new(Quality::Medium, 2, 1.0 / up_ratio)?;
        // The most the engine waits for: the up converter's delay, a block at the unit's rate, the down converter's
        // delay, and a little for the rounding of where the frames fall
        let latency = (up.latency() + (block as f64 + down.latency()) / up_ratio).ceil() as usize + 4;
        let pending = ((GRID as f64 * up_ratio).ceil() as usize + block + 64) * 2;
        let out_block = (block as f64 / up_ratio).ceil() as usize + 64;
        let ring = (latency + GRID + out_block + 64) * 2;
        let mut c = Self {
            up,
            down,
            block,
            staged: vec![0.0; GRID * 2],
            pending: vec![0.0; pending],
            pending_frames: 0,
            input: [vec![0.0; block], vec![0.0; block]],
            output: [vec![0.0; block], vec![0.0; block]],
            interleaved: vec![0.0; block * 2],
            converted: vec![0.0; out_block * 2],
            ring: vec![0.0; ring],
            read: 0,
            filled: 0,
            latency,
        };
        c.clear();
        Ok(c)
    }

    fn clear(&mut self) {
        let _ = self.up.reset();
        let _ = self.down.reset();
        self.pending_frames = 0;
        self.ring.fill(0.0);
        self.read = 0;
        self.filled = self.latency;
    }

    /// `n` frames (at most [`GRID`]) in from `input`, `n` frames out into `output`
    fn process(
        &mut self,
        instances: &mut [Instance],
        input: &[Vec<f32>; 2],
        output: &mut [Vec<f32>; 2],
        n: usize,
        poisoned: &mut bool,
    ) {
        for (frame, (l, r)) in
            self.staged.as_chunks_mut::<2>().0.iter_mut().zip(input[0][..n].iter().zip(&input[1][..n]))
        {
            frame[0] = *l;
            frame[1] = *r;
        }
        let mut used = 0;
        while used < n {
            let room = self.pending.len() / 2 - self.pending_frames;
            let at = 2 * self.pending_frames;
            let Ok(done) = self.up.process(&self.staged[2 * used..2 * n], &mut self.pending[at..at + 2 * room]) else {
                break;
            };
            used += done.input_frames;
            self.pending_frames += done.output_frames;
            while self.pending_frames >= self.block {
                self.unit_block(instances, poisoned);
            }
            if done.input_frames == 0 && done.output_frames == 0 {
                break;
            }
        }
        let frames = self.ring.len() / 2;
        let [out0, out1] = output;
        for (l, r) in out0[..n].iter_mut().zip(&mut out1[..n]) {
            if self.filled == 0 {
                // (never with the latency worked out above: silence rather than a stale frame)
                (*l, *r) = (0.0, 0.0);
                continue;
            }
            (*l, *r) = (self.ring[2 * self.read], self.ring[2 * self.read + 1]);
            self.read = (self.read + 1) % frames;
            self.filled -= 1;
        }
    }

    /// A block of the unit, from the pending signal into the ring
    fn unit_block(&mut self, instances: &mut [Instance], poisoned: &mut bool) {
        let b = self.block;
        for f in 0..b {
            self.input[0][f] = self.pending[2 * f];
            self.input[1][f] = self.pending[2 * f + 1];
        }
        self.pending.copy_within(2 * b..2 * self.pending_frames, 0);
        self.pending_frames -= b;
        run(instances, &mut self.input, &mut self.output, b, poisoned);
        for f in 0..b {
            self.interleaved[2 * f] = self.output[0][f];
            self.interleaved[2 * f + 1] = self.output[1][f];
        }
        let mut consumed = 0;
        while consumed < b {
            let Ok(d) = self.down.process(&self.interleaved[2 * consumed..2 * b], &mut self.converted) else { break };
            consumed += d.input_frames;
            let frames = self.ring.len() / 2;
            for f in 0..d.output_frames {
                if self.filled == frames {
                    break;
                }
                let at = (self.read + self.filled) % frames;
                self.ring[2 * at] = self.converted[2 * f];
                self.ring[2 * at + 1] = self.converted[2 * f + 1];
                self.filled += 1;
            }
            if d.input_frames == 0 && d.output_frames == 0 {
                break;
            }
        }
    }
}

/// How a pair of channels reaches its instances
enum Path {
    /// At 48 kHz, each piece as it comes
    Direct,
    /// At 48 kHz, in blocks of a fixed size
    Fixed(Fifo),
    /// At another rate, converted
    Converted(Box<Converter>),
}

/// The instances of a pair of channels
struct Group {
    left: usize,
    right: usize,
    instances: Vec<Instance>,
    path: Path,
}

impl Group {
    fn latency(&self) -> u32 {
        match &self.path {
            Path::Direct => 0,
            Path::Fixed(f) => f.block as u32,
            Path::Converted(c) => c.latency as u32,
        }
    }

    fn clear(&mut self) {
        for i in &mut self.instances {
            i.clear();
        }
        match &mut self.path {
            Path::Direct => {}
            Path::Fixed(f) => f.clear(),
            Path::Converted(c) => c.clear(),
        }
    }

    /// `n` frames of `input` through the instances into `output`
    fn process(&mut self, input: &mut [Vec<f32>; 2], output: &mut [Vec<f32>; 2], n: usize) {
        let mut poisoned = false;
        match &mut self.path {
            Path::Direct => run(&mut self.instances, input, output, n, &mut poisoned),
            Path::Fixed(fifo) => {
                let mut k = 0;
                while k < n {
                    let m = (n - k).min(fifo.block - fifo.pos);
                    let (p, b) = (fifo.pos, fifo.block);
                    for c in 0..2 {
                        fifo.input[c][p..p + m].copy_from_slice(&input[c][k..k + m]);
                        output[c][k..k + m].copy_from_slice(&fifo.output[c][p..p + m]);
                    }
                    fifo.pos += m;
                    k += m;
                    if fifo.pos == b {
                        run(&mut self.instances, &mut fifo.input, &mut fifo.output, b, &mut poisoned);
                        fifo.pos = 0;
                    }
                }
            }
            Path::Converted(c) => c.process(&mut self.instances, input, output, n, &mut poisoned),
        }
        if poisoned {
            for i in &mut self.instances {
                i.clear();
            }
        }
    }
}

/// Runs the instances on `n` frames of `input` into `output`, then the guard (`poisoned`: a sample was not finite)
fn run(
    instances: &mut [Instance],
    input: &mut [Vec<f32>; 2],
    output: &mut [Vec<f32>; 2],
    n: usize,
    poisoned: &mut bool,
) {
    {
        let [in0, in1] = input;
        let [out0, out1] = output;
        match instances {
            [stereo] if stereo.is_stereo() => {
                stereo.process_stereo([&mut in0[..n], &mut in1[..n]], [&mut out0[..n], &mut out1[..n]]);
            }
            [mono] => {
                // (one mono instance on a channel played as a pair of itself)
                mono.process_mono(&mut in0[..n], &mut out0[..n]);
                out1[..n].copy_from_slice(&out0[..n]);
            }
            [left, right] => {
                left.process_mono(&mut in0[..n], &mut out0[..n]);
                right.process_mono(&mut in1[..n], &mut out1[..n]);
            }
            _ => {}
        }
    }
    let bad = output.iter().any(|ch| ch[..n].iter().any(|x| !x.is_finite()));
    if bad {
        *poisoned = true;
        for ch in output.iter_mut() {
            ch[..n].fill(0.0);
        }
    }
}

/// A logarithmic parameter gliding to its value
#[derive(Clone, Copy, Debug)]
struct Glide {
    param: usize,
    /// Where it is now (what the instances have)
    now: f64,
}

/// A Guitarix unit as a processor
pub struct GuitarixProcessor {
    plan: &'static Plan,
    rate: u32,
    groups: Vec<Group>,
    /// The groups of the processor this one replaced, fading out (a field changed)
    fading: Vec<Group>,
    /// Samples of the crossfade done, and its length
    fade_at: usize,
    fade_len: usize,
    /// The parameters' values (their targets), in the schema's order
    params: Vec<f32>,
    glides: Vec<Glide>,
    /// The factor the distance in the log domain keeps per sample
    glide_keep: f64,
    choices: Vec<f32>,
    reals: Vec<f32>,
    /// The outputs since the window last read them (a peak's highest, another's last)
    outputs: Vec<f32>,
    scratch_in: [Vec<f32>; 2],
    scratch_copy: [Vec<f32>; 2],
    scratch_out: [Vec<f32>; 2],
    scratch_fade: [Vec<f32>; 2],
}

impl std::fmt::Debug for GuitarixProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuitarixProcessor").field("unit", &self.plan.unit.id).finish_non_exhaustive()
    }
}

/// Builds the unit's processor from a complete state: its instances made, their parameters set and started, in the
/// build (they allocate), never on the audio thread
pub fn build(unit: &str, state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let plan = plan(unit);
    let view = StateView::new(state);
    let params = plan.params.iter().map(|p| view.param_f32(p.key)).collect::<Result<Vec<f32>>>()?;
    let choices = plan
        .choices
        .iter()
        .map(|c| view.choice(c.key, |k| c.keys.iter().position(|x| *x == k)).map(|i| c.values[i]))
        .collect::<Result<Vec<f32>>>()?;
    let reals = plan.reals.iter().map(|r| view.real(r.key).map(|v| v as f32)).collect::<Result<Vec<f32>>>()?;
    let rate = cx.spec.sample_rate;
    let channels = usize::from(cx.channels).max(1);
    let block = plan.fixed.map_or(UNIT_BLOCK, |b| b as usize);
    let mut groups = Vec::new();
    for p in 0..channels.div_ceil(2) {
        let (left, right) = (2 * p, (2 * p + 1).min(channels - 1));
        let count = if plan.unit.stereo || left == right { 1 } else { 2 };
        let mut instances = Vec::with_capacity(count);
        for _ in 0..count {
            let mut instance = Instance::new(plan.unit.index)?;
            apply(plan, &mut instance, &params, &choices, &reals);
            instance.start(UNIT_RATE, plan.fixed.unwrap_or(MAX_BLOCK as u32))?;
            instances.push(instance);
        }
        if instances.is_empty() {
            return Err(Error::new("plugin.guitarix.no_instance").arg("unit", plan.unit.id.as_str()));
        }
        let path = if rate != UNIT_RATE {
            Path::Converted(Box::new(Converter::new(rate, block)?))
        } else if let Some(b) = plan.fixed {
            Path::Fixed(Fifo::new(b as usize))
        } else {
            Path::Direct
        };
        groups.push(Group { left, right, instances, path });
    }
    let glides = plan
        .params
        .iter()
        .enumerate()
        .filter(|(_, p)| p.scale == Scale::Log)
        .map(|(i, _)| Glide { param: i, now: f64::from(params[i]) })
        .collect();
    let glide_keep = libm::exp(-1.0 / (GLIDE_MS / 1000.0 * f64::from(rate)));
    let fade_len = ((CROSSFADE_MS / 1000.0) * rate as f32) as usize;
    Ok(Box::new(GuitarixProcessor {
        plan,
        rate,
        groups,
        fading: Vec::new(),
        fade_at: fade_len,
        fade_len,
        params,
        glides,
        glide_keep,
        choices,
        reals,
        outputs: vec![0.0; plan.outputs.len()],
        scratch_in: [vec![0.0; GRID], vec![0.0; GRID]],
        scratch_copy: [vec![0.0; GRID], vec![0.0; GRID]],
        scratch_out: [vec![0.0; GRID], vec![0.0; GRID]],
        scratch_fade: [vec![0.0; GRID], vec![0.0; GRID]],
    }))
}

/// Every value of the state to an instance's variables
fn apply(plan: &Plan, instance: &mut Instance, params: &[f32], choices: &[f32], reals: &[f32]) {
    for (&desc, &v) in plan.param_desc.iter().zip(params) {
        instance.set(desc, v);
    }
    for (c, &v) in plan.choices.iter().zip(choices) {
        instance.set(c.desc, v);
    }
    for (r, &v) in plan.reals.iter().zip(reals) {
        instance.set(r.desc, v);
    }
}

fn set_in(groups: &mut [Group], desc: usize, value: f32) {
    for g in groups {
        for i in &mut g.instances {
            i.set(desc, value);
        }
    }
}

impl GuitarixProcessor {
    fn set_everywhere(&mut self, desc: usize, value: f32) {
        set_in(&mut self.groups, desc, value);
        set_in(&mut self.fading, desc, value);
    }

    fn event(&mut self, kind: EventKind) {
        match kind {
            EventKind::Param { index, value } => {
                let index = index as usize;
                let (Some(&desc), Some(spec)) = (self.plan.param_desc.get(index), self.plan.params.get(index)) else {
                    return;
                };
                if !value.is_finite() {
                    // (NaN would pass the clamp and silence the unit until the parameter moved again)
                    return;
                }
                let value = value.clamp(spec.min as f32, spec.max as f32);
                self.params[index] = value;
                // (a logarithmic one glides there, piece by piece)
                if spec.scale != Scale::Log {
                    self.set_everywhere(desc, value);
                }
            }
            EventKind::AllSoundOff => self.clear(),
            _ => {}
        }
    }

    /// The glides one piece of `n` samples on
    fn glide(&mut self, n: usize) {
        let keep = self.glide_keep.powi(n as i32);
        for k in 0..self.glides.len() {
            let Glide { param, now } = self.glides[k];
            let target = f64::from(self.params[param]);
            if now == target {
                continue;
            }
            let ratio = now / target;
            // In the log domain: the distance keeps `keep` of itself
            let next = if ratio.ln().abs() < 1.0e-4 { target } else { target * libm::pow(ratio, keep) };
            self.glides[k].now = next;
            self.set_everywhere(self.plan.param_desc[param], next as f32);
        }
    }

    /// Silence now: every unit cleared, the glides at their values, no crossfade
    fn clear(&mut self) {
        for g in self.groups.iter_mut().chain(self.fading.iter_mut()) {
            g.clear();
        }
        for k in 0..self.glides.len() {
            let param = self.glides[k].param;
            self.glides[k].now = f64::from(self.params[param]);
            self.set_everywhere(self.plan.param_desc[param], self.params[param]);
        }
        self.fade_at = self.fade_len;
    }

    fn run_piece(&mut self, cx: &mut ProcessContext<'_>, at: usize, n: usize) {
        self.glide(n);
        let fading = self.fade_at < self.fade_len;
        let Self {
            groups, fading: old, scratch_in, scratch_copy, scratch_out, scratch_fade, fade_at, fade_len, ..
        } = self;
        for (p, g) in groups.iter_mut().enumerate() {
            scratch_in[0][..n].copy_from_slice(&cx.buffers.in_out(g.left)[at..at + n]);
            scratch_in[1][..n].copy_from_slice(&cx.buffers.in_out(g.right)[at..at + n]);
            if fading && let Some(o) = old.get_mut(p) {
                // (the units may change their input: each gets its own copy)
                scratch_copy[0][..n].copy_from_slice(&scratch_in[0][..n]);
                scratch_copy[1][..n].copy_from_slice(&scratch_in[1][..n]);
                o.process(scratch_copy, scratch_fade, n);
            }
            g.process(scratch_in, scratch_out, n);
            if fading {
                for f in 0..n {
                    let t = ((*fade_at + f) as f32 / *fade_len as f32).min(1.0);
                    for c in 0..2 {
                        scratch_out[c][f] = scratch_out[c][f] * t + scratch_fade[c][f] * (1.0 - t);
                    }
                }
            }
            cx.buffers.in_out(g.left)[at..at + n].copy_from_slice(&scratch_out[0][..n]);
            if g.right != g.left {
                cx.buffers.in_out(g.right)[at..at + n].copy_from_slice(&scratch_out[1][..n]);
            }
        }
        if fading {
            *fade_at = (*fade_at + n).min(*fade_len);
        }
    }

    /// The outputs after a block (the legacy plugin's collectOutputs, once a period): a peak's highest since the
    /// window read it (the unit's own starts again from 0), another's value now; the highest of the instances
    fn collect_outputs(&mut self) {
        for (o, &desc) in self.outputs.iter_mut().zip(&self.plan.outputs) {
            let peak = self.plan.unit.params[desc].is_peak();
            let mut highest = f32::MIN;
            for g in &mut self.groups {
                for i in &mut g.instances {
                    highest = highest.max(if peak { i.take_peak(desc) } else { i.output(desc) });
                }
            }
            *o = if peak { o.max(highest) } else { highest };
        }
    }
}

impl Processor for GuitarixProcessor {
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
        self.collect_outputs();
        if let Some(writer) = cx.display.as_mut() {
            let outputs = &mut self.outputs;
            let plan = self.plan;
            writer.write(|frame| {
                frame.fill(0);
                let bytes = DISPLAY_BYTES.min(frame.len());
                for (chunk, v) in frame[..bytes].as_chunks_mut::<4>().0.iter_mut().zip(&*outputs) {
                    chunk.copy_from_slice(&v.to_le_bytes());
                }
                // (the peaks start again once read)
                for (o, &desc) in outputs.iter_mut().zip(&plan.outputs) {
                    if plan.unit.params[desc].is_peak() {
                        *o = 0.0;
                    }
                }
            });
        }
    }

    fn reset(&mut self) {
        self.clear();
    }

    fn latency(&self) -> u32 {
        self.groups.first().map_or(0, Group::latency)
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if !std::ptr::eq(old.plan, self.plan) || old.rate != self.rate || old.groups.len() != self.groups.len() {
            return Adopted::No;
        }
        if old.groups.iter().zip(&self.groups).any(|(a, b)| a.instances.len() != b.instances.len()) {
            return Adopted::No;
        }
        // The glides go on from where the old one's were
        for (new, prev) in self.glides.iter_mut().zip(&old.glides) {
            new.now = prev.now;
        }
        if old.choices == self.choices && old.reals == self.reals {
            // The same fields: the old instances (their sound) go on with this state's values; this processor's
            // fresh ones go back with the old processor
            std::mem::swap(&mut self.groups, &mut old.groups);
            std::mem::swap(&mut self.fading, &mut old.fading);
            self.fade_at = old.fade_at;
            for g in self.groups.iter_mut().chain(self.fading.iter_mut()) {
                for i in &mut g.instances {
                    apply(self.plan, i, &self.params, &self.choices, &self.reals);
                }
            }
        } else {
            // Another selector or start setting: the fresh instances fade in over the old ones, which take this
            // state's parameters (their own fields stay)
            std::mem::swap(&mut self.fading, &mut old.groups);
            self.fade_at = 0;
            for (&desc, &v) in self.plan.param_desc.iter().zip(&self.params) {
                set_in(&mut self.fading, desc, v);
            }
        }
        for k in 0..self.glides.len() {
            let g = self.glides[k];
            self.set_everywhere(self.plan.param_desc[g.param], g.now as f32);
        }
        std::mem::swap(&mut self.outputs, &mut old.outputs);
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
