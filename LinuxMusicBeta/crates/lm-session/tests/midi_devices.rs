//! The MIDI devices wired into the session (I-21.1), through a fake device service with lm-io's own types: the
//! settings from the preferences, the input ring installed in the engine, unplugging and replugging (the mappings
//! stay, everything comes back by itself, with a notice; feedback is sent again), learning from the activity, an
//! alias of another machine asked for and assigned, `status.midiPorts`

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use lm_base::ring::Consumer;
use lm_base::{HostTime, Label, Result};
use lm_io::{MidiActivity, MidiDeviceEvent, PortId, PortInfo, PortSettings, SysexIn, SysexOut};
use lm_midimap::{DocSource, MapScope, MapTarget, Mapping, Message, NodeRef, Transform};
use lm_render::{Command, MidiOutEvent, PortIndex, ProducerIndex};
use lm_session::{MidiDeviceLink, Session};
use lm_state::{PathStep, Value};

#[derive(Default)]
struct State {
    settings: Vec<PortSettings>,
    events: Vec<MidiDeviceEvent>,
    activity: Vec<MidiActivity>,
    /// The output sources given: the session's feedback, then the renderer's MIDI out
    outputs: Vec<Consumer<MidiOutEvent>>,
    sysex: Option<Consumer<SysexOut>>,
    wakes: usize,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl MidiDeviceLink for Fake {
    fn set_settings(&mut self, settings: Vec<PortSettings>) -> Result<()> {
        self.0.lock().unwrap().settings = settings;
        Ok(())
    }
    fn poll(&mut self, f: &mut dyn FnMut(MidiDeviceEvent)) {
        for e in std::mem::take(&mut self.0.lock().unwrap().events) {
            f(e);
        }
    }
    fn activity(&mut self, f: &mut dyn FnMut(MidiActivity)) {
        for a in std::mem::take(&mut self.0.lock().unwrap().activity) {
            f(a);
        }
    }
    fn sysex(&mut self, _f: &mut dyn FnMut(SysexIn)) {}
    fn add_output_source(&mut self, source: Consumer<MidiOutEvent>) -> Result<()> {
        self.0.lock().unwrap().outputs.push(source);
        Ok(())
    }
    fn add_sysex_output(&mut self, source: Consumer<SysexOut>) -> Result<()> {
        self.0.lock().unwrap().sysex = Some(source);
        Ok(())
    }
    fn wake(&self) {
        self.0.lock().unwrap().wakes += 1;
    }
    fn ports(&self) -> Vec<PortInfo> {
        self.0
            .lock()
            .unwrap()
            .settings
            .iter()
            .map(|s| PortInfo { id: s.id.clone(), backend: s.backend, readable: true, writable: true, hardware: true })
            .collect()
    }
    fn rescan_raw(&mut self) {}
}

impl Fake {
    fn feedback(&self) -> Vec<MidiOutEvent> {
        let mut st = self.0.lock().unwrap();
        let mut out = Vec::new();
        for c in st.outputs.iter_mut() {
            while let Ok(e) = c.pop() {
                out.push(e);
            }
        }
        out
    }
    fn event(&self, e: MidiDeviceEvent) {
        self.0.lock().unwrap().events.push(e);
    }
    fn activity(&self, port: PortIndex, bytes: [u8; 3]) {
        self.0.lock().unwrap().activity.push(MidiActivity { port, at: HostTime::now(), len: 3, bytes });
    }
}

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

fn prefs_ports(s: &mut Session, list: &[(&str, &str)]) {
    let v = Value::list(list.iter().map(|(client, alias)| {
        Value::object([
            ("client", Value::text(*client)),
            ("port", Value::text("MIDI 1")),
            ("alias", Value::text(*alias)),
            ("role", Value::text("both")),
        ])
    }));
    s.set_preference(&[PathStep::key("midi"), PathStep::key("ports")], v).unwrap();
}

#[test]
fn devices_wired_into_the_session() {
    let dir = TempDir::new("midi-devices");
    let mut s = session(&dir);
    let t = add_synth_track(&mut s, "Synth");
    settle(&mut s);
    prefs_ports(&mut s, &[("nanoKONTROL", "Ctl")]);
    let fake = Fake::default();
    let (mut input, receiver) = lm_render::command_ring(ProducerIndex(1), 1024);
    s.attach_midi_devices(Box::new(fake.clone()), receiver).expect("attach");
    let settings = fake.0.lock().unwrap().settings.clone();
    assert!(settings.iter().any(|p| p.alias == "Ctl" && p.id == PortId::new("nanoKONTROL", "MIDI 1")));
    assert!(settings.iter().any(|p| p.id == PortId::new(lm_io::CLIENT_NAME, lm_io::OWN_INPUT)));
    let ctl = s.midi_port_of("Ctl").unwrap();
    let target = MapTarget::Param { node: NodeRef::Id(t), key: "volume".into() };
    s.midi_add_mapping(&cc("Ctl", 7, target), MapScope::Project, Label::new("test")).expect("map");
    fake.event(MidiDeviceEvent::Connected { index: ctl, id: PortId::new("nanoKONTROL", "MIDI 1") });
    settle(&mut s);
    // The device's ring is the engine's: a control moves the track
    let volume = |s: &Session| s.document().find(t).and_then(|v| v.get("volume").and_then(Value::as_real)).unwrap();
    input.send(Command::Midi { port: ctl, at: HostTime::now(), bytes: [0xB0, 7, 0] }).expect("midi");
    wait_until(&mut s, "the control moved the track", |s| volume(s) == 0.0);
    // status.midiPorts
    let ports = s.status().get("midiPorts").cloned().expect("midiPorts");
    assert!(ports.items().any(|p| p.get("alias").and_then(Value::as_text) == Some("Ctl")
        && p.get("connected").and_then(Value::as_bool) == Some(true)));
    // Unplugged: a notice, the mapping stays; plugged back: a notice, and the controller gets every value again
    fake.event(MidiDeviceEvent::Disconnected { index: ctl, id: PortId::new("nanoKONTROL", "MIDI 1") });
    run_for(&mut s, Duration::from_millis(50));
    assert!(s.notices().keys().contains(&"midi.port.disconnected"));
    assert_eq!(s.document().root().get("midiMap").map_or(0, Value::len), 1, "the mapping stays");
    fake.feedback();
    fake.event(MidiDeviceEvent::Connected { index: ctl, id: PortId::new("nanoKONTROL", "MIDI 1") });
    run_for(&mut s, Duration::from_millis(100));
    assert!(s.notices().keys().contains(&"midi.port.reconnected"));
    let back = fake.feedback();
    assert!(back.iter().any(|e| e.port == ctl && e.bytes[..2] == [0xB0, 7]), "resent: {back:?}");
    assert!(fake.0.lock().unwrap().wakes > 0, "the output thread is woken");
    // Learning hears the devices' activity
    let pan = MapTarget::Param { node: NodeRef::Id(t), key: "volume".into() };
    s.midi_learn(pan, MapScope::Machine);
    fake.activity(ctl, [0x90, 40, 100]);
    fake.activity(ctl, [0x80, 40, 0]);
    wait_until(&mut s, "learned from the devices", |s| s.learning().is_none());
    assert_eq!(s.preferences().get(&["midiMap"]).map_or(0, Value::len), 1);
}

#[test]
fn an_alias_of_another_machine_is_asked_for_and_assigned() {
    let dir = TempDir::new("midi-alias");
    let mut s = session(&dir);
    let t = add_synth_track(&mut s, "Synth");
    settle(&mut s);
    let fake = Fake::default();
    let (_input, receiver) = lm_render::command_ring(ProducerIndex(1), 64);
    s.attach_midi_devices(Box::new(fake.clone()), receiver).expect("attach");
    // The project came from a machine where "Teclado" was a port; here nothing has that alias
    let target = MapTarget::Param { node: NodeRef::Id(t), key: "volume".into() };
    s.midi_add_mapping(&cc("Teclado", 7, target), MapScope::Project, Label::new("test")).expect("map");
    run_for(&mut s, Duration::from_millis(50));
    let notice = s.notices().all().into_iter().find(|n| n.key == "midi.alias.unknown").expect("asked once");
    assert_eq!(notice.actions[0].0.as_str(), "midi.assign_alias");
    // The user picks this machine's port for it
    let assign = Value::object([
        ("alias", Value::text("Teclado")),
        ("client", Value::text("Keystation")),
        ("port", Value::text("MIDI 1")),
    ]);
    s.run_action("midi.assign_alias", &assign).expect("assign");
    run_for(&mut s, Duration::from_millis(50));
    let index = s.midi_port_of("Teclado").expect("the alias has a port now");
    let settings = fake.0.lock().unwrap().settings.clone();
    assert!(settings.iter().any(|p| p.index == index && p.id == PortId::new("Keystation", "MIDI 1")));
    assert!(s.midi_mappings_of(t).iter().any(|m| &*m.source.port == "Teclado"), "the mapping resolves");
}

/// P-38 and F-IJN-017: a track's MIDI output (its program sent once, its notes on its channel with the fixed note)
/// and its input (only its port and channel, at the fixed velocity, recorded so)
#[test]
fn a_tracks_midi_input_and_output() {
    let dir = TempDir::new("midi-io");
    let mut s = session(&dir);
    let t = add_synth_track(&mut s, "Synth");
    prefs_ports(&mut s, &[("Keystation", "Keys"), ("Other", "Other"), ("Moog", "Synth")]);
    let fake = Fake::default();
    let (mut input, receiver) = lm_render::command_ring(ProducerIndex(1), 1024);
    s.attach_midi_devices(Box::new(fake.clone()), receiver).expect("attach");
    let (keys, other, synth) =
        (s.midi_port_of("Keys").unwrap(), s.midi_port_of("Other").unwrap(), s.midi_port_of("Synth").unwrap());
    set(&mut s, t, "armed", Value::bool(true));
    set(
        &mut s,
        t,
        "midiInput",
        Value::object([
            ("ports", Value::list([Value::text("Keys")])),
            ("channel", Value::int(2)),
            ("fixedVelocity", Value::int(99)),
        ]),
    );
    set(
        &mut s,
        t,
        "midiOutput",
        Value::object([
            ("on", Value::bool(true)),
            ("ports", Value::list([Value::text("Synth")])),
            ("channel", Value::int(4)),
            ("fixedNote", Value::int(50)),
            ("program", Value::int(9)),
        ]),
    );
    settle(&mut s);
    run_for(&mut s, Duration::from_millis(50));
    let out = fake.feedback();
    assert_eq!(out.iter().filter(|e| e.port == synth && e.bytes[..2] == [0xC4, 9]).count(), 1, "the program, once");
    // Keys on channel 3 (2 counted from 0) play and go out; another port or channel does not
    let send = |input: &mut lm_render::CommandSender, port, bytes| {
        input.send(Command::Midi { port, at: HostTime::now(), bytes }).expect("midi")
    };
    send(&mut input, keys, [0x92, 60, 20]);
    send(&mut input, other, [0x92, 61, 20]);
    send(&mut input, keys, [0x93, 62, 20]);
    run_for(&mut s, Duration::from_millis(80));
    send(&mut input, keys, [0x82, 60, 0]);
    run_for(&mut s, Duration::from_millis(80));
    let out: Vec<[u8; 3]> = fake.feedback().iter().filter(|e| e.port == synth).map(|e| e.bytes).collect();
    assert_eq!(out, vec![[0x94, 50, 99], [0x84, 50, 0]], "out on channel 5, the fixed note, the fixed velocity");
    // Recorded as played through the input: its channel only, at the fixed velocity
    s.record(None).expect("record");
    wait_until(&mut s, "recording", |s| s.telemetry().position.recording);
    send(&mut input, keys, [0x92, 64, 10]);
    send(&mut input, keys, [0x93, 65, 10]);
    run_for(&mut s, Duration::from_millis(80));
    send(&mut input, keys, [0x82, 64, 0]);
    run_for(&mut s, Duration::from_millis(80));
    s.stop();
    let notes = |s: &Session| -> Vec<(i64, f64)> {
        s.document()
            .find(t)
            .and_then(|v| v.get("items").cloned())
            .map(|l| {
                l.items()
                    .filter(|i| i.get("kind").and_then(Value::as_text) == Some("midi"))
                    .flat_map(|i| {
                        i.get("notes")
                            .map(|n| n.entries().map(|(_, x)| x.clone()).collect::<Vec<_>>())
                            .unwrap_or_default()
                    })
                    .map(|x| {
                        (
                            x.get("key").and_then(Value::as_int).unwrap(),
                            x.get("velocity").and_then(Value::as_real).unwrap(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    wait_until(&mut s, "the take", |s| !notes(s).is_empty());
    let got = notes(&s);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, 64);
    assert!((got[0].1 - 99.0 / 127.0).abs() < 1e-6);
    // The program again when the port comes back
    s.midi_ports(&[]);
    s.midi_ports(&[synth]);
    run_for(&mut s, Duration::from_millis(30));
    assert!(fake.feedback().iter().any(|e| e.port == synth && e.bytes[..2] == [0xC4, 9]));
}
