//! LinuxMusic: the logic of MIDI mapping (interfaces.md, I-21.2 and I-21.3).
//!
//! - [`message`]: what a mapping listens to, and the raw channel messages;
//! - [`transform`]: range, curve, inversion, absolute and the three relative encodings, pickup and scale takeover,
//!   buttons;
//! - [`table`]: the compiled table (14-bit pairs, NRPN/RPN, touch sources, feedback), fed on the audio thread
//!   without allocating;
//! - [`feedback`]: values back to the controller, with the rate limit and the echo window;
//! - [`learn`]: the learn detector (7 and 14 bits, NRPN, relative encoders);
//! - [`mcu`]: the Mackie Control protocol and a simulated surface;
//! - [`doc`] (feature `doc`): the document form, `project.midiMap`.
//!
//! Pure: no threads, no I/O, no clock (times are passed in).
#![forbid(unsafe_code)]

#[cfg(feature = "doc")]
pub mod doc;
pub mod feedback;
pub mod learn;
pub mod mcu;
pub mod message;
pub mod table;
pub mod transform;

#[cfg(feature = "doc")]
pub use doc::{DocSource, MapScope, MapTarget, Mapping, NodeRef, SelRole};
pub use feedback::{Feedback, Limiter, Ring};
pub use learn::{Detector, Learned};
pub use message::{Message, Raw};
pub use table::{ANY_PORT, Entry, Event, MapTable, Source, Timing};
pub use transform::{Button, Encoding, Input, MapState, Mode, Output, Takeover, Transform};
