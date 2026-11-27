//! A string's length is bounded whatever the pitch, detune and length knob it is given (nothing here is in the
//! knobs' ranges: the A/B covers those)

use lm_dsp::vibed::{VibedString, VibedVoice, VibratingString};

const RATE: u32 = 48_000;

#[test]
fn the_length_is_bounded_for_any_pitch() {
    for pitch in [0.0, -5.0, 1e-30, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30] {
        for (oversample, detune) in [(1, 0.0), (16, -0.1), (i32::MAX, -1e9), (i32::MIN, f32::NAN)] {
            let length = VibratingString::length_for(pitch, RATE, oversample, detune);
            assert!((1..=(1 << 21)).contains(&length), "pitch {pitch}: {length}");
        }
    }
}

#[test]
fn a_note_at_an_extreme_pitch_renders_finite_audio() {
    let mut string = VibedString::default();
    string.enabled = true;
    string.volume = 100.0;
    for frequency in [0.0, 1e-20, f32::NAN, 1e12] {
        let mut voice = VibedVoice::new(&[string.clone()], frequency, RATE, 7);
        let mut out = [[0.0f32; 2]; 256];
        voice.render(&mut out, &[string.clone()]);
        assert!(out.iter().flatten().all(|v| v.is_finite()), "frequency {frequency}");
    }
}
