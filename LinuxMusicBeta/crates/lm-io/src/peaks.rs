//! Waveform peaks (interfaces.md, I-12, I-20.4, U3): a pyramid of (minimum, maximum) pairs in `i16`, like texture
//! mipmaps. Level 0 has a pair every [`BASE`] frames per channel; each level above joins [`FACTOR`] pairs of the one
//! below (64, 256, 1024, 4096... frames per pair). Immutable once built, kept by the hash of the audio in
//! `audio/<hash>.peaks`; the interface never reads audio to draw.
//!
//! # The file format (version 1, little endian)
//!
//! | bytes | field |
//! |---|---|
//! | 8 | magic `LMPEAKS\0` |
//! | 4 | version (1) |
//! | 4 | channels |
//! | 8 | frames of the audio |
//! | 4 | base (64) |
//! | 4 | factor (4) |
//! | 4 | levels |
//! | 8 × levels | pairs per channel in each level |
//! | ... | each level in order, each channel in order, `pairs` × (min `i16`, max `i16`) |
//!
//! A reader checks every size against the file's length: a truncated or foreign file is `io.peaks.malformed`.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use lm_base::{Error, Result};

/// Frames per pair at level 0
pub const BASE: usize = 64;
/// Pairs of a level joined into one of the level above
pub const FACTOR: usize = 4;
const MAGIC: &[u8; 8] = b"LMPEAKS\0";
const VERSION: u32 = 1;

/// A sample in `[-1, 1]` as an `i16`, rounding away from zero so a pair always contains the samples it covers
fn quantize_min(x: f32) -> i16 {
    let v = (x.clamp(-1.0, 1.0) * 32767.0).floor();
    v.max(-32767.0) as i16
}

fn quantize_max(x: f32) -> i16 {
    let v = (x.clamp(-1.0, 1.0) * 32767.0).ceil();
    v.min(32767.0) as i16
}

/// The pyramid of one audio resource
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peaks {
    channels: usize,
    frames: u64,
    /// Per level: `channels` runs of `pairs` × [min, max]
    levels: Vec<Level>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Level {
    pairs: usize,
    data: Vec<i16>,
}

impl Peaks {
    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// Frames covered by one pair of `level`
    pub fn frames_per_pair(level: usize) -> u64 {
        BASE as u64 * (FACTOR as u64).pow(level as u32)
    }

    /// Pairs per channel in `level`
    pub fn pairs(&self, level: usize) -> usize {
        self.levels.get(level).map_or(0, |l| l.pairs)
    }

    /// The pairs `from..to` of `channel` in `level`, as `[min, max, min, max, ...]` (clamped to what exists)
    pub fn range(&self, level: usize, channel: usize, from: usize, to: usize) -> &[i16] {
        let Some(l) = self.levels.get(level) else { return &[] };
        if channel >= self.channels {
            return &[];
        }
        let to = to.min(l.pairs);
        let from = from.min(to);
        let base = channel * l.pairs * 2;
        &l.data[base + from * 2..base + to * 2]
    }

    /// One pair: (min, max)
    pub fn pair(&self, level: usize, channel: usize, index: usize) -> Option<(i16, i16)> {
        let r = self.range(level, channel, index, index + 1);
        (r.len() == 2).then(|| (r[0], r[1]))
    }

    /// The level whose pairs are the closest to (not more than) `frames_per_pixel`
    pub fn level_for(&self, frames_per_pixel: f64) -> usize {
        let mut level = 0;
        while level + 1 < self.levels.len() && Self::frames_per_pair(level + 1) as f64 <= frames_per_pixel {
            level += 1;
        }
        level
    }

    /// Writes the file: to `<path>.tmp`, synced, then renamed (an interrupted write never leaves a half file)
    pub fn write(&self, path: &Path) -> Result<()> {
        // A name of its own: two identical takes finalised at once write the same peaks file
        let tmp = path.with_extension(format!("peaks.{}.tmp", crate::take::new_uuid()));
        let io = |e: std::io::Error| {
            Error::new("io.peaks.write_failed").arg("path", path.display().to_string()).arg("message", e.to_string())
        };
        let file = File::create(&tmp).map_err(io)?;
        let mut w = BufWriter::new(file);
        w.write_all(MAGIC).map_err(io)?;
        for v in [VERSION, self.channels as u32] {
            w.write_all(&v.to_le_bytes()).map_err(io)?;
        }
        w.write_all(&self.frames.to_le_bytes()).map_err(io)?;
        for v in [BASE as u32, FACTOR as u32, self.levels.len() as u32] {
            w.write_all(&v.to_le_bytes()).map_err(io)?;
        }
        for l in &self.levels {
            w.write_all(&(l.pairs as u64).to_le_bytes()).map_err(io)?;
        }
        for l in &self.levels {
            let mut bytes = Vec::with_capacity(l.data.len() * 2);
            for v in &l.data {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            w.write_all(&bytes).map_err(io)?;
        }
        let file = w.into_inner().map_err(|e| io(e.into_error()))?;
        file.sync_all().map_err(io)?;
        drop(file);
        std::fs::rename(&tmp, path).map_err(io)?;
        Ok(())
    }

    /// Reads a peaks file, checking every size
    pub fn read(path: &Path) -> Result<Peaks> {
        let mut bytes = Vec::new();
        File::open(path).and_then(|mut f| f.read_to_end(&mut bytes)).map_err(|e| {
            Error::new("io.peaks.read_failed").arg("path", path.display().to_string()).arg("message", e.to_string())
        })?;
        Self::parse(&bytes).ok_or_else(|| Error::new("io.peaks.malformed").arg("path", path.display().to_string()))
    }

    fn parse(bytes: &[u8]) -> Option<Peaks> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = bytes.get(at..at.checked_add(n)?)?;
            at += n;
            Some(s)
        };
        if take(8)? != MAGIC {
            return None;
        }
        let u32_at = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        let u64_at = |s: &[u8]| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
        if u32_at(take(4)?) != VERSION {
            return None;
        }
        let channels = u32_at(take(4)?) as usize;
        let frames = u64_at(take(8)?);
        let base = u32_at(take(4)?) as usize;
        let factor = u32_at(take(4)?) as usize;
        let count = u32_at(take(4)?) as usize;
        if base != BASE || factor != FACTOR || channels == 0 || channels > 1024 || count > 64 {
            return None;
        }
        let mut pairs = Vec::with_capacity(count);
        for level in 0..count {
            let p = usize::try_from(u64_at(take(8)?)).ok()?;
            // Each level's size follows from the frames: a mismatch is a foreign or damaged file
            if p as u64 != frames.div_ceil(Self::frames_per_pair(level)) {
                return None;
            }
            pairs.push(p);
        }
        let mut levels = Vec::with_capacity(count);
        for p in pairs {
            let n = p.checked_mul(channels)?.checked_mul(2)?;
            let raw = take(n.checked_mul(2)?)?;
            let data = raw.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();
            levels.push(Level { pairs: p, data });
        }
        if at != bytes.len() {
            return None;
        }
        Some(Peaks { channels, frames, levels })
    }
}

/// Builds a pyramid as audio streams in (decoding, recording); nothing is kept but the pairs
#[derive(Clone, Debug)]
pub struct PeaksBuilder {
    channels: usize,
    frames: u64,
    /// The level-0 pairs per channel, then joined at `finish`
    level0: Vec<Vec<i16>>,
    current_min: Vec<f32>,
    current_max: Vec<f32>,
    in_pair: usize,
}

impl PeaksBuilder {
    pub fn new(channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            channels,
            frames: 0,
            level0: vec![Vec::new(); channels],
            current_min: vec![f32::INFINITY; channels],
            current_max: vec![f32::NEG_INFINITY; channels],
            in_pair: 0,
        }
    }

    /// Adds interleaved frames. Returns how many level-0 pairs were completed (the newest are [`Self::last_pairs`])
    pub fn push_interleaved(&mut self, samples: &[f32]) -> usize {
        let ch = self.channels;
        let mut completed = 0;
        for frame in samples.chunks_exact(ch) {
            for (c, &x) in frame.iter().enumerate() {
                let x = if x.is_finite() { x } else { 0.0 };
                self.current_min[c] = self.current_min[c].min(x);
                self.current_max[c] = self.current_max[c].max(x);
            }
            self.in_pair += 1;
            self.frames += 1;
            if self.in_pair == BASE {
                self.close_pair();
                completed += 1;
            }
        }
        completed
    }

    /// Adds a level-0 pair made elsewhere (the live peaks of a take being written, U3): one (min, max) per channel
    /// (a channel past the end of the slices takes the last one). Only between whole pairs
    pub fn push_pair(&mut self, mins: &[i16], maxs: &[i16]) {
        if self.in_pair != 0 {
            return;
        }
        for c in 0..self.channels {
            let lo = mins.get(c).or(mins.last()).copied().unwrap_or(0);
            let hi = maxs.get(c).or(maxs.last()).copied().unwrap_or(0);
            self.level0[c].push(lo);
            self.level0[c].push(hi);
        }
        self.frames += BASE as u64;
    }

    /// The pyramid of what came in so far; the builder goes on
    pub fn snapshot(&self) -> Peaks {
        self.clone().finish()
    }

    /// Adds silence (a capture gap, C2)
    pub fn push_silence(&mut self, frames: u64) -> usize {
        let zeros = [0.0f32; 256];
        let per = (zeros.len() / self.channels).max(1);
        let mut left = frames;
        let mut completed = 0;
        while left > 0 {
            let n = (left as usize).min(per);
            completed += self.push_interleaved(&zeros[..n * self.channels]);
            left -= n as u64;
        }
        completed
    }

    fn close_pair(&mut self) {
        for c in 0..self.channels {
            self.level0[c].push(quantize_min(self.current_min[c]));
            self.level0[c].push(quantize_max(self.current_max[c]));
            self.current_min[c] = f32::INFINITY;
            self.current_max[c] = f32::NEG_INFINITY;
        }
        self.in_pair = 0;
    }

    /// Level-0 pairs completed so far, per channel
    pub fn pairs(&self) -> usize {
        self.level0.first().map_or(0, |l| l.len() / 2)
    }

    /// The `count` newest level-0 pairs of `channel`, as [min, max, ...]
    pub fn last_pairs(&self, channel: usize, count: usize) -> &[i16] {
        let l = &self.level0[channel];
        &l[l.len() - (count * 2).min(l.len())..]
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The pyramid: the partial last pair is closed, then each level joins `FACTOR` pairs of the one below, up to a
    /// level with one pair
    pub fn finish(mut self) -> Peaks {
        if self.in_pair > 0 {
            self.close_pair();
        }
        let channels = self.channels;
        let pairs0 = self.pairs();
        let mut level = Level { pairs: pairs0, data: self.level0.concat() };
        let mut levels = Vec::new();
        loop {
            let next_pairs = level.pairs.div_ceil(FACTOR);
            let more = level.pairs > 1;
            if more {
                let mut data = Vec::with_capacity(next_pairs * 2 * channels);
                for c in 0..channels {
                    let run = &level.data[c * level.pairs * 2..(c + 1) * level.pairs * 2];
                    for group in run.chunks(FACTOR * 2) {
                        let mut lo = i16::MAX;
                        let mut hi = i16::MIN;
                        for [l, h] in group.as_chunks::<2>().0 {
                            lo = lo.min(*l);
                            hi = hi.max(*h);
                        }
                        data.push(lo);
                        data.push(hi);
                    }
                }
                let next = Level { pairs: next_pairs, data };
                levels.push(std::mem::replace(&mut level, next));
            } else {
                levels.push(level);
                break;
            }
        }
        Peaks { channels, frames: self.frames, levels }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyramid_levels_and_round_trip() {
        let mut b = PeaksBuilder::new(2);
        let frames = 64 * 4 * 4 * 3 + 10;
        let mut samples = Vec::new();
        for i in 0..frames {
            let x = ((i as f32) * 0.01).sin() * 0.5;
            samples.push(x);
            samples.push(-x * 0.25);
        }
        b.push_interleaved(&samples);
        let p = b.finish();
        assert_eq!(p.frames(), frames as u64);
        assert_eq!(p.pairs(0), (frames as u64).div_ceil(64) as usize);
        for level in 0..p.levels() {
            assert_eq!(p.pairs(level) as u64, (frames as u64).div_ceil(Peaks::frames_per_pair(level)));
        }
        assert_eq!(p.pairs(p.levels() - 1), 1);
        // Each pair contains every sample it covers
        for (i, s) in samples.as_chunks::<2>().0.iter().enumerate() {
            let (lo, hi) = p.pair(0, 0, i / 64).expect("pair");
            assert!(f32::from(lo) / 32767.0 <= s[0] + 1e-6 && f32::from(hi) / 32767.0 >= s[0] - 1e-6);
        }
        let (lo, hi) = p.pair(p.levels() - 1, 0, 0).expect("top");
        assert!(lo <= -16380 && hi >= 16380);
        let dir = std::env::temp_dir().join(format!("lm-io-peaks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("x.peaks");
        p.write(&path).expect("write");
        assert_eq!(Peaks::read(&path).expect("read"), p);
        // A truncated file is refused
        let bytes = std::fs::read(&path).expect("bytes");
        std::fs::write(&path, &bytes[..bytes.len() - 1]).expect("truncate");
        assert_eq!(Peaks::read(&path).expect_err("truncated").key(), "io.peaks.malformed");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn pairs_pushed_one_by_one_make_the_same_pyramid_as_samples() {
        let samples: Vec<f32> = (0..64 * 9).map(|i| ((i as f32) * 0.05).sin() * 0.7).collect();
        let mut from_samples = PeaksBuilder::new(1);
        from_samples.push_interleaved(&samples);
        let reference = from_samples.snapshot();
        let mut by_pairs = PeaksBuilder::new(1);
        for k in 0..9 {
            let pair = reference.pair(0, 0, k).expect("pair");
            by_pairs.push_pair(&[pair.0], &[pair.1]);
        }
        assert_eq!(by_pairs.snapshot(), reference);
        // A snapshot does not end the builder
        by_pairs.push_pair(&[0], &[0]);
        assert_eq!(by_pairs.snapshot().pairs(0), 10);
    }

    #[test]
    fn level_for_zoom() {
        let mut b = PeaksBuilder::new(1);
        b.push_silence(1 << 20);
        let p = b.finish();
        assert_eq!(p.level_for(10.0), 0);
        assert_eq!(p.level_for(256.0), 1);
        assert_eq!(p.level_for(1500.0), 2);
    }
}
