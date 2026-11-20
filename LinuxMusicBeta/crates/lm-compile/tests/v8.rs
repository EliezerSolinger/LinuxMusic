//! V8: in a project of 64 tracks × 10,000 notes, moving one note compiles in less than 5 ms (release) and makes only
//! that item's event list; every other item's list is the same `Arc` in the new program.
//!
//! `cargo test --release -p lm-compile --test v8 -- --nocapture` prints the numbers.

// The numbers are the point of this test
#![allow(clippy::print_stdout)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use lm_compile::{CompilerCore, Environment, NoResources};
use lm_state::{PathStep, Value, note_key};

const TRACKS: usize = 64;
const NOTES: usize = 10_000;

fn project(plugins: &std::sync::Arc<lm_plugin::Registry>) -> Value {
    let doc = new_doc(plugins);
    let mut id = 1000u64;
    let mut next = |kind: char| {
        id += 1;
        Value::text(format!("{kind}:{id}"))
    };
    let mut tracks = Vec::new();
    for t in 0..TRACKS {
        let notes = Value::object((0..NOTES).map(|i| {
            let n = (t * NOTES + i) as u64 + 1;
            (note_key(n), note((36 + (i % 48)) as i64, (i as i64) * 240, 200, 0.8))
        }));
        let item = Value::object([
            ("id", next('c')),
            ("kind", text("midi")),
            ("start", Value::int(0)),
            ("length", Value::int(NOTES as i64 * 240)),
            ("notes", notes),
        ]);
        let instrument = dc().with("id", next('p'));
        let output = Value::object([
            ("id", next('o')),
            ("port", text("main")),
            ("channels", Value::int(2)),
            ("to", text("m:0")),
        ]);
        tracks.push(Value::object([
            ("id", next('t')),
            ("instrument", instrument),
            ("outputs", Value::list([output])),
            ("items", Value::list([item])),
        ]));
    }
    doc.root().with("tracks", Value::list(tracks))
}

#[test]
fn moving_one_note_in_64_tracks_of_10000_notes() {
    let plugins = plugins();
    let root = project(&plugins);
    let mut core = CompilerCore::new(config(&plugins));
    let env = Environment::default();
    let t0 = Instant::now();
    let first = core.compile(&root, &NoResources, &env).expect("compile");
    let full = t0.elapsed();
    assert_eq!(first.made.midi_items, TRACKS);

    // One note moves, 100 times, in a different track each time (the tree shares everything else)
    let mut root = root;
    let mut times: Vec<Duration> = Vec::new();
    for step in 0..100 {
        let t = step % TRACKS;
        let n = (t * NOTES + 17) as u64 + 1;
        let path = [
            PathStep::key("tracks"),
            PathStep::Index(t),
            PathStep::key("items"),
            PathStep::Index(0),
            PathStep::key("notes"),
            PathStep::Note(n),
            PathStep::key("start"),
        ];
        root = root.with_path(&path, Value::int(17 * 240 + 10 + step as i64)).expect("edit");
        let t1 = Instant::now();
        let c = core.compile(&root, &NoResources, &env).expect("compile");
        times.push(t1.elapsed());
        assert_eq!(c.made.midi_items, 1, "only the moved note's item is made again");
        assert_eq!(c.made.audio_items + c.made.curves, 0);
        assert!(c.builds.is_empty());
        // Returned at once, as the audio side would
        core.returned(&lm_render::Returned { generation: c.generation, ..lm_render::Returned::default() });
    }
    times.sort();
    let median = times[times.len() / 2];
    let worst = *times.last().expect("times");
    println!(
        "V8: 64 tracks × 10,000 notes — first compilation {:.2} ms; moving one note: median {:.3} ms, worst {:.3} ms",
        full.as_secs_f64() * 1e3,
        median.as_secs_f64() * 1e3,
        worst.as_secs_f64() * 1e3
    );
    // The budget (5 ms) is asserted where it is measured; shared CI runners (`CI` set) are slower and noisier, so
    // there only a ceiling of three times the budget holds — still far below a real regression (the quadratic event
    // list this test caught took 17 ms) — and the measure is printed (decisoes-autonomas.md)
    if !cfg!(debug_assertions) {
        let budget =
            if std::env::var_os("CI").is_some() { Duration::from_millis(15) } else { Duration::from_millis(5) };
        assert!(median < budget, "median {median:?} (budget {budget:?})");
    }
}
