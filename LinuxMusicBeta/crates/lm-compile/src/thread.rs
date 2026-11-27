//! The compiler on its own thread (I-5.4, R17, T10, H14).
//!
//! The main thread owns a [`Compiler`]: it submits roots (only the newest is compiled; one submitted while another
//! compiles replaces the waiting one), hands over what comes back from the audio side and the processors the
//! session built, and polls the results in order. The audio keeps the previous program while a compilation runs:
//! there is never silence because of one. A panic of the translation is caught: the result is the error
//! `compile.panicked`, and the compiler goes on from the table of its last good install (H14).

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lm_base::{Error, Result, Role};
use lm_render::{NodeCore, NodeIndex, ProgramGen, Returned};
use lm_state::Value;

use crate::catalog::{PluginCatalog, Resources};
use crate::core::{CompilerConfig, CompilerCore};
use crate::types::{Compiled, Environment, InstalledTable, NodeKey, Ticket};
use lm_state::SchemaRegistry;

/// What a compilation running for long shows on the project (T10: past 200 ms the session shows "applying")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileActivity {
    pub since: Instant,
}

enum Message {
    Returned { generation: ProgramGen, freed: Vec<NodeIndex> },
    Ready { ticket: Ticket, core: Box<NodeCore> },
    TailEnded(NodeIndex),
    /// The audio clock passed a tail's safety deadline: look again (carries nothing, the clock is in `Shared`)
    ClockDue,
    Latency(NodeKey, u32),
    Rebuild(Box<InstalledTable>),
    Renew(NodeKey),
    Catalog(Arc<dyn PluginCatalog>, Arc<SchemaRegistry>),
}

struct Job {
    /// The number of the submission (the newest one is compiled, R17)
    seq: u64,
    root: Value,
    resources: Arc<dyn Resources>,
    env: Environment,
}

#[derive(Default)]
struct Inbox {
    job: Option<Job>,
    messages: Vec<Message>,
    shutdown: bool,
}

struct Shared {
    inbox: Mutex<Inbox>,
    wake: Condvar,
    results: Mutex<VecDeque<Result<Compiled>>>,
    activity: Mutex<Option<CompileActivity>>,
    /// Submissions made, and the newest one whose result is in `results`: a submission is pending until then
    submitted: AtomicU64,
    answered: AtomicU64,
    /// The engine's audio clock as last reported, and the clock value the compiler wants to be woken at
    /// (`u64::MAX`: none)
    frames: AtomicU64,
    wake_at: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The compiler, from the main thread's side (`lm-session` keeps one)
pub struct Compiler {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Compiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiler").field("activity", &self.activity()).finish_non_exhaustive()
    }
}

impl Compiler {
    /// A compiler with nothing installed yet
    pub fn new(config: CompilerConfig) -> Self {
        Self::start(CompilerCore::new(config))
    }

    /// A compiler that continues from the table of the last install the session sent (H14)
    pub fn from_table(config: CompilerConfig, table: &InstalledTable) -> Self {
        Self::start(CompilerCore::from_table(config, table))
    }

    fn start(core: CompilerCore) -> Self {
        let shared = Arc::new(Shared {
            inbox: Mutex::new(Inbox::default()),
            wake: Condvar::new(),
            results: Mutex::new(VecDeque::new()),
            activity: Mutex::new(None),
            submitted: AtomicU64::new(0),
            answered: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            wake_at: AtomicU64::new(u64::MAX),
        });
        let s = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("lm-compile".into())
            .spawn(move || run(core, &s))
            .expect("the compile thread starts");
        Self { shared, thread: Some(thread) }
    }

    fn send(&self, m: Message) {
        lock(&self.shared.inbox).messages.push(m);
        self.shared.wake.notify_one();
    }

    /// Compiles `root` (with the resources loaded now and the machine's environment). Only the newest submitted
    /// root is compiled (R17)
    pub fn submit(&mut self, root: Value, resources: Arc<dyn Resources>, env: Environment) {
        let seq = self.shared.submitted.fetch_add(1, Ordering::AcqRel) + 1;
        lock(&self.shared.inbox).job = Some(Job { seq, root, resources, env });
        self.shared.wake.notify_one();
    }

    /// The next result, in order. Each `Ok` must be installed (`EngineControl::install`) before the next one
    pub fn poll(&mut self) -> Option<Result<Compiled>> {
        lock(&self.shared.results).pop_front()
    }

    /// Waits for the next result (tests and the export, which run outside the main thread's loop)
    pub fn wait(&mut self, timeout: Duration) -> Option<Result<Compiled>> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(r) = self.poll() {
                return Some(r);
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// An install came back (`EngineControl::collect`): its indices and slots are free (S6)
    pub fn returned(&mut self, returned: &Returned) {
        self.send(Message::Returned { generation: returned.generation, freed: returned.freed.clone() });
    }

    /// A processor the session built for a request (T6); it goes into the next install
    pub fn ready(&mut self, ticket: Ticket, core: Box<NodeCore>) {
        self.send(Message::Ready { ticket, core });
    }

    /// The engine's audio clock (`TelemetryFrame::frames`). A tail whose end was never reported is dropped when the
    /// clock passes its safety deadline; wall time plays no part, so a slow or fast machine compiles the same
    pub fn observe_frames(&mut self, frames: u64) {
        self.shared.frames.fetch_max(frames, Ordering::AcqRel);
        if frames >= self.shared.wake_at.load(Ordering::Acquire) {
            let mut inbox = lock(&self.shared.inbox);
            if !matches!(inbox.messages.last(), Some(Message::ClockDue)) {
                inbox.messages.push(Message::ClockDue);
            }
            drop(inbox);
            self.shared.wake.notify_one();
        }
    }

    /// `Telemetry::TailEnded`
    pub fn tail_ended(&mut self, node: NodeIndex) {
        self.send(Message::TailEnded(node));
    }

    /// `PluginRequest::LatencyChanged` with the processor's new latency
    pub fn latency_changed(&mut self, node: NodeKey, latency: u32) {
        self.send(Message::Latency(node, latency));
    }

    /// Builds a node's processor again with the same state ([`CompilerCore::renew`]); the next submitted root
    /// requests it
    pub fn renew(&mut self, node: NodeKey) {
        self.send(Message::Renew(node));
    }

    /// The catalog grew ([`CompilerCore::set_catalog`]); applies before the next compilation
    pub fn set_catalog(&mut self, plugins: Arc<dyn PluginCatalog>, schemas: Arc<SchemaRegistry>) {
        self.send(Message::Catalog(plugins, schemas));
    }

    /// Starts over from a table (H14)
    pub fn rebuild(&mut self, table: InstalledTable) {
        self.send(Message::Rebuild(Box::new(table)));
    }

    /// A submitted root has no result yet: queued, compiling, or between the two. Unlike [`Compiler::activity`],
    /// true from the moment of `submit` until its result can be polled (a session waiting to be settled)
    pub fn pending(&self) -> bool {
        self.shared.answered.load(Ordering::Acquire) < self.shared.submitted.load(Ordering::Acquire)
    }

    /// A compilation running, and since when (T10)
    pub fn activity(&self) -> Option<CompileActivity> {
        *lock(&self.shared.activity)
    }
}

impl Drop for Compiler {
    fn drop(&mut self) {
        lock(&self.shared.inbox).shutdown = true;
        self.shared.wake.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(mut core: CompilerCore, shared: &Shared) {
    lm_base::set_role(Role::Compile);
    let mut config = core.config().clone();
    // The last root compiled (a processor arriving or a tail ending recompiles it) and the last good table
    let mut last: Option<Job> = None;
    let mut good = core.table();
    // The last result was refused for want of indices or slots still on their way back
    let mut retry = false;
    loop {
        shared.wake_at.store(core.next_tail_deadline().unwrap_or(u64::MAX), Ordering::Release);
        let (job, messages) = {
            let mut inbox = lock(&shared.inbox);
            loop {
                if inbox.shutdown {
                    return;
                }
                if inbox.job.is_some() || !inbox.messages.is_empty() {
                    break;
                }
                inbox = shared.wake.wait(inbox).unwrap_or_else(PoisonError::into_inner);
            }
            (inbox.job.take(), std::mem::take(&mut inbox.messages))
        };
        core.observe_frames(shared.frames.load(Ordering::Acquire));
        let mut news = false;
        for m in messages {
            match m {
                Message::Returned { generation, freed } => {
                    core.returned_parts(generation, &freed);
                    // A compilation refused for want of indices can go now
                    news = true;
                }
                Message::Ready { ticket, core: c } => news |= core.ready(ticket, c),
                Message::TailEnded(n) => news |= core.tail_ended(n),
                Message::ClockDue => {}
                Message::Latency(k, l) => news |= core.latency_changed(k, l),
                Message::Renew(k) => news |= core.renew(k),
                Message::Catalog(plugins, schemas) => {
                    config.plugins = Arc::clone(&plugins);
                    config.schemas = Arc::clone(&schemas);
                    core.set_catalog(plugins, schemas);
                }
                Message::Rebuild(table) => {
                    core = CompilerCore::from_table(config.clone(), &table);
                    core.observe_frames(shared.frames.load(Ordering::Acquire));
                    good = *table;
                    news = true;
                }
            }
        }
        shared.wake_at.store(core.next_tail_deadline().unwrap_or(u64::MAX), Ordering::Release);
        let job = match job {
            Some(j) => j,
            None => {
                let due = core.has_pending_work() || (news && retry);
                match (&last, due) {
                    (Some(_), true) => last.take().expect("checked"),
                    _ => continue,
                }
            }
        };
        *lock(&shared.activity) = Some(CompileActivity { since: Instant::now() });
        let outcome = catch_unwind(AssertUnwindSafe(|| core.compile(&job.root, job.resources.as_ref(), &job.env)));
        *lock(&shared.activity) = None;
        let result = match outcome {
            Ok(Ok(c)) => {
                good = (*c.table).clone();
                Ok(c)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                core = CompilerCore::from_table(config.clone(), &good);
                Err(Error::new("compile.panicked"))
            }
        };
        retry = result
            .as_ref()
            .err()
            .is_some_and(|e| matches!(e.key(), "compile.limits.nodes_pending" | "compile.limits.params_pending"));
        lock(&shared.results).push_back(result);
        shared.answered.fetch_max(job.seq, Ordering::AcqRel);
        last = Some(job);
    }
}
