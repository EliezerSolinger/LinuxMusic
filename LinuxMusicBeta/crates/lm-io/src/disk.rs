//! The recording's disk thread (interfaces.md, I-11; U5, U6; H3; C2, C10): the only consumer of the recording rings.
//! A thread of its own ("lm-disk", role `Disk`), used for nothing else, so no other `fsync` ever delays it (U6).
//!
//! For each armed input and take it writes `audio/rec-<uuid>.wav` (RF64, 32-bit float) and its `recording.json`
//! (written and synced before the first block, U5), and then:
//! - syncs file and record every ~2 s of audio (a crash leaves the file readable up to the last sync);
//! - keeps the disk reserved ~30 s ahead (`fallocate`), so a full disk shows early, with room to close cleanly (H3);
//! - writes a capture gap as exactly that much silence, marked (C2);
//! - on a full ring, a full disk or a write error, closes that input's file at the last good block and reports the
//!   exact time (I-11: an explicit error, never a silent loss);
//! - publishes the growing waveform as level-0 peak pairs (U3), on a ring that counts what it drops (the final
//!   `.peaks` has everything).
//!
//! Its events go to the main thread on a ring that never loses (a local queue holds what does not fit: the disk
//! thread may wait, the audio thread never does).

use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use lm_base::ring::{Consumer, Producer, capacity_for, ring};
use lm_base::{Error, Result, Role};
use lm_sys_sndfile::{Existing, Sample, WriteFormat, WriteSpec, Writer};

use crate::record::{InputIndex, Marker, RecordReceiver};
use crate::take::{
    LoopCut, RECORD_VERSION, RecordFile, RecordHeader, RecordLine, REFUSE_SECONDS, StopReason, Take, TakeGap, TakeSpan,
    TakeStop, free_space, new_uuid, record_bytes_per_second,
};

/// Seconds of audio between two syncs (I-11)
pub const SYNC_SECONDS: f64 = 2.0;
/// Seconds of disk kept reserved ahead of the recording (H3)
pub const RESERVE_SECONDS: f64 = 30.0;
/// Reserve again when less than this is left of the reservation
const RESERVE_REFILL_SECONDS: f64 = 20.0;
/// The most channels of one input whose live peaks are published
pub const LIVE_PEAK_CHANNELS: usize = 8;
/// How long the thread sleeps when there is nothing to do (the rings hold seconds)
const IDLE: Duration = Duration::from_millis(5);

/// What the disk thread tells the session. Lossless
#[derive(Debug)]
pub enum DiskEvent {
    /// A take's file and record were created (the record is synced): the take can show "recording"
    TakeStarted { input: InputIndex, path: PathBuf, record: PathBuf, start_sample: i64 },
    /// An input stopped before the take ended (I-11, H3): its file ends at `file_frame`, `seconds` into the take,
    /// at timeline sample `sample`. `RecordingOverflow` of the interfaces is the `Overflow` reason
    InputStopped { input: InputIndex, reason: StopReason, file_frame: u64, sample: i64, seconds: f64, error: Option<Error> },
    /// A take is closed (the RF64 has its final header): finalise it in an `Operation` ([`crate::finalize_take`])
    TakeClosed(Take),
    /// A file could not be created or a record could not be written
    Failed { input: Option<InputIndex>, error: Error },
    /// A recording's sender went away and everything was written: the receiver is done
    RecordingFinished,
}

/// One level-0 peak pair of a take being recorded (U3): `pair` is its index in the take (each pair is 64 frames)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LivePeak {
    pub input: InputIndex,
    /// The take's number within its recording
    pub take: u32,
    pub pair: u64,
    pub channels: u8,
    pub min: [i16; LIVE_PEAK_CHANNELS],
    pub max: [i16; LIVE_PEAK_CHANNELS],
}

enum DiskCommand {
    Record(Box<RecordReceiver>),
}

/// The main thread's handle of the disk thread. Its `Drop` lets the thread finish what it holds (every take still
/// open is closed with what was written) and joins it
pub struct DiskWriter {
    commands: Producer<DiskCommand>,
    events: Consumer<DiskEvent>,
    live: Consumer<LivePeak>,
    live_dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for DiskWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskWriter").field("running", &self.thread.is_some()).finish()
    }
}

/// Live peak pairs held for a stopped pump: 10 s (C4) of 16 stereo inputs at 192 kHz is 480 000 pairs; the ring
/// counts what it drops instead of growing
const LIVE_RING: usize = 1 << 16;
const EVENT_RING: usize = 4096;
const COMMAND_RING: usize = 64;

impl DiskWriter {
    /// Starts the disk thread
    pub fn start() -> Result<DiskWriter> {
        let (commands, cmd_rx) = ring(COMMAND_RING);
        let (ev_tx, events) = ring(EVENT_RING);
        let (live_tx, live) = ring(LIVE_RING);
        let stop = Arc::new(AtomicBool::new(false));
        let live_dropped = Arc::new(AtomicU64::new(0));
        let mut worker = Worker {
            commands: cmd_rx,
            events: ev_tx,
            pending: VecDeque::new(),
            live: live_tx,
            live_dropped: live_dropped.clone(),
            active: Vec::new(),
            stop: stop.clone(),
            zeros: vec![0.0; 4096],
            scratch: Vec::new(),
        };
        let thread = std::thread::Builder::new()
            .name("lm-disk".to_owned())
            .spawn(move || {
                lm_base::set_role(Role::Disk);
                worker.run();
            })
            .map_err(|e| Error::new("io.disk.thread_failed").arg("detail", e.to_string()))?;
        Ok(DiskWriter { commands, events, live, live_dropped, stop, thread: Some(thread) })
    }

    /// Hands a recording's receiver to the disk thread (before arming its sender in the device)
    pub fn record(&mut self, receiver: RecordReceiver) -> Result<()> {
        self.commands
            .push(DiskCommand::Record(Box::new(receiver)))
            .map_err(|_| Error::new("io.disk.busy"))?;
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
        Ok(())
    }

    /// From the pump: the disk thread's events
    pub fn poll(&mut self, mut f: impl FnMut(DiskEvent)) {
        while let Ok(e) = self.events.pop() {
            f(e);
        }
    }

    /// From the pump: the growing waveforms (U3)
    pub fn live_peaks(&mut self, mut f: impl FnMut(LivePeak)) {
        while let Ok(p) = self.live.pop() {
            f(p);
        }
    }

    /// Live peak pairs dropped because the pump did not take them in time (the final peaks have them)
    pub fn live_dropped(&self) -> u64 {
        self.live_dropped.load(Ordering::Relaxed)
    }
}

impl Drop for DiskWriter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

struct Worker {
    commands: Consumer<DiskCommand>,
    events: Producer<DiskEvent>,
    pending: VecDeque<DiskEvent>,
    live: Producer<LivePeak>,
    live_dropped: Arc<AtomicU64>,
    active: Vec<Active>,
    stop: Arc<AtomicBool>,
    zeros: Vec<f32>,
    scratch: Vec<f32>,
}

/// A recording being served
struct Active {
    recv: RecordReceiver,
    take: Option<OpenTake>,
    take_no: u32,
}

struct OpenTake {
    inputs: Vec<OpenInput>,
    start_sample: i64,
    spans: Vec<TakeSpan>,
    cuts: Vec<LoopCut>,
    gaps: Vec<TakeGap>,
    /// The last span's end: (file frame, timeline sample)
    span_end: Option<(u64, i64)>,
    take_no: u32,
}

struct OpenInput {
    index: InputIndex,
    channels: usize,
    writer: Option<Writer>,
    path: PathBuf,
    record: Option<RecordFile>,
    /// File frames written (data and gaps)
    written: u64,
    last_sync: u64,
    reserved: u64,
    reserve_supported: bool,
    stopped: Option<TakeStop>,
    peak: LivePeaker,
}

/// The current level-0 pair of an input being recorded
struct LivePeaker {
    min: [f32; LIVE_PEAK_CHANNELS],
    max: [f32; LIVE_PEAK_CHANNELS],
    in_pair: usize,
    pair: u64,
}

impl LivePeaker {
    fn new() -> Self {
        Self { min: [f32::INFINITY; LIVE_PEAK_CHANNELS], max: [f32::NEG_INFINITY; LIVE_PEAK_CHANNELS], in_pair: 0, pair: 0 }
    }
}

fn q(x: f32, up: bool) -> i16 {
    let v = x.clamp(-1.0, 1.0) * 32767.0;
    (if up { v.ceil() } else { v.floor() }).clamp(-32767.0, 32767.0) as i16
}

impl Worker {
    fn run(&mut self) {
        loop {
            while let Ok(DiskCommand::Record(r)) = self.commands.pop() {
                self.active.push(Active { recv: *r, take: None, take_no: 0 });
            }
            let mut busy = false;
            let stopping = self.stop.load(Ordering::Acquire);
            let mut k = 0;
            while k < self.active.len() {
                let mut a = self.active.swap_remove(k);
                let (progressed, done) = self.service(&mut a, stopping);
                busy |= progressed;
                if done {
                    self.emit(DiskEvent::RecordingFinished);
                } else {
                    self.active.push(a);
                    let last = self.active.len() - 1;
                    self.active.swap(k, last);
                    k += 1;
                }
            }
            self.flush_events();
            if stopping && self.active.is_empty() && self.pending.is_empty() {
                break;
            }
            if stopping && self.active.is_empty() {
                // The main thread is gone: what cannot be delivered is dropped with the thread
                break;
            }
            if !busy {
                std::thread::park_timeout(IDLE);
            }
        }
    }

    fn emit(&mut self, e: DiskEvent) {
        self.pending.push_back(e);
        self.flush_events();
    }

    fn flush_events(&mut self) {
        while let Some(e) = self.pending.pop_front() {
            if let Err(rtrb::PushError::Full(e)) = self.events.push(e) {
                self.pending.push_front(e);
                break;
            }
        }
    }

    /// One pass over a recording: markers first (each takes the data up to its frame), then the data that was
    /// there before the markers were read. Returns (did something, is finished)
    fn service(&mut self, a: &mut Active, stopping: bool) -> (bool, bool) {
        let mut busy = false;
        // What the data rings hold now. Every marker is pushed after the data before its frame, so a marker handler
        // drains up to its frame whatever the snapshot says; the final drain below takes only what was there before
        // the markers were read (data that came later may lie past a marker not popped yet)
        let mut allow: Vec<u64> = a
            .recv
            .data
            .iter()
            .zip(&a.recv.plan.inputs)
            .map(|(rx, i)| (rx.slots() / i.channels.len()) as u64)
            .collect();
        if a.recv.failed.load(Ordering::Acquire) {
            // The marker ring overflowed: nothing after this point can be placed; close every input with what it has
            if let Some(t) = a.take.as_mut() {
                for k in 0..t.inputs.len() {
                    let avail = allow[k];
                    self.drain(a, k, u64::MAX, avail);
                    if let Some(t) = a.take.as_mut() {
                        let at = t.inputs[k].written;
                        self.stop_input(a, k, StopReason::Overflow, at, None);
                    }
                }
            }
            self.close_take(a);
            while a.recv.markers.pop().is_ok() {}
            for (k, rx) in a.recv.data.iter_mut().enumerate() {
                let n = rx.slots();
                if let Ok(chunk) = rx.read_chunk(n) {
                    chunk.commit_all();
                }
                let _ = k;
            }
            return (true, a.recv.markers.is_abandoned());
        }
        while let Ok(m) = a.recv.markers.pop() {
            busy = true;
            self.handle(a, m, &mut allow);
        }
        for k in 0..a.recv.data.len() {
            let n = allow[k];
            if n > 0 {
                busy = true;
                self.drain(a, k, u64::MAX, n);
            }
        }
        let abandoned = a.recv.markers.is_abandoned() && a.recv.markers.is_empty();
        if (abandoned || stopping) && a.take.is_some() && a.recv.markers.is_empty() {
            // The sender is gone without an end (or the program is closing): close at what was written
            let ends: Vec<u64> = a.take.as_ref().map(|t| t.inputs.iter().map(|i| i.written).collect()).unwrap_or_default();
            let end = ends.into_iter().max().unwrap_or(0);
            self.end_take(a, end, &mut allow);
        }
        let done = abandoned || (stopping && a.take.is_none());
        (busy, done)
    }

    fn handle(&mut self, a: &mut Active, m: Marker, allow: &mut [u64]) {
        match m {
            Marker::Start { take, sample } => {
                if a.take.is_some() {
                    let end = a.take.as_ref().map_or(0, |t| t.inputs.iter().map(|i| i.written).max().unwrap_or(0));
                    self.end_take(a, end, allow);
                }
                a.take_no = take;
                self.open_take(a, sample);
            }
            Marker::Span { file_frame, sample, frames } => {
                let Some(t) = a.take.as_mut() else { return };
                if let Some((end_file, end_sample)) = t.span_end {
                    let expected = end_sample + (file_frame - end_file.min(file_frame)) as i64;
                    if sample != expected {
                        let cut = LoopCut { file_frame, from_sample: expected, to_sample: sample };
                        t.cuts.push(cut);
                        for i in t.inputs.iter_mut() {
                            append(i, &RecordLine::Cut(cut));
                        }
                    }
                }
                let span = TakeSpan { file_frame, sample, frames };
                // A span sent in pieces (one a second) is one span
                match t.spans.last_mut() {
                    Some(last) if last.file_frame + last.frames == file_frame && last.sample + last.frames as i64 == sample => {
                        last.frames += frames;
                    }
                    _ => t.spans.push(span),
                }
                t.span_end = Some((file_frame + frames, sample + frames as i64));
                for i in t.inputs.iter_mut() {
                    append(i, &RecordLine::Span(span));
                }
            }
            Marker::Gap { file_frame, frames } => {
                let Some(n) = a.take.as_ref().map(|t| t.inputs.len()) else { return };
                for k in 0..n {
                    let used = self.drain(a, k, file_frame, u64::MAX);
                    allow[k] = allow[k].saturating_sub(used);
                    self.silence(a, k, frames);
                }
                if let Some(t) = a.take.as_mut() {
                    let gap = TakeGap { file_frame, frames };
                    t.gaps.push(gap);
                    if let Some((f, s)) = t.span_end.as_mut()
                        && *f == file_frame
                    {
                        // The renderer moved the playhead by the lost frames too: the timeline stays continuous
                        *f += frames;
                        *s += frames as i64;
                    }
                    for i in t.inputs.iter_mut() {
                        append(i, &RecordLine::Gap(gap));
                    }
                }
            }
            Marker::Overflow { input, file_frame } => {
                let k = usize::from(input);
                if a.take.as_ref().is_some_and(|t| k < t.inputs.len()) {
                    let used = self.drain(a, k, file_frame, u64::MAX);
                    allow[k] = allow[k].saturating_sub(used);
                    self.stop_input(a, k, StopReason::Overflow, file_frame, None);
                }
            }
            Marker::End { file_frame } => self.end_take(a, file_frame, allow),
        }
    }

    fn open_take(&mut self, a: &mut Active, start_sample: i64) {
        let plan = a.recv.plan.clone();
        let mut inputs = Vec::with_capacity(plan.inputs.len());
        for input in &plan.inputs {
            let uuid = new_uuid();
            let path = plan.folder.join(format!("rec-{uuid}.wav"));
            let record_path = plan.folder.join(format!("rec-{uuid}.recording.json"));
            let header = RecordHeader {
                version: RECORD_VERSION,
                uuid: uuid.clone(),
                input: input.input.0,
                channels: input.channels.clone(),
                tracks: input.tracks.clone(),
                rate: plan.rate,
                file: format!("rec-{uuid}.wav"),
                start_sample,
                latency: plan.latency,
                pdc: plan.pdc,
            };
            let channels = input.channels.len();
            let mut open = OpenInput {
                index: input.input,
                channels,
                writer: None,
                path: path.clone(),
                record: None,
                written: 0,
                last_sync: 0,
                reserved: 0,
                reserve_supported: true,
                stopped: None,
                peak: LivePeaker::new(),
            };
            // The record first, synced, before any audio (U5)
            let created = std::fs::create_dir_all(&plan.folder)
                .map_err(|e| crate::take::io_error("io.record.create_failed", &plan.folder, &e))
                .and_then(|()| RecordFile::create(&record_path, &header))
                .and_then(|record| {
                    let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate: plan.rate, channels };
                    let writer = Writer::create(&path, spec, Existing::Fail)?;
                    Ok((record, writer))
                });
            match created {
                Ok((record, writer)) => {
                    open.record = Some(record);
                    open.writer = Some(writer);
                    self.reserve(&mut open, plan.rate);
                    self.emit(DiskEvent::TakeStarted {
                        input: input.input,
                        path,
                        record: record_path,
                        start_sample,
                    });
                }
                Err(error) => {
                    open.stopped = Some(TakeStop { reason: StopReason::WriteFailed, file_frame: 0 });
                    self.emit(DiskEvent::Failed { input: Some(input.input), error });
                }
            }
            inputs.push(open);
        }
        a.take = Some(OpenTake {
            inputs,
            start_sample,
            spans: Vec::new(),
            cuts: Vec::new(),
            gaps: Vec::new(),
            span_end: None,
            take_no: a.take_no,
        });
    }

    /// Keeps the disk reserved ~30 s ahead of what is written (H3). A full disk stops the input now, with the
    /// header and the file still closable
    fn reserve(&mut self, i: &mut OpenInput, rate: u32) -> Option<Error> {
        let bps = record_bytes_per_second(i.channels, rate);
        let frame_bytes = i.channels as u64 * 4;
        let reserved_frames = i.reserved / frame_bytes.max(1);
        if i.reserved > 0 && reserved_frames > i.written + (RESERVE_REFILL_SECONDS * f64::from(rate)) as u64 {
            return None;
        }
        let total = 4096 + (i.written * frame_bytes) + (RESERVE_SECONDS * bps as f64) as u64;
        let writer = i.writer.as_mut()?;
        if i.reserve_supported {
            match writer.reserve(total) {
                Ok(()) => {
                    i.reserved = total;
                    None
                }
                Err(e) if e.key() == "sndfile.reserve_unsupported" => {
                    i.reserve_supported = false;
                    None
                }
                Err(e) => Some(e),
            }
        } else {
            // No reservation on this file system: watch the free space instead
            match free_space(&i.path) {
                Ok(free) if (free as f64) < REFUSE_SECONDS * bps as f64 => Some(Error::new("sndfile.disk_full")),
                _ => None,
            }
        }
    }

    /// Writes up to `limit` frames of input `k` from its ring, not past file frame `until`. Returns frames read
    fn drain(&mut self, a: &mut Active, k: usize, until: u64, limit: u64) -> u64 {
        let rate = a.recv.plan.rate;
        let channels = a.recv.plan.inputs[k].channels.len();
        let Some(t) = a.take.as_mut() else {
            // No take open: data with no take is impossible (the sender opens one first); drop it
            return 0;
        };
        let input = &mut t.inputs[k];
        let want = limit.min(until.saturating_sub(input.written));
        if want == 0 {
            return 0;
        }
        let rx = &mut a.recv.data[k];
        let n = (want as usize).min(rx.slots() / channels);
        if n == 0 {
            return 0;
        }
        let Ok(chunk) = rx.read_chunk(n * channels) else { return 0 };
        let (first, second) = chunk.as_slices();
        self.scratch.clear();
        self.scratch.extend_from_slice(first);
        self.scratch.extend_from_slice(second);
        chunk.commit_all();
        let take_no = t.take_no;
        let stop = if input.stopped.is_some() || input.writer.is_none() {
            // A stopped input: its data is read and dropped (the file ended at the last good block)
            None
        } else {
            let result = input.writer.as_mut().map(|w| w.write_f32(&self.scratch)).unwrap_or(Ok(()));
            match result {
                Ok(()) => {
                    input.written += n as u64;
                    Self::live(&mut self.live, &self.live_dropped, input, take_no, &self.scratch);
                    self.after_write(input, rate)
                }
                Err(e) => {
                    let written = input.writer.as_ref().map_or(input.written, Writer::frames);
                    input.written = written;
                    let reason = if e.key() == "sndfile.write_failed" { StopReason::DiskFull } else { StopReason::WriteFailed };
                    Some((reason, e))
                }
            }
        };
        if let Some((reason, e)) = stop {
            let at = t.inputs[k].written;
            self.stop_input(a, k, reason, at, Some(e));
        }
        n as u64
    }

    /// Sync every ~2 s, reserve ahead; a failure stops the input
    fn after_write(&mut self, i: &mut OpenInput, rate: u32) -> Option<(StopReason, Error)> {
        if (i.written - i.last_sync) as f64 >= SYNC_SECONDS * f64::from(rate) {
            i.last_sync = i.written;
            if let Some(w) = i.writer.as_mut()
                && let Err(e) = w.sync()
            {
                return Some((StopReason::WriteFailed, e));
            }
            if let Some(r) = i.record.as_mut() {
                let _ = r.sync();
            }
        }
        self.reserve(i, rate).map(|e| (StopReason::DiskFull, e))
    }

    fn live(live: &mut Producer<LivePeak>, dropped: &AtomicU64, i: &mut OpenInput, take: u32, samples: &[f32]) {
        let ch = i.channels;
        let shown = ch.min(LIVE_PEAK_CHANNELS);
        let p = &mut i.peak;
        for frame in samples.chunks_exact(ch) {
            for c in 0..shown {
                let x = if frame[c].is_finite() { frame[c] } else { 0.0 };
                p.min[c] = p.min[c].min(x);
                p.max[c] = p.max[c].max(x);
            }
            p.in_pair += 1;
            if p.in_pair == crate::peaks::BASE {
                Self::emit_pair(live, dropped, i.index, take, p, shown);
            }
        }
    }

    fn emit_pair(live: &mut Producer<LivePeak>, dropped: &AtomicU64, input: InputIndex, take: u32, p: &mut LivePeaker, shown: usize) {
        let mut out = LivePeak { input, take, pair: p.pair, channels: shown as u8, min: [0; 8], max: [0; 8] };
        for c in 0..shown {
            out.min[c] = q(p.min[c], false);
            out.max[c] = q(p.max[c], true);
            p.min[c] = f32::INFINITY;
            p.max[c] = f32::NEG_INFINITY;
        }
        p.pair += 1;
        p.in_pair = 0;
        if live.push(out).is_err() {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn silence(&mut self, a: &mut Active, k: usize, frames: u64) {
        let rate = a.recv.plan.rate;
        let Some(t) = a.take.as_mut() else { return };
        let take_no = t.take_no;
        let input = &mut t.inputs[k];
        if input.stopped.is_some() || input.writer.is_none() {
            return;
        }
        let ch = input.channels;
        let per = (self.zeros.len() / ch).max(1);
        let mut left = frames;
        let mut stop = None;
        while left > 0 {
            let n = (left as usize).min(per);
            let zeros = &self.zeros[..n * ch];
            if let Some(w) = input.writer.as_mut()
                && let Err(e) = w.write_f32(zeros)
            {
                input.written = w.frames();
                stop = Some((StopReason::DiskFull, e));
                break;
            }
            input.written += n as u64;
            Self::live(&mut self.live, &self.live_dropped, input, take_no, zeros);
            left -= n as u64;
        }
        if stop.is_none() {
            stop = self.after_write(input, rate);
        }
        if let Some((reason, e)) = stop {
            let at = t.inputs[k].written;
            self.stop_input(a, k, reason, at, Some(e));
        }
    }

    /// Closes input `k`'s file at `file_frame` (the last good block) and reports the exact time (I-11)
    fn stop_input(&mut self, a: &mut Active, k: usize, reason: StopReason, file_frame: u64, error: Option<Error>) {
        let rate = a.recv.plan.rate;
        let Some(t) = a.take.as_mut() else { return };
        let start = t.start_sample;
        let input = &mut t.inputs[k];
        if input.stopped.is_some() {
            return;
        }
        let stop = TakeStop { reason, file_frame };
        input.stopped = Some(stop);
        let mut close_error = None;
        if let Some(w) = input.writer.take() {
            match w.close() {
                Ok(frames) => input.written = frames,
                Err(e) => close_error = Some(e),
            }
        }
        append(input, &RecordLine::Stop(stop));
        if let Some(r) = input.record.as_mut() {
            let _ = r.sync();
        }
        let index = input.index;
        self.emit(DiskEvent::InputStopped {
            input: index,
            reason,
            file_frame,
            sample: start + file_frame as i64,
            seconds: file_frame as f64 / f64::from(rate),
            error: error.or(close_error),
        });
    }

    fn end_take(&mut self, a: &mut Active, file_frame: u64, allow: &mut [u64]) {
        let Some(n) = a.take.as_ref().map(|t| t.inputs.len()) else { return };
        for k in 0..n {
            let used = self.drain(a, k, file_frame, u64::MAX);
            allow[k] = allow[k].saturating_sub(used);
        }
        self.close_take(a);
    }

    fn close_take(&mut self, a: &mut Active) {
        let Some(mut t) = a.take.take() else { return };
        let plan = &a.recv.plan;
        for (k, mut input) in t.inputs.drain(..).enumerate() {
            let mut frames = input.written;
            if let Some(w) = input.writer.take() {
                match w.close() {
                    Ok(f) => frames = f,
                    Err(error) => self.pending.push_back(DiskEvent::Failed { input: Some(input.index), error }),
                }
            }
            if input.record.is_none() {
                continue;
            }
            append(&mut input, &RecordLine::End { frames });
            if let Some(r) = input.record.as_mut() {
                let _ = r.sync();
            }
            let record = input.record.as_ref().map(|r| r.path.clone()).unwrap_or_default();
            let clip = |v: u64| v.min(frames);
            let take = Take {
                input: input.index,
                path: input.path.clone(),
                record,
                start_sample: t.start_sample,
                frames: frames as i64,
                channels: input.channels,
                rate: plan.rate,
                spans: t
                    .spans
                    .iter()
                    .filter(|s| s.file_frame < frames)
                    .map(|s| TakeSpan { frames: clip(s.file_frame + s.frames) - s.file_frame, ..*s })
                    .collect(),
                cuts: t.cuts.iter().filter(|c| c.file_frame <= frames).copied().collect(),
                gaps: t.gaps.iter().filter(|g| g.file_frame < frames).copied().collect(),
                stopped: input.stopped,
                latency: plan.latency,
                pdc: plan.pdc,
                tracks: plan.inputs[k].tracks.clone(),
            };
            self.pending.push_back(DiskEvent::TakeClosed(take));
        }
        self.flush_events();
    }
}

fn append(i: &mut OpenInput, line: &RecordLine) {
    if let Some(r) = i.record.as_mut() {
        // A record that cannot be appended to is reported at the close (its sync fails too); the audio goes on
        let _ = r.append(line);
    }
}

/// Sizes a recording ring for `seconds` (C4); for the tests and the session's own accounting
pub fn ring_frames(rate: u32, seconds: f32) -> usize {
    capacity_for(f64::from(rate), f64::from(seconds), 1024)
}
