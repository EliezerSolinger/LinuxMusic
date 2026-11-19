//! The audio-thread calls neither allocate nor free: under `assert_no_alloc` (Rust's allocator) and the C heap trap
//! (libsamplerate's malloc). `harness = false`: measured on the main thread, with nothing else running

mod common;
use common::heap;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_sys_samplerate::{Quality, Resampler};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn main() {
    heap::install();
    heap::self_test();
    for quality in [Quality::Best, Quality::Medium, Quality::Fastest, Quality::ZeroOrderHold, Quality::Linear] {
        let mut r = Resampler::new(quality, 2, 48000.0 / 44100.0).unwrap();
        let input: Vec<f32> = (0..2 * 512).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut output = vec![0.0_f32; 2 * 1024];
        let events = heap::measure(|| {
            assert_no_alloc(|| {
                for block in 0..2000 {
                    let ratio = 48000.0 / 44100.0 * (1.0 + 0.001 * f64::from(block % 7));
                    r.set_ratio(ratio).unwrap();
                    let _ = r.process(&input, &mut output).unwrap();
                    if block % 500 == 0 {
                        r.set_ratio_now(1.5).unwrap();
                        r.reset().unwrap();
                    }
                }
                while r.finish(&mut output).unwrap().output_frames > 0 {}
            });
        });
        assert_eq!(events, 0, "{quality:?}: the C heap was touched {events} times");
    }
    // The test harness is off: success is the exit status
}
