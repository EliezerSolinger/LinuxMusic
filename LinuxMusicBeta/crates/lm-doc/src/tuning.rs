//! The tuning files (F-IJN-019/020): Scala scales (`.scl`) and keymaps (`.kbm`) read from the bytes of a resource of
//! kind `"tuning"`. A track cites the files; nothing of them is kept in the tree. The math of the frequencies is the
//! compiler's (`lm-compile`, `keys.rs`); this module only reads and checks the formats.

use lm_base::{Error, Result};

/// The largest tuning file read (a scale or a keymap is a few kilobytes; a bigger file is not one)
pub const FILE_MAX_BYTES: u64 = 1 << 20;

/// A Scala scale: its description and its intervals above the unison, the last one the period
#[derive(Clone, Debug, PartialEq)]
pub struct Scale {
    pub description: String,
    /// Each a cents value with a point ("701.955") or a ratio ("3/2")
    pub intervals: Vec<String>,
}

/// A Scala keymap
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    /// The scale degree of each key from `middle` on, −1 = silent ("x"); empty = the linear map
    pub degrees: Vec<i64>,
    pub first: i32,
    pub last: i32,
    pub middle: i32,
    pub base_key: i32,
    pub base_frequency: f64,
}

/// One interval in cents: a number with a point is cents ("701.955"), `a/b` or a whole number is a ratio
pub fn interval_cents(text: &str) -> Option<f64> {
    let t = text.split_whitespace().next()?;
    if t.contains('.') {
        return t.parse::<f64>().ok().filter(|c| c.is_finite());
    }
    let (a, b) = t.split_once('/').unwrap_or((t, "1"));
    let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
    // `inf/1`, `1e400/1` (parsed as inf) and ratios that over- or underflow give no finite interval
    (a > 0.0 && b > 0.0).then(|| 1200.0 * (a / b).log2()).filter(|c| c.is_finite())
}

/// The intervals of a scale in cents, or the number (1-based) of the first one that is not an interval
pub fn parse_intervals<'a>(lines: impl IntoIterator<Item = &'a str>) -> std::result::Result<Vec<f64>, usize> {
    let mut out = Vec::new();
    for (i, l) in lines.into_iter().enumerate() {
        out.push(interval_cents(l).ok_or(i + 1)?);
    }
    if out.is_empty() { Err(1) } else { Ok(out) }
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim_end).filter(|l| !l.trim_start().starts_with('!'))
}

fn bad(file: &str, line: usize) -> Error {
    Error::new("tuning.file.invalid").arg("file", file).arg("line", line as i64)
}

/// A `.scl`: a description line, the number of notes, then that many intervals
pub fn parse_scl(text: &str) -> Result<Scale> {
    let mut it = lines(text.trim_start_matches('\u{feff}'));
    let description = it.next().ok_or_else(|| bad("scl", 1))?.trim().to_string();
    let count: usize =
        it.next().and_then(|l| l.split_whitespace().next()?.parse().ok()).ok_or_else(|| bad("scl", 2))?;
    let intervals: Vec<String> = it
        .filter(|l| !l.trim().is_empty())
        .take(count)
        .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
        .collect();
    if intervals.len() != count || count == 0 {
        return Err(bad("scl", 3 + intervals.len()));
    }
    if let Err(k) = parse_intervals(intervals.iter().map(String::as_str)) {
        return Err(bad("scl", 2 + k));
    }
    Ok(Scale { description, intervals })
}

/// A `.kbm`: map size, first and last key, middle key, reference key, reference frequency, the octave's degree,
/// then a degree per key of the map (`x` = silent)
pub fn parse_kbm(text: &str) -> Result<Keymap> {
    let mut it = lines(text.trim_start_matches('\u{feff}')).filter(|l| !l.trim().is_empty());
    let mut num = |n: usize| -> Result<f64> {
        it.next().and_then(|l| l.split_whitespace().next()?.parse::<f64>().ok()).ok_or_else(|| bad("kbm", n))
    };
    let size = num(1)? as usize;
    let key = |v: f64, n: usize| -> Result<i32> {
        if (0.0..=127.0).contains(&v) && v.fract() == 0.0 { Ok(v as i32) } else { Err(bad("kbm", n)) }
    };
    let first = key(num(2)?, 2)?;
    let last = key(num(3)?, 3)?;
    let middle = key(num(4)?, 4)?;
    let base_key = key(num(5)?, 5)?;
    let freq = num(6)?;
    if !(0.1..=9999.999).contains(&freq) || first > last {
        return Err(bad("kbm", 6));
    }
    let _octave = num(7)?;
    let mut degrees = Vec::with_capacity(size.min(128));
    for k in 0..size.min(1024) {
        let Some(l) = it.next() else { break };
        let t = l.split_whitespace().next().unwrap_or("x");
        degrees.push(if t == "x" || t == "X" {
            -1
        } else {
            t.parse::<i64>().ok().filter(|d| (0..=1023).contains(d)).ok_or_else(|| bad("kbm", 8 + k))?
        });
    }
    Ok(Keymap { degrees, first, last, middle, base_key, base_frequency: freq })
}

/// A file's bytes as the text of a tuning file (invalid UTF-8 replaced: the files are ASCII in practice)
pub fn text_of(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_are_cents_or_ratios() {
        assert!((interval_cents("701.955").unwrap() - 701.955).abs() < 1e-9);
        assert!((interval_cents("3/2").unwrap() - 701.955).abs() < 0.001);
        assert_eq!(interval_cents("2"), Some(1200.0));
        assert_eq!(interval_cents("0/2"), None);
        assert_eq!(interval_cents("inf/1"), None);
        assert_eq!(interval_cents("1e400/1"), None);
        assert_eq!(interval_cents("inf/inf"), None);
        assert_eq!(interval_cents("1e-300/1e300"), None);
        assert_eq!(parse_intervals(["100.0", "abc"]), Err(2));
    }

    #[test]
    fn a_scala_scale_and_keymap() {
        let scl = "! meantone.scl\n!\nQuarter-comma meantone\n 3\n!\n 193.157\n 5/4\n 2/1\n";
        let s = parse_scl(scl).unwrap();
        assert_eq!(s.description, "Quarter-comma meantone");
        assert_eq!(s.intervals.len(), 3);
        assert!(parse_scl("x\n2\n100.0\nnope\n").is_err());
        let kbm = "! white keys\n12\n0\n127\n60\n69\n440.0\n7\n0\nx\n1\nx\n2\n3\nx\n4\nx\n5\nx\n6\n";
        let k = parse_kbm(kbm).unwrap();
        assert_eq!(k.degrees.iter().filter(|d| **d == -1).count(), 5);
        assert_eq!(k.base_frequency, 440.0);
        assert!(parse_kbm("12\n0\n127\n60\n69\n0\n7\n").is_err(), "a frequency of 0");
    }

    #[test]
    fn a_scale_with_a_byte_order_mark_and_an_empty_description() {
        let s = parse_scl("\u{feff}\n 1\n 2/1\n").unwrap();
        assert_eq!(s.description, "");
        assert_eq!(s.intervals, vec!["2/1".to_string()]);
    }
}
