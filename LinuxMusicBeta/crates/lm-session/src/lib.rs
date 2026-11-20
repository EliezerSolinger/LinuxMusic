//! LinuxMusic: the session (interfaces.md, I-16; Parte 4, step 9).
#![forbid(unsafe_code)]

pub mod activity;
pub mod folder;
pub mod journal;
pub mod lock;
pub mod notices;
pub mod prefs;
pub mod rack;
mod session;

pub use session::*;
