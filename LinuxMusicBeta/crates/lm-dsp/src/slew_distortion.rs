//! The legacy program's Slew Distortion (Lost Robot's): a slew-rate limiter into a waveshaper, on one band or two
//! (split by a Linkwitz-Riley crossover), oversampled up to 32 times. It gives the wet signal: dry and wet are the
//! host's.

use crate::basic_filters::LinkwitzRiley;
use crate::math::{dbfs_to_amp, expf, f32c::PI, powf};
use crate::oscillator::lerp;
use crate::oversampling::{MAX_FACTOR, MAX_STAGES, VariableDownsampler, VariableUpsampler};

/// -72 dBFS: the envelopes' floor
const MIN_FLOOR: f32 = 0.0012589;
/// Hz: the DC follower's corner
const DC_FREQ: f32 = 7.0;

/// The shapes of the distortion
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlewShape {
    HardClip,
    Tanh,
    FastSoftClip1,
    FastSoftClip2,
    Sinusoidal,
    Foldback,
    FullRectify,
    HalfRectify,
    SmoothRectify,
    Bitcrush,
}

impl SlewShape {
    /// In the legacy program's order
    pub const ALL: [SlewShape; 10] = [
        SlewShape::HardClip,
        SlewShape::Tanh,
        SlewShape::FastSoftClip1,
        SlewShape::FastSoftClip2,
        SlewShape::Sinusoidal,
        SlewShape::Foldback,
        SlewShape::FullRectify,
        SlewShape::HalfRectify,
        SlewShape::SmoothRectify,
        SlewShape::Bitcrush,
    ];

    pub fn key(self) -> &'static str {
        match self {
            SlewShape::HardClip => "hard-clip",
            SlewShape::Tanh => "tanh",
            SlewShape::FastSoftClip1 => "fast-soft-clip-1",
            SlewShape::FastSoftClip2 => "fast-soft-clip-2",
            SlewShape::Sinusoidal => "sinusoidal",
            SlewShape::Foldback => "foldback",
            SlewShape::FullRectify => "full-rectify",
            SlewShape::HalfRectify => "half-rectify",
            SlewShape::SmoothRectify => "smooth-rectify",
            SlewShape::Bitcrush => "bitcrush",
        }
    }

    pub fn from_key(key: &str) -> Option<SlewShape> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// One band's knobs (the high band's, or the only one's; then the low band's)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlewBand {
    pub shape: SlewShape,
    /// dB, -24 to 24
    pub drive: f32,
    /// dB, -96 to 6: the most the signal rises in a sample
    pub slew_up: f32,
    /// dB, -96 to 6: the most it falls (when not linked to the rise)
    pub slew_down: f32,
    /// -2 to 2
    pub bias: f32,
    /// 0 to 0.99
    pub warp: f32,
    /// dB, 0 to 24
    pub crush: f32,
    /// dB, -24 to 24
    pub out_volume: f32,
    /// ms, 0.01 to 200: the envelope followers'
    pub attack: f32,
    /// ms, 0.01 to 800
    pub release: f32,
    /// 0 to 1: how much the output follows the input's envelope
    pub dynamics: f32,
    /// -8 to 8: how much the input's envelope scales the slew
    pub dynamic_slew: f32,
    /// 0 to 1
    pub mix: f32,
    /// The fall limited as the rise
    pub slew_link: bool,
}

impl Default for SlewBand {
    fn default() -> Self {
        Self {
            shape: SlewShape::HardClip,
            drive: 0.0,
            slew_up: 6.0,
            slew_down: 6.0,
            bias: 0.0,
            warp: 0.0,
            crush: 0.0,
            out_volume: 0.0,
            attack: 2.0,
            release: 20.0,
            dynamics: 0.0,
            dynamic_slew: 0.0,
            mix: 1.0,
            slew_link: true,
        }
    }
}

/// The Slew Distortion's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlewDistortionSettings {
    pub bands: [SlewBand; 2],
    pub dc_remove: bool,
    pub multiband: bool,
    /// 0 to 5: oversampled 2^this times
    pub oversampling: usize,
    /// Hz, 100 to 20000: the bands' split
    pub split: f32,
}

impl Default for SlewDistortionSettings {
    fn default() -> Self {
        Self { bands: [SlewBand::default(); 2], dc_remove: true, multiband: false, oversampling: 0, split: 200.0 }
    }
}

/// One band's values worked out for a call
#[derive(Clone, Copy)]
struct Prepared {
    shape: SlewShape,
    drive: f32,
    slew_up: f32,
    slew_down: f32,
    warp: f32,
    crush: f32,
    out_volume: f32,
    attack: f32,
    release: f32,
    dynamics: f32,
    dynamic_slew: f32,
    mix: f32,
    slew_link: bool,
}

/// The Slew Distortion
#[derive(Clone, Debug)]
pub struct SlewDistortion {
    sample_rate: f32,
    stages: Option<usize>,
    upsampler: [VariableUpsampler; 2],
    downsampler: [VariableDownsampler; 2],
    lp: LinkwitzRiley,
    hp: LinkwitzRiley,
    split: Option<f32>,
    coeff_precalc: f32,
    dc_coeff: f32,
    bias_coeff: f32,
    // Per channel of each band: high (or only) left and right, low left and right
    slew_out: [f32; 4],
    dc_offset: [f32; 4],
    in_env: [f32; 4],
    out_env: [f32; 4],
    // The loudest input (times the band's drive) and output of each channel since the last `take_peaks`
    in_peak: [f32; 4],
    out_peak: [f32; 4],
    over: [[f32; MAX_FACTOR]; 2],
    true_bias: [f32; 2],
}

impl SlewDistortion {
    pub fn new(sample_rate: f32) -> Self {
        let mut s = Self {
            sample_rate,
            stages: None,
            upsampler: [VariableUpsampler::new(0, sample_rate), VariableUpsampler::new(0, sample_rate)],
            downsampler: [VariableDownsampler::new(0, sample_rate), VariableDownsampler::new(0, sample_rate)],
            lp: LinkwitzRiley::new(sample_rate),
            hp: LinkwitzRiley::new(sample_rate),
            split: None,
            coeff_precalc: 0.0,
            dc_coeff: 0.0,
            bias_coeff: 0.0,
            slew_out: [0.0; 4],
            dc_offset: [0.0; 4],
            in_env: [0.0; 4],
            out_env: [0.0; 4],
            in_peak: [0.0; 4],
            out_peak: [0.0; 4],
            over: [[0.0; MAX_FACTOR]; 2],
            true_bias: [0.0; 2],
        };
        s.set_stages(0);
        s
    }

    /// The panel's meters (the legacy's): the peaks since the last call, as the input's (times the band's drive) and
    /// the output's per channel (high band left and right, low band left and right), and starts again
    pub fn take_peaks(&mut self) -> ([f32; 4], [f32; 4]) {
        (std::mem::take(&mut self.in_peak), std::mem::take(&mut self.out_peak))
    }

    pub fn reset(&mut self) {
        self.slew_out = [0.0; 4];
        self.dc_offset = [0.0; 4];
        self.in_env = [0.0; 4];
        self.out_env = [0.0; 4];
        self.over = [[0.0; MAX_FACTOR]; 2];
        for (up, down) in self.upsampler.iter_mut().zip(self.downsampler.iter_mut()) {
            up.reset();
            down.reset();
        }
        self.lp.clear_history();
        self.hp.clear_history();
    }

    /// Prepared for 2^@p stages times oversampling: the filters at the new rate and the state let go (not the bias)
    fn set_stages(&mut self, stages: usize) {
        let stages = stages.min(MAX_STAGES);
        let rate_over = self.sample_rate * (1 << stages) as f32;
        self.upsampler = core::array::from_fn(|_| VariableUpsampler::new(stages, self.sample_rate));
        self.downsampler = core::array::from_fn(|_| VariableDownsampler::new(stages, self.sample_rate));
        self.lp.set_sample_rate(rate_over);
        self.hp.set_sample_rate(rate_over);
        self.lp.clear_history();
        self.hp.clear_history();
        self.split = None;
        self.coeff_precalc = -1.0 / (rate_over * 0.001);
        self.dc_coeff = expf(-2.0 * PI * DC_FREQ / rate_over);
        self.slew_out = [0.0; 4];
        self.dc_offset = [0.0; 4];
        self.in_env = [0.0; 4];
        self.out_env = [0.0; 4];
        self.over = [[0.0; MAX_FACTOR]; 2];
        self.bias_coeff = expf(-1.0 / (0.01 * self.sample_rate));
        self.stages = Some(stages);
    }

    fn ms_to_coeff(&self, ms: f32) -> f32 {
        if ms == 0.0 { 0.0 } else { expf(self.coeff_precalc / ms) }
    }

    fn prepare(&self, b: &SlewBand, factor: f32) -> Prepared {
        Prepared {
            shape: b.shape,
            drive: dbfs_to_amp(b.drive),
            slew_up: dbfs_to_amp(b.slew_up) / factor,
            slew_down: dbfs_to_amp(b.slew_down) / factor,
            warp: b.warp,
            crush: dbfs_to_amp(b.crush),
            out_volume: dbfs_to_amp(b.out_volume),
            attack: self.ms_to_coeff(b.attack),
            release: self.ms_to_coeff(b.release),
            dynamics: b.dynamics,
            dynamic_slew: b.dynamic_slew,
            mix: b.mix,
            slew_link: b.slew_link,
        }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &SlewDistortionSettings) {
        if self.stages != Some(s.oversampling.min(MAX_STAGES)) {
            self.set_stages(s.oversampling);
        }
        let factor = 1usize << self.stages.unwrap_or(0);
        if self.split != Some(s.split) {
            self.lp.set_lowpass(s.split);
            self.hp.set_highpass(s.split);
            self.split = Some(s.split);
        }
        let high = self.prepare(&s.bands[0], factor as f32);
        let low = self.prepare(&s.bands[1], factor as f32);
        let band = [high, high, low, low];
        let channels = if s.multiband { 4 } else { 2 };

        for frame in frames.iter_mut() {
            // The bias glides, so that moving it does not crackle
            for (b, bias) in self.true_bias.iter_mut().zip([s.bands[0].bias, s.bands[1].bias]) {
                *b = self.bias_coeff * *b + (1.0 - self.bias_coeff) * bias;
            }
            let bias = [self.true_bias[0], self.true_bias[0], self.true_bias[1], self.true_bias[1]];
            if factor > 1 {
                for ch in 0..2 {
                    self.upsampler[ch].process(&mut self.over[ch], frame[ch]);
                }
            } else {
                self.over[0][0] = frame[0];
                self.over[1][0] = frame[1];
            }

            for n in 0..factor {
                let input = if s.multiband {
                    let (l, r) = (self.over[0][n], self.over[1][n]);
                    [self.hp.update(l, 0), self.hp.update(r, 1), self.lp.update(l, 0), self.lp.update(r, 1)]
                } else {
                    [self.over[0][n], self.over[1][n], 0.0, 0.0]
                };
                // (one band: the low band's outputs are silent; the legacy program kept their last values, a
                // constant left on the output after turning the bands off)
                let mut out = [0.0f32; 4];
                for i in 0..channels {
                    let p = &band[i];
                    let abs_in = input[i].abs();
                    self.in_peak[i] = self.in_peak[i].max(abs_in * p.drive);
                    self.in_env[i] = if abs_in > self.in_env[i] {
                        self.in_env[i] * p.attack + abs_in * (1.0 - p.attack)
                    } else {
                        self.in_env[i] * p.release + abs_in * (1.0 - p.release)
                    };
                    self.in_env[i] = self.in_env[i].max(MIN_FLOOR);

                    let mut rate = input[i] - self.slew_out[i];
                    let slew_mult = if p.dynamic_slew != 0.0 { powf(self.in_env[i], p.dynamic_slew) } else { 1.0 };
                    let true_slew = (if rate >= 0.0 || p.slew_link { p.slew_up } else { p.slew_down }) * slew_mult;
                    rate = rate.clamp(-true_slew, true_slew);
                    self.slew_out[i] += rate;

                    let biased = self.slew_out[i] * p.drive + bias[i];
                    let dist_in = (biased - (p.warp / p.crush).copysign(biased)) / (1.0 - p.warp);
                    let mut dist = shape(p.shape, dist_in, p.drive);
                    dist = dist * (1.0 - p.warp) + p.warp.copysign(biased);
                    if biased.abs() < p.warp / p.crush {
                        dist = biased * p.crush;
                    }

                    self.dc_offset[i] = self.dc_offset[i] * self.dc_coeff + dist * (1.0 - self.dc_coeff);
                    // (the envelope follows the output without its offset even when the offset is left in)
                    let abs_out = (dist - self.dc_offset[i]).abs();
                    self.out_env[i] = if abs_out > self.out_env[i] {
                        self.out_env[i] * p.attack + abs_out * (1.0 - p.attack)
                    } else {
                        self.out_env[i] * p.release + abs_out * (1.0 - p.release)
                    };
                    self.out_env[i] = self.out_env[i].max(MIN_FLOOR);
                    if s.dc_remove {
                        dist -= self.dc_offset[i];
                    }
                    dist *= lerp(1.0, self.in_env[i] / self.out_env[i], p.dynamics);
                    out[i] = lerp(input[i], dist, p.mix) * p.out_volume;
                    self.out_peak[i] = self.out_peak[i].max(out[i].abs());
                }
                self.over[0][n] = out[0] + out[2];
                self.over[1][n] = out[1] + out[3];
            }

            *frame = if factor > 1 {
                [self.downsampler[0].process(&mut self.over[0]), self.downsampler[1].process(&mut self.over[1])]
            } else {
                [self.over[0][0], self.over[1][0]]
            };
        }
    }
}

/// The waveshaper
#[inline]
fn shape(shape: SlewShape, x: f32, drive: f32) -> f32 {
    match shape {
        SlewShape::HardClip => x.clamp(-1.0, 1.0),
        SlewShape::Tanh => {
            let t = x.clamp(-3.0, 3.0);
            let t2 = t * t;
            (t * (27.0 + t2) / (27.0 + 9.0 * t2)).clamp(-1.0, 1.0)
        }
        SlewShape::FastSoftClip1 => {
            let t = x.clamp(-2.0, 2.0);
            t / (1.0 + 0.25 * t * t)
        }
        SlewShape::FastSoftClip2 => {
            let t = x.clamp(-1.5, 1.5);
            t - (4.0 / 27.0) * t * t * t
        }
        SlewShape::Sinusoidal => {
            // A polynomial sine (x - x^3/6 + x^5/120) over a folded phase
            let mut m = (x - PI * 0.5) % (2.0 * PI);
            if m < 0.0 {
                m += 2.0 * PI;
            }
            let v = (m - PI).abs() - PI * 0.5;
            let v2 = v * v;
            let v3 = v2 * v;
            let v5 = v3 * v2;
            v - (v3 / 6.0) + (v5 / 120.0)
        }
        SlewShape::Foldback => (((x - 1.0) % 4.0).abs() - 2.0).abs() - 1.0,
        SlewShape::FullRectify => x.abs(),
        SlewShape::SmoothRectify => (x * x + 0.04).sqrt() - 0.2,
        SlewShape::HalfRectify => x.max(0.0),
        SlewShape::Bitcrush => {
            let scale = 16.0 / drive;
            (x / drive * scale).round() / scale
        }
    }
}
