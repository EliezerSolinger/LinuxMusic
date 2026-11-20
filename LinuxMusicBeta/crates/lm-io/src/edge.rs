//! The device edge (arquitetura-nova.md, item 8; interfaces.md I-10.3 step 3; U7): what sits between a device period
//! and the renderer when they do not line up one to one.
//!
//! - **Edge resampler:** the device runs at another rate than the project (it cannot do the project's): one
//!   streaming converter on the capture and one on the playback (`lm-sys-samplerate`, best quality), with FIFOs so
//!   the renderer always gets whole blocks of `block` project frames.
//! - **Drift correction** (only when the user turned it on, U7): capture and playback are on different cards, with
//!   different clocks. The playback card is the clock; the capture is read as it comes and converted by an
//!   **adaptive** resampler whose ratio a PI controller keeps steering so the input FIFO stays at its target. It
//!   adds the FIFO's latency, which the device reports.
//!
//! Everything is allocated in [`Edge::new`]; [`Edge::period`] neither allocates nor locks (the resamplers' process
//! calls do not either).

use lm_base::{Error, HostTime, Result};
use lm_render::DeviceBlock;
use lm_sys_samplerate::{Quality, Resampler};

/// The most channels a device may have on one side (the fixed arrays of slices given to the renderer)
pub const MAX_CHANNELS: usize = 64;

/// A first-in first-out buffer of interleaved frames, fixed capacity
#[derive(Debug)]
struct Fifo {
    data: Vec<f32>,
    start: usize,
    end: usize,
    channels: usize,
}

impl Fifo {
    fn new(channels: usize, frames: usize) -> Self {
        Self { data: vec![0.0; channels.max(1) * frames], start: 0, end: 0, channels: channels.max(1) }
    }

    fn frames(&self) -> usize {
        (self.end - self.start) / self.channels
    }

    fn room(&self) -> usize {
        (self.data.len() - (self.end - self.start)) / self.channels
    }

    /// Room at the end for `frames`, moving what is held to the front if needed; returns the writable slice
    fn tail(&mut self, frames: usize) -> &mut [f32] {
        let need = frames * self.channels;
        if self.end + need > self.data.len() && self.start > 0 {
            self.data.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        let avail = (self.data.len() - self.end).min(need);
        &mut self.data[self.end..self.end + avail]
    }

    fn commit(&mut self, frames: usize) {
        self.end = (self.end + frames * self.channels).min(self.data.len());
    }

    fn push_silence(&mut self, frames: usize) {
        let n = frames.min(self.room());
        self.tail(n).fill(0.0);
        self.commit(n);
    }

    fn head(&self) -> &[f32] {
        &self.data[self.start..self.end]
    }

    fn consume(&mut self, frames: usize) {
        self.start = (self.start + frames * self.channels).min(self.end);
        if self.start == self.end {
            self.start = 0;
            self.end = 0;
        }
    }
}

/// The PI controller of the drift correction: steers the capture resampler's ratio so the input FIFO stays at
/// `target` frames
#[derive(Debug)]
struct Drift {
    target: f64,
    level: f64,
    integral: f64,
    nominal: f64,
}

/// Largest correction of the ratio (2000 ppm: far beyond any two crystals, small enough not to be heard)
const DRIFT_LIMIT: f64 = 2.0e-3;
const DRIFT_KP: f64 = 4.0e-6;
const DRIFT_KI: f64 = 4.0e-7;
/// The level's smoothing per period (the FIFO level jumps by a period at each render)
const DRIFT_SMOOTH: f64 = 0.02;

impl Drift {
    /// The ratio for the next capture conversion, from the FIFO level now; `dt` is the period's duration (s)
    fn ratio(&mut self, level: usize, dt: f64) -> f64 {
        self.level += (level as f64 - self.level) * DRIFT_SMOOTH;
        let err = self.level - self.target;
        self.integral = (self.integral + err * dt).clamp(-DRIFT_LIMIT / DRIFT_KI, DRIFT_LIMIT / DRIFT_KI);
        let correction = (err * DRIFT_KP + self.integral * DRIFT_KI).clamp(-DRIFT_LIMIT, DRIFT_LIMIT);
        // Too many frames waiting: produce fewer per input frame
        self.nominal * (1.0 - correction)
    }
}

/// Statistics of the edge, for tests and the device view
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeStats {
    /// Renders that found the input FIFO short (silence was given): never in steady state
    pub input_underruns: u64,
    /// Capture frames that found the input FIFO full (dropped): never in steady state
    pub input_overflows: u64,
    /// The input FIFO's level now, in project frames
    pub input_level: usize,
    /// The capture ratio now (drift correction), output/input
    pub capture_ratio: f64,
}

/// The adapter between a device period and the renderer's blocks
#[derive(Debug)]
pub struct Edge {
    rate: u32,
    device_rate: u32,
    inputs: usize,
    outputs: usize,
    block: usize,
    capture: Option<Resampler>,
    playback: Option<Resampler>,
    in_fifo: Fifo,
    out_fifo: Fifo,
    in_planar: Vec<Vec<f32>>,
    out_planar: Vec<Vec<f32>>,
    dev_scratch: Vec<f32>,
    rs_scratch: Vec<f32>,
    drift: Option<Drift>,
    lost: u64,
    lost_frac: f64,
    late: u32,
    stats: EdgeStats,
    latency: u32,
}

impl Edge {
    /// An adapter for a device at `device_rate` with periods of up to `device_period` frames, feeding a renderer at
    /// `rate` in blocks of `block` frames. `drift` turns on the adaptive capture resampler (two cards, U7). Allocates
    pub fn new(
        rate: u32,
        device_rate: u32,
        inputs: usize,
        outputs: usize,
        device_period: usize,
        drift: bool,
    ) -> Result<Self> {
        if inputs > MAX_CHANNELS || outputs > MAX_CHANNELS {
            return Err(Error::new("io.device.too_many_channels").arg("max", MAX_CHANNELS as i64));
        }
        if rate == 0 || device_rate == 0 || device_period == 0 {
            return Err(Error::new("io.device.bad_config"));
        }
        let ratio_in = f64::from(rate) / f64::from(device_rate);
        let ratio_out = f64::from(device_rate) / f64::from(rate);
        // The project block that covers a device period
        let block = (device_period as f64 * ratio_in).ceil() as usize + 1;
        let resample = rate != device_rate;
        let capture = if inputs > 0 && (resample || drift) {
            Some(Resampler::new(Quality::Best, inputs, ratio_in)?)
        } else {
            None
        };
        let playback = if outputs > 0 && resample { Some(Resampler::new(Quality::Best, outputs, ratio_out)?) } else { None };
        let cap_latency = capture.as_ref().map_or(0.0, Resampler::latency);
        let play_latency = playback.as_ref().map_or(0.0, |r| r.latency() * ratio_in);
        // The input FIFO keeps one block plus a period of margin (and with drift, a second period for the clock
        // difference to move in)
        // and what the two filters hold back at the start (the playback converter gives fewer frames until its
        // filter is full, so the first renders take more input than a period brings)
        let filters = (cap_latency + play_latency).ceil() as usize;
        let margin = (device_period as f64 * ratio_in).ceil() as usize + filters + if drift { block } else { 0 };
        let prefill = block + margin;
        let fifo_frames = 4 * (block + margin) + 64;
        let mut in_fifo = Fifo::new(inputs, fifo_frames);
        if inputs > 0 {
            in_fifo.push_silence(prefill);
        }
        let out_fifo = Fifo::new(outputs, (block as f64 * ratio_out).ceil() as usize * 4 + device_period * 2 + 64);
        let drift = drift.then(|| Drift {
            target: prefill as f64,
            level: prefill as f64,
            integral: 0.0,
            nominal: ratio_in,
        });
        let scratch_frames = (block as f64 * ratio_out.max(1.0)).ceil() as usize * 2 + device_period * 4 + 64;
        let max_ch = inputs.max(outputs).max(1);
        Ok(Self {
            rate,
            device_rate,
            inputs,
            outputs,
            block,
            capture,
            playback,
            in_fifo,
            out_fifo,
            in_planar: vec![vec![0.0; block]; inputs],
            out_planar: vec![vec![0.0; block]; outputs],
            dev_scratch: vec![0.0; (device_period * 4).max(block * 2) * max_ch],
            rs_scratch: vec![0.0; scratch_frames * max_ch],
            drift,
            lost: 0,
            lost_frac: 0.0,
            late: 0,
            stats: EdgeStats { capture_ratio: ratio_in, ..EdgeStats::default() },
            latency: (prefill as f64 + cap_latency + play_latency).round() as u32,
        })
    }

    /// The renderer's block size here, the engine's `max_block` (C7)
    pub fn block(&self) -> usize {
        self.block
    }

    /// The latency the edge adds (FIFO and filters), in project frames
    pub fn latency(&self) -> u32 {
        self.latency
    }

    pub fn stats(&self) -> EdgeStats {
        EdgeStats { input_level: self.in_fifo.frames(), ..self.stats }
    }

    /// The capture xrun's lost device frames, given to the next render in project frames (C2)
    pub fn add_capture_lost(&mut self, device_frames: u32) {
        let exact = f64::from(device_frames) * f64::from(self.rate) / f64::from(self.device_rate) + self.lost_frac;
        let whole = exact.floor();
        self.lost_frac = exact - whole;
        self.lost += whole as u64;
    }

    pub fn add_late(&mut self, device_frames: u32) {
        let frames = (f64::from(device_frames) * f64::from(self.rate) / f64::from(self.device_rate)).round() as u32;
        self.late = self.late.saturating_add(frames.max(1));
    }

    /// Converts `frames` of capture (one slice per device channel) into the input FIFO
    pub fn push_capture(&mut self, input: &[&[f32]], frames: usize) {
        if self.inputs == 0 || frames == 0 {
            return;
        }
        let ch = self.inputs;
        let mut done = 0;
        let chunk_max = self.dev_scratch.len() / ch;
        while done < frames {
            let n = (frames - done).min(chunk_max);
            for f in 0..n {
                for c in 0..ch {
                    self.dev_scratch[f * ch + c] = input.get(c).and_then(|s| s.get(done + f)).copied().unwrap_or(0.0);
                }
            }
            if self.capture.is_some() {
                if let Some(drift) = self.drift.as_mut() {
                    let dt = n as f64 / f64::from(self.device_rate);
                    let ratio = drift.ratio(self.in_fifo.frames(), dt);
                    self.stats.capture_ratio = ratio;
                    if let Some(rs) = self.capture.as_mut() {
                        let _ = rs.set_ratio(ratio);
                    }
                }
                let mut consumed = 0;
                while consumed < n {
                    let Some(rs) = self.capture.as_mut() else { break };
                    let room = self.rs_scratch.len() / ch;
                    let input = &self.dev_scratch[consumed * ch..n * ch];
                    let Ok(p) = rs.process(input, &mut self.rs_scratch[..room * ch]) else { break };
                    let out = &self.rs_scratch[..p.output_frames * ch];
                    let fit = p.output_frames.min(self.in_fifo.room());
                    self.in_fifo.tail(fit).copy_from_slice(&out[..fit * ch]);
                    self.in_fifo.commit(fit);
                    self.stats.input_overflows += (p.output_frames - fit) as u64;
                    consumed += p.input_frames;
                    if p.input_frames == 0 && p.output_frames == 0 {
                        break;
                    }
                }
            } else {
                let fit = n.min(self.in_fifo.room());
                self.in_fifo.tail(fit).copy_from_slice(&self.dev_scratch[..fit * ch]);
                self.in_fifo.commit(fit);
                self.stats.input_overflows += (n - fit) as u64;
            }
            done += n;
        }
    }

    /// One render of `block` frames from the input FIFO, its output converted into the output FIFO
    fn render_one(&mut self, host_time: HostTime, call: &mut dyn FnMut(DeviceBlock<'_, '_>)) {
        let block = self.block;
        let ch_in = self.inputs;
        if ch_in > 0 {
            let have = self.in_fifo.frames().min(block);
            if have < block {
                self.stats.input_underruns += 1;
            }
            let head = self.in_fifo.head();
            for (c, plane) in self.in_planar.iter_mut().enumerate() {
                for (f, x) in plane.iter_mut().enumerate() {
                    *x = if f < have { head[f * ch_in + c] } else { 0.0 };
                }
            }
            self.in_fifo.consume(have);
        }
        {
            let mut in_refs: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
            for (r, p) in in_refs.iter_mut().zip(&self.in_planar) {
                *r = &p[..];
            }
            let mut out_refs: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| &mut [][..]);
            for (r, p) in out_refs.iter_mut().zip(self.out_planar.iter_mut()) {
                *r = &mut p[..];
            }
            let capture_lost = u32::try_from(std::mem::take(&mut self.lost)).unwrap_or(u32::MAX);
            let late_by = std::mem::take(&mut self.late);
            call(DeviceBlock {
                frames: block,
                host_time,
                inputs: &in_refs[..self.inputs],
                outputs: &mut out_refs[..self.outputs],
                capture_lost,
                late_by,
            });
        }
        let ch = self.outputs;
        if ch == 0 {
            return;
        }
        let scratch = &mut self.dev_scratch;
        // Interleave the block (the device scratch is large enough for a block: 4 periods)
        let n = block.min(scratch.len() / ch);
        for f in 0..n {
            for c in 0..ch {
                scratch[f * ch + c] = self.out_planar[c][f];
            }
        }
        match self.playback.as_mut() {
            Some(rs) => {
                let mut consumed = 0;
                while consumed < n {
                    let room = self.rs_scratch.len() / ch;
                    let Ok(p) = rs.process(&scratch[consumed * ch..n * ch], &mut self.rs_scratch[..room * ch]) else {
                        break;
                    };
                    let fit = p.output_frames.min(self.out_fifo.room());
                    self.out_fifo.tail(fit).copy_from_slice(&self.rs_scratch[..fit * ch]);
                    self.out_fifo.commit(fit);
                    consumed += p.input_frames;
                    if p.input_frames == 0 && p.output_frames == 0 {
                        break;
                    }
                }
            }
            None => {
                let fit = n.min(self.out_fifo.room());
                self.out_fifo.tail(fit).copy_from_slice(&scratch[..fit * ch]);
                self.out_fifo.commit(fit);
            }
        }
    }

    /// One device period: the capture (already pushed with [`Edge::push_capture`]) feeds renders until the output
    /// FIFO covers the period, which is then written to `output` (one slice per device channel, `frames` long).
    /// `host_time` is the device period's (C6). A capture-only device renders whenever a whole block is in
    pub fn period(
        &mut self,
        host_time: HostTime,
        output: &mut [&mut [f32]],
        frames: usize,
        call: &mut dyn FnMut(DeviceBlock<'_, '_>),
    ) {
        let block_ns = self.block as u64 * 1_000_000_000 / u64::from(self.rate);
        let mut t = HostTime(
            host_time.0.saturating_sub(self.in_fifo.frames() as u64 * 1_000_000_000 / u64::from(self.rate)),
        );
        if self.outputs == 0 {
            while self.in_fifo.frames() >= self.block {
                self.render_one(t, call);
                t = HostTime(t.0 + block_ns);
            }
            return;
        }
        let mut guard = 0;
        while self.out_fifo.frames() < frames && guard < 64 {
            self.render_one(t, call);
            t = HostTime(t.0 + block_ns);
            guard += 1;
        }
        let ch = self.outputs;
        let have = self.out_fifo.frames().min(frames);
        let head = self.out_fifo.head();
        for (c, out) in output.iter_mut().enumerate().take(ch) {
            for f in 0..frames.min(out.len()) {
                out[f] = if f < have { head[f * ch + c] } else { 0.0 };
            }
        }
        self.out_fifo.consume(have);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frames: usize, start: usize, freq: f64, rate: u32) -> Vec<f32> {
        (0..frames).map(|i| (((start + i) as f64 * freq / f64::from(rate)) * std::f64::consts::TAU).sin() as f32 * 0.5).collect()
    }

    /// The renderer sees whole blocks at the project rate; a sine through capture → render (copy) → playback keeps
    /// its frequency and level at the device rate
    #[test]
    fn resampled_edge_round_trips_a_sine() {
        let (rate, dev) = (44_100, 48_000);
        let mut edge = Edge::new(rate, dev, 1, 1, 256, false).expect("edge");
        let block = edge.block();
        let mut out_all = Vec::new();
        let mut pos = 0;
        let mut blocks_seen = 0;
        let mut sizes_ok = true;
        for _ in 0..400 {
            let input = sine(256, pos, 1000.0, dev);
            pos += 256;
            edge.push_capture(&[&input], 256);
            let mut out = vec![0.0f32; 256];
            edge.period(HostTime(0), &mut [&mut out[..]], 256, &mut |b: DeviceBlock<'_, '_>| {
                sizes_ok &= b.frames == block;
                blocks_seen += 1;
                let DeviceBlock { inputs, outputs, frames, .. } = b;
                outputs[0][..frames].copy_from_slice(&inputs[0][..frames]);
            });
            out_all.extend_from_slice(&out);
        }
        assert!(sizes_ok);
        assert!(blocks_seen > 300);
        assert_eq!(edge.stats().input_underruns, 0);
        // Steady part: zero crossings give 1000 Hz at 48 kHz, level ~0.5
        let tail = &out_all[out_all.len() - 48_000..];
        let crossings = tail.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((999..=1001).contains(&crossings), "{crossings} crossings");
        let peak = tail.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
    }

    /// Two clocks 300 ppm apart: the controller holds the FIFO without underruns or overflows
    #[test]
    fn drift_correction_holds_the_fifo() {
        let rate = 48_000;
        let mut edge = Edge::new(rate, rate, 1, 1, 128, true).expect("edge");
        let target = edge.stats().input_level;
        let mut acc = 0.0f64;
        let input = vec![0.25f32; 256];
        let mut out = vec![0.0f32; 128];
        let mut levels = Vec::new();
        for i in 0..(48_000 * 120 / 128) {
            // The capture card runs 300 ppm fast
            acc += 128.0 * 1.0003;
            let n = acc.floor() as usize;
            acc -= n as f64;
            edge.push_capture(&[&input[..n]], n);
            edge.period(HostTime(0), &mut [&mut out[..]], 128, &mut |b: DeviceBlock<'_, '_>| {
                let DeviceBlock { inputs, outputs, frames, .. } = b;
                outputs[0][..frames].copy_from_slice(&inputs[0][..frames]);
            });
            if i > 48_000 * 60 / 128 {
                levels.push(edge.stats().input_level);
            }
        }
        let s = edge.stats();
        assert_eq!(s.input_underruns, 0);
        assert_eq!(s.input_overflows, 0);
        let max = *levels.iter().max().expect("levels");
        let min = *levels.iter().min().expect("levels");
        assert!(max < target + 3 * 128 && min + 3 * 128 > target, "level {min}..{max}, target {target}");
        assert!((s.capture_ratio - 1.0 / 1.0003).abs() < 1.0e-4, "ratio {}", s.capture_ratio);
    }
}
