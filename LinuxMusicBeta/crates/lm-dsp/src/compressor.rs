//! The legacy program's Compressor plugin (legacy `plugins/Compressor`, Lost Robot's): RMS or peak detection with
//! automatic attack and release from the crest factor, hold, lookahead, a tilt filter on the sidechain, mid/side,
//! five ways of linking the sides, range, and automatic make-up.

use crate::math::{amp_to_dbfs, dbfs_to_amp, expf, f32c};
use crate::oscillator::lerp;

const COMP_LOG: f32 = -2.2;
/// -120 dBFS
const NOISE_FLOOR: f32 = 0.000_001;

/// How the two sides' gains are linked
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StereoLink {
    Unlinked,
    Maximum,
    Average,
    Minimum,
    Blend,
}

impl StereoLink {
    pub fn key(self) -> &'static str {
        match self {
            StereoLink::Unlinked => "unlinked",
            StereoLink::Maximum => "maximum",
            StereoLink::Average => "average",
            StereoLink::Minimum => "minimum",
            StereoLink::Blend => "blend",
        }
    }
}

/// The Compressor's controls (the plugin's knobs, in their units)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressorPluginSettings {
    /// dBFS, -60 to 0
    pub threshold: f32,
    /// 1 to 20
    pub ratio: f32,
    /// ms, 0.005 to 250
    pub attack: f32,
    /// ms, 1 to 2500
    pub release: f32,
    /// dB, 0 to 96
    pub knee: f32,
    /// ms, 0 to 500
    pub hold: f32,
    /// dB, -240 (off) to 0
    pub range: f32,
    /// ms, 0 to 250
    pub rms: f32,
    pub mid_side: bool,
    pub peak_mode: bool,
    /// ms, 0 to 20
    pub lookahead_length: f32,
    /// -1 to 1
    pub in_balance: f32,
    pub out_balance: f32,
    pub limiter: bool,
    /// dB, -60 to 30
    pub out_gain: f32,
    pub in_gain: f32,
    /// 0 to 3
    pub blend: f32,
    /// -1 to 1
    pub stereo_balance: f32,
    pub auto_makeup: bool,
    pub audition: bool,
    pub feedback: bool,
    /// %, 0 to 100
    pub auto_attack: f32,
    pub auto_release: f32,
    pub lookahead: bool,
    /// dB, -6 to 6
    pub tilt: f32,
    /// Hz
    pub tilt_freq: f32,
    pub stereo_link: StereoLink,
    /// %, 0 to 100
    pub mix: f32,
}

impl Default for CompressorPluginSettings {
    fn default() -> Self {
        Self {
            threshold: -8.0,
            ratio: 1.8,
            attack: 10.0,
            release: 100.0,
            knee: 12.0,
            hold: 0.0,
            range: -240.0,
            rms: 1.0,
            mid_side: false,
            peak_mode: false,
            lookahead_length: 0.0,
            in_balance: 0.0,
            out_balance: 0.0,
            limiter: false,
            out_gain: 0.0,
            in_gain: 0.0,
            blend: 1.0,
            stereo_balance: 0.0,
            auto_makeup: false,
            audition: false,
            feedback: false,
            auto_attack: 0.0,
            auto_release: 0.0,
            lookahead: false,
            tilt: 0.0,
            tilt_freq: 150.0,
            stereo_link: StereoLink::Maximum,
            mix: 100.0,
        }
    }
}

/// The range's knob turned all the way down: no range
const RANGE_OFF: f32 = -240.0;

/// The Compressor. The wet signal: dry and wet are mixed by the host, as for every effect (its own MIX knob is its
/// own)
#[derive(Clone, Debug)]
pub struct CompressorPlugin {
    sample_rate: f32,
    settings: Option<CompressorPluginSettings>,
    coeff_precalc: f32,
    crest_time_const: f32,
    in_look: [Vec<f32>; 2],
    sc_look: [Vec<f32>; 2],
    look_write: usize,
    look_buf_length: usize,

    att_coeff: f32,
    rel_coeff: f32,
    auto_att: f32,
    auto_rel: f32,
    hold_length: i32,
    hold_timer: [i32; 2],
    lookahead_length: usize,
    threshold: f32,
    knee: f32,
    ratio: f32,
    auto_makeup: f32,
    out_gain: f32,
    in_gain: f32,
    range: f32,
    tilt: f32,
    mix: f32,
    rms_time_const: f32,
    rms: [f32; 2],
    crest_peak: [f32; 2],
    crest_rms: [f32; 2],
    crest_factor: [f32; 2],
    tilt_out: [f32; 2],
    lgain: f32,
    hgain: f32,
    a0: f32,
    b1: f32,
    prev_out: [f32; 2],
    y_l: [f32; 2],
    gain_result: [f32; 2],
}

impl CompressorPlugin {
    /// Prepared for @p sample_rate: allocates the lookahead's buffers
    pub fn new(sample_rate: f32) -> Self {
        let look_buf_length = ((20.0f32 / 1000.0) * sample_rate).ceil() as usize + 2;
        Self {
            sample_rate,
            settings: None,
            coeff_precalc: COMP_LOG / (sample_rate * 0.001),
            crest_time_const: expf(-1.0 / (0.2 * sample_rate)),
            in_look: [vec![0.0; look_buf_length], vec![0.0; look_buf_length]],
            sc_look: [vec![NOISE_FLOOR; look_buf_length], vec![NOISE_FLOOR; look_buf_length]],
            look_write: 0,
            look_buf_length,
            att_coeff: 0.0,
            rel_coeff: 0.0,
            auto_att: 0.0,
            auto_rel: 0.0,
            hold_length: 0,
            hold_timer: [0; 2],
            lookahead_length: 0,
            threshold: 0.0,
            knee: 0.0,
            ratio: 1.0,
            auto_makeup: 1.0,
            out_gain: 1.0,
            in_gain: 1.0,
            range: 0.0,
            tilt: 0.0,
            mix: 1.0,
            rms_time_const: 0.0,
            rms: [0.0; 2],
            crest_peak: [0.0; 2],
            crest_rms: [0.0; 2],
            crest_factor: [0.0; 2],
            tilt_out: [0.0; 2],
            lgain: 0.0,
            hgain: 0.0,
            a0: 0.0,
            b1: 0.0,
            prev_out: [0.0; 2],
            y_l: [NOISE_FLOOR; 2],
            gain_result: [1.0; 2],
        }
    }

    fn ms_to_coeff(&self, ms: f32) -> f32 {
        // A time in ms to a low-pass coefficient
        expf(self.coeff_precalc / ms)
    }

    /// What the knobs mean, worked out again for those that changed
    fn apply(&mut self, s: &CompressorPluginSettings) {
        let old = self.settings;
        let changed = |f: fn(&CompressorPluginSettings) -> f32| old.is_none_or(|o| f(&o) != f(s));
        let flag = |f: fn(&CompressorPluginSettings) -> bool| old.is_none_or(|o| f(&o) != f(s));
        let sr = self.sample_rate;
        if changed(|s| s.attack) {
            self.att_coeff = self.ms_to_coeff(s.attack);
        }
        if changed(|s| s.release) {
            self.rel_coeff = self.ms_to_coeff(s.release);
        }
        if changed(|s| s.hold) {
            self.hold_length = (s.hold * 0.001 * sr) as i32;
            self.hold_timer = [0; 2];
        }
        let threshold = changed(|s| s.threshold);
        let ratio = changed(|s| s.ratio);
        let knee = changed(|s| s.knee);
        let limiter = flag(|s| s.limiter);
        if threshold {
            self.threshold = s.threshold;
        }
        if ratio {
            self.ratio = 1.0 / s.ratio;
        }
        if knee {
            self.knee = s.knee * 0.5;
        }
        if changed(|s| s.range) {
            // Range is inactive when turned all the way down
            self.range = if s.range > RANGE_OFF { dbfs_to_amp(s.range) } else { 0.0 };
        }
        if changed(|s| s.rms) {
            self.rms_time_const = if s.rms > 0.0 { expf(-1.0 / (s.rms * 0.001 * sr)) } else { 0.0 };
        }
        if changed(|s| s.lookahead_length) {
            // (the knob's top, 20 ms, fits the buffer made for it; a longer one is held to the buffer's, the most it
            // can delay)
            self.lookahead_length =
                (((s.lookahead_length / 1000.0) * sr).ceil() as usize).min(self.look_buf_length - 1);
        }
        if changed(|s| s.out_gain) {
            // 0.999 keeps the values from crossing the threshold all the time (limiters most), in every mode
            self.out_gain = (f64::from(dbfs_to_amp(s.out_gain)) * 0.999) as f32;
        }
        if changed(|s| s.in_gain) {
            self.in_gain = dbfs_to_amp(s.in_gain);
        }
        if changed(|s| s.tilt) || changed(|s| s.tilt_freq) {
            self.tilt = s.tilt;
            let amp = 6.0 / core::f32::consts::LN_2;
            let gfactor = 5.0;
            let g1 = if self.tilt > 0.0 { -gfactor * self.tilt } else { -self.tilt };
            let g2 = if self.tilt > 0.0 { self.tilt } else { gfactor * self.tilt };
            self.lgain = expf(g1 / amp) - 1.0;
            self.hgain = expf(g2 / amp) - 1.0;
            let omega = 2.0 * f32c::PI * s.tilt_freq;
            let n = 1.0 / (sr * 3.0 + omega);
            self.a0 = 2.0 * omega * n;
            self.b1 = (sr * 3.0 - omega) * n;
        }
        if changed(|s| s.mix) {
            self.mix = (f64::from(s.mix) * 0.01) as f32;
        }
        if changed(|s| s.auto_attack) {
            self.auto_att = s.auto_attack * 0.01;
        }
        if changed(|s| s.auto_release) {
            self.auto_rel = s.auto_release * 0.01;
        }
        if threshold || ratio || knee || limiter || flag(|s| s.auto_makeup) {
            // An estimate of a good make-up from the threshold, the ratio and the knee
            let gain = if -self.threshold < self.knee {
                let temp = -self.threshold + self.knee;
                ((if s.limiter { 0.0 } else { self.ratio }) - 1.0) * temp * temp / (4.0 * self.knee)
            } else if s.limiter {
                self.threshold
            } else {
                self.threshold - self.threshold * self.ratio
            };
            self.auto_makeup = 1.0 / dbfs_to_amp(gain);
        }
        self.settings = Some(*s);
    }

    #[inline]
    fn tilt_filter(&mut self, input: f32, n: usize) -> f32 {
        self.tilt_out[n] = self.a0 * input + self.b1 * self.tilt_out[n];
        input + self.lgain * self.tilt_out[n] + self.hgain * (input - self.tilt_out[n])
    }

    /// Compresses @p frames in place
    pub fn process(&mut self, frames: &mut [[f32; 2]], s: &CompressorPluginSettings) {
        self.apply(s);
        let (threshold, knee, ratio) = (self.threshold, self.knee, self.ratio);
        let lookahead = s.lookahead;
        let (in_balance, out_balance) = (s.in_balance, s.out_balance);

        for frame in frames.iter_mut() {
            let dry = *frame;
            let mut x = [dry[0] * self.in_gain, dry[1] * self.in_gain];

            // The tilt filters, to bias the sidechain to the lows or the highs
            if self.tilt != 0.0 {
                x[0] = self.tilt_filter(x[0], 0);
                x[1] = self.tilt_filter(x[1], 1);
            }
            if s.mid_side {
                let temp = x[0];
                x[0] = (temp + x[1]) * 0.5;
                x[1] = temp - x[1];
            }
            x[0] *= if in_balance > 0.0 { 1.0 - in_balance } else { 1.0 };
            x[1] *= if in_balance < 0.0 { 1.0 + in_balance } else { 1.0 };

            self.gain_result = [0.0; 2];
            for i in 0..2 {
                let mut input = if s.feedback && !lookahead { self.prev_out[i] } else { x[i] };

                // The crest factor: the peak over the RMS
                let squared = input * input;
                self.crest_peak[i] = NOISE_FLOOR
                    .max(squared)
                    .max(self.crest_time_const * self.crest_peak[i] + (1.0 - self.crest_time_const) * squared);
                self.crest_rms[i] = NOISE_FLOOR
                    .max(self.crest_time_const * self.crest_rms[i] + (1.0 - self.crest_time_const) * squared);
                self.crest_factor[i] = self.crest_peak[i] / self.crest_rms[i];
                self.rms[i] = self.rms_time_const * self.rms[i] + (1.0 - self.rms_time_const) * squared;

                // The peak or the RMS
                input = NOISE_FLOOR.max(if s.peak_mode { input.abs() } else { self.rms[i].sqrt() });
                let t = input;

                if t > self.y_l[i] {
                    // Attack. The crest factor rests at 2 with a sine: pulled to 0, scaled by how much automatic
                    // attack is applied, and put back
                    let crest = (self.crest_factor[i] - 2.0) * self.auto_att + 2.0;
                    let att =
                        if self.auto_att != 0.0 { self.ms_to_coeff(2.0 * s.attack / crest) } else { self.att_coeff };
                    self.y_l[i] = self.y_l[i] * att + (1.0 - att) * t;
                    self.hold_timer[i] = self.hold_length;
                } else {
                    // Release
                    let crest = (self.crest_factor[i] - 2.0) * self.auto_rel + 2.0;
                    let rel =
                        if self.auto_rel != 0.0 { self.ms_to_coeff(2.0 * s.release / crest) } else { self.rel_coeff };
                    if self.hold_timer[i] != 0 {
                        // The peak holds while the hold lasts
                        self.hold_timer[i] -= 1;
                    } else {
                        self.y_l[i] = self.y_l[i] * rel + (1.0 - rel) * t;
                    }
                }
                self.y_l[i] = NOISE_FLOOR.max(self.y_l[i]);

                let mut sc = self.y_l[i];
                if lookahead {
                    // The larger of the sidechain now and the sidechain delayed
                    let temp = sc;
                    let len = self.look_buf_length;
                    let delayed = (self.look_write + len - self.lookahead_length) % len;
                    sc = self.sc_look[i][self.look_write].max(self.sc_look[i][delayed]);
                    self.sc_look[i][self.look_write] = temp;
                }

                // The gain change for the level
                let level = amp_to_dbfs(sc);
                let gain = if level - threshold < -knee {
                    level
                } else if level - threshold < knee {
                    let temp = level - threshold + knee;
                    level + ((if s.limiter { 0.0 } else { ratio }) - 1.0) * temp * temp / (4.0 * knee)
                } else if s.limiter {
                    threshold
                } else {
                    threshold + (level - threshold) * ratio
                };
                self.gain_result[i] = self.range.max(dbfs_to_amp(gain) / sc);
            }

            let g = &mut self.gain_result;
            match s.stereo_link {
                StereoLink::Unlinked => {}
                StereoLink::Maximum => *g = [g[0].min(g[1]); 2],
                StereoLink::Average => *g = [(g[0] + g[1]) * 0.5; 2],
                StereoLink::Minimum => *g = [g[0].max(g[1]); 2],
                StereoLink::Blend => {
                    let blend = s.blend;
                    // 0 is unlinked
                    if blend > 0.0 {
                        if blend <= 1.0 {
                            // To the lower volume
                            let low = g[0].min(g[1]);
                            *g = [lerp(g[0], low, blend), lerp(g[1], low, blend)];
                        } else if blend <= 2.0 {
                            // To the average
                            let low = g[0].min(g[1]);
                            let average = (g[0] + g[1]) * 0.5;
                            *g = [lerp(low, average, blend - 1.0); 2];
                        } else {
                            // To the higher volume
                            let average = (g[0] + g[1]) * 0.5;
                            let high = g[0].max(g[1]);
                            *g = [lerp(average, high, blend - 2.0); 2];
                        }
                    }
                }
            }

            // Compression biased to the left or the right (or the mid or the side)
            let balance = s.stereo_balance;
            if balance != 0.0 {
                g[0] = 1.0 - (1.0 - g[0]) * if balance > 0.0 { 1.0 - balance } else { 1.0 };
                g[1] = 1.0 - (1.0 - g[1]) * if balance < 0.0 { 1.0 + balance } else { 1.0 };
            }

            // The signal delayed by the lookahead, through a ring
            if lookahead {
                x = [self.in_look[0][self.look_write], self.in_look[1][self.look_write]];
                self.in_look[0][self.look_write] = dry[0];
                self.in_look[1][self.look_write] = dry[1];
            } else {
                x = dry;
            }
            let delayed_dry = x;

            if s.mid_side {
                let temp = x[0];
                x[0] = (temp + x[1]) * 0.5;
                x[1] = temp - x[1];
            }
            x[0] *= if in_balance > 0.0 { 1.0 - in_balance } else { 1.0 };
            x[1] *= if in_balance < 0.0 { 1.0 + in_balance } else { 1.0 };

            x[0] *= g[0] * self.in_gain * self.out_gain * if out_balance > 0.0 { 1.0 - out_balance } else { 1.0 };
            x[1] *= g[1] * self.in_gain * self.out_gain * if out_balance < 0.0 { 1.0 + out_balance } else { 1.0 };

            if s.mid_side {
                let temp1 = x[0];
                let temp2 = x[1] * 0.5;
                x[0] = temp1 + temp2;
                x[1] = temp1 - temp2;
            }
            self.prev_out = x;

            if s.audition {
                // What the compression takes away
                x[0] = -x[0] + delayed_dry[0] * self.out_gain * self.in_gain;
                x[1] = -x[1] + delayed_dry[1] * self.out_gain * self.in_gain;
            } else if s.auto_makeup {
                x[0] *= self.auto_makeup;
                x[1] *= self.auto_makeup;
            }

            // Its own MIX knob
            frame[0] = (1.0 - self.mix) * delayed_dry[0] + self.mix * x[0];
            frame[1] = (1.0 - self.mix) * delayed_dry[1] + self.mix * x[1];

            self.look_write = if self.look_write == 0 { self.look_buf_length - 1 } else { self.look_write - 1 };
        }
    }

    /// The gains applied last, linear, for the meters
    pub fn gains(&self) -> [f32; 2] {
        self.gain_result
    }

    /// Back to rest, as the legacy program's bypass did
    pub fn reset(&mut self) {
        self.y_l = [NOISE_FLOOR; 2];
        self.gain_result = [1.0; 2];
        for buffer in &mut self.sc_look {
            buffer.fill(NOISE_FLOOR);
        }
        for buffer in &mut self.in_look {
            buffer.fill(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loud_signal_is_compressed_and_a_quiet_one_is_not() {
        let mut c = CompressorPlugin::new(48000.0);
        let settings = CompressorPluginSettings { threshold: -20.0, ratio: 4.0, ..CompressorPluginSettings::default() };
        let mut loud = vec![[0.9f32, 0.9]; 48000];
        c.process(&mut loud, &settings);
        assert!(loud[47999][0] < 0.5);
        let mut c = CompressorPlugin::new(48000.0);
        let mut quiet = vec![[0.01f32, 0.01]; 48000];
        c.process(&mut quiet, &settings);
        assert!((quiet[47999][0] - 0.01 * 0.999).abs() < 1e-4);
    }
}
