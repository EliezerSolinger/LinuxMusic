//! Stereo Matrix: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::stereo_matrix::DESCRIPTOR, "stereomatrix");

use common::{Knob, nulls};
use lm_plugins_effects::stereo_matrix::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Left to Left", Knob::Param("leftToLeft")),
    ("Left to Right", Knob::Param("leftToRight")),
    ("Right to Left", Knob::Param("rightToLeft")),
    ("Right to Right", Knob::Param("rightToRight")),
];

#[test]
fn ab_against_the_legacy_plugin() {
    nulls(&DESCRIPTOR, "stereomatrix", KNOBS);
}
