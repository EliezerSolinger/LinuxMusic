//! Decoded audio, immutable and shared (Parte 3: `SampleData` lives here because the processors and the preview
//! consume it; `lm-io` produces it). It enters the audio thread inside a program or a preview command and always comes
//! back to the main thread to be dropped (rule 5)

/// Audio decoded at the project's rate: `channels` × `frames` samples of `f32`, stored channel after channel
#[derive(Clone, Debug, PartialEq)]
pub struct SampleData {
    rate: u32,
    channels: usize,
    frames: usize,
    data: Box<[f32]>,
}

impl SampleData {
    /// From one vector per channel; every channel must have the same length (key `render.sample.ragged`) and there
    /// must be at least one channel (`render.sample.no_channels`)
    pub fn from_channels(rate: u32, channels: Vec<Vec<f32>>) -> lm_base::Result<Self> {
        let Some(first) = channels.first() else {
            return Err(lm_base::Error::new("render.sample.no_channels"));
        };
        let frames = first.len();
        if channels.iter().any(|c| c.len() != frames) {
            return Err(lm_base::Error::new("render.sample.ragged"));
        }
        let count = channels.len();
        let mut data = Vec::with_capacity(count * frames);
        for c in channels {
            data.extend_from_slice(&c);
        }
        Ok(Self { rate, channels: count, frames, data: data.into_boxed_slice() })
    }

    /// From planar data already laid out channel after channel (`data.len() == channels * frames`)
    pub fn from_planar(rate: u32, channels: usize, data: Box<[f32]>) -> lm_base::Result<Self> {
        if channels == 0 {
            return Err(lm_base::Error::new("render.sample.no_channels"));
        }
        if !data.len().is_multiple_of(channels) {
            return Err(lm_base::Error::new("render.sample.ragged"));
        }
        Ok(Self { rate, channels, frames: data.len() / channels, data })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    /// One channel's samples
    pub fn channel(&self, channel: usize) -> &[f32] {
        &self.data[channel * self.frames..(channel + 1) * self.frames]
    }
}
