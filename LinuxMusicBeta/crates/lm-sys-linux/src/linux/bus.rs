//! The D-Bus services, on a thread of their own ("lm-dbus"): rtkit and logind on the system bus, ReserveDevice1 on
//! the session bus (decisoes-autonomas.md: zbus, blocking, own thread).
//!
//! Every request is a message to the thread and comes back as a [`Reply`]: the main thread never waits on D-Bus
//! (Parte 2, rule 8). It polls the reply ([`Reply::try_take`]) from its 10 ms pump, or a worker waits for it
//! ([`Reply::wait`]). The thread connects to each bus on the first request that needs it (starting the services
//! never blocks), and tries again on a later request if the bus was missing.
//!
//! zbus 6 has no `blocking` module: the thread runs each request inside `zbus::block_on`, which is the blocking mode.
//! Between requests, zbus's own helper thread keeps the connections alive and answers the method calls on our
//! ReserveDevice1 objects (see the safety sheet).

use super::reserve::{self, ReserveDevice};
use super::rt::{ThreadId, cap_rttime};
use lm_base::{Error, Result};
use std::collections::HashMap;
use std::fmt;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use zbus::Connection;
use zbus::connection::Builder;

/// The longest any one D-Bus call may take before it fails with `linux.dbus.timeout`
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// A missing bus is looked for again after this long
const RETRY: Duration = Duration::from_secs(2);

/// The answer to a request, arriving later from the D-Bus thread
#[must_use = "the request's result (and, for a guard, the resource) is in the reply"]
pub struct Reply<T> {
    rx: Receiver<Result<T>>,
}

impl<T> fmt::Debug for Reply<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Reply")
    }
}

fn thread_gone() -> Error {
    Error::new("linux.dbus.thread_gone")
}

impl<T> Reply<T> {
    /// The result if it arrived, without waiting (for the main thread)
    pub fn try_take(&self) -> Option<Result<T>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(thread_gone())),
        }
    }

    /// Waits up to `timeout` (a worker or a starting audio thread; never the main thread). `linux.dbus.timeout`
    /// when it did not arrive; a guard that arrives later is dropped, which releases it
    pub fn wait(self, timeout: Duration) -> Result<T> {
        match self.rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => Err(Error::new("linux.dbus.timeout")),
            Err(RecvTimeoutError::Disconnected) => Err(thread_gone()),
        }
    }
}

/// What rtkit allows (its properties)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtkitInfo {
    pub max_realtime_priority: i32,
    pub min_nice_level: i32,
    pub rttime_usec_max: i64,
}

/// The systemd-logind inhibitor lock ("sleep:idle", mode "block"): while it lives, the machine neither sleeps nor
/// goes idle (H2). Dropping it closes the descriptor logind gave, which releases the lock; no D-Bus call is needed
#[must_use = "the machine may sleep as soon as it is dropped"]
#[derive(Debug)]
pub struct SleepInhibitor {
    _fd: OwnedFd,
}

/// A card reserved with ReserveDevice1 (`org.freedesktop.ReserveDevice1.Audio<N>` on the session bus): PipeWire and
/// PulseAudio have released it and will not take it while this lives. Released (name and object) on drop, by a
/// message to the D-Bus thread (dropping never blocks)
#[must_use = "the reservation ends when it is dropped"]
pub struct Reservation {
    card: u32,
    release_requests: Arc<AtomicU32>,
    tx: Sender<Job>,
}

impl fmt::Debug for Reservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reservation")
            .field("card", &self.card)
            .field("release_requests", &self.release_requests())
            .finish()
    }
}

impl Reservation {
    pub fn card(&self) -> u32 {
        self.card
    }

    /// How many times another program asked us to release the card (we always refuse while recording or playing;
    /// the session may tell the user who wants it)
    pub fn release_requests(&self) -> u32 {
        self.release_requests.load(Ordering::Relaxed)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let _ = self.tx.send(Job::Release { card: self.card });
    }
}

/// What a program asking for our card sees
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReserveRequest {
    pub card: u32,
    /// Our name, as other programs show it ("LinuxMusic")
    pub application_name: String,
    /// The device's name for people ("Scarlett 2i2 USB")
    pub device_name: String,
    /// Our priority: owners with a lower one are asked to give the card up. JACK uses 20 by default; PulseAudio and
    /// PipeWire register 0 and give way to any higher one
    pub priority: i32,
}

enum Job {
    RtkitInfo {
        reply: SyncSender<Result<RtkitInfo>>,
    },
    MakeRealtime {
        tid: ThreadId,
        priority: u32,
        reply: SyncSender<Result<u32>>,
    },
    Inhibit {
        who: String,
        why: String,
        reply: SyncSender<Result<SleepInhibitor>>,
    },
    /// `tx`: the sender the reservation will keep, to release itself
    Reserve {
        request: ReserveRequest,
        tx: Sender<Job>,
        reply: SyncSender<Result<Reservation>>,
    },
    Release {
        card: u32,
    },
    Shutdown,
}

/// The handle of the D-Bus thread. `Send + Sync`; requests from any thread. Dropping it lets the thread finish once
/// every [`Reservation`] is gone (they keep talking to it); [`shutdown`](SystemBus::shutdown) ends it now
pub struct SystemBus {
    tx: Sender<Job>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for SystemBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemBus").finish()
    }
}

fn request<T>(tx: &Sender<Job>, make: impl FnOnce(SyncSender<Result<T>>) -> Job) -> Reply<T> {
    let (reply, rx) = mpsc::sync_channel(1);
    // If the thread is gone, the reply's sender is dropped with the job, and the Reply says so
    let _ = tx.send(make(reply));
    Reply { rx }
}

impl SystemBus {
    /// Starts the D-Bus thread. Never blocks on D-Bus (connections are made on demand, on the thread)
    pub fn start() -> Result<SystemBus> {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("lm-dbus".to_owned())
            .spawn(move || {
                lm_base::set_role(lm_base::Role::Worker);
                Worker::new().run(rx);
            })
            .map_err(|e| Error::new("linux.thread_failed").arg("detail", e.to_string()))?;
        Ok(SystemBus { tx, thread: Some(thread) })
    }

    /// rtkit's limits
    pub fn rtkit_info(&self) -> Reply<RtkitInfo> {
        request(&self.tx, |reply| Job::RtkitInfo { reply })
    }

    /// Asks rtkit to make thread `tid` of this process `SCHED_FIFO` at `priority` (clamped to rtkit's maximum; the
    /// reply is the priority granted). Lowers `RLIMIT_RTTIME` to rtkit's maximum first, as rtkit requires.
    /// Usually called through [`make_current_thread_realtime`](crate::make_current_thread_realtime)
    pub fn make_thread_realtime(&self, tid: ThreadId, priority: u32) -> Reply<u32> {
        request(&self.tx, |reply| Job::MakeRealtime { tid, priority, reply })
    }

    /// Takes logind's "sleep:idle" inhibitor lock (H2). `who` is the program's name, `why` the reason people see
    /// ("Recording")
    pub fn inhibit_sleep(&self, who: &str, why: &str) -> Reply<SleepInhibitor> {
        request(&self.tx, |reply| Job::Inhibit { who: who.to_owned(), why: why.to_owned(), reply })
    }

    /// Reserves ALSA card `card` with ReserveDevice1, asking its current owner (PipeWire, PulseAudio, JACK) to let
    /// it go, as the JACK server does (U8). Errors: `linux.reserve.refused` (the owner said no; `owner` is its
    /// `ApplicationName` when it has one), `linux.dbus.unavailable` (no session bus: nobody can hold the card
    /// through ReserveDevice1, so just try to open it), `linux.reserve.failed`
    pub fn reserve_audio_device(&self, request: ReserveRequest) -> Reply<Reservation> {
        let tx = self.tx.clone();
        self::request(&self.tx, |reply| Job::Reserve { request, tx, reply })
    }

    /// Ends the thread: closes both connections (which also drops every reservation still held) and waits up to
    /// `timeout`. False if it did not finish in time (it is then left to finish alone; it owns everything it uses)
    pub fn shutdown(mut self, timeout: Duration) -> bool {
        let _ = self.tx.send(Job::Shutdown);
        let Some(thread) = self.thread.take() else { return true };
        let deadline = Instant::now() + timeout;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        thread.join().is_ok()
    }
}

/// A bus connection, made on demand
#[derive(Default)]
struct Bus {
    conn: Option<Connection>,
    failed_at: Option<(Instant, Error)>,
}

impl Bus {
    fn get(&mut self, system: bool) -> Result<Connection> {
        if let Some(c) = &self.conn {
            if !c.is_closed() {
                return Ok(c.clone());
            }
            self.conn = None;
        }
        if let Some((at, e)) = &self.failed_at
            && at.elapsed() < RETRY
        {
            return Err(e.clone());
        }
        let builder = if system { Builder::system() } else { Builder::session() };
        match zbus::block_on(builder.method_timeout(CALL_TIMEOUT).build()) {
            Ok(c) => {
                self.failed_at = None;
                self.conn = Some(c.clone());
                Ok(c)
            }
            Err(e) => {
                let err = Error::new("linux.dbus.unavailable")
                    .arg("bus", if system { "system" } else { "session" })
                    .arg("detail", e.to_string());
                self.failed_at = Some((Instant::now(), err.clone()));
                Err(err)
            }
        }
    }
}

struct Held {
    name: String,
    path: String,
}

struct Worker {
    system: Bus,
    session: Bus,
    reservations: HashMap<u32, Held>,
}

pub(crate) fn dbus_error(key: &'static str, e: &zbus::Error) -> Error {
    match e {
        zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            Error::new("linux.dbus.timeout").arg("detail", e.to_string())
        }
        _ => Error::new(key).arg("detail", e.to_string()),
    }
}

impl Worker {
    fn new() -> Self {
        Self { system: Bus::default(), session: Bus::default(), reservations: HashMap::new() }
    }

    fn run(mut self, rx: Receiver<Job>) {
        // The senders are the handle and the live reservations: the loop ends when all are gone, or on Shutdown
        while let Ok(job) = rx.recv() {
            match job {
                Job::RtkitInfo { reply } => {
                    let _ = reply.send(self.rtkit_info());
                }
                Job::MakeRealtime { tid, priority, reply } => {
                    let _ = reply.send(self.make_realtime(tid, priority));
                }
                Job::Inhibit { who, why, reply } => {
                    let _ = reply.send(self.inhibit(&who, &why));
                }
                Job::Reserve { request, tx, reply } => {
                    let result = self.reserve(&request, tx);
                    if let Err(Ok(reservation)) = reply.send(result).map_err(|e| e.0) {
                        // Nobody waits any more: give the card back
                        drop(reservation);
                    }
                }
                Job::Release { card } => self.release(card),
                Job::Shutdown => break,
            }
        }
        for card in self.reservations.keys().copied().collect::<Vec<_>>() {
            self.release(card);
        }
    }

    fn rtkit_info(&mut self) -> Result<RtkitInfo> {
        let conn = self.system.get(true)?;
        zbus::block_on(async {
            let get = |name: &'static str| {
                let conn = conn.clone();
                async move {
                    let msg = conn
                        .call_method(
                            Some("org.freedesktop.RealtimeKit1"),
                            "/org/freedesktop/RealtimeKit1",
                            Some("org.freedesktop.DBus.Properties"),
                            "Get",
                            &("org.freedesktop.RealtimeKit1", name),
                        )
                        .await
                        .map_err(|e| dbus_error("linux.rtkit.unavailable", &e))?;
                    msg.body()
                        .deserialize::<zbus::OwnedVariant>()
                        .map_err(|e| dbus_error("linux.rtkit.unavailable", &e))
                }
            };
            let int = |v: zbus::OwnedVariant| -> Result<i64> {
                if let Ok(x) = i32::try_from(v.try_clone().map_err(|e| dbus_error("linux.rtkit.unavailable", &e))?) {
                    return Ok(i64::from(x));
                }
                i64::try_from(v).map_err(|e| dbus_error("linux.rtkit.unavailable", &e))
            };
            Ok(RtkitInfo {
                max_realtime_priority: int(get("MaxRealtimePriority").await?)? as i32,
                min_nice_level: int(get("MinNiceLevel").await?)? as i32,
                rttime_usec_max: int(get("RTTimeUSecMax").await?)?,
            })
        })
    }

    fn make_realtime(&mut self, tid: ThreadId, priority: u32) -> Result<u32> {
        let info = self.rtkit_info()?;
        let granted = priority.min(info.max_realtime_priority.max(1) as u32);
        cap_rttime(info.rttime_usec_max.max(1) as u64)?;
        let conn = self.system.get(true)?;
        zbus::block_on(conn.call_method(
            Some("org.freedesktop.RealtimeKit1"),
            "/org/freedesktop/RealtimeKit1",
            Some("org.freedesktop.RealtimeKit1"),
            "MakeThreadRealtime",
            &(tid.0 as u64, granted),
        ))
        .map_err(|e| match &e {
            zbus::Error::MethodError(name, ..) => {
                Error::new("linux.rtkit.refused").arg("dbus_error", name.as_str()).arg("detail", e.to_string())
            }
            _ => dbus_error("linux.rtkit.unavailable", &e),
        })?;
        Ok(granted)
    }

    fn inhibit(&mut self, who: &str, why: &str) -> Result<SleepInhibitor> {
        let conn = self.system.get(true)?;
        let msg = zbus::block_on(conn.call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            &("sleep:idle", who, why, "block"),
        ))
        .map_err(|e| dbus_error("linux.logind.unavailable", &e))?;
        let fd: zbus::OwnedFd = msg.body().deserialize().map_err(|e| dbus_error("linux.logind.unavailable", &e))?;
        Ok(SleepInhibitor { _fd: OwnedFd::from(fd) })
    }

    fn reserve(&mut self, request: &ReserveRequest, tx: Sender<Job>) -> Result<Reservation> {
        let card = request.card;
        if self.reservations.contains_key(&card) {
            return Err(Error::new("linux.reserve.already_held").arg("card", i64::from(card)));
        }
        let conn = self.session.get(false)?;
        let counter = Arc::new(AtomicU32::new(0));
        let object = ReserveDevice {
            priority: request.priority,
            application_name: request.application_name.clone(),
            device_name: request.device_name.clone(),
            release_requests: counter.clone(),
        };
        let (name, path) = (reserve::bus_name(card), reserve::object_path(card));
        zbus::block_on(reserve::acquire(&conn, &name, &path, object, request.priority))?;
        self.reservations.insert(card, Held { name, path });
        Ok(Reservation { card, release_requests: counter, tx })
    }

    fn release(&mut self, card: u32) {
        let Some(held) = self.reservations.remove(&card) else { return };
        if let Ok(conn) = self.session.get(false) {
            let _ = zbus::block_on(reserve::release(&conn, &held.name, &held.path));
        }
    }
}
