//! The legacy program's Frequency Shifter (Lost Robot's): a frequency shifter and barber-pole phaser, with a delay
//! in its feedback loop. Unlike the other effects it mixes its own dry signal (its Mix knob, times the host's wet).

use crate::math::{cosf, expf, f32c::PI, hermite, sinf};
use crate::oscillator::lerp;

/// The Hilbert transform's order
const ORDER: usize = 12;

/// A Hilbert transform by IIR (Geraint Luff's, Signalsmith Audio, 0BSD): the analytic signal of a real or complex
/// input, on two channels
#[derive(Clone, Debug)]
pub struct HilbertIir {
    coeffs_r: [f32; ORDER],
    coeffs_i: [f32; ORDER],
    poles_r: [f32; ORDER],
    poles_i: [f32; ORDER],
    state_r: [[f32; ORDER]; 2],
    state_i: [[f32; ORDER]; 2],
    direct: f32,
}

impl HilbertIir {
    const BASE_COEFFS_R: [f32; ORDER] = [
        -0.000224352093802,
        0.0107500557815,
        -0.0456795873917,
        0.11282500582,
        -0.208067578452,
        0.28717837501,
        -0.254675294431,
        0.0481081835026,
        0.227861357867,
        -0.365411839137,
        0.280729061131,
        -0.0935061787728,
    ];
    const BASE_COEFFS_I: [f32; ORDER] = [
        0.00543499018201,
        -0.0173890685681,
        0.0229166931429,
        0.00278413661237,
        -0.104628958675,
        0.33619239719,
        -0.683033899655,
        0.954061589374,
        -0.891273574569,
        0.525088317271,
        -0.155131206606,
        0.00512245855404,
    ];
    const BASE_POLES_R: [f32; ORDER] = [
        -0.00495335976478,
        -0.017859491302,
        -0.0413714373155,
        -0.0882148408885,
        -0.17922965812,
        -0.338261800753,
        -0.557688699732,
        -0.735157736148,
        -0.719057381172,
        -0.517871025209,
        -0.280197469471,
        -0.0852751354531,
    ];
    const BASE_POLES_I: [f32; ORDER] = [
        0.0092579876872,
        0.0273493725543,
        0.0744756910287,
        0.178349677457,
        0.39601340223,
        0.829229533354,
        1.61298538328,
        2.79987398682,
        4.16396166128,
        5.29724826804,
        5.99598602388,
        6.3048492377,
    ];
    const BASE_DIRECT: f32 = 0.000262057212648;

    pub fn new(sample_rate: f32, passband_gain: f32) -> Self {
        let freq_factor = 0.46f32.min(20000.0 / sample_rate);
        let coeff_scale = freq_factor * passband_gain;
        let mut h = Self {
            coeffs_r: [0.0; ORDER],
            coeffs_i: [0.0; ORDER],
            poles_r: [0.0; ORDER],
            poles_i: [0.0; ORDER],
            state_r: [[0.0; ORDER]; 2],
            state_i: [[0.0; ORDER]; 2],
            direct: Self::BASE_DIRECT * 2.0 * passband_gain * freq_factor,
        };
        for i in 0..ORDER {
            h.coeffs_r[i] = Self::BASE_COEFFS_R[i] * coeff_scale;
            h.coeffs_i[i] = Self::BASE_COEFFS_I[i] * coeff_scale;
            let a = Self::BASE_POLES_R[i] * freq_factor;
            let b = Self::BASE_POLES_I[i] * freq_factor;
            let ea = expf(a);
            h.poles_r[i] = ea * cosf(b);
            h.poles_i[i] = ea * sinf(b);
        }
        h
    }

    pub fn reset(&mut self) {
        self.state_r = [[0.0; ORDER]; 2];
        self.state_i = [[0.0; ORDER]; 2];
    }

    /// The analytic signal (real, imaginary) of a real input
    #[inline]
    pub fn process_real(&mut self, x: f32, ch: usize) -> [f32; 2] {
        let (mut sum_r, mut sum_i) = (0.0f32, 0.0f32);
        for i in 0..ORDER {
            let (r, im) = (self.state_r[ch][i], self.state_i[ch][i]);
            let (pr, pi) = (self.poles_r[i], self.poles_i[i]);
            let nr = r * pr - im * pi + x * self.coeffs_r[i];
            let ni = r * pi + im * pr + x * self.coeffs_i[i];
            self.state_r[ch][i] = nr;
            self.state_i[ch][i] = ni;
            sum_r += nr;
            sum_i += ni;
        }
        [x * self.direct + sum_r, sum_i]
    }

    /// The positive frequencies of a complex input
    #[inline]
    pub fn process_complex(&mut self, x: [f32; 2], ch: usize) -> [f32; 2] {
        let [xr, xi] = x;
        let (mut sum_r, mut sum_i) = (0.0f32, 0.0f32);
        for i in 0..ORDER {
            let (r, im) = (self.state_r[ch][i], self.state_i[ch][i]);
            let (pr, pi, cr, ci) = (self.poles_r[i], self.poles_i[i], self.coeffs_r[i], self.coeffs_i[i]);
            let (xrcr, xici, xrci, xicr) = (xr * cr, xi * ci, xr * ci, xi * cr);
            let nr = r * pr - im * pi + (xrcr - xici);
            let ni = r * pi + im * pr + (xrci + xicr);
            self.state_r[ch][i] = nr;
            self.state_i[ch][i] = ni;
            sum_r += nr;
            sum_i += ni;
        }
        [xr * self.direct + sum_r, xi * self.direct + sum_i]
    }
}

/// Where the shifter's input comes from and where its output goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteMode {
    /// The input and the delay's feedback through the shifter
    Series,
    /// The delay in parallel, its feedback alone shifted, the output added to the input
    ParallelAdd,
    /// The delay in parallel, the output mixed with the input
    ParallelMix,
}

impl RouteMode {
    pub const ALL: [RouteMode; 3] = [RouteMode::Series, RouteMode::ParallelAdd, RouteMode::ParallelMix];

    pub fn key(self) -> &'static str {
        match self {
            RouteMode::Series => "series",
            RouteMode::ParallelAdd => "parallel-add",
            RouteMode::ParallelMix => "parallel-mix",
        }
    }

    pub fn from_key(key: &str) -> Option<RouteMode> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }
}

/// The Frequency Shifter's knobs
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrequencyShifterSettings {
    /// 0 to 1, times the host's wet level
    pub mix: f32,
    /// Hz, -20000 to 20000
    pub shift: f32,
    /// Hz, -50 to 50: the left shifted by that less, the right by that more
    pub spread: f32,
    /// 0 (single sideband) to 1 (ring modulation)
    pub ring: f32,
    /// 0 to 1
    pub feedback: f32,
    /// ms, 0 to 2000
    pub delay: f32,
    /// ms, 0 to 20
    pub fine_delay: f32,
    /// Hz, 100 to 22000: the feedback's low-pass
    pub delay_damp: f32,
    /// s, 0 to 1
    pub delay_glide: f32,
    /// Hz, 0 to 2000
    pub lfo_amount: f32,
    /// Hz, 0 to 200
    pub lfo_rate: f32,
    /// Turns, 0 to 1
    pub lfo_stereo_phase: f32,
    /// s, 0 to 1
    pub glide: f32,
    /// Hz, 100 to 22000: the output's low-pass
    pub tone: f32,
    /// Turns, -2 to 2
    pub phase: f32,
    pub antireflect: bool,
    pub route: RouteMode,
    /// 0 to 1
    pub harmonics: f32,
}

impl Default for FrequencyShifterSettings {
    fn default() -> Self {
        Self {
            mix: 1.0,
            shift: 0.0,
            spread: 0.0,
            ring: 0.0,
            feedback: 0.0,
            delay: 0.0,
            fine_delay: 0.0,
            delay_damp: 22000.0,
            delay_glide: 0.05,
            lfo_amount: 0.0,
            lfo_rate: 0.2,
            lfo_stereo_phase: 0.0,
            glide: 0.05,
            tone: 22000.0,
            phase: 0.0,
            antireflect: false,
            route: RouteMode::Series,
            harmonics: 0.0,
        }
    }
}

/// The longest delay, ms (the coarse and the fine knobs' tops)
const MAX_DELAY_MS: f32 = 2000.0 + 20.0;

/// The Frequency Shifter
#[derive(Clone, Debug)]
pub struct FrequencyShifter {
    sample_rate: f32,
    two_pi_over_sr: f32,
    hilbert1: HilbertIir,
    hilbert2: HilbertIir,
    ring: Vec<[f32; 2]>,
    write_index: usize,
    phase: [f32; 2],
    true_shift: [f32; 2],
    damp_state: [f32; 2],
    tone_state: [f32; 2],
    lfo_phase: f32,
    true_phase: f32,
    true_delay: f32,
}

impl FrequencyShifter {
    pub fn new(sample_rate: f32) -> Self {
        let size = (MAX_DELAY_MS * 0.001 * sample_rate + 6.0) as usize;
        Self {
            sample_rate,
            two_pi_over_sr: PI * 2.0 / sample_rate,
            hilbert1: HilbertIir::new(sample_rate, 2.0),
            hilbert2: HilbertIir::new(sample_rate, 2.0),
            ring: vec![[0.0; 2]; size],
            write_index: 0,
            phase: [0.0; 2],
            true_shift: [0.0; 2],
            damp_state: [0.0; 2],
            tone_state: [0.0; 2],
            lfo_phase: 0.0,
            true_phase: 0.0,
            true_delay: 1.0,
        }
    }

    pub fn reset(&mut self) {
        self.ring.fill([0.0; 2]);
        self.write_index = 0;
        self.hilbert1.reset();
        self.hilbert2.reset();
        self.phase = [0.0; 2];
        self.lfo_phase = 0.0;
        self.damp_state = [0.0; 2];
        self.tone_state = [0.0; 2];
    }

    /// The shifter's oscillators from their start (the interface's button)
    pub fn reset_shifter(&mut self) {
        self.phase = [0.0; 2];
    }

    /// The LFO from its start (the interface's button)
    pub fn reset_lfo(&mut self) {
        self.lfo_phase = 0.0;
    }

    fn hermite_sample(&self, floor: usize, frac: f32) -> [f32; 2] {
        let size = self.ring.len();
        let i0 = if floor == 0 { size - 1 } else { floor - 1 };
        let i1 = floor;
        let i2 = if floor + 1 >= size { floor + 1 - size } else { floor + 1 };
        let i3 = if floor + 2 >= size { floor + 2 - size } else { floor + 2 };
        core::array::from_fn(|ch| {
            hermite(self.ring[i0][ch], self.ring[i1][ch], self.ring[i2][ch], self.ring[i3][ch], frac)
        })
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &FrequencyShifterSettings) {
        let two_pi = PI * 2.0;
        let sr = self.sample_rate;
        let delay_len = (s.delay + s.fine_delay) * 0.001 * sr;
        let lfo_rate = (s.lfo_rate / sr) * two_pi;
        let lfo_st = s.lfo_stereo_phase * two_pi;
        let phase = s.phase * two_pi;
        let inv_ring = 1.0 - s.ring;
        let parallel = s.route != RouteMode::Series;
        let add = s.route == RouteMode::ParallelAdd;
        let harmonics = s.harmonics;
        let do_harm = harmonics > 0.0;
        let harm_factor = harmonics * 20.0 + 1.0;
        let harm_div = 1.0 / (harmonics * 0.3 + 1.0);
        let damp = expf(-self.two_pi_over_sr * s.delay_damp);
        let tone = expf(-self.two_pi_over_sr * s.tone);
        let delay_glide = if s.delay_glide != 0.0 { expf(-1.0 / (s.delay_glide * sr)) } else { 0.0 };
        let glide = if s.glide != 0.0 { expf(-1.0 / (s.glide * sr)) } else { 0.0 };
        let max_delay = (self.ring.len() - 4) as f32;
        let size = self.ring.len();

        // (the phases wrapped once a call)
        self.lfo_phase %= two_pi;
        for p in &mut self.phase {
            *p %= two_pi;
        }
        for frame in frames.iter_mut() {
            let (lfo0, lfo1) = if s.lfo_amount > 0.0 {
                (sinf(self.lfo_phase) * s.lfo_amount, sinf(self.lfo_phase + lfo_st) * s.lfo_amount)
            } else {
                (0.0, 0.0)
            };
            self.lfo_phase += lfo_rate;

            // The knobs glide
            let base = [s.shift - s.spread, s.shift + s.spread];
            for ch in 0..2 {
                self.true_shift[ch] = (1.0 - glide) * base[ch] + glide * self.true_shift[ch];
            }
            self.true_delay = ((1.0 - delay_glide) * delay_len + delay_glide * self.true_delay).clamp(1.0, max_delay);
            self.true_phase = (1.0 - glide) * phase + glide * self.true_phase;

            // The delay, read with 4-point Hermite interpolation
            let mut read = self.write_index as f32 - self.true_delay;
            if read < 0.0 {
                read += size as f32;
            }
            let mut floor = read as usize;
            let frac = read - floor as f32;
            // A read a hair below 0 (a delay just above a whole number of frames) wraps to exactly the ring's length
            // in single precision: that is the position 0 of the ring (frac 0), not one past its end. The Hermite
            // read there is the same point either way (the curve passes through its samples)
            if floor >= size {
                floor -= size;
            }
            let dly = self.hermite_sample(floor, frac);
            self.write_index += 1;
            if self.write_index == size {
                self.write_index = 0;
            }

            let input = *frame;
            let fx_in: [f32; 2] =
                core::array::from_fn(
                    |ch| {
                        if parallel { dly[ch] * s.feedback } else { input[ch] + dly[ch] * s.feedback }
                    },
                );
            let d_ph =
                [(self.true_shift[0] + lfo0) * self.two_pi_over_sr, (self.true_shift[1] + lfo1) * self.two_pi_over_sr];
            let mut out = [0.0f32; 2];
            for ch in 0..2 {
                let phase_value = self.phase[ch] + self.true_phase;
                let mut sin_p = sinf(phase_value);
                let mut cos_p = cosf(phase_value);
                if do_harm {
                    // A distortion of the oscillator, crossfaded with it
                    let xc = (harm_factor * cos_p).clamp(-3.0, 3.0);
                    let xs = (harm_factor * sin_p).clamp(-3.0, 3.0);
                    let (xc2, xs2) = (xc * xc, xs * xs);
                    let tc = xc * (27.0 + xc2) / (27.0 + 9.0 * xc2);
                    let ts = xs * (27.0 + xs2) / (27.0 + 9.0 * xs2);
                    cos_p = lerp(cos_p, tc * harm_div, harmonics);
                    sin_p = lerp(sin_p, ts * harm_div, harmonics);
                }
                let analytic = self.hilbert1.process_real(fx_in[ch], ch);
                // A ring modulator shifts down and up at once: the imaginary part cancels one side
                let osc_r = cos_p;
                let osc_i = sin_p * inv_ring;
                let mod_r = analytic[0] * osc_r - analytic[1] * osc_i;
                let mod_i = analytic[0] * osc_i + analytic[1] * osc_r;
                out[ch] = if s.antireflect {
                    // The negative frequencies taken out, so nothing folds through 0 Hz or Nyquist
                    self.hilbert2.process_complex([mod_r, mod_i], ch)[0] * 0.5
                } else {
                    mod_r
                };
                self.phase[ch] += d_ph[ch];
            }

            const FB_SATURATION: f32 = 16.0;
            for ch in 0..2 {
                let mut delay_in = out[ch] + if parallel { input[ch] } else { 0.0 };
                delay_in = (FB_SATURATION * delay_in) / (FB_SATURATION + delay_in.abs());
                self.damp_state[ch] = (1.0 - damp) * delay_in + damp * self.damp_state[ch];
                self.ring[self.write_index][ch] = self.damp_state[ch];
            }
            for ch in 0..2 {
                self.tone_state[ch] = (1.0 - tone) * out[ch] + tone * self.tone_state[ch];
                let wet = self.tone_state[ch];
                frame[ch] = if add { input[ch] + s.mix * wet } else { (1.0 - s.mix) * input[ch] + s.mix * wet };
            }
        }
    }
}
