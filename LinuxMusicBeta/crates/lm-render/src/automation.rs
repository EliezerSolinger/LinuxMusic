//! Automation curves (arquitetura-nova.md, base item 1): points with a shape from −1 to 1, evaluated by the renderer
//! per sample (`Rate::Sample`, the host's gains and pans) or on a grid of 32 samples (`Rate::Block`). Discrete
//! parameters are never interpolated: they change at the exact sample of a point.
//!
//! The compiler converts the points' ticks to samples with the program's tempo map (a tempo edit recompiles), so the
//! audio thread never converts a curve's time

use std::sync::Arc;

use lm_base::{Error, Result};

/// The steepness of a shaped segment: with shape `c`, the segment follows `(e^(k·t) − 1) / (e^k − 1)` with
/// `k = CURVE_STEEPNESS · c` (`c > 0` starts slow and ends fast, `c < 0` the opposite, `c = 0` is a straight line)
pub const CURVE_STEEPNESS: f64 = 6.0;

/// One point of a curve. `shape` is the shape of the segment that **ends** at this point (the document's `curve`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePoint {
    pub sample: i64,
    pub value: f32,
    pub shape: f32,
}

/// A curve: points in increasing sample order. Shared (an `Arc`), so programs that did not change it share it
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    points: Arc<[CurvePoint]>,
}

impl Curve {
    /// Keys: `render.curve.empty`, `render.curve.not_increasing` (two points may share a sample: a jump),
    /// `render.curve.not_finite`, `render.curve.shape_out_of_range`
    pub fn new(points: Vec<CurvePoint>) -> Result<Self> {
        if points.is_empty() {
            return Err(Error::new("render.curve.empty"));
        }
        for (i, p) in points.iter().enumerate() {
            if !p.value.is_finite() {
                return Err(Error::new("render.curve.not_finite").arg("point", i as i64));
            }
            if !(-1.0..=1.0).contains(&p.shape) {
                return Err(Error::new("render.curve.shape_out_of_range").arg("point", i as i64));
            }
            if i > 0 && p.sample < points[i - 1].sample {
                return Err(Error::new("render.curve.not_increasing").arg("point", i as i64));
            }
        }
        Ok(Self { points: points.into() })
    }

    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// Index of the last point at or before `sample`, or `None` before the first
    fn last_at(&self, sample: i64) -> Option<usize> {
        self.points.partition_point(|p| p.sample <= sample).checked_sub(1)
    }

    /// The continuous value at a sample
    pub fn value_at(&self, sample: i64) -> f32 {
        match self.last_at(sample) {
            None => self.points[0].value,
            Some(i) if i + 1 == self.points.len() => self.points[i].value,
            Some(i) => segment(&self.points[i], &self.points[i + 1], sample),
        }
    }

    /// The discrete value at a sample: the last point's value, never interpolated
    pub fn step_at(&self, sample: i64) -> f32 {
        self.points[self.last_at(sample).unwrap_or(0)].value
    }

    /// Fills `out[i]` with the value at `start + i` (one search, then a walk)
    pub fn fill(&self, start: i64, out: &mut [f32]) {
        let n = self.points.len();
        let mut i = self.last_at(start);
        for (k, o) in out.iter_mut().enumerate() {
            let s = start + k as i64;
            // Advance to the last point at or before s
            loop {
                let next = i.map_or(0, |i| i + 1);
                if next < n && self.points[next].sample <= s {
                    i = Some(next);
                } else {
                    break;
                }
            }
            *o = match i {
                None => self.points[0].value,
                Some(i) if i + 1 == n => self.points[i].value,
                Some(i) => segment(&self.points[i], &self.points[i + 1], s),
            };
        }
    }

    /// The samples in `from..to` where a discrete value changes (the points there), in order
    pub(crate) fn points_in(&self, from: i64, to: i64) -> &[CurvePoint] {
        let a = self.points.partition_point(|p| p.sample < from);
        let b = self.points.partition_point(|p| p.sample < to);
        &self.points[a..b]
    }
}

fn segment(a: &CurvePoint, b: &CurvePoint, sample: i64) -> f32 {
    let span = (b.sample - a.sample) as f64;
    if span <= 0.0 {
        return b.value;
    }
    let t = ((sample - a.sample) as f64 / span).clamp(0.0, 1.0);
    let f = shape(t, f64::from(b.shape));
    (f64::from(a.value) + (f64::from(b.value) - f64::from(a.value)) * f) as f32
}

/// The shape function of a segment (see [`CURVE_STEEPNESS`])
pub fn shape(t: f64, c: f64) -> f64 {
    let k = CURVE_STEEPNESS * c;
    if k.abs() < 1e-9 {
        return t;
    }
    libm::expm1(k * t) / libm::expm1(k)
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    fn p(sample: i64, value: f32, shape: f32) -> CurvePoint {
        CurvePoint { sample, value, shape }
    }

    #[test]
    fn linear_and_holds() {
        let c = Curve::new(vec![p(100, 0.0, 0.0), p(200, 1.0, 0.0)]).expect("curve");
        assert_eq!(c.value_at(0), 0.0);
        assert_eq!(c.value_at(150), 0.5);
        assert_eq!(c.value_at(500), 1.0);
        assert_eq!(c.step_at(199), 0.0);
        assert_eq!(c.step_at(200), 1.0);
        let mut out = [0.0; 8];
        c.fill(146, &mut out);
        for (k, v) in out.iter().enumerate() {
            assert_eq!(*v, c.value_at(146 + k as i64));
        }
    }

    #[test]
    fn shapes_bend_and_stay_in_range() {
        for c in [-1.0, -0.3, 0.3, 1.0] {
            assert!((shape(0.0, c)).abs() < 1e-12);
            assert!((shape(1.0, c) - 1.0).abs() < 1e-12);
            let mid = shape(0.5, c);
            assert!(mid > 0.0 && mid < 1.0);
            assert_eq!(mid < 0.5, c > 0.0);
        }
    }

    #[test]
    fn jumps_and_errors() {
        let c = Curve::new(vec![p(0, 0.0, 0.0), p(10, 0.0, 0.0), p(10, 1.0, 0.0)]).expect("jump");
        assert_eq!(c.value_at(9), 0.0);
        assert_eq!(c.value_at(10), 1.0);
        assert!(Curve::new(vec![]).is_err());
        assert!(Curve::new(vec![p(5, 0.0, 0.0), p(4, 0.0, 0.0)]).is_err());
        assert!(Curve::new(vec![p(5, f32::NAN, 0.0)]).is_err());
        assert_eq!(c.points_in(0, 10).len(), 1);
        assert_eq!(c.points_in(10, 11).len(), 2);
    }
}
