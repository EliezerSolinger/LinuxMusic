//! A/B of the plugin against the legacy OpulenZ itself (`ab/`: the references the legacy plugin wrote, its notes
//! sent as MIDI by the legacy NotePlayHandles and the chip rendered through its InstrumentPlayHandle, by the chip
//! instruments' harness, `crates/lm-plugins-sid/ab`). The plugin is built from a state that says what the case's
//! legacy settings said, plays the same notes in the contract's bench at blocks of 256, 32 and 1024, and must null
//! below −90 dBFS (U34) over the case's frames. The knobs change on the legacy's periods (the plugin applies a change
//! at its sample, the legacy at the next period).

#![allow(clippy::print_stderr)]

use std::path::{Path, PathBuf};

use lm_plugin::testing::{Bench, Timed, note_off, note_on, test_resources};
use lm_plugin::{BuildContext, Registry};
use lm_plugins_opulenz::{DESCRIPTOR, WAVEFORM_KEYS};
use lm_render::{EventKind, ProcessSpec};
use lm_state::{PARAMS_KEY, Value};
use serde_json::Value as Json;

const RATE: u32 = 48_000;
const PERIOD: usize = 256;
/// −90 dBFS
const NULL: f32 = 3.162_277_7e-5;
/// Where the note's fade-in has ended (at most 85 frames at 44.1 kHz: 93 at 48 kHz)
const FADED_IN: usize = 128;

fn ab_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ab")
}

fn cases() -> Vec<Json> {
    let text = std::fs::read_to_string(ab_dir().join("cases.json")).unwrap();
    let cases: Json = serde_json::from_str(&text).unwrap();
    cases["cases"].as_array().unwrap().clone()
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

/// The plugin's parameters and fields for the legacy saved attributes
fn state_for(settings: &Json) -> (Value, Value) {
    let mut params = Value::empty_object();
    let mut fields = Value::empty_object();
    let ops = [
        ("_a", "Attack"),
        ("_d", "Decay"),
        ("_s", "Sustain"),
        ("_r", "Release"),
        ("_lvl", "Level"),
        ("_scale", "LevelScaling"),
        ("_mul", "Multiplier"),
        ("_ksr", "KeyScalingRate"),
        ("_perc", "Percussive"),
        ("_trem", "Tremolo"),
        ("_vib", "Vibrato"),
    ];
    for (legacy, value) in settings.as_object().unwrap() {
        let v = value.as_f64().unwrap();
        if let Some(op) = legacy.strip_suffix("_waveform") {
            fields = fields.with(&format!("{op}Waveform"), Value::text(WAVEFORM_KEYS[v as usize]));
            continue;
        }
        let key = match legacy.as_str() {
            "feedback" => "feedback".to_owned(),
            "fm" => "fm".to_owned(),
            "vib_depth" => "vibratoDepth".to_owned(),
            "trem_depth" => "tremoloDepth".to_owned(),
            "volume" => "volume".to_owned(),
            other => {
                let (op, rest) = other.split_at(3);
                let (_, name) =
                    ops.iter().find(|(s, _)| *s == rest).unwrap_or_else(|| panic!("no mapping for {other}"));
                format!("{op}{name}")
            }
        };
        params = params.with(&key, Value::real(v).unwrap());
    }
    (params, fields)
}

fn state_of(settings: &Json) -> Value {
    let (params, fields) = state_for(settings);
    fields.with(PARAMS_KEY, params)
}

/// Plays the case through the plugin
fn play(c: &Json, block: usize) -> Vec<Vec<f32>> {
    let state = state_of(&c["settings"]);
    let registry = Registry::new(&[&DESCRIPTOR]).unwrap();
    let complete = registry.complete(DESCRIPTOR.id, &state).unwrap();
    let resources = test_resources(registry.schemas(), &DESCRIPTOR.type_name(), &complete, RATE);
    let spec = ProcessSpec { sample_rate: RATE, max_block: 1024 };
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let processor = registry.build(DESCRIPTOR.id, &state, &cx).unwrap();
    let mut events = Vec::new();
    for (i, n) in c["notes"].as_array().unwrap().iter().enumerate() {
        let key = n["key"].as_u64().unwrap() as u8;
        let start = n["start"].as_u64().unwrap();
        let length = n["length"].as_u64().unwrap();
        // (the legacy note's MIDI velocity at the base velocity 63, over 63)
        let volume = n["volume"].as_u64().unwrap_or(100);
        let velocity = (volume * 63 / 100) as f32 / 63.0;
        events.push(note_on(start, i as u32 + 1, key, velocity));
        events.push(note_off(start + length, i as u32 + 1, key));
    }
    if let Some(at) = c["then"]["at"].as_u64() {
        let schema = DESCRIPTOR.schema();
        let (changed, _) = state_for(&c["then"]["settings"]);
        for (key, value) in changed.entries() {
            let index = schema.params.iter().position(|s| s.key == key).unwrap();
            events
                .push(Timed::new(at, EventKind::Param { index: index as u32, value: value.as_real().unwrap() as f32 }));
        }
    }
    let frames = c["frames"].as_u64().unwrap() as usize;
    Bench::instrument(processor, spec, 2, 0).render(frames, block, &events, false)
}

/// The worst difference from the reference over `from..to` (but the fade-ins of the notes starting at `starts`), and
/// where
fn worst_but(out: &[Vec<f32>], expected: &[[f32; 2]], from: usize, to: usize, starts: &[usize]) -> (f32, usize) {
    let mut worst = 0.0f32;
    let mut at = from;
    for (n, want) in expected.iter().enumerate().take(to).skip(from) {
        if starts.iter().any(|&s| n >= s && n < s + FADED_IN) {
            continue;
        }
        for ch in 0..2 {
            let d = (out[ch][n] - want[ch]).abs();
            if d > worst {
                worst = d;
                at = n;
            }
        }
    }
    (worst, at)
}

fn db(x: f32) -> f32 {
    if x > 0.0 { 20.0 * x.log10() } else { -400.0 }
}

#[test]
fn the_plugin_plays_as_the_legacy_opulenz() {
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let until = c["until"].as_u64().unwrap() as usize;
        let expected = reference(name);
        assert_eq!(expected.len(), c["frames"].as_u64().unwrap() as usize, "{name}: the reference's length");
        if let Ok(trace) = std::env::var("AB_TRACE")
            && trace == name
        {
            let out = play(&c, PERIOD);
            let from: usize = std::env::var("AB_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
            for n in from..(from + 64).min(expected.len()) {
                eprintln!("{n:6} {:+.7} {:+.7} {:+.3e}", out[0][n], expected[n][0], out[0][n] - expected[n][0]);
            }
        }
        for block in [PERIOD, 32, 1024] {
            let out = play(&c, block);
            let (w, at) = worst_but(&out, &expected, 0, until, &[]);
            eprintln!("{name:<24} blocks of {block:>4}: worst {:7.1} dBFS at frame {at} (frames 0..{until})", db(w));
            if w > NULL {
                failures.push(format!("{name} (blocks of {block}): {:.1} dBFS at frame {at}", db(w)));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
