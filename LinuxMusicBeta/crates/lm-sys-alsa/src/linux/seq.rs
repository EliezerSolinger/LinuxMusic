//! The ALSA sequencer: MIDI ports, their connections, and the input reader of the MIDI thread.
//!
//! Two kinds of client:
//! - [`Seq`]: a plain client for the control work (list ports, connect and disconnect, send messages). Runs on a
//!   worker or on a MIDI output thread.
//! - [`MidiInput`]: the reading client, owned by the dedicated MIDI thread (I-10.2). Its input port stamps every event
//!   it receives with the time of its own queue, running on real time; the reader turns that time into
//!   `CLOCK_MONOTONIC` host time with an offset measured at the start and again every second (C6). If the queue cannot
//!   be made, events are stamped when the reader takes them, and [`MidiInput::stamps_on_arrival`] says so (more
//!   jitter). The reader blocks in `poll` and is woken to stop by its [`MidiStopper`], never killed.
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

/// What the reader hands over
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqInput {
    Midi(MidiInputEvent),
    /// A client or port appeared, went away or changed (only with `watch_ports`): list the ports again (U11)
    PortsChanged,
}

/// Why [`MidiInput::read`] returned
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// This many items went to the sink
    Delivered(usize),
    /// The stopper was used: the thread should leave
    Stopped,
    TimedOut,
}

/// Counters of what the reader could not deliver (losses are counted and reported, never silent: Parte 2, rule 4)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputStats {
    /// Times the kernel's input buffer overflowed (`ENOSPC`): events were lost
    pub overruns: u64,
    /// Events that are not short MIDI messages (sysex in F1, T14; queue events)
    pub dropped: u64,
}

struct StopSignal {
    stopped: AtomicBool,
    waker: Waker,
}

/// Stops a [`MidiInput`] blocked in [`read`](MidiInput::read) on another thread: the read returns
/// [`ReadOutcome::Stopped`], now and every time after. `Send + Sync + Clone`; real-time safe
#[derive(Clone)]
pub struct MidiStopper(Arc<StopSignal>);

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

/// The MIDI input of the program: a sequencer client with one input port, owned by the MIDI thread. `Send`, not
/// `Sync`. Not real-time: it runs on its own thread and hands events to the engine through a ring (I-7.1)
pub struct MidiInput {
    seq: Seq,
    port: Addr,
    /// Our queue, or None when stamping on arrival
    queue: Option<(c_int, QueueStatusObj, QueueClock)>,
    fds: Vec<libc::pollfd>,
    signal: Arc<StopSignal>,
    stats: InputStats,
}

// SAFETY: the queue status object is used only through &mut self, by the one thread that owns the MidiInput; the
// Seq inside is Send for the same reason (one thread at a time). The StopSignal is Sync on its own
unsafe impl Send for MidiInput {}

impl fmt::Debug for MidiInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiInput")
            .field("port", &self.port)
            .field("queue", &self.queue.as_ref().map(|q| q.0))
            .field("stats", &self.stats)
            .finish()
    }
}

impl MidiInput {
    /// Opens the client `client_name` with one input port `port_name`, any client can connect to. With
    /// `watch_ports`, the system's announcements (ports and clients coming and going) arrive as
    /// [`SeqInput::PortsChanged`]. Not real-time safe
    pub fn open(client_name: &str, port_name: &str, watch_ports: bool) -> Result<MidiInput> {
        let mut seq = Seq::open(client_name)?;
        let queue = Self::make_queue(&mut seq);
        let port = seq.create_port(
            port_name,
            ffi::SND_SEQ_PORT_CAP_WRITE | ffi::SND_SEQ_PORT_CAP_SUBS_WRITE,
            queue.as_ref().map(|q| q.0),
        )?;
        if watch_ports {
            seq.connect(Addr::new(ffi::SND_SEQ_CLIENT_SYSTEM, ffi::SND_SEQ_PORT_SYSTEM_ANNOUNCE), port)?;
        }
        let signal = Arc::new(StopSignal { stopped: AtomicBool::new(false), waker: Waker::new()? });
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
        let mut input = MidiInput { seq, port, queue, fds, signal, stats: InputStats::default() };
        input.remeasure();
        Ok(input)
    }

    /// A queue on real time, started; None when the sequencer will not make one (then: stamps on arrival)
    fn make_queue(seq: &mut Seq) -> Option<(c_int, QueueStatusObj, QueueClock)> {
        let raw = seq.raw();
        // SAFETY: our handle; a queue made here is freed when the client closes
        let q = unsafe { ffi::snd_seq_alloc_named_queue(raw, c"LinuxMusic MIDI input clock".as_ptr()) };
        if q < 0 {
            return None;
        }
        let mut status = ptr::null_mut();
        // SAFETY: valid out pointer
        if unsafe { ffi::snd_seq_queue_status_malloc(&mut status) } < 0 {
            return None;
        }
        let status = QueueStatusObj(NonNull::new(status)?);
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
        Some((q, status, QueueClock { offset: 0, measured: Instant::now() - REMEASURE * 2 }))
    }

    /// Measures the queue-to-host offset: the narrowest of three brackets around a queue status query
    fn remeasure(&mut self) {
        let raw = self.seq.raw();
        let Some((q, status, clock)) = &mut self.queue else { return };
        let mut best: Option<(u64, i128)> = None;
        for _ in 0..3 {
            let t0 = HostTime::now();
            // SAFETY: our handle, queue and status object
            let r = unsafe { ffi::snd_seq_get_queue_status(raw, *q, status.0.as_ptr()) };
            let t1 = HostTime::now();
            if r < 0 {
                continue;
            }
            // SAFETY: the status was just filled; the pointer is into it
            let rt = unsafe { *ffi::snd_seq_queue_status_get_real_time(status.0.as_ptr()) };
            let queue_ns = i128::from(rt.tv_sec) * 1_000_000_000 + i128::from(rt.tv_nsec);
            let width = t1.since(t0);
            let mid = i128::from(t0.0) + i128::from(width / 2);
            if best.is_none_or(|(w, _)| width < w) {
                best = Some((width, mid - queue_ns));
            }
        }
        if let Some((_, offset)) = best {
            *clock = QueueClock { offset, measured: Instant::now() };
        }
    }

    /// The input port's address (what others connect to)
    pub fn addr(&self) -> Addr {
        self.port
    }

    /// The client, for connections made by this same thread (`connect` from a source to [`addr`](MidiInput::addr))
    pub fn seq(&mut self) -> &mut Seq {
        &mut self.seq
    }

    /// The stopper, for the thread that will stop this reader
    pub fn stopper(&self) -> MidiStopper {
        MidiStopper(self.signal.clone())
    }

    /// True when events are stamped on arrival (no queue: more jitter, C6)
    pub fn stamps_on_arrival(&self) -> bool {
        self.queue.is_none()
    }

    pub fn stats(&self) -> InputStats {
        self.stats
    }

    /// Hands every pending event to `sink`; if there is none, blocks until there is one, the stopper is used, or
    /// `timeout` passes (`None`: forever). Returns as soon as at least one item was delivered. No allocation per
    /// event. For the MIDI thread only (it blocks)
    pub fn read(&mut self, timeout: Option<Duration>, sink: &mut dyn FnMut(SeqInput)) -> Result<ReadOutcome> {
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            if self.signal.stopped.load(Ordering::Acquire) {
                return Ok(ReadOutcome::Stopped);
            }
            if self.queue.as_ref().is_some_and(|q| q.2.measured.elapsed() >= REMEASURE) {
                self.remeasure();
            }
            let delivered = self.drain(sink)?;
            if delivered > 0 {
                return Ok(ReadOutcome::Delivered(delivered));
            }
            let ms = match deadline {
                None => -1,
                Some(d) => {
                    let left = d.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Ok(ReadOutcome::TimedOut);
                    }
                    // Wake at least every REMEASURE, to keep the clock fresh
                    i32::try_from(left.min(REMEASURE).as_millis().max(1)).unwrap_or(i32::MAX)
                }
            };
            let ms = if ms < 0 { REMEASURE.as_millis() as i32 } else { ms };
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
    fn drain(&mut self, sink: &mut dyn FnMut(SeqInput)) -> Result<usize> {
        let raw = self.seq.raw();
        let mut delivered = 0;
        loop {
            let mut ev: *mut ffi::snd_seq_event_t = ptr::null_mut();
            // SAFETY: our handle; the event stays valid until the next input call, and is copied before it
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
            // SAFETY: a valid event from snd_seq_event_input (see above)
            let ev = unsafe { *ev };
            if self.deliver(&ev, sink) {
                delivered += 1;
            }
        }
    }

    fn deliver(&mut self, ev: &ffi::snd_seq_event_t, sink: &mut dyn FnMut(SeqInput)) -> bool {
        use crate::midi::ev as t;
        if matches!(
            ev.type_,
            t::CLIENT_START
                | t::CLIENT_EXIT
                | t::CLIENT_CHANGE
                | t::PORT_START
                | t::PORT_EXIT
                | t::PORT_CHANGE
                | t::PORT_SUBSCRIBED
                | t::PORT_UNSUBSCRIBED
        ) {
            if ev.source.client == ffi::SND_SEQ_CLIENT_SYSTEM {
                sink(SeqInput::PortsChanged);
                return true;
            }
            self.stats.dropped += 1;
            return false;
        }
        let messages = seq_to_midi(fields_from_event(ev));
        if messages.is_empty() {
            self.stats.dropped += 1;
            return false;
        }
        let (time, stamp) = match &self.queue {
            Some((q, _, clock))
                if ev.flags & ffi::SND_SEQ_TIME_STAMP_MASK == ffi::SND_SEQ_TIME_STAMP_REAL
                    && c_int::from(ev.queue) == *q =>
            {
                let queue_ns = i128::from(ev.time.tv_sec) * 1_000_000_000 + i128::from(ev.time.tv_nsec);
                let host = (queue_ns + clock.offset).clamp(0, i128::from(u64::MAX)) as u64;
                (HostTime(host), Stamp::Queue)
            }
            _ => (HostTime::now(), Stamp::Arrival),
        };
        let port = Addr::new(ev.dest.client, ev.dest.port);
        let source = Addr::new(ev.source.client, ev.source.port);
        for message in messages.iter() {
            sink(SeqInput::Midi(MidiInputEvent { port, source, time, stamp, message }));
        }
        true
    }
}
