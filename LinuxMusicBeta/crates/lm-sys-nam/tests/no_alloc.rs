//! What the audio thread calls neither allocates nor frees: under `assert_no_alloc` (Rust's allocator) and the C heap
//! trap (NAM's and Eigen's `new`). `harness = false`: measured on the main thread, with nothing else running

mod common;
use common::heap;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_sys_nam::Model;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn main() {
    heap::install();
    heap::self_test();
    for name in ["lstm.nam", "wavenet.nam", "slimmable_wavenet.nam"] {
        let bytes =
            std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).unwrap();
        let mut model = Model::load(&bytes).unwrap();
        model.reset(48_000.0, 256).unwrap();
        let mut io = vec![0.0f32; 1_000];
        model.process(&mut io);
        let events = heap::measure(|| {
            assert_no_alloc(|| {
                for block in 0..500usize {
                    let n = [256, 32, 1, 100, 1_000][block % 5];
                    for (i, x) in io[..n].iter_mut().enumerate() {
                        *x = ((block * 31 + i) % 97) as f32 / 97.0 - 0.5;
                    }
                    model.process(&mut io[..n]);
                }
            });
        });
        assert_eq!(events, 0, "{name}: the C heap was touched {events} times");
    }
}
