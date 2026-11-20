//! The MIDI devices of the session (interfaces.md, I-21.1, I-10.2, C6; decisoes-autonomas.md P-15, P-16, P-36, P-38).
//!
//! - **A port has a stable identity:** client name + port name ([`PortId`]), never the ALSA numbers, which change at
//!   every connection. The machine's preferences give each port a [`PortSettings`]: input and output on or off, the
//!   role (instrument, controller or both), the input and output latency, an alias; the session gives each alias a
//!   [`PortIndex`] that stays the same for the whole session, through disconnections.
//! - **One sequencer client, "LinuxMusicBeta"**, with the program's own ports "LinuxMusicBeta entrada" (others connect
//!   to it) and "LinuxMusicBeta saída". Every enabled input is connected to "entrada"; every event is stamped by the
//!   queue's real time, converted to `CLOCK_MONOTONIC` (C6), minus the port's input latency, and goes into **one**
//!   command ring as `Command::Midi { port, at, bytes }` (sized for 2 s of 16 ports at 1000 events per second, C4; a
//!   full ring counts the loss, reported as [`MidiDeviceEvent::Lost`]).
//! - **Hotplug:** the sequencer's announcements trigger a new listing; a configured port that appears is connected
//!   ([`MidiDeviceEvent::Connected`]), one that goes away is reported ([`MidiDeviceEvent::Disconnected`]) and
//!   reconnected by itself when the same identity comes back.
//! - **Activity:** every input message, mapped or not, also goes to a lossy ring ([`MidiActivity`]) for "learn" and the
//!   activity monitor (I-21.2).
//! - **Raw MIDI** (P-16): a configured raw port gets its own reader thread and its own command ring, handed to the
//!   session as [`MidiDeviceEvent::ProducerReady`] (the renderer's rings are single-producer).

use std::fmt;

use lm_base::{Error, HostTime};
use lm_render::{PortIndex, ProducerIndex};

/// The stable identity of a port (I-21.1): never the ALSA client or port number. A raw MIDI port is
/// `{ client: <card id>, port: "MIDI <device>" }`
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortId {
    pub client: String,
    pub port: String,
}

impl PortId {
    pub fn new(client: &str, port: &str) -> Self {
        Self { client: client.to_owned(), port: port.to_owned() }
    }
}

impl fmt::Display for PortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.client, self.port)
    }
}

/// Which backend a port belongs to
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MidiBackendKind {
    Seq,
    Raw,
}

/// What a port is for (I-21.1): the compiler's port table routes instrument ports to the armed tracks and controller
/// ports to the mappings
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PortRole {
    #[default]
    Instrument,
    Controller,
    Both,
}

/// One port as the machine's preferences describe it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSettings {
    /// Given by the session for the alias; stays the same through disconnections
    pub index: PortIndex,
    pub id: PortId,
    pub backend: MidiBackendKind,
    pub alias: String,
    pub role: PortRole,
    pub input: bool,
    pub output: bool,
    /// Subtracted from every input stamp (P-15), nanoseconds
    pub input_latency_ns: i64,
    /// Every output event is sent this much earlier, nanoseconds
    pub output_latency_ns: i64,
}

/// A port found on the machine
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub id: PortId,
    pub backend: MidiBackendKind,
    /// It sends (we can read it)
    pub readable: bool,
    /// It receives (we can write to it)
    pub writable: bool,
    pub hardware: bool,
}

/// What the MIDI devices tell the session (lossless)
#[derive(Debug)]
pub enum MidiDeviceEvent {
    /// A port appeared on the machine
    Appeared(PortInfo),
    /// A port went away
    Gone(PortId),
    /// A configured port is connected (at the start, or back after a disconnection: the I-14 notice)
    Connected {
        index: PortIndex,
        id: PortId,
    },
    /// A configured port went away: "desconectado" in the interface; it reconnects by itself
    Disconnected {
        index: PortIndex,
        id: PortId,
    },
    /// Messages lost because a ring was full (`index`: the raw port, or `None` for the sequencer's ring)
    Lost {
        index: Option<PortIndex>,
        count: u64,
    },
    /// Output events dropped (their port is not connected, or the output fell behind)
    OutputDropped {
        index: Option<PortIndex>,
        count: u64,
    },
    /// A raw port's reader is running: install its ring (`Install::add_producer`)
    ProducerReady(lm_render::CommandReceiver),
    /// A raw port's reader ended: remove its ring (`Install::remove_producer`)
    ProducerDone(ProducerIndex),
    Failed {
        index: Option<PortIndex>,
        error: Error,
    },
}

/// One input message, for "learn" and the activity monitor (lossy ring, I-21.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiActivity {
    pub port: PortIndex,
    /// `CLOCK_MONOTONIC`, minus the port's input latency
    pub at: HostTime,
    pub len: u8,
    pub bytes: [u8; 3],
}

/// The length of a short message from its status (running status never reaches here)
pub fn message_len(status: u8) -> u8 {
    match status & 0xf0 {
        0x80 | 0x90 | 0xa0 | 0xb0 | 0xe0 => 3,
        0xc0 | 0xd0 => 2,
        _ => match status {
            0xf1 | 0xf3 => 2,
            0xf2 => 3,
            _ => 1,
        },
    }
}

/// Input ring capacity (C4): 16 ports at 1000 events per second for 2 s (the renderer drains it every block)
pub const INPUT_RING: usize = 32_768;
/// Activity ring capacity: 2 s of 16 busy ports; lossy, counted
pub const ACTIVITY_RING: usize = 32_768;

/// The client name of the program's own sequencer client
pub const CLIENT_NAME: &str = "LinuxMusicBeta";
/// The program's own input port: others connect to it (I-21.1)
pub const OWN_INPUT: &str = "LinuxMusicBeta entrada";
/// The program's own output port: others subscribe to it
pub const OWN_OUTPUT: &str = "LinuxMusicBeta saída";

/// A SysEx message to send (I-21.3: the controller profiles, the MCU display): `data[..len]` is F0 ... F7
#[derive(Clone, Copy)]
pub struct SysexOut {
    pub port: PortIndex,
    pub at: HostTime,
    pub len: u16,
    pub data: [u8; SYSEX_OUT_MAX],
}

impl fmt::Debug for SysexOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SysexOut").field("port", &self.port).field("at", &self.at).field("len", &self.len).finish()
    }
}

/// The longest SysEx the session sends (an MCU display line is 7 + 56 + 1 bytes)
pub const SYSEX_OUT_MAX: usize = 512;
/// The longest SysEx received (longer ones are counted and dropped by the binding)
pub const SYSEX_IN_MAX: usize = 4096;

/// A SysEx message received: to the session only (controller profiles, the MCU handshake); never recorded (I-21.1)
#[derive(Clone)]
pub struct SysexIn {
    pub port: PortIndex,
    pub at: HostTime,
    pub data: Vec<u8>,
}

impl fmt::Debug for SysexIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SysexIn")
            .field("port", &self.port)
            .field("at", &self.at)
            .field("len", &self.data.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_lengths_follow_the_status() {
        assert_eq!(message_len(0x90), 3);
        assert_eq!(message_len(0xb5), 3);
        assert_eq!(message_len(0xc0), 2);
        assert_eq!(message_len(0xd3), 2);
        assert_eq!(message_len(0xe0), 3);
        assert_eq!(message_len(0xf2), 3);
        assert_eq!(message_len(0xf3), 2);
        assert_eq!(message_len(0xf8), 1);
    }

    #[test]
    fn identity_is_by_names() {
        let a = PortId::new("Launchkey", "MIDI 1");
        assert_eq!(a, PortId::new("Launchkey", "MIDI 1"));
        assert_ne!(a, PortId::new("Launchkey", "MIDI 2"));
        assert_eq!(a.to_string(), "Launchkey: MIDI 1");
    }
}
