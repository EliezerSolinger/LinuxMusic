//! From the audio side to the main thread (interfaces.md, I-8, T5, T9, H16).
//!
//! - **Continuous values** (meters, position) go in a [`TelemetryFrame`] published through a triple buffer at most
//!   ~60 times a second; the reader takes the newest.
//! - **Events** ([`Telemetry`]) go in a ring that counts what it drops (policy "count and report").
//! - **Ownership** (installs coming back, finished previews, panic payloads) and **what must not be lost**
//!   ([`TransportEvent`], [`Touched`]) go in their own rings, sized so they never fill in 10 s of a stopped pump
//!   (C4); the arithmetic is beside each size in `engine.rs`

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use lm_base::HostTime;

use crate::ids::{NodeIndex, ParamSlot, PreviewId, ProgramGen};
use crate::processor::{PluginRequest, Processor};
use crate::sample::SampleData;

/// Channels a meter keeps (a node with more shows its first ones)
pub const METER_CHANNELS: usize = 8;

/// One node's meter, by [`NodeIndex::index`]; `generation` tells which occupant of the slot it measured
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Meter {
    pub generation: u32,
    pub channels: u8,
    /// Peak absolute sample since the previous frame, per channel (after the node's gain, i.e. what it sends on)
    pub peak: [f32; METER_CHANNELS],
    /// The node panicked or produced NaN or infinity and is silenced until it is replaced (I-9.3, U20)
    pub dead: bool,
}

/// The newest continuous values. Allocated once by `engine()`; the renderer rewrites it in place
#[derive(Clone, Debug, PartialEq)]
pub struct TelemetryFrame {
    /// The program the meters belong to
    pub generation: ProgramGen,
    pub position: i64,
    pub tick: i64,
    pub host_time: HostTime,
    pub playing: bool,
    pub recording: bool,
    /// Blocks rendered since the engine started (the watchdog's heartbeat as well)
    pub periods: u64,
    /// Frames rendered since the engine started: the audio clock (tails and other deadlines count in it, never in
    /// wall time, so a faster or slower machine renders the same)
    pub frames: u64,
    /// Device blocks that arrived late (xruns) since the engine started
    pub xruns: u64,
    /// Render time over block time, of the slowest block since the previous frame (0 to 1 and beyond)
    pub load: f32,
    /// Whether subnormals are flushed on the audio thread (H6)
    pub flushing_denormals: bool,
    /// The master protection is cutting the output (U20)
    pub protection: bool,
    /// The count-in or the pre-roll is running (F-REC-026)
    pub counting_in: bool,
    /// The peak of each device input channel since the previous frame (linear): the input meters (F-REC-009)
    pub inputs: Box<[f32]>,
    pub meters: Box<[Meter]>,
    /// The current value of every automated parameter slot, for the interface to move the knob (arquitetura,
    /// piece 4); other slots are not written
    pub params: Box<[f32]>,
}

impl TelemetryFrame {
    pub(crate) fn new(max_nodes: usize, max_params: usize) -> Self {
        Self {
            generation: ProgramGen(0),
            position: 0,
            tick: 0,
            host_time: HostTime(0),
            playing: false,
            recording: false,
            periods: 0,
            frames: 0,
            xruns: 0,
            load: 0.0,
            flushing_denormals: false,
            protection: false,
            counting_in: false,
            inputs: vec![0.0; INPUT_METERS].into_boxed_slice(),
            meters: vec![Meter::default(); max_nodes].into_boxed_slice(),
            params: vec![0.0; max_params].into_boxed_slice(),
        }
    }
}

/// The device input channels whose peak is published each frame
pub const INPUT_METERS: usize = 64;

/// What kind of event was dropped
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TelemetryKind {
    /// This ring itself was full
    Telemetry,
    /// A node's event list was full for a block (notes or parameter changes were lost)
    NodeEvents,
    /// The active-notes table was full: a note-on was not played
    ActiveNotes,
    /// More live MIDI in one block than the renderer takes
    LiveMidi,
    /// A processor made more requests in one call than its outbox holds
    PluginRequests,
}

/// Events from the audio side; the ring counts what it drops ([`Telemetry::Dropped`])
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Telemetry {
    PluginRequest {
        generation: ProgramGen,
        node: NodeIndex,
        request: PluginRequest,
    },
    Xrun {
        late_by: u32,
    },
    NodePanicked {
        generation: ProgramGen,
        node: NodeIndex,
    },
    /// NaN or infinity in a node's output: the node is silenced and marked (U20)
    NodeNonFinite {
        generation: ProgramGen,
        node: NodeIndex,
    },
    /// A worker did not give a node's job back within the deadline: the audio thread stopped waiting, and the node
    /// is silent until it is replaced (R-8)
    NodeStuck {
        generation: ProgramGen,
        node: NodeIndex,
    },
    /// A streamed item (P-32) needed frames the disk had not loaded yet: they played as silence
    StreamUnderrun {
        generation: ProgramGen,
        node: NodeIndex,
        frames: u32,
    },
    /// The master went above +24 dBFS: the output is cut (U20)
    MasterProtection {
        generation: ProgramGen,
        peak: f32,
    },
    /// A tail node (U32) went quiet or reached 2 s; the compiler may remove it
    TailEnded {
        generation: ProgramGen,
        node: NodeIndex,
    },
    /// `Locate` while recording is refused (I-7.2)
    LocateRefused,
    /// A touched value could not be kept (its ring was full): automation recording of that slot stopped (C3)
    TouchedOverflow {
        slot: ParamSlot,
    },
    /// The MIDI map events ring was full: mapped fields and actions were lost (it is sized for 10 s of 16 busy
    /// ports, C4, so the session's pump stopped longer than that)
    MapEventsOverflow {
        count: u32,
    },
    /// The recorded MIDI ring was full: MIDI recording stopped (an explicit error, never a silent loss)
    MidiRecordOverflow,
    /// MIDI out to devices dropped (its ring full: the output thread fell behind)
    MidiOutDropped {
        count: u32,
    },
    /// The transport events ring was full: recording stopped (U1)
    TransportEventsOverflow,
    /// An install named a node outside the table or a producer outside the limits; that part was skipped
    InstallInvalid {
        generation: ProgramGen,
    },
    Dropped {
        kind: TelemetryKind,
        count: u32,
    },
}

/// What the recorder and the session must never miss (U1, U4, C2). Lossless ring
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportEvent {
    /// Recording started at this timeline sample (`frame` is the engine frame, for the capture side)
    RecordStarted {
        sample: i64,
        frame: u64,
    },
    RecordStopped {
        sample: i64,
        frame: u64,
    },
    /// The loop wrapped while recording: `sample_in_take` frames of the take had been recorded (U1)
    LoopWrapped {
        from: i64,
        to: i64,
        sample_in_take: u64,
    },
    /// Capture lost `frames` frames at this timeline sample (an xrun); the recorder writes exactly that much
    /// silence, marked, so the take stays on the grid (C2)
    CaptureGap {
        sample: i64,
        frames: u32,
    },
    /// A stretch of `frames` frames at this timeline sample could not be recorded (the block's span table was
    /// full): recording stopped right there, and a `RecordStopped` follows. The take ends at `sample` (R-7)
    RecordFailed {
        sample: i64,
        frame: u64,
        frames: u32,
    },
    /// The tempo map changed while playing and the playhead moved to keep the musical position (H9)
    Relocated {
        from: i64,
        to: i64,
    },
}

/// A touched slot's value while playing, for automation recording (F2, I-6.4, C3). Lossless ring
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touched {
    pub slot: ParamSlot,
    pub value: f32,
    pub sample: i64,
    /// `false`: the hand let go here (a release ends a Touch pass, I-21.5)
    pub touching: bool,
}

/// A finished preview gives back what it played (I-7.3, S7). Lossless ring
#[derive(Debug)]
pub struct PreviewEnded {
    pub id: PreviewId,
    pub returned: PreviewReturn,
}

/// What a preview gives back
pub enum PreviewReturn {
    Sample(Arc<SampleData>),
    Processor(Box<dyn Processor>),
}

impl fmt::Debug for PreviewReturn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreviewReturn::Sample(_) => f.write_str("Sample"),
            PreviewReturn::Processor(_) => f.write_str("Processor"),
        }
    }
}

/// A panic caught on a render thread (I-9.3, S11). The payload is not dropped on the audio thread: it comes here, and
/// the main thread prints it ([`PanicReport::message`])
pub struct PanicReport {
    pub generation: ProgramGen,
    /// The node, or `None` for a preview's processor
    pub node: Option<NodeIndex>,
    pub preview: Option<PreviewId>,
    pub payload: Box<dyn Any + Send>,
}

impl PanicReport {
    /// The panic's message, when it was a string
    pub fn message(&self) -> &str {
        if let Some(s) = self.payload.downcast_ref::<&'static str>() {
            s
        } else if let Some(s) = self.payload.downcast_ref::<String>() {
            s
        } else {
            "(non-string panic payload)"
        }
    }
}

impl fmt::Debug for PanicReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PanicReport")
            .field("generation", &self.generation)
            .field("node", &self.node)
            .field("preview", &self.preview)
            .field("message", &self.message())
            .finish()
    }
}
