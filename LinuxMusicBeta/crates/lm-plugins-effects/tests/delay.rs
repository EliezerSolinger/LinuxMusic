//! Delay: the contract and the A/B against the legacy plugin's delay

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::delay::DESCRIPTOR, "delay");

use common::{Knob, NULL, run};
use lm_plugins_effects::delay::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("time", Knob::Param("delayTime")),
    ("feedback", Knob::Param("feedback")),
    ("lfoTime", Knob::Param("lfoTime")),
    ("lfoAmount", Knob::Param("lfoAmount")),
    ("outGain", Knob::Param("outGain")),
];

#[test]
fn ab_against_the_legacy_plugin() {
    assert!(run(&DESCRIPTOR, "delay-plain", KNOBS, 0, true) <= NULL);
}

/// Every knob changed at once in the middle, as parameter events
#[test]
fn ab_against_the_legacy_plugin_with_a_change() {
    assert!(run(&DESCRIPTOR, "delay-lfo", KNOBS, 0, true) <= NULL);
}
