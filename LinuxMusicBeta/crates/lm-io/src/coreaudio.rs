//! The CoreAudio backend (interfaces.md, I-10; U7, U11, U17; C2, C6, C7), over `lm-sys-coreaudio`.
//!
//! **One device**, input and output in the same IO proc (one clock, one callback per buffer). The program asks
//! the device for the project's rate and the period's buffer size, and takes what it answers (C7). A device that
//! cannot run at the project's rate gets the edge resampler (only when allowed), like ALSA.
//!
//! **Input and output on different devices are joined only by an explicit private aggregate**, made here, when
//! the user chose that combination (`DeviceConfig::drift_correction`; U7: never behind the user's back). The
//! playback device is the aggregate's clock master (and first stream); the capture device follows with CoreAudio's
//! drift compensation on. Without `drift_correction` two different devices are refused (`io.device.two_cards`).
//! The aggregate lives while the backend is open and is destroyed on `close`.
//!
//! **The microphone permission** (macOS asks the first time a program reads an input): the input is opened only
//! when `DeviceConfig::capture` is set. A playback-only configuration turns the device's input streams off for
//! the IO proc, so no prompt appears.
//!
//! **Threads.** CoreAudio runs the IO proc on its own real-time thread, which the backend does not control: the
//! proc takes the engine (the [`CallbackHandle`], the planar buffers and the edge) from a mutex nobody else touches
//! while the device runs (an uncontended `try_lock`: one atomic). A **control thread** per start waits for a stop
//! or a device that went away, stops the device (CoreAudio's `AudioDeviceStop` returns after the last proc call:
//! this is the barrier's "the callback came back" step), reports what happened and drops the handle, which hands
//! the callback back (S16). `stop` only wakes it and never blocks.
//!
//! **Xruns:** `kAudioDeviceProcessorOverload` is counted by a listener and reported at the next cycle as
//! `DeviceEvent::Xrun` (the late frames go to that block's `late_by`). **A removed device** (U11) is seen by a
//! `DeviceIsAlive` listener: the control thread stops, reports `Disconnected` and leaves; the session falls back
//! to the Dummy.
//!
//! **Latency** reported: the device's, its safety offset and the largest stream latency (CoreAudio's own numbers),
//! plus one buffer on each side (plus one more on the input of an aggregate, for the drift compensation), scaled to
//! the project's rate, plus the edge resampler's.

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use lm_base::{Error, HostTime, Result, Role};
use lm_render::DeviceBlock;
use lm_sys_coreaudio::{self as ca, Aggregate, CaError, Cycle, Device, Event, Io, Scope};

use crate::backend::{Backend, CallbackHandle, DeviceConfig, DeviceEvent, DeviceInfo};
use crate::choice::{ChoiceCaps, DeviceChoice};
use crate::edge::{Edge, MAX_CHANNELS};
use crate::rt::RtBuffer;

/// Rates tried, in order, when the device cannot run at the project's and the edge resampler is allowed
const FALLBACK_RATES: [u32; 6] = [48_000, 44_100, 96_000, 88_200, 192_000, 176_400];

/// The usual rates, to list what a device offers when it gives ranges
const USUAL_RATES: [u32; 9] = [22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 384_000];

/// How long a rate change may take before the device is said not to do it
const RATE_TIMEOUT: Duration = Duration::from_secs(2);

/// A CoreAudio failure as an error with a translatable key
fn ca_error(e: CaError, device: &str) -> Error {
    if e.is_disconnected() {
        Error::new("io.device.not_found").arg("device", device)
    } else {
        Error::new("io.device.coreaudio").arg("device", device).arg("call", e.call).arg("status", e.code())
    }
}

/// What the control thread waits on
#[derive(Debug, Default)]
struct WakeState {
    stop: bool,
    gone: bool,
}

#[derive(Debug, Default)]
struct Wake {
    state: Mutex<WakeState>,
    changed: Condvar,
}

impl Wake {
    fn set(&self, f: impl FnOnce(&mut WakeState)) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut s);
        self.changed.notify_all();
    }
}

/// What `open` made, moved to the engine by `start`
struct Opened {
    info: DeviceInfo,
    device: Device,
    /// Every real device involved, watched for removal
    watched: Vec<Device>,
    inputs: usize,
    outputs: usize,
    /// Channels of the device's streams before the ones we use (an aggregate's first device may have inputs)
    in_offset: usize,
    out_offset: usize,
    edge: Option<Edge>,
}

/// The CoreAudio backend
pub struct CoreAudioBackend {
    open: Option<Opened>,
    aggregate: Option<Aggregate>,
    wake: Option<Arc<Wake>>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for CoreAudioBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoreAudioBackend")
            .field("open", &self.open.as_ref().map(|o| &o.info))
            .field("aggregate", &self.aggregate.is_some())
            .field("running", &self.thread.is_some())
            .finish()
    }
}

impl Default for CoreAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreAudioBackend {
    pub fn new() -> Self {
        Self { open: None, aggregate: None, wake: None, thread: None }
    }
}

/// The device the user chose for a direction: `default`, a UID or a name
fn resolve(name: &str, scope: Scope) -> Result<Device> {
    let device = Device::find(name, scope).map_err(|e| ca_error(e, name))?;
    match device {
        Some(d) if d.channels(scope).unwrap_or(0) > 0 => Ok(d),
        _ => Err(Error::new("io.device.not_found").arg("device", name)),
    }
}

/// Whether a rate fits what the device offers (a device that does not say is tried)
fn fits(device: Device, rate: u32) -> bool {
    match device.rate_ranges() {
        Ok(r) if !r.is_empty() => device.supports_rate(f64::from(rate)),
        _ => true,
    }
}

impl Backend for CoreAudioBackend {
    fn open(&mut self, config: &DeviceConfig) -> Result<DeviceInfo> {
        if self.thread.is_some() {
            return Err(Error::new("io.device.running"));
        }
        self.open = None;
        self.aggregate = None;
        if config.playback.is_none() && config.capture.is_none() {
            return Err(Error::new("io.device.bad_config"));
        }
        let play = config.playback.as_deref().map(|n| resolve(n, Scope::Output)).transpose()?;
        let cap = config.capture.as_deref().map(|n| resolve(n, Scope::Input)).transpose()?;
        // U7: two different devices only with the user's explicit combination
        let mut aggregate = None;
        let mut in_offset = 0usize;
        let mut watched = Vec::new();
        let device = match (play, cap) {
            (Some(p), Some(c)) if p != c => {
                if !config.drift_correction {
                    return Err(Error::new("io.device.two_cards")
                        .arg("playback", config.playback.clone().unwrap_or_default())
                        .arg("capture", config.capture.clone().unwrap_or_default()));
                }
                // The playback device is first: its outputs come first; its inputs (if any) come before the
                // capture device's
                let agg = Aggregate::create(p, c).map_err(|e| ca_error(e, &config.name()))?;
                in_offset = p.channels(Scope::Input).unwrap_or(0) as usize;
                let d = agg.device();
                aggregate = Some(agg);
                watched.extend([p, c, d]);
                d
            }
            (Some(p), _) => {
                watched.push(p);
                p
            }
            (None, Some(c)) => {
                watched.push(c);
                c
            }
            (None, None) => return Err(Error::new("io.device.bad_config")),
        };
        let name = config.name();
        let real_out = play.map_or(0, |p| p.channels(Scope::Output).unwrap_or(0));
        let real_in = cap.map_or(0, |c| c.channels(Scope::Input).unwrap_or(0));
        let outputs = config.playback_channels.map_or(real_out, |n| n.min(real_out)).min(MAX_CHANNELS as u32) as usize;
        let inputs = config.capture_channels.map_or(real_in, |n| n.min(real_in)).min(MAX_CHANNELS as u32) as usize;
        if (play.is_some() && outputs == 0) || (cap.is_some() && inputs == 0) {
            return Err(Error::new("io.device.not_found").arg("device", name));
        }
        // The rate: the project's when the device can, else (with the edge resampler) the first fallback it has
        let device_rate = if fits(device, config.rate) {
            config.rate
        } else if !config.allow_edge_resampler {
            return Err(Error::new("io.device.rate_unsupported").arg("rate", i64::from(config.rate)));
        } else {
            FALLBACK_RATES
                .iter()
                .copied()
                .find(|r| fits(device, *r))
                .ok_or_else(|| Error::new("io.device.rate_unsupported").arg("rate", i64::from(config.rate)))?
        };
        let current = device.nominal_rate().map_err(|e| ca_error(e, &name))?;
        if (current - f64::from(device_rate)).abs() > 0.5 {
            device.set_nominal_rate(f64::from(device_rate)).map_err(|e| ca_error(e, &name))?;
            if !device.wait_rate(f64::from(device_rate), RATE_TIMEOUT) {
                return Err(Error::new("io.device.rate_unsupported").arg("rate", i64::from(device_rate)));
            }
        }
        // The buffer: the period asked, inside the device's range; the answer is the negotiated period (C7)
        let (lo, hi) = device.buffer_range().map_err(|e| ca_error(e, &name))?;
        let wanted = config.period.max(16).clamp(lo.max(1), hi.max(lo.max(1)));
        if device.buffer_size().ok() != Some(wanted) {
            device.set_buffer_size(wanted).map_err(|e| ca_error(e, &name))?;
        }
        let period = device.buffer_size().map_err(|e| ca_error(e, &name))?.max(1);
        let periods = config.periods.clamp(2, 8);
        let edge = if device_rate != config.rate {
            Some(Edge::new(config.rate, device_rate, inputs, outputs, period as usize * 2, false)?)
        } else {
            None
        };
        let scale = |frames: u32| (f64::from(frames) * f64::from(config.rate) / f64::from(device_rate)).round() as u32;
        let edge_latency = edge.as_ref().map_or(0, Edge::latency);
        let drift = aggregate.is_some();
        let in_extra =
            cap.map_or(0, |c| c.latency(Scope::Input).unwrap_or(0)) + period + if drift { period } else { 0 };
        let out_extra = play.map_or(0, |p| p.latency(Scope::Output).unwrap_or(0)) + period;
        let info = DeviceInfo {
            name,
            rate: config.rate,
            device_rate,
            period,
            periods,
            buffer: period * periods,
            max_block: edge.as_ref().map_or(period as usize, Edge::block),
            inputs,
            outputs,
            input_latency: if inputs > 0 { scale(in_extra) + edge_latency / 2 } else { 0 },
            output_latency: if outputs > 0 { scale(out_extra) + edge_latency - edge_latency / 2 } else { 0 },
            edge_resampler: device_rate != config.rate,
            drift_correction: drift,
            // CoreAudio runs both sides of one device on its clock; an aggregate corrects the follower's drift
            linked: !drift && inputs > 0 && outputs > 0,
        };
        self.aggregate = aggregate;
        self.open =
            Some(Opened { info: info.clone(), device, watched, inputs, outputs, in_offset, out_offset: 0, edge });
        Ok(info)
    }

    fn start(&mut self, callback: CallbackHandle) -> Result<()> {
        if self.thread.is_some() {
            return Err(Error::new("io.device.running"));
        }
        let Some(opened) = self.open.take() else {
            return Err(Error::new("io.device.not_open"));
        };
        let Opened { info, device, watched, inputs, outputs, in_offset, out_offset, edge } = opened;
        let period = info.period as usize;
        let wake = Arc::new(Wake::default());
        let overloads = Arc::new(AtomicU32::new(0));
        let engine = Engine {
            handle: callback,
            inputs: (0..inputs).map(|_| RtBuffer::zeroed(period)).collect(),
            outputs: (0..outputs).map(|_| RtBuffer::zeroed(period)).collect(),
            cap: period,
            in_offset,
            out_offset,
            device_rate: info.device_rate,
            edge,
            overloads: overloads.clone(),
            lost: 0,
            late: 0,
            role_set: false,
        };
        let engine = Arc::new(Mutex::new(engine));
        // Everything that can fail is made here, so `start` can say so; the callback comes back on any error
        // (dropping the engine drops the handle)
        let shared = engine.clone();
        let mut io = Io::new(device, inputs > 0, move |cycle| match shared.try_lock() {
            Ok(mut e) => e.cycle(cycle),
            // Poisoned (a panic in an earlier cycle) or held: silence, the control thread reports it
            Err(_) => cycle.silence(),
        })
        .map_err(|e| ca_error(e, &info.name))?;
        let mut watches = Vec::new();
        for d in &watched {
            let w = wake.clone();
            watches.push(d.watch(Event::Gone, move || w.set(|s| s.gone = true)).map_err(|e| ca_error(e, &info.name))?);
        }
        {
            let o = overloads.clone();
            watches.push(
                device
                    .watch(Event::Overload, move || {
                        o.fetch_add(1, Ordering::Relaxed);
                    })
                    .map_err(|e| ca_error(e, &info.name))?,
            );
        }
        if let Ok(mut e) = engine.lock() {
            e.handle.report(DeviceEvent::Started { realtime: None });
        }
        if let Err(e) = io.start() {
            drop(io);
            return Err(ca_error(e, &info.name));
        }
        let control = wake.clone();
        let thread = std::thread::Builder::new()
            .name("lm-audio-coreaudio".to_owned())
            .spawn(move || {
                // Waits for a stop, a removed device, or a panic in the proc
                {
                    let mut s = control.state.lock().unwrap_or_else(|p| p.into_inner());
                    while !s.stop && !s.gone && !io.panicked() {
                        let (g, _) = control
                            .changed
                            .wait_timeout(s, Duration::from_millis(100))
                            .unwrap_or_else(|p| p.into_inner());
                        s = g;
                    }
                }
                // The barrier: `stop` returns after the last cycle of the proc. From here nobody else touches the
                // engine
                let stopped = io.stop();
                let gone = control.state.lock().unwrap_or_else(|p| p.into_inner()).gone;
                let panicked = io.panicked();
                {
                    let mut e = engine.lock().unwrap_or_else(|p| p.into_inner());
                    if panicked {
                        e.handle.report(DeviceEvent::Panicked);
                    } else if gone {
                        e.handle.report(DeviceEvent::Disconnected);
                    } else if let Err(err) = stopped
                        && !err.is_disconnected()
                    {
                        e.handle.report(DeviceEvent::Failed { errno: err.status, call: err.call });
                    }
                }
                // The listeners and the proc go first (they hold the engine), then the engine: dropping its
                // handle hands the callback back
                drop(watches);
                drop(io);
                drop(engine);
            })
            .map_err(|e| Error::new("io.device.thread_failed").arg("detail", e.to_string()))?;
        self.wake = Some(wake);
        self.thread = Some(thread);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(w) = &self.wake {
            w.set(|s| s.stop = true);
        }
    }

    fn join(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn close(&mut self) {
        self.open = None;
        self.wake = None;
        // Destroys the private aggregate: the devices are free again
        self.aggregate = None;
    }

    fn kind(&self) -> &'static str {
        "coreaudio"
    }
}

/// What runs inside the IO proc: the callback, the planar buffers and the edge
struct Engine {
    handle: CallbackHandle,
    inputs: Vec<RtBuffer>,
    outputs: Vec<RtBuffer>,
    /// Frames of one chunk: the negotiated period, and the length of every buffer
    cap: usize,
    in_offset: usize,
    out_offset: usize,
    device_rate: u32,
    edge: Option<Edge>,
    overloads: Arc<AtomicU32>,
    lost: u32,
    late: u32,
    role_set: bool,
}

impl Engine {
    /// One IO proc cycle (CoreAudio's real-time thread). Never allocates (the `CallbackHandle` runs the
    /// renderer under the allocation trap in debug builds)
    fn cycle(&mut self, c: &mut Cycle<'_>) {
        if !self.role_set {
            lm_base::set_role(Role::Audio);
            self.role_set = true;
        }
        if self.handle.should_stop() {
            c.silence();
            return;
        }
        let overloaded = self.overloads.swap(0, Ordering::Relaxed);
        if overloaded > 0 {
            let late = self.cap as u32 * overloaded;
            self.late = self.late.saturating_add(late);
            self.handle.report(DeviceEvent::Xrun { capture_lost: 0, late_by: late });
        }
        let frames = cycle_frames(c);
        c.silence();
        if frames == 0 {
            return;
        }
        // C6: the host time of the first captured sample (capture: it was taken before the proc ran; playback
        // only: the proc's own time). Converted to the program's monotonic clock through the difference
        let now = HostTime::now().0;
        let reference = if self.inputs.is_empty() { c.now() } else { c.input_time().unwrap_or(c.now()) };
        let host0 = now.saturating_sub(ca::host_nanos(c.now().saturating_sub(reference)));
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.cap);
            self.read_inputs(c, done, n);
            let t = HostTime(host0 + done as u64 * 1_000_000_000 / u64::from(self.device_rate));
            self.render(t, n);
            self.write_outputs(c, done, n);
            done += n;
        }
    }

    /// Deinterleaves `n` frames of the input streams, from frame `done`, into the planar buffers
    fn read_inputs(&mut self, c: &Cycle<'_>, done: usize, n: usize) {
        let ch_in = self.inputs.len();
        if ch_in == 0 {
            return;
        }
        let mut global = 0usize;
        for s in 0..c.input_streams() {
            let Some((channels, data)) = c.input(s) else { continue };
            let channels = channels as usize;
            for k in 0..channels {
                let g = global + k;
                if g >= self.in_offset && g < self.in_offset + ch_in {
                    let dst = &mut self.inputs[g - self.in_offset];
                    let src = data.iter().skip(done * channels + k).step_by(channels.max(1));
                    for (d, v) in dst[..n].iter_mut().zip(src) {
                        *d = *v;
                    }
                }
            }
            global += channels;
        }
    }

    /// Interleaves the planar outputs into the output streams
    fn write_outputs(&mut self, c: &mut Cycle<'_>, done: usize, n: usize) {
        let ch_out = self.outputs.len();
        if ch_out == 0 {
            return;
        }
        let mut global = 0usize;
        for s in 0..c.output_streams() {
            let Some((channels, data)) = c.output(s) else { continue };
            let channels = channels as usize;
            for k in 0..channels {
                let g = global + k;
                if g >= self.out_offset && g < self.out_offset + ch_out {
                    let src = &self.outputs[g - self.out_offset];
                    let dst = data.iter_mut().skip(done * channels + k).step_by(channels.max(1));
                    for (d, v) in dst.zip(&src[..n]) {
                        *d = *v;
                    }
                }
            }
            global += channels;
        }
    }

    /// Runs the renderer on `n` frames of the planar buffers (through the edge when the rates differ)
    fn render(&mut self, host_time: HostTime, n: usize) {
        let ch_in = self.inputs.len();
        let ch_out = self.outputs.len();
        let mut src: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        for (s, b) in src.iter_mut().zip(self.inputs.iter()) {
            *s = &b[..n];
        }
        let mut dst: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
        for (d, b) in dst.iter_mut().zip(self.outputs.iter_mut()) {
            *d = &mut b[..n];
        }
        let handle = &mut self.handle;
        if let Some(edge) = self.edge.as_mut() {
            if self.lost > 0 {
                edge.add_capture_lost(std::mem::take(&mut self.lost));
            }
            if self.late > 0 {
                edge.add_late(std::mem::take(&mut self.late));
            }
            edge.push_capture(&src[..ch_in], n);
            edge.period(host_time, &mut dst[..ch_out], n, &mut |b| handle.call(b));
        } else {
            handle.call(DeviceBlock {
                frames: n,
                host_time,
                inputs: &src[..ch_in],
                outputs: &mut dst[..ch_out],
                capture_lost: std::mem::take(&mut self.lost),
                late_by: std::mem::take(&mut self.late),
            });
        }
    }
}

/// Frames in this cycle: from the output streams, else the input's
fn cycle_frames(c: &mut Cycle<'_>) -> usize {
    for s in 0..c.output_streams() {
        if let Some((channels, data)) = c.output(s)
            && channels > 0
        {
            return data.len() / channels as usize;
        }
    }
    for s in 0..c.input_streams() {
        if let Some((channels, data)) = c.input(s)
            && channels > 0
        {
            return data.len() / channels as usize;
        }
    }
    0
}

/// The rates of a device, as the list shows them: the discrete ones, or the usual ones inside the ranges
fn rates_of(ranges: &[(f64, f64)]) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for (lo, hi) in ranges {
        if (hi - lo).abs() < 0.5 {
            out.push(lo.round() as u32);
        } else {
            out.extend(
                USUAL_RATES.iter().copied().filter(|r| f64::from(*r) >= *lo - 0.5 && f64::from(*r) <= *hi + 0.5),
            );
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Lists the devices (U17). Enumerating can take time with USB devices: run it in an `Operation`. Cancellable
/// between devices. `name` is the UID (stable); `card` and `description` the name the user sees
pub fn list_devices(cancel: &lm_base::Cancel, progress: &lm_base::Progress) -> Result<Vec<DeviceChoice>> {
    let devices = Device::all().map_err(|e| ca_error(e, ""))?;
    let mut out = Vec::new();
    for (k, device) in devices.iter().enumerate() {
        cancel.check()?;
        progress.set(k as f32 / devices.len().max(1) as f32);
        let Ok(d) = device.describe() else { continue };
        if d.inputs + d.outputs == 0 {
            continue;
        }
        let caps = |channels: u32| {
            (channels > 0).then(|| ChoiceCaps { channels_min: 1, channels_max: channels, rates: rates_of(&d.rates) })
        };
        out.push(DeviceChoice {
            name: d.uid.clone(),
            card: d.name.clone(),
            description: d.name.clone(),
            playback: caps(d.outputs),
            capture: caps(d.inputs),
            busy: false,
            denied: false,
        });
    }
    progress.set(1.0);
    Ok(out)
}

/// Whether the device is present (U11: the periodic scan that offers to reconnect)
pub fn device_present(name: &str) -> bool {
    match Device::find(name, Scope::Output) {
        Ok(Some(_)) => true,
        _ => matches!(Device::find(name, Scope::Input), Ok(Some(_))),
    }
}
