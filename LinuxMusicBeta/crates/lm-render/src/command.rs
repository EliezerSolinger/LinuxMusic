//! Commands to the audio side (interfaces.md, I-7.1, S17, D5): one ring per producer, a fixed number of producers.
//! The main thread has producer 0; every MIDI input gets its own ring, handed to the renderer inside an `Install`
//! and taken out by another, which returns the receiver (T9)

use std::fmt;
use std::sync::Arc;

use lm_base::HostTime;
use lm_base::ring::{Consumer, Producer, PushError, ring};

use crate::ids::{PortIndex, PreviewId, ProducerIndex};
use crate::processor::Processor;
use crate::sample::SampleData;

/// A command. Closed (rule 10); no sysex in F1 (T14)
pub enum Command {
    Play,
    Stop,
    /// Refused while recording (I-7.2), with [`crate::Telemetry::LocateRefused`]
    Locate { sample: i64 },
    /// Arms (or disarms) recording; it records while playing, inside the punch range if there is one
    Record { on: bool },
    /// Every note off and every processor reset
    Panic,
    /// A MIDI message stamped with the host clock (C6); routed by the program's port table
    Midi { port: PortIndex, at: HostTime, bytes: [u8; 3] },
    /// F2 (I-7.3)
    Preview(PreviewCommand),
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Play => f.write_str("Play"),
            Command::Stop => f.write_str("Stop"),
            Command::Locate { sample } => write!(f, "Locate({sample})"),
            Command::Record { on } => write!(f, "Record({on})"),
            Command::Panic => f.write_str("Panic"),
            Command::Midi { port, at, bytes } => write!(f, "Midi({port:?}, {at}, {bytes:02x?})"),
            Command::Preview(p) => write!(f, "Preview({p:?})"),
        }
    }
}

/// The preview bus (I-7.3). It does not go through the compiler: the command brings the data or the built processor,
/// and everything comes back in a [`crate::PreviewEnded`]
pub enum PreviewCommand {
    Sample { id: PreviewId, data: Arc<SampleData>, gain: f32 },
    /// A built instrument plays `key` until `NoteOff` (built before, in an `Operation`). It must declare stereo or
    /// mono output; it gets two output channels
    Preset { id: PreviewId, processor: Box<dyn Processor>, key: u8 },
    NoteOff { id: PreviewId },
    Stop { id: PreviewId },
}

impl fmt::Debug for PreviewCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreviewCommand::Sample { id, gain, .. } => write!(f, "Sample({id:?}, gain {gain})"),
            PreviewCommand::Preset { id, key, .. } => write!(f, "Preset({id:?}, key {key})"),
            PreviewCommand::NoteOff { id } => write!(f, "NoteOff({id:?})"),
            PreviewCommand::Stop { id } => write!(f, "Stop({id:?})"),
        }
    }
}

/// The sending end of one producer's ring. Sending never blocks; a full ring hands the command back, and what to do
/// then is the producer's written policy (the main thread reports it; a MIDI input counts it as lost)
#[derive(Debug)]
pub struct CommandSender {
    producer: ProducerIndex,
    tx: Producer<Command>,
}

/// The receiving end, owned by the renderer
#[derive(Debug)]
pub struct CommandReceiver {
    producer: ProducerIndex,
    pub(crate) rx: Consumer<Command>,
}

/// The capacity of a command ring (C4). The renderer drains every ring at every block, and keeps a MIDI message in
/// the ring only while its time is still ahead (at most one block, R14), so the worst case is one producer's
/// traffic during one block plus the jitter. The default of 1024: 16 MIDI ports at 1000 events per second are 16
/// events per millisecond, and a 8192-sample block at 44.1 kHz is 186 ms, so one port sees at most 186 events per
/// block and the main thread a handful (transport, previews). The main thread's ring is never drained by the pump,
/// so a stopped pump (H2) cannot fill it
pub const COMMAND_RING_CAPACITY: usize = 1024;

/// A ring for a producer
pub fn command_ring(producer: ProducerIndex, capacity: usize) -> (CommandSender, CommandReceiver) {
    let (tx, rx) = ring(capacity);
    (CommandSender { producer, tx }, CommandReceiver { producer, rx })
}

impl CommandSender {
    pub fn producer(&self) -> ProducerIndex {
        self.producer
    }

    /// Sends a command, or hands it back if the ring is full
    pub fn send(&mut self, command: Command) -> Result<(), Command> {
        self.tx.push(command).map_err(|PushError::Full(c)| c)
    }
}

impl CommandReceiver {
    pub fn producer(&self) -> ProducerIndex {
        self.producer
    }
}
