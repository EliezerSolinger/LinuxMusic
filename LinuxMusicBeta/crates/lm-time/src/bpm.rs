//! The tempo as an exact rational: millionths of a beat per minute

use std::fmt;
use std::str::FromStr;

use lm_base::{Error, Result};

/// A tempo in beats (quarter notes) per minute, exact.
///
/// The value is a rational with a decimal denominator: it is kept as a whole number of millionths of a BPM, so
/// `"119.88"` is exactly `11988/100` (= `2997/25`), never a binary float. The document stores it as decimal text
/// (`"bpm": "119.88"`), and [`Bpm::from_str`] / [`fmt::Display`] go back and forth without loss.
///
/// Range: [`Bpm::MIN`] (1) to [`Bpm::MAX`] (1000), at most [`Bpm::FRACTION_DIGITS`] (6) decimal places. These
/// bounds are what keeps every conversion of the tempo map inside fixed-width integers (see the crate docs).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Bpm {
    micro: u64,
}

impl Bpm {
    /// Millionths of a BPM per BPM: the fixed denominator of the stored value
    pub const SCALE: u64 = 1_000_000;
    /// Most decimal places a tempo can have
    pub const FRACTION_DIGITS: u32 = 6;
    /// The slowest tempo: 1 BPM
    pub const MIN: Bpm = Bpm { micro: Self::SCALE };
    /// The fastest tempo: 1000 BPM
    pub const MAX: Bpm = Bpm { micro: 1000 * Self::SCALE };

    /// From a whole number of millionths of a BPM (`120_000_000` is 120 BPM)
    pub fn from_micro(micro: u64) -> Result<Self> {
        if !(Self::MIN.micro..=Self::MAX.micro).contains(&micro) {
            return Err(Error::new("time.bpm_out_of_range")
                .arg("micro_bpm", i64::try_from(micro).unwrap_or(i64::MAX))
                .arg("min", Self::MIN.to_f64())
                .arg("max", Self::MAX.to_f64()));
        }
        Ok(Self { micro })
    }

    /// From a whole number of BPM
    pub fn from_integer(bpm: u32) -> Result<Self> {
        Self::from_micro(u64::from(bpm) * Self::SCALE)
    }

    /// Parses decimal text: ASCII digits, optionally a `.` followed by at least one digit (`"120"`, `"119.88"`,
    /// `"0120.500"`). No sign, exponent, spaces, comma or other separator. Leading zeros and trailing zeros after
    /// the point are accepted and do not count as precision.
    ///
    /// Errors (keys): `time.bpm_invalid` (not a decimal number), `time.bpm_too_precise` (more than six significant
    /// decimal places), `time.bpm_out_of_range`.
    pub fn parse(text: &str) -> Result<Self> {
        let invalid = || Error::new("time.bpm_invalid").arg("text", shorten(text));
        let (int_part, frac_part) = match text.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (text, None),
        };
        if int_part.is_empty() || !int_part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let frac_part = match frac_part {
            Some(f) if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) => return Err(invalid()),
            Some(f) => f.trim_end_matches('0'),
            None => "",
        };
        if frac_part.len() > Self::FRACTION_DIGITS as usize {
            return Err(Error::new("time.bpm_too_precise")
                .arg("text", shorten(text))
                .arg("digits", i64::from(Self::FRACTION_DIGITS)));
        }
        let int_part = int_part.trim_start_matches('0');
        // 1000 is the largest allowed whole part; anything with more digits is out of range without parsing it
        if int_part.len() > 4 {
            return Err(Error::new("time.bpm_out_of_range")
                .arg("text", shorten(text))
                .arg("min", Self::MIN.to_f64())
                .arg("max", Self::MAX.to_f64()));
        }
        let mut micro: u64 = 0;
        for b in int_part.bytes() {
            micro = micro * 10 + u64::from(b - b'0');
        }
        let mut frac: u64 = 0;
        for b in frac_part.bytes() {
            frac = frac * 10 + u64::from(b - b'0');
        }
        for _ in frac_part.len()..Self::FRACTION_DIGITS as usize {
            frac *= 10;
        }
        micro = micro * Self::SCALE + frac;
        Self::from_micro(micro).map_err(|e| e.arg("text", shorten(text)))
    }

    /// The value in millionths of a BPM
    pub fn micro(self) -> u64 {
        self.micro
    }

    /// The numerator of the value as a reduced fraction (`119.88` → `2997`)
    pub fn numer(self) -> u64 {
        self.micro / gcd(self.micro, Self::SCALE)
    }

    /// The denominator of the value as a reduced fraction (`119.88` → `25`)
    pub fn denom(self) -> u64 {
        Self::SCALE / gcd(self.micro, Self::SCALE)
    }

    /// The nearest `f64`, for display and for plugins' host-tempo fields. Never used in a conversion
    pub fn to_f64(self) -> f64 {
        // Both parts are exact in f64 (< 2^53), so this is the correctly rounded quotient
        self.micro as f64 / Self::SCALE as f64
    }
}

impl Default for Bpm {
    /// 120 BPM
    fn default() -> Self {
        Self { micro: 120 * Self::SCALE }
    }
}

impl FromStr for Bpm {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse(text)
    }
}

impl fmt::Display for Bpm {
    /// The canonical decimal text: no leading zeros, no trailing zeros after the point, no point for whole values.
    /// `Bpm::parse(&b.to_string()) == Ok(b)` for every `b`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.micro / Self::SCALE;
        let frac = self.micro % Self::SCALE;
        if frac == 0 {
            return write!(f, "{whole}");
        }
        let digits = format!("{frac:06}");
        write!(f, "{whole}.{}", digits.trim_end_matches('0'))
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Error arguments keep at most 32 characters of the offending text
fn shorten(text: &str) -> String {
    text.chars().take(32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_text_exactly() {
        let b = Bpm::parse("119.88").unwrap();
        assert_eq!(b.micro(), 119_880_000);
        assert_eq!((b.numer(), b.denom()), (2997, 25));
        assert_eq!(Bpm::parse("120").unwrap(), Bpm::default());
        assert_eq!((Bpm::default().numer(), Bpm::default().denom()), (120, 1));
        assert_eq!(Bpm::parse("0120.500").unwrap().micro(), 120_500_000);
        assert_eq!(Bpm::parse("1").unwrap(), Bpm::MIN);
        assert_eq!(Bpm::parse("1000.000000000").unwrap(), Bpm::MAX);
        assert_eq!(Bpm::parse("1.000001").unwrap().micro(), 1_000_001);
        assert_eq!(Bpm::parse("999.999999").unwrap().micro(), 999_999_999);
    }

    #[test]
    fn rejects_what_is_not_a_plain_decimal() {
        for text in [
            "",
            ".",
            ".5",
            "5.",
            "+120",
            "-120",
            " 120",
            "120 ",
            "1e2",
            "12,5",
            "120..0",
            "1.2.3",
            "0x10",
            "١٢٠",
            "１２０",
            "120.5a",
            "NaN",
            "inf",
        ] {
            let e = Bpm::parse(text).unwrap_err();
            assert_eq!(e.key(), "time.bpm_invalid", "{text:?}");
        }
    }

    #[test]
    fn rejects_out_of_range_and_too_precise() {
        for text in ["0", "0.999999", "1000.000001", "1001", "10000", "99999999999999999999999999", "0.0"] {
            assert_eq!(Bpm::parse(text).unwrap_err().key(), "time.bpm_out_of_range", "{text:?}");
        }
        assert_eq!(Bpm::parse("120.0000001").unwrap_err().key(), "time.bpm_too_precise");
        assert_eq!(Bpm::parse("120.1234567").unwrap_err().key(), "time.bpm_too_precise");
        assert!(Bpm::from_micro(999_999).is_err());
        assert!(Bpm::from_integer(1001).is_err());
        assert!(Bpm::from_integer(0).is_err());
    }

    #[test]
    fn display_is_canonical_and_round_trips() {
        assert_eq!(Bpm::parse("0120.500").unwrap().to_string(), "120.5");
        assert_eq!(Bpm::parse("119.880").unwrap().to_string(), "119.88");
        assert_eq!(Bpm::parse("1.000001").unwrap().to_string(), "1.000001");
        assert_eq!(Bpm::MAX.to_string(), "1000");
        for micro in (Bpm::MIN.micro()..=Bpm::MAX.micro()).step_by(7_919_993) {
            let b = Bpm::from_micro(micro).unwrap();
            assert_eq!(Bpm::parse(&b.to_string()).unwrap(), b);
        }
    }

    #[test]
    fn ordering_is_by_value() {
        assert!(Bpm::parse("119.99").unwrap() < Bpm::parse("120").unwrap());
        assert!(Bpm::parse("99.5").unwrap() < Bpm::parse("100").unwrap());
    }
}
