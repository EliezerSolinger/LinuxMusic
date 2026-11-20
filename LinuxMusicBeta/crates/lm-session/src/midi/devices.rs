//! The MIDI devices' settings from the machine's preferences (I-21.1): every port of `midi.ports` with the index the
//! session gives it for the whole session (an alias keeps its index through disconnections; a port without an alias
//! gets one by its stable id), and the program's own input port, for programs that connect to it.

use std::sync::Arc;

use lm_io::{CLIENT_NAME, MidiBackendKind, OWN_INPUT, PortId, PortRole, PortSettings};
use lm_render::PortIndex;
use lm_state::Value;

use crate::session::Session;

/// The key of a port without an alias in the session's index table (never a valid alias: it starts with a NUL)
fn id_key(client: &str, port: &str) -> Arc<str> {
    Arc::from(format!("\u{0}{client}\u{0}{port}"))
}

fn ms_to_ns(v: Option<&Value>) -> i64 {
    v.and_then(|x| x.as_real().or_else(|| x.as_int().map(|i| i as f64))).map_or(0, |ms| (ms * 1e6).round() as i64)
}

impl Session {
    /// The index of a key in the session's table, given when first seen
    fn port_index(&mut self, key: Arc<str>, controller: bool) -> PortIndex {
        let next = PortIndex(u16::try_from(self.midi.aliases.len()).unwrap_or(u16::MAX));
        self.midi.aliases.entry(key).or_insert((next, controller)).0
    }

    /// The settings `lm_io::MidiDevices` opens the ports with: every port of the preferences, then the program's own
    /// input (its alias is the client name)
    pub fn midi_port_settings(&mut self) -> Vec<PortSettings> {
        let ports = self.preferences.get(&["midi", "ports"]).cloned().unwrap_or_else(Value::empty_list);
        let mut out = Vec::new();
        for p in ports.items() {
            let text = |k: &str| p.get(k).and_then(Value::as_text).unwrap_or("").to_owned();
            let (client, port, alias) = (text("client"), text("port"), text("alias"));
            if client.is_empty() || port.is_empty() {
                continue;
            }
            let role = match p.get("role").and_then(Value::as_text) {
                Some("controller") => PortRole::Controller,
                Some("both") => PortRole::Both,
                _ => PortRole::Instrument,
            };
            let controller = role != PortRole::Instrument;
            let key = if alias.is_empty() { id_key(&client, &port) } else { Arc::from(alias.as_str()) };
            let index = self.port_index(key, controller);
            out.push(PortSettings {
                index,
                id: PortId::new(&client, &port),
                backend: if p.get("backend").and_then(Value::as_text) == Some("raw") {
                    MidiBackendKind::Raw
                } else {
                    MidiBackendKind::Seq
                },
                alias,
                role,
                input: p.get("input").and_then(Value::as_bool).unwrap_or(true),
                output: p.get("output").and_then(Value::as_bool).unwrap_or(true),
                input_latency_ns: ms_to_ns(p.get("inputLatencyMs")),
                output_latency_ns: ms_to_ns(p.get("outputLatencyMs")),
            });
        }
        let own = self.port_index(Arc::from(CLIENT_NAME), true);
        out.push(PortSettings {
            index: own,
            id: PortId::new(CLIENT_NAME, OWN_INPUT),
            backend: MidiBackendKind::Seq,
            alias: CLIENT_NAME.to_owned(),
            role: PortRole::Both,
            input: true,
            output: false,
            input_latency_ns: 0,
            output_latency_ns: 0,
        });
        out
    }
}
