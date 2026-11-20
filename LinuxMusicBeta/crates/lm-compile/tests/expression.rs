//! Note pan and bend (note expressions), the MIDI item's loop and swing: rendered through the Dummy, every event at
//! its exact sample (120 BPM at 960 PPQ and 48 kHz: 25 samples a tick)

mod common;

use common::*;
use lm_render::{EventKind, Expression};
use lm_state::Value;

/// Renders `doc` from the start and gives what the probe heard
fn heard(doc: &lm_doc::Document, samples: usize) -> Vec<(i64, EventKind)> {
    let plugins = plugins();
    let mut h = Harness::new(&plugins);
    h.settle(doc.root());
    PROBE.lock().unwrap().clear();
    h.play();
    h.render(samples / BLOCK + 2);
    std::mem::take(&mut *PROBE.lock().unwrap())
}

fn ons(events: &[(i64, EventKind)]) -> Vec<i64> {
    events.iter().filter(|(_, e)| matches!(e, EventKind::NoteOn { .. })).map(|(s, _)| *s).collect()
}

fn expressions(events: &[(i64, EventKind)]) -> Vec<(i64, Expression, f32, u32)> {
    events
        .iter()
        .filter_map(|(s, e)| match e {
            EventKind::NoteExpression { expression, value, ramp, .. } => Some((*s, *expression, *value, *ramp)),
            _ => None,
        })
        .collect()
}

fn project(item: Value, notes: Vec<Value>) -> lm_doc::Document {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    digital(&mut doc);
    let t = add_track(&mut doc, Some(plugin("probe", Value::empty_object())));
    let it = add_item(&mut doc, t, "midi", 0, 3840);
    edit(&mut doc, |e| {
        for (k, v) in item.entries() {
            e.set(it, &[k_(k)], v.clone())?;
        }
        for n in notes {
            e.add_note(it, n)?;
        }
        Ok(())
    });
    doc
}

fn k_(key: &str) -> lm_state::PathStep {
    lm_state::PathStep::key(key)
}

// The tests share the probe's record: one test, in order
#[test]
fn note_expressions_loop_and_swing_at_exact_samples() {
    // Pan: at the note's start, after its note-on, at once
    let doc = project(Value::empty_object(), vec![note(60, 480, 240, 0.8).with("pan", real(0.5))]);
    let e = heard(&doc, 20_000);
    assert_eq!(ons(&e), [12_000]);
    assert_eq!(expressions(&e), [(12_000, Expression::Pan, 0.5, 0)]);
    let on = e.iter().position(|(_, k)| matches!(k, EventKind::NoteOn { .. })).unwrap();
    let ex = e.iter().position(|(_, k)| matches!(k, EventKind::NoteExpression { .. })).unwrap();
    assert!(on < ex, "the expression after its note-on");

    // Bend: the first point at once, then one ramp per segment, in samples through the tempo map; cut at the end
    let bend = Value::list(
        [(0, 0.0), (480, 2.0), (960, -1.0), (5000, 7.0)]
            .iter()
            .map(|(at, st)| Value::object([("at", Value::int(*at)), ("semitones", real(*st))])),
    );
    let doc = project(Value::empty_object(), vec![note(60, 0, 1920, 0.8).with("bend", bend)]);
    let e = heard(&doc, 30_000);
    assert_eq!(
        expressions(&e),
        [
            (0, Expression::Tuning, 0.0, 0),
            (0, Expression::Tuning, 2.0, 12_000),
            (12_000, Expression::Tuning, -1.0, 12_000),
            (24_000, Expression::Tuning, 7.0, 4040 * 25),
        ]
    );

    // Absent fields keep the old meaning: no expression at all
    let doc = project(Value::empty_object(), vec![note(60, 0, 240, 0.8)]);
    assert!(expressions(&heard(&doc, 10_000)).is_empty());

    // Loop: the first 960 ticks repeat until the item's 3840 end; a note past the loop is not played
    let doc = project(
        Value::object([("loop", Value::int(960))]),
        vec![note(60, 0, 240, 0.8), note(62, 600, 600, 0.8), note(64, 1200, 100, 0.8)],
    );
    let e = heard(&doc, 100_000);
    assert_eq!(ons(&e), [0, 15_000, 24_000, 39_000, 48_000, 63_000, 72_000, 87_000]);
    // The note across the item's end is cut there: its last note-off at 3840 ticks
    let last_off = e.iter().filter(|(_, k)| matches!(k, EventKind::NoteOff { .. })).map(|(s, _)| *s).max().unwrap();
    assert_eq!(last_off, 3840 * 25);

    // Swing: every second step (of 240) delayed by swing × step / 2 = 60 ticks at 0.5
    let doc = project(
        Value::object([("swing", real(0.5)), ("step", Value::int(240))]),
        vec![note(60, 0, 100, 0.8), note(61, 240, 100, 0.8), note(62, 480, 100, 0.8), note(63, 720 + 30, 100, 0.8)],
    );
    let e = heard(&doc, 25_000);
    assert_eq!(ons(&e), [0, 300 * 25, 480 * 25, (750 + 60) * 25]);
}
