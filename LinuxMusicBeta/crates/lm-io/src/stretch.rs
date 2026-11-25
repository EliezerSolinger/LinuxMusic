//! The rendering of a take that follows the tempo (F-REC-090), inside an `Operation`: the ORIGINAL file read, its
//! key frames stretched by Rubber Band's R3 engine (offline, two passes), into a file of exactly the frames asked for
//! (silence before the first key frame and after the last), `render/<key>.wav` (RF64, 32-bit float) in the project.
//! Written to a `.tmp` of its own and renamed whole: a rendering is there complete or not at all.
//!
//! The renderings are a cache named by the hash of their plan (`lm-compile`'s `stretch`): they can be removed at any
//! time and are made again from the originals.

use std::path::{Path, PathBuf};

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_sys_rubberband::{Format, Offline, Settings};
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
    let result = write(job, &mut reader, info.frames, channels, info.rate, &tmp, cancel, progress);
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

#[allow(clippy::too_many_arguments)]
fn write(
    job: &StretchJob,
    reader: &mut Reader,
    source_frames: u64,
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
            following(source.reader, &mut sink, source_frames, format, &job.key_frames, pitch, cancel, progress)?;
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

/// Input frames rendered on each side of a piece beyond its own (the engine's context, kept out of the result)
const MARGIN: u64 = 8_192;
/// Output frames of the crossfade on each side of a key frame between two pieces (at most a third of a piece)
const FADE: u64 = 256;

/// One stretch of the map with one ratio: input `a..b` to output `aa..bb`
#[derive(Clone, Copy, Debug)]
struct Piece {
    a: u64,
    b: u64,
    aa: u64,
    bb: u64,
}

impl Piece {
    fn ratio(&self) -> f64 {
        (self.bb - self.aa) as f64 / (self.b - self.a) as f64
    }
}

/// The pieces between the key frames; neighbours whose ratios differ by less than one part in 10⁶ (the rounding of
/// the frames) are one piece
fn pieces(points: &[(u64, u64)]) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for w in points.windows(2) {
        let p = Piece { a: w[0].0, b: w[1].0, aa: w[0].1, bb: w[1].1 };
        if let Some(last) = out.last_mut()
            && (last.ratio() / p.ratio() - 1.0).abs() < 1e-6
        {
            last.b = p.b;
            last.bb = p.bb;
            continue;
        }
        out.push(p);
    }
    out
}

/// Several ratios (a tempo change, a ramp): each piece of the map stretched on its own by the offline engine at its
/// ratio, with [`MARGIN`] frames of the source on each side for context, its result placed exactly between its key
/// frames, and two neighbours crossfaded over [`FADE`] frames on each side of the key frame between them (equal
/// gain: they are the same sound at nearly the same rate). Each key frame is where the plan says, by construction.
///
/// (The library's own key-frame map, offline, misses: with R3 a squeezed piece followed by a stretched one comes out
/// several percent short and ends early; measured, decisoes-autonomas.md.)
#[allow(clippy::too_many_arguments)]
fn following(
    reader: &mut Reader,
    sink: &mut Sink<'_>,
    source_frames: u64,
    format: Format,
    points: &[(u64, u64)],
    pitch: f64,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<()> {
    let channels = format.channels;
    let list = pieces(points);
    let n = list.len();
    // The crossfade around the key frame after piece k
    let fade = |k: usize| -> u64 {
        let len = |p: &Piece| p.bb - p.aa;
        FADE.min(len(&list[k]) / 3).min(len(&list[k + 1]) / 3)
    };
    let total: u64 = list.iter().map(|p| p.b - p.a).sum();
    let mut done_in = 0u64;
    let search = u64::from(format.sample_rate / 100);
    let mut placer =
        Placer { channels, pending: Vec::new(), next: Vec::new(), head: Vec::new(), head_g: 0, aligned: true };
    let mut buf = vec![0.0f32; CHUNK * channels];
    let mut out = Vec::with_capacity(CHUNK * channels * 2);
    for (k, p) in list.iter().enumerate() {
        let fin = if k > 0 { fade(k - 1) } else { 0 };
        let fout = if k + 1 < n { fade(k) } else { 0 };
        let r = p.ratio();
        let extra = |f: u64| (f as f64 / r).ceil() as u64;
        // The run: the piece, its fades and the margins, inside the source
        let s = p.a.saturating_sub(MARGIN + extra(fin));
        let e = (p.b + MARGIN + extra(fout)).min(source_frames).max(p.b);
        let run_out = ((e - s) as f64 * r).round() as usize;
        let mut run = Run {
            g: p.aa as i64 - ((p.a - s) as f64 * r).round() as i64,
            cursor: p.aa - fin,
            aa: p.aa,
            bb: p.bb,
            fin,
            fout,
            search,
        };
        placer.begin(&run);
        let mut stretcher = Offline::new(format, (e - s) as usize, run_out, pitch, &settings())?;
        for pass in 0..2 {
            reader.seek(s.min(source_frames))?;
            let mut at = s;
            while at < e {
                cancel.check()?;
                let m = ((e - at) as usize).min(CHUNK);
                let real = m.min(source_frames.saturating_sub(at) as usize);
                let got = if real > 0 { reader.read_f32(&mut buf[..real * channels])? } else { 0 };
                // (a file shorter than it said: silence for the rest)
                buf[got * channels..m * channels].fill(0.0);
                at += m as u64;
                if pass == 0 {
                    stretcher.study(&buf[..m * channels])?;
                } else {
                    stretcher.process(&buf[..m * channels], &mut out)?;
                    placer.place(&mut run, &out, sink)?;
                    out.clear();
                }
                let piece = f64::from(stretcher.fraction()) * (p.b - p.a) as f64;
                progress.set(((done_in as f64 + piece) / total.max(1) as f64 * 0.98) as f32);
            }
        }
        stretcher.finish(&mut out)?;
        placer.place(&mut run, &out, sink)?;
        out.clear();
        placer.align(&mut run, sink)?;
        // What the run did not reach (it ends at the source's end): silence
        placer.silence_until(&mut run, p.bb + fout, sink)?;
        std::mem::swap(&mut placer.pending, &mut placer.next);
        placer.next.clear();
        done_in += p.b - p.a;
    }
    Ok(())
}

/// Where a run's frames go: the place in the rendering of its next frame (`g`), the next place not filled yet
/// (`cursor`), and the piece's key frames with its fades
#[derive(Clone, Copy, Debug)]
struct Run {
    g: i64,
    cursor: u64,
    aa: u64,
    bb: u64,
    fin: u64,
    fout: u64,
    /// How far the piece may move to meet the previous one in phase
    search: u64,
}

/// The rendering's frames as the pieces make them: written in order, the fades mixed
struct Placer {
    channels: usize,
    /// The previous piece's frames in its fade-out (around this piece's first key frame)
    pending: Vec<f32>,
    /// This piece's frames in its fade-out, for the next
    next: Vec<f32>,
    /// A piece's first frames, kept until it is placed in phase with the previous one, and the place of the first
    head: Vec<f32>,
    head_g: i64,
    aligned: bool,
}

impl Placer {
    /// A piece starts: the first is placed as it comes, the others first meet the previous one in phase
    fn begin(&mut self, run: &Run) {
        self.head.clear();
        self.head_g = run.g;
        self.aligned = run.fin == 0 || self.pending.is_empty();
    }

    /// Places the run's frames `frames` (interleaved), from its place `run.g` on: those before the piece's fade-in
    /// are context, a gap before the first one (a run cut by the source's start) is silence. Until the piece is in
    /// phase with the previous one, its frames are kept
    fn place(&mut self, run: &mut Run, frames: &[f32], sink: &mut Sink<'_>) -> Result<()> {
        if !self.aligned {
            self.head.extend_from_slice(frames);
            run.g += (frames.len() / self.channels) as i64;
            if run.g >= (run.aa + run.fin + run.search) as i64 {
                self.align(run, sink)?;
            }
            return Ok(());
        }
        self.put(run, frames, sink)
    }

    /// Moves the piece by up to `search` frames to where its frames around the key frame best match the previous
    /// piece's (their normalised correlation over the crossfade: two phase vocoders run apart meet in phase, not
    /// cancelling each other), then places what was kept. The key frame is then within `search` of its place
    fn align(&mut self, run: &mut Run, sink: &mut Sink<'_>) -> Result<()> {
        if self.aligned {
            return Ok(());
        }
        self.aligned = true;
        let ch = self.channels;
        let mono = |v: &[f32], i: usize| -> f64 { v[i * ch..(i + 1) * ch].iter().map(|x| f64::from(*x)).sum() };
        let len = self.pending.len() / ch;
        let start = (run.aa - run.fin) as i64;
        let frames = (self.head.len() / ch) as i64;
        let prev: Vec<f64> = (0..len).map(|i| mono(&self.pending, i)).collect();
        let prev_energy: f64 = prev.iter().map(|x| x * x).sum();
        let mut best = (0i64, f64::MIN);
        // Every shift's match, from 0 outwards
        let mut matches: Vec<(i64, f64)> = Vec::new();
        if prev_energy > 0.0 {
            let d = run.search as i64;
            for shift in (0..=2 * d).map(|k| if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) }) {
                let (mut dot, mut energy) = (0.0, 0.0);
                for (i, p) in prev.iter().enumerate() {
                    let at = start + i as i64 - shift - self.head_g;
                    if (0..frames).contains(&at) {
                        let h = mono(&self.head, at as usize);
                        dot += p * h;
                        energy += h * h;
                    }
                }
                if energy > 0.0 {
                    let c = dot / (prev_energy * energy).sqrt();
                    matches.push((shift, c));
                    if c > best.1 {
                        best = (shift, c);
                    }
                }
            }
        }
        // The smallest move that matches nearly as well as the best (a steady tone matches every period alike)
        if let Some(near) = matches.iter().find(|m| m.1 >= best.1 - 0.02) {
            best = *near;
        }
        let head = std::mem::take(&mut self.head);
        run.g = self.head_g + best.0;
        self.put(run, &head, sink)?;
        self.head = head;
        self.head.clear();
        Ok(())
    }

    fn put(&mut self, run: &mut Run, frames: &[f32], sink: &mut Sink<'_>) -> Result<()> {
        for f in frames.chunks_exact(self.channels) {
            if run.g >= run.cursor as i64 {
                self.silence_until(run, run.g as u64, sink)?;
                self.frame(run, f, sink)?;
            }
            run.g += 1;
        }
        Ok(())
    }

    /// Silence at the places not filled yet, up to `until`
    fn silence_until(&mut self, run: &mut Run, until: u64, sink: &mut Sink<'_>) -> Result<()> {
        let zero = [0.0f32; lm_sys_rubberband::MAX_CHANNELS];
        while run.cursor < until {
            self.frame(run, &zero[..self.channels], sink)?;
        }
        Ok(())
    }

    /// The frame at the run's cursor: written (mixed with the previous piece in the fade-in), kept for the next
    /// piece (the fade-out), or past the piece and its fade
    fn frame(&mut self, run: &mut Run, f: &[f32], sink: &mut Sink<'_>) -> Result<()> {
        let g = run.cursor;
        run.cursor += 1;
        if g >= run.bb + run.fout {
            return Ok(());
        }
        if g >= run.bb - run.fout {
            self.next.extend_from_slice(f);
            return Ok(());
        }
        debug_assert_eq!(g, *sink.written, "the rendering is written in order");
        if g < run.aa + run.fin {
            // The fade from the previous piece: raised cosines summing to one
            let i = (g - (run.aa - run.fin)) as usize;
            let t = (i as f64 + 0.5) / (2 * run.fin) as f64;
            let w = (0.5 - 0.5 * (std::f64::consts::PI * t).cos()) as f32;
            let mut mixed = [0.0f32; lm_sys_rubberband::MAX_CHANNELS];
            for (c, v) in f.iter().enumerate() {
                let prev = self.pending.get(i * self.channels + c).copied().unwrap_or(0.0);
                mixed[c] = prev * (1.0 - w) + v * w;
            }
            sink.write(&mixed[..self.channels], self.channels)?;
        } else {
            sink.write(f, self.channels)?;
        }
        Ok(())
    }
}
