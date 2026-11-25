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
    /// The ports of the instrument role (instrument or both): a track whose input names no port listens to these
    pub(crate) fn midi_instrument_ports(&self) -> Vec<PortIndex> {
        let ports = self.preferences.get(&["midi", "ports"]).cloned().unwrap_or_else(Value::empty_list);
        ports
            .items()
            .filter(|p| p.get("role").and_then(Value::as_text) != Some("controller"))
            .filter_map(|p| {
                let alias = p.get("alias").and_then(Value::as_text).unwrap_or("");
                let key: Arc<str> = if alias.is_empty() {
                    id_key(
                        p.get("client").and_then(Value::as_text).unwrap_or(""),
                        p.get("port").and_then(Value::as_text).unwrap_or(""),
                    )
                } else {
                    Arc::from(alias)
                };
                self.midi.aliases.get(&key).map(|(i, _)| *i)
            })
            .collect()
    }

    /// Every alias with its index (for the compiler: tracks name ports by alias)
    pub(crate) fn midi_aliases(&self) -> Vec<(Arc<str>, PortIndex)> {
        self.midi.aliases.iter().filter(|(a, _)| !a.starts_with('\u{0}')).map(|(a, (i, _))| (a.clone(), *i)).collect()
    }

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

// ---- the device service (lm-io's `MidiDevices`) ----

use std::collections::BTreeSet;

use lm_base::ring::{Producer, ring};
use lm_base::{HostTime, Result};
use lm_io::{MidiDeviceEvent, PortInfo, SYSEX_OUT_MAX, SysexIn, SysexOut};
use lm_render::{CommandReceiver, Install, MidiOutEvent};

/// What the session needs from a MIDI device service. `lm_io::MidiDevices` is the one on Linux; a test drives the
/// session through a fake. Everything here is called from the pump
pub trait MidiDeviceLink {
    fn set_settings(&mut self, settings: Vec<PortSettings>) -> Result<()>;
    fn poll(&mut self, f: &mut dyn FnMut(MidiDeviceEvent));
    fn activity(&mut self, f: &mut dyn FnMut(lm_io::MidiActivity));
    fn sysex(&mut self, f: &mut dyn FnMut(SysexIn));
    fn add_output_source(&mut self, source: lm_base::ring::Consumer<MidiOutEvent>) -> Result<()>;
    fn add_sysex_output(&mut self, source: lm_base::ring::Consumer<SysexOut>) -> Result<()>;
    /// Wakes the output thread: what was pushed leaves now
    fn wake(&self);
    fn ports(&self) -> Vec<PortInfo>;
    /// Raw MIDI ports have no announcements: the session looks again every 2 s
    fn rescan_raw(&mut self);
}

#[cfg(target_os = "linux")]
impl MidiDeviceLink for lm_io::MidiDevices {
    fn set_settings(&mut self, settings: Vec<PortSettings>) -> Result<()> {
        lm_io::MidiDevices::set_settings(self, settings)
    }
    fn poll(&mut self, f: &mut dyn FnMut(MidiDeviceEvent)) {
        lm_io::MidiDevices::poll(self, f)
    }
    fn activity(&mut self, f: &mut dyn FnMut(lm_io::MidiActivity)) {
        lm_io::MidiDevices::activity(self, f)
    }
    fn sysex(&mut self, f: &mut dyn FnMut(SysexIn)) {
        lm_io::MidiDevices::sysex(self, f)
    }
    fn add_output_source(&mut self, source: lm_base::ring::Consumer<MidiOutEvent>) -> Result<()> {
        lm_io::MidiDevices::add_output_source(self, source)
    }
    fn add_sysex_output(&mut self, source: lm_base::ring::Consumer<SysexOut>) -> Result<()> {
        lm_io::MidiDevices::add_sysex_output(self, source)
    }
    fn wake(&self) {
        self.waker().wake();
    }
    fn ports(&self) -> Vec<PortInfo> {
        lm_io::MidiDevices::ports(self)
    }
    fn rescan_raw(&mut self) {
        lm_io::MidiDevices::rescan_raw(self)
    }
}

/// The SysEx ring to the devices: 64 messages (the MCU display is 16 cells at most per refresh)
const SYSEX_RING: usize = 64;
/// Raw MIDI ports are looked for again this often (ns)
const RESCAN: u64 = 2_000_000_000;
/// The command rings the devices may take: the sequencer's shared ring, then one per raw port (producer 0 is the
/// main thread's)
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const SEQ_PRODUCER: u16 = 1;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const RAW_PRODUCERS: std::ops::Range<u16> = 2..10;

/// The devices attached to the session, and what the session keeps about them
#[derive(Default)]
pub(crate) struct Devices {
    pub(crate) link: Option<Box<dyn MidiDeviceLink>>,
    sysex_out: Option<Producer<SysexOut>>,
    settings_from: Value,
    last_rescan: u64,
    pub(crate) unknown_aliases: BTreeSet<Arc<str>>,
    #[cfg(target_os = "linux")]
    pub(crate) opening: Option<lm_base::OperationHandle<(lm_io::MidiDevices, CommandReceiver)>>,
}

impl std::fmt::Debug for Devices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Devices").field("attached", &self.link.is_some()).finish_non_exhaustive()
    }
}

impl Session {
    /// Opens the machine's MIDI devices (`lm_io::MidiDevices`) in an operation: it talks to the kernel (I-2). The
    /// session calls it when it opens a real audio device; the result is attached at a later pump
    #[cfg(target_os = "linux")]
    pub fn midi_open_devices(&mut self) {
        let settings = self.midi_port_settings();
        let handle = self.workers.start(move |_, _| {
            lm_io::MidiDevices::start(
                settings,
                lm_render::ProducerIndex(SEQ_PRODUCER),
                RAW_PRODUCERS.map(lm_render::ProducerIndex).collect(),
            )
        });
        self.midi.devices.opening = Some(handle);
    }

    /// Attaches a device service: its input ring goes to the engine, the feedback and SysEx rings go to it, and it
    /// gets the ports of the preferences
    pub fn attach_midi_devices(&mut self, mut link: Box<dyn MidiDeviceLink>, input: CommandReceiver) -> Result<()> {
        let mut install = Install::producers_only(self.table.generation);
        install.add_producer(input);
        self.engine.install(install)?;
        if let Some(feedback) = self.take_midi_feedback() {
            link.add_output_source(feedback)?;
        }
        // What the tracks play out to devices (P-38)
        if let Some(out) = self.engine.take_midi_out() {
            link.add_output_source(out)?;
        }
        let (tx, rx) = ring(SYSEX_RING);
        link.add_sysex_output(rx)?;
        let settings = self.midi_port_settings();
        link.set_settings(settings)?;
        self.midi.devices.settings_from = self.preferences.get(&["midi", "ports"]).cloned().unwrap_or(Value::NULL);
        self.midi.devices.sysex_out = Some(tx);
        self.midi.devices.link = Some(link);
        self.midi.mark_dirty();
        Ok(())
    }

    /// From the pump, before the map's rings: the device events, the activity (learning, step recording, the
    /// monitor), new settings when the preferences changed
    pub(crate) fn midi_devices_in(&mut self, now: HostTime) {
        #[cfg(target_os = "linux")]
        if let Some(mut h) = self.midi.devices.opening.take() {
            match h.poll() {
                None => self.midi.devices.opening = Some(h),
                Some(Ok((devices, input))) => {
                    if let Err(e) = self.attach_midi_devices(Box::new(devices), input) {
                        self.notices.error(&e, None, None);
                    }
                }
                Some(Err(e)) => self.notices.error(&e, None, None),
            }
        }
        if self.midi.devices.link.is_none() {
            return;
        }
        let ports = self.preferences.get(&["midi", "ports"]).cloned().unwrap_or(Value::NULL);
        if ports != self.midi.devices.settings_from {
            self.midi.devices.settings_from = ports;
            let settings = self.midi_port_settings();
            if let Some(link) = self.midi.devices.link.as_mut()
                && let Err(e) = link.set_settings(settings)
            {
                self.notices.error(&e, None, None);
            }
            self.midi.mark_dirty();
        }
        let mut events = Vec::new();
        let mut activity = Vec::new();
        if let Some(link) = self.midi.devices.link.as_mut() {
            link.poll(&mut |e| events.push(e));
            link.activity(&mut |a| activity.push(a));
            link.sysex(&mut |_s: SysexIn| {}); // no profile answers SysEx yet: taken so the ring never fills
            if now.0.saturating_sub(self.midi.devices.last_rescan) >= RESCAN {
                self.midi.devices.last_rescan = now.0;
                link.rescan_raw();
            }
        }
        let ports_changed = !events.is_empty();
        for e in events {
            self.midi_device_event(e);
        }
        for a in activity {
            let len = usize::from(a.len).min(3);
            self.midi_input(a.port, a.at, &a.bytes[..len]);
        }
        if ports_changed {
            self.midi_publish_ports();
        }
    }

    fn midi_device_event(&mut self, e: MidiDeviceEvent) {
        let alias = |s: &Session, index: PortIndex| s.midi.port_alias(index);
        match e {
            MidiDeviceEvent::Connected { index, .. } => {
                let mut connected: Vec<PortIndex> = self.midi.connected_ports();
                if !connected.contains(&index) {
                    connected.push(index);
                }
                let was_seen = self.midi.was_connected(index);
                self.midi_ports(&connected);
                if was_seen {
                    // Back after a disconnection: everything works again by itself (I-21.1, I-14)
                    let a = alias(self, index);
                    self.notices.push(
                        "midi.port.reconnected",
                        vec![("port", Value::text(a))],
                        lm_port::Severity::Info,
                        None,
                        Vec::new(),
                    );
                }
            }
            MidiDeviceEvent::Disconnected { index, .. } => {
                let connected: Vec<PortIndex> =
                    self.midi.connected_ports().into_iter().filter(|p| *p != index).collect();
                self.midi_ports(&connected);
                let a = alias(self, index);
                self.notices.warn("midi.port.disconnected", vec![("port", Value::text(a))]);
            }
            MidiDeviceEvent::ProducerReady(rx) => {
                let mut install = Install::producers_only(self.table.generation);
                install.add_producer(rx);
                if let Err(e) = self.engine.install(install) {
                    self.notices.error(&e, None, None);
                }
            }
            MidiDeviceEvent::ProducerDone(p) => {
                let mut install = Install::producers_only(self.table.generation);
                install.remove_producer(p);
                if let Err(e) = self.engine.install(install) {
                    self.notices.error(&e, None, None);
                }
            }
            MidiDeviceEvent::Lost { count, .. } => self
                .notices
                .warn("midi.input.lost", vec![("count", Value::int(i64::try_from(count).unwrap_or(i64::MAX)))]),
            MidiDeviceEvent::OutputDropped { count, .. } => self
                .notices
                .warn("midi.output.dropped", vec![("count", Value::int(i64::try_from(count).unwrap_or(i64::MAX)))]),
            MidiDeviceEvent::Failed { error, .. } => self.notices.error(&error, None, None),
            MidiDeviceEvent::Appeared(_) | MidiDeviceEvent::Gone(_) => {}
        }
    }

    /// `status.midiPorts`: every port the machine has, with its alias and whether it is connected
    fn midi_publish_ports(&mut self) {
        let Some(link) = self.midi.devices.link.as_ref() else { return };
        let infos = link.ports();
        let prefs = self.preferences.get(&["midi", "ports"]).cloned().unwrap_or_else(Value::empty_list);
        let list: Vec<Value> = infos
            .iter()
            .map(|p| {
                let entry = prefs.items().find(|e| {
                    e.get("client").and_then(Value::as_text) == Some(p.id.client.as_str())
                        && e.get("port").and_then(Value::as_text) == Some(p.id.port.as_str())
                });
                let alias = entry.and_then(|e| e.get("alias").and_then(Value::as_text)).unwrap_or("").to_owned();
                let connected = !alias.is_empty()
                    && self.midi_port_of(&alias).is_some_and(|i| self.midi.connected_ports().contains(&i));
                Value::object([
                    ("client", Value::text(p.id.client.as_str())),
                    ("port", Value::text(p.id.port.as_str())),
                    ("backend", Value::text(if p.backend == MidiBackendKind::Raw { "raw" } else { "seq" })),
                    ("alias", Value::text(alias)),
                    ("connected", Value::bool(connected)),
                    ("readable", Value::bool(p.readable)),
                    ("writable", Value::bool(p.writable)),
                ])
            })
            .collect();
        self.status = self.status.with("midiPorts", Value::list(list));
    }

    /// From the pump, after feedback: the SysEx to the devices, and the output thread woken
    pub(crate) fn midi_devices_out(&mut self, now: HostTime) {
        if self.midi.devices.link.is_none() {
            return;
        }
        for (port, data) in self.take_midi_sysex() {
            let Some(tx) = self.midi.devices.sysex_out.as_mut() else { break };
            let len = data.len().min(SYSEX_OUT_MAX);
            let mut buf = [0u8; SYSEX_OUT_MAX];
            buf[..len].copy_from_slice(&data[..len]);
            let _ = tx.push(SysexOut { port, at: now, len: len as u16, data: buf });
        }
        if let Some(link) = self.midi.devices.link.as_ref() {
            link.wake();
        }
    }

    /// A project opened on another machine (P-36): each alias of its mappings that no port of this machine has is
    /// asked for once, with the action that assigns it
    pub(crate) fn midi_unknown_aliases(&mut self, aliases: Vec<Arc<str>>) {
        for a in aliases {
            if a.is_empty() || self.midi_port_of(&a).is_some() || !self.midi.devices.unknown_aliases.insert(a.clone()) {
                continue;
            }
            self.notices.push(
                "midi.alias.unknown",
                vec![("alias", Value::text(a.clone()))],
                lm_port::Severity::Warning,
                None,
                vec![(lm_port::ActionId::from("midi.assign_alias"), Value::object([("alias", Value::text(a))]))],
            );
        }
    }
}
