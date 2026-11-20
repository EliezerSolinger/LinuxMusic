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

/// The kind of the segment that ends at a point (I-21.5; the document's `shape`)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Segment {
    /// Bent by the point's `shape` (−1..1; 0 is a straight line)
    #[default]
    Curve,
    /// Holds the previous value until the point, then jumps
    Step,
    /// A cardinal spline through the neighbours; `tension` 0 is Catmull-Rom, 1 has flat tangents. Kept inside the
    /// two values it joins (never overshoots)
    Smooth { tension: f32 },
}

/// A curve: points in increasing sample order. Shared (an `Arc`), so programs that did not change it share it
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    points: Arc<[CurvePoint]>,
    /// The kind of the segment ending at each point (empty: all `Curve`)
    segments: Arc<[Segment]>,
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
        Ok(Self { points: points.into(), segments: Arc::from(Vec::new()) })
    }

    /// A curve with a kind per segment (`segments[i]` is the segment ending at point `i`). Keys: those of
    /// [`Curve::new`], and `render.curve.segments_length`, `render.curve.tension_out_of_range`
    pub fn with_segments(points: Vec<CurvePoint>, segments: Vec<Segment>) -> Result<Self> {
        if segments.len() != points.len() {
            return Err(Error::new("render.curve.segments_length"));
        }
        for (i, s) in segments.iter().enumerate() {
            if let Segment::Smooth { tension } = s
                && !(0.0..=1.0).contains(tension)
            {
                return Err(Error::new("render.curve.tension_out_of_range").arg("point", i as i64));
            }
        }
        let mut c = Self::new(points)?;
        if segments.iter().any(|s| *s != Segment::Curve) {
            c.segments = segments.into();
        }
        Ok(c)
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    fn segment_kind(&self, i: usize) -> Segment {
        self.segments.get(i).copied().unwrap_or_default()
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
            Some(i) => self.segment(i, sample),
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
                Some(i) => self.segment(i, s),
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

impl Curve {
    /// The value inside the segment from point `i` to point `i + 1`
    fn segment(&self, i: usize, sample: i64) -> f32 {
        let (a, b) = (&self.points[i], &self.points[i + 1]);
        let span = (b.sample - a.sample) as f64;
        if span <= 0.0 {
            return b.value;
        }
        let t = ((sample - a.sample) as f64 / span).clamp(0.0, 1.0);
        let (va, vb) = (f64::from(a.value), f64::from(b.value));
        match self.segment_kind(i + 1) {
            Segment::Curve => (va + (vb - va) * shape(t, f64::from(b.shape))) as f32,
            Segment::Step => {
                if sample >= b.sample {
                    b.value
                } else {
                    a.value
                }
            }
            Segment::Smooth { tension } => {
                // Tangents (value per sample) from the neighbours, scaled by 1 − tension; flat at the ends
                let c = 1.0 - f64::from(tension);
                let tangent = |k: usize| -> f64 {
                    if k == 0 || k + 1 >= self.points.len() {
                        return 0.0;
                    }
                    let (p, n) = (&self.points[k - 1], &self.points[k + 1]);
                    let dt = (n.sample - p.sample) as f64;
                    if dt <= 0.0 { 0.0 } else { c * (f64::from(n.value) - f64::from(p.value)) / dt }
                };
                let (ma, mb) = (tangent(i) * span, tangent(i + 1) * span);
                let (t2, t3) = (t * t, t * t * t);
                let v = (2.0 * t3 - 3.0 * t2 + 1.0) * va
                    + (t3 - 2.0 * t2 + t) * ma
                    + (-2.0 * t3 + 3.0 * t2) * vb
                    + (t3 - t2) * mb;
                v.clamp(va.min(vb), va.max(vb)) as f32
            }
        }
    }
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
    fn step_and_smooth_segments() {
        let pts = vec![p(0, 0.0, 0.0), p(100, 1.0, 0.0), p(200, 0.0, 0.0), p(300, 1.0, 0.0)];
        let c = Curve::with_segments(
            pts.clone(),
            vec![Segment::Curve, Segment::Step, Segment::Smooth { tension: 0.0 }, Segment::Smooth { tension: 1.0 }],
        )
        .expect("curve");
        assert_eq!(c.value_at(99), 0.0, "a step holds");
        assert_eq!(c.value_at(100), 1.0);
        // Smooth: continuous at the ends, inside the two values, symmetric with flat tangents (tension 1)
        assert!((c.value_at(101) - 1.0).abs() < 0.01);
        assert!((c.value_at(199) - 0.0).abs() < 0.01);
        let mut last = f32::MAX;
        for s in 100..=200 {
            let v = c.value_at(s);
            assert!((0.0..=1.0).contains(&v));
            assert!(v <= last + 1e-6, "monotone between a high and a low point");
            last = v;
        }
        assert!((c.value_at(250) - 0.5).abs() < 1e-6);
        let mut out = [0.0; 300];
        c.fill(0, &mut out);
        for (k, v) in out.iter().enumerate() {
            assert_eq!(*v, c.value_at(k as i64));
        }
        assert!(Curve::with_segments(pts.clone(), vec![Segment::Curve]).is_err());
        assert!(Curve::with_segments(pts, vec![Segment::Smooth { tension: 2.0 }; 4]).is_err());
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
