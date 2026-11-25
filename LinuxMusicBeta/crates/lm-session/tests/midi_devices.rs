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
    feedback: Option<Consumer<MidiOutEvent>>,
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
        self.0.lock().unwrap().feedback = Some(source);
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
        if let Some(c) = st.feedback.as_mut() {
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
