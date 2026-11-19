//! Flush-to-zero and denormals-are-zero on the calling thread (interfaces.md, H6).
//!
//! Every thread that renders turns the flushing of subnormal numbers on when it starts, so that a decaying filter or
//! reverb tail never falls into the slow subnormal range, and so that the audio thread and the export give the same
//! result for the same signal:
//!
//! - **x86-64:** the `FTZ` (bit 15, results) and `DAZ` (bit 6, inputs) bits of `MXCSR`;
//! - **aarch64** (Linux ARM and the Mac): the `FZ` bit (24) of `FPCR`, which flushes both inputs and results.
//!
//! Touching these registers needs inline assembly, and that is why this is an `lm-sys` crate (Parte 2, rule 9).
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | There is no C handle. The only state is the calling thread's floating-point control register. [`FlushDenormals`] owns the value to restore and gives it back in `Drop`. |
//! | Which thread may call each function? | Each function acts on **the calling thread only** (the register is per thread). [`FlushDenormals`] is `!Send` and `!Sync`, so it is restored on the thread that changed it; [`enable_on_this_thread`] is meant for the first line of a render thread. |
//! | Allocates, locks, does I/O? | Nothing allocates, locks or does I/O: each function is one or two register instructions. All of them may be called from the audio thread. |
//! | Creates threads? | No. |
//! | Calls back into our code? | No. |
//! | Tests | `tests/flush.rs`: a subnormal result is flushed (FTZ) and a subnormal input is read as zero (DAZ), with `black_box` so the compiler cannot fold the arithmetic; the guard restores the previous state, nests, and the state of one thread does not leak into another. |
//!
//! # Why the `unsafe` here is sound
//!
//! The assembly reads or writes one control register of the current thread and nothing else (`nostack`, and no
//! memory but the one local word on x86-64). The values written are the values read with only the FTZ/DAZ/FZ bits
//! changed, so a reserved bit is never set (a reserved bit set in `MXCSR` would fault). Every x86-64 processor has
//! `DAZ`; every aarch64 processor has `FZ`.
//!
//! The compiler assumes the default floating-point environment when it folds constants, so a value computed at
//! compile time is never flushed. Turning the flushing on is not a memory-safety question, but it changes results:
//! do it once, when the thread starts, before any audio is computed, not around single operations.

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("lm-sys-fpu supports x86-64 and aarch64 (the platforms of interfaces.md, Parte 4)");

use std::fmt;
use std::marker::PhantomData;

/// The flushing state of the calling thread's floating-point unit
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FpuState {
    /// Results that would be subnormal become zero (x86-64 `FTZ`, aarch64 `FZ`)
    pub flush_to_zero: bool,
    /// Subnormal inputs are read as zero (x86-64 `DAZ`; on aarch64 `FZ` does both, so this equals `flush_to_zero`)
    pub denormals_are_zero: bool,
}

impl FpuState {
    /// Both halves on: what every render thread runs with (H6)
    pub fn flushing(&self) -> bool {
        self.flush_to_zero && self.denormals_are_zero
    }
}

#[cfg(target_arch = "x86_64")]
mod arch {
    use std::arch::asm;

    pub type Word = u32;
    /// `FTZ` (bit 15) and `DAZ` (bit 6) of `MXCSR`
    pub const FTZ: Word = 1 << 15;
    pub const DAZ: Word = 1 << 6;
    pub const FLUSH: Word = FTZ | DAZ;

    #[inline]
    pub fn read() -> Word {
        let mut value: Word = 0;
        // SAFETY: `stmxcsr` stores the 32-bit MXCSR into the local `value`, which is valid, aligned and writable for
        // four bytes; it touches no other memory and no stack.
        unsafe {
            asm!("stmxcsr [{ptr}]", ptr = in(reg) &mut value, options(nostack, preserves_flags));
        }
        value
    }

    #[inline]
    pub fn write(value: Word) {
        // SAFETY: `ldmxcsr` loads MXCSR from the local `value`, valid for four bytes. Every caller passes a value read
        // with `read()` with only FTZ/DAZ changed, so no reserved bit is set (which would raise #GP); DAZ exists on
        // every x86-64 processor.
        unsafe {
            asm!("ldmxcsr [{ptr}]", ptr = in(reg) &value, options(nostack, preserves_flags, readonly));
        }
    }

    pub fn state(value: Word) -> super::FpuState {
        super::FpuState { flush_to_zero: value & FTZ != 0, denormals_are_zero: value & DAZ != 0 }
    }
}

#[cfg(target_arch = "aarch64")]
mod arch {
    use std::arch::asm;

    pub type Word = u64;
    /// `FZ` (bit 24) of `FPCR`: flushes subnormal inputs and results of single and double precision
    pub const FZ: Word = 1 << 24;
    pub const FLUSH: Word = FZ;

    #[inline]
    pub fn read() -> Word {
        let value: Word;
        // SAFETY: `mrs` copies FPCR into a general register; no memory, no stack, no flags.
        unsafe {
            asm!("mrs {v}, fpcr", v = out(reg) value, options(nomem, nostack, preserves_flags));
        }
        value
    }

    #[inline]
    pub fn write(value: Word) {
        // SAFETY: `msr` writes FPCR from a general register; the value is one read by `read()` with only FZ changed,
        // so no reserved bit is set. No memory, no stack, no flags.
        unsafe {
            asm!("msr fpcr, {v}", v = in(reg) value, options(nomem, nostack, preserves_flags));
        }
    }

    pub fn state(value: Word) -> super::FpuState {
        let fz = value & FZ != 0;
        super::FpuState { flush_to_zero: fz, denormals_are_zero: fz }
    }
}

/// The flushing state of the calling thread
pub fn state() -> FpuState {
    arch::state(arch::read())
}

/// Turns flush-to-zero and denormals-are-zero on for the rest of the calling thread's life. The first line of every
/// render thread (H6). Returns the state it found
pub fn enable_on_this_thread() -> FpuState {
    let before = arch::read();
    arch::write(before | arch::FLUSH);
    arch::state(before)
}

/// Turns flushing on for the calling thread until dropped, then restores exactly what was there (for a thread that is
/// lent to rendering, like a worker of an export, and goes back to other work after).
///
/// `!Send` and `!Sync`: it must be dropped on the thread that made it, since the register is per thread
#[must_use = "flushing is turned off again as soon as the guard is dropped"]
pub struct FlushDenormals {
    previous: arch::Word,
    _this_thread: PhantomData<*const ()>,
}

impl FlushDenormals {
    pub fn enable() -> Self {
        let previous = arch::read();
        arch::write(previous | arch::FLUSH);
        Self { previous, _this_thread: PhantomData }
    }

    /// The state the guard will restore
    pub fn previous(&self) -> FpuState {
        arch::state(self.previous)
    }
}

impl Drop for FlushDenormals {
    fn drop(&mut self) {
        // Only the flushing bits go back: anything else changed meanwhile (rounding, by someone else) is kept
        let now = arch::read();
        arch::write((now & !arch::FLUSH) | (self.previous & arch::FLUSH));
    }
}

impl fmt::Debug for FlushDenormals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FlushDenormals").field("previous", &self.previous()).finish()
    }
}
