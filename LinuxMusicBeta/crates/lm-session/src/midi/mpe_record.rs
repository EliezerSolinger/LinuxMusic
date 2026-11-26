//! MPE recording (`decisoes-autonomas.md`, 26/11, item 5): on a take of an MPE port, the pitch bend, the channel
//! pressure and the CC 74 of a member channel are not control lanes but the curves of the note sounding on that
//! channel (`bend`, `pressure`, `timbre` of the note). A value sent before the note-on (the specification asks for
//! the bend first) is the note's first point.

use std::collections::BTreeMap;

use lm_render::mpe::MpePort;
use lm_state::Value;

use super::mpe::MpeInfo;

/// A note's curves: (ticks from the note's start, value). The bend in semitones, the others 0..1
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Curves {
    pub bend: Vec<(i64, f64)>,
    pub pressure: Vec<(i64, f64)>,
    pub timbre: Vec<(i64, f64)>,
}

impl Curves {
    pub(crate) fn is_empty(&self) -> bool {
        self.bend.is_empty() && self.pressure.is_empty() && self.timbre.is_empty()
    }
}

/// Follows the member channels of one take
#[derive(Debug)]
pub(crate) struct MpeTracker {
    port: MpePort,
    /// By channel: the last bend (semitones), pressure and timbre seen
    last: [[Option<f64>; 3]; 16],
    /// By channel: the note that sounds, its start tick and its curves so far
    live: BTreeMap<u8, (u8, i64, Curves)>,
}

impl MpeTracker {
    pub(crate) fn new(info: &MpeInfo) -> Self {
        Self { port: info.port(), last: [[None; 3]; 16], live: BTreeMap::new() }
    }

    /// A note starts on `channel` at `tick`: what the channel sent before is its first point
    pub(crate) fn note_on(&mut self, channel: u8, key: u8, tick: i64) {
        if !self.port.is_member(channel) {
            return;
        }
        let v = self.last[usize::from(channel & 0x0F)];
        let mut c = Curves::default();
        if let Some(b) = v[0] {
            c.bend.push((0, b));
        }
        if let Some(p) = v[1] {
            c.pressure.push((0, p));
        }
        if let Some(t) = v[2] {
            c.timbre.push((0, t));
        }
        self.live.insert(channel, (key, tick, c));
    }

    /// A channel message: `true` when it is a member channel's bend, pressure or CC 74 (taken here), `false` when it
    /// is for the control lanes
    pub(crate) fn message(&mut self, bytes: [u8; 3], tick: i64) -> bool {
        let channel = bytes[0] & 0x0F;
        if !self.port.is_member(channel) {
            return false;
        }
        let Some((expression, value)) = self.port.expression(bytes) else { return false };
        let slot = match expression {
            lm_render::Expression::Tuning => 0,
            lm_render::Expression::Pressure => 1,
            _ => 2,
        };
        let value = f64::from(value);
        self.last[usize::from(channel)][slot] = Some(value);
        if let Some((_, start, c)) = self.live.get_mut(&channel) {
            let at = (tick - *start).max(0);
            let list = match slot {
                0 => &mut c.bend,
                1 => &mut c.pressure,
                _ => &mut c.timbre,
            };
            // The same tick keeps the last value
            if list.last().is_some_and(|p| p.0 == at) {
                list.pop();
            }
            list.push((at, value));
        }
        true
    }

    /// The note of `key` on `channel` ended: its curves
    pub(crate) fn note_off(&mut self, channel: u8, key: u8) -> Curves {
        match self.live.get(&channel) {
            Some((k, _, _)) if *k == key => self.live.remove(&channel).map(|(_, _, c)| c).unwrap_or_default(),
            _ => Curves::default(),
        }
    }
}

/// A note's value with its curves as the note schema keeps them (`bend` in semitones, `pressure` and `timbre` 0..1;
/// a curve with no points is left out)
pub(crate) fn with_curves(note: Value, curves: Option<&Curves>) -> lm_base::Result<Value> {
    let Some(c) = curves else { return Ok(note) };
    let list = |points: &[(i64, f64)], key: &str, lo: f64, hi: f64| -> lm_base::Result<Option<Value>> {
        if points.is_empty() {
            return Ok(None);
        }
        let items = points
            .iter()
            .map(|(at, v)| Ok(Value::object([("at", Value::int(*at)), (key, Value::real(v.clamp(lo, hi))?)])))
            .collect::<lm_base::Result<Vec<_>>>()?;
        Ok(Some(Value::list(items)))
    };
    let mut note = note;
    if let Some(l) = list(&c.bend, "semitones", -24.0, 24.0)? {
        note = note.with("bend", l);
    }
    if let Some(l) = list(&c.pressure, "value", 0.0, 1.0)? {
        note = note.with("pressure", l);
    }
    if let Some(l) = list(&c.timbre, "value", 0.0, 1.0)? {
        note = note.with("timbre", l);
    }
    Ok(note)
}

#[cfg(test)]
mod tests {
    use lm_render::mpe::Zone;

    use super::*;

    fn tracker() -> MpeTracker {
        MpeTracker::new(&MpeInfo { zone: Zone::Lower, members: 15, bend_semitones: 48.0 })
    }

    #[test]
    fn a_note_collects_the_curves_of_its_channel() {
        let mut t = tracker();
        // The bend comes before the note-on: it is the first point
        assert!(t.message([0xE2, 0x00, 0x60], 0));
        t.note_on(2, 60, 10);
        assert!(t.message([0xD2, 100, 0], 30));
        assert!(t.message([0xB2, 74, 127], 40));
        assert!(t.message([0xB2, 74, 0], 40), "the same tick keeps the last value");
        let c = t.note_off(2, 60);
        assert_eq!(c.bend.len(), 1);
        assert_eq!(c.bend[0].0, 0);
        assert!((c.bend[0].1 - 12.0).abs() < 1e-3, "a quarter of the range up: {}", c.bend[0].1);
        assert_eq!(c.pressure, vec![(20, 100.0 / 127.0)]);
        assert_eq!(c.timbre, vec![(30, 0.0)]);
        assert_eq!(t.note_off(2, 60), Curves::default());
    }

    #[test]
    fn the_master_channel_and_other_controllers_are_not_taken() {
        let mut t = tracker();
        assert!(!t.message([0xE0, 0, 0x40], 0), "the master channel's bend is a control lane");
        assert!(!t.message([0xB3, 7, 100], 0), "another controller is a control lane");
    }
}
