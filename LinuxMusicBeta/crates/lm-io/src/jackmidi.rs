//! JACK MIDI on the audio client (interfaces.md I-21.1, I-10.2; F-REC-125): the second implementation of the session's
//! MIDI device service, next to the ALSA sequencer's [`crate::MidiDevices`].
//!
//! **One client, audio and MIDI.** JACK's MIDI ports belong to the client that also runs the audio, so the MIDI half is
//! not a service of its own: the [`crate::JackBackend`] registers the MIDI ports when it opens, runs this module's
//! per-cycle code inside its process callback, and its control thread keeps the connections up to date. The session's
//! side is the [`JackMidiLink`] (the `MidiDeviceLink` the session already drives), tied to the backend by a
//! [`JackMidiShared`] both are given.
//!
//! - **Ports.** The client registers [`JACK_MIDI_PORTS`] MIDI inputs and as many outputs. Slot 0 is the program's own
//!   pair ("midi_in", "midi_out": others connect to them, as to "LinuxMusicBeta entrada" and "saída" under ALSA);
//!   every other slot serves one configured port of the preferences ([`PortSettings`], identified by the JACK port's
//!   `client:port` name, never a number), connected to it by the control thread. The slot of an index never changes
//!   while the index is wanted, so the cycle's table is only rewritten, never reshuffled.
//! - **Time.** An input event's stamp is the cycle's first frame on the monotonic clock (JACK's cycle times, moved to
//!   `CLOCK_MONOTONIC`), plus the event's frame, minus the port's input latency (C6, P-15). An output event is written
//!   to the first cycle that reaches its time minus the port's output latency, at the frame it falls on; events from
//!   a later time are held (a fixed array, never allocating) until their cycle.
//! - **Real time.** The cycle code takes the shared state with `try_lock` (uncontended except for the instant the
//!   control thread rewrites the routing table, when that cycle's MIDI waits one cycle), never allocates and never
//!   blocks. Only channel messages and the system-common ones go to the engine; system real-time messages (clock,
//!   active sensing) are dropped here, as the sequencer binding drops them. SysEx goes to the session alone, by a
//!   fixed-size slot ([`SYSEX_SLOT`]); a longer one is counted as lost.
//! - **Hotplug.** The server's port announcements wake the control thread, which lists the MIDI ports again, reports
//!   the ones that appeared or went away, connects the configured ports that are present and reports
//!   [`MidiDeviceEvent::Connected`] and [`MidiDeviceEvent::Disconnected`] (JACK removes a gone port's connections by
//!   itself; the same identity coming back is connected again).

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use lm_base::ring::{Consumer, Producer, ring};
use lm_base::{Error, HostTime, Result};
use lm_render::{Command, CommandReceiver, CommandSender, MidiOutEvent, PortIndex, ProducerIndex, command_ring};
use lm_sys_jack::{Client, Cycle, Direction, Port, PortFilter, PortKind};

use crate::jack::Wake;
use crate::mididev::{
    ACTIVITY_RING, CLIENT_NAME, INPUT_RING, MidiActivity, MidiBackendKind, MidiDeviceEvent, OWN_INPUT, OWN_OUTPUT,
    PortId, PortInfo, PortSettings, SysexIn, SysexOut, message_len,
};

/// MIDI inputs and outputs the client registers (slot 0 is the program's own)
pub const JACK_MIDI_PORTS: usize = 16;
/// The longest SysEx the cycle takes in (longer ones are counted as lost)
pub const SYSEX_SLOT: usize = 1024;
/// Output events the cycle can hold for a later cycle (the sessions' feedback is a few messages a second)
const HELD: usize = 1024;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A SysEx message read in the cycle, as plain data
struct RawSysex {
    port: PortIndex,
    at: HostTime,
    len: u16,
    data: [u8; SYSEX_SLOT],
}

#[derive(Clone, Copy, Debug, Default)]
struct Route {
    index: Option<PortIndex>,
    /// Input: subtracted from the stamp. Output: the event leaves this much earlier. Nanoseconds
    latency_ns: i64,
}

#[derive(Debug, Default)]
struct Counters {
    /// Input messages the engine's ring had no room for
    lost: AtomicU64,
    sysex_lost: AtomicU64,
    activity_lost: AtomicU64,
    dropped_out: AtomicU64,
}

/// What the cycle owns while a link is installed
struct MidiRt {
    token: u64,
    sink: CommandSender,
    activity: Producer<MidiActivity>,
    sysex: Producer<RawSysex>,
    outputs: Vec<Consumer<MidiOutEvent>>,
    sysex_outputs: Vec<Consumer<SysexOut>>,
    inputs: [Route; JACK_MIDI_PORTS],
    out_routes: [Route; JACK_MIDI_PORTS],
    held: Vec<MidiOutEvent>,
    last_frame: [u32; JACK_MIDI_PORTS],
}

struct Inner {
    rt: Mutex<Option<MidiRt>>,
    settings: Mutex<Arc<Vec<PortSettings>>>,
    events: Mutex<VecDeque<MidiDeviceEvent>>,
    ports: Mutex<Vec<PortInfo>>,
    wake: Mutex<Option<Arc<Wake>>>,
    counters: Counters,
    tokens: AtomicU64,
}

/// What the JACK backend and the session's MIDI link share. Make one, give a clone to the backend
/// ([`crate::JackOptions::midi`]) and keep one to start the link with
#[derive(Clone)]
pub struct JackMidiShared {
    inner: Arc<Inner>,
}

impl fmt::Debug for JackMidiShared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JackMidiShared")
            .field("linked", &lock(&self.inner.rt).is_some())
            .field("ports", &lock(&self.inner.ports).len())
            .finish()
    }
}

impl Default for JackMidiShared {
    fn default() -> Self {
        Self::new()
    }
}

impl JackMidiShared {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                rt: Mutex::new(None),
                settings: Mutex::new(Arc::new(Vec::new())),
                events: Mutex::new(VecDeque::new()),
                ports: Mutex::new(Vec::new()),
                wake: Mutex::new(None),
                counters: Counters::default(),
                tokens: AtomicU64::new(0),
            }),
        }
    }

    /// Asks the control thread (when the backend runs) to look at the ports and the settings again
    pub fn request_reconcile(&self) {
        if let Some(w) = lock(&self.inner.wake).as_ref() {
            w.set(|s| s.dirty = true);
        }
    }

    pub(crate) fn set_wake(&self, wake: Option<Arc<Wake>>) {
        *lock(&self.inner.wake) = wake;
    }

    fn push_event(&self, e: MidiDeviceEvent) {
        let mut q = lock(&self.inner.events);
        // A control thread that nobody polls does not grow without bound
        if q.len() >= 4096 {
            q.pop_front();
        }
        q.push_back(e);
    }
}

// ---- the pure parts (tested) ----

/// `client:port` of a JACK port name (the first colon splits: client names have none, port names may)
pub(crate) fn split_port_name(name: &str) -> PortId {
    match name.split_once(':') {
        Some((client, port)) => PortId::new(client, port),
        None => PortId::new("", name),
    }
}

/// The JACK full name of a port identity
pub(crate) fn full_name(id: &PortId) -> String {
    format!("{}:{}", id.client, id.port)
}

/// What an input event is for the engine
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Incoming {
    Short { bytes: [u8; 3], len: u8 },
    Sysex,
    Ignore,
}

/// Channel messages and the system-common ones go on; real-time messages, running status (JACK never sends it) and
/// malformed events are dropped
pub(crate) fn classify(bytes: &[u8]) -> Incoming {
    let Some(&status) = bytes.first() else { return Incoming::Ignore };
    match status {
        0xf0 => Incoming::Sysex,
        0x80..=0xef | 0xf1..=0xf3 | 0xf6 => {
            let len = message_len(status);
            if bytes.len() < usize::from(len) {
                return Incoming::Ignore;
            }
            let mut out = [0u8; 3];
            out[..usize::from(len)].copy_from_slice(&bytes[..usize::from(len)]);
            Incoming::Short { bytes: out, len }
        }
        _ => Incoming::Ignore,
    }
}

/// The frame of the cycle an instant falls on (the cycle starts at `start`, nanoseconds, and has `frames` frames)
pub(crate) fn frame_in_cycle(at: u64, start: u64, frames: u32, rate: u32) -> u32 {
    if at <= start || frames == 0 {
        return 0;
    }
    let f = u128::from(at - start) * u128::from(rate) / 1_000_000_000;
    u32::try_from(f).unwrap_or(u32::MAX).min(frames - 1)
}

/// The instant of a frame of the cycle, nanoseconds
pub(crate) fn instant_of_frame(start: u64, frame: u32, rate: u32) -> u64 {
    start.saturating_add(u64::from(frame) * 1_000_000_000 / u64::from(rate.max(1)))
}

/// Every wanted index keeps the slot it had; new ones take the lowest free slot from 1 (slot 0 is the program's own).
/// An index that finds no free slot gets none
pub(crate) fn assign_slots(prev: &[Option<PortIndex>], wanted: &[PortIndex]) -> Vec<Option<PortIndex>> {
    let mut out: Vec<Option<PortIndex>> = vec![None; prev.len()];
    for (slot, p) in prev.iter().enumerate().skip(1) {
        if let Some(i) = p
            && wanted.contains(i)
        {
            out[slot] = Some(*i);
        }
    }
    for i in wanted {
        if out.contains(&Some(*i)) {
            continue;
        }
        if let Some(slot) = (1..out.len()).find(|s| out[*s].is_none()) {
            out[slot] = Some(*i);
        }
    }
    out
}

fn is_own(id: &PortId, port: &str) -> bool {
    id.client == CLIENT_NAME && id.port == port
}

// ---- the real-time half ----

impl MidiRt {
    fn new(token: u64, sink: CommandSender, activity: Producer<MidiActivity>, sysex: Producer<RawSysex>) -> Self {
        Self {
            token,
            sink,
            activity,
            sysex,
            outputs: Vec::new(),
            sysex_outputs: Vec::new(),
            inputs: [Route::default(); JACK_MIDI_PORTS],
            out_routes: [Route::default(); JACK_MIDI_PORTS],
            held: Vec::with_capacity(HELD),
            last_frame: [0; JACK_MIDI_PORTS],
        }
    }

    /// Reads the inputs' events of this cycle. `start` is the cycle's first frame on the monotonic clock
    fn cycle_in(&mut self, c: &Cycle<'_>, ports: &[Port], counters: &Counters, start: u64, rate: u32) {
        for (slot, port) in ports.iter().enumerate().take(JACK_MIDI_PORTS) {
            let route = self.inputs[slot];
            let Some(index) = route.index else { continue };
            let Self { sink, activity, sysex, .. } = self;
            c.read_midi(*port, |frame, bytes| {
                let at = HostTime(instant_of_frame(start, frame, rate).saturating_add_signed(-route.latency_ns));
                match classify(bytes) {
                    Incoming::Short { bytes, len } => {
                        if sink.send(Command::Midi { port: index, at, bytes }).is_err() {
                            counters.lost.fetch_add(1, Ordering::Relaxed);
                        }
                        if activity.push(MidiActivity { port: index, at, len, bytes }).is_err() {
                            counters.activity_lost.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Incoming::Sysex => {
                        if bytes.len() > SYSEX_SLOT {
                            counters.sysex_lost.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                        let mut data = [0u8; SYSEX_SLOT];
                        data[..bytes.len()].copy_from_slice(bytes);
                        let msg = RawSysex { port: index, at, len: bytes.len() as u16, data };
                        if sysex.push(msg).is_err() {
                            counters.sysex_lost.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Incoming::Ignore => {}
                }
            });
        }
    }

    /// Writes what the output rings hold and is due in this cycle (`start` .. `start` + `frames`)
    fn cycle_out(
        &mut self,
        c: &mut Cycle<'_>,
        ports: &[Port],
        counters: &Counters,
        start: u64,
        frames: u32,
        rate: u32,
    ) {
        let Self { outputs, sysex_outputs, out_routes, held, last_frame, .. } = self;
        last_frame.fill(0);
        for ring in outputs.iter_mut() {
            while let Ok(e) = ring.pop() {
                if held.len() < HELD {
                    held.push(e);
                } else {
                    counters.dropped_out.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        held.sort_unstable_by_key(|e| e.at.0);
        let end = instant_of_frame(start, frames, rate);
        held.retain(|e| {
            let Some(slot) = out_routes.iter().position(|r| r.index == Some(e.port)) else {
                counters.dropped_out.fetch_add(1, Ordering::Relaxed);
                return false;
            };
            let at = e.at.0.saturating_add_signed(-out_routes[slot].latency_ns);
            if at >= end {
                return true;
            }
            // (events of one port never go back in time inside the cycle: JACK refuses them)
            let frame = frame_in_cycle(at, start, frames, rate).max(last_frame[slot]);
            let len = usize::from(e.len.clamp(1, 3));
            match ports.get(slot) {
                Some(p) if c.write_midi(*p, frame, &e.bytes[..len]) => last_frame[slot] = frame,
                _ => {
                    counters.dropped_out.fetch_add(1, Ordering::Relaxed);
                }
            }
            false
        });
        for ring in sysex_outputs.iter_mut() {
            while let Ok(e) = ring.pop() {
                let Some(slot) = out_routes.iter().position(|r| r.index == Some(e.port)) else {
                    counters.dropped_out.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let len = usize::from(e.len).min(e.data.len());
                let sent = ports.get(slot).is_some_and(|p| c.write_midi(*p, last_frame[slot], &e.data[..len]));
                if !sent {
                    counters.dropped_out.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

/// The backend's per-cycle handle on the MIDI half: the registered ports and the shared state
#[derive(Debug)]
pub(crate) struct MidiCycle {
    shared: JackMidiShared,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
}

impl MidiCycle {
    pub(crate) fn input(&self, c: &Cycle<'_>, start: u64, rate: u32) {
        if let Ok(mut g) = self.shared.inner.rt.try_lock()
            && let Some(rt) = g.as_mut()
        {
            rt.cycle_in(c, &self.inputs, &self.shared.inner.counters, start, rate);
        }
    }

    pub(crate) fn output(&self, c: &mut Cycle<'_>, start: u64, frames: u32, rate: u32) {
        if let Ok(mut g) = self.shared.inner.rt.try_lock()
            && let Some(rt) = g.as_mut()
        {
            rt.cycle_out(c, &self.outputs, &self.shared.inner.counters, start, frames, rate);
        }
    }
}

// ---- opening, and the control thread's half ----

/// The ports registered for the MIDI half
#[derive(Debug)]
pub(crate) struct MidiPorts {
    pub(crate) inputs: Vec<Port>,
    pub(crate) outputs: Vec<Port>,
    in_names: Vec<String>,
    out_names: Vec<String>,
}

/// Registers the MIDI ports on the client (before it is activated)
pub(crate) fn register_ports(client: &mut Client) -> Result<MidiPorts> {
    let mut ports = MidiPorts { inputs: Vec::new(), outputs: Vec::new(), in_names: Vec::new(), out_names: Vec::new() };
    for slot in 0..JACK_MIDI_PORTS {
        let (i, o) = if slot == 0 {
            ("midi_in".to_owned(), "midi_out".to_owned())
        } else {
            (format!("midi_in_{slot}"), format!("midi_out_{slot}"))
        };
        let pi = client.register(&i, PortKind::Midi, Direction::Input)?;
        let po = client.register(&o, PortKind::Midi, Direction::Output)?;
        ports.in_names.push(client.port_name(pi));
        ports.out_names.push(client.port_name(po));
        ports.inputs.push(pi);
        ports.outputs.push(po);
    }
    Ok(ports)
}

impl MidiPorts {
    pub(crate) fn cycle(&self, shared: &JackMidiShared) -> MidiCycle {
        MidiCycle { shared: shared.clone(), inputs: self.inputs.clone(), outputs: self.outputs.clone() }
    }

    pub(crate) fn control(self, shared: &JackMidiShared) -> MidiControl {
        MidiControl {
            shared: shared.clone(),
            ports: self,
            in_slots: vec![None; JACK_MIDI_PORTS],
            out_slots: vec![None; JACK_MIDI_PORTS],
            in_connected: Vec::new(),
            out_connected: Vec::new(),
            seen: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct Connection {
    index: PortIndex,
    id: PortId,
    external: String,
    slot: usize,
}

/// The control thread's state for the MIDI half: the slots, the connections made, the ports seen
#[derive(Debug)]
pub(crate) struct MidiControl {
    shared: JackMidiShared,
    ports: MidiPorts,
    in_slots: Vec<Option<PortIndex>>,
    out_slots: Vec<Option<PortIndex>>,
    in_connected: Vec<Connection>,
    out_connected: Vec<Connection>,
    seen: Vec<PortInfo>,
}

impl MidiControl {
    /// Lists the MIDI ports, reports what appeared and went away, and makes the connections match the settings
    /// (hotplug). Not real time: it talks to the server
    pub(crate) fn reconcile(&mut self, client: &Client) {
        let own = format!("{}:", client.name());
        let list = |direction: Direction, physical: bool| {
            client.ports(PortFilter { kind: Some(PortKind::Midi), direction: Some(direction), physical })
        };
        let (sources, sinks) = (list(Direction::Output, false), list(Direction::Input, false));
        let (hw_sources, hw_sinks) = (list(Direction::Output, true), list(Direction::Input, true));
        let mut now: Vec<PortInfo> = Vec::new();
        for n in sources.iter().filter(|n| !n.starts_with(&own)) {
            now.push(PortInfo {
                id: split_port_name(n),
                backend: MidiBackendKind::Jack,
                readable: true,
                writable: false,
                hardware: hw_sources.contains(n),
            });
        }
        for n in sinks.iter().filter(|n| !n.starts_with(&own)) {
            now.push(PortInfo {
                id: split_port_name(n),
                backend: MidiBackendKind::Jack,
                readable: false,
                writable: true,
                hardware: hw_sinks.contains(n),
            });
        }
        let same = |a: &PortInfo, b: &PortInfo| a.id == b.id && a.readable == b.readable;
        for p in now.iter().filter(|p| !self.seen.iter().any(|s| same(s, p))) {
            self.shared.push_event(MidiDeviceEvent::Appeared(p.clone()));
        }
        for s in self.seen.iter().filter(|s| !now.iter().any(|p| same(s, p))) {
            self.shared.push_event(MidiDeviceEvent::Gone(s.id.clone()));
        }
        self.seen = now.clone();
        *lock(&self.shared.inner.ports) = now;

        let settings = lock(&self.shared.inner.settings).clone();
        let mut in_route = [Route::default(); JACK_MIDI_PORTS];
        let mut out_route = [Route::default(); JACK_MIDI_PORTS];
        // Slot 0: the program's own ports
        for s in settings.iter() {
            if s.input && is_own(&s.id, OWN_INPUT) {
                in_route[0] = Route { index: Some(s.index), latency_ns: s.input_latency_ns };
            }
            if s.output && is_own(&s.id, OWN_OUTPUT) {
                out_route[0] = Route { index: Some(s.index), latency_ns: s.output_latency_ns };
            }
        }
        // The others: a slot each, kept through disconnections
        let wanted_in: Vec<&PortSettings> = settings.iter().filter(|s| s.input && !is_own(&s.id, OWN_INPUT)).collect();
        let wanted_out: Vec<&PortSettings> =
            settings.iter().filter(|s| s.output && !is_own(&s.id, OWN_OUTPUT)).collect();
        self.in_slots = assign_slots(&self.in_slots, &wanted_in.iter().map(|s| s.index).collect::<Vec<_>>());
        self.out_slots = assign_slots(&self.out_slots, &wanted_out.iter().map(|s| s.index).collect::<Vec<_>>());
        for (slot, index) in self.in_slots.iter().enumerate().skip(1) {
            if let Some(s) = index.and_then(|i| wanted_in.iter().find(|s| s.index == i)) {
                in_route[slot] = Route { index: *index, latency_ns: s.input_latency_ns };
            }
        }
        for (slot, index) in self.out_slots.iter().enumerate().skip(1) {
            if let Some(s) = index.and_then(|i| wanted_out.iter().find(|s| s.index == i)) {
                out_route[slot] = Route { index: *index, latency_ns: s.output_latency_ns };
            }
        }

        // Inputs: connect what should be (and is present), undo what should not be, tell what changed
        let old = std::mem::take(&mut self.in_connected);
        for c in &old {
            let still = self.in_slots.get(c.slot) == Some(&Some(c.index));
            let present = sources.contains(&c.external);
            if !still || !present {
                if present {
                    // Disabled but still there: our side undoes the connection. A port that went away took it along
                    let _ = client.disconnect(&c.external, &self.ports.in_names[c.slot]);
                }
                self.shared.push_event(MidiDeviceEvent::Disconnected { index: c.index, id: c.id.clone() });
            }
        }
        for (slot, index) in self.in_slots.iter().enumerate().skip(1) {
            let Some(index) = *index else { continue };
            let Some(s) = wanted_in.iter().find(|s| s.index == index) else { continue };
            let external = full_name(&s.id);
            if !sources.contains(&external) {
                continue;
            }
            if old.iter().any(|c| c.index == index && c.slot == slot && c.external == external) {
                self.in_connected.push(Connection { index, id: s.id.clone(), external, slot });
                continue;
            }
            match client.connect(&external, &self.ports.in_names[slot]) {
                Ok(()) => {
                    self.shared.push_event(MidiDeviceEvent::Connected { index, id: s.id.clone() });
                    self.in_connected.push(Connection { index, id: s.id.clone(), external, slot });
                }
                Err(error) => self.shared.push_event(MidiDeviceEvent::Failed { index: Some(index), error }),
            }
        }
        // Outputs: the same, without notices (the output follows its input's)
        let old = std::mem::take(&mut self.out_connected);
        for c in &old {
            let still = self.out_slots.get(c.slot) == Some(&Some(c.index));
            if !still && sinks.contains(&c.external) {
                let _ = client.disconnect(&self.ports.out_names[c.slot], &c.external);
            }
        }
        for (slot, index) in self.out_slots.iter().enumerate().skip(1) {
            let Some(index) = *index else { continue };
            let Some(s) = wanted_out.iter().find(|s| s.index == index) else { continue };
            let external = full_name(&s.id);
            if !sinks.contains(&external) {
                continue;
            }
            if old.iter().any(|c| c.index == index && c.slot == slot && c.external == external) {
                self.out_connected.push(Connection { index, id: s.id.clone(), external, slot });
                continue;
            }
            match client.connect(&self.ports.out_names[slot], &external) {
                Ok(()) => self.out_connected.push(Connection { index, id: s.id.clone(), external, slot }),
                Err(error) => self.shared.push_event(MidiDeviceEvent::Failed { index: Some(index), error }),
            }
        }

        if let Some(rt) = lock(&self.shared.inner.rt).as_mut() {
            rt.inputs = in_route;
            rt.out_routes = out_route;
        }
    }
}

// ---- the session's side ----

/// The session's MIDI device service over JACK: the `MidiDeviceLink` of the session (I-21.1) for the ports of the
/// backend's client. Dropping it stops the cycle's MIDI
pub struct JackMidiLink {
    shared: JackMidiShared,
    token: u64,
    activity: Consumer<MidiActivity>,
    sysex: Consumer<RawSysex>,
    reported: [u64; 3],
}

impl fmt::Debug for JackMidiLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JackMidiLink").field("shared", &self.shared).finish()
    }
}

impl JackMidiLink {
    /// Installs the link on the shared state: `seq` is the producer of the input ring (its receiver comes back:
    /// `Install::add_producer`). Cheap and not blocking: the ports exist already (the backend registered them)
    pub fn start(
        shared: &JackMidiShared,
        settings: Vec<PortSettings>,
        seq: ProducerIndex,
    ) -> Result<(JackMidiLink, CommandReceiver)> {
        let (sink, receiver) = command_ring(seq, INPUT_RING);
        let (activity_tx, activity) = ring(ACTIVITY_RING);
        let (sysex_tx, sysex) = ring(256);
        let token = shared.inner.tokens.fetch_add(1, Ordering::Relaxed) + 1;
        *lock(&shared.inner.settings) = Arc::new(settings);
        *lock(&shared.inner.rt) = Some(MidiRt::new(token, sink, activity_tx, sysex_tx));
        shared.request_reconcile();
        let link = JackMidiLink { shared: shared.clone(), token, activity, sysex, reported: [0; 3] };
        Ok((link, receiver))
    }

    /// The settings changed (aliases, roles, latencies, enable flags): the connections follow
    pub fn set_settings(&mut self, settings: Vec<PortSettings>) -> Result<()> {
        *lock(&self.shared.inner.settings) = Arc::new(settings);
        self.shared.request_reconcile();
        Ok(())
    }

    /// A ring of output events (the renderer's, the session's feedback): written at each event's time
    pub fn add_output_source(&mut self, source: Consumer<MidiOutEvent>) -> Result<()> {
        match lock(&self.shared.inner.rt).as_mut() {
            Some(rt) if rt.token == self.token => {
                rt.outputs.push(source);
                Ok(())
            }
            _ => Err(Error::new("io.midi.busy")),
        }
    }

    /// A ring of SysEx to send (the MCU display)
    pub fn add_sysex_output(&mut self, source: Consumer<SysexOut>) -> Result<()> {
        match lock(&self.shared.inner.rt).as_mut() {
            Some(rt) if rt.token == self.token => {
                rt.sysex_outputs.push(source);
                Ok(())
            }
            _ => Err(Error::new("io.midi.busy")),
        }
    }

    /// The cycle takes the rings itself: nothing to wake
    pub fn wake(&self) {}

    /// The MIDI ports of the server at the last listing (the program's own not among them)
    pub fn ports(&self) -> Vec<PortInfo> {
        lock(&self.shared.inner.ports).clone()
    }

    /// The server announces ports by itself: this only asks for a new listing
    pub fn rescan(&mut self) {
        self.shared.request_reconcile();
    }

    /// From the pump: the devices' events
    pub fn poll(&mut self, mut f: impl FnMut(MidiDeviceEvent)) {
        let events: Vec<MidiDeviceEvent> = lock(&self.shared.inner.events).drain(..).collect();
        for e in events {
            f(e);
        }
        let c = &self.shared.inner.counters;
        let now = [
            c.lost.load(Ordering::Relaxed) + c.sysex_lost.load(Ordering::Relaxed),
            c.dropped_out.load(Ordering::Relaxed),
        ];
        if now[0] > self.reported[0] {
            f(MidiDeviceEvent::Lost { index: None, count: now[0] - self.reported[0] });
            self.reported[0] = now[0];
        }
        if now[1] > self.reported[1] {
            f(MidiDeviceEvent::OutputDropped { index: None, count: now[1] - self.reported[1] });
            self.reported[1] = now[1];
        }
    }

    /// From the pump: every input message (lossy)
    pub fn activity(&mut self, mut f: impl FnMut(MidiActivity)) {
        while let Ok(a) = self.activity.pop() {
            f(a);
        }
    }

    /// Activity messages lost because the pump did not take them in time
    pub fn activity_lost(&self) -> u64 {
        self.shared.inner.counters.activity_lost.load(Ordering::Relaxed)
    }

    /// From the pump: SysEx received (for the session's controller profiles only)
    pub fn sysex(&mut self, mut f: impl FnMut(SysexIn)) {
        while let Ok(s) = self.sysex.pop() {
            let len = usize::from(s.len).min(SYSEX_SLOT);
            f(SysexIn { port: s.port, at: s.at, data: s.data[..len].to_vec() });
        }
    }
}

impl Drop for JackMidiLink {
    fn drop(&mut self) {
        let mut g = lock(&self.shared.inner.rt);
        if g.as_ref().is_some_and(|rt| rt.token == self.token) {
            *g = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_names_split_at_the_first_colon() {
        assert_eq!(split_port_name("system:midi_capture_1"), PortId::new("system", "midi_capture_1"));
        assert_eq!(split_port_name("a2j:Keystation 49 [20] (capture): Keystation 49 MIDI 1").client, "a2j");
        assert_eq!(
            split_port_name("a2j:Keystation 49 [20] (capture): Keystation 49 MIDI 1").port,
            "Keystation 49 [20] (capture): Keystation 49 MIDI 1"
        );
        assert_eq!(split_port_name("alone"), PortId::new("", "alone"));
        let id = PortId::new("system", "midi_playback_1");
        assert_eq!(split_port_name(&full_name(&id)), id);
    }

    #[test]
    fn channel_and_common_messages_pass_and_real_time_ones_do_not() {
        assert_eq!(classify(&[0x90, 60, 100]), Incoming::Short { bytes: [0x90, 60, 100], len: 3 });
        assert_eq!(classify(&[0xc3, 5]), Incoming::Short { bytes: [0xc3, 5, 0], len: 2 });
        assert_eq!(classify(&[0xf2, 1, 2]), Incoming::Short { bytes: [0xf2, 1, 2], len: 3 });
        assert_eq!(classify(&[0xf6]), Incoming::Short { bytes: [0xf6, 0, 0], len: 1 });
        assert_eq!(classify(&[0xf0, 1, 2, 0xf7]), Incoming::Sysex);
        assert_eq!(classify(&[0xf8]), Incoming::Ignore);
        assert_eq!(classify(&[0xfe]), Incoming::Ignore);
        // Truncated, running status, empty
        assert_eq!(classify(&[0x90, 60]), Incoming::Ignore);
        assert_eq!(classify(&[0x3c, 100]), Incoming::Ignore);
        assert_eq!(classify(&[]), Incoming::Ignore);
    }

    #[test]
    fn an_instant_falls_on_a_frame_of_the_cycle() {
        let rate = 48_000;
        assert_eq!(frame_in_cycle(1_000, 5_000, 256, rate), 0, "before the cycle: its first frame");
        assert_eq!(frame_in_cycle(5_000, 5_000, 256, rate), 0);
        // 1 ms is 48 frames
        assert_eq!(frame_in_cycle(5_000 + 1_000_000, 5_000, 256, rate), 48);
        assert_eq!(frame_in_cycle(5_000 + 100_000_000, 5_000, 256, rate), 255, "after it: the last frame");
        assert_eq!(instant_of_frame(5_000, 48, rate), 5_000 + 1_000_000);
    }

    #[test]
    fn slots_are_kept_and_new_indices_take_the_lowest_free_one() {
        let p = PortIndex;
        let mut prev = vec![None; 6];
        let a = assign_slots(&prev, &[p(7), p(8)]);
        assert_eq!(a, vec![None, Some(p(7)), Some(p(8)), None, None, None], "slot 0 stays the program's");
        prev = a;
        // 7 goes away, 9 arrives: 8 keeps its slot, 9 takes the one 7 left
        let b = assign_slots(&prev, &[p(8), p(9)]);
        assert_eq!(b, vec![None, Some(p(9)), Some(p(8)), None, None, None]);
        // More than the slots hold: the rest get none
        let many: Vec<PortIndex> = (0..20).map(p).collect();
        let c = assign_slots(&vec![None; 6], &many);
        assert_eq!(c.iter().filter(|s| s.is_some()).count(), 5);
        assert_eq!(c[0], None);
    }

    #[test]
    fn the_link_installs_and_uninstalls_its_state() {
        let shared = JackMidiShared::new();
        let (mut link, _rx) = JackMidiLink::start(&shared, Vec::new(), ProducerIndex(1)).unwrap();
        let (_tx, rx) = ring::<MidiOutEvent>(8);
        link.add_output_source(rx).unwrap();
        assert!(lock(&shared.inner.rt).is_some());
        // A second link takes over; the first one's drop leaves the second's state alone
        let (second, _rx2) = JackMidiLink::start(&shared, Vec::new(), ProducerIndex(1)).unwrap();
        drop(link);
        assert!(lock(&shared.inner.rt).is_some());
        drop(second);
        assert!(lock(&shared.inner.rt).is_none());
    }

    #[test]
    fn a_link_without_a_backend_has_no_ports_and_no_events() {
        let shared = JackMidiShared::new();
        let (mut link, _rx) = JackMidiLink::start(&shared, Vec::new(), ProducerIndex(1)).unwrap();
        let mut n = 0;
        link.poll(|_| n += 1);
        link.activity(|_| n += 1);
        link.sysex(|_| n += 1);
        assert_eq!(n, 0);
        assert!(link.ports().is_empty());
    }
}
