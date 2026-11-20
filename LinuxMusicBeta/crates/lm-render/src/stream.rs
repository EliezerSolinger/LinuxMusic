//! Audio streamed from the disk (P-32: items above 30 s are not held in memory). A disk thread (`lm-io`'s
//! `DiskReader`) fills a fixed ring of chunks ahead of the playhead; the audio thread only reads, never waits, never
//! allocates. A chunk that is not there when it is needed plays as exact silence and is counted as an underrun
//! (reported by the renderer as `Telemetry::StreamUnderrun`).
//!
//! # The ring
//!
//! The source is cut in chunks of `chunk_frames` frames (chunk `c` is frames `c·chunk .. (c+1)·chunk`, the last one
//! shorter). Two regions of slots, each a direct map by chunk number:
//! - **ahead**: the chunks from the one under the playhead on, `ahead_chunks` of them;
//! - **cue**: the chunks from the loop's start on, `cue_chunks` of them, so a loop wraps without waiting for the
//!   disk.
//!
//! Each slot is a seqlock over atomics (no `unsafe`, no lock): a tag (`0` empty, odd while written, `2·(c+1)` when
//! it holds chunk `c`) and the samples as `f32` bits, planar. The writer only ever overwrites a slot whose chunk is
//! out of the window it fills, and the reader checks the tag before and after copying, so a torn read is never
//! played (it counts as an underrun).
//!
//! The renderer publishes, after every block, the source frame each streamed item needs next and its loop cue;
//! [`StreamFeeder::next_request`] answers with the nearest missing chunk.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

use lm_base::{Error, Result};

const NONE: u64 = u64::MAX;

/// How a stream is cut and how much it keeps ahead
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSpec {
    /// The project's rate (the source is converted to it by the reader)
    pub rate: u32,
    pub channels: usize,
    /// The source's length in frames
    pub frames: u64,
    pub chunk_frames: usize,
    /// Chunks kept from the playhead on
    pub ahead_chunks: usize,
    /// Chunks kept from the loop's start on
    pub cue_chunks: usize,
}

impl StreamSpec {
    /// Chunks of 8192 frames, about 3 s ahead and 1 s of loop cue
    pub fn new(rate: u32, channels: usize, frames: u64) -> Self {
        let chunk_frames = 8192;
        let per_second = (rate as usize).div_ceil(chunk_frames).max(1);
        Self { rate, channels, frames, chunk_frames, ahead_chunks: 3 * per_second + 1, cue_chunks: per_second + 1 }
    }

    fn chunks(&self) -> u64 {
        self.frames.div_ceil(self.chunk_frames as u64)
    }

    fn chunk_len(&self, c: u64) -> usize {
        let start = c * self.chunk_frames as u64;
        (self.frames.saturating_sub(start)).min(self.chunk_frames as u64) as usize
    }
}

struct Slot {
    tag: AtomicU64,
    /// `channels × chunk_frames`, channel after channel
    data: Box<[AtomicU32]>,
}

struct Region {
    slots: Box<[Slot]>,
}

impl Region {
    fn new(count: usize, samples: usize) -> Self {
        Self {
            slots: (0..count)
                .map(|_| Slot { tag: AtomicU64::new(0), data: (0..samples).map(|_| AtomicU32::new(0)).collect() })
                .collect(),
        }
    }

    fn slot(&self, c: u64) -> Option<&Slot> {
        if self.slots.is_empty() { None } else { Some(&self.slots[(c % self.slots.len() as u64) as usize]) }
    }

    fn holds(&self, c: u64) -> bool {
        self.slot(c).is_some_and(|s| s.tag.load(Ordering::Acquire) == ready_tag(c))
    }
}

fn ready_tag(c: u64) -> u64 {
    2 * (c + 1)
}

struct Shared {
    spec: StreamSpec,
    ahead: Region,
    cue: Region,
    /// The source frame the audio needs next (`NONE`: nothing)
    playhead: AtomicU64,
    /// The source frame where the loop comes back (`NONE`: no loop in the item)
    cue_at: AtomicU64,
    underruns: AtomicU64,
}

impl Shared {
    fn window(&self, at: u64, len: usize) -> std::ops::Range<u64> {
        if at == NONE || at >= self.spec.frames {
            return 0..0;
        }
        let first = at / self.spec.chunk_frames as u64;
        first..(first + len as u64).min(self.spec.chunks())
    }

    fn ahead_window(&self) -> std::ops::Range<u64> {
        self.window(self.playhead.load(Ordering::Acquire), self.ahead.slots.len())
    }

    fn cue_window(&self) -> std::ops::Range<u64> {
        self.window(self.cue_at.load(Ordering::Acquire), self.cue.slots.len())
    }

    fn missing(&self) -> Option<u64> {
        if let Some(c) = self.ahead_window().find(|c| !self.ahead.holds(*c)) {
            return Some(c);
        }
        self.cue_window().find(|c| !self.cue.holds(*c) && !self.ahead.holds(*c))
    }
}

/// The audio side of a stream: goes into a program's audio item ([`crate::AudioSource::Stream`]). Cloning shares
/// the same ring
#[derive(Clone)]
pub struct Stream {
    shared: Arc<Shared>,
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stream").field("spec", &self.shared.spec).finish()
    }
}

impl PartialEq for Stream {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}

/// The disk side of a stream: `Send`, one per stream, served by a disk thread
pub struct StreamFeeder {
    shared: Arc<Shared>,
}

impl fmt::Debug for StreamFeeder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamFeeder").field("spec", &self.shared.spec).finish()
    }
}

/// Makes a stream: all its memory is allocated here. Keys: `render.stream.channels` (1 to 64),
/// `render.stream.chunk` (a chunk of at least one frame), `render.stream.ahead` (at least 2 chunks ahead)
pub fn stream(spec: StreamSpec) -> Result<(Stream, StreamFeeder)> {
    if spec.channels == 0 || spec.channels > 64 {
        return Err(Error::new("render.stream.channels").arg("channels", spec.channels as i64));
    }
    if spec.chunk_frames == 0 {
        return Err(Error::new("render.stream.chunk"));
    }
    if spec.ahead_chunks < 2 {
        return Err(Error::new("render.stream.ahead"));
    }
    let samples = spec.channels * spec.chunk_frames;
    let shared = Arc::new(Shared {
        spec,
        ahead: Region::new(spec.ahead_chunks, samples),
        cue: Region::new(spec.cue_chunks, samples),
        playhead: AtomicU64::new(0),
        cue_at: AtomicU64::new(NONE),
        underruns: AtomicU64::new(0),
    });
    Ok((Stream { shared: shared.clone() }, StreamFeeder { shared }))
}

impl Stream {
    pub fn spec(&self) -> StreamSpec {
        self.shared.spec
    }

    pub fn rate(&self) -> u32 {
        self.shared.spec.rate
    }

    pub fn channels(&self) -> usize {
        self.shared.spec.channels
    }

    pub fn frames(&self) -> u64 {
        self.shared.spec.frames
    }

    /// Where the audio needs data next (`None`: nowhere). The renderer sets it after every block; the compiler sets
    /// the item's first frame when it makes the stream, so the head is read before the item is installed
    pub fn set_playhead(&self, frame: Option<u64>) {
        self.shared.playhead.store(frame.unwrap_or(NONE), Ordering::Release);
    }

    /// The frame where the loop comes back into this item (`None`: no loop through it)
    pub fn set_cue(&self, frame: Option<u64>) {
        self.shared.cue_at.store(frame.unwrap_or(NONE), Ordering::Release);
    }

    /// Something the playhead or the cue needs is not loaded yet (an export waits for this to be false, T8)
    pub fn pending(&self) -> bool {
        self.shared.missing().is_some()
    }

    /// Frames played as silence because they were not loaded, since the stream was made
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// Copies frames `from .. from + n` of every channel into `out` (planar: channel `ch` at `ch × n`), never
    /// waiting. Frames beyond the source are silence; frames not loaded are silence too and are counted and
    /// returned as missing. No allocation
    pub fn read(&self, from: u64, n: usize, out: &mut [f32]) -> usize {
        let spec = &self.shared.spec;
        let channels = spec.channels;
        debug_assert!(out.len() >= channels * n);
        let mut missing = 0usize;
        let mut done = 0usize;
        while done < n {
            let frame = from + done as u64;
            if frame >= spec.frames {
                for ch in 0..channels {
                    out[ch * n + done..ch * n + n].fill(0.0);
                }
                break;
            }
            let c = frame / spec.chunk_frames as u64;
            let in_chunk = (frame - c * spec.chunk_frames as u64) as usize;
            let span = (spec.chunk_len(c) - in_chunk).min(n - done);
            let ok = [&self.shared.ahead, &self.shared.cue]
                .into_iter()
                .any(|region| copy_from(region, c, in_chunk, span, channels, spec.chunk_frames, out, n, done));
            if !ok {
                for ch in 0..channels {
                    out[ch * n + done..ch * n + done + span].fill(0.0);
                }
                missing += span;
            }
            done += span;
        }
        if missing > 0 {
            self.shared.underruns.fetch_add(missing as u64, Ordering::Relaxed);
        }
        missing
    }
}

/// One seqlock read of a chunk's span; `false` (and nothing usable written) if the slot does not hold it whole
#[allow(clippy::too_many_arguments)]
fn copy_from(
    region: &Region,
    c: u64,
    in_chunk: usize,
    span: usize,
    channels: usize,
    chunk: usize,
    out: &mut [f32],
    n: usize,
    at: usize,
) -> bool {
    let Some(slot) = region.slot(c) else { return false };
    let want = ready_tag(c);
    if slot.tag.load(Ordering::Acquire) != want {
        return false;
    }
    for ch in 0..channels {
        let src = &slot.data[ch * chunk + in_chunk..ch * chunk + in_chunk + span];
        for (d, s) in out[ch * n + at..ch * n + at + span].iter_mut().zip(src) {
            *d = f32::from_bits(s.load(Ordering::Relaxed));
        }
    }
    fence(Ordering::Acquire);
    slot.tag.load(Ordering::Relaxed) == want
}

impl StreamFeeder {
    pub fn spec(&self) -> StreamSpec {
        self.shared.spec
    }

    pub fn chunk_frames(&self) -> usize {
        self.shared.spec.chunk_frames
    }

    pub fn channels(&self) -> usize {
        self.shared.spec.channels
    }

    pub fn frames(&self) -> u64 {
        self.shared.spec.frames
    }

    pub fn rate(&self) -> u32 {
        self.shared.spec.rate
    }

    /// The chunk to read next: the nearest one from the playhead that is missing, then the loop cue's
    pub fn next_request(&self) -> Option<u64> {
        self.shared.missing()
    }

    /// Something wanted is missing
    pub fn pending(&self) -> bool {
        self.shared.missing().is_some()
    }

    /// The frames of chunk `c` (planar: channel after channel, `len` frames each, `len` the chunk's length — the
    /// last chunk may be shorter). Stored where the playhead's window or the cue's wants it; a chunk neither wants
    /// any more (the playhead moved on) is dropped. Never blocks. Key `render.stream.chunk_size` when `planar` is
    /// not `channels × len`
    pub fn write(&self, c: u64, planar: &[f32]) -> Result<bool> {
        let s = &self.shared;
        let len = s.spec.chunk_len(c);
        if len == 0 || planar.len() != s.spec.channels * len {
            return Err(Error::new("render.stream.chunk_size").arg("chunk", c as i64));
        }
        let mut stored = false;
        if s.ahead_window().contains(&c) {
            fill(&s.ahead, c, planar, len, s.spec.chunk_frames, s.spec.channels);
            stored = true;
        }
        if s.cue_window().contains(&c) {
            fill(&s.cue, c, planar, len, s.spec.chunk_frames, s.spec.channels);
            stored = true;
        }
        Ok(stored)
    }

    /// The program side is gone (every [`Stream`] clone dropped): the reader can drop this feeder
    pub fn is_abandoned(&self) -> bool {
        Arc::strong_count(&self.shared) == 1
    }
}

fn fill(region: &Region, c: u64, planar: &[f32], len: usize, chunk: usize, channels: usize) {
    let Some(slot) = region.slot(c) else { return };
    slot.tag.store(1, Ordering::Relaxed);
    fence(Ordering::Release);
    for ch in 0..channels {
        for (d, s) in slot.data[ch * chunk..ch * chunk + len].iter().zip(&planar[ch * len..(ch + 1) * len]) {
            d.store(s.to_bits(), Ordering::Relaxed);
        }
    }
    slot.tag.store(ready_tag(c), Ordering::Release);
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    fn spec() -> StreamSpec {
        StreamSpec { rate: 48_000, channels: 2, frames: 10_000, chunk_frames: 1000, ahead_chunks: 3, cue_chunks: 2 }
    }

    fn chunk(f: &StreamFeeder, c: u64) -> Vec<f32> {
        let len = f.shared.spec.chunk_len(c);
        let mut v = Vec::with_capacity(2 * len);
        for ch in 0..2 {
            for i in 0..len {
                v.push((c as usize * 1000 + i) as f32 + ch as f32 * 0.5);
            }
        }
        v
    }

    #[test]
    fn reads_what_was_written_and_counts_the_rest() {
        let (s, f) = stream(spec()).unwrap();
        s.set_playhead(Some(1500));
        assert_eq!(f.next_request(), Some(1));
        assert!(f.write(1, &chunk(&f, 1)).unwrap());
        assert_eq!(f.next_request(), Some(2));
        let mut out = vec![9.0; 2 * 600];
        // 1500..2100: 500 loaded, 100 missing (chunk 2)
        assert_eq!(s.read(1500, 600, &mut out), 100);
        assert_eq!(out[0], 1500.0);
        assert_eq!(out[600], 1500.5);
        assert_eq!(out[499], 1999.0);
        assert!(out[500..600].iter().all(|x| *x == 0.0));
        assert_eq!(s.underruns(), 100);
        // Out of the window: dropped
        assert!(!f.write(7, &chunk(&f, 7)).unwrap());
        // The cue keeps the loop's start
        s.set_cue(Some(0));
        f.write(2, &chunk(&f, 2)).unwrap();
        f.write(3, &chunk(&f, 3)).unwrap();
        assert_eq!(f.next_request(), Some(0));
        f.write(0, &chunk(&f, 0)).unwrap();
        assert_eq!(f.next_request(), None);
        assert!(!s.pending());
        assert_eq!(s.read(0, 600, &mut out), 0);
        assert_eq!(out[1], 1.0);
        // Past the end: silence, not an underrun
        s.set_playhead(Some(9800));
        f.write(9, &chunk(&f, 9)).unwrap();
        assert_eq!(s.read(9800, 600, &mut out), 0);
        assert_eq!(out[199], 9999.0);
        assert!(out[200..600].iter().all(|x| *x == 0.0));
        drop(s);
        assert!(f.is_abandoned());
    }
}
