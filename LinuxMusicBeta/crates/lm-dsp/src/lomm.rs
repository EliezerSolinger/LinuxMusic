//! The legacy program's LOMM (Lost Robot's): an upward and downward multiband compressor, in three bands (the high,
//! the mid and the low), with lookahead, feedback and mid/side. It gives the wet signal: dry and wet are the host's.

use crate::basic_filters::{BasicFilter, FilterType, LinkwitzRiley};
use crate::math::{amp_to_dbfs, dbfs_to_amp, expf};
use crate::oscillator::lerp;

/// -72 dBFS: the detectors' floor
const MIN_FLOOR: f32 = 0.00012589;
/// ms: the longest lookahead
pub const MAX_LOOKAHEAD: f32 = 20.0;
/// The crest factor at which the Auto Time knob leaves the times as they are
const AUTO_TIME_ADJUST: f32 = 5.0;

/// One band's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LommBand {
    /// dB, -48 to 48
    pub input: f32,
    /// dB, -48 to 48
    pub output: f32,
    /// dBFS: where the downward compression starts
    pub above_threshold: f32,
    /// 1 to 99.99
    pub above_ratio: f32,
    /// dBFS: where the upward compression starts
    pub below_threshold: f32,
    /// 1 to 99.99
    pub below_ratio: f32,
    /// ms, 0 to 1000 (times the Time knob)
    pub attack: f32,
    /// ms, 0 to 1000 (times the Time knob)
    pub release: f32,
    pub enabled: bool,
}

/// The LOMM's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LommSettings {
    /// The high, the mid and the low band
    pub bands: [LommBand; 3],
    /// 0 to 1
    pub depth: f32,
    /// 0 to 10: scales every band's attack and release
    pub time: f32,
    /// dB, -48 to 48
    pub input: f32,
    /// dB, -48 to 48
    pub output: f32,
    /// 0 to 2
    pub upward: f32,
    /// 0 to 2
    pub downward: f32,
    /// Hz: the high/mid and the mid/low splits
    pub splits: [f32; 2],
    pub splits_enabled: [bool; 2],
    /// ms, 0 (peak) to 500
    pub rms_time: f32,
    /// dB, 0 to 36
    pub knee: f32,
    /// dB, 0 to 96: the most gain
    pub range: f32,
    /// dB, -18 to 18: the right side louder into the detectors (the left quieter)
    pub balance: f32,
    /// The output volume scaled with the depth
    pub depth_scaling: bool,
    pub stereo_link: bool,
    /// 0 to 1: the times follow the crest factor
    pub auto_time: f32,
    /// 0 to 1
    pub mix: f32,
    /// The detectors fed by the output (not with lookahead)
    pub feedback: bool,
    pub mid_side: bool,
    pub lookahead_enabled: bool,
    /// ms, 0 to 20
    pub lookahead: f32,
    /// In mid/side, the low side band not lifted
    pub low_side_upward_suppress: bool,
}

impl Default for LommSettings {
    fn default() -> Self {
        let band = |output, above_threshold, above_ratio, below_threshold, attack, release| LommBand {
            input: 0.0,
            output,
            above_threshold,
            above_ratio,
            below_threshold,
            below_ratio: 4.17,
            attack,
            release,
            enabled: true,
        };
        Self {
            bands: [
                band(4.6, -30.3, 99.99, -35.6, 13.5, 132.0),
                band(0.0, -25.0, 66.7, -36.6, 22.4, 282.0),
                band(4.6, -28.6, 66.7, -35.6, 47.8, 282.0),
            ],
            depth: 0.4,
            time: 1.0,
            input: 0.0,
            output: 8.0,
            upward: 1.0,
            downward: 1.0,
            splits: [2500.0, 88.3],
            splits_enabled: [true; 2],
            rms_time: 10.0,
            knee: 6.0,
            range: 36.0,
            balance: 0.0,
            depth_scaling: true,
            stereo_link: false,
            auto_time: 0.0,
            mix: 1.0,
            feedback: false,
            mid_side: false,
            lookahead_enabled: false,
            lookahead: 0.0,
            low_side_upward_suppress: false,
        }
    }
}

/// The LOMM
#[derive(Clone, Debug)]
pub struct Lomm {
    sample_rate: f32,
    lp1: LinkwitzRiley,
    lp2: LinkwitzRiley,
    hp1: LinkwitzRiley,
    hp2: LinkwitzRiley,
    /// The low band's all-pass, to keep it in phase with the other two
    ap: BasicFilter,
    splits: [Option<f32>; 2],
    coeff_precalc: f32,
    crest_time: f32,
    // [band][channel]
    y: [[f32; 2]; 3],
    rms: [[f32; 2]; 3],
    gain: [[f32; 2]; 3],
    prev_out: [[f32; 2]; 3],
    crest_peak: [f32; 2],
    crest_rms: [f32; 2],
    in_look: [[Vec<f32>; 2]; 3],
    sc_look: [[Vec<f32>; 2]; 3],
    look_write: usize,
    /// The meters: each band's detected level and its level after the gain, dBFS
    pub display_in: [[f32; 2]; 3],
    pub display_out: [[f32; 2]; 3],
}

impl Lomm {
    pub fn new(sample_rate: f32) -> Self {
        let look = ((MAX_LOOKAHEAD / 1000.0) * sample_rate).ceil() as usize + 2;
        let mut ap = BasicFilter::new(sample_rate);
        ap.set_filter_type(FilterType::AllPass);
        Self {
            sample_rate,
            lp1: LinkwitzRiley::new(sample_rate),
            lp2: LinkwitzRiley::new(sample_rate),
            hp1: LinkwitzRiley::new(sample_rate),
            hp2: LinkwitzRiley::new(sample_rate),
            ap,
            splits: [None; 2],
            coeff_precalc: -2.2 / (sample_rate * 0.001),
            crest_time: expf(-1.0 / (0.2 * sample_rate)),
            y: [[MIN_FLOOR; 2]; 3],
            rms: [[MIN_FLOOR; 2]; 3],
            gain: [[MIN_FLOOR; 2]; 3],
            prev_out: [[MIN_FLOOR; 2]; 3],
            crest_peak: [MIN_FLOOR; 2],
            crest_rms: [MIN_FLOOR; 2],
            in_look: core::array::from_fn(|_| core::array::from_fn(|_| vec![0.0; look])),
            sc_look: core::array::from_fn(|_| core::array::from_fn(|_| vec![MIN_FLOOR; look])),
            look_write: 0,
            display_in: [[MIN_FLOOR; 2]; 3],
            display_out: [[MIN_FLOOR; 2]; 3],
        }
    }

    pub fn reset(&mut self) {
        for filter in [&mut self.lp1, &mut self.lp2, &mut self.hp1, &mut self.hp2] {
            filter.clear_history();
        }
        self.ap.clear_history();
        for band in &mut self.in_look {
            for side in band {
                side.fill(0.0);
            }
        }
        for band in &mut self.sc_look {
            for side in band {
                side.fill(MIN_FLOOR);
            }
        }
        self.look_write = 0;
        for state in [&mut self.y, &mut self.rms, &mut self.gain, &mut self.prev_out] {
            *state = [[MIN_FLOOR; 2]; 3];
        }
        self.display_in = [[MIN_FLOOR; 2]; 3];
        self.display_out = [[MIN_FLOOR; 2]; 3];
        self.crest_peak = [MIN_FLOOR; 2];
        self.crest_rms = [MIN_FLOOR; 2];
    }

    fn ms_to_coeff(&self, ms: f32) -> f32 {
        if ms == 0.0 { 0.0 } else { expf(self.coeff_precalc / ms) }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &LommSettings) {
        if self.splits[0] != Some(s.splits[0]) {
            self.lp1.set_lowpass(s.splits[0]);
            self.hp1.set_highpass(s.splits[0]);
            self.ap.calc_filter_coeffs(s.splits[0], core::f32::consts::FRAC_1_SQRT_2);
            self.splits[0] = Some(s.splits[0]);
        }
        if self.splits[1] != Some(s.splits[1]) {
            self.lp2.set_lowpass(s.splits[1]);
            self.hp2.set_highpass(s.splits[1]);
            self.splits[1] = Some(s.splits[1]);
        }
        let sr = self.sample_rate;
        let in_vol = dbfs_to_amp(s.input);
        let out_vol = dbfs_to_amp(s.output);
        let in_band = s.bands.map(|b| dbfs_to_amp(b.input));
        let out_band = s.bands.map(|b| dbfs_to_amp(b.output));
        let a_thresh = s.bands.map(|b| b.above_threshold);
        let a_ratio = s.bands.map(|b| 1.0 / b.above_ratio);
        let b_thresh = s.bands.map(|b| b.below_threshold);
        let b_ratio = s.bands.map(|b| 1.0 / b.below_ratio);
        let atk = s.bands.map(|b| b.attack * s.time);
        let rel = s.bands.map(|b| b.release * s.time);
        let atk_coef = atk.map(|ms| self.ms_to_coeff(ms));
        let rel_coef = rel.map(|ms| self.ms_to_coeff(ms));
        let rms_time = s.rms_time;
        let rms_const = if rms_time == 0.0 { 0.0 } else { expf(-1.0 / (rms_time * 0.001 * sr)) };
        let knee = s.knee * 0.5;
        let range_amp = dbfs_to_amp(s.range);
        let balance = dbfs_to_amp(s.balance);
        let balance_amp = [1.0 / balance, balance];
        let auto_time = s.auto_time * s.auto_time;
        let mix = s.mix;
        let look_len = self.in_look[0][0].len();
        let lookahead = ((s.lookahead / 1000.0) * sr).ceil() as usize;
        let feedback = s.feedback && !s.lookahead_enabled;
        let suppress = s.low_side_upward_suppress && s.mid_side;
        let enabled = s.bands.map(|b| if b.enabled { 1.0f32 } else { 0.0 });
        let depth = s.depth;

        for frame in frames.iter_mut() {
            let mut x = *frame;
            // Mid and side, the side 6 dB louder to bring it near the mid
            if s.mid_side {
                let l = x[0];
                x[0] = (x[0] + x[1]) * 0.5;
                x[1] = l - x[1];
            }
            let mut bands = [[0.0f32; 2]; 3];
            let mut dry = [[0.0f32; 2]; 3];
            for i in 0..2 {
                // The crest factor, for the Auto Time knob: the higher, the faster
                let sq = x[i] * x[i];
                self.crest_peak[i] =
                    MIN_FLOOR.max(sq).max(self.crest_time * self.crest_peak[i] + (1.0 - self.crest_time) * sq);
                self.crest_rms[i] = MIN_FLOOR.max(self.crest_time * self.crest_rms[i] + ((1.0 - self.crest_time) * sq));
                let crest = self.crest_peak[i] / self.crest_rms[i];
                let crest_adjusted = ((crest - AUTO_TIME_ADJUST) * auto_time) + AUTO_TIME_ADJUST;

                // The crossovers
                bands[2][i] = self.lp2.update(x[i], i);
                bands[1][i] = self.hp2.update(x[i], i);
                bands[0][i] = self.hp1.update(bands[1][i], i);
                bands[1][i] = self.lp1.update(bands[1][i], i);
                bands[2][i] = self.ap.update(bands[2][i], i);
                if !s.splits_enabled[0] {
                    bands[1][i] += bands[0][i];
                    bands[0][i] = 0.0;
                }
                if !s.splits_enabled[1] {
                    bands[1][i] += bands[2][i];
                    bands[2][i] = 0.0;
                }
                for j in 0..3 {
                    bands[j][i] *= enabled[j];
                }

                for j in 0..3 {
                    dry[j][i] = bands[j][i];
                    if feedback {
                        bands[j][i] = self.prev_out[j][i];
                    }
                    bands[j][i] *= in_band[j] * in_vol * balance_amp[i];
                    let detect = if rms_time > 0.0 {
                        self.rms[j][i] = rms_const * self.rms[j][i] + ((1.0 - rms_const) * (bands[j][i] * bands[j][i]));
                        MIN_FLOOR.max(self.rms[j][i].sqrt())
                    } else {
                        MIN_FLOOR.max(bands[j][i].abs())
                    };
                    let coeff = if detect > self.y[j][i] {
                        if auto_time != 0.0 {
                            self.ms_to_coeff(AUTO_TIME_ADJUST * atk[j] / crest_adjusted)
                        } else {
                            atk_coef[j]
                        }
                    } else if auto_time != 0.0 {
                        self.ms_to_coeff(AUTO_TIME_ADJUST * rel[j] / crest_adjusted)
                    } else {
                        rel_coef[j]
                    };
                    self.y[j][i] = MIN_FLOOR.max(self.y[j][i] * coeff + (1.0 - coeff) * detect);

                    let mut y_amp = self.y[j][i];
                    if s.lookahead_enabled {
                        // The louder of the level now and the level the lookahead ago
                        let now = y_amp;
                        let sc = &mut self.sc_look[j][i];
                        y_amp = sc[self.look_write].max(sc[(self.look_write + look_len - lookahead) % look_len]);
                        sc[self.look_write] = now;
                    }
                    let y_db = amp_to_dbfs(y_amp);

                    // Downward
                    let mut above = if y_db - a_thresh[j] < -knee {
                        y_db
                    } else if y_db - a_thresh[j] < knee {
                        let t = y_db - a_thresh[j] + knee;
                        y_db + (a_ratio[j] - 1.0) * t * t / (4.0 * knee)
                    } else {
                        a_thresh[j] + (y_db - a_thresh[j]) * a_ratio[j]
                    };
                    if above < y_db {
                        let amount = s.downward * depth;
                        above = if amount <= 1.0 {
                            lerp(y_db, above, amount)
                        } else {
                            lerp(above, a_thresh[j], amount - 1.0)
                        };
                    }
                    // Upward
                    let mut below = if y_db - b_thresh[j] > knee {
                        y_db
                    } else if b_thresh[j] - y_db < knee {
                        let t = b_thresh[j] - y_db + knee;
                        y_db + (1.0 - b_ratio[j]) * t * t / (4.0 * knee)
                    } else {
                        b_thresh[j] + (y_db - b_thresh[j]) * b_ratio[j]
                    };
                    if below > y_db {
                        let amount = s.upward * depth;
                        below = if amount <= 1.0 {
                            lerp(y_db, below, amount)
                        } else {
                            lerp(below, b_thresh[j], amount - 1.0)
                        };
                    }

                    self.display_in[j][i] = y_db;
                    let mut gain = (dbfs_to_amp(above) / y_amp) * (dbfs_to_amp(below) / y_amp);
                    if suppress && gain > 1.0 && j == 2 && i == 1 {
                        gain = 1.0;
                    }
                    self.gain[j][i] = gain.min(range_amp);
                    self.display_out[j][i] = amp_to_dbfs(MIN_FLOOR.max(y_amp * self.gain[j][i]));

                    // Linked: both sides by the more reduced
                    if s.stereo_link && i == 1 {
                        if self.gain[j][1] < self.gain[j][0] {
                            self.gain[j][0] = self.gain[j][1];
                            self.display_out[j][0] =
                                self.display_in[j][0] - (self.display_in[j][1] - self.display_out[j][1]);
                        } else {
                            self.gain[j][1] = self.gain[j][0];
                            self.display_out[j][1] =
                                self.display_in[j][1] - (self.display_in[j][0] - self.display_out[j][0]);
                        }
                    }
                }
            }

            for i in 0..2 {
                for j in 0..3 {
                    if s.lookahead_enabled {
                        core::mem::swap(&mut self.in_look[j][i][self.look_write], &mut bands[j][i]);
                        dry[j][i] = bands[j][i];
                    } else if feedback {
                        bands[j][i] = dry[j][i] * in_band[j] * in_vol * balance_amp[i];
                    }
                    bands[j][i] *= self.gain[j][i];
                    self.prev_out[j][i] = bands[j][i];
                    bands[j][i] *= out_band[j];
                    bands[j][i] = lerp(dry[j][i], bands[j][i], mix);
                }
                x[i] = bands[0][i] + bands[1][i] + bands[2][i];
                x[i] *= lerp(1.0, out_vol, mix * if s.depth_scaling { depth } else { 1.0 });
            }

            if s.mid_side {
                let m = x[0];
                x[0] += x[1] * 0.5;
                x[1] = m - (x[1] * 0.5);
            }
            self.look_write = if self.look_write == 0 { look_len - 1 } else { self.look_write - 1 };
            *frame = x;
        }
    }
}
