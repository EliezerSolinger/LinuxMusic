//! The A/B of every unit against the legacy Guitarix plugin itself (`ab/`: the legacy `libguitarix.so`, rebuilt
//! without fused multiply-adds as lm-sys-guitarix compiles Guitarix's code, run by `ab/legacy-harness.cpp` as the
//! legacy mixer runs an effect; `ab/build-references.sh` writes `ab/reference`). The same input, the plugin built
//! from a state that says what the case's legacy settings said, run by the contract's bench in blocks of the case's
//! period, its latency taken off (a unit in fixed blocks answers one block later).
//!
//! The rule (U34): bit for bit, or the difference below -90 dBFS (90 dB below the reference's peak when that goes
//! above full scale). `AB_PRINT=1` prints every case's worst difference.

#![allow(clippy::print_stderr, clippy::neg_cmp_op_on_partial_ord)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use lm_plugin::testing::{Bench, Timed};
use lm_plugin::{BuildContext, Registry, ResourceView};
use lm_plugins_guitarix::{ALL, plan};
use lm_render::{EventKind, ProcessSpec};
use lm_state::{PARAMS_KEY, Value};
use serde_json::Value as Json;

/// -90 dBFS
const NULL: f32 = 3.162_277_7e-5;
const RATE: i64 = 48_000;

fn ab_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ab")
}

fn split_mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The harness's signal (the same code is in ab/legacy-harness.cpp)
struct Signal {
    sine: bool,
    hz: i64,
    amp: f64,
    state: u64,
}

impl Signal {
    fn new(o: &Json) -> Self {
        Self {
            sine: o["kind"] == "sine",
            hz: o["hz"].as_f64().unwrap_or(0.0) as i64,
            amp: o["amp"].as_f64().unwrap_or(0.0),
            state: o["seed"].as_f64().unwrap_or(0.0) as u64,
        }
    }

    fn at(&mut self, n: i64) -> f32 {
        let value = if self.sine {
            let phase = ((n * self.hz) % RATE) as f64 / RATE as f64;
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
        let t = n % (RATE / 2);
        let envelope = 1.0 / (1.0 + t as f64 * (8.0 / RATE as f64));
        (value * self.amp * envelope) as f32
    }
}

thread_local! {
    static INPUT: RefCell<Vec<[f32; 2]>> = const { RefCell::new(Vec::new()) };
}

fn input(frame: u64, ch: usize, _rate: u32) -> f32 {
    INPUT.with(|i| i.borrow().get(frame as usize).map_or(0.0, |f| f[ch.min(1)]))
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

/// The state for the legacy settings `settings` ({Guitarix id: value}) of `unit`
fn state_for(unit: &str, settings: &Json) -> Value {
    let plan = plan(unit);
    let mut params = Value::empty_object();
    let mut state = Value::empty_object();
    for (id, v) in settings.as_object().into_iter().flatten() {
        let v = v.as_f64().unwrap();
        let desc = plan.unit.param(id).unwrap_or_else(|| panic!("{unit}: no parameter {id}"));
        if let Some(i) = plan.param_desc.iter().position(|&d| d == desc) {
            params = params.with(plan.params[i].key, Value::real(v).unwrap());
        } else if let Some(c) = plan.choices.iter().find(|c| c.desc == desc) {
            let at = (v as f32 - plan.unit.params[desc].low).round() as usize;
            state = state.with(c.key, Value::text(c.keys[at]));
        } else if let Some(r) = plan.reals.iter().find(|r| r.desc == desc) {
            state = state.with(r.key, Value::real(v).unwrap());
        } else {
            panic!("{unit}: {id} is an output");
        }
    }
    state.with(PARAMS_KEY, params)
}

/// The worst difference of the case, in dBFS
fn run(c: &Json) -> f32 {
    let name = c["name"].as_str().unwrap();
    let unit = c["unit"].as_str().unwrap();
    let frames = c["frames"].as_u64().unwrap() as usize;
    let block = c["block"].as_u64().unwrap() as usize;
    let mut left = Signal::new(&c["signal"]);
    let mut right = c["signal"].get("right").map(Signal::new);
    let data: Vec<[f32; 2]> = (0..frames as i64)
        .map(|n| {
            let l = left.at(n);
            [l, right.as_mut().map_or(l, |r| r.at(n))]
        })
        .collect();
    INPUT.with(|i| *i.borrow_mut() = data);

    let d = ALL.iter().find(|(u, _)| *u == unit).unwrap().1;
    let registry = Registry::new(&[d]).unwrap();
    let state = state_for(unit, &c["settings"]);
    let spec = ProcessSpec { sample_rate: RATE as u32, max_block: block.max(32) };
    let resources = ResourceView::new();
    let cx = BuildContext { spec, resources: &resources, channels: 2 };
    let processor = registry.build(d.id, &state, &cx).unwrap_or_else(|e| panic!("{name}: {e}"));
    let latency = processor.latency() as usize;

    let mut events = Vec::new();
    if let Some(at) = c["then"]["at"].as_u64() {
        let schema = d.schema();
        let changed = state_for(unit, &c["then"]["settings"]);
        for (key, value) in changed.get(PARAMS_KEY).unwrap().entries() {
            let index = schema.params.iter().position(|s| s.key == key).unwrap();
            let value = value.as_real().unwrap() as f32;
            events.push(Timed::new(at, EventKind::Param { index: index as u32, value }));
        }
    }
    let mut bench = Bench::effect(processor, spec, 2, 0, input);
    let out = bench.render(frames + latency, block, &events, false);
    let expected = reference(name);
    assert_eq!(expected.len(), frames, "{name}: the reference's length");
    let peak = expected.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    let scale = peak.max(1.0);
    let mut worst = 0.0f32;
    for (n, want) in expected.iter().enumerate() {
        for ch in 0..2 {
            let d = (out[ch][n + latency] - want[ch]).abs() / scale;
            if !(d <= worst) {
                worst = d;
            }
        }
    }
    let silent = peak < 1.0e-4;
    assert!(!silent, "{name}: the reference is silent (the case shows nothing)");
    if worst > 0.0 { 20.0 * worst.log10() } else { -400.0 }
}

#[test]
fn every_unit_against_the_legacy_plugin() {
    let text = std::fs::read_to_string(ab_dir().join("cases.json")).unwrap();
    let cases: Json = serde_json::from_str(&text).unwrap();
    let only = std::env::var("AB_CASE").ok();
    let print = std::env::var("AB_PRINT").is_ok();
    let mut failures = Vec::new();
    for c in cases["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        if only.as_deref().is_some_and(|o| !name.starts_with(o)) {
            continue;
        }
        let db = run(c);
        if print {
            eprintln!("{name:<48} worst {db:7.1} dBFS");
        }
        if !(db <= 20.0 * NULL.log10()) {
            failures.push(format!("{name}: {db:.1} dBFS"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
