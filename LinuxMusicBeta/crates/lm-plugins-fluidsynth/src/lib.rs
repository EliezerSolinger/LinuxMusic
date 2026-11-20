//! The instruments played by FluidSynth (through `lm-sys-fluidsynth`): [`avldrums`] (F-INS-002) and the Sf2 Player
//! ([`sf2player`], F-INS-014). One crate for both: they share the synthesizer's binding, the way they reach rates
//! above FluidSynth's 96 kHz ([`half_rate`]) and their A/B bench against the legacy plugins.

#![forbid(unsafe_code)]

pub mod avldrums;
pub mod half_rate;
pub mod sf2player;
