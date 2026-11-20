//! Dual Filter: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::dual_filter::DESCRIPTOR, "dualfilter");

use common::{Knob, NULL, run};
use lm_dsp::basic_filters::FilterType;
use lm_plugins_effects::dual_filter::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Filter 1 enabled", Knob::Param("filter1Enabled")),
    ("Filter 1 type", Knob::Choice("filter1Type", |v| FilterType::ALL[v as usize].key())),
    ("Cutoff frequency 1", Knob::Param("cutoff1")),
    ("Q/Resonance 1", Knob::Param("resonance1")),
    ("Gain 1", Knob::Param("gain1")),
    ("Mix", Knob::Param("mix")),
    ("Filter 2 enabled", Knob::Param("filter2Enabled")),
    ("Filter 2 type", Knob::Choice("filter2Type", |v| FilterType::ALL[v as usize].key())),
    ("Cutoff frequency 2", Knob::Param("cutoff2")),
    ("Q/Resonance 2", Knob::Param("resonance2")),
    ("Gain 2", Knob::Param("gain2")),
];

// (dualfilter and dualfilter-exact-moog cannot be compared through the plugin: their Moog, at a resonance of 2.5,
// self-oscillates, so the phase of its tone depends on how the knobs got there, and the legacy harness ramped them
// from their defaults over the first period while the plugin starts where they are. lm-dsp's own A/B runs them with
// the legacy ramps and nulls)

/// The legacy knobs ramp from their defaults over the first period (the harness turns them before it), and the
/// filter remembers that period: compared once it has died away
#[test]
fn ab_against_the_legacy_plugin_one_formant() {
    let worst = run(&DESCRIPTOR, "dualfilter-formant-one", KNOBS, 1024, false);
    assert!(worst <= NULL, "{worst}");
}
