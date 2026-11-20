//! LB302: the contract, and the A/B against the legacy synth by composition (U34): `lm-dsp`'s synth is A/B-tested
//! against the legacy one (`lm-dsp/ab`, lb302 cases, notes at period starts); here, that the plugin drives it as the
//! legacy did: the newest note sounds, a release lets the stream decay, the knobs each period, the note's volume

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::lb302::DESCRIPTOR, "lb302");

use common::{render, state};
use lm_dsp::lb302::{Lb302, VcoShape};
use lm_plugin::testing::{note_off, note_on};
use lm_plugins_instruments::lb302::{DESCRIPTOR, settings};
use lm_state::Value;

const RATE: u32 = 44_100;

/// The notes: (frame, key, velocity, release frame)
const NOTES: [(usize, u8, f32, usize); 4] =
    [(0, 45, 1.0, 4096), (4096, 50, 0.8, 9600), (9600, 43, 0.5, 14_400), (14_400, 57, 1.0, 20_000)];

fn legacy(knobs: &[f32; 9], shape: VcoShape, frames: usize) -> Vec<[f32; 2]> {
    let mut synth = Lb302::new(RATE as f32, 0);
    let mut out = vec![[0.0f32; 2]; frames];
    let mut playing: Option<(f32, usize)> = None;
    for start in (0..frames).step_by(32) {
        synth.prepare_period(&settings(knobs, shape));
        for (at, key, velocity, off) in NOTES {
            if at == start {
                synth.note_on(440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0));
                playing = Some((velocity, off));
            }
        }
        let (held, volume) = match playing {
            Some((v, off)) => (start < off, [v, v]),
            None => (false, [0.0; 2]),
        };
        let end = (start + 32).min(frames);
        synth.render(&mut out[start..end], if held { end - start } else { 0 }, held, volume);
    }
    out
}

fn compare(params: &[(&str, f64)], shape: &str) {
    let frames = 24_000;
    let mut events = Vec::new();
    for (i, (at, key, velocity, off)) in NOTES.iter().enumerate() {
        events.push(note_on(*at as u64, i as u32 + 1, *key, *velocity));
        events.push(note_off(*off as u64, i as u32 + 1, *key));
    }
    let ours = render(&DESCRIPTOR, &state(params, &[("waveform", Value::text(shape))]), RATE, 1, frames, &events);
    let defaults = [0.75, 0.75, 0.1, 0.1, 0.0, 0.6, 0.0, 0.0, 0.0];
    let keys = lm_plugins_instruments::lb302::KEYS;
    let mut k = defaults;
    for (key, v) in params {
        k[keys.iter().position(|x| x == key).unwrap()] = *v as f32;
    }
    let i = lm_plugins_instruments::lb302::WAVEFORM_KEYS.iter().position(|x| *x == shape).unwrap();
    let theirs = legacy(&k, VcoShape::ALL[i], frames);
    for (n, t) in theirs.iter().enumerate() {
        for ch in 0..2 {
            assert!(
                (ours[ch][n] - t[ch]).abs() <= 1.0e-6,
                "{params:?} {shape}: frame {n}: {} and {}",
                ours[ch][n],
                t[ch]
            );
        }
    }
}

#[test]
fn ab_against_the_legacy_synth_at_its_defaults() {
    compare(&[], "bl-saw");
}

#[test]
fn ab_against_the_legacy_synth_sliding_through_the_24_db_filter() {
    compare(
        &[
            ("slide", 1.0),
            ("db24", 1.0),
            ("vcfResonance", 1.1),
            ("vcfCutoffFrequency", 0.4),
            ("vcfEnvelopeMod", 0.8),
            ("distortion", 0.3),
        ],
        "saw",
    );
}

#[test]
fn ab_against_the_legacy_synth_dead_and_shaped() {
    compare(&[("vcfEnvelopeDecay", 0.7), ("distortion", 0.5), ("dead", 1.0)], "round-square");
    compare(&[("vcfResonance", 0.2)], "bl-moog");
}
