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

/// The integrated loudness of a stereo signal (ITU-R BS.1770-4: K-weighting, 400 ms blocks with 75 % overlap, the
/// absolute gate at -70 LUFS and the relative gate at -10 LU), `None` when it is all gated out
pub fn integrated_loudness(left: &[f32], right: &[f32], rate: u32) -> Option<f64> {
    let rate = f64::from(rate);
    // The two stages of the K filter, by the bilinear transform of the standard's analog prototypes
    let shelf = {
        let (f0, g, q) = (1_681.974_450_955_533, 3.999_843_853_973_347, 0.707_175_236_955_419_6);
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / q + k * k) / a0,
        ]
    };
    let high_pass = {
        let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        [1.0, -2.0, 1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0]
    };
    let filter = |x: &[f32]| -> Vec<f64> {
        let mut out = Vec::with_capacity(x.len());
        let mut s = [[0.0f64; 2]; 2];
        for &v in x {
            let mut y = f64::from(v);
            for (stage, c) in [shelf, high_pass].iter().enumerate() {
                let o = c[0] * y + s[stage][0];
                s[stage][0] = c[1] * y - c[3] * o + s[stage][1];
                s[stage][1] = c[2] * y - c[4] * o;
                y = o;
            }
            out.push(y);
        }
        out
    };
    let (l, r) = (filter(left), filter(right));
    let block = (0.4 * rate) as usize;
    let hop = (0.1 * rate) as usize;
    if l.len() < block || hop == 0 {
        return None;
    }
    let mut powers = Vec::new();
    let mut at = 0;
    while at + block <= l.len() {
        let p = (at..at + block).map(|i| l[i] * l[i] + r[i] * r[i]).sum::<f64>() / block as f64;
        powers.push(p);
        at += hop;
    }
    let lufs = |p: f64| -0.691 + 10.0 * p.log10();
    let above: Vec<f64> = powers.iter().copied().filter(|p| lufs(*p) > -70.0).collect();
    if above.is_empty() {
        return None;
    }
    let relative = lufs(above.iter().sum::<f64>() / above.len() as f64) - 10.0;
    let gated: Vec<f64> = above.into_iter().filter(|p| lufs(*p) > relative).collect();
    if gated.is_empty() {
        return None;
    }
    Some(lufs(gated.iter().sum::<f64>() / gated.len() as f64))
}

/// Renders the tree into the target's file, deterministically. Runs in an operation: progress, cancel. The report
/// is the status tree's `lastExport` (`file`, `seconds`, `peakDb`, `lufs`, `ok`)
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

pub fn export(job: ExportJob, target: &ExportTarget, cancel: &Cancel, progress: &Progress) -> Result<Value> {
    let ExportJob { root, project, plugins, schemas, limits } = job;
    let project: &Path = &project;
    progress.stage(Label::new("session.export.preparing"));
    let rate = project_rate(&root);
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
        let compiled = core.compile(&root, &resources, &env)?;
        let complete = compiled.complete();
        let builds = compiled.builds.clone();
        control.install(compiled.install)?;
        if complete {
            break;
        }
        for b in builds {
            cancel.check()?;
            let mut p = build_processor(&plugins, &schemas, &b.build, b.spec, &plugin_view, b.seed)?;
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
    // The range, then the transport, then the blocks
    let (from, to) = match target.range {
        Some(r) => r,
        None => (0, song_end(&root)? + (target.tail * f64::from(rate)) as i64),
    };
    let total = usize::try_from((to - from).max(0)).unwrap_or(0);
    let mut dummy = lm_render::Dummy::new(renderer, 0, 2);
    control.commands().send(Command::Locate { sample: from }).map_err(|_| Error::new("session.export.commands"))?;
    control.commands().send(Command::Play).map_err(|_| Error::new("session.export.commands"))?;
    let mut writer =
        Writer::create(&target.path, WriteSpec { format: target.format, rate, channels: 2 }, Existing::Replace)?;
    let mut left = Vec::with_capacity(total);
    let mut right = Vec::with_capacity(total);
    let mut interleaved = vec![0.0f32; EXPORT_BLOCK * 2];
    let mut done = 0;
    progress.stage(Label::new("session.export.rendering"));
    while done < total {
        cancel.check()?;
        let n = EXPORT_BLOCK.min(total - done);
        dummy.block(n);
        let outs = dummy.outputs();
        for k in 0..n {
            interleaved[2 * k] = outs[0][k];
            interleaved[2 * k + 1] = outs[1][k];
        }
        left.extend_from_slice(&outs[0][..n]);
        right.extend_from_slice(&outs[1][..n]);
        writer.write_f32(&interleaved[..2 * n])?;
        control.collect(|r| core.returned(&r));
        done += n;
        progress.set(done as f32 / total.max(1) as f32);
    }
    writer.close()?;
    let peak = left.iter().chain(right.iter()).fold(0.0f32, |p, x| p.max(x.abs()));
    let peak_db = if peak > 0.0 { 20.0 * f64::from(peak).log10() } else { -200.0 };
    let lufs = integrated_loudness(&left, &right, rate).unwrap_or(-200.0);
    Ok(Value::object([
        ("file", Value::text(target.path.display().to_string())),
        ("seconds", Value::real(total as f64 / f64::from(rate))?),
        ("peakDb", Value::real(peak_db)?),
        ("lufs", Value::real(lufs)?),
        ("ok", Value::bool(true)),
        ("error", Value::text("")),
        ("block", Value::int(EXPORT_BLOCK as i64)),
    ]))
}
