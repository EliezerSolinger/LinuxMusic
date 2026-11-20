//! Flanger: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::flanger::DESCRIPTOR, "flanger");

use common::{Knob, nulls};
use lm_plugins_effects::flanger::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Delay samples", Knob::Param("delayTime")),
    ("LFO frequency", Knob::Param("lfoPeriod")),
    ("Amount", Knob::Param("lfoAmount")),
    ("Stereo phase", Knob::Param("stereoPhase")),
    ("Feedback", Knob::Param("feedback")),
    ("Invert", Knob::Param("invert")),
];

#[test]
fn ab_against_the_legacy_plugin_at_its_defaults() {
    nulls(&DESCRIPTOR, "flanger-defaults", KNOBS);
}

#[test]
fn ab_against_the_legacy_plugin() {
    nulls(&DESCRIPTOR, "flanger", KNOBS);
}
