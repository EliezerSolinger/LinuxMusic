//! Granular Pitch Shifter: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::granular_pitch_shifter::DESCRIPTOR, "granularpitchshifter");

use common::{Knob, nulls};
use lm_plugins_effects::granular_pitch_shifter::{DESCRIPTOR, RANGE_KEYS};

const KNOBS: &[(&str, Knob)] = &[
    ("Pitch", Knob::Param("pitch")),
    ("Grain Size", Knob::Param("grainSize")),
    ("Spray", Knob::Param("spray")),
    ("Jitter", Knob::Param("jitter")),
    ("Twitch", Knob::Param("twitch")),
    ("Pitch Stereo Spread", Knob::Param("pitchStereoSpread")),
    ("Spray Stereo", Knob::Param("sprayStereo")),
    ("Shape", Knob::Param("shape")),
    ("Fade Length", Knob::Param("fadeLength")),
    ("Feedback", Knob::Param("feedback")),
    ("Minimum Allowed Latency", Knob::Param("minimumAllowedLatency")),
    ("Prefilter", Knob::Param("prefilter")),
    ("Density", Knob::Param("density")),
    ("Glide", Knob::Param("glide")),
    ("Ring Buffer Length", Knob::Choice("ringBufferLength", |v| RANGE_KEYS[v as usize])),
];

/// At its defaults (without spray, whose randomness is the legacy C library's), and with every knob but the pitch moved:
/// both bit for bit
#[test]
fn ab_against_the_legacy_plugin() {
    for case in ["granular-defaults-without-spray", "granular-grains"] {
        nulls(&DESCRIPTOR, case, KNOBS);
    }
}

// (granular-fifth, granular-dense-feedback and granular-range cannot be compared through the plugin: the legacy harness
// turns the knobs before the first period, so the sample-exact pitch ramps from its default over it, and the ring's
// length follows a period late; the grains then fall elsewhere for good. lm-dsp's own A/B runs them the legacy way and
// nulls)
