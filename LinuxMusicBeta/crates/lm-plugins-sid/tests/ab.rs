//! A/B of the plugin against the legacy Sid itself (`ab/`: the references the legacy plugin wrote, played through
//! the legacy NotePlayHandle and track by `ab/legacy-harness.cpp`, `ab/build-references.sh`).
//!
//! 1. Through the plugin: built from a state that says what the case's legacy settings said, the same notes in the
//!    contract's bench, at blocks of 256, 32 and 1024; it must null below −90 dBFS (U34) from frame 128 (the note's
//!    fade-in is the instrument functions', not the plugin's: lm-plugin works it out per piece of 32 frames, the
//!    legacy per period, as the TripleOscillator's A/B notes; the same for each later note's first 128 frames) up to
//!    the case's `until`:
//!    - the notes' end: the legacy release lasted `relTime` frames instead of milliseconds (an intentional
//!      difference, decisoes-autonomas.md);
//!    - the filtered cases, their first period: the legacy set the chip model at every period, which clears reSID's
//!      filter (an intentional difference).
//! 2. The legacy loop: the chip driven exactly as the legacy `playNoteImpl` drove it, period after period (the chip
//!    model set again each period, the 25 registers worked out by the plugin's own [`SidSynth::registers`] and written
//!    by `sid_fillbuffer`'s sequence), against the whole note of every single-note case, filtered ones included. With
//!    1, this shows that the plugin's only differences from the legacy plugin are the two above.

#![allow(clippy::print_stderr)]

use std::path::{Path, PathBuf};

use lm_plugin::testing::{Bench, Timed, note_off, note_on, test_resources};
use lm_plugin::{BuildContext, Registry};
use lm_plugins_sid::{DESCRIPTOR, FilterMode, SidSynth, WAVE_DELAY, WRITE_DELAY, WRITE_ORDER, Wave};
use lm_render::{EventKind, ProcessSpec};
use lm_state::{PARAMS_KEY, Value};
use lm_sys_resid::{PAL_CLOCK, Sid};
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
    let voice_keys = [
        ("pulsewidth", "pulseWidth"),
        ("attack", "attack"),
        ("decay", "decay"),
        ("sustain", "sustain"),
        ("release", "release"),
        ("coarse", "coarse"),
        ("sync", "sync"),
        ("ringmod", "ringMod"),
        ("filtered", "filtered"),
        ("test", "test"),
    ];
    for (legacy, value) in settings.as_object().unwrap() {
        let v = value.as_f64().unwrap();
        let real = |v: f64| Value::real(v).unwrap();
        if let Some(n) = legacy.strip_prefix("waveform") {
            let n: usize = n.parse().unwrap();
            fields = fields.with(&format!("wave{}", n + 1), Value::text(Wave::ALL[v as usize].key()));
            continue;
        }
        if let Some((key, n)) = voice_keys.iter().find_map(|(l, k)| legacy.strip_prefix(l).map(|n| (*k, n))) {
            let n: usize = n.parse().unwrap();
            params = params.with(&format!("{key}{}", n + 1), real(v));
            continue;
        }
        match legacy.as_str() {
            "filterFC" => params = params.with("filterCutoff", real(v)),
            "filterResonance" => params = params.with("filterResonance", real(v)),
            "voice3Off" => params = params.with("voice3Off", real(v)),
            "volume" => params = params.with("volume", real(v)),
            "filterMode" => fields = fields.with("filterMode", Value::text(FilterMode::ALL[v as usize].key())),
            "chipModel" => fields = fields.with("chipModel", Value::text(["mos6581", "mos8580"][v as usize])),
            other => panic!("the legacy setting {other:?} has no mapping"),
        }
    }
    (params, fields)
}

fn state_of(settings: &Json) -> Value {
    let (params, fields) = state_for(settings);
    fields.with(PARAMS_KEY, params)
}

fn worst(out: &[Vec<f32>], expected: &[[f32; 2]], from: usize, to: usize) -> (f32, usize) {
    worst_but(out, expected, from, to, &[])
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
        events.push(note_on(start, i as u32 + 1, key, 1.0));
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
fn the_plugin_plays_as_the_legacy_sid() {
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
            let starts: Vec<usize> =
                c["notes"].as_array().unwrap().iter().map(|n| n["start"].as_u64().unwrap() as usize).collect();
            let (w, at) = worst_but(&out, &expected, FADED_IN, until, &starts);
            eprintln!(
                "{name:<24} blocks of {block:>4}: worst {:7.1} dBFS at frame {at} (frames {FADED_IN}..{until})",
                db(w)
            );
            if w > NULL {
                failures.push(format!("{name} (blocks of {block}): {:.1} dBFS at frame {at}", db(w)));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// One note as the legacy `playNoteImpl` played it, period after period (`sid_fillbuffer`, written from the legacy
/// source), the registers from the plugin's synth
fn legacy_loop(c: &Json) -> Vec<f32> {
    let registry = Registry::new(&[&DESCRIPTOR]).unwrap();
    let complete = registry.complete(DESCRIPTOR.id, &state_of(&c["settings"])).unwrap();
    let synth = SidSynth::from_state(&complete, f64::from(RATE)).unwrap();
    let note = &c["notes"][0];
    let key = note["key"].as_u64().unwrap() as f32;
    let length = note["length"].as_u64().unwrap() as usize;
    // NotePlayHandle::updateFrequency, in single precision
    let freq = 440.0 * libm::exp2f((key - 69.0) / 12.0);
    let mut sid = Sid::new(f64::from(PAL_CLOCK), f64::from(RATE)).unwrap();
    let frames = c["frames"].as_u64().unwrap() as usize;
    let mut out = vec![0.0f32; frames];
    let mut buf = vec![0i16; PERIOD];
    for start in (0..frames).step_by(PERIOD) {
        sid.set_chip_model(synth.chip_model());
        let regs = synth.registers(freq, start < length);
        let mut delta_t = (i64::from(PAL_CLOCK) * PERIOD as i64 / i64::from(RATE)) as i32 + 4;
        let mut made = 0usize;
        let mut clock = |sid: &mut Sid, dt: i32, made: &mut usize| {
            let mut dt = dt;
            *made += sid.clock(&mut dt, &mut buf[*made..]);
        };
        for &o in &WRITE_ORDER {
            if matches!(o, 4 | 11 | 18) {
                clock(&mut sid, WAVE_DELAY, &mut made);
                delta_t -= WAVE_DELAY;
            }
            sid.write(o, regs[usize::from(o)]);
            clock(&mut sid, WRITE_DELAY, &mut made);
            delta_t -= WRITE_DELAY;
        }
        clock(&mut sid, delta_t, &mut made);
        assert_eq!(made, PERIOD, "the legacy loop made {made} samples");
        for (o, s) in out[start..start + PERIOD].iter_mut().zip(&buf) {
            *o = f32::from(*s) / 32768.0;
        }
    }
    out
}

#[test]
fn the_legacy_loop_plays_as_the_legacy_sid() {
    let mut failures = Vec::new();
    for c in cases() {
        if c["notes"].as_array().unwrap().len() != 1 || c.get("then").is_some() {
            continue;
        }
        let name = c["name"].as_str().unwrap();
        let length = c["notes"][0]["length"].as_u64().unwrap() as usize;
        let expected = reference(name);
        let out = legacy_loop(&c);
        let (w, at) = worst(&[out.clone(), out], &expected, FADED_IN, length);
        eprintln!("{name:<24} the legacy loop: worst {:7.1} dBFS at frame {at} (frames {FADED_IN}..{length})", db(w));
        if w > 0.0 {
            failures.push(format!("{name}: {:.1} dBFS at frame {at}", db(w)));
        }
    }
    assert!(failures.is_empty(), "the legacy loop is not the legacy plugin bit for bit: {failures:#?}");
}
