//! A take on disk (interfaces.md, I-11; U1, U5; C2, C10; H3; decisoes-autonomas.md P-12): what the session gets back,
//! the `recording.json` written beside the file before the first block, the finalisation (optional conversion,
//! BLAKE3 hash, peaks, rename) and the recovery of takes interrupted by a crash.
//!
//! # `recording.json` (U5)
//!
//! `audio/rec-<uuid>.recording.json`, beside `audio/rec-<uuid>.wav`, one JSON object per line: the header first
//! (written and synced before the first block), then one line per event as it happens (spans, loop cuts, gaps,
//! stops, the end). A crash can cut the last line; the reader stops at the first line it cannot parse. After the
//! finalisation it is renamed `audio/<hash>.recording.json`, and the session removes it ([`forget_record`]) once
//! the take is in its journal.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_sys_sndfile::{Existing, Reader, Sample, WriteFormat, WriteSpec, Writer};
use serde::{Deserialize, Serialize};

use crate::peaks::{Peaks, PeaksBuilder};
use crate::record::InputIndex;

/// Version of the `recording.json` format
pub const RECORD_VERSION: u32 = 1;

/// Part of the file that is the take proper (inside the punch range, U4): file frames `file_frame..+frames` are the
/// timeline samples `sample..+frames`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeSpan {
    pub file_frame: u64,
    pub sample: i64,
    pub frames: u64,
}

/// A loop wrap (U1): at `file_frame` the timeline jumped from `from_sample` back to `to_sample`; the session makes
/// one lane per pass
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCut {
    pub file_frame: u64,
    pub from_sample: i64,
    pub to_sample: i64,
}

/// Silence written for a capture xrun (C2): marked on the item
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeGap {
    pub file_frame: u64,
    pub frames: u64,
}

/// Why an input stopped before the take ended (I-11, H3): the file ends at `file_frame`, the last good block
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The ring was full: the disk did not keep up
    Overflow,
    /// The disk is full (the reservation ahead failed, or a write was short)
    DiskFull,
    /// Writing failed for another reason
    WriteFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeStop {
    pub reason: StopReason,
    pub file_frame: u64,
}

/// A recorded take of one input (I-11): the session turns it into the track's item or lane (T2)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Take {
    pub input: InputIndex,
    /// The audio file (`rec-<uuid>.wav` until finalised, then `<hash>.wav` or `.flac`)
    pub path: PathBuf,
    /// Its `recording.json`
    pub record: PathBuf,
    /// Timeline sample of the file's first frame (handles included, U4), before the latency is subtracted
    pub start_sample: i64,
    /// Frames in the file
    pub frames: i64,
    pub channels: usize,
    pub rate: u32,
    pub spans: Vec<TakeSpan>,
    pub cuts: Vec<LoopCut>,
    pub gaps: Vec<TakeGap>,
    pub stopped: Option<TakeStop>,
    /// From the plan (U5): latency and PDC used to place the take, and the tracks it records into
    pub latency: i64,
    pub pdc: i64,
    pub tracks: Vec<String>,
}

/// The header line of `recording.json`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordHeader {
    pub version: u32,
    pub uuid: String,
    pub input: u32,
    pub channels: Vec<u16>,
    pub tracks: Vec<String>,
    pub rate: u32,
    pub file: String,
    pub start_sample: i64,
    pub latency: i64,
    pub pdc: i64,
}

/// One line of `recording.json`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordLine {
    Header(RecordHeader),
    Span(TakeSpan),
    Cut(LoopCut),
    Gap(TakeGap),
    Stop(TakeStop),
    End { frames: u64 },
    /// Written by the finalisation: the file's new name
    Final { file: String },
}

/// The append-only `recording.json` of an open take
#[derive(Debug)]
pub(crate) struct RecordFile {
    file: File,
    pub(crate) path: PathBuf,
}

impl RecordFile {
    /// Creates it with the header, synced (with the folder) before anything is recorded (U5)
    pub(crate) fn create(path: &Path, header: &RecordHeader) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| io_error("io.record.create_failed", path, &e))?;
        let mut rf = Self { file, path: path.to_path_buf() };
        rf.append(&RecordLine::Header(header.clone()))?;
        rf.sync()?;
        if let Some(dir) = path.parent() {
            sync_dir(dir);
        }
        Ok(rf)
    }

    pub(crate) fn append(&mut self, line: &RecordLine) -> Result<()> {
        let mut text = serde_json::to_string(line).map_err(|e| Error::new("io.record.encode").arg("message", e.to_string()))?;
        text.push('\n');
        self.file.write_all(text.as_bytes()).map_err(|e| io_error("io.record.write_failed", &self.path, &e))
    }

    pub(crate) fn sync(&mut self) -> Result<()> {
        self.file.sync_data().map_err(|e| io_error("io.record.write_failed", &self.path, &e))
    }
}

pub(crate) fn io_error(key: &'static str, path: &Path, e: &std::io::Error) -> Error {
    Error::new(key).arg("path", path.display().to_string()).arg("message", e.to_string())
}

/// Syncs a folder so a new or renamed entry survives a crash (U6). Best effort: some file systems refuse it
pub(crate) fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

/// A new random id (version 4 UUID text) from the system's random source
pub fn new_uuid() -> String {
    let mut b = [0u8; 16];
    let filled = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_ok();
    if !filled {
        // No random source: the time and a counter are unique enough for a file name in one folder
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let t = lm_base::HostTime::now().0 ^ (u64::from(std::process::id()) << 32);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let h = blake3::hash(&[t.to_le_bytes(), n.to_le_bytes()].concat());
        b.copy_from_slice(&h.as_bytes()[..16]);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// Reads a `recording.json`: the header and the events up to the first line that does not parse (a crash cuts the
/// last one). `io.record.malformed` if the header is missing
pub fn read_record(path: &Path) -> Result<(RecordHeader, Vec<RecordLine>)> {
    let file = File::open(path).map_err(|e| io_error("io.record.read_failed", path, &e))?;
    let mut lines = BufReader::new(file).lines();
    let header = match lines.next() {
        Some(Ok(text)) => match serde_json::from_str::<RecordLine>(&text) {
            Ok(RecordLine::Header(h)) if h.version <= RECORD_VERSION => h,
            Ok(RecordLine::Header(h)) => {
                return Err(Error::new("io.record.newer_version").arg("version", i64::from(h.version)));
            }
            _ => return Err(Error::new("io.record.malformed").arg("path", path.display().to_string())),
        },
        _ => return Err(Error::new("io.record.malformed").arg("path", path.display().to_string())),
    };
    let mut events = Vec::new();
    for line in lines {
        let Ok(text) = line else { break };
        if text.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<RecordLine>(&text) {
            Ok(l) => events.push(l),
            Err(_) => break,
        }
    }
    Ok((header, events))
}

/// The take a `recording.json` describes, with the audio file's frames as written so far (`frames`)
pub(crate) fn take_from_record(record: &Path, header: &RecordHeader, events: &[RecordLine], frames: u64, channels: usize) -> Take {
    let mut take = Take {
        input: InputIndex(header.input),
        path: record.with_file_name(&header.file),
        record: record.to_path_buf(),
        start_sample: header.start_sample,
        frames: frames as i64,
        channels,
        rate: header.rate,
        spans: Vec::new(),
        cuts: Vec::new(),
        gaps: Vec::new(),
        stopped: None,
        latency: header.latency,
        pdc: header.pdc,
        tracks: header.tracks.clone(),
    };
    for e in events {
        match *e {
            RecordLine::Span(s) if s.file_frame < frames => {
                let mut s = s;
                s.frames = s.frames.min(frames - s.file_frame);
                match take.spans.last_mut() {
                    Some(last) if last.file_frame + last.frames == s.file_frame && last.sample + last.frames as i64 == s.sample => {
                        last.frames += s.frames;
                    }
                    _ => take.spans.push(s),
                }
            }
            RecordLine::Cut(c) if c.file_frame <= frames => take.cuts.push(c),
            RecordLine::Gap(g) if g.file_frame < frames => {
                let mut g = g;
                g.frames = g.frames.min(frames - g.file_frame);
                take.gaps.push(g);
            }
            RecordLine::Stop(s) => take.stopped = Some(s),
            RecordLine::Final { ref file } => take.path = record.with_file_name(file),
            _ => {}
        }
    }
    take
}

/// The `rec-<uuid>` takes left in `audio/` by a crash (C10): a `rec-*.recording.json` whose audio file exists and
/// whose record has no `end`. Listing a folder can be slow: inside an `Operation`
pub fn find_interrupted(audio: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(audio) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_error("io.record.read_failed", audio, &e)),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !(name.starts_with("rec-") && name.ends_with(".recording.json")) {
            continue;
        }
        let path = entry.path();
        let Ok((header, events)) = read_record(&path) else { continue };
        let ended = events.iter().any(|e| matches!(e, RecordLine::End { .. }));
        if !ended && path.with_file_name(&header.file).exists() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Recovers an interrupted take (I-11, U5): the audio reads back up to its last sync (~2 s), and the record says
/// where it goes. The take is then finalised like any other
pub fn recover_take(record: &Path, cancel: &Cancel, progress: &Progress) -> Result<Take> {
    progress.stage(Label::new("io.record.recovering"));
    cancel.check()?;
    let (header, events) = read_record(record)?;
    let audio = record.with_file_name(&header.file);
    let reader = Reader::open(&audio)?;
    let frames = reader.info().frames;
    let channels = reader.info().channels;
    drop(reader);
    let mut file = OpenOptions::new().append(true).open(record).map_err(|e| io_error("io.record.write_failed", record, &e))?;
    let line = serde_json::to_string(&RecordLine::End { frames }).map_err(|e| Error::new("io.record.encode").arg("message", e.to_string()))?;
    // A crash may have cut the last line: start the end on a line of its own
    file.write_all(format!("\n{line}\n").as_bytes()).map_err(|e| io_error("io.record.write_failed", record, &e))?;
    file.sync_data().map_err(|e| io_error("io.record.write_failed", record, &e))?;
    progress.set(1.0);
    Ok(take_from_record(record, &header, &events, frames, channels))
}

/// The format a take is kept in (P-12): always recorded as RF64 float; converted at the finalisation if asked
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TakeFormat {
    /// RF64 32-bit float, as recorded (no conversion)
    #[default]
    Float32,
    /// RF64 24-bit integer
    Int24,
    /// FLAC 24-bit
    Flac24,
}

impl TakeFormat {
    fn extension(self) -> &'static str {
        match self {
            TakeFormat::Float32 | TakeFormat::Int24 => "wav",
            TakeFormat::Flac24 => "flac",
        }
    }
}

/// A finalised take: the file is named by its BLAKE3 hash (C10), its peaks are written
#[derive(Clone, Debug)]
pub struct FinalizedTake {
    pub take: Take,
    /// The resource key: the BLAKE3 hash of the file, lowercase hex
    pub key: String,
    pub peaks_path: PathBuf,
    pub peaks: Arc<Peaks>,
}

/// Bytes read per step when hashing and building peaks
const CHUNK_FRAMES: usize = 65_536;

/// Finalises a take (I-11, C10, P-12) inside an `Operation` on the take: converts it if asked, hashes the file,
/// writes `<hash>.peaks`, renames the file to `<hash>.<ext>` and the record to `<hash>.recording.json`. Not
/// cancellable in effect (the take must close): a cancel is honoured only before anything was renamed
pub fn finalize_take(take: &Take, format: TakeFormat, cancel: &Cancel, progress: &Progress) -> Result<FinalizedTake> {
    let folder = take.path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut source = take.path.clone();
    if format != TakeFormat::Float32 {
        progress.stage(Label::new("io.take.converting"));
        let converted = folder.join(format!(
            "{}.convert.tmp",
            take.path.file_stem().and_then(|s| s.to_str()).unwrap_or("take")
        ));
        convert(&source, &converted, format, cancel, progress)?;
        source = converted;
    }
    progress.stage(Label::new("io.take.hashing"));
    let key = hash_file(&source, cancel, progress)?;
    progress.stage(Label::new("io.take.peaks"));
    let peaks = peaks_of_file(&source, cancel, progress)?;
    let peaks_path = folder.join(format!("{key}.peaks"));
    peaks.write(&peaks_path)?;
    let final_path = folder.join(format!("{key}.{}", format.extension()));
    if final_path.exists() {
        // The same content is already there (a take recorded twice bit for bit): keep the one there
        std::fs::remove_file(&source).map_err(|e| io_error("io.take.rename_failed", &source, &e))?;
    } else {
        std::fs::rename(&source, &final_path).map_err(|e| io_error("io.take.rename_failed", &source, &e))?;
    }
    if source != take.path && take.path.exists() {
        std::fs::remove_file(&take.path).map_err(|e| io_error("io.take.rename_failed", &take.path, &e))?;
    }
    let record = folder.join(format!("{key}.recording.json"));
    if take.record.exists() {
        let mut f = OpenOptions::new().append(true).open(&take.record).map_err(|e| io_error("io.record.write_failed", &take.record, &e))?;
        let file_name = final_path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let line = serde_json::to_string(&RecordLine::Final { file: file_name })
            .map_err(|e| Error::new("io.record.encode").arg("message", e.to_string()))?;
        f.write_all(format!("{line}\n").as_bytes()).map_err(|e| io_error("io.record.write_failed", &take.record, &e))?;
        f.sync_data().map_err(|e| io_error("io.record.write_failed", &take.record, &e))?;
        std::fs::rename(&take.record, &record).map_err(|e| io_error("io.take.rename_failed", &take.record, &e))?;
    }
    sync_dir(&folder);
    let mut out = take.clone();
    out.path = final_path;
    out.record = record;
    progress.set(1.0);
    Ok(FinalizedTake { take: out, key, peaks_path, peaks: Arc::new(peaks) })
}

/// Removes a finalised take's record once the take is in the session's journal
pub fn forget_record(record: &Path) -> Result<()> {
    match std::fs::remove_file(record) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error("io.record.remove_failed", record, &e)),
    }
}

/// The BLAKE3 hash of a file, lowercase hex (C10). Cancellable, with progress
pub fn hash_file(path: &Path, cancel: &Cancel, progress: &Progress) -> Result<String> {
    let mut file = File::open(path).map_err(|e| io_error("io.hash.read_failed", path, &e))?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0).max(1);
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        cancel.check()?;
        let n = file.read(&mut buf).map_err(|e| io_error("io.hash.read_failed", path, &e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        progress.set((done as f64 / total as f64) as f32);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// The peaks of an audio file, read in chunks. Cancellable, with progress
pub fn peaks_of_file(path: &Path, cancel: &Cancel, progress: &Progress) -> Result<Peaks> {
    let mut reader = Reader::open(path)?;
    let channels = reader.info().channels;
    let total = reader.info().frames.max(1);
    let mut builder = PeaksBuilder::new(channels);
    let mut buf = vec![0.0f32; CHUNK_FRAMES * channels];
    loop {
        cancel.check()?;
        let n = reader.read_f32(&mut buf)?;
        if n == 0 {
            break;
        }
        builder.push_interleaved(&buf[..n * channels]);
        progress.set((builder.frames() as f64 / total as f64).min(1.0) as f32);
    }
    Ok(builder.finish())
}

fn convert(from: &Path, to: &Path, format: TakeFormat, cancel: &Cancel, progress: &Progress) -> Result<()> {
    let mut reader = Reader::open(from)?;
    let info = reader.info().clone();
    let write = match format {
        TakeFormat::Float32 => WriteFormat::Rf64(Sample::Float32),
        TakeFormat::Int24 => WriteFormat::Rf64(Sample::Int24),
        TakeFormat::Flac24 => WriteFormat::Flac { depth: lm_sys_sndfile::FlacDepth::Int24, compression: 0.5 },
    };
    let spec = WriteSpec { format: write, rate: info.rate, channels: info.channels };
    let result = (|| {
        let mut writer = Writer::create(to, spec, Existing::Replace)?;
        let mut buf = vec![0.0f32; CHUNK_FRAMES * info.channels];
        let mut done = 0u64;
        loop {
            cancel.check()?;
            let n = reader.read_f32(&mut buf)?;
            if n == 0 {
                break;
            }
            writer.write_f32(&buf[..n * info.channels])?;
            done += n as u64;
            progress.set((done as f64 / info.frames.max(1) as f64) as f32);
        }
        writer.close()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(to);
    }
    result
}

/// Free space on the file system that holds `path`, in bytes (H3)
pub fn free_space(path: &Path) -> Result<u64> {
    let st = rustix::fs::statvfs(path).map_err(|e| {
        Error::new("io.disk.statvfs_failed").arg("path", path.display().to_string()).arg("errno", i64::from(e.raw_os_error()))
    })?;
    Ok(st.f_bavail.saturating_mul(st.f_frsize))
}

/// Room for recording (H3): how many seconds of the armed inputs fit
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpaceCheck {
    /// At least 10 minutes
    Enough { seconds: f64 },
    /// Less than 10 minutes: arm, with a warning
    Low { seconds: f64 },
    /// Less than 30 seconds: arming is refused (the user's decision)
    Refused { seconds: f64 },
}

/// Below this, arming warns (H3)
pub const WARN_SECONDS: f64 = 600.0;
/// Below this, arming is refused (H3)
pub const REFUSE_SECONDS: f64 = 30.0;

/// The bytes one second of `channels` channels takes on disk at `rate` (RF64 float)
pub fn record_bytes_per_second(channels: usize, rate: u32) -> u64 {
    channels as u64 * 4 * u64::from(rate)
}

/// What arming `channels` channels (all armed inputs together) at `rate` leaves on the disk of `folder` (H3)
pub fn check_record_space(folder: &Path, channels: usize, rate: u32) -> Result<SpaceCheck> {
    Ok(space_for(free_space(folder)?, channels, rate))
}

/// The same, for a known free space (tests, and the disk thread's own check)
pub fn space_for(free_bytes: u64, channels: usize, rate: u32) -> SpaceCheck {
    let bps = record_bytes_per_second(channels, rate).max(1);
    let seconds = free_bytes as f64 / bps as f64;
    if seconds < REFUSE_SECONDS {
        SpaceCheck::Refused { seconds }
    } else if seconds < WARN_SECONDS {
        SpaceCheck::Low { seconds }
    } else {
        SpaceCheck::Enough { seconds }
    }
}
