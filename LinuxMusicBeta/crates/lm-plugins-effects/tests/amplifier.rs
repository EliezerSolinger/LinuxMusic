//! Amplifier: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::amplifier::DESCRIPTOR, "amplifier");

use common::{Knob, nulls_from};
use lm_plugins_effects::amplifier::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Volume", Knob::Param("volume")),
    ("Panning", Knob::Param("panning")),
    ("Left gain", Knob::Param("leftGain")),
    ("Right gain", Knob::Param("rightGain")),
];

/// The legacy harness turns the knobs before the first period, so the legacy value buffers ramp from the defaults
/// over it; a plugin built with the knobs where they are starts there: compared from the second period
#[test]
fn ab_against_the_legacy_plugin() {
    nulls_from(&DESCRIPTOR, "amplifier", KNOBS, 256);
}
