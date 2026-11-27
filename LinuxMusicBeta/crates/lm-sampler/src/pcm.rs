//! Sample data: planar float frames, mono or stereo, shared by the zones that play them, and a small WAV reader
//! (the one format every importer can read without a library).

use std::sync::Arc;

use lm_base::{Error, Result};

use crate::stream::TailSource;

/// The frames of a sample at its own rate: one channel, or two. Immutable once made; zones share it in an `Arc`
#[derive(Clone, Debug)]
pub struct Pcm {
    pub rate: u32,
    /// The frames in memory: all of them, or the head of a streamed sample
    left: Box<[f32]>,
    right: Option<Box<[f32]>>,
    /// Where the rest of a streamed sample is (`None`: the sample is all here)
    tail: Option<Arc<dyn TailSource>>,
}

impl PartialEq for Pcm {
    fn eq(&self, other: &Self) -> bool {
        self.rate == other.rate
            && self.left == other.left
            && self.right == other.right
            && match (&self.tail, &other.tail) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

impl Pcm {
    /// A mono sample
    pub fn mono(rate: u32, frames: Vec<f32>) -> Self {
        Self { rate: rate.max(1), left: frames.into_boxed_slice(), right: None, tail: None }
    }

    /// A stereo sample (the shorter channel is padded with silence to the longer)
    pub fn stereo(rate: u32, mut left: Vec<f32>, mut right: Vec<f32>) -> Self {
        let n = left.len().max(right.len());
        left.resize(n, 0.0);
        right.resize(n, 0.0);
        Self { rate: rate.max(1), left: left.into_boxed_slice(), right: Some(right.into_boxed_slice()), tail: None }
    }

    /// From stereo frames; mono when both sides are identical everywhere
    pub fn from_frames(rate: u32, frames: &[[f32; 2]]) -> Self {
        if frames.iter().all(|f| f[0] == f[1]) {
            Self::mono(rate, frames.iter().map(|f| f[0]).collect())
        } else {
            Self::stereo(rate, frames.iter().map(|f| f[0]).collect(), frames.iter().map(|f| f[1]).collect())
        }
    }

    /// A streamed sample: `head` (its first frames, in memory) and the rest in `tail`, whose frame count is the
    /// sample's. The head must not be longer than the tail's frames
    pub fn streamed(head: Pcm, tail: Arc<dyn TailSource>) -> Self {
        let n = tail.frames().max(head.left.len());
        let mut head = head;
        head.left = head.left[..head.left.len().min(n)].into();
        if let Some(r) = head.right.take() {
            head.right = Some(r[..head.left.len()].into());
        }
        Self { tail: Some(tail), ..head }
    }

    /// The sample's frames: all of them, also the ones not in memory
    pub fn frames(&self) -> usize {
        match &self.tail {
            Some(t) => t.frames().max(self.left.len()),
            None => self.left.len(),
        }
    }

    /// The frames in memory: the sample's own for a resident one, the head of a streamed one
    pub fn head_frames(&self) -> usize {
        self.left.len()
    }

    pub fn is_streamed(&self) -> bool {
        self.tail.is_some()
    }

    pub fn tail(&self) -> Option<&Arc<dyn TailSource>> {
        self.tail.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.frames() == 0
    }

    pub fn is_stereo(&self) -> bool {
        self.right.is_some() || self.tail.as_ref().is_some_and(|t| t.stereo())
    }

    pub fn left(&self) -> &[f32] {
        &self.left
    }

    /// The right channel (the left again for a mono sample)
    pub fn right(&self) -> &[f32] {
        self.right.as_deref().unwrap_or(&self.left)
    }

    /// The frame at `i`; silence outside the sample
    #[inline]
    pub fn at(&self, i: i64) -> [f32; 2] {
        if i < 0 || i as usize >= self.left.len() {
            return [0.0; 2];
        }
        let i = i as usize;
        [self.left[i], self.right.as_deref().map_or(self.left[i], |r| r[i])]
    }
}

fn bad(reason: &'static str) -> Error {
    Error::new("sampler.wav.invalid").arg("reason", reason)
}

/// Decodes a RIFF WAVE file: integer PCM of 8, 16, 24 and 32 bits and 32 and 64-bit float, any number of channels
/// (the first two are kept). Away from the audio thread
pub fn read_wav(bytes: &[u8]) -> Result<Pcm> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a RIFF WAVE"));
    }
    let mut at = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        let body_at = at + 8;
        let end = (body_at + size).min(bytes.len());
        let body = &bytes[body_at..end];
        if id == b"fmt " && body.len() >= 16 {
            let mut tag = u16::from_le_bytes([body[0], body[1]]);
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if tag == 0xFFFE && body.len() >= 26 {
                // WAVE_FORMAT_EXTENSIBLE: the sub-format's first two bytes
                tag = u16::from_le_bytes([body[24], body[25]]);
            }
            format = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            data = Some(body);
        }
        at = body_at + size + (size & 1);
    }
    let (tag, channels, rate, bits) = format.ok_or_else(|| bad("no fmt chunk"))?;
    let data = data.ok_or_else(|| bad("no data chunk"))?;
    let channels = usize::from(channels);
    if channels == 0 || rate == 0 {
        return Err(bad("no channels"));
    }
    let width = usize::from(bits / 8);
    if width == 0 || !matches!((tag, bits), (1, 8 | 16 | 24 | 32) | (3, 32 | 64)) {
        return Err(bad("unsupported sample format"));
    }
    let frame = width * channels;
    let n = data.len() / frame;
    let sample = |b: &[u8]| -> f32 {
        match (tag, bits) {
            (1, 8) => (f32::from(b[0]) - 128.0) / 128.0,
            (1, 16) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            (1, _) => (i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 2_147_483_648.0) as f32,
            (_, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            _ => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32,
        }
    };
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(if channels > 1 { n } else { 0 });
    for f in data.chunks_exact(frame).take(n) {
        left.push(sample(&f[..width]));
        if channels > 1 {
            right.push(sample(&f[width..2 * width]));
        }
    }
    Ok(if channels > 1 { Pcm::stereo(rate, left, right) } else { Pcm::mono(rate, left) })
}

/// Reads sample files for the importers that cite them (SFZ regions, kits): the plugin gives one that also decodes
/// the formats it has libraries for (FLAC, Ogg); [`WavLoader`] reads WAVE only
pub trait SampleLoader {
    /// The sample of a file. Away from the audio thread
    fn load(&mut self, path: &std::path::Path) -> Result<Pcm>;
}

/// A loader that reads WAVE files
#[derive(Clone, Copy, Debug, Default)]
pub struct WavLoader;

impl SampleLoader for WavLoader {
    fn load(&mut self, path: &std::path::Path) -> Result<Pcm> {
        let bytes = std::fs::read(path).map_err(|e| {
            Error::new("sampler.read_failed").arg("path", path.display().to_string()).arg("error", e.to_string())
        })?;
        read_wav(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn wav16(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let mut v = Vec::new();
        let data_len = (samples.len() * 2) as u32;
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        v.extend_from_slice(&(channels * 2).to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    #[test]
    fn reads_mono_and_stereo_16_bit() {
        let m = read_wav(&wav16(44100, 1, &[0, 16384, -16384])).expect("mono");
        assert_eq!((m.rate, m.frames(), m.is_stereo()), (44100, 3, false));
        assert!((m.left()[1] - 0.5).abs() < 1e-4);
        let s = read_wav(&wav16(48000, 2, &[0, 32767, 16384, -16384])).expect("stereo");
        assert_eq!((s.frames(), s.is_stereo()), (2, true));
        assert!((s.right()[1] + 0.5).abs() < 1e-4);
    }

    #[test]
    fn rejects_garbage() {
        assert!(read_wav(b"nope").is_err());
        assert!(read_wav(&wav16(8000, 1, &[1])[..30]).is_err());
    }

    #[test]
    fn outside_the_sample_is_silence() {
        let p = Pcm::mono(1000, vec![1.0, 2.0]);
        assert_eq!(p.at(-1), [0.0; 2]);
        assert_eq!(p.at(1), [2.0, 2.0]);
        assert_eq!(p.at(2), [0.0; 2]);
    }
}
