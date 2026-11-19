//! Gestures (I-3.7, T4, G1–G3) and feeds (I-3.6, T13)

mod common;

use common::*;
use lm_doc::{Document, Origin, Topic};
use lm_state::{NodeId, ROOT, Value};

fn volume(doc: &Document, id: NodeId) -> f64 {
    doc.find(id).unwrap().get("volume").unwrap().as_real().unwrap()
}

fn setup() -> (Document, NodeId, NodeId) {
    let mut doc = new_doc();
    let a = add_channel(&mut doc, "a");
    let b = add_channel(&mut doc, "b");
    (doc, a, b)
}

#[test]
fn a_gesture_is_one_step_at_commit() {
    let (mut doc, a, _) = setup();
    let steps = doc.history_len();
    let g = doc.begin_gesture(label("test.drag"));
    for v in [0.1, 0.2, 0.3] {
        let d = doc.gesture_update(&g, |e| e.set_param(a, "volume", v)).unwrap();
        assert_eq!(d.origin, Origin::Gesture);
        assert_eq!(d.params, vec![(a, std::sync::Arc::from("volume"), v)]);
    }
    assert_eq!(doc.history_len(), steps, "no step while open");
    let d = doc.gesture_commit(g).unwrap();
    assert!(d.changed_node(a).is_some());
    assert_eq!(doc.history_len(), steps + 1);
    doc.undo().unwrap();
    assert_eq!(volume(&doc, a), 1.0);
    doc.redo().unwrap();
    assert_eq!(volume(&doc, a), 0.3);
}

#[test]
fn a_foreign_transaction_in_the_middle_is_its_own_step_with_the_root_as_it_was() {
    let (mut doc, a, b) = setup();
    let g = doc.begin_gesture(label("test.drag"));
    doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.5)).unwrap();
    doc.transact(label("test.foreign"), |e| e.set_param(b, "pan", -1.0)).unwrap();
    doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.7)).unwrap();
    doc.gesture_commit(g).unwrap();
    // Undo the gesture's step: back to the state when the foreign step was taken (the gesture at 0.5)
    doc.undo().unwrap();
    assert_eq!(volume(&doc, a), 0.5);
    assert_eq!(doc.param(b, "pan"), Some(-1.0));
    // Undo the foreign step: back to before both
    doc.undo().unwrap();
    assert_eq!(volume(&doc, a), 1.0);
    assert_eq!(doc.param(b, "pan"), Some(0.0));
}

#[test]
fn two_alternating_gestures_give_two_steps() {
    let (mut doc, a, b) = setup();
    let steps = doc.history_len();
    let g1 = doc.begin_gesture(label("test.g1"));
    let g2 = doc.begin_gesture(label("test.g2"));
    doc.gesture_update(&g1, |e| e.set_param(a, "volume", 0.5)).unwrap();
    doc.gesture_update(&g2, |e| e.set_param(b, "volume", 0.6)).unwrap();
    doc.gesture_update(&g1, |e| e.set_param(a, "volume", 0.4)).unwrap();
    doc.gesture_commit(g1).unwrap();
    doc.gesture_update(&g2, |e| e.set_param(b, "volume", 0.2)).unwrap();
    doc.gesture_commit(g2).unwrap();
    assert_eq!(doc.history_len(), steps + 2);
    doc.undo().unwrap();
    assert_eq!((volume(&doc, a), volume(&doc, b)), (0.4, 0.6));
}

#[test]
fn cancel_restores_only_the_gestures_paths_and_keeps_the_foreign_change_g1() {
    let (mut doc, a, b) = setup();
    let steps = doc.history_len();
    let g = doc.begin_gesture(label("test.drag"));
    doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.5)).unwrap();
    doc.transact(label("test.foreign"), |e| e.set_param(b, "pan", 0.5)).unwrap();
    doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.25)).unwrap();
    let d = doc.gesture_cancel(g).unwrap();
    assert_eq!(volume(&doc, a), 1.0);
    assert_eq!(doc.param(b, "pan"), Some(0.5), "the foreign change stays");
    assert_eq!(d.params, vec![(a, std::sync::Arc::from("volume"), 1.0)]);
    assert_eq!(doc.history_len(), steps + 2, "the foreign step and the cancel step");
    assert_eq!(doc.undo_label().unwrap().key, "doc.gesture.cancel");
    doc.verify_index().unwrap();
}

#[test]
fn cancel_without_foreign_steps_is_no_step_and_exact() {
    let (mut doc, a, b) = setup();
    let before = doc.root().clone();
    let steps = doc.history_len();
    let g = doc.begin_gesture(label("test.drag"));
    let h = doc.begin_gesture(label("test.other"));
    doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.5)).unwrap();
    doc.gesture_update(&h, |e| e.set_param(b, "volume", 0.5)).unwrap();
    // Another gesture is open: only this one's paths go back
    doc.gesture_cancel(g).unwrap();
    assert_eq!((volume(&doc, a), volume(&doc, b)), (1.0, 0.5));
    // The last one: back to the last step exactly
    doc.gesture_cancel(h).unwrap();
    assert_eq!(doc.root(), &before);
    assert_eq!(doc.history_len(), steps);
    // Moves are put back too
    let ts: Vec<NodeId> = (0..3).map(|i| add_track(&mut doc, &format!("{i}"), None)).collect();
    let g = doc.begin_gesture(label("test.drag"));
    doc.gesture_update(&g, |e| e.move_to(ts[0], ROOT, "tracks", 2)).unwrap();
    doc.transact(label("test.foreign"), |e| e.set(ts[1], &[k("name")], text("x"))).unwrap();
    doc.gesture_cancel(g).unwrap();
    assert_eq!(doc.tracks().iter().map(|t| t.id()).collect::<Vec<_>>(), ts);
    assert_eq!(doc.find(ts[1]).unwrap().get("name"), Some(&text("x")));
}

#[test]
fn undo_with_an_open_gesture_commits_and_undoes_it_first_g2() {
    let (mut doc, a, _) = setup();
    let g = doc.begin_gesture(label("test.drag"));
    doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.5)).unwrap();
    doc.undo().unwrap();
    assert_eq!(volume(&doc, a), 1.0, "the first undo undoes the drag");
    assert_eq!(doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.1)).unwrap_err().key(), "doc.gesture.closed");
    assert_eq!(doc.gesture_commit(g).unwrap_err().key(), "doc.gesture.closed");
    doc.redo().unwrap();
    assert_eq!(volume(&doc, a), 0.5);
}

#[test]
fn a_removed_target_ends_the_gesture_without_a_step_g3() {
    let (mut doc, a, _) = setup();
    let g = doc.begin_gesture(label("test.drag"));
    doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.5)).unwrap();
    doc.transact(label("test.remove"), |e| e.remove(a)).unwrap();
    let steps = doc.history_len();
    assert_eq!(doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.1)).unwrap_err().key(), "doc.gesture.target_gone");
    assert_eq!(doc.history_len(), steps);
    assert_eq!(doc.gesture_commit(g).unwrap_err().key(), "doc.gesture.closed");
}

#[test]
fn gestures_change_values_not_structure() {
    let (mut doc, a, _) = setup();
    let g = doc.begin_gesture(label("test.drag"));
    let e = doc.gesture_update(&g, |e| e.insert(a, "sends", 0, Value::object([("to", id_text(lm_doc::MASTER))])).map(|_| ())).unwrap_err();
    assert_eq!(e.key(), "doc.gesture.structural");
    let e = doc.gesture_update(&g, |e| e.remove(a)).unwrap_err();
    assert_eq!(e.key(), "doc.gesture.structural");
    doc.gesture_commit(g).unwrap();
}

#[test]
fn a_dropped_gesture_is_committed_at_the_next_delivery() {
    let (mut doc, a, _) = setup();
    let steps = doc.history_len();
    {
        let g = doc.begin_gesture(label("test.drag"));
        doc.gesture_update(&g, |e| e.set_param(a, "volume", 0.5)).unwrap();
        drop(g);
    }
    assert_eq!(doc.open_gestures(), 1);
    let delivered = doc.deliver();
    assert_eq!(delivered.committed.len(), 1);
    assert!(delivered.committed[0].1.is_ok());
    assert_eq!(doc.open_gestures(), 0);
    assert_eq!(doc.history_len(), steps + 1);
}

#[test]
fn feeds_merge_by_final_state_t13() {
    let (mut doc, a, b) = setup();
    let mut all = doc.feed(Topic::All);
    let mut only_a = doc.feed(Topic::Node(a));
    let mut tracks = doc.feed(Topic::Type("track"));
    // Created and removed between two drains: not there
    let t = add_track(&mut doc, "gone", None);
    doc.transact(label("x"), |e| e.remove(t)).unwrap();
    // Changed several times: once, with the union of the paths
    doc.transact(label("x"), |e| e.set_param(a, "volume", 0.5)).unwrap();
    doc.transact(label("x"), |e| e.set_param(a, "pan", 0.5)).unwrap();
    doc.transact(label("x"), |e| e.set_param(a, "volume", 0.25)).unwrap();
    // Changed then removed: removed
    doc.transact(label("x"), |e| e.set_param(b, "volume", 0.5)).unwrap();
    doc.transact(label("x"), |e| e.remove(b)).unwrap();
    let d = all.drain(&doc).unwrap();
    assert!(!d.touches(t));
    assert_eq!(d.removed, [b]);
    assert_eq!(d.changed_node(a).unwrap().paths, vec![vec![k("pan")], vec![k("volume")]]);
    assert_eq!(d.params, vec![(a, std::sync::Arc::from("pan"), 0.5), (a, std::sync::Arc::from("volume"), 0.25)]);
    assert!(all.drain(&doc).is_none(), "drained");
    let d = only_a.drain(&doc).unwrap();
    assert_eq!(d.changed.len(), 1);
    assert!(d.removed.is_empty() && d.created.is_empty());
    assert!(tracks.drain(&doc).is_none(), "the track came and went");
    // A dropped feed stops gathering
    assert_eq!(doc.feed_count(), 3);
    drop(only_a);
    doc.transact(label("x"), |e| e.set_param(a, "volume", 0.1)).unwrap();
    assert_eq!(doc.feed_count(), 2);
    // Undo through a feed: origin and the old values
    all.drain(&doc);
    doc.undo().unwrap();
    let d = all.drain(&doc).unwrap();
    assert_eq!(d.origin, Origin::Undo);
    assert_eq!(d.params, vec![(a, std::sync::Arc::from("volume"), 0.25)]);
    // A track created shows in the type feed
    let t2 = add_track(&mut doc, "kept", None);
    let d = tracks.drain(&doc).unwrap();
    assert_eq!(d.created, [t2]);
}

#[test]
fn feed_of_notes_merges_by_number() {
    let mut doc = new_doc();
    let t = add_track(&mut doc, "a", Some(synth()));
    let c = add_item(&mut doc, t, "midi");
    let mut feed = doc.feed(Topic::Node(c));
    let mut n = 0;
    doc.transact(label("x"), |e| {
        n = e.add_note(c, Value::empty_object())?;
        Ok(())
    })
    .unwrap();
    let mut m = 0;
    doc.transact(label("x"), |e| {
        m = e.add_note(c, Value::empty_object())?;
        Ok(())
    })
    .unwrap();
    doc.transact(label("x"), |e| e.set_note(c, m, &[k("key")], Value::int(1))).unwrap();
    doc.transact(label("x"), |e| e.remove_note(c, n)).unwrap();
    let d = feed.drain(&doc).unwrap();
    let notes = &d.changed_node(c).unwrap().notes;
    assert_eq!(notes.created, [m]);
    assert!(notes.changed.is_empty() && notes.removed.is_empty());
}

#[test]
fn the_journal_carries_gestures_cancels_and_undo() {
    let (mut doc, a, b) = setup();
    let _ = doc.take_journal();
    let base = doc.root().clone();
    let mut patches = Vec::new();
    for round in 0..20 {
        let v = f64::from(round) / 40.0;
        let g = doc.begin_gesture(label("test.drag"));
        doc.gesture_update(&g, |e| e.set_param(a, "volume", v)).unwrap();
        patches.extend(doc.take_journal());
        if round % 3 == 0 {
            doc.transact(label("test.foreign"), |e| e.set_param(b, "pan", v - 0.25)).unwrap();
        }
        doc.gesture_update(&g, |e| e.set_param(a, "volume", v + 0.1)).unwrap();
        match round % 4 {
            0 => {
                doc.gesture_cancel(g).unwrap();
            }
            1 => {
                doc.gesture_commit(g).unwrap();
            }
            2 => {
                doc.undo().unwrap();
                drop(g);
            }
            _ => {
                let h = doc.begin_gesture(label("test.other"));
                doc.gesture_update(&h, |e| e.set_param(b, "volume", v)).unwrap();
                doc.gesture_cancel(g).unwrap();
                doc.gesture_commit(h).unwrap();
            }
        }
        doc.deliver();
        doc.verify_index().unwrap();
        patches.extend(doc.take_journal());
    }
    assert_eq!(lm_doc::replay(&base, &patches).unwrap(), *doc.root());
}
