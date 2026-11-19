//! The A/B of lm-dsp against the legacy program (interfaces.md, U34): every case of `ab/cases.json` is run here and
//! compared with what the legacy code gave (`ab/reference/`, written by `ab/build-references.sh`).
//!
//! The rule: the difference stays below -90 dBFS at every sample (a null test), unless the case is listed in
//! `INTENTIONAL` with the reason the port differs on purpose.

// The report goes to stderr; and `!(d <= worst)` on purpose, so that a NaN counts as the worst difference
#![allow(clippy::print_stderr, clippy::neg_cmp_op_on_partial_ord)]

use std::path::{Path, PathBuf};

use lm_dsp::channel_strip::{ChannelStrip, InsertPoint, StripSettings};
use lm_dsp::dynamics::{CompressorModel, CompressorSettings, CompressorSpeed, GateMode, GateSettings};
use lm_dsp::gain_stage::GainModel;
use lm_dsp::master::{
    BusCompressor, BusCompressorSettings, BusRatio, MasterAnalog, MonitorSettings, Monitoring, PreFaders,
    StereoSection, StereoSettings,
};
use lm_dsp::tolerance::ChannelTolerance;
use serde_json::Value;

const RATE: f32 = 48000.0;
/// -90 dBFS
const NULL: f32 = 3.162_277_7e-5;

/// Cases where the port differs from the legacy program on purpose, with the largest difference allowed and why
const INTENTIONAL: &[(&str, f32, &str)] = &[(
    "strip-12at7",
    0.0316, // -30 dBFS
    "The pot's gain at 0.4 is dbfs_to_amp(12 dB): the libm crate's expf gives one ulp less than Apple's (V2 chose \
     libm, for the same samples on every system), and the 12AT7's light model, this hard driven, amplifies that ulp \
     into a difference of about -37 dBFS. strip-12at7-exact-gain, at 0.45 where the two exp agree, nulls bit for bit",
)];

fn ab_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ab")
}

// ----------------------------------------------------------------------------------------------------------------
// The signals (the same code is in ab/legacy-harness.cpp)

fn split_mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

struct Signal {
    sine: bool,
    hz: i64,
    amp: f64,
    state: u64,
    steps: Vec<(i64, f64)>,
}

impl Signal {
    fn new(o: &Value) -> Self {
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
            // A parabolic sine, refined: + and * only
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

// ----------------------------------------------------------------------------------------------------------------
// The settings, as the harness reads them

fn f(o: &Value, key: &str, default: f32) -> f32 {
    o[key].as_f64().map_or(default, |v| v as f32)
}

fn strip_settings(s: &Value) -> (StripSettings, ChannelTolerance) {
    let mut settings = StripSettings::default();
    settings.gain = f(s, "gain", settings.gain);
    if let Some(key) = s["drive"].as_str() {
        settings.drive = GainModel::from_key(key).expect("a gain model");
    }
    settings.realistic = s["realistic"].as_bool().unwrap_or(false);
    settings.low_cut = s["lowCut"].as_bool().unwrap_or(false);
    settings.high_pass = f(s, "highPass", settings.high_pass);
    settings.low_pass = f(s, "lowPass", settings.low_pass);
    settings.mono_bass = f(s, "monoBass", settings.mono_bass);
    settings.mono_bass_frequency = f(s, "monoBassFrequency", settings.mono_bass_frequency);
    if let Some(c) = s.get("compressor") {
        settings.compressor = CompressorSettings {
            model: CompressorModel::from_key(c["model"].as_str().unwrap()).expect("a compressor"),
            amount: f(c, "amount", 0.0),
            ratio: f(c, "ratio", 0.0),
            speed: match c["speed"].as_str() {
                Some("fast") => CompressorSpeed::Fast,
                Some("slow") => CompressorSpeed::Slow,
                _ => CompressorSpeed::Norm,
            },
            release: f(c, "release", 0.5),
        };
    }
    if let Some(g) = s.get("gate") {
        settings.gate = GateSettings {
            threshold: f(g, "threshold", -10.0),
            range: f(g, "range", 0.0),
            mode: GateMode::from_key(g["mode"].as_str().unwrap()).expect("a gate mode"),
        };
    }
    if let Some(eq) = s["eq"].as_array() {
        for (band, gain) in eq.iter().enumerate() {
            settings.eq[band] = gain.as_f64().unwrap() as f32;
        }
    }
    let channel = s["channel"].as_i64().unwrap_or(0) as i32;
    let tolerance = if channel > 0 { ChannelTolerance::for_channel(channel) } else { ChannelTolerance::NONE };
    (settings, tolerance)
}

struct Master {
    pre: PreFaders,
    analog: MasterAnalog,
    stereo: StereoSection,
    bus: BusCompressor,
    monitoring: Monitoring,
    pre_gains: [f32; 2],
    drive: Option<GainModel>,
    realistic: bool,
    stereo_settings: StereoSettings,
    bus_settings: BusCompressorSettings,
    transformer: bool,
    monitor: MonitorSettings,
}

impl Master {
    fn new() -> Self {
        Self {
            pre: PreFaders::default(),
            analog: MasterAnalog::new(RATE),
            stereo: StereoSection::new(RATE),
            bus: BusCompressor::new(RATE),
            monitoring: Monitoring::default(),
            pre_gains: [1.0; 2],
            drive: None,
            realistic: false,
            stereo_settings: StereoSettings::default(),
            bus_settings: BusCompressorSettings::default(),
            transformer: false,
            monitor: MonitorSettings::default(),
        }
    }

    fn apply(&mut self, s: &Value) {
        self.pre_gains =
            s["pre"].as_array().map_or([1.0; 2], |p| [p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32]);
        self.drive = s["drive"].as_str().map(|k| GainModel::from_key(k).expect("a gain model"));
        self.realistic = s["realistic"].as_bool().unwrap_or(false);
        let st = &s["stereo"];
        self.stereo_settings = StereoSettings {
            width: f(st, "width", 100.0),
            mono_bass: f(st, "monoBass", 0.0),
            bass_frequency: f(st, "bassFrequency", 120.0),
            air: f(st, "air", 0.0),
        };
        let b = &s["bus"];
        let defaults = BusCompressorSettings::default();
        self.bus_settings = BusCompressorSettings {
            on: b["on"].as_bool().unwrap_or(false),
            threshold: f(b, "threshold", defaults.threshold),
            ratio: match b["ratio"].as_i64().unwrap_or(1) {
                0 => BusRatio::Two,
                2 => BusRatio::Ten,
                _ => BusRatio::Four,
            },
            attack: b["attack"].as_u64().map_or(defaults.attack, |v| v as usize),
            release: b["release"].as_u64().map_or(defaults.release, |v| v as usize),
            makeup: f(b, "makeup", 0.0),
        };
        self.transformer = s["transformer"].as_bool().unwrap_or(false);
        let m = &s["monitor"];
        self.monitor = MonitorSettings {
            level: f(m, "level", 0.0),
            dim: m["dim"].as_bool().unwrap_or(false),
            cut: m["cut"].as_bool().unwrap_or(false),
            mono: m["mono"].as_bool().unwrap_or(false),
            side: m["side"].as_bool().unwrap_or(false),
        };
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        self.pre.process(frames, self.pre_gains);
        if let Some(model) = self.drive {
            self.analog.process_drive(frames, model, self.realistic);
        }
        self.stereo.process(frames, &self.stereo_settings);
        self.bus.process(frames, &self.bus_settings);
        if self.transformer {
            self.analog.process_transformer(frames);
        }
        self.monitoring.process(frames, &self.monitor);
    }
}

fn run(case: &Value) -> Vec<[f32; 2]> {
    let frames = case["frames"].as_i64().unwrap();
    let block = case["block"].as_i64().unwrap();
    let signal = &case["signal"];
    let mut left = Signal::new(signal);
    let mut right = signal.get("right").map(Signal::new);
    let mut out: Vec<[f32; 2]> = (0..frames)
        .map(|n| {
            let l = left.at(n);
            [l, right.as_mut().map_or(l, |r| r.at(n))]
        })
        .collect();

    if case["unit"] == "osc" {
        // A chain of oscillators, as the TripleOscillator makes them
        use lm_dsp::oscillator::{ModulationAlgo, OscParams, Oscillator, WaveShape};
        let rate = case["rate"].as_f64().unwrap();
        let oscs = case["settings"]["oscs"].as_array().unwrap();
        // A user's wave: one period of the test's noise
        let users: Vec<Option<lm_dsp::oscillator::UserWave>> = oscs
            .iter()
            .map(|o| {
                o.get("user").map(|u| {
                    let mut noise = Signal::new(&serde_json::json!({"kind": "noise", "seed": u["seed"], "amp": 1.0}));
                    let period = (0..u["frames"].as_i64().unwrap()).map(|f| noise.at(f)).collect();
                    lm_dsp::oscillator::UserWave::new(period)
                })
            })
            .collect();
        let params: Vec<OscParams<'_>> = oscs
            .iter()
            .map(|o| OscParams {
                shape: WaveShape::from_key(o["shape"].as_str().unwrap()).unwrap(),
                algo: ModulationAlgo::from_key(o["algo"].as_str().unwrap()).unwrap(),
                freq: o["freq"].as_f64().unwrap() as f32,
                detuning: o["ratio"].as_f64().unwrap() / rate,
                phase_offset: o["phase"].as_f64().unwrap() as f32,
                volume: o["volume"].as_f64().unwrap() as f32,
                use_wave_table: o["tables"].as_bool().unwrap(),
                user_wave: None,
            })
            .zip(&users)
            .map(|(p, u)| OscParams { user_wave: u.as_ref(), ..p })
            .collect();
        let mut chain: Vec<Oscillator> =
            params.iter().map(|p| Oscillator::new(p.phase_offset, rate as f32, 0)).collect();
        let mut mono = vec![0.0f32; frames as usize];
        for block_start in (0..frames as usize).step_by(block as usize) {
            let end = (block_start + block as usize).min(frames as usize);
            lm_dsp::oscillator::update(&mut chain, &params, &mut mono[block_start..end], false);
        }
        return mono.into_iter().map(|x| [x, x]).collect();
    }
    if case["unit"] == "tube-curve" {
        // The static curve alone: the tube's transfer function and where it clips
        let model = GainModel::from_key(case["settings"]["drive"].as_str().unwrap()).unwrap();
        for frame in &mut out {
            *frame = [lm_dsp::gain_stage::tube(model, frame[0]), lm_dsp::gain_stage::tube_clip_level(model)];
        }
        return out;
    }
    let strip_unit = case["unit"] == "strip";
    let then = &case["then"];
    let change_at = then["at"].as_i64().unwrap_or(frames);
    let mut strip = ChannelStrip::new(RATE);
    let mut master = Master::new();
    let (mut settings, mut tolerance) = strip_settings(&case["settings"]);
    if !strip_unit {
        master.apply(&case["settings"]);
    }
    let mut start = 0;
    while start < frames {
        if start == change_at {
            if strip_unit {
                (settings, tolerance) = strip_settings(&then["settings"]);
            } else {
                master.apply(&then["settings"]);
            }
        }
        let count = block.min(frames - start);
        let span = &mut out[start as usize..(start + count) as usize];
        if strip_unit {
            strip.process(span, &settings, true, &tolerance, InsertPoint::PostEq, None);
        } else {
            master.process(span);
        }
        start += block;
    }
    out
}

fn reference(name: &str) -> Vec<[f32; 2]> {
    let bytes = std::fs::read(ab_dir().join("reference").join(format!("{name}.f32")))
        .unwrap_or_else(|e| panic!("{name}: no reference ({e}); run ab/build-references.sh"));
    let (frames, rest) = bytes.as_chunks::<8>();
    assert!(rest.is_empty(), "{name}: not whole frames");
    frames
        .iter()
        .map(|c| [f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]])])
        .collect()
}

fn db(x: f32) -> f32 {
    if x > 0.0 { lm_dsp::math::amp_to_dbfs(x) } else { -400.0 }
}

#[test]
fn every_case_nulls_against_the_legacy_program() {
    let cases: Value = serde_json::from_str(&std::fs::read_to_string(ab_dir().join("cases.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    let mut report = String::new();
    for case in cases["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let ours = run(case);
        let theirs = reference(name);
        assert_eq!(ours.len(), theirs.len(), "{name}: length");
        let (mut worst, mut at) = (0.0f32, 0usize);
        for (n, (a, b)) in ours.iter().zip(&theirs).enumerate() {
            for ch in 0..2 {
                let d = (a[ch] - b[ch]).abs();
                if !(d <= worst) {
                    worst = d;
                    at = n;
                }
            }
        }
        // AB_TRACE=<case> prints where it first parts from the legacy program, to find the cause
        if std::env::var("AB_TRACE").is_ok_and(|t| t == name) {
            for (n, (a, b)) in ours.iter().zip(&theirs).enumerate().filter(|(_, (a, b))| a != b).take(8) {
                eprintln!("{name} frame {n}: ours {a:?} legacy {b:?}");
            }
        }
        let allowed = INTENTIONAL.iter().find(|(n, _, _)| *n == name).map_or(NULL, |(_, limit, _)| *limit);
        report.push_str(&format!("{name:32} worst {:7.1} dBFS at frame {at}\n", db(worst)));
        if !(worst <= allowed) {
            failures.push(format!(
                "{name}: {:.1} dBFS at frame {at} (ours {:?}, legacy {:?})",
                db(worst),
                ours[at],
                theirs[at]
            ));
        }
    }
    eprintln!("{report}");
    assert!(failures.is_empty(), "differences above the rule:\n{}", failures.join("\n"));
}
