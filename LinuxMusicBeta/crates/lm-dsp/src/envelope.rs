//! An instrument's envelope and LFO, for its volume, its filter's cutoff and its resonance (legacy
//! `EnvelopeAndLfoParameters`).
//!
//! The legacy program kept the envelope in tables, grown (allocating) when automation made a time longer; here each
//! value is worked out where it is read, by the same single precision arithmetic: the same values, and nothing to
//! allocate.

use crate::oscillator::{NoiseRng, saw_sample, sin_sample, square_sample, triangle_sample, user_wave_sample};

/// The longest of each of the envelope's segments, s (the knob at its end)
pub const SECS_PER_ENV_SEGMENT: f32 = 5.0;
/// The longest LFO period, s
pub const SECS_PER_LFO_OSCILLATION: f32 = 20.0;
/// The fewest frames a segment that must not be empty has
const MINIMUM_FRAMES: i32 = 1;

/// The knob's value squared, keeping its sign: the envelope's times grow with the square of their knob
pub fn exp_knob_val(value: f32) -> f32 {
    (if value < 0.0 { -value } else { value }) * value
}

/// The LFO's wave shapes. Their keys are saved with projects and never change (U30).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    Saw,
    Square,
    UserDefined,
    Random,
}

impl LfoShape {
    pub const ALL: [LfoShape; 6] =
        [LfoShape::Sine, LfoShape::Triangle, LfoShape::Saw, LfoShape::Square, LfoShape::UserDefined, LfoShape::Random];

    pub fn key(self) -> &'static str {
        match self {
            LfoShape::Sine => "sine",
            LfoShape::Triangle => "triangle",
            LfoShape::Saw => "saw",
            LfoShape::Square => "square",
            LfoShape::UserDefined => "user",
            LfoShape::Random => "random",
        }
    }

    pub fn from_key(key: &str) -> Option<LfoShape> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// The knobs of one envelope and LFO
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeSettings {
    /// 0 to 2 (the knob: its time is 5 s times its square)
    pub predelay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    /// 0 to 1
    pub sustain: f32,
    pub release: f32,
    /// -1 to 1; 0 is off
    pub amount: f32,
    /// 0 to 1
    pub lfo_predelay: f32,
    pub lfo_attack: f32,
    /// 0.001 to 1: the LFO's period, in 20 s
    pub lfo_speed: f32,
    /// -1 to 1; 0 is off
    pub lfo_amount: f32,
    pub lfo_shape: LfoShape,
    /// The LFO 100 times faster
    pub x100: bool,
    /// The LFO modulates the envelope's amount, rather than adding to it
    pub control_env_amount: bool,
}

impl Default for EnvelopeSettings {
    fn default() -> Self {
        Self {
            predelay: 0.0,
            attack: 0.0,
            hold: 0.5,
            decay: 0.5,
            sustain: 0.5,
            release: 0.1,
            amount: 0.0,
            lfo_predelay: 0.0,
            lfo_attack: 0.0,
            lfo_speed: 0.1,
            lfo_amount: 0.0,
            lfo_shape: LfoShape::Sine,
            x100: false,
            control_env_amount: false,
        }
    }
}

/// One envelope and LFO, as an instrument has three (volume, cutoff, resonance). The LFO runs for the instrument,
/// not for each note: it moves on by each period (see [`EnvelopeAndLfo::next_period`])
#[derive(Clone, Debug)]
pub struct EnvelopeAndLfo {
    sample_rate: f32,
    value_for_zero_amount: f32,
    settings: EnvelopeSettings,
    used: bool,
    // The envelope
    predelay_frames: i32,
    attack_frames: i32,
    hold_frames: i32,
    decay_frames: i32,
    pahd_frames: i32,
    r_frames: i32,
    amount_add: f32,
    afi: f32,
    amsum: f32,
    dfi: f32,
    rfi: f32,
    sustain_level: f32,
    // The LFO
    lfo_predelay_frames: i32,
    lfo_attack_frames: i32,
    lfo_oscillation_frames: i32,
    lfo_frame: i64,
    lfo_amount: f32,
    lfo_amount_is_zero: bool,
    /// One period's frames of the LFO
    lfo_shape_data: Vec<f32>,
    random: f32,
    noise: NoiseRng,
}

impl EnvelopeAndLfo {
    /// Prepared for @p sample_rate and periods of up to @p max_period frames. @p value_for_zero_amount is its level
    /// when its amount is 0 (1 for the volume's, 0 for the filter's); @p seed is the random LFO's
    pub fn new(value_for_zero_amount: f32, sample_rate: f32, max_period: usize, seed: u64) -> Self {
        let mut e = Self {
            sample_rate,
            value_for_zero_amount,
            settings: EnvelopeSettings::default(),
            used: false,
            predelay_frames: 0,
            attack_frames: 1,
            hold_frames: 0,
            decay_frames: 1,
            pahd_frames: 0,
            r_frames: 1,
            amount_add: 0.0,
            afi: 0.0,
            amsum: 0.0,
            dfi: 0.0,
            rfi: 0.0,
            sustain_level: 0.0,
            lfo_predelay_frames: 0,
            lfo_attack_frames: 0,
            lfo_oscillation_frames: 1,
            lfo_frame: 0,
            lfo_amount: 0.0,
            lfo_amount_is_zero: true,
            lfo_shape_data: vec![0.0; max_period],
            random: 0.0,
            noise: NoiseRng::new(seed),
        };
        e.set(&EnvelopeSettings::default(), None, max_period);
        e
    }

    /// Whether it does anything: its amount or its LFO's is not 0
    pub fn is_used(&self) -> bool {
        self.used
    }

    /// The frames of its predelay, attack, hold and decay
    pub fn pahd_frames(&self) -> i32 {
        self.pahd_frames
    }

    /// The frames of its release
    pub fn release_frames(&self) -> i32 {
        self.r_frames
    }

    /// Takes @p settings (the legacy updateSampleVars); @p user_wave is the LFO's own wave; @p period the frames of
    /// the period the LFO's shape is worked out for
    pub fn set(&mut self, settings: &EnvelopeSettings, user_wave: Option<&[f32]>, period: usize) {
        let s = settings;
        self.settings = *s;
        let frames_per_env_seg = SECS_PER_ENV_SEGMENT * self.sample_rate;
        self.predelay_frames = (frames_per_env_seg * exp_knob_val(s.predelay)) as i32;
        self.attack_frames = MINIMUM_FRAMES.max((frames_per_env_seg * exp_knob_val(s.attack)) as i32);
        self.hold_frames = (frames_per_env_seg * exp_knob_val(s.hold)) as i32;
        self.decay_frames = MINIMUM_FRAMES.max((frames_per_env_seg * exp_knob_val(s.decay * (1.0 - s.sustain))) as i32);
        let amount = s.amount;
        self.amount_add =
            if amount >= 0.0 { (1.0 - amount) * self.value_for_zero_amount } else { self.value_for_zero_amount };
        self.pahd_frames = self.predelay_frames + self.attack_frames + self.hold_frames + self.decay_frames;
        self.r_frames = MINIMUM_FRAMES.max((frames_per_env_seg * exp_knob_val(s.release)) as i32);
        if (amount * 1000.0).floor() as i32 == 0 {
            self.r_frames = MINIMUM_FRAMES;
        }
        self.afi = (1.0 / self.attack_frames as f32) * amount;
        self.amsum = amount + self.amount_add;
        // (in double, as the legacy program: 1.0 is a double there)
        self.dfi = ((1.0 / f64::from(self.decay_frames)) * f64::from(s.sustain - 1.0) * f64::from(amount)) as f32;
        self.rfi = (1.0 / self.r_frames as f32) * amount;
        self.sustain_level = s.sustain * amount + self.amount_add;

        let frames_per_lfo_oscillation = SECS_PER_LFO_OSCILLATION * self.sample_rate;
        self.lfo_predelay_frames = (frames_per_lfo_oscillation * exp_knob_val(s.lfo_predelay)) as i32;
        self.lfo_attack_frames = (frames_per_lfo_oscillation * exp_knob_val(s.lfo_attack)) as i32;
        let mut oscillation = (frames_per_lfo_oscillation * s.lfo_speed) as i32;
        if s.x100 {
            oscillation /= 100;
        }
        self.lfo_oscillation_frames = oscillation.max(1);
        self.lfo_amount = s.lfo_amount * 0.5;
        self.lfo_amount_is_zero = (self.lfo_amount * 1000.0).floor() as i32 == 0;
        self.used = !self.lfo_amount_is_zero || (amount * 1000.0).floor() as i32 != 0;
        self.update_lfo_shape_data(user_wave, period);
    }

    /// The LFO moves on by @p frames (the period that ended) and works out its shape for the next period (the legacy
    /// `LfoInstances::trigger`, after each period)
    pub fn next_period(&mut self, frames: usize, user_wave: Option<&[f32]>, next_period: usize) {
        self.lfo_frame += frames as i64;
        if !self.lfo_amount_is_zero {
            self.update_lfo_shape_data(user_wave, next_period);
        }
    }

    /// The LFO from its start (the song starts)
    pub fn reset_lfo(&mut self, user_wave: Option<&[f32]>, period: usize) {
        self.lfo_frame = 0;
        if !self.lfo_amount_is_zero {
            self.update_lfo_shape_data(user_wave, period);
        }
    }

    fn update_lfo_shape_data(&mut self, user_wave: Option<&[f32]>, period: usize) {
        let frames = period.min(self.lfo_shape_data.len());
        for offset in 0..frames {
            let frame = ((self.lfo_frame + offset as i64) % i64::from(self.lfo_oscillation_frames)) as i32;
            let phase = frame as f32 / self.lfo_oscillation_frames as f32;
            let sample = match self.settings.lfo_shape {
                LfoShape::Triangle => triangle_sample(phase),
                LfoShape::Square => square_sample(phase),
                LfoShape::Saw => saw_sample(phase),
                LfoShape::UserDefined => user_wave.map_or(0.0, |w| user_wave_sample(w, phase)),
                LfoShape::Random => {
                    if frame == 0 {
                        self.random = self.noise.sample();
                    }
                    self.random
                }
                LfoShape::Sine => sin_sample(phase),
            };
            self.lfo_shape_data[offset] = sample * self.lfo_amount;
        }
    }

    /// The envelope until the release, at @p frame
    #[inline]
    fn pahd(&self, frame: i32) -> f32 {
        let attack_start = self.predelay_frames;
        let hold_start = attack_start + self.attack_frames;
        let decay_start = hold_start + self.hold_frames;
        if frame < attack_start {
            self.amount_add
        } else if frame < hold_start {
            (frame - attack_start) as f32 * self.afi + self.amount_add
        } else if frame < decay_start {
            self.amsum
        } else {
            self.amsum + (frame - decay_start) as f32 * self.dfi
        }
    }

    /// Writes into @p buf its level for the frames from @p frame (since the note started), the release beginning
    /// at @p release_begin (the legacy fillLevel)
    pub fn fill_level(&self, buf: &mut [f32], frame: i32, release_begin: i32) {
        // The LFO first
        if self.lfo_amount_is_zero || frame <= self.lfo_predelay_frames {
            buf.fill(0.0);
        } else {
            let lafi = 1.0 / MINIMUM_FRAMES.max(self.lfo_attack_frames) as f32;
            for (lfo_frame, (offset, out)) in (frame - self.lfo_predelay_frames..).zip(buf.iter_mut().enumerate()) {
                let shape = self.lfo_shape_data.get(offset).copied().unwrap_or(0.0);
                *out = if lfo_frame < self.lfo_attack_frames { shape * lfo_frame as f32 * lafi } else { shape };
            }
        }

        for (frame, out) in (frame..).zip(buf.iter_mut()) {
            let env = if frame < release_begin {
                if frame < self.pahd_frames { self.pahd(frame) } else { self.sustain_level }
            } else if frame - release_begin < self.r_frames {
                let from = if release_begin < self.pahd_frames { self.pahd(release_begin) } else { self.sustain_level };
                (self.r_frames - (frame - release_begin)) as f32 * self.rfi * from
            } else {
                0.0
            };
            // Here `out` holds the LFO's level
            *out = if self.settings.control_env_amount { env * (0.5 + *out) } else { env + *out };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_neutral_volume_envelope_is_unity() {
        let e = EnvelopeAndLfo::new(1.0, 48000.0, 256, 1);
        assert!(!e.is_used());
        let mut buf = [0.0f32; 64];
        e.fill_level(&mut buf, 0, 1000);
        assert!(buf.iter().all(|&x| x == 1.0));
    }

    #[test]
    fn the_envelope_attacks_holds_decays_sustains_and_releases() {
        let mut e = EnvelopeAndLfo::new(0.0, 1000.0, 256, 1);
        let s = EnvelopeSettings {
            attack: 0.1,
            hold: 0.1,
            decay: 0.2,
            sustain: 0.5,
            release: 0.1,
            amount: 1.0,
            ..EnvelopeSettings::default()
        };
        e.set(&s, None, 256);
        let mut buf = vec![0.0f32; 400];
        e.fill_level(&mut buf, 0, 300);
        assert_eq!(buf[0], 0.0);
        assert!((buf[49] - 0.98).abs() < 1e-6); // 50 frames of attack
        assert_eq!(buf[60], 1.0);
        assert!((buf[250] - 0.5).abs() < 1e-6);
        assert!(buf[399] < 0.5 && buf[399] >= 0.0);
    }
}
