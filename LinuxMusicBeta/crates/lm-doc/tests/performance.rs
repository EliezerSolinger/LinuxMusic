//! The 10,000-note measurement at the document's level (Parte 4, step 4; C1): an edit in a project of 64 tracks of
//! 10,000 notes each stays well under 1 ms (a whole transaction: rules, indices, diff, history step, journal), a
//! note dragged at 60 frames per second by a gesture too, and undoing a paste of 10,000 notes is fast.
//! `cargo test --release -p lm-doc --test performance -- --nocapture` prints the numbers.

mod common;

use std::time::{Duration, Instant};

use common::*;
use lm_doc::Document;
use lm_state::{NodeId, ROOT, Value, note_key};

const TRACKS: usize = 64;
const NOTES: usize = 10_000;

fn notes(rng: &mut Rng) -> Value {
    Value::object((0..NOTES).map(|i| {
        let note = Value::object([
            ("key", Value::int(rng.below(128) as i64)),
            ("start", Value::int((i * 120) as i64)),
            ("length", Value::int(120)),
            ("velocity", real(rng.unit())),
        ]);
        (note_key(i as u64), note)
    }))
}

fn stats(times: &mut [Duration]) -> (Duration, Duration, Duration) {
    times.sort_unstable();
    let mean = times.iter().sum::<Duration>() / times.len() as u32;
    (mean, times[times.len() / 2], times[times.len() - 1])
}

#[test]
#[allow(clippy::print_stdout, clippy::cast_precision_loss)]
fn edits_in_64_tracks_of_10000_notes() {
    let mut rng = Rng::new(42);
    let mut doc = new_doc();
    let built = Instant::now();
    let mut items: Vec<NodeId> = Vec::new();
    for t in 0..TRACKS {
        let notes = notes(&mut rng);
        let mut item = None;
        doc.transact(label("test.build"), |e| {
            let track = e.insert(ROOT, "tracks", t, Value::object([("instrument", synth())]))?;
            item = Some(e.insert(track, "items", 0, Value::object([("kind", text("midi")), ("length", Value::int(1_200_000)), ("notes", notes)]))?);
            Ok(())
        })
        .unwrap();
        items.push(item.unwrap());
    }
    let build_time = built.elapsed();
    let numbers: Vec<Vec<u64>> = items
        .iter()
        .map(|c| doc.find(*c).unwrap().get("notes").unwrap().keys().map(|k| k.parse().unwrap()).collect())
        .collect();
    assert!(numbers.iter().all(|n| n.len() == NOTES));

    // One transaction per edit: a note's key, in any item
    let mut times = Vec::new();
    for _ in 0..1000 {
        let i = rng.below(TRACKS);
        let n = numbers[i][rng.below(NOTES)];
        let key = Value::int(rng.below(128) as i64);
        let t0 = Instant::now();
        doc.transact(label("test.edit"), |e| e.set_note(items[i], n, &[k("key")], key)).unwrap();
        times.push(t0.elapsed());
    }
    let (edit_mean, edit_median, edit_max) = stats(&mut times);

    // A note dragged by a gesture, 60 updates (one second at 60 frames per second)
    let mut times = Vec::new();
    let g = doc.begin_gesture(label("test.drag"));
    let (item, n) = (items[7], numbers[7][5000]);
    for f in 0..60 {
        let t0 = Instant::now();
        doc.gesture_update(&g, |e| e.set_note(item, n, &[k("start")], Value::int(600_000 + f * 10))).unwrap();
        times.push(t0.elapsed());
    }
    doc.gesture_commit(g).unwrap();
    let (drag_mean, _, drag_max) = stats(&mut times);

    // An undo and a redo of one edit
    let t0 = Instant::now();
    doc.undo().unwrap();
    doc.redo().unwrap();
    let undo_redo = t0.elapsed();

    // Paste an item of 10,000 notes, then undo and redo it
    let clip = doc.copy(items[0]).unwrap();
    let t0 = Instant::now();
    let track = doc.loc(items[0]).unwrap().parent;
    doc.transact(label("test.paste"), |e| e.paste(track, "items", 0, &clip).map(|_| ())).unwrap();
    let paste = t0.elapsed();
    let t0 = Instant::now();
    let undo = doc.undo().unwrap();
    let paste_undo = t0.elapsed();
    let t0 = Instant::now();
    doc.redo().unwrap();
    let paste_redo = t0.elapsed();
    assert_eq!(undo.removed.len(), 1);
    doc.verify_index().unwrap();

    // Paste 10,000 notes into an existing item (a heavy edit's patch), then undo and redo it
    let base = doc.find(items[1]).unwrap();
    let ops = (0..NOTES)
        .map(|i| lm_doc::PatchOp::AddNote { item: items[1], note: Value::object([("key", Value::int((i % 128) as i64)), ("start", Value::int(i as i64))]) })
        .collect();
    let patch = lm_doc::Patch { ops, based_on: vec![(items[1], base)] };
    let t0 = Instant::now();
    doc.apply_patch(label("test.paste_notes"), &patch).unwrap();
    let notes_paste = t0.elapsed();
    let t0 = Instant::now();
    let undo = doc.undo().unwrap();
    let notes_undo = t0.elapsed();
    let t0 = Instant::now();
    doc.redo().unwrap();
    let notes_redo = t0.elapsed();
    assert_eq!(undo.changed_node(items[1]).unwrap().notes.removed.len(), NOTES);
    assert_eq!(doc.find(items[1]).unwrap().get("notes").unwrap().len(), 2 * NOTES);

    println!("--- lm-doc: 64 tracks x 10,000 notes ---");
    println!("build (64 transactions of one 10,000-note item): {build_time:?}");
    println!("edit (transaction of one note): mean {edit_mean:?}, median {edit_median:?}, max {edit_max:?}");
    println!("drag (gesture update of one note, 60 frames): mean {drag_mean:?}, max {drag_max:?}");
    println!("undo + redo of one edit: {undo_redo:?}");
    println!("paste of a 10,000-note item: {paste:?}; its undo {paste_undo:?}; its redo {paste_redo:?}");
    println!("paste of 10,000 notes into an item: {notes_paste:?}; its undo {notes_undo:?}; its redo {notes_redo:?}");
    // Generous bounds, valid in debug too; the numbers above are the measurement
    assert!(edit_median < Duration::from_millis(1), "an edit takes {edit_median:?}");
    assert!(drag_mean < Duration::from_millis(1), "a drag frame takes {drag_mean:?}");
    assert!(paste_undo < Duration::from_millis(5), "undo of the paste takes {paste_undo:?}");
    assert!(notes_undo < Duration::from_millis(50), "undo of 10,000 notes takes {notes_undo:?}");
}

#[test]
fn the_document_is_not_send_but_its_snapshot_is() {
    fn send<T: Send + Sync>(_: &T) {}
    let doc: Document = new_doc();
    send(&doc.snapshot());
}
