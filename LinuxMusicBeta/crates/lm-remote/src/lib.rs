//! LinuxMusic: the network protocol (interfaces.md, I-20; I-15).
//!
//! - [`Server`]: serves a `CorePort` (the session's `LocalPort`, in `lm-headless serve`): network threads that never
//!   touch the session, and a [`Server::pump`] on the main thread that turns each request into port calls and sends
//!   the mirrors the tree's differences and the core's diffs;
//! - [`RemotePort`]: the interface's `CorePort` over the network, with a mirror of the tree (its feeds give the
//!   core's diffs), `Pending` results collected by `finished`, gestures and parameter writes that never wait, and
//!   reconnection with a resync;
//! - the wire: length-prefixed frames, JSON messages (the protocol's own JSON, `lm-state`), binary for the dense
//!   data (telemetry, peaks, displays); a handshake with the protocol's and the program's versions; roles.
//!
//! Transport (decisoes-autonomas.md): a Unix socket (0600) on the machine, TCP bound to 127.0.0.1 reached through
//! an SSH tunnel from another one; TCP clients pair by token.

#![forbid(unsafe_code)]

mod client;
pub mod codec;
pub mod delta;
mod files;
mod proto;
mod server;
pub mod wire;

pub use client::{ClientConfig, Connecting, Connection, Endpoint, RemotePort, Traffic};
pub use proto::{Mode, PROGRAM, PROTOCOL, Role, TOOL_APIS, admin_action};
pub use server::{ClientLimits, Server, ServerConfig};
