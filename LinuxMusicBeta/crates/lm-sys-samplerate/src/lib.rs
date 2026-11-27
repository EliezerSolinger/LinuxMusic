//! Safe binding to libsamplerate (Secret Rabbit Code), the system's library found by pkg-config.
//!
//! Two uses:
//!
//! - [`convert`]: a whole buffer at once, for imports. The project rate is fixed and an imported file is converted
//!   once, at the best quality, inside an `Operation` (spec item 8, interfaces.md I-12). Cancellable, with progress,
//!   and the result has exactly [`output_frames`] frames, so the length of an imported file is known before the
//!   work starts.
//! - [`Resampler`]: a streaming converter for the device edge (a device at another rate than the project, or the
//!   drift correction between two cards, U7). Made off the audio thread; [`Resampler::process`] neither allocates
//!   nor locks, and the ratio may change from block to block (the change is spread over the block).
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | `SRC_STATE*`: one [`Resampler`], freed by `src_delete` in its `Drop`. [`convert`] makes its own `Resampler` and drops it before returning. Nothing else holds a C pointer. |
//! | Which thread may call each function? | A `Resampler` is `Send` (a state is plain memory, used by one thread at a time) and **not `Sync`**; every call that touches the state takes `&mut self`, so two threads can never use it at once. It is made on a preparing thread and moved to the audio thread; it must come back (the return ring, rule 5) to be dropped. |
//! | Allocates, locks, does I/O? | `Resampler::new` allocates (the filter state) and [`latency`](Resampler::latency) is measured there. `process`, `finish`, `set_ratio`, `set_ratio_now` and `reset` do not allocate, lock or do I/O (checked by `tests/no_alloc.rs`, which counts the C heap as well as Rust's) and may run on the audio thread. `Drop` frees: never on the audio thread. [`convert`] allocates the output and is for an `Operation`. |
//! | Creates threads? | No. |
//! | Calls back into our code? | No: the callback API (`src_callback_new`) is not used. |
//! | Tests | `tests/convert.rs` (length exact for every common rate pair, a sine keeps its frequency and level, the alias rejection of the best quality, cancel and progress, identical rates are a bit-exact copy), `tests/stream.rs` (streaming equals one-shot, ratio changes, latency measured matches an impulse), `tests/no_alloc.rs` (process under `assert_no_alloc` and the C heap counter). ASan in CI. |

use lm_base::{Cancel, Error, Progress, Result};
use std::ffi::{CStr, c_char, c_double, c_float, c_int, c_long};
use std::fmt;
use std::ptr::NonNull;

mod ffi {
    use super::{c_char, c_double, c_float, c_int, c_long};

    #[repr(C)]
    pub struct SrcState {
        _private: [u8; 0],
    }

    /// `SRC_DATA` of samplerate.h
    #[repr(C)]
    pub struct SrcData {
        pub data_in: *const c_float,
        pub data_out: *mut c_float,
        pub input_frames: c_long,
        pub output_frames: c_long,
        pub input_frames_used: c_long,
        pub output_frames_gen: c_long,
        pub end_of_input: c_int,
        pub src_ratio: c_double,
    }

    unsafe extern "C" {
        pub fn src_new(converter_type: c_int, channels: c_int, error: *mut c_int) -> *mut SrcState;
        pub fn src_delete(state: *mut SrcState) -> *mut SrcState;
        pub fn src_process(state: *mut SrcState, data: *mut SrcData) -> c_int;
        pub fn src_set_ratio(state: *mut SrcState, new_ratio: c_double) -> c_int;
        pub fn src_reset(state: *mut SrcState) -> c_int;
        pub safe fn src_is_valid_ratio(ratio: c_double) -> c_int;
        pub safe fn src_strerror(error: c_int) -> *const c_char;
        pub safe fn src_get_version() -> *const c_char;
    }
}

/// The converter, from the best (slowest) to the plainest
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    /// Band-limited sinc, 97 dB of rejection: imports and export
    Best,
    /// Band-limited sinc, 97 dB, narrower band than `Best`
    Medium,
    /// Band-limited sinc, 97 dB, the narrowest band: a device edge that must be cheap
    Fastest,
    /// Not band-limited, and trails the input by one input frame: tests and special cases only
    ZeroOrderHold,
    /// Not band-limited, and trails the input by one input frame: tests and special cases only
    Linear,
}

impl Quality {
    fn converter(self) -> c_int {
        match self {
            Self::Best => 0,
            Self::Medium => 1,
            Self::Fastest => 2,
            Self::ZeroOrderHold => 3,
            Self::Linear => 4,
        }
    }
}

/// The version string of the linked library ("libsamplerate-0.2.2 ...")
pub fn version() -> &'static str {
    let text = ffi::src_get_version();
    if text.is_null() {
        return "";
    }
    // SAFETY: src_get_version returns a pointer to a static NUL-terminated string that lives as long as the library
    unsafe { CStr::from_ptr(text) }.to_str().unwrap_or("")
}

fn library_error(key: &'static str, code: c_int) -> Error {
    let text = ffi::src_strerror(code);
    let message = if text.is_null() {
        String::new()
    } else {
        // SAFETY: src_strerror returns a pointer into a static table of NUL-terminated strings (or NULL, handled)
        unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
    };
    Error::new(key).arg("code", i64::from(code)).arg("message", message)
}

/// The ratio (output rate / input rate) libsamplerate accepts: 1/256 to 256
pub fn is_valid_ratio(ratio: f64) -> bool {
    ratio.is_finite() && ffi::src_is_valid_ratio(ratio) != 0
}

/// The number of frames [`convert`] returns for `input_frames` at `from_rate` converted to `to_rate`:
/// `input_frames * to_rate / from_rate`, rounded to the nearest frame (half up), in exact integer arithmetic
pub fn output_frames(input_frames: usize, from_rate: u32, to_rate: u32) -> usize {
    if from_rate == 0 {
        return 0;
    }
    let scaled = input_frames as u128 * u128::from(to_rate);
    let from = u128::from(from_rate);
    ((scaled + from / 2) / from) as usize
}

fn check_rates(from_rate: u32, to_rate: u32) -> Result<f64> {
    let ratio = f64::from(to_rate) / f64::from(from_rate);
    if from_rate == 0 || to_rate == 0 || !is_valid_ratio(ratio) {
        return Err(Error::new("samplerate.bad_ratio").arg("from", i64::from(from_rate)).arg("to", i64::from(to_rate)));
    }
    Ok(ratio)
}

/// The most channels a converter takes (a probe buffer is allocated per channel)
pub const MAX_CHANNELS: usize = 64;

fn check_channels(channels: usize) -> Result<c_int> {
    match c_int::try_from(channels) {
        Ok(c) if c > 0 && channels <= MAX_CHANNELS => Ok(c),
        _ => Err(Error::new("samplerate.bad_channels").arg("channels", channels as i64)),
    }
}

/// How much input [`convert`] gives libsamplerate per call: big enough to be efficient, small enough for the cancel
/// and the progress to answer within a fraction of a second
const CONVERT_BLOCK_FRAMES: usize = 1 << 16;

/// Converts an interleaved buffer of `channels` channels from `from_rate` to `to_rate`, at `quality`, all at once.
///
/// Returns exactly [`output_frames`] frames (the converter's own count can differ by a frame at the end; the tail is
/// trimmed or completed with silence). Identical rates return a bit-exact copy. Allocates the output: run it in an
/// `Operation`, never on the audio thread. Checks `cancel` and reports to `progress` every block of 65 536 frames.
///
/// Errors: `samplerate.bad_channels`, `samplerate.bad_length` (the buffer is not whole frames),
/// `samplerate.bad_ratio`, `samplerate.new_failed`, `samplerate.process_failed`, `error.cancelled`.
pub fn convert(
    input: &[f32],
    channels: usize,
    from_rate: u32,
    to_rate: u32,
    quality: Quality,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Vec<f32>> {
    check_channels(channels)?;
    if !input.len().is_multiple_of(channels) {
        return Err(Error::new("samplerate.bad_length")
            .arg("samples", input.len() as i64)
            .arg("channels", channels as i64));
    }
    let ratio = check_rates(from_rate, to_rate)?;
    cancel.check()?;
    let in_frames = input.len() / channels;
    let want_frames = output_frames(in_frames, from_rate, to_rate);
    if from_rate == to_rate {
        progress.set(1.0);
        return Ok(input.to_vec());
    }

    let mut resampler = Resampler::new(quality, channels, ratio)?;
    // The converter may give a frame or two more than the exact count before it is told the input ended
    let mut output = vec![0.0_f32; (want_frames + 64) * channels];
    let mut written = 0; // frames
    let mut consumed = 0; // frames
    while consumed < in_frames {
        cancel.check()?;
        let end = (consumed + CONVERT_BLOCK_FRAMES).min(in_frames);
        let block = &input[consumed * channels..end * channels];
        let mut offset = 0;
        while offset < block.len() / channels {
            if written * channels >= output.len() {
                output.resize(output.len() + CONVERT_BLOCK_FRAMES * channels, 0.0);
            }
            let done = resampler.process(&block[offset * channels..], &mut output[written * channels..])?;
            offset += done.input_frames;
            written += done.output_frames;
            if done.input_frames == 0 && done.output_frames == 0 {
                // Neither side moved: the output must be full; the loop grows it
                output.resize(output.len() + CONVERT_BLOCK_FRAMES * channels, 0.0);
            }
        }
        consumed = end;
        progress.set(consumed as f32 / in_frames as f32);
    }
    // The tail the filter still holds
    loop {
        if written * channels >= output.len() {
            output.resize(output.len() + CONVERT_BLOCK_FRAMES * channels, 0.0);
        }
        let done = resampler.finish(&mut output[written * channels..])?;
        written += done.output_frames;
        if done.output_frames == 0 {
            break;
        }
    }
    output.truncate(written.min(want_frames) * channels);
    output.resize(want_frames * channels, 0.0);
    progress.set(1.0);
    Ok(output)
}

/// What one call to [`Resampler::process`] did, in frames
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Processed {
    /// Input frames consumed (the rest must be given again in the next call)
    pub input_frames: usize,
    /// Output frames written at the start of the output buffer
    pub output_frames: usize,
}

/// A streaming converter: one `SRC_STATE`, for interleaved `f32` of a fixed channel count.
///
/// Made off the audio thread ([`new`](Self::new) allocates and measures the latency). On the audio thread,
/// [`process`](Self::process) and the ratio setters neither allocate nor lock. `Send` but not `Sync`
pub struct Resampler {
    state: NonNull<ffi::SrcState>,
    channels: usize,
    quality: Quality,
    ratio: f64,
    latency: f64,
}

// SAFETY: an SRC_STATE is plain heap memory with no thread affinity and no thread-local state; libsamplerate keeps no
// global mutable state. It may be used from any thread as long as only one thread uses it at a time, which `&mut self`
// on every use guarantees (and `Resampler` is not `Sync`: the raw pointer field keeps it so).
unsafe impl Send for Resampler {}

impl Resampler {
    /// A converter for `channels` interleaved channels at `ratio` (output rate / input rate). Allocates: never on the
    /// audio thread
    pub fn new(quality: Quality, channels: usize, ratio: f64) -> Result<Self> {
        let c_channels = check_channels(channels)?;
        if !is_valid_ratio(ratio) {
            return Err(Error::new("samplerate.bad_ratio").arg("ratio", ratio));
        }
        let mut error: c_int = 0;
        // SAFETY: `error` is a valid out pointer; the converter and channel count were validated (libsamplerate checks
        // them too and returns NULL with an error code)
        let raw = unsafe { ffi::src_new(quality.converter(), c_channels, &mut error) };
        let Some(state) = NonNull::new(raw) else {
            return Err(library_error("samplerate.new_failed", error));
        };
        let mut resampler = Self { state, channels, quality, ratio, latency: 0.0 };
        resampler.latency = resampler.measure_latency()?;
        resampler.reset()?;
        Ok(resampler)
    }

    /// How many input frames the converter holds back before its output catches up (its delay, in input frames, at
    /// the ratio it was made with): after `n` input frames, about `(n - latency) * ratio` output frames have come
    /// out. With the sinc converters the output is aligned with the input (no group delay): the latency is only how far the output trails in
    /// a stream. Measured once at construction
    pub fn latency(&self) -> f64 {
        self.latency
    }

    fn measure_latency(&mut self) -> Result<f64> {
        const PROBE: usize = 1 << 14;
        let input = vec![0.0_f32; PROBE * self.channels];
        let mut output = vec![0.0_f32; (PROBE as f64 * self.ratio) as usize * self.channels + 64 * self.channels];
        let (mut used, mut made) = (0, 0);
        while used < PROBE {
            let done = self.process(&input[used * self.channels..], &mut output[made * self.channels..])?;
            used += done.input_frames;
            made += done.output_frames;
            if done.input_frames == 0 && done.output_frames == 0 {
                break;
            }
        }
        Ok((used as f64 - made as f64 / self.ratio).max(0.0))
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// The ratio the next [`process`](Self::process) ends at
    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    /// Sets the ratio for the next [`process`](Self::process): the converter glides from the ratio it ended the last
    /// block at to this one, linearly over that block (no click; drift correction). libsamplerate spreads the glide
    /// over the **whole output slice** given to `process`, not over what it writes: give an output slice of about the
    /// size the block will produce (input frames × ratio) for the glide to end where expected. Does not allocate
    pub fn set_ratio(&mut self, ratio: f64) -> Result<()> {
        if !is_valid_ratio(ratio) {
            return Err(Error::new("samplerate.bad_ratio"));
        }
        self.ratio = ratio;
        Ok(())
    }

    /// Sets the ratio as a step, with no glide. Does not allocate
    pub fn set_ratio_now(&mut self, ratio: f64) -> Result<()> {
        if !is_valid_ratio(ratio) {
            return Err(Error::new("samplerate.bad_ratio"));
        }
        // SAFETY: the state is valid (owned by self) and used by this thread only (&mut self)
        let code = unsafe { ffi::src_set_ratio(self.state.as_ptr(), ratio) };
        if code != 0 {
            return Err(Error::new("samplerate.process_failed"));
        }
        self.ratio = ratio;
        Ok(())
    }

    /// Forgets the signal held in the filter (after a stop or a locate). Does not allocate
    pub fn reset(&mut self) -> Result<()> {
        // SAFETY: the state is valid (owned by self) and used by this thread only (&mut self)
        let code = unsafe { ffi::src_reset(self.state.as_ptr()) };
        if code != 0 {
            return Err(Error::new("samplerate.process_failed"));
        }
        Ok(())
    }

    /// Converts as much of `input` as fits in `output` (both interleaved, whole frames; a partial frame at the end
    /// of either is ignored). Returns how many frames were consumed and written; the input not consumed must be
    /// given again. Does not allocate, lock or do I/O: for the audio thread. The error carries no arguments, so that
    /// making it does not allocate either
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<Processed> {
        self.run(input, output, false)
    }

    /// Tells the converter the input has ended and writes the tail it still holds. Call until it writes nothing.
    /// Does not allocate
    pub fn finish(&mut self, output: &mut [f32]) -> Result<Processed> {
        self.run(&[], output, true)
    }

    fn run(&mut self, input: &[f32], output: &mut [f32], end_of_input: bool) -> Result<Processed> {
        let in_frames = input.len() / self.channels;
        let out_frames = output.len() / self.channels;
        debug_assert!(input.len().is_multiple_of(self.channels), "the input is not whole frames");
        debug_assert!(output.len().is_multiple_of(self.channels), "the output is not whole frames");
        let (Ok(input_frames), Ok(output_frames)) = (c_long::try_from(in_frames), c_long::try_from(out_frames)) else {
            return Err(Error::new("samplerate.process_failed"));
        };
        let mut data = ffi::SrcData {
            data_in: input.as_ptr(),
            data_out: output.as_mut_ptr(),
            input_frames,
            output_frames,
            input_frames_used: 0,
            output_frames_gen: 0,
            end_of_input: c_int::from(end_of_input),
            src_ratio: self.ratio,
        };
        // SAFETY: the state is valid and exclusively ours (&mut self). `data_in` points to `input_frames` whole frames
        // of `channels` floats that stay borrowed for the call, `data_out` to `output_frames` whole frames of a
        // mutable borrow; libsamplerate reads and writes only within those counts. The input and output never
        // overlap (a shared and a mutable borrow cannot)
        let code = unsafe { ffi::src_process(self.state.as_ptr(), &mut data) };
        if code != 0 {
            return Err(Error::new("samplerate.process_failed"));
        }
        Ok(Processed {
            // Never more than was offered, whatever the library says (the callers slice by these counts)
            input_frames: usize::try_from(data.input_frames_used).unwrap_or(0).min(in_frames),
            output_frames: usize::try_from(data.output_frames_gen).unwrap_or(0).min(out_frames),
        })
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        // SAFETY: the state came from src_new, is owned by self alone and is freed exactly once, here
        unsafe { ffi::src_delete(self.state.as_ptr()) };
    }
}

impl fmt::Debug for Resampler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resampler")
            .field("quality", &self.quality)
            .field("channels", &self.channels)
            .field("ratio", &self.ratio)
            .field("latency", &self.latency)
            .finish()
    }
}
