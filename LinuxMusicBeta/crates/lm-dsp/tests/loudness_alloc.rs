//! The loudness meter runs on the audio thread: processing and reading never allocate (under the allocation trap)

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_dsp::loudness::Loudness;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn processing_and_reading_do_not_allocate() {
    let mut m = Loudness::new(48_000);
    let block = vec![0.1f32; 256];
    assert_no_alloc(|| {
        for _ in 0..400 {
            m.process(&block, &block);
        }
        let _ = (m.momentary(), m.short_term(), m.integrated(), m.range(), m.true_peak());
    });
    assert!(m.integrated().is_finite());
}
