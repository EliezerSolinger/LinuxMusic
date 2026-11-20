//! The proxy (I-9.6): the processor the core's renderer runs for a third-party plugin. It hands each block to the
//! helper through the shared block and waits for it until the deadline, never longer: a block that misses it plays
//! silence for this node only (the rest of the mix goes on), and after [`HANG_BLOCKS`] in a row the plugin is
//! declared hung — the supervisor kills the helper and the session offers to recreate it. A helper that crashed
//! leaves the node silent the same way. No allocation and no lock: atomics, the wake (`futex`) and a wait bounded by
//! the deadline.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use lm_render::{Adopted, PluginErrorKey, PluginRequest, ProcessContext, Processor};
use lm_sys_ipc::SharedMemory;

use crate::layout::{EVENT_WORDS, Layout, OUT_WORDS, encode, get, put, w};

/// Blocks missed in a row after which the plugin counts as hung
pub const HANG_BLOCKS: u32 = 16;

/// How long a block may take
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deadline {
    /// Live: this fraction of the block's duration (the rest is the core's)
    Live(f64),
    /// Export: no deadline (S19), so the file is the same bit for bit
    Wait,
}

impl Default for Deadline {
    fn default() -> Self {
        Deadline::Live(0.6)
    }
}

/// What the proxy and the supervisor share
#[derive(Debug, Default)]
pub struct ProxyState {
    /// The helper is gone (crashed, killed or ended)
    pub dead: AtomicBool,
    /// Blocks missed in a row
    pub misses: AtomicU32,
    /// Blocks missed since the start (for the notice and the tests)
    pub missed_total: AtomicU32,
    /// The proxy declared the plugin hung (the supervisor kills it)
    pub hung: AtomicBool,
}

/// The core's side of one plugin instance
pub struct Proxy {
    shm: Arc<SharedMemory>,
    layout: Layout,
    state: Arc<ProxyState>,
    deadline: Deadline,
    rate: u32,
    seq: u32,
    latency: u32,
    reported: bool,
    /// Keeps the helper (and its supervisor) alive while the processor lives; dropped on the main thread
    _owner: Arc<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy").field("layout", &self.layout).field("latency", &self.latency).finish_non_exhaustive()
    }
}

impl Proxy {
    pub fn new(
        shm: Arc<SharedMemory>,
        layout: Layout,
        state: Arc<ProxyState>,
        deadline: Deadline,
        rate: u32,
        latency: u32,
        owner: Arc<dyn Any + Send + Sync>,
    ) -> Proxy {
        let seq = shm.words()[w::DONE].load(Ordering::Acquire);
        Proxy { shm, layout, state, deadline, rate, seq, latency, reported: false, _owner: owner }
    }

    pub fn state(&self) -> &Arc<ProxyState> {
        &self.state
    }

    fn silence(cx: &mut ProcessContext<'_>) {
        let n = cx.frames;
        for ch in 0..cx.buffers.outputs() {
            cx.buffers.output(ch)[..n].fill(0.0);
        }
    }

    fn report(&mut self, cx: &mut ProcessContext<'_>, key: &'static str) {
        if !self.reported {
            self.reported = cx.host.request(PluginRequest::Error(PluginErrorKey(key)));
        }
    }

    fn miss(&mut self, cx: &mut ProcessContext<'_>) {
        Self::silence(cx);
        self.state.missed_total.fetch_add(1, Ordering::Relaxed);
        let m = self.state.misses.fetch_add(1, Ordering::Relaxed) + 1;
        if m >= HANG_BLOCKS && !self.state.hung.swap(true, Ordering::AcqRel) {
            self.report(cx, "plugin_host.hung");
        }
    }
}

impl Processor for Proxy {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let words = self.shm.words();
        if self.state.dead.load(Ordering::Acquire) {
            Self::silence(cx);
            self.report(cx, "plugin_host.crashed");
            return;
        }
        if self.state.hung.load(Ordering::Acquire) {
            Self::silence(cx);
            return;
        }
        // The helper still busy with an older block: this one is silence, not queued
        if words[w::DONE].load(Ordering::Acquire) != self.seq {
            self.miss(cx);
            return;
        }
        let frames = cx.frames.min(self.layout.max_block);
        let l = self.layout;
        {
            let (ins, mut outs, side) = cx.buffers.split();
            if ins.is_empty() {
                // In place: the outputs hold the input
                for ch in 0..l.inputs.min(outs.len()) {
                    put(words, l.input(ch), &outs.get_mut(ch)[..frames]);
                }
            } else {
                for ch in 0..l.inputs.min(ins.len()) {
                    put(words, l.input(ch), &ins.get(ch)[..frames]);
                }
            }
            for ch in 0..l.sidechain.min(side.len()) {
                put(words, l.sidechain(ch), &side.get(ch)[..frames]);
            }
        }
        let mut n = 0usize;
        let base = l.events();
        for e in cx.events.iter().take(l.max_events) {
            if let Some(x) = encode(e.offset, &e.kind) {
                for (k, v) in x.iter().enumerate() {
                    words[base + n * EVENT_WORDS + k].store(*v, Ordering::Relaxed);
                }
                n += 1;
            }
        }
        let t = cx.transport;
        words[w::FRAMES].store(frames as u32, Ordering::Relaxed);
        words[w::EVENTS].store(n as u32, Ordering::Relaxed);
        words[w::FLAGS].store(u32::from(t.playing) | (u32::from(t.recording) << 1), Ordering::Relaxed);
        words[w::POSITION_LO].store(t.position as u32, Ordering::Relaxed);
        words[w::POSITION_HI].store((t.position >> 32) as u32, Ordering::Relaxed);
        words[w::TICK_LO].store(t.tick as u32, Ordering::Relaxed);
        words[w::TICK_HI].store((t.tick >> 32) as u32, Ordering::Relaxed);
        words[w::BPM].store((t.bpm as f32).to_bits(), Ordering::Relaxed);
        // Hand it over
        self.seq = self.seq.wrapping_add(1);
        words[w::REQUEST].store(self.seq, Ordering::Release);
        lm_sys_ipc::wake(&words[w::REQUEST], 1);
        // Wait for it, bounded by the deadline
        let start = Instant::now();
        let budget = match self.deadline {
            Deadline::Live(f) => Some(Duration::from_secs_f64(frames as f64 / f64::from(self.rate.max(1)) * f)),
            Deadline::Wait => None,
        };
        let mut spins = 0u32;
        loop {
            let done = words[w::DONE].load(Ordering::Acquire);
            if done == self.seq {
                break;
            }
            if self.state.dead.load(Ordering::Acquire) {
                Self::silence(cx);
                return;
            }
            // A short spin first: a fast plugin answers within microseconds, cheaper than sleeping
            if spins < 64 {
                spins += 1;
                std::hint::spin_loop();
                continue;
            }
            match budget {
                Some(b) => {
                    let spent = start.elapsed();
                    if spent >= b {
                        self.miss(cx);
                        return;
                    }
                    lm_sys_ipc::wait(&words[w::DONE], done, Some(b - spent));
                }
                None => lm_sys_ipc::wait(&words[w::DONE], done, Some(Duration::from_millis(20))),
            }
        }
        self.state.misses.store(0, Ordering::Relaxed);
        {
            let (_, mut outs, _) = cx.buffers.split();
            for ch in 0..l.outputs.min(outs.len()) {
                get(words, l.output(ch), &mut outs.get_mut(ch)[..frames]);
            }
            for ch in l.outputs..outs.len() {
                outs.get_mut(ch)[..frames].fill(0.0);
            }
        }
        // What the plugin says back: its latency, its own parameter changes (its window, a preset)
        let latency = words[w::LATENCY].load(Ordering::Acquire);
        if latency != self.latency {
            self.latency = latency;
            cx.host.request(PluginRequest::LatencyChanged);
        }
        let outs = (words[w::OUT_EVENTS].load(Ordering::Acquire) as usize).min(l.max_out);
        let at = l.out_events();
        for k in 0..outs {
            let index = words[at + k * OUT_WORDS].load(Ordering::Relaxed);
            let value = f32::from_bits(words[at + k * OUT_WORDS + 1].load(Ordering::Relaxed));
            cx.host.request(PluginRequest::ParamChanged { index, value });
        }
    }

    fn reset(&mut self) {
        // The panic command reaches the plugin as an event in its next block (AllSoundOff)
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn adopt(&mut self, _old: &mut dyn Processor) -> Adopted {
        // A third-party plugin's voices cannot move between instances (U32)
        Adopted::No
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
