//! The legacy program's PatMan: Gravis Ultrasound patches (.pat), their samples read as the legacy program read them,
//! a note playing the sample nearest its pitch (tuned to the note, or at its own), looped or not, through the sample
//! player.

use crate::sample_player::{Bounds, DEFAULT_BASE_FREQ, Interpolator, Loop, PlaybackState, Sample};

const MODES_16BIT: u8 = 1 << 0;
const MODES_UNSIGNED: u8 = 1 << 1;
const MODES_LOOPING: u8 = 1 << 2;
/// The release, ms
pub const RELEASE_MS: f32 = 3.0;

/// Why a patch could not be read
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// Not a Gravis Ultrasound patch
    NotGus,
    /// More than one instrument
    Instruments,
    /// More than one layer
    Layers,
    /// It ends before its data does
    Truncated,
}

impl PatchError {
    pub fn key(self) -> &'static str {
        match self {
            PatchError::NotGus => "patman.not_gus",
            PatchError::Instruments => "patman.instruments",
            PatchError::Layers => "patman.layers",
            PatchError::Truncated => "patman.truncated",
        }
    }
}

/// One sample of a patch
#[derive(Clone, Debug, PartialEq)]
pub struct PatchSample {
    pub frames: Vec<[f32; 2]>,
    pub sample_rate: u32,
    /// Hz: the pitch it was recorded at
    pub root_freq: f32,
    /// Its loop's start and end, frames, if it loops
    pub looping: Option<(i32, i32)>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], PatchError> {
        let slice = self.bytes.get(self.at..self.at + n).ok_or(PatchError::Truncated)?;
        self.at += n;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16, PatchError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, PatchError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Reads a patch (its file's bytes): its samples, mono made stereo. Away from the audio thread
pub fn parse_patch(bytes: &[u8]) -> Result<Vec<PatchSample>, PatchError> {
    let header = bytes.get(..239).ok_or(PatchError::NotGus)?;
    if &header[..22] != b"GF1PATCH110\0ID#000002\0" && &header[..22] != b"GF1PATCH100\0ID#000002\0" {
        return Err(PatchError::NotGus);
    }
    if header[82] > 1 {
        return Err(PatchError::Instruments);
    }
    if header[151] > 1 {
        return Err(PatchError::Layers);
    }
    let count = header[198] as usize;
    let mut r = Reader { bytes, at: 239 };
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        // The wave's name and fractions
        r.take(7 + 1)?;
        let data_length = r.u32()?;
        let mut loop_start = r.u32()?;
        let mut loop_end = r.u32()?;
        let sample_rate = u32::from(r.u16()?);
        // The low and high frequencies
        r.take(4 + 4)?;
        let root_freq = r.u32()?;
        // Tuning, panning, envelope, tremolo, vibrato
        r.take(2 + 1 + 12 + 3 + 3)?;
        let modes = r.take(1)?[0];
        // Scale frequency, scale factor, reserved
        r.take(2 + 2 + 36)?;

        let frames: Vec<[f32; 2]> = if modes & MODES_16BIT != 0 {
            let n = (data_length >> 1) as usize;
            let data = r.take(n * 2)?;
            loop_start >>= 1;
            loop_end >>= 1;
            data.as_chunks::<2>()
                .0
                .iter()
                .map(|b| {
                    let mut s = i16::from_le_bytes([b[0], b[1]]);
                    if modes & MODES_UNSIGNED != 0 {
                        s ^= i16::MIN;
                    }
                    let v = f32::from(s) / 32767.0;
                    [v, v]
                })
                .collect()
        } else {
            let data = r.take(data_length as usize)?;
            data.iter()
                .map(|&b| {
                    // (a signed char, as on the systems the legacy program was built for)
                    let mut s = b as i8;
                    if modes & MODES_UNSIGNED != 0 {
                        s ^= i8::MIN;
                    }
                    let v = f32::from(s) / 127.0;
                    [v, v]
                })
                .collect()
        };
        samples.push(PatchSample {
            frames,
            sample_rate,
            root_freq: root_freq as f32 / 1000.0,
            looping: (modes & MODES_LOOPING != 0).then_some((loop_start as i32, loop_end as i32)),
        });
    }
    Ok(samples)
}

/// The sample nearest @p frequency (the first if none is: one without a usable pitch is near nothing)
pub fn select_sample(samples: &[PatchSample], frequency: f32) -> usize {
    let mut min = f32::INFINITY;
    let mut chosen = 0;
    for (i, s) in samples.iter().enumerate() {
        let f = s.root_freq;
        let dist = if frequency >= f { frequency / f } else { f / frequency };
        if dist < min {
            min = dist;
            chosen = i;
        }
    }
    chosen
}

/// One note of the PatMan. Its `clone_from` copies into its own buffer (a host copies a note on the audio thread)
#[derive(Debug)]
pub struct PatmanVoice<I> {
    sample: usize,
    tuned: bool,
    state: PlaybackState<I>,
}

impl<I: Clone> Clone for PatmanVoice<I> {
    fn clone(&self) -> Self {
        Self { sample: self.sample, tuned: self.tuned, state: self.state.clone() }
    }

    fn clone_from(&mut self, source: &Self) {
        self.sample = source.sample;
        self.tuned = source.tuned;
        self.state.clone_from(&source.state);
    }
}

impl<I: Interpolator> PatmanVoice<I> {
    /// A note at @p frequency of @p samples (not empty); @p tuned: the sample tuned to the note (else at its own
    /// pitch); @p interpolator: linear, as the legacy program's
    pub fn new(samples: &[PatchSample], frequency: f32, tuned: bool, interpolator: I) -> Self {
        Self { sample: select_sample(samples, frequency), tuned, state: PlaybackState::new(interpolator, 0) }
    }

    /// A new note in this voice, as [`PatmanVoice::new`] makes one, without allocating
    pub fn restart(&mut self, samples: &[PatchSample], frequency: f32, tuned: bool) {
        self.sample = select_sample(samples, frequency);
        self.tuned = tuned;
        self.state.restart(0);
    }

    /// The next @p out frames of the note at @p frequency, into a stream at @p output_rate in periods of @p period
    /// frames. False once the sample has ended (@p out is then silent: the host lets the note go; else its release
    /// is [`crate::instrument::apply_release`])
    pub fn render(
        &mut self,
        out: &mut [[f32; 2]],
        samples: &[PatchSample],
        frequency: f32,
        looped: bool,
        output_rate: f32,
        period: usize,
    ) -> bool {
        let s = &samples[self.sample];
        let own = s.root_freq;
        // (untuned, at the sample's own pitch: one with none usable plays at the note's)
        let play_freq = if self.tuned || !own.is_finite() || own <= 0.0 { frequency } else { own };
        let (loop_start, loop_end) = s.looping.unwrap_or((0, s.frames.len() as i32));
        let sample = Sample {
            frames: &s.frames,
            sample_rate: s.sample_rate as f32,
            bounds: Bounds { start: 0, end: s.frames.len() as i32, loop_start, loop_end },
            amplification: 1.0,
            frequency: own,
            reversed: false,
            declick: false,
        };
        let looping = if looped { Loop::On } else { Loop::Off };
        if sample.play(out, &mut self.state, looping, f64::from(DEFAULT_BASE_FREQ / play_freq), output_rate, period) {
            true
        } else {
            out.fill([0.0; 2]);
            false
        }
    }
}
