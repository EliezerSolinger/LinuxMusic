//! LinuxMusic: the note effects, plugins of `PluginKind::NoteEffect` (interfaces.md I-9; decisoes-autonomas.md,
//! "O que deixa de seguir o legado", item 2). They sit in a track's `noteEffects` list, before its instrument, and
//! turn the note stream (items and live input) into another one: what comes out of the last is what the instrument
//! plays and what MIDI out sends. They have no audio.
//!
//! - [`arpeggiator`]: the legacy arpeggio (F-IJN-015), its directions, range, repeats, cycle, skip and miss, time (free
//!   or synced to the tempo), gate, and the free, sort and sync modes;
//! - [`chords`]: the legacy chord stacking (F-IJN-013) with its 95-entry table ([`table`]) and its range;
//! - [`scale`]: snaps the notes to a scale and transposes them;
//! - [`velocity`]: fixed, scaled, curved or random (from a seed) velocity.
//!
//! Every numeric setting is a parameter of the schema (automatable and MIDI-mappable); the choices are fields.

#![forbid(unsafe_code)]

pub mod arpeggiator;
pub mod chords;
pub mod scale;
pub mod table;
pub mod velocity;

use lm_plugin::Descriptor;

/// Every note effect of the crate
pub static ALL: &[&Descriptor] =
    &[&arpeggiator::DESCRIPTOR, &chords::DESCRIPTOR, &scale::DESCRIPTOR, &velocity::DESCRIPTOR];

/// The ports of a note effect: none (no audio)
pub(crate) fn no_ports(_: &lm_state::Value) -> lm_plugin::Ports {
    lm_plugin::Ports { outputs: Vec::new(), sidechains: Vec::new() }
}
