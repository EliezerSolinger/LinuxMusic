//! sfxr: the contract, and the A/B against the legacy note by composition (U34): `lm-dsp`'s synth is the legacy
//! `SfxrSynth` (its unit tests); here, that the plugin drives it as the legacy `SfxrInstrument::playNoteImpl` did: the
//! knobs at the note's start, the sound at 44.1 kHz stretched to the note's pitch over A4, the note's end with the
//! sound's, velocity as level

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::sfxr::DESCRIPTOR, "sfxr");

use common::{agree, render, state};
use lm_dsp::sfxr::{SfxrVoice, SfxrWave};
use lm_plugin::testing::note_on;
use lm_plugins_instruments::sfxr::{BASE_FREQ, DESCRIPTOR, PARAMS, WAVE_KEYS, settings};
use lm_render::Rng;
use lm_state::Value;

const RATE: u32 = 48_000;
const SEED: u64 = 17;
const FRAMES: usize = 48_000;

fn compare(params: &[(&str, f64)], wave: usize, key: u8, velocity: f32) {
    let mut k: [f32; 22] = core::array::from_fn(|i| PARAMS[i].2 as f32);
    for (name, v) in params {
        k[PARAMS.iter().position(|p| p.0 == *name).unwrap()] = *v as f32;
    }
    let s = settings(&k, SfxrWave::ALL[wave]);
    let mut rng = Rng::new(SEED);
    let mut voice = SfxrVoice::new(&s, rng.next_u64());
    let freq = 440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0);
    let mut theirs = vec![[0.0f32; 2]; FRAMES];
    for chunk in theirs.chunks_mut(32) {
        voice.render(chunk, f64::from(freq) / f64::from(BASE_FREQ), RATE as f32);
    }
    let fields = [("wave", Value::text(WAVE_KEYS[wave]))];
    let ours = render(&DESCRIPTOR, &state(params, &fields), RATE, SEED, FRAMES, &[note_on(0, 1, key, velocity)]);
    for ch in 0..2 {
        let t: Vec<f32> = theirs.iter().map(|f| f[ch] * velocity).collect();
        agree(&ours[ch], &t, RATE, 1.0e-6, "sfxr");
    }
    assert!(ours[0].iter().any(|v| *v != 0.0), "it sounds");
}

#[test]
fn ab_against_the_legacy_note_at_the_defaults() {
    compare(&[], 0, 69, 1.0);
}

#[test]
fn ab_against_the_legacy_note_swept_vibrato_phaser_filters() {
    let params = [
        ("startFrequency", 0.6),
        ("slide", -0.3),
        ("deltaSlide", 0.1),
        ("vibratoDepth", 0.4),
        ("vibratoSpeed", 0.5),
        ("changeAmount", 0.5),
        ("changeSpeed", 0.6),
        ("dutySweep", 0.2),
        ("phaserOffset", 0.3),
        ("phaserSweep", -0.2),
        ("lpFilterCutoff", 0.7),
        ("lpFilterResonance", 0.5),
        ("hpFilterCutoff", 0.1),
        ("repeatSpeed", 0.4),
        ("sustainPunch", 0.5),
    ];
    compare(&params, 1, 64, 0.8);
    compare(&[("decayTime", 0.6)], 3, 81, 1.0);
}

#[test]
fn the_note_ends_with_its_sound() {
    let ours = render(
        &DESCRIPTOR,
        &state(&[("sustainTime", 0.1), ("decayTime", 0.1)], &[]),
        RATE,
        SEED,
        FRAMES,
        &[note_on(0, 1, 69, 1.0)],
    );
    assert!(ours[0][FRAMES - 1000..].iter().all(|v| *v == 0.0), "silent, its voice freed");
}
