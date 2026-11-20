//! The disk streamer (decisoes-autonomas.md P-32; arquitetura-nova.md: the legacy `DiskReader`): audio items longer
//! than 30 s play from disk. The compiler makes an `lm_render::stream` per played segment; the session hands its
//! [`StreamFeeder`] to the [`DiskReader`] with the resource's file. Its own thread ("lm-stream", role `Disk`, never
//! the recording's, U6) answers each feeder's requests (the chunk nearest the playhead first, then the loop cue) with
//! a seek and a read, and drops a feeder once its program side is gone.
//!
//! The file must be at the project's rate: imports are converted when they enter `audio/` (arquitetura-nova.md, item
//! 8), so another rate is a foreign file and is refused (`io.stream.rate_mismatch`).

use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use lm_base::ring::{Consumer, Producer, ring};
use lm_base::{Error, Result, Role};
use lm_render::StreamFeeder;
use lm_sys_sndfile::Reader;

/// What the streamer reports. Lossless (a local queue holds what does not fit)
#[derive(Debug)]
pub enum StreamEvent {
    /// A file could not be opened or read: the item plays silence (and the renderer counts underruns)
    Failed { path: PathBuf, error: Error },
}

struct Served {
    path: PathBuf,
    reader: Reader,
    feeder: StreamFeeder,
    interleaved: Vec<f32>,
    planar: Vec<f32>,
}

/// The streamer's thread
pub struct DiskReader {
    serve: Producer<(PathBuf, StreamFeeder)>,
    events: Consumer<StreamEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for DiskReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskReader").field("running", &self.thread.is_some()).finish()
    }
}

/// New feeders at once: the compiler's streams of one install
const SERVE_RING: usize = 1024;
const EVENT_RING: usize = 256;
const IDLE: Duration = Duration::from_millis(2);

impl DiskReader {
    pub fn start() -> Result<DiskReader> {
        let (serve, rx) = ring(SERVE_RING);
        let (tx, events) = ring(EVENT_RING);
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::Builder::new()
            .name("lm-stream".to_owned())
            .spawn(move || {
                lm_base::set_role(Role::Disk);
                run(rx, tx, &s);
            })
            .map_err(|e| Error::new("io.stream.thread_failed").arg("detail", e.to_string()))?;
        Ok(DiskReader { serve, events, stop, thread: Some(thread) })
    }

    /// Serves `feeder` from `path` (the resource's file, at the project's rate) until its program side is gone
    pub fn serve(&mut self, path: PathBuf, feeder: StreamFeeder) -> Result<()> {
        self.serve.push((path, feeder)).map_err(|_| Error::new("io.stream.busy"))?;
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
        Ok(())
    }

    /// From the pump
    pub fn poll(&mut self, mut f: impl FnMut(StreamEvent)) {
        while let Ok(e) = self.events.pop() {
            f(e);
        }
    }
}

impl Drop for DiskReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

fn open(path: PathBuf, feeder: StreamFeeder) -> std::result::Result<Served, (PathBuf, Error)> {
    let reader = match Reader::open(&path) {
        Ok(r) => r,
        Err(e) => return Err((path, e)),
    };
    let info = reader.info();
    if info.rate != feeder.rate() {
        let e = Error::new("io.stream.rate_mismatch")
            .arg("file", i64::from(info.rate))
            .arg("project", i64::from(feeder.rate()));
        return Err((path, e));
    }
    if info.channels != feeder.channels() {
        let e = Error::new("io.stream.channels_mismatch")
            .arg("file", info.channels as i64)
            .arg("stream", feeder.channels() as i64);
        return Err((path, e));
    }
    let n = feeder.chunk_frames() * feeder.channels();
    Ok(Served { path, reader, feeder, interleaved: vec![0.0; n], planar: vec![0.0; n] })
}

/// Reads one chunk; Ok(false) when nothing was wanted
fn serve_one(s: &mut Served) -> Result<bool> {
    let Some(c) = s.feeder.next_request() else { return Ok(false) };
    let chunk = s.feeder.chunk_frames();
    let ch = s.feeder.channels();
    let start = c * chunk as u64;
    let len = (s.feeder.frames().saturating_sub(start)).min(chunk as u64) as usize;
    if len == 0 {
        return Ok(false);
    }
    s.reader.seek(start)?;
    let mut got = 0;
    while got < len {
        let n = s.reader.read_f32(&mut s.interleaved[got * ch..len * ch])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    // A file shorter than its header (a recovered take): the rest is silence
    s.interleaved[got * ch..len * ch].fill(0.0);
    for f in 0..len {
        for c in 0..ch {
            s.planar[c * len + f] = s.interleaved[f * ch + c];
        }
    }
    s.feeder.write(c, &s.planar[..len * ch])?;
    Ok(true)
}

fn run(mut rx: Consumer<(PathBuf, StreamFeeder)>, mut tx: Producer<StreamEvent>, stop: &AtomicBool) {
    let mut served: Vec<Served> = Vec::new();
    let mut pending: VecDeque<StreamEvent> = VecDeque::new();
    while !stop.load(Ordering::Acquire) {
        while let Ok((path, feeder)) = rx.pop() {
            match open(path, feeder) {
                Ok(s) => served.push(s),
                Err((path, error)) => pending.push_back(StreamEvent::Failed { path, error }),
            }
        }
        served.retain(|s| !s.feeder.is_abandoned());
        // Round robin: one chunk per stream per pass, so no stream starves another
        let mut busy = false;
        let mut k = 0;
        while k < served.len() {
            match serve_one(&mut served[k]) {
                Ok(did) => {
                    busy |= did;
                    k += 1;
                }
                Err(error) => {
                    let s = served.swap_remove(k);
                    pending.push_back(StreamEvent::Failed { path: s.path, error });
                }
            }
        }
        while let Some(e) = pending.pop_front() {
            if let Err(rtrb::PushError::Full(e)) = tx.push(e) {
                pending.push_front(e);
                break;
            }
        }
        if !busy {
            std::thread::park_timeout(IDLE);
        }
    }
}
