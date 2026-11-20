//! The legacy program's Dynamics Processor: a gain curve drawn over the input's RMS level. It gives the wet signal:
//! dry and wet are the host's.

use crate::math::exp;
use crate::oscillator::lerp;

/// The points of the curve, from 0 to 1 of level
pub const CURVE_POINTS: usize = 200;
/// -100 dBFS: below it, the signal is left alone
const NOISE_FLOOR: f32 = 0.00001;

/// How the two sides' levels drive their gains
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StereoMode {
    /// Both by the louder
    Maximum,
    /// Both by their mean
    Average,
    /// Each by its own
    Unlinked,
}

impl StereoMode {
    pub const ALL: [StereoMode; 3] = [StereoMode::Maximum, StereoMode::Average, StereoMode::Unlinked];

    pub fn key(self) -> &'static str {
        match self {
            StereoMode::Maximum => "maximum",
            StereoMode::Average => "average",
            StereoMode::Unlinked => "unlinked",
        }
    }

    pub fn from_key(key: &str) -> Option<StereoMode> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }
}

/// The Dynamics Processor's knobs and curve
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicsProcessorSettings {
    /// 0 to 5
    pub input_gain: f32,
    /// 0 to 5
    pub output_gain: f32,
    /// ms, 1 to 500
    pub attack: f32,
    /// ms, 1 to 500
    pub release: f32,
    pub stereo_mode: StereoMode,
    /// The output level for each input level (0 to 1, in 200 steps)
    pub curve: [f32; CURVE_POINTS],
}

impl Default for DynamicsProcessorSettings {
    fn default() -> Self {
        Self {
            input_gain: 1.0,
            output_gain: 1.0,
            attack: 10.0,
            release: 100.0,
            stereo_mode: StereoMode::Maximum,
            curve: default_curve(),
        }
    }
}

/// The straight line: out as in
pub fn default_curve() -> [f32; CURVE_POINTS] {
    core::array::from_fn(|i| (i as f32 + 1.0) / CURVE_POINTS as f32)
}

/// A running RMS over a window
#[derive(Clone, Debug)]
pub struct Rms {
    buffer: Vec<f32>,
    sum: f32,
    pos: usize,
    size_f: f32,
}

impl Rms {
    pub fn new(size: usize) -> Self {
        let size = size.max(1);
        Self { buffer: vec![0.0; size], sum: 0.0, pos: 0, size_f: 1.0 / size as f32 }
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.sum = 0.0;
        self.pos = 0;
    }

    #[inline]
    pub fn update(&mut self, input: f32) -> f32 {
        self.sum -= self.buffer[self.pos];
        self.buffer[self.pos] = input * input;
        self.sum += self.buffer[self.pos];
        self.pos = (self.pos + 1) % self.buffer.len();
        (self.sum * self.size_f).sqrt()
    }
}

/// The Dynamics Processor (Vesa Kivimäki's)
#[derive(Clone, Debug)]
pub struct DynamicsProcessor {
    sample_rate: f32,
    rms: [Rms; 2],
    peak: [f32; 2],
    /// The attack and release the coefficients are for
    times: Option<(f32, f32)>,
    attack: f64,
    release: f64,
}

impl DynamicsProcessor {
    pub fn new(sample_rate: f32) -> Self {
        let window = 64 * sample_rate as i32 / 44100;
        let rms = Rms::new(window.max(1) as usize);
        Self { sample_rate, rms: [rms.clone(), rms], peak: [NOISE_FLOOR; 2], times: None, attack: 0.0, release: 0.0 }
    }

    pub fn reset(&mut self) {
        for rms in &mut self.rms {
            rms.reset();
        }
        self.peak = [NOISE_FLOOR; 2];
    }

    /// While bypassed the levels are let go (they would not decay without being run)
    pub fn bypassed(&mut self) {
        self.peak = [NOISE_FLOOR; 2];
    }

    fn coefficient(&self, ms: f32) -> f64 {
        exp((-1.0 / (f64::from(ms) * 0.001)) / f64::from(self.sample_rate as i32))
    }

    /// The level each side's gain was last read at (the followed RMS), for the window's point on the curve
    pub fn levels(&self) -> [f32; 2] {
        self.peak
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &DynamicsProcessorSettings) {
        if self.times != Some((s.attack, s.release)) {
            self.attack = self.coefficient(s.attack);
            self.release = self.coefficient(s.release);
            self.times = Some((s.attack, s.release));
        }
        for frame in frames.iter_mut() {
            let mut x = [frame[0] * s.input_gain, frame[1] * s.input_gain];
            for i in 0..2 {
                let t = f64::from(self.rms[i].update(x[i]));
                let peak = f64::from(self.peak[i]);
                if t > peak {
                    self.peak[i] = (peak * self.attack + (1.0 - self.attack) * t) as f32;
                } else if t < peak {
                    self.peak[i] = (peak * self.release + (1.0 - self.release) * t) as f32;
                }
                self.peak[i] = self.peak[i].max(NOISE_FLOOR);
            }
            let sm_peak = match s.stereo_mode {
                StereoMode::Maximum => [self.peak[0].max(self.peak[1]); 2],
                StereoMode::Average => [((f64::from(self.peak[0] + self.peak[1])) * 0.5) as f32; 2],
                StereoMode::Unlinked => self.peak,
            };
            for i in 0..2 {
                let at = sm_peak[i] * 200.0;
                let lookup = at as i32;
                let frac = at - at.trunc();
                if sm_peak[i] > NOISE_FLOOR {
                    let gain = if lookup < 1 {
                        frac * s.curve[0]
                    } else if lookup < 200 {
                        lerp(s.curve[lookup as usize - 1], s.curve[lookup as usize], frac)
                    } else {
                        s.curve[199]
                    };
                    x[i] *= gain;
                    x[i] /= sm_peak[i];
                }
            }
            *frame = [x[0] * s.output_gain, x[1] * s.output_gain];
        }
    }
}
