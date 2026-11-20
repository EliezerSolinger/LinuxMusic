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
    // The slot moved at once; the document only when the gesture ends (500 ms after the last message)
    assert_eq!(volume(&s, t), 1.0);
    wait_until(&mut s, "the gesture's end", |s| volume(s, t) > 3.9);
    assert!((volume(&s, t) - 4.0).abs() < 1e-3, "full scale is the fader's maximum: {}", volume(&s, t));
    assert_eq!(s.document().history_len(), undo_before + 1, "one undo step for the whole gesture");
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
