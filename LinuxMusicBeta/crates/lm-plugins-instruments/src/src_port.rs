//! libsamplerate's zero-order-hold and linear converters (`src_zoh.c`, `src_linear.c` and `src_process`, 0.2.2) in
//! Rust, for two interleaved channels, sample for sample: the AudioFileProcessor's "none" and "linear" interpolation.
//!
//! Why a port and not the library: a note's converter state must be copied on the audio thread (the crossfade of a
//! change plays a copy of every sounding voice), and libsamplerate copies a state only by allocating a new one
//! (`src_clone`). These two converters hold a handful of numbers, copied here as plain data. Their output equals the
//! library's (tested against it, `tests/audio_file_processor.rs`); the sinc converters stay the library's. One
//! difference: given one or two frames with the position past the first, the library's linear converter reads the
//! frame before its input from the memory there; the port takes the last frame of the call before, which is that
//! frame when the input is contiguous.

/// libsamplerate's `SRC_MAX_RATIO`
const MAX_RATIO: f64 = 256.0;
/// libsamplerate's `SRC_MIN_RATIO_DIFF`
const MIN_RATIO_DIFF: f64 = 1e-20;
const CHANNELS: usize = 2;

fn is_bad_ratio(ratio: f64) -> bool {
    !(1.0 / MAX_RATIO..=MAX_RATIO).contains(&ratio)
}

/// `lrint`: to the nearest integer, ties to even (the default rounding mode)
fn lrint(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// `fmod_one`
fn fmod_one(x: f64) -> f64 {
    let res = x - lrint(x) as f64;
    if res < 0.0 { res + 1.0 } else { res }
}

/// Which converter
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ZeroOrderHold,
    Linear,
}

/// One converter's state: what `SRC_STATE` and its private data hold for these two
#[derive(Clone, Copy, Debug)]
pub struct Converter {
    kind: Kind,
    dirty: bool,
    last_value: [f32; CHANNELS],
    last_position: f64,
    last_ratio: f64,
}

impl Converter {
    /// As `src_new` then `src_reset` leave it
    pub fn new(kind: Kind) -> Self {
        Self { kind, dirty: false, last_value: [0.0; CHANNELS], last_position: 0.0, last_ratio: 0.0 }
    }

    /// `src_reset`
    pub fn reset(&mut self) {
        *self = Self::new(self.kind);
    }

    /// `src_process` with `end_of_input` false: converts as much of `input` as fits in `output`, gliding from the
    /// last ratio to `ratio` over the output's length. Returns the frames used and written; a ratio out of the
    /// library's range is its error, here nothing done
    pub fn process(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]], ratio: f64) -> (usize, usize) {
        if is_bad_ratio(ratio) {
            return (0, 0);
        }
        // Special case for when last_ratio has not been set
        if self.last_ratio < 1.0 / MAX_RATIO {
            self.last_ratio = ratio;
        }
        if input.is_empty() {
            return (0, 0);
        }
        let ch = CHANNELS as i64;
        let in_count = input.len() as i64 * ch;
        let out_count = output.len() as i64 * ch;
        let data_in = input.as_flattened();
        let data_out = output.as_flattened_mut();
        if !self.dirty {
            // Just reset: the last value is the first frame
            self.last_value.copy_from_slice(&data_in[..CHANNELS]);
            self.dirty = true;
        }
        let (mut in_used, mut out_gen) = (0i64, 0i64);
        let mut src_ratio = self.last_ratio;
        if is_bad_ratio(src_ratio) {
            return (0, 0);
        }
        let gliding = (self.last_ratio - ratio).abs() > MIN_RATIO_DIFF;
        let mut input_index = self.last_position;
        let linear = self.kind == Kind::Linear;

        // Before the first frame of the input
        while input_index < 1.0 && out_gen < out_count {
            let ahead = if linear { 1.0 + input_index } else { input_index };
            if in_used as f64 + ch as f64 * ahead >= in_count as f64 {
                break;
            }
            if out_count > 0 && gliding {
                src_ratio = self.last_ratio + out_gen as f64 * (ratio - self.last_ratio) / out_count as f64;
            }
            for (last, first) in self.last_value.iter().zip(data_in) {
                let l = f64::from(*last);
                data_out[out_gen as usize] =
                    if linear { (l + input_index * (f64::from(*first) - l)) as f32 } else { *last };
                out_gen += 1;
            }
            input_index += 1.0 / src_ratio;
        }
        let rem = fmod_one(input_index);
        in_used += ch * lrint(input_index - rem);
        input_index = rem;

        // The main loop
        loop {
            let room = in_used as f64 + ch as f64 * input_index;
            let more = if linear { room < in_count as f64 } else { room <= in_count as f64 };
            if !(out_gen < out_count && more) {
                break;
            }
            if out_count > 0 && gliding {
                src_ratio = self.last_ratio + out_gen as f64 * (ratio - self.last_ratio) / out_count as f64;
            }
            for c in 0..CHANNELS {
                // The frame before the input is the last one of the call before (the library reads the memory
                // before its input there: the same frame when the caller's frames are contiguous, as a sample
                // player's buffer is, and anything when not; `src_linear.c`'s own debug check calls it a mistake)
                let prev = if in_used >= ch { data_in[(in_used - ch) as usize + c] } else { self.last_value[c] };
                data_out[out_gen as usize] = if linear {
                    let next = f64::from(data_in[in_used as usize + c]);
                    (f64::from(prev) + input_index * (next - f64::from(prev))) as f32
                } else {
                    prev
                };
                out_gen += 1;
            }
            input_index += 1.0 / src_ratio;
            let rem = fmod_one(input_index);
            in_used += ch * lrint(input_index - rem);
            input_index = rem;
        }
        if in_used > in_count {
            input_index += ((in_used - in_count) / ch) as f64;
            in_used = in_count;
        }
        self.last_position = input_index;
        if in_used > 0 {
            for c in 0..CHANNELS {
                self.last_value[c] = data_in[(in_used - ch) as usize + c];
            }
        }
        // The ratio reached, not the target
        self.last_ratio = src_ratio;
        ((in_used / ch) as usize, (out_gen / ch) as usize)
    }
}
