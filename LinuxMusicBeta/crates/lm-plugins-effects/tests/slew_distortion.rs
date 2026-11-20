//! Slew Distortion: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::slew_distortion::DESCRIPTOR, "slewdistortion");

use common::{Knob, nulls};
use lm_dsp::slew_distortion::SlewShape;
use lm_plugins_effects::slew_distortion::DESCRIPTOR;

fn knobs() -> Vec<(&'static str, Knob)> {
    let mut k = vec![
        ("DC Offset Remover", Knob::Param("dcRemove")),
        ("Multiband", Knob::Param("multiband")),
        ("Oversample", Knob::Param("oversample")),
        ("Split", Knob::Param("split")),
        ("Type 1", Knob::Choice("type1", |v| SlewShape::ALL[v as usize].key())),
        ("Type 2", Knob::Choice("type2", |v| SlewShape::ALL[v as usize].key())),
    ];
    let names = [
        ("Drive", "drive"),
        ("Slew Up", "slewUp"),
        ("Slew Down", "slewDown"),
        ("Bias", "bias"),
        ("Warp", "warp"),
        ("Crush", "crush"),
        ("Out Vol", "outVol"),
        ("Attack", "attack"),
        ("Release", "release"),
        ("Dynamics", "dynamics"),
        ("Dynamic Slew", "dynamicSlew"),
        ("Mix", "mix"),
        ("Slew Link", "slewLink"),
    ];
    for band in 1..=2 {
        for (legacy, key) in names {
            let leak = |s: String| -> &'static str { lm_base::intern(&s) };
            k.push((leak(format!("{legacy} {band}")), Knob::Param(leak(format!("{key}{band}")))));
        }
    }
    k
}

#[test]
fn ab_against_the_legacy_plugin() {
    for case in ["slew-defaults", "slew-tanh", "slew-multiband-oversampled", "slew-32x", "slew-full-rectify"] {
        nulls(&DESCRIPTOR, case, &knobs());
    }
}
