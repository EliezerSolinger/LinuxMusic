//! The base every LinuxMusic crate builds on (interfaces.md, I-1 and I-2):
//!
//! - [`Main`]: the proof of being on the main thread, and [`Role`]: what each thread is for;
//! - [`Error`], [`Label`], [`Result`]: errors are values, with translatable keys, never ready-made text;
//! - [`Cancel`], [`Progress`], [`Phase`], [`OperationHandle`], [`Workers`]: long operations off the main thread;
//! - [`ring`]: the fixed-size single-producer single-consumer rings that cross threads;
//! - [`HostTime`], [`Clock`]: the monotonic host clock, injectable in tests.
//!
//! Nothing here allocates on its own on an audio path: the rings are allocated when made, and the audio thread only
//! pushes and pops.

mod error;
mod main_thread;
mod operation;
pub mod ring;
mod sync;
mod time;

pub use error::{Arg, Error, Label, Result};
pub use main_thread::{role, set_role, Main, Role};
pub use operation::{Cancel, OperationHandle, OperationId, Phase, Progress, Workers};
pub use time::{Clock, HostTime, ManualClock, SystemClock};
