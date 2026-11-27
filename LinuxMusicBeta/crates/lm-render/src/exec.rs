//! Running nodes: one node's call ([`run_job`]) and the worker threads that run the nodes of a level in parallel.
//!
//! Ownership moves, nothing is shared: for each node it hands out, the audio thread moves the node's processor and
//! its buffer block into a [`Job`] and pushes it into that worker's ring; the worker runs it and pushes it back; the
//! audio thread puts both back in their places. So no node is ever reachable from two threads, and there is no lock
//! and no `unsafe`. A worker with nothing to do spins briefly, then parks (a futex: no lock); the audio thread
//! unparks it after giving it a job. The audio thread runs nodes itself while workers are busy, and never sleeps.
//!
//! Which thread runs a node never changes the result: each node is computed alone, and the sums that read the
//! nodes' outputs happen afterwards on the audio thread in the program's fixed order (base item 9)

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread::{JoinHandle, Thread};

use lm_base::ring::{Consumer, Producer, PushError, ring};

use crate::ids::ProgramGen;
use crate::install::NodeCore;
use crate::panic::{RenderThreadGuard, catching};
use crate::processor::{Buffers, ProcessContext, TransportInfo};
use crate::program::NodeIo;
use crate::spec::{RenderThreads, ThreadKind};

/// One node's call, moving between threads
pub(crate) struct Job {
    pub(crate) node: u32,
    pub(crate) slot: u32,
    pub(crate) core: Box<NodeCore>,
    pub(crate) io: Box<NodeIo>,
    pub(crate) frames: usize,
    pub(crate) transport: TransportInfo,
    pub(crate) panic: Option<Box<dyn Any + Send>>,
    pub(crate) non_finite: bool,
    /// How long the call took, in nanoseconds (to decide whether the node is worth a worker)
    pub(crate) cost_ns: f32,
}

/// The peak absolute value of a block and whether every sample is finite, eight lanes at a time (one sequential
/// reduction would not vectorize: this scan runs on every node's output every block, U20)
pub(crate) fn scan(x: &[f32]) -> (f32, bool) {
    let mut peak = [0.0f32; 8];
    let mut zero = [0.0f32; 8];
    let (chunks, rest) = x.as_chunks::<8>();
    for c in chunks {
        for i in 0..8 {
            peak[i] = peak[i].max(c[i].abs());
            // A finite sample times zero is zero; NaN and infinity give NaN
            zero[i] += c[i] * 0.0;
        }
    }
    let mut p = peak.iter().fold(0.0f32, |a, b| a.max(*b));
    let mut z: f32 = zero.iter().sum();
    for v in rest {
        p = p.max(v.abs());
        z += v * 0.0;
    }
    (p, z == 0.0)
}

/// What a watchdog reads (U21): the node each render thread is in (slot + 1, 0 for none), and the blocks rendered
#[derive(Debug)]
pub struct Watch {
    pub(crate) current: Arc<[AtomicU32]>,
    pub(crate) periods: AtomicU64,
}

impl Watch {
    pub(crate) fn new(threads: usize) -> Self {
        Self { current: (0..threads).map(|_| AtomicU32::new(0)).collect(), periods: AtomicU64::new(0) }
    }

    /// The raw table (slot + 1, 0 for none; audio thread first): shared with the native-crash handler, which reads
    /// it inside a signal handler
    pub fn table(&self) -> Arc<[AtomicU32]> {
        self.current.clone()
    }

    /// The table slot each render thread (audio first, then the workers) is processing right now
    pub fn current_nodes(&self) -> impl Iterator<Item = Option<u32>> + '_ {
        self.current.iter().map(|a| a.load(Ordering::Relaxed).checked_sub(1))
    }

    /// Blocks rendered so far; a watchdog that sees it stop while the device runs knows the callback is stuck
    pub fn periods(&self) -> u64 {
        self.periods.load(Ordering::Relaxed)
    }
}

/// Zeroes what the host zeroes (U23), runs the processor inside `catch_unwind`, and checks the output for NaN and
/// infinity (U20). A panicking or non-finite node leaves silence
pub(crate) fn run_job(job: &mut Job, watch: &AtomicU32) {
    let layout = job.io.layout;
    let frames = job.frames;
    if !layout.in_place {
        for ch in 0..layout.outputs {
            let o = layout.output_offset(ch);
            job.io.buffer[o..o + frames].fill(0.0);
        }
    }
    let started = std::time::Instant::now();
    watch.store(job.slot + 1, Ordering::Relaxed);
    let NodeIo { buffer, events, .. } = &mut *job.io;
    let core = &mut *job.core;
    let result = catching(AssertUnwindSafe(|| {
        let mut cx = ProcessContext {
            frames,
            buffers: Buffers::new(buffer, layout, frames),
            events: events.sorted(),
            transport: &job.transport,
            rng: &mut core.rng,
            host: &mut core.outbox,
            display: core.display.as_mut(),
        };
        core.processor.process(&mut cx);
    }));
    watch.store(0, Ordering::Relaxed);
    job.cost_ns = started.elapsed().as_nanos() as f32;
    let silence = match result {
        Ok(()) => {
            job.non_finite = (0..layout.outputs).any(|ch| {
                let o = layout.output_offset(ch);
                !scan(&job.io.buffer[o..o + frames]).1
            });
            job.non_finite
        }
        Err(payload) => {
            job.panic = Some(payload);
            true
        }
    };
    if silence {
        for ch in 0..layout.outputs {
            let o = layout.output_offset(ch);
            job.io.buffer[o..o + frames].fill(0.0);
        }
    }
}

struct Link {
    jobs: Producer<Job>,
    done: Consumer<Job>,
    thread: Thread,
    busy: bool,
    /// The audio thread stopped waiting for this worker's job (R-8): the worker is not used again until the job
    /// comes back, if it ever does
    lost: bool,
    /// The program node and the table slot of the job the worker holds, and the program it belongs to
    node: u32,
    slot: u32,
    generation: ProgramGen,
}

/// The worker threads and the audio thread's ends of their rings
pub(crate) struct Workers {
    links: Box<[Link]>,
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Workers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workers").field("count", &self.links.len()).finish()
    }
}

/// Spins before a worker parks (a few microseconds), so back-to-back levels do not pay a wake-up each
const SPIN_BEFORE_PARK: u32 = 4096;

impl Workers {
    pub(crate) fn spawn(count: usize, threads: &RenderThreads, watch: &Arc<Watch>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let mut links = Vec::with_capacity(count);
        let mut handles = Vec::with_capacity(count);
        for i in 0..count {
            // Each worker holds at most one job and gives it back before taking another: rings of 2 never fill
            let (jtx, mut jrx) = ring::<Job>(2);
            let (mut dtx, drx) = ring::<Job>(2);
            let stop = stop.clone();
            let watch = watch.clone();
            let threads = threads.clone();
            let handle = std::thread::Builder::new()
                .name(format!("lm-render-{i}"))
                .spawn(move || {
                    lm_base::set_role(lm_base::Role::Audio);
                    let _render = RenderThreadGuard::enter();
                    // H6: for the worker's whole life
                    let _flush = lm_sys_fpu::FlushDenormals::enable();
                    threads.run(ThreadKind::Worker(i), &mut || {
                        let slot = &watch.current[i + 1];
                        let mut idle = 0u32;
                        loop {
                            if let Ok(mut job) = jrx.pop() {
                                run_job(&mut job, slot);
                                while let Err(PushError::Full(back)) = dtx.push(job) {
                                    job = back;
                                    std::hint::spin_loop();
                                }
                                idle = 0;
                                continue;
                            }
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            if idle < SPIN_BEFORE_PARK {
                                idle += 1;
                                std::hint::spin_loop();
                            } else {
                                std::thread::park();
                            }
                        }
                    });
                })
                .expect("spawning a render worker");
            links.push(Link {
                jobs: jtx,
                done: drx,
                thread: handle.thread().clone(),
                busy: false,
                lost: false,
                node: 0,
                slot: 0,
                generation: ProgramGen(0),
            });
            handles.push(handle);
        }
        Self { links: links.into_boxed_slice(), stop, handles }
    }

    pub(crate) fn count(&self) -> usize {
        self.links.len()
    }

    /// A worker with nothing to do, if any
    pub(crate) fn idle(&self) -> Option<usize> {
        self.links.iter().position(|l| !l.busy && !l.lost)
    }

    pub(crate) fn any_busy(&self) -> bool {
        self.links.iter().any(|l| l.busy)
    }

    /// Gives a job to an idle worker (the ring has room: the worker has none)
    pub(crate) fn give(&mut self, worker: usize, job: Job, generation: ProgramGen) {
        let link = &mut self.links[worker];
        debug_assert!(!link.busy && !link.lost);
        (link.node, link.slot, link.generation) = (job.node, job.slot, generation);
        if let Err(PushError::Full(job)) = link.jobs.push(job) {
            // Impossible (an idle worker's ring is empty); never drop a processor on the audio thread
            std::mem::forget(job);
            return;
        }
        link.busy = true;
        link.thread.unpark();
    }

    /// Stops waiting for every busy worker: each is marked lost and its job (node, slot, program) is reported
    /// to `each`. The jobs themselves are in the workers' hands and come back through `take_late` if they ever do
    pub(crate) fn abandon_busy(&mut self, mut each: impl FnMut(u32, u32, ProgramGen)) {
        for link in self.links.iter_mut().filter(|l| l.busy) {
            link.busy = false;
            link.lost = true;
            each(link.node, link.slot, link.generation);
        }
    }

    /// A job that came back after its worker was abandoned, with the program it was given under
    pub(crate) fn take_late(&mut self) -> Option<(Job, ProgramGen)> {
        for link in self.links.iter_mut() {
            if link.lost
                && let Ok(job) = link.done.pop()
            {
                link.lost = false;
                return Some((job, link.generation));
            }
        }
        None
    }

    /// A finished job, if any worker has one
    pub(crate) fn take_done(&mut self) -> Option<Job> {
        for link in self.links.iter_mut() {
            if link.busy
                && let Ok(job) = link.done.pop()
            {
                link.busy = false;
                return Some(job);
            }
        }
        None
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for link in self.links.iter() {
            link.thread.unpark();
        }
        for handle in self.handles.drain(..) {
            // A worker only panics if its hook does; the renderer is being dropped either way
            let _ = handle.join();
        }
    }
}
