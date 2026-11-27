//! The document's basic contract: a new project, ids from the counter, set/insert/remove/move, notes by number,
//! OnRemove in each case, the indices, the frozen defaults (U29), save and open

mod common;

use std::sync::Arc;

use common::*;
use lm_doc::{Document, MASTER, Options, PROJECT, schemas};
use lm_state::{
    FieldKind, FieldSpec, NodeId, NodeSchema, OnRemove, PathStep, ROOT, SchemaRegistry, Value, from_json, to_json,
};

#[test]
fn frozen_defaults() {
    let reference = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/defaults.json");
    let r = schemas::project_registry().unwrap();
    if let Err(mismatch) = lm_state::frozen::check_frozen_defaults(&r, reference.as_ref()) {
        panic!("{mismatch}");
    }
}

#[test]
fn a_new_project_is_valid_and_has_the_master() {
    let doc = new_doc();
    doc.verify_index().unwrap();
    assert_eq!(doc.type_of(MASTER), Some(schemas::types::MASTER));
    assert_eq!(doc.issued(), 1);
    assert_eq!(doc.uuid().unwrap().len(), 36);
    assert!(doc.find(ROOT).is_some());
    assert_eq!(doc.master().volume(&doc), Some(1.0));
}

/// The program's new project starts with the two effect returns (P-19), valid, and the counter past their ids
#[test]
fn a_desk_project_has_the_two_returns() {
    let mut doc = lm_doc::Document::new_desk_project(registry(), options()).unwrap();
    doc.verify_index().unwrap();
    assert_eq!(doc.issued(), 3);
    let channels = doc.root().get("mixer").and_then(|m| m.get("channels")).unwrap().clone();
    let names: Vec<_> = channels.items().filter_map(|c| c.get("name").and_then(|n| n.as_text())).collect();
    assert_eq!(names, ["FX 1", "FX 2"]);
    assert!(channels.items().all(|c| c.get("return").and_then(|v| v.as_bool()) == Some(true)));
    let t = add_track(&mut doc, "a", None);
    assert_eq!(t.number, 3, "the next id is past the returns");
}

#[test]
fn ids_come_from_one_counter_never_given_twice() {
    let mut doc = new_doc();
    let t = add_track(&mut doc, "a", None);
    let o = first_output(&doc, t);
    assert_eq!(t, NodeId { kind: b't', number: 1 });
    assert_eq!(o, NodeId { kind: b'o', number: 2 });
    let c = add_item(&mut doc, t, "midi");
    let mut n = 0;
    doc.transact(label("x"), |e| {
        n = e.add_note(c, Value::object([("key", Value::int(64))]))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(n, 4);
    doc.undo().unwrap();
    let mut n2 = 0;
    doc.transact(label("x"), |e| {
        n2 = e.add_note(c, Value::empty_object())?;
        Ok(())
    })
    .unwrap();
    assert_eq!(n2, 5, "undo never gives a number back (S12)");
    assert_eq!(doc.root().get("meta").unwrap().get("issued"), Some(&Value::int(6)));
}

#[test]
fn set_goes_through_nested_nodes_and_reports_params() {
    let mut doc = new_doc();
    let m = add_channel(&mut doc, "Bass");
    let t = add_track(&mut doc, "bass", Some(synth()));
    // A path through the track into its output is a change of the output
    let d = doc.transact(label("x"), |e| e.set(t, &[k("outputs"), PathStep::Index(0), k("to")], id_text(m))).unwrap();
    let o = first_output(&doc, t);
    assert!(d.changed_node(o).is_some());
    assert!(d.changed_node(t).is_none());
    assert_eq!(doc.referrers(m).len(), 1);
    // Parameters: the desk's rack, the plugin's params (by alias too)
    let d = doc.transact(label("x"), |e| e.set(m, &[k("rack"), k("tube"), k("drive")], real(0.5))).unwrap();
    assert_eq!(d.params, vec![(m, Arc::from("rack.tube.drive"), 0.5)]);
    assert!(!d.structural);
    let pr = lm_doc::TrackRef(t).instrument(&doc).unwrap();
    let p = pr.id();
    let d = doc.transact(label("x"), |e| e.set_param(p, "vol_1", 0.25)).unwrap();
    assert_eq!(d.params, vec![(p, Arc::from("vol1"), 0.25)]);
    assert_eq!(pr.param(&doc, "vol1"), Some(0.25));
    // Structural changes
    let d = doc.transact(label("x"), |e| e.set(m, &[k("rack"), k("tube"), k("model")], text("12ax7"))).unwrap();
    assert!(d.structural && d.params.is_empty());
    // Refused: out of range, unknown field, ids, meta, a bad plugin state
    let bad = |doc: &mut Document, f: &dyn Fn(&mut lm_doc::Edit<'_>) -> lm_base::Result<()>| {
        let before = doc.root().clone();
        let e = doc.transact(label("x"), |e| f(e)).unwrap_err();
        assert_eq!(doc.root(), &before);
        e.key()
    };
    assert_eq!(bad(&mut doc, &|e| e.set(m, &[k("pan")], real(3.0))), "doc.value.out_of_range");
    assert_eq!(bad(&mut doc, &|e| e.set(m, &[k("nope")], real(3.0))), "doc.path.unknown_field");
    assert_eq!(bad(&mut doc, &|e| e.set(m, &[k("id")], text("m:99"))), "doc.set.id");
    assert_eq!(bad(&mut doc, &|e| e.set(ROOT, &[k("meta"), k("issued")], Value::int(0))), "doc.meta.read_only");
    assert_eq!(
        bad(&mut doc, &|e| e.set(p, &[k("state"), k("params"), k("vol1")], real(7.0))),
        "state.schema.param_out_of_range"
    );
    assert_eq!(bad(&mut doc, &|e| e.set(m, &[k("output")], text("m:999"))), "doc.ref.dangling");
    doc.verify_index().unwrap();
}

#[test]
fn notes_by_number() {
    let mut doc = new_doc();
    let t = add_track(&mut doc, "a", Some(synth()));
    let c = add_item(&mut doc, t, "midi");
    let mut ns = Vec::new();
    let d = doc
        .transact(label("x"), |e| {
            for key in [60, 64, 67] {
                ns.push(e.add_note(c, Value::object([("key", Value::int(key))]))?);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(d.changed_node(c).unwrap().notes.created, ns);
    let d = doc.transact(label("x"), |e| e.set_note(c, ns[1], &[k("key")], Value::int(65))).unwrap();
    assert_eq!(d.changed_node(c).unwrap().notes.changed, [ns[1]]);
    let d = doc.transact(label("x"), |e| e.remove_note(c, ns[0])).unwrap();
    assert_eq!(d.changed_node(c).unwrap().notes.removed, [ns[0]]);
    assert_eq!(lm_doc::ItemRef(c).note(&doc, ns[1]).unwrap().get("key"), Some(&Value::int(65)));
    assert_eq!(lm_doc::ItemRef(c).note_count(&doc), 2);
    let e = doc.transact(label("x"), |e| e.remove_note(c, ns[0])).unwrap_err();
    assert_eq!(e.key(), "doc.note.not_found");
    let e = doc.transact(label("x"), |e| e.set_note(c, ns[1], &[k("key")], Value::int(200))).unwrap_err();
    assert_eq!(e.key(), "doc.value.out_of_range");
    // An audio item has no notes
    let a = add_item(&mut doc, t, "audio");
    let e = doc.transact(label("x"), |e| e.add_note(a, Value::empty_object()).map(|_| ())).unwrap_err();
    assert_eq!(e.key(), "doc.item.invalid");
}

#[test]
fn move_uses_the_final_index() {
    let mut doc = new_doc();
    let ts: Vec<NodeId> = (0..4).map(|i| add_track(&mut doc, &format!("t{i}"), None)).collect();
    doc.transact(label("x"), |e| e.move_to(ts[0], ROOT, "tracks", 2)).unwrap();
    let order: Vec<NodeId> = doc.tracks().iter().map(|t| t.id()).collect();
    assert_eq!(order, [ts[1], ts[2], ts[0], ts[3]]);
    doc.transact(label("x"), |e| e.move_to(ts[3], ROOT, "tracks", 0)).unwrap();
    let order: Vec<NodeId> = doc.tracks().iter().map(|t| t.id()).collect();
    assert_eq!(order, [ts[3], ts[1], ts[2], ts[0]]);
    // An item to another track
    let c = add_item(&mut doc, ts[0], "midi");
    let d = doc.transact(label("x"), |e| e.move_to(c, ts[1], "items", 0)).unwrap();
    assert!(d.created.is_empty() && d.removed.is_empty());
    assert_eq!(doc.loc(c).unwrap().parent, ts[1]);
    doc.verify_index().unwrap();
    let e = doc.transact(label("x"), |e| e.move_to(c, ts[1], "outputs", 0)).unwrap_err();
    assert_eq!(e.key(), "doc.move.wrong_type");
    let e = doc.transact(label("x"), |e| e.move_to(c, ts[1], "items", 5)).unwrap_err();
    assert_eq!(e.key(), "doc.move.index");
}

#[test]
fn removing_a_channel_redirects_outputs_and_channels_to_the_master_and_removes_sends() {
    let mut doc = new_doc();
    let bus = add_channel(&mut doc, "bus");
    let rev = add_channel(&mut doc, "reverb");
    let other = add_channel(&mut doc, "other");
    let t = add_track(&mut doc, "a", Some(drums("small")));
    let outs = lm_doc::TrackRef(t).outputs(&doc);
    assert_eq!(outs.len(), 3);
    doc.transact(label("x"), |e| {
        for o in &outs {
            e.set(o.id(), &[k("to")], id_text(bus))?;
        }
        e.set(other, &[k("output")], id_text(bus))?;
        e.insert(other, "sends", 0, Value::object([("to", id_text(rev))]))?;
        e.insert(bus, "sends", 0, Value::object([("to", id_text(rev))]))?;
        Ok(())
    })
    .unwrap();
    let send = lm_doc::ChannelRef(other).sends(&doc)[0].id();
    // Redirect: the outputs and the channel that fed the bus now go to the master
    let d = doc.transact(label("x"), |e| e.remove(bus)).unwrap();
    assert!(outputs(&doc, t).iter().all(|(_, _, to)| to == "m:0"));
    assert_eq!(lm_doc::ChannelRef(other).output(&doc), Some(MASTER));
    assert!(d.removed.contains(&bus));
    // RemoveOwner: the send to the reverb leaves with it
    let d = doc.transact(label("x"), |e| e.remove(rev)).unwrap();
    assert!(d.removed.contains(&send));
    assert!(lm_doc::ChannelRef(other).sends(&doc).is_empty());
    doc.verify_index().unwrap();
    // The master cannot be removed
    let e = doc.transact(label("x"), |e| e.remove(MASTER)).unwrap_err();
    assert_eq!(e.key(), "doc.remove.required");
}

#[test]
fn removing_a_plugin_removes_its_automation_and_its_sidechain_sends() {
    let mut doc = new_doc();
    let m = add_channel(&mut doc, "vox");
    let src = add_channel(&mut doc, "kick");
    let mut insert = None;
    doc.transact(label("x"), |e| {
        let p = e.insert(m, "inserts", 0, comp())?;
        insert = Some(p);
        e.insert(src, "sends", 0, Value::object([("to", id_text(p)), ("port", text("sidechain"))]))?;
        e.insert(
            m,
            "automation",
            0,
            Value::object([("target", Value::object([("node", id_text(p)), ("param", text("threshold"))]))]),
        )?;
        Ok(())
    })
    .unwrap();
    let p = insert.unwrap();
    assert_eq!(doc.referrers(p).len(), 2);
    let d = doc.transact(label("x"), |e| e.remove(p)).unwrap();
    assert_eq!(d.removed.len(), 3);
    assert!(lm_doc::ChannelRef(src).sends(&doc).is_empty());
    assert!(doc.find(m).unwrap().get("automation").unwrap().is_empty());
    doc.verify_index().unwrap();
}

#[test]
fn removing_a_folder_clears_the_parents_and_a_lane_clears_the_comp() {
    let mut doc = new_doc();
    let f = add_track(&mut doc, "folder", None);
    let t = add_track(&mut doc, "child", None);
    doc.transact(label("x"), |e| {
        e.set(f, &[k("folder")], Value::bool(true))?;
        e.set(t, &[k("parent")], id_text(f))
    })
    .unwrap();
    doc.transact(label("x"), |e| e.remove(f)).unwrap();
    assert_eq!(doc.find(t).unwrap().get("parent"), Some(&Value::NULL));

    let c = add_item(&mut doc, t, "audio");
    let hash = "0".repeat(64);
    let res = Value::resource(lm_state::Resource::new("audio", &hash).unwrap());
    let mut lanes = Vec::new();
    doc.transact(label("x"), |e| {
        lanes.push(e.insert(c, "lanes", 0, Value::object([("source", res.clone())]))?);
        lanes.push(e.insert(c, "lanes", 1, Value::object([("source", res.clone())]))?);
        let seg = |from: i64, to: i64, lane: NodeId| {
            Value::object([("from", Value::int(from)), ("to", Value::int(to)), ("lane", id_text(lane))])
        };
        e.set(c, &[k("comp")], Value::list([seg(0, 1920, lanes[1]), seg(1920, 3840, lanes[0])]))
    })
    .unwrap();
    doc.transact(label("x"), |e| e.remove(lanes[1])).unwrap();
    let comp = doc.find(c).unwrap().get("comp").cloned().unwrap();
    assert_eq!(comp.at(0).unwrap().get("lane"), Some(&Value::NULL));
    assert_eq!(comp.at(1).unwrap().get("lane"), Some(&id_text(lanes[0])));
    doc.verify_index().unwrap();
    // A comp that points to a lane of another item is refused
    let c2 = add_item(&mut doc, t, "audio");
    let e = doc
        .transact(label("x"), |e| {
            e.set(c2, &[k("comp")], Value::list([Value::object([("to", Value::int(10)), ("lane", id_text(lanes[0]))])]))
        })
        .unwrap_err();
    assert_eq!(e.key(), "doc.item.invalid");
}

/// A registry with a list of references (`Clear` removes the item) and a `Refuse` reference
fn ref_registry() -> Arc<SchemaRegistry> {
    let mut r = SchemaRegistry::new();
    r.add(
        NodeSchema::new("test.root", 1)
            .field(FieldSpec::new("things", FieldKind::List(Box::new(FieldKind::Object("test.thing")))))
            .field(FieldSpec::new("holders", FieldKind::List(Box::new(FieldKind::Object("test.holder"))))),
    )
    .unwrap();
    r.add(NodeSchema::new("test.thing", 1).field(FieldSpec::id(b'x'))).unwrap();
    r.add(
        NodeSchema::new("test.holder", 1)
            .field(FieldSpec::id(b'h'))
            .field(FieldSpec::new(
                "many",
                FieldKind::List(Box::new(FieldKind::Ref { to: b"x", on_remove: OnRemove::Clear })),
            ))
            .field(
                FieldSpec::new("pinned", FieldKind::Ref { to: b"x", on_remove: OnRemove::Refuse })
                    .nullable()
                    .default(Value::NULL),
            ),
    )
    .unwrap();
    r.check_complete().unwrap();
    Arc::new(r)
}

#[test]
fn clear_in_a_list_removes_the_item_and_refuse_refuses() {
    let root = Value::object([
        ("things", Value::list((1..=3).map(|i| Value::object([("id", text(&format!("x:{i}")))])))),
        (
            "holders",
            Value::list([Value::object([
                ("id", text("h:4")),
                ("many", Value::list([text("x:1"), text("x:2"), text("x:3"), text("x:2")])),
                ("pinned", text("x:3")),
            ])]),
        ),
    ]);
    let opts = Options { rules: Vec::new(), ..Options::default() };
    let mut doc = Document::from_root(ref_registry(), "test.root", root, opts).unwrap().document;
    let x = |n| NodeId { kind: b'x', number: n };
    let h = NodeId { kind: b'h', number: 4 };
    doc.transact(label("x"), |e| e.remove(x(2))).unwrap();
    assert_eq!(doc.find(h).unwrap().get("many"), Some(&Value::list([text("x:1"), text("x:3")])));
    doc.verify_index().unwrap();
    let e = doc.transact(label("x"), |e| e.remove(x(3))).unwrap_err();
    assert_eq!(e.key(), "doc.remove.refused");
    doc.transact(label("x"), |e| {
        e.set(h, &[k("pinned")], Value::NULL)?;
        e.remove(x(3))
    })
    .unwrap();
    assert_eq!(doc.find(h).unwrap().get("many"), Some(&Value::list([text("x:1")])));
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.find(h).unwrap().get("many").unwrap().len(), 4);
    doc.verify_index().unwrap();
}

#[test]
fn save_and_open_give_the_same_tree() {
    let mut doc = new_doc();
    let m = add_channel(&mut doc, "c");
    let t = add_track(&mut doc, "drums", Some(drums("big")));
    let c = add_item(&mut doc, t, "midi");
    doc.transact(label("x"), |e| {
        e.add_note(c, Value::empty_object())?;
        e.set(m, &[k("name")], text("C"))
    })
    .unwrap();
    let json = to_json(&doc.saved()).unwrap();
    let opened = Document::open(registry(), from_json(&json).unwrap(), options()).unwrap();
    assert!(!opened.upgraded && !opened.document.read_only(), "{:?}", opened.notices);
    assert_eq!(opened.document.root(), doc.root());
    assert_eq!(opened.document.index(), doc.index());
    assert_eq!(to_json(&opened.document.saved()).unwrap(), json);
}

#[test]
fn the_root_type_is_the_project() {
    assert_eq!(new_doc().type_of(ROOT), Some(PROJECT));
}

#[test]
fn the_interface_document_has_no_history_but_has_feeds() {
    let root = Value::object([("things", Value::empty_list()), ("holders", Value::empty_list())]);
    let mut doc = Document::from_root(ref_registry(), "test.root", root, Options::interface()).unwrap().document;
    let mut feed = doc.feed(lm_doc::Topic::All);
    let mut id = None;
    doc.transact(label("x"), |e| {
        id = Some(e.insert(ROOT, "things", 0, Value::empty_object())?);
        Ok(())
    })
    .unwrap();
    assert_eq!(feed.drain(&doc).unwrap().created, [id.unwrap()]);
    assert_eq!(doc.undo().unwrap_err().key(), "doc.history.nothing_to_undo");
    assert_eq!(doc.history_len(), 0);
    assert!(doc.take_journal().is_none());
    // Its own counter, without meta: ids still never repeat
    let mut id2 = None;
    doc.transact(label("x"), |e| {
        id2 = Some(e.insert(ROOT, "things", 0, Value::empty_object())?);
        Ok(())
    })
    .unwrap();
    assert_ne!(id, id2);
}

/// The legacy strip's input controls and the master's PRE faders: optional fields, absent in a new project, set like
/// any other and checked against their ranges
#[test]
fn the_legacy_strip_fields_are_optional_and_checked() {
    let mut doc = new_doc();
    let k = PathStep::key;
    let mut ch = MASTER;
    doc.transact(label("c"), |e| {
        ch = e.insert(ROOT, "mixer.channels", 0, Value::object([("name", Value::text("a"))]))?;
        Ok(())
    })
    .unwrap();
    let node = doc.find(ch).unwrap();
    for key in ["trim", "mono", "phaseInvert", "swap", "color"] {
        assert!(node.get(key).is_none(), "{key} is absent in a new channel");
    }
    doc.transact(label("set"), |e| {
        e.set(ch, &[k("trim")], Value::real(6.0)?)?;
        e.set(ch, &[k("mono")], Value::bool(true))?;
        e.set(ch, &[k("phaseInvert")], Value::bool(true))?;
        e.set(ch, &[k("swap")], Value::bool(true))?;
        e.set(ch, &[k("color")], Value::text("#3366cc"))?;
        e.set(MASTER, &[k("preLeft")], Value::real(0.5)?)?;
        e.set(MASTER, &[k("preLink")], Value::bool(false))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(doc.find(ch).unwrap().get("trim"), Some(&Value::real(6.0).unwrap()));
    assert_eq!(doc.find(MASTER).unwrap().get("preLink"), Some(&Value::bool(false)));
    let too_much = doc.transact(label("set"), |e| e.set(ch, &[k("trim")], Value::real(30.0)?));
    assert!(too_much.is_err(), "trim is at most +24 dB");
}
