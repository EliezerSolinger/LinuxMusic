//! Bitcrush: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::bitcrush::DESCRIPTOR, "bitcrush");

use common::{Knob, nulls, run_window};
use lm_plugins_effects::bitcrush::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Input gain", Knob::Param("inputGain")),
    ("Input noise", Knob::Param("inputNoise")),
    ("Output gain", Knob::Param("outputGain")),
    ("Output clip", Knob::Param("outputClip")),
    ("Sample rate", Knob::Param("sampleRate")),
    ("Stereo difference", Knob::Param("stereoDifference")),
    ("Levels", Knob::Param("levels")),
    ("Rate enabled", Knob::Param("rateEnabled")),
    ("Depth enabled", Knob::Param("depthEnabled")),
];

#[test]
fn ab_against_the_legacy_plugin_without_the_rate() {
    nulls(&DESCRIPTOR, "bitcrush-no-rate", KNOBS);
}

/// With the rate on, the legacy plugin reset its rate counters again in the second and third periods, for the change
/// flags its first one left (its plumbing, not its DSP; see the module): the first period is the same sound
#[test]
fn ab_against_the_legacy_plugin_first_period() {
    let worst = run_window(&DESCRIPTOR, "bitcrush", KNOBS, 0, 256);
    assert!(worst <= common::NULL, "{worst}");
}
