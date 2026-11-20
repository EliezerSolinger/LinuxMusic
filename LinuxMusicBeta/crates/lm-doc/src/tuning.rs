//! The microtuner's files (F-IJN-019/020): Scala scales (`.scl`) and keymaps (`.kbm`) read into the project's
//! `tuning` nodes (schemas `tuning.scale`, `tuning.keymap`), and the check of a scale's intervals. The core's
//! actions `tuning.import_scale` and `tuning.import_keymap` read a file with these and set a slot in one edit.

use lm_base::{Error, Result};
use lm_state::Value;

/// One interval in cents: a number with a point is cents ("701.955"), `a/b` or a whole number is a ratio
pub fn interval_cents(text: &str) -> Option<f64> {
    let t = text.split_whitespace().next()?;
    if t.contains('.') {
        return t.parse::<f64>().ok().filter(|c| c.is_finite());
    }
    let (a, b) = t.split_once('/').unwrap_or((t, "1"));
    let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
    (a > 0.0 && b > 0.0).then(|| 1200.0 * (a / b).log2())
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

/// A `.scl`: a description line, the number of notes, then that many intervals. Returns a `tuning.scale` node
pub fn parse_scl(text: &str) -> Result<Value> {
    let mut it = lines(text);
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
    Ok(Value::object([
        ("description", Value::text(description)),
        ("intervals", Value::list(intervals.into_iter().map(Value::text))),
    ]))
}

/// A `.kbm`: map size, first and last key, middle key, reference key, reference frequency, the octave's degree,
/// then a degree per key of the map (`x` = silent). Returns a `tuning.keymap` node
pub fn parse_kbm(text: &str, description: &str) -> Result<Value> {
    let mut it = lines(text).filter(|l| !l.trim().is_empty());
    let mut num = |n: usize| -> Result<f64> {
        it.next().and_then(|l| l.split_whitespace().next()?.parse::<f64>().ok()).ok_or_else(|| bad("kbm", n))
    };
    let size = num(1)? as usize;
    let key = |v: f64, n: usize| -> Result<i64> {
        if (0.0..=127.0).contains(&v) && v.fract() == 0.0 { Ok(v as i64) } else { Err(bad("kbm", n)) }
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
    let mut degrees = Vec::with_capacity(size);
    for k in 0..size {
        let Some(l) = it.next() else { break };
        let t = l.split_whitespace().next().unwrap_or("x");
        degrees.push(if t == "x" {
            -1
        } else {
            t.parse::<i64>().ok().filter(|d| (0..=1023).contains(d)).ok_or_else(|| bad("kbm", 8 + k))?
        });
    }
    Ok(Value::object([
        ("description", Value::text(description)),
        ("degrees", Value::list(degrees.into_iter().map(Value::int))),
        ("first", Value::int(first)),
        ("last", Value::int(last)),
        ("middle", Value::int(middle)),
        ("baseKey", Value::int(base_key)),
        ("baseFrequency", Value::real(freq)?),
    ]))
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
        assert_eq!(parse_intervals(["100.0", "abc"]), Err(2));
    }

    #[test]
    fn a_scala_scale_and_keymap() {
        let scl = "! meantone.scl\n!\nQuarter-comma meantone\n 3\n!\n 193.157\n 5/4\n 2/1\n";
        let s = parse_scl(scl).unwrap();
        assert_eq!(s.get("description").and_then(Value::as_text), Some("Quarter-comma meantone"));
        assert_eq!(s.get("intervals").unwrap().items().count(), 3);
        assert!(parse_scl("x\n2\n100.0\nnope\n").is_err());
        let kbm = "! white keys\n12\n0\n127\n60\n69\n440.0\n7\n0\nx\n1\nx\n2\n3\nx\n4\nx\n5\nx\n6\n";
        let k = parse_kbm(kbm, "White keys").unwrap();
        assert_eq!(k.get("degrees").unwrap().items().filter(|d| d.as_int() == Some(-1)).count(), 5);
        assert_eq!(k.get("baseFrequency").and_then(Value::as_real), Some(440.0));
        assert!(parse_kbm("12\n0\n127\n60\n69\n0\n7\n", "").is_err(), "a frequency of 0");
    }
}
