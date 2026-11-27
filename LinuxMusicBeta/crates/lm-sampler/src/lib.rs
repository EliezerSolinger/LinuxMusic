//! LinuxMusic's one sampler engine. Pure safe Rust, no threads of its own, deterministic, and no allocation on the
//! audio path.
//!
//! - [`model`]: the instrument as plain data: zones (key and velocity ranges, root key and tuning, loop modes, gain and
//!   pan, an AHDSR amplitude envelope, an optional filter with its envelope, exclusive groups, release triggers,
//!   round-robin and random selection) and the output groups of a drum kit;
//! - [`engine`]: the voice pool, voice stealing, the notes and the mix, with a windowed-sinc interpolator
//!   ([`interp`]);
//! - [`import`]: what reads a format into the model, away from the audio thread: SF2, SFZ, PAT, a single audio file
//!   (the AudioFileProcessor's start, end, loop, reverse and stutter), drum kits (the AVL kits, and the DrumMachines'
//!   `.ds` rendered by `lm_dsp::drumsynth`), GIG (a pure-Rust RIFF/DLS reader).
//!
//! The plugin (`lm-plugins-sampler`) reads the resources, calls an importer in its build and plays the engine.

#![forbid(unsafe_code)]

pub mod engine;
pub mod env;
pub mod filter;
pub mod import;
pub mod interp;
pub mod model;
pub mod pcm;
pub mod stream;

pub use engine::{Engine, EngineConfig, EnvOverride, MAX_POLYPHONY, Params};
pub use interp::Quality;
pub use model::{
    Ahdsr, Filter, FilterKind, Instrument, LoopMode, OutputGroup, PresetInfo, Sequence, Trigger, VelCurve, Window, Zone,
};
pub use pcm::{Pcm, SampleLoader, WavLoader, read_wav};
