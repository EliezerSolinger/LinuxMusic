//! MIDI input (interfaces.md, I-10.2, I-7.1; C6; decisoes-autonomas.md P-16).
//!
//! Each input has its own thread (it blocks reading) and its own command ring: the session makes the ring with
//! [`midi_ring`] (`ProducerIndex` 1 and up; 0 is the main thread's) and hands the receiver to the renderer with
//! `Install::add_producer`; the input pushes `Command::Midi { port, at, bytes }`. The ring never waits: a full one
//! counts the message as lost ([`MidiStats::lost`]), reported by the session.
//!
//! - **ALSA sequencer** first: a client with one input port connected to the source; time stamps from a real-time
//!   queue converted to the host clock (`CLOCK_MONOTONIC`, C6), or on arrival when no queue can be made (more jitter,
//!   reported by [`MidiInput::stamps_on_arrival`]).
//! - **ALSA raw MIDI** second (P-16): the `/dev/snd/midiC<card>D<device>` byte stream, parsed with running status,
//!   real-time bytes inside messages and sysex skipped (F1 has no sysex, T14); stamped on arrival.
//!
//! [`MidiInput::stop`] (and `Drop`) waits for the thread to leave. Listing the ports is an `Operation`
//! ([`list_midi_ports`]).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

use lm_base::HostTime;
use lm_render::{COMMAND_RING_CAPACITY, Command, CommandReceiver, CommandSender, PortIndex, ProducerIndex, command_ring};

/// Where a MIDI input's messages go
pub trait MidiSink: Send {
    /// One short message; false when it could not be taken (counted as lost)
    fn deliver(&mut self, port: PortIndex, at: HostTime, bytes: [u8; 3]) -> bool;
}

impl MidiSink for CommandSender {
    fn deliver(&mut self, port: PortIndex, at: HostTime, bytes: [u8; 3]) -> bool {
        self.send(Command::Midi { port, at, bytes }).is_ok()
    }
}

/// A MIDI input's command ring (I-7.1): `producer` 1 and up. The capacity is the engine's (C4: 16 ports at 1000
/// events per second during one block)
pub fn midi_ring(producer: ProducerIndex) -> (CommandSender, CommandReceiver) {
    command_ring(producer, COMMAND_RING_CAPACITY)
}

/// What an input counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MidiStats {
    /// Messages delivered to the ring
    pub received: u64,
    /// Messages the ring had no room for (Parte 2, rule 4: counted and reported)
    pub lost: u64,
    /// Times the kernel's buffer overflowed (messages lost before we could read them)
    pub overruns: u64,
    /// Messages that are not short MIDI (sysex in F1) or could not be parsed
    pub dropped: u64,
}

#[derive(Debug, Default)]
struct Counters {
    received: AtomicU64,
    lost: AtomicU64,
    overruns: AtomicU64,
    dropped: AtomicU64,
    ports_changed: AtomicBool,
    failed: AtomicBool,
}

/// Parses a raw MIDI byte stream into short messages (P-16): running status, system real-time bytes anywhere
/// (even inside another message), system common messages, and sysex skipped to its end. Platform independent
#[derive(Clone, Copy, Debug, Default)]
pub struct RawParser {
    running: u8,
    data: [u8; 2],
    have: usize,
    in_sysex: bool,
}

/// What one byte completed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parsed {
    Message([u8; 3]),
    /// A sysex ended (skipped: not in F1)
    SysexSkipped,
}

impl RawParser {
    fn data_len(status: u8) -> usize {
        match status & 0xf0 {
            0x80 | 0x90 | 0xa0 | 0xb0 | 0xe0 => 2,
            0xc0 | 0xd0 => 1,
            _ => match status {
                0xf1 | 0xf3 => 1,
                0xf2 => 2,
                _ => 0,
            },
        }
    }

    /// Feeds one byte
    pub fn push(&mut self, b: u8) -> Option<Parsed> {
        if b >= 0xf8 {
            // Real time: one byte, anywhere, never disturbs the running status
            return (b != 0xf9 && b != 0xfd).then_some(Parsed::Message([b, 0, 0]));
        }
        if b == 0xf0 {
            self.in_sysex = true;
            self.running = 0;
            return None;
        }
        if b == 0xf7 {
            let was = self.in_sysex;
            self.in_sysex = false;
            return was.then_some(Parsed::SysexSkipped);
        }
        if b & 0x80 != 0 {
            // A status ends a sysex without its end byte
            let ended_sysex = std::mem::take(&mut self.in_sysex);
            self.have = 0;
            if b >= 0xf0 {
                // System common: cancels the running status
                self.running = 0;
                if Self::data_len(b) == 0 {
                    return Some(Parsed::Message([b, 0, 0]));
                }
                self.running = b;
            } else {
                self.running = b;
            }
            return ended_sysex.then_some(Parsed::SysexSkipped);
        }
        if self.in_sysex || self.running == 0 {
            return None;
        }
        self.data[self.have] = b;
        self.have += 1;
        let need = Self::data_len(self.running);
        if self.have < need {
            return None;
        }
        self.have = 0;
        let msg = [self.running, self.data[0], if need == 2 { self.data[1] } else { 0 }];
        if self.running >= 0xf0 {
            // System common has no running status
            self.running = 0;
        }
        Some(Parsed::Message(msg))
    }
}

/// How an input reads
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MidiSource {
    /// A sequencer port (client, port); `None`: no connection made here (others connect to our port)
    Seq(Option<(u8, u8)>),
    /// A raw MIDI device file (`/dev/snd/midiC1D0`)
    Raw(std::path::PathBuf),
}

/// One port the user can choose (U17-like: listed in an `Operation`)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiPortInfo {
    pub source: MidiSource,
    pub name: String,
    /// A hardware port (a USB keyboard), not a program
    pub hardware: bool,
}

enum Stopper {
    #[cfg(target_os = "linux")]
    Seq(lm_sys_alsa::MidiStopper),
    #[cfg(target_os = "linux")]
    Raw(Arc<std::os::fd::OwnedFd>),
}

/// A running MIDI input: its thread and its counters
pub struct MidiInput {
    name: String,
    port: PortIndex,
    stopper: Option<Stopper>,
    thread: Option<JoinHandle<()>>,
    counters: Arc<Counters>,
    on_arrival: bool,
}

impl fmt::Debug for MidiInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiInput")
            .field("name", &self.name)
            .field("port", &self.port)
            .field("stats", &self.stats())
            .finish()
    }
}

impl MidiInput {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn port(&self) -> PortIndex {
        self.port
    }

    pub fn stats(&self) -> MidiStats {
        let c = &self.counters;
        MidiStats {
            received: c.received.load(Ordering::Relaxed),
            lost: c.lost.load(Ordering::Relaxed),
            overruns: c.overruns.load(Ordering::Relaxed),
            dropped: c.dropped.load(Ordering::Relaxed),
        }
    }

    /// True when the messages are stamped on arrival (no sequencer queue, or raw MIDI): more jitter (C6)
    pub fn stamps_on_arrival(&self) -> bool {
        self.on_arrival
    }

    /// Ports came or went since the last call (the session lists them again, U11)
    pub fn take_ports_changed(&self) -> bool {
        self.counters.ports_changed.swap(false, Ordering::AcqRel)
    }

    /// The thread stopped on an error (the device went away)
    pub fn has_failed(&self) -> bool {
        self.counters.failed.load(Ordering::Acquire)
    }

    /// Stops the thread and waits for it (I-10.2)
    pub fn stop(&mut self) {
        if let Some(stopper) = self.stopper.take() {
            match stopper {
                #[cfg(target_os = "linux")]
                Stopper::Seq(s) => s.stop(),
                #[cfg(target_os = "linux")]
                Stopper::Raw(fd) => {
                    let _ = rustix::io::write(&*fd, &1u64.to_ne_bytes());
                }
            }
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Opens a sequencer input connected to `source` (`(client, port)`), delivering to `sink` as `port`. Not
    /// real-time; opening talks to the kernel: from an `Operation`
    #[cfg(target_os = "linux")]
    pub fn open_seq(name: &str, source: Option<(u8, u8)>, port: PortIndex, mut sink: Box<dyn MidiSink>) -> lm_base::Result<MidiInput> {
        use lm_sys_alsa::{Addr, ReadOutcome, SeqInput};
        let mut input = lm_sys_alsa::MidiInput::open("LinuxMusic", name, true)?;
        if let Some((client, p)) = source {
            let ours = input.addr();
            input.seq().connect(Addr::new(client, p), ours)?;
        }
        let on_arrival = input.stamps_on_arrival();
        let stopper = input.stopper();
        let counters = Arc::new(Counters::default());
        let c = counters.clone();
        let thread = std::thread::Builder::new()
            .name(format!("lm-midi-{}", port.0))
            .spawn(move || {
                lm_base::set_role(lm_base::Role::Midi);
                loop {
                    let r = input.read(None, &mut |item| match item {
                        SeqInput::Midi(ev) => {
                            let m = ev.message;
                            if sink.deliver(port, ev.time, m.bytes) {
                                c.received.fetch_add(1, Ordering::Relaxed);
                            } else {
                                c.lost.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        SeqInput::PortsChanged => c.ports_changed.store(true, Ordering::Release),
                    });
                    let stats = input.stats();
                    c.overruns.store(stats.overruns, Ordering::Relaxed);
                    c.dropped.store(stats.dropped, Ordering::Relaxed);
                    match r {
                        Ok(ReadOutcome::Stopped) => break,
                        Ok(_) => {}
                        Err(_) => {
                            c.failed.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            })
            .map_err(|e| lm_base::Error::new("io.midi.thread_failed").arg("detail", e.to_string()))?;
        Ok(MidiInput {
            name: name.to_owned(),
            port,
            stopper: Some(Stopper::Seq(stopper)),
            thread: Some(thread),
            counters,
            on_arrival,
        })
    }

    /// Opens a raw MIDI device file (P-16), delivering to `sink` as `port`, stamped on arrival
    #[cfg(target_os = "linux")]
    pub fn open_raw(path: &std::path::Path, port: PortIndex, mut sink: Box<dyn MidiSink>) -> lm_base::Result<MidiInput> {
        use rustix::event::{EventfdFlags, PollFd, PollFlags, eventfd, poll};
        use rustix::fs::{Mode, OFlags, open};
        let fd = open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map_err(|e| {
            let key = match e {
                rustix::io::Errno::ACCESS | rustix::io::Errno::PERM => "io.device.permission_denied",
                rustix::io::Errno::BUSY => "io.device.busy",
                rustix::io::Errno::NOENT | rustix::io::Errno::NODEV | rustix::io::Errno::NXIO => "io.device.not_found",
                _ => "io.midi.open_failed",
            };
            lm_base::Error::new(key).arg("device", path.display().to_string()).arg("errno", i64::from(e.raw_os_error()))
        })?;
        let wake = Arc::new(
            eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)
                .map_err(|e| lm_base::Error::new("io.midi.open_failed").arg("errno", i64::from(e.raw_os_error())))?,
        );
        let wake_thread = wake.clone();
        let counters = Arc::new(Counters::default());
        let c = counters.clone();
        let thread = std::thread::Builder::new()
            .name(format!("lm-midi-{}", port.0))
            .spawn(move || {
                lm_base::set_role(lm_base::Role::Midi);
                let mut parser = RawParser::default();
                let mut buf = [0u8; 256];
                loop {
                    let mut fds = [PollFd::new(&fd, PollFlags::IN), PollFd::new(&*wake_thread, PollFlags::IN)];
                    match poll(&mut fds, None) {
                        Ok(_) => {}
                        Err(rustix::io::Errno::INTR) => continue,
                        Err(_) => {
                            c.failed.store(true, Ordering::Release);
                            break;
                        }
                    }
                    let (dev, stop) = (fds[0].revents(), fds[1].revents());
                    if stop.contains(PollFlags::IN) {
                        break;
                    }
                    if dev.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) && !dev.contains(PollFlags::IN) {
                        // The device went away
                        c.failed.store(true, Ordering::Release);
                        break;
                    }
                    match rustix::io::read(&fd, &mut buf) {
                        Ok(0) => {}
                        Ok(n) => {
                            let at = HostTime::now();
                            for &b in &buf[..n] {
                                match parser.push(b) {
                                    Some(Parsed::Message(m)) => {
                                        if sink.deliver(port, at, m) {
                                            c.received.fetch_add(1, Ordering::Relaxed);
                                        } else {
                                            c.lost.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    Some(Parsed::SysexSkipped) => {
                                        c.dropped.fetch_add(1, Ordering::Relaxed);
                                    }
                                    None => {}
                                }
                            }
                        }
                        Err(rustix::io::Errno::AGAIN) | Err(rustix::io::Errno::INTR) => {}
                        Err(_) => {
                            c.failed.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            })
            .map_err(|e| lm_base::Error::new("io.midi.thread_failed").arg("detail", e.to_string()))?;
        Ok(MidiInput {
            name: path.display().to_string(),
            port,
            stopper: Some(Stopper::Raw(wake)),
            thread: Some(thread),
            counters,
            on_arrival: true,
        })
    }
}

impl Drop for MidiInput {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Lists the MIDI ports to choose from (an `Operation`, I-10.2): the sequencer's readable ports (ours and the
/// system's excluded), then the raw MIDI devices
#[cfg(target_os = "linux")]
pub fn list_midi_ports(cancel: &lm_base::Cancel) -> lm_base::Result<Vec<MidiPortInfo>> {
    let mut out = Vec::new();
    match lm_sys_alsa::Seq::open("LinuxMusic scan") {
        Ok(mut seq) => {
            let me = seq.client_id();
            for p in seq.ports()? {
                if p.readable && p.midi && !p.system && !p.hidden && p.addr.client != me {
                    out.push(MidiPortInfo {
                        source: MidiSource::Seq(Some((p.addr.client, p.addr.port))),
                        name: format!("{}: {}", p.client_name, p.port_name),
                        hardware: p.hardware,
                    });
                }
            }
        }
        // No sequencer (snd-seq not loaded): the raw devices are still there
        Err(e) if e.key() == "alsa.seq_unavailable" => {}
        Err(e) => return Err(e),
    }
    cancel.check()?;
    if let Ok(entries) = std::fs::read_dir("/dev/snd") {
        let mut raw: Vec<_> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_owned();
                let rest = name.strip_prefix("midiC")?;
                let (card, dev) = rest.split_once('D')?;
                Some((card.parse::<u32>().ok()?, dev.parse::<u32>().ok()?, e.path()))
            })
            .collect();
        raw.sort();
        for (card, dev, path) in raw {
            let card_name = std::fs::read_to_string(format!("/proc/asound/card{card}/id")).unwrap_or_default();
            out.push(MidiPortInfo {
                source: MidiSource::Raw(path),
                name: format!("{} MIDI {dev} (raw)", card_name.trim()),
                hardware: true,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Vec<Parsed> {
        let mut p = RawParser::default();
        bytes.iter().filter_map(|b| p.push(*b)).collect()
    }

    #[test]
    fn running_status_and_real_time() {
        let out = parse(&[0x90, 60, 100, 62, 90, 0xf8, 64, 0xfe, 80, 0x80, 60, 0]);
        assert_eq!(
            out,
            vec![
                Parsed::Message([0x90, 60, 100]),
                Parsed::Message([0x90, 62, 90]),
                Parsed::Message([0xf8, 0, 0]),
                Parsed::Message([0xfe, 0, 0]),
                Parsed::Message([0x90, 64, 80]),
                Parsed::Message([0x80, 60, 0]),
            ]
        );
    }

    #[test]
    fn sysex_is_skipped_and_cancels_running_status() {
        let out = parse(&[0x90, 60, 100, 0xf0, 1, 2, 3, 0xf7, 61, 62, 0xc0, 5, 6]);
        assert_eq!(
            out,
            vec![Parsed::Message([0x90, 60, 100]), Parsed::SysexSkipped, Parsed::Message([0xc0, 5, 0]), Parsed::Message([0xc0, 6, 0])]
        );
    }

    #[test]
    fn system_common_and_stray_data() {
        let out = parse(&[12, 0xf2, 1, 2, 3, 0xf6, 0xb0, 7, 127]);
        assert_eq!(out, vec![Parsed::Message([0xf2, 1, 2]), Parsed::Message([0xf6, 0, 0]), Parsed::Message([0xb0, 7, 127])]);
    }

    #[test]
    fn command_sender_is_a_sink() {
        let (mut tx, _rx) = midi_ring(ProducerIndex(1));
        assert!(tx.deliver(PortIndex(2), HostTime(5), [0x90, 1, 2]));
        assert_eq!(tx.producer(), ProducerIndex(1));
    }
}
