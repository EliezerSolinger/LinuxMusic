//! Crossover EQ: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::crossover_eq::DESCRIPTOR, "crossovereq");

use common::{Knob, nulls};
use lm_plugins_effects::crossover_eq::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Band 1/2 Crossover", Knob::Param("crossover12")),
    ("Band 2/3 Crossover", Knob::Param("crossover23")),
    ("Band 3/4 Crossover", Knob::Param("crossover34")),
    ("Band 1 Gain", Knob::Param("gain1")),
    ("Band 2 Gain", Knob::Param("gain2")),
    ("Band 3 Gain", Knob::Param("gain3")),
    ("Band 4 Gain", Knob::Param("gain4")),
    ("Mute Band 1", Knob::Param("band1Active")),
    ("Mute Band 2", Knob::Param("band2Active")),
    ("Mute Band 3", Knob::Param("band3Active")),
    ("Mute Band 4", Knob::Param("band4Active")),
];

#[test]
fn ab_against_the_legacy_plugin_at_its_defaults() {
    nulls(&DESCRIPTOR, "crossovereq-defaults", KNOBS);
}

#[test]
fn ab_against_the_legacy_plugin() {
    nulls(&DESCRIPTOR, "crossovereq", KNOBS);
}
