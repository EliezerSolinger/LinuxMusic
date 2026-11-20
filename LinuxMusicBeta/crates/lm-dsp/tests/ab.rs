//! The A/B of lm-dsp against the legacy program (interfaces.md, U34): every case of `ab/cases.json` is run here and
//! compared with what the legacy code gave (`ab/reference/`, written by `ab/build-references.sh`).
//!
//! The rule: the difference stays below -90 dBFS at every sample (a null test), or, when the legacy output goes
//! above full scale (a self-oscillating filter, a hard driven stage), 90 dB below its peak: a null at the signal's
//! own scale. Unless the case is listed in `INTENTIONAL` with the reason the port differs on purpose.

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
const INTENTIONAL: &[(&str, f32, &str)] = &[
    (
        "kicker-default",
        0.001,
        "The legacy SweepOsc kept its phase growing in single precision: its pitch drifts with the rate and the note's length (2 cents at 96 kHz after a second, so the tuning differs between rates, U36). The port wraps the phase into one period, the same sine; the two part by the legacy's rounding, about -64 dBFS half a second into the note",
    ),
    (
        "kicker-dist-sweep",
        0.001,
        "The legacy SweepOsc kept its phase growing in single precision: its pitch drifts with the rate and the note's length (2 cents at 96 kHz after a second, so the tuning differs between rates, U36). The port wraps the phase into one period, the same sine; the two part by the legacy's rounding, about -64 dBFS half a second into the note",
    ),
    (
        "kicker-slow",
        0.001,
        "The legacy SweepOsc kept its phase growing in single precision: its pitch drifts with the rate and the note's length (2 cents at 96 kHz after a second, so the tuning differs between rates, U36). The port wraps the phase into one period, the same sine; the two part by the legacy's rounding, about -64 dBFS half a second into the note",
    ),
    (
        "strip-12at7",
        0.0316, // -30 dBFS
        "The pot's gain at 0.4 is dbfs_to_amp(12 dB): the libm crate's expf gives one ulp less than Apple's (V2 chose \
     libm, for the same samples on every system), and the 12AT7's light model, this hard driven, amplifies that ulp \
     into a difference of about -37 dBFS. strip-12at7-exact-gain, at 0.45 where the two exp agree, nulls bit for bit",
    ),
    (
        "dualfilter",
        0.001, // -60 dBFS
        "The Moog's coefficients follow the cutoff knob's ramp frame by frame, through expf: on 40 of the first period's \
     256 cutoffs the libm crate's expf and Apple's differ by one ulp (V2: libm), and the Moog, this resonant, \
     amplifies that into about -66 dBFS. dualfilter-exact-moog, the same without the Moog's ramp, stays near -100 dBFS",
    ),
];

/// Cases compared only up to a frame, because the legacy program is wrong after it on purpose of the port, with why
const UNTIL: &[(&str, usize, &str)] = &[
    (
        "lomm-deep-linked",
        6144,
        "At frame 6144 the high/mid split moves: the legacy Linkwitz-Riley's burst, as in crossovereq (below)",
    ),
    (
        "crossovereq",
        6144,
        "At frame 6144 the crossovers move. The legacy Linkwitz-Riley was one 4th-order section in direct form II, whose \
     inner state is about (rate / cutoff)^4 times the signal: changing its cutoff while it plays releases that state \
     as a burst (here 24 dB above the input; a sweep from 50 Hz to 20 kHz gave 1e37). The port computes the same \
     filter as two Butterworth sections in transposed form, and nulls with the legacy until the change",
    ),
];

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
    if case["unit"] == "plugin-effect"
        && ["amplifier", "stereomatrix", "stereoenhancer", "bassbooster", "dispersion", "waveshaper", "bitcrush"]
            .contains(&case["plugin"].as_str().unwrap())
    {
        use lm_dsp::small_effects::*;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let block = block as usize;
        // A knob's value in a block, and its ramp (the legacy value buffer: in the block where it changed; the first
        // block too, from the knob's default, as the harness turns the knobs before the first period)
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        let ramp = |key: &str, default: f32, start: usize| {
            if start == 0 {
                Ramp { from: default, to: value(key, default, 0) }
            } else if start == at {
                Ramp { from: value(key, default, 0), to: value(key, default, start) }
            } else {
                Ramp::constant(value(key, default, start))
            }
        };
        let rate = case["rate"].as_f64().unwrap() as f32;
        use lm_dsp::distortion_effects::*;
        let on = |key: &str, default: bool, start: usize| value(key, if default { 1.0 } else { 0.0 }, start) != 0.0;
        let mut enhancer = StereoEnhancer::default();
        let mut dispersion = Dispersion::new(rate);
        let mut bitcrush = Bitcrush::new(rate, block, 0);
        let shape = default_wave_shape();
        let mut booster = BassBooster::new(rate);
        for start in (0..frames as usize).step_by(block) {
            let span = &mut out[start..(start + block).min(frames as usize)];
            match case["plugin"].as_str().unwrap() {
                "amplifier" => amplifier(
                    span,
                    &AmplifierSettings {
                        volume: ramp("Volume", 100.0, start),
                        pan: ramp("Panning", 0.0, start),
                        left: ramp("Left gain", 100.0, start),
                        right: ramp("Right gain", 100.0, start),
                    },
                ),
                "stereomatrix" => stereo_matrix(
                    span,
                    &StereoMatrixSettings {
                        left_to_left: value("Left to Left", 1.0, start),
                        left_to_right: value("Left to Right", 0.0, start),
                        right_to_left: value("Right to Left", 0.0, start),
                        right_to_right: value("Right to Right", 1.0, start),
                    },
                ),
                "stereoenhancer" => enhancer.process(span, value("Width", 0.0, start)),
                "dispersion" => dispersion.process(
                    span,
                    &DispersionSettings {
                        amount: value("Amount", 0.0, start) as usize,
                        frequency: value("Frequency", 200.0, start),
                        resonance: value("Resonance", 0.707, start),
                        feedback: value("Feedback", 0.0, start),
                        dc: on("DC Offset Removal", false, start),
                    },
                ),
                "waveshaper" => wave_shaper(
                    span,
                    &shape,
                    ramp("Input gain", 1.0, start),
                    ramp("Output gain", 1.0, start),
                    on("Clip input", false, start),
                ),
                "bitcrush" => {
                    // The legacy Bitcrush asks `needsUpdate || rate changed || difference changed`: the first period
                    // recalculates for its first reason, leaving the knobs' change flags to the next periods, which
                    // reset the rate counters again, one period for each flag left (rate first, then the difference;
                    // a change of both later does the same). The legacy plumbing, not the DSP: done here by hand
                    let set = |s: &Value| {
                        ["Sample rate", "Stereo difference"].iter().filter(|k| s.get(**k).is_some()).count()
                    };
                    let extra_after = |from: usize, count: usize| (1..=count).any(|n| start == from + n * block);
                    if extra_after(0, set(first))
                        || (at < frames as usize && extra_after(at, set(then).saturating_sub(1)))
                    {
                        bitcrush.reset_counters();
                    }
                    bitcrush.process(
                        span,
                        &BitcrushSettings {
                            in_gain: value("Input gain", 0.0, start),
                            in_noise: value("Input noise", 0.0, start),
                            out_gain: value("Output gain", 0.0, start),
                            out_clip: value("Output clip", 0.0, start),
                            rate: value("Sample rate", 44100.0, start),
                            stereo_diff: value("Stereo difference", 0.0, start),
                            levels: value("Levels", 256.0, start),
                            rate_enabled: on("Rate enabled", true, start),
                            depth_enabled: on("Depth enabled", true, start),
                        },
                    )
                }
                _ => booster.process(
                    span,
                    &BassBoosterSettings {
                        frequency: value("Frequency", 100.0, start),
                        gain: ramp("Gain", 1.0, start),
                        ratio: value("Ratio", 2.0, start),
                    },
                ),
            }
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && case["plugin"] == "granularpitchshifter" {
        use lm_dsp::granular_pitch_shifter::*;
        use lm_dsp::small_effects::Ramp;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let block = block as usize;
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        // (the pitch knobs are sample-exact: the legacy value buffer ramps in the period they change, the first one
        // from the default)
        let ramp = |key: &str, default: f32, start: usize| {
            if start == 0 {
                Ramp { from: default, to: value(key, default, 0) }
            } else if start == at {
                Ramp { from: value(key, default, 0), to: value(key, default, start) }
            } else {
                Ramp::constant(value(key, default, start))
            }
        };
        let d = GranularSettings::default();
        // As the legacy plugin starts: the shortest ring, the default pitch (the case's range follows a period late)
        let mut shifter = GranularPitchShifter::new(case["rate"].as_f64().unwrap() as f32, 0, 1.0, 0.0, 0);
        for start in (0..frames as usize).step_by(block) {
            let span = &mut out[start..(start + block).min(frames as usize)];
            let settings = GranularSettings {
                pitch: ramp("Pitch", 1.0, start),
                pitch_spread: ramp("Pitch Stereo Spread", 0.0, start),
                size: value("Grain Size", d.size, start),
                spray: value("Spray", d.spray, start),
                jitter: value("Jitter", d.jitter, start),
                twitch: value("Twitch", d.twitch, start),
                spray_spread: value("Spray Stereo", d.spray_spread, start),
                shape: value("Shape", d.shape, start),
                fade_length: value("Fade Length", d.fade_length, start),
                feedback: value("Feedback", d.feedback, start),
                min_latency: value("Minimum Allowed Latency", d.min_latency, start),
                prefilter: value("Prefilter", 1.0, start) != 0.0,
                density: value("Density", d.density, start),
                glide: value("Glide", d.glide, start),
                range: value("Ring Buffer Length", 0.0, start) as usize,
            };
            shifter.process(span, &settings);
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && case["plugin"] == "lomm" {
        use lm_dsp::lomm::*;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        let on = |key: &str, default: bool, start: usize| value(key, if default { 1.0 } else { 0.0 }, start) != 0.0;
        let mut lomm = Lomm::new(case["rate"].as_f64().unwrap() as f32);
        for start in (0..frames as usize).step_by(block as usize) {
            let span = &mut out[start..(start + block as usize).min(frames as usize)];
            let d = LommSettings::default();
            let names = [("High", "H"), ("Mid", "M"), ("Low", "L")];
            let band = |j: usize| {
                let (name, _) = names[j];
                let b = d.bands[j];
                LommBand {
                    input: value(&format!("{name} Input Volume"), b.input, start),
                    output: value(&format!("{name} Output Volume"), b.output, start),
                    above_threshold: value(&format!("Above Threshold {name}"), b.above_threshold, start),
                    above_ratio: value(&format!("Above Ratio {name}"), b.above_ratio, start),
                    below_threshold: value(&format!("Below Threshold {name}"), b.below_threshold, start),
                    below_ratio: value(&format!("Below Ratio {name}"), b.below_ratio, start),
                    attack: value(&format!("Attack {name}"), b.attack, start),
                    release: value(&format!("Release {name}"), b.release, start),
                    enabled: on(&format!("Enable {name} Band"), true, start),
                }
            };
            let settings = LommSettings {
                bands: [band(0), band(1), band(2)],
                depth: value("Depth", d.depth, start),
                time: value("Time", d.time, start),
                input: value("Input Volume", d.input, start),
                output: value("Output Volume", d.output, start),
                upward: value("Upward Depth", d.upward, start),
                downward: value("Downward Depth", d.downward, start),
                splits: [value("High/Mid Split", d.splits[0], start), value("Mid/Low Split", d.splits[1], start)],
                splits_enabled: [on("Enable High/Mid Split", true, start), on("Enable Mid/Low Split", true, start)],
                rms_time: value("RMS Time", d.rms_time, start),
                knee: value("Knee", d.knee, start),
                range: value("Range", d.range, start),
                balance: value("Balance", d.balance, start),
                depth_scaling: on("Scale output volume with Depth", true, start),
                stereo_link: on("Stereo Link", false, start),
                auto_time: value("Auto Time", d.auto_time, start),
                mix: value("Mix", d.mix, start),
                feedback: on("Feedback", false, start),
                mid_side: on("Mid/Side", false, start),
                lookahead_enabled: on("Lookahead", false, start),
                lookahead: value("Lookahead Length", d.lookahead, start),
                low_side_upward_suppress: on("Suppress upward compression for side band", false, start),
            };
            lomm.process(span, &settings);
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && case["plugin"] == "slewdistortion" {
        use lm_dsp::slew_distortion::*;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        let mut slew = SlewDistortion::new(case["rate"].as_f64().unwrap() as f32);
        for start in (0..frames as usize).step_by(block as usize) {
            let span = &mut out[start..(start + block as usize).min(frames as usize)];
            let d = SlewBand::default();
            let band = |n: usize| SlewBand {
                shape: SlewShape::ALL[value(&format!("Type {n}"), 0.0, start) as usize],
                drive: value(&format!("Drive {n}"), d.drive, start),
                slew_up: value(&format!("Slew Up {n}"), d.slew_up, start),
                slew_down: value(&format!("Slew Down {n}"), d.slew_down, start),
                bias: value(&format!("Bias {n}"), d.bias, start),
                warp: value(&format!("Warp {n}"), d.warp, start),
                crush: value(&format!("Crush {n}"), d.crush, start),
                out_volume: value(&format!("Out Vol {n}"), d.out_volume, start),
                attack: value(&format!("Attack {n}"), d.attack, start),
                release: value(&format!("Release {n}"), d.release, start),
                dynamics: value(&format!("Dynamics {n}"), d.dynamics, start),
                dynamic_slew: value(&format!("Dynamic Slew {n}"), d.dynamic_slew, start),
                mix: value(&format!("Mix {n}"), d.mix, start),
                slew_link: value(&format!("Slew Link {n}"), 1.0, start) != 0.0,
            };
            let settings = SlewDistortionSettings {
                bands: [band(1), band(2)],
                dc_remove: value("DC Offset Remover", 1.0, start) != 0.0,
                multiband: value("Multiband", 0.0, start) != 0.0,
                oversampling: value("Oversample", 0.0, start) as usize,
                split: value("Split", 200.0, start),
            };
            slew.process(span, &settings);
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && case["plugin"] == "frequencyshifter" {
        use lm_dsp::frequency_shifter::*;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        let mut shifter = FrequencyShifter::new(case["rate"].as_f64().unwrap() as f32);
        for start in (0..frames as usize).step_by(block as usize) {
            let span = &mut out[start..(start + block as usize).min(frames as usize)];
            let d = FrequencyShifterSettings::default();
            let settings = FrequencyShifterSettings {
                mix: value("Mix", d.mix, start),
                shift: value("Frequency Shift", d.shift, start),
                spread: value("Spread Shift", d.spread, start),
                ring: value("Ring", d.ring, start),
                feedback: value("Feedback", d.feedback, start),
                delay: value("Delay Length", d.delay, start),
                fine_delay: value("Fine Delay Length", d.fine_delay, start),
                delay_damp: value("Delay Damping", d.delay_damp, start),
                delay_glide: value("Delay Glide", d.delay_glide, start),
                lfo_amount: value("LFO Amount", d.lfo_amount, start),
                lfo_rate: value("LFO Rate", d.lfo_rate, start),
                lfo_stereo_phase: value("LFO StereoPhase", d.lfo_stereo_phase, start),
                glide: value("Glide", d.glide, start),
                tone: value("Tone", d.tone, start),
                phase: value("Phase", d.phase, start),
                antireflect: value("Antireflect", 0.0, start) != 0.0,
                route: RouteMode::ALL[value("Route Mode", 0.0, start) as usize],
                harmonics: value("Harmonics", d.harmonics, start),
            };
            shifter.process(span, &settings);
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && case["plugin"] == "dynamicsprocessor" {
        use lm_dsp::dynamics_processor::*;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        let mut curve = default_curve();
        if let Some(graph) = case["graphs"][0].as_array() {
            for (point, v) in curve.iter_mut().zip(graph) {
                *point = v.as_f64().unwrap() as f32;
            }
        }
        let mut processor = DynamicsProcessor::new(case["rate"].as_f64().unwrap() as f32);
        for start in (0..frames as usize).step_by(block as usize) {
            let span = &mut out[start..(start + block as usize).min(frames as usize)];
            let settings = DynamicsProcessorSettings {
                input_gain: value("Input gain", 1.0, start),
                output_gain: value("Output gain", 1.0, start),
                attack: value("Attack time", 10.0, start),
                release: value("Release time", 100.0, start),
                stereo_mode: StereoMode::ALL[value("Stereo mode", 0.0, start) as usize],
                curve,
            };
            processor.process(span, &settings);
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && ["dualfilter", "crossovereq"].contains(&case["plugin"].as_str().unwrap()) {
        use lm_dsp::basic_filters::FilterType;
        use lm_dsp::filter_effects::*;
        use lm_dsp::small_effects::Ramp;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let block = block as usize;
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        // (sample-exact knobs: the legacy value buffer ramps in the period a knob changed, the first one from its
        // default)
        let ramp = |key: &str, default: f32, start: usize| {
            if start == 0 {
                Ramp { from: default, to: value(key, default, 0) }
            } else if start == at {
                Ramp { from: value(key, default, 0), to: value(key, default, start) }
            } else {
                Ramp::constant(value(key, default, start))
            }
        };
        let rate = case["rate"].as_f64().unwrap() as f32;
        let mut dual = DualFilter::new(rate);
        let mut crossover = CrossoverEq::new(rate, block);
        for start in (0..frames as usize).step_by(block) {
            let span = &mut out[start..(start + block).min(frames as usize)];
            if case["plugin"] == "dualfilter" {
                let side = |n: usize| DualFilterSide {
                    enabled: value(&format!("Filter {n} enabled"), 1.0, start) != 0.0,
                    filter: FilterType::ALL[value(&format!("Filter {n} type"), 0.0, start) as usize],
                    cutoff: ramp(&format!("Cutoff frequency {n}"), 7000.0, start),
                    resonance: ramp(&format!("Q/Resonance {n}"), 0.5, start),
                    gain: ramp(&format!("Gain {n}"), 100.0, start),
                };
                dual.process(span, &DualFilterSettings { sides: [side(1), side(2)], mix: ramp("Mix", 0.0, start) });
            } else {
                let settings = CrossoverSettings {
                    crossovers: [
                        value("Band 1/2 Crossover", 125.0, start),
                        value("Band 2/3 Crossover", 1250.0, start),
                        value("Band 3/4 Crossover", 5000.0, start),
                    ],
                    gains: core::array::from_fn(|b| value(&format!("Band {} Gain", b + 1), 0.0, start)),
                    enabled: core::array::from_fn(|b| value(&format!("Mute Band {}", b + 1), 1.0, start) != 0.0),
                };
                crossover.process(span, &settings);
            }
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && ["multitapecho", "flanger"].contains(&case["plugin"].as_str().unwrap()) {
        use lm_dsp::modulation_effects::*;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        let rate = case["rate"].as_f64().unwrap() as f32;
        let block = block as usize;
        let mut taps = MultitapSettings::default();
        if let Some(graphs) = case["graphs"].as_array() {
            for (g, target) in graphs.iter().zip([&mut taps.amp, &mut taps.lp]) {
                for (i, v) in g.as_array().unwrap().iter().enumerate() {
                    target[i] = v.as_f64().unwrap() as f32;
                }
            }
        }
        let mut echo = MultitapEcho::new(rate, block);
        let mut flanger = Flanger::new(rate, 0);
        for start in (0..frames as usize).step_by(block) {
            let span = &mut out[start..(start + block).min(frames as usize)];
            if case["plugin"] == "multitapecho" {
                let settings = MultitapSettings {
                    steps: value("Steps", 16.0, start) as usize,
                    step_length: value("Step length", 100.0, start),
                    dry_gain: value("Dry gain", 0.0, start),
                    swap_inputs: value("Swap inputs", 0.0, start) != 0.0,
                    stages: value("Lowpass stages", 1.0, start) as usize,
                    ..taps
                };
                echo.process(span, &settings);
            } else {
                let settings = FlangerSettings {
                    delay: value("Delay samples", 0.001, start),
                    lfo_period: value("LFO frequency", 0.25, start),
                    lfo_amount: value("Amount", 0.0, start),
                    stereo_phase: value("Stereo phase", 90.0, start),
                    feedback: value("Feedback", 0.0, start),
                    noise: 0.0,
                    invert_feedback: value("Invert", 0.0, start) != 0.0,
                };
                flanger.process(span, &settings);
            }
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && case["plugin"] == "eq" {
        use lm_dsp::eq::{Eq, EqBand, EqSettings, Slope};
        // The knobs by the legacy plugin's names
        let apply = |s: &Value, e: &mut EqSettings| {
            let on = |key: &str, default: bool| s[key].as_f64().map_or(default, |v| v != 0.0);
            let band = |b: &mut EqBand, name: &str, q: &str| {
                b.active = on(&format!("{name} active"), b.active);
                b.freq = f(s, &format!("{name} freq"), b.freq);
                b.q = f(s, &format!("{name} {q}"), b.q);
                b.gain = f(s, &format!("{name} gain"), b.gain);
            };
            e.in_gain = f(s, "Input gain", e.in_gain);
            e.out_gain = f(s, "Output gain", e.out_gain);
            band(&mut e.high_pass, "HP", "res");
            band(&mut e.low_shelf, "Low-shelf", "res");
            for (i, peak) in e.peaks.iter_mut().enumerate() {
                band(peak, &format!("Peak {}", i + 1), "BW");
            }
            band(&mut e.high_shelf, "High-shelf", "res");
            band(&mut e.low_pass, "LP", "res");
            let slope = |prefix: &str, current: Slope| {
                if on(&format!("{prefix} 48"), current == Slope::Db48) {
                    Slope::Db48
                } else if on(&format!("{prefix} 24"), current == Slope::Db24) {
                    Slope::Db24
                } else {
                    Slope::Db12
                }
            };
            e.high_pass_slope = slope("HP", e.high_pass_slope);
            e.low_pass_slope = slope("LP", e.low_pass_slope);
        };
        let rate = case["rate"].as_f64().unwrap() as f32;
        let mut settings = EqSettings::default();
        apply(&case["settings"], &mut settings);
        let mut eq = Eq::new(rate);
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        for start in (0..frames as usize).step_by(block as usize) {
            if start == at {
                apply(&case["then"]["settings"], &mut settings);
            }
            let end = (start + block as usize).min(frames as usize);
            eq.process(&mut out[start..end], &settings);
        }
        return out;
    }
    if case["unit"] == "plugin-effect" && case["plugin"] == "compressor" {
        use lm_dsp::compressor::{CompressorPlugin, CompressorPluginSettings, StereoLink};
        // The knobs by the legacy plugin's names
        let apply = |s: &Value, c: &mut CompressorPluginSettings| {
            let on = |key: &str, default: bool| s[key].as_f64().map_or(default, |v| v != 0.0);
            c.threshold = f(s, "Threshold", c.threshold);
            c.ratio = f(s, "Ratio", c.ratio);
            c.attack = f(s, "Attack", c.attack);
            c.release = f(s, "Release", c.release);
            c.knee = f(s, "Knee", c.knee);
            c.hold = f(s, "Hold", c.hold);
            c.range = f(s, "Range", c.range);
            c.rms = f(s, "RMS Size", c.rms);
            c.mid_side = on("Mid/Side", c.mid_side);
            c.peak_mode = on("Peak Mode", c.peak_mode);
            c.lookahead_length = f(s, "Lookahead Length", c.lookahead_length);
            c.in_balance = f(s, "Input Balance", c.in_balance);
            c.out_balance = f(s, "Output Balance", c.out_balance);
            c.limiter = on("Limiter", c.limiter);
            c.out_gain = f(s, "Output Gain", c.out_gain);
            c.in_gain = f(s, "Input Gain", c.in_gain);
            c.blend = f(s, "Blend", c.blend);
            c.stereo_balance = f(s, "Stereo Balance", c.stereo_balance);
            c.auto_makeup = on("Auto Makeup Gain", c.auto_makeup);
            c.audition = on("Audition", c.audition);
            c.feedback = on("Feedback", c.feedback);
            c.auto_attack = f(s, "Auto Attack", c.auto_attack);
            c.auto_release = f(s, "Auto Release", c.auto_release);
            c.lookahead = on("Lookahead", c.lookahead);
            c.tilt = f(s, "Tilt", c.tilt);
            c.tilt_freq = f(s, "Tilt Frequency", c.tilt_freq);
            if let Some(link) = s["Stereo Link"].as_f64() {
                c.stereo_link = [
                    StereoLink::Unlinked,
                    StereoLink::Maximum,
                    StereoLink::Average,
                    StereoLink::Minimum,
                    StereoLink::Blend,
                ][link as usize];
            }
            c.mix = f(s, "Mix", c.mix);
        };
        let rate = case["rate"].as_f64().unwrap() as f32;
        let mut settings = CompressorPluginSettings::default();
        apply(&case["settings"], &mut settings);
        let mut compressor = CompressorPlugin::new(rate);
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        for start in (0..frames as usize).step_by(block as usize) {
            if start == at {
                apply(&case["then"]["settings"], &mut settings);
            }
            let end = (start + block as usize).min(frames as usize);
            compressor.process(&mut out[start..end], &settings);
        }
        return out;
    }
    if case["unit"] == "sample" {
        // The sample player, with libsamplerate as the interpolator (as the AudioFileProcessor gives it)
        use lm_dsp::sample_player::{Bounds, Interpolator, Loop, PlaybackState, Sample};
        use lm_sys_samplerate::{Quality, Resampler};
        struct Src(Resampler);
        impl Interpolator for Src {
            fn process(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]], ratio: f64) -> (usize, usize) {
                self.0.set_ratio(ratio).unwrap();
                let done = self.0.process(input.as_flattened(), output.as_flattened_mut()).unwrap();
                (done.input_frames, done.output_frames)
            }
            fn reset(&mut self) {
                self.0.reset().unwrap();
            }
        }
        let s = &case["settings"];
        let bounds = s["bounds"].as_array().unwrap();
        let b = |i: usize| bounds[i].as_i64().unwrap() as i32;
        let sample = Sample {
            frames: &out,
            sample_rate: f(s, "rate", 44100.0),
            bounds: Bounds { start: b(0), end: b(1), loop_start: b(2), loop_end: b(3) },
            amplification: f(s, "amp", 1.0),
            frequency: f(s, "frequency", 440.0),
            reversed: s["reversed"].as_bool().unwrap(),
            declick: s["declick"].as_bool().unwrap(),
        };
        let quality = match s["interpolation"].as_str().unwrap() {
            "zoh" => Quality::ZeroOrderHold,
            "sinc-fastest" => Quality::Fastest,
            "sinc-medium" => Quality::Medium,
            "sinc-best" => Quality::Best,
            _ => Quality::Linear,
        };
        let looping = match s["loop"].as_str().unwrap() {
            "on" => Loop::On,
            "ping-pong" => Loop::PingPong,
            _ => Loop::Off,
        };
        let mut state = PlaybackState::new(Src(Resampler::new(quality, 2, 1.0).unwrap()), 0);
        let length = s["frames"].as_i64().unwrap() as usize;
        let mut played = vec![[0.0f32; 2]; length];
        for start in (0..length).step_by(block as usize) {
            let end = (start + block as usize).min(length);
            sample.play(&mut played[start..end], &mut state, looping, s["ratio"].as_f64().unwrap(), 44100.0, 256);
        }
        return played;
    }
    if case["unit"] == "basic-filter" {
        use lm_dsp::basic_filters::{BasicFilter, FilterType};
        let s = &case["settings"];
        let mut filter = BasicFilter::new(RATE);
        filter.set_filter_type(FilterType::from_key(s["key"].as_str().unwrap()).unwrap());
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        for start in (0..frames as usize).step_by(block as usize) {
            let now = if start < at { s } else { &case["then"]["settings"] };
            filter.calc_filter_coeffs(f(now, "freq", 0.0), f(now, "q", 0.0));
            for frame in &mut out[start..(start + block as usize).min(frames as usize)] {
                *frame = [filter.update(frame[0], 0), filter.update(frame[1], 1)];
            }
        }
        return out;
    }
    if case["unit"] == "lb302" {
        use lm_dsp::lb302::*;
        let first = &case["settings"];
        let then = &case["then"]["settings"];
        let at = case["then"]["at"].as_i64().unwrap_or(frames) as usize;
        let value = |key: &str, default: f32, start: usize| {
            let before = f(first, key, default);
            if start < at { before } else { f(then, key, before) }
        };
        let rate = case["rate"].as_f64().unwrap() as f32;
        let release = case["release"].as_u64().unwrap() as usize;
        let notes = case["notes"].as_array().unwrap();
        let mut synth = Lb302::new(rate, 0);
        let d = Lb302Settings::default();
        for start in (0..frames as usize).step_by(block as usize) {
            let settings = Lb302Settings {
                cutoff: value("VCF Cutoff Frequency", d.cutoff, start),
                resonance: value("VCF Resonance", d.resonance, start),
                env_mod: value("VCF Envelope Mod", d.env_mod, start),
                decay: value("VCF Envelope Decay", d.decay, start),
                distortion: value("Distortion", d.distortion, start),
                shape: VcoShape::ALL[value("Waveform", 8.0, start) as usize],
                slide_decay: value("Slide Decay", d.slide_decay, start),
                slide: value("Slide", 0.0, start) != 0.0,
                dead: value("Dead", 0.0, start) != 0.0,
                db24: value("24dB/oct Filter", 0.0, start) != 0.0,
            };
            synth.prepare_period(&settings);
            for n in notes {
                if n["at"].as_u64().unwrap() as usize == start {
                    synth.note_on(n["freq"].as_f64().unwrap() as f32);
                }
            }
            synth.render(&mut out[start..(start + block as usize).min(frames as usize)], release, false, [1.0, 1.0]);
        }
        return out;
    }
    if case["unit"] == "vibed" {
        use lm_dsp::vibed::*;
        let s = &case["settings"];
        let strings: Vec<VibedString> = s["strings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                let v = |key: &str| o[key].as_f64().unwrap() as f32;
                let mut shape = [0.0f32; IMPULSE_LENGTH];
                for (p, x) in shape.iter_mut().zip(o["shape"].as_array().unwrap()) {
                    *p = x.as_f64().unwrap() as f32;
                }
                VibedString {
                    enabled: true,
                    harmonic: o["harmonic"].as_u64().unwrap() as usize,
                    pick: v("pick"),
                    pickup: v("pickup"),
                    stiffness: v("stiffness"),
                    volume: v("volume"),
                    pan: v("pan"),
                    detune: v("detune"),
                    fuzziness: v("fuzziness"),
                    length: o["length"].as_i64().unwrap() as i32,
                    impulse: o["impulse"].as_bool().unwrap(),
                    shape,
                }
            })
            .collect();
        let mut voice = VibedVoice::new(
            &strings,
            s["freq"].as_f64().unwrap() as f32,
            case["rate"].as_f64().unwrap() as u32,
            s["seed"].as_u64().unwrap(),
        );
        voice.render(&mut out, &strings);
        return out;
    }
    if case["unit"] == "organic" {
        use lm_dsp::organic::*;
        let s = &case["settings"];
        let oscs = s["oscs"].as_array().unwrap();
        let o = |i: usize, key: &str| oscs[i][key].as_f64().unwrap() as f32;
        let settings = OrganicSettings {
            oscillators: core::array::from_fn(|i| OrganicOsc {
                shape: SHAPES[oscs[i]["shape"].as_u64().unwrap() as usize],
                harmonic: oscs[i]["harmonic"].as_u64().unwrap() as usize,
                volume: o(i, "volume"),
                pan: o(i, "pan"),
                detune: o(i, "detune"),
            }),
            distortion: s["distortion"].as_f64().unwrap() as f32,
            volume: s["volume"].as_f64().unwrap() as f32,
        };
        let phases = [core::array::from_fn(|i| o(i, "phaseLeft")), core::array::from_fn(|i| o(i, "phaseRight"))];
        let rate = case["rate"].as_f64().unwrap() as f32;
        let mut voice = OrganicVoice::with_phases(rate, block as usize, phases);
        let freq = s["freq"].as_f64().unwrap() as f32;
        for start in (0..frames as usize).step_by(block as usize) {
            voice.render(&mut out[start..(start + block as usize).min(frames as usize)], &settings, freq, rate);
        }
        return out;
    }
    if case["unit"] == "drawn-wave" {
        use lm_dsp::drawn_wave::{DrawnWave, Hold, MOST_HARMONICS};
        let s = &case["settings"];
        let points: Vec<f32> = s["points"].as_array().unwrap().iter().map(|p| p.as_f64().unwrap() as f32).collect();
        let hold = if s["hold"] == "step" { Hold::Step } else { Hold::Linear };
        let wave = DrawnWave::new(&points, hold, MOST_HARMONICS);
        let rate = case["rate"].as_f64().unwrap() as f32;
        let (from, to) = (s["from"].as_f64().unwrap() as f32, s["to"].as_f64().unwrap() as f32);
        let mut phase = 0.0f32;
        for (n, frame) in out.iter_mut().enumerate() {
            let freq = from + (to - from) * (n as f32 / frames as f32);
            let v = wave.table_for(freq, rate).map_or(0.0, |t| DrawnWave::at(t, phase));
            *frame = [v, v];
            phase += freq / rate;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
        return out;
    }
    if case["unit"] == "kicker-osc" {
        let s = &case["settings"];
        let v = |key: &str| s[key].as_f64().unwrap() as f32;
        let rate = case["rate"].as_f64().unwrap() as f32;
        let mut voice = lm_dsp::kicker::KickerVoice::from_parts(
            v("start"),
            v("end"),
            0.0,
            v("offset"),
            v("slope"),
            v("env"),
            v("dist"),
            v("distEnd"),
            v("gain"),
            v("length"),
            v("attack"),
            0,
        );
        for start in (0..frames as usize).step_by(block as usize) {
            voice.render(&mut out[start..(start + block as usize).min(frames as usize)], rate);
        }
        return out;
    }
    if case["unit"] == "envelope" {
        use lm_dsp::envelope::{EnvelopeAndLfo, EnvelopeSettings, LfoShape};
        let s = &case["settings"];
        let d = EnvelopeSettings::default();
        let settings = EnvelopeSettings {
            predelay: f(s, "Env pre-delay", d.predelay),
            attack: f(s, "Env attack", d.attack),
            hold: f(s, "Env hold", d.hold),
            decay: f(s, "Env decay", d.decay),
            sustain: f(s, "Env sustain", d.sustain),
            release: f(s, "Env release", d.release),
            amount: f(s, "Env mod amount", d.amount),
            lfo_predelay: f(s, "LFO pre-delay", d.lfo_predelay),
            lfo_attack: f(s, "LFO attack", d.lfo_attack),
            lfo_speed: f(s, "LFO frequency", d.lfo_speed),
            lfo_amount: f(s, "LFO mod amount", d.lfo_amount),
            lfo_shape: LfoShape::ALL[s["LFO wave shape"].as_i64().unwrap_or(0) as usize],
            x100: s["LFO frequency x 100"].as_i64().unwrap_or(0) != 0,
            control_env_amount: s["Modulate env amount"].as_i64().unwrap_or(0) != 0,
        };
        let rate = case["rate"].as_f64().unwrap() as f32;
        let block = block as usize;
        let mut envelope = EnvelopeAndLfo::new(f(s, "zero", 0.0), rate, block, 0);
        envelope.set(&settings, None, block);
        let release = s["release"].as_i64().unwrap() as i32;
        let mut level = vec![0.0f32; block];
        for start in (0..frames as usize).step_by(block) {
            envelope.fill_level(&mut level, start as i32, release);
            for (n, l) in level.iter().enumerate() {
                out[start + n] = [*l, *l];
            }
            envelope.next_period(block, None, block);
        }
        return out;
    }
    if case["unit"] == "delay" || case["unit"] == "reverbsc" {
        // The effects, wet only
        let then = &case["then"];
        let at = then["at"].as_i64().map_or(frames as usize, |a| a as usize);
        let delay_settings = |s: &Value| lm_dsp::delay::DelaySettings {
            time: f(s, "time", 0.5),
            feedback: f(s, "feedback", 0.0),
            lfo_time: f(s, "lfoTime", 2.0),
            lfo_amount: f(s, "lfoAmount", 0.0),
            out_gain: f(s, "outGain", 0.0),
        };
        let reverb_settings = |s: &Value| lm_dsp::reverb_sc::ReverbScSettings {
            input_gain: f(s, "inputGain", 0.0),
            size: f(s, "size", 0.89),
            color: f(s, "color", 10000.0),
            output_gain: f(s, "outputGain", 0.0),
        };
        let (first, second) = out.split_at_mut(at);
        if case["unit"] == "delay" {
            let mut delay = lm_dsp::delay::Delay::new(RATE);
            delay.process(first, &delay_settings(&case["settings"]));
            delay.process(second, &delay_settings(&then["settings"]));
        } else {
            let mut reverb = lm_dsp::reverb_sc::ReverbSc::new(RATE);
            reverb.process(first, &reverb_settings(&case["settings"]));
            reverb.process(second, &reverb_settings(&then["settings"]));
        }
        return out;
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
        let until = UNTIL.iter().find(|(n, _, _)| *n == name).map_or(ours.len(), |(_, until, _)| *until);
        let (mut worst, mut at) = (0.0f32, 0usize);
        for (n, (a, b)) in ours.iter().zip(&theirs).enumerate().take(until) {
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
            let above: Vec<usize> =
                (0..ours.len()).filter(|&n| (0..2).any(|ch| (ours[n][ch] - theirs[n][ch]).abs() > NULL)).collect();
            eprintln!("{name}: {} frames above -90 dBFS, from {:?} to {:?}", above.len(), above.first(), above.last());
            for (n, (a, b)) in ours.iter().zip(&theirs).enumerate().filter(|(_, (a, b))| a != b).take(8) {
                eprintln!("{name} frame {n}: ours {a:?} legacy {b:?}");
            }
        }
        let peak = theirs.iter().fold(0.0f32, |p, f| p.max(f[0].abs()).max(f[1].abs()));
        let allowed =
            INTENTIONAL.iter().find(|(n, _, _)| *n == name).map_or(NULL * peak.max(1.0), |(_, limit, _)| *limit);
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
