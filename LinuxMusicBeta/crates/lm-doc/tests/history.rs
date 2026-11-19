//! Random transactions (Parte 4, step 4): undo all gives back the original root bit for bit, redo all the final
//! one; the indices equal the ones rebuilt from scratch after every step; the journal replayed onto the saved root
//! gives the live root

mod common;

use common::*;
use lm_doc::{Document, Edit, JournalPatch, replay};
use lm_state::{NodeId, PathStep, ROOT, Value, from_json, to_json};

fn ids_of(doc_root: &Value, path: &[PathStep]) -> Vec<NodeId> {
    doc_root
        .at_path(path)
        .map(|l| {
            l.items()
                .filter_map(|i| i.get("id").and_then(Value::as_text))
                .filter_map(|t| NodeId::parse(t).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn tracks(e: &Edit<'_>) -> Vec<NodeId> {
    ids_of(e.root(), &[k("tracks")])
}

fn channels(e: &Edit<'_>) -> Vec<NodeId> {
    ids_of(e.root(), &[k("mixer"), k("channels")])
}

fn items(e: &Edit<'_>) -> Vec<NodeId> {
    tracks(e).into_iter().flat_map(|t| ids_of(e.find(t).unwrap(), &[k("items")])).collect()
}

/// One random change; errors are allowed (the transaction is then refused whole)
fn random_op(e: &mut Edit<'_>, rng: &mut Rng) -> lm_base::Result<()> {
    let ts = tracks(e);
    let cs = channels(e);
    let midi: Vec<NodeId> = items(e)
        .into_iter()
        .filter(|c| e.find(*c).and_then(|i| i.get("kind")).and_then(Value::as_text) == Some("midi"))
        .collect();
    match rng.below(16) {
        0 => {
            let instrument = match rng.below(3) {
                0 => Value::NULL,
                1 => synth(),
                _ => drums(KITS[rng.below(2)]),
            };
            e.insert(
                ROOT,
                "tracks",
                rng.below(ts.len() + 1),
                Value::object([("name", text("t")), ("instrument", instrument)]),
            )?;
        }
        1 => {
            if let Some(t) = rng.pick(&ts) {
                e.remove(*t)?;
            }
        }
        2 => {
            if let Some(t) = rng.pick(&ts) {
                e.move_to(*t, ROOT, "tracks", rng.below(ts.len()))?;
            }
        }
        3 => {
            e.insert(ROOT, "mixer.channels", rng.below(cs.len() + 1), Value::object([("name", text("c"))]))?;
        }
        4 => {
            if let Some(c) = rng.pick(&cs) {
                e.remove(*c)?;
            }
        }
        5 => {
            if let (Some(a), Some(b)) = (rng.pick(&cs), rng.pick(&cs)) {
                e.set(*a, &[k("output")], id_text(*b))?;
            }
        }
        6 => {
            if let (Some(a), Some(b)) = (rng.pick(&cs), rng.pick(&cs)) {
                e.insert(*a, "sends", 0, Value::object([("to", id_text(*b)), ("amount", real(rng.unit()))]))?;
            }
        }
        7 => {
            if let (Some(t), Some(c)) = (rng.pick(&ts), rng.pick(&cs)) {
                e.set(*t, &[k("outputs"), PathStep::Index(0), k("to")], id_text(*c))?;
            }
        }
        8 => {
            if let Some(t) = rng.pick(&ts) {
                e.insert(
                    *t,
                    "items",
                    0,
                    Value::object([("kind", text("midi")), ("start", Value::int(rng.below(10_000) as i64))]),
                )?;
            }
        }
        9 => {
            if let Some(c) = rng.pick(&midi) {
                for _ in 0..1 + rng.below(4) {
                    e.add_note(
                        *c,
                        Value::object([
                            ("key", Value::int(rng.below(128) as i64)),
                            ("start", Value::int(rng.below(3840) as i64)),
                        ]),
                    )?;
                }
            }
        }
        10 => {
            if let Some(c) = rng.pick(&midi) {
                let notes: Vec<u64> =
                    e.find(*c).unwrap().get("notes").unwrap().keys().filter_map(|k| k.parse().ok()).collect();
                if let Some(n) = rng.pick(&notes) {
                    if rng.chance(2) {
                        e.remove_note(*c, *n)?;
                    } else {
                        e.set_note(*c, *n, &[k("velocity")], real(rng.unit()))?;
                    }
                }
            }
        }
        11 => {
            if let (Some(c), Some(t)) = (rng.pick(&midi), rng.pick(&ts)) {
                e.move_to(*c, *t, "items", 0)?;
            }
        }
        12 => {
            if let Some(t) = rng.pick(&ts) {
                e.set(*t, &[k("volume")], real(rng.unit() * 2.0))?;
            }
            if let Some(c) = rng.pick(&cs) {
                e.set_param(*c, "rack.eq.low", rng.unit() * 10.0)?;
            }
        }
        13 => {
            // A kit change: the outputs follow the ports
            for t in &ts {
                let has_drums =
                    e.find(*t).and_then(|t| t.get("instrument")).and_then(|i| i.get("plugin")).and_then(Value::as_text)
                        == Some("drums");
                if has_drums {
                    e.set(*t, &[k("instrument"), k("state"), k("kit")], text(KITS[rng.below(2)]))?;
                    break;
                }
            }
        }
        14 => {
            if let (Some(a), Some(b)) = (rng.pick(&ts), rng.pick(&ts)) {
                e.set(*b, &[k("folder")], Value::bool(true))?;
                e.set(*a, &[k("parent")], id_text(*b))?;
            }
        }
        _ => {
            if let Some(t) = rng.pick(&ts) {
                let instrument = if rng.chance(2) { Value::NULL } else { synth() };
                e.set(*t, &[k("instrument")], instrument)?;
            }
        }
    }
    Ok(())
}

#[test]
fn random_transactions_undo_and_redo_bit_exact_with_exact_indices_and_journal() {
    for seed in 1..=6u64 {
        let mut rng = Rng::new(seed * 7919);
        let mut doc = new_doc();
        let base = doc.root().clone();
        let mut states = vec![content(&doc)];
        let mut patches: Vec<JournalPatch> = Vec::new();
        let (mut ok, mut refused) = (0, 0);
        for step in 0..250 {
            let before = doc.root().clone();
            let ops = 1 + rng.below(3);
            let mut r = rng.clone();
            let result = doc.transact(label("test.random"), |e| {
                for _ in 0..ops {
                    random_op(e, &mut r)?;
                }
                Ok(())
            });
            rng = r;
            match result {
                Ok(diff) => {
                    ok += 1;
                    if diff.is_empty() {
                        assert_eq!(doc.root(), &before, "an empty diff changed nothing");
                    } else {
                        states.push(content(&doc));
                    }
                }
                Err(_) => {
                    refused += 1;
                    assert_eq!(doc.root(), &before, "seed {seed} step {step}: a refused transaction changed nothing");
                }
            }
            doc.verify_index().unwrap_or_else(|e| panic!("seed {seed} step {step}: {e}"));
            if rng.chance(5) {
                patches.extend(doc.take_journal());
            }
        }
        assert!(ok > 100 && refused > 0, "seed {seed}: {ok} ok, {refused} refused");
        // Undo everything
        for i in (0..states.len() - 1).rev() {
            doc.undo().unwrap();
            assert!(content(&doc) == states[i], "seed {seed}: undo to state {i}");
            doc.verify_index().unwrap();
        }
        assert!(doc.undo().is_err());
        assert_eq!(content(&doc), states[0]);
        // Redo everything
        for (i, state) in states.iter().enumerate().skip(1) {
            doc.redo().unwrap();
            assert!(content(&doc) == *state, "seed {seed}: redo to state {i}");
            doc.verify_index().unwrap();
        }
        // Some undo in the middle, then a new branch
        for _ in 0..10 {
            doc.undo().unwrap();
        }
        doc.transact(label("x"), |e| e.set(ROOT, &[k("project"), k("ppq")], Value::int(480))).unwrap();
        assert!(doc.redo().is_err());
        patches.extend(doc.take_journal());
        // The journal, through JSON, replayed onto the saved root
        let through_json: Vec<JournalPatch> = patches
            .iter()
            .map(|p| JournalPatch::from_value(&from_json(&to_json(&p.to_value().unwrap()).unwrap()).unwrap()).unwrap())
            .collect();
        for (a, b) in through_json.iter().zip(&patches) {
            assert_eq!(a.to_value().unwrap(), b.to_value().unwrap());
        }
        let replayed = replay(&base, &through_json).unwrap();
        assert!(replayed == *doc.root(), "seed {seed}: journal replay differs from the live root");
        // And the replayed root opens as a valid project
        let reopened = Document::from_root(registry(), "project", replayed, options()).unwrap().document;
        assert_eq!(reopened.index(), doc.index());
    }
}

#[test]
fn history_has_labels_and_a_limit() {
    let mut doc = Document::new_project(registry(), lm_doc::Options { history_limit: 3, ..options() }).unwrap();
    for i in 0..5 {
        doc.transact(label("test.step"), |e| e.set(ROOT, &[k("project"), k("seed")], Value::int(i))).unwrap();
    }
    assert_eq!(doc.history_len(), 3);
    assert_eq!(doc.undo_label().unwrap().key, "test.step");
    for _ in 0..3 {
        doc.undo().unwrap();
    }
    assert_eq!(doc.undo().unwrap_err().key(), "doc.history.nothing_to_undo");
    assert_eq!(doc.root().get("project").unwrap().get("seed"), Some(&Value::int(1)));
}

#[test]
fn undo_diffs_are_the_inverse_with_the_old_param_values() {
    let mut doc = new_doc();
    let m = add_channel(&mut doc, "c");
    doc.transact(label("x"), |e| e.set_param(m, "pan", 0.5)).unwrap();
    let d = doc.undo().unwrap();
    assert_eq!(d.origin, lm_doc::Origin::Undo);
    assert_eq!(d.params, vec![(m, std::sync::Arc::from("pan"), 0.0)]);
    let d = doc.undo().unwrap();
    assert_eq!(d.removed, [m]);
    let d = doc.redo().unwrap();
    assert_eq!(d.created, [m]);
    assert_eq!(d.origin, lm_doc::Origin::Redo);
}
