//! The LocalPort (I-15), paste between projects (H13), the loading states with the injected clock (I-2, T10), and
//! the session made and destroyed while playing

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use lm_base::{HostTime, Label, ManualClock};
use lm_port::{ActionId, CorePort, EditOp, Loading, Pending, Ref, Target};
use lm_session::{LocalPort, Session};
use lm_state::{NodeId, PathStep, ROOT, Value};

fn synth() -> Value {
    Value::object([
        ("name", Value::text("Port")),
        (
            "instrument",
            Value::object([
                ("plugin", Value::text("tripleoscillator")),
                ("version", Value::int(1)),
                ("state", Value::empty_object()),
            ]),
        ),
    ])
}

fn instrument_of(port: &LocalPort, track: NodeId) -> NodeId {
    let inst = port.find(track).and_then(|t| t.get("instrument").cloned()).expect("instrument");
    NodeId::parse(inst.get("id").and_then(Value::as_text).expect("id")).expect("p:")
}

#[test]
fn edits_as_data_undo_redo_gestures_and_parameters() {
    let dir = TempDir::new("port");
    let mut port = LocalPort::new(session(&dir));
    let r = port.apply(
        Label::new("add"),
        vec![
            EditOp::Insert { temp: 1, parent: Ref::Id(ROOT), list: "tracks".into(), index: 0, node: synth() },
            EditOp::Set { id: Ref::Temp(1), path: vec![PathStep::key("volume")], value: Value::real(0.5).expect("r") },
        ],
    );
    let Pending::Ready(Ok(applied)) = r else { panic!("apply") };
    let track = applied.created[0].1;
    assert_eq!(port.find(track).and_then(|t| t.get("volume").and_then(Value::as_real)), Some(0.5));
    assert!(port.can_undo());
    let _ = port.undo();
    assert!(port.find(track).is_none());
    let _ = port.redo();
    assert!(port.find(track).is_some());
    let s = port.session_mut();
    settle(s);
    let p = instrument_of(&port, track);
    // A plugin parameter, through the edit: its slot exists once the instrument is installed
    let set = EditOp::Set {
        id: Ref::Id(p),
        path: vec![PathStep::key("state"), PathStep::key("params"), PathStep::key("vol1")],
        value: Value::real(80.0).expect("r"),
    };
    assert!(port.apply(Label::new("vol"), vec![set]).ok().is_some());
    assert!(!port.session().table().slots_of(p, "vol1").is_empty());
    // A gesture: many updates, one step
    let g = port.begin_gesture(Label::new("drag"));
    for v in [0.1, 0.2, 0.3] {
        port.gesture_apply(&g, vec![EditOp::set(track, &["volume"], Value::real(v).expect("r"))]);
    }
    port.gesture_commit(g);
    port.pump();
    assert_eq!(port.find(track).and_then(|t| t.get("volume").and_then(Value::as_real)), Some(0.3));
    let _ = port.undo();
    assert_eq!(port.find(track).and_then(|t| t.get("volume").and_then(Value::as_real)), Some(0.5));
    // The status and preferences trees have their keys
    assert!(port.status().get("project").is_some() && port.preferences().get("audio").is_some());
}

#[test]
fn paste_between_projects_never_repeats_ids() {
    let dir = TempDir::new("paste");
    let mut port = LocalPort::new(session(&dir));
    let folder = |n: &str| Value::object([("folder", Value::text(dir.join(n).display().to_string()))]);
    assert!(port.run_action(ActionId::from("project.new"), folder("a")).ok().is_some());
    let Pending::Ready(Ok(applied)) = port.apply(
        Label::new("add"),
        vec![EditOp::Insert { temp: 1, parent: Ref::Id(ROOT), list: "tracks".into(), index: 0, node: synth() }],
    ) else {
        panic!()
    };
    let track = applied.created[0].1;
    assert!(port.run_action(ActionId::from("edit.copy"), Value::text(track.to_string())).ok().is_some());
    assert!(port.run_action(ActionId::from("project.new"), folder("b")).ok().is_some());
    // Pasted twice into another project, and once more in the same one
    for _ in 0..2 {
        assert!(port.run_action(ActionId::from("edit.paste"), Value::empty_object()).ok().is_some());
    }
    let tracks = port.root().get("tracks").expect("tracks").clone();
    assert_eq!(tracks.len(), 2);
    let mut ids = Vec::new();
    port.root().walk(&mut |_, v| {
        if let Some(t) = v.as_text()
            && let Ok(id) = NodeId::parse(t)
        {
            ids.push(id);
        }
    });
    // Every id given out exactly once (references count their target again, so compare the nodes' own ids)
    let own: Vec<NodeId> = port.session().document().index().ids().map(|(id, _)| *id).collect();
    let mut sorted = own.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), own.len());
    settle(port.session_mut());
}

#[test]
fn a_node_never_shows_a_state_in_between_with_the_injected_clock() {
    let dir = TempDir::new("loading");
    let clock = ManualClock::new(HostTime(1_000_000_000));
    let mut config = config(&dir);
    config.clock = Arc::new(clock.clone());
    config.delay = Duration::from_millis(400);
    let mut s = Session::new(config).expect("session");
    let track = add_synth_track(&mut s, "Slow");
    s.pump();
    let p = {
        let inst = s.document().find(track).and_then(|t| t.get("instrument").cloned()).expect("instrument");
        NodeId::parse(inst.get("id").and_then(Value::as_text).expect("id")).expect("p:")
    };
    let mut saw_pending = false;
    let mut saw_visible = false;
    let start = std::time::Instant::now();
    loop {
        s.pump();
        let l = s.loading(&Target::Node(p));
        match l {
            Loading::Ready => {
                assert!(!s.is_waiting(p), "ready while still being built");
                if !s.settled() {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                break;
            }
            Loading::Pending => {
                assert!(s.is_waiting(p));
                saw_pending = true;
                // The clock moves only when the test moves it: past 200 ms the loading shows
                clock.advance_millis(50);
            }
            Loading::Visible { .. } => {
                assert!(s.is_waiting(p));
                saw_visible = true;
            }
            other => panic!("{other:?}"),
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(saw_pending && saw_visible);
}

#[test]
fn a_session_made_and_destroyed_while_playing_many_times() {
    let cycles: usize = std::env::var("LM_SESSION_CYCLES").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
    let dir = TempDir::new("cycles");
    for i in 0..cycles {
        let mut s = Session::new(config(&dir)).expect("session");
        if i % 50 == 0 {
            add_synth_track(&mut s, "Cycle");
        }
        s.play();
        for _ in 0..3 {
            s.pump();
        }
        drop(s);
    }
}
