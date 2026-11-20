//! A parameter's value law: how a normalized value (0..=1: a knob's travel, a controller, an automation lane's
//! height) becomes a value in the parameter's unit, and back. ONE law for the whole program: the compiler and the
//! renderer (mapped MIDI, automation), the session's automation edits and the interface (served through the
//! `CorePort`) all use this type, so a value means the same everywhere.

/// How a normalized control value (0..=1) becomes a parameter's value in its unit
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeScale {
    Linear,
    /// Logarithmic (frequencies, times): the minimum is above 0
    Log,
    /// Whole numbers, from `min` to `max`
    Stepped,
    /// A linear gain (a fader, a send), on the legacy fader's law: normalized `((dB + 120) / (top + 120))³`, 0 is
    /// silence; 0 dB at about 75 % with a maximum of +12 dB, so a controller's fader, the desk and the legacy agree
    Gain,
}

/// A parameter's range for mapping (from its schema)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamRange {
    pub min: f32,
    pub max: f32,
    pub scale: RangeScale,
}

/// The bottom of the gain scale
/// The legacy fader's law (`legado/src/gui/widgets/Fader.cpp`, `computeScaledRatio`): the travel is
/// `((dB − floor) / (top − floor))³`, with the floor at −120 dB (`c_faderMinDb`) and the exponent 3
/// (`c_dBScalingExponent`); below the floor is silence. 0 dB sits at about 75 % for a maximum of +12 dB
const GAIN_FLOOR_DB: f32 = -120.0;
const GAIN_EXPONENT: f32 = 3.0;

impl ParamRange {
    pub fn linear(min: f32, max: f32) -> ParamRange {
        ParamRange { min, max, scale: RangeScale::Linear }
    }

    /// Normalized → the parameter's unit
    pub fn to_units(&self, y: f32) -> f32 {
        let (lo, hi) = (self.min.min(self.max), self.min.max(self.max));
        self.units_unclamped(y).clamp(lo, hi)
    }

    fn units_unclamped(&self, y: f32) -> f32 {
        let y = y.clamp(0.0, 1.0);
        match self.scale {
            RangeScale::Linear => self.min + (self.max - self.min) * y,
            RangeScale::Stepped => (self.min + (self.max - self.min) * y).round().clamp(self.min, self.max),
            RangeScale::Log if self.min > 0.0 && self.max > 0.0 => self.min * libm::powf(self.max / self.min, y),
            RangeScale::Log => self.min + (self.max - self.min) * y,
            RangeScale::Gain => {
                if y <= 0.0 || self.max <= 0.0 {
                    return self.min.max(0.0);
                }
                let top = 20.0 * libm::log10f(self.max);
                let db = GAIN_FLOOR_DB + libm::powf(y, 1.0 / GAIN_EXPONENT) * (top - GAIN_FLOOR_DB);
                libm::powf(10.0, db / 20.0).clamp(self.min.max(0.0), self.max)
            }
        }
    }

    /// The parameter's unit → normalized
    pub fn normalize(&self, v: f32) -> f32 {
        let span = self.max - self.min;
        let y = match self.scale {
            RangeScale::Linear | RangeScale::Stepped => {
                if span.abs() < f32::EPSILON {
                    0.0
                } else {
                    (v - self.min) / span
                }
            }
            RangeScale::Log if self.min > 0.0 && self.max > self.min && v > 0.0 => {
                libm::logf(v / self.min) / libm::logf(self.max / self.min)
            }
            RangeScale::Log => {
                if span.abs() < f32::EPSILON {
                    0.0
                } else {
                    (v - self.min) / span
                }
            }
            RangeScale::Gain => {
                if v <= 0.0 || self.max <= 0.0 {
                    return 0.0;
                }
                let top = 20.0 * libm::log10f(self.max);
                let db = 20.0 * libm::log10f(v);
                if db <= GAIN_FLOOR_DB {
                    0.0
                } else {
                    libm::powf((db - GAIN_FLOOR_DB) / (top - GAIN_FLOOR_DB), GAIN_EXPONENT)
                }
            }
        };
        y.clamp(0.0, 1.0)
    }
}
