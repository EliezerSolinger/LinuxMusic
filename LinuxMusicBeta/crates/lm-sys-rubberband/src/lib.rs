//! Safe binding to the Rubber Band library (time-stretch and pitch-shift), the system's library found by pkg-config.
//!
//! Two uses:
//!
//! - [`Offline`] and [`stretch`]: the time-stretch of an audio item, rendered **off the audio thread** into a new
//!   resource (decisoes-autonomas.md: recorded items are `musical` and follow the tempo by time-stretch; the original
//!   file is always kept, and the stretched one is rebuilt from it). Two passes, as the library's offline mode
//!   wants: [`Offline::study`] over the whole input, then [`Offline::process`] over the whole input again. The
//!   output length is **exactly** the one asked for (the caller computes it from the tempo map, so it is exact in
//!   samples, never a rounded ratio); cancellable and with progress between chunks. Interleaved, any channel count.
//! - [`Realtime`]: the library's real-time mode, for a future live preview (a tempo or pitch change heard before the
//!   offline render ends). Planar (one slice per channel), like the render's buffers.
//!
//! The engine is R3 ("finer", [`Engine::Finer`], the default) or R2 ("faster"); every library this crate accepts
//! (3.0 or later) has both, and [`Offline::engine_version`] says which one runs.
//!
//! # Safety sheet (ficha, interfaces.md I-17)
//!
//! | Question | Answer |
//! |---|---|
//! | Who owns each handle? | `RubberBandState`: one private `Handle`, inside one [`Offline`] or one [`Realtime`], freed by `rubberband_delete` in its `Drop`. [`stretch`] makes its own `Offline` and drops it before returning. Nothing else holds a C pointer. |
//! | Which thread may call each function? | A stretcher is a C++ object with no thread affinity and no thread-local state: `Offline` and `Realtime` are `Send` and **not `Sync`**, and every call that touches the state takes `&mut self`. Creating and deleting a stretcher take this crate's lock (the library's FFT setup is process-wide), so several `Offline`s may run at once on several worker threads (one item each). |
//! | Allocates, locks, does I/O? | `Offline` allocates throughout (it is for an `Operation`). `Realtime::new` allocates (the whole state, sized for `max_block`) and takes the crate lock. On `Realtime`, `process`, `retrieve`, `available`, `samples_required` and `start_delay` neither allocate nor lock nor do I/O (checked by `tests/no_alloc.rs` with the C heap trap, R2 and R3, both formant modes, pitch 1 and 1.25) and may run on the audio thread. `set_time_ratio` is real-time safe on **both engines** (a ratio gliding every block, checked by the same test; R2's `process` after a ratio change no longer touches the heap). **`set_pitch_scale` and `reset` are not real-time safe**: R3 reallocates its resampler on a pitch change and on reset (measured by the same test); call them off the audio thread, or make a new `Realtime` with the new pitch. `Drop` frees and locks: never on the audio thread. No I/O anywhere; the library writes its error messages (debug level 0) to stderr. |
//! | Creates threads? | No: every stretcher is made with `OptionThreadingNever` (R2 would otherwise start one thread per channel in offline mode). Parallelism is per item, by the caller's `Workers`. |
//! | Calls back into our code? | No. A C++ exception escaping the library (only `std::bad_alloc` is possible) cannot unwind through `extern "C"`: the process aborts, as on any out-of-memory. |
//! | Tests | `tests/offline.rs` (a stretched sine keeps its pitch within 0.5 % and gets exactly the length asked for, R3 and R2, mono and stereo; a pitch shift moves the frequency by the scale; formant preservation runs; cancel before and during; progress reaches 1; key frames put each click where the map says; chunked feeding equals one call; bad arguments are keyed errors), `tests/realtime.rs` (stream keeps the pitch, the start delay is reported), `tests/no_alloc.rs` (the real-time calls under `assert_no_alloc` and the C heap trap). ASan in CI. |

use lm_base::{Cancel, Error, Progress, Result};
use std::ffi::{c_double, c_float, c_int, c_uint};
use std::fmt;
use std::ptr::NonNull;
use std::sync::{Mutex, PoisonError};

mod ffi {
    use super::{c_double, c_float, c_int, c_uint};

    #[repr(C)]
    pub struct RubberBandState {
        _private: [u8; 0],
    }

    pub const PROCESS_OFFLINE: c_int = 0x0000_0000;
    pub const PROCESS_REAL_TIME: c_int = 0x0000_0001;
    pub const TRANSIENTS_CRISP: c_int = 0x0000_0000;
    pub const TRANSIENTS_MIXED: c_int = 0x0000_0100;
    pub const TRANSIENTS_SMOOTH: c_int = 0x0000_0200;
    pub const THREADING_NEVER: c_int = 0x0001_0000;
    pub const WINDOW_STANDARD: c_int = 0x0000_0000;
    pub const WINDOW_SHORT: c_int = 0x0010_0000;
    pub const WINDOW_LONG: c_int = 0x0020_0000;
    pub const FORMANT_SHIFTED: c_int = 0x0000_0000;
    pub const FORMANT_PRESERVED: c_int = 0x0100_0000;
    pub const PITCH_HIGH_QUALITY: c_int = 0x0200_0000;
    pub const PITCH_HIGH_CONSISTENCY: c_int = 0x0400_0000;
    pub const CHANNELS_APART: c_int = 0x0000_0000;
    pub const CHANNELS_TOGETHER: c_int = 0x1000_0000;
    pub const ENGINE_FASTER: c_int = 0x0000_0000;
    pub const ENGINE_FINER: c_int = 0x2000_0000;

    unsafe extern "C" {
        pub fn rubberband_new(
            sample_rate: c_uint,
            channels: c_uint,
            options: c_int,
            initial_time_ratio: c_double,
            initial_pitch_scale: c_double,
        ) -> *mut RubberBandState;
        pub fn rubberband_delete(state: *mut RubberBandState);
        pub fn rubberband_reset(state: *mut RubberBandState);
        pub fn rubberband_get_engine_version(state: *mut RubberBandState) -> c_int;
        pub fn rubberband_set_time_ratio(state: *mut RubberBandState, ratio: c_double);
        pub fn rubberband_set_pitch_scale(state: *mut RubberBandState, scale: c_double);
        pub fn rubberband_get_preferred_start_pad(state: *const RubberBandState) -> c_uint;
        pub fn rubberband_get_start_delay(state: *const RubberBandState) -> c_uint;
        pub fn rubberband_set_expected_input_duration(state: *mut RubberBandState, samples: c_uint);
        pub fn rubberband_get_samples_required(state: *const RubberBandState) -> c_uint;
        pub fn rubberband_set_max_process_size(state: *mut RubberBandState, samples: c_uint);
        pub fn rubberband_set_key_frame_map(
            state: *mut RubberBandState,
            keyframecount: c_uint,
            from: *mut c_uint,
            to: *mut c_uint,
        );
        pub fn rubberband_study(
            state: *mut RubberBandState,
            input: *const *const c_float,
            samples: c_uint,
            final_: c_int,
        );
        pub fn rubberband_process(
            state: *mut RubberBandState,
            input: *const *const c_float,
            samples: c_uint,
            final_: c_int,
        );
        pub fn rubberband_available(state: *const RubberBandState) -> c_int;
        pub fn rubberband_retrieve(
            state: *const RubberBandState,
            output: *const *mut c_float,
            samples: c_uint,
        ) -> c_uint;
    }
}

/// The library's version, as pkg-config reported it at build time ("4.0.0")
pub fn version() -> &'static str {
    env!("LM_SYS_LINKED_VERSION")
}

/// Which of the library's two engines
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Engine {
    /// R3: the better sound, at a higher CPU cost. The default
    #[default]
    Finer,
    /// R2: cheaper, and the only one where [`Transients`] and the long window apply. Meant for moderate ratios:
    /// measured on a sine, a 4x stretch comes out about 10 dB quieter (R3 keeps the level)
    Faster,
}

/// What a pitch shift does to the formants (the spectral envelope)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Formants {
    /// They move with the pitch (the "chipmunk" effect on voices): the default, and the right one for instruments
    #[default]
    Shifted,
    /// They stay where they were: a voice shifted up or down still sounds like the same person
    Preserved,
}

/// How the channels of a multichannel input are treated
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Channels {
    /// Analysed together: the stereo image and the mono sum are kept (the phase between the channels is coherent).
    /// The default
    #[default]
    Together,
    /// Each channel on its own: the best quality per channel, at the cost of the image
    Apart,
}

/// The analysis window
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Window {
    /// The library's default
    #[default]
    Standard,
    /// Shorter: sharper transients and less delay in real time, less low-frequency resolution (R2 and R3)
    Short,
    /// Longer: smoother for pads and sustained material (R2 only; R3 uses its standard window)
    Long,
}

/// How R2 treats transients (ignored by R3, which handles them on its own).
///
/// `Crisp` resets the phase at every onset R2 detects. On steady tones (a held note, a pad, a test sine) the detector
/// misfires and the pitch wobbles by a percent or two (measured: 440 Hz shifted an octave down came out between
/// 215 and 222 Hz): choose `Smooth` for sustained material on R2, or use R3
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Transients {
    /// Kept sharp: the default, best for drums and mixed material
    #[default]
    Crisp,
    /// Sharp only in the high frequencies
    Mixed,
    /// Not treated: best for smooth material (strings, pads)
    Smooth,
}

/// The sound of a stretcher. The [`Default`] is R3, formants shifted, channels together, the standard window
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Settings {
    pub engine: Engine,
    pub formants: Formants,
    pub channels: Channels,
    pub window: Window,
    pub transients: Transients,
}

impl Settings {
    fn options(&self) -> c_int {
        let engine = match self.engine {
            Engine::Finer => ffi::ENGINE_FINER,
            Engine::Faster => ffi::ENGINE_FASTER,
        };
        let formants = match self.formants {
            Formants::Shifted => ffi::FORMANT_SHIFTED,
            Formants::Preserved => ffi::FORMANT_PRESERVED,
        };
        let channels = match self.channels {
            Channels::Together => ffi::CHANNELS_TOGETHER,
            Channels::Apart => ffi::CHANNELS_APART,
        };
        let window = match self.window {
            Window::Standard => ffi::WINDOW_STANDARD,
            Window::Short => ffi::WINDOW_SHORT,
            Window::Long => ffi::WINDOW_LONG,
        };
        let transients = match self.transients {
            Transients::Crisp => ffi::TRANSIENTS_CRISP,
            Transients::Mixed => ffi::TRANSIENTS_MIXED,
            Transients::Smooth => ffi::TRANSIENTS_SMOOTH,
        };
        engine | formants | channels | window | transients | ffi::THREADING_NEVER
    }
}

/// The smallest and the largest time ratio accepted (output length / input length)
pub const TIME_RATIO_RANGE: (f64, f64) = (1.0 / 64.0, 64.0);
/// The smallest and the largest pitch scale accepted (output frequency / input frequency): four octaves each way
pub const PITCH_SCALE_RANGE: (f64, f64) = (1.0 / 16.0, 16.0);
/// The most channels a stretcher takes
pub const MAX_CHANNELS: usize = 64;

/// The output length of `input_frames` stretched by `time_ratio`: the product, rounded to the nearest frame (half up).
/// A caller that knows the exact target length (from the tempo map) passes it to [`Offline::new`] directly instead
pub fn frames_for_ratio(input_frames: usize, time_ratio: f64) -> usize {
    (input_frames as f64 * time_ratio + 0.5).floor().max(0.0) as usize
}

fn in_range(value: f64, (low, high): (f64, f64)) -> bool {
    value.is_finite() && value >= low && value <= high
}

fn check_pitch(pitch_scale: f64) -> Result<()> {
    if in_range(pitch_scale, PITCH_SCALE_RANGE) { Ok(()) } else { Err(Error::new("rubberband.bad_pitch")) }
}

fn check_time_ratio(time_ratio: f64) -> Result<()> {
    if in_range(time_ratio, TIME_RATIO_RANGE) { Ok(()) } else { Err(Error::new("rubberband.bad_ratio")) }
}

/// The sample rate and the channel count of a stream
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Format {
    pub sample_rate: u32,
    pub channels: usize,
}

impl Format {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        Self { sample_rate, channels }
    }

    fn check(self) -> Result<()> {
        if self.sample_rate == 0 {
            return Err(Error::new("rubberband.bad_rate").arg("rate", i64::from(self.sample_rate)));
        }
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return Err(Error::new("rubberband.bad_channels").arg("channels", self.channels as i64));
        }
        Ok(())
    }
}

/// Making and deleting stretchers is serialised: the library sets up its FFT tables process-wide on first use
static LIFECYCLE: Mutex<()> = Mutex::new(());

/// One `RubberBandState`, owned alone, deleted once
struct Handle(NonNull<ffi::RubberBandState>);

// SAFETY: a stretcher is a C++ object with no thread affinity and no thread-local state, used by one thread at a time:
// every wrapper call takes `&mut self` (or `&self` for the read-only getters, which the wrappers never share across
// threads: `Offline` and `Realtime` are not `Sync`, the raw pointer keeps them so)
unsafe impl Send for Handle {}

impl Handle {
    fn new(format: Format, options: c_int, time_ratio: f64, pitch_scale: f64) -> Result<Self> {
        let Format { sample_rate, channels } = format;
        let _guard = LIFECYCLE.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: plain values, validated by the callers (rate > 0, 1..=64 channels, ratio and scale finite and in
        // range); the library returns NULL or a valid state
        let raw = unsafe { ffi::rubberband_new(sample_rate, channels as c_uint, options, time_ratio, pitch_scale) };
        NonNull::new(raw).map(Self).ok_or_else(|| Error::new("rubberband.new_failed"))
    }

    fn ptr(&self) -> *mut ffi::RubberBandState {
        self.0.as_ptr()
    }

    fn engine_version(&self) -> u32 {
        // SAFETY: the state is valid (owned by self); the call only reads it
        let version = unsafe { ffi::rubberband_get_engine_version(self.ptr()) };
        u32::try_from(version).unwrap_or(0)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _guard = LIFECYCLE.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: the state came from rubberband_new, is owned by self alone and is freed exactly once, here
        unsafe { ffi::rubberband_delete(self.ptr()) };
    }
}

/// How many frames the offline stretcher is given per call: the cancel and the progress are checked between them
const OFFLINE_BLOCK: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pass {
    Study,
    Process,
    Finished,
}

/// An offline stretch of one item: study the whole input, process the whole input again, [`finish`](Self::finish).
///
/// Interleaved `f32` of a fixed channel count. Every call allocates: for an `Operation`, never the audio thread.
/// The output is exactly `output_frames` long: the library's own count may differ by a few frames at the end, so the
/// tail is trimmed or completed with silence ([`library_frames`](Self::library_frames) tells how many it made).
///
/// ```text
/// let mut s = Offline::new(Format::new(48_000, 2), input_frames, output_frames, 1.0, &Settings::default())?;
/// for chunk in source.chunks(...) { s.study(chunk)?; }            // the whole input, in order
/// for chunk in source.chunks(...) { s.process(chunk, &mut out)?; } // the whole input again
/// s.finish(&mut out)?;                                             // out.len() == output_frames * channels
/// ```
pub struct Offline {
    handle: Handle,
    channels: usize,
    input_frames: usize,
    output_frames: usize,
    pitch_scale: f64,
    settings: Settings,
    pass: Pass,
    studied: usize,
    processed: usize,
    /// Frames appended to the caller's output so far (never more than `output_frames`)
    emitted: usize,
    /// Frames the library made in all (may exceed `output_frames`; the excess is dropped)
    made: usize,
    planar_in: Vec<Vec<f32>>,
    planar_out: Vec<Vec<f32>>,
}

impl Offline {
    /// A stretcher that turns `input_frames` frames of the `format`'s interleaved channels into exactly
    /// `output_frames` frames, shifting the pitch by `pitch_scale` (1 = unchanged, 2 = an octave up).
    ///
    /// Errors: `rubberband.bad_rate`, `rubberband.bad_channels`, `rubberband.bad_pitch`, `rubberband.bad_ratio`
    /// (the time ratio `output_frames / input_frames` is outside [`TIME_RATIO_RANGE`], or one length is zero and the
    /// other is not), `rubberband.too_long` (more than 2³² − 1 frames), `rubberband.new_failed`.
    pub fn new(
        format: Format,
        input_frames: usize,
        output_frames: usize,
        pitch_scale: f64,
        settings: &Settings,
    ) -> Result<Self> {
        format.check()?;
        let channels = format.channels;
        check_pitch(pitch_scale)?;
        if (input_frames == 0) != (output_frames == 0) {
            return Err(Error::new("rubberband.bad_ratio")
                .arg("input_frames", input_frames as i64)
                .arg("output_frames", output_frames as i64));
        }
        let time_ratio = if input_frames == 0 { 1.0 } else { output_frames as f64 / input_frames as f64 };
        check_time_ratio(time_ratio)
            .map_err(|e| e.arg("input_frames", input_frames as i64).arg("output_frames", output_frames as i64))?;
        let (Ok(duration), Ok(_)) = (c_uint::try_from(input_frames), c_uint::try_from(output_frames)) else {
            return Err(Error::new("rubberband.too_long").arg("frames", input_frames.max(output_frames) as i64));
        };
        // High quality: the best resampler for the pitch shift (R2; R3 always uses its own)
        let options = settings.options() | ffi::PROCESS_OFFLINE | ffi::PITCH_HIGH_QUALITY;
        let handle = Handle::new(format, options, time_ratio, pitch_scale)?;
        // SAFETY: the state is valid and exclusively ours; both calls only store a number, before any study
        unsafe {
            ffi::rubberband_set_expected_input_duration(handle.ptr(), duration);
            ffi::rubberband_set_max_process_size(handle.ptr(), OFFLINE_BLOCK as c_uint);
        }
        Ok(Self {
            handle,
            channels,
            input_frames,
            output_frames,
            pitch_scale,
            settings: *settings,
            pass: Pass::Study,
            studied: 0,
            processed: 0,
            emitted: 0,
            made: 0,
            planar_in: vec![vec![0.0; OFFLINE_BLOCK]; channels],
            planar_out: vec![vec![0.0; OFFLINE_BLOCK]; channels],
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn input_frames(&self) -> usize {
        self.input_frames
    }

    /// The exact number of frames [`finish`](Self::finish) leaves in the output
    pub fn output_frames(&self) -> usize {
        self.output_frames
    }

    pub fn pitch_scale(&self) -> f64 {
        self.pitch_scale
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The engine that runs: 3 (R3, finer) or 2 (R2, faster)
    pub fn engine_version(&self) -> u32 {
        self.handle.engine_version()
    }

    /// How many frames the library itself made (known after [`finish`](Self::finish)); the output is trimmed or padded
    /// from this to [`output_frames`](Self::output_frames)
    pub fn library_frames(&self) -> usize {
        self.made
    }

    /// Frames given to [`study`](Self::study) and to [`process`](Self::process) so far
    pub fn position(&self) -> (usize, usize) {
        (self.studied, self.processed)
    }

    /// The fraction of the work done: the study pass counts for the first fifth
    pub fn fraction(&self) -> f32 {
        if self.input_frames == 0 {
            return if self.pass == Pass::Finished { 1.0 } else { 0.0 };
        }
        let total = self.input_frames as f64;
        (0.2 * self.studied as f64 / total + 0.8 * self.processed as f64 / total) as f32
    }

    fn check_chunk(&self, chunk: &[f32], done: usize) -> Result<usize> {
        if !chunk.len().is_multiple_of(self.channels) {
            return Err(Error::new("rubberband.bad_length")
                .arg("samples", chunk.len() as i64)
                .arg("channels", self.channels as i64));
        }
        let frames = chunk.len() / self.channels;
        if done + frames > self.input_frames {
            return Err(Error::new("rubberband.too_much_input")
                .arg("given", (done + frames) as i64)
                .arg("expected", self.input_frames as i64));
        }
        Ok(frames)
    }

    fn deinterleave(&mut self, block: &[f32]) -> usize {
        let frames = block.len() / self.channels;
        for (c, plane) in self.planar_in.iter_mut().enumerate() {
            for (f, sample) in plane[..frames].iter_mut().enumerate() {
                *sample = block[f * self.channels + c];
            }
        }
        frames
    }

    fn input_pointers(&self) -> Vec<*const f32> {
        self.planar_in.iter().map(|p| p.as_ptr()).collect()
    }

    /// The study pass: give the whole input, in order, in chunks of any size (whole frames). The last chunk ends the
    /// pass. Errors: `rubberband.bad_length`, `rubberband.too_much_input`, `rubberband.out_of_order`
    pub fn study(&mut self, chunk: &[f32]) -> Result<()> {
        if self.pass != Pass::Study {
            return Err(Error::new("rubberband.out_of_order"));
        }
        let frames = self.check_chunk(chunk, self.studied)?;
        for block in chunk.chunks(OFFLINE_BLOCK * self.channels) {
            let n = self.deinterleave(block);
            let last = self.studied + n == self.input_frames;
            let pointers = self.input_pointers();
            // SAFETY: the state is ours (&mut self); `pointers` holds `channels` pointers to planes of OFFLINE_BLOCK
            // floats, of which the first `n` are read; they live until the call returns
            unsafe { ffi::rubberband_study(self.handle.ptr(), pointers.as_ptr(), n as c_uint, c_int::from(last)) };
            self.studied += n;
        }
        debug_assert_eq!(frames * self.channels, chunk.len());
        if self.studied == self.input_frames {
            self.pass = Pass::Process;
        }
        Ok(())
    }

    /// Key frames: each `(input frame, output frame)` lands exactly there, the stretch between two of them even (a
    /// tempo map with several tempos: the frames where the tempo changes). Between the study pass and the processing
    /// pass, both coordinates strictly increasing and inside the input and the output. The overall ratio still comes
    /// from [`Offline::new`]. Errors: `rubberband.out_of_order` (not between the passes), `rubberband.bad_key_frames`
    pub fn set_key_frames(&mut self, frames: &[(usize, usize)]) -> Result<()> {
        if self.pass != Pass::Process || self.processed != 0 {
            return Err(Error::new("rubberband.out_of_order"));
        }
        let mut last = (0usize, 0usize);
        for (i, &(from, to)) in frames.iter().enumerate() {
            let increasing = i == 0 || (from > last.0 && to > last.1);
            if !increasing || from >= self.input_frames || to >= self.output_frames {
                return Err(Error::new("rubberband.bad_key_frames").arg("index", i as i64));
            }
            last = (from, to);
        }
        let (Ok(count), Ok(mut from), Ok(mut to)) = (
            c_uint::try_from(frames.len()),
            frames.iter().map(|f| c_uint::try_from(f.0)).collect::<std::result::Result<Vec<_>, _>>(),
            frames.iter().map(|f| c_uint::try_from(f.1)).collect::<std::result::Result<Vec<_>, _>>(),
        ) else {
            return Err(Error::new("rubberband.too_long"));
        };
        // SAFETY: the state is ours (&mut self); `from` and `to` hold `count` values each and live until the call
        // returns (the library copies them into its own map)
        unsafe { ffi::rubberband_set_key_frame_map(self.handle.ptr(), count, from.as_mut_ptr(), to.as_mut_ptr()) };
        Ok(())
    }

    /// The processing pass: give the whole input again, in order, in chunks of any size. The stretched audio made so
    /// far is appended to `output` (interleaved, never beyond [`output_frames`](Self::output_frames) frames).
    /// Errors: `rubberband.bad_length`, `rubberband.too_much_input`, `rubberband.out_of_order` (the study pass has not
    /// covered the whole input)
    pub fn process(&mut self, chunk: &[f32], output: &mut Vec<f32>) -> Result<()> {
        if self.pass != Pass::Process {
            return Err(Error::new("rubberband.out_of_order"));
        }
        self.check_chunk(chunk, self.processed)?;
        for block in chunk.chunks(OFFLINE_BLOCK * self.channels) {
            let n = self.deinterleave(block);
            let last = self.processed + n == self.input_frames;
            let pointers = self.input_pointers();
            // SAFETY: as in `study`; process copies the input into the stretcher's own buffers
            unsafe { ffi::rubberband_process(self.handle.ptr(), pointers.as_ptr(), n as c_uint, c_int::from(last)) };
            self.processed += n;
            self.drain(output);
        }
        Ok(())
    }

    /// Moves what the library has ready to `output`
    fn drain(&mut self, output: &mut Vec<f32>) {
        loop {
            // SAFETY: the state is ours; available only reads it
            let available = unsafe { ffi::rubberband_available(self.handle.ptr()) };
            if available <= 0 {
                return;
            }
            let want = (available as usize).min(OFFLINE_BLOCK);
            let pointers: Vec<*mut f32> = self.planar_out.iter_mut().map(|p| p.as_mut_ptr()).collect();
            // SAFETY: `pointers` holds `channels` pointers to planes of OFFLINE_BLOCK floats each, `want` of which
            // are written; the planes are exclusively borrowed for the call
            let got = unsafe { ffi::rubberband_retrieve(self.handle.ptr(), pointers.as_ptr(), want as c_uint) };
            // Never more than asked, whatever the library says (the planes hold `want`)
            let got = (got as usize).min(want);
            if got == 0 {
                return;
            }
            self.made += got;
            let keep = got.min(self.output_frames - self.emitted);
            output.reserve(keep * self.channels);
            for f in 0..keep {
                for plane in &self.planar_out {
                    output.push(plane[f]);
                }
            }
            self.emitted += keep;
        }
    }

    /// Ends the stretch: appends the rest the library holds and completes `output` to exactly
    /// [`output_frames`](Self::output_frames) frames written in all. Errors: `rubberband.out_of_order` (the
    /// processing pass has not covered the whole input)
    pub fn finish(&mut self, output: &mut Vec<f32>) -> Result<()> {
        match self.pass {
            Pass::Process if self.processed == self.input_frames => {}
            Pass::Study if self.input_frames == 0 => {}
            _ => return Err(Error::new("rubberband.out_of_order")),
        }
        if self.input_frames > 0 {
            // All the input given: what the library still holds comes out as it is processed (it says -1 once it
            // has nothing more); an empty final call moves it on when it has nothing ready yet
            loop {
                self.drain(output);
                if self.emitted >= self.output_frames {
                    break;
                }
                // SAFETY: the state is ours; available only reads it
                let available = unsafe { ffi::rubberband_available(self.handle.ptr()) };
                if available < 0 {
                    break;
                }
                if available == 0 {
                    let pointers = self.input_pointers();
                    // SAFETY: as in `process`, with no frame read
                    unsafe { ffi::rubberband_process(self.handle.ptr(), pointers.as_ptr(), 0, 1) };
                    // SAFETY: as above
                    if unsafe { ffi::rubberband_available(self.handle.ptr()) } == 0 {
                        break;
                    }
                }
            }
        }
        let missing = self.output_frames - self.emitted;
        output.resize(output.len() + missing * self.channels, 0.0);
        self.emitted = self.output_frames;
        self.pass = Pass::Finished;
        Ok(())
    }
}

impl fmt::Debug for Offline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Offline")
            .field("channels", &self.channels)
            .field("input_frames", &self.input_frames)
            .field("output_frames", &self.output_frames)
            .field("pitch_scale", &self.pitch_scale)
            .field("settings", &self.settings)
            .field("pass", &self.pass)
            .field("studied", &self.studied)
            .field("processed", &self.processed)
            .finish()
    }
}

/// Stretches a whole interleaved buffer to exactly `output_frames` frames, shifting the pitch by `pitch_scale`: both
/// passes of [`Offline`], checking `cancel` and reporting to `progress` every 8192 frames. Allocates the output: run
/// it in an `Operation`, never on the audio thread. For a long item, drive [`Offline`] from the file in chunks
/// instead of holding the whole input in memory.
///
/// Errors: those of [`Offline::new`], `rubberband.bad_length` (the buffer is not whole frames, or does not match
/// `input_frames`), `error.cancelled`.
pub fn stretch(
    input: &[f32],
    format: Format,
    output_frames: usize,
    pitch_scale: f64,
    settings: &Settings,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Vec<f32>> {
    format.check()?;
    let channels = format.channels;
    if !input.len().is_multiple_of(channels) {
        return Err(Error::new("rubberband.bad_length")
            .arg("samples", input.len() as i64)
            .arg("channels", channels as i64));
    }
    let input_frames = input.len() / channels;
    let mut stretcher = Offline::new(format, input_frames, output_frames, pitch_scale, settings)?;
    cancel.check()?;
    let block = OFFLINE_BLOCK * channels;
    for chunk in input.chunks(block) {
        cancel.check()?;
        stretcher.study(chunk)?;
        progress.set(stretcher.fraction());
    }
    let mut output = Vec::with_capacity(output_frames * channels);
    for chunk in input.chunks(block) {
        cancel.check()?;
        stretcher.process(chunk, &mut output)?;
        progress.set(stretcher.fraction());
    }
    stretcher.finish(&mut output)?;
    progress.set(1.0);
    Ok(output)
}

/// The library in real-time mode: a stream in, a stream out, with the time ratio and the pitch changeable while it
/// runs. For a live preview.
///
/// Made off the audio thread ([`new`](Self::new) allocates everything, sized for `max_block` frames per call). On
/// the audio thread: [`process`](Self::process), [`retrieve`](Self::retrieve), [`available`](Self::available),
/// [`samples_required`](Self::samples_required) and [`start_delay`](Self::start_delay) neither allocate nor lock;
/// [`set_time_ratio`](Self::set_time_ratio) neither on R3 (on R2 the next `process` allocates).
/// **[`set_pitch_scale`](Self::set_pitch_scale) and [`reset`](Self::reset) may allocate** (R3 rebuilds its
/// resampler): never on the audio thread. `Send`, not
/// `Sync`; dropped off the audio thread.
///
/// The output trails the input by [`start_delay`](Self::start_delay) frames: drop that many output frames to align
/// it, or feed [`preferred_start_pad`](Self::preferred_start_pad) frames of silence first and then drop the delay.
pub struct Realtime {
    handle: Handle,
    channels: usize,
    max_block: usize,
    time_ratio: f64,
    pitch_scale: f64,
    settings: Settings,
    inputs: Box<[*const f32]>,
    outputs: Box<[*mut f32]>,
}

// SAFETY: the stretcher is `Send` (see `Handle`); the two pointer arrays are scratch space owned by self, written and
// read only inside `&mut self` calls, and never point to anything after the call returns
unsafe impl Send for Realtime {}

impl Realtime {
    /// A real-time stretcher for the `format`, taking at most `max_block` frames per
    /// [`process`](Self::process). Allocates and locks: never on the audio thread.
    ///
    /// Errors: `rubberband.bad_rate`, `rubberband.bad_channels`, `rubberband.bad_ratio`, `rubberband.bad_pitch`,
    /// `rubberband.bad_block` (zero or above 2¹⁶ frames), `rubberband.new_failed`.
    pub fn new(
        format: Format,
        max_block: usize,
        time_ratio: f64,
        pitch_scale: f64,
        settings: &Settings,
    ) -> Result<Self> {
        format.check()?;
        let channels = format.channels;
        check_time_ratio(time_ratio)?;
        check_pitch(pitch_scale)?;
        if max_block == 0 || max_block > 1 << 16 {
            return Err(Error::new("rubberband.bad_block").arg("frames", max_block as i64));
        }
        // High consistency: R2 can change the pitch while running without a discontinuity (R3 always can)
        let options = settings.options() | ffi::PROCESS_REAL_TIME | ffi::PITCH_HIGH_CONSISTENCY;
        let handle = Handle::new(format, options, time_ratio, pitch_scale)?;
        // SAFETY: the state is valid and exclusively ours; this sizes the internal buffers now, so that process never
        // has to grow them
        unsafe { ffi::rubberband_set_max_process_size(handle.ptr(), max_block as c_uint) };
        let mut stretcher = Self {
            handle,
            channels,
            max_block,
            time_ratio,
            pitch_scale,
            settings: *settings,
            inputs: vec![std::ptr::null(); channels].into_boxed_slice(),
            outputs: vec![std::ptr::null_mut(); channels].into_boxed_slice(),
        };
        // A pitch other than 1 makes R3 build its resampler: done here, not on the first audio block
        if pitch_scale != 1.0 {
            stretcher.set_pitch_scale(pitch_scale)?;
        }
        Ok(stretcher)
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn max_block(&self) -> usize {
        self.max_block
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The engine that runs: 3 (R3, finer) or 2 (R2, faster)
    pub fn engine_version(&self) -> u32 {
        self.handle.engine_version()
    }

    pub fn time_ratio(&self) -> f64 {
        self.time_ratio
    }

    pub fn pitch_scale(&self) -> f64 {
        self.pitch_scale
    }

    /// Changes the time ratio from the next [`process`](Self::process) on. Real-time safe on R3 (the default); on R2
    /// the next `process` recomputes the stretch profile and allocates. The error
    /// (`rubberband.bad_ratio`) carries no arguments, so making it does not allocate either
    pub fn set_time_ratio(&mut self, time_ratio: f64) -> Result<()> {
        check_time_ratio(time_ratio)?;
        // SAFETY: the state is ours (&mut self); in real-time mode the ratio may change at any time
        unsafe { ffi::rubberband_set_time_ratio(self.handle.ptr(), time_ratio) };
        self.time_ratio = time_ratio;
        Ok(())
    }

    /// Changes the pitch scale. **May allocate** (R3 builds or rebuilds its resampler): not on the audio thread.
    /// Error: `rubberband.bad_pitch`
    pub fn set_pitch_scale(&mut self, pitch_scale: f64) -> Result<()> {
        check_pitch(pitch_scale)?;
        // SAFETY: the state is ours (&mut self); in real-time mode the pitch may change at any time
        unsafe { ffi::rubberband_set_pitch_scale(self.handle.ptr(), pitch_scale) };
        self.pitch_scale = pitch_scale;
        Ok(())
    }

    /// How many output frames trail the input: drop this many from the start of the output to align it
    pub fn start_delay(&self) -> usize {
        // SAFETY: the state is valid; the call only reads it
        unsafe { ffi::rubberband_get_start_delay(self.handle.ptr()) as usize }
    }

    /// How many frames of silence the library would like before the first input (optional)
    pub fn preferred_start_pad(&self) -> usize {
        // SAFETY: the state is valid; the call only reads it
        unsafe { ffi::rubberband_get_preferred_start_pad(self.handle.ptr()) as usize }
    }

    /// How many input frames the next [`process`](Self::process) needs before more output is available (at most
    /// `max_block` is ever accepted)
    pub fn samples_required(&self) -> usize {
        // SAFETY: the state is valid; the call only reads it
        unsafe { ffi::rubberband_get_samples_required(self.handle.ptr()) as usize }
    }

    /// Forgets the stream (after a stop or a locate). **May allocate** on R3: not on the audio thread
    pub fn reset(&mut self) {
        // SAFETY: the state is ours (&mut self)
        unsafe { ffi::rubberband_reset(self.handle.ptr()) };
    }

    /// Gives one block of input: one slice per channel, all the same length, at most `max_block` frames. Does not
    /// allocate. Error: `rubberband.bad_block` (wrong channel count, unequal or too long slices), without arguments
    pub fn process(&mut self, input: &[&[f32]]) -> Result<()> {
        let frames = input.first().map_or(0, |c| c.len());
        if input.len() != self.channels || frames > self.max_block || input.iter().any(|c| c.len() != frames) {
            return Err(Error::new("rubberband.bad_block"));
        }
        for (slot, channel) in self.inputs.iter_mut().zip(input) {
            *slot = channel.as_ptr();
        }
        // SAFETY: the state is ours; `inputs` holds `channels` pointers, each to `frames` readable floats borrowed for
        // the call; the library copies them into its own ring buffers (sized by set_max_process_size) and keeps no
        // pointer
        unsafe { ffi::rubberband_process(self.handle.ptr(), self.inputs.as_ptr(), frames as c_uint, 0) };
        self.inputs.fill(std::ptr::null());
        Ok(())
    }

    /// How many output frames [`retrieve`](Self::retrieve) can give now. Does not allocate
    pub fn available(&self) -> usize {
        // SAFETY: the state is valid; the call only reads it
        let n = unsafe { ffi::rubberband_available(self.handle.ptr()) };
        usize::try_from(n).unwrap_or(0)
    }

    /// Writes up to the length of the output slices (one per channel, all the same length) and returns how many
    /// frames were written at their start. Does not allocate. Error: `rubberband.bad_block`, without arguments
    pub fn retrieve(&mut self, output: &mut [&mut [f32]]) -> Result<usize> {
        let frames = output.first().map_or(0, |c| c.len());
        if output.len() != self.channels || output.iter().any(|c| c.len() != frames) {
            return Err(Error::new("rubberband.bad_block"));
        }
        // Never more than is ready: the library warns on stderr when asked for more
        let Ok(frames) = c_uint::try_from(frames.min(self.available())) else {
            return Err(Error::new("rubberband.bad_block"));
        };
        if frames == 0 {
            return Ok(0);
        }
        for (slot, channel) in self.outputs.iter_mut().zip(output.iter_mut()) {
            *slot = channel.as_mut_ptr();
        }
        // SAFETY: the state is ours; `outputs` holds `channels` pointers to `frames` writable floats each, exclusively
        // borrowed for the call; the library writes at most `frames` per channel
        let got = unsafe { ffi::rubberband_retrieve(self.handle.ptr(), self.outputs.as_ptr(), frames) };
        self.outputs.fill(std::ptr::null_mut());
        Ok((got as usize).min(frames as usize))
    }
}

impl fmt::Debug for Realtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Realtime")
            .field("channels", &self.channels)
            .field("max_block", &self.max_block)
            .field("time_ratio", &self.time_ratio)
            .field("pitch_scale", &self.pitch_scale)
            .field("settings", &self.settings)
            .finish()
    }
}
