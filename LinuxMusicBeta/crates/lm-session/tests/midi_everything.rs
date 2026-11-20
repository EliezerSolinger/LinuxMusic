//! The generated test of I-21.6: every parameter of every plugin, of the desk and of the tracks, every field of a
//! track and a channel that a control can set, and every action of the registry, each mapped to a control (NRPN, so
//! one port has room for all), each answering it. Following the selection and changing banks recompile only the map

mod common;

use std::time::{Duration, Instant};

use common::*;
use lm_base::{HostTime, Label};
use lm_midimap::{DocSource, MapTarget, Mapping, Message, NodeRef, SelRole, Transform};
use lm_plugin::PluginKind;
use lm_render::{Command, PortIndex};
use lm_session::Session;
use lm_state::{FieldKind, NodeId, PathStep, ROOT, Value};

fn run_for(s: &mut Session, d: Duration) {
    let start = Instant::now();
    while start.elapsed() < d {
        s.pump();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Polls until `f` holds (the debug build renders every plugin slower than real time, so the device's clock, and
/// with it the time a stamped message is due, falls behind the wall clock)
fn eventually(s: &mut Session, what: &str, mut f: impl FnMut(&mut Session) -> Vec<String>) {
    let start = Instant::now();
    loop {
        let missing = f(s);
        if missing.is_empty() {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "{what}: {missing:?} {:?}", s.notices().keys());
        run_for(s, Duration::from_millis(50));
    }
}

fn nrpn(number: u16, target: MapTarget) -> Mapping {
    Mapping {
        id: None,
        source: DocSource { port: "Ctl".into(), channel: Some(0), message: Message::Nrpn { number } },
        touch: None,
        target,
        transform: Transform { takeover: lm_midimap::Takeover::Jump, ..Transform::default() },
        feedback: false,
        consume: true,
        feedback_to: None,
    }
}

/// The four messages that set NRPN `number` to its maximum
fn nrpn_max(number: u16) -> [[u8; 3]; 4] {
    [[0xB0, 99, (number >> 7) as u8], [0xB0, 98, (number & 0x7F) as u8], [0xB0, 6, 127], [0xB0, 38, 127]]
}

fn send_all(s: &mut Session, port: PortIndex, msgs: &[[u8; 3]]) {
    for chunk in msgs.chunks(64) {
        for m in chunk {
            s.engine().commands().send(Command::Midi { port, at: HostTime::now(), bytes: *m }).expect("midi");
        }
        run_for(s, Duration::from_millis(15));
    }
}

fn add_all_mappings(s: &mut Session, maps: Vec<Mapping>) {
    let values: Vec<Value> = maps.iter().map(|m| m.to_value().without("id")).collect();
    s.document_mut()
        .transact(Label::new("test.maps"), |e| {
            for (i, v) in values.into_iter().enumerate() {
                e.insert(ROOT, "midiMap", i, v)?;
            }
            Ok(())
        })
        .expect("maps");
}

fn setup() -> (TempDir, Session, PortIndex, Vec<NodeId>, Vec<NodeId>) {
    let dir = TempDir::new("midi-everything");
    let mut s = session(&dir);
    let registry = s.registry().clone();
    let mut tracks = Vec::new();
    let channels_n = 2;
    s.document_mut()
        .transact(Label::new("test.setup"), |e| {
            for i in 0..channels_n {
                e.insert(ROOT, "mixer.channels", i, Value::object([("name", Value::text(format!("c{i}")))]))?;
            }
            Ok(())
        })
        .expect("channels");
    let channels: Vec<NodeId> = s
        .document()
        .root()
        .get("mixer")
        .and_then(|m| m.get("channels"))
        .map(|l| l.items().filter_map(|c| NodeId::parse(c.get("id")?.as_text()?).ok()).collect())
        .unwrap_or_default();
    for d in registry.descriptors() {
        let state = registry.defaults(d.id).unwrap_or_else(|_| Value::empty_object());
        let p = Value::object([
            ("plugin", Value::text(d.id)),
            ("version", Value::int(i64::from(registry.version(d.id).unwrap_or(1)))),
            ("state", state),
        ]);
        let kind = registry.kind(d.id);
        s.document_mut()
            .transact(Label::new("test.plugin"), |e| {
                if kind == Some(PluginKind::Instrument) {
                    let n = e.root().get("tracks").map_or(0, Value::len);
                    let t =
                        e.insert(ROOT, "tracks", n, Value::object([("name", Value::text(d.id)), ("instrument", p)]))?;
                    tracks.push(t);
                } else {
                    let c = channels[0];
                    let n = e.find(c).and_then(|c| c.get("inserts").map(Value::len)).unwrap_or(0);
                    e.insert(c, "inserts", n, p)?;
                }
                Ok(())
            })
            .expect("plugin");
    }
    settle(&mut s);
    let list = Value::list([Value::object([
        ("client", Value::text("c")),
        ("port", Value::text("p")),
        ("alias", Value::text("Ctl")),
        ("role", Value::text("controller")),
    ])]);
    s.set_preference(&[PathStep::key("midi"), PathStep::key("ports")], list).expect("ports");
    s.midi_refresh();
    let port = s.midi_port_of("Ctl").unwrap();
    (dir, s, port, tracks, channels)
}

#[test]
fn every_parameter_field_and_action_answers_a_control() {
    let (_dir, mut s, port, tracks, channels) = setup();
    // Every parameter the engine has a slot for: plugins, the rack, faders, pans, the master
    let table = s.table().clone();
    let params: Vec<(NodeId, String)> = table.slots.keys().map(|(n, k)| (*n, k.to_string())).collect();
    assert!(params.len() > 100, "{} parameters", params.len());
    let mut maps: Vec<Mapping> = params
        .iter()
        .enumerate()
        .map(|(i, (n, k))| nrpn(i as u16, MapTarget::Param { node: NodeRef::Id(*n), key: k.clone() }))
        .collect();
    // Every field of a track and a channel a control can set (switches, choices, numbers)
    let mut fields: Vec<(NodeId, String, FieldKind)> = Vec::new();
    for (node, ty) in [(tracks[0], "track"), (channels[1], "mixer.channel")] {
        for f in &s.schemas().find(ty).expect(ty).fields {
            let mappable = matches!(
                f.kind,
                FieldKind::Bool | FieldKind::Choice(_) | FieldKind::Int { .. } | FieldKind::Real { .. }
            );
            if mappable && lm_doc::param_key(ty, &[PathStep::key(f.key)]).is_none() {
                fields.push((node, f.key.to_owned(), f.kind.clone()));
            }
        }
    }
    assert!(fields.len() >= 6, "{fields:?}");
    let base = maps.len() as u16;
    for (i, (n, k, _)) in fields.iter().enumerate() {
        maps.push(nrpn(base + i as u16, MapTarget::Field { node: NodeRef::Id(*n), path: vec![PathStep::key(k)] }));
    }
    // Every action of the registry
    let actions: Vec<&str> = lm_session::actions::all().map(|a| a.id).collect();
    let abase = maps.len() as u16;
    for (i, id) in actions.iter().enumerate() {
        maps.push(nrpn(
            abase + i as u16,
            MapTarget::Action { id: (*id).into(), params: Value::empty_object(), value_into: None },
        ));
    }
    add_all_mappings(&mut s, maps.clone());
    settle(&mut s);
    s.midi_refresh();
    s.midi_dry_run(true);
    // The parameters first (their slots), then the fields and actions (a field may recompile the program)
    let send_range = |s: &mut Session, from: usize, to: usize| {
        let mut msgs = Vec::new();
        for i in from..to {
            msgs.extend(nrpn_max(i as u16));
        }
        send_all(s, port, &msgs);
        run_for(s, Duration::from_millis(200));
    };
    send_range(&mut s, 0, params.len());
    // Parameters: the slot is at the top of the range
    eventually(&mut s, "parameters that did not answer", |s| {
        let table = s.table().clone();
        let mut missing = Vec::new();
        for (n, k) in &params {
            let range = table.ranges[&(*n, std::sync::Arc::from(k.as_str()))];
            let want = range.to_units(1.0);
            let slot = table.slots_of(*n, k)[0];
            let got = s.engine().params().read(slot);
            if (got - want).abs() > 1e-3 * want.abs().max(1.0) {
                missing.push(format!("{n} {k}: {got} (want {want})"));
            }
        }
        missing
    });
    // Their gestures end as document writes, none refused
    run_for(&mut s, Duration::from_millis(700));
    let refused: Vec<&str> =
        s.notices().keys().into_iter().filter(|k| k.starts_with("state.") || k.starts_with("doc.")).collect();
    assert!(refused.is_empty(), "{refused:?}");
    send_range(&mut s, params.len(), maps.len());
    // Fields: true, the last choice, the maximum
    eventually(&mut s, "fields", |s| {
        let mut missing = Vec::new();
        for (n, k, kind) in &fields {
            let v = s.document().find(*n).and_then(|x| x.get(k).cloned()).unwrap_or(Value::NULL);
            let ok = match kind {
                FieldKind::Bool => v.as_bool() == Some(true),
                FieldKind::Choice(o) => v.as_text() == o.last().copied(),
                FieldKind::Int { max, .. } => v.as_int() == Some(*max),
                FieldKind::Real { max, .. } => v.as_real() == Some(*max),
                _ => true,
            };
            if !ok {
                missing.push(format!("{n} {k} = {v:?}"));
            }
        }
        missing
    });
    // Actions: each one dispatched
    let mut dispatched: Vec<std::sync::Arc<str>> = Vec::new();
    eventually(&mut s, "actions not dispatched", |s| {
        dispatched.extend(s.midi_dispatched());
        actions.iter().filter(|id| !dispatched.iter().any(|d| &**d == **id)).map(|id| id.to_string()).collect()
    });
}

#[test]
fn following_the_selection_and_banks_recompile_only_the_map() {
    let (_dir, mut s, port, tracks, _) = setup();
    let target = MapTarget::Param { node: NodeRef::Selected { role: SelRole::Track, offset: 0 }, key: "volume".into() };
    add_all_mappings(&mut s, vec![nrpn(1, target)]);
    settle(&mut s);
    let generation = s.table().generation;
    let volume_slot = |s: &Session, t: NodeId| s.table().slots_of(t, "volume")[0];
    for (k, t) in [tracks[0], tracks[1]].into_iter().enumerate() {
        s.set_ui(&[PathStep::key("selection")], Value::object([("track", Value::text(t.to_string()))])).unwrap();
        run_for(&mut s, Duration::from_millis(50));
        let msgs = [[0xB0, 99, 0], [0xB0, 98, 1], [0xB0, 6, 64 + k as u8], [0xB0, 38, 0]];
        send_all(&mut s, port, &msgs);
        let slot = volume_slot(&s, t);
        let range = s.table().ranges[&(t, std::sync::Arc::from("volume"))];
        let want = range.to_units(f32::from(64 + k as u8) * 128.0 / 16383.0);
        eventually(&mut s, "the selected track moved", |s| {
            let got = s.engine().params().read(slot);
            if (got - want).abs() < 1e-4 { Vec::new() } else { vec![format!("{t}: {got} (want {want})")] }
        });
    }
    assert_eq!(s.table().generation, generation, "no program was compiled: only the map");
    // A bank follows the same way
    s.run_action("midi.bank", &Value::object([("role", Value::text("track")), ("delta", Value::int(2))])).unwrap();
    run_for(&mut s, Duration::from_millis(50));
    assert_eq!(s.table().generation, generation);
}
