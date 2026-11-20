//! What the audio thread calls neither allocates nor frees: under `assert_no_alloc` (Rust's allocator) and the C heap
//! trap (reSID's `new`). `harness = false`: measured on the main thread, with nothing else running

mod common;
use common::heap;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_sys_resid::{ChipModel, PAL_CLOCK, Sid};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn main() {
    heap::install();
    heap::self_test();
    let mut sid = Sid::new(f64::from(PAL_CLOCK), 48_000.0).unwrap();
    let mut other = Sid::new(f64::from(PAL_CLOCK), 48_000.0).unwrap();
    let mut out = [0i16; 256];
    let events = heap::measure(|| {
        assert_no_alloc(|| {
            for block in 0..2_000u32 {
                for reg in 0..25u8 {
                    sid.write(reg, (block as u8).wrapping_mul(reg).wrapping_add(reg));
                }
                if block % 50 == 0 {
                    sid.set_chip_model(if block % 100 == 0 { ChipModel::Mos6581 } else { ChipModel::Mos8580 });
                }
                let mut cycles = 6_000;
                let _ = sid.clock(&mut cycles, &mut out);
                if block % 7 == 0 {
                    other.clone_from(&sid);
                }
                if block % 333 == 0 {
                    sid.reset();
                }
            }
        });
    });
    assert_eq!(events, 0, "the C heap was touched {events} times");
}
