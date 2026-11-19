//! Real-time scheduling of the audio threads (interfaces.md, H1): `SCHED_FIFO` directly when the limits allow
//! (`RLIMIT_RTPRIO`, from the `audio` group and `limits.conf`), otherwise through rtkit on the system bus; and what is
//! missing when neither works, for the notice the program shows once.

use super::bus::SystemBus;
use lm_base::{Error, Result};
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

/// One resource limit; `None` is unlimited
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit {
    pub soft: Option<u64>,
    pub hard: Option<u64>,
}

impl Limit {
    fn from_raw(r: libc::rlimit) -> Self {
        let f = |v: libc::rlim_t| if v == libc::RLIM_INFINITY { None } else { Some(v) };
        Self { soft: f(r.rlim_cur), hard: f(r.rlim_max) }
    }

    /// The soft limit as a label argument: -1 for unlimited
    pub fn soft_arg(&self) -> i64 {
        self.soft.map_or(-1, |v| i64::try_from(v).unwrap_or(i64::MAX))
    }
}

/// The limits that decide real-time scheduling and memory locking
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    /// The highest `SCHED_FIFO` priority allowed without privileges (0: none)
    pub rtprio: Limit,
    /// Bytes that may be locked in memory
    pub memlock: Limit,
    /// Microseconds of CPU a real-time thread may use without blocking (rtkit requires it to be set)
    pub rttime: Limit,
}

pub(crate) fn get_limit(resource: libc::__rlimit_resource_t) -> Result<Limit> {
    let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: valid out pointer
    if unsafe { libc::getrlimit(resource, &mut r) } != 0 {
        return Err(os_error("linux.rlimit_failed", "getrlimit"));
    }
    Ok(Limit::from_raw(r))
}

/// The process's limits
pub fn resource_limits() -> Result<ResourceLimits> {
    Ok(ResourceLimits {
        rtprio: get_limit(libc::RLIMIT_RTPRIO)?,
        memlock: get_limit(libc::RLIMIT_MEMLOCK)?,
        rttime: get_limit(libc::RLIMIT_RTTIME)?,
    })
}

/// Lowers `RLIMIT_RTTIME` to `usec` (soft and hard) if it is higher or unlimited: rtkit refuses a process without
/// this limit (it is the watchdog of a runaway real-time thread). Process-wide and irreversible for the hard limit
pub(crate) fn cap_rttime(usec: u64) -> Result<()> {
    let current = get_limit(libc::RLIMIT_RTTIME)?;
    if current.hard.is_some_and(|h| h <= usec) && current.soft.is_some_and(|s| s <= usec) {
        return Ok(());
    }
    let r = libc::rlimit { rlim_cur: usec as libc::rlim_t, rlim_max: usec as libc::rlim_t };
    // SAFETY: valid in pointer; lowering a limit needs no privilege
    if unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &r) } != 0 {
        return Err(os_error("linux.rlimit_failed", "setrlimit"));
    }
    Ok(())
}

pub(crate) fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

pub(crate) fn os_error(key: &'static str, call: &'static str) -> Error {
    Error::new(key).arg("call", call).arg("errno", i64::from(errno()))
}

/// Whether the user is in the group `name` (as primary group or supplementary)
pub fn in_group(name: &str) -> Result<bool> {
    let Some(gid) = group_id(name)? else { return Ok(false) };
    // SAFETY: plain call
    if unsafe { libc::getgid() } == gid || unsafe { libc::getegid() } == gid {
        return Ok(true);
    }
    // SAFETY: a query of the count, then a buffer of that size
    let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if n < 0 {
        return Err(os_error("linux.groups_failed", "getgroups"));
    }
    let mut groups = vec![0; n as usize];
    // SAFETY: room for n groups
    let n = unsafe { libc::getgroups(n, groups.as_mut_ptr()) };
    if n < 0 {
        return Err(os_error("linux.groups_failed", "getgroups"));
    }
    Ok(groups[..n as usize].contains(&gid))
}

/// Whether the user is in the `audio` group (U8: access to `/dev/snd/*` and, usually, the real-time limits)
pub fn in_audio_group() -> Result<bool> {
    in_group("audio")
}

fn group_id(name: &str) -> Result<Option<libc::gid_t>> {
    let c_name = std::ffi::CString::new(name).map_err(|_| Error::new("linux.invalid_argument").arg("group", name))?;
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    loop {
        // SAFETY: an all-zero group struct is a valid out value; the buffer and the out pointers are valid
        let mut group: libc::group = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        // SAFETY: the reentrant lookup with our buffer
        let r = unsafe { libc::getgrnam_r(c_name.as_ptr(), &mut group, buf.as_mut_ptr(), buf.len(), &mut result) };
        if r == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 4, 0);
            continue;
        }
        if r != 0 {
            return Err(Error::new("linux.groups_failed").arg("call", "getgrnam_r").arg("errno", i64::from(r)));
        }
        return Ok(if result.is_null() { None } else { Some(group.gr_gid) });
    }
}

/// The kernel id of the calling thread (what rtkit takes)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThreadId(pub i32);

pub fn current_thread_id() -> ThreadId {
    // SAFETY: plain system call
    ThreadId(unsafe { libc::gettid() })
}

/// The scheduling policy of a thread
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedPolicy {
    Other,
    Batch,
    Idle,
    Fifo(u32),
    RoundRobin(u32),
    Unknown(i32),
}

/// The calling thread's policy and priority
pub fn current_policy() -> SchedPolicy {
    // SAFETY: pid 0 is the calling thread
    let policy = unsafe { libc::sched_getscheduler(0) } & !libc::SCHED_RESET_ON_FORK;
    let mut param = libc::sched_param { sched_priority: 0 };
    // SAFETY: valid out pointer
    unsafe { libc::sched_getparam(0, &mut param) };
    let prio = param.sched_priority.max(0) as u32;
    match policy {
        libc::SCHED_OTHER => SchedPolicy::Other,
        libc::SCHED_BATCH => SchedPolicy::Batch,
        libc::SCHED_IDLE => SchedPolicy::Idle,
        libc::SCHED_FIFO => SchedPolicy::Fifo(prio),
        libc::SCHED_RR => SchedPolicy::RoundRobin(prio),
        other => SchedPolicy::Unknown(other),
    }
}

/// How the thread got its real-time priority
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtMethod {
    /// `sched_setscheduler` by the thread itself (`RLIMIT_RTPRIO` allowed it)
    Direct,
    /// rtkit (`org.freedesktop.RealtimeKit1`) did it
    Rtkit,
}

/// A thread running `SCHED_FIFO`: on drop, the thread goes back to the policy it had. Not `Send`: it must be dropped
/// on the thread it belongs to (H1: `RtMembership` leaves on the same thread that joined)
#[must_use = "dropping it at once takes the thread back out of real time"]
pub struct RealtimeGuard {
    method: RtMethod,
    priority: u32,
    previous_policy: i32,
    previous_param: libc::sched_param,
    _not_send: PhantomData<*const ()>,
}

impl fmt::Debug for RealtimeGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealtimeGuard").field("method", &self.method).field("priority", &self.priority).finish()
    }
}

impl RealtimeGuard {
    pub fn method(&self) -> RtMethod {
        self.method
    }

    /// The priority granted (rtkit may give less than asked: its `MaxRealtimePriority`)
    pub fn priority(&self) -> u32 {
        self.priority
    }
}

impl Drop for RealtimeGuard {
    fn drop(&mut self) {
        // SAFETY: the calling thread (pid 0) back to its previous policy; lowering needs no privilege
        unsafe { libc::sched_setscheduler(0, self.previous_policy, &self.previous_param) };
    }
}

fn set_fifo(priority: u32) -> std::result::Result<(), i32> {
    let param = libc::sched_param { sched_priority: priority as i32 };
    // SAFETY: the calling thread (pid 0), a valid param. RESET_ON_FORK: a child process never inherits real time
    if unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO | libc::SCHED_RESET_ON_FORK, &param) } == 0 {
        Ok(())
    } else {
        Err(errno())
    }
}

/// Makes the calling thread `SCHED_FIFO` at `priority` (1..=99): directly if the limits allow, otherwise through
/// rtkit when `bus` is given (waiting up to `timeout` for it). Blocks (the D-Bus round trip): call it once when the
/// thread starts, before its real-time loop, never inside it.
///
/// When neither works: `linux.realtime.denied`, with what to configure: `priority`, `rtprio_limit` (-1 unlimited),
/// `audio_group` (1 if the user is in it), and `rtkit` (the key of rtkit's failure, or "not asked")
pub fn make_current_thread_realtime(
    priority: u32,
    bus: Option<&SystemBus>,
    timeout: Duration,
) -> Result<RealtimeGuard> {
    let priority = priority.clamp(1, 99);
    // SAFETY: pid 0 is the calling thread; valid out pointer
    let previous_policy = unsafe { libc::sched_getscheduler(0) };
    let mut previous_param = libc::sched_param { sched_priority: 0 };
    // SAFETY: valid out pointer
    unsafe { libc::sched_getparam(0, &mut previous_param) };
    let guard =
        |method, priority| RealtimeGuard { method, priority, previous_policy, previous_param, _not_send: PhantomData };

    let direct = set_fifo(priority);
    if direct.is_ok() {
        return Ok(guard(RtMethod::Direct, priority));
    }

    let rtkit_failure = match bus {
        None => "not asked".to_owned(),
        Some(bus) => match bus.make_thread_realtime(current_thread_id(), priority).wait(timeout) {
            Ok(granted) => {
                if matches!(current_policy(), SchedPolicy::Fifo(_) | SchedPolicy::RoundRobin(_)) {
                    return Ok(guard(RtMethod::Rtkit, granted));
                }
                "linux.rtkit.not_applied".to_owned()
            }
            Err(e) => e.to_string(),
        },
    };

    let limits = resource_limits()?;
    Err(Error::new("linux.realtime.denied")
        .arg("priority", i64::from(priority))
        .arg("errno", i64::from(direct.err().unwrap_or(0)))
        .arg("rtprio_limit", limits.rtprio.soft_arg())
        .arg("audio_group", i64::from(in_audio_group().unwrap_or(false)))
        .arg("rtkit", rtkit_failure))
}
