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

use crate::layout::{
    EVENT_WORDS, Layout, OUT_BEGIN, OUT_END, OUT_INDEX, OUT_WORDS, clamp_latency, encode, get, put, scrub, w,
};

/// Blocks missed in a row after which the plugin counts as hung
pub const HANG_BLOCKS: u32 = 16;

/// The longest a block may take when there is no deadline (export): a helper alive but stuck is then counted as a
/// missed block, so the plugin is declared hung after [`HANG_BLOCKS`] of them and the export goes on without it
pub const WAIT_LIMIT: Duration = Duration::from_secs(10);

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
        let latency = clamp_latency(latency);
        let seq = shm.words()[w::DONE].load(Ordering::Acquire);
        Proxy { shm, layout, state, deadline, rate, seq, latency, reported: false, _owner: owner }
    }

    pub fn state(&self) -> &Arc<ProxyState> {
        &self.state
    }

    /// The helper this proxy runs (the session keeps it per node, for the state and the window)
    pub fn hosted(&self) -> Option<Arc<crate::hosted::Hosted>> {
        Arc::clone(&self._owner).downcast::<crate::hosted::Hosted>().ok()
    }

    /// The export waits for every block (S19); a live proxy has a deadline
    pub fn set_deadline(&mut self, deadline: Deadline) {
        self.deadline = deadline;
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
        // A block with more events than the shared block holds is run as consecutive pieces, each cut where the
        // events that do not fit begin, so none is dropped (a lost note-off would leave a note hanging). The
        // helper sees the pieces as ordinary blocks, with the transport moved on by the frames already done
        let frames = cx.frames.min(self.layout.max_block);
        let max = self.layout.max_events;
        let (mut start, mut next) = (0usize, 0usize);
        loop {
            let (end, upto) = piece(cx.events, next, start, frames, max);
            if !self.block(cx, start, end, next, upto) {
                return;
            }
            start = end;
            next = upto;
            // Events left at or past the block's end are not played
            if start >= frames {
                break;
            }
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

/// Where the piece of a block that starts at frame `start` and takes events from `next` ends: `(end frame, end
/// event index)`. The events are in time order. A piece holds at most `max` events and ends at the offset of the
/// first one it leaves out; at least one frame long (more than `max` events at one offset go on in the next piece,
/// one frame late, which is better than lost). The last piece ends at `frames`
fn piece(events: &[lm_render::Event], next: usize, start: usize, frames: usize, max: usize) -> (usize, usize) {
    let left = events.len() - next;
    // Nothing left to cut by (events at or past the block's end are not the plugin's to play): the rest is one piece
    if left <= max || start + 1 > frames {
        return (frames, events.len());
    }
    let cut = (events[next + max].offset as usize).clamp(start + 1, frames.max(start + 1));
    let mut upto = next;
    while upto < events.len() && upto - next < max && (events[upto].offset as usize) < cut {
        upto += 1;
    }
    (cut, upto)
}

impl Proxy {
    /// One round trip with the helper: frames `start..end` of the block and `cx.events[first..last]`, the offsets
    /// counted from `start`. `false`: the block was missed (silence for all of it) or the helper is gone
    fn block(&mut self, cx: &mut ProcessContext<'_>, start: usize, end: usize, first: usize, last: usize) -> bool {
        let words = self.shm.words();
        let l = self.layout;
        let frames = end - start;
        {
            let (ins, mut outs, side) = cx.buffers.split();
            if ins.is_empty() {
                // In place: the outputs hold the input
                for ch in 0..l.inputs.min(outs.len()) {
                    put(words, l.input(ch), &outs.get_mut(ch)[start..end]);
                }
            } else {
                for ch in 0..l.inputs.min(ins.len()) {
                    put(words, l.input(ch), &ins.get(ch)[start..end]);
                }
            }
            for ch in 0..l.sidechain.min(side.len()) {
                put(words, l.sidechain(ch), &side.get(ch)[start..end]);
            }
        }
        let mut n = 0usize;
        let base = l.events();
        for e in cx.events[first..last].iter().take(l.max_events) {
            if let Some(x) = encode((e.offset as usize).saturating_sub(start) as u32, &e.kind) {
                for (k, v) in x.iter().enumerate() {
                    words[base + n * EVENT_WORDS + k].store(*v, Ordering::Relaxed);
                }
                n += 1;
            }
        }
        let t = cx.transport;
        // The transport at the piece's first frame
        let (position, tick) = if start == 0 {
            (t.position, t.tick)
        } else {
            let advance = if t.playing { start as i64 } else { 0 };
            let ticks = if t.playing && t.sample_rate > 0 {
                (start as f64 / f64::from(t.sample_rate) * t.bpm / 60.0 * f64::from(t.ppq)).round() as i64
            } else {
                0
            };
            (t.position.saturating_add(advance), t.tick.saturating_add(ticks))
        };
        words[w::FRAMES].store(frames as u32, Ordering::Relaxed);
        words[w::EVENTS].store(n as u32, Ordering::Relaxed);
        words[w::FLAGS].store(u32::from(t.playing) | (u32::from(t.recording) << 1), Ordering::Relaxed);
        words[w::POSITION_LO].store(position as u32, Ordering::Relaxed);
        words[w::POSITION_HI].store((position >> 32) as u32, Ordering::Relaxed);
        words[w::TICK_LO].store(tick as u32, Ordering::Relaxed);
        words[w::TICK_HI].store((tick >> 32) as u32, Ordering::Relaxed);
        words[w::BPM].store((t.bpm as f32).to_bits(), Ordering::Relaxed);
        words[w::PPQ].store(t.ppq, Ordering::Relaxed);
        let sig = (u32::from(t.time_signature.numerator()) << 8) | u32::from(t.time_signature.denominator());
        words[w::SIGNATURE].store(sig, Ordering::Relaxed);
        // Hand it over
        self.seq = self.seq.wrapping_add(1);
        words[w::REQUEST].store(self.seq, Ordering::Release);
        lm_sys_ipc::wake(&words[w::REQUEST], 1);
        // Wait for it, bounded by the deadline
        let started = Instant::now();
        let budget = match self.deadline {
            Deadline::Live(f) => {
                // `from_secs_f64` panics on a negative or non-finite number
                let secs = frames as f64 / f64::from(self.rate.max(1)) * f;
                Some(Duration::from_secs_f64(if secs.is_finite() { secs.clamp(0.0, 10.0) } else { 0.0 }))
            }
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
                return false;
            }
            // A short spin first: a fast plugin answers within microseconds, cheaper than sleeping
            if spins < 64 {
                spins += 1;
                std::hint::spin_loop();
                continue;
            }
            match budget {
                Some(b) => {
                    let spent = started.elapsed();
                    if spent >= b {
                        self.miss(cx);
                        return false;
                    }
                    lm_sys_ipc::wait(&words[w::DONE], done, Some(b - spent));
                }
                None => {
                    if started.elapsed() >= WAIT_LIMIT {
                        self.miss(cx);
                        return false;
                    }
                    lm_sys_ipc::wait(&words[w::DONE], done, Some(Duration::from_millis(20)));
                }
            }
        }
        self.state.misses.store(0, Ordering::Relaxed);
        {
            let (_, mut outs, _) = cx.buffers.split();
            for ch in 0..l.outputs.min(outs.len()) {
                let out = &mut outs.get_mut(ch)[start..end];
                get(words, l.output(ch), out);
                scrub(out);
            }
            for ch in l.outputs..outs.len() {
                outs.get_mut(ch)[start..end].fill(0.0);
            }
        }
        // What the plugin says back: its latency, its own parameter changes (its window, a preset)
        let latency = clamp_latency(words[w::LATENCY].load(Ordering::Acquire));
        if latency != self.latency {
            self.latency = latency;
            cx.host.request(PluginRequest::LatencyChanged);
        }
        let outs = (words[w::OUT_EVENTS].load(Ordering::Acquire) as usize).min(l.max_out);
        let at = l.out_events();
        // The gesture edges first (the outbox holds few requests per block: a begin or an end must never be the one
        // dropped), then the values
        for edges in [true, false] {
            for k in 0..outs {
                let index = words[at + k * OUT_WORDS].load(Ordering::Relaxed);
                let value = f32::from_bits(words[at + k * OUT_WORDS + 1].load(Ordering::Relaxed));
                let request = if index & OUT_BEGIN != 0 {
                    PluginRequest::GestureBegin { index: index & OUT_INDEX }
                } else if index & OUT_END != 0 {
                    PluginRequest::GestureEnd { index: index & OUT_INDEX }
                } else if value.is_finite() {
                    PluginRequest::ParamChanged { index, value }
                } else {
                    // A value that is not a number would fail the session's whole batch of parameters
                    continue;
                };
                let is_edge = !matches!(request, PluginRequest::ParamChanged { .. });
                if is_edge == edges {
                    cx.host.request(request);
                }
            }
        }
        // Its own state changed: the session asks the helper for it, outside the audio (I-9.4)
        if words[w::STATE].swap(0, Ordering::AcqRel) != 0 {
            cx.host.request(PluginRequest::StateChanged);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lm_render::{Event, EventKind};

    fn at(offsets: impl IntoIterator<Item = u32>) -> Vec<Event> {
        offsets.into_iter().map(|offset| Event { offset, kind: EventKind::AllNotesRelease }).collect()
    }

    /// Runs `piece` over the whole block and returns the pieces `(start, end, first, last)`
    fn pieces(events: &[Event], frames: usize, max: usize) -> Vec<(usize, usize, usize, usize)> {
        let (mut start, mut next, mut out) = (0, 0, Vec::new());
        loop {
            let (end, upto) = piece(events, next, start, frames, max);
            out.push((start, end, next, upto));
            start = end;
            next = upto;
            if start >= frames {
                return out;
            }
            assert!(out.len() < 10_000, "no progress");
        }
    }

    #[test]
    fn a_block_with_few_events_is_one_piece() {
        let e = at([0, 5, 9]);
        assert_eq!(pieces(&e, 64, 4096), [(0, 64, 0, 3)]);
        assert_eq!(pieces(&[], 64, 4096), [(0, 64, 0, 0)]);
    }

    #[test]
    fn too_many_events_are_split_over_pieces_and_none_is_lost() {
        // 10 events a frame apart, room for 4 per piece
        let e = at(0..10);
        let p = pieces(&e, 16, 4);
        let mut seen = 0;
        let mut at_frame = 0;
        for (start, end, first, last) in &p {
            assert_eq!(*start, at_frame, "pieces follow each other");
            assert!(end > start && last - first <= 4);
            // Every event of a piece falls inside it
            assert!(e[*first..*last].iter().all(|x| (*start..*end).contains(&(x.offset as usize))));
            seen += last - first;
            at_frame = *end;
        }
        assert_eq!(seen, 10);
        assert_eq!(at_frame, 16);
        assert!(p.len() >= 3);
    }

    #[test]
    fn many_events_at_one_offset_move_on_instead_of_getting_lost() {
        let e = at(std::iter::repeat_n(3, 20));
        let p = pieces(&e, 8, 8);
        assert_eq!(p.iter().map(|(_, _, f, l)| l - f).sum::<usize>(), 20);
        assert!(p.iter().all(|(s, e, f, l)| e > s && l - f <= 8));
    }

    #[test]
    fn events_past_the_end_do_not_panic() {
        let e = at(std::iter::repeat_n(100, 10));
        let p = pieces(&e, 4, 3);
        assert!(p.iter().all(|(_, end, _, _)| *end <= 4));
    }
}
