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
pub mod dispersion;
pub mod stereo_enhancer;
pub mod stereo_matrix;
pub mod wave_shaper;

use lm_plugin::Descriptor;

/// Every effect of this crate, for the registry
pub static ALL: &[&Descriptor] = &[
    &amplifier::DESCRIPTOR,
    &bass_booster::DESCRIPTOR,
    &bitcrush::DESCRIPTOR,
    &dispersion::DESCRIPTOR,
    &stereo_enhancer::DESCRIPTOR,
    &stereo_matrix::DESCRIPTOR,
    &wave_shaper::DESCRIPTOR,
];
