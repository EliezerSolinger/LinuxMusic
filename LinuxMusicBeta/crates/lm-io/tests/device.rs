//! The device and its barrier (interfaces.md, I-10.1, R5, S16, D11; U11, U16; C7)

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::*;
use lm_io::*;
use lm_render::*;

fn config(rate: u32, period: u32) -> DeviceConfig {
    DeviceConfig::duplex("dummy", rate, period)
}

/// The renderer comes back after stop, and the device can start again with it
#[test]
fn stop_gives_the_renderer_back() {
    let (mut device, mut engine, control, info) = dummy_device(DummyOptions::default(), &config(48_000, 256));
    assert_eq!(info.max_block, 256, "C7: the engine's block is the negotiated period");
    wait_for(5, "periods", || control.periods() > 10);
    let renderer = device.stop().expect("stop");
    assert!(!device.guard().is_inside());
    let calls = device.guard().calls();
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(device.guard().calls(), calls, "no callback after the barrier");
    // Again, with the same renderer
    device.open(&config(48_000, 256)).expect("reopen");
    device.start(renderer).map_err(lm_io::StartError::into_error).expect("restart");
    wait_for(5, "periods again", || control.periods() > 20);
    engine.commands().send(Command::Play).expect("play");
    wait_for(5, "playing", || engine.frame().playing);
    let renderer = device.stop().expect("stop again");
    assert!(renderer.is_playing());
}

/// A slow callback (a plugin taking 300 ms): stop waits for it, and the renderer comes back only after it returned
#[test]
fn barrier_waits_for_a_slow_callback() {
    let (mut device, mut engine, _control, info) = dummy_device(DummyOptions::default(), &config(48_000, 256));
    let sleep_ms = Arc::new(AtomicU64::new(0));
    let calls = Arc::new(AtomicU64::new(0));
    engine
        .install(program_with(info.spec(), 1, Box::new(Sleepy { sleep_ms: sleep_ms.clone(), calls: calls.clone() })))
        .expect("install");
    wait_for(5, "processor running", || calls.load(Ordering::Relaxed) > 5);
    sleep_ms.store(300, Ordering::Relaxed);
    wait_for(5, "inside the slow call", || device.guard().is_inside());
    let t0 = Instant::now();
    let renderer = device.stop().expect("the barrier passes once the call returns");
    let waited = t0.elapsed();
    assert!(waited < STOP_TIMEOUT, "waited {waited:?}");
    assert!(!device.guard().is_inside(), "the renderer came back while a callback ran");
    drop(renderer);
}

/// A stuck callback (4 s): stop gives up after 2 s and the device is abandoned, never killed (D11)
#[test]
fn stuck_callback_is_abandoned_after_the_timeout() {
    let (mut device, mut engine, _control, info) = dummy_device(DummyOptions::default(), &config(48_000, 256));
    let sleep_ms = Arc::new(AtomicU64::new(0));
    let calls = Arc::new(AtomicU64::new(0));
    engine
        .install(program_with(info.spec(), 1, Box::new(Sleepy { sleep_ms: sleep_ms.clone(), calls: calls.clone() })))
        .expect("install");
    wait_for(5, "processor running", || calls.load(Ordering::Relaxed) > 5);
    sleep_ms.store(4_000, Ordering::Relaxed);
    wait_for(5, "inside the stuck call", || device.guard().is_inside());
    let before = abandoned_devices();
    let t0 = Instant::now();
    let err = device.stop().expect_err("the call is stuck");
    let waited = t0.elapsed();
    assert!(waited >= STOP_TIMEOUT && waited < STOP_TIMEOUT + Duration::from_millis(500), "waited {waited:?}");
    assert_eq!(err.to_error().key(), "io.device.stop_timed_out");
    assert_eq!(abandoned_devices(), before + 1);
    // The stuck thread finishes later on its own; the session makes a new engine (here: nothing else to do)
    sleep_ms.store(0, Ordering::Relaxed);
}

/// S16: dropping a running device runs the barrier
#[test]
fn drop_runs_the_barrier() {
    let (device, _engine, control, _) = dummy_device(DummyOptions::default(), &config(48_000, 128));
    wait_for(5, "periods", || control.periods() > 3);
    let guard = device.guard().clone();
    drop(device);
    assert!(!guard.is_inside());
    let calls = guard.calls();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(guard.calls(), calls);
}

/// U11: the card goes away; the device reports it, stop returns at once, and the renderer goes on in a new Dummy
#[test]
fn removed_card_falls_back_to_the_dummy() {
    let (mut device, mut engine, control, _) = dummy_device(DummyOptions::default(), &config(48_000, 256));
    engine.commands().send(Command::Play).expect("play");
    wait_for(5, "playing", || engine.frame().playing);
    control.disconnect();
    let mut disconnected = false;
    wait_for(5, "the disconnection", || {
        let _ = device.poll(|e| disconnected |= matches!(e, DeviceEvent::Disconnected));
        disconnected
    });
    assert!(device.is_lost());
    let t0 = Instant::now();
    let renderer = device.stop().expect("the thread already left");
    assert!(t0.elapsed() < Duration::from_millis(500));
    let mut fallback = Device::new(Box::new(DummyBackend::new(DummyOptions::default())), RenderThreads::default());
    fallback.open(&config(48_000, 256)).expect("dummy");
    fallback.start(renderer).map_err(lm_io::StartError::into_error).expect("start on the dummy");
    let p = engine.frame().position;
    wait_for(5, "the playhead moves on the dummy", || engine.frame().position > p + 4800);
}

/// C2 at the device: an injected capture xrun is reported and reaches the renderer's capture gap
#[test]
fn capture_xrun_is_reported() {
    let (mut device, _engine, control, _) = dummy_device(DummyOptions::default(), &config(48_000, 256));
    wait_for(5, "periods", || control.periods() > 3);
    control.inject_capture_lost(500);
    let mut seen = None;
    wait_for(5, "the xrun", || {
        let _ = device.poll(|e| {
            if let DeviceEvent::Xrun { capture_lost, .. } = e {
                seen = Some(capture_lost);
            }
        });
        seen.is_some()
    });
    assert_eq!(seen, Some(500));
}

/// A device that cannot run at the project's rate: the edge resampler gives the renderer whole blocks at the
/// project's rate, and a tone played comes out at the device's rate with its frequency
#[test]
fn edge_resampler_device() {
    let options = DummyOptions { device_rate: Some(48_000), ..DummyOptions::default() };
    let cfg = config(44_100, 256);
    let (mut device, mut engine, control, info) = dummy_device(options, &cfg);
    assert!(info.edge_resampler);
    assert_eq!(info.device_rate, 48_000);
    assert!(info.max_block >= 236, "block {}", info.max_block);
    engine
        .install(program_with(info.spec(), 1, Box::new(Tone { freq: 1000.0, phase: 0.0, gain: 0.5 })))
        .expect("install");
    wait_for(10, "a second", || control.frames() > 48_000);
    let renderer = device.stop().expect("stop");
    assert!(renderer.spec().sample_rate == 44_100);
    // Without permission for an edge resampler, the rate is refused
    let mut strict = cfg.clone();
    strict.allow_edge_resampler = false;
    let mut d = Device::new(Box::new(DummyBackend::new(options)), RenderThreads::default());
    assert_eq!(d.open(&strict).expect_err("refused").key(), "io.device.rate_unsupported");
}

/// The engine's block must cover the device's (C7)
#[test]
fn spec_mismatch_is_refused() {
    let mut device = Device::new(Box::new(DummyBackend::new(DummyOptions::default())), RenderThreads::default());
    let info = device.open(&config(48_000, 512)).expect("open");
    let (_engine, renderer) = engine(ProcessSpec { sample_rate: 48_000, max_block: 256 }, limits());
    let err = device.start(renderer).expect_err("too small").into_error();
    assert_eq!(err.key(), "io.device.spec_mismatch");
    assert_eq!(info.max_block, 512);
}

/// I-10.3: switching devices keeps the renderer (and its position)
#[test]
fn switch_keeps_the_renderer() {
    let (device, mut engine, _control, _) = dummy_device(DummyOptions::default(), &config(48_000, 256));
    engine.commands().send(Command::Play).expect("play");
    wait_for(5, "playing", || engine.frame().position > 4800);
    let new = DummyBackend::new(DummyOptions { inputs: 4, outputs: 4, ..DummyOptions::default() });
    let control = new.control();
    let mut device =
        switch_device(device, Box::new(new), &config(48_000, 128), RenderThreads::default()).expect("switch");
    wait_for(5, "the new device runs", || control.periods() > 10);
    assert_eq!(device.info().map(|i| i.inputs), Some(4));
    let renderer = device.stop().expect("stop");
    assert!(renderer.is_playing());
}

/// Broadband noise, like a fan in front of the microphone
struct Fan(u32);

impl DummySource for Fan {
    fn fill(&mut self, _: u64, _: lm_base::HostTime, inputs: &mut [Vec<f32>], frames: usize) {
        for c in inputs.iter_mut() {
            for x in c[..frames].iter_mut() {
                self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *x = ((self.0 >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.2;
            }
        }
    }
}

/// I-10.4, H19: five measures through a loopback of known latency, under fan noise; the residual is the measure
/// minus what the device reports
#[test]
fn calibration_measures_the_loopback() {
    let latency = 1_000;
    let options = DummyOptions {
        pace: Pace::Free { speed: Some(20.0) },
        loopback: Some(latency),
        reported_latency: 100,
        ..DummyOptions::default()
    };
    let backend = DummyBackend::new(options).with_source(Box::new(Fan(7)));
    let cfg = config(48_000, 256);
    let (result, _backend) = calibrate(
        Box::new(backend),
        &cfg,
        CalibrationOptions::default(),
        RenderThreads::default(),
        &lm_base::Cancel::new(),
        &lm_base::Progress::new(),
    );
    let c = result.expect("calibration");
    assert_eq!(c.measures.len(), 5);
    assert_eq!(c.round_trip, i64::from(latency), "{c:?}");
    assert_eq!(c.reported, 100 + 100 + 512);
    assert_eq!(c.residual, i64::from(latency) - c.reported);
    assert_eq!(c.key, CalibrationKey { device: "dummy".into(), rate: 48_000, edge_resampler: false });
    // Without a loopback there is nothing to find: refused, not guessed
    let silent = DummyBackend::new(DummyOptions { pace: Pace::Free { speed: Some(20.0) }, ..DummyOptions::default() })
        .with_source(Box::new(Fan(3)));
    let (result, _) = calibrate(
        Box::new(silent),
        &cfg,
        CalibrationOptions::default(),
        RenderThreads::default(),
        &lm_base::Cancel::new(),
        &lm_base::Progress::new(),
    );
    assert_eq!(result.expect_err("no signal").key(), "io.calibration.no_signal");
}
