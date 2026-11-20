//! BitInvader: the contract, and the A/B against the legacy note by composition (U34): `lm-dsp`'s voice is the
//! legacy `BSynth` (its unit tests, and the drawn wave's A/B in `lm-dsp/ab`); here, that the plugin drives it as the
//! legacy `BitInvader::playNoteImpl` did: the graph's first `sampleLength` points, the normalization read at the
//! note's start, the table of the note's pitch, velocity as level

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::bit_invader::DESCRIPTOR, "bitinvader");

use common::{agree, render, state};
use lm_dsp::bit_invader::{BitInvaderVoice, BitInvaderWave};
use lm_plugin::testing::note_on;
use lm_plugins_instruments::bit_invader::{DESCRIPTOR, default_wave};
use lm_state::Value;

const RATE: u32 = 44_100;
const FRAMES: usize = 20_000;

fn legacy(points: &[f32], interpolation: bool, aa: bool, normalize: bool, freq: f32, velocity: f32) -> Vec<f32> {
    let wave = BitInvaderWave::new(points, interpolation, aa);
    let mut voice = BitInvaderVoice::new(&wave, normalize);
    let mut out = vec![[0.0f32; 2]; FRAMES];
    for chunk in out.chunks_mut(32) {
        voice.render(chunk, &wave, freq, RATE as f32);
    }
    out.iter().map(|f| f[0] * velocity).collect()
}

fn reals(v: &[f32]) -> Value {
    Value::list(v.iter().map(|x| Value::real(f64::from(*x)).unwrap()))
}

#[test]
fn ab_against_the_legacy_note_at_the_defaults() {
    let ours = render(&DESCRIPTOR, &state(&[], &[]), RATE, 1, FRAMES, &[note_on(0, 1, 69, 1.0)]);
    agree(&ours[0], &legacy(&default_wave(), false, true, false, 440.0, 1.0), RATE, 1.0e-6, "defaults");
    assert_eq!(ours[0], ours[1]);
}

#[test]
fn ab_against_the_legacy_note_drawn_short_interpolated_normalized() {
    let points: Vec<f32> = (0..200).map(|i| ((i * 37 % 101) as f32 / 101.0 - 0.5) * 0.6).collect();
    let fields = [
        ("wave", reals(&points)),
        ("sampleLength", Value::int(37)),
        ("interpolation", Value::bool(true)),
        ("antiAliasing", Value::bool(false)),
    ];
    let s = state(&[("normalize", 1.0)], &fields);
    let ours = render(&DESCRIPTOR, &s, RATE, 1, FRAMES, &[note_on(0, 1, 57, 0.5)]);
    agree(&ours[0], &legacy(&points[..37], true, false, true, 220.0, 0.5), RATE, 1.0e-6, "drawn");
    let s = state(&[], &[("wave", reals(&points)), ("sampleLength", Value::int(64))]);
    let ours = render(&DESCRIPTOR, &s, RATE, 1, FRAMES, &[note_on(0, 1, 81, 1.0)]);
    agree(&ours[0], &legacy(&points[..64], false, true, false, 880.0, 1.0), RATE, 1.0e-6, "band limited");
}
