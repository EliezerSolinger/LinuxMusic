//! The real-time calls neither allocate nor free: under `assert_no_alloc` (Rust's allocator) and the C heap trap
//! (Rubber Band's `new`/`malloc`). `harness = false`: measured on the main thread, with nothing else running.
//!
//! Also records what is **not** real-time safe, so that the safety sheet stays true: `set_pitch_scale` (to a new
//! value) and `reset` touch the heap on R3; a changing time ratio does not, on R2 or R3.

mod common;
use common::heap;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_sys_rubberband::{Engine, Formants, Format, Realtime, Settings};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const BLOCK: usize = 256;

fn exercise(r: &mut Realtime, input: &[Vec<f32>], output: &mut [Vec<f32>], blocks: usize, glide: bool) {
    let inputs: [&[f32]; 2] = [&input[0], &input[1]];
    for block in 0..blocks {
        // The tempo glides, as a live preview of a tempo change would
        if glide {
            r.set_time_ratio(1.0 + 0.25 * ((block % 50) as f64 / 50.0)).unwrap();
        }
        r.process(&inputs).unwrap();
        let _ = r.samples_required();
        let _ = r.start_delay();
        while r.available() > 0 {
            let (left, right) = output.split_at_mut(1);
            let mut outs: [&mut [f32]; 2] = [&mut left[0], &mut right[0]];
            if r.retrieve(&mut outs).unwrap() == 0 {
                break;
            }
        }
    }
}

fn main() {
    heap::install();
    heap::self_test();
    let input: Vec<Vec<f32>> =
        (0..2).map(|c| (0..BLOCK).map(|i| ((i + c) as f32 * 0.03).sin() * 0.5).collect()).collect();
    let mut output = vec![vec![0.0_f32; BLOCK * 2]; 2];

    for engine in [Engine::Finer, Engine::Faster] {
        for formants in [Formants::Shifted, Formants::Preserved] {
            for pitch in [1.0, 1.25] {
                let settings = Settings { engine, formants, ..Settings::default() };
                let mut r = Realtime::new(Format::new(48_000, 2), BLOCK, 1.0, pitch, &settings).unwrap();
                // A first stretch of blocks outside the measure: the library sizes nothing lazily that matters, but
                // the first blocks fill its rings; then the steady state is measured
                exercise(&mut r, &input, &mut output, 20, false);
                let events = heap::measure(|| assert_no_alloc(|| exercise(&mut r, &input, &mut output, 2000, false)));
                assert_eq!(events, 0, "{engine:?} {formants:?} pitch {pitch}: the C heap was touched {events} times");
                // A gliding time ratio: real-time safe on both engines (the safety sheet says so; R2 used to
                // recompute its stretch profile on the heap, and no longer does in the libraries CI builds against)
                let glide = heap::measure(|| assert_no_alloc(|| exercise(&mut r, &input, &mut output, 2000, true)));
                assert_eq!(
                    glide, 0,
                    "{engine:?} {formants:?} pitch {pitch}: a ratio glide touched the heap {glide} times"
                );
            }
        }
    }

    // What the safety sheet says is not real-time safe really is not, on R3 (if this ever becomes 0, the sheet can be
    // relaxed)
    let mut r = Realtime::new(Format::new(48_000, 2), BLOCK, 1.0, 1.0, &Settings::default()).unwrap();
    exercise(&mut r, &input, &mut output, 20, false);
    let pitch = heap::measure(|| r.set_pitch_scale(1.5).unwrap());
    let reset = heap::measure(|| r.reset());
    assert!(pitch + reset > 0, "set_pitch_scale and reset no longer allocate on R3: update the safety sheet");
    // The test harness is off: success is the exit status
}
