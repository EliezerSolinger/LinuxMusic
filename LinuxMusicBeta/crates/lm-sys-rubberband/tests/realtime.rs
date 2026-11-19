//! The real-time mode against the real Rubber Band library

mod common;
use common::signal::{frequency, rms};

use lm_sys_rubberband::{Engine, Format, Realtime, Settings, Transients};
use std::f64::consts::TAU;

const RATE: u32 = 48_000;
const BLOCK: usize = 512;

/// Streams `seconds` of a stereo sine through `r` block by block; returns the left output, the start delay dropped
fn stream(r: &mut Realtime, freq: f64, seconds: f64) -> Vec<f32> {
    let mut out = stream_raw(r, freq, seconds);
    out.drain(..r.start_delay().min(out.len()));
    out
}

/// The same, without dropping anything
fn stream_raw(r: &mut Realtime, freq: f64, seconds: f64) -> Vec<f32> {
    let frames = (f64::from(RATE) * seconds) as usize;
    let mut out = Vec::new();
    let mut left = vec![0.0_f32; BLOCK];
    let mut right = vec![0.0_f32; BLOCK];
    let mut scratch = [vec![0.0_f32; BLOCK * 4], vec![0.0_f32; BLOCK * 4]];
    let mut n = 0;
    while n < frames {
        for i in 0..BLOCK {
            let v = (0.5 * (TAU * freq * (n + i) as f64 / f64::from(RATE)).sin()) as f32;
            left[i] = v;
            right[i] = v * 0.5;
        }
        r.process(&[&left, &right]).unwrap();
        n += BLOCK;
        loop {
            let (a, b) = scratch.split_at_mut(1);
            let got = r.retrieve(&mut [&mut a[0], &mut b[0]]).unwrap();
            if got == 0 {
                break;
            }
            out.extend_from_slice(&scratch[0][..got]);
        }
    }
    out
}

#[test]
fn a_stream_keeps_its_pitch_and_follows_the_ratio() {
    for engine in [Engine::Finer, Engine::Faster] {
        let settings = Settings { engine, transients: Transients::Smooth, ..Settings::default() };
        for ratio in [0.8, 1.0, 1.5] {
            let mut r = Realtime::new(Format::new(RATE, 2), BLOCK, ratio, 1.0, &settings).unwrap();
            assert_eq!(r.engine_version(), if engine == Engine::Finer { 3 } else { 2 });
            assert!(r.start_delay() > 0, "{engine:?}: the real-time mode has a delay");
            let out = stream(&mut r, 440.0, 3.0);
            // Once the stretcher is full, the output grows at the ratio: three more seconds in give 3 × ratio out,
            // within what the analysis window holds back at either end
            let more = stream_raw(&mut r, 440.0, 3.0).len() as f64;
            let expected = 3.0 * f64::from(RATE) * ratio;
            assert!(
                (more - expected).abs() < 4.0 * BLOCK as f64 * ratio.max(1.0),
                "{engine:?} x{ratio}: {more} frames, expected about {expected}"
            );
            let (from, to) = (out.len() / 4, out.len() * 3 / 4);
            let f = frequency(&out, f64::from(RATE), from, to);
            assert!((f / 440.0 - 1.0).abs() < 0.005, "{engine:?} x{ratio}: {f} Hz");
            assert!(rms(&out, from, to) > 0.25, "{engine:?} x{ratio}: level {}", rms(&out, from, to));
        }
    }
}

#[test]
fn a_pitch_change_moves_the_frequency() {
    let mut r = Realtime::new(Format::new(RATE, 2), BLOCK, 1.0, 1.5, &Settings::default()).unwrap();
    let out = stream(&mut r, 440.0, 2.0);
    let f = frequency(&out, f64::from(RATE), out.len() / 4, out.len() * 3 / 4);
    assert!((f / 660.0 - 1.0).abs() < 0.005, "{f} Hz");

    // Changed while it runs (off the audio thread here, as the sheet asks)
    r.set_pitch_scale(0.75).unwrap();
    let out = stream(&mut r, 440.0, 2.0);
    let f = frequency(&out, f64::from(RATE), out.len() / 2, out.len() * 7 / 8);
    assert!((f / 330.0 - 1.0).abs() < 0.005, "{f} Hz");
    assert_eq!(r.pitch_scale(), 0.75);
}

#[test]
fn bad_blocks_are_errors_with_keys() {
    let mut r = Realtime::new(Format::new(RATE, 2), BLOCK, 1.0, 1.0, &Settings::default()).unwrap();
    let a = vec![0.0_f32; BLOCK];
    let long = vec![0.0_f32; BLOCK + 1];
    assert_eq!(r.process(&[&a]).unwrap_err().key(), "rubberband.bad_block", "one channel of two");
    assert_eq!(r.process(&[&a, &a[..10]]).unwrap_err().key(), "rubberband.bad_block", "unequal");
    assert_eq!(r.process(&[&long, &long]).unwrap_err().key(), "rubberband.bad_block", "over max_block");
    r.process(&[&a, &a]).unwrap();
    let mut x = vec![0.0_f32; 8];
    assert_eq!(r.retrieve(&mut [&mut x]).unwrap_err().key(), "rubberband.bad_block");
    assert_eq!(r.set_time_ratio(0.0).unwrap_err().key(), "rubberband.bad_ratio");
    assert_eq!(r.set_pitch_scale(-1.0).unwrap_err().key(), "rubberband.bad_pitch");
    let s = Settings::default();
    assert_eq!(Realtime::new(Format::new(RATE, 2), 0, 1.0, 1.0, &s).unwrap_err().key(), "rubberband.bad_block");
    assert_eq!(Realtime::new(Format::new(RATE, 2), 1 << 17, 1.0, 1.0, &s).unwrap_err().key(), "rubberband.bad_block");
    assert_eq!(Realtime::new(Format::new(0, 2), BLOCK, 1.0, 1.0, &s).unwrap_err().key(), "rubberband.bad_rate");
}

#[test]
fn reset_forgets_the_stream() {
    let mut r = Realtime::new(Format::new(RATE, 2), BLOCK, 1.0, 1.0, &Settings::default()).unwrap();
    let _ = stream(&mut r, 440.0, 0.5);
    r.reset();
    assert_eq!(r.available(), 0);
    let out = stream(&mut r, 440.0, 1.0);
    let f = frequency(&out, f64::from(RATE), out.len() / 4, out.len() * 3 / 4);
    assert!((f / 440.0 - 1.0).abs() < 0.005, "{f} Hz");
}

#[test]
fn stretchers_run_on_several_threads_at_once() {
    // Made, used and dropped on several workers at the same time (the crate lock covers new and delete)
    std::thread::scope(|scope| {
        for t in 0..4 {
            scope.spawn(move || {
                let mut r =
                    Realtime::new(Format::new(RATE, 2), BLOCK, 1.0 + 0.1 * f64::from(t), 1.0, &Settings::default())
                        .unwrap();
                let out = stream(&mut r, 440.0, 1.0);
                let f = frequency(&out, f64::from(RATE), out.len() / 4, out.len() * 3 / 4);
                assert!((f / 440.0 - 1.0).abs() < 0.005, "thread {t}: {f} Hz");
            });
        }
    });
}
