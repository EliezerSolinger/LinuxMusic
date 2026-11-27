//! The LV2 worker (worker:schedule): the host side of a plugin's non-real-time work.
//!
//! Requests go from `run` (the audio thread) to the worker through a lock-free byte ring, and responses come back
//! through another; each message is a little-endian `u32` length and the bytes. The worker thread belongs to the
//! instance and is **joined before the instance is freed** (`Instance`'s `Drop`; the legacy program freed the
//! instance first, lacuna 2 of plano-ciclo-de-vida.md). In [`WorkerMode::Synchronous`] there is no thread: the work
//! runs inside `schedule_work`, on the thread that runs the plugin (offline export: deterministic).

use crate::ffi;
use rtrb::{Consumer, Producer, RingBuffer};
use std::ffi::{c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{JoinHandle, Thread};
use std::time::Duration;

/// How the instance runs its plugin's work
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum WorkerMode {
    /// A worker thread of the instance's own, woken by `schedule_work`; responses are delivered after a later
    /// `run`. For live playing
    #[default]
    Threaded,
    /// The work runs inside `schedule_work`, on the plugin's thread, and its responses are delivered after the same
    /// `run`. Not real-time safe (the work may load files): for offline export, where it makes the render
    /// deterministic
    Synchronous,
}

const HEADER: usize = 4;

/// Writes one message whole, or nothing (no space). Does not allocate
pub(crate) fn push_message(ring: &mut Producer<u8>, data: &[u8]) -> bool {
    let Ok(len) = u32::try_from(data.len()) else {
        return false;
    };
    let Ok(mut chunk) = ring.write_chunk(HEADER + data.len()) else {
        return false;
    };
    let header = len.to_le_bytes();
    let (first, second) = chunk.as_mut_slices();
    let mut source = header.iter().chain(data.iter());
    for slot in first.iter_mut().chain(second.iter_mut()) {
        // The chunk is exactly HEADER + len long: the source cannot run out first
        if let Some(byte) = source.next() {
            *slot = *byte;
        }
    }
    chunk.commit_all();
    true
}

/// Reads one whole message into `scratch` and returns its length; `None` when the ring is empty. A message longer
/// than `scratch` is skipped (it cannot happen: both sides use the ring's capacity). Does not allocate
pub(crate) fn pop_message(ring: &mut Consumer<u8>, scratch: &mut [u8]) -> Option<usize> {
    loop {
        // Peek at the header without consuming it (a chunk that is not committed consumes nothing)
        let mut bytes = [0_u8; HEADER];
        {
            let header = ring.read_chunk(HEADER).ok()?;
            let (a, b) = header.as_slices();
            for (slot, byte) in bytes.iter_mut().zip(a.iter().chain(b.iter())) {
                *slot = *byte;
            }
        }
        let len = u32::from_le_bytes(bytes) as usize;
        // A message is committed whole, so its body is there once its header is
        let chunk = ring.read_chunk(HEADER + len).ok()?;
        let (a, b) = chunk.as_slices();
        let fits = len <= scratch.len();
        if fits {
            for (slot, byte) in scratch.iter_mut().zip(a.iter().chain(b.iter()).skip(HEADER)) {
                *slot = *byte;
            }
        }
        chunk.commit_all();
        if fits {
            return Some(len);
        }
    }
}

/// A byte buffer aligned to 8, so that a plugin may read its message as a struct
pub(crate) struct Scratch(Box<[u64]>);

impl Scratch {
    pub fn new(bytes: usize) -> Self {
        Self(vec![0_u64; bytes.div_ceil(8)].into_boxed_slice())
    }

    pub fn bytes(&mut self) -> &mut [u8] {
        let len = self.0.len() * 8;
        // SAFETY: the u64 slice is `len` bytes of initialised memory, exclusively borrowed; u8 has no alignment or
        // validity requirements
        unsafe { std::slice::from_raw_parts_mut(self.0.as_mut_ptr().cast::<u8>(), len) }
    }

    pub fn ptr(&self) -> *const c_void {
        self.0.as_ptr().cast()
    }
}

/// The plugin's handle and its worker interface
#[derive(Clone, Copy)]
pub(crate) struct Target {
    pub handle: ffi::LV2_Handle,
    pub iface: *const ffi::LV2_Worker_Interface,
}

// SAFETY: the LV2 worker spec lets the host call `work` from any non-real-time thread, one call at a time; the handle
// stays valid until the worker thread is joined (Instance's Drop joins it before lilv_instance_free)
unsafe impl Send for Target {}

impl Target {
    /// Calls the plugin's `work`, catching nothing (the plugin is C); `respond` and its handle go along
    ///
    /// # Safety
    /// The handle must be alive, `work` must not run concurrently with another `work` of the same instance, and
    /// `respond_handle` must be what `respond` expects
    pub unsafe fn work(
        self,
        respond: ffi::LV2_Worker_Respond_Function,
        respond_handle: *mut c_void,
        size: u32,
        data: *const c_void,
    ) -> c_int {
        // SAFETY: the interface pointer came from the plugin's extension_data and lives as long as the plugin
        let Some(work) = (unsafe { &*self.iface }).work else {
            return ffi::LV2_WORKER_ERR_UNKNOWN;
        };
        // SAFETY: the caller's contract
        unsafe { work(self.handle, respond, respond_handle, size, data) }
    }
}

/// `respond` for a ring: the handle is a `*mut Producer<u8>` used by one thread for the duration of a `work` call
unsafe extern "C" fn respond_to_ring(handle: *mut c_void, size: u32, data: *const c_void) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() || (data.is_null() && size > 0) {
            return ffi::LV2_WORKER_ERR_UNKNOWN;
        }
        // SAFETY: the handle is the producer the host passed with this `work` call, not used elsewhere meanwhile
        let ring = unsafe { &mut *handle.cast::<Producer<u8>>() };
        let bytes = if size == 0 {
            &[][..]
        } else {
            // SAFETY: the plugin passes `size` readable bytes, valid for the call
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }
        };
        if push_message(ring, bytes) { ffi::LV2_WORKER_SUCCESS } else { ffi::LV2_WORKER_ERR_NO_SPACE }
    }))
    .unwrap_or(ffi::LV2_WORKER_ERR_UNKNOWN)
}

/// `respond` into a list (restore time, where allocating is fine): the handle is a `*mut Vec<Vec<u8>>`
unsafe extern "C" fn respond_to_list(handle: *mut c_void, size: u32, data: *const c_void) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() || (data.is_null() && size > 0) {
            return ffi::LV2_WORKER_ERR_UNKNOWN;
        }
        // SAFETY: the handle is the list of the restore in progress, used by this thread only
        let list = unsafe { &mut *handle.cast::<Vec<Vec<u8>>>() };
        let bytes = if size == 0 {
            Vec::new()
        } else {
            // SAFETY: `size` readable bytes, valid for the call
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }.to_vec()
        };
        list.push(bytes);
        ffi::LV2_WORKER_SUCCESS
    }))
    .unwrap_or(ffi::LV2_WORKER_ERR_UNKNOWN)
}

/// Counters the audio side and the worker share, and the lock the restore waits on
#[derive(Default)]
pub(crate) struct Shared {
    pub stop: AtomicBool,
    pub scheduled: AtomicU64,
    pub done: AtomicU64,
    lock: Mutex<()>,
    idle: Condvar,
}

/// What `schedule_work`'s handle points to. Allocated once per instance and never moved (its address is in the
/// plugin's features); freed after the plugin
pub(crate) struct Schedule {
    pub mode: WorkerMode,
    /// Set after instantiation (a plugin schedules from `run`, never from `instantiate`)
    pub target: Option<Target>,
    /// Threaded: the audio side of the request ring
    pub requests: Option<Producer<u8>>,
    /// Threaded: the worker, to wake
    pub thread: Option<Thread>,
    /// Synchronous: where `work`'s responses go (read after `run` like the threaded ones)
    pub sync_responses: Option<Producer<u8>>,
    pub shared: Arc<Shared>,
}

/// `LV2_Worker_Schedule::schedule_work`, called by the plugin from `run`. Real-time safe in threaded mode (one ring
/// write, two atomics, `Thread::unpark`); runs the work in place in synchronous mode
pub(crate) unsafe extern "C" fn schedule_work(handle: *mut c_void, size: u32, data: *const c_void) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() || (data.is_null() && size > 0) {
            return ffi::LV2_WORKER_ERR_UNKNOWN;
        }
        let ctx = handle.cast::<Schedule>();
        // SAFETY: `ctx` is the instance's `Schedule`, alive for the plugin's life. The plugin calls this from `run`,
        // which the instance calls with `&mut self`: no other code touches the Schedule meanwhile. Fields are reached
        // through raw pointers and no reference is held across the call into the plugin's `work` below
        unsafe {
            match (*ctx).mode {
                WorkerMode::Threaded => {
                    let Some(ring) = (*ctx).requests.as_mut() else {
                        return ffi::LV2_WORKER_ERR_UNKNOWN;
                    };
                    let bytes =
                        if size == 0 { &[][..] } else { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) };
                    // Counted before the push: a quick `work` may finish (and count `done`) before this returns, and
                    // `wait_idle` must never see `done` ahead of `scheduled`
                    let shared: &Shared = &(*ctx).shared;
                    shared.scheduled.fetch_add(1, Ordering::AcqRel);
                    if !push_message(ring, bytes) {
                        shared.scheduled.fetch_sub(1, Ordering::AcqRel);
                        return ffi::LV2_WORKER_ERR_NO_SPACE;
                    }
                    if let Some(thread) = (*ctx).thread.as_ref() {
                        thread.unpark();
                    }
                    ffi::LV2_WORKER_SUCCESS
                }
                WorkerMode::Synchronous => {
                    let Some(target) = (*ctx).target else {
                        return ffi::LV2_WORKER_ERR_UNKNOWN;
                    };
                    let Some(ring) = (*ctx).sync_responses.as_mut() else {
                        return ffi::LV2_WORKER_ERR_UNKNOWN;
                    };
                    let ring: *mut Producer<u8> = ring;
                    target.work(respond_to_ring, ring.cast(), size, data)
                }
            }
        }
    }))
    .unwrap_or(ffi::LV2_WORKER_ERR_UNKNOWN)
}

/// The worker of a restore: the work runs in place and the responses are kept in a list, delivered when the restore
/// returns (allocating is fine there)
pub(crate) struct RestoreWorker {
    pub target: Target,
    pub responses: Vec<Vec<u8>>,
}

/// `schedule_work` for a restore
pub(crate) unsafe extern "C" fn schedule_restore_work(handle: *mut c_void, size: u32, data: *const c_void) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() || (data.is_null() && size > 0) {
            return ffi::LV2_WORKER_ERR_UNKNOWN;
        }
        let ctx = handle.cast::<RestoreWorker>();
        // SAFETY: the RestoreWorker of the restore in progress, on this thread; the responses list is reached by raw
        // pointer and not borrowed across the plugin's call
        unsafe {
            let target = (*ctx).target;
            let list: *mut Vec<Vec<u8>> = std::ptr::addr_of_mut!((*ctx).responses);
            target.work(respond_to_list, list.cast(), size, data)
        }
    }))
    .unwrap_or(ffi::LV2_WORKER_ERR_UNKNOWN)
}

/// The worker thread of one instance
pub(crate) struct WorkerThread {
    shared: Arc<Shared>,
    join: Option<JoinHandle<()>>,
}

impl WorkerThread {
    /// Starts the thread; it takes the worker side of both rings
    pub fn start(
        target: Target,
        requests: Consumer<u8>,
        responses: Producer<u8>,
        capacity: usize,
        shared: Arc<Shared>,
    ) -> std::io::Result<Self> {
        let thread_shared = Arc::clone(&shared);
        let join = std::thread::Builder::new()
            .name("lv2-worker".into())
            .spawn(move || worker_main(target, requests, responses, capacity, &thread_shared))?;
        Ok(Self { shared, join: Some(join) })
    }

    pub fn thread(&self) -> Option<Thread> {
        self.join.as_ref().map(|j| j.thread().clone())
    }

    /// Waits until every request scheduled so far has been worked (before a restore, which may call `work` itself).
    /// Blocks: never on the audio thread
    pub fn wait_idle(&self) {
        let mut guard = self.shared.lock.lock().unwrap_or_else(PoisonError::into_inner);
        while self.shared.done.load(Ordering::Acquire) < self.shared.scheduled.load(Ordering::Acquire) {
            // A worker thread that has died will never catch up: waiting for it would hang the restore
            if self.join.as_ref().is_none_or(|j| j.is_finished()) {
                return;
            }
            if let Some(join) = &self.join {
                join.thread().unpark();
            }
            guard = self
                .shared
                .idle
                .wait_timeout(guard, Duration::from_millis(10))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Works every request still queued, then stops and **joins** the thread. After this, no plugin code runs on the
    /// worker any more: the instance may be freed
    pub fn stop_and_join(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.thread().unpark();
            // A panic cannot happen in the loop (it only moves bytes and calls C); if it did, the thread is gone
            // either way, which is what matters here
            let _ = join.join();
        }
    }
}

impl Drop for WorkerThread {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn worker_main(
    target: Target,
    mut requests: Consumer<u8>,
    mut responses: Producer<u8>,
    capacity: usize,
    shared: &Shared,
) {
    let mut scratch = Scratch::new(capacity);
    loop {
        while let Some(len) = pop_message(&mut requests, scratch.bytes()) {
            let ring: *mut Producer<u8> = &mut responses;
            // SAFETY: the plugin is alive until this thread is joined; this thread is the only one calling `work`
            // (the restore waits for `done == scheduled` first, and synchronous mode has no thread); the message is
            // in an 8-aligned scratch buffer valid for the call; the response ring is this thread's own
            unsafe { target.work(respond_to_ring, ring.cast(), len as u32, scratch.ptr()) };
            shared.done.fetch_add(1, Ordering::AcqRel);
            let _guard = shared.lock.lock().unwrap_or_else(PoisonError::into_inner);
            shared.idle.notify_all();
        }
        if shared.stop.load(Ordering::Acquire) && requests.is_empty() {
            return;
        }
        std::thread::park();
    }
}

/// The rings of a threaded worker: (audio requests, worker requests, worker responses, audio responses)
pub(crate) fn rings(capacity: usize) -> (Producer<u8>, Consumer<u8>, Producer<u8>, Consumer<u8>) {
    let (req_tx, req_rx) = RingBuffer::new(capacity);
    let (resp_tx, resp_rx) = RingBuffer::new(capacity);
    (req_tx, req_rx, resp_tx, resp_rx)
}
