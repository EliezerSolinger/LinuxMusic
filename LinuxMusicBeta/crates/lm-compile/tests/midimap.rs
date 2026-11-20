//! The MIDI map compiled into the install (interfaces.md, I-21.2) and the automation details of I-21.5: parameter
//! ranges and slots (a send's amount included), selection and bank targets, aliases, the project winning over the
//! machine; lane modes and the global button; point shapes and jumps

mod common;

use std::sync::Arc;

use common::*;
use lm_compile::{CompilerCore, Environment, MapInput, PortAlias, Selection, compile_map};
use lm_midimap::{DocSource, MapTarget as DocTarget, Mapping, Message, NodeRef, SelRole, Transform};
use lm_render::{MapTarget, PortIndex, RangeScale, Segment};
use lm_state::{NodeId, PathStep, ROOT, Value};

fn mapping(id: u64, port: &str, number: u8, target: DocTarget) -> Mapping {
    Mapping {
        id: Some(NodeId::new(b'k', id).unwrap()),
        source: DocSource { port: Arc::from(port), channel: Some(0), message: Message::Cc { number } },
        touch: None,
        target,
        transform: Transform::default(),
        feedback: true,
        consume: true,
        feedback_to: None,
    }
}

fn param(node: NodeRef, key: &str) -> DocTarget {
    DocTarget::Param { node, key: key.to_owned() }
}

fn ports() -> Vec<PortAlias> {
    vec![PortAlias { alias: Arc::from("Controlador 1"), index: PortIndex(2), controller: true }]
}

#[test]
fn ranges_slots_and_the_map() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t1 = add_track(&mut doc, Some(dc()));
    let t2 = add_track(&mut doc, Some(dc()));
    let c1 = add_channel(&mut doc);
    let c2 = add_channel(&mut doc);
    let mut send = None;
    edit(&mut doc, |e| {
        send = Some(e.insert(c1, "sends", 0, Value::object([("to", text(&c2.to_string())), ("amount", real(0.5))]))?);
        Ok(())
    });
    let send = send.unwrap();
    let mut core = CompilerCore::new(config(&plugins));
    let c = core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    let table = c.table.clone();
    // Ranges: faders and sends on the gain scale, the pan linear, the rack and plugins from their schemas
    let r = |n: NodeId, k: &str| table.ranges.get(&(n, Arc::from(k))).copied();
    assert_eq!(r(t1, "volume").unwrap().scale, RangeScale::Gain);
    assert_eq!(r(c1, "pan").unwrap().min, -1.0);
    assert_eq!(r(send, "amount").unwrap().scale, RangeScale::Gain);
    assert!(!table.slots_of(send, "amount").is_empty(), "a send's amount has a slot");
    let drive = r(c1, "rack.tube.drive").unwrap();
    assert_eq!((drive.min, drive.max), (0.0, 1.0));

    let mappings = vec![
        mapping(1, "Controlador 1", 7, param(NodeRef::Id(t1), "volume")),
        mapping(2, "Controlador 1", 8, DocTarget::Field { node: NodeRef::Id(t1), path: vec![PathStep::key("mute")] }),
        mapping(3, "", 9, DocTarget::Action { id: Arc::from("transport.play"), params: Value::NULL, value_into: None }),
        mapping(4, "Controlador 1", 10, param(NodeRef::Selected { role: SelRole::Track, offset: 1 }, "volume")),
        mapping(5, "Controlador 1", 11, param(NodeRef::Selected { role: SelRole::TrackBank, offset: 0 }, "volume")),
        mapping(6, "Teclado", 12, param(NodeRef::Id(t1), "volume")), // the alias has no port here
        mapping(7, "Controlador 1", 13, param(NodeRef::Id(send), "amount")),
        // A machine mapping on a source the project uses loses; one on a free source works
        mapping(8, "Controlador 1", 7, param(NodeRef::Id(t2), "volume")),
        mapping(9, "Controlador 1", 14, param(NodeRef::Id(t2), "volume")),
    ];
    let input = MapInput {
        mappings,
        project_count: 7,
        ports: ports(),
        port_count: 4,
        selection: Selection { track: Some(t1), track_bank: 1, ..Selection::default() },
    };
    let compiled = compile_map(doc.root(), &table, &input, spec());
    assert_eq!(compiled.inactive, vec![5, 7], "the missing alias and the losing machine mapping");
    let entries = compiled.map.entries();
    let target_of = |id: u64| entries.iter().filter(|e| e.id == id).map(|e| e.target).collect::<Vec<_>>();
    let slots = |n: NodeId, k: &str| table.slots_of(n, k).to_vec();
    match target_of(1)[..] {
        [MapTarget::Param { slot, range }] => {
            assert_eq!(vec![slot], slots(t1, "volume"));
            assert_eq!(range.scale, RangeScale::Gain);
        }
        ref other => panic!("{other:?}"),
    }
    assert_eq!(target_of(2), vec![MapTarget::Session]);
    assert_eq!(target_of(3), vec![MapTarget::Session]);
    // The selected track is t1: offset 1 is t2; the bank starts at track 1, which is t2 too
    let t2_slot = slots(t2, "volume")[0];
    assert!(matches!(target_of(4)[..], [MapTarget::Param { slot, .. }] if slot == t2_slot));
    assert!(matches!(target_of(5)[..], [MapTarget::Param { slot, .. }] if slot == t2_slot));
    assert!(matches!(target_of(7)[..], [MapTarget::Param { .. }]));
    assert!(matches!(target_of(9)[..], [MapTarget::Param { slot, .. }] if slot == t2_slot));
    // Sources: the alias became the port index; "" is any port
    let e1 = entries.iter().find(|e| e.id == 1).unwrap();
    assert_eq!(e1.source.port, 2);
    assert_eq!(entries.iter().find(|e| e.id == 3).unwrap().source.port, lm_midimap::ANY_PORT);
    // A switch: a CC on a bool field or an action is a button; on a parameter it is a knob
    assert!(entries.iter().find(|e| e.id == 2).unwrap().switch);
    assert!(entries.iter().find(|e| e.id == 3).unwrap().switch);
    assert!(!e1.switch);
    let r = compiled.resolved.iter().find(|r| r.id == 4).unwrap();
    assert_eq!(r.node, Some(t2));
    // Selecting the last track: offset 1 is past the end, the mapping does nothing
    let input = MapInput { selection: Selection { track: Some(t2), ..Selection::default() }, ..input };
    let compiled = compile_map(doc.root(), &table, &input, spec());
    assert!(compiled.inactive.contains(&3));
}

fn lane(node: NodeId, points: Value, mode: Option<&str>) -> Value {
    let mut l = Value::object([
        ("target", Value::object([("node", text(&node.to_string())), ("param", text("volume"))])),
        ("points", points),
    ]);
    if let Some(m) = mode {
        l = l.with("mode", text(m));
    }
    l
}

#[test]
fn lane_modes_shapes_and_jumps() {
    let plugins = plugins();
    let mut doc = new_doc(&plugins);
    let t = add_track(&mut doc, Some(dc()));
    let points = Value::list([
        Value::object([("tick", Value::int(0)), ("value", real(0.0))]),
        Value::object([("tick", Value::int(960)), ("value", real(1.0)), ("shape", text("step")), ("out", real(0.5))]),
        Value::object([
            ("tick", Value::int(1920)),
            ("value", real(0.0)),
            ("shape", text("smooth")),
            ("tension", real(0.25)),
        ]),
    ]);
    edit(&mut doc, |e| e.insert(t, "automation", 0, lane(t, points.clone(), None)).map(|_| ()));
    let mut core = CompilerCore::new(config(&plugins));
    core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    let curve = core.curve(t, "volume").expect("a curve").clone();
    assert_eq!(curve.points().len(), 4, "the jump is two points");
    assert_eq!(curve.points()[1].sample, curve.points()[2].sample);
    assert_eq!(curve.points()[2].value, 0.5);
    assert_eq!(curve.segments()[1], Segment::Step);
    assert_eq!(curve.segments()[3], Segment::Smooth { tension: 0.25 });
    assert_eq!(curve.value_at(curve.points()[1].sample - 1), 0.0, "a step holds");
    // A lane that is off, and the global button off, play no curve
    edit(&mut doc, |e| e.set(t, &[k("automation"), PathStep::Index(0), k("mode")], text("off")));
    core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    assert!(core.curve(t, "volume").is_none());
    edit(&mut doc, |e| {
        e.set(t, &[k("automation"), PathStep::Index(0), k("mode")], text("latch"))?;
        e.set(ROOT, &[k("transport"), k("automationMode")], text("off"))
    });
    core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    assert!(core.curve(t, "volume").is_none());
    edit(&mut doc, |e| e.set(ROOT, &[k("transport"), k("automationMode")], text("lanes")));
    core.compile(doc.root(), &lm_compile::NoResources, &Environment::default()).unwrap();
    assert!(core.curve(t, "volume").is_some(), "back to the lane's own mode (latch plays)");
}
