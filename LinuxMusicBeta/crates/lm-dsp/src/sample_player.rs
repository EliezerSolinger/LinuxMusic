//! Playing a sample (legacy `Sample::play` and `Sample::render`): its start and end, a loop (on or ping-pong),
//! reversed, amplified, with microfades against clicks, at any pitch through an interpolator.
//!
//! The interpolator is a trait: the plugins give libsamplerate's (`lm-sys-samplerate`, as the legacy program used),
//! so that a sample plays as it did. At a ratio of exactly 1 the frames are copied as they are.

/// What converts the rendered frames to the pitch asked: libsamplerate's converters, in the plugins
pub trait Interpolator {
    /// Converts as much of @p input as fits in @p output at @p ratio (output rate over input rate, gliding from the
    /// last ratio as libsamplerate does). Returns the frames used and the frames written. Never allocates
    fn process(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]], ratio: f64) -> (usize, usize);
    /// Forgets the signal it held
    fn reset(&mut self);
}

/// How a sample loops
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Loop {
    #[default]
    Off,
    On,
    PingPong,
}

impl Loop {
    pub fn key(self) -> &'static str {
        match self {
            Loop::Off => "off",
            Loop::On => "on",
            Loop::PingPong => "ping-pong",
        }
    }
}

/// The points of a sample, in frames
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    pub start: i32,
    pub end: i32,
    pub loop_start: i32,
    pub loop_end: i32,
}

impl Bounds {
    /// The points moved inside a sample of @p frames: each in 0..=frames, the end not before the start, the loop's
    /// end not before its start
    pub fn within(&self, frames: usize) -> Self {
        let size = i32::try_from(frames).unwrap_or(i32::MAX);
        let start = self.start.clamp(0, size);
        let loop_start = self.loop_start.clamp(0, size);
        Self { start, end: self.end.clamp(start, size), loop_start, loop_end: self.loop_end.clamp(loop_start, size) }
    }
}

/// A sample in memory and how it plays
#[derive(Clone, Copy, Debug)]
pub struct Sample<'a> {
    pub frames: &'a [[f32; 2]],
    /// Its own rate
    pub sample_rate: f32,
    pub bounds: Bounds,
    pub amplification: f32,
    /// The pitch it plays at a ratio of 1 (Hz; 440 is as recorded)
    pub frequency: f32,
    pub reversed: bool,
    /// The microfades: 2 ms of the sample (one frame at least) at its start, its end, and into a loop's start
    pub declick: bool,
}

/// The pitch at which a sample plays as it was recorded
pub const DEFAULT_BASE_FREQ: f32 = 440.0;

/// The frames rendered at a time before the interpolator (the legacy program's period)
pub const BUFFER_FRAMES: usize = 256;

/// Where in the sample the rendering is
#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    pub frame_index: i32,
    pub backwards: bool,
    /// It went round the loop (no fade in at the start again)
    pub wrapped: bool,
}

/// Where a sample's playing is: one for each note or clip that plays it. Its `clone_from` copies into its own buffer
/// (a host copies a note's state on the audio thread)
#[derive(Debug)]
pub struct PlaybackState<I> {
    interpolator: I,
    buffer: Box<[[f32; 2]; BUFFER_FRAMES]>,
    /// What of `buffer` is left to give the interpolator
    view: core::ops::Range<usize>,
    cursor: Cursor,
    /// Played without the interpolator (a ratio of exactly 1) the last time
    direct: bool,
}

impl<I: Clone> Clone for PlaybackState<I> {
    fn clone(&self) -> Self {
        Self {
            interpolator: self.interpolator.clone(),
            buffer: self.buffer.clone(),
            view: self.view.clone(),
            cursor: self.cursor,
            direct: self.direct,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.interpolator.clone_from(&source.interpolator);
        *self.buffer = *source.buffer;
        self.view.clone_from(&source.view);
        self.cursor = source.cursor;
        self.direct = source.direct;
    }
}

impl<I: Interpolator> PlaybackState<I> {
    /// From @p frame_index, with @p interpolator; allocates its buffer
    pub fn new(interpolator: I, frame_index: i32) -> Self {
        Self {
            interpolator,
            buffer: Box::new([[0.0; 2]; BUFFER_FRAMES]),
            view: 0..0,
            cursor: Cursor { frame_index, backwards: false, wrapped: false },
            direct: false,
        }
    }

    pub fn frame_index(&self) -> i32 {
        self.cursor.frame_index
    }

    pub fn backwards(&self) -> bool {
        self.cursor.backwards
    }

    pub fn set_frame_index(&mut self, index: i32) {
        self.cursor.frame_index = index;
    }

    /// Its interpolator (a host changes its mode between notes)
    pub fn interpolator_mut(&mut self) -> &mut I {
        &mut self.interpolator
    }

    /// Plays on backwards (a ping-pong loop's way back) or forwards from where it is
    pub fn set_backwards(&mut self, backwards: bool) {
        self.cursor.backwards = backwards;
    }

    /// From @p frame_index again, as a new note
    pub fn restart(&mut self, frame_index: i32) {
        self.interpolator.reset();
        self.view = 0..0;
        self.cursor = Cursor { frame_index, backwards: false, wrapped: false };
        self.direct = false;
    }
}

impl Sample<'_> {
    /// Plays @p out.len() frames of the sample at @p ratio (on top of its rate and frequency) into a stream at
    /// @p output_rate, looping as @p looping says. Returns whether it still plays: fewer than @p period frames were
    /// left unplayed (the legacy program's test against its period)
    pub fn play<I: Interpolator>(
        &self,
        out: &mut [[f32; 2]],
        state: &mut PlaybackState<I>,
        mut looping: Loop,
        ratio: f64,
        output_rate: f32,
        period: usize,
    ) -> bool {
        if self.frames.is_empty() {
            return false;
        }
        let b = self.bounds.within(self.frames.len());
        // A loop with nothing in it plays as no loop (it would not move)
        if b.loop_end <= b.loop_start {
            looping = Loop::Off;
        }
        state.cursor.frame_index = state.cursor.frame_index.max(b.start);

        let rate_ratio = f64::from(output_rate) / f64::from(self.sample_rate as i32);
        let freq_ratio = self.frequency / DEFAULT_BASE_FREQ;
        let total = rate_ratio * f64::from(freq_ratio) * ratio;
        let mut left = out.len();
        let mut at = 0usize;

        if total == 1.0 {
            // At a ratio of exactly 1 the frames are played as they are: the interpolator would delay them by one
            state.direct = true;
            while left > 0 {
                if state.view.is_empty() {
                    let rendered = self.render(&mut state.buffer[..], &mut state.cursor, looping, &b);
                    state.view = 0..rendered;
                    if rendered == 0 {
                        out[at..].fill([0.0; 2]);
                        break;
                    }
                }
                let count = left.min(state.view.len());
                out[at..at + count].copy_from_slice(&state.buffer[state.view.start..state.view.start + count]);
                state.view.start += count;
                at += count;
                left -= count;
            }
            return left < period;
        }
        // Back to interpolating: it starts afresh from where the frames are
        if state.direct {
            state.interpolator.reset();
            state.direct = false;
        }

        while left > 0 {
            if state.view.is_empty() {
                let rendered = self.render(&mut state.buffer[..], &mut state.cursor, looping, &b);
                state.view = 0..rendered;
            }
            let (used, generated) =
                state.interpolator.process(&state.buffer[state.view.clone()], &mut out[at..], total);
            if used == 0 && generated == 0 {
                out[at..].fill([0.0; 2]);
                break;
            }
            state.view.start += used;
            at += generated;
            left -= generated;
        }
        left < period
    }

    /// Renders up to @p dst.len() frames from where @p state is, at the sample's own rate; returns how many
    pub fn render(&self, dst: &mut [[f32; 2]], state: &mut Cursor, mut looping: Loop, b: &Bounds) -> usize {
        // (play() gives bounds already within the sample; a caller of render() may not: every index below stays in
        // the sample only with them inside it)
        let b = &b.within(self.frames.len());
        if b.loop_end <= b.loop_start {
            looping = Loop::Off;
        }
        // A looping cursor before the sample (set by hand) starts at its first frame; a cursor that is not looping
        // ends there instead (below)
        if looping != Loop::Off {
            state.frame_index = state.frame_index.max(0);
        }
        let total = self.frames.len() as i32;
        // The microfades: 2 ms of the sample, one frame at least
        let declick_frames = if self.declick { 1.max((self.sample_rate as i32) / 500) } else { 0 };
        let at =
            |index: i32| -> [f32; 2] { self.frames[if self.reversed { total - index - 1 } else { index } as usize] };
        let scale = |f: [f32; 2], g: f32| [f[0] * g, f[1] * g];
        let amplification = self.amplification;
        for (n, out) in dst.iter_mut().enumerate() {
            match looping {
                Loop::Off => {
                    if state.frame_index < 0 || state.frame_index >= b.end {
                        return n;
                    }
                }
                Loop::On => {
                    if state.frame_index < b.loop_start && state.backwards {
                        state.frame_index = b.loop_end - 1;
                        state.wrapped = true;
                    } else if state.frame_index >= b.loop_end {
                        state.frame_index = b.loop_start;
                        state.wrapped = true;
                    }
                }
                Loop::PingPong => {
                    if state.frame_index < b.loop_start && state.backwards {
                        state.frame_index = b.loop_start;
                        state.backwards = false;
                    } else if state.frame_index >= b.loop_end {
                        state.frame_index = b.loop_end - 1;
                        state.backwards = true;
                    }
                }
            }

            let index = state.frame_index;
            let mut value = scale(at(index), amplification);
            if declick_frames > 0 {
                if looping == Loop::On
                    && !state.backwards
                    && index >= b.loop_end - declick_frames
                    && index < b.loop_end
                    && b.loop_start - (b.loop_end - index) >= 0
                {
                    // Into the loop's start: towards the frames just before it, which lead into it without a jump
                    let t = (index - (b.loop_end - declick_frames) + 1) as f32 / declick_frames as f32;
                    let here = at(index);
                    let there = at(b.loop_start - (b.loop_end - index));
                    value =
                        scale([here[0] * (1.0 - t) + there[0] * t, here[1] * (1.0 - t) + there[1] * t], amplification);
                } else if looping == Loop::Off && index >= b.end - declick_frames {
                    value = scale(value, (b.end - index) as f32 / declick_frames as f32);
                }
                if !state.wrapped && index - b.start < declick_frames && index >= b.start {
                    // The first time from the start only: a loop that starts there crossfades instead
                    value = scale(value, (index - b.start + 1) as f32 / (declick_frames + 1) as f32);
                }
            }
            *out = value;
            if state.backwards {
                state.frame_index -= 1;
            } else {
                state.frame_index += 1;
            }
        }
        dst.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No interpolation: the ratio is always 1 in these tests
    struct Never;
    impl Interpolator for Never {
        fn process(&mut self, _: &[[f32; 2]], _: &mut [[f32; 2]], _: f64) -> (usize, usize) {
            (0, 0)
        }
        fn reset(&mut self) {}
    }

    fn sample(frames: &[[f32; 2]], bounds: Bounds) -> Sample<'_> {
        Sample {
            frames,
            sample_rate: 48000.0,
            bounds,
            amplification: 1.0,
            frequency: DEFAULT_BASE_FREQ,
            reversed: false,
            declick: false,
        }
    }

    #[test]
    fn a_sample_plays_once_as_it_is() {
        let frames: Vec<[f32; 2]> = (0..100).map(|n| [n as f32, -(n as f32)]).collect();
        let s = sample(&frames, Bounds { start: 10, end: 60, loop_start: 0, loop_end: 0 });
        let mut state = PlaybackState::new(Never, 0);
        let mut out = vec![[9.0f32; 2]; 64];
        let ended = s.play(&mut out, &mut state, Loop::Off, 1.0, 48000.0, 256);
        assert!(ended);
        assert_eq!(out[0], [10.0, -10.0]);
        assert_eq!(out[49], [59.0, -59.0]);
        assert_eq!(out[50], [0.0, 0.0]);
    }

    #[test]
    fn ping_pong_turns_at_the_loop_end() {
        let frames: Vec<[f32; 2]> = (0..10).map(|n| [n as f32, 0.0]).collect();
        let s = sample(&frames, Bounds { start: 0, end: 10, loop_start: 4, loop_end: 8 });
        let mut state = PlaybackState::new(Never, 0);
        let mut out = vec![[0.0f32; 2]; 16];
        s.play(&mut out, &mut state, Loop::PingPong, 1.0, 48000.0, 256);
        let played: Vec<f32> = out.iter().map(|f| f[0]).collect();
        assert_eq!(&played[..12], &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 7.0, 6.0, 5.0, 4.0]);
    }
}
