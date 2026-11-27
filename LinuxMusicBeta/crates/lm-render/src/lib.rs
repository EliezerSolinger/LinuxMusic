//! LinuxMusic: the real-time audio engine (interfaces.md, I-5 to I-9; arquitetura-nova.md, pieces 3 to 6).
//!
//! Two halves, made together by [`engine()`]:
//! - [`EngineControl`], on the main thread: sends [`Install`]s (never blocking), collects what comes back, writes
//!   [`ParamSlots`], sends [`Command`]s, reads the [`TelemetryFrame`] and the event rings;
//! - [`Renderer`], owned by the device callback: applies installs in order, runs the [`Program`] over its worker
//!   threads, sums in the program's fixed order, publishes telemetry.
//!
//! Only four things cross between threads (Parte 2, rule 3): install and return messages, parameter slots,
//! single-producer single-consumer rings of fixed size (and the telemetry frame), and immutable shared data. The
//! audio side never allocates nor frees (rule 5) and never sees the document (rule 6): this crate does not depend on
//! `lm-state`.

#![forbid(unsafe_code)]

mod automation;
mod command;
pub mod dummy;
mod engine;
mod event;
mod exec;
mod handoff;
mod ids;
mod install;
pub mod midimap;
pub mod modulation;
pub mod mpe;
mod notes;
mod panic;
mod params;
mod processor;
mod program;
mod renderer;
mod rng;
mod sample;
mod spec;
mod stream;
mod sync;
mod telemetry;
mod voice;

pub use automation::{CURVE_STEEPNESS, Curve, CurvePoint, Segment, shape};
pub use command::{COMMAND_RING_CAPACITY, Command, CommandReceiver, CommandSender, PreviewCommand, command_ring};
pub use dummy::Dummy;
pub use engine::{EngineControl, engine, engine_with};
pub use event::{Event, EventKind, Expression, NoteOut};
pub use exec::Watch;
pub use ids::{GUI_PORT_BASE, NodeIndex, NoteId, ParamSlot, PortIndex, PreviewId, ProducerIndex, ProgramGen, gui_port};
pub use install::{Install, NodeCore, Relocate, Retired, Returned};
pub use midimap::{MapEvent, MapTarget, MidiMap, MidiOutEvent, ParamRange, RangeScale, RecordedMidi};
pub use panic::{install_panic_hook, is_render_thread, note_caught_panic, take_render_panics};
pub use params::ParamSlots;
pub use processor::{
    Adopted, BlockShape, Buffers, Channels, ChannelsMut, DisplayReader, DisplayWriter, HostOutbox, NoteContext,
    PluginErrorKey, PluginRequest, ProcessContext, Processor, TransportInfo, display_channel,
};
pub use program::{
    AUTOMATION_GRID, AudioItem, AudioSource, AutomationMode, ChannelMix, CountIn, Feed, HostParam, ItemEvent,
    ItemEventKind, LiveKeys, Metronome, MidiItem, MidiOutRoute, MidiTiming, NodeDesc, NodeKind, OutputRoute, ParamInfo,
    PortRoute, Program, ProgramBuilder, Rate, Strip, TransportSettings,
};
pub use renderer::{BlockReport, DeviceBlock, PROTECTION_THRESHOLD, RecordSpan, Renderer, TAIL_SECONDS};
pub use rng::Rng;
pub use sample::SampleData;
pub use spec::{Limits, ProcessSpec, RenderThreads, ThreadHook, ThreadKind, flushing_denormals};
pub use stream::{Stream, StreamFeeder, StreamSpec, stream};
pub use telemetry::{
    INPUT_METERS, METER_CHANNELS, Meter, PanicReport, PreviewEnded, PreviewReturn, Telemetry, TelemetryFrame,
    TelemetryKind, Touched, TransportEvent,
};
pub use voice::{Policy, Voice, VoiceGain, VoiceNote, VoicePool};
