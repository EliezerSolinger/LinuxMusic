//! Organic: the contract, and the A/B against the legacy note by composition (U34): `lm-dsp`'s voice is A/B-tested
//! against the legacy `playNoteImpl`'s oscillators (`lm-dsp/ab`, organic cases); here, that the plugin drives it
//! with the knobs as the legacy did (each oscillator's shape, harmonic, volume, panning and detuning, the
//! distortion and volume), the phases drawn from the note's seed, velocity as level

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::organic::DESCRIPTOR, "organic");

use common::{agree, render, state};
use lm_dsp::organic::{OrganicOsc, OrganicSettings, OrganicVoice};
use lm_dsp::oscillator::{WaveShape, prepare_wave_tables};
use lm_plugin::testing::note_on;
use lm_plugins_instruments::organic::DESCRIPTOR;
use lm_render::Rng;
use lm_state::Value;

const RATE: u32 = 48_000;
const SEED: u64 = 31;
const FRAMES: usize = 12_000;

fn legacy(s: &OrganicSettings, freq: f32, velocity: f32) -> [Vec<f32>; 2] {
    prepare_wave_tables();
    let mut rng = Rng::new(SEED);
    let mut voice = OrganicVoice::new(RATE as f32, 32, rng.next_u64());
    let mut out = vec![[0.0f32; 2]; FRAMES];
    for chunk in out.chunks_mut(32) {
        voice.render(chunk, s, freq, RATE as f32);
    }
    [out.iter().map(|f| f[0] * velocity).collect(), out.iter().map(|f| f[1] * velocity).collect()]
}

#[test]
fn ab_against_the_legacy_note_at_the_defaults() {
    let ours = render(&DESCRIPTOR, &state(&[], &[]), RATE, SEED, FRAMES, &[note_on(0, 1, 60, 1.0)]);
    let theirs = legacy(&OrganicSettings::default(), 440.0 * libm::exp2f(-9.0 / 12.0), 1.0);
    agree(&ours[0], &theirs[0], RATE, 1.0e-5, "left");
    agree(&ours[1], &theirs[1], RATE, 1.0e-5, "right");
}

#[test]
fn ab_against_the_legacy_note_with_every_knob_moved() {
    let shapes = [WaveShape::Saw, WaveShape::Square, WaveShape::Triangle, WaveShape::MoogSaw];
    let mut s = OrganicSettings { distortion: 0.4, volume: 150.0, ..OrganicSettings::default() };
    let mut params = vec![("distortion", 0.4), ("volume", 150.0)];
    let mut fields = Vec::new();
    let keys = ["1", "2", "3", "4", "5", "6", "7", "8"];
    for (i, n) in keys.iter().enumerate() {
        let o = OrganicOsc {
            shape: if i < 4 { shapes[i] } else { WaveShape::Sine },
            harmonic: (i * 2 + 1).min(17),
            volume: 30.0 + 8.0 * i as f32,
            pan: -80.0 + 20.0 * i as f32,
            detune: -300.0 + 77.0 * i as f32,
        };
        s.oscillators[i] = o;
        let k = |name: &str| -> &'static str { lm_base::intern(&format!("{name}{n}")) };
        params.push((k("harmonic"), o.harmonic as f64));
        params.push((k("volume"), f64::from(o.volume)));
        params.push((k("panning"), f64::from(o.pan)));
        params.push((k("detune"), f64::from(o.detune)));
        fields.push((k("shape"), Value::text(o.shape.key())));
    }
    let ours = render(&DESCRIPTOR, &state(&params, &fields), RATE, SEED, FRAMES, &[note_on(0, 1, 57, 0.7)]);
    let theirs = legacy(&s, 220.0, 0.7);
    agree(&ours[0], &theirs[0], RATE, 1.0e-5, "left");
    agree(&ours[1], &theirs[1], RATE, 1.0e-5, "right");
}
