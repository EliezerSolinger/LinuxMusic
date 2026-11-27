//! The ALSA sequencer: MIDI ports, their connections, and the client of the MIDI thread.
//!
//! Two kinds of client:
//! - [`Seq`]: a plain client for the control work (list ports, connect and disconnect, send at once).
//! - [`MidiClient`]: the program's client (I-21.1), owned by the one MIDI thread, which reads and sends. Its input
//!   ports stamp every event with the time of its own queue, running on real time; its output is scheduled on that
//!   same queue (`send_at`, `send_sysex_at`), so the message leaves at its exact time. One offset, measured at the
//!   start and again every second, converts between the queue's time and `CLOCK_MONOTONIC` host time both ways (C6).
//!   If the queue cannot be made, input is stamped on arrival and output leaves at once, and
//!   [`MidiClient::stamps_on_arrival`] says so (more jitter). The thread blocks in `poll`; its [`MidiWaker`] wakes it
//!   to send, its [`MidiStopper`] to leave; it is never killed.
//! - [`MidiInput`]: the input-only reader of the first version, a thin layer over [`MidiClient`].
//!
//! Connections can be made by any client (the control `Seq` connects a keyboard to the `MidiInput`'s port): the port
//! stamps whatever reaches it, whoever made the connection.

use super::ffi;
use super::poll::{Waker, last_errno};
use super::{c_string, check, keyed, text};
use crate::error::{Phase, errno};
use crate::midi::{SeqFields, midi_to_seq, seq_to_midi};
use crate::{Addr, MidiMessage};
use lm_base::{Error, HostTime, Result};
use std::ffi::{c_int, c_short, c_uint};
use std::fmt;
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A port of the sequencer, as listed by [`Seq::ports`]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeqPortInfo {
    pub addr: Addr,
    pub client_name: String,
    pub port_name: String,
    /// Others can subscribe to read from it: a source (a keyboard's output), something to connect to our input
    pub readable: bool,
    /// Others can subscribe to write to it: a destination (a synth's input)
    pub writable: bool,
    /// A hardware port, or a port of a kernel client
    pub hardware: bool,
    /// It speaks MIDI (`MIDI_GENERIC`)
    pub midi: bool,
    /// The system client (timer, announce): never a musical port
    pub system: bool,
    /// The port asked not to be listed for connections (`NO_EXPORT`)
    pub hidden: bool,
}

fn seq_open_error(r: c_int) -> Error {
    match r.saturating_abs() {
        errno::ENOENT | errno::ENODEV | errno::ENXIO => keyed("alsa.seq_unavailable", "snd_seq_open", r, ""),
        _ => crate::error::error_from_code(r, "snd_seq_open", "", Phase::Open),
    }
}

/// One sequencer client. Owns the `snd_seq_t` (closed on drop: its ports, connections and queues go with it).
/// `Send`, not `Sync`
pub struct Seq {
    raw: NonNull<ffi::snd_seq_t>,
    client: u8,
}

// SAFETY: a snd_seq_t may be used from any thread, one at a time; Seq is not Sync and its calls take &mut self
unsafe impl Send for Seq {}

impl fmt::Debug for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Seq").field("client", &self.client).finish()
    }
}

struct PortInfoObj(NonNull<ffi::snd_seq_port_info_t>);

impl PortInfoObj {
    fn new() -> Result<Self> {
        let mut p = ptr::null_mut();
        // SAFETY: valid out pointer; freed in Drop
        check(unsafe { ffi::snd_seq_port_info_malloc(&mut p) }, "snd_seq_port_info_malloc", "", Phase::Open)?;
        NonNull::new(p).map(Self).ok_or_else(|| Error::new("alsa.out_of_memory"))
    }
}

impl Drop for PortInfoObj {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_seq_port_info_malloc, freed once
        unsafe { ffi::snd_seq_port_info_free(self.0.as_ptr()) }
    }
}

struct ClientInfoObj(NonNull<ffi::snd_seq_client_info_t>);

impl Drop for ClientInfoObj {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_seq_client_info_malloc, freed once
        unsafe { ffi::snd_seq_client_info_free(self.0.as_ptr()) }
    }
}

struct SubscribeObj(NonNull<ffi::snd_seq_port_subscribe_t>);

impl Drop for SubscribeObj {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_seq_port_subscribe_malloc, freed once
        unsafe { ffi::snd_seq_port_subscribe_free(self.0.as_ptr()) }
    }
}

impl Seq {
    /// Opens a client (input and output, non-blocking) named `client_name`. `alsa.seq_unavailable` when the
    /// sequencer is missing (the `snd-seq` kernel module). Not real-time safe
    pub fn open(client_name: &str) -> Result<Seq> {
        let name = c_string(client_name, "client_name")?;
        let mut raw = ptr::null_mut();
        // SAFETY: valid out pointer and name; on success the handle is ours, closed in Drop
        let r = unsafe {
            ffi::snd_seq_open(&mut raw, c"default".as_ptr(), ffi::SND_SEQ_OPEN_DUPLEX, ffi::SND_SEQ_NONBLOCK)
        };
        if r < 0 {
            return Err(seq_open_error(r));
        }
        let raw = NonNull::new(raw).ok_or_else(|| Error::new("alsa.out_of_memory"))?;
        // From here the handle is owned: an error below closes it
        let mut seq = Seq { raw, client: 0 };
        // SAFETY: our open handle
        let client = unsafe { ffi::snd_seq_client_id(raw.as_ptr()) };
        seq.client = check(client, "snd_seq_client_id", "", Phase::Open)? as u8;
        // SAFETY: our open handle and a NUL-terminated name
        let r = unsafe { ffi::snd_seq_set_client_name(raw.as_ptr(), name.as_ptr()) };
        check(r, "snd_seq_set_client_name", "", Phase::Open)?;
        Ok(seq)
    }

    /// This client's number
    pub fn client_id(&self) -> u8 {
        self.client
    }

    pub(crate) fn raw(&self) -> *mut ffi::snd_seq_t {
        self.raw.as_ptr()
    }

    /// Every port of every client (this one included). Not real-time safe
    pub fn ports(&mut self) -> Result<Vec<SeqPortInfo>> {
        let raw = self.raw.as_ptr();
        let mut ci = ptr::null_mut();
        // SAFETY: valid out pointer
        check(unsafe { ffi::snd_seq_client_info_malloc(&mut ci) }, "snd_seq_client_info_malloc", "", Phase::Open)?;
        let ci = ClientInfoObj(NonNull::new(ci).ok_or_else(|| Error::new("alsa.out_of_memory"))?);
        let pi = PortInfoObj::new()?;
        let mut out = Vec::new();
        // SAFETY: (the whole walk) our handle and two live info objects; strings copied before the next query
        unsafe {
            ffi::snd_seq_client_info_set_client(ci.0.as_ptr(), -1);
            while ffi::snd_seq_query_next_client(raw, ci.0.as_ptr()) >= 0 {
                let client = ffi::snd_seq_client_info_get_client(ci.0.as_ptr());
                let client_name = text(ffi::snd_seq_client_info_get_name(ci.0.as_ptr()));
                let kernel = ffi::snd_seq_client_info_get_type(ci.0.as_ptr()) == ffi::SND_SEQ_KERNEL_CLIENT;
                ffi::snd_seq_port_info_set_client(pi.0.as_ptr(), client);
                ffi::snd_seq_port_info_set_port(pi.0.as_ptr(), -1);
                while ffi::snd_seq_query_next_port(raw, pi.0.as_ptr()) >= 0 {
                    let p = pi.0.as_ptr();
                    let caps = ffi::snd_seq_port_info_get_capability(p);
                    let kind = ffi::snd_seq_port_info_get_type(p);
                    out.push(SeqPortInfo {
                        addr: Addr::new(client as u8, ffi::snd_seq_port_info_get_port(p) as u8),
                        client_name: client_name.clone(),
                        port_name: text(ffi::snd_seq_port_info_get_name(p)),
                        readable: caps & ffi::SND_SEQ_PORT_CAP_READ != 0 && caps & ffi::SND_SEQ_PORT_CAP_SUBS_READ != 0,
                        writable: caps & ffi::SND_SEQ_PORT_CAP_WRITE != 0
                            && caps & ffi::SND_SEQ_PORT_CAP_SUBS_WRITE != 0,
                        hardware: kind & ffi::SND_SEQ_PORT_TYPE_HARDWARE != 0 || kernel,
                        midi: kind & ffi::SND_SEQ_PORT_TYPE_MIDI_GENERIC != 0,
                        system: client == c_int::from(ffi::SND_SEQ_CLIENT_SYSTEM),
                        hidden: caps & ffi::SND_SEQ_PORT_CAP_NO_EXPORT != 0,
                    });
                }
            }
        }
        Ok(out)
    }

    /// Creates a port others can subscribe to (to receive what this client sends from it). Not real-time safe
    pub fn create_output_port(&mut self, name: &str) -> Result<Addr> {
        self.create_port(name, ffi::SND_SEQ_PORT_CAP_READ | ffi::SND_SEQ_PORT_CAP_SUBS_READ, None)
    }

    /// A port with these capabilities; with `stamp_queue`, every event delivered to it is stamped with that queue's
    /// real time
    pub(crate) fn create_port(&mut self, name: &str, caps: c_uint, stamp_queue: Option<c_int>) -> Result<Addr> {
        let c_name = c_string(name, "port_name")?;
        let pi = PortInfoObj::new()?;
        let p = pi.0.as_ptr();
        // SAFETY: live info object, our handle
        unsafe {
            ffi::snd_seq_port_info_set_name(p, c_name.as_ptr());
            ffi::snd_seq_port_info_set_capability(p, caps);
            ffi::snd_seq_port_info_set_type(
                p,
                ffi::SND_SEQ_PORT_TYPE_MIDI_GENERIC
                    | ffi::SND_SEQ_PORT_TYPE_SOFTWARE
                    | ffi::SND_SEQ_PORT_TYPE_APPLICATION
                    | ffi::SND_SEQ_PORT_TYPE_PORT,
            );
            ffi::snd_seq_port_info_set_midi_channels(p, 16);
            if let Some(q) = stamp_queue {
                ffi::snd_seq_port_info_set_timestamping(p, 1);
                ffi::snd_seq_port_info_set_timestamp_real(p, 1);
                ffi::snd_seq_port_info_set_timestamp_queue(p, q);
            }
            check(ffi::snd_seq_create_port(self.raw.as_ptr(), p), "snd_seq_create_port", "", Phase::Open)?;
            Ok(Addr::new(self.client, ffi::snd_seq_port_info_get_port(p) as u8))
        }
    }

    /// Deletes a port of this client. Not real-time safe
    pub fn delete_port(&mut self, port: Addr) -> Result<()> {
        // SAFETY: our handle
        let r = unsafe { ffi::snd_seq_delete_port(self.raw.as_ptr(), c_int::from(port.port)) };
        check(r, "snd_seq_delete_port", "", Phase::Running).map(drop)
    }

    fn subscription(&self, sender: Addr, dest: Addr) -> Result<SubscribeObj> {
        let mut s = ptr::null_mut();
        // SAFETY: valid out pointer
        check(unsafe { ffi::snd_seq_port_subscribe_malloc(&mut s) }, "snd_seq_port_subscribe_malloc", "", Phase::Open)?;
        let s = SubscribeObj(NonNull::new(s).ok_or_else(|| Error::new("alsa.out_of_memory"))?);
        let sender = ffi::snd_seq_addr_t { client: sender.client, port: sender.port };
        let dest = ffi::snd_seq_addr_t { client: dest.client, port: dest.port };
        // SAFETY: live object; the addresses are copied in
        unsafe {
            ffi::snd_seq_port_subscribe_set_sender(s.0.as_ptr(), &sender);
            ffi::snd_seq_port_subscribe_set_dest(s.0.as_ptr(), &dest);
        }
        Ok(s)
    }

    /// Connects `sender` to `dest` (any two ports; `aconnect`'s job). Already connected is not an error.
    /// `alsa.seq_port_not_found` when a port does not exist, `alsa.permission_denied` when a port refuses.
    /// Not real-time safe
    pub fn connect(&mut self, sender: Addr, dest: Addr) -> Result<()> {
        let s = self.subscription(sender, dest)?;
        // SAFETY: our handle and a live subscription object
        let r = unsafe { ffi::snd_seq_subscribe_port(self.raw.as_ptr(), s.0.as_ptr()) };
        match r.saturating_abs() {
            0 | errno::EBUSY => Ok(()),
            errno::ENOENT | errno::ENXIO | errno::EINVAL => {
                Err(keyed("alsa.seq_port_not_found", "snd_seq_subscribe_port", r, "")
                    .arg("sender", addr_text(sender))
                    .arg("dest", addr_text(dest)))
            }
            _ => Err(crate::error::error_from_code(r, "snd_seq_subscribe_port", "", Phase::Running)),
        }
    }

    /// Undoes [`connect`](Seq::connect). Not connected is not an error. Not real-time safe
    pub fn disconnect(&mut self, sender: Addr, dest: Addr) -> Result<()> {
        let s = self.subscription(sender, dest)?;
        // SAFETY: our handle and a live subscription object
        let r = unsafe { ffi::snd_seq_unsubscribe_port(self.raw.as_ptr(), s.0.as_ptr()) };
        match r.saturating_abs() {
            0 | errno::ENOENT | errno::ENXIO => Ok(()),
            _ => Err(crate::error::error_from_code(r, "snd_seq_unsubscribe_port", "", Phase::Running)),
        }
    }

    /// Sends a message now (direct, not queued) from our port `from` to `dest`, or to every subscriber of `from` when
    /// `dest` is `None`. No allocation; one `write` (a MIDI output thread may call it, not the audio thread).
    /// `alsa.invalid_argument` for a message the sequencer cannot carry
    pub fn send(&mut self, from: Addr, dest: Option<Addr>, message: &MidiMessage) -> Result<()> {
        let fields = midi_to_seq(message).ok_or_else(|| {
            Error::new("alsa.invalid_argument")
                .arg("argument", "message")
                .arg("bytes", format!("{:02x?}", message.as_slice()))
        })?;
        let mut ev = event_from_fields(fields);
        ev.flags = ffi::SND_SEQ_TIME_STAMP_REAL | ffi::SND_SEQ_TIME_MODE_ABS | ffi::SND_SEQ_EVENT_LENGTH_FIXED;
        ev.queue = ffi::SND_SEQ_QUEUE_DIRECT;
        ev.source = ffi::snd_seq_addr_t { client: self.client, port: from.port };
        ev.dest = match dest {
            Some(d) => ffi::snd_seq_addr_t { client: d.client, port: d.port },
            None => {
                ffi::snd_seq_addr_t { client: ffi::SND_SEQ_ADDRESS_SUBSCRIBERS, port: ffi::SND_SEQ_ADDRESS_UNKNOWN }
            }
        };
        // SAFETY: our handle and a complete fixed-length event on the stack
        let r = unsafe { ffi::snd_seq_event_output_direct(self.raw.as_ptr(), &mut ev) };
        check(r, "snd_seq_event_output_direct", "", Phase::Running).map(drop)
    }
}

impl Drop for Seq {
    fn drop(&mut self) {
        // SAFETY: our handle, closed once
        unsafe { ffi::snd_seq_close(self.raw.as_ptr()) };
    }
}

fn addr_text(a: Addr) -> String {
    format!("{}:{}", a.client, a.port)
}

fn event_from_fields(f: SeqFields) -> ffi::snd_seq_event_t {
    let mut ev = ffi::snd_seq_event_t { type_: f.kind, ..Default::default() };
    match f.kind {
        crate::midi::ev::NOTEON | crate::midi::ev::NOTEOFF | crate::midi::ev::KEYPRESS => {
            ev.data[0] = u32::from_ne_bytes([f.channel, f.note, f.velocity, 0]);
        }
        _ => {
            ev.data[0] = u32::from_ne_bytes([f.channel, 0, 0, 0]);
            ev.data[1] = f.param;
            ev.data[2] = f.value as u32;
        }
    }
    ev
}

fn fields_from_event(ev: &ffi::snd_seq_event_t) -> SeqFields {
    let d0 = ev.data[0].to_ne_bytes();
    SeqFields {
        kind: ev.type_,
        channel: d0[0],
        note: d0[1],
        velocity: d0[2],
        param: ev.data[1],
        value: ev.data[2] as i32,
    }
}

/// How an event's time was taken
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// From the queue's real time when the kernel delivered it, converted to host time (C6)
    Queue,
    /// When the reader took it (no queue, or an event stamped by another queue)
    Arrival,
}

/// One MIDI message received
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiInputEvent {
    /// Our port it arrived at
    pub port: Addr,
    /// The port it came from (what lm-io maps to a `PortIndex`)
    pub source: Addr,
    pub time: HostTime,
    pub stamp: Stamp,
    pub message: MidiMessage,
}

/// One complete SysEx message received (`F0 ... F7`, reassembled when the sender cut it in pieces)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SysexEvent<'a> {
    pub port: Addr,
    pub source: Addr,
    /// The time of its first byte
    pub time: HostTime,
    pub stamp: Stamp,
    /// `F0 ... F7`, at most [`SYSEX_MAX`] bytes. Valid only during the sink call: copy what you keep
    pub data: &'a [u8],
}

/// The largest SysEx message the client sends or delivers (I-21.1). Longer incoming ones are counted in
/// [`InputStats::sysex_dropped`]
pub const SYSEX_MAX: usize = 4096;

/// What changed in the sequencer's graph (the system's announcements). The ALSA numbers only: the stable identity
/// (client and port names) comes from [`Seq::ports`], and after an exit the names are already gone, so keep a map
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortChange {
    ClientStart(u8),
    ClientExit(u8),
    ClientChange(u8),
    PortStart(Addr),
    PortExit(Addr),
    PortChange(Addr),
    Subscribed { sender: Addr, dest: Addr },
    Unsubscribed { sender: Addr, dest: Addr },
}

/// What [`MidiClient::read`] hands over
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqEvent<'a> {
    Midi(MidiInputEvent),
    Sysex(SysexEvent<'a>),
    /// Only when the client watches ports
    PortsChanged(PortChange),
}

/// What [`MidiInput::read`] hands over (the input-only reader; [`MidiClient`] is the full one)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqInput {
    Midi(MidiInputEvent),
    /// A client or port appeared, went away or changed (only with `watch_ports`): list the ports again (U11)
    PortsChanged,
}

/// Why a read returned
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// This many events went to the sink
    Delivered(usize),
    /// The [`MidiWaker`] was used: the thread has output to send (only [`MidiClient`])
    Woken,
    /// The stopper was used: the thread should leave
    Stopped,
    TimedOut,
}

/// Counters of what the client could not deliver or schedule (losses are counted and reported, never silent:
/// Parte 2, rule 4)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputStats {
    /// Times the kernel's input buffer overflowed (`ENOSPC`): events were lost
    pub overruns: u64,
    /// Events that are neither MIDI messages nor announcements (queue and echo events, and, for [`MidiInput`],
    /// SysEx)
    pub dropped: u64,
    /// SysEx messages longer than [`SYSEX_MAX`], or cut off by another message, not delivered
    pub sysex_dropped: u64,
    /// Messages sent at once instead of at their time, because there is no queue
    pub unscheduled: u64,
}

struct Signal {
    stopped: AtomicBool,
    woken: AtomicBool,
    waker: Waker,
}

/// Stops a client blocked in `read` on another thread: the read returns [`ReadOutcome::Stopped`], now and every time
/// after. `Send + Sync + Clone`; real-time safe (one `write` to an eventfd)
#[derive(Clone)]
pub struct MidiStopper(Arc<Signal>);

impl fmt::Debug for MidiStopper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiStopper").field("stopped", &self.0.stopped.load(Ordering::Relaxed)).finish()
    }
}

impl MidiStopper {
    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::Release);
        self.0.waker.wake();
    }
}

/// Wakes a [`MidiClient`] blocked in `read`, which returns [`ReadOutcome::Woken`] (once per wake, after delivering
/// what had arrived): the audio side has pushed output into its ring. `Send + Sync + Clone`; real-time safe (one
/// `write` to an eventfd, never blocks)
#[derive(Clone)]
pub struct MidiWaker(Arc<Signal>);

impl fmt::Debug for MidiWaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MidiWaker")
    }
}

impl MidiWaker {
    pub fn wake(&self) {
        self.0.woken.store(true, Ordering::Release);
        self.0.waker.wake();
    }
}

/// The offset between the queue's real time and the host clock
#[derive(Clone, Copy, Debug)]
struct QueueClock {
    /// host time = queue time + offset (nanoseconds)
    offset: i128,
    measured: Instant,
}

const REMEASURE: Duration = Duration::from_secs(1);

struct QueueStatusObj(NonNull<ffi::snd_seq_queue_status_t>);

impl Drop for QueueStatusObj {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_seq_queue_status_malloc, freed once
        unsafe { ffi::snd_seq_queue_status_free(self.0.as_ptr()) }
    }
}

struct Queue {
    id: c_int,
    status: QueueStatusObj,
    clock: QueueClock,
}

/// A SysEx message being reassembled
struct Assembly {
    port: Addr,
    source: Addr,
    time: HostTime,
    stamp: Stamp,
    /// Longer than SYSEX_MAX: the rest is skipped until its F7
    overflow: bool,
}

/// The program's sequencer client (I-21.1): one handle with its input and output ports, its real-time queue, and the
/// reader. Input ports stamp every event with the queue's time; output is scheduled on the same queue, so input and
/// output share one clock, converted to host time by one offset measured at the start and every second (C6).
///
/// Owned by the one MIDI thread, which both reads and sends (alsa-lib's sequencer handle is not thread-safe: every
/// call takes `&mut self`, and the type is `Send`, not `Sync`). Other threads reach it only through its
/// [`MidiStopper`] and [`MidiWaker`]. Nothing here is real-time: the audio side hands output over by a ring and wakes
/// the thread.
pub struct MidiClient {
    seq: Seq,
    queue: Option<Queue>,
    fds: Vec<libc::pollfd>,
    signal: Arc<Signal>,
    stats: InputStats,
    /// Preallocated to SYSEX_MAX: reassembly never allocates
    sysex: Vec<u8>,
    assembly: Option<Assembly>,
    announce_port: Option<u8>,
}

// SAFETY: the queue status object is used only through &mut self, by the one thread that owns the client; the Seq
// inside is Send for the same reason (one thread at a time). The Signal is Sync on its own
unsafe impl Send for MidiClient {}

impl fmt::Debug for MidiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiClient")
            .field("client", &self.seq.client_id())
            .field("queue", &self.queue.as_ref().map(|q| q.id))
            .field("stats", &self.stats)
            .finish()
    }
}

impl MidiClient {
    /// Opens the client `client_name` with its queue (and no ports yet). With `watch_ports`, the system's
    /// announcements arrive as [`SeqEvent::PortsChanged`] (through a hidden port of their own). Not real-time safe
    pub fn open(client_name: &str, watch_ports: bool) -> Result<MidiClient> {
        let mut seq = Seq::open(client_name)?;
        let queue = make_queue(&mut seq);
        let announce_port = if watch_ports {
            let port =
                seq.create_port("announcements", ffi::SND_SEQ_PORT_CAP_WRITE | ffi::SND_SEQ_PORT_CAP_NO_EXPORT, None)?;
            seq.connect(Addr::new(ffi::SND_SEQ_CLIENT_SYSTEM, ffi::SND_SEQ_PORT_SYSTEM_ANNOUNCE), port)?;
            Some(port.port)
        } else {
            None
        };
        let signal =
            Arc::new(Signal { stopped: AtomicBool::new(false), woken: AtomicBool::new(false), waker: Waker::new()? });
        let raw = seq.raw();
        // SAFETY: our handle
        let count = unsafe { ffi::snd_seq_poll_descriptors_count(raw, libc::POLLIN as c_short) };
        let count = check(count, "snd_seq_poll_descriptors_count", "", Phase::Open)? as usize;
        let mut fds = vec![libc::pollfd { fd: -1, events: 0, revents: 0 }; count];
        // SAFETY: room for `count` descriptors
        let got =
            unsafe { ffi::snd_seq_poll_descriptors(raw, fds.as_mut_ptr(), count as c_uint, libc::POLLIN as c_short) };
        fds.truncate(check(got, "snd_seq_poll_descriptors", "", Phase::Open)? as usize);
        fds.push(signal.waker.pollfd());
        let mut client = MidiClient {
            seq,
            queue,
            fds,
            signal,
            stats: InputStats::default(),
            sysex: Vec::with_capacity(SYSEX_MAX),
            assembly: None,
            announce_port,
        };
        client.remeasure();
        Ok(client)
    }

    /// An input port anyone can connect to; every event reaching it is stamped with the queue's time
    pub fn create_input_port(&mut self, name: &str) -> Result<Addr> {
        let q = self.queue.as_ref().map(|q| q.id);
        self.seq.create_port(name, ffi::SND_SEQ_PORT_CAP_WRITE | ffi::SND_SEQ_PORT_CAP_SUBS_WRITE, q)
    }

    /// An output port anyone can connect to
    pub fn create_output_port(&mut self, name: &str) -> Result<Addr> {
        self.seq.create_output_port(name)
    }

    /// The client, for connections and port listings made from this same thread
    pub fn seq(&mut self) -> &mut Seq {
        &mut self.seq
    }

    pub fn stopper(&self) -> MidiStopper {
        MidiStopper(self.signal.clone())
    }

    pub fn waker(&self) -> MidiWaker {
        MidiWaker(self.signal.clone())
    }

    /// True when there is no queue: input is stamped on arrival and output leaves at once (more jitter, C6)
    pub fn stamps_on_arrival(&self) -> bool {
        self.queue.is_none()
    }

    pub fn stats(&self) -> InputStats {
        self.stats
    }

    /// Measures the queue-to-host offset: the narrowest of three brackets around a queue status query
    fn remeasure(&mut self) {
        let raw = self.seq.raw();
        let Some(queue) = &mut self.queue else { return };
        let mut best: Option<(u64, i128)> = None;
        for _ in 0..3 {
            let t0 = HostTime::now();
            // SAFETY: our handle, queue and status object
            let r = unsafe { ffi::snd_seq_get_queue_status(raw, queue.id, queue.status.0.as_ptr()) };
            let t1 = HostTime::now();
            if r < 0 {
                continue;
            }
            // SAFETY: the status was just filled; the pointer is into it
            let rt = unsafe { *ffi::snd_seq_queue_status_get_real_time(queue.status.0.as_ptr()) };
            let queue_ns = i128::from(rt.tv_sec) * 1_000_000_000 + i128::from(rt.tv_nsec);
            let width = t1.since(t0);
            let mid = i128::from(t0.0) + i128::from(width / 2);
            if best.is_none_or(|(w, _)| width < w) {
                best = Some((width, mid - queue_ns));
            }
        }
        if let Some((_, offset)) = best {
            queue.clock = QueueClock { offset, measured: Instant::now() };
        }
    }

    fn remeasure_if_old(&mut self) {
        if self.queue.as_ref().is_some_and(|q| q.clock.measured.elapsed() >= REMEASURE) {
            self.remeasure();
        }
    }

    /// Hands every pending event to `sink`; if there is none, blocks until there is one, the waker or the stopper is
    /// used, or `timeout` passes (`None`: forever). Returns as soon as at least one event was delivered. No allocation
    /// per event. For the MIDI thread only (it blocks)
    pub fn read(&mut self, timeout: Option<Duration>, sink: &mut dyn FnMut(SeqEvent<'_>)) -> Result<ReadOutcome> {
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            if self.signal.stopped.load(Ordering::Acquire) {
                return Ok(ReadOutcome::Stopped);
            }
            self.remeasure_if_old();
            let delivered = self.drain(sink)?;
            if delivered > 0 {
                return Ok(ReadOutcome::Delivered(delivered));
            }
            if self.signal.woken.swap(false, Ordering::AcqRel) {
                return Ok(ReadOutcome::Woken);
            }
            let wait = match deadline {
                None => REMEASURE,
                Some(d) => {
                    let left = d.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Ok(ReadOutcome::TimedOut);
                    }
                    // Wake at least every REMEASURE, to keep the clock fresh
                    left.min(REMEASURE)
                }
            };
            let ms = i32::try_from(wait.as_millis().max(1)).unwrap_or(i32::MAX);
            for fd in &mut self.fds {
                fd.revents = 0;
            }
            // SAFETY: a valid array of pollfds
            let r = unsafe { libc::poll(self.fds.as_mut_ptr(), self.fds.len() as libc::nfds_t, ms) };
            if r < 0 {
                let e = last_errno();
                if e != libc::EINTR {
                    return Err(Error::new("alsa.error").arg("call", "poll").arg("errno", i64::from(e)));
                }
            }
            let waker_fd = self.fds[self.fds.len() - 1];
            if waker_fd.revents & libc::POLLIN != 0 {
                self.signal.waker.clear();
            }
        }
    }

    /// Reads every event already available, without blocking
    fn drain(&mut self, sink: &mut dyn FnMut(SeqEvent<'_>)) -> Result<usize> {
        let raw = self.seq.raw();
        let mut delivered = 0;
        loop {
            let mut ev: *mut ffi::snd_seq_event_t = ptr::null_mut();
            // SAFETY: our handle; the event (and a variable-length payload) stays valid until the next input call,
            // and is used before it
            let r = unsafe { ffi::snd_seq_event_input(raw, &mut ev) };
            if r == -errno::EAGAIN {
                return Ok(delivered);
            }
            if r == -errno::ENOSPC {
                self.stats.overruns += 1;
                continue;
            }
            if r < 0 {
                return Err(crate::error::error_from_code(r, "snd_seq_event_input", "", Phase::Running));
            }
            if ev.is_null() {
                continue;
            }
            // SAFETY: as above
            let ev = unsafe { &*ev };
            if self.deliver(ev, sink) {
                delivered += 1;
            }
        }
    }

    fn stamp(&self, ev: &ffi::snd_seq_event_t) -> (HostTime, Stamp) {
        match &self.queue {
            Some(q)
                if ev.flags & ffi::SND_SEQ_TIME_STAMP_MASK == ffi::SND_SEQ_TIME_STAMP_REAL
                    && c_int::from(ev.queue) == q.id =>
            {
                let queue_ns = i128::from(ev.time.tv_sec) * 1_000_000_000 + i128::from(ev.time.tv_nsec);
                let host = (queue_ns + q.clock.offset).clamp(0, i128::from(u64::MAX)) as u64;
                (HostTime(host), Stamp::Queue)
            }
            _ => (HostTime::now(), Stamp::Arrival),
        }
    }

    fn deliver(&mut self, ev: &ffi::snd_seq_event_t, sink: &mut dyn FnMut(SeqEvent<'_>)) -> bool {
        use crate::midi::ev as t;
        let bytes = event_bytes(ev);
        let addr = |b: &[u8]| Addr::new(b[0], b[1]);
        let change = match ev.type_ {
            t::CLIENT_START => Some(PortChange::ClientStart(bytes[0])),
            t::CLIENT_EXIT => Some(PortChange::ClientExit(bytes[0])),
            t::CLIENT_CHANGE => Some(PortChange::ClientChange(bytes[0])),
            t::PORT_START => Some(PortChange::PortStart(addr(&bytes[0..2]))),
            t::PORT_EXIT => Some(PortChange::PortExit(addr(&bytes[0..2]))),
            t::PORT_CHANGE => Some(PortChange::PortChange(addr(&bytes[0..2]))),
            t::PORT_SUBSCRIBED => Some(PortChange::Subscribed { sender: addr(&bytes[0..2]), dest: addr(&bytes[2..4]) }),
            t::PORT_UNSUBSCRIBED => {
                Some(PortChange::Unsubscribed { sender: addr(&bytes[0..2]), dest: addr(&bytes[2..4]) })
            }
            _ => None,
        };
        if let Some(change) = change {
            if ev.source.client == ffi::SND_SEQ_CLIENT_SYSTEM && Some(ev.dest.port) == self.announce_port {
                sink(SeqEvent::PortsChanged(change));
                return true;
            }
            self.stats.dropped += 1;
            return false;
        }
        let port = Addr::new(ev.dest.client, ev.dest.port);
        let source = Addr::new(ev.source.client, ev.source.port);
        if ev.type_ == ffi::SND_SEQ_EVENT_SYSEX {
            // SAFETY: a variable-length event: data.ext gives the payload's length and address, valid until the
            // next input call
            let chunk = unsafe { sysex_payload(ev) };
            return self.sysex_chunk(ev, port, source, chunk, sink);
        }
        let messages = seq_to_midi(fields_from_event(ev));
        if messages.is_empty() {
            self.stats.dropped += 1;
            return false;
        }
        let (time, stamp) = self.stamp(ev);
        for message in messages.iter() {
            sink(SeqEvent::Midi(MidiInputEvent { port, source, time, stamp, message }));
        }
        true
    }

    /// One piece of a SysEx message: a hardware port's driver cuts long ones into pieces, the first starting with F0
    /// and the last ending with F7
    fn sysex_chunk(
        &mut self,
        ev: &ffi::snd_seq_event_t,
        port: Addr,
        source: Addr,
        chunk: &[u8],
        sink: &mut dyn FnMut(SeqEvent<'_>),
    ) -> bool {
        if chunk.first() == Some(&0xf0) {
            if self.assembly.take().is_some() {
                // The previous one never ended
                self.stats.sysex_dropped += 1;
            }
            let (time, stamp) = self.stamp(ev);
            self.sysex.clear();
            self.assembly = Some(Assembly { port, source, time, stamp, overflow: false });
        }
        let Some(assembly) = &mut self.assembly else {
            // A continuation without a start
            self.stats.dropped += 1;
            return false;
        };
        if assembly.source != source {
            // Another sender's piece in the middle: the message in progress is lost
            self.assembly = None;
            self.stats.sysex_dropped += 1;
            return false;
        }
        if !assembly.overflow {
            if self.sysex.len() + chunk.len() > SYSEX_MAX {
                assembly.overflow = true;
            } else {
                self.sysex.extend_from_slice(chunk);
            }
        }
        if chunk.last() != Some(&0xf7) {
            return false;
        }
        let Some(done) = self.assembly.take() else { return false };
        if done.overflow {
            self.stats.sysex_dropped += 1;
            return false;
        }
        sink(SeqEvent::Sysex(SysexEvent {
            port: done.port,
            source: done.source,
            time: done.time,
            stamp: done.stamp,
            data: &self.sysex,
        }));
        true
    }

    /// The queue time for host time `at` (never before the queue started)
    fn queue_time(&self, q: &Queue, at: HostTime) -> ffi::snd_seq_real_time_t {
        let ns = (i128::from(at.0) - q.clock.offset).max(0);
        ffi::snd_seq_real_time_t {
            tv_sec: (ns / 1_000_000_000).min(i128::from(u32::MAX)) as c_uint,
            tv_nsec: (ns % 1_000_000_000) as c_uint,
        }
    }

    /// Fills the routing and timing of an event: scheduled on our queue at `at`, or direct (now) when `at` is None or
    /// there is no queue
    fn address(&mut self, ev: &mut ffi::snd_seq_event_t, from: Addr, dest: Option<Addr>, at: Option<HostTime>) {
        ev.source = ffi::snd_seq_addr_t { client: self.seq.client_id(), port: from.port };
        ev.dest = match dest {
            Some(d) => ffi::snd_seq_addr_t { client: d.client, port: d.port },
            None => {
                ffi::snd_seq_addr_t { client: ffi::SND_SEQ_ADDRESS_SUBSCRIBERS, port: ffi::SND_SEQ_ADDRESS_UNKNOWN }
            }
        };
        let length = ev.flags & ffi::SND_SEQ_EVENT_LENGTH_MASK;
        match (at, &self.queue) {
            (Some(at), Some(q)) => {
                ev.flags = ffi::SND_SEQ_TIME_STAMP_REAL | ffi::SND_SEQ_TIME_MODE_ABS | length;
                ev.queue = q.id as u8;
                ev.time = self.queue_time(q, at);
            }
            (at, _) => {
                if at.is_some() {
                    self.stats.unscheduled += 1;
                }
                ev.flags = ffi::SND_SEQ_TIME_STAMP_REAL | ffi::SND_SEQ_TIME_MODE_ABS | length;
                ev.queue = ffi::SND_SEQ_QUEUE_DIRECT;
            }
        }
    }

    fn output(&mut self, ev: &mut ffi::snd_seq_event_t) -> Result<()> {
        // SAFETY: our handle and a complete event (with its payload alive for a variable-length one)
        let r = unsafe { ffi::snd_seq_event_output_direct(self.seq.raw(), ev) };
        if r == -errno::EAGAIN || r == -errno::ENOMEM {
            return Err(keyed("alsa.seq_output_full", "snd_seq_event_output_direct", r, ""));
        }
        check(r, "snd_seq_event_output_direct", "", Phase::Running).map(drop)
    }

    fn short_event(message: &MidiMessage) -> Result<ffi::snd_seq_event_t> {
        let fields = midi_to_seq(message).ok_or_else(|| {
            Error::new("alsa.invalid_argument")
                .arg("argument", "message")
                .arg("bytes", format!("{:02x?}", message.as_slice()))
        })?;
        Ok(event_from_fields(fields))
    }

    /// Schedules `message` from our port `from` to `dest` (every subscriber of `from` when `None`) at host time `at`,
    /// on the queue: it leaves at that exact time, not when this call runs. A time already past leaves at once.
    /// Never blocks: a full kernel pool is `alsa.seq_output_full`. Without a queue the message leaves at once and is
    /// counted in [`InputStats::unscheduled`]
    pub fn send_at(&mut self, from: Addr, dest: Option<Addr>, message: &MidiMessage, at: HostTime) -> Result<()> {
        self.remeasure_if_old();
        let mut ev = Self::short_event(message)?;
        self.address(&mut ev, from, dest, Some(at));
        self.output(&mut ev)
    }

    /// Sends `message` at once (not queued)
    pub fn send_now(&mut self, from: Addr, dest: Option<Addr>, message: &MidiMessage) -> Result<()> {
        let mut ev = Self::short_event(message)?;
        self.address(&mut ev, from, dest, None);
        self.output(&mut ev)
    }

    /// Schedules a whole SysEx message (`F0 ... F7`, at most [`SYSEX_MAX`] bytes) like [`send_at`](Self::send_at).
    /// `alsa.invalid_argument` for anything else
    pub fn send_sysex_at(&mut self, from: Addr, dest: Option<Addr>, data: &[u8], at: HostTime) -> Result<()> {
        if data.len() < 2 || data.len() > SYSEX_MAX || data[0] != 0xf0 || data[data.len() - 1] != 0xf7 {
            return Err(Error::new("alsa.invalid_argument").arg("argument", "sysex").arg("len", data.len() as i64));
        }
        self.remeasure_if_old();
        let mut ev = ffi::snd_seq_event_t { type_: ffi::SND_SEQ_EVENT_SYSEX, ..Default::default() };
        ev.flags = ffi::SND_SEQ_EVENT_LENGTH_VARIABLE;
        set_sysex_payload(&mut ev, data);
        self.address(&mut ev, from, dest, Some(at));
        // The kernel copies the payload during the call: `data` only has to live until it returns
        self.output(&mut ev)
    }
}

/// The 12-byte image of an event's data union
fn event_bytes(ev: &ffi::snd_seq_event_t) -> [u8; 12] {
    let mut b = [0u8; 12];
    for (i, w) in ev.data.iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&w.to_ne_bytes());
    }
    b
}

/// The payload of a variable-length event (`data.ext`: a 4-byte length, then a pointer, packed).
///
/// # Safety
/// `ev` must be a variable-length event whose payload is still valid (until the next input call)
unsafe fn sysex_payload(ev: &ffi::snd_seq_event_t) -> &[u8] {
    let b = event_bytes(ev);
    let len = u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let mut p = [0u8; size_of::<usize>()];
    p.copy_from_slice(&b[4..4 + size_of::<usize>()]);
    let ptr = usize::from_ne_bytes(p) as *const u8;
    if ptr.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: the caller's contract; alsa-lib points ext.ptr at `len` bytes after the event in its input buffer
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

fn set_sysex_payload(ev: &mut ffi::snd_seq_event_t, data: &[u8]) {
    let mut b = [0u8; 12];
    b[0..4].copy_from_slice(&(data.len() as u32).to_ne_bytes());
    b[4..4 + size_of::<usize>()].copy_from_slice(&(data.as_ptr() as usize).to_ne_bytes());
    for (i, w) in ev.data.iter_mut().enumerate() {
        *w = u32::from_ne_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
    }
}

/// The queue's tick rate. The sequencer dispatches scheduled events on its ticks, 1 kHz by default, which made output
/// up to 1 ms late (measured on the ThinkPad). At 4 kHz an event leaves within 250 us of its time; the cost is 4000
/// high-resolution timer wake-ups a second while the client is open, a few microseconds of kernel work each
pub const QUEUE_TICK_HZ: u32 = 4000;

/// Asks for [`QUEUE_TICK_HZ`] on the queue's timer, before the queue starts. A kernel that refuses keeps its default
/// (the output is then coarser, never wrong)
fn set_queue_tick(raw: *mut ffi::snd_seq_t, q: c_int) {
    let mut timer = ptr::null_mut();
    // SAFETY: valid out pointer; the object is freed below
    if unsafe { ffi::snd_seq_queue_timer_malloc(&mut timer) } < 0 || timer.is_null() {
        return;
    }
    // SAFETY: our handle, our stopped queue and the live timer object
    unsafe {
        if ffi::snd_seq_get_queue_timer(raw, q, timer) >= 0 {
            ffi::snd_seq_queue_timer_set_resolution(timer, QUEUE_TICK_HZ);
            ffi::snd_seq_set_queue_timer(raw, q, timer);
        }
        ffi::snd_seq_queue_timer_free(timer);
    }
}

/// A queue on real time, started; None when the sequencer will not make one (then: stamps on arrival)
fn make_queue(seq: &mut Seq) -> Option<Queue> {
    let raw = seq.raw();
    // SAFETY: our handle; a queue made here is freed when the client closes
    let q = unsafe { ffi::snd_seq_alloc_named_queue(raw, c"LinuxMusic MIDI clock".as_ptr()) };
    if q < 0 {
        return None;
    }
    let mut status = ptr::null_mut();
    // SAFETY: valid out pointer
    if unsafe { ffi::snd_seq_queue_status_malloc(&mut status) } < 0 {
        return None;
    }
    let status = QueueStatusObj(NonNull::new(status)?);
    set_queue_tick(raw, q);
    // SAFETY: our handle and our queue; a null event means "make one"
    let started = unsafe {
        ffi::snd_seq_control_queue(raw, q, ffi::SND_SEQ_EVENT_START, 0, ptr::null_mut()) >= 0
            && ffi::snd_seq_drain_output(raw) >= 0
    };
    if !started {
        // SAFETY: our queue
        unsafe { ffi::snd_seq_free_queue(raw, q) };
        return None;
    }
    Some(Queue { id: q, status, clock: QueueClock { offset: 0, measured: Instant::now().checked_sub(REMEASURE * 2).unwrap_or_else(Instant::now) } })
}

/// The input-only reader of the first F1 version: a [`MidiClient`] with one input port, delivering short messages
/// and announcements ([`SeqInput`]). SysEx is counted in `dropped`. New code uses [`MidiClient`]
#[derive(Debug)]
pub struct MidiInput {
    client: MidiClient,
    port: Addr,
}

impl MidiInput {
    /// Opens the client `client_name` with one input port `port_name`, any client can connect to. With
    /// `watch_ports`, the system's announcements arrive as [`SeqInput::PortsChanged`]. Not real-time safe
    pub fn open(client_name: &str, port_name: &str, watch_ports: bool) -> Result<MidiInput> {
        let mut client = MidiClient::open(client_name, watch_ports)?;
        let port = client.create_input_port(port_name)?;
        Ok(MidiInput { client, port })
    }

    /// The input port's address (what others connect to)
    pub fn addr(&self) -> Addr {
        self.port
    }

    /// The client, for connections made by this same thread
    pub fn seq(&mut self) -> &mut Seq {
        self.client.seq()
    }

    pub fn stopper(&self) -> MidiStopper {
        self.client.stopper()
    }

    pub fn stamps_on_arrival(&self) -> bool {
        self.client.stamps_on_arrival()
    }

    pub fn stats(&self) -> InputStats {
        self.client.stats()
    }

    /// As [`MidiClient::read`], with the short messages and announcements only
    pub fn read(&mut self, timeout: Option<Duration>, sink: &mut dyn FnMut(SeqInput)) -> Result<ReadOutcome> {
        let mut sysex = 0;
        let outcome = self.client.read(timeout, &mut |event| match event {
            SeqEvent::Midi(e) => sink(SeqInput::Midi(e)),
            SeqEvent::Sysex(_) => sysex += 1,
            SeqEvent::PortsChanged(_) => sink(SeqInput::PortsChanged),
        });
        self.client.stats.dropped += sysex;
        outcome
    }
}
