//! The per-note MPE curves of the Piano Roll (`decisoes-autonomas.md`, 26/11, item 5): beside the bend, a note keeps
//! a pressure and a timbre curve (0..1, linear between points, from the note's start). The Pitch tool edits the one
//! the curve selector says.

use lm_port::Value;

use crate::model;

/// One point of a 0..1 curve: ticks from the note's start and the value
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unit {
    pub at: i64,
    pub value: f64,
}

/// Which curve of a note the Pitch tool draws
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Curve {
    #[default]
    Bend,
    Pressure,
    Timbre,
}

impl Curve {
    pub const ALL: [Curve; 3] = [Curve::Bend, Curve::Pressure, Curve::Timbre];

    pub fn id(self) -> &'static str {
        match self {
            Curve::Bend => "pr-curve-bend",
            Curve::Pressure => "pr-curve-pressure",
            Curve::Timbre => "pr-curve-timbre",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Curve::Bend => "Bend",
            Curve::Pressure => "Pressure",
            Curve::Timbre => "Timbre",
        }
    }
}

/// A note's pressure and timbre curves (the bend stays a field of the note)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MpeCurves {
    pub pressure: Vec<Unit>,
    pub timbre: Vec<Unit>,
}

fn read_list(v: &Value, key: &str) -> Vec<Unit> {
    model::list(v, key)
        .map(|p| Unit { at: model::int(p, "at", 0).max(0), value: model::real(p, "value", 0.0).clamp(0.0, 1.0) })
        .collect()
}

fn list_value(points: &[Unit]) -> Value {
    Value::list(points.iter().map(|p| {
        Value::object([
            ("at", Value::int(p.at)),
            ("value", Value::real(p.value.clamp(0.0, 1.0)).unwrap_or(Value::NULL)),
        ])
    }))
}

impl MpeCurves {
    pub fn read(note: &Value) -> Self {
        Self { pressure: read_list(note, "pressure"), timbre: read_list(note, "timbre") }
    }

    /// The fields of a note's object for the curves that say something (optional fields)
    pub fn fields(&self) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        if !self.pressure.is_empty() {
            out.push(("pressure", list_value(&self.pressure)));
        }
        if !self.timbre.is_empty() {
            out.push(("timbre", list_value(&self.timbre)));
        }
        out
    }

    /// The fields that differ from `old`
    pub fn changed(&self, old: &MpeCurves) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        if self.pressure != old.pressure {
            out.push(("pressure", list_value(&self.pressure)));
        }
        if self.timbre != old.timbre {
            out.push(("timbre", list_value(&self.timbre)));
        }
        out
    }

    pub fn get(&self, c: Curve) -> &[Unit] {
        match c {
            Curve::Pressure => &self.pressure,
            Curve::Timbre => &self.timbre,
            Curve::Bend => &[],
        }
    }

    /// The same curves with one replaced
    pub fn with(&self, c: Curve, points: Vec<Unit>) -> Self {
        let mut x = self.clone();
        match c {
            Curve::Pressure => x.pressure = points,
            Curve::Timbre => x.timbre = points,
            Curve::Bend => {}
        }
        x
    }
}

/// The value of a curve at a tick from the note's start (0 for a curve with no points)
pub fn value_at(points: &[Unit], at: i64) -> f64 {
    match points {
        [] => 0.0,
        [p] => p.value,
        pts => {
            if at <= pts[0].at {
                return pts[0].value;
            }
            for w in pts.windows(2) {
                if at <= w[1].at {
                    let f = (at - w[0].at) as f64 / (w[1].at - w[0].at).max(1) as f64;
                    return w[0].value + (w[1].value - w[0].value) * f;
                }
            }
            pts[pts.len() - 1].value
        }
    }
}

/// The curve after a drag from tick `at0` to tick `at1` that moved the value by `dv`: a straight line from the value
/// at the start of the drag to that plus `dv` (what the bend's drag does), the points under it replaced
pub fn line(points: &[Unit], at0: i64, at1: i64, dv: f64) -> Vec<Unit> {
    let v0 = value_at(points, at0);
    let v1 = (v0 + dv).clamp(0.0, 1.0);
    let (a, b) = if at0 <= at1 { ((at0, v0), (at1, v1)) } else { ((at1, v1), (at0, v0)) };
    let mut pts: Vec<Unit> = points.iter().copied().filter(|p| p.at < a.0 || p.at > b.0).collect();
    if !pts.iter().any(|p| p.at == 0) && a.0 > 0 {
        pts.push(Unit { at: 0, value: value_at(points, 0) });
    }
    pts.push(Unit { at: a.0, value: a.1 });
    if b.0 != a.0 {
        pts.push(Unit { at: b.0, value: b.1 });
    }
    pts.sort_by_key(|p| p.at);
    pts.dedup_by_key(|p| p.at);
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_draws_a_line_and_keeps_the_start() {
        let pts = line(&[], 100, 300, 0.5);
        assert_eq!(pts, vec![Unit { at: 0, value: 0.0 }, Unit { at: 100, value: 0.0 }, Unit { at: 300, value: 0.5 }]);
        assert_eq!(value_at(&pts, 200), 0.25);
        let again = line(&pts, 300, 400, 1.0);
        assert_eq!(again.last().copied(), Some(Unit { at: 400, value: 1.0 }));
    }

    #[test]
    fn the_curves_read_back_from_the_note() {
        let c = MpeCurves { pressure: vec![Unit { at: 0, value: 0.5 }], timbre: Vec::new() };
        let note = Value::object(c.fields());
        assert_eq!(MpeCurves::read(&note), c);
        assert!(c.changed(&c).is_empty());
    }
}
