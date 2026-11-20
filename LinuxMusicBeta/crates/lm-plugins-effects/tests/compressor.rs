//! Compressor: the contract and the A/B against the legacy plugin

mod common;

lm_plugin::contract_tests!(lm_plugins_effects::compressor::DESCRIPTOR, "compressor");

use common::{Knob, nulls};
use lm_dsp::compressor::StereoLink;
use lm_plugins_effects::compressor::DESCRIPTOR;

const KNOBS: &[(&str, Knob)] = &[
    ("Threshold", Knob::Param("threshold")),
    ("Ratio", Knob::Param("ratio")),
    ("Attack", Knob::Param("attack")),
    ("Release", Knob::Param("release")),
    ("Knee", Knob::Param("knee")),
    ("Hold", Knob::Param("hold")),
    ("Range", Knob::Param("range")),
    ("RMS Size", Knob::Param("rmsSize")),
    ("Mid/Side", Knob::Param("midSide")),
    ("Peak Mode", Knob::Param("peakMode")),
    ("Lookahead Length", Knob::Param("lookaheadLength")),
    ("Input Balance", Knob::Param("inputBalance")),
    ("Output Balance", Knob::Param("outputBalance")),
    ("Limiter", Knob::Param("limiter")),
    ("Output Gain", Knob::Param("outputGain")),
    ("Input Gain", Knob::Param("inputGain")),
    ("Blend", Knob::Param("blend")),
    ("Stereo Balance", Knob::Param("stereoBalance")),
    ("Auto Makeup Gain", Knob::Param("autoMakeupGain")),
    ("Audition", Knob::Param("audition")),
    ("Feedback", Knob::Param("feedback")),
    ("Auto Attack", Knob::Param("autoAttack")),
    ("Auto Release", Knob::Param("autoRelease")),
    ("Lookahead", Knob::Param("lookahead")),
    ("Tilt", Knob::Param("tilt")),
    ("Tilt Frequency", Knob::Param("tiltFrequency")),
    ("Stereo Link", Knob::Choice("stereoLink", |v| StereoLink::ALL[v as usize].key())),
    ("Mix", Knob::Param("mix")),
];

#[test]
fn ab_against_the_legacy_plugin() {
    for case in [
        "compressor-default",
        "compressor-peak-hard",
        "compressor-limiter-lookahead-hold",
        "compressor-auto-crest-tilt",
        "compressor-midside-blend-range",
        "compressor-feedback-audition",
        "compressor-limiter-lookahead",
        "compressor-limiter-hold",
        "compressor-hold",
        "compressor-hold-makeup",
    ] {
        nulls(&DESCRIPTOR, case, KNOBS);
    }
}
