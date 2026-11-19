//! Ticks per quarter note, time signatures and the bar:beat:tick position

use std::fmt;

use lm_base::{Error, Result};

/// Ticks per quarter note (`project.ppq`), stored per project. The default is 960
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Ppq(u32);

impl Ppq {
    /// The default resolution: 960 ticks per quarter note
    pub const DEFAULT: Ppq = Ppq(960);
    /// The largest accepted resolution (it bounds the integer sizes of the conversions)
    pub const MAX: u32 = 100_000;

    /// `1..=100_000`; key `time.ppq_invalid` otherwise
    pub fn new(ppq: u32) -> Result<Self> {
        if ppq == 0 || ppq > Self::MAX {
            return Err(Error::new("time.ppq_invalid").arg("ppq", i64::from(ppq)).arg("max", i64::from(Self::MAX)));
        }
        Ok(Self(ppq))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl Default for Ppq {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A time signature: `numerator` beats of `1/denominator` notes per bar
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimeSignature {
    numerator: u8,
    denominator: u8,
}

impl TimeSignature {
    /// 4/4
    pub const COMMON: TimeSignature = TimeSignature { numerator: 4, denominator: 4 };
    /// The largest numerator
    pub const MAX_NUMERATOR: u8 = 128;
    /// The largest denominator (denominators are powers of two)
    pub const MAX_DENOMINATOR: u8 = 64;

    /// Numerator `1..=128`, denominator a power of two in `1..=64`; key `time.signature_invalid` otherwise.
    /// Whether the beat is a whole number of ticks depends on the project's PPQ and is checked by the tempo map
    pub fn new(numerator: u8, denominator: u8) -> Result<Self> {
        if numerator == 0
            || numerator > Self::MAX_NUMERATOR
            || !denominator.is_power_of_two()
            || denominator > Self::MAX_DENOMINATOR
        {
            return Err(Error::new("time.signature_invalid")
                .arg("numerator", i64::from(numerator))
                .arg("denominator", i64::from(denominator)));
        }
        Ok(Self { numerator, denominator })
    }

    pub fn numerator(self) -> u8 {
        self.numerator
    }

    pub fn denominator(self) -> u8 {
        self.denominator
    }

    /// Ticks per beat (one `1/denominator` note), if that is a whole number at this PPQ
    pub fn beat_ticks(self, ppq: Ppq) -> Option<i64> {
        let whole = 4 * i64::from(ppq.get());
        let den = i64::from(self.denominator);
        (whole % den == 0).then_some(whole / den)
    }

    /// Ticks per full bar, if the beat is a whole number of ticks at this PPQ
    pub fn bar_ticks(self, ppq: Ppq) -> Option<i64> {
        self.beat_ticks(ppq).map(|b| b * i64::from(self.numerator))
    }
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self::COMMON
    }
}

impl fmt::Display for TimeSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.numerator, self.denominator)
    }
}

/// A position as bar, beat and tick, as the user reads it.
///
/// `bar` counts from 1 at tick 0 (bars before the start are 0, -1, ...); `beat` counts from 1 inside the bar;
/// `tick` counts from 0 inside the beat. A beat is one `1/denominator` note of the time signature in force
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BarBeatTick {
    pub bar: i64,
    pub beat: u32,
    pub tick: u32,
}

impl fmt::Display for BarBeatTick {
    /// `bar:beat:tick`, for logs and tests (the interface formats it itself)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.bar, self.beat, self.tick)
    }
}

/// Everything the interface shows about where the music is, computed from a sample position
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MusicalPosition {
    /// The canonical position
    pub sample: i64,
    /// The tick reached at that sample ([`crate::TempoMap::tick_at`])
    pub tick: i64,
    pub bar_beat_tick: BarBeatTick,
    pub time_signature: TimeSignature,
    /// The tempo at that sample, for display only (inside a ramp it changes continuously)
    pub bpm: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_are_validated() {
        assert!(TimeSignature::new(7, 8).is_ok());
        assert!(TimeSignature::new(1, 1).is_ok());
        assert!(TimeSignature::new(128, 64).is_ok());
        for (n, d) in [(0, 4), (4, 0), (4, 3), (4, 12), (129, 4), (4, 128)] {
            assert_eq!(TimeSignature::new(n, d).unwrap_err().key(), "time.signature_invalid", "{n}/{d}");
        }
    }

    #[test]
    fn beat_and_bar_lengths() {
        let ppq = Ppq::DEFAULT;
        assert_eq!(TimeSignature::COMMON.bar_ticks(ppq), Some(3840));
        assert_eq!(TimeSignature::new(7, 8).unwrap().beat_ticks(ppq), Some(480));
        assert_eq!(TimeSignature::new(7, 8).unwrap().bar_ticks(ppq), Some(3360));
        assert_eq!(TimeSignature::new(3, 64).unwrap().beat_ticks(ppq), Some(60));
        // At 3 ticks per quarter, a sixteenth is not a whole number of ticks
        assert_eq!(TimeSignature::new(4, 16).unwrap().beat_ticks(Ppq::new(3).unwrap()), None);
    }

    #[test]
    fn ppq_is_validated() {
        assert_eq!(Ppq::default().get(), 960);
        assert!(Ppq::new(0).is_err());
        assert!(Ppq::new(100_001).is_err());
        assert_eq!(Ppq::new(1).unwrap().get(), 1);
    }
}
