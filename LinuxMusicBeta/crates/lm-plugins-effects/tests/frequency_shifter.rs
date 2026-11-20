//! Frequency Shifter: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::frequency_shifter::DESCRIPTOR, "frequencyshifter");

use common::{Knob, nulls};
use lm_dsp::frequency_shifter::RouteMode;
use lm_plugins_effects::frequency_shifter::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Mix", Knob::Param("mix")),
    ("Frequency Shift", Knob::Param("frequencyShift")),
    ("Spread Shift", Knob::Param("spreadShift")),
    ("Ring", Knob::Param("ring")),
    ("Feedback", Knob::Param("feedback")),
    ("Delay Length", Knob::Param("delayLength")),
    ("Fine Delay Length", Knob::Param("fineDelayLength")),
    ("Delay Damping", Knob::Param("delayDamping")),
    ("Delay Glide", Knob::Param("delayGlide")),
    ("LFO Amount", Knob::Param("lfoAmount")),
    ("LFO Rate", Knob::Param("lfoRate")),
    ("LFO StereoPhase", Knob::Param("lfoStereoPhase")),
    ("Glide", Knob::Param("glide")),
    ("Tone", Knob::Param("tone")),
    ("Phase", Knob::Param("phase")),
    ("Antireflect", Knob::Param("antireflect")),
    ("Route Mode", Knob::Choice("routeMode", |v| RouteMode::ALL[v as usize].key())),
    ("Harmonics", Knob::Param("harmonics")),
];

#[test]
fn ab_against_the_legacy_plugin_at_its_defaults() {
    nulls(&DESCRIPTOR, "freqshift-defaults", KNOBS);
}

#[test]
fn ab_against_the_legacy_plugin() {
    nulls(&DESCRIPTOR, "freqshift", KNOBS);
}

#[test]
fn ab_against_the_legacy_plugin_barberpole() {
    nulls(&DESCRIPTOR, "freqshift-barberpole", KNOBS);
}
