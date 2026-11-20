//! A test instrument: a sine per note, with a short linear attack and release. Small and exact, for the tests of the
//! compiler and the session, and the smallest example of the plugin contract written without the instrument
//! functions (its own voices on lm-render's `VoicePool`).
//!
//! - parameters: `gain` (0–1, the old key `level` is an alias, C5), `detune` (cents), `release` (ms);
//! - fields: `octave` (a choice, U30) and `aux` (a second output port, mono, that the state turns on: the ports
//!   depend on the state, I-9.1);
//! - version 2; version 1 had a `wave` field that is gone (its fixture is in `tests/data/fixtures`).

#![forbid(unsafe_code)]

use std::any::Any;

use lm_base::Result;
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Adopted, EventKind, NoteId, Policy, ProcessContext, Processor, Voice, VoiceNote, VoicePool};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

pub const ID: &str = "testsine";
pub const VOICES: usize = 8;
const ATTACK_MS: f64 = 1.0;
const GRID: usize = 32;

pub static OCTAVES: [&str; 3] = ["down", "same", "up"];

/// The descriptor
pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.testsine.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    velocity: Velocity::Level,
    display: None,
    build,
    tests: TestHints {
        tuning: Some(TuningProbe { state: tuning_state, key: 69, hz: 440.0 }),
        no_adopt: &[],
        voices: VOICES,
        max_level: 1.0e3,
    },
};

fn upgrade_1(state: Value) -> Result<Value> {
    // Version 1 had a waveform choice with a single value: it goes
    Ok(state.without("wave"))
}

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:testsine", 2)
            .field(FieldSpec::choice("octave", &OCTAVES, "same"))
            .field(FieldSpec::bool("aux", false))
            .param(ParamSpec::new("gain", 0.0, 1.0, 0.5).aliases(&["level"]))
            .param(ParamSpec::new("detune", -100.0, 100.0, 0.0).unit("cents"))
            .param(ParamSpec::new("release", 1.0, 1000.0, 10.0).scale(Scale::Log).unit("ms"))
            .upgrade(1, upgrade_1),
    ]
}

fn tuning_state() -> Value {
    Value::empty_object()
}

fn ports(state: &Value) -> Ports {
    let mut outputs = vec![Port::new("main", 2)];
    if state.get("aux").and_then(Value::as_bool) == Some(true) {
        outputs.push(Port::new("aux", 1));
    }
    Ports { outputs, sidechains: Vec::new() }
}

const GAIN: u32 = 0;
const DETUNE: u32 = 1;
const RELEASE: u32 = 2;

#[derive(Debug, Default)]
struct SineVoice {
    note: NoteId,
    key: u8,
    velocity: f32,
    phase: f64,
    /// The envelope: up by `attack_step` to 1, then down by `release_step` after the release
    level: f64,
    released: bool,
    active: bool,
    attack_step: f64,
    release_step: f64,
}

impl Voice for SineVoice {
    fn start(&mut self, note: VoiceNote) {
        self.note = note.note;
        self.key = note.key;
        self.velocity = note.velocity;
        self.phase = 0.0;
        self.level = 0.0;
        self.released = false;
        self.active = true;
    }
    fn glide(&mut self, note: VoiceNote) {
        self.key = note.key;
    }
    fn release(&mut self) {
        self.released = true;
    }
    fn level(&self) -> f32 {
        (self.level * f64::from(self.velocity)) as f32
    }
    fn is_finished(&self) -> bool {
        !self.active
    }
    fn kill(&mut self) {
        self.active = false;
        self.level = 0.0;
    }
}

/// The processor
#[derive(Debug)]
pub struct TestSine {
    rate: f64,
    octave: i32,
    aux: bool,
    gain: f32,
    detune: f32,
    release_ms: f32,
    /// The attack's and the release's steps per sample
    steps: (f64, f64),
    /// Note-offs of this block whose note-on had not come yet
    early_offs: Vec<NoteId>,
    pool: VoicePool<SineVoice>,
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let octave = s.choice("octave", |k| OCTAVES.iter().position(|o| *o == k))? as i32 - 1;
    let rate = f64::from(cx.spec.sample_rate);
    let mut p = TestSine {
        rate,
        octave,
        aux: s.boolean("aux")?,
        gain: s.param_f32("gain")?,
        detune: s.param_f32("detune")?,
        release_ms: s.param_f32("release")?,
        steps: (0.0, 0.0),
        early_offs: Vec::with_capacity(64),
        pool: VoicePool::new(Policy::Poly, VOICES, (rate * 0.005) as u32, SineVoice::default),
    };
    p.set_steps();
    Ok(Box::new(p))
}

impl TestSine {
    fn set_steps(&mut self) {
        let attack = 1.0 / (ATTACK_MS / 1000.0 * self.rate).max(1.0);
        let release = 1.0 / (f64::from(self.release_ms) / 1000.0 * self.rate).max(1.0);
        for (v, _) in self.pool.voices_mut() {
            v.attack_step = attack;
            v.release_step = release;
        }
        self.steps = (attack, release);
    }
}

impl Processor for TestSine {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let outputs = cx.buffers.outputs();
        let (_, mut outs, _) = cx.buffers.split();
        let mut at = 0usize;
        let mut e = 0usize;
        let events = cx.events;
        while at < frames {
            while e < events.len() && events[e].offset as usize <= at {
                match events[e].kind {
                    EventKind::NoteOn { note, channel, key, velocity } => {
                        let (attack, release) = self.steps;
                        self.pool.note_on(VoiceNote { note, channel, key, velocity });
                        for (v, _) in self.pool.voices_mut() {
                            v.attack_step = attack;
                            v.release_step = release;
                        }
                        // Its note-off came first, at the same sample (a note of no length, U26)
                        if let Some(i) = self.early_offs.iter().position(|n| *n == note) {
                            self.early_offs.swap_remove(i);
                            self.pool.note_off(note);
                        }
                    }
                    EventKind::NoteOff { note, .. } => {
                        if self.pool.voices_mut().any(|(v, _)| v.note == note && v.active) {
                            self.pool.note_off(note);
                        } else if self.early_offs.len() < self.early_offs.capacity() {
                            self.early_offs.push(note);
                        }
                    }
                    EventKind::Param { index, value } if value.is_finite() => match index {
                        GAIN => self.gain = value.clamp(0.0, 1.0),
                        DETUNE => self.detune = value.clamp(-100.0, 100.0),
                        RELEASE => {
                            self.release_ms = value.clamp(1.0, 1000.0);
                            self.set_steps();
                        }
                        _ => {}
                    },
                    EventKind::AllNotesRelease => self.pool.release_all(),
                    EventKind::AllSoundOff => self.pool.kill_all(),
                    _ => {}
                }
                e += 1;
            }
            let mut end = if e < events.len() { (events[e].offset as usize).clamp(at + 1, frames) } else { frames };
            // Pieces on a grid of the engine's frames: the same audio for any block size that is a multiple (V3)
            let grid = GRID - ((cx.transport.frame + at as u64) % GRID as u64) as usize;
            end = end.min(at + grid);
            let shift = f64::from(self.detune) / 100.0 + f64::from(12 * self.octave);
            let gain = f64::from(self.gain);
            let rate = self.rate;
            for (v, g) in self.pool.voices_mut() {
                if !v.active {
                    continue;
                }
                let step = 440.0 * libm::exp2((f64::from(v.key) - 69.0 + shift) / 12.0) / rate;
                for (i, k) in (at..end).enumerate() {
                    if v.released {
                        v.level -= v.release_step;
                        if v.level <= 0.0 {
                            v.level = 0.0;
                            v.active = false;
                            break;
                        }
                    } else if v.level < 1.0 {
                        v.level = (v.level + v.attack_step).min(1.0);
                    }
                    let x = (libm::sin(v.phase * std::f64::consts::TAU) * v.level * gain * f64::from(v.velocity))
                        as f32
                        * g.at(i);
                    v.phase += step;
                    v.phase -= v.phase.floor();
                    for ch in 0..outputs.min(2) {
                        outs.get_mut(ch)[k] += x;
                    }
                    if self.aux && outputs > 2 {
                        outs.get_mut(2)[k] += x;
                    }
                }
            }
            self.pool.advance(end - at);
            at = end;
        }
        self.early_offs.clear();
    }

    fn reset(&mut self) {
        self.pool.kill_all();
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        match old.as_any_mut().downcast_mut::<TestSine>() {
            Some(o) if o.rate == self.rate => {
                std::mem::swap(&mut self.pool, &mut o.pool);
                self.gain = o.gain;
                self.detune = o.detune;
                self.release_ms = o.release_ms;
                self.set_steps();
                Adopted::Yes
            }
            _ => Adopted::No,
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
