//! Safe binding to Neural Amp Modeler's core (NAM, Steven Atkinson, MIT), for the Amp & IR effect: a model read from
//! the bytes of a `.nam` file, set up for a rate and a largest block, then run on mono audio.
//!
//! The core is vendored (`vendor/nam`, the revision the legacy program built: 0b3d3c9), with the Eigen headers it
//! includes (`vendor/eigen`, MPL-2.0, the revision NAM pins) and nlohmann/json (`vendor/nlohmann`, MIT). Vendored,
//! not the system's: NAM pins an Eigen newer than Ubuntu 24.04's 3.4.0 (Homebrew has 5.0.1), and three Eigen
//! versions are three matrix kernels, so three slightly different outputs of the same model; the copy here is the
//! code the legacy program ran, which its A/B checks (decisoes-autonomas.md, "Do Amp & IR").
//!
//! The life of a model has two halves:
//!
//! 1. [`Model::load`] and [`Model::reset`]: while the plugin is **built**, off the audio thread. Loading parses the
//!    JSON and makes the network (allocating); resetting sets the rate and the largest block, allocates the
//!    buffers and prewarms the model.
//! 2. [`Model::process`]: on the audio thread, for blocks up to the reset's size.
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | `LmNamModel*` (the shim's box around NAM's `std::unique_ptr<nam::DSP>`): one [`Model`], freed by `lm_nam_free` in its `Drop`. Nothing else holds the pointer. |
//! | Which thread may call each function? | A `Model` is `Send` (a network is plain heap memory) and **not `Sync`**; everything that changes it takes `&mut self`. It is made in the build and moved to the audio thread; it goes back to the main thread to be dropped (rule 5). |
//! | Allocates, locks, does I/O? | `load` allocates, and may print to stderr (NAM warns there about a model of a partially supported version): build time only. `reset` allocates and prewarms (runs the network on silence): build time only. `process` neither allocates, frees, locks nor does I/O for blocks up to the reset's size (checked by `tests/no_alloc.rs` under `assert_no_alloc` and the C heap trap, for every architecture of the test models). `Drop` frees: never on the audio thread. The architectures' registry and the version checkers are static objects of the library, made before `main`; the version check takes NAM's mutex, at load only. |
//! | Creates threads? | No: Eigen is built without OpenMP, and NAM starts none. |
//! | Calls back into our code? | No. |
//! | Exceptions | No C++ exception crosses into Rust: the shim catches every one; a load or reset that throws is an [`Error`] with NAM's message, a `process` that throws (none does) gives silence. |
//! | Tests | `tests/model.rs` (the example models load, report their rate and loudness, process a sine to a finite, non-silent output, the same in any chunking of the reset's size; broken JSON, an unknown architecture, a stereo model are errors with NAM's message); `tests/no_alloc.rs` (process under `assert_no_alloc` and the C heap trap). ASan in CI. |

use lm_base::{Error, Result};
use std::ffi::{CStr, c_char, c_double, c_int};
use std::fmt;
use std::ptr::NonNull;

mod ffi {
    use super::{c_char, c_double, c_int};

    #[repr(C)]
    pub struct LmNamModel {
        _private: [u8; 0],
    }

    unsafe extern "C" {
        pub fn lm_nam_load(json: *const c_char, len: usize, error: *mut c_char, room: usize) -> *mut LmNamModel;
        pub fn lm_nam_free(model: *mut LmNamModel);
        pub fn lm_nam_input_channels(model: *const LmNamModel) -> c_int;
        pub fn lm_nam_output_channels(model: *const LmNamModel) -> c_int;
        pub fn lm_nam_expected_rate(model: *const LmNamModel) -> c_double;
        pub fn lm_nam_arbitrary_rate(model: *const LmNamModel) -> c_int;
        pub fn lm_nam_has_loudness(model: *const LmNamModel) -> c_int;
        pub fn lm_nam_loudness(model: *const LmNamModel, ok: *mut c_int) -> c_double;
        pub fn lm_nam_reset(
            model: *mut LmNamModel,
            rate: c_double,
            max_frames: c_int,
            error: *mut c_char,
            room: usize,
        ) -> c_int;
        pub fn lm_nam_process(model: *mut LmNamModel, input: *mut f32, output: *mut f32, frames: c_int) -> c_int;
    }
}

/// Room for NAM's messages (a longer one is cut)
const MESSAGE: usize = 512;

fn message(buffer: &[c_char; MESSAGE]) -> String {
    // SAFETY: the shim always writes a NUL-terminated string within the buffer (strncpy of room - 1, then a NUL at
    // the end), and the buffer starts zeroed, so a NUL is there even when nothing was written
    unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned()
}

/// A Neural Amp Modeler model (mono in, mono out)
pub struct Model {
    raw: NonNull<ffi::LmNamModel>,
    /// The most frames a [`Model::process`] takes (0 before [`Model::reset`])
    max_frames: usize,
}

// SAFETY: a model is heap memory (the network's weights and buffers) with no thread affinity and no thread-local
// state of its own; it is used by one thread at a time, which `&mut self` on every change guarantees (and `Model` is
// not `Sync`: the raw pointer keeps it so). The library's shared state (the registries) is only read after start-up,
// at load, under NAM's own mutex.
unsafe impl Send for Model {}

impl Model {
    /// Reads a model from the bytes of a `.nam` file (JSON). Keys: `nam.load` (NAM's reason in `message`: broken
    /// JSON, an unknown architecture, an unsupported version...), `nam.not_mono` (not one input and one output).
    /// Allocates: never on the audio thread
    pub fn load(bytes: &[u8]) -> Result<Self> {
        let mut error = [0 as c_char; MESSAGE];
        // SAFETY: `bytes` is valid for `len` bytes for the call (the shim parses from the range, it does not keep it);
        // `error` is a writable buffer of MESSAGE chars
        let raw = unsafe { ffi::lm_nam_load(bytes.as_ptr().cast(), bytes.len(), error.as_mut_ptr(), MESSAGE) };
        let Some(raw) = NonNull::new(raw) else {
            return Err(Error::new("nam.load").arg("message", message(&error)));
        };
        let model = Self { raw, max_frames: 0 };
        // SAFETY: the model is valid (just made, owned here)
        let (inputs, outputs) =
            unsafe { (ffi::lm_nam_input_channels(raw.as_ptr()), ffi::lm_nam_output_channels(raw.as_ptr())) };
        if inputs != 1 || outputs != 1 {
            return Err(Error::new("nam.not_mono").arg("inputs", i64::from(inputs)).arg("outputs", i64::from(outputs)));
        }
        Ok(model)
    }

    /// The rate it was trained at, Hz (`None` if the file does not say)
    pub fn expected_rate(&self) -> Option<f64> {
        // SAFETY: the model is valid and only read
        let rate = unsafe { ffi::lm_nam_expected_rate(self.raw.as_ptr()) };
        (rate > 0.0).then_some(rate)
    }

    /// Whether it can run at any rate (else it must run at [`Model::expected_rate`])
    pub fn supports_arbitrary_rate(&self) -> bool {
        // SAFETY: the model is valid and only read (the call is const in effect: a virtual that returns a constant)
        unsafe { ffi::lm_nam_arbitrary_rate(self.raw.as_ptr()) != 0 }
    }

    /// How loud it plays a standard input, dB (`None` if the file does not say)
    pub fn loudness(&self) -> Option<f64> {
        // SAFETY: the model is valid and only read; `ok` is a valid out pointer
        unsafe {
            if ffi::lm_nam_has_loudness(self.raw.as_ptr()) == 0 {
                return None;
            }
            let mut ok: c_int = 0;
            let loudness = ffi::lm_nam_loudness(self.raw.as_ptr(), &mut ok);
            (ok != 0).then_some(loudness)
        }
    }

    /// Sets the rate it runs at and the most frames a [`Model::process`] takes, and prewarms it. Key `nam.reset`.
    /// Allocates: never on the audio thread
    pub fn reset(&mut self, rate: f64, max_frames: usize) -> Result<()> {
        let frames = c_int::try_from(max_frames.max(1)).map_err(|_| Error::new("nam.reset").arg("message", "block"))?;
        let mut error = [0 as c_char; MESSAGE];
        // SAFETY: the model is valid and exclusively ours (&mut self); `error` is a writable buffer of MESSAGE chars
        let code = unsafe { ffi::lm_nam_reset(self.raw.as_ptr(), rate, frames, error.as_mut_ptr(), MESSAGE) };
        if code != 0 {
            return Err(Error::new("nam.reset").arg("message", message(&error)));
        }
        self.max_frames = max_frames.max(1);
        Ok(())
    }

    /// The most frames a [`Model::process`] takes
    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// Runs `io` through the model, in place, in pieces of at most [`Model::max_frames`]. Neither allocates, locks
    /// nor does I/O: for the audio thread. Before any [`Model::reset`], it gives silence
    pub fn process(&mut self, io: &mut [f32]) {
        if self.max_frames == 0 {
            io.fill(0.0);
            return;
        }
        for piece in io.chunks_mut(self.max_frames) {
            let frames = piece.len() as c_int; // (at most max_frames, which fitted a c_int)
            let p = piece.as_mut_ptr();
            // SAFETY: the model is valid and exclusively ours; it was reset for up to `max_frames` frames and the
            // piece is no longer; input and output are the same buffer of `frames` floats, which NAM supports (it
            // copies the input into its own buffers before writing the output, as the legacy program relied on)
            unsafe { ffi::lm_nam_process(self.raw.as_ptr(), p, p, frames) };
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // SAFETY: the pointer came from lm_nam_load, is owned by self alone and is freed exactly once, here
        unsafe { ffi::lm_nam_free(self.raw.as_ptr()) };
    }
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("expected_rate", &self.expected_rate())
            .field("loudness", &self.loudness())
            .field("max_frames", &self.max_frames)
            .finish()
    }
}
