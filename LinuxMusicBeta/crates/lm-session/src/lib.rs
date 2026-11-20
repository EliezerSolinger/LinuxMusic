//! LinuxMusic: the session (interfaces.md, I-16; Parte 4, step 9).
#![forbid(unsafe_code)]

pub mod actions;
pub mod activity;
mod crashnote;
pub mod export;
pub mod folder;
pub mod journal;
pub mod lock;
pub mod notices;
mod port;
pub mod prefs;
pub mod rack;
mod session;
pub mod slice;
pub mod watchdog;

pub use port::LocalPort;
pub use session::*;
