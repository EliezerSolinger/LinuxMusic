//! The amp: a Neural Amp Modeler model at the engine's rate (the legacy `AmpModel.cpp`, ported). A model runs at the
//! rate it was trained at: at another engine rate the audio is resampled to it and back (libsamplerate, streaming,
//! medium quality), into a FIFO that starts with a cushion of silence, so that the converters' jitter never starves
//! it; the latency is that cushion and the two converters' delays, measured when it is made with an impulse through
//! converters of the same kind. A model that gives its loudness is brought to −18 dB, as NAM's own plugin does.
//!
//! Made off the audio thread; [`Amp::process`] neither allocates nor locks.

use lm_base::Result;
use lm_sys_nam::Model;
use lm_sys_samplerate::{Quality, Resampler};

/// The most frames the legacy amp was given at once: the model is set up for this many (so that it is prewarmed as
/// the legacy one was)
pub const MAX_FRAMES: usize = 4096;
const CONVERTER: Quality = Quality::Medium;
/// dB, as NAM's plugin
const TARGET_LOUDNESS: f64 = -18.0;
const CUSHION: usize = 256;
/// Larger than any real `.nam` file (they are well under a few MiB): a bigger one is refused before it is parsed
const MAX_FILE_BYTES: usize = 64 << 20;
/// The rates a model may ask to run at (its header is untrusted: the buffers below are sized by the ratio)
const MIN_MODEL_RATE: f64 = 8_000.0;
const MAX_MODEL_RATE: f64 = 768_000.0;

/// The rate a model asks for, if it is a sane one
fn checked_model_rate(rate: f64, engine: f64) -> Result<f64> {
    if rate.is_finite() && (MIN_MODEL_RATE..=MAX_MODEL_RATE).contains(&rate) && engine.is_finite() && engine > 0.0 {
        let ratio = rate / engine;
        if (1.0 / 64.0..=64.0).contains(&ratio) {
            return Ok(rate);
        }
    }
    Err(lm_base::Error::new("nam.bad_rate").arg("rate", rate))
}

/// Runs `input` through `state` at its ratio: all of it is consumed; the output frames written are returned (the
/// legacy `convert`)
fn convert(state: &mut Resampler, mut input: &[f32], output: &mut [f32]) -> usize {
    let mut produced = 0;
    while !input.is_empty() || produced == 0 {
        let Ok(done) = state.process(input, &mut output[produced..]) else { break };
        produced += done.output_frames;
        input = &input[done.input_frames..];
        if done.input_frames == 0 && done.output_frames == 0 {
            break;
        }
    }
    produced
}

/// The delay (in output frames) of a converter of this kind at `ratio`: where an impulse comes out, its peak refined
/// between its neighbours (a parabola)
fn converter_delay(ratio: f64) -> Result<f64> {
    let mut state = Resampler::new(CONVERTER, 1, ratio)?;
    let frames = 4096usize;
    let mut input = vec![0.0f32; frames];
    input[0] = 1.0;
    let mut output = vec![0.0f32; (frames as f64 * ratio + 256.0) as usize];
    let produced = convert(&mut state, &input, &mut output);
    let out = &output[..produced];
    // (the first of the loudest, as std::max_element)
    let mut peak = 0;
    for (i, x) in out.iter().enumerate() {
        if x.abs() > out[peak].abs() {
            peak = i;
        }
    }
    if peak == 0 || peak + 1 >= produced {
        return Ok(peak as f64);
    }
    let (a, b, c) = (f64::from(out[peak - 1]), f64::from(out[peak]), f64::from(out[peak + 1]));
    let denominator = a - 2.0 * b + c;
    Ok(peak as f64 + if denominator != 0.0 { 0.5 * (a - c) / denominator } else { 0.0 })
}

/// The resampling around a model that runs at another rate than the engine's
struct Resampling {
    up: Resampler,
    down: Resampler,
    at_model: Vec<f32>,
    model_output: Vec<f32>,
    down_output: Vec<f32>,
    fifo: Vec<f32>,
    read: usize,
    fill: usize,
}

/// An amp model at the engine's rate (mono)
pub struct Amp {
    model: Model,
    gain: f32,
    latency: u32,
    resampling: Option<Resampling>,
}

impl std::fmt::Debug for Amp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Amp").field("gain", &self.gain).field("latency", &self.latency).finish_non_exhaustive()
    }
}

impl Amp {
    /// The model of `bytes` (a `.nam` file) for the engine's `rate`
    pub fn new(bytes: &[u8], rate: f64) -> Result<Self> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(lm_base::Error::new("nam.too_large").arg("bytes", bytes.len() as i64));
        }
        let mut model = Model::load(bytes)?;
        let model_rate = match model.expected_rate() {
            Some(trained) if !model.supports_arbitrary_rate() => checked_model_rate(trained, rate)?,
            _ => checked_model_rate(rate, rate)?,
        };
        let resampled = (model_rate - rate).abs() > 0.5;
        let gain = model.loudness().filter(|l| l.is_finite()).map_or(1.0, |l| 10f64.powf((TARGET_LOUDNESS - l) / 20.0) as f32);
        let model_frames = (MAX_FRAMES as f64 * model_rate / rate).ceil() as usize + 64;
        model.reset(model_rate, model_frames)?;
        let mut latency = 0;
        let resampling = if resampled {
            let up = model_rate / rate;
            let delay = converter_delay(up)? / up + converter_delay(1.0 / up)?;
            latency = CUSHION as u32 + delay.round() as u32;
            Some(Resampling {
                up: Resampler::new(CONVERTER, 1, up)?,
                down: Resampler::new(CONVERTER, 1, 1.0 / up)?,
                at_model: vec![0.0; model_frames],
                model_output: vec![0.0; model_frames],
                down_output: vec![0.0; MAX_FRAMES + 256],
                fifo: vec![0.0; 4 * MAX_FRAMES + CUSHION],
                read: 0,
                fill: CUSHION,
            })
        } else {
            None
        };
        Ok(Self { model, gain, latency, resampling })
    }

    /// Frames the output comes after the input
    pub fn latency(&self) -> u32 {
        self.latency
    }

    /// The loudness normalization's gain
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Runs `io` (at most [`MAX_FRAMES`]) through the model, in place
    pub fn process(&mut self, io: &mut [f32]) {
        let gain = self.gain;
        let Some(r) = self.resampling.as_mut() else {
            self.model.process(io);
            for x in io {
                *x *= gain;
            }
            return;
        };
        // Up to the model's rate, through the model, back down into the FIFO
        let made = convert(&mut r.up, io, &mut r.at_model);
        if made > 0 {
            r.model_output[..made].copy_from_slice(&r.at_model[..made]);
            self.model.process(&mut r.model_output[..made]);
            let back = convert(&mut r.down, &r.model_output[..made], &mut r.down_output);
            let size = r.fifo.len();
            for &x in &r.down_output[..back] {
                if r.fill >= size {
                    break;
                }
                r.fifo[(r.read + r.fill) % size] = x;
                r.fill += 1;
            }
        }
        // Out of the FIFO (silence only if it ran dry, which its cushion is there to prevent)
        let size = r.fifo.len();
        for x in io {
            if r.fill == 0 {
                *x = 0.0;
                continue;
            }
            *x = r.fifo[r.read] * gain;
            r.read = (r.read + 1) % size;
            r.fill -= 1;
        }
    }

    /// Lets go of the audio the resampling holds (the FIFO back to its cushion of silence). The model's own memory
    /// of the last few milliseconds stays (NAM has no way to clear it without allocating): it fades as it would
    pub fn reset(&mut self) {
        if let Some(r) = self.resampling.as_mut() {
            let _ = r.up.reset();
            let _ = r.down.reset();
            r.fifo.fill(0.0);
            r.read = 0;
            r.fill = CUSHION;
        }
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    #[test]
    fn a_hostile_rate_is_refused() {
        assert!(checked_model_rate(f64::INFINITY, 48000.0).is_err());
        assert!(checked_model_rate(f64::NAN, 48000.0).is_err());
        assert!(checked_model_rate(1.0e12, 48000.0).is_err());
        assert!(checked_model_rate(1.0, 48000.0).is_err());
        assert_eq!(checked_model_rate(48000.0, 44100.0).unwrap(), 48000.0);
    }

    #[test]
    fn an_oversized_file_is_refused_unparsed() {
        let big = vec![0u8; MAX_FILE_BYTES + 1];
        assert!(Amp::new(&big, 48000.0).is_err());
    }
}
