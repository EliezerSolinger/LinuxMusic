//! What every audio backend implements (interfaces.md, I-10.1, S16): open a device with an explicit configuration
//! (U7), run a thread that calls the [`CallbackHandle`] once per period, and stop when asked. The backends (Dummy,
//! ALSA; later JACK and CoreAudio) are written in safe Rust over the `lm-sys-*` bindings; the barrier, the renderer
//! and the recorder are the [`crate::Device`]'s, not the backend's.

use std::any::Any;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;

use lm_base::ring::{Consumer, Producer, ring};
use lm_base::{Error, Result};
use lm_render::{DeviceBlock, ProcessSpec, RenderThreads};

/// The device the user chose, as data (U7: input and output are explicit; nothing is ever joined behind the user's
/// back)
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceConfig {
    /// The playback device (`hw:CARD=PCH,DEV=0` for ALSA), or `None` for capture only
    pub playback: Option<String>,
    /// The capture device, or `None` for playback only. In F1 the same `hw` card as the playback, so the two streams
    /// are linked on one clock (U7)
    pub capture: Option<String>,
    /// The project's sample rate. The device is opened at this rate; if it cannot do it, the edge resampler converts
    /// (only when `allow_edge_resampler`)
    pub rate: u32,
    /// Frames per period wished; the device answers with what it can do (C7)
    pub period: u32,
    /// Periods in the device buffer (2 or 3)
    pub periods: u32,
    /// Device channels to open; `None`: all the device has
    pub playback_channels: Option<u32>,
    pub capture_channels: Option<u32>,
    /// Convert at the device edge when the device cannot run at the project's rate (arquitetura-nova.md, item 8)
    pub allow_edge_resampler: bool,
    /// Capture and playback on different cards (different clocks), with an adaptive resampler on the input (U7). Off
    /// by default: two cards are refused without it
    pub drift_correction: bool,
}

impl DeviceConfig {
    /// One duplex device: the F1 configuration (U7)
    pub fn duplex(device: &str, rate: u32, period: u32) -> Self {
        Self {
            playback: Some(device.to_owned()),
            capture: Some(device.to_owned()),
            rate,
            period,
            periods: 2,
            playback_channels: None,
            capture_channels: None,
            allow_edge_resampler: true,
            drift_correction: false,
        }
    }

    /// A name for logs and calibration keys: the playback and capture names
    pub fn name(&self) -> String {
        match (&self.playback, &self.capture) {
            (Some(p), Some(c)) if p == c => p.clone(),
            (Some(p), Some(c)) => format!("{p} + {c}"),
            (Some(p), None) => p.clone(),
            (None, Some(c)) => c.clone(),
            (None, None) => String::new(),
        }
    }
}

/// What the device agreed to (C7: the period is the negotiated one)
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    /// The project's rate: what the renderer runs at
    pub rate: u32,
    /// The device's rate (differs from `rate` only with the edge resampler)
    pub device_rate: u32,
    /// The negotiated period, in device frames
    pub period: u32,
    pub periods: u32,
    pub buffer: u32,
    /// The largest block the renderer will be asked for, in project frames: the engine's `ProcessSpec::max_block`
    pub max_block: usize,
    /// Device channels
    pub inputs: usize,
    pub outputs: usize,
    /// Latency the device reports, in project frames: capture (from the converter to the program) and playback
    /// (from the program to the converter), the edge resampler included
    pub input_latency: u32,
    pub output_latency: u32,
    /// The edge resampler is on (the calibration is kept per rate and per this flag, H19)
    pub edge_resampler: bool,
    /// Capture and playback are on different cards, with the adaptive resampler (U7)
    pub drift_correction: bool,
    /// Capture and playback are linked on one clock (`snd_pcm_link`, U7)
    pub linked: bool,
}

impl DeviceInfo {
    /// The engine's stream for this device (C7)
    pub fn spec(&self) -> ProcessSpec {
        ProcessSpec { sample_rate: self.rate, max_block: self.max_block }
    }

    /// The round-trip latency the device reports, in project frames
    pub fn reported_round_trip(&self) -> u32 {
        self.input_latency + self.output_latency
    }
}

/// What the device thread tells the main thread. A lossy ring that counts what it drops ([`DeviceEvent::Dropped`])
#[derive(Debug)]
pub enum DeviceEvent {
    /// The thread is running; `realtime` is the result of joining the real-time group (H1). Sent before the loop
    Started { realtime: Option<Error> },
    /// A playback underrun (`late_by` frames) or a capture overrun (`capture_lost` frames, filled with silence in the
    /// take, C2)
    Xrun { capture_lost: u32, late_by: u32 },
    /// The machine was suspended and the stream resumed
    Resumed,
    /// The card went away (U11). The thread has left: stop the device (the barrier passes at once), switch the
    /// renderer to the Dummy and offer to reconnect
    Disconnected,
    /// The thread stopped on an error it cannot recover from (the errno ALSA gave)
    Failed { errno: i32 },
    /// The thread panicked outside the renderer (a bug): it has left, like `Failed`
    Panicked,
    /// Events lost because the ring was full
    Dropped { count: u64 },
}

/// The events ring: an event per xrun at most, so 256 slots are seconds of continuous xruns; the loss is counted
pub(crate) const EVENT_RING: usize = 256;

/// The barrier's shared half (R5, S16, D11): the device thread marks every entry into and exit from the callback,
/// so the main thread can tell a stuck callback from a finished one, and the watchdog can see a stalled period
#[derive(Debug, Default)]
pub struct CallbackGuard {
    inside: AtomicBool,
    calls: AtomicU64,
    /// The thread was asked to leave
    stopping: AtomicBool,
}

impl CallbackGuard {
    /// True while a callback is running
    pub fn is_inside(&self) -> bool {
        self.inside.load(Ordering::Acquire)
    }

    /// Callbacks completed so far
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Acquire)
    }

    /// True once the device was asked to stop: the backend's loop checks it every period
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    pub(crate) fn request_stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }
}

/// What the device thread runs every period. The renderer's driver is the usual one; the calibration has its own
pub trait DeviceCallback: Send {
    /// One period: read `block.inputs`, write every `block.outputs`
    fn process(&mut self, block: DeviceBlock<'_, '_>);
    /// Called once, on the device thread, when it leaves (a stop, a removed card, a failure): close what must be
    /// closed (the recorder ends its takes here, U11)
    fn finish(&mut self) {}
    /// For the device to take its renderer back
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;
}

/// The only way a backend calls the render (I-10.1): it owns the callback while the device runs, counts every entry
/// and exit in the [`CallbackGuard`], and hands the callback back to the [`crate::Device`] when it is dropped (the
/// device thread leaving for any reason, a panic included). So "the callback came back" is the barrier: after it,
/// no callback can run again
pub struct CallbackHandle {
    callback: Option<Box<dyn DeviceCallback>>,
    guard: Arc<CallbackGuard>,
    back: Option<SyncSender<Box<dyn DeviceCallback>>>,
    events: Producer<DeviceEvent>,
    dropped: u64,
    threads: RenderThreads,
}

impl fmt::Debug for CallbackHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallbackHandle").field("guard", &self.guard).finish()
    }
}

impl CallbackHandle {
    pub(crate) fn new(
        callback: Box<dyn DeviceCallback>,
        guard: Arc<CallbackGuard>,
        back: SyncSender<Box<dyn DeviceCallback>>,
        threads: RenderThreads,
    ) -> (Self, Consumer<DeviceEvent>) {
        let (events, rx) = ring(EVENT_RING);
        (Self { callback: Some(callback), guard, back: Some(back), events, dropped: 0, threads }, rx)
    }

    /// A handle for a backend's own tests: the callback comes back through `back`
    pub fn for_test(
        callback: Box<dyn DeviceCallback>,
        back: SyncSender<Box<dyn DeviceCallback>>,
    ) -> (Self, Consumer<DeviceEvent>, Arc<CallbackGuard>) {
        let guard = Arc::new(CallbackGuard::default());
        let (handle, rx) = Self::new(callback, guard.clone(), back, RenderThreads::default());
        (handle, rx, guard)
    }

    /// Runs one period. Real-time safe: two atomic stores around the callback. In debug builds the whole call runs
    /// under the allocation trap (Parte 2, rule 5)
    pub fn call(&mut self, block: DeviceBlock<'_, '_>) {
        let Some(callback) = self.callback.as_mut() else { return };
        self.guard.inside.store(true, Ordering::Release);
        #[cfg(debug_assertions)]
        assert_no_alloc::assert_no_alloc(|| callback.process(block));
        #[cfg(not(debug_assertions))]
        callback.process(block);
        self.guard.inside.store(false, Ordering::Release);
        self.guard.calls.fetch_add(1, Ordering::AcqRel);
    }

    /// True once the device was asked to stop: the loop leaves at the next check
    pub fn should_stop(&self) -> bool {
        self.guard.is_stopping()
    }

    /// Tells the main thread something. Real-time safe; a full ring counts the loss, reported with the next event
    /// that fits
    pub fn report(&mut self, event: DeviceEvent) {
        if self.dropped > 0 && self.events.slots() >= 2 {
            let count = std::mem::take(&mut self.dropped);
            let _ = self.events.push(DeviceEvent::Dropped { count });
        }
        if self.events.push(event).is_err() {
            self.dropped += 1;
        }
    }

    /// The render-thread hooks (H1, H6): the backend runs its whole loop inside
    /// `threads().run(ThreadKind::Audio, ...)`
    pub fn threads(&self) -> RenderThreads {
        self.threads.clone()
    }
}

impl Drop for CallbackHandle {
    fn drop(&mut self) {
        if let Some(mut callback) = self.callback.take() {
            callback.finish();
            if let Some(back) = self.back.take() {
                // The device may already have given up on it (a stop that timed out): then it is dropped here
                let _ = back.send(callback);
            }
        }
    }
}

/// An audio backend (I-10.1). `open` and `start` may block (they run inside an `Operation`); `stop` never blocks.
pub trait Backend: Send {
    /// Opens the device with this configuration. Errors carry translatable keys: `io.device.busy` (another program
    /// has it and would not give it up, U8), `io.device.permission_denied` (with `audio_group`, U8),
    /// `io.device.not_found`, `io.device.rate_unsupported`, `io.device.two_cards` (U7) and the backend's own
    fn open(&mut self, config: &DeviceConfig) -> Result<DeviceInfo>;
    /// Starts the device thread, which calls `callback` once per period until asked to stop, and drops it when it
    /// leaves. On an error the callback is dropped at once (and so comes back)
    fn start(&mut self, callback: CallbackHandle) -> Result<()>;
    /// Asks the thread to leave (the guard is already marked). Never blocks
    fn stop(&mut self);
    /// After the callback came back: waits for the thread's last steps (it has left the callback). Short
    fn join(&mut self);
    /// Closes the device (after `join`). The backend may be opened again
    fn close(&mut self);
    /// The backend's name, for logs ("alsa", "dummy")
    fn kind(&self) -> &'static str;
}
