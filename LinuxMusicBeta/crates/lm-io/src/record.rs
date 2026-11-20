//! The audio side of a recording (interfaces.md, I-11, R4; U1, U4, U5; C2, C4): a lossless single-producer
//! single-consumer path from the device callback to the disk thread, sized in seconds.
//!
//! - One data ring per input (interleaved `f32`, the input's device channels), sized for `seconds` of audio.
//! - One ring of [`Marker`]s, the take's structure in file frames: where a take starts and on which timeline sample,
//!   the recorded spans (punch and loop cuts, U1, U4), the capture gaps (C2), an input's overflow and the end.
//!
//! **Full is an explicit error, never a silent loss** (I-11): an input whose ring has no room for a block stops
//! recording right there, and the disk thread closes its file at the last good block and reports the exact time.
//!
//! **What is recorded:** while the sender is armed in the device and the transport plays, every block of the armed
//! inputs, from the first played block to the last: the pre-roll and the parts outside the punch are recorded as
//! handles (U4), and the renderer's record spans say which part is the take proper. One file per input and per take
//! (a take ends when the transport stops); loop passes stay in the same file with their cuts marked (U1).
//!
//! **The grid never moves** (C2): a capture xrun's lost frames go into the file as exactly that much silence,
//! marked, where the renderer reported them.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lm_base::ring::{Consumer, Producer, capacity_for, ring};
use lm_base::{Error, Result};
use lm_render::RecordSpan;

/// An input of the device as the session names it: a number the session gives, with the device channels it reads
/// (a mono input is one channel, a stereo one two). `lm-io` knows inputs, not tracks (T2)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InputIndex(pub u32);

/// One armed input
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordInput {
    pub input: InputIndex,
    /// Device channels, in the order they go into the file
    pub channels: Vec<u16>,
    /// What the session needs to put the take back after a crash (U5): the ids of the tracks this input records
    /// into, as text. Opaque to `lm-io`
    pub tracks: Vec<String>,
}

/// Everything a recording needs to know, given when it is armed
#[derive(Clone, Debug, PartialEq)]
pub struct RecordPlan {
    /// The project's `audio/` folder
    pub folder: PathBuf,
    pub inputs: Vec<RecordInput>,
    pub rate: u32,
    /// Seconds of audio each input's ring holds (default 10, I-11)
    pub seconds: f32,
    /// The latency subtracted to place the take (round trip, H5), in samples: written to `recording.json` (U5)
    pub latency: i64,
    /// The PDC of the playback path at the start of the recording (H5), in samples
    pub pdc: i64,
}

impl RecordPlan {
    pub fn new(folder: PathBuf, inputs: Vec<RecordInput>, rate: u32) -> Self {
        Self { folder, inputs, rate, seconds: DEFAULT_SECONDS, latency: 0, pdc: 0 }
    }
}

/// The default size of the recording ring (I-11)
pub const DEFAULT_SECONDS: f32 = 10.0;

/// The most spans one block can cut (the renderer cuts at most three per loop wrap, and the loop is at least 50 ms)
pub const MAX_SPANS: usize = 64;

/// The take's structure, in file frames (the file of every input of the take has the same frames)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Marker {
    /// A take starts: file frame 0 is timeline sample `sample` (estimated from the first block when it has no
    /// recorded span)
    Start { take: u32, sample: i64 },
    /// File frames `file_frame..+frames` are the timeline samples `sample..+frames` inside the record range
    Span { file_frame: u64, sample: i64, frames: u64 },
    /// `frames` of silence go at `file_frame`: the capture lost them in an xrun (C2)
    Gap { file_frame: u64, frames: u64 },
    /// Input `input` had no room at `file_frame`: it stops there (I-11)
    Overflow { input: u16, file_frame: u64 },
    /// The take ends at `file_frame` (its length)
    End { file_frame: u64 },
}

/// The device side of a recording. Lives in the device callback (it is armed with [`crate::Device::arm`]); every
/// method is real-time safe: no allocation, no lock, no system call
pub struct RecordSender {
    inputs: Box<[SenderInput]>,
    markers: Producer<Marker>,
    marker_room: usize,
    failed: Arc<AtomicBool>,
    take_open: bool,
    take: u32,
    file_pos: u64,
    pending: Option<(u64, i64, u64)>,
    max_channel: usize,
    /// A span longer than this is sent in pieces, so `recording.json` knows the take proper even after a crash (U5)
    span_flush: u64,
}

struct SenderInput {
    tx: Producer<f32>,
    channels: Box<[u16]>,
    enabled: bool,
}

impl fmt::Debug for RecordSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordSender")
            .field("inputs", &self.inputs.len())
            .field("take_open", &self.take_open)
            .field("file_pos", &self.file_pos)
            .finish()
    }
}

/// The disk side of a recording: given to the [`crate::DiskWriter`]
pub struct RecordReceiver {
    pub(crate) plan: RecordPlan,
    pub(crate) data: Vec<Consumer<f32>>,
    pub(crate) markers: Consumer<Marker>,
    pub(crate) failed: Arc<AtomicBool>,
}

impl fmt::Debug for RecordReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordReceiver").field("plan", &self.plan).finish()
    }
}

/// The marker ring: a block cuts at most `MAX_SPANS` spans, plus a start, a gap, an end and an overflow per input;
/// spans are merged across blocks, so a block pushes a handful at most. Sized for the data ring's seconds at one
/// marker set per 64 frames (blocks are never shorter in practice), at least 4096
fn marker_capacity(plan: &RecordPlan) -> usize {
    let per_second = f64::from(plan.rate) / 64.0 * (4 + plan.inputs.len()) as f64;
    capacity_for(per_second, f64::from(plan.seconds), 4096)
}

/// Makes the two ends of a recording (I-11). Allocates the rings: on the main thread or a worker, never the audio
/// thread. Errors: `io.record.no_inputs`, `io.record.no_channels`, `io.record.bad_seconds`
pub fn recording_channel(plan: RecordPlan) -> Result<(RecordSender, RecordReceiver)> {
    if plan.inputs.is_empty() {
        return Err(Error::new("io.record.no_inputs"));
    }
    if !(plan.seconds.is_finite() && plan.seconds >= 0.5) || plan.rate == 0 {
        return Err(Error::new("io.record.bad_seconds").arg("seconds", f64::from(plan.seconds)));
    }
    let mut inputs = Vec::with_capacity(plan.inputs.len());
    let mut data = Vec::with_capacity(plan.inputs.len());
    let mut max_channel = 0usize;
    for input in &plan.inputs {
        if input.channels.is_empty() {
            return Err(Error::new("io.record.no_channels").arg("input", i64::from(input.input.0)));
        }
        let size = capacity_for(f64::from(plan.rate) * input.channels.len() as f64, f64::from(plan.seconds), 1024);
        let (tx, rx) = ring::<f32>(size);
        max_channel = max_channel.max(input.channels.iter().map(|c| usize::from(*c) + 1).max().unwrap_or(0));
        inputs.push(SenderInput { tx, channels: input.channels.clone().into_boxed_slice(), enabled: true });
        data.push(rx);
    }
    let (mtx, mrx) = ring(marker_capacity(&plan));
    let failed = Arc::new(AtomicBool::new(false));
    let sender = RecordSender {
        inputs: inputs.into_boxed_slice(),
        markers: mtx,
        marker_room: 0,
        failed: failed.clone(),
        take_open: false,
        take: 0,
        file_pos: 0,
        pending: None,
        max_channel,
        span_flush: u64::from(plan.rate),
    };
    Ok((sender, RecordReceiver { plan, data, markers: mrx, failed }))
}

impl RecordSender {
    /// The highest device channel any input reads, plus one
    pub fn channels_needed(&self) -> usize {
        self.max_channel
    }

    /// True while a take is being written
    pub fn is_recording(&self) -> bool {
        self.take_open
    }

    /// One device block, after the renderer rendered it. `spans` and `gap` are the renderer's
    /// [`lm_render::BlockReport`]; `playing` and `position` are the renderer's state after the block. Real-time safe
    pub fn capture(
        &mut self,
        frames: usize,
        spans: &[RecordSpan],
        gap: Option<(i64, u32)>,
        inputs: &[&[f32]],
        playing: bool,
        position: i64,
    ) {
        if self.failed.load(Ordering::Relaxed) {
            return;
        }
        let active = playing || !spans.is_empty();
        if !active {
            if self.take_open {
                self.end();
            }
            return;
        }
        // Room for the worst case of this block, checked once: the markers never fill in the middle of a block
        self.marker_room = self.markers.slots();
        if self.marker_room < MAX_SPANS + 4 + self.inputs.len() {
            self.fail();
            return;
        }
        if !self.take_open {
            let sample = match spans.first() {
                Some(s) => s.sample - s.offset as i64,
                None => position - frames as i64,
            };
            self.take = self.take.wrapping_add(1);
            self.push(Marker::Start { take: self.take, sample });
            self.take_open = true;
            self.file_pos = 0;
            self.pending = None;
            for input in self.inputs.iter_mut() {
                input.enabled = true;
            }
        } else if let Some((_, lost)) = gap {
            let lost = u64::from(lost);
            self.push(Marker::Gap { file_frame: self.file_pos, frames: lost });
            // The renderer moved the playhead by the lost frames too: the silence is on the grid, inside the span
            if let Some((f, _, n)) = &mut self.pending
                && *f + *n == self.file_pos
            {
                *n += lost;
            }
            self.file_pos += lost;
        }
        for s in spans {
            let file_frame = self.file_pos + s.offset as u64;
            let frames = s.frames as u64;
            match &mut self.pending {
                Some((f, smp, n)) if *f + *n == file_frame && *smp + *n as i64 == s.sample => *n += frames,
                _ => {
                    self.flush_span();
                    self.pending = Some((file_frame, s.sample, frames));
                }
            }
        }
        if self.pending.is_some_and(|(_, _, n)| n >= self.span_flush) {
            self.flush_span();
        }
        for (k, input) in self.inputs.iter_mut().enumerate() {
            if !input.enabled {
                continue;
            }
            let needed = frames * input.channels.len();
            if input.tx.slots() < needed {
                input.enabled = false;
                // Room was checked above
                let _ = self.markers.push(Marker::Overflow { input: k as u16, file_frame: self.file_pos });
                continue;
            }
            let Ok(chunk) = input.tx.write_chunk_uninit(needed) else { continue };
            let channels = &input.channels;
            let samples = (0..frames).flat_map(|f| {
                channels
                    .iter()
                    .map(move |&c| inputs.get(usize::from(c)).and_then(|ch| ch.get(f)).copied().unwrap_or(0.0))
            });
            chunk.fill_from_iter(samples);
        }
        self.file_pos += frames as u64;
    }

    /// Ends the take now (the device stops, the sender is disarmed, the card went away). Real-time safe
    pub fn end(&mut self) {
        if !self.take_open {
            return;
        }
        self.flush_span();
        // The end must reach the disk; a full ring here is the explicit failure of the whole recording
        if self.markers.push(Marker::End { file_frame: self.file_pos }).is_err() {
            self.fail();
        }
        self.take_open = false;
    }

    fn flush_span(&mut self) {
        if let Some((file_frame, sample, frames)) = self.pending.take() {
            self.push(Marker::Span { file_frame, sample, frames });
        }
    }

    fn push(&mut self, marker: Marker) {
        if self.markers.push(marker).is_err() {
            self.fail();
        }
    }

    /// The marker ring is full: every input stops, and the disk thread closes every file at what it has (I-11)
    fn fail(&mut self) {
        self.failed.store(true, Ordering::Release);
        self.take_open = false;
        for input in self.inputs.iter_mut() {
            input.enabled = false;
        }
    }
}

impl Drop for RecordSender {
    fn drop(&mut self) {
        self.end();
    }
}

impl RecordReceiver {
    pub fn plan(&self) -> &RecordPlan {
        &self.plan
    }
}
