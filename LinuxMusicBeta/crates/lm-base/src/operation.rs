//! Long operations (interfaces.md, I-2): anything that may take more than 200 ms runs on a worker, has a phase, a
//! progress when it can be measured, and can be cancelled when that makes sense. The main thread never waits on one:
//! it polls the handle.

use std::collections::VecDeque;
use std::fmt;
use std::thread::JoinHandle;

use crate::error::{Error, Label, Result};
use crate::main_thread::{set_role, Role};
use crate::sync::{Arc, AtomicBool, AtomicU32, Mutex, Ordering};

/// Asks an operation to stop. Cheap to clone and to check; the work checks it at its own steps
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// `Err(cancelled)` once cancelled: `cancel.check()?` at each step of the work
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(Error::cancelled()) } else { Ok(()) }
    }
}

/// How far an operation is: a fraction from 0 to 1 when measurable, and the stage it is in
#[derive(Clone, Debug, Default)]
pub struct Progress(Arc<ProgressState>);

#[derive(Debug, Default)]
struct ProgressState {
    /// f32 bits; `u32::MAX` means "not measurable"
    fraction: AtomicU32,
    measured: AtomicBool,
    stage: Mutex<Option<Label>>,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the fraction done, clamped to 0..=1
    pub fn set(&self, fraction: f32) {
        let fraction = if fraction.is_nan() { 0.0 } else { fraction.clamp(0.0, 1.0) };
        self.0.fraction.store(fraction.to_bits(), Ordering::Relaxed);
        self.0.measured.store(true, Ordering::Release);
    }

    /// Sets the stage shown to the user ("Decoding", "Preparing the instrument")
    pub fn stage(&self, label: Label) {
        if let Ok(mut stage) = self.0.stage.lock() {
            *stage = Some(label);
        }
    }

    /// The fraction, if the work measured one
    pub fn fraction(&self) -> Option<f32> {
        self.0
            .measured
            .load(Ordering::Acquire)
            .then(|| f32::from_bits(self.0.fraction.load(Ordering::Relaxed)))
    }

    pub fn current_stage(&self) -> Option<Label> {
        self.0.stage.lock().ok().and_then(|s| s.clone())
    }
}

/// What a long operation is doing (interfaces.md, I-2.2)
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Queued,
    Running { progress: Option<f32>, stage: Option<Label> },
    Done,
    Failed(Error),
    Cancelled,
}

/// A number that names an operation; never repeated within a run
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId(pub u64);

static NEXT_OPERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl OperationId {
    fn next() -> Self {
        Self(NEXT_OPERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

/// What the worker writes and the handle reads
#[derive(Debug)]
struct Shared<T> {
    started: AtomicBool,
    finished: AtomicBool,
    result: Mutex<Option<Result<T>>>,
}

/// The main thread's handle to a long operation. Dropping it without taking the result cancels the work and discards
/// the result (interfaces.md, I-2; `#[must_use]`, S21)
#[must_use = "dropping an operation's handle cancels it"]
pub struct OperationHandle<T> {
    id: OperationId,
    cancel: Cancel,
    progress: Progress,
    shared: Arc<Shared<T>>,
    taken: bool,
}

impl<T> fmt::Debug for OperationHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationHandle").field("id", &self.id).field("phase", &self.phase()).finish()
    }
}

impl<T> OperationHandle<T> {
    pub fn id(&self) -> OperationId {
        self.id
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// The phase now. After the result has been taken by `poll`, it stays at what the result was
    pub fn phase(&self) -> Phase {
        if self.shared.finished.load(Ordering::Acquire) {
            match self.shared.result.lock().ok().as_deref() {
                Some(Some(Ok(_))) | Some(None) => Phase::Done,
                Some(Some(Err(e))) if e.is_cancelled() => Phase::Cancelled,
                Some(Some(Err(e))) => Phase::Failed(e.clone()),
                None => Phase::Failed(Error::new("error.operation.poisoned")),
            }
        } else if self.shared.started.load(Ordering::Acquire) {
            Phase::Running { progress: self.progress.fraction(), stage: self.progress.current_stage() }
        } else {
            Phase::Queued
        }
    }

    /// The result, once: `None` while running, `Some` exactly once when finished
    pub fn poll(&mut self) -> Option<Result<T>> {
        if self.taken || !self.shared.finished.load(Ordering::Acquire) {
            return None;
        }
        let result = self.shared.result.lock().ok()?.take();
        if result.is_some() {
            self.taken = true;
        }
        result
    }

    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
    }
}

impl<T> Drop for OperationHandle<T> {
    fn drop(&mut self) {
        if !self.taken {
            self.cancel.cancel();
        }
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct Queue {
    jobs: std::sync::Mutex<VecDeque<Job>>,
    ready: std::sync::Condvar,
    closing: std::sync::atomic::AtomicBool,
}

impl fmt::Debug for Queue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let queued = self.jobs.lock().map(|jobs| jobs.len()).unwrap_or(0);
        f.debug_struct("Queue").field("queued", &queued).finish()
    }
}

/// A fixed pool of worker threads that runs long operations (interfaces.md, I-2). Its `Drop` cancels nothing by
/// itself (the handles do) but closes the queue and joins every thread, so no work outlives the pool (T18)
#[derive(Debug)]
pub struct Workers {
    queue: std::sync::Arc<Queue>,
    threads: Vec<JoinHandle<()>>,
    name: &'static str,
}

impl Workers {
    /// A pool of `count` threads, named `<name>-<n>`, with the given role
    pub fn new(name: &'static str, count: usize, role: Role) -> Self {
        let queue = std::sync::Arc::new(Queue::default());
        let threads = (0..count.max(1))
            .map(|n| {
                let queue = queue.clone();
                std::thread::Builder::new()
                    .name(format!("{name}-{n}"))
                    .spawn(move || {
                        set_role(role);
                        worker_loop(&queue);
                    })
                    .expect("the system refused to make a worker thread")
            })
            .collect();
        Self { queue, threads, name }
    }

    /// Starts `work` on a worker. The work gets the operation's `Cancel` and `Progress`, and must never touch the
    /// session: it receives data and returns a value (T18)
    pub fn start<T, F>(&self, work: F) -> OperationHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(&Cancel, &Progress) -> Result<T> + Send + 'static,
    {
        let cancel = Cancel::new();
        let progress = Progress::new();
        let shared = Arc::new(Shared { started: AtomicBool::new(false), finished: AtomicBool::new(false), result: Mutex::new(None) });
        let job_cancel = cancel.clone();
        let job_progress = progress.clone();
        let job_shared = shared.clone();
        let job: Job = Box::new(move || {
            job_shared.started.store(true, Ordering::Release);
            let result = if job_cancel.is_cancelled() {
                Err(Error::cancelled())
            } else {
                // A panic in an operation is a bug: it becomes a failure, never a dead worker (H14)
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&job_cancel, &job_progress))) {
                    Ok(result) => result,
                    Err(_) => Err(Error::new("error.operation.panicked")),
                }
            };
            if let Ok(mut slot) = job_shared.result.lock() {
                *slot = Some(result);
            }
            job_shared.finished.store(true, Ordering::Release);
        });
        if let Ok(mut jobs) = self.queue.jobs.lock() {
            jobs.push_back(job);
        }
        self.queue.ready.notify_one();
        OperationHandle { id: OperationId::next(), cancel, progress, shared, taken: false }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn threads(&self) -> usize {
        self.threads.len()
    }
}

fn worker_loop(queue: &Queue) {
    loop {
        let job = {
            let Ok(mut jobs) = queue.jobs.lock() else { return };
            loop {
                if let Some(job) = jobs.pop_front() {
                    break Some(job);
                }
                if queue.closing.load(std::sync::atomic::Ordering::Acquire) {
                    break None;
                }
                jobs = match queue.ready.wait(jobs) {
                    Ok(jobs) => jobs,
                    Err(_) => return,
                };
            }
        };
        match job {
            Some(job) => job(),
            None => return,
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.queue.closing.store(true, std::sync::atomic::Ordering::Release);
        self.queue.ready.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait<T>(handle: &mut OperationHandle<T>) -> Result<T> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(result) = handle.poll() {
                return result;
            }
            assert!(Instant::now() < deadline, "operation did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn runs_and_returns_once() {
        let workers = Workers::new("test", 2, Role::Worker);
        let mut op = workers.start(|_, progress| {
            progress.set(0.5);
            Ok(42)
        });
        assert_eq!(wait(&mut op).expect("value"), 42);
        assert!(op.poll().is_none(), "the result is given once");
        assert_eq!(op.phase(), Phase::Done);
    }

    #[test]
    fn cancel_reaches_the_work() {
        let workers = Workers::new("test", 1, Role::Worker);
        let mut op = workers.start(|cancel, _| {
            for _ in 0..10_000 {
                cancel.check()?;
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        });
        op.cancel();
        let result = wait(&mut op);
        assert!(result.expect_err("cancelled").is_cancelled());
    }

    #[test]
    fn a_panic_is_a_failure_not_a_dead_worker() {
        let workers = Workers::new("test", 1, Role::Worker);
        let mut bad = workers.start(|_, _| -> Result<()> { panic!("bug") });
        assert_eq!(wait(&mut bad).expect_err("failed").key(), "error.operation.panicked");
        let mut good = workers.start(|_, _| Ok(1));
        assert_eq!(wait(&mut good).expect("the worker survived"), 1);
    }

    #[test]
    fn dropping_the_handle_cancels() {
        let workers = Workers::new("test", 1, Role::Worker);
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen_in_work = seen.clone();
        let op = workers.start(move |cancel, _| {
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            seen_in_work.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        std::thread::sleep(Duration::from_millis(5));
        drop(op);
        drop(workers); // joins: the work saw the cancel and returned
        assert!(seen.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn progress_is_clamped_and_optional() {
        let progress = Progress::new();
        assert_eq!(progress.fraction(), None);
        progress.set(2.0);
        assert_eq!(progress.fraction(), Some(1.0));
        progress.set(f32::NAN);
        assert_eq!(progress.fraction(), Some(0.0));
    }
}

#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    /// The handshake between the worker that finishes and the main thread that polls: the result is seen once,
    /// and never before `finished`
    #[test]
    fn finish_and_poll() {
        loom::model(|| {
            let shared = Arc::new(Shared::<u32> {
                started: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                result: Mutex::new(None),
            });
            let worker = {
                let shared = shared.clone();
                loom::thread::spawn(move || {
                    shared.started.store(true, Ordering::Release);
                    *shared.result.lock().unwrap() = Some(Ok(7));
                    shared.finished.store(true, Ordering::Release);
                })
            };
            let mut handle = OperationHandle {
                id: OperationId(1),
                cancel: Cancel::new(),
                progress: Progress::new(),
                shared,
                taken: false,
            };
            let mut seen = 0;
            for _ in 0..3 {
                if let Some(Ok(v)) = handle.poll() {
                    assert_eq!(v, 7);
                    seen += 1;
                }
                loom::thread::yield_now();
            }
            worker.join().unwrap();
            if let Some(Ok(_)) = handle.poll() {
                seen += 1;
            }
            assert_eq!(seen, 1);
        });
    }
}
