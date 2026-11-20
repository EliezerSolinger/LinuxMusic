//! MIDI and automation through the session (interfaces.md, I-21.6): mapped parameters, fields and actions; the end
//! of a MIDI gesture as one undo step; learning; feedback without echo; the Mackie Control against a simulated
//! surface; MIDI recording with its crash log; automation recording, one pass per undo step

mod common;

use std::time::{Duration, Instant};

use common::*;
use lm_base::{HostTime, Label};
use lm_midimap::mcu::{self, Surface};
use lm_midimap::{DocSource, MapScope, MapTarget, Mapping, Message, NodeRef, Transform};
use lm_render::{Command, PortIndex};
use lm_session::Session;
use lm_state::{NodeId, PathStep, ROOT, Value};

fn run_for(s: &mut Session, d: Duration) {
    let start = Instant::now();
    while start.elapsed() < d {
        s.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_until(s: &mut Session, what: &str, mut f: impl FnMut(&mut Session) -> bool) {
    let start = Instant::now();
    while !f(s) {
        assert!(start.elapsed() < Duration::from_secs(20), "{what}: {:?}", s.notices().keys());
        s.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A controller port "Ctl" (and others) in the machine's preferences
fn ports(s: &mut Session, aliases: &[&str]) {
    let list = Value::list(aliases.iter().map(|a| {
        Value::object([
            ("client", Value::text(format!("client {a}"))),
            ("port", Value::text("port")),
            ("alias", Value::text(*a)),
            ("role", Value::text("both")),
        ])
    }));
    s.set_preference(&[PathStep::key("midi"), PathStep::key("ports")], list).expect("ports");
    s.midi_refresh();
}

fn send(s: &mut Session, port: PortIndex, bytes: [u8; 3]) {
    s.engine().commands().send(Command::Midi { port, at: HostTime::now(), bytes }).expect("midi");
}

fn cc(port: &str, number: u8, target: MapTarget) -> Mapping {
    Mapping {
        id: None,
        source: DocSource { port: port.into(), channel: Some(0), message: Message::Cc { number } },
        touch: None,
        target,
        transform: Transform { takeover: lm_midimap::Takeover::Jump, ..Transform::default() },
        feedback: true,
        consume: true,
        feedback_to: None,
    }
}

fn volume(s: &Session, t: NodeId) -> f64 {
    s.document().find(t).and_then(|v| v.get("volume").and_then(Value::as_real)).unwrap_or(-1.0)
}

fn ready(dir: &TempDir) -> (Session, NodeId, PortIndex) {
    let mut s = session(dir);
    let t = add_synth_track(&mut s, "Synth");
    settle(&mut s);
    ports(&mut s, &["Ctl"]);
    let port = s.midi_port_of("Ctl").expect("an index for the alias");
    (s, t, port)
}

#[test]
fn a_mapped_cc_moves_the_parameter_and_its_gesture_is_one_undo_step() {
    let dir = TempDir::new("midi-param");
    let (mut s, t, port) = ready(&dir);
    let target = MapTarget::Param { node: NodeRef::Id(t), key: "volume".into() };
    s.midi_add_mapping(&cc("Ctl", 7, target), MapScope::Project, Label::new("test")).expect("map");
    settle(&mut s);
    let undo_before = s.document().history_len();
    for v in [10u8, 40, 80, 127] {
        send(&mut s, port, [0xB0, 7, v]);
        run_for(&mut s, Duration::from_millis(20));
    }
    // The tree follows the control at each pump (a document gesture: no undo step yet); the step comes when the
    // gesture ends, 500 ms after the last message
    wait_until(&mut s, "the tree follows", |s| volume(s, t) > 3.9);
    assert!((volume(&s, t) - 4.0).abs() < 1e-3, "full scale is the fader's maximum: {}", volume(&s, t));
    assert_eq!(s.document().history_len(), undo_before, "no step while the gesture is open");
    wait_until(&mut s, "the gesture's end", |s| s.document().history_len() == undo_before + 1);
    s.document_mut().undo().expect("undo");
    assert_eq!(volume(&s, t), 1.0);
}

#[test]
fn fields_actions_and_interface_actions() {
    let dir = TempDir::new("midi-fields");
    let (mut s, t, port) = ready(&dir);
    let mute = MapTarget::Field { node: NodeRef::Id(t), path: vec![PathStep::key("mute")] };
    let mut note = cc("Ctl", 0, mute);
    note.source.message = Message::Note { key: 36 };
    s.midi_add_mapping(&note, MapScope::Project, Label::new("test")).expect("map");
    let play = MapTarget::Action { id: "transport.play".into(), params: Value::empty_object(), value_into: None };
    s.midi_add_mapping(&cc("Ctl", 20, play), MapScope::Machine, Label::new("test")).expect("map");
    let zoom = MapTarget::Action { id: "view.zoom_in".into(), params: Value::empty_object(), value_into: None };
    s.midi_add_mapping(&cc("Ctl", 21, zoom), MapScope::Project, Label::new("test")).expect("map");
    settle(&mut s);
    let muted = |s: &mut Session| s.document().find(t).and_then(|v| v.get("mute").and_then(Value::as_bool));
    send(&mut s, port, [0x90, 36, 100]);
    send(&mut s, port, [0x80, 36, 0]);
    wait_until(&mut s, "mute on", |s| muted(s) == Some(true));
    send(&mut s, port, [0x90, 36, 100]);
    send(&mut s, port, [0x80, 36, 0]);
    wait_until(&mut s, "mute off again (a toggle)", |s| muted(s) == Some(false));
    send(&mut s, port, [0xB0, 20, 127]);
    wait_until(&mut s, "the machine's mapping plays", |s| s.telemetry().position.playing);
    send(&mut s, port, [0xB0, 21, 127]);
    let mut ui = Vec::new();
    wait_until(&mut s, "the interface's action", |s| {
        s.drain_midi_ui(&mut ui);
        !ui.is_empty()
    });
    assert_eq!(ui[0].action.as_str(), "view.zoom_in");
    s.stop();
}

#[test]
fn learning_detects_fourteen_bits_and_makes_the_mapping() {
    let dir = TempDir::new("midi-learn");
    let (mut s, t, port) = ready(&dir);
    let target = MapTarget::Param { node: NodeRef::Id(t), key: "volume".into() };
    let id = s.midi_learn(target.clone(), MapScope::Project);
    assert_eq!(s.learning(), Some(id));
    let now = HostTime::now();
    for i in 0..10u64 {
        let at = HostTime(now.0 + i * 5_000_000);
        s.midi_input(port, at, &[0xB0, 7, 20 + i as u8]);
        s.midi_input(port, at, &[0xB0, 39, 3 * i as u8]);
    }
    wait_until(&mut s, "learned", |s| s.learning().is_none());
    let map = s.document().root().get("midiMap").cloned().expect("a project mapping");
    let m = Mapping::from_value(map.at(0).expect("one")).expect("readable");
    assert_eq!(m.source.message, Message::Cc14 { msb: 7 });
    assert_eq!(&*m.source.port, "Ctl");
    assert_eq!(m.target, target);
    // The activity monitor saw the messages
    let (_, seen) = s.midi_activity(0);
    assert!(seen.len() >= 20);
    // Unmapping removes it; undo brings it back
    s.midi_unmap(&target, MapScope::Project).expect("unmap");
    assert_eq!(s.document().root().get("midiMap").map_or(0, Value::len), 0);
    s.document_mut().undo().expect("undo");
    assert_eq!(s.document().root().get("midiMap").map_or(0, Value::len), 1);
}

#[test]
fn feedback_follows_every_change_without_echo() {
    let dir = TempDir::new("midi-feedback");
    let (mut s, t, port) = ready(&dir);
    let mut out = s.take_midi_feedback().expect("the feedback ring");
    let target = MapTarget::Param { node: NodeRef::Id(t), key: "volume".into() };
    s.midi_add_mapping(&cc("Ctl", 7, target), MapScope::Project, Label::new("test")).expect("map");
    settle(&mut s);
    let drain = |out: &mut lm_base::ring::Consumer<lm_render::MidiOutEvent>| {
        let mut v = Vec::new();
        while let Ok(e) = out.pop() {
            v.push(e);
        }
        v
    };
    // Opening sends everything once
    run_for(&mut s, Duration::from_millis(100));
    let first = drain(&mut out);
    assert!(first.iter().any(|e| e.port == port && e.bytes[..2] == [0xB0, 7]), "{first:?}");
    // An edit from anywhere (here a transaction, like undo or the mouse) goes back to the controller
    set(&mut s, t, "volume", Value::real(0.0).unwrap());
    run_for(&mut s, Duration::from_millis(100));
    let after = drain(&mut out);
    assert!(after.iter().any(|e| e.bytes == [0xB0, 7, 0]), "{after:?}");
    // The controller's own value does not come back to it
    send(&mut s, port, [0xB0, 7, 100]);
    run_for(&mut s, Duration::from_millis(150));
    let echo = drain(&mut out);
    assert!(!echo.iter().any(|e| e.bytes == [0xB0, 7, 100]), "an echo: {echo:?}");
    // A reconnected port gets every value again
    s.midi_ports(&[]);
    s.midi_ports(&[port]);
    run_for(&mut s, Duration::from_millis(100));
    assert!(!drain(&mut out).is_empty());
}

#[test]
fn a_mackie_control_against_a_simulated_surface() {
    let dir = TempDir::new("midi-mcu");
    let mut s = session(&dir);
    let names = ["Kick", "Snare", "Bass", "Keys", "Gtr", "Vox", "Pad", "Lead", "Choir", "FX"];
    let tracks: Vec<NodeId> = names.iter().map(|n| add_synth_track(&mut s, n)).collect();
    settle(&mut s);
    ports(&mut s, &["MCU"]);
    let controllers = Value::list([Value::object([("profile", Value::text("mcu")), ("port", Value::text("MCU"))])]);
    s.set_preference(&[PathStep::key("controllers")], controllers).expect("controllers");
    s.midi_refresh();
    let port = s.midi_port_of("MCU").unwrap();
    let mut out = s.take_midi_feedback().unwrap();
    let mut surface = Surface::default();
    let mut feed = |s: &mut Session, surface: &mut Surface| {
        for (p, data) in s.take_midi_sysex() {
            assert_eq!(p, port);
            surface.receive(&data);
        }
        while let Ok(e) = out.pop() {
            surface.receive(&e.bytes[..usize::from(e.len)]);
        }
    };
    run_for(&mut s, Duration::from_millis(200));
    feed(&mut s, &mut surface);
    // The display: names of the bank's tracks, their volumes in dB
    assert_eq!(surface.cell_text(0, 0), "Kick");
    assert_eq!(surface.cell_text(0, 7), "Lead");
    assert_eq!(surface.cell_text(1, 2), "0.0");
    // The motor faders went where the volumes are (0 dB on the gain scale)
    assert!(surface.faders[1] > 12_000, "fader 2 at {}", surface.faders[1]);
    // Touch fader 1 and move it to the bottom: the track's volume follows; release: one undo step
    send(&mut s, port, Surface::touch(1, true));
    send(&mut s, port, Surface::move_fader(1, 0));
    run_for(&mut s, Duration::from_millis(50));
    send(&mut s, port, Surface::touch(1, false));
    wait_until(&mut s, "the fader's gesture ends", |s| volume(s, tracks[1]) == 0.0);
    // Mute 3 lights its LED; the V-pot turns the pan of the bank's third channel (none here: nothing breaks)
    send(&mut s, port, Surface::press(mcu::MUTE + 3, true));
    send(&mut s, port, Surface::press(mcu::MUTE + 3, false));
    wait_until(&mut s, "mute 3", |s| {
        s.document().find(tracks[3]).and_then(|v| v.get("mute").and_then(Value::as_bool)) == Some(true)
    });
    run_for(&mut s, Duration::from_millis(100));
    feed(&mut s, &mut surface);
    assert!(surface.leds[usize::from(mcu::MUTE + 3)], "the mute LED");
    send(&mut s, port, Surface::turn(0, 3));
    // Bank right: the strips show the next eight tracks
    send(&mut s, port, Surface::press(mcu::BANK_RIGHT, true));
    send(&mut s, port, Surface::press(mcu::BANK_RIGHT, false));
    wait_until(&mut s, "bank right", |s| {
        feed(s, &mut surface);
        surface.cell_text(0, 0) == "Choir"
    });
    assert_eq!(surface.cell_text(0, 1), "FX");
    assert_eq!(surface.cell_text(0, 2), "");
    assert_eq!(surface.assignment, [mcu::segment('0'), mcu::segment('2')]);
    // Select 1 of the new bank selects "FX" in the interface document
    send(&mut s, port, Surface::press(mcu::SELECT + 1, true));
    wait_until(&mut s, "select", |s| {
        s.ui_document().root().get("selection").and_then(|x| x.get("track")).and_then(Value::as_text)
            == Some(&tracks[9].to_string())
    });
}

#[test]
fn midi_recording_is_one_undo_step_and_a_crash_log_comes_back() {
    let dir = TempDir::new("midi-record");
    let (mut s, t, port) = ready(&dir);
    set(&mut s, t, "armed", Value::bool(true));
    settle(&mut s);
    s.record(None).expect("record");
    wait_until(&mut s, "recording", |s| s.telemetry().position.recording);
    send(&mut s, port, [0x90, 60, 100]);
    run_for(&mut s, Duration::from_millis(100));
    send(&mut s, port, [0xB0, 1, 64]);
    send(&mut s, port, [0x80, 60, 0]);
    send(&mut s, port, [0x90, 64, 90]); // held: ends at the stop
    run_for(&mut s, Duration::from_millis(100));
    s.stop();
    let midi_items = |s: &mut Session| -> Vec<Value> {
        s.document()
            .find(t)
            .and_then(|v| v.get("items").cloned())
            .map(|l| l.items().filter(|i| i.get("kind").and_then(Value::as_text) == Some("midi")).cloned().collect())
            .unwrap_or_default()
    };
    wait_until(&mut s, "the take", |s| !midi_items(s).is_empty());
    let item = midi_items(&mut s).remove(0);
    assert_eq!(item.get("notes").map(Value::len), Some(2));
    let controls = item.get("controls").expect("the CC lane");
    assert_eq!(controls.at(0).and_then(|c| c.get("number")).and_then(Value::as_int), Some(1));
    // No log is left behind, and one undo removes the whole recording
    let audio = s.project().folder.audio();
    let logs = || {
        std::fs::read_dir(&audio)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "midilog"))
            .count()
    };
    assert_eq!(logs(), 0);
    s.document_mut().undo().expect("undo");
    assert!(midi_items(&mut s).is_empty());
    // A log a crash left: back as a recovered take
    let log = format!("midilog 1 0 0 -1 -1 overdub 0 0 false\n0 0 {t} 144 62 100\n24000 24000 {t} 128 62 0\n");
    std::fs::write(audio.join("rec-crash.midilog"), log).unwrap();
    s.recover_takes().expect("recover");
    let items = midi_items(&mut s);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].get("name").and_then(Value::as_text), Some("Take recuperado"));
    assert_eq!(logs(), 0);
}

#[test]
fn automation_touch_records_one_pass_per_undo_step() {
    let dir = TempDir::new("midi-automation");
    let (mut s, t, port) = ready(&dir);
    let target = MapTarget::Param { node: NodeRef::Id(t), key: "volume".into() };
    s.midi_add_mapping(&cc("Ctl", 7, target), MapScope::Project, Label::new("test")).expect("map");
    s.run_action("automation.global", &Value::object([("mode", Value::text("touch"))])).expect("touch");
    settle(&mut s);
    let undo_before = s.document().history_len();
    s.play();
    wait_until(&mut s, "playing", |s| s.telemetry().position.playing);
    for v in [20u8, 60, 100, 127, 90] {
        send(&mut s, port, [0xB0, 7, v]);
        run_for(&mut s, Duration::from_millis(40));
    }
    // The pass ends with the gesture (500 ms after the last message), as one transaction
    let lane = |s: &mut Session| s.document().find(t).and_then(|v| v.get("automation").and_then(|l| l.at(0).cloned()));
    wait_until(&mut s, "the lane", |s| lane(s).is_some());
    let points = lane(&mut s).unwrap().get("points").map(Value::len).unwrap_or(0);
    assert!(points >= 2, "{points} points");
    s.stop();
    run_for(&mut s, Duration::from_millis(100));
    // The global mode, the gesture's document write and the pass: the pass is one step of its own
    assert!(s.document().history_len() > undo_before);
    let label = s.document().undo_label().map(|l| l.key);
    assert!(matches!(label, Some("automation.record" | "midi.gesture")), "{label:?}");
    // Re-arm clears "the hand won"
    s.run_action("automation.rearm", &Value::empty_object()).expect("rearm");
    assert!(s.hand_won().is_empty());
    let _ = ROOT;
}

/// S20: a control moved while its plugin is rebuilt (a wave shape changed in the middle of the gesture) keeps its
/// value — the tree follows the gesture at each pump, so the new program and the new processor start from it; the
/// gesture is still one undo step
#[test]
fn a_control_moved_during_a_rebuild_keeps_its_value() {
    let dir = TempDir::new("midi-s20");
    let (mut s, t, port) = ready(&dir);
    let synth = s
        .document()
        .find(t)
        .and_then(|v| v.get("instrument").and_then(|i| i.get("id")).and_then(Value::as_text).map(str::to_owned))
        .and_then(|x| NodeId::parse(&x).ok())
        .expect("the instrument");
    let target = MapTarget::Param { node: NodeRef::Id(synth), key: "vol1".into() };
    s.midi_add_mapping(&cc("Ctl", 7, target), MapScope::Project, Label::new("test")).expect("map");
    settle(&mut s);
    let undo_before = s.document().history_len();
    let vol = |s: &Session| {
        s.document()
            .find(synth)
            .and_then(|p| p.at_path(&[PathStep::key("state"), PathStep::key("params"), PathStep::key("vol1")]).cloned())
            .and_then(|v| v.as_real())
            .unwrap_or(-1.0)
    };
    send(&mut s, port, [0xB0, 7, 20]);
    run_for(&mut s, Duration::from_millis(60));
    // The tree follows the control while the gesture is open
    wait_until(&mut s, "the tree follows", |s| (vol(s) - 200.0 * 20.0 / 127.0).abs() < 0.01);
    // A rebuild in the middle: another wave shape (a new processor, built from the tree)
    s.document_mut()
        .transact(Label::new("test.wave"), |e| {
            e.set(synth, &[PathStep::key("state"), PathStep::key("wave1")], Value::text("saw"))
        })
        .expect("wave");
    for v in [40u8, 80, 100] {
        send(&mut s, port, [0xB0, 7, v]);
        run_for(&mut s, Duration::from_millis(30));
    }
    settle(&mut s); // the new processor is built and installed while the gesture is still open
    let want = 200.0 * 100.0 / 127.0;
    let slot = s.table().slots_of(synth, "vol1")[0];
    assert!((f64::from(s.engine().params().read(slot)) - want).abs() < 0.01, "the slot kept the moved value");
    // The gesture ends: the final value in the tree, one undo step for it
    wait_until(&mut s, "the gesture's end", |s| s.document().history_len() == undo_before + 2);
    assert!((vol(&s) - want).abs() < 0.01, "the tree has the moved value: {}", vol(&s));
    assert!((f64::from(s.engine().params().read(slot)) - want).abs() < 0.01);
    assert_eq!(s.document().undo_label().map(|l| l.key), Some("midi.gesture"));
    // T4: the wave change entered as its own step with the gesture's value at that moment; undo walks back through
    // states that existed
    s.document_mut().undo().expect("undo");
    assert!((vol(&s) - 200.0 * 20.0 / 127.0).abs() < 0.01, "back to the value when the wave changed: {}", vol(&s));
    s.document_mut().undo().expect("undo");
    assert!((vol(&s) - 20.0).abs() < 0.01, "and to before the gesture: {}", vol(&s));
}

fn notes_of(s: &Session, item: NodeId) -> Vec<(i64, i64, i64)> {
    let mut v: Vec<(i64, i64, i64)> = s
        .document()
        .find(item)
        .and_then(|i| i.get("notes").cloned())
        .map(|n| {
            n.entries()
                .map(|(_, x)| {
                    let g = |k: &str| x.get(k).and_then(Value::as_int).unwrap_or(-1);
                    (g("start"), g("key"), g("length"))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// F-PAS-001: step recording in the core — chords at the cursor when the last key goes up, rests, lengthening,
/// going back; each chord one undo step; the cursor in the interface document
#[test]
fn step_recording() {
    let dir = TempDir::new("midi-step");
    let (mut s, t, port) = ready(&dir);
    let item = add_notes(&mut s, t, &[]);
    let p = |pairs: Vec<(&str, Value)>| Value::object(pairs);
    s.run_action("notes.step_record", &p(vec![("item", Value::text(item.to_string())), ("step", Value::int(240))]))
        .expect("step on");
    let cursor = |s: &Session| {
        s.ui_document().root().get("stepRecord").and_then(|c| c.get("at")).and_then(Value::as_int).unwrap_or(-1)
    };
    assert_eq!(cursor(&s), 0);
    let undo_before = s.document().history_len();
    let now = HostTime::now();
    for b in [[0x90u8, 60, 100], [0x90, 64, 100], [0x80, 60, 0], [0x80, 64, 0]] {
        s.midi_input(port, now, &b);
    }
    assert_eq!(notes_of(&s, item), vec![(0, 60, 240), (0, 64, 240)], "a chord at the cursor");
    assert_eq!(cursor(&s), 240);
    assert_eq!(s.document().history_len(), undo_before + 1, "one undo step per chord");
    s.run_action("notes.step_rest", &Value::empty_object()).expect("rest");
    assert_eq!(cursor(&s), 480);
    // The computer keyboard's chord, then → lengthens it
    s.run_action("notes.step_keys", &p(vec![("keys", Value::list([Value::int(67)]))])).expect("keys");
    s.run_action("notes.step_length", &p(vec![("delta", Value::int(240))])).expect("longer");
    assert!(notes_of(&s, item).contains(&(480, 67, 480)));
    assert_eq!(cursor(&s), 960);
    // Back: the last chord goes, the cursor returns
    s.run_action("notes.step_back", &Value::empty_object()).expect("back");
    assert!(!notes_of(&s, item).iter().any(|n| n.1 == 67));
    assert_eq!(cursor(&s), 480);
    // Another port's keys, when the track listens to one port only, are not recorded
    ports(&mut s, &["Ctl", "Other"]);
    let other = s.midi_port_of("Other").unwrap();
    set(&mut s, t, "midiInput", Value::object([("ports", Value::list([Value::text("Ctl")]))]));
    s.midi_input(other, now, &[0x90, 72, 100]);
    s.midi_input(other, now, &[0x80, 72, 0]);
    assert!(!notes_of(&s, item).iter().any(|n| n.1 == 72));
    s.run_action("notes.step_stop", &Value::empty_object()).expect("off");
    assert!(s.ui_document().root().get("stepRecord").is_none_or(Value::is_null));
}

/// F-NOT-018, F-REC-029: a recording goes into the item open in the Piano Roll, not into a new item
#[test]
fn recording_into_the_open_item() {
    let dir = TempDir::new("midi-into");
    let (mut s, t, port) = ready(&dir);
    let item = add_notes(&mut s, t, &[(48, 0, 240)]);
    set(&mut s, t, "armed", Value::bool(true));
    s.run_action("notes.record_into", &Value::object([("item", Value::text(item.to_string()))])).expect("into");
    settle(&mut s);
    let midi_items = |s: &Session| {
        s.document()
            .find(t)
            .and_then(|v| v.get("items").cloned())
            .map(|l| l.items().filter(|i| i.get("kind").and_then(Value::as_text) == Some("midi")).count())
            .unwrap_or(0)
    };
    s.record(None).expect("record");
    wait_until(&mut s, "recording", |s| s.telemetry().position.recording);
    send(&mut s, port, [0x90, 60, 100]);
    run_for(&mut s, Duration::from_millis(80));
    send(&mut s, port, [0x80, 60, 0]);
    send(&mut s, port, [0xB0, 74, 50]);
    run_for(&mut s, Duration::from_millis(50));
    s.stop();
    wait_until(&mut s, "the take in the item", |s| notes_of(s, item).len() == 2);
    assert_eq!(midi_items(&s), 1, "no new MIDI item");
    assert!(notes_of(&s, item).iter().any(|n| n.1 == 60));
    let controls = s.document().find(item).and_then(|i| i.get("controls").cloned()).expect("its control lane");
    assert_eq!(controls.at(0).and_then(|c| c.get("number")).and_then(Value::as_int), Some(74));
}

/// I-21.1: the devices' settings from the preferences — an alias keeps its index for the whole session (through
/// reordering and disconnection), a port without an alias has one too, and the program's own input is there
#[test]
fn port_settings_keep_their_indices() {
    let dir = TempDir::new("midi-ports");
    let mut s = session(&dir);
    let port = |client: &str, alias: &str, role: &str| {
        Value::object([
            ("client", Value::text(client)),
            ("port", Value::text("MIDI 1")),
            ("alias", Value::text(alias)),
            ("role", Value::text(role)),
            ("inputLatencyMs", Value::real(2.5).unwrap()),
        ])
    };
    let set_ports = |s: &mut Session, list: Vec<Value>| {
        s.set_preference(&[PathStep::key("midi"), PathStep::key("ports")], Value::list(list)).unwrap()
    };
    set_ports(&mut s, vec![port("Keystation", "Teclado", "instrument"), port("nanoKONTROL", "", "controller")]);
    let a = s.midi_port_settings();
    assert_eq!(a.len(), 3);
    assert_eq!(a[0].alias, "Teclado");
    assert_eq!(a[0].input_latency_ns, 2_500_000);
    assert_eq!(a[2].id, lm_io::PortId::new(lm_io::CLIENT_NAME, lm_io::OWN_INPUT));
    assert_eq!(Some(a[0].index), s.midi_port_of("Teclado"));
    // Reordered, and a new port added: the old ones keep their indices
    set_ports(
        &mut s,
        vec![
            port("Other", "Pads", "both"),
            port("nanoKONTROL", "", "controller"),
            port("Keystation", "Teclado", "instrument"),
        ],
    );
    let b = s.midi_port_settings();
    let index_of = |v: &[lm_io::PortSettings], client: &str| v.iter().find(|p| p.id.client == client).unwrap().index;
    assert_eq!(index_of(&b, "Keystation"), a[0].index);
    assert_eq!(index_of(&b, "nanoKONTROL"), a[1].index);
    assert_ne!(index_of(&b, "Other"), a[0].index);
    assert_eq!(b.iter().filter(|p| p.index == index_of(&b, "Other")).count(), 1);
}
