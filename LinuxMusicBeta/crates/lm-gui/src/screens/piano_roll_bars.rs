//! The Piano Roll's bars by the tempo map (F-NOT-013): where each bar of the song falls on the clip's own time (ticks
//! from the clip's start), with the bar's length and its beat by the time signature there. A signature change
//! (3/4, 6/8, 7/8...) shows in the grid and the ruler as it does in the arrangement.

use lm_port::Value;

use crate::model;

/// One bar, in the clip's ticks
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    /// From the clip's start (negative before it)
    pub start: i64,
    /// The song's bar number, from 1
    pub number: i64,
    pub len: i64,
    /// The beat's length in this bar
    pub beat: i64,
}

/// The bars that touch `from..to` (clip ticks) for a clip starting at `clip_start` (song ticks)
pub fn bars(root: &Value, clip_start: i64, from: f64, to: f64) -> Vec<Bar> {
    let map = model::tempo_map(root);
    let ppq = lm_time::Ppq::new(model::ppq(root)).unwrap_or(lm_time::Ppq::DEFAULT);
    let first = clip_start + from.floor() as i64;
    let mut number = map.bar_beat_tick(first.max(0)).bar;
    let mut out = Vec::new();
    for _ in 0..20_000 {
        let Ok(abs) = map.bar_start(number) else { break };
        let end = map.bar_start(number + 1).unwrap_or(abs + 4 * i64::from(ppq.get()));
        let sig = map.time_signature_at(abs);
        let len = (end - abs).max(1);
        let bar = Bar { start: abs - clip_start, number, len, beat: sig.beat_ticks(ppq).unwrap_or(len).max(1) };
        if (bar.start as f64) > to {
            break;
        }
        if ((bar.start + bar.len) as f64) >= from {
            out.push(bar);
        }
        number += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(sig: [i64; 2]) -> Value {
        let point = Value::object([
            ("tick", Value::int(0)),
            ("bpm", Value::text("120")),
            ("timeSig", Value::list([Value::int(sig[0]), Value::int(sig[1])])),
            ("ramp", Value::bool(false)),
        ]);
        Value::object([("tempoMap", Value::list([point]))])
    }

    #[test]
    fn bars_follow_the_signature() {
        let b = bars(&root([3, 4]), 0, 0.0, 6000.0);
        assert_eq!((b[0].start, b[0].len, b[0].beat), (0, 2880, 960));
        assert_eq!(b[1].start, 2880);
        let b = bars(&root([6, 8]), 0, 0.0, 6000.0);
        assert_eq!((b[0].len, b[0].beat), (2880, 480));
    }

    #[test]
    fn a_clip_inside_a_bar_sees_that_bar_first() {
        let b = bars(&root([4, 4]), 1000, 0.0, 5000.0);
        assert_eq!(b[0].number, 1);
        assert_eq!(b[0].start, -1000);
    }
}
