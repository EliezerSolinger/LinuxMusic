//! The legacy program's Eq plugin (Dave French's): high-pass and low-pass of 12, 24 or 48 dB/oct, a low and a high
//! shelf and four peaks, the cookbook's filters in single precision, crossfaded into their new settings over each call.
//! (Its spectrum analyser is the interface's: the telemetry will give it the samples.)

use crate::math::dbfs_to_amp;
use crate::rbj::{CrossfadedBiquad, Shape};

/// One band's settings
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqBand {
    pub active: bool,
    /// Hz
    pub freq: f32,
    /// The resonance (the passes and the shelves) or the bandwidth in octaves (the peaks)
    pub q: f32,
    /// dB (the shelves and the peaks)
    pub gain: f32,
}

/// The slope of a pass: 12, 24 or 48 dB/oct
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Slope {
    #[default]
    Db12,
    Db24,
    Db48,
}

/// The Eq's controls
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSettings {
    /// dB, -60 to 20
    pub in_gain: f32,
    pub out_gain: f32,
    pub high_pass: EqBand,
    pub high_pass_slope: Slope,
    pub low_shelf: EqBand,
    pub peaks: [EqBand; 4],
    pub high_shelf: EqBand,
    pub low_pass: EqBand,
    pub low_pass_slope: Slope,
    /// The legacy program's steep passes: each of the 2 or 4 sections of a 24 or 48 dB/oct pass at the resonance, so
    /// that its peak is the resonance's to the power of 2 or 4 (+80 dB at a resonance of 10 at 48 dB/oct). Off: the
    /// resonance on the first section and the others at 0.707, the legacy default, so that the peak is the
    /// resonance's at every slope, and every setting at the default resonance sounds as in the legacy program
    pub resonance_on_every_section: bool,
}

impl Default for EqSettings {
    fn default() -> Self {
        let band = |freq, q| EqBand { active: false, freq, q, gain: 0.0 };
        Self {
            in_gain: 0.0,
            out_gain: 0.0,
            high_pass: band(31.0, 0.707),
            high_pass_slope: Slope::Db12,
            low_shelf: band(80.0, 0.707),
            peaks: [band(120.0, 0.3), band(250.0, 0.3), band(2000.0, 0.3), band(4000.0, 0.3)],
            high_shelf: band(12000.0, 0.707),
            low_pass: band(18000.0, 0.707),
            low_pass_slope: Slope::Db12,
            resonance_on_every_section: true,
        }
    }
}

/// The Eq. The wet signal: dry and wet are mixed by the host, as for every effect
#[derive(Clone, Debug)]
pub struct Eq {
    sample_rate: f32,
    hp: [CrossfadedBiquad; 4],
    low_shelf: CrossfadedBiquad,
    peaks: [CrossfadedBiquad; 4],
    high_shelf: CrossfadedBiquad,
    lp: [CrossfadedBiquad; 4],
}

impl Eq {
    pub fn new(sample_rate: f32) -> Self {
        let four = |shape| core::array::from_fn(|_| CrossfadedBiquad::new(shape));
        Self {
            sample_rate,
            hp: four(Shape::HighPass),
            low_shelf: CrossfadedBiquad::new(Shape::LowShelf),
            peaks: four(Shape::Peak),
            high_shelf: CrossfadedBiquad::new(Shape::HighShelf),
            lp: four(Shape::LowPass),
        }
    }

    /// Forgets every filter's memory (no allocation)
    pub fn reset(&mut self) {
        for f in self.hp.iter_mut().chain(self.peaks.iter_mut()).chain(self.lp.iter_mut()) {
            f.clear();
        }
        self.low_shelf.clear();
        self.high_shelf.clear();
    }

    /// Equalizes @p frames in place; returns the output's peaks, for the meters
    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &EqSettings) -> [f32; 2] {
        let rate = self.sample_rate;
        // Every filter's settings once a call; each crossfades into them over the call
        let q = |section: usize, q: f32| if section == 0 || s.resonance_on_every_section { q } else { 0.707 };
        for (i, f) in self.hp.iter_mut().enumerate() {
            f.set_parameters(rate, s.high_pass.freq, q(i, s.high_pass.q), 1.0);
        }
        self.low_shelf.set_parameters(rate, s.low_shelf.freq, s.low_shelf.q, s.low_shelf.gain);
        for (f, band) in self.peaks.iter_mut().zip(&s.peaks) {
            f.set_parameters(rate, band.freq, band.q, band.gain);
        }
        self.high_shelf.set_parameters(rate, s.high_shelf.freq, s.high_shelf.q, s.high_shelf.gain);
        for (i, f) in self.lp.iter_mut().enumerate() {
            f.set_parameters(rate, s.low_pass.freq, q(i, s.low_pass.q), 1.0);
        }

        let in_gain = dbfs_to_amp(s.in_gain);
        for frame in frames.iter_mut() {
            frame[0] *= in_gain;
            frame[1] *= in_gain;
        }

        let count = frames.len();
        for (n, frame) in frames.iter_mut().enumerate() {
            // (a call of one frame ends the crossfade on it: count - 1 would be 0)
            let progress = if count > 1 { n as f32 / (count - 1) as f32 } else { 1.0 };
            let mut through = |filter: &mut CrossfadedBiquad| {
                frame[0] = filter.process(frame[0], 0, progress);
                frame[1] = filter.process(frame[1], 1, progress);
            };
            if s.high_pass.active {
                through(&mut self.hp[0]);
                if s.high_pass_slope != Slope::Db12 {
                    through(&mut self.hp[1]);
                }
                if s.high_pass_slope == Slope::Db48 {
                    through(&mut self.hp[2]);
                    through(&mut self.hp[3]);
                }
            }
            if s.low_shelf.active {
                through(&mut self.low_shelf);
            }
            for (filter, band) in self.peaks.iter_mut().zip(&s.peaks) {
                if band.active {
                    through(filter);
                }
            }
            if s.high_shelf.active {
                through(&mut self.high_shelf);
            }
            if s.low_pass.active {
                through(&mut self.lp[0]);
                if s.low_pass_slope != Slope::Db12 {
                    through(&mut self.lp[1]);
                }
                if s.low_pass_slope == Slope::Db48 {
                    through(&mut self.lp[2]);
                    through(&mut self.lp[3]);
                }
            }
        }

        let out_gain = dbfs_to_amp(s.out_gain);
        let mut peak = [0.0f32; 2];
        for frame in frames.iter_mut() {
            frame[0] *= out_gain;
            frame[1] *= out_gain;
            peak = [peak[0].max(frame[0].abs()), peak[1].max(frame[1].abs())];
        }
        peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_neutral_eq_leaves_the_audio_alone() {
        let mut eq = Eq::new(48000.0);
        let mut frames: Vec<[f32; 2]> = (0..256).map(|n| [n as f32 / 256.0, -0.5]).collect();
        let input = frames.clone();
        eq.process(&mut frames, &EqSettings::default());
        assert_eq!(frames, input);
    }
}
