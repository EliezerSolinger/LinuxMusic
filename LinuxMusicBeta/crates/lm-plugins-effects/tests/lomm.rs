//! LOMM: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::lomm::DESCRIPTOR, "lomm");

use common::{Knob, nulls};
use lm_plugins_effects::lomm::{DESCRIPTOR, LEGACY_NAMES};

fn knobs() -> Vec<(&'static str, Knob)> {
    LEGACY_NAMES.iter().map(|(legacy, key)| (*legacy, Knob::Param(key))).collect()
}

#[test]
fn ab_against_the_legacy_plugin() {
    for case in ["lomm-defaults", "lomm-deep-linked", "lomm-lookahead-midside", "lomm-feedback"] {
        nulls(&DESCRIPTOR, case, &knobs());
    }
}
