//! The spectrum analyser runs on the audio thread: pushing and analysing allocate nothing

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_dsp::spectrum::{FFT_LEN, Spectrum};

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

#[test]
fn pushing_and_analysing_do_not_allocate() {
    let mut s = Spectrum::new(48_000.0);
    let block = [[0.25f32, -0.25]; 256];
    assert_no_alloc(|| {
        for _ in 0..(2 * FFT_LEN / 256) {
            s.push(&block);
            s.update();
        }
    });
}
