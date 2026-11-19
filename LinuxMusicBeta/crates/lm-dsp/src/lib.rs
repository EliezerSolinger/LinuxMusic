//! LinuxMusic's DSP: the algorithms alone, with no knowledge of the project, the threads or the interface
//! (interfaces.md, Parte 3, and the user's decision of 19/11 that the analog desk's rack is ported).
//!
//! The rules every algorithm here follows:
//! - **preparing apart from processing:** `new()` (or `prepare`) allocates and works out what is costly; processing
//!   never allocates, locks or waits;
//! - **deterministic:** the math is the pure `libm` crate, never the system's (V2; `clippy.toml` forbids the
//!   standard float functions), and phases and time counters are `f64` or integers (U35);
//! - **no `unsafe`** (the workspace forbids it);
//! - **ported from the legacy program with an A/B test** against it (U34: see `tests/ab.rs` and `ab/`).
//!
//! The analog desk's rack, from the legacy program:
//! - [`filters`]: biquads matched to their analog prototypes, and the desk's 4-band EQ;
//! - [`tolerance`]: each channel's component tolerances, and the output transformer;
//! - [`tube`], [`gain_stage`]: the op-amps and the tubes of the gain stage, oversampled ([`oversampling`]);
//! - [`dynamics`]: the compressors of nine units, and the gate;
//! - [`mono_bass`], [`channel_strip`]: the channel's whole strip;
//! - [`master`]: the master section.
//!
//! The instruments' sources and shaping:
//! - [`basic_filters`]: the general filters (every instrument's filter, the DualFilter's);
//! - [`envelope`], [`sound_shaping`]: every instrument's envelopes and LFOs, and its filter;
//! - [`oscillator`]: the legacy program's oscillator (the TripleOscillator's), with its band-limited tables;
//! - [`sample_player`]: playing a sample (the AudioFileProcessor's and the sample clips').
//!
//! The effects of the validation plugins:
//! - [`compressor`]: the Compressor;
//! - [`delay`]: the Delay;
//! - [`distortion_effects`]: the Dispersion, the Wave Shaper and the Bitcrush;
//! - [`dynamics_processor`]: the Dynamics Processor;
//! - [`eq`]: the Eq;
//! - [`filter_effects`]: the Dual Filter and the Crossover EQ;
//! - [`modulation_effects`]: the Multitap Echo and the Flanger;
//! - [`small_effects`]: the Amplifier, the Stereo Matrix, the Stereo Enhancer and the Bass Booster;
//! - [`reverb_sc`]: the ReverbSC (Csound's reverbsc).

// The ports keep the legacy program's index loops and its literal constants, so that a reader can set the two side by
// side line by line (the A/B is against that code)
#![allow(clippy::needless_range_loop, clippy::excessive_precision)]

pub mod basic_filters;
pub mod channel_strip;
pub mod compressor;
pub mod delay;
pub mod distortion_effects;
pub mod dynamics;
pub mod dynamics_processor;
pub mod envelope;
pub mod eq;
pub mod filter_effects;
pub mod filters;
pub mod gain_stage;
pub mod master;
pub mod math;
pub mod modulation_effects;
pub mod mono_bass;
pub mod oscillator;
pub mod oversampling;
pub mod rbj;
pub mod reverb_sc;
pub mod sample_player;
pub mod small_effects;
pub mod sound_shaping;
pub mod tolerance;
pub mod tube;
