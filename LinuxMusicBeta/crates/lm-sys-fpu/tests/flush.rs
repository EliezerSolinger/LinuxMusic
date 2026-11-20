//! FTZ/DAZ really flush, on this machine's processor (H6)

use lm_sys_fpu::{FlushDenormals, enable_on_this_thread, state};
use std::hint::black_box;

/// A result that would be subnormal (FTZ)
fn tiny_result() -> (f32, f64) {
    let f = black_box(f32::MIN_POSITIVE) / black_box(4.0_f32);
    let d = black_box(f64::MIN_POSITIVE) / black_box(4.0_f64);
    (f, d)
}

/// A subnormal input that, scaled, would give a normal result (DAZ)
fn subnormal_input() -> (f32, f64) {
    let f = black_box(f32::from_bits(0x0000_1000)) * black_box(2.0_f32.powi(40));
    let d = black_box(f64::from_bits(0x0000_0000_0010_0000)) * black_box(2.0_f64.powi(100));
    (f, d)
}

#[test]
fn starts_off_and_the_arithmetic_keeps_subnormals() {
    // A fresh test thread runs with the default environment
    assert!(!state().flushing());
    let (f, d) = tiny_result();
    assert!(f.is_subnormal() && d.is_subnormal());
    let (f, d) = subnormal_input();
    assert!(f.is_normal() && d.is_normal());
}

#[test]
fn the_guard_flushes_results_and_inputs_then_restores() {
    let before = state();
    {
        let guard = FlushDenormals::enable();
        assert_eq!(guard.previous(), before);
        assert!(state().flush_to_zero && state().denormals_are_zero);
        let (f, d) = tiny_result();
        assert_eq!((f.to_bits(), d.to_bits()), (0, 0), "FTZ: a subnormal result becomes +0");
        let (f, d) = subnormal_input();
        assert_eq!((f, d), (0.0, 0.0), "DAZ: a subnormal input is read as zero");
    }
    assert_eq!(state(), before);
    let (f, _) = tiny_result();
    assert!(f.is_subnormal());
}

#[test]
fn guards_nest() {
    let outer = FlushDenormals::enable();
    {
        let inner = FlushDenormals::enable();
        assert!(inner.previous().flushing());
    }
    assert!(state().flushing(), "the inner guard restores the outer state, which was flushing");
    drop(outer);
    assert!(!state().flushing());
}

#[test]
fn the_state_is_per_thread() {
    // Whether a new thread starts with the creator's FPU control word is the platform's choice (Linux x86-64 copies
    // MXCSR on clone, macOS arm64 does not), so nothing may rely on inheritance: every render thread enables the
    // flushing itself (H6). What the crate promises is that a change on one thread never reaches another
    let found = enable_on_this_thread();
    assert!(!found.flushing());
    assert!(state().flushing());
    let other = std::thread::spawn(|| {
        enable_on_this_thread();
        (state().flushing(), tiny_result().0)
    })
    .join()
    .unwrap();
    assert!(other.0, "enabling on another thread works there");
    assert_eq!(other.1, 0.0);
    let off = std::thread::spawn(|| {
        let initial = state();
        if initial.flushing() {
            // Inherited (Linux x86-64): nothing more to check about inheritance
            return None;
        }
        Some(tiny_result().0.is_subnormal())
    })
    .join()
    .unwrap();
    if let Some(subnormal) = off {
        assert!(subnormal, "a thread that does not flush keeps subnormals");
    }
    assert_eq!(tiny_result().0, 0.0, "this thread still flushes whatever other threads did");
}
