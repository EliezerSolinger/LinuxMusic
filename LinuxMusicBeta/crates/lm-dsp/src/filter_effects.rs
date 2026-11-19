//! The legacy program's filter effects: the Dual Filter and the Crossover EQ. Each gives the wet signal: dry and wet
//! are the host's.

use crate::basic_filters::{BasicFilter, FilterType, LinkwitzRiley};
use crate::math::dbfs_to_amp;
use crate::small_effects::Ramp;

/// One of the Dual Filter's two filters
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DualFilterSide {
    pub enabled: bool,
    pub filter: FilterType,
    /// Hz, 1 to 20000 (sample-exact)
    pub cutoff: Ramp,
    /// 0.01 to 10 (sample-exact)
    pub resonance: Ramp,
    /// %, 0 to 200 (sample-exact)
    pub gain: Ramp,
}

impl Default for DualFilterSide {
    fn default() -> Self {
        Self {
            enabled: true,
            filter: FilterType::LowPass,
            cutoff: Ramp::constant(7000.0),
            resonance: Ramp::constant(0.5),
            gain: Ramp::constant(100.0),
        }
    }
}

/// The Dual Filter's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DualFilterSettings {
    pub sides: [DualFilterSide; 2],
    /// -1 (only the first) to 1 (only the second) (sample-exact)
    pub mix: Ramp,
}

impl Default for DualFilterSettings {
    fn default() -> Self {
        Self { sides: [DualFilterSide::default(); 2], mix: Ramp::constant(0.0) }
    }
}

/// The Dual Filter: two of the instruments' filters in parallel, each with its gain, crossfaded
#[derive(Clone, Debug)]
pub struct DualFilter {
    filters: [BasicFilter; 2],
    types: [Option<FilterType>; 2],
    /// The cutoff and resonance each filter's coefficients are for (none: to work out)
    current: [Option<(f32, f32)>; 2],
}

impl DualFilter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            filters: [BasicFilter::new(sample_rate), BasicFilter::new(sample_rate)],
            types: [None; 2],
            current: [None; 2],
        }
    }

    pub fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.clear_history();
        }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &DualFilterSettings) {
        for (i, side) in s.sides.iter().enumerate() {
            if self.types[i] != Some(side.filter) {
                self.filters[i].set_filter_type(side.filter);
                self.types[i] = Some(side.filter);
                self.current[i] = None;
            }
        }
        let n = frames.len();
        for (f, frame) in frames.iter_mut().enumerate() {
            let mix2 = (s.mix.at(f, n) + 1.0) * 0.5;
            let mix1 = 1.0 - mix2;
            let mut out = [0.0f32; 2];
            for (i, side) in s.sides.iter().enumerate() {
                if !side.enabled {
                    continue;
                }
                let gain = side.gain.at(f, n) * 0.01;
                let (cut, res) = (side.cutoff.at(f, n), side.resonance.at(f, n));
                if self.current[i] != Some((cut, res)) {
                    self.filters[i].calc_filter_coeffs(cut, res);
                    self.current[i] = Some((cut, res));
                }
                let mix = if i == 0 { mix1 } else { mix2 };
                for ch in 0..2 {
                    let y = self.filters[i].update(frame[ch], ch) * gain;
                    out[ch] += y * mix;
                }
            }
            *frame = out;
        }
    }
}

/// The Crossover EQ's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrossoverSettings {
    /// Hz, the bands' edges, from low to high (the host keeps them in order)
    pub crossovers: [f32; 3],
    /// dB, -60 to 30
    pub gains: [f32; 4],
    /// Each band heard (the legacy plugin's "mute" buttons, lit when the band plays)
    pub enabled: [bool; 4],
}

impl Default for CrossoverSettings {
    fn default() -> Self {
        Self { crossovers: [125.0, 1250.0, 5000.0], gains: [0.0; 4], enabled: [true; 4] }
    }
}

/// The Crossover EQ: four bands split by Linkwitz-Riley filters (the middle crossover first), each with its gain
#[derive(Clone, Debug)]
pub struct CrossoverEq {
    lp: [LinkwitzRiley; 3],
    hp: [LinkwitzRiley; 3],
    /// The crossovers the filters are set for (none: to set)
    current: [Option<f32>; 3],
    low: Vec<[f32; 2]>,
    high: Vec<[f32; 2]>,
}

impl CrossoverEq {
    /// Prepared for @p sample_rate and calls of up to @p max_period frames
    pub fn new(sample_rate: f32, max_period: usize) -> Self {
        let lr = LinkwitzRiley::new(sample_rate);
        Self {
            lp: [lr.clone(), lr.clone(), lr.clone()],
            hp: [lr.clone(), lr.clone(), lr],
            current: [None; 3],
            low: vec![[0.0; 2]; max_period],
            high: vec![[0.0; 2]; max_period],
        }
    }

    pub fn reset(&mut self) {
        for filter in self.lp.iter_mut().chain(self.hp.iter_mut()) {
            filter.clear_history();
        }
    }

    /// @p frames: up to the period it was prepared for
    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &CrossoverSettings) {
        // lp[i] and hp[i] split at crossover i (hp[0] is band 2's, lp[1] and hp[1] the first split, lp[2] band 3's)
        for i in 0..3 {
            if self.current[i] != Some(s.crossovers[i]) {
                self.lp[i].set_lowpass(s.crossovers[i]);
                self.hp[i].set_highpass(s.crossovers[i]);
                self.current[i] = Some(s.crossovers[i]);
            }
        }
        let gains = s.gains.map(dbfs_to_amp);
        let n = frames.len();
        let (low, high) = (&mut self.low[..n], &mut self.high[..n]);
        for ((l, h), x) in low.iter_mut().zip(high.iter_mut()).zip(frames.iter()) {
            for ch in 0..2 {
                l[ch] = self.lp[1].update(x[ch], ch);
                h[ch] = self.hp[1].update(x[ch], ch);
            }
        }
        frames.fill([0.0; 2]);
        // The bands in the legacy order: below the first crossover, above it, below the third, above it
        let bands: [(&mut LinkwitzRiley, &[[f32; 2]]); 4] = {
            let [lp0, _, lp2] = &mut self.lp;
            let [hp0, _, hp2] = &mut self.hp;
            [(lp0, &*low), (hp0, &*low), (lp2, &*high), (hp2, &*high)]
        };
        for (band, (filter, source)) in bands.into_iter().enumerate() {
            if !s.enabled[band] {
                continue;
            }
            for (frame, x) in frames.iter_mut().zip(source.iter()) {
                for ch in 0..2 {
                    frame[ch] += filter.update(x[ch], ch) * gains[band];
                }
            }
        }
    }
}
