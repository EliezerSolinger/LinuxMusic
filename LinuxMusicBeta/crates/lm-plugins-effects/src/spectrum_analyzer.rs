//! Spectrum Analyzer (F-FX-026: legacy `plugins/SpectrumAnalyzer`, Martin Pavelek's): the incoming audio is passed
//! through unchanged and analyzed for the window: the spectrum of each side (or of their average), averaged, with
//! peak hold or a frozen reference, and a waterfall of the last spectra.
//!
//! State, version 1 (the legacy's models, same ranges and defaults; its save tags in parentheses):
//! - switches (parameters of 0 to 1): `pause` and `refFreeze` (not saved by the legacy), `waterfall` (off), `smooth`
//!   (Averaging, off), `stereo` (off), `peakHold` (off), `logX` (on) and `logY` (on);
//! - fields (choices): `freqRange` (RangeX: `full`, `audible`, `bass`, `mids`, `high`; `full`), `ampRange` (RangeY:
//!   `extended`, `audible`, `loud`, `silent`; `audible`), `blockSize` (BlockSize: `"256"` to `"16384"`; `"2048"`) and
//!   `window` (WindowType: `rectangular`, `blackmanHarris`, `hamming`, `hanning`; `blackmanHarris`);
//! - the advanced knobs (parameters): `envelopeRes` (0.1–3, 0.25) and `spectrumRes` (0.1–3, 1.5), the points the
//!   window draws per pixel; `peakDecayFactor` (0.95–0.999, 0.992), `averageWeight` (0.01–0.5, 0.15),
//!   `waterfallHeight` (50–1000 lines, 300), `waterfallGamma` (0.1–1, 0.3), `windowOverlap` (1–4, 2) and
//!   `zeroPadding` (0–4, 2: the FFT is that many sizes bigger than the block).
//!
//! The legacy's thread: the audio thread only copies the frames into a lock-free ring (and only while the window is
//! open and not paused), a worker computes. Here the worker is made with the processor (a change of a field builds
//! another one: the analysis starts again, as the legacy's reallocation does) and reads the knobs the audio side
//! writes into atomics; it allocates what it needs when a knob that sizes its buffers changes (it is not the audio
//! thread), and hands each result to the audio side through a triple buffer, from which the window's frame is
//! copied. The audio thread allocates, locks and waits for nothing. The worker ends when the processor is dropped.
//!
//! What the legacy does in its view at each paint (the display buffers: the exponential average, the peak hold's
//! decay, the reference freeze) the worker does at each FFT, and the bins are reduced to the window's columns (the
//! highest bin of each column, as the legacy's path points) so that the frame has a fixed size.
//!
//! The window's live frame, little-endian `f32`: [`HEADER`] numbers (1, the project's rate, the FFT's size, its bins,
//! the waterfall's lines in use, 1 when the spectrum has anything to show, 1 when the waterfall has, 1 while a
//! reference is frozen), then four series of [`SPEC_COLS`] normalized amplitudes (1.0 is a full scale sine): the
//! left, the right, the left's peak (or reference) and the right's; then [`WF_ROWS`] lines of [`WF_COLS`] pixels, the
//! newest first, each a whole number `0xRRGGBB` made as the legacy's `makePixel`. Columns span the frequency range
//! on the linear or logarithmic axis the knobs ask for.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use lm_base::{Error, Result};
use lm_dsp::fft::{Complex, RealFft, Window};
use lm_dsp::math::{log10f, powf};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, StateView, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s};

pub const ID: &str = "spectrumanalyzer";

/// Columns of each spectrum series of the frame
pub const SPEC_COLS: usize = 1024;
/// Columns and lines of the waterfall of the frame
pub const WF_COLS: usize = 256;
pub const WF_ROWS: usize = 96;
/// The numbers before the series
pub const HEADER: usize = 8;
/// The numbers of a frame
pub const FRAME_FLOATS: usize = HEADER + 4 * SPEC_COLS + WF_ROWS * WF_COLS;

/// The FFT block sizes (`FFT_BLOCK_SIZES`)
pub const BLOCK_SIZES: [usize; 7] = [256, 512, 1024, 2048, 4096, 8192, 16384];
const BLOCK_KEYS: [&str; 7] = ["256", "512", "1024", "2048", "4096", "8192", "16384"];
const FREQ_KEYS: [&str; 5] = ["full", "audible", "bass", "mids", "high"];
const AMP_KEYS: [&str; 4] = ["extended", "audible", "loud", "silent"];
/// The lowest frequency of the logarithmic axis of the full range (`LOWEST_LOG_FREQ`)
pub const LOWEST_LOG_FREQ: f32 = 5.0;

/// The frames the ring holds: a bit over a second at 48 kHz (the legacy's is four times the engine's block)
const RING_FRAMES: usize = 65_536;
/// How long the audio side goes on feeding the worker after the window's last frame was asked for
const WINDOW_GRACE_SECONDS: f32 = 0.5;
/// The worker publishes at most this often
const PUBLISH_EVERY: Duration = Duration::from_millis(8);

const P_PAUSE: usize = 0;
const P_FREEZE: usize = 1;
const P_WATERFALL: usize = 2;
const P_SMOOTH: usize = 3;
const P_STEREO: usize = 4;
const P_PEAK_HOLD: usize = 5;
const P_LOG_X: usize = 6;
const P_DECAY: usize = 10;
const P_WEIGHT: usize = 11;
const P_HEIGHT: usize = 12;
const P_GAMMA: usize = 13;
const P_OVERLAP: usize = 14;
const P_PADDING: usize = 15;
const N_PARAMS: usize = 16;

/// The parameters: key, min, max, default, scale, unit
const PARAMS: [(&str, f64, f64, f64, Scale, &str); N_PARAMS] = [
    ("pause", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("refFreeze", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("waterfall", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("smooth", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("stereo", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("peakHold", 0.0, 1.0, 0.0, Scale::Stepped, ""),
    ("logX", 0.0, 1.0, 1.0, Scale::Stepped, ""),
    ("logY", 0.0, 1.0, 1.0, Scale::Stepped, ""),
    ("envelopeRes", 0.1, 3.0, 0.25, Scale::Linear, ""),
    ("spectrumRes", 0.1, 3.0, 1.5, Scale::Linear, ""),
    ("peakDecayFactor", 0.95, 0.999, 0.992, Scale::Linear, ""),
    ("averageWeight", 0.01, 0.5, 0.15, Scale::Linear, ""),
    ("waterfallHeight", 50.0, 1000.0, 300.0, Scale::Linear, ""),
    ("waterfallGamma", 0.1, 1.0, 0.3, Scale::Linear, ""),
    ("windowOverlap", 1.0, 4.0, 2.0, Scale::Stepped, ""),
    ("zeroPadding", 0.0, 4.0, 2.0, Scale::Stepped, ""),
];
const KEYS: [&str; N_PARAMS] = {
    let mut keys = [""; N_PARAMS];
    let mut i = 0;
    while i < N_PARAMS {
        keys[i] = PARAMS[i].0;
        i += 1;
    }
    keys
};

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.spectrumanalyzer.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: Some(DisplaySpec { bytes: FRAME_FLOATS * 4 }),
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:spectrumanalyzer", 1);
    for (key, min, max, def, scale, unit) in PARAMS {
        let mut p = ParamSpec::new(key, min, max, def).scale(scale);
        if !unit.is_empty() {
            p = p.unit(unit);
        }
        s = s.param(p);
    }
    let window_keys: &'static [&'static str] = &["rectangular", "blackmanHarris", "hamming", "hanning"];
    vec![
        s.field(FieldSpec::choice("freqRange", &FREQ_KEYS, "full"))
            .field(FieldSpec::choice("ampRange", &AMP_KEYS, "audible"))
            .field(FieldSpec::choice("blockSize", &BLOCK_KEYS, "2048"))
            .field(FieldSpec::choice("window", window_keys, "blackmanHarris")),
    ]
}

/// The frequency range's ends, Hz: the legacy's `getFreqRangeMin` and `getFreqRangeMax` (the full range starts at
/// 0 on the linear axis)
pub fn freq_range(index: usize, log: bool, nyquist: f32) -> (f32, f32) {
    match index {
        1 => (20.0, 20_000.0),
        2 => (20.0, 300.0),
        3 => (200.0, 5_000.0),
        4 => (4_000.0, 20_000.0),
        _ => (if log { LOWEST_LOG_FREQ } else { 0.0 }, nyquist),
    }
}

/// The frequency at `x` (0 to 1) across the axis: the legacy's `xPixelToFreq`
pub fn x_to_freq(x: f32, min: f32, max: f32, log: bool) -> f32 {
    if log {
        let lo = log10f(min.max(1.0e-3));
        let hi = log10f(max);
        powf(10.0, lo + x * (hi - lo))
    } else {
        min + x * (max - min)
    }
}

/// What the worker reads of the knobs, written by the audio side
struct Shared {
    params: [AtomicU32; N_PARAMS],
    stop: AtomicBool,
    /// Bumped by a reset: the worker forgets its sound
    clears: AtomicU32,
}

impl Shared {
    fn set(&self, index: usize, value: f32) {
        if let Some(p) = self.params.get(index) {
            p.store(value.to_bits(), Ordering::Relaxed);
        }
    }

    fn get(&self, index: usize) -> f32 {
        f32::from_bits(self.params[index].load(Ordering::Relaxed))
    }

    fn on(&self, index: usize) -> bool {
        self.get(index) >= 0.5
    }
}

/// What the fields fixed when the processor was built
#[derive(Clone, Copy)]
struct Config {
    rate: f32,
    freq_range: usize,
    block_index: usize,
    window: Window,
}

// ---------------------------------------------------------------------------------------------------------------
// The audio side

struct Analyze {
    knobs: Knobs<N_PARAMS>,
    shared: Arc<Shared>,
    /// The first pair feeds the worker and shows; the others only pass the audio
    ring: Option<rtrb::Producer<[f32; 2]>>,
    out: Option<triple_buffer::Output<Box<[f32]>>>,
    rate: f32,
    /// Frames since the window last asked for a frame
    idle: usize,
}

impl Drop for Analyze {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

impl Effect for Analyze {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
        self.shared.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let Some(ring) = self.ring.as_mut() else { return };
        // As the legacy: nothing is copied while the window is closed or the analyzer paused
        if self.knobs.on(P_PAUSE) || self.idle as f32 > self.rate * WINDOW_GRACE_SECONDS {
            self.idle = self.idle.saturating_add(frames.len());
            return;
        }
        self.idle += frames.len();
        for f in frames.iter() {
            if ring.push(*f).is_err() {
                // The worker cannot keep up: the rest of the block is lost (it shows as a red line in the waterfall)
                break;
            }
        }
    }

    fn reset(&mut self) {
        self.shared.clears.fetch_add(1, Ordering::Relaxed);
    }

    fn adopt(&mut self, _old: &mut Self) -> bool {
        // Nothing to keep: the worker of the old one ends with it, and the analysis starts again
        true
    }

    fn display(&mut self, frame: &mut [u8]) {
        self.idle = 0;
        match self.out.as_mut() {
            Some(out) => put_f32s(frame, out.read()),
            None => frame.fill(0),
        }
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let view = StateView::new(state);
    let freq_range = view.choice("freqRange", |k| FREQ_KEYS.iter().position(|x| *x == k))?;
    let block_index = view.choice("blockSize", |k| BLOCK_KEYS.iter().position(|x| *x == k))?;
    let window = view.choice("window", Window::from_key)?;
    let rate = cx.spec.sample_rate as f32;
    let config = Config { rate, freq_range, block_index, window };
    let mut first = true;
    EffectProcessor::build(cx, || {
        let shared = Arc::new(Shared {
            params: std::array::from_fn(|i| AtomicU32::new(knobs.get(i).to_bits())),
            stop: AtomicBool::new(false),
            clears: AtomicU32::new(0),
        });
        if !std::mem::take(&mut first) {
            return Ok(Analyze { knobs, shared, ring: None, out: None, rate, idle: 0 });
        }
        let (producer, consumer) = rtrb::RingBuffer::new(RING_FRAMES);
        let (input, output) = triple_buffer::triple_buffer(&vec![0.0f32; FRAME_FLOATS].into_boxed_slice());
        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("lm-spectrum-analyzer".to_owned())
            .spawn(move || Worker::new(config, worker_shared, consumer, input).run())
            .map_err(|e| Error::new("plugin.worker.spawn").arg("reason", e.to_string()))?;
        Ok(Analyze { knobs, shared, ring: Some(producer), out: Some(output), rate, idle: usize::MAX / 2 })
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The worker: the legacy's `SaProcessor::analyze`

/// The legacy's waterfall colors
const COLOR_L: [f32; 3] = [51.0, 148.0, 204.0];
const COLOR_R: [f32; 3] = [204.0, 107.0, 51.0];
const COLOR_MONO_W: [f32; 3] = [64.0, 185.0, 255.0];

/// What a change in the knobs asks the worker to size again
#[derive(Clone, Copy, PartialEq, Eq)]
struct Sizes {
    padding: usize,
    height: usize,
}

struct Worker {
    config: Config,
    shared: Arc<Shared>,
    rx: rtrb::Consumer<[f32; 2]>,
    tx: triple_buffer::Input<Box<[f32]>>,

    sizes: Sizes,
    in_size: usize,
    fft_size: usize,
    bins: usize,
    fft: RealFft,
    window: Vec<f32>,
    /// Time domain samples of each side, `in_size` long
    buf: [Vec<f32>; 2],
    filled: usize,
    /// The windowed block, `fft_size` long (the tail stays zero)
    time: Vec<f32>,
    spectrum: Vec<Complex>,
    /// The normalized spectrum of each side
    norm: [Vec<f32>; 2],
    /// The averaged spectrum and the peak or reference of each side
    display: [Vec<f32>; 2],
    peak: [Vec<f32>; 2],
    /// A reference freeze was asked for and is not taken yet
    freeze_request: bool,
    frozen: bool,
    /// The waterfall: `sizes.height` lines of [`WF_COLS`] pixels, `head` the newest
    history: Vec<u32>,
    head: usize,
    wf_remaining: usize,
    block_nonempty: bool,
    /// The bins of each column of the spectrum, and of the waterfall, for the axis they were made for
    map: AxisMap,
    clears_seen: u32,
    dirty: bool,
    last_publish: Instant,
}

/// The bin span of each column
#[derive(Default)]
struct AxisMap {
    key: Option<(bool, usize)>,
    /// The first and last bin of each spectrum column
    spec: Vec<(usize, usize)>,
    /// The bin positions (bin `k` spans `k..k + 1`) each waterfall column covers
    wf: Vec<(f32, f32)>,
}

impl Worker {
    fn new(
        config: Config,
        shared: Arc<Shared>,
        rx: rtrb::Consumer<[f32; 2]>,
        tx: triple_buffer::Input<Box<[f32]>>,
    ) -> Self {
        let sizes = Sizes { padding: shared.get(P_PADDING) as usize, height: shared.get(P_HEIGHT) as usize };
        let (in_size, fft_size) = Self::block_sizes(&config, sizes.padding);
        let bins = fft_size / 2 + 1;
        let mut w = Self {
            config,
            clears_seen: shared.clears.load(Ordering::Relaxed),
            shared,
            rx,
            tx,
            sizes,
            in_size,
            fft_size,
            bins,
            fft: RealFft::new(fft_size),
            window: config.window.coefficients(in_size),
            buf: [vec![0.0; in_size], vec![0.0; in_size]],
            filled: 0,
            time: vec![0.0; fft_size],
            spectrum: vec![Complex::ZERO; bins],
            norm: [vec![0.0; bins], vec![0.0; bins]],
            display: [vec![0.0; bins], vec![0.0; bins]],
            peak: [vec![0.0; bins], vec![0.0; bins]],
            freeze_request: false,
            frozen: false,
            history: vec![0; sizes.height * WF_COLS],
            head: 0,
            wf_remaining: 0,
            block_nonempty: false,
            map: AxisMap::default(),
            dirty: true,
            last_publish: Instant::now(),
        };
        w.clear();
        w
    }

    /// The block's and the FFT's sizes for a zero padding (the legacy's `reallocateBuffers`)
    fn block_sizes(config: &Config, padding: usize) -> (usize, usize) {
        let last = BLOCK_SIZES.len() - 1;
        let fft = if config.block_index + padding < BLOCK_SIZES.len() { config.block_index + padding } else { last };
        (BLOCK_SIZES[config.block_index], BLOCK_SIZES[fft])
    }

    fn overlap(&self) -> usize {
        (self.shared.get(P_OVERLAP) as usize).clamp(1, 4)
    }

    /// `SaProcessor::clear`
    fn clear(&mut self) {
        let overlap = self.overlap();
        self.filled = self.in_size - self.in_size / overlap;
        for v in self.buf.iter_mut().chain(self.norm.iter_mut()).chain(self.display.iter_mut()) {
            v.fill(0.0);
        }
        for v in &mut self.peak {
            v.fill(0.0);
        }
        self.time.fill(0.0);
        self.history.fill(0);
        self.wf_remaining = 0;
        self.dirty = true;
    }

    /// `SaProcessor::reallocateBuffers`, for the knobs that size the buffers
    fn resize(&mut self) {
        let sizes = Sizes {
            padding: (self.shared.get(P_PADDING) as usize).min(4),
            height: (self.shared.get(P_HEIGHT) as usize).clamp(50, 1000),
        };
        if sizes == self.sizes {
            return;
        }
        let (in_size, fft_size) = Self::block_sizes(&self.config, sizes.padding);
        self.sizes = sizes;
        self.in_size = in_size;
        self.fft_size = fft_size;
        self.bins = fft_size / 2 + 1;
        self.fft = RealFft::new(fft_size);
        self.window = self.config.window.coefficients(in_size);
        self.buf = [vec![0.0; in_size], vec![0.0; in_size]];
        self.time = vec![0.0; fft_size];
        self.spectrum = vec![Complex::ZERO; self.bins];
        self.norm = [vec![0.0; self.bins], vec![0.0; self.bins]];
        self.display = [vec![0.0; self.bins], vec![0.0; self.bins]];
        self.peak = [vec![0.0; self.bins], vec![0.0; self.bins]];
        self.history = vec![0; sizes.height * WF_COLS];
        self.head = 0;
        self.map = AxisMap::default();
        self.clear();
    }

    fn run(mut self) {
        while !self.shared.stop.load(Ordering::Relaxed) && !self.rx.is_abandoned() {
            self.resize();
            let clears = self.shared.clears.load(Ordering::Relaxed);
            if clears != self.clears_seen {
                self.clears_seen = clears;
                self.clear();
            }
            let waiting = self.rx.slots();
            if waiting == 0 {
                self.publish_if_due();
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            // The legacy's waterfall shows a red line when the analysis cannot keep up
            let overload = waiting > RING_FRAMES / 2;
            for _ in 0..waiting {
                let Ok(frame) = self.rx.pop() else { break };
                self.feed(frame, overload);
            }
            self.publish_if_due();
        }
    }

    /// One frame from the audio side: into the block, and the analysis when the block is full
    fn feed(&mut self, frame: [f32; 2], overload: bool) {
        if self.shared.on(P_PAUSE) {
            return;
        }
        let (l, r) = if self.shared.on(P_STEREO) {
            (frame[0], frame[1])
        } else {
            (0.5 * (frame[0] + frame[1]), 0.5 * (frame[0] + frame[1]))
        };
        if self.filled < self.in_size {
            self.buf[0][self.filled] = l;
            self.buf[1][self.filled] = r;
            self.filled += 1;
        }
        if frame != [0.0, 0.0] {
            self.block_nonempty = true;
        }
        if self.filled >= self.in_size {
            self.analyze(overload);
        }
    }

    /// The FFT of the full block (each side in stereo), the waterfall's line and the display buffers
    fn analyze(&mut self, overload: bool) {
        let stereo = self.shared.on(P_STEREO);
        let scale = 2.0 / self.in_size as f32;
        for side in 0..if stereo { 2 } else { 1 } {
            for (t, (x, w)) in self.time.iter_mut().zip(self.buf[side].iter().zip(&self.window)) {
                *t = x * w;
            }
            self.fft.forward(&self.time, &mut self.spectrum);
            for (n, c) in self.norm[side].iter_mut().zip(&self.spectrum) {
                *n = (c.re * c.re + c.im * c.im).sqrt() * scale;
            }
        }
        if !stereo {
            self.norm[1].fill(0.0);
        }
        // Empty lines are counted so that an empty history does not scroll
        let height = self.sizes.height;
        if !self.block_nonempty && self.wf_remaining > 0 {
            self.wf_remaining -= 1;
        } else if self.block_nonempty {
            self.wf_remaining = height + 2;
        }
        if self.shared.on(P_WATERFALL) && self.wf_remaining > 0 {
            self.push_line(stereo, overload);
        }
        self.block_nonempty = false;
        self.update_buffers(stereo);
        self.dirty = true;
        // Drop only a part of the block, so that the older samples are analyzed again in another place of the window
        let overlap = self.overlap();
        if overlap == 1 {
            self.filled = 0;
        } else {
            let drop = self.in_size / overlap;
            for b in &mut self.buf {
                b.copy_within(drop.., 0);
            }
            self.filled -= drop;
        }
        self.publish_if_due();
    }

    /// `SaSpectrumView::updateBuffers`: the average, the peak hold and the reference freeze
    fn update_buffers(&mut self, stereo: bool) {
        let freeze = self.shared.on(P_FREEZE);
        if freeze && !self.frozen {
            self.freeze_request = true;
        } else if !freeze {
            self.frozen = false;
            self.freeze_request = false;
        }
        let pause = self.shared.on(P_PAUSE);
        let smooth = self.shared.on(P_SMOOTH);
        let hold = self.shared.on(P_PEAK_HOLD);
        let weight = self.shared.get(P_WEIGHT);
        let decay = self.shared.get(P_DECAY);
        for side in 0..if stereo { 2 } else { 1 } {
            let spectrum = &self.norm[side];
            for n in 0..self.bins {
                let spec = spectrum[n];
                let display = &mut self.display[side][n];
                let peak = &mut self.peak[side][n];
                if !pause {
                    *display = if smooth { spec * weight + *display * (1.0 - weight) } else { spec };
                }
                if freeze && self.freeze_request {
                    *peak = *display;
                } else if hold && !pause {
                    if spec > *peak {
                        *peak = spec;
                    } else if !freeze {
                        *peak *= decay;
                    }
                } else if !freeze && !hold {
                    *peak = 0.0;
                }
            }
        }
        if !stereo {
            self.display[1].fill(0.0);
            self.peak[1].fill(0.0);
        }
        if freeze && self.freeze_request {
            self.freeze_request = false;
            self.frozen = true;
        }
    }

    /// The columns' bins for the axis the knobs ask for (made again only when the axis changes)
    fn ensure_map(&mut self) {
        let log = self.shared.on(P_LOG_X);
        if self.map.key == Some((log, self.bins)) {
            return;
        }
        let nyquist = self.config.rate / 2.0;
        let bandwidth = nyquist / (self.bins - 1) as f32;
        let (fmin, fmax) = freq_range(self.config.freq_range, log, nyquist);
        // The legacy shows no bin below the range's start (nor the DC bin), and stretches the first one to the edge
        let first = ((fmin / bandwidth).ceil() as usize).clamp(1, self.bins - 1);
        let last = ((fmax / bandwidth).floor() as usize).clamp(first, self.bins - 1);
        let bin_of = |f: f32| ((f / bandwidth).round().max(0.0) as usize).clamp(first, last);
        self.map.spec.clear();
        for c in 0..SPEC_COLS {
            let f0 = x_to_freq(c as f32 / SPEC_COLS as f32, fmin, fmax, log);
            let f1 = x_to_freq((c + 1) as f32 / SPEC_COLS as f32, fmin, fmax, log);
            let (a, b) = (bin_of(f0), bin_of(f1));
            self.map.spec.push((a.min(b), a.max(b)));
        }
        self.map.wf.clear();
        for c in 0..WF_COLS {
            let f0 = x_to_freq(c as f32 / WF_COLS as f32, fmin, fmax, log);
            let f1 = x_to_freq((c + 1) as f32 / WF_COLS as f32, fmin, fmax, log);
            let to_pos = |f: f32| (f / bandwidth + 0.5).clamp(first as f32, (last + 1) as f32);
            self.map.wf.push((to_pos(f0), to_pos(f1)));
        }
        self.map.key = Some((log, self.bins));
    }

    /// The waterfall's newest line: each column's share of the bins as `makePixel` colors it
    fn push_line(&mut self, stereo: bool, overload: bool) {
        self.ensure_map();
        self.head = (self.head + self.sizes.height - 1) % self.sizes.height;
        let row = &mut self.history[self.head * WF_COLS..(self.head + 1) * WF_COLS];
        if overload {
            row.fill(0x2a_0000);
            return;
        }
        let gamma = self.shared.get(P_GAMMA);
        for (px, &(b0, b1)) in row.iter_mut().zip(&self.map.wf) {
            let left = average(&self.norm[0], b0, b1);
            let right = if stereo { average(&self.norm[1], b0, b1) } else { 0.0 };
            *px = pixel(left, right, stereo, gamma);
        }
    }

    fn publish_if_due(&mut self) {
        if self.dirty && self.last_publish.elapsed() >= PUBLISH_EVERY {
            self.publish();
        }
    }

    /// Fills the frame the triple buffer holds and hands it over
    fn publish(&mut self) {
        self.ensure_map();
        let frame = self.tx.input_buffer_mut();
        let mut shown = 0.0f32;
        for (series, source) in
            [&self.display[0], &self.display[1], &self.peak[0], &self.peak[1]].into_iter().enumerate()
        {
            let out = &mut frame[HEADER + series * SPEC_COLS..HEADER + (series + 1) * SPEC_COLS];
            for (v, &(a, b)) in out.iter_mut().zip(&self.map.spec) {
                *v = source[a..=b].iter().fold(0.0f32, |m, x| m.max(*x));
                shown += *v;
            }
        }
        let height = self.sizes.height;
        for j in 0..WF_ROWS {
            let src = (self.head + j * height / WF_ROWS) % height;
            let from = &self.history[src * WF_COLS..(src + 1) * WF_COLS];
            let at = HEADER + 4 * SPEC_COLS + j * WF_COLS;
            for (o, p) in frame[at..at + WF_COLS].iter_mut().zip(from) {
                *o = *p as f32;
            }
        }
        frame[..HEADER].copy_from_slice(&[
            1.0,
            self.config.rate,
            self.fft_size as f32,
            self.bins as f32,
            WF_ROWS as f32,
            if shown > 0.0 { 1.0 } else { 0.0 },
            if self.wf_remaining > 0 { 1.0 } else { 0.0 },
            if self.frozen { 1.0 } else { 0.0 },
        ]);
        self.tx.publish();
        self.last_publish = Instant::now();
        self.dirty = false;
    }
}

/// The mean of `spectrum` over the bin positions `b0..b1` (bin `k` spans `k..k + 1`), weighted by the overlap
fn average(spectrum: &[f32], b0: f32, b1: f32) -> f32 {
    let last = spectrum.len() - 1;
    let first = (b0.floor() as usize).min(last);
    if b1 - b0 < 1.0e-6 {
        return spectrum[first];
    }
    let (mut sum, mut weight) = (0.0f32, 0.0f32);
    for k in first..=(b1.ceil() as usize).min(spectrum.len()).saturating_sub(1).max(first) {
        let overlap = (b1.min(k as f32 + 1.0) - b0.max(k as f32)).max(0.0);
        sum += spectrum[k] * overlap;
        weight += overlap;
    }
    if weight > 0.0 { sum / weight } else { spectrum[first] }
}

/// `SaProcessor::makePixel`: a whole number `0xRRGGBB`. A value over 1 makes a component overflow, which the legacy
/// leaves as it is: it shows which frequency is clipping
fn pixel(left: f32, right: f32, stereo: bool, gamma: f32) -> u32 {
    let al = powf(left.max(0.0), gamma);
    let component = |i: usize| -> u32 {
        let v = if stereo {
            let ar = powf(right.max(0.0), gamma);
            COLOR_L[i] * al + COLOR_R[i] * ar
        } else {
            COLOR_MONO_W[i] * al
        };
        (v as i32 & 0xff) as u32
    };
    (component(0) << 16) | (component(1) << 8) | component(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(
        block_index: usize,
        window: Window,
    ) -> (Worker, rtrb::Producer<[f32; 2]>, triple_buffer::Output<Box<[f32]>>) {
        let shared = Arc::new(Shared {
            params: std::array::from_fn(|i| AtomicU32::new((PARAMS[i].3 as f32).to_bits())),
            stop: AtomicBool::new(false),
            clears: AtomicU32::new(0),
        });
        let (producer, consumer) = rtrb::RingBuffer::new(RING_FRAMES);
        let (input, output) = triple_buffer::triple_buffer(&vec![0.0f32; FRAME_FLOATS].into_boxed_slice());
        let config = Config { rate: 48_000.0, freq_range: 0, block_index, window };
        (Worker::new(config, shared, consumer, input), producer, output)
    }

    fn sine(worker: &mut Worker, hz: f32, amp: f32, frames: usize) {
        for i in 0..frames {
            let x = amp * lm_dsp::math::sinf(2.0 * core::f32::consts::PI * hz * i as f32 / 48_000.0);
            worker.feed([x, x], false);
        }
    }

    fn peak_bin(worker: &Worker) -> usize {
        let n = &worker.norm[0];
        (0..n.len()).max_by(|a, b| n[*a].total_cmp(&n[*b])).unwrap()
    }

    #[test]
    fn a_full_scale_sine_is_one_at_its_bin() {
        let (mut w, _p, _o) = worker(3, Window::BlackmanHarris);
        // 2048-sample block, FFT 8192 (zero padding 2): the bin of 1500 Hz is 1500 * 8192 / 48000
        sine(&mut w, 1500.0, 1.0, 8192);
        let bin = peak_bin(&w);
        assert!((bin as f32 - 1500.0 * w.fft_size as f32 / 48_000.0).abs() <= 1.0, "bin {bin}");
        assert!((w.norm[0][bin] - 1.0).abs() < 0.1, "amplitude {}", w.norm[0][bin]);
    }

    #[test]
    fn stereo_keeps_the_sides_apart() {
        let (mut w, _p, _o) = worker(2, Window::Hanning);
        w.shared.set(P_STEREO, 1.0);
        for i in 0..4096 {
            let x = lm_dsp::math::sinf(2.0 * core::f32::consts::PI * 3000.0 * i as f32 / 48_000.0);
            w.feed([x, 0.0], false);
        }
        assert!(w.norm[0].iter().fold(0.0f32, |m, x| m.max(*x)) > 0.5);
        assert!(w.norm[1].iter().all(|x| *x < 1.0e-6));
    }

    #[test]
    fn peak_hold_falls_and_the_freeze_keeps_the_reference() {
        let (mut w, _p, _o) = worker(1, Window::Hamming);
        w.shared.set(P_PEAK_HOLD, 1.0);
        sine(&mut w, 1000.0, 1.0, 2048);
        let bin = peak_bin(&w);
        let held = w.peak[0][bin];
        assert!(held > 0.5);
        for _ in 0..4096 {
            w.feed([0.0, 0.0], false);
        }
        assert!(w.peak[0][bin] < held, "the peak decays");
        w.shared.set(P_FREEZE, 1.0);
        sine(&mut w, 1000.0, 0.5, 1024);
        let frozen = w.peak[0][bin];
        for _ in 0..4096 {
            w.feed([0.0, 0.0], false);
        }
        assert_eq!(w.peak[0][bin], frozen, "a frozen reference does not decay");
        assert!(w.frozen);
    }

    #[test]
    fn the_averaging_follows_the_weight() {
        let (mut w, _p, _o) = worker(1, Window::Hanning);
        w.shared.set(P_SMOOTH, 1.0);
        w.shared.set(P_WEIGHT, 0.5);
        sine(&mut w, 1000.0, 1.0, 1024);
        let bin = peak_bin(&w);
        let first = w.display[0][bin];
        assert!(first > 0.0 && first <= 0.5 * w.norm[0][bin] + 1.0e-6, "half of the first result");
    }

    #[test]
    fn the_columns_span_the_axis_and_the_frame_is_whole() {
        let (mut w, _p, mut o) = worker(3, Window::BlackmanHarris);
        w.shared.set(P_WATERFALL, 1.0);
        sine(&mut w, 1000.0, 1.0, 8192);
        w.publish();
        let frame = o.read().to_vec();
        assert_eq!(frame.len(), FRAME_FLOATS);
        assert_eq!(frame[0], 1.0);
        assert_eq!(frame[5], 1.0, "there is something to show");
        let series = &frame[HEADER..HEADER + SPEC_COLS];
        let top = (0..SPEC_COLS).max_by(|a, b| series[*a].total_cmp(&series[*b])).unwrap();
        // 1 kHz on the logarithmic axis from 5 Hz to 24 kHz
        let expect = (log10f(1000.0) - log10f(LOWEST_LOG_FREQ)) / (log10f(24_000.0) - log10f(LOWEST_LOG_FREQ));
        assert!((top as f32 / SPEC_COLS as f32 - expect).abs() < 0.02, "column {top}");
        let newest = &frame[HEADER + 4 * SPEC_COLS..HEADER + 4 * SPEC_COLS + WF_COLS];
        assert!(newest.iter().any(|p| *p > 0.0), "the waterfall's newest line is drawn");
    }

    #[test]
    fn the_waterfall_pixel_is_the_legacys() {
        assert_eq!(pixel(0.0, 0.0, false, 0.3), 0);
        // mono: the bright blue at amplitude 1
        assert_eq!(pixel(1.0, 0.0, false, 0.3), (64 << 16) | (185 << 8) | 255);
        // stereo at 1 and 1: the two colors summed, overflowing as a byte
        let p = pixel(1.0, 1.0, true, 0.3);
        assert_eq!(p >> 16, (51 + 204) & 0xff);
        // over full scale wraps
        assert_ne!(pixel(10.0, 0.0, false, 1.0), pixel(1.0, 0.0, false, 1.0));
    }

    #[test]
    fn knobs_that_size_the_buffers_make_the_worker_size_them_again() {
        let (mut w, _p, _o) = worker(3, Window::Hanning);
        assert_eq!(w.fft_size, 8192);
        w.shared.set(P_PADDING, 0.0);
        w.shared.set(P_HEIGHT, 100.0);
        w.resize();
        assert_eq!((w.in_size, w.fft_size, w.history.len()), (2048, 2048, 100 * WF_COLS));
        w.shared.set(P_PADDING, 4.0);
        w.resize();
        assert_eq!(w.fft_size, 16384, "capped at the last size");
    }

    #[test]
    fn the_audio_side_feeds_only_an_open_window() {
        let shared = Arc::new(Shared {
            params: std::array::from_fn(|i| AtomicU32::new((PARAMS[i].3 as f32).to_bits())),
            stop: AtomicBool::new(false),
            clears: AtomicU32::new(0),
        });
        let (producer, mut consumer) = rtrb::RingBuffer::new(1024);
        let defaults: [f32; N_PARAMS] = std::array::from_fn(|i| PARAMS[i].3 as f32);
        let mut a = Analyze {
            knobs: Knobs::from_values(defaults),
            shared,
            ring: Some(producer),
            out: None,
            rate: 48_000.0,
            idle: usize::MAX / 2,
        };
        a.process(&mut [[1.0, 1.0]; 16]);
        assert!(consumer.pop().is_err(), "closed window: nothing copied");
        let mut frame = vec![0u8; FRAME_FLOATS * 4];
        a.display(&mut frame);
        a.process(&mut [[1.0, 1.0]; 16]);
        assert_eq!(consumer.slots(), 16);
        a.set_param(P_PAUSE, 1.0);
        a.process(&mut [[1.0, 1.0]; 16]);
        assert_eq!(consumer.slots(), 16, "paused: nothing more");
    }
}
