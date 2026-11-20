//! The instrument functions' behaviour, with a probe synth whose output is its pitch (a constant: `freq / 10000` on
//! both sides), so that the sum heard tells which keys sound

use lm_base::Result;
use lm_render::{EventKind, NoteId, ProcessSpec, Processor, Rng};
use lm_state::{NodeSchema, Value};

use super::*;
use crate::descriptor::{BuildContext, Descriptor, PluginKind, TestHints, Velocity};
use crate::registry::Registry;
use crate::testing::{Bench, Timed, note_off, note_on};
use crate::{Port, Ports};

#[derive(Clone, Debug)]
struct Probe;

impl Synth for Probe {
    type Voice = ();
    const PARAMS: usize = 0;
    fn voice(&self) {}
    fn set_param(&mut self, _: usize, _: f32) {}
    fn start(&self, _: &mut (), _: &NoteStart, _: &mut Rng) {}
    fn render(&self, _: &mut (), freq: f32, left: &mut [f32], right: &mut [f32]) {
        left.fill(freq / 10_000.0);
        right.fill(freq / 10_000.0);
    }
}

fn schemas() -> Vec<NodeSchema> {
    vec![add_instrument_functions(NodeSchema::new("plugin:probe", 1))]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let params = schemas().remove(0).params;
    Ok(Box::new(Instrument::new(Probe, &s, &params, &cx.spec, cx.resources, 16)?))
}

static PROBE: Descriptor = Descriptor {
    id: "probe",
    name: "plugin.probe.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    velocity: Velocity::Level,
    display: None,
    build,
    tests: TestHints::new(),
};

const RATE: u32 = 48_000;

/// A bench for the defaults with these parameters and fields changed
fn bench(params: &[(&str, f64)], fields: &[(&str, &str)]) -> Bench {
    let registry = Registry::new(&[&PROBE]).expect("registry");
    let mut p = Value::empty_object();
    for (k, v) in params {
        p = p.with(k, Value::real(*v).expect("real"));
    }
    let mut state = Value::empty_object().with("params", p);
    for (k, v) in fields {
        state = state.with(k, Value::text(*v));
    }
    let spec = ProcessSpec { sample_rate: RATE, max_block: 1024 };
    let resources = ResourceView::new();
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    Bench::instrument(registry.build("probe", &state, &cx).expect("build"), spec, 2, 1)
}

fn hz(key: f64) -> f32 {
    (440.0 * libm::exp2((key - 69.0) / 12.0)) as f32 / 10_000.0
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0e-5 * b.abs().max(1.0)
}

fn left(events: &[Timed], frames: usize, b: &mut Bench) -> Vec<f32> {
    b.render(frames, 256, events, true).swap_remove(0)
}

#[test]
fn a_note_sounds_its_key_after_the_fade_in() {
    let mut b = bench(&[], &[]);
    let x = left(&[note_on(0, 1, 69, 1.0)], 1000, &mut b);
    assert!(x[0] == 0.0 && x[10] < hz(69.0), "fades in");
    assert!(close(x[500], hz(69.0)), "{} {}", x[500], hz(69.0));
    let mut b = bench(&[("pitch", 50.0)], &[]);
    let x = left(&[note_on(0, 1, 69, 0.5)], 1000, &mut b);
    assert!(close(x[500], 0.5 * hz(69.5)), "pitch in cents and velocity as level");
}

#[test]
fn chords_stack_over_octaves() {
    let mut b = bench(&[("chordOn", 1.0), ("chordRange", 2.0)], &[("chord", "major")]);
    let x = left(&[note_on(0, 1, 60, 1.0)], 1000, &mut b);
    let expected: f32 = [60.0, 64.0, 67.0, 72.0, 76.0, 79.0].iter().map(|k| hz(*k)).sum();
    assert!(close(x[500], expected), "{} {expected}", x[500]);
}

#[test]
fn the_arpeggio_steps_through_the_chord() {
    let step = (0.025 * RATE as f64) as usize;
    let mut b = bench(&[("arpOn", 1.0), ("arpTime", 25.0)], &[("arpChord", "major")]);
    let x = left(&[note_on(0, 1, 60, 1.0)], 5 * step, &mut b);
    for (n, key) in [60.0, 64.0, 67.0, 60.0, 64.0].iter().enumerate() {
        let at = n * step + step / 2;
        assert!(close(x[at], hz(*key)), "step {n}: {} {}", x[at], hz(*key));
    }
    let mut b = bench(&[("arpOn", 1.0), ("arpTime", 25.0), ("arpGate", 50.0)], &[("arpDirection", "down")]);
    let x = left(&[note_on(0, 1, 60, 1.0)], 2 * step, &mut b);
    assert!(close(x[step / 4], hz(60.0)) && x[3 * step / 4].abs() < 1.0e-6, "a gate of half the step");
}

#[test]
fn mono_plays_the_last_key_held_and_goes_back() {
    let mut b = bench(&[], &[("voiceMode", "mono")]);
    let ev = [note_on(0, 1, 60, 1.0), note_on(2000, 2, 67, 1.0), note_off(4000, 2, 67), note_off(6000, 1, 60)];
    let x = left(&ev, 8000, &mut b);
    assert!(close(x[1500], hz(60.0)));
    assert!(close(x[3500], hz(67.0)), "only the last key sounds");
    assert!(close(x[5500], hz(60.0)), "back to the key still held");
    assert!(x[7900].abs() < 1.0e-6, "all released");
}

#[test]
fn legato_glides_the_same_voice() {
    let glide = 100.0;
    let mut b = bench(&[("glide", glide)], &[("voiceMode", "legato")]);
    let ev = [note_on(0, 1, 60, 1.0), note_on(2000, 2, 72, 1.0)];
    let x = left(&ev, 2000 + 4800 + 1000, &mut b);
    let half = x[2000 + 2400];
    assert!(half > hz(61.0) && half < hz(71.0), "half way through the glide: {half}");
    assert!(close(x[2000 + 4800 + 500], hz(72.0)), "arrived");
    // The voice did not start again: no fade-in at the new key
    assert!(x[2001] > 0.9 * hz(60.0));
}

#[test]
fn the_sustain_pedal_holds_released_notes() {
    let mut b = bench(&[], &[]);
    let ev = [
        Timed::new(0, EventKind::Midi { bytes: [0xB0, 64, 127] }),
        note_on(10, 1, 69, 1.0),
        note_off(1000, 1, 69),
        Timed::new(3000, EventKind::Midi { bytes: [0xB0, 64, 0] }),
    ];
    let x = left(&ev, 4000, &mut b);
    assert!(close(x[2500], hz(69.0)), "held by the pedal");
    assert!(x[3500].abs() < 1.0e-6, "released with the pedal");
}

#[test]
fn pitch_bend_follows_the_range() {
    let mut b = bench(&[("bendRange", 12.0)], &[]);
    let ev = [note_on(0, 1, 57, 1.0), Timed::new(1000, EventKind::Midi { bytes: [0xE0, 0x00, 0x60] })];
    let x = left(&ev, 2000, &mut b);
    // 0x60 << 7 = 12288: half way up, six semitones
    assert!(close(x[1500], hz(63.0)), "{} {}", x[1500], hz(63.0));
}

#[test]
fn notes_of_no_length_and_all_release() {
    let mut b = bench(&[], &[]);
    let ev = [
        note_on(100, 1, 60, 1.0),
        note_off(100, 1, 60),
        note_on(200, 2, 62, 1.0),
        note_on(201, 3, 64, 1.0),
        Timed::new(3000, EventKind::AllNotesRelease),
    ];
    let x = left(&ev, 4000, &mut b);
    assert!(close(x[1000], hz(62.0) + hz(64.0)), "the note of no length is gone");
    assert!(x[3900].abs() < 1.0e-6);
    let _ = NoteId(0);
}

// ---- the generated suite on the probe, and the registry ----

#[test]
fn the_probe_passes_the_generated_suite() {
    let d = &PROBE;
    crate::testing::defaults_round_trip(d);
    crate::testing::random_states_round_trip(d);
    crate::testing::no_hidden_state(d);
    crate::testing::adopt_while_playing(d);
    crate::testing::param_sweep(d);
    crate::testing::every_choice(d);
    crate::testing::short_notes(d);
    crate::testing::block_sizes(d);
    crate::testing::velocity(d);
    crate::testing::ports_follow_the_state(d);
}

fn panics(_: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    panic!("a build that panics")
}

static PANICS: Descriptor =
    Descriptor { id: "panics", build: panics, schemas: || vec![NodeSchema::new("plugin:panics", 1)], ..PROBE };

#[test]
fn the_registry() {
    let registry = Registry::new(&[&PROBE, &PANICS]).expect("registry");
    assert!(Registry::new(&[&PROBE, &PROBE]).is_err(), "an id twice");
    assert_eq!(registry.param_index("probe", "pitch"), Some(0));
    assert_eq!(registry.param_index("probe", "glide"), Some(p::GLIDE as u32));
    assert_eq!(registry.param_index("probe", "nothing"), None);
    let ports = registry.ports_fn();
    assert_eq!(ports("probe", &Value::empty_object()).map(|p| p.outputs.len()), Some(1));
    assert!(ports("nobody", &Value::empty_object()).is_none());
    let spec = ProcessSpec { sample_rate: RATE, max_block: 256 };
    let resources = ResourceView::new();
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let bad = Value::empty_object().with("chord", Value::text("no-such-chord"));
    assert!(registry.build("probe", &bad, &cx).is_err(), "an invalid state is refused");
    let e = registry.build("panics", &Value::empty_object(), &cx).err().map(|e| e.key());
    assert_eq!(e, Some("plugin.build.panicked"));
    let project = registry.project_schemas().expect("project schemas");
    assert!(project.find("plugin:probe").is_some() && project.find("project").is_some());
}

fn expression(at: u64, note: u32, expression: lm_render::Expression, value: f32, ramp: u32) -> Timed {
    Timed::new(at, EventKind::NoteExpression { note: NoteId(note), expression, value, ramp })
}

fn both(events: &[Timed], frames: usize, b: &mut Bench) -> (Vec<f32>, Vec<f32>) {
    let mut out = b.render(frames, 256, events, true);
    let right = out.swap_remove(1);
    (out.swap_remove(0), right)
}

#[test]
fn a_tuning_expression_moves_every_note_of_its_base_and_ramps() {
    use lm_render::Expression::Tuning;
    let mut b = bench(&[("chordOn", 1.0)], &[("chord", "major")]);
    let ev = [note_on(0, 1, 60, 1.0), note_on(0, 2, 72, 1.0), expression(1000, 1, Tuning, 2.0, 0)];
    let x = left(&ev, 2000, &mut b);
    let chord = |root: f64| -> f32 { [0.0, 4.0, 7.0].iter().map(|i| hz(root + i)).sum() };
    assert!(close(x[900], chord(60.0) + chord(72.0)));
    assert!(close(x[1500], chord(62.0) + chord(72.0)), "the base's chord moves, the other note stays");
    // A ramp of 3200 frames: half way at 1600 (by piece: the value at the piece's start)
    let mut b = bench(&[], &[]);
    let ev = [note_on(0, 1, 69, 1.0), expression(1024, 1, Tuning, 12.0, 3200)];
    let x = left(&ev, 6000, &mut b);
    assert!(close(x[1024 + 1600], hz(75.0)), "{} {}", x[1024 + 1600], hz(75.0));
    assert!(close(x[5000], hz(81.0)), "arrived");
    // A note started later by the same base (an arpeggio step) has the base's tuning
    let step = (0.025 * RATE as f64) as usize;
    let mut b = bench(&[("arpOn", 1.0), ("arpTime", 25.0)], &[("arpChord", "major")]);
    let ev = [note_on(0, 1, 60, 1.0), expression(10, 1, Tuning, 1.0, 0)];
    let x = left(&ev, 3 * step, &mut b);
    assert!(close(x[step + step / 2], hz(65.0)), "the second step, a semitone up");
}

#[test]
fn a_pan_expression_is_a_balance_after_the_velocity() {
    use lm_render::Expression::Pan;
    let mut b = bench(&[], &[]);
    let ev = [note_on(0, 1, 69, 0.5), expression(1024, 1, Pan, 0.5, 0), expression(2048, 1, Pan, -1.0, 0)];
    let (l, r) = both(&ev, 3000, &mut b);
    let full = 0.5 * hz(69.0);
    assert!(close(l[900], full) && close(r[900], full));
    assert!(close(l[1500], 0.5 * full) && close(r[1500], full), "right: the left at 1 - p");
    assert!(close(l[2500], full) && r[2500].abs() < 1.0e-7, "hard left: the right silent");
    // A ramp moves sample by sample: never a step
    let mut b = bench(&[], &[]);
    let ev = [note_on(0, 1, 69, 1.0), expression(1024, 1, Pan, 1.0, 1000)];
    let (l, _) = both(&ev, 3000, &mut b);
    let steps = (1025..2024).map(|i| (l[i] - l[i - 1]).abs()).fold(0.0f32, f32::max);
    assert!(steps < 2.0 * hz(69.0) / 1000.0, "the largest step {steps}");
    assert!(close(l[1524], 0.5 * hz(69.0)) && l[2500].abs() < 1.0e-7);
}

#[test]
fn expressions_do_not_depend_on_the_block_size() {
    use lm_render::Expression::{Pan, Tuning};
    let ev = [note_on(0, 1, 60, 1.0), expression(333, 1, Pan, 0.7, 777), expression(1111, 1, Tuning, -3.5, 1234)];
    let a = bench(&[], &[]).render(4096, 32, &ev, true);
    let b = bench(&[], &[]).render(4096, 1024, &ev, true);
    assert_eq!(a, b);
}

/// A synth whose voice ends by itself after 100 frames
#[derive(Clone, Debug)]
struct Short;

impl Synth for Short {
    type Voice = u32;
    const PARAMS: usize = 0;
    fn voice(&self) -> u32 {
        0
    }
    fn set_param(&mut self, _: usize, _: f32) {}
    fn start(&self, v: &mut u32, _: &NoteStart, _: &mut Rng) {
        *v = 0;
    }
    fn render(&self, v: &mut u32, _: f32, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let x = if *v < 100 { 0.5 } else { 0.0 };
            *l = x;
            *r = x;
            *v += 1;
        }
    }
    fn finished(&self, v: &u32) -> bool {
        *v >= 100
    }
}

#[test]
fn a_voice_the_synth_ends_is_freed_without_a_note_off() {
    let registry = Registry::new(&[&PROBE]).expect("registry");
    let complete = registry.complete("probe", &Value::empty_object()).expect("complete");
    let params = schemas().remove(0).params;
    let spec = ProcessSpec { sample_rate: RATE, max_block: 1024 };
    let inst = Instrument::new(Short, &StateView::new(&complete), &params, &spec, &ResourceView::new(), 4).expect("ok");
    let mut b = Bench::instrument(Box::new(inst), spec, 2, 1);
    let x = b.render(256, 256, &[note_on(0, 1, 60, 1.0)], true).swap_remove(0);
    assert!(x[50] > 0.0 && x[200] == 0.0);
    let inst = b.processor_mut().as_any_mut().downcast_mut::<Instrument<Short>>().expect("instrument");
    assert_eq!(inst.active_voices(), 0, "its only voice ended at 100 frames although the note is held");
}
