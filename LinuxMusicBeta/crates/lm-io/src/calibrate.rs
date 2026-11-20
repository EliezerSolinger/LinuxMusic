//! Latency calibration (interfaces.md, I-10.4, H19): an `Operation` on the target `Device`. The device plays a short
//! broadband burst five times and records what comes back (a loopback cable, or the speaker into the microphone);
//! each round trip is the lag of the cross-correlation peak between the burst and the capture; the median of the
//! five is kept. What is stored is **the residual**: the measure minus the latency the device reports, per device,
//! per rate and per edge resampler on or off (H19), so it stays valid when the period changes.
//!
//! A noisy room (a fan) is broadband but does not correlate with the burst: the peak stands out of it. A measure
//! whose peak does not stand clearly above the correlation's own noise is refused (`io.calibration.no_signal`), and
//! the five must agree within a millisecond (`io.calibration.unstable`).

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_render::{DeviceBlock, RenderThreads};

use crate::backend::{Backend, CallbackGuard, CallbackHandle, DeviceCallback, DeviceConfig};

/// The burst's length in frames
pub const BURST: usize = 2048;

/// How to measure
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalibrationOptions {
    pub output_channel: usize,
    pub input_channel: usize,
    /// Measures taken; the median is kept (five, I-10.4)
    pub measures: usize,
    /// The burst's peak level (linear)
    pub level: f32,
    /// Time between bursts (the room's echo must die out)
    pub spacing: Duration,
    /// The longest round trip searched
    pub max_latency: Duration,
    /// How far the correlation peak must stand above the correlation's RMS
    pub min_snr: f32,
}

impl Default for CalibrationOptions {
    fn default() -> Self {
        Self {
            output_channel: 0,
            input_channel: 0,
            measures: 5,
            level: 0.5,
            spacing: Duration::from_millis(700),
            max_latency: Duration::from_millis(500),
            min_snr: 8.0,
        }
    }
}

/// Where a calibration belongs (H19)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CalibrationKey {
    pub device: String,
    pub rate: u32,
    pub edge_resampler: bool,
}

/// The outcome of a calibration
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    pub key: CalibrationKey,
    /// Each round trip measured, in project frames
    pub measures: Vec<i64>,
    /// Their median
    pub round_trip: i64,
    /// What the device reports (input + output latency)
    pub reported: i64,
    /// `round_trip - reported`: what is stored (H19)
    pub residual: i64,
    /// The weakest measure's peak over the correlation's RMS
    pub snr: f32,
}

/// The burst: deterministic white noise under a Hann window
pub fn burst(level: f32) -> Vec<f32> {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    (0..BURST)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let noise = ((x >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0;
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (BURST - 1) as f32).cos();
            noise * w * level
        })
        .collect()
}

/// The device callback of a measurement: plays the bursts, records the input
struct Measure {
    burst: Vec<f32>,
    starts: Vec<u64>,
    capture: Vec<f32>,
    frame: u64,
    out_ch: usize,
    in_ch: usize,
    done: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
}

impl DeviceCallback for Measure {
    fn process(&mut self, block: DeviceBlock<'_, '_>) {
        let DeviceBlock { frames, inputs, outputs, .. } = block;
        for (c, out) in outputs.iter_mut().enumerate() {
            for (k, x) in out[..frames].iter_mut().enumerate() {
                *x = 0.0;
                if c == self.out_ch {
                    let f = self.frame + k as u64;
                    for s in &self.starts {
                        if f >= *s && f < *s + BURST as u64 {
                            *x = self.burst[(f - s) as usize];
                        }
                    }
                }
            }
        }
        if let Some(input) = inputs.get(self.in_ch) {
            for (k, x) in input[..frames].iter().enumerate() {
                let f = (self.frame + k as u64) as usize;
                if f < self.capture.len() {
                    self.capture[f] = *x;
                }
            }
        }
        self.frame += frames as u64;
        self.progress.store(self.frame, Ordering::Relaxed);
        if self.frame as usize >= self.capture.len() {
            self.done.store(true, Ordering::Release);
        }
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

/// The lag (in frames, from 0 to `max_lag`) at which `burst` best matches `capture[from..]`, and how far the peak
/// stands above the correlation's RMS
pub fn find_lag(capture: &[f32], from: usize, burst: &[f32], max_lag: usize) -> (usize, f32) {
    let n = burst.len();
    let mut corr = Vec::with_capacity(max_lag + 1);
    for lag in 0..=max_lag {
        let start = from + lag;
        if start + n > capture.len() {
            break;
        }
        let window = &capture[start..start + n];
        let mut acc = 0.0f64;
        for (a, b) in window.iter().zip(burst) {
            acc += f64::from(*a) * f64::from(*b);
        }
        corr.push(acc);
    }
    if corr.is_empty() {
        return (0, 0.0);
    }
    let (best, peak) = corr.iter().enumerate().fold((0, 0.0f64), |(bi, bv), (i, v)| if v.abs() > bv { (i, v.abs()) } else { (bi, bv) });
    // The noise floor: the correlation away from the peak (a burst wide on each side excluded)
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for (i, v) in corr.iter().enumerate() {
        if i + n < best || i > best + n {
            sum += v * v;
            count += 1;
        }
    }
    let rms = if count > 0 { (sum / count as f64).sqrt() } else { 0.0 };
    let snr = if rms > 0.0 { (peak / rms) as f32 } else if peak > 0.0 { f32::INFINITY } else { 0.0 };
    (best, snr)
}

/// Measures the round-trip latency of a device (I-10.4), inside an `Operation` on the target `Device`. The device
/// must not be running: the session stops it (the barrier) and gives its backend; the backend comes back closed.
/// `config` is the device's configuration
pub fn calibrate(
    mut backend: Box<dyn Backend>,
    config: &DeviceConfig,
    options: CalibrationOptions,
    threads: RenderThreads,
    cancel: &Cancel,
    progress: &Progress,
) -> (Result<Calibration>, Box<dyn Backend>) {
    let result = run(&mut *backend, config, options, threads, cancel, progress);
    backend.close();
    (result, backend)
}

fn run(
    backend: &mut dyn Backend,
    config: &DeviceConfig,
    o: CalibrationOptions,
    threads: RenderThreads,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Calibration> {
    progress.stage(Label::new("io.calibration.measuring"));
    let info = backend.open(config)?;
    if o.output_channel >= info.outputs || o.input_channel >= info.inputs {
        return Err(Error::new("io.calibration.no_channel")
            .arg("output", o.output_channel as i64)
            .arg("input", o.input_channel as i64));
    }
    let rate = f64::from(info.rate);
    let spacing = (o.spacing.as_secs_f64() * rate) as u64;
    let max_lag = (o.max_latency.as_secs_f64() * rate) as usize;
    // Half a second of settling before the first burst (the device's start, filters filling)
    let first = (0.5 * rate) as u64;
    let starts: Vec<u64> = (0..o.measures.max(1) as u64).map(|k| first + k * spacing).collect();
    let total = starts.last().copied().unwrap_or(0) as usize + BURST + max_lag + info.max_block * 4;
    let done = Arc::new(AtomicBool::new(false));
    let frames = Arc::new(AtomicU64::new(0));
    let signal = burst(o.level);
    let measure = Measure {
        burst: signal.clone(),
        starts: starts.clone(),
        capture: vec![0.0; total],
        frame: 0,
        out_ch: o.output_channel,
        in_ch: o.input_channel,
        done: done.clone(),
        progress: frames.clone(),
    };
    let guard = Arc::new(CallbackGuard::default());
    let (tx, rx) = mpsc::sync_channel(1);
    let (handle, _events) = CallbackHandle::new(Box::new(measure), guard.clone(), tx, threads);
    backend.start(handle)?;
    let timeout = Duration::from_secs_f64(total as f64 / rate * 3.0 + 5.0);
    let started = std::time::Instant::now();
    let mut cancelled = false;
    while !done.load(Ordering::Acquire) {
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        if started.elapsed() > timeout {
            break;
        }
        progress.set(frames.load(Ordering::Relaxed) as f32 / total as f32);
        std::thread::sleep(Duration::from_millis(10));
    }
    guard.request_stop();
    backend.stop();
    let callback = rx.recv_timeout(crate::STOP_TIMEOUT).map_err(|_| Error::new("io.device.stop_timed_out"))?;
    backend.join();
    if cancelled {
        return Err(Error::cancelled());
    }
    let Ok(m) = callback.into_any().downcast::<Measure>() else {
        return Err(Error::new("io.calibration.failed"));
    };
    if !done.load(Ordering::Acquire) {
        return Err(Error::new("io.calibration.device_stalled").arg("frames", m.frame as i64));
    }
    progress.stage(Label::new("io.calibration.analysing"));
    let mut measures = Vec::with_capacity(starts.len());
    let mut snr_min = f32::INFINITY;
    for s in &starts {
        cancel.check()?;
        let (lag, snr) = find_lag(&m.capture, *s as usize, &signal, max_lag);
        if snr < o.min_snr {
            return Err(Error::new("io.calibration.no_signal").arg("snr", f64::from(snr)));
        }
        snr_min = snr_min.min(snr);
        measures.push(lag as i64);
    }
    let mut sorted = measures.clone();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let spread = sorted[sorted.len() - 1] - sorted[0];
    if spread as f64 > rate / 1000.0 {
        return Err(Error::new("io.calibration.unstable").arg("spread", spread));
    }
    let reported = i64::from(info.reported_round_trip());
    progress.set(1.0);
    Ok(Calibration {
        key: CalibrationKey { device: config.name(), rate: info.rate, edge_resampler: info.edge_resampler },
        measures,
        round_trip: median,
        reported,
        residual: median - reported,
        snr: snr_min,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lag_is_found_under_broadband_noise() {
        let b = burst(0.3);
        let mut capture = vec![0.0f32; 20_000];
        let mut x: u32 = 12345;
        for s in capture.iter_mut() {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *s = ((x >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.3;
        }
        for (k, v) in b.iter().enumerate() {
            capture[1000 + 3333 + k] += v * 0.2;
        }
        let (lag, snr) = find_lag(&capture, 1000, &b, 10_000);
        assert_eq!(lag, 3333);
        assert!(snr > 8.0, "snr {snr}");
    }
}
