//! Note effects (interfaces.md, I-9; decisoes-autonomas.md, "O que deixa de seguir o legado", item 2): plugins of
//! [`crate::PluginKind::NoteEffect`] that turn the note stream of a track into another one before its instrument —
//! chords, an arpeggio, a transposition, a velocity curve. They have no audio and no ports; they receive note and
//! controller events and emit events, sample-accurate and deterministic.
//!
//! A plugin implements [`NoteEffect`] (its parameters, how an incoming event changes the stream, and what falls due
//! between events: an arpeggio's steps) and builds a [`NoteFx`] around it, the [`lm_render::Processor`] the host runs
//! (its `process_notes`). The driver splits the block at every event: [`NoteEffect::run`] first emits whatever falls
//! due before the event's offset, then the event is handled, so parameters apply at their exact offset.
//!
//! # What an effect must do
//!
//! - **Notes in, notes out.** A note-off ends the notes its note-on made; an effect that makes notes of its own
//!   (a chord, an arpeggio) numbers them from [`GENERATED_IDS`], never reusing a number of a note still sounding, and
//!   replaces every note it receives (it does not forward the base note). An effect that changes a note without
//!   making new ones (a transposition, a velocity curve) keeps the note's number.
//! - **Expressions follow the notes.** A `NoteExpression` names a note it received: an effect that renumbers
//!   forwards it to the notes it made from it, and a note it makes later starts with the expressions its base
//!   already has (see [`Expressions`]).
//! - **Releases are forwarded.** `AllNotesRelease` and `AllSoundOff` end every note the effect holds, then go on.
//! - **Time is the engine's.** Steps are placed by `cx.transport.frame` (samples since the engine started: it never
//!   jumps, also while stopped) and the tempo `cx.transport.bpm`; random choices come from `cx.rng`.

use std::any::Any;

use lm_base::Result;
use lm_render::{
    Adopted, EventKind, Expression, NoteContext, NoteId, NoteOut, ProcessContext, Processor, Rng, TransportInfo,
};
use lm_state::ParamSpec;

use crate::state::StateView;

/// The first number of a note an effect makes (the instrument's own sounding notes start at `0x8000_0000`, the
/// renderer's base notes count up from 1)
pub const GENERATED_IDS: u32 = 0x4000_0000;

/// What an effect may read of the block being processed
pub struct BlockCx<'a> {
    /// The frames of the block
    pub frames: usize,
    pub transport: &'a TransportInfo,
    pub rng: &'a mut Rng,
}

impl std::fmt::Debug for BlockCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockCx").field("frames", &self.frames).finish_non_exhaustive()
    }
}

impl BlockCx<'_> {
    /// The engine frame of an offset of this block: never jumps, the time steps are placed on
    pub fn frame_at(&self, offset: u32) -> u64 {
        self.transport.frame + u64::from(offset)
    }

    /// The tempo, or 120 when it is unknown
    pub fn bpm(&self) -> f64 {
        if self.transport.bpm > 0.0 { self.transport.bpm } else { 120.0 }
    }

    /// Frames in `beats` quarter notes at the tempo (at least 1)
    pub fn beats_to_frames(&self, beats: f64) -> u64 {
        ((beats * 60.0 / self.bpm() * f64::from(self.transport.sample_rate)) as u64).max(1)
    }

    /// Frames in `ms` milliseconds (at least 1)
    pub fn ms_to_frames(&self, ms: f64) -> u64 {
        ((ms / 1000.0 * f64::from(self.transport.sample_rate)) as u64).max(1)
    }
}

/// A note effect's own part: its parameters and its handling of the stream
pub trait NoteEffect: Send + 'static {
    /// The plugin's parameters (the indices `0..PARAMS` of its schema)
    const PARAMS: usize;

    /// A parameter changed (audio thread: no allocation). The value is inside the parameter's range
    fn set_param(&mut self, index: usize, value: f32);

    /// An event of the stream arrives at `at` (an offset of the block): emit what it makes to `out`, at `at` or
    /// later. Never a `Param`
    fn event(&mut self, at: u32, kind: EventKind, cx: &mut BlockCx<'_>, out: &mut NoteOut);

    /// Emits what falls due at offsets `from..to` of the block (arpeggio steps and the ends of their gates), each
    /// at its own offset. Called between events; `from == to` never happens
    fn run(&mut self, from: u32, to: u32, cx: &mut BlockCx<'_>, out: &mut NoteOut) {
        let _ = (from, to, cx, out);
    }

    /// Ends what it holds without emitting anything (the panic command resets processors): held notes forgotten
    fn reset(&mut self) {}

    /// Takes over the notes the effect it replaces holds (a change of state while notes play): its running state,
    /// not its settings, which the new effect was built with
    fn take_state(&mut self, old: &mut Self);
}

/// The processor of a [`NoteEffect`]
pub struct NoteFx<E: NoteEffect> {
    effect: E,
    values: Box<[f32]>,
    ranges: Box<[(f32, f32)]>,
}

impl<E: NoteEffect> std::fmt::Debug for NoteFx<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoteFx").finish_non_exhaustive()
    }
}

impl<E: NoteEffect> NoteFx<E> {
    /// The processor for `effect` with the parameters of a complete `state` (`params` are all of the plugin's, in
    /// index order: `E::PARAMS` of them). Allocates (U31)
    pub fn new(mut effect: E, state: &StateView<'_>, params: &[ParamSpec]) -> Result<Self> {
        let mut values = Vec::with_capacity(params.len());
        for (i, p) in params.iter().enumerate() {
            let v = state.param_f32(p.key)?;
            values.push(v);
            effect.set_param(i, v);
        }
        let ranges = params.iter().map(|p| (p.min as f32, p.max as f32)).collect();
        Ok(Self { effect, values: values.into_boxed_slice(), ranges })
    }

    fn set(&mut self, index: u32, value: f32) {
        let i = index as usize;
        let Some(&(lo, hi)) = self.ranges.get(i) else { return };
        if !value.is_finite() {
            return;
        }
        let v = value.clamp(lo, hi);
        self.values[i] = v;
        self.effect.set_param(i, v);
    }
}

impl<E: NoteEffect> Processor for NoteFx<E> {
    fn process(&mut self, _: &mut ProcessContext<'_>) {}

    fn process_notes(&mut self, cx: &mut NoteContext<'_>) {
        let frames = cx.frames as u32;
        let mut block = BlockCx { frames: cx.frames, transport: cx.transport, rng: cx.rng };
        let mut at = 0u32;
        for e in cx.events {
            let o = e.offset.min(frames.saturating_sub(1));
            if o > at {
                self.effect.run(at, o, &mut block, cx.out);
                at = o;
            }
            match e.kind {
                EventKind::Param { index, value } => self.set(index, value),
                kind => self.effect.event(o, kind, &mut block, cx.out),
            }
        }
        if at < frames {
            self.effect.run(at, frames, &mut block, cx.out);
        }
    }

    fn reset(&mut self) {
        self.effect.reset();
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if old.values.len() != self.values.len() {
            return Adopted::No;
        }
        self.effect.take_state(&mut old.effect);
        // The values the old one heard last (automation and gestures since the tree was read)
        for i in 0..self.values.len() {
            let v = old.values[i];
            self.set(i as u32, v);
        }
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The note expressions of the notes an effect received, by their number: a note the effect makes from one of them
/// starts with the same values, and a later expression moves the notes already made. Fixed capacity
#[derive(Clone, Debug)]
pub struct Expressions {
    bases: Vec<(NoteId, [f32; EXPRESSION_COUNT])>,
}

impl Expressions {
    /// Room for `capacity` notes at once (more are ignored)
    pub fn new(capacity: usize) -> Self {
        Self { bases: Vec::with_capacity(capacity) }
    }

    pub fn clear(&mut self) {
        self.bases.clear();
    }

    /// A note began (all its expressions at 0)
    pub fn begin(&mut self, note: NoteId) {
        if self.bases.len() < self.bases.capacity() && !self.bases.iter().any(|(n, _)| *n == note) {
            self.bases.push((note, [0.0; EXPRESSION_COUNT]));
        }
    }

    /// A note ended
    pub fn end(&mut self, note: NoteId) {
        if let Some(i) = self.bases.iter().position(|(n, _)| *n == note) {
            self.bases.swap_remove(i);
        }
    }

    /// A new value of an expression of `note`; false when the note is not known
    pub fn set(&mut self, note: NoteId, expression: Expression, value: f32) -> bool {
        let value = if value.is_finite() { value } else { 0.0 };
        match self.bases.iter_mut().find(|(n, _)| *n == note) {
            Some((_, v)) => {
                v[slot(expression)] = value;
                true
            }
            None => false,
        }
    }

    /// Emits the expressions of `base` that are not 0 for the new note `made`, at `at`
    pub fn apply(&self, base: NoteId, made: NoteId, at: u32, out: &mut NoteOut) {
        let Some((_, v)) = self.bases.iter().find(|(n, _)| *n == base) else { return };
        for (i, x) in v.iter().enumerate() {
            if *x != 0.0 {
                out.push(at, EventKind::NoteExpression { note: made, expression: EXPRESSIONS[i], value: *x, ramp: 0 });
            }
        }
    }
}

/// Every note expression, in the order [`Expressions`] keeps their values
const EXPRESSIONS: [Expression; EXPRESSION_COUNT] =
    [Expression::Pan, Expression::Tuning, Expression::Pressure, Expression::Timbre];
const EXPRESSION_COUNT: usize = 4;

fn slot(e: Expression) -> usize {
    match e {
        Expression::Pan => 0,
        Expression::Tuning => 1,
        Expression::Pressure => 2,
        Expression::Timbre => 3,
    }
}

/// The numbers of the notes an effect makes
#[derive(Clone, Copy, Debug)]
pub struct IdCounter(u32);

impl Default for IdCounter {
    fn default() -> Self {
        Self(GENERATED_IDS)
    }
}

impl IdCounter {
    pub fn next(&mut self) -> NoteId {
        let id = NoteId(self.0);
        self.0 = if self.0 >= 0x7FFF_FFFF { GENERATED_IDS } else { self.0 + 1 };
        id
    }
}

#[cfg(test)]
mod expression_tests {
    use super::*;

    #[test]
    fn a_non_finite_expression_is_stored_as_zero() {
        let mut e = Expressions::new(4);
        e.begin(NoteId(1));
        assert!(e.set(NoteId(1), Expression::Tuning, f32::NAN));
        assert!(e.bases[0].1.iter().all(|x| x.is_finite()));
    }
}
