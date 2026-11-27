//! The device without hardware (interfaces.md, Parte 3: the first backend, the model of a device loop): a thread that
//! calls the callback once per period, paced by the clock or as fast as allowed, with capture from a source (a test
//! signal, a loopback of the output) and faults injected on demand (capture xruns, late blocks, a removed card).
//!
//! It is the session's fallback when the card goes away (U11), the engine of the tests that need a running device
//! (the barrier, recording an hour in seconds, the calibration against a known latency), and the reference of what a
//! backend must do.

use std::fmt;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lm_base::{Error, HostTime, Result, Role};
use lm_render::{DeviceBlock, ThreadKind};

use crate::backend::{Backend, CallbackHandle, DeviceConfig, DeviceEvent, DeviceInfo};
use crate::edge::{Edge, MAX_CHANNELS};

/// How the Dummy's periods follow the clock
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pace {
    /// One period per period of real time; host times from the monotonic clock
    RealTime,
    /// As fast as the callback runs, at most `speed` times real time when given; host times from a fake clock that
    /// advances by the period's duration (an hour of recording in seconds)
    Free { speed: Option<f64> },
}

/// What the Dummy captures
pub trait DummySource: Send {
    /// Fills `frames` of each input channel; `first_frame` is the device frame count since the start, `time` the
    /// host time of the first frame
    fn fill(&mut self, first_frame: u64, time: HostTime, inputs: &mut [Vec<f32>], frames: usize);
}

/// Silence
#[derive(Debug, Default)]
pub struct Silence;

impl DummySource for Silence {
    fn fill(&mut self, _: u64, _: HostTime, inputs: &mut [Vec<f32>], frames: usize) {
        for c in inputs {
            c[..frames].fill(0.0);
        }
    }
}

/// Faults to inject and counters to read, shared with the test or the session
#[derive(Debug, Default)]
pub struct DummyControl {
    capture_lost: AtomicU32,
    late_by: AtomicU32,
    disconnect: AtomicBool,
    frames: AtomicU64,
    periods: AtomicU64,
}

impl DummyControl {
    /// The next period comes after a capture xrun that lost `frames` device frames (C2): the frames the source would
    /// have given are skipped, as a real capture loses them
    pub fn inject_capture_lost(&self, frames: u32) {
        self.capture_lost.fetch_add(frames, Ordering::AcqRel);
    }

    /// The next period is late by `frames` (a playback xrun)
    pub fn inject_late(&self, frames: u32) {
        self.late_by.fetch_add(frames, Ordering::AcqRel);
    }

    /// The card goes away at the next period (U11)
    pub fn disconnect(&self) {
        self.disconnect.store(true, Ordering::Release);
    }

    /// Device frames run since the start (lost frames included)
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Acquire)
    }

    pub fn periods(&self) -> u64 {
        self.periods.load(Ordering::Acquire)
    }
}

/// The Dummy's shape
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DummyOptions {
    pub inputs: usize,
    pub outputs: usize,
    /// The device's rate; `None`: the project's. Another rate runs the edge resampler (if the config allows it)
    pub device_rate: Option<u32>,
    pub pace: Pace,
    /// Outputs come back into the inputs `latency` device frames later (added to the source), for the calibration
    pub loopback: Option<u32>,
    /// Latency reported by the "device", each way, in device frames
    pub reported_latency: u32,
}

impl Default for DummyOptions {
    fn default() -> Self {
        Self { inputs: 2, outputs: 2, device_rate: None, pace: Pace::RealTime, loopback: None, reported_latency: 0 }
    }
}

struct Run {
    options: DummyOptions,
    period: usize,
    device_rate: u32,
    edge: Option<Edge>,
    source: Box<dyn DummySource>,
    control: Arc<DummyControl>,
}

/// The Dummy backend
pub struct DummyBackend {
    options: DummyOptions,
    control: Arc<DummyControl>,
    source: Arc<Mutex<Option<Box<dyn DummySource>>>>,
    open: Option<(DeviceInfo, Option<Edge>)>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for DummyBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DummyBackend").field("options", &self.options).field("running", &self.thread.is_some()).finish()
    }
}

impl DummyBackend {
    pub fn new(options: DummyOptions) -> Self {
        Self {
            options,
            control: Arc::new(DummyControl::default()),
            source: Arc::new(Mutex::new(Some(Box::new(Silence)))),
            open: None,
            thread: None,
        }
    }

    /// The capture comes from `source`
    pub fn with_source(self, source: Box<dyn DummySource>) -> Self {
        if let Ok(mut s) = self.source.lock() {
            *s = Some(source);
        }
        self
    }

    pub fn control(&self) -> Arc<DummyControl> {
        self.control.clone()
    }
}

impl Backend for DummyBackend {
    fn open(&mut self, config: &DeviceConfig) -> Result<DeviceInfo> {
        if self.thread.is_some() {
            return Err(Error::new("io.device.running"));
        }
        let o = self.options;
        let inputs = config.capture.as_ref().map_or(0, |_| config.capture_channels.map_or(o.inputs, |c| c as usize));
        let outputs =
            config.playback.as_ref().map_or(0, |_| config.playback_channels.map_or(o.outputs, |c| c as usize));
        if inputs > o.inputs || outputs > o.outputs || inputs > MAX_CHANNELS || outputs > MAX_CHANNELS {
            return Err(Error::new("io.device.channels_unsupported")
                .arg("inputs", inputs as i64)
                .arg("outputs", outputs as i64));
        }
        if config.rate == 0 || config.period == 0 {
            return Err(Error::new("io.device.bad_config"));
        }
        let device_rate = o.device_rate.unwrap_or(config.rate);
        if device_rate != config.rate && !config.allow_edge_resampler {
            return Err(Error::new("io.device.rate_unsupported")
                .arg("rate", i64::from(config.rate))
                .arg("device_rate", i64::from(device_rate)));
        }
        let period = config.period;
        let edge = if device_rate != config.rate {
            Some(Edge::new(config.rate, device_rate, inputs, outputs, period as usize, false)?)
        } else {
            None
        };
        let scale = |frames: u32| (f64::from(frames) * f64::from(config.rate) / f64::from(device_rate)).round() as u32;
        let edge_latency = edge.as_ref().map_or(0, Edge::latency);
        let max_block = edge.as_ref().map_or(period as usize, Edge::block);
        let periods = config.periods.max(1);
        let info = DeviceInfo {
            name: format!("dummy:{}", config.name()),
            rate: config.rate,
            device_rate,
            period,
            periods,
            buffer: period * periods,
            max_block,
            inputs,
            outputs,
            input_latency: scale(o.reported_latency) + edge_latency / 2,
            output_latency: scale(o.reported_latency + period * periods) + edge_latency - edge_latency / 2,
            edge_resampler: edge.is_some(),
            drift_correction: false,
            linked: inputs > 0 && outputs > 0,
        };
        self.open = Some((info.clone(), edge));
        Ok(info)
    }

    fn start(&mut self, mut callback: CallbackHandle) -> Result<()> {
        if self.thread.is_some() {
            return Err(Error::new("io.device.running"));
        }
        let Some((info, edge)) = self.open.take() else {
            return Err(Error::new("io.device.not_open"));
        };
        let source = self.source.lock().ok().and_then(|mut s| s.take()).unwrap_or_else(|| Box::new(Silence));
        let mut run = Run {
            options: self.options,
            period: info.period as usize,
            device_rate: info.device_rate,
            edge,
            source,
            control: self.control.clone(),
        };
        let slot = self.source.clone();
        let reopen = info.clone();
        let thread = std::thread::Builder::new().name("lm-audio-dummy".to_owned()).spawn(move || {
            lm_base::set_role(Role::Audio);
            let threads = callback.threads();
            threads.run(ThreadKind::Audio, &mut || {
                callback.report(DeviceEvent::Started { realtime: None });
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| run.run(&mut callback, &reopen)));
                if let Err(payload) = result {
                    lm_render::note_caught_panic(&*payload);
                    callback.report(DeviceEvent::Panicked);
                }
            });
            if let Ok(mut s) = slot.lock() {
                *s = Some(std::mem::replace(&mut run.source, Box::new(Silence)));
            }
            // Dropping the handle hands the callback back: the barrier
            drop(callback);
        });
        match thread {
            Ok(t) => {
                self.thread = Some(t);
                Ok(())
            }
            Err(e) => Err(Error::new("io.device.thread_failed").arg("detail", e.to_string())),
        }
    }

    fn stop(&mut self) {}

    fn join(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn close(&mut self) {
        self.open = None;
    }

    fn kind(&self) -> &'static str {
        "dummy"
    }
}

impl Run {
    fn run(&mut self, handle: &mut CallbackHandle, info: &DeviceInfo) {
        let period = self.period;
        let inputs_n = info.inputs;
        let outputs_n = info.outputs;
        let mut inputs = vec![vec![0.0f32; period]; inputs_n];
        let mut outputs = vec![vec![0.0f32; period]; outputs_n];
        // The loopback's delay line: a ring of `latency + period` frames per channel
        let loop_len = self.options.loopback.map(|l| (l as usize).max(period) + period);
        let mut delay = loop_len.map(|len| vec![vec![0.0f32; len]; inputs_n.min(outputs_n)]).unwrap_or_default();
        let mut delay_pos = 0usize;
        let started = Instant::now();
        let start_time = match self.options.pace {
            Pace::RealTime => HostTime::now(),
            Pace::Free { .. } => HostTime(1_000_000_000),
        };
        let mut frames: u64 = 0;
        loop {
            if handle.should_stop() {
                break;
            }
            if self.control.disconnect.swap(false, Ordering::AcqRel) {
                handle.report(DeviceEvent::Disconnected);
                break;
            }
            let lost = self.control.capture_lost.swap(0, Ordering::AcqRel);
            let late = self.control.late_by.swap(0, Ordering::AcqRel);
            frames += u64::from(lost);
            let time = HostTime(start_time.0 + frames * 1_000_000_000 / u64::from(self.device_rate));
            match self.options.pace {
                Pace::RealTime => sleep_until(started, time.0 - start_time.0),
                Pace::Free { speed: Some(speed) } if speed > 0.0 => {
                    let real = ((time.0 - start_time.0) as f64 / speed) as u64;
                    sleep_until(started, real);
                }
                Pace::Free { .. } => {}
            }
            if handle.should_stop() {
                break;
            }
            if lost > 0 || late > 0 {
                handle.report(DeviceEvent::Xrun { capture_lost: lost, late_by: late });
            }
            self.source.fill(frames, time, &mut inputs, period);
            if let Some(len) = loop_len {
                // A block's output can come back at the earliest one period later
                let lat = (self.options.loopback.unwrap_or(0) as usize).max(period);
                for (input, line) in inputs.iter_mut().zip(delay.iter()) {
                    for (f, x) in input[..period].iter_mut().enumerate() {
                        // What was played `lat` frames before this frame
                        *x += line[(delay_pos + f + len - lat) % len];
                    }
                }
            }
            match self.edge.as_mut() {
                Some(edge) => {
                    if lost > 0 {
                        edge.add_capture_lost(lost);
                    }
                    if late > 0 {
                        edge.add_late(late);
                    }
                    let mut in_refs: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                    for (r, c) in in_refs.iter_mut().zip(&inputs) {
                        *r = &c[..];
                    }
                    edge.push_capture(&in_refs[..inputs_n], period);
                    let mut out_refs: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
                    for (r, c) in out_refs.iter_mut().zip(outputs.iter_mut()) {
                        *r = &mut c[..];
                    }
                    edge.period(time, &mut out_refs[..outputs_n], period, &mut |b| handle.call(b));
                }
                None => {
                    let mut in_refs: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                    for (r, c) in in_refs.iter_mut().zip(&inputs) {
                        *r = &c[..];
                    }
                    let mut out_refs: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
                    for (r, c) in out_refs.iter_mut().zip(outputs.iter_mut()) {
                        *r = &mut c[..];
                    }
                    handle.call(DeviceBlock {
                        frames: period,
                        host_time: time,
                        inputs: &in_refs[..inputs_n],
                        outputs: &mut out_refs[..outputs_n],
                        capture_lost: lost,
                        late_by: late,
                    });
                }
            }
            if let Some(len) = loop_len {
                for (c, line) in delay.iter_mut().enumerate() {
                    for f in 0..period {
                        line[(delay_pos + f) % len] = outputs[c][f];
                    }
                }
                delay_pos = (delay_pos + period) % len;
            }
            frames += period as u64;
            self.control.frames.store(frames, Ordering::Release);
            self.control.periods.fetch_add(1, Ordering::AcqRel);
        }
    }
}

fn sleep_until(started: Instant, nanos: u64) {
    let deadline = started + Duration::from_nanos(nanos);
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(deadline - now);
    }
}
