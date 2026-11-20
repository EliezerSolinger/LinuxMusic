//! The legacy program's ReverbSC: Csound's `reverbsc` (Sean Costello and Istvan Varga, 1999 and 2005), as Soundpipe
//! made it a module (`sp_revsc`), with Soundpipe's DC blocker (`sp_dcblock`, after Perry R. Cook) on each output.
//! The C mixes single and double precision; the port keeps where it is which, so that it nulls against it.

use crate::math::{cos, expf, f32c};

const DEFAULT_SRATE: f32 = 44100.0;
const DELAYPOS_SHIFT: i32 = 28;
const DELAYPOS_SCALE: i32 = 0x1000_0000;
const DELAYPOS_MASK: i32 = 0x0FFF_FFFF;
const OUTPUT_GAIN: f32 = 0.35;
const JP_SCALE: f32 = 0.25;
const PITCH_MOD: f32 = 1.0;

/// For each delay line: its time (s), the random variation of its time (s), the variation's frequency (1/s) and
/// its random seed (0 - 32767)
const REVERB_PARAMS: [[f32; 4]; 8] = [
    [2473.0 / DEFAULT_SRATE, 0.0010, 3.100, 1966.0],
    [2767.0 / DEFAULT_SRATE, 0.0011, 3.500, 29491.0],
    [3217.0 / DEFAULT_SRATE, 0.0017, 1.110, 22937.0],
    [3557.0 / DEFAULT_SRATE, 0.0006, 3.973, 9830.0],
    [3907.0 / DEFAULT_SRATE, 0.0010, 2.341, 20643.0],
    [4127.0 / DEFAULT_SRATE, 0.0011, 1.897, 22937.0],
    [2143.0 / DEFAULT_SRATE, 0.0017, 0.891, 29491.0],
    [1933.0 / DEFAULT_SRATE, 0.0006, 3.221, 14417.0],
];

fn delay_line_max_samples(sample_rate: f32, n: usize) -> usize {
    // maxDel += params[1] * pitchMod * 1.125, the product with the double in double precision
    let max_delay = (f64::from(REVERB_PARAMS[n][0]) + f64::from(REVERB_PARAMS[n][1] * PITCH_MOD) * 1.125) as f32;
    (f64::from(max_delay * sample_rate) + 16.5) as usize
}

#[derive(Clone, Debug)]
struct DelayLine {
    write_pos: i32,
    buffer_size: i32,
    read_pos: i32,
    read_pos_frac: i32,
    read_pos_frac_inc: i32,
    seed: i32,
    rand_line_count: i32,
    filter_state: f32,
    buf: Vec<f32>,
}

/// The reverb: eight delay lines whose lengths wander at random, in a feedback network with a low-pass in each
#[derive(Clone, Debug)]
pub struct RevSc {
    sample_rate: f32,
    /// The "size": the feedback, 0 to 1
    pub feedback: f32,
    /// The "color": the low-pass in the feedback, Hz
    pub lp_freq: f32,
    damp_fact: f32,
    prv_lp_freq: f32,
    lines: [DelayLine; 8],
}

impl RevSc {
    /// Prepared for @p sample_rate: allocates the delay lines
    pub fn new(sample_rate: f32) -> Self {
        let lines = core::array::from_fn(|n| {
            let size = delay_line_max_samples(sample_rate, n);
            DelayLine {
                write_pos: 0,
                buffer_size: size as i32,
                read_pos: 0,
                read_pos_frac: 0,
                read_pos_frac_inc: 0,
                seed: 0,
                rand_line_count: 0,
                filter_state: 0.0,
                buf: vec![0.0; size],
            }
        });
        let mut reverb =
            Self { sample_rate, feedback: 0.97, lp_freq: 10000.0, damp_fact: 1.0, prv_lp_freq: 0.0, lines };
        reverb.reset();
        reverb
    }

    /// The delay lines emptied and set as when made (no allocation)
    pub fn reset(&mut self) {
        self.damp_fact = 1.0;
        self.prv_lp_freq = 0.0;
        for n in 0..8 {
            self.init_delay_line(n);
        }
    }

    fn init_delay_line(&mut self, n: usize) {
        let sample_rate = self.sample_rate;
        let lp = &mut self.lines[n];
        lp.buffer_size = delay_line_max_samples(sample_rate, n) as i32;
        lp.write_pos = 0;
        // The random seed
        lp.seed = (f64::from(REVERB_PARAMS[n][3]) + 0.5) as i32;
        // The first delay time
        let mut read_pos = lp.seed as f32 * REVERB_PARAMS[n][1] / 32768.0;
        read_pos = REVERB_PARAMS[n][0] + read_pos * PITCH_MOD;
        read_pos = lp.buffer_size as f32 - read_pos * sample_rate;
        lp.read_pos = read_pos as i32;
        read_pos = (read_pos - lp.read_pos as f32) * DELAYPOS_SCALE as f32;
        lp.read_pos_frac = (f64::from(read_pos) + 0.5) as i32;
        self.next_random_lineseg(n);
        let lp = &mut self.lines[n];
        lp.filter_state = 0.0;
        lp.buf.fill(0.0);
    }

    fn next_random_lineseg(&mut self, n: usize) {
        let sample_rate = self.sample_rate;
        let lp = &mut self.lines[n];
        // The random seed
        if lp.seed < 0 {
            lp.seed += 0x10000;
        }
        lp.seed = (lp.seed.wrapping_mul(15625) + 1) & 0xFFFF;
        if lp.seed >= 0x8000 {
            lp.seed -= 0x10000;
        }
        // The next segment's length, in samples
        lp.rand_line_count = (f64::from(sample_rate / REVERB_PARAMS[n][2]) + 0.5) as i32;
        let mut previous = lp.write_pos as f32;
        previous -= lp.read_pos as f32 + lp.read_pos_frac as f32 / DELAYPOS_SCALE as f32;
        while previous < 0.0 {
            previous += lp.buffer_size as f32;
        }
        previous /= sample_rate; // the previous delay time, s
        let mut next = (f64::from(lp.seed as f32 * REVERB_PARAMS[n][1]) / 32768.0) as f32;
        next = REVERB_PARAMS[n][0] + next * PITCH_MOD; // the next delay time, s
        // The phase increment a sample
        let mut increment = (previous - next) / lp.rand_line_count as f32;
        increment = (f64::from(increment * sample_rate) + 1.0) as f32;
        lp.read_pos_frac_inc = (f64::from(increment * DELAYPOS_SCALE as f32) + 0.5) as i32;
    }

    /// One frame
    #[inline]
    pub fn compute(&mut self, in1: f32, in2: f32) -> (f32, f32) {
        // The tone filter's coefficient, when its frequency changed (in double, as the C)
        if self.lp_freq != self.prv_lp_freq {
            self.prv_lp_freq = self.lp_freq;
            let damp = (2.0
                - cos(f64::from(self.prv_lp_freq) * (2.0 * core::f64::consts::PI) / f64::from(self.sample_rate)))
                as f32;
            self.damp_fact = (f64::from(damp) - (f64::from(damp * damp) - 1.0).sqrt()) as f32;
        }
        let damp_fact = self.damp_fact;
        let feedback = self.feedback;

        // The "resultant junction pressure", mixed to the inputs
        let mut ain_l = 0.0f32;
        for line in &self.lines {
            ain_l += line.filter_state;
        }
        ain_l *= JP_SCALE;
        let ain_r = ain_l + in2;
        ain_l += in1;
        let (mut aout_l, mut aout_r) = (0.0f32, 0.0f32);

        for n in 0..8 {
            let lp = &mut self.lines[n];
            let buffer_size = lp.buffer_size;
            // The input and the feedback into the line
            lp.buf[lp.write_pos as usize] = (if n & 1 != 0 { ain_r } else { ain_l }) - lp.filter_state;
            lp.write_pos += 1;
            if lp.write_pos >= buffer_size {
                lp.write_pos -= buffer_size;
            }

            // Read with cubic interpolation
            if lp.read_pos_frac >= DELAYPOS_SCALE {
                lp.read_pos += lp.read_pos_frac >> DELAYPOS_SHIFT;
                lp.read_pos_frac &= DELAYPOS_MASK;
            }
            // (rem_euclid: back into the line for any advance; the advance is one frame or two, so this is the
            // legacy program's single subtraction whenever that one was enough)
            lp.read_pos = lp.read_pos.rem_euclid(buffer_size);
            let mut read_pos = lp.read_pos;
            let frac = (f64::from(lp.read_pos_frac as f32) * (1.0 / f64::from(DELAYPOS_SCALE as f32))) as f32;

            // The interpolation's coefficients
            let mut a2 = frac * frac;
            a2 -= 1.0;
            a2 *= 1.0 / 6.0;
            let mut a1 = frac;
            a1 += 1.0;
            a1 *= 0.5;
            let mut am1 = a1 - 1.0;
            let mut a0 = 3.0 * a2;
            a1 -= a0;
            am1 -= a2;
            a0 -= frac;

            // The four samples
            let buf = &lp.buf;
            let (vm1, mut v0, v1, v2);
            if read_pos > 0 && read_pos < buffer_size - 2 {
                let r = read_pos as usize;
                vm1 = buf[r - 1];
                v0 = buf[r];
                v1 = buf[r + 1];
                v2 = buf[r + 2];
            } else {
                // At the wrap-around, each index checked
                read_pos -= 1;
                if read_pos < 0 {
                    read_pos += buffer_size;
                }
                vm1 = buf[read_pos as usize];
                read_pos += 1;
                if read_pos >= buffer_size {
                    read_pos -= buffer_size;
                }
                v0 = buf[read_pos as usize];
                read_pos += 1;
                if read_pos >= buffer_size {
                    read_pos -= buffer_size;
                }
                v1 = buf[read_pos as usize];
                read_pos += 1;
                if read_pos >= buffer_size {
                    read_pos -= buffer_size;
                }
                v2 = buf[read_pos as usize];
            }
            v0 = (am1 * vm1 + a0 * v0 + a1 * v1 + a2 * v2) * frac + v0;

            lp.read_pos_frac = lp.read_pos_frac.wrapping_add(lp.read_pos_frac_inc);

            // The feedback gain and the low-pass
            v0 *= feedback;
            v0 = (lp.filter_state - v0) * damp_fact + v0;
            lp.filter_state = v0;

            if n & 1 != 0 {
                aout_r += v0;
            } else {
                aout_l += v0;
            }

            // The next random segment, when this one has reached its end
            lp.rand_line_count -= 1;
            if lp.rand_line_count <= 0 {
                self.next_random_lineseg(n);
            }
        }
        (aout_l * OUTPUT_GAIN, aout_r * OUTPUT_GAIN)
    }
}

/// Soundpipe's DC blocker: y = x - x1 + 0.99 y1
#[derive(Clone, Debug)]
pub struct DcBlock {
    gain: f32,
    inputs: f32,
    outputs: f32,
}

impl Default for DcBlock {
    fn default() -> Self {
        // pow(0.99, 1.0f / 1), kept in a float
        Self { gain: 0.99f64 as f32, inputs: 0.0, outputs: 0.0 }
    }
}

impl DcBlock {
    /// The blocker for @p sample_rate: the legacy pole (0.99) at 48 kHz, and the same cutoff (about 77 Hz) at any rate
    /// (U36; Soundpipe's fixed 0.99 put the cutoff at 153 Hz at 96 kHz, and the reverb lost 1.6 dB of its low end)
    pub fn new(sample_rate: f32) -> Self {
        let gain = crate::math::pow(0.99, 48_000.0 / f64::from(sample_rate)) as f32;
        Self { gain, inputs: 0.0, outputs: 0.0 }
    }

    /// Empties it, keeping its pole
    pub fn clear(&mut self) {
        self.inputs = 0.0;
        self.outputs = 0.0;
    }

    #[inline]
    pub fn compute(&mut self, sample: f32) -> f32 {
        self.outputs = sample - self.inputs + self.gain * self.outputs;
        self.inputs = sample;
        self.outputs
    }
}

/// The ReverbSC's controls
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbScSettings {
    /// dB, -60 to +15
    pub input_gain: f32,
    /// 0 to 1
    pub size: f32,
    /// Hz, 100 to 15000
    pub color: f32,
    /// dB, -60 to +15
    pub output_gain: f32,
}

impl Default for ReverbScSettings {
    fn default() -> Self {
        Self { input_gain: 0.0, size: 0.89, color: 10000.0, output_gain: 0.0 }
    }
}

/// The ReverbSC effect: the input gain, the reverb, a DC blocker on each side, the output gain. The wet signal only:
/// dry and wet are mixed by the host, as for every effect
#[derive(Clone, Debug)]
pub struct ReverbSc {
    reverb: RevSc,
    dc: [DcBlock; 2],
}

impl ReverbSc {
    pub fn new(sample_rate: f32) -> Self {
        Self { reverb: RevSc::new(sample_rate), dc: [DcBlock::new(sample_rate), DcBlock::new(sample_rate)] }
    }

    pub fn reset(&mut self) {
        self.reverb.reset();
        for dc in &mut self.dc {
            dc.clear();
        }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], settings: &ReverbScSettings) {
        // (The legacy program's fastPow10f in single precision)
        let in_gain = expf(f32c::LN_10 * (settings.input_gain / 20.0));
        let out_gain = expf(f32c::LN_10 * (settings.output_gain / 20.0));
        self.reverb.feedback = settings.size;
        self.reverb.lp_freq = settings.color;
        for frame in frames.iter_mut() {
            let (l, r) = self.reverb.compute(frame[0] * in_gain, frame[1] * in_gain);
            frame[0] = self.dc[0].compute(l) * out_gain;
            frame[1] = self.dc[1].compute(r) * out_gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impulse_rings_and_decays() {
        let mut reverb = ReverbSc::new(48000.0);
        let mut frames = vec![[0.0f32; 2]; 48000 * 2];
        frames[0] = [1.0, 1.0];
        reverb.process(&mut frames, &ReverbScSettings { size: 0.7, ..ReverbScSettings::default() });
        let energy = |range: core::ops::Range<usize>| frames[range].iter().map(|f| f[0] * f[0]).sum::<f32>();
        let early = energy(2000..12000);
        let late = energy(80000..90000);
        assert!(early > 1e-4, "{early}");
        assert!(late < early * 1e-3, "{late} {early}");
    }
}
