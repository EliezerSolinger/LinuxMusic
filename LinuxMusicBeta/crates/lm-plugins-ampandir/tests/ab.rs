//! The A/B of Amp & IR against the legacy plugin itself: `ab/reference/<case>.f32` is what the legacy program's
//! `libampandir.so`, as it was built, wrote for each case of `ab/cases.json` (`ab/build-references.sh`), with the
//! IR files and models of `ab/data` loaded through its own settings. Here the same input goes through the plugin
//! built from the same knobs and files, by the contract's bench, in the same blocks.
//!
//! The IR reaches the plugin as the project gives it: decoded and, when its rate is not the project's, converted
//! as `lm-io` imports it (libsamplerate's best sinc, the whole file at once); the legacy plugin converted it with the
//! same converter.
//!
//! The rule (U34): the difference stays below −90 dBFS (or 90 dB below the reference's peak above full scale).

#![allow(clippy::print_stderr)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lm_base::{Cancel, Progress};
use lm_plugin::testing::{Bench, Timed};
use lm_plugin::{BuildContext, Loaded, Registry, ResourceView};
use lm_plugins_ampandir::DESCRIPTOR;
use lm_render::{EventKind, ProcessSpec, SampleData};
use lm_state::{PARAMS_KEY, Resource, Value};
use serde_json::Value as Json;

/// -90 dBFS
const NULL: f32 = 3.162_277_7e-5;

fn ab() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ab")
}

fn split_mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The harness's signal
struct Signal {
    sine: bool,
    hz: i64,
    amp: f64,
    state: u64,
    rate: i64,
    steps: Vec<(i64, f64)>,
}

impl Signal {
    fn new(o: &Json, rate: i64) -> Self {
        Self {
            sine: o["kind"] == "sine",
            hz: o["hz"].as_f64().unwrap_or(0.0) as i64,
            amp: o["amp"].as_f64().unwrap_or(0.0),
            state: o["seed"].as_f64().unwrap_or(0.0) as u64,
            rate,
            steps: o["steps"]
                .as_array()
                .map(|a| a.iter().map(|p| (p[0].as_f64().unwrap() as i64, p[1].as_f64().unwrap())).collect())
                .unwrap_or_default(),
        }
    }

    fn at(&mut self, n: i64) -> f32 {
        let value = if self.sine {
            let phase = ((n * self.hz) % self.rate) as f64 / self.rate as f64;
            let x = phase * 2.0 - 1.0;
            let a = 4.0 * x;
            let b = 4.0 * x * x.abs();
            let y = a - b;
            0.225 * (y * y.abs() - y) + y
        } else {
            self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            (split_mix(self.state) >> 40) as f64 / 16_777_216.0 * 2.0 - 1.0
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
    static INPUT: RefCell<Vec<[f32; 2]>> = const { RefCell::new(Vec::new()) };
}

fn input(frame: u64, ch: usize, _rate: u32) -> f32 {
    INPUT.with(|i| i.borrow().get(frame as usize).map_or(0.0, |f| f[ch.min(1)]))
}

/// An IR file as the project holds it: decoded, at the project's rate (lm-io's import)
fn impulse(path: &Path, rate: u32) -> SampleData {
    let mut reader = lm_sys_sndfile::Reader::open(path).unwrap();
    let info = *reader.info();
    let mut all = vec![0.0f32; info.frames as usize * info.channels];
    let n = reader.read_f32(&mut all).unwrap();
    all.truncate(n * info.channels);
    if info.rate != rate {
        all = lm_sys_samplerate::convert(
            &all,
            info.channels,
            info.rate,
            rate,
            lm_sys_samplerate::Quality::Best,
            &Cancel::new(),
            &Progress::new(),
        )
        .unwrap();
    }
    let channels = (0..info.channels).map(|c| all.iter().skip(c).step_by(info.channels).copied().collect()).collect();
    SampleData::from_channels(rate, channels).unwrap()
}

fn reference(name: &str) -> Vec<[f32; 2]> {
    let bytes = std::fs::read(ab().join("reference").join(format!("{name}.f32"))).unwrap();
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| [f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]])])
        .collect()
}

/// The worst difference of the case through the plugin, relative to full scale
fn run(name: &str) -> f32 {
    let text = std::fs::read_to_string(ab().join("cases.json")).unwrap();
    let cases: Json = serde_json::from_str(&text).unwrap();
    let c = cases["cases"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap().clone();
    let frames = c["frames"].as_u64().unwrap() as usize;
    let block = c["block"].as_u64().unwrap() as usize;
    let rate = c["rate"].as_u64().unwrap() as u32;
    let signal = &c["signal"];
    let mut left = Signal::new(signal, i64::from(rate));
    let mut right = signal.get("right").map(|s| Signal::new(s, i64::from(rate)));
    let data: Vec<[f32; 2]> = (0..frames as i64)
        .map(|n| {
            let l = left.at(n);
            [l, right.as_mut().map_or(l, |r| r.at(n))]
        })
        .collect();
    INPUT.with(|i| *i.borrow_mut() = data);

    let settings = &c["settings"];
    let mut params = Value::empty_object();
    for key in ["input", "output"] {
        if let Some(v) = settings[key].as_f64() {
            params = params.with(key, Value::real(v).unwrap());
        }
    }
    let mut state = Value::empty_object().with(PARAMS_KEY, params);
    let mut resources = ResourceView::new();
    if let Some(file) = settings["ir"].as_str() {
        let path = ab().join("data").join(file);
        let r = Resource::of_content("audio", &std::fs::read(&path).unwrap()).unwrap();
        resources.insert(r.clone(), Loaded::Audio(Arc::new(impulse(&path, rate))));
        state = state.with("ir", Value::resource(r));
    }
    if let Some(file) = settings["nam"].as_str() {
        let bytes = std::fs::read(ab().join("data").join(file)).unwrap();
        let r = Resource::of_content("nam", &bytes).unwrap();
        resources.insert(r.clone(), Loaded::Bytes(Arc::from(bytes.into_boxed_slice())));
        state = state.with("nam", Value::resource(r));
    }
    let registry = Registry::new(&[&DESCRIPTOR]).unwrap();
    let spec = ProcessSpec { sample_rate: rate, max_block: block };
    let cx = BuildContext { spec, resources: &resources, channels: 2 };
    let processor = registry.build(DESCRIPTOR.id, &state, &cx).unwrap_or_else(|e| panic!("{name}: {e}"));

    let mut events = Vec::new();
    if let Some(at) = c["then"]["at"].as_u64() {
        let schema = DESCRIPTOR.schema();
        for (key, value) in c["then"]["settings"].as_object().unwrap() {
            let index = schema.params.iter().position(|s| s.key == key).unwrap() as u32;
            events.push(Timed::new(at, EventKind::Param { index, value: value.as_f64().unwrap() as f32 }));
        }
    }
    let out = Bench::effect(processor, spec, 2, 0, input).render(frames, block, &events, false);
    let expected = reference(name);
    assert_eq!(expected.len(), frames);
    let scale = expected.iter().fold(1.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    let (mut worst, mut worst_at, mut exact) = (0.0f32, 0, true);
    for (n, want) in expected.iter().enumerate() {
        for ch in 0..2 {
            exact &= out[ch][n].to_bits() == want[ch].to_bits();
            let d = (out[ch][n] - want[ch]).abs() / scale;
            if d > worst || d.is_nan() {
                worst = d;
                worst_at = n;
            }
        }
    }
    let db = if worst > 0.0 { 20.0 * worst.log10() } else { -400.0 };
    eprintln!("{name:<20} worst {db:7.1} dBFS at frame {worst_at}{}", if exact { " (bit for bit)" } else { "" });
    worst
}

macro_rules! case {
    ($test:ident, $name:literal) => {
        #[test]
        fn $test() {
            let worst = run($name);
            assert!(worst <= NULL, "{}: the plugin differs from the legacy plugin by {worst}", $name);
        }
    };
}

case!(gains, "gains");
case!(ir_mono, "ir-mono");
case!(ir_stereo, "ir-stereo");
case!(ir_resampled, "ir-resampled");
case!(amp_at_the_engine_rate, "amp");
case!(amp_resampled, "amp-resampled");
case!(amp_and_ir, "amp-and-ir");
case!(amp_and_ir_with_gains, "amp-and-ir-gains");
