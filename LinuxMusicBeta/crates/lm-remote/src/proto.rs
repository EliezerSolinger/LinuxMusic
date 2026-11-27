//! The protocol's versions, modes and roles (I-20, I-20.6).
//!
//! Two publics: the **graphical interface** (mode `"gui"`: the whole `CorePort`, a mirror of the tree; client and
//! server must be the very same program version) and **external tools** (mode `"tool"`: a stable, versioned subset —
//! reading by id or path, edits, actions, undo and redo, telemetry, `health` — compatible across program versions
//! within the same `TOOL_API`).

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

use lm_base::{Error, Result};

/// The wire format's version (frames, message names): both ends must have it
pub const PROTOCOL: i64 = 1;
/// The program's version: the graphical interface requires the same on both ends (I-20)
pub const PROGRAM: &str = env!("CARGO_PKG_VERSION");
/// The stable subset's versions this server speaks
pub const TOOL_APIS: &[i64] = &[1];

/// What a client may do (I-20.6), given at the handshake (by the token over TCP; the Unix socket's own user, who
/// alone can open it, is `Admin`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Read, subscribe, telemetry
    Observe,
    /// And transactions, actions, gestures, transport
    Control,
    /// And projects, devices, preferences, quitting
    Admin,
}

impl Role {
    pub fn key(self) -> &'static str {
        match self {
            Role::Observe => "observe",
            Role::Control => "control",
            Role::Admin => "admin",
        }
    }

    pub fn parse(t: &str) -> Option<Role> {
        match t {
            "observe" => Some(Role::Observe),
            "control" => Some(Role::Control),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// The actions that manage the session rather than the music: they need `Admin`
pub fn admin_action(action: &str) -> bool {
    // `browser.` opens a path with the server's desktop: it has no meaning for a remote control
    ["project.", "session.", "app.", "device.", "preferences.", "browser."].iter().any(|p| action.starts_with(p))
}

/// Which client is connecting
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Gui,
    Tool,
}

pub(crate) fn mismatch(server: &str, client: &str, protocol_server: i64, protocol_client: i64) -> Error {
    Error::new("remote.version_mismatch")
        .arg("server", server)
        .arg("client", client)
        .arg("server_protocol", protocol_server)
        .arg("client_protocol", protocol_client)
}

/// A connection, over TCP or a Unix socket
#[derive(Debug)]
pub(crate) enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Stream {
    pub(crate) fn try_clone(&self) -> Result<Stream> {
        let e = |e: std::io::Error| Error::new("remote.io").arg("error", e.to_string());
        Ok(match self {
            Stream::Tcp(s) => Stream::Tcp(s.try_clone().map_err(e)?),
            #[cfg(unix)]
            Stream::Unix(s) => Stream::Unix(s.try_clone().map_err(e)?),
        })
    }

    pub(crate) fn shutdown(&self) {
        let _ = match self {
            Stream::Tcp(s) => s.shutdown(Shutdown::Both),
            #[cfg(unix)]
            Stream::Unix(s) => s.shutdown(Shutdown::Both),
        };
    }

    pub(crate) fn set_read_timeout(&self, t: Option<std::time::Duration>) {
        let _ = match self {
            Stream::Tcp(s) => s.set_read_timeout(t),
            #[cfg(unix)]
            Stream::Unix(s) => s.set_read_timeout(t),
        };
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            Stream::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Stream::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Stream::Unix(s) => s.flush(),
        }
    }
}
