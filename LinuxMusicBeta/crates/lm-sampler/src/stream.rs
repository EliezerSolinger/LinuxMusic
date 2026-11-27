//! Streaming of long samples (the design of decisoes-autonomas.md, "Sampler: streaming").
//!
//! A streamed [`crate::Pcm`] keeps only its *head* (the first [`HEAD_FRAMES`] frames, the time the disk takes to answer)
//! in memory and says where the rest is ([`TailSource`]). A voice that plays past the head reads the rest through a
//! [`Ring`] of [`BLOCK`]-frame blocks the engine keeps for it: the engine asks for the blocks ahead of the voice
//! ([`Request`]) on a lock-free queue and a thread of the plugin ([`Feeder`]) reads them from the file and answers
//! ([`Response`]) in buffers the audio side gave, so nothing is allocated, locked or read from disk on the audio
//! thread. A block that has not arrived when the voice needs it is a *miss*: that frame is silence and the engine
//! counts it ([`crate::Engine::stream_misses`]) instead of waiting.
//!
//! Without a [`Link`] the engine reads the blocks itself, in the call: slow but exact, for an offline render and the
//! tests (a miss cannot happen).
//!
//! What streams: a sample whose frames are raw PCM in a file (an SF2's 16-bit `smpl` chunk, a GIG's 16 and 24-bit
//! waves). A compressed sample (SF3, Ogg) and the files the host decodes (a single audio file) stay in memory.

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::Arc;

use rtrb::{Consumer, Producer, RingBuffer};

/// Frames of one block
pub const BLOCK: usize = 8192;
/// Blocks a voice holds: the ones under and ahead of it, and the start of its loop
pub const RING: usize = 6;
/// Frames a streamed sample keeps in memory (1.4 s at 48 kHz): the head covers the disk's answer time
pub const HEAD_FRAMES: usize = 65_536;
/// Voices that can stream at once (each holds a ring of [`RING`] blocks, 6 x 2 x 32 KiB: 24 MiB for an instrument that streams)
pub const STREAM_VOICES: usize = 64;
/// The frames a sinc read reaches before and after the position
pub const REACH: i64 = 16;

/// Where the rest of a streamed sample is. Read from the plugin's thread, never from the audio thread
pub trait TailSource: Send + Sync + std::fmt::Debug {
    /// The sample's frames
    fn frames(&self) -> usize;
    fn stereo(&self) -> bool;
    /// Fills `left` (and `right`, for a stereo sample) with the frames from `start`; the count read (less at the end
    /// of the sample or on a read error: the rest of the block is silence)
    fn read(&self, start: usize, left: &mut [f32], right: &mut [f32]) -> usize;
}

/// What an importer needs to stream: the file the bytes it parses came from (opened by the plugin, which keeps it
/// alive as long as the instrument), the length from which a sample streams and how much of its start stays in memory
#[derive(Clone, Debug)]
pub struct StreamPlan {
    pub file: Arc<File>,
    /// Samples of more frames than this stream (the others are decoded whole)
    pub min_frames: usize,
    pub head: usize,
}

impl StreamPlan {
    /// The default thresholds: a sample longer than two heads streams
    pub fn new(file: Arc<File>) -> Self {
        Self { file, min_frames: 2 * HEAD_FRAMES, head: HEAD_FRAMES }
    }
}

/// Where `part` starts in `whole`, when it is a piece of it (the file offset of a chunk the parser sliced out of the
/// file's bytes)
pub fn offset_in(whole: &[u8], part: &[u8]) -> Option<usize> {
    let (w, p) = (whole.as_ptr() as usize, part.as_ptr() as usize);
    (p >= w && p + part.len() <= w + whole.len()).then(|| p - w)
}

/// How raw PCM is laid out in a file: little-endian, interleaved
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raw {
    I16,
    I24,
}

impl Raw {
    fn width(self) -> usize {
        match self {
            Raw::I16 => 2,
            Raw::I24 => 3,
        }
    }
}

/// A sample's raw PCM in a file, read by position (`pread`)
#[derive(Debug)]
pub struct FileTail {
    file: Arc<File>,
    offset: u64,
    frames: usize,
    channels: usize,
    raw: Raw,
    gain: f32,
}

impl FileTail {
    /// `frames` frames of `channels` (1 or 2) channels of `raw` PCM at byte `offset` of `file`
    pub fn new(file: Arc<File>, offset: u64, frames: usize, channels: usize, raw: Raw) -> Self {
        Self { file, offset, frames, channels: channels.clamp(1, 2), raw, gain: 1.0 }
    }

    /// The frames scaled by `gain` (the GIG's waves are read at half amplitude)
    #[must_use]
    pub fn scaled(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// The value of the sample at the start of `b`
    fn decode(&self, b: &[u8]) -> f32 {
        self.gain
            * match self.raw {
                Raw::I16 => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
                Raw::I24 => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            }
    }
}

impl TailSource for FileTail {
    fn frames(&self) -> usize {
        self.frames
    }

    fn stereo(&self) -> bool {
        self.channels == 2
    }

    fn read(&self, start: usize, left: &mut [f32], right: &mut [f32]) -> usize {
        if start >= self.frames {
            return 0;
        }
        let want = left.len().min(self.frames - start);
        let frame_bytes = self.channels * self.raw.width();
        let mut bytes = vec![0u8; want * frame_bytes];
        let at = self.offset + (start * frame_bytes) as u64;
        // (a short file gives what it has: `read_at` may return less than asked)
        let mut got = 0;
        while got < bytes.len() {
            match self.file.read_at(&mut bytes[got..], at + got as u64) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        let n = got / frame_bytes;
        let width = self.raw.width();
        for (i, f) in bytes[..n * frame_bytes].chunks_exact(frame_bytes).enumerate() {
            left[i] = self.decode(&f[..width]);
            if self.channels == 2 {
                if let Some(r) = right.get_mut(i) {
                    *r = self.decode(&f[width..]);
                }
            }
        }
        n
    }
}

/// A block of frames of a streamed sample: its own buffers, moved between the engine and the feeder, never freed on
/// the audio thread
#[derive(Debug)]
pub struct Block {
    pub left: Box<[f32]>,
    pub right: Box<[f32]>,
    /// The frames read into it (the rest is silence)
    pub frames: usize,
}

impl Block {
    pub fn new() -> Box<Block> {
        Box::new(Block { left: vec![0.0; BLOCK].into(), right: vec![0.0; BLOCK].into(), frames: 0 })
    }
}

/// What the engine asks of the feeder: block `index` (frames `index * BLOCK ..`) of `source`, into `block`
#[derive(Debug)]
pub struct Request {
    pub ring: u32,
    pub slot: u8,
    pub epoch: u32,
    pub index: u32,
    pub source: Arc<dyn TailSource>,
    pub block: Box<Block>,
}

/// What the feeder answers: the same buffer, filled
#[derive(Debug)]
pub struct Response {
    pub ring: u32,
    pub slot: u8,
    pub epoch: u32,
    pub index: u32,
    pub block: Box<Block>,
}

/// The engine's end of the queues
#[derive(Debug)]
pub struct Link {
    pub(crate) requests: Producer<Request>,
    pub(crate) responses: Consumer<Response>,
}

/// The feeder's end: [`Feeder::serve`] answers every request waiting
#[derive(Debug)]
pub struct Feeder {
    requests: Consumer<Request>,
    responses: Producer<Response>,
}

/// The two ends of a pair of queues that hold `capacity` blocks
pub fn link(capacity: usize) -> (Link, Feeder) {
    let (req_tx, req_rx) = RingBuffer::new(capacity.max(1));
    let (res_tx, res_rx) = RingBuffer::new(capacity.max(1));
    (Link { requests: req_tx, responses: res_rx }, Feeder { requests: req_rx, responses: res_tx })
}

/// The queues' size for an engine: a request for every slot of every ring
pub const QUEUE: usize = STREAM_VOICES * RING;

impl Feeder {
    /// Reads and answers every request waiting; the count served. Off the audio thread
    pub fn serve(&mut self) -> usize {
        let mut served = 0;
        while let Ok(mut r) = self.requests.pop() {
            let b = &mut *r.block;
            b.left.fill(0.0);
            b.right.fill(0.0);
            let start = r.index as usize * BLOCK;
            b.frames = r.source.read(start, &mut b.left, &mut b.right);
            if !r.source.stereo() {
                let (l, rt) = (&b.left, &mut b.right);
                rt.copy_from_slice(l);
            }
            // (the response queue holds as many as the requests: it cannot be full)
            let _ = self.responses.push(Response { ring: r.ring, slot: r.slot, epoch: r.epoch, index: r.index, block: r.block });
            served += 1;
        }
        served
    }

    /// Whether the engine's end is gone (the processor was dropped): the feeder's thread ends
    pub fn abandoned(&self) -> bool {
        self.requests.is_abandoned()
    }
}

#[derive(Debug)]
enum State {
    /// The slot holds a buffer and nothing useful
    Empty,
    /// A request for this block is out (the buffer is with the feeder), made in this epoch of the ring
    Pending(u32, u32),
    /// The slot holds this block
    Ready(u32),
}

#[derive(Debug)]
struct Slot {
    block: Option<Box<Block>>,
    state: State,
}

/// The blocks one voice reads beyond its sample's head
#[derive(Debug)]
pub struct Ring {
    pub(crate) used: bool,
    pub(crate) voice: u32,
    pub(crate) order: u64,
    epoch: u32,
    slots: [Slot; RING],
    /// Frames asked of the ring that were not there
    pub(crate) misses: u32,
}

impl Ring {
    pub(crate) fn new() -> Self {
        Self {
            used: false,
            voice: 0,
            order: 0,
            epoch: 0,
            slots: std::array::from_fn(|_| Slot { block: Some(Block::new()), state: State::Empty }),
            misses: 0,
        }
    }

    /// The ring is for a new voice: what it held is of no use (a request still out is answered into an empty slot)
    pub(crate) fn take(&mut self, voice: u32, order: u64) {
        self.used = true;
        self.voice = voice;
        self.order = order;
        self.epoch = self.epoch.wrapping_add(1);
        for s in &mut self.slots {
            // (a request still out is answered into an empty slot: its epoch is the old one)
            if matches!(s.state, State::Ready(_)) {
                s.state = State::Empty;
            }
        }
    }

    /// An answer from the feeder: the buffer comes back, and holds the block if it is still wanted
    pub(crate) fn accept(&mut self, r: Response) {
        let Some(slot) = self.slots.get_mut(usize::from(r.slot)) else { return };
        slot.state = if r.epoch == self.epoch && matches!(slot.state, State::Pending(i, e) if i == r.index && e == r.epoch) {
            State::Ready(r.index)
        } else {
            State::Empty
        };
        slot.block = Some(r.block);
    }

    fn holds(&self, index: u32) -> bool {
        self.slots.iter().any(|s| match s.state {
            State::Ready(i) => i == index,
            State::Pending(i, e) => i == index && e == self.epoch,
            State::Empty => false,
        })
    }

    /// The frame `i` of the sample past its head, or silence (a miss, counted) when its block is not here
    #[inline]
    fn block_frame(&mut self, i: i64) -> [f32; 2] {
        let index = (i / BLOCK as i64) as u32;
        let at = (i % BLOCK as i64) as usize;
        for s in &self.slots {
            if let (State::Ready(n), Some(b)) = (&s.state, &s.block)
                && *n == index
            {
                return if at < b.frames { [b.left[at], b.right[at]] } else { [0.0; 2] };
            }
        }
        self.misses = self.misses.wrapping_add(1);
        [0.0; 2]
    }

    /// The frame `i` of a streamed sample: its head, or this ring's block
    #[inline]
    pub(crate) fn at(&mut self, pcm: &crate::Pcm, i: i64) -> [f32; 2] {
        if i < 0 || i as usize >= pcm.frames() {
            return [0.0; 2];
        }
        if (i as usize) < pcm.head_frames() {
            return pcm.at(i);
        }
        self.block_frame(i)
    }

    /// Asks for the blocks that cover frames `lo..=hi` of the sample past its head that the ring does not hold, in the
    /// order they will be needed: `ahead` says whether that is upward (a block that is not wanted may be given up).
    /// Returns the requests that did not fit (never, with the queue sized by [`QUEUE`])
    pub(crate) fn want(
        &mut self,
        ring: u32,
        pcm: &crate::Pcm,
        spans: &[(i64, i64)],
        link: &mut Option<Link>,
    ) {
        let Some(source) = pcm.tail() else { return };
        let head = pcm.head_frames() as i64;
        let total = pcm.frames() as i64;
        // The blocks wanted, first to last (at most RING)
        let mut wanted = [u32::MAX; RING];
        let mut count = 0;
        for &(lo, hi) in spans {
            let (lo, hi) = (lo.max(head).max(0), hi.min(total - 1));
            if hi < lo {
                continue;
            }
            for b in (lo / BLOCK as i64)..=(hi / BLOCK as i64) {
                let b = b as u32;
                if count < RING && !wanted[..count].contains(&b) {
                    wanted[count] = b;
                    count += 1;
                }
            }
        }
        for &b in &wanted[..count] {
            if self.holds(b) {
                continue;
            }
            // A slot to fill: one that is empty, else a block that is no longer wanted
            let victim = self
                .slots
                .iter()
                .position(|s| matches!(s.state, State::Empty) && s.block.is_some())
                .or_else(|| {
                    self.slots.iter().position(|s| {
                        matches!(s.state, State::Ready(i) if !wanted[..count].contains(&i)) && s.block.is_some()
                    })
                });
            let Some(v) = victim else { return };
            let Some(mut block) = self.slots[v].block.take() else { return };
            match link {
                Some(l) => {
                    let request = Request {
                        ring,
                        slot: v as u8,
                        epoch: self.epoch,
                        index: b,
                        source: Arc::clone(source),
                        block,
                    };
                    match l.requests.push(request) {
                        Ok(()) => self.slots[v].state = State::Pending(b, self.epoch),
                        Err(rtrb::PushError::Full(r)) => {
                            // (the buffer comes back: asked again at the next block)
                            self.slots[v].block = Some(r.block);
                            return;
                        }
                    }
                }
                None => {
                    // Without a feeder: read it here
                    block.left.fill(0.0);
                    block.right.fill(0.0);
                    block.frames = source.read(b as usize * BLOCK, &mut block.left, &mut block.right);
                    if !source.stereo() {
                        let (l, r) = (&block.left, &mut block.right);
                        r.copy_from_slice(l);
                    }
                    self.slots[v].block = Some(block);
                    self.slots[v].state = State::Ready(b);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
