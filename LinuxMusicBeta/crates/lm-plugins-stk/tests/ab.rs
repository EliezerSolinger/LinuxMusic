//! A/B against the legacy Mallets (U34): `ab/reference/<case>.f32` is what the legacy MalletsSynth, around the STK
//! 5.0.1 voices, played for each case of `ab/cases.txt` (`ab/build-references.sh`), at 44.1 kHz in periods of 32
//! frames. Two comparisons, each below −90 dBFS:
//!
//! - **the note** ([`mallets::Note`]): the port of `playNoteImpl` and `MalletsSynth` around the port of STK, its
//!   output before the level the plugin applies (the legacy plugin's own output), from the first frame;
//! - **the plugin**: the same note through the instrument functions (a note on at velocity `volume / 200`, the knobs
//!   turned by parameter events), against the reference times the legacy track's `volume / 100` and U28's half, from
//!   frame 128 (before it, the instrument's fade-in, `lm-plugin`'s, which the reference has not).
//!
//! `AB_REPORT=1` prints each case's largest difference.

#![allow(clippy::print_stdout)] // (the report)

use std::collections::BTreeMap;

use lm_dsp::math::LegacyRand;
use lm_plugin::testing::{Bench, Timed, note_on};
use lm_plugin::{BuildContext, Registry, ResourceView};
use lm_plugins_stk::DESCRIPTOR;
use lm_plugins_stk::mallets::{BANDED, Knobs, Model, Note, PRESET_KEYS, Start, TUBE_BELL};
use lm_plugins_stk::stk::Rate;
use lm_plugins_stk::stk::banded_wg::BandedWG;
use lm_plugins_stk::stk::modal_bar::ModalBar;
use lm_plugins_stk::stk::tube_bell::TubeBell;
use lm_render::{EventKind, ProcessSpec, Rng};
use lm_state::Value;

const RATE: u32 = 44_100;
const PIECE: usize = 32;
/// The bench's seed: the node's Rng, whose first draw seeds the first note's randomness
const SEED: u64 = 77;
const LIMIT_DBFS: f64 = -90.0;

#[derive(Clone, Debug)]
struct Case {
    name: String,
    preset: usize,
    key: u8,
    volume: u32,
    frames: usize,
    seed: u64,
    knobs: BTreeMap<String, f32>,
    changes: Vec<(usize, String, f32)>,
}

const DEFAULTS: [(&str, f32); 18] = [
    ("hardness", 64.0),
    ("position", 64.0),
    ("vibratoGain", 0.0),
    ("vibratoFreq", 0.0),
    ("stickMix", 0.0),
    ("modulator", 64.0),
    ("crossfade", 64.0),
    ("lfoSpeed", 64.0),
    ("lfoDepth", 64.0),
    ("adsr", 64.0),
    ("pressure", 64.0),
    ("speed", 64.0),
    ("bowed", 1.0),
    ("spread", 0.0),
    ("randomness", 0.0),
    ("pitch", 0.0),
    ("levelled", 1.0),
    ("oldVersion", 0.0),
];

fn cases() -> Vec<Case> {
    let text = include_str!("../ab/cases.txt");
    let mut cases = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let mut words = line.split_whitespace();
        let mut c = Case {
            name: words.next().expect("name").to_owned(),
            preset: 0,
            key: 69,
            volume: 100,
            frames: 0,
            seed: 0,
            knobs: DEFAULTS.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            changes: Vec::new(),
        };
        for word in words {
            let (key, value) = word.split_once('=').expect("key=value");
            match key {
                "preset" => c.preset = PRESET_KEYS.iter().position(|p| *p == value).expect("preset"),
                "key" => c.key = value.parse().expect("key"),
                "volume" => c.volume = value.parse().expect("volume"),
                "frames" => c.frames = value.parse().expect("frames"),
                "seed" => c.seed = value.parse().expect("seed"),
                "change" => {
                    let mut parts = value.split(':');
                    let frame = parts.next().and_then(|f| f.parse().ok()).expect("frame");
                    let knob = parts.next().expect("knob").to_owned();
                    let v = parts.next().and_then(|v| v.parse().ok()).expect("value");
                    c.changes.push((frame, knob, v));
                }
                _ => {
                    assert!(c.knobs.contains_key(key), "{}: unknown knob {key}", c.name);
                    c.knobs.insert(key.to_owned(), value.parse().expect("value"));
                }
            }
        }
        cases.push(c);
    }
    cases
}

/// The reference, interleaved stereo (a file of one side is both: see `legacy-harness.cpp`)
fn reference(c: &Case) -> Vec<f32> {
    let path = format!("{}/ab/reference/{}.f32", env!("CARGO_MANIFEST_DIR"), c.name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let x: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    if x.len() == 2 * c.frames { x } else { x.iter().flat_map(|v| [*v, *v]).collect() }
}

fn knobs(k: &BTreeMap<String, f32>) -> Knobs {
    Knobs {
        hardness: k["hardness"],
        position: k["position"],
        vibrato_gain: k["vibratoGain"],
        vibrato_freq: k["vibratoFreq"],
        stick_mix: k["stickMix"],
        modulator: k["modulator"],
        crossfade: k["crossfade"],
        lfo_speed: k["lfoSpeed"],
        lfo_depth: k["lfoDepth"],
        adsr: k["adsr"],
        pressure: k["pressure"],
        speed: k["speed"],
        bowed: k["bowed"] >= 0.5,
        spread: k["spread"],
        randomness: k["randomness"],
    }
}

/// The instrument functions' frequency (pitch in cents, no bend, no glide)
fn frequency(key: u8, cents: f32) -> f32 {
    let pitch = cents / 100.0 + 0.0 * 2.0;
    let keys = f32::from(key) - 69.0 + 0.0 + pitch;
    440.0 * libm::exp2f(keys / 12.0)
}

/// The note, by the port of `playNoteImpl`, raw (interleaved stereo)
fn note<M: Model>(c: &Case) -> Vec<f32> {
    let rate = Rate::new(f64::from(RATE));
    let fresh = M::new(rate);
    let mut note = Note::new(&fresh, rate);
    let mut k = c.knobs.clone();
    let start = Start {
        preset: c.preset,
        freq: frequency(c.key, k["pitch"]),
        volume: c.volume as f32,
        levelled: k["levelled"] >= 0.5,
        old_version: k["oldVersion"] >= 0.5,
    };
    note.start(&fresh, &knobs(&k), &start, &mut LegacyRand::new(c.seed));
    let mut out = Vec::with_capacity(2 * c.frames);
    let (mut l, mut r) = ([0.0f32; PIECE], [0.0f32; PIECE]);
    for at in (0..c.frames).step_by(PIECE) {
        for (frame, knob, v) in &c.changes {
            if *frame == at {
                k.insert(knob.clone(), *v);
            }
        }
        let n = PIECE.min(c.frames - at);
        let freq = frequency(c.key, k["pitch"]);
        note.render_with(&knobs(&k), start.old_version, freq, &mut l[..n], &mut r[..n], true);
        for i in 0..n {
            out.push(l[i]);
            out.push(r[i]);
        }
    }
    out
}

/// The note through the plugin (interleaved stereo)
fn plugin(c: &Case) -> Vec<f32> {
    let registry = Registry::new(&[&DESCRIPTOR]).expect("registry");
    let schema = DESCRIPTOR.schema();
    let index = |key: &str| schema.params.iter().position(|p| p.key == key).unwrap_or_else(|| panic!("{key}"));
    let mut params = Value::empty_object();
    for (key, v) in &c.knobs {
        if key != "levelled" && key != "oldVersion" {
            params = params.with(key, Value::real(f64::from(*v)).expect("real"));
        }
    }
    let state = Value::empty_object()
        .with("preset", Value::text(PRESET_KEYS[c.preset]))
        .with("levelled", Value::bool(c.knobs["levelled"] >= 0.5))
        .with("oldVersion", Value::bool(c.knobs["oldVersion"] >= 0.5))
        .with("params", params);
    let state = registry.complete("mallets", &state).expect("complete");
    let resources = ResourceView::new();
    let spec = ProcessSpec { sample_rate: RATE, max_block: 1024 };
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let processor = registry.build("mallets", &state, &cx).expect("build");
    let mut events = vec![note_on(0, 1, c.key, c.volume as f32 / 200.0)];
    for (frame, knob, v) in &c.changes {
        events.push(Timed::new(*frame as u64, EventKind::Param { index: index(knob) as u32, value: *v }));
    }
    let out = Bench::instrument(processor, spec, 2, SEED).render(c.frames, 256, &events, false);
    out[0].iter().zip(&out[1]).flat_map(|(a, b)| [*a, *b]).collect()
}

fn dbfs(x: f32) -> f64 {
    20.0 * f64::from(x).max(1.0e-30).log10()
}

/// The largest difference from frame `from` on, dBFS, and the frame where it is
fn difference(a: &[f32], b: &[f32], from: usize) -> (f64, usize) {
    assert_eq!(a.len(), b.len());
    let mut worst = (0.0f32, 0);
    for i in 2 * from..a.len() {
        let d = (a[i] - b[i]).abs();
        if d > worst.0 || d.is_nan() {
            worst = (d, i / 2);
        }
    }
    (dbfs(worst.0), worst.1)
}

#[test]
fn the_seed_is_the_benchs_first_draw() {
    assert_eq!(Rng::new(SEED).next_u64(), 8_103_657_047_149_143_059);
}

#[test]
fn the_note_is_the_legacy_note() {
    let report = std::env::var_os("AB_REPORT").is_some();
    let mut failures = Vec::new();
    for c in cases() {
        let reference = reference(&c);
        let out = if c.preset < TUBE_BELL {
            note::<ModalBar>(&c)
        } else if c.preset < BANDED {
            note::<TubeBell>(&c)
        } else {
            note::<BandedWG>(&c)
        };
        let (db, at) = difference(&out, &reference, 0);
        let peak = reference.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        if report {
            println!("note {}: {db:.1} dBFS at frame {at} (peak {:.1} dBFS)", c.name, dbfs(peak));
        }
        if db >= LIMIT_DBFS {
            failures.push(format!("{}: {db:.1} dBFS at frame {at}", c.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_plugin_plays_the_legacy_note() {
    let report = std::env::var_os("AB_REPORT").is_some();
    let mut failures = Vec::new();
    for c in cases() {
        // The legacy track's volume (`volume * (1 / DefaultVolume)`) and U28's half
        let gain = c.volume as f32 * (1.0 / 100.0) * 0.5;
        let reference: Vec<f32> = reference(&c).iter().map(|x| x * gain).collect();
        let out = plugin(&c);
        let (db, at) = difference(&out, &reference, 128);
        if report {
            println!("plugin {}: {db:.1} dBFS at frame {at}", c.name);
        }
        if db >= LIMIT_DBFS {
            failures.push(format!("{}: {db:.1} dBFS at frame {at}", c.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
