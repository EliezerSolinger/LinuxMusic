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
use lm_sys_sndfile::{Existing, FlacDepth, Sample, WriteFormat, WriteSpec, Writer};

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
    /// The bytes of the resources of other kinds (the tuning files)
    bytes: std::collections::BTreeMap<Resource, Arc<[u8]>>,
}

impl Resources for AllLoaded {
    fn audio(&self, resource: &Resource) -> ResourceState {
        match self.audio.get(resource.key()) {
            Some(d) => ResourceState::Ready(d.clone()),
            None => ResourceState::Missing,
        }
    }

    fn bytes(&self, resource: &Resource) -> Option<Arc<[u8]>> {
        self.bytes.get(resource).cloned()
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
    let mut bytes = std::collections::BTreeMap::new();
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
            if let lm_io::Content::Bytes(b) = &c {
                bytes.insert(r.clone(), Arc::clone(b));
            }
            plugin_view.insert(r.clone(), crate::session::plugin_content(&c));
        }
    }
    let resources = AllLoaded { audio, bytes };
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

// ---- the export request of the export screen (lm_port::trees, `project.export`) ----

/// The file formats of a request
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FileFormat {
    /// 16, 24 or 32 (float) bits
    Wav(u32),
    /// 16 or 24 bits
    Flac(u32),
    /// Vorbis quality 0–10
    Ogg(i64),
    /// LAME's rate, 96–320 kb/s
    Mp3(i64),
}

impl FileFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Wav(_) => "wav",
            Self::Flac(_) => "flac",
            Self::Ogg(_) => "ogg",
            Self::Mp3(_) => "mp3",
        }
    }

    fn write_format(self) -> WriteFormat {
        match self {
            Self::Wav(16) => WriteFormat::Wav(Sample::Int16),
            Self::Wav(24) => WriteFormat::Wav(Sample::Int24),
            Self::Wav(_) => WriteFormat::Wav(Sample::Float32),
            Self::Flac(16) => WriteFormat::Flac { depth: lm_sys_sndfile::FlacDepth::Int16, compression: 0.5 },
            Self::Flac(_) => WriteFormat::Flac { depth: lm_sys_sndfile::FlacDepth::Int24, compression: 0.5 },
            Self::Ogg(q) => WriteFormat::OggVorbis { quality: (q as f64 / 10.0).clamp(0.0, 1.0) },
            // libsndfile takes LAME's quality, not a rate: the rates of the screen spread over it
            Self::Mp3(kbps) => WriteFormat::Mp3 { quality: ((kbps as f64 - 96.0) / (320.0 - 96.0)).clamp(0.0, 1.0) },
        }
    }

    /// The step of an integer format, for the dither (`None` for float and the lossy ones)
    fn lsb(self) -> Option<f32> {
        match self {
            Self::Wav(16) | Self::Flac(16) => Some(1.0 / 32_768.0),
            Self::Wav(24) | Self::Flac(_) => Some(1.0 / 8_388_608.0),
            _ => None,
        }
    }
}

/// Normalisation of what is exported
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Normalize {
    None,
    /// The sample peak to this dBFS
    Peak(f64),
    /// The integrated loudness (BS.1770) to this LUFS
    Lufs(f64),
}

/// A range of the timeline, in ticks (`to` exclusive)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeRequest {
    pub from: i64,
    pub to: i64,
    /// Start a warm-up before the range instead of rendering the song from its start (U37): not bit-identical to
    /// the same range of a whole export
    pub fast: bool,
    pub warmup: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StemsBy {
    Tracks,
    Channels,
}

/// What the export screen asks (lm_port::trees, `project.export`)
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    pub folder: PathBuf,
    pub name: String,
    pub formats: Vec<FileFormat>,
    /// The files' rate (the song renders at the project's and is converted)
    pub rate: Option<u32>,
    pub dither: bool,
    pub normalize: Normalize,
    pub fade_in: f64,
    pub fade_out: f64,
    pub range: Option<RangeRequest>,
    /// Without the tail
    pub as_loop: bool,
    /// The song (or the range) this many times over, played as a loop
    pub repeats: u32,
    /// One track alone (soloed)
    pub only_track: Option<lm_state::NodeId>,
    /// One file per track or per desk channel (all of them when `nodes` is empty)
    pub stems: Option<(StemsBy, Vec<lm_state::NodeId>)>,
    /// Seconds of tail after the end
    pub tail: f64,
}

impl ExportRequest {
    /// Reads the request's `Value` (unknown keys ignored; absent ones are the screen's defaults).
    /// Error: `action.params` (no folder, a bad format)
    pub fn from_value(p: &Value) -> Result<Self> {
        let params = || Error::new("action.params");
        let text = |k: &str| p.get(k).and_then(Value::as_text).map(str::to_owned);
        let int = |k: &str, d: i64| p.get(k).and_then(Value::as_int).unwrap_or(d);
        let real = |v: Option<&Value>, d: f64| v.and_then(Value::as_real).unwrap_or(d);
        let folder = PathBuf::from(text("folder").filter(|f| !f.trim().is_empty()).ok_or_else(params)?);
        let name = text("name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "export".to_owned());
        if name.contains('/') {
            return Err(params());
        }
        let depth = u32::try_from(int("depth", 24)).map_err(|_| params())?;
        let flac_depth = u32::try_from(int("flacDepth", i64::from(depth))).map_err(|_| params())?;
        let mut formats = Vec::new();
        for f in p.get("formats").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
            formats.push(match f.as_text().unwrap_or("") {
                "wav" => FileFormat::Wav(depth),
                "flac" => FileFormat::Flac(if flac_depth == 16 { 16 } else { 24 }),
                "ogg" => FileFormat::Ogg(int("oggQuality", 6)),
                "mp3" => FileFormat::Mp3(int("mp3Kbps", 320)),
                other => return Err(Error::new("session.export.format").arg("format", other)),
            });
        }
        if formats.is_empty() {
            formats.push(FileFormat::Wav(depth));
        }
        let normalize = match p.get("normalize") {
            Some(n) => match n.get("kind").and_then(Value::as_text).unwrap_or("none") {
                "peak" => Normalize::Peak(real(n.get("target"), -1.0)),
                "lufs" => Normalize::Lufs(real(n.get("target"), -14.0)),
                _ => Normalize::None,
            },
            None => Normalize::None,
        };
        let range = p.get("range").filter(|r| !r.is_null()).map(|r| RangeRequest {
            from: r.get("from").and_then(Value::as_int).unwrap_or(0).max(0),
            to: r.get("to").and_then(Value::as_int).unwrap_or(0).max(0),
            fast: r.get("fast").and_then(Value::as_bool).unwrap_or(false),
            warmup: real(r.get("warmup"), 10.0).max(0.0),
        });
        let node = |v: &Value| v.as_text().and_then(|t| lm_state::NodeId::parse(t).ok());
        let stems = match p.get("stems").filter(|s| !s.is_null()) {
            Some(s) => {
                let by = match s.get("by").and_then(Value::as_text) {
                    Some("channels") => StemsBy::Channels,
                    _ => StemsBy::Tracks,
                };
                let nodes = s.get("nodes").map(|l| l.items().filter_map(node).collect()).unwrap_or_default();
                Some((by, nodes))
            }
            None => None,
        };
        Ok(Self {
            folder,
            name,
            formats,
            rate: p.get("rate").and_then(Value::as_int).and_then(|r| u32::try_from(r).ok()).filter(|r| *r > 0),
            dither: p.get("dither").and_then(Value::as_bool).unwrap_or(false),
            normalize,
            fade_in: real(p.get("fadeIn"), 0.0).max(0.0),
            fade_out: real(p.get("fadeOut"), 0.0).max(0.0),
            range,
            as_loop: p.get("asLoop").and_then(Value::as_bool).unwrap_or(false),
            repeats: u32::try_from(int("repeats", 1).clamp(1, 64)).unwrap_or(1),
            only_track: p.get("onlyTrack").and_then(node),
            stems,
            tail: real(p.get("tail"), 2.0).max(0.0),
        })
    }
}

/// The tree with only `keep` soloed among the tracks or among the desk channels
fn soloed(root: &Value, list: &[&str], keep: lm_state::NodeId) -> Value {
    let path: Vec<lm_state::PathStep> = list.iter().map(|k| lm_state::PathStep::key(k)).collect();
    let Some(items) = root.at_path(&path).cloned() else { return root.clone() };
    let keep = keep.to_string();
    let solo: Vec<Value> = items
        .items()
        .map(|t| t.with("solo", Value::bool(t.get("id").and_then(Value::as_text) == Some(keep.as_str()))))
        .collect();
    root.with_path(&path, Value::list(solo)).unwrap_or_else(|_| root.clone())
}

/// A file name part from a track's or channel's name
fn file_part(name: &str) -> String {
    name.chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Exports as the request asks: every stem (or the song) rendered once at the project's rate into a temporary
/// float file in the folder, converted to the asked rate, faded, normalised, dithered and written in every format.
/// The same request on the same tree gives the same files, bit for bit (the dither's noise has a fixed seed).
/// The report is `lastExport`
pub fn export_request(job: ExportJob, req: &ExportRequest, cancel: &Cancel, progress: &Progress) -> Result<Value> {
    let rate = project_rate(&job.root);
    let out_rate = req.rate.unwrap_or(rate);
    for f in &req.formats {
        if !lm_sys_sndfile::is_supported(f.write_format(), out_rate, 2) {
            return Err(Error::new("session.export.format_unsupported").arg("format", f.extension()));
        }
    }
    std::fs::create_dir_all(&req.folder)
        .map_err(|e| crate::folder::io_error("session.export.folder", &req.folder, &e))?;
    let map = tempo_map(&job.root)?;
    // What to render: the range (ticks to samples) or the song, as a loop when repeated
    let (from, to) = match req.range {
        Some(r) => (map.sample_at(r.from), map.sample_at(r.to)),
        None => (0, song_end(&job.root)?),
    };
    if to <= from {
        return Err(Error::new("session.export.empty_range"));
    }
    let len = usize::try_from(to - from).unwrap_or(0);
    let looped = (req.repeats > 1).then_some((from, to));
    let tail = if req.as_loop || req.repeats > 1 { 0 } else { (req.tail * f64::from(rate)) as usize };
    let (locate, discard) = match req.range {
        // Fast: a warm-up before the range; exact: the song from its start, dropped up to the range (U37)
        Some(r) if r.fast => {
            let start = (from - (r.warmup * f64::from(rate)) as i64).max(0);
            (start, usize::try_from(from - start).unwrap_or(0))
        }
        Some(_) => (0, usize::try_from(from).unwrap_or(0)),
        None => (0, 0),
    };
    let run = Run { locate, discard, count: len * req.repeats as usize + tail };
    let identical = !req.range.is_some_and(|r| r.fast && r.from > 0);
    // The renders: one, or one per stem, each with its name
    // The transport's loop over the range, in ticks, for the repeats
    let loop_ticks = looped.map(|_| req.range.map_or((0, map.tick_at(to)), |r| (r.from, r.to)));
    let mut base = export_root(&job.root, loop_ticks);
    if let Some(t) = req.only_track {
        base = soloed(&base, &["tracks"], t);
    }
    let mut renders: Vec<(String, Value)> = Vec::new();
    match &req.stems {
        None => renders.push((req.name.clone(), base.clone())),
        Some((by, nodes)) => {
            let list: &[&str] = if *by == StemsBy::Tracks { &["tracks"] } else { &["mixer", "channels"] };
            let path: Vec<lm_state::PathStep> = list.iter().map(|k| lm_state::PathStep::key(k)).collect();
            let all = base.at_path(&path).cloned().unwrap_or_else(Value::empty_list);
            for item in all.items() {
                let Some(id) = item.get("id").and_then(Value::as_text).and_then(|t| lm_state::NodeId::parse(t).ok())
                else {
                    continue;
                };
                if item.get("folder").and_then(Value::as_bool) == Some(true)
                    || (!nodes.is_empty() && !nodes.contains(&id))
                {
                    continue;
                }
                let label = item
                    .get("name")
                    .and_then(Value::as_text)
                    .filter(|n| !n.is_empty())
                    .map_or_else(|| id.to_string(), file_part);
                renders.push((format!("{} - {label}", req.name), soloed(&base, list, id)));
            }
        }
    }
    let mut files = Vec::new();
    let mut first: Option<Meter> = None;
    let count = renders.len();
    for (i, (name, root)) in renders.into_iter().enumerate() {
        cancel.check()?;
        let part = Progress::new();
        let temp = req.folder.join(format!(".{name}.render.rf64"));
        let result = render_stem(&job, &root, run, rate, out_rate, req, &name, &temp, cancel, &part, &mut files);
        let _ = std::fs::remove_file(&temp);
        let _ = std::fs::remove_file(req.folder.join(format!(".{name}.resampled.rf64")));
        let meter = result?;
        first.get_or_insert(meter);
        progress.set((i + 1) as f32 / count.max(1) as f32);
    }
    let meter = first.ok_or_else(|| Error::new("session.export.nothing"))?;
    Ok(Value::object([
        ("file", Value::text(files.first().map(|f: &PathBuf| f.display().to_string()).unwrap_or_default())),
        ("files", Value::int(files.len() as i64)),
        ("seconds", Value::real(meter.frames as f64 / f64::from(out_rate))?),
        ("peakDb", Value::real(meter.peak_db())?),
        ("lufs", Value::real(meter.lufs())?),
        ("ok", Value::bool(true)),
        ("error", Value::text("")),
        ("identical", Value::bool(identical)),
        ("block", Value::int(EXPORT_BLOCK as i64)),
    ]))
}

/// Reads a float file whole-frame block by block
fn each_block(path: &Path, cancel: &Cancel, f: &mut dyn FnMut(&[f32]) -> Result<()>) -> Result<()> {
    let mut r = lm_sys_sndfile::Reader::open(path)?;
    let mut buf = vec![0.0f32; 8192 * 2];
    loop {
        cancel.check()?;
        let n = r.read_f32(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        f(&buf[..n * 2])?;
    }
}

/// The gain of the fades at output frame `i` of `total`
fn fade_gain(i: usize, total: usize, fade_in: usize, fade_out: usize) -> f32 {
    let mut g = 1.0f32;
    if fade_in > 0 && i < fade_in {
        g *= i as f32 / fade_in as f32;
    }
    if fade_out > 0 && i + fade_out >= total {
        g *= (total - i) as f32 / fade_out as f32;
    }
    g
}

/// A small, fixed-seed generator for the dither (xorshift): the same request gives the same noise
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Triangular noise of ±1 step
    fn tpdf(&mut self) -> f32 {
        self.next() - self.next()
    }
}

/// One stem: rendered, converted, measured, and written in every format
#[allow(clippy::too_many_arguments)]
fn render_stem(
    job: &ExportJob,
    root: &Value,
    run: Run,
    rate: u32,
    out_rate: u32,
    req: &ExportRequest,
    name: &str,
    temp: &Path,
    cancel: &Cancel,
    progress: &Progress,
    files: &mut Vec<PathBuf>,
) -> Result<Meter> {
    // 1. Render at the project's rate into a float file (any length: nothing of the song is held in memory)
    {
        let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate, channels: 2 };
        let mut w = Writer::create(temp, spec, Existing::Replace)?;
        render(job, root, run, cancel, progress, &mut |x| w.write_f32(x))?;
        w.close()?;
    }
    // 2. The asked rate (libsamplerate's best converter, aligned, exactly as long as the rates say)
    let source = if out_rate == rate {
        temp.to_path_buf()
    } else {
        let to = temp.with_file_name(format!(".{name}.resampled.rf64"));
        let ratio = f64::from(out_rate) / f64::from(rate);
        let want = lm_sys_samplerate::output_frames(run.count, rate, out_rate);
        let mut rs = lm_sys_samplerate::Resampler::new(lm_sys_samplerate::Quality::Best, 2, ratio)?;
        let spec = WriteSpec { format: WriteFormat::Rf64(Sample::Float32), rate: out_rate, channels: 2 };
        let mut w = Writer::create(&to, spec, Existing::Replace)?;
        let mut out = vec![0.0f32; 1 << 16];
        let mut written = 0usize;
        let put = |w: &mut Writer, data: &[f32], written: &mut usize| -> Result<()> {
            let n = (data.len() / 2).min(want.saturating_sub(*written));
            *written += n;
            w.write_f32(&data[..n * 2])
        };
        each_block(temp, cancel, &mut |block| {
            let mut at = 0;
            while at < block.len() / 2 {
                let done = rs.process(&block[at * 2..], &mut out)?;
                at += done.input_frames;
                put(&mut w, &out[..done.output_frames * 2], &mut written)?;
                if done.input_frames == 0 && done.output_frames == 0 {
                    break;
                }
            }
            Ok(())
        })?;
        loop {
            let done = rs.finish(&mut out)?;
            if done.output_frames == 0 {
                break;
            }
            put(&mut w, &out[..done.output_frames * 2], &mut written)?;
        }
        let silence = vec![0.0f32; 2 * want.saturating_sub(written)];
        w.write_f32(&silence)?;
        w.close()?;
        to
    };
    let total = lm_sys_sndfile::Reader::open(&source)?.info().frames as usize;
    let fade_in = (req.fade_in * f64::from(out_rate)) as usize;
    let fade_out = (req.fade_out * f64::from(out_rate)) as usize;
    // 3. The gain of the normalisation, measured with the fades
    let gain = if req.normalize == Normalize::None {
        1.0f32
    } else {
        let mut m = Meter::new(out_rate);
        let mut i = 0;
        let mut faded = Vec::new();
        each_block(&source, cancel, &mut |b| {
            faded.clear();
            for f in b.as_chunks::<2>().0 {
                let g = fade_gain(i, total, fade_in, fade_out);
                faded.extend_from_slice(&[f[0] * g, f[1] * g]);
                i += 1;
            }
            m.add(&faded);
            Ok(())
        })?;
        let db = match req.normalize {
            Normalize::Peak(t) if m.peak > 0.0 => t - m.peak_db(),
            Normalize::Lufs(t) if m.loudness.integrated().is_finite() => t - m.lufs(),
            _ => 0.0,
        };
        10f64.powf(db / 20.0) as f32
    };
    // 4. Every format from the same samples; the integer ones dithered when asked
    let mut writers = Vec::new();
    for f in &req.formats {
        let path = req.folder.join(format!("{name}.{}", f.extension()));
        let w = Writer::create(
            &path,
            WriteSpec { format: f.write_format(), rate: out_rate, channels: 2 },
            Existing::Replace,
        )?;
        writers.push((w, f.lsb().filter(|_| req.dither), Noise(0x9E37_79B9_7F4A_7C15), path));
    }
    let mut meter = Meter::new(out_rate);
    let mut i = 0;
    let mut processed = Vec::new();
    let mut dithered = Vec::new();
    each_block(&source, cancel, &mut |b| {
        processed.clear();
        for f in b.as_chunks::<2>().0 {
            let g = gain * fade_gain(i, total, fade_in, fade_out);
            processed.extend_from_slice(&[f[0] * g, f[1] * g]);
            i += 1;
        }
        meter.add(&processed);
        for (w, lsb, noise, _) in &mut writers {
            match lsb {
                Some(step) => {
                    dithered.clear();
                    dithered.extend(processed.iter().map(|x| x + noise.tpdf() * *step));
                    w.write_f32(&dithered)?;
                }
                None => w.write_f32(&processed)?,
            }
        }
        Ok(())
    })?;
    for (w, _, _, path) in writers {
        w.close()?;
        files.push(path);
    }
    Ok(meter)
}impl crate::session::Session {
    /// "clip.export_sample" (F-PRJ-026): the sample an audio item plays (its first lane, from the lane's offset, as
    /// long as the item) written as a 24-bit FLAC at `path`, in an operation with cancel. The item's gain, fades and
    /// the comp are not applied: it is the sample, not the render (the track's export is that)
    pub fn export_clip_sample(&mut self, item: lm_state::NodeId, path: PathBuf) -> Result<lm_base::OperationId> {
        let root = self.document().root().clone();
        let node = self
            .document()
            .find(item)
            .ok_or_else(|| Error::new("action.node.missing").arg("node", item.to_string()))?;
        let lane = node
            .get("lanes")
            .and_then(|l| l.items().next())
            .ok_or_else(|| Error::new("export.sample.no_audio").arg("item", item.to_string()))?;
        let key = lane
            .get("source")
            .and_then(Value::as_resource)
            .map(|r| r.key().to_owned())
            .ok_or_else(|| Error::new("export.sample.no_audio").arg("item", item.to_string()))?;
        let offset = lane.get("sourceOffset").and_then(Value::as_int).unwrap_or(0).max(0) as usize;
        let start = node.get("start").and_then(Value::as_int).unwrap_or(0);
        let length = node.get("length").and_then(Value::as_int).unwrap_or(1).max(1);
        // the item's length in seconds, by the tempo map (a musical item is as long as its bars say)
        let map = tempo_map(&root)?;
        let rate = f64::from(project_rate(&root));
        let seconds = (map.sample_at(start + length) - map.sample_at(start)).max(1) as f64 / rate;
        let folder = self.project().folder.root().to_path_buf();
        let allowed = crate::session::allowed_folders(self.preferences());
        let faults = self.faults().clone();
        Ok(self.start(
            Label::new("session.export.sample"),
            lm_port::Target::Project,
            true,
            "export_sample",
            move |cancel, progress| {
                let file = lm_io::resource_path_in(&folder, &key, Some(&allowed))?
                    .ok_or_else(|| Error::new("io.resource.missing").arg("key", key.as_str()))?;
                let data = lm_io::load_file(&file, cancel, progress)?;
                cancel.check()?;
                let channels = data.channels();
                let first = offset.min(data.frames());
                let last = (first + (seconds * f64::from(data.rate())).round() as usize).min(data.frames());
                let mut inter = Vec::with_capacity((last - first) * channels);
                for f in first..last {
                    for c in 0..channels {
                        inter.push(data.channel(c)[f]);
                    }
                }
                faults.check(inter.len() as u64 * 3).map_err(|e| e.arg("file", path.display().to_string()))?;
                let tmp = path.with_extension("flac.tmp");
                let spec = WriteSpec {
                    format: WriteFormat::Flac { depth: FlacDepth::Int24, compression: 0.5 },
                    rate: data.rate(),
                    channels,
                };
                let written = (|| -> Result<()> {
                    let mut w = Writer::create(&tmp, spec, Existing::Replace)?;
                    for block in inter.chunks(channels << 15) {
                        cancel.check()?;
                        w.write_f32(block)?;
                    }
                    w.close()?;
                    std::fs::rename(&tmp, &path).map_err(|e| crate::folder::io_error("session.file.rename", &path, &e))
                })();
                if written.is_err() {
                    let _ = std::fs::remove_file(&tmp);
                }
                written?;
                Ok(Value::text(path.display().to_string()))
            },
        ))
    }
}


