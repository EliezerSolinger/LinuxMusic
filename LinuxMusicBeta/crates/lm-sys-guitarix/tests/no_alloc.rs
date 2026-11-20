//! What the audio thread calls neither allocates nor frees: every unit's process, parameter and output calls under
//! `assert_no_alloc` (Rust's allocator) and the C heap trap (Guitarix's `new`, zita's, FFTW's). `harness = false`:
//! measured on the main thread, with nothing else running

mod common;
use common::heap;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_sys_guitarix::{Instance, MIN_CONVOLVER_BLOCK, units};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn main() {
    heap::install();
    heap::self_test();
    for unit in units() {
        // The block a unit in fixed blocks runs in (the Detune: exactly its block), 64 for the others' convolvers
        let block = if unit.id == "smbPitchShift" { 256 } else { MIN_CONVOLVER_BLOCK as usize };
        let count = if unit.stereo { 1 } else { 2 };
        let mut instances: Vec<Instance> = (0..count).map(|_| Instance::new(unit.index).unwrap()).collect();
        for i in &mut instances {
            i.start(48_000, block as u32);
        }
        let mut input = [vec![0.0f32; block], vec![0.0f32; block]];
        let mut output = [vec![0.0f32; block], vec![0.0f32; block]];
        let mut n = 0u64;
        let events = heap::measure(|| {
            assert_no_alloc(|| {
                for round in 0..200 {
                    for ch in &mut input {
                        for x in ch.iter_mut() {
                            *x = (0.3 * (2.0 * ((n as f64 * 110.0 / 48_000.0).fract()) - 1.0)) as f32;
                            n += 1;
                        }
                    }
                    for i in &mut instances {
                        // Every setting moved now and then (a selector switches its module, which is cleared)
                        if round % 20 == 0 {
                            for (p, d) in unit.params.iter().enumerate() {
                                if !d.is_output() {
                                    let t = (round / 20 % 3) as f32 / 2.0;
                                    i.set(p, d.low + (d.up - d.low) * t);
                                }
                            }
                        }
                        if round == 150 {
                            i.clear();
                        }
                    }
                    let [in0, in1] = &mut input;
                    let [out0, out1] = &mut output;
                    if unit.stereo {
                        instances[0].process_stereo([in0, in1], [out0, out1]);
                    } else {
                        instances[0].process_mono(in0, out0);
                        instances[1].process_mono(in1, out1);
                    }
                    for (p, d) in unit.params.iter().enumerate() {
                        if d.is_output() {
                            let _ = if d.is_peak() { instances[0].take_peak(p) } else { instances[0].output(p) };
                        }
                    }
                }
            });
        });
        assert_eq!(events, 0, "{}: the C heap was touched {events} times", unit.id);
    }
}
