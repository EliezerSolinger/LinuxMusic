//! Eq: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::eq::DESCRIPTOR, "eq");

use common::{Knob, nulls_from};
use lm_plugins_effects::eq::DESCRIPTOR;

fn knobs() -> Vec<(&'static str, Knob)> {
    let mut k = vec![
        ("Input gain", Knob::Param("inputGain")),
        ("Output gain", Knob::Param("outputGain")),
        ("HP 24", Knob::Choice("hpSlope", |v| if v != 0.0 { "24db" } else { "12db" })),
        ("HP 48", Knob::Choice("hpSlope", |v| if v != 0.0 { "48db" } else { "12db" })),
        ("LP 24", Knob::Choice("lpSlope", |v| if v != 0.0 { "24db" } else { "12db" })),
        ("LP 48", Knob::Choice("lpSlope", |v| if v != 0.0 { "48db" } else { "12db" })),
    ];
    // (legacy band, key prefix, the name of its third knob, the key of it)
    let bands: [(&'static str, &'static str, &'static str, &'static str); 8] = [
        ("HP", "hp", "res", "Res"),
        ("Low-shelf", "lowShelf", "res", "Res"),
        ("Peak 1", "peak1", "BW", "Bw"),
        ("Peak 2", "peak2", "BW", "Bw"),
        ("Peak 3", "peak3", "BW", "Bw"),
        ("Peak 4", "peak4", "BW", "Bw"),
        ("High-shelf", "highShelf", "res", "Res"),
        ("LP", "lp", "res", "Res"),
    ];
    for (legacy, key, third, third_key) in bands {
        let leak = |s: String| -> &'static str { lm_base::intern(&s) };
        k.push((leak(format!("{legacy} active")), Knob::Param(leak(format!("{key}Active")))));
        k.push((leak(format!("{legacy} freq")), Knob::Param(leak(format!("{key}Freq")))));
        k.push((leak(format!("{legacy} {third}")), Knob::Param(leak(format!("{key}{third_key}")))));
        k.push((leak(format!("{legacy} gain")), Knob::Param(leak(format!("{key}Gain")))));
    }
    k
}

/// The legacy filters crossfade into their first settings over the first call (a period of 256 frames), the plugin's
/// over its first piece (32): the filters part there and meet again once that has died away
const FROM: usize = 1024;

#[test]
fn ab_against_the_legacy_plugin_every_band() {
    nulls_from(&DESCRIPTOR, "eq-every-band", &knobs(), FROM);
}

// (eq-steep-passes, a 24 dB/oct low-pass at a resonance of 2, is where the plugin differs from the legacy on purpose:
// the resonance on one section, not on both; see the module. lm-dsp's own A/B runs it the legacy way and nulls)

#[test]
fn ab_against_the_legacy_plugin_before_a_sweep() {
    nulls_from(&DESCRIPTOR, "eq-sweep", &knobs(), FROM);
}
