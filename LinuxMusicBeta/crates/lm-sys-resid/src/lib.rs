//! Safe binding to reSID 1.0, the MOS6581/MOS8580 SID emulator by Dag Lem, for the Sid instrument (F-INS-016): the
//! sources the legacy program compiled (`legado/plugins/Sid/resid`), vendored in `vendor/resid` and compiled by
//! `build.rs` with the legacy build's flags, behind the small C interface of `src/shim.cc`.
//!
//! One [`Sid`] is one chip, sampled with reSID's `SAMPLE_FAST` (the nearest cycle to each sample, as the legacy
//! plugin set it). A chip is made in a build (it allocates); on the audio thread it is written, clocked and copied
//! ([`Sid::copy_from`], the instrument's crossfade of a change) without allocating.
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | A `reSID::SID` (a subclass in the shim, to fix its copies) per [`Sid`], made by `lm_resid_new`, deleted by its `Drop`. One owner: `Sid` is moved, never shared; [`Clone`] makes a new chip. reSID's class tables (filter models, waveforms: process-wide `static`s) are made by the first chip's constructor and never freed. |
//! | Which thread may call each function? | `Sid` is `Send` and not `Sync`, every call takes `&mut self` or `&self` of one chip: one thread at a time. reSID makes its class tables in the first constructor **without a guard**: [`Sid::new`] makes the first chip of the process under a `Once` (on the building thread) before any other, so no two constructors race on the tables. |
//! | Allocates, locks, does I/O? | [`Sid::new`] and [`Clone::clone`] allocate (the chip; the first one also builds about 5 MB of tables in the `static`s): build time only. `write`, `clock`, `set_chip_model`, `reset` and `copy_from` neither allocate nor lock nor do I/O (checked by `tests/no_alloc.rs` under `assert_no_alloc` and the C heap trap): for the audio thread. `Drop` frees: never on the audio thread. |
//! | Creates threads? | No. |
//! | Calls back into our code? | No. No C++ exception crosses: `new` is `nothrow` in the shim, nothing else throws. |
//! | Tests | `tests/sid.rs`: a voice sounds at its pitch, the chip models differ, a copy plays on exactly as its original (and does not touch it), the sync ring of a copy is its own; `tests/no_alloc.rs`: the audio-thread calls under the traps. ASan in CI. |

use std::ffi::{CStr, c_char, c_int, c_short, c_uint, c_void};
use std::fmt;
use std::ptr::NonNull;
use std::sync::Once;

use lm_base::{Error, Result};

mod ffi {
    use super::{c_char, c_int, c_short, c_uint, c_void};

    unsafe extern "C" {
        pub fn lm_resid_new(clock: f64, rate: f64) -> *mut c_void;
        pub fn lm_resid_delete(sid: *mut c_void);
        pub fn lm_resid_set_chip_model(sid: *mut c_void, mos6581: c_int);
        pub fn lm_resid_write(sid: *mut c_void, offset: c_uint, value: c_uint);
        pub fn lm_resid_clock(sid: *mut c_void, cycles: *mut c_int, buffer: *mut c_short, n: c_int) -> c_int;
        pub fn lm_resid_reset(sid: *mut c_void);
        pub fn lm_resid_copy(to: *mut c_void, from: *const c_void);
        pub fn lm_resid_version() -> *const c_char;
    }
}

/// The PAL Commodore 64's clock, cycles per second (the legacy plugin's `C64_PAL_CYCLES_PER_SEC`)
pub const PAL_CLOCK: u32 = 985_248;
/// The chip's registers (0x00 to 0x18); the last four are read-only and not written here
pub const REGISTERS: usize = 0x19;

/// Which chip
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipModel {
    Mos6581,
    Mos8580,
}

/// One SID chip
pub struct Sid {
    chip: NonNull<c_void>,
    clock: f64,
    rate: f64,
}

// SAFETY: the chip is plain memory owned by this value alone (no thread-local state, no reference to the creating
// thread); the class tables it reads are process-wide and only written by the first constructor, which `new`
// serializes before any other chip exists
unsafe impl Send for Sid {}

impl fmt::Debug for Sid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sid").field("clock", &self.clock).field("rate", &self.rate).finish_non_exhaustive()
    }
}

/// The first chip of the process, made under the `Once`: it builds reSID's class tables, unguarded in reSID
static TABLES: Once = Once::new();

fn first_chip() {
    TABLES.call_once(|| {
        // SAFETY: a chip made and deleted at once, on this thread, while no other chip can be made (call_once
        // blocks every other caller until it returns)
        unsafe {
            let chip = ffi::lm_resid_new(f64::from(PAL_CLOCK), 44_100.0);
            if !chip.is_null() {
                ffi::lm_resid_delete(chip);
            }
        }
    });
}

impl Sid {
    /// A chip clocked at `clock` Hz, sampled at `rate` Hz (`SAMPLE_FAST`), as a MOS8580 with its filter on, reset.
    /// Allocates (build time only). Key `resid.cannot_make` if reSID refuses the rates or memory runs out
    pub fn new(clock: f64, rate: f64) -> Result<Self> {
        first_chip();
        if !(clock.is_finite() && rate.is_finite() && clock > 0.0 && rate > 0.0) {
            return Err(Error::new("resid.cannot_make").arg("rate", rate));
        }
        // SAFETY: the tables are made (first_chip); the arguments are checked finite and positive
        let chip = unsafe { ffi::lm_resid_new(clock, rate) };
        let chip = NonNull::new(chip).ok_or_else(|| Error::new("resid.cannot_make").arg("rate", rate))?;
        Ok(Self { chip, clock, rate })
    }

    pub fn clock_rate(&self) -> f64 {
        self.clock
    }

    pub fn sample_rate(&self) -> f64 {
        self.rate
    }

    /// Sets the chip model. reSID also clears the filter's integrators here (it expects it only with a reset)
    pub fn set_chip_model(&mut self, model: ChipModel) {
        // SAFETY: a live chip, used by this thread alone (&mut self)
        unsafe { ffi::lm_resid_set_chip_model(self.chip.as_ptr(), c_int::from(model == ChipModel::Mos6581)) }
    }

    /// Writes `value` to register `offset` (0x00–0x1f; on the MOS8580 the write lands on the next cycle clocked, as
    /// reSID fakes the chip's pipeline in `SAMPLE_FAST`)
    pub fn write(&mut self, offset: u8, value: u8) {
        // SAFETY: a live chip, used by this thread alone; the shim masks the offset to the register file
        unsafe { ffi::lm_resid_write(self.chip.as_ptr(), c_uint::from(offset), c_uint::from(value)) }
    }

    /// Clocks the chip for at most `*cycles` cycles, or until `out` is full; `*cycles` keeps the cycles not clocked.
    /// Returns the samples made (written at the start of `out`)
    pub fn clock(&mut self, cycles: &mut i32, out: &mut [i16]) -> usize {
        let n = c_int::try_from(out.len()).unwrap_or(c_int::MAX);
        let mut left: c_int = *cycles;
        // SAFETY: a live chip, used by this thread alone; `out` holds at least `n` samples and is written only up to
        // what is returned (reSID writes one sample per step, at most `n`)
        let made = unsafe { ffi::lm_resid_clock(self.chip.as_ptr(), &mut left, out.as_mut_ptr(), n) };
        *cycles = left;
        made.max(0) as usize
    }

    /// The chip's reset (registers, oscillators, envelopes, filters)
    pub fn reset(&mut self) {
        // SAFETY: a live chip, used by this thread alone
        unsafe { ffi::lm_resid_reset(self.chip.as_ptr()) }
    }

    /// Becomes a copy of `other` (its whole state; rates included), without allocating. The copy plays on exactly as
    /// `other` would, and `other` is not touched
    pub fn copy_from(&mut self, other: &Sid) {
        // SAFETY: two live chips (distinct: `&mut self` and `&other` cannot alias), each used by this thread alone;
        // the shim copies the members and points the copy's sync ring at its own voices
        unsafe { ffi::lm_resid_copy(self.chip.as_ptr(), other.chip.as_ptr()) };
        self.clock = other.clock;
        self.rate = other.rate;
    }

    /// A new chip in the same state (allocates: build time only). Key `resid.cannot_make` if memory ran out (the
    /// rates were accepted for `self`, so they are not the cause)
    pub fn try_clone(&self) -> Result<Self> {
        let mut copy = Self::new(self.clock, self.rate)?;
        copy.copy_from(self);
        Ok(copy)
    }

    /// reSID's version text
    pub fn version() -> &'static str {
        // SAFETY: a pointer to a static NUL-terminated string of the library
        unsafe { CStr::from_ptr(ffi::lm_resid_version()) }.to_str().unwrap_or("")
    }
}

impl Clone for Sid {
    /// A new chip (allocates: build time only) in the same state. [`Sid::try_clone`] reports a failure; this one
    /// aborts as every Rust allocation does when memory runs out (a panic here could only be caught by luck)
    fn clone(&self) -> Self {
        self.try_clone().unwrap_or_else(|_| std::alloc::handle_alloc_error(std::alloc::Layout::new::<Self>()))
    }

    /// In place, without allocating (audio thread)
    fn clone_from(&mut self, source: &Self) {
        self.copy_from(source);
    }
}

impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: the chip made by lm_resid_new, owned by this value alone, deleted once
        unsafe { ffi::lm_resid_delete(self.chip.as_ptr()) }
    }
}
