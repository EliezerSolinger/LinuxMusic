//! The legacy program's instrument plugins (F-INS), each on its instrument ported to `lm-dsp` with its A/B: one module
//! and one `static Descriptor` per instrument, each a [`lm_plugin::instrument::Synth`] inside the instrument functions
//! (envelopes, LFOs, filter, chords, arpeggio, mono and legato, note expressions).
//!
//! The parameter keys are the legacy knobs' names in camelCase, with their ranges, defaults and units; a legacy switch
//! is a stepped parameter of 0 to 1, a legacy list of named options a `Choice` field (U30).

#![forbid(unsafe_code)]

pub mod audio_file_processor;
pub mod bit_invader;
pub mod kicker;
pub mod lb302;
pub mod monstro;
pub mod nes;
pub mod organic;
pub mod sfxr;
pub mod src_port;
pub mod vibed;
pub mod watsyn;

use lm_plugin::Descriptor;

/// Every instrument of this crate, for the registry
pub static ALL: &[&Descriptor] = &[
    &audio_file_processor::DESCRIPTOR,
    &bit_invader::DESCRIPTOR,
    &kicker::DESCRIPTOR,
    &lb302::DESCRIPTOR,
    &monstro::DESCRIPTOR,
    &nes::DESCRIPTOR,
    &organic::DESCRIPTOR,
    &sfxr::DESCRIPTOR,
    &vibed::DESCRIPTOR,
    &watsyn::DESCRIPTOR,
];

/// The main stereo output every instrument here has
pub fn stereo_out(_: &lm_state::Value) -> lm_plugin::Ports {
    lm_plugin::Ports { outputs: vec![lm_plugin::Port::new("main", 2)], sidechains: Vec::new() }
}
