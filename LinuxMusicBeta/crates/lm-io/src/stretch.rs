//! The rendering of a take that follows the tempo (F-REC-090), inside an `Operation`: the ORIGINAL file read, its
//! key frames stretched by Rubber Band's R3 engine (offline, two passes), into a file of exactly the frames asked for
//! (silence before the first key frame and after the last), `render/<key>.wav` (RF64, 32-bit float) in the project.
//! Written to a `.tmp` of its own and renamed whole: a rendering is there complete or not at all.
//!
//! The renderings are a cache named by the hash of their plan (`lm-compile`'s `stretch`): they can be removed at any
//! time and are made again from the originals.

use std::path::{Path, PathBuf};

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_sys_rubberband::{Format, Offline, Realtime, Settings};
use lm_sys_sndfile::{Existing, Reader, Sample, WriteFormat, WriteSpec, Writer};

use crate::resources::is_valid_key;
use crate::take::{free_space, io_error, new_uuid, sync_dir};

/// Frames read or written at a time
const CHUNK: usize = 16_384;

/// The project's folder of renderings
pub fn render_folder(project: &Path) -> PathBuf {
    project.join("render")
}

/// The file of rendering `key` in the project (whether it is there or not)
pub fn render_path(project: &Path, key: &str) -> PathBuf {
    render_folder(project).join(format!("{key}.wav"))
}

/// A rendering to make
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StretchJob {
    /// The original's file
    pub source: PathBuf,
    pub project: PathBuf,
    /// The plan's hash (64 hex digits)
    pub key: String,
    /// (source frame, output frame), strictly increasing; empty: silence only
    pub key_frames: Vec<(u64, u64)>,
    pub output_frames: u64,
    /// The rate of the rendering (the project's)
    pub rate: u32,
}

/// The stretch's sound: R3 ("finer"), the library's defaults (decisoes-autonomas: R3 for the tempo)
pub fn settings() -> Settings {
    Settings::default()
}

/// Renders `job` into `render/<key>.wav` (a rendering already there is kept). Cancellable, with progress; a cancel
/// or a failure leaves nothing behind. Errors: `io.stretch.invalid_key`, `io.stretch.bad_plan` (key frames outside the
/// source or the output), `io.stretch.disk_full`, the reader's, the writer's and Rubber Band's
pub fn render_stretch(job: &StretchJob, cancel: &Cancel, progress: &Progress) -> Result<PathBuf> {
    if !is_valid_key(&job.key) {
        return Err(Error::new("io.stretch.invalid_key"));
    }
    let target = render_path(&job.project, &job.key);
    if target.exists() {
        return Ok(target);
    }
    progress.stage(Label::new("io.stretch.rendering"));
    let mut reader = Reader::open(&job.source)?;
    let info = *reader.info();
    let channels = info.channels;
    if let (Some(first), Some(last)) = (job.key_frames.first(), job.key_frames.last())
        && (last.0 > info.frames || last.1 > job.output_frames || first.0 >= last.0 || first.1 >= last.1)
    {
        return Err(Error::new("io.stretch.bad_plan").arg("path", job.source.display().to_string()));
    }
    let folder = render_folder(&job.project);
    std::fs::create_dir_all(&folder).map_err(|e| io_error("io.stretch.write_failed", &folder, &e))?;
    let needed = job.output_frames * channels as u64 * 4 + (1 << 20);
    if free_space(&folder)? < needed {
        return Err(Error::new("io.stretch.disk_full").arg("needed", needed as i64));
    }
    let tmp = folder.join(format!("{}.{}.tmp", job.key, new_uuid()));
    let result = write(job, &mut reader, channels, info.rate, &tmp, cancel, progress);
    match result {
        Ok(()) => {
            std::fs::rename(&tmp, &target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                io_error("io.stretch.write_failed", &target, &e)
            })?;
            sync_dir(&folder);
            progress.set(1.0);
            Ok(target)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn write(
    job: &StretchJob,
    reader: &mut Reader,
    channels: usize,
    source_rate: u32,
    tmp: &Path,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<()> {
    let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate: job.rate, channels };
    let mut writer = Writer::create(tmp, spec, Existing::Fail)?;
    let silence = vec![0.0f32; CHUNK * channels];
    let mut written = 0u64;
    let pad = |writer: &mut Writer, until: u64, written: &mut u64| -> Result<()> {
        while *written < until {
            let n = (until - *written).min(CHUNK as u64) as usize;
            writer.write_f32(&silence[..n * channels])?;
            *written += n as u64;
        }
        Ok(())
    };
    if let (Some(&(in0, out0)), Some(&(in1, out1))) = (job.key_frames.first(), job.key_frames.last()) {
        pad(&mut writer, out0, &mut written)?;
        // The source plays at its rate, the rendering at the project's: the pitch scaled back by their ratio
        let pitch = f64::from(source_rate) / f64::from(job.rate);
        let format = Format::new(source_rate, channels);
        reader.seek(in0)?;
        let mut source = Source { reader, channels, left: in1 - in0 };
        let mut sink = Sink { writer: &mut writer, written: &mut written, end: out1 };
        if job.key_frames.len() == 2 {
            one_ratio(&mut source, &mut sink, format, (out1 - out0) as usize, pitch, cancel, progress)?;
        } else {
            let points: Vec<(u64, u64)> = job.key_frames.iter().map(|(a, b)| (a - in0, b - out0)).collect();
            following(&mut source, &mut sink, format, &points, pitch, cancel, progress)?;
        }
        debug_assert_eq!(written, out1, "exactly the frames of the key frames");
    }
    pad(&mut writer, job.output_frames, &mut written)?;
    writer.sync()?;
    let frames = writer.close()?;
    debug_assert_eq!(frames, job.output_frames);
    Ok(())
}

/// The source's frames still to read, as they are read (a file shorter than it said gives silence)
struct Source<'a> {
    reader: &'a mut Reader,
    channels: usize,
    left: u64,
}

impl Source<'_> {
    /// Up to `frames` interleaved frames into `buf`; how many
    fn read(&mut self, buf: &mut [f32], frames: usize) -> Result<usize> {
        let n = (self.left as usize).min(frames);
        let got = self.reader.read_f32(&mut buf[..n * self.channels])?;
        buf[got * self.channels..n * self.channels].fill(0.0);
        self.left -= n as u64;
        Ok(n)
    }
}

/// The rendering's file, up to its last key frame
struct Sink<'a> {
    writer: &'a mut Writer,
    written: &'a mut u64,
    end: u64,
}

impl Sink<'_> {
    /// Writes interleaved frames, no further than the last key frame; true once there
    fn write(&mut self, frames: &[f32], channels: usize) -> Result<bool> {
        let n = ((self.end - *self.written) as usize).min(frames.len() / channels);
        self.writer.write_f32(&frames[..n * channels])?;
        *self.written += n as u64;
        Ok(*self.written >= self.end)
    }
}

/// One ratio from the first key frame to the last: the offline engine, two passes (the best of the library, its
/// length exact)
fn one_ratio(
    source: &mut Source<'_>,
    sink: &mut Sink<'_>,
    format: Format,
    outputs: usize,
    pitch: f64,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<()> {
    let channels = format.channels;
    let inputs = source.left as usize;
    let start = source.reader.position();
    let mut stretcher = Offline::new(format, inputs, outputs, pitch, &settings())?;
    let mut buf = vec![0.0f32; CHUNK * channels];
    while source.left > 0 {
        cancel.check()?;
        let n = source.read(&mut buf, CHUNK)?;
        stretcher.study(&buf[..n * channels])?;
        progress.set(stretcher.fraction() * 0.98);
    }
    source.reader.seek(start)?;
    source.left = inputs as u64;
    let mut out = Vec::with_capacity(CHUNK * channels * 2);
    while source.left > 0 {
        cancel.check()?;
        let n = source.read(&mut buf, CHUNK)?;
        stretcher.process(&buf[..n * channels], &mut out)?;
        sink.write(&out, channels)?;
        out.clear();
        progress.set(stretcher.fraction() * 0.98);
    }
    stretcher.finish(&mut out)?;
    sink.write(&out, channels)?;
    Ok(())
}

/// Frames given to the real-time engine at a time
const BLOCK: usize = 1024;

/// Several ratios (a tempo change, a ramp): the engine follows the key frames, its ratio set for each block to the
/// ratio of the piece of the map the block is in, every block cut at the key frames. (The library's own key-frame
/// map, offline, misses: with R3 a squeezed piece followed by a stretched one comes out several percent short and
/// its end early; measured, `decisoes-autonomas.md`.) The engine's delay is fed as silence first and dropped from the
/// output; the input's end is followed by silence until the last key frame is reached
fn following(
    source: &mut Source<'_>,
    sink: &mut Sink<'_>,
    format: Format,
    points: &[(u64, u64)],
    pitch: f64,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<()> {
    let channels = format.channels;
    let total_in = source.left;
    let ratio_at = |i: usize| {
        let (a, b) = (points[i], points[i + 1]);
        (b.1 - a.1) as f64 / (b.0 - a.0) as f64
    };
    let mut stretcher = Realtime::new(format, BLOCK, ratio_at(0), pitch, &settings())?;
    let mut planar_in = vec![vec![0.0f32; BLOCK]; channels];
    let mut planar_out = vec![vec![0.0f32; BLOCK * 4]; channels];
    let mut interleaved = vec![0.0f32; BLOCK * channels];
    let mut out = Vec::with_capacity(BLOCK * 4 * channels);
    let mut drop = stretcher.start_delay();
    let mut done = false;

    // Feeds n frames from `interleaved` (or silence), then moves what came out to the file
    let mut feed = |stretcher: &mut Realtime, frames: Option<&[f32]>, n: usize, drop: &mut usize, done: &mut bool| -> Result<()> {
        for (c, plane) in planar_in.iter_mut().enumerate() {
            for (f, v) in plane[..n].iter_mut().enumerate() {
                *v = frames.map_or(0.0, |x| x[f * channels + c]);
            }
        }
        let input: Vec<&[f32]> = planar_in.iter().map(|p| &p[..n]).collect();
        stretcher.process(&input)?;
        loop {
            let ready = stretcher.available().min(BLOCK * 4);
            if ready == 0 {
                return Ok(());
            }
            let mut output: Vec<&mut [f32]> = planar_out.iter_mut().map(|p| &mut p[..ready]).collect();
            let got = stretcher.retrieve(&mut output)?;
            let skip = (*drop).min(got);
            *drop -= skip;
            out.clear();
            for f in skip..got {
                for plane in &planar_out {
                    out.push(plane[f]);
                }
            }
            if !*done {
                *done = sink.write(&out, channels)?;
            }
        }
    };

    // The engine's preferred lead-in, as silence (its output is part of the delay dropped)
    let mut pad = stretcher.preferred_start_pad();
    while pad > 0 {
        let n = pad.min(BLOCK);
        feed(&mut stretcher, None, n, &mut drop, &mut done)?;
        pad -= n;
    }
    // The engine works on the frames it was given some time before (its delay, in input frames): each ratio is set
    // when the frames it is for reach it
    let lag = std::env::var("LAG").ok().and_then(|v| v.parse().ok()).unwrap_or(stretcher.start_delay() as u64); eprintln!("start delay {} pad {}", stretcher.start_delay(), stretcher.preferred_start_pad());
    let mut at = 0u64;
    let mut piece = 0;
    let mut guard = 0u64;
    while !done {
        cancel.check()?;
        let there = at.saturating_sub(lag);
        while piece + 2 < points.len() && there >= points[piece + 1].0 {
            piece += 1;
        }
        stretcher.set_time_ratio(ratio_at(piece))?;
        let next = if piece + 2 < points.len() { points[piece + 1].0 + lag } else { u64::MAX };
        let n = (next - at).clamp(1, BLOCK as u64) as usize;
        let got = source.read(&mut interleaved, n)?;
        interleaved[got * channels..n * channels].fill(0.0);
        feed(&mut stretcher, Some(&interleaved), n, &mut drop, &mut done)?;
        at += n as u64;
        progress.set((at as f64 / total_in.max(1) as f64 * 0.98).min(0.98) as f32);
        guard += 1;
        if at > total_in + (1 << 26) || guard > 1 << 26 {
            return Err(Error::new("io.stretch.stalled"));
        }
    }
    Ok(())
}
