//! The note functions of a single-stream instrument, with a probe stream whose output is the sum of its notes'
//! pitches (`hz / 10000` each, a constant), so that the sum heard tells which notes sound and at what pitch

use lm_base::Result;
use lm_render::{EventKind, NoteId, ProcessSpec, Processor};
use lm_state::{NodeSchema, Value};

use super::*;
use crate::descriptor::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use crate::registry::Registry;
use crate::testing::{Bench, Timed, note_off, note_on};
use crate::{Port, Ports};

const SLOTS: usize = 64;

/// A stream that sounds a constant per note: its frequency over 10000 (detune and bend applied)
struct Probe {
    notes: [(u32, f32, bool); SLOTS],
    bend: f32,
}

impl Probe {
    fn new() -> Self {
        Self { notes: [(0, 0.0, false); SLOTS], bend: 0.0 }
    }
}

impl Stream for Probe {
    const PARAMS: usize = 0;
    fn set_param(&mut self, _: usize, _: f32) {}
    fn note_on(&mut self, note: NoteId, key: u8, _: f32, detune: f32) {
        // (the key in thousands, the detune below them)
        if let Some(s) = self.notes.iter_mut().find(|s| !s.2) {
            *s = (note.0, f32::from(key) * 1000.0 + detune, true);
        }
    }
    fn detune(&mut self, note: NoteId, semitones: f32) {
        for s in self.notes.iter_mut().filter(|s| s.2 && s.0 == note.0) {
            let key = (s.1 / 1000.0).round();
            s.1 = key * 1000.0 + semitones;
        }
    }
    fn note_off(&mut self, note: NoteId) {
        for s in self.notes.iter_mut().filter(|s| s.0 == note.0) {
            s.2 = false;
        }
    }
    fn bend(&mut self, bend: f32, range: f32) {
        self.bend = bend * range;
    }
    fn release_all(&mut self) {
        self.notes.iter_mut().for_each(|s| s.2 = false);
    }
    fn sound_off(&mut self) {
        self.release_all();
    }
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut sum = 0.0;
        for s in self.notes.iter().filter(|s| s.2) {
            let key = (s.1 / 1000.0).round();
            let detune = s.1 - key * 1000.0;
            sum += 440.0 * libm::exp2f((key + detune + self.bend - 69.0) / 12.0) / 10_000.0;
        }
        left.fill(sum);
        right.fill(sum);
    }
}

fn schemas() -> Vec<NodeSchema> {
    vec![add_note_functions(NodeSchema::new("plugin:streamprobe", 1))]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let params = schemas().remove(0).params;
    Ok(Box::new(StreamInstrument::new(Probe::new(), &s, &params, &cx.spec)?))
}

static PROBE: Descriptor = Descriptor {
    id: "streamprobe",
    name: "plugin.streamprobe.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

const RATE: u32 = 48_000;

fn state(params: &[(&str, f64)], fields: &[(&str, &str)]) -> Value {
    let registry = Registry::new(&[&PROBE]).expect("registry");
    let mut p = Value::empty_object();
    for (k, v) in params {
        p = p.with(k, Value::real(*v).expect("finite"));
    }
    let mut s = Value::empty_object().with(lm_state::PARAMS_KEY, p);
    for (k, v) in fields {
        s = s.with(k, Value::text(*v));
    }
    registry.complete("streamprobe", &s).expect("complete")
}

fn bench_of(state: &Value) -> Bench {
    let registry = Registry::new(&[&PROBE]).expect("registry");
    let spec = ProcessSpec { sample_rate: RATE, max_block: 1024 };
    let r = crate::ResourceView::new();
    let cx = BuildContext { spec, resources: &r, channels: 2 };
    Bench::instrument(registry.build("streamprobe", state, &cx).expect("build"), spec, 2, 1)
}

fn hz(keys: f32) -> f32 {
    440.0 * libm::exp2f((keys - 69.0) / 12.0) / 10_000.0
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0e-5
}

#[test]
fn the_pitch_knob_and_the_bend_move_every_note() {
    let s = state(&[("pitch", 50.0), ("bendRange", 12.0)], &[]);
    let events = [note_on(0, 1, 60, 1.0), Timed::new(4_800, EventKind::Midi { bytes: [0xE0, 0x7F, 0x7F] })];
    let out = bench_of(&s).render(9_600, 64, &events, true);
    assert!(near(out[0][100], hz(60.5)), "{}", out[0][100]);
    let full = 12.0 * (16_383.0 - 8_192.0) / 8_192.0;
    assert!(near(out[0][9_000], hz(60.5 + full)), "{}", out[0][9_000]);
}

#[test]
fn legato_glides_the_same_note() {
    let s = state(&[("glide", 100.0)], &[("voiceMode", "legato")]);
    let events = [note_on(0, 1, 60, 1.0), note_on(4_800, 2, 72, 1.0)];
    let out = bench_of(&s).render(19_200, 32, &events, true);
    // One note only, gliding from 60 to 72 over 100 ms (4800 frames), then at 72
    assert!(near(out[0][4_000], hz(60.0)));
    let mid = out[0][4_800 + 2_400];
    assert!(mid > hz(64.0) && mid < hz(68.0), "half way: {mid}");
    assert!(near(out[0][15_000], hz(72.0)), "{}", out[0][15_000]);
}

#[test]
fn a_new_synth_lets_the_old_one_finish_its_notes() {
    let a = state(&[], &[]);
    let b = state(&[("glide", 1.0)], &[]);
    let mut bench = bench_of(&a);
    let first = bench.render(4_800, 256, &[note_on(0, 1, 60, 1.0)], true);
    assert!(near(first[0][4_000], hz(60.0)));
    let registry = Registry::new(&[&PROBE]).expect("registry");
    let spec = ProcessSpec { sample_rate: RATE, max_block: 1024 };
    let r = crate::ResourceView::new();
    let cx = BuildContext { spec, resources: &r, channels: 2 };
    let new = registry.build("streamprobe", &b, &cx).expect("build");
    let (adopted, old) = bench.replace(new, true);
    assert_eq!(adopted, lm_render::Adopted::Yes);
    drop(old);
    // The held note goes on (on the old stream); a new note plays on the new one
    let events = [note_on(4_800 + 1_000, 2, 48, 1.0), note_off(4_800 + 3_000, 1, 60)];
    let second = bench.render(4_800, 256, &events, true);
    assert!(near(second[0][500], hz(60.0)), "{}", second[0][500]);
    let both = hz(60.0) + hz(48.0);
    assert!(near(second[0][2_000], both), "{} {both}", second[0][2_000]);
    assert!(near(second[0][4_000], hz(48.0)), "{}", second[0][4_000]);
}
