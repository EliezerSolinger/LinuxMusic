//! LB302 (F-INS-008: legacy `plugins/Lb302`): a monophonic imitation of the TB-303: one oscillator, a resonant filter
//! with its decaying envelope, slides between notes, the dead switch.
//!
//! State, version 1: parameters `vcfCutoffFrequency` (0–1.5, 0.75), `vcfResonance` (0–1.25, 0.75),
//! `vcfEnvelopeMod` (0–1, 0.1), `vcfEnvelopeDecay` (0–1, 0.1), `distortion` (0–1, 0), `slideDecay` (0–1, 0.6), and the
//! switches `slide`, `dead` and `db24` (the 24 dB/oct filter), all off; field `waveform` (one of the twelve shapes,
//! `bl-saw` by default: the legacy knob's 8).
//!
//! One stream, as the legacy synth: the newest note sounds (a new note while one sounds is a new note of the stream,
//! sliding to it with the slide on, not restarting its envelopes with the dead switch), and its release lets the
//! stream decay. Its velocity is its level and a pan expression its place (the legacy note's volume and panning, the
//! track's law); a tuning expression moves its pitch without starting it again. The legacy synth took its notes from
//! the instrument track, which gave a single-streamed instrument no envelope, LFO or filter; neither does this one,
//! whose notes come straight from the host: chords and the arpeggio are the instrument functions of the polyphonic
//! instruments and are not offered here (a monophonic stream would sound the newest of their notes only).

use std::any::Any;
use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::lb302::{Lb302, Lb302Settings, VcoShape, panning_to_volume};
use lm_plugin::instrument::GRID;
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Adopted, EventKind, Expression, NoteId, ProcessContext, Processor};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::stereo_out;

pub const ID: &str = "lb302";
pub const KEYS: [&str; 9] = [
    "vcfCutoffFrequency",
    "vcfResonance",
    "vcfEnvelopeMod",
    "vcfEnvelopeDecay",
    "distortion",
    "slideDecay",
    "slide",
    "dead",
    "db24",
];
/// The shapes' keys, in the order of the legacy knob
pub const WAVEFORM_KEYS: [&str; 12] = [
    "saw",
    "triangle",
    "square",
    "round-square",
    "moog",
    "sine",
    "exponential",
    "white-noise",
    "bl-saw",
    "bl-square",
    "bl-triangle",
    "bl-moog",
];
static WAVEFORMS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| WAVEFORM_KEYS.into());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.lb302.name",
    kind: PluginKind::Instrument,
    schemas,
    ports: stereo_out,
    velocity: Velocity::Level,
    display: None,
    build,
    tests: TestHints {
        // A sine, the filter open and no resonance: the note's pitch
        tuning: Some(TuningProbe { state: probe_state, key: 57, hz: 220.0 }),
        voices: 1,
        ..TestHints::new()
    },
};

/// A clean sine at the key (the filter open, no resonance, no envelope modulation)
pub fn probe_state() -> Value {
    let real = |v: f64| Value::real(v).unwrap_or(Value::NULL);
    let params = Value::empty_object()
        .with("vcfCutoffFrequency", real(1.5))
        .with("vcfResonance", real(0.0))
        .with("vcfEnvelopeMod", real(0.0));
    Value::empty_object().with(lm_state::PARAMS_KEY, params).with("waveform", Value::text("sine"))
}

fn schemas() -> Vec<NodeSchema> {
    let p = ParamSpec::new;
    let switch = |key| ParamSpec::new(key, 0.0, 1.0, 0.0).scale(Scale::Stepped);
    vec![
        NodeSchema::new("plugin:lb302", 1)
            .param(p(KEYS[0], 0.0, 1.5, 0.75))
            .param(p(KEYS[1], 0.0, 1.25, 0.75))
            .param(p(KEYS[2], 0.0, 1.0, 0.1))
            .param(p(KEYS[3], 0.0, 1.0, 0.1))
            .param(p(KEYS[4], 0.0, 1.0, 0.0))
            .param(p(KEYS[5], 0.0, 1.0, 0.6))
            .param(switch(KEYS[6]))
            .param(switch(KEYS[7]))
            .param(switch(KEYS[8]))
            .field(FieldSpec::choice("waveform", &WAVEFORMS, "bl-saw")),
    ]
}

/// The knobs as the synth reads them
pub fn settings(k: &[f32; 9], shape: VcoShape) -> Lb302Settings {
    Lb302Settings {
        cutoff: k[0],
        resonance: k[1],
        env_mod: k[2],
        decay: k[3],
        distortion: k[4],
        shape,
        slide_decay: k[5],
        slide: k[6] >= 0.5,
        dead: k[7] >= 0.5,
        db24: k[8] >= 0.5,
    }
}

/// The note the stream plays
#[derive(Clone, Copy, Debug)]
struct Sounding {
    id: NoteId,
    key: u8,
    velocity: f32,
    released: bool,
    tuning: f32,
    pan: f32,
    /// The frequency the synth was given last
    freq: f32,
}

pub struct Lb302Plugin {
    synth: Lb302,
    knobs: [f32; 9],
    shape: VcoShape,
    sounding: Option<Sounding>,
    buf: [[f32; 2]; GRID],
}

impl std::fmt::Debug for Lb302Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lb302Plugin").field("knobs", &self.knobs).finish_non_exhaustive()
    }
}

fn frequency(key: u8, tuning: f32) -> f32 {
    440.0 * libm::exp2f((f32::from(key) - 69.0 + tuning) / 12.0)
}

impl Lb302Plugin {
    fn event(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn { note, key, velocity, .. } => {
                let key = key.min(127);
                let freq = frequency(key, 0.0);
                self.sounding = Some(Sounding {
                    id: note,
                    key,
                    velocity: velocity.clamp(0.0, 1.0),
                    released: false,
                    tuning: 0.0,
                    pan: 0.0,
                    freq,
                });
                self.synth.note_on(freq);
            }
            EventKind::NoteOff { note, .. } => {
                if let Some(s) = self.sounding.as_mut()
                    && s.id == note
                {
                    s.released = true;
                }
            }
            EventKind::Param { index, value } => {
                if let Some(k) = self.knobs.get_mut(index as usize) {
                    *k = value;
                }
            }
            EventKind::NoteExpression { note, expression, value, .. } => {
                if let Some(s) = self.sounding.as_mut()
                    && s.id == note
                {
                    match expression {
                        Expression::Tuning => s.tuning = value,
                        Expression::Pan => s.pan = value.clamp(-1.0, 1.0),
                    }
                }
            }
            EventKind::AllNotesRelease => {
                if let Some(s) = self.sounding.as_mut() {
                    s.released = true;
                }
            }
            EventKind::AllSoundOff => self.reset(),
            EventKind::Midi { .. } => {}
        }
    }

    fn piece(&mut self, out: &mut [[f32; 2]]) {
        self.synth.prepare_period(&settings(&self.knobs, self.shape));
        let (playing, volume) = match self.sounding.as_mut() {
            Some(s) => {
                let freq = frequency(s.key, s.tuning);
                if freq != s.freq {
                    s.freq = freq;
                    self.synth.set_frequency(freq);
                }
                (!s.released, panning_to_volume(s.pan * 100.0, s.velocity))
            }
            None => (false, [0.0; 2]),
        };
        let release = if playing { out.len() } else { 0 };
        self.synth.render(out, release, playing, volume);
    }
}

impl Processor for Lb302Plugin {
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
            let n = end - at;
            let mut buf = self.buf;
            self.piece(&mut buf[..n]);
            let (_, mut outs, _) = cx.buffers.split();
            let (l, r) = outs.pair_mut(0, 1);
            for (i, f) in buf[..n].iter().enumerate() {
                l[at + i] = f[0];
                r[at + i] = f[1];
            }
            at = end;
        }
        for ev in &events[e..] {
            self.event(ev.kind);
        }
    }

    fn reset(&mut self) {
        self.synth.reset();
        self.sounding = None;
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        // The stream goes on: the old synth's state and sounding note, with the new knobs and shape
        std::mem::swap(&mut self.synth, &mut old.synth);
        self.sounding = old.sounding;
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut knobs = [0.0f32; 9];
    for (k, key) in knobs.iter_mut().zip(KEYS) {
        *k = s.param_f32(key)?;
    }
    let shape = s.choice("waveform", |key| WAVEFORM_KEYS.iter().position(|k| *k == key).map(|i| VcoShape::ALL[i]))?;
    let mut synth = Lb302::new(cx.spec.sample_rate as f32, 0);
    synth.prepare_period(&settings(&knobs, shape));
    Ok(Box::new(Lb302Plugin { synth, knobs, shape, sounding: None, buf: [[0.0; 2]; GRID] }))
}
