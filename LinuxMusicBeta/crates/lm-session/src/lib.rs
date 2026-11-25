//! LinuxMusic: the session (interfaces.md, I-16; Parte 4, step 9).
#![forbid(unsafe_code)]

pub mod actions;
pub mod activity;
pub mod arrange;
mod crashnote;
pub mod export;
pub mod folder;
pub mod journal;
pub mod lock;
pub mod midifile;
pub mod notices;
mod plugin_actions;
mod port;
pub mod prefs;
pub mod rack;
mod session;
pub mod slice;
pub mod watchdog;

pub use port::LocalPort;
/// The MIDI device service the session drives (`lm_io::MidiDevices` on Linux; a fake in tests)
pub use session::midi::devices::MidiDeviceLink;
pub use session::*;
