//! Bass Booster: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::bass_booster::DESCRIPTOR, "bassbooster");

use common::{Knob, run};
use lm_plugins_effects::bass_booster::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] =
    &[("Frequency", Knob::Param("frequency")), ("Gain", Knob::Param("gain")), ("Ratio", Knob::Param("ratio"))];

/// The legacy harness turns the knobs before the first period, so the legacy gain ramps from its default over it: the
/// gain multiplies the output only (the filter's memory never sees it), so from the second period on the two agree
#[test]
fn ab_against_the_legacy_plugin() {
    let worst = run(&DESCRIPTOR, "bassbooster", KNOBS, 256, false);
    assert!(worst <= common::NULL, "{worst}");
}
