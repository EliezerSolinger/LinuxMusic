//! The MIDI devices of the session (interfaces.md, I-21.1, I-10.2, C6; decisoes-autonomas.md P-15, P-16, P-36, P-38).
//!
//! - **A port has a stable identity:** client name + port name ([`PortId`]), never the ALSA numbers, which change at
//!   every connection. The machine's preferences give each port a [`PortSettings`]: input and output on or off, the
//!   role (instrument, controller or both), the input and output latency, an alias; the session gives each alias a
//!   [`PortIndex`] that stays the same for the whole session, through disconnections.
//! - **One sequencer client, "LinuxMusicBeta"**, with the program's own ports "LinuxMusicBeta entrada" (others connect
//!   to it) and "LinuxMusicBeta saída". Every enabled input is connected to "entrada"; every event is stamped by the
//!   queue's real time, converted to `CLOCK_MONOTONIC` (C6), minus the port's input latency, and goes into **one**
//!   command ring as `Command::Midi { port, at, bytes }` (sized for 2 s of 16 ports at 1000 events per second, C4; a
//!   full ring counts the loss, reported as [`MidiDeviceEvent::Lost`]).
//! - **Hotplug:** the sequencer's announcements trigger a new listing; a configured port that appears is connected
//!   ([`MidiDeviceEvent::Connected`]), one that goes away is reported ([`MidiDeviceEvent::Disconnected`]) and
//!   reconnected by itself when the same identity comes back.
//! - **Activity:** every input message, mapped or not, also goes to a lossy ring ([`MidiActivity`]) for "learn" and the
//!   activity monitor (I-21.2).
//! - **Raw MIDI** (P-16): a configured raw port gets its own reader thread and its own command ring, handed to the
//!   session as [`MidiDeviceEvent::ProducerReady`] (the renderer's rings are single-producer).

use std::fmt;

use lm_base::{Error, HostTime};
use lm_render::{PortIndex, ProducerIndex};

/// The stable identity of a port (I-21.1): never the ALSA client or port number. A raw MIDI port is
/// `{ client: <card id>, port: "MIDI <device>" }`
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortId {
    pub client: String,
    pub port: String,
}

impl PortId {
    pub fn new(client: &str, port: &str) -> Self {
        Self { client: client.to_owned(), port: port.to_owned() }
    }
}

impl fmt::Display for PortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.client, self.port)
    }
}

/// Which backend a port belongs to
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MidiBackendKind {
    Seq,
    Raw,
}

/// What a port is for (I-21.1): the compiler's port table routes instrument ports to the armed tracks and controller
/// ports to the mappings
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PortRole {
    #[default]
    Instrument,
    Controller,
    Both,
}

/// One port as the machine's preferences describe it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSettings {
    /// Given by the session for the alias; stays the same through disconnections
    pub index: PortIndex,
    pub id: PortId,
    pub backend: MidiBackendKind,
    pub alias: String,
    pub role: PortRole,
    pub input: bool,
    pub output: bool,
    /// Subtracted from every input stamp (P-15), nanoseconds
    pub input_latency_ns: i64,
    /// Every output event is sent this much earlier, nanoseconds
    pub output_latency_ns: i64,
}

/// A port found on the machine
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub id: PortId,
    pub backend: MidiBackendKind,
    /// It sends (we can read it)
    pub readable: bool,
    /// It receives (we can write to it)
    pub writable: bool,
    pub hardware: bool,
}

/// What the MIDI devices tell the session (lossless)
#[derive(Debug)]
pub enum MidiDeviceEvent {
    /// A port appeared on the machine
    Appeared(PortInfo),
    /// A port went away
    Gone(PortId),
    /// A configured port is connected (at the start, or back after a disconnection: the I-14 notice)
    Connected {
        index: PortIndex,
        id: PortId,
    },
    /// A configured port went away: "desconectado" in the interface; it reconnects by itself
    Disconnected {
        index: PortIndex,
        id: PortId,
    },
    /// Messages lost because a ring was full (`index`: the raw port, or `None` for the sequencer's ring)
    Lost {
        index: Option<PortIndex>,
        count: u64,
    },
    /// Output events dropped (their port is not connected, or the output fell behind)
    OutputDropped {
        index: Option<PortIndex>,
        count: u64,
    },
    /// A raw port's reader is running: install its ring (`Install::add_producer`)
    ProducerReady(lm_render::CommandReceiver),
    /// A raw port's reader ended: remove its ring (`Install::remove_producer`)
    ProducerDone(ProducerIndex),
    Failed {
        index: Option<PortIndex>,
        error: Error,
    },
}

/// One input message, for "learn" and the activity monitor (lossy ring, I-21.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiActivity {
    pub port: PortIndex,
    /// `CLOCK_MONOTONIC`, minus the port's input latency
    pub at: HostTime,
    pub len: u8,
    pub bytes: [u8; 3],
}

/// The length of a short message from its status (running status never reaches here)
pub fn message_len(status: u8) -> u8 {
    match status & 0xf0 {
        0x80 | 0x90 | 0xa0 | 0xb0 | 0xe0 => 3,
        0xc0 | 0xd0 => 2,
        _ => match status {
            0xf1 | 0xf3 => 2,
            0xf2 => 3,
            _ => 1,
        },
    }
}

/// Input ring capacity (C4): 16 ports at 1000 events per second for 2 s (the renderer drains it every block)
pub const INPUT_RING: usize = 32_768;
/// Activity ring capacity: 2 s of 16 busy ports; lossy, counted
pub const ACTIVITY_RING: usize = 32_768;

/// The client name of the program's own sequencer client
pub const CLIENT_NAME: &str = "LinuxMusicBeta";
/// The program's own input port: others connect to it (I-21.1)
pub const OWN_INPUT: &str = "LinuxMusicBeta entrada";
/// The program's own output port: others subscribe to it
pub const OWN_OUTPUT: &str = "LinuxMusicBeta saída";

/// A SysEx message to send (I-21.3: the controller profiles, the MCU display): `data[..len]` is F0 ... F7
#[derive(Clone, Copy)]
pub struct SysexOut {
    pub port: PortIndex,
    pub at: HostTime,
    pub len: u16,
    pub data: [u8; SYSEX_OUT_MAX],
}

impl fmt::Debug for SysexOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SysexOut").field("port", &self.port).field("at", &self.at).field("len", &self.len).finish()
    }
}

/// The longest SysEx the session sends (an MCU display line is 7 + 56 + 1 bytes)
pub const SYSEX_OUT_MAX: usize = 512;
/// The longest SysEx received (longer ones are counted and dropped by the binding)
pub const SYSEX_IN_MAX: usize = 4096;

/// A SysEx message received: to the session only (controller profiles, the MCU handshake); never recorded (I-21.1)
#[derive(Clone)]
pub struct SysexIn {
    pub port: PortIndex,
    pub at: HostTime,
    pub data: Vec<u8>,
}

impl fmt::Debug for SysexIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SysexIn")
            .field("port", &self.port)
            .field("at", &self.at)
            .field("len", &self.data.len())
            .finish()
    }
}

#[cfg(target_os = "linux")]
pub use linux::{MidiDevices, raw_devices};

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::VecDeque;
    use std::fmt;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use lm_base::ring::{Consumer, Producer, ring};
    use lm_base::{Error, HostTime, Result};
    use lm_render::{Command, CommandReceiver, CommandSender, MidiOutEvent, PortIndex, ProducerIndex, command_ring};
    use lm_sys_alsa::{Addr, MidiClient, MidiMessage, MidiStopper, MidiWaker, ReadOutcome, SeqEvent, SeqPortInfo};

    use super::*;
    use crate::midi::{MidiInput as RawInput, MidiSink};

    enum Control {
        Settings(Arc<Vec<PortSettings>>),
        Output(Consumer<MidiOutEvent>),
        SysexOutput(Consumer<SysexOut>),
    }

    /// The session's MIDI devices (I-21.1). Owned by the session; its `Drop` stops every thread
    pub struct MidiDevices {
        control: Producer<Control>,
        events: Consumer<MidiDeviceEvent>,
        activity: Consumer<MidiActivity>,
        sysex: Consumer<SysexIn>,
        activity_lost: Arc<AtomicU64>,
        stopper: MidiStopper,
        waker: MidiWaker,
        thread: Option<JoinHandle<()>>,
        ports: Arc<std::sync::Mutex<Vec<PortInfo>>>,
        raw: Vec<RawReader>,
        raw_producers: Vec<ProducerIndex>,
        raw_events: Vec<MidiDeviceEvent>,
        settings: Arc<Vec<PortSettings>>,
        own: (Addr, Addr),
        raw_out: RawOutput,
    }

    struct RawReader {
        index: PortIndex,
        id: PortId,
        producer: ProducerIndex,
        reader: RawInput,
        reported_failure: bool,
    }

    impl fmt::Debug for MidiDevices {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("MidiDevices").field("settings", &self.settings.len()).field("raw", &self.raw.len()).finish()
        }
    }

    /// Where the sequencer thread sends what it reads
    struct Feed {
        sink: CommandSender,
        activity: Producer<MidiActivity>,
        activity_lost: Arc<AtomicU64>,
        sysex: Producer<SysexIn>,
        lost: u64,
        sysex_lost: u64,
    }

    impl Feed {
        fn deliver(&mut self, port: PortIndex, at: HostTime, bytes: [u8; 3], len: u8) {
            if self.sink.send(Command::Midi { port, at, bytes }).is_err() {
                self.lost += 1;
            }
            if self.activity.push(MidiActivity { port, at, len, bytes }).is_err() {
                self.activity_lost.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// The sequencer thread: reads, reconnects, and sends the output rings' events on the queue at their time
    struct SeqThread {
        client: MidiClient,
        own_in: Addr,
        own_out: Addr,
        settings: Arc<Vec<PortSettings>>,
        /// Connected sources: address → (index, input latency)
        routes: Vec<(Addr, PortIndex, i64)>,
        /// Output destinations: index → (address or None for our own port's subscribers, output latency)
        dests: Vec<(PortIndex, Option<Addr>, i64)>,
        seen: Vec<SeqPortInfo>,
        control: Consumer<Control>,
        outputs: Vec<Consumer<MidiOutEvent>>,
        sysex_outputs: Vec<Consumer<SysexOut>>,
        raw_out: Producer<RawOut>,
        raw_out_indices: Arc<std::sync::Mutex<Vec<PortIndex>>>,
        events: Producer<MidiDeviceEvent>,
        pending: VecDeque<MidiDeviceEvent>,
        feed: Feed,
        reported_lost: u64,
        reported_sysex_lost: u64,
        dropped_out: u64,
        reported_dropped_out: u64,
        ports_out: Arc<std::sync::Mutex<Vec<PortInfo>>>,
    }

    fn info_of(p: &SeqPortInfo) -> PortInfo {
        PortInfo {
            id: PortId::new(&p.client_name, &p.port_name),
            backend: MidiBackendKind::Seq,
            readable: p.readable,
            writable: p.writable,
            hardware: p.hardware,
        }
    }

    fn is_own(id: &PortId, port: &str) -> bool {
        id.client == CLIENT_NAME && id.port == port
    }

    impl SeqThread {
        fn emit(&mut self, e: MidiDeviceEvent) {
            self.pending.push_back(e);
        }

        fn flush(&mut self) {
            if self.feed.lost > self.reported_lost {
                let count = self.feed.lost - self.reported_lost;
                self.reported_lost = self.feed.lost;
                self.pending.push_back(MidiDeviceEvent::Lost { index: None, count });
            }
            if self.feed.sysex_lost > self.reported_sysex_lost {
                let count = self.feed.sysex_lost - self.reported_sysex_lost;
                self.reported_sysex_lost = self.feed.sysex_lost;
                self.pending.push_back(MidiDeviceEvent::Lost { index: None, count });
            }
            if self.dropped_out > self.reported_dropped_out {
                let count = self.dropped_out - self.reported_dropped_out;
                self.reported_dropped_out = self.dropped_out;
                self.pending.push_back(MidiDeviceEvent::OutputDropped { index: None, count });
            }
            while let Some(e) = self.pending.pop_front() {
                if let Err(rtrb::PushError::Full(e)) = self.events.push(e) {
                    self.pending.push_front(e);
                    break;
                }
            }
        }

        /// The ports the user can use: musical, not the system's, not ours
        fn usable(&self, p: &SeqPortInfo) -> bool {
            p.midi && !p.system && !p.hidden && p.addr.client != self.own_in.client
        }

        /// Lists the ports again and makes the connections match the settings (hotplug, I-21.1)
        fn reconcile(&mut self) {
            let ports = match self.client.seq().ports() {
                Ok(p) => p,
                Err(error) => {
                    self.emit(MidiDeviceEvent::Failed { index: None, error });
                    return;
                }
            };
            let now: Vec<SeqPortInfo> = ports.into_iter().filter(|p| self.usable(p)).collect();
            let same = |a: &SeqPortInfo, b: &SeqPortInfo| a.client_name == b.client_name && a.port_name == b.port_name;
            let appeared: Vec<PortInfo> =
                now.iter().filter(|p| !self.seen.iter().any(|s| same(s, p))).map(info_of).collect();
            let gone: Vec<PortId> = self
                .seen
                .iter()
                .filter(|s| !now.iter().any(|p| same(s, p)))
                .map(|s| PortId::new(&s.client_name, &s.port_name))
                .collect();
            for p in appeared {
                self.emit(MidiDeviceEvent::Appeared(p));
            }
            for g in gone {
                self.emit(MidiDeviceEvent::Gone(g));
            }
            if let Ok(mut out) = self.ports_out.lock() {
                *out = now.iter().map(info_of).collect();
            }
            let settings = self.settings.clone();
            // Inputs: the sources that should be connected to our input port now
            let mut wanted: Vec<(Addr, PortIndex, i64, PortId)> = Vec::new();
            for s in
                settings.iter().filter(|s| s.backend == MidiBackendKind::Seq && s.input && !is_own(&s.id, OWN_INPUT))
            {
                if let Some(p) =
                    now.iter().find(|p| p.readable && p.client_name == s.id.client && p.port_name == s.id.port)
                {
                    wanted.push((p.addr, s.index, s.input_latency_ns, s.id.clone()));
                }
            }
            let old = std::mem::take(&mut self.routes);
            for (addr, index, _) in &old {
                if !wanted.iter().any(|(a, i, _, _)| a == addr && i == index) {
                    let id = settings
                        .iter()
                        .find(|s| s.index == *index)
                        .map_or_else(|| PortId::new("", ""), |s| s.id.clone());
                    if now.iter().any(|p| p.addr == *addr) {
                        // Still there but disabled: our side undoes the connection. A port that went away took it along
                        let own = self.own_in;
                        let _ = self.client.seq().disconnect(*addr, own);
                    }
                    self.emit(MidiDeviceEvent::Disconnected { index: *index, id });
                }
            }
            for (addr, index, latency, id) in wanted {
                if !old.iter().any(|(a, i, _)| *a == addr && *i == index) {
                    let own = self.own_in;
                    match self.client.seq().connect(addr, own) {
                        Ok(()) => self.emit(MidiDeviceEvent::Connected { index, id }),
                        Err(error) => {
                            self.emit(MidiDeviceEvent::Failed { index: Some(index), error });
                            continue;
                        }
                    }
                }
                self.routes.push((addr, index, latency));
            }
            // Outputs: where each output index goes now (our own port: its subscribers)
            let raw_indices: Vec<PortIndex> =
                settings.iter().filter(|s| s.backend == MidiBackendKind::Raw && s.output).map(|s| s.index).collect();
            if let Ok(mut r) = self.raw_out_indices.lock() {
                *r = raw_indices;
            }
            self.dests.clear();
            for s in settings.iter().filter(|s| s.backend == MidiBackendKind::Seq && s.output) {
                if is_own(&s.id, OWN_OUTPUT) {
                    self.dests.push((s.index, None, s.output_latency_ns));
                } else if let Some(p) =
                    now.iter().find(|p| p.writable && p.client_name == s.id.client && p.port_name == s.id.port)
                {
                    self.dests.push((s.index, Some(p.addr), s.output_latency_ns));
                }
            }
            self.seen = now;
        }

        /// Sends what the output rings hold, scheduled on the queue at each event's time minus the port's latency
        fn drain_outputs(&mut self) {
            for k in 0..self.outputs.len() {
                while let Ok(e) = self.outputs[k].pop() {
                    let raw = self.raw_out_indices.lock().map(|r| r.contains(&e.port)).unwrap_or(false);
                    if raw {
                        if self.raw_out.push(RawOut::Short(e)).is_err() {
                            self.dropped_out += 1;
                        }
                        continue;
                    }
                    let Some(&(_, dest, latency)) = self.dests.iter().find(|(i, _, _)| *i == e.port) else {
                        self.dropped_out += 1;
                        continue;
                    };
                    let Some(msg) = MidiMessage::from_bytes(&e.bytes[..usize::from(e.len.clamp(1, 3))]) else {
                        self.dropped_out += 1;
                        continue;
                    };
                    let at = HostTime(e.at.0.saturating_add_signed(-latency));
                    if self.client.send_at(self.own_out, dest, &msg, at).is_err() {
                        self.dropped_out += 1;
                    }
                }
            }
            for k in 0..self.sysex_outputs.len() {
                while let Ok(e) = self.sysex_outputs[k].pop() {
                    let Some(&(_, dest, latency)) = self.dests.iter().find(|(i, _, _)| *i == e.port) else {
                        self.dropped_out += 1;
                        continue;
                    };
                    let at = HostTime(e.at.0.saturating_add_signed(-latency));
                    let data = &e.data[..usize::from(e.len).min(SYSEX_OUT_MAX)];
                    if self.client.send_sysex_at(self.own_out, dest, data, at).is_err() {
                        self.dropped_out += 1;
                    }
                }
            }
        }

        fn take_control(&mut self) -> bool {
            let mut changed = false;
            while let Ok(c) = self.control.pop() {
                match c {
                    Control::Settings(s) => {
                        self.settings = s;
                        changed = true;
                    }
                    Control::Output(o) => self.outputs.push(o),
                    Control::SysexOutput(o) => self.sysex_outputs.push(o),
                }
            }
            changed
        }

        fn run(&mut self) {
            self.reconcile();
            self.flush();
            loop {
                if self.take_control() {
                    self.reconcile();
                }
                self.drain_outputs();
                let routes = &self.routes;
                let settings = &self.settings;
                let feed = &mut self.feed;
                let mut ports_changed = false;
                let own_index = settings
                    .iter()
                    .find(|s| s.input && is_own(&s.id, OWN_INPUT))
                    .map(|s| (s.index, s.input_latency_ns));
                let route_of =
                    |source: Addr| routes.iter().find(|(a, _, _)| *a == source).map(|(_, i, l)| (*i, *l)).or(own_index);
                // With output sources, the rings are looked at every millisecond even if nobody wakes the thread
                let timeout = if self.outputs.is_empty() && self.sysex_outputs.is_empty() { 50 } else { 1 };
                let r = self.client.read(Some(Duration::from_millis(timeout)), &mut |item| match item {
                    SeqEvent::Midi(ev) => {
                        if let Some((index, latency)) = route_of(ev.source) {
                            let at = HostTime(ev.time.0.saturating_add_signed(-latency));
                            feed.deliver(index, at, ev.message.bytes, ev.message.len);
                        }
                    }
                    SeqEvent::Sysex(sx) => {
                        if let Some((index, latency)) = route_of(sx.source) {
                            let at = HostTime(sx.time.0.saturating_add_signed(-latency));
                            if feed.sysex.push(SysexIn { port: index, at, data: sx.data.to_vec() }).is_err() {
                                feed.sysex_lost += 1;
                            }
                        }
                    }
                    SeqEvent::PortsChanged(_) => ports_changed = true,
                });
                match r {
                    Ok(ReadOutcome::Stopped) => break,
                    Ok(_) => {}
                    Err(error) => {
                        self.emit(MidiDeviceEvent::Failed { index: None, error });
                        self.flush();
                        break;
                    }
                }
                if ports_changed {
                    self.reconcile();
                }
                self.flush();
            }
            self.flush();
        }
    }

    /// What goes to the raw output thread
    enum RawOut {
        Short(MidiOutEvent),
        Settings(Vec<(PortIndex, PathBuf, i64)>),
        Stop,
    }

    /// Raw MIDI output (P-16): one thread writes each event to its device file at its time (raw MIDI has no queue).
    /// Events come from the sequencer thread, settings from the main thread: one ring each (single producer)
    struct RawOutput {
        settings: Producer<RawOut>,
        lost: Arc<AtomicU64>,
        thread: Option<JoinHandle<()>>,
    }

    impl RawOutput {
        fn start(events: Consumer<RawOut>) -> Result<RawOutput> {
            let (settings, settings_rx) = ring(64);
            let lost = Arc::new(AtomicU64::new(0));
            let l = lost.clone();
            let thread = std::thread::Builder::new()
                .name("lm-midi-raw-out".to_owned())
                .spawn(move || raw_output(events, settings_rx, &l))
                .map_err(|e| Error::new("io.midi.thread_failed").arg("detail", e.to_string()))?;
            Ok(RawOutput { settings, lost, thread: Some(thread) })
        }

        fn stop(&mut self) {
            if let Some(t) = self.thread.take() {
                let _ = self.settings.push(RawOut::Stop);
                t.thread().unpark();
                let _ = t.join();
            }
        }
    }

    fn raw_output(mut rx: Consumer<RawOut>, mut settings: Consumer<RawOut>, lost: &AtomicU64) {
        lm_base::set_role(lm_base::Role::Midi);
        let mut files: Vec<(PortIndex, std::fs::File, i64)> = Vec::new();
        // Events waiting for their time, in time order (equal times keep their arrival order)
        let mut queue: VecDeque<(HostTime, PortIndex, [u8; 3], u8)> = VecDeque::with_capacity(4096);
        loop {
            while let Ok(m) = settings.pop().or_else(|_| rx.pop()) {
                match m {
                    RawOut::Stop => return,
                    RawOut::Settings(list) => {
                        files.clear();
                        for (index, path, latency) in list {
                            match std::fs::OpenOptions::new().write(true).open(&path) {
                                Ok(f) => files.push((index, f, latency)),
                                Err(_) => {
                                    lost.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    RawOut::Short(e) => {
                        let latency = files.iter().find(|(i, _, _)| *i == e.port).map_or(0, |(_, _, l)| *l);
                        let at = HostTime(e.at.0.saturating_add_signed(-latency));
                        let pos = queue.iter().position(|(t, ..)| *t > at).unwrap_or(queue.len());
                        queue.insert(pos, (at, e.port, e.bytes, e.len));
                    }
                }
            }
            let now = HostTime::now();
            while queue.front().is_some_and(|(t, ..)| *t <= now) {
                let Some((_, port, bytes, len)) = queue.pop_front() else { break };
                match files.iter_mut().find(|(i, _, _)| *i == port) {
                    Some((_, f, _)) => {
                        use std::io::Write;
                        if f.write_all(&bytes[..usize::from(len.clamp(1, 3))]).is_err() {
                            lost.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        lost.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            let wait = queue.front().map_or(Duration::from_millis(1), |(t, ..)| {
                Duration::from_nanos(t.since(HostTime::now()).min(1_000_000))
            });
            std::thread::park_timeout(wait.max(Duration::from_micros(50)));
        }
    }

    impl MidiDevices {
        /// Opens the program's sequencer client with its own ports ("LinuxMusicBeta entrada" and "saída") and starts
        /// the MIDI thread (I-21.1). `seq` is the producer of the shared input ring (its receiver comes back:
        /// `Install::add_producer`); `raw` are the producers raw input ports may take, one each. Opening talks to the
        /// kernel: from an `Operation`
        pub fn start(
            settings: Vec<PortSettings>,
            seq: ProducerIndex,
            raw: Vec<ProducerIndex>,
        ) -> Result<(MidiDevices, CommandReceiver)> {
            let mut client = MidiClient::open(CLIENT_NAME, true)?;
            let own_in = client.create_input_port(OWN_INPUT)?;
            let own_out = client.create_output_port(OWN_OUTPUT)?;
            let stopper = client.stopper();
            let waker = client.waker();
            let (sink, receiver) = command_ring(seq, INPUT_RING);
            let (control, control_rx) = ring(64);
            let (events_tx, events) = ring(4096);
            let (activity_tx, activity) = ring(ACTIVITY_RING);
            let (sysex_tx, sysex) = ring(256);
            let (raw_tx, raw_rx) = ring(4096);
            let activity_lost = Arc::new(AtomicU64::new(0));
            let ports = Arc::new(std::sync::Mutex::new(Vec::new()));
            let raw_out_indices = Arc::new(std::sync::Mutex::new(Vec::new()));
            let settings = Arc::new(settings);
            // The raw output thread takes both the sequencer thread's events and the settings: two producers, so two
            // rings merged by the thread
            let raw_out = RawOutput::start(raw_rx)?;
            let mut state = SeqThread {
                client,
                own_in,
                own_out,
                settings: settings.clone(),
                routes: Vec::new(),
                dests: Vec::new(),
                seen: Vec::new(),
                control: control_rx,
                outputs: Vec::new(),
                sysex_outputs: Vec::new(),
                raw_out: raw_tx,
                raw_out_indices,
                events: events_tx,
                pending: VecDeque::new(),
                feed: Feed {
                    sink,
                    activity: activity_tx,
                    activity_lost: activity_lost.clone(),
                    sysex: sysex_tx,
                    lost: 0,
                    sysex_lost: 0,
                },
                reported_lost: 0,
                reported_sysex_lost: 0,
                dropped_out: 0,
                reported_dropped_out: 0,
                ports_out: ports.clone(),
            };
            let thread = std::thread::Builder::new()
                .name("lm-midi-seq".to_owned())
                .spawn(move || {
                    lm_base::set_role(lm_base::Role::Midi);
                    state.run();
                })
                .map_err(|e| Error::new("io.midi.thread_failed").arg("detail", e.to_string()))?;
            let mut devices = MidiDevices {
                control,
                events,
                activity,
                sysex,
                activity_lost,
                stopper,
                waker,
                thread: Some(thread),
                ports,
                raw: Vec::new(),
                raw_producers: raw,
                raw_events: Vec::new(),
                settings,
                own: (own_in, own_out),
                raw_out,
            };
            devices.reconcile_raw();
            Ok((devices, receiver))
        }
    }

    impl MidiDevices {
        /// The settings changed (aliases, roles, latencies, enable flags): the connections follow
        pub fn set_settings(&mut self, settings: Vec<PortSettings>) -> Result<()> {
            self.settings = Arc::new(settings);
            self.control.push(Control::Settings(self.settings.clone())).map_err(|_| Error::new("io.midi.busy"))?;
            self.waker.wake();
            self.reconcile_raw();
            Ok(())
        }

        /// A ring of output events (the renderer's, the session's feedback): sent at each event's time (I-21.1)
        pub fn add_output_source(&mut self, source: Consumer<MidiOutEvent>) -> Result<()> {
            self.control.push(Control::Output(source)).map_err(|_| Error::new("io.midi.busy"))?;
            self.waker.wake();
            Ok(())
        }

        /// A ring of SysEx to send (the MCU display)
        pub fn add_sysex_output(&mut self, source: Consumer<SysexOut>) -> Result<()> {
            self.control.push(Control::SysexOutput(source)).map_err(|_| Error::new("io.midi.busy"))?;
            self.waker.wake();
            Ok(())
        }

        /// Wakes the MIDI thread to send what was just pushed (real-time safe; otherwise it looks every millisecond)
        pub fn waker(&self) -> MidiWaker {
            self.waker.clone()
        }

        /// The sequencer ports seen at the last listing
        pub fn ports(&self) -> Vec<PortInfo> {
            self.ports.lock().map(|p| p.clone()).unwrap_or_default()
        }

        /// The program's own ports: (input, output), as (client, port)
        pub fn own_ports(&self) -> ((u8, u8), (u8, u8)) {
            ((self.own.0.client, self.own.0.port), (self.own.1.client, self.own.1.port))
        }

        /// From the pump: the devices' events
        pub fn poll(&mut self, mut f: impl FnMut(MidiDeviceEvent)) {
            while let Ok(e) = self.events.pop() {
                f(e);
            }
            let mut failed = false;
            for r in self.raw.iter_mut() {
                if r.reader.has_failed() && !r.reported_failure {
                    r.reported_failure = true;
                    failed = true;
                    f(MidiDeviceEvent::Disconnected { index: r.index, id: r.id.clone() });
                }
                let lost = r.reader.stats().lost;
                if lost > 0 {
                    f(MidiDeviceEvent::Lost { index: Some(r.index), count: lost });
                }
            }
            if failed {
                self.reconcile_raw();
            }
            let lost = self.raw_out.lost.swap(0, Ordering::Relaxed);
            if lost > 0 {
                f(MidiDeviceEvent::OutputDropped { index: None, count: lost });
            }
            for e in self.raw_events.drain(..) {
                f(e);
            }
        }

        /// Looks for raw devices that came back (the session calls it on a timer, e.g. every 2 s, and after a
        /// sequencer announcement)
        pub fn rescan_raw(&mut self) {
            self.reconcile_raw();
        }

        /// From the pump: every input message (lossy)
        pub fn activity(&mut self, mut f: impl FnMut(MidiActivity)) {
            while let Ok(a) = self.activity.pop() {
                f(a);
            }
        }

        /// Activity messages lost because the pump did not take them in time
        pub fn activity_lost(&self) -> u64 {
            self.activity_lost.load(Ordering::Relaxed)
        }

        /// From the pump: SysEx received (I-21.1: for the session's controller profiles only)
        pub fn sysex(&mut self, mut f: impl FnMut(SysexIn)) {
            while let Ok(s) = self.sysex.pop() {
                f(s);
            }
        }

        /// Raw ports (P-16): a reader for each enabled raw input present, the output thread's files
        fn reconcile_raw(&mut self) {
            let present = raw_devices();
            let wanted: Vec<PortSettings> = self
                .settings
                .iter()
                .filter(|s| s.backend == MidiBackendKind::Raw && s.input)
                .filter(|s| present.iter().any(|(id, _)| *id == s.id))
                .cloned()
                .collect();
            let mut keep = Vec::new();
            for r in self.raw.drain(..) {
                if wanted.iter().any(|s| s.index == r.index && s.id == r.id) && !r.reader.has_failed() {
                    keep.push(r);
                } else {
                    let producer = r.producer;
                    drop(r);
                    self.raw_producers.push(producer);
                    self.raw_events.push(MidiDeviceEvent::ProducerDone(producer));
                }
            }
            self.raw = keep;
            for s in wanted {
                if self.raw.iter().any(|r| r.index == s.index) {
                    continue;
                }
                let Some(path) = present.iter().find(|(id, _)| *id == s.id).map(|(_, p)| p.clone()) else { continue };
                let Some(producer) = self.raw_producers.pop() else {
                    self.raw_events.push(MidiDeviceEvent::Failed {
                        index: Some(s.index),
                        error: Error::new("io.midi.too_many_raw_ports"),
                    });
                    continue;
                };
                let (sink, receiver) = command_ring(producer, INPUT_RING);
                match RawInput::open_raw(&path, s.index, Box::new(Latency { sink, latency: s.input_latency_ns })) {
                    Ok(reader) => {
                        self.raw_events.push(MidiDeviceEvent::ProducerReady(receiver));
                        self.raw_events.push(MidiDeviceEvent::Connected { index: s.index, id: s.id.clone() });
                        self.raw.push(RawReader {
                            index: s.index,
                            id: s.id.clone(),
                            producer,
                            reader,
                            reported_failure: false,
                        });
                    }
                    Err(error) => {
                        self.raw_producers.push(producer);
                        self.raw_events.push(MidiDeviceEvent::Failed { index: Some(s.index), error });
                    }
                }
            }
            let outputs: Vec<(PortIndex, PathBuf, i64)> = self
                .settings
                .iter()
                .filter(|s| s.backend == MidiBackendKind::Raw && s.output)
                .filter_map(|s| {
                    present.iter().find(|(id, _)| *id == s.id).map(|(_, p)| (s.index, p.clone(), s.output_latency_ns))
                })
                .collect();
            let _ = self.raw_out.settings.push(RawOut::Settings(outputs));
        }
    }

    /// A raw reader's sink: the port's latency subtracted, into the port's own ring
    struct Latency {
        sink: CommandSender,
        latency: i64,
    }

    impl MidiSink for Latency {
        fn deliver(&mut self, port: PortIndex, at: HostTime, bytes: [u8; 3]) -> bool {
            let at = HostTime(at.0.saturating_add_signed(-self.latency));
            self.sink.send(Command::Midi { port, at, bytes }).is_ok()
        }
    }

    /// The raw MIDI devices present (P-16): their identity (card id, `"MIDI <device>"`) and file
    pub fn raw_devices() -> Vec<(PortId, PathBuf)> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir("/dev/snd") else { return out };
        for e in entries.flatten() {
            let Some(name) = e.file_name().to_str().map(str::to_owned) else { continue };
            let Some(rest) = name.strip_prefix("midiC") else { continue };
            let Some((card, dev)) = rest.split_once('D') else { continue };
            let (Ok(card), Ok(dev)) = (card.parse::<u32>(), dev.parse::<u32>()) else { continue };
            let id = std::fs::read_to_string(format!("/proc/asound/card{card}/id"))
                .unwrap_or_else(|_| format!("card{card}"));
            out.push((PortId::new(id.trim(), &format!("MIDI {dev}")), e.path()));
        }
        out.sort();
        out
    }

    impl Drop for MidiDevices {
        fn drop(&mut self) {
            self.stopper.stop();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            self.raw.clear();
            self.raw_out.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_lengths_follow_the_status() {
        assert_eq!(message_len(0x90), 3);
        assert_eq!(message_len(0xb5), 3);
        assert_eq!(message_len(0xc0), 2);
        assert_eq!(message_len(0xd3), 2);
        assert_eq!(message_len(0xe0), 3);
        assert_eq!(message_len(0xf2), 3);
        assert_eq!(message_len(0xf3), 2);
        assert_eq!(message_len(0xf8), 1);
    }

    #[test]
    fn identity_is_by_names() {
        let a = PortId::new("Launchkey", "MIDI 1");
        assert_eq!(a, PortId::new("Launchkey", "MIDI 1"));
        assert_ne!(a, PortId::new("Launchkey", "MIDI 2"));
        assert_eq!(a.to_string(), "Launchkey: MIDI 1");
    }
}
