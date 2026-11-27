//! Shared helpers of the lm-io integration tests
#![allow(dead_code)]

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lm_base::HostTime;
use lm_io::*;
use lm_render::*;
use lm_time::{Bpm, Ppq, TempoMap, TimeSignature};

pub fn limits() -> Limits {
    Limits { workers: 0, max_nodes: 64, max_params: 64, ..Limits::default() }
}

pub fn tempo(rate: u32) -> TempoMap {
    TempoMap::constant(rate, Ppq::DEFAULT, Bpm::from_integer(120).expect("bpm"), TimeSignature::COMMON).expect("map")
}

pub fn n(index: u32) -> NodeIndex {
    NodeIndex::new(index, 1)
}

/// A program: master sum (node 0) to outputs 0/1, and a processor (node 1) into it
pub fn program_with(spec: ProcessSpec, generation: u64, processor: Box<dyn Processor>) -> Install {
    let mut b = ProgramBuilder::new(ProgramGen(generation), spec, &limits(), tempo(spec.sample_rate));
    b.node(NodeDesc::sum(n(0), 2));
    b.master(n(0));
    b.output(OutputRoute { node: n(0), channel: 0, device_channel: 0 });
    b.output(OutputRoute { node: n(0), channel: 1, device_channel: 1 });
    b.node(NodeDesc::processor(n(1), 0, 2));
    b.feed(Feed::new(n(1), n(0), 2));
    let mut i = Install::new(b.build().expect("program"));
    i.add(n(1), NodeCore::new(processor, 1, None));
    i
}

/// A processor that sleeps while `sleep_ms` is non-zero (a stuck plugin) and counts its calls
pub struct Sleepy {
    pub sleep_ms: Arc<AtomicU64>,
    pub calls: Arc<AtomicU64>,
}

impl Processor for Sleepy {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let ms = self.sleep_ms.load(Ordering::Relaxed);
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
        for ch in 0..cx.buffers.outputs() {
            cx.buffers.output(ch).fill(0.0);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A tone at `freq` on both outputs
pub struct Tone {
    pub freq: f64,
    pub phase: f64,
    pub gain: f32,
}

impl Processor for Tone {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let step = self.freq / f64::from(cx.transport.sample_rate);
        let mut phase = self.phase;
        for k in 0..cx.frames {
            let v = (phase * std::f64::consts::TAU).sin() as f32 * self.gain;
            for ch in 0..cx.buffers.outputs() {
                cx.buffers.output(ch)[k] = v;
            }
            phase = (phase + step).fract();
        }
        self.phase = phase;
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Captures a ramp: device frame `f` reads `(f mod 2^23) / 2^23` on every input channel, exactly representable, so
/// a test can tell which device frame each recorded frame came from
pub struct Ramp;

pub const RAMP: f64 = 8_388_608.0;

impl DummySource for Ramp {
    fn fill(&mut self, first: u64, _: HostTime, inputs: &mut [Vec<f32>], frames: usize) {
        for c in inputs.iter_mut() {
            for (k, x) in c[..frames].iter_mut().enumerate() {
                *x = (((first + k as u64) % (1 << 23)) as f64 / RAMP) as f32;
            }
        }
    }
}

pub fn ramp_frame(x: f32) -> u64 {
    (f64::from(x) * RAMP).round() as u64
}

/// A fresh folder under the system's temporary folder, removed on drop
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("lm-io-{name}-{}-{}", std::process::id(), new_uuid()));
        std::fs::create_dir_all(&p).expect("temp dir");
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Waits until `f` is true, or panics after `secs`
pub fn wait_for(secs: u64, what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A started Dummy device with an engine
pub fn dummy_device(
    options: DummyOptions,
    config: &DeviceConfig,
) -> (Device, EngineControl, Arc<DummyControl>, DeviceInfo) {
    dummy_device_with(DummyBackend::new(options), config)
}

pub fn dummy_device_with(
    backend: DummyBackend,
    config: &DeviceConfig,
) -> (Device, EngineControl, Arc<DummyControl>, DeviceInfo) {
    install_panic_hook();
    let control = backend.control();
    let mut device = Device::new(Box::new(backend), RenderThreads::default());
    let info = device.open(config).expect("open");
    let (engine, renderer) = engine(info.spec(), limits());
    device.start(renderer).map_err(lm_io::StartError::into_error).expect("start");
    (device, engine, control, info)
}
