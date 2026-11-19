//! The domain rules: the instrument's ports and the track's outputs (M1), routing cycles, folders, the engine's
//! limits (H12); paste (H13); heavy edits (H7); opening (upgrades, H11)

mod common;

use std::sync::Arc;

use common::*;
use lm_doc::{Document, Limits, MASTER, Options, Patch, PatchOp};
use lm_state::{NodeId, PathStep, ROOT, Value, from_json, to_json};

#[test]
fn a_new_track_gets_its_main_output_to_the_master() {
    let mut doc = new_doc();
    let t = add_track(&mut doc, "audio", None);
    assert_eq!(outputs(&doc, t), [("main".to_owned(), 2, "m:0".to_owned())]);
    let f = add_track(&mut doc, "synth", Some(synth()));
    assert_eq!(outputs(&doc, f), [("main".to_owned(), 2, "m:0".to_owned())]);
}

#[test]
fn a_kit_change_reconciles_the_outputs_m1() {
    let mut doc = new_doc();
    let kick = add_channel(&mut doc, "kick");
    let drums_bus = add_channel(&mut doc, "drums");
    let t = add_track(&mut doc, "drums", Some(drums("small")));
    assert_eq!(outputs(&doc, t).iter().map(|o| o.0.as_str()).collect::<Vec<_>>(), ["kick", "snare", "oh"]);
    let outs = lm_doc::TrackRef(t).outputs(&doc);
    doc.transact(label("x"), |e| {
        e.set(outs[0].id(), &[k("to")], id_text(drums_bus))?;
        e.set(outs[1].id(), &[k("to")], id_text(kick))?;
        e.set(outs[2].id(), &[k("to")], id_text(drums_bus))
    })
    .unwrap();
    let kick_output = outs[0].id();
    // The bigger kit: kick and snare stay with their routing, oh changes to mono, tom and room are new and go
    // where the main output goes
    let d = doc.transact(label("x"), |e| e.set(t, &[k("instrument"), k("state"), k("kit")], text("big"))).unwrap();
    let after = outputs(&doc, t);
    let names: Vec<&str> = after.iter().map(|o| o.0.as_str()).collect();
    assert_eq!(names, ["kick", "snare", "tom", "oh", "room"]);
    assert_eq!(after[0], ("kick".to_owned(), 1, drums_bus.to_string()));
    assert_eq!(after[1].2, kick.to_string());
    assert_eq!(after[2].2, drums_bus.to_string());
    assert_eq!(after[3], ("oh".to_owned(), 1, drums_bus.to_string()));
    assert_eq!(after[4], ("room".to_owned(), 2, drums_bus.to_string()));
    assert_eq!(first_output(&doc, t), kick_output, "the kept output keeps its id");
    assert_eq!(d.created.len(), 2);
    // Back to the small kit: tom and room leave, with a notice
    let d = doc.transact(label("x"), |e| e.set(t, &[k("instrument"), k("state"), k("kit")], text("small"))).unwrap();
    assert_eq!(d.removed.len(), 2);
    assert_eq!(d.notices[0].key, "doc.outputs.removed");
    assert_eq!(outputs(&doc, t).iter().map(|o| o.0.as_str()).collect::<Vec<_>>(), ["kick", "snare", "oh"]);
    // Undo brings the big kit and its outputs back, with their ids
    doc.undo().unwrap();
    assert_eq!(outputs(&doc, t).len(), 5);
    doc.verify_index().unwrap();
    // The rule also runs for a whole instrument replaced, and in a heavy edit
    doc.transact(label("x"), |e| e.set(t, &[k("instrument")], synth())).unwrap();
    assert_eq!(outputs(&doc, t), [("main".to_owned(), 2, drums_bus.to_string())]);
}

#[test]
fn a_plugin_without_known_ports_keeps_its_outputs() {
    let mut doc = Document::new_project(registry(), Options::default()).unwrap();
    let t = add_track(&mut doc, "a", Some(drums("small")));
    // No ports callback: the outputs stay as they are (none were made for the instrument)
    assert!(outputs(&doc, t).is_empty());
}

#[test]
fn routing_cycles_are_refused() {
    let mut doc = new_doc();
    let a = add_channel(&mut doc, "a");
    let b = add_channel(&mut doc, "b");
    let c = add_channel(&mut doc, "c");
    doc.transact(label("x"), |e| {
        e.set(a, &[k("output")], id_text(b))?;
        e.set(b, &[k("output")], id_text(c))
    })
    .unwrap();
    let before = doc.root().clone();
    let e = doc.transact(label("x"), |e| e.set(c, &[k("output")], id_text(a))).unwrap_err();
    assert_eq!(e.key(), "doc.routing.cycle");
    let e = doc.transact(label("x"), |e| e.set(a, &[k("output")], id_text(a))).unwrap_err();
    assert_eq!(e.key(), "doc.routing.cycle");
    let e = doc.transact(label("x"), |e| e.insert(c, "sends", 0, Value::object([("to", id_text(a))])).map(|_| ())).unwrap_err();
    assert_eq!(e.key(), "doc.routing.cycle");
    // Through a sidechain: c feeds an insert of a, which goes to b, then c
    let e = doc
        .transact(label("x"), |e| {
            let p = e.insert(a, "inserts", 0, comp())?;
            e.insert(c, "sends", 0, Value::object([("to", id_text(p)), ("port", text("sidechain"))]))?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(e.key(), "doc.routing.cycle");
    // A sidechain must name a port the insert has, and only an insert can take one
    let e = doc
        .transact(label("x"), |e| {
            let p = e.insert(c, "inserts", 0, comp())?;
            e.insert(a, "sends", 0, Value::object([("to", id_text(p)), ("port", text("nope"))]))?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(e.key(), "doc.send.no_such_port");
    assert_eq!(doc.root(), &before);
}

#[test]
fn a_folder_inside_itself_is_refused() {
    let mut doc = new_doc();
    let a = add_track(&mut doc, "a", None);
    let b = add_track(&mut doc, "b", None);
    doc.transact(label("x"), |e| {
        e.set(a, &[k("folder")], Value::bool(true))?;
        e.set(b, &[k("folder")], Value::bool(true))?;
        e.set(b, &[k("parent")], id_text(a))
    })
    .unwrap();
    let e = doc.transact(label("x"), |e| e.set(a, &[k("parent")], id_text(b))).unwrap_err();
    assert_eq!(e.key(), "doc.track.folder_cycle");
    let e = doc.transact(label("x"), |e| e.set(a, &[k("parent")], id_text(a))).unwrap_err();
    assert_eq!(e.key(), "doc.track.folder_cycle");
    let e = doc.transact(label("x"), |e| e.set(a, &[k("folder")], Value::bool(false))).unwrap_err();
    assert_eq!(e.key(), "doc.track.parent_not_folder");
}

#[test]
fn the_engine_limits_are_document_rules_h12() {
    let limits = Limits { max_nodes: 12, max_params: 1000 };
    let mut doc = Document::new_project(registry(), Options { limits: Some(limits), ..options() }).unwrap();
    // The master: 2 nodes. Each track with a synth and one output: 3
    for i in 0..3 {
        add_track(&mut doc, &format!("{i}"), Some(synth()));
    }
    let e = doc
        .transact(label("x"), |e| e.insert(ROOT, "tracks", 0, Value::object([("instrument", synth())])).map(|_| ()))
        .unwrap_err();
    assert_eq!(e.key(), "doc.limits.nodes");
    assert_eq!(doc.tracks().len(), 3);
    // Parameters
    let limits = Limits { max_nodes: 1000, max_params: 30 };
    let mut doc = Document::new_project(registry(), Options { limits: Some(limits), ..options() }).unwrap();
    let e = doc
        .transact(label("x"), |e| {
            for _ in 0..3 {
                e.insert(ROOT, "mixer.channels", 0, Value::empty_object())?;
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(e.key(), "doc.limits.params");
}

#[test]
fn tempo_map_rules() {
    let mut doc = new_doc();
    let point = |tick: i64, bpm: &str, ramp: bool| {
        Value::object([("tick", Value::int(tick)), ("bpm", text(bpm)), ("ramp", Value::bool(ramp))])
    };
    doc.transact(label("x"), |e| e.set(ROOT, &[k("tempoMap")], Value::list([point(0, "120", true), point(3840, "140.5", false)]))).unwrap();
    for (map, problem) in [
        (Value::list([point(10, "120", false)]), "first_not_at_zero"),
        (Value::list([point(0, "120", false), point(0, "90", false)]), "ticks_not_increasing"),
        (Value::list([point(0, "0", false)]), "bpm"),
        (Value::list([point(0, "120", true)]), "last_is_ramp"),
        (Value::empty_list(), "empty"),
    ] {
        let e = doc.transact(label("x"), |e| e.set(ROOT, &[k("tempoMap")], map.clone())).unwrap_err();
        assert_eq!(e.key(), "doc.tempo.invalid", "{problem}");
        assert!(e.to_string().contains(problem), "{e}");
    }
}

#[test]
fn automation_targets_existing_parameters() {
    let mut doc = new_doc();
    let m = add_channel(&mut doc, "c");
    let t = add_track(&mut doc, "a", Some(synth()));
    let p = lm_doc::TrackRef(t).instrument(&doc).unwrap().id();
    let lane = |node: NodeId, param: &str| {
        Value::object([
            ("target", Value::object([("node", id_text(node)), ("param", text(param))])),
            ("points", Value::list([Value::object([("tick", Value::int(0)), ("value", real(0.5))])])),
        ])
    };
    doc.transact(label("x"), |e| {
        e.insert(m, "automation", 0, lane(m, "rack.tube.drive"))?;
        e.insert(t, "automation", 0, lane(p, "vol1"))?;
        e.insert(t, "automation", 1, lane(t, "volume"))?;
        Ok(())
    })
    .unwrap();
    for (node, param) in [(m, "rack.tube.model"), (p, "nope"), (t, "mute")] {
        let e = doc.transact(label("x"), |e| e.insert(m, "automation", 0, lane(node, param)).map(|_| ())).unwrap_err();
        assert_eq!(e.key(), "doc.automation.no_such_param", "{param}");
    }
    // Another plugin: the lanes of parameters it does not have leave
    let d = doc.transact(label("x"), |e| e.set(t, &[k("instrument"), k("plugin")], text("drums"))).unwrap_err();
    assert!(d.key().starts_with("state.schema"), "the synth's state is not a drum kit's: {d}");
    let d = doc.transact(label("x"), |e| {
        e.set(t, &[k("instrument"), k("state")], Value::object([("kit", text("small"))]))?;
        e.set(t, &[k("instrument"), k("plugin")], text("drums"))
    });
    let d = d.unwrap();
    assert_eq!(d.notices.iter().filter(|n| n.key == "doc.automation.removed").count(), 1);
    assert_eq!(lm_doc::TrackRef(t).automation(&doc).len(), 1);
}

#[test]
fn paste_in_the_same_project_keeps_outside_references() {
    let mut doc = new_doc();
    let rev = add_channel(&mut doc, "reverb");
    let src = add_channel(&mut doc, "src");
    doc.transact(label("x"), |e| {
        e.insert(src, "sends", 0, Value::object([("to", id_text(rev))]))?;
        e.set(src, &[k("output")], id_text(rev))
    })
    .unwrap();
    let clip = doc.copy(src).unwrap();
    let mut pasted = None;
    doc.transact(label("x"), |e| {
        pasted = e.paste(ROOT, "mixer.channels", 0, &clip)?;
        Ok(())
    })
    .unwrap();
    let new = pasted.unwrap();
    assert_ne!(new, src);
    let ch = lm_doc::ChannelRef(new);
    assert_eq!(ch.output(&doc), Some(rev));
    let send = ch.sends(&doc)[0];
    assert_ne!(send.id(), lm_doc::ChannelRef(src).sends(&doc)[0].id());
    assert_eq!(send.to(&doc), Some(rev));
    doc.verify_index().unwrap();
}

#[test]
fn paste_from_another_project_rules_outside_references() {
    let mut other = new_doc();
    let rev = add_channel(&mut other, "reverb");
    let src = add_channel(&mut other, "src");
    let t = add_track(&mut other, "keys", Some(synth()));
    let c = add_item(&mut other, t, "midi");
    other
        .transact(label("x"), |e| {
            e.insert(src, "sends", 0, Value::object([("to", id_text(rev))]))?;
            e.set(src, &[k("output")], id_text(rev))?;
            e.set(t, &[k("outputs"), PathStep::Index(0), k("to")], id_text(rev))?;
            let p = lm_doc::TrackRef(t).instrument_in(e);
            e.insert(t, "automation", 0, Value::object([
                ("target", Value::object([("node", id_text(p)), ("param", text("vol1"))])),
            ]))?;
            for key in 0..5 {
                e.add_note(c, Value::object([("key", Value::int(60 + key))]))?;
            }
            Ok(())
        })
        .unwrap();
    let mut doc = new_doc();
    // Ids in both projects collide, and they are new in the paste
    let clip = other.copy(src).unwrap();
    let mut pasted = None;
    doc.transact(label("x"), |e| {
        pasted = e.paste(ROOT, "mixer.channels", 0, &clip)?;
        Ok(())
    })
    .unwrap();
    let ch = lm_doc::ChannelRef(pasted.unwrap());
    assert_eq!(ch.output(&doc), Some(MASTER), "Redirect");
    assert!(ch.sends(&doc).is_empty(), "RemoveOwner");
    // A track with its instrument, automation of it (inside the clip: follows the new ids) and notes
    let clip = other.copy(t).unwrap();
    let mut pasted = None;
    doc.transact(label("x"), |e| {
        pasted = e.paste(ROOT, "tracks", 0, &clip)?;
        Ok(())
    })
    .unwrap();
    let t2 = lm_doc::TrackRef(pasted.unwrap());
    let p2 = t2.instrument(&doc).unwrap().id();
    assert_eq!(t2.automation(&doc)[0].target(&doc).unwrap().0, p2);
    assert_eq!(outputs(&doc, t2.id())[0].2, "m:0");
    let item = t2.items(&doc)[0];
    assert_eq!(item.note_count(&doc), 5);
    let numbers: Vec<u64> = item.value(&doc).unwrap().get("notes").unwrap().keys().map(|k| k.parse().unwrap()).collect();
    assert!(numbers.iter().all(|n| *n < doc.issued()));
    doc.verify_index().unwrap();
    // Pasting twice never repeats an id
    doc.transact(label("x"), |e| e.paste(ROOT, "tracks", 0, &clip).map(|_| ())).unwrap();
    doc.verify_index().unwrap();
}

#[test]
fn heavy_edits_apply_unless_their_base_changed_h7() {
    let mut doc = new_doc();
    let t = add_track(&mut doc, "a", Some(synth()));
    let c = add_item(&mut doc, t, "midi");
    let snapshot = doc.snapshot();
    // Computed elsewhere, from the snapshot
    let compute = move |root: &Value| {
        let path = [k("tracks"), PathStep::Index(0), k("items"), PathStep::Index(0)];
        let item = root.at_path(&path).unwrap().clone();
        let ops = (0..100).map(|i| PatchOp::AddNote { item: c, note: Value::object([("key", Value::int(i % 128))]) }).collect();
        Patch { ops, based_on: vec![(c, item)] }
    };
    let patch = std::thread::spawn(move || compute(&snapshot)).join().unwrap();
    let d = doc.apply_patch(label("x"), &patch).unwrap();
    assert_eq!(d.changed_node(c).unwrap().notes.created.len(), 100);
    // The base changed: a conflict
    let e = doc.apply_patch(label("x"), &patch).unwrap_err();
    assert_eq!(e.key(), "doc.patch.conflict");
}

#[test]
fn opening_upgrades_older_nodes_and_newer_ones_open_read_only_h11() {
    // Saved by a program whose synth was at version 1
    let mut doc = new_doc();
    add_track(&mut doc, "a", Some(synth()));
    let json = to_json(&doc.saved()).unwrap();
    let r2 = Arc::new(registry_with(2));
    let opened = Document::open(Arc::clone(&r2), from_json(&json).unwrap(), options()).unwrap();
    assert!(opened.upgraded && !opened.document.read_only());
    let p = lm_doc::TrackRef(opened.document.tracks()[0].id()).instrument(&opened.document).unwrap();
    assert_eq!(p.state(&opened.document).unwrap().get("wave"), Some(&text("square")));
    assert_eq!(p.value(&opened.document).unwrap().get("version"), Some(&Value::int(2)));
    // Saved by the newer program, opened by the older one: read-only
    let newer = to_json(&opened.document.saved()).unwrap();
    let back = Document::open(registry(), from_json(&newer).unwrap(), options()).unwrap();
    assert!(back.document.read_only());
    assert!(back.notices.iter().any(|n| n.key == "doc.load.newer"));
    // A type of a newer version
    let saved = doc.saved();
    let bumped = saved.with_path(&[k("versions"), k("track")], Value::int(9)).unwrap();
    let opened = Document::open(registry(), bumped, options()).unwrap();
    assert!(opened.document.read_only());
}

#[test]
fn opening_checks_across_nodes() {
    let mut doc = new_doc();
    let m = add_channel(&mut doc, "c");
    let saved = doc.saved();
    // A reference to nothing
    let path = [k("mixer"), k("channels"), PathStep::Index(0), k("output")];
    let e = Document::open(registry(), saved.with_path(&path, text("m:77")).unwrap(), options()).unwrap_err();
    assert_eq!(e.key(), "doc.load.dangling_ref");
    // A cycle
    let e = Document::open(registry(), saved.with_path(&path, id_text(m)).unwrap(), options()).unwrap_err();
    assert_eq!(e.key(), "doc.routing.cycle");
    // A counter below the ids is repaired
    let low = saved.with_path(&[k("meta"), k("issued")], Value::int(1)).unwrap();
    let opened = Document::open(registry(), low, options()).unwrap();
    assert!(opened.document.issued() > m.number);
    assert!(opened.notices.iter().any(|n| n.key == "doc.load.issued_repaired"));
    // No versions
    assert_eq!(Document::open(registry(), doc.root().clone(), options()).unwrap_err().key(), "doc.load.no_versions");
}

trait InstrumentIn {
    fn instrument_in(self, e: &lm_doc::Edit<'_>) -> NodeId;
}

impl InstrumentIn for lm_doc::TrackRef {
    fn instrument_in(self, e: &lm_doc::Edit<'_>) -> NodeId {
        NodeId::parse(e.find(self.id()).unwrap().get("instrument").unwrap().get("id").unwrap().as_text().unwrap()).unwrap()
    }
}
