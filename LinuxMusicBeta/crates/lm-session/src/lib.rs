//! LinuxMusic: the session (interfaces.md, I-16; Parte 4, step 9).
#![forbid(unsafe_code)]

pub mod actions;
pub mod activity;
pub mod arrange;
pub mod browser;
pub mod clips;
mod crashnote;
pub mod crashreport;
pub mod export;
mod field_load;
pub mod folder;
pub mod index;
pub mod journal;
pub mod lock;
pub mod midifile;
mod modulators;
pub mod notices;
mod plugin_actions;
mod port;
pub mod prefs;
pub mod preview;
pub mod rack;
mod recording;
mod resample;
mod sampler_presets;
mod session;
pub mod slice;
mod tuning_actions;
pub mod watchdog;

pub use port::LocalPort;
/// The MIDI device service the session drives (`lm_io::MidiDevices` on Linux; a fake in tests)
pub use session::midi::devices::MidiDeviceLink;
pub use session::*;
