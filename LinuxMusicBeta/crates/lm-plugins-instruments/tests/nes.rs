//! Nescaline: the contract, and the A/B against the legacy note by composition (U34): `lm-dsp`'s voice is the legacy
//! `NesObject` (its unit tests and A/B); here, that the plugin drives it as the legacy `NesInstrument::playNoteImpl`
//! did: every knob read as the note plays, the note's pitch, the dither seeded from the note, velocity as level

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::nes::DESCRIPTOR, "nes");

use common::{agree, render, state};
use lm_dsp::nes::{NesSettings, NesVoice};
use lm_plugin::testing::note_on;
use lm_plugins_instruments::nes::{DESCRIPTOR, PARAMS, set};
use lm_render::Rng;

const RATE: u32 = 48_000;
const SEED: u64 = 23;
const FRAMES: usize = 24_000;

fn compare(params: &[(&str, f64)], key: u8, velocity: f32) {
    let mut s = NesSettings::default();
    for (name, v) in params {
        set(&mut s, PARAMS.iter().position(|p| p.0 == *name).unwrap(), *v as f32);
    }
    let freq = 440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0);
    let mut rng = Rng::new(SEED);
    let mut voice = NesVoice::new(&s, freq, RATE, rng.next_u64());
    let mut theirs = vec![[0.0f32; 2]; FRAMES];
    for chunk in theirs.chunks_mut(32) {
        voice.render(chunk, &s, freq);
    }
    let ours = render(&DESCRIPTOR, &state(params, &[]), RATE, SEED, FRAMES, &[note_on(0, 1, key, velocity)]);
    for ch in 0..2 {
        let t: Vec<f32> = theirs.iter().map(|f| f[ch] * velocity).collect();
        agree(&ours[ch], &t, RATE, 1.0e-6, "nes");
    }
    assert!(ours[0].iter().any(|v| *v != 0.0), "it sounds");
}

#[test]
fn ab_against_the_legacy_note_at_the_defaults() {
    compare(&[], 69, 1.0);
}

#[test]
fn ab_against_the_legacy_note_with_envelopes_sweeps_and_vibrato() {
    let params = [
        ("ch1Coarse", 12.0),
        ("ch1Volume", 11.0),
        ("ch1EnvEnabled", 1.0),
        ("ch1EnvLooped", 1.0),
        ("ch1EnvLength", 6.0),
        ("ch1DutyCycle", 1.0),
        ("ch1SweepEnabled", 1.0),
        ("ch1SweepAmount", -3.0),
        ("ch1SweepRate", 4.0),
        ("ch2Coarse", -7.0),
        ("ch2EnvEnabled", 1.0),
        ("ch2EnvLength", 9.0),
        ("ch2DutyCycle", 3.0),
        ("ch2SweepEnabled", 1.0),
        ("ch2SweepAmount", 2.0),
        ("ch2SweepRate", 2.0),
        ("ch3Coarse", -12.0),
        ("ch3Volume", 9.0),
        ("masterVolume", 1.4),
        ("vibrato", 7.0),
    ];
    compare(&params, 57, 0.7);
}

#[test]
fn ab_against_the_legacy_note_with_the_noise() {
    let fixed = [
        ("ch1Enabled", 0.0),
        ("ch2Enabled", 0.0),
        ("ch3Enabled", 0.0),
        ("ch4Enabled", 1.0),
        ("ch4EnvEnabled", 1.0),
        ("ch4EnvLooped", 1.0),
        ("ch4EnvLength", 4.0),
        ("ch4NoiseMode", 1.0),
        ("ch4NoiseFreq", 9.0),
        ("ch4Sweep", 3.0),
    ];
    compare(&fixed, 64, 1.0);
    let by_note = [("ch3Enabled", 0.0), ("ch4Enabled", 1.0), ("ch4NoiseFreqMode", 1.0), ("ch4NoiseQuantize", 0.0)];
    compare(&by_note, 76, 0.9);
    compare(&[("ch4Enabled", 1.0), ("ch4NoiseFreqMode", 1.0), ("ch4Sweep", -5.0)], 40, 1.0);
}
