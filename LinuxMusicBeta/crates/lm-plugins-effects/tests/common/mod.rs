//! The A/B of the effect plugins against the legacy program, through the plugin: the cases and references of
//! `lm-dsp/ab` (written by the legacy plugins themselves, `lm-dsp/ab/build-references.sh`), the same input signal,
//! the plugin built from a state that says what the case's legacy knobs said, run by the contract's bench.
//!
//! The rule is lm-dsp's (U34): the difference stays below -90 dBFS, or 90 dB below the reference's peak when that
//! goes above full scale.

#![allow(dead_code, clippy::print_stderr, clippy::neg_cmp_op_on_partial_ord)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use lm_plugin::testing::{Bench, Timed, test_resources};
use lm_plugin::{BuildContext, Descriptor, Registry};
use lm_render::{EventKind, ProcessSpec};
use lm_state::{PARAMS_KEY, Value};
use serde_json::Value as Json;

/// -90 dBFS
pub const NULL: f32 = 3.162_277_7e-5;

fn ab_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lm-dsp/ab")
}

fn split_mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The test signal of `lm-dsp/ab` (the same code is in its `legacy-harness.cpp`)
struct Signal {
    sine: bool,
    hz: i64,
    amp: f64,
    state: u64,
    steps: Vec<(i64, f64)>,
}

impl Signal {
    fn new(o: &Json) -> Self {
        Self {
            sine: o["kind"] == "sine",
            hz: o["hz"].as_f64().unwrap_or(0.0) as i64,
            amp: o["amp"].as_f64().unwrap_or(0.0),
            state: o["seed"].as_f64().unwrap_or(0.0) as u64,
            steps: o["steps"]
                .as_array()
                .map(|a| a.iter().map(|p| (p[0].as_f64().unwrap() as i64, p[1].as_f64().unwrap())).collect())
                .unwrap_or_default(),
        }
    }

    fn at(&mut self, n: i64) -> f32 {
        let value = if self.sine {
            let phase = ((n * self.hz) % 48000) as f64 / 48000.0;
            let x = phase * 2.0 - 1.0;
            let a = 4.0 * x;
            let b = 4.0 * x * x.abs();
            let y = a - b;
            0.225 * (y * y.abs() - y) + y
        } else {
            self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let z = split_mix(self.state);
            (z >> 40) as f64 / 16_777_216.0 * 2.0 - 1.0
        };
        let mut gain = 1.0;
        for &(frame, g) in &self.steps {
            if frame <= n {
                gain = g;
            }
        }
        (value * self.amp * gain) as f32
    }
}

thread_local! {
    /// The case's input, read by the bench through a plain function
    static INPUT: RefCell<Vec<[f32; 2]>> = const { RefCell::new(Vec::new()) };
}

fn input(frame: u64, ch: usize, _rate: u32) -> f32 {
    INPUT.with(|i| i.borrow().get(frame as usize).map_or(0.0, |f| f[ch.min(1)]))
}

/// A case of `lm-dsp/ab/cases.json`
pub fn case(name: &str) -> Json {
    let text = std::fs::read_to_string(ab_dir().join("cases.json")).unwrap();
    let cases: Json = serde_json::from_str(&text).unwrap();
    cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no case {name}"))
        .clone()
}

fn reference(name: &str) -> Vec<[f32; 2]> {
    let bytes = std::fs::read(ab_dir().join("reference").join(format!("{name}.f32")))
        .unwrap_or_else(|e| panic!("{name}: no reference ({e})"));
    let (frames, rest) = bytes.as_chunks::<8>();
    assert!(rest.is_empty());
    frames
        .iter()
        .map(|c| [f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]])])
        .collect()
}

/// How a legacy knob of a case becomes the plugin's state
#[derive(Clone, Copy)]
pub enum Knob {
    /// A parameter, with the legacy value as it is
    Param(&'static str),
    /// A parameter, with the legacy value mapped
    ParamBy(&'static str, fn(f64) -> f64),
    /// A choice field: the legacy index (or value) to the key
    Choice(&'static str, fn(f64) -> &'static str),
    /// A boolean field
    Bool(&'static str),
}

/// The plugin's state for the legacy knobs @p settings (the defaults for the rest)
pub fn state_for(settings: &Json, knobs: &[(&str, Knob)]) -> Value {
    let mut params = Value::empty_object();
    let mut state = Value::empty_object();
    if let Some(map) = settings.as_object() {
        for (legacy, value) in map {
            let v = value.as_f64().unwrap_or_else(|| if value.as_bool() == Some(true) { 1.0 } else { 0.0 });
            let Some((_, knob)) = knobs.iter().find(|(k, _)| k == legacy) else {
                panic!("the legacy knob {legacy:?} has no mapping");
            };
            match *knob {
                Knob::Param(key) => params = params.with(key, Value::real(v).unwrap()),
                Knob::ParamBy(key, map) => params = params.with(key, Value::real(map(v)).unwrap()),
                Knob::Choice(key, map) => state = state.with(key, Value::text(map(v))),
                Knob::Bool(key) => state = state.with(key, Value::bool(v != 0.0)),
            }
        }
    }
    state.with(PARAMS_KEY, params)
}

/// Runs the case through the plugin and returns the worst difference from the legacy reference over the frames from
/// @p from. With @p follow_then the case's change of settings is sent as parameter events at its frame and the whole
/// case is compared; without, the comparison stops at the change
pub fn run(d: &'static Descriptor, name: &str, knobs: &[(&str, Knob)], from: usize, follow_then: bool) -> f32 {
    compare(d, name, knobs, from, usize::MAX, follow_then, &|_, s| s)
}

/// [`run`], with the state built from the case's knobs then completed by @p extend (a graph, a curve: what the
/// case says outside its `settings`)
pub fn run_extended(
    d: &'static Descriptor,
    name: &str,
    knobs: &[(&str, Knob)],
    from: usize,
    follow_then: bool,
    extend: &dyn Fn(&Json, Value) -> Value,
) -> f32 {
    compare(d, name, knobs, from, usize::MAX, follow_then, extend)
}

/// [`run`] over the frames @p from to @p to (at most up to the case's change of settings)
pub fn run_window(d: &'static Descriptor, name: &str, knobs: &[(&str, Knob)], from: usize, to: usize) -> f32 {
    compare(d, name, knobs, from, to, false, &|_, s| s)
}

fn compare(
    d: &'static Descriptor,
    name: &str,
    knobs: &[(&str, Knob)],
    from: usize,
    until: usize,
    follow_then: bool,
    extend: &dyn Fn(&Json, Value) -> Value,
) -> f32 {
    let c = case(name);
    let frames = c["frames"].as_u64().unwrap() as usize;
    let block = c["block"].as_u64().unwrap() as usize;
    let rate = c["rate"].as_f64().unwrap_or(48_000.0) as u32;
    let signal = &c["signal"];
    let mut left = Signal::new(signal);
    let mut right = signal.get("right").map(Signal::new);
    let data: Vec<[f32; 2]> = (0..frames as i64)
        .map(|n| {
            let l = left.at(n);
            [l, right.as_mut().map_or(l, |r| r.at(n))]
        })
        .collect();
    INPUT.with(|i| *i.borrow_mut() = data);

    let registry = Registry::new(&[d]).unwrap();
    let state = extend(&c, state_for(&c["settings"], knobs));
    let complete = registry.complete(d.id, &state).unwrap();
    let resources = test_resources(registry.schemas(), &d.type_name(), &complete, rate);
    let spec = ProcessSpec { sample_rate: rate, max_block: block.max(32) };
    let cx = BuildContext { spec, resources: &resources, channels: 2 };
    let processor = registry.build(d.id, &state, &cx).unwrap_or_else(|e| panic!("{name}: {e}"));

    let mut events = Vec::new();
    let mut to = frames;
    if let Some(at) = c["then"]["at"].as_u64() {
        if follow_then {
            let schema = d.schema();
            let changed = state_for(&c["then"]["settings"], knobs);
            if let Some(p) = changed.get(PARAMS_KEY) {
                for (key, value) in p.entries() {
                    let index = schema.params.iter().position(|s| s.key == key).unwrap();
                    let value = value.as_real().unwrap() as f32;
                    events.push(Timed::new(at, EventKind::Param { index: index as u32, value }));
                }
            }
        } else {
            to = at as usize;
        }
    }
    let mut bench = Bench::effect(processor, spec, 2, 0, input);
    let out = bench.render(frames, block, &events, false);
    let expected = reference(name);
    assert_eq!(expected.len(), frames, "{name}: the reference's length");
    let to = to.min(until);
    let peak = expected[from..to].iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    let scale = peak.max(1.0);
    let mut worst = 0.0f32;
    let mut worst_at = 0;
    for (n, want) in expected.iter().enumerate().take(to).skip(from) {
        for ch in 0..2 {
            let d = (out[ch][n] - want[ch]).abs() / scale;
            if !(d <= worst) {
                worst = d;
                worst_at = n;
            }
        }
    }
    let db = if worst > 0.0 { 20.0 * worst.log10() } else { -400.0 };
    eprintln!("{name:<32} through the plugin: worst {db:7.1} dBFS at frame {worst_at} (frames {from}..{to})");
    worst
}

/// Asserts that the case, up to its change of settings, nulls through the plugin
pub fn nulls(d: &'static Descriptor, name: &str, knobs: &[(&str, Knob)]) {
    nulls_from(d, name, knobs, 0);
}

/// The same from frame @p from (the legacy harness ramps some knobs from their defaults over the first period)
pub fn nulls_from(d: &'static Descriptor, name: &str, knobs: &[(&str, Knob)], from: usize) {
    let worst = run(d, name, knobs, from, false);
    assert!(worst <= NULL, "{name}: the plugin differs from the legacy plugin by {worst}");
}
