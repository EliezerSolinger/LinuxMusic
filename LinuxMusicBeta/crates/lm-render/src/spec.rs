//! What the engine is made for: the stream format and the fixed capacities (interfaces.md, I-5.1)

use std::fmt;
use std::sync::Arc;

/// The stream: the project's sample rate and the largest block the device will ask for. `max_block` is the period
/// **negotiated** with the device (C7); the renderer accepts any block up to it and splits a larger one
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessSpec {
    pub sample_rate: u32,
    pub max_block: usize,
}

/// The fixed capacities of an engine. Everything the audio side will ever hold is allocated from these in `engine()`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Size of the node table (processors, strips, sums)
    pub max_nodes: usize,
    /// Number of parameter slots
    pub max_params: usize,
    /// Command rings the renderer can hold at once; producer 0 is the main thread's
    pub max_command_producers: usize,
    /// Preview voices on the preview bus (I-7.3)
    pub max_previews: usize,
    /// Installs on their way to the audio thread at once; more wait in a main-thread queue (I-5.3)
    pub installs_in_flight: usize,
    /// Worker threads besides the audio thread; 0 renders everything on the audio thread
    pub workers: usize,
    /// Notes sounding at once, from all ports and items (the active-notes table, U25)
    pub max_active_notes: usize,
    /// Live MIDI messages taken from the command rings in one block
    pub max_live_midi_per_block: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_nodes: 2048,
            max_params: 16384,
            max_command_producers: 32,
            max_previews: 8,
            installs_in_flight: 4,
            workers: 0,
            max_active_notes: 4096,
            max_live_midi_per_block: 1024,
        }
    }
}

/// What every render thread runs at its start. `body` is the thread's whole life: the hook sets the thread up (the
/// flush of subnormals, H6; the real-time membership, H1), calls `body`, and undoes what it did when `body` returns.
///
/// The renderer's workers call it when they start; the device thread that calls [`crate::Renderer::render`] belongs
/// to `lm-io`, which calls the same hook (via [`RenderThreads::run`]) around its callback loop
pub type ThreadHook = Arc<dyn Fn(ThreadKind, &mut dyn FnMut()) + Send + Sync>;

/// Which render thread a [`ThreadHook`] is setting up
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadKind {
    /// The device's thread, the one that calls `render`
    Audio,
    /// A worker of the renderer, by number
    Worker(usize),
}

/// The hooks of the render threads. The default runs the body as is.
///
/// The flush of subnormals (H6) is the renderer's own: every render call and every worker turn it on through
/// `lm-sys-fpu`, so no hook is needed for it. The session's hook joins the real-time group (H1) of each worker;
/// [`crate::TelemetryFrame::flushing_denormals`] reports, from the audio thread, that the flush is really on
#[derive(Clone, Default)]
pub struct RenderThreads {
    pub hook: Option<ThreadHook>,
}

impl RenderThreads {
    pub fn new(hook: ThreadHook) -> Self {
        Self { hook: Some(hook) }
    }

    /// Runs `body` inside the hook (or plainly, without one)
    pub fn run(&self, kind: ThreadKind, body: &mut dyn FnMut()) {
        match &self.hook {
            Some(hook) => hook(kind, body),
            None => body(),
        }
    }
}

impl fmt::Debug for RenderThreads {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderThreads").field("hook", &self.hook.is_some()).finish()
    }
}

/// Whether subnormal numbers are flushed on the calling thread, tested by arithmetic (no register access): the
/// product of the smallest normal number and one half is subnormal, so it is zero only when FTZ is on
pub fn flushing_denormals() -> bool {
    let tiny = std::hint::black_box(f32::MIN_POSITIVE);
    let half = std::hint::black_box(0.5f32);
    tiny * half == 0.0
}
