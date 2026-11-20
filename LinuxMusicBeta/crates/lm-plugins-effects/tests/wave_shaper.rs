//! Wave Shaper: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::wave_shaper::DESCRIPTOR, "waveshaper");

use common::{Knob, nulls_from};
use lm_plugins_effects::wave_shaper::{DESCRIPTOR, curve};

const KNOBS: &[(&str, Knob)] = &[
    ("Input gain", Knob::Param("inputGain")),
    ("Output gain", Knob::Param("outputGain")),
    ("Clip input", Knob::Param("clipInput")),
];

/// The legacy gains ramp from their defaults over the first period (the harness turns the knobs before it); the
/// shaper has no memory, so from the second period on the two agree
#[test]
fn ab_against_the_legacy_plugin() {
    nulls_from(&DESCRIPTOR, "waveshaper", KNOBS, 256);
}

#[test]
fn a_curve_of_another_length_is_resampled_over_the_same_range() {
    let line = curve(&[0.0, 1.0]);
    assert_eq!(line[0], 0.0);
    assert_eq!(line[199], 1.0);
    assert!((line[100] - 100.0 / 199.0).abs() < 1e-6);
    assert_eq!(curve(&[0.25]), [0.25; 200]);
    assert_eq!(curve(&[]), lm_dsp::distortion_effects::default_wave_shape());
}
