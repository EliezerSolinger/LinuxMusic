//! What every effect plugin shares (the legacy effects, ported to `lm-dsp`): the processor around a stereo algorithm.
//!
//! An effect works in place on its channel's channels ([`BuildContext::channels`]): one copy of the algorithm for
//! each pair (a last odd channel is played as a pair of itself, its left side kept). The block is cut into pieces on
//! a grid of [`GRID`] samples of the engine's frame count, and, unless the effect asks otherwise
//! ([`Effect::CUT_AT_EVENTS`]), at every event: each parameter change takes effect where it falls, and the pieces —
//! so the audio — are the same for any block size that is a multiple of the grid (V3). An effect that ramps a knob
//! does so over the piece in which the knob changed.
//!
//! A rebuilt effect adopts its predecessor's sound ([`Effect::adopt`]): delay lines, filter memories, envelopes go
//! on where they were, with the new settings.

use std::any::Any;

use lm_base::Result;
use lm_dsp::small_effects::Ramp;
use lm_render::{Adopted, EventKind, ProcessContext, Processor};
use lm_state::Value;

use lm_plugin::instrument::GRID;
use lm_plugin::{BuildContext, Ports, StateView};

/// A stereo effect algorithm with its settings
pub trait Effect: Send + 'static {
    /// Whether a parameter change cuts the piece where it falls (`true`), or waits for the next grid point (an
    /// algorithm that must always run whole grid pieces, such as one whose period is the grid)
    const CUT_AT_EVENTS: bool = true;

    /// Parameter `index` (its place in the schema's `params`) takes `value`, in its own unit
    fn set_param(&mut self, index: usize, value: f32);

    /// Processes a piece of up to [`GRID`] frames in place. Never allocates, locks or blocks
    fn process(&mut self, frames: &mut [[f32; 2]]);

    /// Forgets the sound (delay lines, filter memories, envelopes): the panic command
    fn reset(&mut self);

    /// Takes over the sound of the effect it replaces (same plugin), keeping its own settings; `false` when it cannot
    /// (the old one then plays its tail elsewhere, U32)
    fn adopt(&mut self, old: &mut Self) -> bool;

    /// Latency in samples
    fn latency(&self) -> u32 {
        0
    }

    /// Fills the plugin window's live frame (its [`lm_plugin::DisplaySpec`]'s bytes, all of them) with what was
    /// measured since the last frame, and starts measuring again. Called once per block, on the first pair, when the
    /// window is open
    fn display(&mut self, frame: &mut [u8]) {
        frame.fill(0);
    }
}

/// Writes @p values into a display frame as little-endian `f32`s, from the start (the rest of the frame is zeroed)
pub fn put_f32s(frame: &mut [u8], values: &[f32]) {
    frame.fill(0);
    for (chunk, v) in frame.as_chunks_mut::<4>().0.iter_mut().zip(values) {
        chunk.copy_from_slice(&v.to_le_bytes());
    }
}

/// The peak of each side of @p frames, raised into @p peaks
pub fn raise_peaks(peaks: &mut [f32; 2], frames: &[[f32; 2]]) {
    for f in frames {
        peaks[0] = peaks[0].max(f[0].abs());
        peaks[1] = peaks[1].max(f[1].abs());
    }
}

/// The processor of an effect: one algorithm per pair of channels
pub struct EffectProcessor<E: Effect> {
    pairs: Vec<E>,
    channels: usize,
    piece: [[f32; 2]; GRID],
}

impl<E: Effect> std::fmt::Debug for EffectProcessor<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectProcessor").field("channels", &self.channels).finish_non_exhaustive()
    }
}

impl<E: Effect> EffectProcessor<E> {
    /// Builds one algorithm per pair of the channel's channels with `make` (in the build, never on the audio thread)
    pub fn build(cx: &BuildContext<'_>, mut make: impl FnMut() -> Result<E>) -> Result<Box<dyn Processor>> {
        let channels = usize::from(cx.channels).max(1);
        let pairs = (0..channels.div_ceil(2)).map(|_| make()).collect::<Result<Vec<E>>>()?;
        Ok(Box::new(Self { pairs, channels, piece: [[0.0; 2]; GRID] }))
    }

    /// The algorithm of the first pair (tests)
    pub fn first(&self) -> &E {
        &self.pairs[0]
    }

    fn run_piece(&mut self, cx: &mut ProcessContext<'_>, at: usize, n: usize) {
        let channels = self.channels;
        for (p, fx) in self.pairs.iter_mut().enumerate() {
            let (l, r) = (2 * p, (2 * p + 1).min(channels - 1));
            let piece = &mut self.piece[..n];
            {
                let left = &cx.buffers.in_out(l)[at..at + n];
                for (frame, x) in piece.iter_mut().zip(left) {
                    frame[0] = *x;
                }
            }
            {
                let right = &cx.buffers.in_out(r)[at..at + n];
                for (frame, x) in piece.iter_mut().zip(right) {
                    frame[1] = *x;
                }
            }
            fx.process(piece);
            {
                let left = &mut cx.buffers.in_out(l)[at..at + n];
                for (x, frame) in left.iter_mut().zip(piece.iter()) {
                    *x = frame[0];
                }
            }
            if r != l {
                let right = &mut cx.buffers.in_out(r)[at..at + n];
                for (x, frame) in right.iter_mut().zip(piece.iter()) {
                    *x = frame[1];
                }
            }
        }
    }

    fn event(&mut self, kind: EventKind) {
        match kind {
            EventKind::Param { index, value } => {
                for fx in &mut self.pairs {
                    fx.set_param(index as usize, value);
                }
            }
            EventKind::AllSoundOff => {
                for fx in &mut self.pairs {
                    fx.reset();
                }
            }
            _ => {}
        }
    }
}

impl<E: Effect> Processor for EffectProcessor<E> {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let events = cx.events;
        let origin = cx.transport.frame;
        let mut e = 0;
        let mut at = 0;
        while at < frames {
            // The events due here (with the grid only: every event up to here)
            while e < events.len() && (events[e].offset as usize) <= at {
                self.event(events[e].kind);
                e += 1;
            }
            let grid = GRID - ((origin + at as u64) % GRID as u64) as usize;
            let mut end = (at + grid).min(frames);
            if E::CUT_AT_EVENTS && e < events.len() {
                end = end.min(events[e].offset as usize).max(at + 1);
            }
            self.run_piece(cx, at, end - at);
            at = end;
        }
        // Events past the block's end (none, from a well-behaved host) still count
        for ev in &events[e..] {
            self.event(ev.kind);
        }
        if let Some(writer) = cx.display.as_mut() {
            let fx = &mut self.pairs[0];
            writer.write(|frame| fx.display(frame));
        }
    }

    fn reset(&mut self) {
        for fx in &mut self.pairs {
            fx.reset();
        }
    }

    fn latency(&self) -> u32 {
        self.pairs[0].latency()
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if old.pairs.len() != self.pairs.len() {
            return Adopted::No;
        }
        for (new, old) in self.pairs.iter_mut().zip(old.pairs.iter_mut()) {
            if !new.adopt(old) {
                return Adopted::No;
            }
        }
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A list of reals of a complete state (a drawn curve, a graph); a missing field is `plugin.state.missing`
pub fn reals(state: &Value, key: &str) -> Result<Vec<f64>> {
    let list = state.get(key).ok_or_else(|| lm_base::Error::new("plugin.state.missing").arg("key", key))?;
    Ok(list.items().map(|v| v.as_real().unwrap_or(0.0)).collect())
}

/// An effect's ports: none (it works in place on its channel's channels)
pub fn no_ports(_: &Value) -> Ports {
    Ports { outputs: Vec::new(), sidechains: Vec::new() }
}

/// An effect's parameters, in the order of its schema's `params`: their values now, and the values the last piece
/// ended at (a knob that ramps goes from one to the other over the piece in which it changed)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knobs<const N: usize> {
    now: [f32; N],
    before: [f32; N],
}

impl<const N: usize> Knobs<N> {
    /// From a complete state
    pub fn from_state(state: &Value, keys: &[&str; N]) -> Result<Self> {
        let view = StateView::new(state);
        let mut now = [0.0; N];
        for (v, key) in now.iter_mut().zip(keys) {
            *v = view.param_f32(key)?;
        }
        Ok(Self { now, before: now })
    }

    /// A parameter event; an index past the parameters is ignored
    pub fn set(&mut self, index: usize, value: f32) {
        if let Some(v) = self.now.get_mut(index) {
            *v = value;
        }
    }

    pub fn get(&self, index: usize) -> f32 {
        self.now[index]
    }

    /// Whether the parameter is on (a switch: a stepped parameter of 0 to 1)
    pub fn on(&self, index: usize) -> bool {
        self.now[index] >= 0.5
    }

    /// From where the last piece ended to now
    pub fn ramp(&self, index: usize) -> Ramp {
        Ramp { from: self.before[index], to: self.now[index] }
    }

    /// The piece is done: the ramps end here
    pub fn settle(&mut self) {
        self.before = self.now;
    }

    /// Adopting: the ramps start where the old effect's ended, so a changed knob glides instead of jumping
    pub fn adopt(&mut self, old: &Self) {
        self.before = old.before;
    }
}
