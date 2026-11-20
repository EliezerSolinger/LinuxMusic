//! Dispersion: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::dispersion::DESCRIPTOR, "dispersion");

use common::{Knob, nulls};
use lm_plugins_effects::dispersion::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Amount", Knob::Param("amount")),
    ("Frequency", Knob::Param("frequency")),
    ("Resonance", Knob::Param("resonance")),
    ("Feedback", Knob::Param("feedback")),
    ("DC Offset Removal", Knob::Param("dcOffsetRemoval")),
];

#[test]
fn ab_against_the_legacy_plugin() {
    nulls(&DESCRIPTOR, "dispersion", KNOBS);
}
