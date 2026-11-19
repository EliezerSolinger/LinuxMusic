//! The waveform peak pyramid of I-20.4: level 0 has one (min, max) pair of `i16` every 64 samples; each level above
//! joins 4 pairs of the one below (256, 1024, 4096... samples per pair). Mono here: the prototype measures drawing.

/// Samples per pair at level 0
pub const LEVEL0: u64 = 64;
/// Pairs of a level joined into one pair of the level above
pub const FACTOR: u64 = 4;

#[derive(Debug)]
pub struct Peaks {
    pub sample_rate: u32,
    /// Length of the audio in samples
    pub length: u64,
    /// `levels[l][i]` = (min, max) of samples `i * spp(l) .. (i + 1) * spp(l)`
    pub levels: Vec<Vec<(i16, i16)>>,
}

impl Peaks {
    pub fn samples_per_pair(level: usize) -> u64 {
        LEVEL0 * FACTOR.pow(level as u32)
    }

    /// The coarsest level that still has at least one pair per pixel (the right level for the zoom)
    pub fn level_for(&self, samples_per_px: f64) -> usize {
        let mut level = 0;
        while level + 1 < self.levels.len() && Self::samples_per_pair(level + 1) as f64 <= samples_per_px {
            level += 1;
        }
        level
    }

    /// One (min, max) per pixel column: column `c` covers samples `start + c * spp .. start + (c + 1) * spp`.
    /// Columns outside the audio are `None`. Reuses `out`'s allocation.
    pub fn columns(&self, start: f64, samples_per_px: f64, n: usize, out: &mut Vec<Option<(i16, i16)>>) {
        out.clear();
        let level = self.level_for(samples_per_px);
        let pairs = &self.levels[level];
        let spp = Self::samples_per_pair(level) as f64;
        for c in 0..n {
            let s0 = start + c as f64 * samples_per_px;
            let s1 = s0 + samples_per_px;
            if s1 <= 0.0 || s0 >= self.length as f64 {
                out.push(None);
                continue;
            }
            let i0 = (s0.max(0.0) / spp) as usize;
            // at least one pair even when a pair is wider than the pixel (zoomed in past level 0)
            let i1 = ((s1 / spp).ceil() as usize).max(i0 + 1).min(pairs.len());
            let (mut lo, mut hi) = (i16::MAX, i16::MIN);
            for &(a, b) in &pairs[i0.min(pairs.len() - 1)..i1] {
                lo = lo.min(a);
                hi = hi.max(b);
            }
            out.push(Some((lo, hi)));
        }
    }

    /// Peak amplitude (0..1) of the audio around `sample`, over `span` samples: what the fake engine meters
    pub fn amplitude_at(&self, sample: i64, span: u64) -> f32 {
        if sample < 0 || sample as u64 >= self.length {
            return 0.0;
        }
        let level = 1.min(self.levels.len() - 1);
        let spp = Self::samples_per_pair(level);
        let pairs = &self.levels[level];
        let i0 = (sample as u64 / spp) as usize;
        let i1 = (((sample as u64 + span) / spp) as usize + 1).min(pairs.len());
        let mut peak = 0i32;
        for &(a, b) in &pairs[i0.min(pairs.len() - 1)..i1] {
            peak = peak.max((a as i32).abs()).max(b as i32);
        }
        peak as f32 / i16::MAX as f32
    }

    /// A deterministic fake recording: `kind` picks drums, a sustained part or speech-like phrases
    pub fn synthesize(seed: u64, seconds: f64, sample_rate: u32) -> Peaks {
        let length = (seconds * sample_rate as f64) as u64;
        let n0 = length.div_ceil(LEVEL0) as usize;
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let kind = seed % 3;
        let beat = (sample_rate as f64 * 0.5 / LEVEL0 as f64) as usize; // 120 BPM, in pairs
        let gain = 0.25 + 0.6 * (seed % 5) as f64 / 4.0;
        let mut level0 = Vec::with_capacity(n0);
        for i in 0..n0 {
            let t = i as f64 / beat as f64;
            let env = match kind {
                0 => (-(t.fract()) * 6.0).exp(),                                  // drum hits on every beat
                1 => 0.55 + 0.45 * (t * 0.13).sin(),                              // a sustained part
                _ => if (t * 0.5).sin() > -0.2 { 0.4 + 0.6 * (t * 2.7).sin().abs() } else { 0.02 }, // phrases
            };
            // a silent bar now and then, so the waveform has holes
            let env = if (i / (beat * 4)) % 11 == 7 { 0.0 } else { env };
            let a = gain * env * (0.6 + 0.4 * rng.unit());
            let hi = (a * i16::MAX as f64) as i16;
            let lo = -((a * (0.7 + 0.3 * rng.unit())) * i16::MAX as f64) as i16;
            level0.push((lo, hi));
        }
        let mut levels = vec![level0];
        while levels.last().unwrap().len() > 1 {
            let prev = levels.last().unwrap();
            let next: Vec<(i16, i16)> = prev
                .chunks(FACTOR as usize)
                .map(|c| c.iter().fold((i16::MAX, i16::MIN), |(lo, hi), &(a, b)| (lo.min(a), hi.max(b))))
                .collect();
            levels.push(next);
        }
        Peaks { sample_rate, length, levels }
    }

    pub fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.len() * 4).sum()
    }
}

/// xorshift64*, enough for fake data
pub(crate) struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyramid_levels_join_four() {
        let p = Peaks::synthesize(3, 10.0, 48_000);
        assert_eq!(p.levels[0].len(), (480_000u64).div_ceil(64) as usize);
        for l in 1..p.levels.len() {
            assert_eq!(p.levels[l].len(), p.levels[l - 1].len().div_ceil(4));
            let (a, b) = p.levels[l][0];
            let (lo, hi) = p.levels[l - 1][..4.min(p.levels[l - 1].len())]
                .iter()
                .fold((i16::MAX, i16::MIN), |(lo, hi), &(a, b)| (lo.min(a), hi.max(b)));
            assert_eq!((a, b), (lo, hi));
        }
    }

    #[test]
    fn level_matches_zoom() {
        let p = Peaks::synthesize(1, 60.0, 48_000);
        assert_eq!(p.level_for(10.0), 0);
        assert_eq!(p.level_for(64.0), 0);
        assert_eq!(p.level_for(256.0), 1);
        assert_eq!(p.level_for(1000.0), 1);
        assert_eq!(p.level_for(1024.0), 2);
        let mut out = Vec::new();
        p.columns(-1000.0, 500.0, 10, &mut out);
        assert_eq!(out.len(), 10);
        assert!(out[0].is_none());
        assert!(out[5].is_some());
    }
}
