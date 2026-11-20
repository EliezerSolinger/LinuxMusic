//! Vibed: the contract, and the A/B against the legacy note by composition (U34): `lm-dsp`'s strings are A/B-tested
//! against the legacy `VibratingString` (`lm-dsp/ab`, vibed cases); here, that the plugin drives them as the legacy
//! `Vibed::playNoteImpl` did: the enabled strings set at the note's start with their knobs, harmonic, length and drawn
//! impulse, the randomness from the note's seed, velocity as level

mod common;

lm_plugin::contract_tests!(lm_plugins_instruments::vibed::DESCRIPTOR, "vibed");

use common::{agree, render, state};
use lm_dsp::vibed::{VibedString, VibedVoice};
use lm_plugin::testing::note_on;
use lm_plugins_instruments::vibed::{DESCRIPTOR, HARMONIC_KEYS};
use lm_render::Rng;
use lm_state::Value;

const RATE: u32 = 44_100;
const SEED: u64 = 5;
const FRAMES: usize = 20_000;

fn legacy(strings: &[VibedString], freq: f32, velocity: f32) -> [Vec<f32>; 2] {
    let mut rng = Rng::new(SEED);
    let mut voice = VibedVoice::new(strings, freq, RATE, rng.next_u64());
    let mut out = vec![[0.0f32; 2]; FRAMES];
    for chunk in out.chunks_mut(32) {
        voice.render(chunk, strings);
    }
    [out.iter().map(|f| f[0] * velocity).collect(), out.iter().map(|f| f[1] * velocity).collect()]
}

fn compare(strings: &[VibedString; 9], key: u8, velocity: f32) {
    let mut params = Vec::new();
    let mut fields = Vec::new();
    for (i, s) in strings.iter().enumerate() {
        let n = i + 1;
        let k = |name: &str| -> &'static str { lm_base::intern(&format!("{name}{n}")) };
        // (the knob as written in a project: its decimal value, not the f32's widening)
        let d = |v: f32| (f64::from(v) * 1.0e6).round() / 1.0e6;
        params.extend([
            (k("string"), if s.enabled { 1.0 } else { 0.0 }),
            (k("volume"), d(s.volume)),
            (k("stiffness"), d(s.stiffness)),
            (k("pick"), d(s.pick)),
            (k("pickup"), d(s.pickup)),
            (k("panning"), d(s.pan)),
            (k("detune"), d(s.detune)),
            (k("fuzziness"), d(s.fuzziness)),
            (k("impulse"), if s.impulse { 1.0 } else { 0.0 }),
        ]);
        fields.push((k("harmonic"), Value::text(HARMONIC_KEYS[s.harmonic])));
        fields.push((k("length"), Value::int(i64::from(s.length))));
        fields.push((k("shape"), Value::list(s.shape.iter().map(|v| Value::real(f64::from(*v)).unwrap()))));
    }
    let ours = render(&DESCRIPTOR, &state(&params, &fields), RATE, SEED, FRAMES, &[note_on(0, 1, key, velocity)]);
    let freq = 440.0 * libm::exp2f((f32::from(key) - 69.0) / 12.0);
    let theirs = legacy(strings, freq, velocity);
    agree(&ours[0], &theirs[0], RATE, 1.0e-6, "left");
    agree(&ours[1], &theirs[1], RATE, 1.0e-6, "right");
}

fn defaults() -> [VibedString; 9] {
    core::array::from_fn(|i| VibedString { enabled: i == 0, ..VibedString::default() })
}

#[test]
fn ab_against_the_legacy_note_at_the_defaults() {
    compare(&defaults(), 57, 1.0);
}

#[test]
fn ab_against_the_legacy_note_with_three_strings() {
    let mut s = defaults();
    s[0] = VibedString { harmonic: 1, pick: 0.02, pickup: 0.03, stiffness: 0.01, pan: -0.6, length: 4, ..s[0].clone() };
    s[3] = VibedString {
        enabled: true,
        harmonic: 4,
        volume: 70.0,
        pan: 0.5,
        detune: 0.03,
        fuzziness: 0.3,
        ..s[3].clone()
    };
    let saw: [f32; 128] = core::array::from_fn(|i| i as f32 / 64.0 - 1.0);
    s[8] = VibedString { enabled: true, harmonic: 0, impulse: true, shape: saw, length: 2, ..s[8].clone() };
    compare(&s, 52, 0.6);
}
