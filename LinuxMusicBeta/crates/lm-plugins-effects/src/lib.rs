//! The legacy program's effect plugins (F-EFX), each on its effect ported to `lm-dsp` with its A/B: one module and
//! one `static Descriptor` per effect, the shared processor in [`effect`].
//!
//! Every effect gives the wet signal only: the dry/wet mix is the host's (as in the legacy program). The parameter
//! keys are the legacy knobs' names in camelCase, with their ranges, defaults and units; a legacy switch is a stepped
//! parameter of 0 to 1, a legacy list of named options a `Choice` field (U30).

#![forbid(unsafe_code)]

pub mod effect;

pub mod amplifier;
pub mod bass_booster;
pub mod bitcrush;
pub mod compressor;
pub mod crossover_eq;
pub mod delay;
pub mod dispersion;
pub mod dual_filter;
pub mod dynamics_processor;
pub mod eq;
pub mod flanger;
pub mod frequency_shifter;
pub mod granular_pitch_shifter;
pub mod lomm;
pub mod multitap_echo;
pub mod oscilloscope;
pub mod reverb_sc;
pub mod slew_distortion;
pub mod stereo_enhancer;
pub mod stereo_matrix;
pub mod wave_shaper;

use lm_plugin::Descriptor;

/// Every effect of this crate, for the registry
pub static ALL: &[&Descriptor] = &[
    &amplifier::DESCRIPTOR,
    &bass_booster::DESCRIPTOR,
    &bitcrush::DESCRIPTOR,
    &compressor::DESCRIPTOR,
    &crossover_eq::DESCRIPTOR,
    &delay::DESCRIPTOR,
    &dispersion::DESCRIPTOR,
    &dual_filter::DESCRIPTOR,
    &dynamics_processor::DESCRIPTOR,
    &eq::DESCRIPTOR,
    &flanger::DESCRIPTOR,
    &frequency_shifter::DESCRIPTOR,
    &granular_pitch_shifter::DESCRIPTOR,
    &lomm::DESCRIPTOR,
    &multitap_echo::DESCRIPTOR,
    &oscilloscope::DESCRIPTOR,
    &reverb_sc::DESCRIPTOR,
    &slew_distortion::DESCRIPTOR,
    &stereo_enhancer::DESCRIPTOR,
    &stereo_matrix::DESCRIPTOR,
    &wave_shaper::DESCRIPTOR,
];
