//! The export (I-13, T8, T19, U37): a function over a copy of the tree, with its own compiler and renderer and no
//! device. It waits for everything — every resource loaded, every processor built — before the first block, so a
//! node "waiting" never becomes silence in the file, and an export started while things still load gives the same
//! file as one started with everything ready. `lm-headless render` is exactly this path: the reference test of the
//! deterministic render (base item 9).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lm_base::{Cancel, Error, Label, Progress, Result};
use lm_compile::{CompilerConfig, CompilerCore, Environment, ResourceState, Resources};
use lm_plugin::Registry;
use lm_render::{Command, Limits, NodeCore, ProcessSpec, SampleData};
use lm_state::{Resource, SchemaRegistry, Value};
use lm_sys_sndfile::{Existing, Sample, WriteFormat, WriteSpec, Writer};

use crate::session::{build_processor, project_rate, tempo_map};

/// The block of an export: fixed, written in the report (C7)
pub const EXPORT_BLOCK: usize = 512;
/// Compilations an export waits through before it gives up (each one builds what the last asked for)
const MAX_ROUNDS: usize = 64;

/// What to export
#[derive(Clone, Debug, PartialEq)]
pub struct ExportTarget {
    pub path: PathBuf,
    pub format: WriteFormat,
    /// Timeline samples `from..to`; `None`: from the start to the end of the last item, plus the tail
    pub range: Option<(i64, i64)>,
    /// Seconds rendered after the end (the reverbs' tails)
    pub tail: f64,
}

impl ExportTarget {
    /// A 32-bit float WAV of the whole song with 2 s of tail
    pub fn wav(path: PathBuf) -> Self {
        Self { path, format: WriteFormat::Wav(Sample::Float32), range: None, tail: 2.0 }
    }
}

/// Every audio resource cited, loaded: the export never starts on something still loading (T8)
struct AllLoaded {
    audio: std::collections::BTreeMap<String, Arc<SampleData>>,
}

impl Resources for AllLoaded {
    fn audio(&self, resource: &Resource) -> ResourceState {
        match self.audio.get(resource.key()) {
            Some(d) => ResourceState::Ready(d.clone()),
            None => ResourceState::Missing,
        }
    }
}

/// A resource of another kind, read as the session's store reads it (`lm_io::kind_policy`): bytes, or a checked
/// file; `None` when the project does not have it
fn load_kind(project: &Path, r: &Resource, cancel: &Cancel) -> Result<Option<lm_io::Content>> {
    let Some(path) = lm_io::resource_path_of(project, r.kind(), r.key())? else { return Ok(None) };
    let io = |e: std::io::Error| crate::folder::io_error("session.export.read", &path, &e);
    let size = std::fs::metadata(&path).map_err(io)?.len();
    cancel.check()?;
    if lm_io::kind_policy(r.kind()) == lm_io::KindPolicy::Bytes && size <= lm_io::BYTES_MAX {
        let bytes = std::fs::read(&path).map_err(io)?;
        Ok(Some(lm_io::Content::Bytes(Arc::from(bytes.into_boxed_slice()))))
    } else {
        std::fs::File::open(&path).map_err(io)?;
        Ok(Some(lm_io::Content::File(path)))
    }
}

/// The last sample of the song: the end of its last item
pub fn song_end(root: &Value) -> Result<i64> {
    let map = tempo_map(root)?;
    let mut end_tick = 0i64;
    for t in root.get("tracks").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        for item in t.get("items").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
            let start = item.get("start").and_then(Value::as_int).unwrap_or(0);
            let length = item.get("length").and_then(Value::as_int).unwrap_or(0);
            end_tick = end_tick.max(start + length);
        }
    }
    Ok(map.sample_at(end_tick))
}

/// What an export renders: a copy of the tree, the project's folder (its `audio/`), the plugins and the schemas, and
/// the engine's capacities
#[derive(Clone, Debug)]
pub struct ExportJob {
    pub root: Value,
    pub project: PathBuf,
    pub plugins: Arc<Registry>,
    pub schemas: Arc<SchemaRegistry>,
    pub limits: Limits,
}

/// One pass of the renderer: where the transport starts, how much of the start is rendered and dropped (the
/// warm-up of a range), and how much is kept
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    locate: i64,
    discard: usize,
    count: usize,
}

/// Renders `root` (a copy of the project's tree, maybe changed for a stem) and hands the kept samples, interleaved
/// stereo, to `sink` block by block. Waits for every resource and every build first (T8): the same tree and run
/// give the same samples, bit for bit (T19)
fn render(
    job: &ExportJob,
    root: &Value,
    run: Run,
    cancel: &Cancel,
    progress: &Progress,
    sink: &mut dyn FnMut(&[f32]) -> Result<()>,
) -> Result<()> {
    let ExportJob { project, plugins, schemas, limits, .. } = job;
    let limits = *limits;
    let project: &Path = project;
    progress.stage(Label::new("session.export.preparing"));
    let rate = project_rate(root);
    let spec = ProcessSpec { sample_rate: rate, max_block: EXPORT_BLOCK };
    // Every resource first (T8)
    let mut audio = std::collections::BTreeMap::new();
    let mut cited = Vec::new();
    root.walk(&mut |_, v| {
        if let Some(r) = v.as_resource() {
            cited.push(r.clone());
        }
    });
    cited.sort();
    cited.dedup();
    let mut plugin_view = lm_plugin::ResourceView::new();
    for r in &cited {
        cancel.check()?;
        if r.kind() == "audio" {
            let path = lm_io::resource_path(project, r.key())?
                .ok_or_else(|| Error::new("session.export.missing").arg("resource", r.key()))?;
            let d = lm_io::load_file(&path, cancel, &Progress::new())?;
            plugin_view.insert(r.clone(), lm_plugin::Loaded::Audio(d.clone()));
            audio.insert(r.key().to_owned(), d);
        } else if let Some(c) = load_kind(project, r, cancel)? {
            // A missing one of another kind: the plugin builds without it, as in the session
            plugin_view.insert(r.clone(), crate::session::plugin_content(&c));
        }
    }
    let resources = AllLoaded { audio };
    // Compile, build what it asks, compile again, until nothing waits
    let mut core =
        CompilerCore::new(CompilerConfig { spec, limits, schemas: schemas.clone(), plugins: plugins.clone() });
    let (mut control, renderer) = lm_render::engine(spec, limits);
    let env = Environment::default();
    let mut rounds = 0;
    loop {
        cancel.check()?;
        let compiled = core.compile(root, &resources, &env)?;
        let complete = compiled.complete();
        let builds = compiled.builds.clone();
        control.install(compiled.install)?;
        if complete {
            break;
        }
        for b in builds {
            cancel.check()?;
            let mut p = build_processor(plugins, schemas, &b.build, b.spec, &plugin_view, b.seed)?;
            // A third-party plugin is waited for at every block: the file is the same bit for bit (S19)
            if let Some(proxy) = p.as_any_mut().downcast_mut::<lm_plugin_proxy::Proxy>() {
                proxy.set_deadline(lm_plugin_proxy::Deadline::Wait);
            }
            core.ready(b.ticket, NodeCore::new(p, b.seed, None));
        }
        rounds += 1;
        if rounds > MAX_ROUNDS {
            return Err(Error::new("session.export.not_ready"));
        }
    }
    let total = run.discard + run.count;
    let mut dummy = lm_render::Dummy::new(renderer, 0, 2);
    control
        .commands()
        .send(Command::Locate { sample: run.locate })
        .map_err(|_| Error::new("session.export.commands"))?;
    control.commands().send(Command::Play).map_err(|_| Error::new("session.export.commands"))?;
    let mut interleaved = vec![0.0f32; EXPORT_BLOCK * 2];
    let mut done = 0;
    progress.stage(Label::new("session.export.rendering"));
    while done < total {
        cancel.check()?;
        let n = EXPORT_BLOCK.min(total - done);
        dummy.block(n);
        let outs = dummy.outputs();
        // The warm-up is rendered and dropped
        let skip = run.discard.saturating_sub(done).min(n);
        let m = n - skip;
        for (frame, (l, r)) in
            interleaved.as_chunks_mut::<2>().0.iter_mut().zip(outs[0][skip..n].iter().zip(&outs[1][skip..n]))
        {
            frame[0] = *l;
            frame[1] = *r;
        }
        if m > 0 {
            sink(&interleaved[..2 * m])?;
        }
        control.collect(|r| core.returned(&r));
        done += n;
        progress.set(done as f32 / total.max(1) as f32);
    }
    Ok(())
}

/// The tree an export renders: the loop of the transport off (an export plays the song through), or on over
/// `looped` (the repeats of a loop export)
fn export_root(root: &Value, looped: Option<(i64, i64)>) -> Value {
    let transport = root.get("transport").cloned().unwrap_or_else(Value::empty_object);
    let lp = match looped {
        Some((a, b)) => Value::object([("on", Value::bool(true)), ("from", Value::int(a)), ("to", Value::int(b))]),
        None => transport.get("loop").cloned().unwrap_or_else(Value::empty_object).with("on", Value::bool(false)),
    };
    root.with("transport", transport.with("loop", lp).with("punch", Value::empty_object()))
}

/// Peak and loudness of what an export wrote
struct Meter {
    peak: f32,
    loudness: lm_dsp::loudness::Loudness,
    frames: usize,
    l: Vec<f32>,
    r: Vec<f32>,
}

impl Meter {
    fn new(rate: u32) -> Self {
        Self { peak: 0.0, loudness: lm_dsp::loudness::Loudness::new(rate), frames: 0, l: Vec::new(), r: Vec::new() }
    }

    fn add(&mut self, interleaved: &[f32]) {
        self.l.clear();
        self.r.clear();
        for f in interleaved.as_chunks::<2>().0 {
            self.peak = self.peak.max(f[0].abs()).max(f[1].abs());
            self.l.push(f[0]);
            self.r.push(f[1]);
        }
        self.loudness.process(&self.l, &self.r);
        self.frames += interleaved.len() / 2;
    }

    fn peak_db(&self) -> f64 {
        if self.peak > 0.0 { 20.0 * f64::from(self.peak).log10() } else { -200.0 }
    }

    fn lufs(&self) -> f64 {
        let i = self.loudness.integrated();
        if i.is_finite() { i } else { -200.0 }
    }
}

/// Renders the tree into the target's file, deterministically: the song (or the target's range of timeline
/// samples) with the loop of the transport off. Runs in an operation: progress, cancel. The report is the status
/// tree's `lastExport` (`file`, `seconds`, `peakDb`, `lufs`, `ok`, `block`)
pub fn export(job: ExportJob, target: &ExportTarget, cancel: &Cancel, progress: &Progress) -> Result<Value> {
    let rate = project_rate(&job.root);
    let (from, to) = match target.range {
        Some(r) => r,
        None => (0, song_end(&job.root)? + (target.tail * f64::from(rate)) as i64),
    };
    let run = Run { locate: from, discard: 0, count: usize::try_from((to - from).max(0)).unwrap_or(0) };
    let root = export_root(&job.root, None);
    let mut writer =
        Writer::create(&target.path, WriteSpec { format: target.format, rate, channels: 2 }, Existing::Replace)?;
    let mut meter = Meter::new(rate);
    render(&job, &root, run, cancel, progress, &mut |x| {
        meter.add(x);
        writer.write_f32(x)
    })?;
    writer.close()?;
    Ok(Value::object([
        ("file", Value::text(target.path.display().to_string())),
        ("seconds", Value::real(meter.frames as f64 / f64::from(rate))?),
        ("peakDb", Value::real(meter.peak_db())?),
        ("lufs", Value::real(meter.lufs())?),
        ("ok", Value::bool(true)),
        ("error", Value::text("")),
        ("block", Value::int(EXPORT_BLOCK as i64)),
    ]))
}
