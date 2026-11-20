//! Real-time priority and memory of the audio threads (interfaces.md, H1; revisão V6).
//!
//! Every thread that renders (the device's thread and the renderer's workers) joins the real-time group when it
//! starts and leaves it, on the same thread, when it ends: [`RealtimeThreads::join`] gives an [`RtMembership`] that is
//! not `Send` and undoes everything on drop. [`realtime_hook`] makes the `lm-render` thread hook that does it for every
//! render thread, so the session gives the same [`RenderThreads`] to `engine_with` and to the [`crate::Device`].
//!
//! - **Linux:** `SCHED_FIFO`, all render threads at the same priority, directly when `RLIMIT_RTPRIO` allows it,
//!   otherwise through rtkit (`lm-sys-linux`); the thread's stack pre-touched, and locked when `RLIMIT_MEMLOCK`
//!   allows. Without permission the program plays anyway and the session shows one notice with what to configure
//!   (the `audio` group and the limits): [`RtReport::take_failure`].
//! - **Memory:** [`lock_memory`] locks the whole process (`mlockall`) only when `RLIMIT_MEMLOCK` is unlimited (V6);
//!   otherwise the device buffers are locked one by one ([`RtBuffer`]).
//! - **Mac** (later): the workgroup of the device and the time-constraint policy. Until then joining gives a
//!   membership without priority, and the report says so.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lm_base::{Error, Result};
use lm_render::{RenderThreads, ThreadKind};

/// How a thread got (or did not get) real-time priority
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtMethod {
    /// `sched_setscheduler` by the thread itself
    Direct,
    /// rtkit did it
    Rtkit,
    /// No real-time scheduling on this platform yet (Mac until the workgroups)
    Unsupported,
    /// Asked not to (tests, exports)
    None,
}

/// Membership of the real-time group: the thread's priority and its locked stack, undone on drop. Not `Send`: it is
/// dropped on the thread that joined (H1)
#[must_use = "dropping it takes the thread out of real time at once"]
pub struct RtMembership {
    method: RtMethod,
    priority: u32,
    stack_locked: usize,
    #[cfg(target_os = "linux")]
    _guard: Option<lm_sys_linux::RealtimeGuard>,
    #[cfg(target_os = "linux")]
    _stack: Option<lm_sys_linux::StackLock>,
    _not_send: PhantomData<*const ()>,
}

impl fmt::Debug for RtMembership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtMembership")
            .field("method", &self.method)
            .field("priority", &self.priority)
            .field("stack_locked", &self.stack_locked)
            .finish()
    }
}

impl RtMembership {
    /// A membership that changes nothing
    pub fn none(method: RtMethod) -> Self {
        Self {
            method,
            priority: 0,
            stack_locked: 0,
            #[cfg(target_os = "linux")]
            _guard: None,
            #[cfg(target_os = "linux")]
            _stack: None,
            _not_send: PhantomData,
        }
    }

    pub fn method(&self) -> RtMethod {
        self.method
    }

    pub fn priority(&self) -> u32 {
        self.priority
    }

    /// Bytes of the thread's stack locked in RAM (0 when the limit did not allow it)
    pub fn stack_locked(&self) -> usize {
        self.stack_locked
    }
}

/// What every audio-rendering thread does when it starts (H1)
pub trait RealtimeThreads: Send + Sync {
    /// Joins the calling thread to the real-time group for blocks of `period`. Blocks (the rtkit round trip): call it
    /// before the thread's loop. Errors say what to configure (`linux.realtime.denied` with the limit and the group)
    fn join(&self, period: Duration) -> Result<RtMembership>;
}

/// No real-time scheduling (tests, and the exports, which are not real time, H1)
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRealtime;

impl RealtimeThreads for NoRealtime {
    fn join(&self, _period: Duration) -> Result<RtMembership> {
        Ok(RtMembership::none(RtMethod::None))
    }
}

/// The system's real-time scheduling
pub struct SystemRealtime {
    /// `SCHED_FIFO` priority of every render thread (rtkit may grant less)
    pub priority: u32,
    /// Stack pre-touched (and locked when allowed) for each render thread
    pub stack_bytes: usize,
    /// How long to wait for rtkit
    pub timeout: Duration,
    #[cfg(target_os = "linux")]
    bus: Option<Arc<lm_sys_linux::SystemBus>>,
}

impl fmt::Debug for SystemRealtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemRealtime")
            .field("priority", &self.priority)
            .field("stack_bytes", &self.stack_bytes)
            .finish()
    }
}

/// The default priority of the render threads: above the usual desktop services, below the kernel's IRQ threads
/// (50) and JACK's default watchdog
pub const DEFAULT_PRIORITY: u32 = 40;
/// Stack pre-touched per render thread (the threads are made with at least 2 MiB)
pub const DEFAULT_STACK: usize = 256 * 1024;

impl SystemRealtime {
    /// Real-time scheduling, through rtkit on `bus` when the limits do not allow it directly (Linux). On other
    /// platforms `bus` is ignored
    #[cfg(target_os = "linux")]
    pub fn new(bus: Option<Arc<lm_sys_linux::SystemBus>>) -> Self {
        Self { priority: DEFAULT_PRIORITY, stack_bytes: DEFAULT_STACK, timeout: Duration::from_secs(2), bus }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new() -> Self {
        Self { priority: DEFAULT_PRIORITY, stack_bytes: DEFAULT_STACK, timeout: Duration::from_secs(2) }
    }
}

#[cfg(not(target_os = "linux"))]
impl Default for SystemRealtime {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeThreads for SystemRealtime {
    #[cfg(target_os = "linux")]
    fn join(&self, _period: Duration) -> Result<RtMembership> {
        lm_sys_linux::prefault_stack(self.stack_bytes);
        let guard = lm_sys_linux::make_current_thread_realtime(self.priority, self.bus.as_deref(), self.timeout)?;
        let method = match guard.method() {
            lm_sys_linux::RtMethod::Direct => RtMethod::Direct,
            lm_sys_linux::RtMethod::Rtkit => RtMethod::Rtkit,
        };
        let priority = guard.priority();
        // The stack locked only within the limit (V6); without it, pre-touched is the best there is
        let stack = lm_sys_linux::lock_current_stack(self.stack_bytes).ok();
        Ok(RtMembership {
            method,
            priority,
            stack_locked: stack.as_ref().map_or(0, lm_sys_linux::StackLock::bytes),
            _guard: Some(guard),
            _stack: stack,
            _not_send: PhantomData,
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn join(&self, _period: Duration) -> Result<RtMembership> {
        Ok(RtMembership::none(RtMethod::Unsupported))
    }
}

/// What the render threads got, for the session's one notice (H1)
#[derive(Debug, Default)]
pub struct RtReport {
    joined: AtomicU32,
    failed: AtomicU32,
    first_failure: Mutex<Option<Error>>,
    last_method: Mutex<Option<(RtMethod, u32)>>,
}

impl RtReport {
    /// Threads that joined the group
    pub fn joined(&self) -> u32 {
        self.joined.load(Ordering::Acquire)
    }

    /// Threads that run without real-time priority
    pub fn failed(&self) -> u32 {
        self.failed.load(Ordering::Acquire)
    }

    /// The first failure, once (the notice is shown once)
    pub fn take_failure(&self) -> Option<Error> {
        self.first_failure.lock().ok().and_then(|mut f| f.take())
    }

    /// How the last thread joined, and at what priority
    pub fn last_method(&self) -> Option<(RtMethod, u32)> {
        self.last_method.lock().ok().and_then(|m| *m)
    }
}

/// The render-thread hook that joins every render thread to the real-time group for its whole life (H1), and
/// records the outcome in `report`. Give the same [`RenderThreads`] to `lm_render::engine_with` (the workers) and
/// to [`crate::Device::new`] (the device thread)
pub fn realtime_hook(rt: Arc<dyn RealtimeThreads>, period: Duration, report: Arc<RtReport>) -> RenderThreads {
    RenderThreads::new(Arc::new(move |_kind: ThreadKind, body: &mut dyn FnMut()| match rt.join(period) {
        Ok(membership) => {
            report.joined.fetch_add(1, Ordering::AcqRel);
            if let Ok(mut m) = report.last_method.lock() {
                *m = Some((membership.method(), membership.priority()));
            }
            body();
            drop(membership);
        }
        Err(e) => {
            report.failed.fetch_add(1, Ordering::AcqRel);
            if let Ok(mut f) = report.first_failure.lock()
                && f.is_none()
            {
                *f = Some(e);
            }
            body();
        }
    }))
}

/// The whole process locked in RAM while it lives (V6)
#[must_use = "dropping it unlocks the memory"]
#[derive(Debug)]
pub struct MemoryLock {
    #[cfg(target_os = "linux")]
    _lock: lm_sys_linux::LockAll,
}

/// Locks all the process's memory when armed or playing (H1), **only when `RLIMIT_MEMLOCK` is unlimited** (V6).
/// Otherwise the error (`linux.memlock.limited`) says so, and the program relies on its locked device buffers and
/// stacks. Mac: `io.memlock.unsupported`
pub fn lock_memory() -> Result<MemoryLock> {
    #[cfg(target_os = "linux")]
    {
        Ok(MemoryLock { _lock: lm_sys_linux::lock_all()? })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error::new("io.memlock.unsupported"))
    }
}

/// A buffer of the device loop: locked in RAM when the limit allows (V6), plain memory otherwise
pub enum RtBuffer {
    #[cfg(target_os = "linux")]
    Locked(lm_sys_linux::LockedBuffer<f32>),
    Plain(Vec<f32>),
}

impl fmt::Debug for RtBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtBuffer").field("len", &self.len()).field("locked", &self.is_locked()).finish()
    }
}

impl RtBuffer {
    /// `len` zeroes, locked if possible
    pub fn zeroed(len: usize) -> Self {
        #[cfg(target_os = "linux")]
        if let Ok(b) = lm_sys_linux::LockedBuffer::zeroed(len.max(1)) {
            return RtBuffer::Locked(b);
        }
        RtBuffer::Plain(vec![0.0; len.max(1)])
    }

    pub fn is_locked(&self) -> bool {
        #[cfg(target_os = "linux")]
        let locked = matches!(self, RtBuffer::Locked(_));
        #[cfg(not(target_os = "linux"))]
        let locked = false;
        locked
    }
}

impl Deref for RtBuffer {
    type Target = [f32];
    fn deref(&self) -> &[f32] {
        match self {
            #[cfg(target_os = "linux")]
            RtBuffer::Locked(b) => b,
            RtBuffer::Plain(v) => v,
        }
    }
}

impl DerefMut for RtBuffer {
    fn deref_mut(&mut self) -> &mut [f32] {
        match self {
            #[cfg(target_os = "linux")]
            RtBuffer::Locked(b) => b,
            RtBuffer::Plain(v) => v,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_joins_and_reports() {
        let report = Arc::new(RtReport::default());
        let threads = realtime_hook(Arc::new(NoRealtime), Duration::from_millis(5), report.clone());
        let mut ran = false;
        threads.run(ThreadKind::Audio, &mut || ran = true);
        assert!(ran);
        assert_eq!(report.joined(), 1);
        assert_eq!(report.last_method(), Some((RtMethod::None, 0)));
    }

    struct Refuses;
    impl RealtimeThreads for Refuses {
        fn join(&self, _: Duration) -> Result<RtMembership> {
            Err(Error::new("linux.realtime.denied"))
        }
    }

    #[test]
    fn a_refusal_still_runs_the_thread_and_is_reported_once() {
        let report = Arc::new(RtReport::default());
        let threads = realtime_hook(Arc::new(Refuses), Duration::from_millis(5), report.clone());
        let mut ran = 0;
        threads.run(ThreadKind::Audio, &mut || ran += 1);
        threads.run(ThreadKind::Worker(0), &mut || ran += 1);
        assert_eq!(ran, 2);
        assert_eq!(report.failed(), 2);
        assert_eq!(report.take_failure().map(|e| e.key()), Some("linux.realtime.denied"));
        assert!(report.take_failure().is_none());
    }

    #[test]
    fn buffers_are_usable_locked_or_not() {
        let mut b = RtBuffer::zeroed(1000);
        b[999] = 1.0;
        assert_eq!(b.len(), 1000);
        assert_eq!(b[999], 1.0);
    }
}
