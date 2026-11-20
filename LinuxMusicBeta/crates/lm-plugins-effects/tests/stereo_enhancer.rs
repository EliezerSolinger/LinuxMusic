//! Stereo Enhancer: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::stereo_enhancer::DESCRIPTOR, "stereoenhancer");

use common::{Knob, nulls};
use lm_plugins_effects::stereo_enhancer::DESCRIPTOR;

#[test]
fn ab_against_the_legacy_plugin() {
    nulls(&DESCRIPTOR, "stereoenhancer", &[("Width", Knob::Param("width"))]);
}
