//! The device (interfaces.md, I-10.1, R5, S16, D11; U11, U16): a backend, the renderer it drives, and the barrier.
//!
//! [`Device::stop`] gives the [`Renderer`] back only after the last callback has returned: the callback is owned by
//! the device thread and comes back when the thread leaves the loop for good. If it does not come back within
//! [`STOP_TIMEOUT`] (2 s, U16), the callback is stuck (a plugin in an endless loop): the device is **abandoned, never
//! killed** (D11). Its thread keeps the renderer; a watcher thread waits for it, and closes the device if it ever
//! returns. The session then makes a new engine and recompiles from the tree.
//!
//! The `Drop` of a device runs the same barrier (S16): a session that goes away without calling `stop` still waits
//! for the last callback.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use lm_base::ring::{Consumer, Producer, ring};
use lm_base::{Error, Result};
use lm_render::{DeviceBlock, RecordSpan, RenderThreads, Renderer};

use crate::backend::{Backend, CallbackGuard, CallbackHandle, DeviceCallback, DeviceConfig, DeviceEvent, DeviceInfo};
use crate::record::{MAX_SPANS, RecordSender};

/// How long `stop` waits for the last callback (U16)
pub const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Devices abandoned with a stuck callback since the program started
static ABANDONED: AtomicU64 = AtomicU64::new(0);

/// Abandoned devices whose callback has not returned yet: each one still holds a thread, a renderer and, for a
/// hardware backend, the card
static STUCK: AtomicU64 = AtomicU64::new(0);

/// A hardware device is refused while this many abandoned callbacks are still stuck (A7): every one holds the
/// card or a stream, so opening more only piles up threads and renderers. The Dummy is never refused, so the
/// session can always fall back to it. A stuck callback that returns frees its slot
pub const MAX_STUCK_DEVICES: u64 = 3;

/// How many abandoned devices are still stuck right now
pub fn stuck_devices() -> u64 {
    STUCK.load(Ordering::Relaxed)
}

/// How many devices were abandoned with a stuck callback in this process (D11)
pub fn abandoned_devices() -> u64 {
    ABANDONED.load(Ordering::Relaxed)
}

/// The stop did not finish: the callback is stuck (D11). The renderer stays with the abandoned thread
#[derive(Debug)]
pub struct StopTimedOut {
    /// Callbacks that had completed when the device was abandoned
    pub calls: u64,
}

impl StopTimedOut {
    pub fn to_error(&self) -> Error {
        Error::new("io.device.stop_timed_out").arg("seconds", STOP_TIMEOUT.as_secs_f64())
    }
}

/// `Device::start` failed. The renderer comes back whenever it can be given back
#[derive(Debug)]
pub enum StartError {
    /// Refused before the callback was handed over, or the backend returned it: the renderer is free
    Refused { error: Error, renderer: Box<Renderer> },
    /// The backend failed to start and kept the callback, so the renderer is with it. The device was abandoned
    /// (D11), like a stuck stop; the error is `io.device.start_lost`, with the cause's key
    Lost { error: Error },
}

impl StartError {
    pub fn error(&self) -> &Error {
        match self {
            Self::Refused { error, .. } | Self::Lost { error } => error,
        }
    }

    pub fn into_error(self) -> Error {
        match self {
            Self::Refused { error, .. } | Self::Lost { error } => error,
        }
    }

    /// The error and, when it exists, the renderer
    pub fn into_parts(self) -> (Error, Option<Renderer>) {
        match self {
            Self::Refused { error, renderer } => (error, Some(*renderer)),
            Self::Lost { error } => (error, None),
        }
    }
}

/// Messages from the main thread to the driver: ownership crosses, so the ring never drops (T9). Sized for the
/// most in flight, which the device enforces
enum DriverMsg {
    Arm(Box<RecordSender>),
    Disarm,
}

const DRIVER_RING: usize = 8;

/// The renderer's callback: the renderer, then the recorder of the armed inputs, in the same callback (the
/// recorder needs the block's record spans at once)
struct Driver {
    renderer: Renderer,
    recorder: Option<Box<RecordSender>>,
    control: Consumer<DriverMsg>,
    back: Producer<Box<RecordSender>>,
    spans: [RecordSpan; MAX_SPANS],
    /// Arms taken from the ring (the device compares with the arms it sent)
    arms_taken: Arc<AtomicU64>,
}

impl Driver {
    fn take_messages(&mut self) {
        while let Ok(msg) = self.control.pop() {
            let old = match msg {
                DriverMsg::Arm(sender) => {
                    self.arms_taken.fetch_add(1, Ordering::Release);
                    self.recorder.replace(sender)
                }
                DriverMsg::Disarm => self.recorder.take(),
            };
            if let Some(mut old) = old {
                old.end();
                // The device never has more senders out than the ring holds
                let _ = self.back.push(old);
            }
        }
    }
}

impl DeviceCallback for Driver {
    fn process(&mut self, block: DeviceBlock<'_, '_>) {
        self.take_messages();
        let frames = block.frames;
        let inputs = block.inputs;
        let report = self.renderer.render(block);
        let mut count = 0;
        for (slot, span) in self.spans.iter_mut().zip(report.recorded()) {
            *slot = *span;
            count += 1;
        }
        let gap = report.capture_gap;
        if let Some(recorder) = self.recorder.as_mut() {
            let playing = self.renderer.is_playing();
            let position = self.renderer.position();
            recorder.capture(frames, &self.spans[..count], gap, inputs, playing, position);
        }
    }

    fn finish(&mut self) {
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.end();
        }
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

enum State {
    Stopped,
    Running {
        returned: Receiver<Box<dyn DeviceCallback>>,
        control: Producer<DriverMsg>,
        back: Consumer<Box<RecordSender>>,
    },
}

/// A device with its backend (I-10.1). `Send`: it moves into the `Operation` that switches devices (I-10.3). The
/// session keeps it as its first field, so it stops before anything else goes (S16)
pub struct Device {
    backend: Option<Box<dyn Backend>>,
    info: Option<DeviceInfo>,
    config: Option<DeviceConfig>,
    guard: Arc<CallbackGuard>,
    threads: RenderThreads,
    state: State,
    events: Option<Consumer<DeviceEvent>>,
    /// Armed before start, or come back from the driver
    armed: Option<Box<RecordSender>>,
    returned_senders: Vec<RecordSender>,
    senders_out: usize,
    disconnected: bool,
    arms_sent: u64,
    arms_taken: Arc<AtomicU64>,
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("backend", &self.backend.as_ref().map(|b| b.kind()))
            .field("info", &self.info)
            .field("running", &self.is_running())
            .finish()
    }
}

impl Device {
    /// A device over `backend`; `threads` are the render-thread hooks (the same the engine was made with: the
    /// real-time membership, H1)
    pub fn new(backend: Box<dyn Backend>, threads: RenderThreads) -> Self {
        Self {
            backend: Some(backend),
            info: None,
            config: None,
            guard: Arc::new(CallbackGuard::default()),
            threads,
            state: State::Stopped,
            events: None,
            armed: None,
            returned_senders: Vec::new(),
            senders_out: 0,
            disconnected: false,
            arms_sent: 0,
            arms_taken: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Opens the device (blocks: inside an `Operation`). The negotiated [`DeviceInfo::spec`] is what the engine must
    /// be made with (C7)
    pub fn open(&mut self, config: &DeviceConfig) -> Result<DeviceInfo> {
        if self.is_running() {
            return Err(Error::new("io.device.running"));
        }
        let backend = self.backend.as_mut().ok_or_else(|| Error::new("io.device.abandoned"))?;
        check_stuck(stuck_devices(), backend.kind())?;
        let info = backend.open(config)?;
        self.info = Some(info.clone());
        self.config = Some(config.clone());
        self.disconnected = false;
        Ok(info)
    }

    pub fn info(&self) -> Option<&DeviceInfo> {
        self.info.as_ref()
    }

    pub fn config(&self) -> Option<&DeviceConfig> {
        self.config.as_ref()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }

    /// The barrier's counters (for the watchdog: a period count that stops moving is a stuck callback)
    pub fn guard(&self) -> &Arc<CallbackGuard> {
        &self.guard
    }

    /// Starts the device thread with `renderer`. The renderer's `max_block` must be at least the device's (C7). On a
    /// refusal the renderer comes back with the error; if the backend kept the callback, nothing can (see
    /// [`StartError`])
    pub fn start(&mut self, renderer: Renderer) -> std::result::Result<(), StartError> {
        let refuse = |error: Error, renderer: Renderer| StartError::Refused { error, renderer: Box::new(renderer) };
        if self.is_running() {
            return Err(refuse(Error::new("io.device.running"), renderer));
        }
        let Some(info) = self.info.clone() else {
            return Err(refuse(Error::new("io.device.not_open"), renderer));
        };
        let spec = renderer.spec();
        if spec.max_block < info.max_block || spec.sample_rate != info.rate {
            return Err(refuse(
                Error::new("io.device.spec_mismatch")
                    .arg("engine_block", spec.max_block as i64)
                    .arg("device_block", info.max_block as i64)
                    .arg("engine_rate", i64::from(spec.sample_rate))
                    .arg("device_rate", i64::from(info.rate)),
                renderer,
            ));
        }
        let Some(backend) = self.backend.as_mut() else {
            return Err(refuse(Error::new("io.device.abandoned"), renderer));
        };
        let (control, control_rx) = ring(DRIVER_RING);
        let (back_tx, back) = ring(DRIVER_RING);
        self.arms_sent = 0;
        self.arms_taken = Arc::new(AtomicU64::new(0));
        let driver = Driver {
            renderer,
            recorder: None,
            control: control_rx,
            back: back_tx,
            spans: [RecordSpan::default(); MAX_SPANS],
            arms_taken: self.arms_taken.clone(),
        };
        let (returned_tx, returned) = mpsc::sync_channel(1);
        self.guard = Arc::new(CallbackGuard::default());
        let (handle, events) =
            CallbackHandle::new(Box::new(driver), self.guard.clone(), returned_tx, self.threads.clone());
        if let Err(e) = backend.start(handle) {
            // The handle was dropped by the backend: the driver is already back
            return Err(match returned.recv_timeout(STOP_TIMEOUT) {
                Ok(cb) => refuse(e, take_driver(cb).renderer),
                Err(_) => {
                    // A backend that failed to start yet kept the callback: it is abandoned like a stuck stop,
                    // and its watcher frees it if the callback ever comes back
                    self.abandon(returned);
                    StartError::Lost { error: Error::new("io.device.start_lost").arg("cause", e.key()) }
                }
            });
        }
        self.events = Some(events);
        self.state = State::Running { returned, control, back };
        self.senders_out = 0;
        if let Some(sender) = self.armed.take()
            && let Err(sender) = self.arm(*sender)
        {
            self.armed = Some(Box::new(sender));
        }
        Ok(())
    }

    /// Arms a recording: the sender goes to the device callback, which records from the next played block. If the
    /// device is not running, the sender waits here for `start`. Returns the sender if it cannot be sent now
    pub fn arm(&mut self, sender: RecordSender) -> std::result::Result<(), RecordSender> {
        match &mut self.state {
            State::Stopped => {
                if self.armed.is_some() {
                    return Err(sender);
                }
                self.armed = Some(Box::new(sender));
                Ok(())
            }
            State::Running { control, .. } => {
                // One ring slot is always kept for a Disarm
                if self.senders_out + 2 > DRIVER_RING {
                    return Err(sender);
                }
                match control.push(DriverMsg::Arm(Box::new(sender))) {
                    Ok(()) => {
                        self.senders_out += 1;
                        self.arms_sent += 1;
                        Ok(())
                    }
                    Err(rtrb::PushError::Full(DriverMsg::Arm(s))) => Err(*s),
                    Err(rtrb::PushError::Full(DriverMsg::Disarm)) => unreachable!("pushed an Arm"),
                }
            }
        }
    }

    /// True once the device callback has taken every sender given to [`Device::arm`]. The session arms, waits for
    /// this (one period), and only then sends `Command::Record`: a recorder that arrives after the transport started
    /// recording would miss the first blocks
    pub fn is_armed(&self) -> bool {
        match self.state {
            State::Stopped => self.armed.is_some(),
            State::Running { .. } => self.arms_sent > 0 && self.arms_taken.load(Ordering::Acquire) == self.arms_sent,
        }
    }

    /// Disarms: the take in progress ends at the next block, and the sender comes back through [`Device::poll`]
    pub fn disarm(&mut self) {
        match &mut self.state {
            State::Stopped => {
                if let Some(mut s) = self.armed.take() {
                    s.end();
                    self.returned_senders.push(*s);
                }
            }
            State::Running { control, .. } => {
                let _ = control.push(DriverMsg::Disarm);
            }
        }
    }

    /// From the main thread's pump: the device's events, and the senders that came back (drop them on the main
    /// thread, rule 5)
    pub fn poll(&mut self, mut f: impl FnMut(DeviceEvent)) -> Vec<RecordSender> {
        if let Some(events) = self.events.as_mut() {
            while let Ok(e) = events.pop() {
                if matches!(e, DeviceEvent::Disconnected | DeviceEvent::Failed { .. } | DeviceEvent::Panicked) {
                    self.disconnected = true;
                }
                f(e);
            }
        }
        if let State::Running { back, .. } = &mut self.state {
            while let Ok(s) = back.pop() {
                self.senders_out = self.senders_out.saturating_sub(1);
                self.returned_senders.push(*s);
            }
        }
        std::mem::take(&mut self.returned_senders)
    }

    /// True after the thread reported that the card went away or it failed (U11): `stop` returns at once
    pub fn is_lost(&self) -> bool {
        self.disconnected
    }

    /// Stops the device: the renderer comes back only after the last callback has returned (the barrier). Blocks up
    /// to [`STOP_TIMEOUT`]: call it from an `Operation` or the session's `Drop` (U16). On a timeout the device is
    /// abandoned (D11) and cannot be used again
    pub fn stop(&mut self) -> std::result::Result<Renderer, StopTimedOut> {
        let State::Running { returned, control, back } = std::mem::replace(&mut self.state, State::Stopped) else {
            return Err(StopTimedOut { calls: self.guard.calls() });
        };
        self.guard.request_stop();
        let Some(backend) = self.backend.as_mut() else {
            return Err(StopTimedOut { calls: self.guard.calls() });
        };
        backend.stop();
        match returned.recv_timeout(STOP_TIMEOUT) {
            Ok(callback) => {
                backend.join();
                let mut driver = take_driver(callback);
                drop(control);
                // Senders still in the control ring or already back, and the one in the driver (ended by `finish`)
                while let Ok(msg) = driver.control.pop() {
                    if let DriverMsg::Arm(s) = msg {
                        self.returned_senders.push(*s);
                    }
                }
                let mut back = back;
                while let Ok(s) = back.pop() {
                    self.returned_senders.push(*s);
                }
                if let Some(s) = driver.recorder.take() {
                    self.returned_senders.push(*s);
                }
                self.senders_out = 0;
                if let Some(events) = self.events.as_mut() {
                    while let Ok(e) = events.pop() {
                        if matches!(e, DeviceEvent::Disconnected | DeviceEvent::Failed { .. }) {
                            self.disconnected = true;
                        }
                    }
                }
                Ok(driver.renderer)
            }
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                let calls = self.guard.calls();
                self.abandon(returned);
                Err(StopTimedOut { calls })
            }
        }
    }

    /// Closes the device after a stop (the backend can be opened again)
    pub fn close(&mut self) {
        if !self.is_running()
            && let Some(b) = self.backend.as_mut()
        {
            b.close();
        }
        self.info = None;
    }

    /// The backend, after a stop (to open another configuration, or to hand it to the calibration)
    pub fn into_backend(mut self) -> Option<Box<dyn Backend>> {
        if self.is_running() {
            return None;
        }
        self.backend.take()
    }

    /// D11: the stuck thread keeps the renderer; a watcher waits for it forever and closes the device if it ever
    /// comes back. Nothing is killed
    fn abandon(&mut self, returned: Receiver<Box<dyn DeviceCallback>>) {
        ABANDONED.fetch_add(1, Ordering::Relaxed);
        STUCK.fetch_add(1, Ordering::Relaxed);
        self.events = None;
        self.info = None;
        if let Some(backend) = self.backend.take() {
            // Shared with the watcher, so that a watcher that never starts does not drop the backend (and its
            // callback) with the closure: the failed spawn leaves this handle, which is then leaked on purpose
            let shared = Arc::new(Mutex::new(Some(backend)));
            let held = shared.clone();
            let watcher = std::thread::Builder::new().name("lm-abandoned-device".to_owned()).spawn(move || {
                let callback = returned.recv();
                // Back (or the thread is gone): the slot is free
                STUCK.fetch_sub(1, Ordering::Relaxed);
                if let Ok(callback) = callback {
                    let backend = held.lock().ok().and_then(|mut b| b.take());
                    if let Some(mut backend) = backend {
                        backend.join();
                        backend.close();
                    }
                    drop(callback);
                }
            });
            // Without a watcher thread the device is leaked as it is: still never killed
            if watcher.is_err() {
                std::mem::forget(shared);
            }
        }
    }
}

impl Drop for Device {
    /// The barrier (S16): a session dropped without `stop` still waits for the last callback
    fn drop(&mut self) {
        if self.is_running() {
            match self.stop() {
                Ok(renderer) => drop(renderer),
                Err(timeout) => {
                    #[allow(clippy::print_stderr)]
                    {
                        eprintln!(
                            "lm-io: the audio callback did not return within {} s; the device was abandoned \
                             ({} callbacks completed)",
                            STOP_TIMEOUT.as_secs(),
                            timeout.calls
                        );
                    }
                }
            }
        }
        if let Some(b) = self.backend.as_mut() {
            b.close();
        }
    }
}

/// The A7 policy: a hardware backend is refused while `stuck` abandoned callbacks still hold their resources
fn check_stuck(stuck: u64, kind: &str) -> Result<()> {
    if stuck >= MAX_STUCK_DEVICES && kind != "dummy" {
        return Err(Error::new("io.device.too_many_abandoned").arg("stuck", stuck as i64).arg("max", MAX_STUCK_DEVICES as i64));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_is_refused_over_the_stuck_cap_but_the_dummy_never() {
        assert!(check_stuck(MAX_STUCK_DEVICES - 1, "alsa").is_ok());
        let e = check_stuck(MAX_STUCK_DEVICES, "alsa").expect_err("refused");
        assert_eq!(e.key(), "io.device.too_many_abandoned");
        assert!(check_stuck(MAX_STUCK_DEVICES + 5, "dummy").is_ok());
    }
}

fn take_driver(callback: Box<dyn DeviceCallback>) -> Driver {
    match callback.into_any().downcast::<Driver>() {
        Ok(d) => *d,
        Err(_) => unreachable!("the device only starts its own driver"),
    }
}

/// Switches to another device (I-10.3), inside an `Operation` on the target `Device`: the barrier, open the new
/// one, and start it with the same renderer. A different negotiated block or rate than the engine's is refused
/// (`io.device.spec_mismatch`): the session then makes a new engine
pub fn switch_device(
    mut old: Device,
    backend: Box<dyn Backend>,
    config: &DeviceConfig,
    threads: RenderThreads,
) -> std::result::Result<Device, SwitchFailed> {
    let renderer = match old.stop() {
        Ok(r) => r,
        Err(t) => return Err(SwitchFailed::Stuck(t)),
    };
    let senders = old.poll(|_| {});
    old.close();
    drop(old);
    let mut new = Device::new(backend, threads);
    if let Err(e) = new.open(config) {
        return Err(SwitchFailed::Open { error: e, renderer: Box::new(renderer), senders });
    }
    match new.start(renderer) {
        Ok(()) => {
            new.returned_senders = senders;
            Ok(new)
        }
        Err(StartError::Refused { error, renderer }) => Err(SwitchFailed::Open { error, renderer, senders }),
        Err(StartError::Lost { error }) => Err(SwitchFailed::Lost { error }),
    }
}

/// Why a switch failed; the renderer comes back when it can
#[derive(Debug)]
pub enum SwitchFailed {
    /// The old device's callback is stuck: it was abandoned (D11)
    Stuck(StopTimedOut),
    /// The new device did not open or start; the renderer is free for the Dummy
    Open { error: Error, renderer: Box<Renderer>, senders: Vec<RecordSender> },
    /// The new device failed to start and kept the renderer: it was abandoned, and the session builds a new engine
    Lost { error: Error },
}
