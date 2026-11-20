//! The session (I-16): the one owner of everything the program holds while it runs — the device, the journal, the
//! compiler, the engine, the operations, the documents, the resources, the plugins, the preferences — on the main
//! thread (it is `!Send`: it holds the [`Main`] proof and the documents).
//!
//! Its fields drop in declaration order, what must die first coming first: the clock thread, then the device (its
//! `Drop` runs the barrier, S16), the recording's disk writer, the journal, the compiler and the engine, the
//! operations (their handles cancel), the documents, the resources, and the workers last (they join every operation
//! still running, T18).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use lm_base::{Cancel, Clock, Error, HostTime, Label, Main, Progress, Result, Role, Workers};
use lm_compile::{
    BuildRequest, Compiled, Compiler, CompilerConfig, CompilerCore, Environment, InstalledTable, NodeKey,
    ResourceState, Resources, Ticket,
};
use lm_doc::{Diff, Document, Feed, JournalPatch, Options, Origin, Topic};
use lm_io::{
    Device, DeviceConfig, DeviceEvent, DiskEvent, DiskWriter, DummyBackend, DummyOptions, FinalizedTake, Pace,
    RecordInput, RecordPlan, ResourceStore, Take, TakeFormat,
};
use lm_plugin::Registry;
use lm_port::{Loading, OperationInfo, Position, Severity, Target, TelemetryView};
use lm_render::{
    Command, EngineControl, Limits, NodeCore, ProcessSpec, Processor, RenderThreads, Telemetry, TransportEvent,
};
use lm_state::{NodeId, PathStep, Resource, SchemaRegistry, Value, from_json};

use crate::activity::{Activity, Finished, Op, OpList, Ops, loading_of};
use crate::folder::{DiskFaults, ProjectFolder};
use crate::journal::{Journal, JournalEvent, base_hash, read_autosave, read_journal, set_aside};
use crate::lock::{Locking, ProjectLock};
use crate::notices::Notices;
use crate::prefs::Preferences;
use crate::rack::RackProcessor;

/// How often the clock thread wakes the main thread (H2)
pub const PUMP_PERIOD: Duration = Duration::from_millis(10);
/// While a gesture is open, its changes reach the journal at least this often (T11)
pub const GESTURE_JOURNAL: Duration = Duration::from_secs(1);

/// The audio device the session opens
#[derive(Clone, Debug)]
pub enum DeviceSetup {
    /// The device without hardware (tests, and the fallback after a card goes away, U11)
    Dummy(DummyOptions),
    /// An ALSA device by name (`hw:CARD=PCH,DEV=0`), Linux only
    Alsa { device: String },
}

/// What a session is made from
#[derive(Clone)]
pub struct SessionConfig {
    pub registry: Arc<Registry>,
    pub clock: Arc<dyn Clock>,
    pub preferences: Preferences,
    pub faults: Arc<DiskFaults>,
    pub limits: Limits,
    pub device: DeviceSetup,
    /// The block asked of the device (the negotiated one is used, C7)
    pub block: u32,
    pub workers: usize,
    /// Called by the clock thread every 10 ms (H2): it must make the main thread call `pump` (the interface's event
    /// loop, or the headless loop). `None`: no clock thread (the caller pumps by itself)
    pub waker: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Test mode (I-2.4): every plugin build and resource load takes this much longer
    pub delay: Duration,
    /// Where an untitled project lives until it is saved somewhere
    pub untitled: PathBuf,
}

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfig").field("device", &self.device).field("block", &self.block).finish()
    }
}

/// Every internal plugin of this build
pub fn internal_plugins() -> Result<Arc<Registry>> {
    Registry::new(&[&lm_plugins_testsine::DESCRIPTOR, &lm_plugins_tripleoscillator::DESCRIPTOR])
}

impl SessionConfig {
    /// A session for tests and the headless tools: the Dummy device running freely (as fast as it can), the
    /// preferences in memory, the system clock, no clock thread
    pub fn headless(untitled: PathBuf) -> Result<Self> {
        Ok(Self {
            registry: internal_plugins()?,
            clock: Arc::new(lm_base::SystemClock),
            preferences: Preferences::in_memory(),
            faults: DiskFaults::new(),
            limits: Limits { max_nodes: 1024, max_params: 8192, ..Limits::default() },
            device: DeviceSetup::Dummy(DummyOptions { pace: Pace::RealTime, ..DummyOptions::default() }),
            block: 256,
            workers: 2,
            waker: None,
            delay: Duration::ZERO,
            untitled,
        })
    }
}

/// The clock thread (H2): wakes the main thread every 10 ms, whether anything is drawn or not
struct Ticker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Ticker {
    fn start(waker: Arc<dyn Fn() + Send + Sync>) -> Ticker {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::Builder::new()
            .name("lm-clock".into())
            .spawn(move || {
                while !s.load(Ordering::Acquire) {
                    std::thread::sleep(PUMP_PERIOD);
                    waker();
                }
            })
            .ok();
        Ticker { stop, thread }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Removes an untitled project's folder when the session ends, if nothing worth keeping is in it (no audio)
#[derive(Debug)]
struct UntitledGuard(Option<PathBuf>);

impl Drop for UntitledGuard {
    fn drop(&mut self) {
        let Some(path) = self.0.take() else { return };
        let audio = path.join(crate::folder::AUDIO_DIR);
        let empty = std::fs::read_dir(&audio).map(|mut d| d.next().is_none()).unwrap_or(true);
        if empty {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// The project open now
#[derive(Debug)]
pub struct Project {
    pub folder: ProjectFolder,
    pub lock: ProjectLock,
    /// Saving, the journal and the autosave are off (a lock held elsewhere H10, a newer schema H11)
    pub read_only: bool,
    /// Never saved by the user (it lives in the untitled folder)
    pub untitled: bool,
    /// The project's version when it was migrated on opening: its original is kept before the first save (H11)
    pub migrated_from: Option<u32>,
    /// The root as last saved, to tell `modified`
    pub saved_root: Value,
    pub locked_by: Option<crate::lock::LockOwner>,
}

/// What an open operation produced, applied by the pump
#[derive(Debug)]
pub struct ReadProject {
    pub folder: ProjectFolder,
    pub bytes_hash: String,
    pub saved: Value,
    pub journal: crate::journal::ReadJournal,
    pub autosave: Option<crate::journal::ReadAutosave>,
    pub interface: Option<Value>,
    pub lock: ProjectLock,
    pub locking: Locking,
    pub interrupted: Vec<PathBuf>,
}

/// The resources as the compiler sees them: loaded, loading or missing
struct ResourceSnapshot {
    view: lm_io::ResourceView,
    missing: BTreeSet<String>,
}

impl Resources for ResourceSnapshot {
    fn audio(&self, resource: &Resource) -> ResourceState {
        if let Some(d) = self.view.get(resource.key()) {
            return ResourceState::Ready(d.clone());
        }
        if self.missing.contains(resource.key()) { ResourceState::Missing } else { ResourceState::Loading }
    }
}

/// A plugin or rack build waiting for its resources, or running
#[derive(Debug)]
pub(crate) struct BuildData {
    ticket: Ticket,
    node: NodeKey,
    seed: u64,
}

/// What was copied: the tree and the project it came from (its uuid is in the clip, its folder holds its audio)
#[derive(Clone, Debug)]
pub struct Clipboard {
    pub clip: lm_doc::Clip,
    pub project: PathBuf,
}

/// A take to close: one just recorded, or an interrupted one to recover first
#[derive(Debug)]
enum TakeJob {
    Recorded(Take),
    Interrupted(PathBuf),
}

/// The takes of one recording, gathered until every input's is closed
#[derive(Debug, Default)]
struct PendingRecording {
    inputs: usize,
    takes: Vec<FinalizedTake>,
    failed: usize,
}

/// A request for a recording that waits for the device to arm (lm-io: arm first, then record)
#[derive(Debug)]
struct RecordStart {
    since: HostTime,
}

/// The session
pub struct Session {
    /// Dropped first: the clock stops waking the main thread
    _ticker: Option<Ticker>,
    device: Option<Device>,
    disk: Option<DiskWriter>,
    journal: Journal,
    /// After the journal: removes the untitled folder of a session that ends without a recording or a save in it
    untitled_guard: UntitledGuard,
    compiler: Compiler,
    engine: EngineControl,
    // The operations, by kind (I-2)
    open_ops: Ops<ReadProject, ()>,
    build_ops: Ops<Box<dyn Processor>, BuildData>,
    load_ops: Ops<lm_io::Loaded, String>,
    import_ops: Ops<lm_io::ImportedFile, ()>,
    /// Takes being closed, by the recording they belong to (0: a recovered take, alone)
    take_ops: Ops<FinalizedTake, u64>,
    generic_ops: Ops<Value, &'static str>,
    pub(crate) activity: Activity,
    pub(crate) notices: Notices,
    ui_document: Document,
    document: Document,
    resources: ResourceStore,
    plugins: Arc<Registry>,
    schemas: Arc<SchemaRegistry>,
    preferences: Preferences,
    workers: Workers,
    // State kept with them
    pub(crate) project: Project,
    feed: Feed,
    table: Arc<InstalledTable>,
    spec: ProcessSpec,
    limits: Limits,
    clock: Arc<dyn Clock>,
    faults: Arc<DiskFaults>,
    delay: Duration,
    device_setup: DeviceSetup,
    untitled: PathBuf,
    /// A "save as" copying the audio: where the project goes, with the lock already taken
    pending_save_as: Option<(lm_base::OperationId, ProjectFolder, ProjectLock)>,
    block: u32,
    /// Builds waiting for the resources their state cites (C11)
    builds_waiting: Vec<BuildRequest>,
    /// New nodes silent until built; nodes whose old processor plays while the new one builds (I-2.3, T7)
    waiting_nodes: BTreeSet<NodeKey>,
    replacing_nodes: BTreeSet<NodeKey>,
    /// The compiler was asked to compile and has not answered
    compiling_since: Option<HostTime>,
    needs_compile: bool,
    telemetry: TelemetryView,
    status: Value,
    save_serial: u64,
    export_serial: u64,
    saving: Option<(u64, Value, HostTime)>,
    last_autosave: HostTime,
    last_gesture_journal: HostTime,
    autosave_warned: bool,
    record_start: Option<RecordStart>,
    recording_tracks: Vec<NodeId>,
    /// The recordings whose takes are being closed: all of one recording enter the tree in one transaction
    recordings: std::collections::BTreeMap<u64, PendingRecording>,
    recording_serial: u64,
    /// Takes waiting to be closed: one at a time (two takes of the same content share their files' names)
    take_queue: std::collections::VecDeque<(TakeJob, u64)>,
    next_take_after: bool,
    /// Records forgotten once the journal line that holds their take is on the disk (U5)
    forget_after: Vec<(Arc<AtomicBool>, PathBuf)>,
    xruns: u32,
    /// The imports that ended, by their operation, for the port to answer (`take_imported`)
    imported: Vec<(lm_base::OperationId, Result<Resource>)>,
    /// What `edit.copy` copied (H13)
    pub(crate) clipboard: Option<Clipboard>,
    /// The finished operations the headless printer has not shown
    pub(crate) finished: Vec<Finished>,
    main: Main,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("folder", &self.project.folder).field("spec", &self.spec).finish()
    }
}

pub(crate) fn err(key: &'static str) -> Error {
    Error::new(key)
}

/// The project's tempo map (as lm-compile reads it)
pub fn tempo_map(root: &Value) -> Result<lm_time::TempoMap> {
    use lm_time::{Bpm, Ppq, TempoMap, TempoPoint, TimeSignature};
    let p = root.get("project");
    let int = |k: &str, d: i64| p.and_then(|p| p.get(k)).and_then(Value::as_int).unwrap_or(d);
    let rate = u32::try_from(int("sampleRate", 48_000)).unwrap_or(48_000);
    let ppq = Ppq::new(u32::try_from(int("ppq", 960)).unwrap_or(960))?;
    let mut points = Vec::new();
    for t in root.get("tempoMap").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        let bpm = Bpm::parse(t.get("bpm").and_then(Value::as_text).unwrap_or("120"))?;
        let sig: Vec<i64> = t.get("timeSig").map(|l| l.items().filter_map(Value::as_int).collect()).unwrap_or_default();
        let (n, d) = match sig.as_slice() {
            [n, d] => (*n, *d),
            _ => (4, 4),
        };
        let ts = TimeSignature::new(u8::try_from(n).unwrap_or(4), u8::try_from(d).unwrap_or(4))?;
        let mut point = TempoPoint::new(t.get("tick").and_then(Value::as_int).unwrap_or(0), bpm, ts);
        if t.get("ramp").and_then(Value::as_bool) == Some(true) {
            point = point.linear();
        }
        points.push(point);
    }
    if points.is_empty() {
        points.push(TempoPoint::new(0, Bpm::from_integer(120)?, TimeSignature::COMMON));
    }
    TempoMap::new(rate, ppq, &points)
}

/// The project's sample rate
pub fn project_rate(root: &Value) -> u32 {
    root.get("project")
        .and_then(|p| p.get("sampleRate"))
        .and_then(Value::as_int)
        .and_then(|r| u32::try_from(r).ok())
        .unwrap_or(48_000)
}

/// The document's options for a project: the plugin layer's ports, the engine's limits as rules, and the compiler's
/// own count of what a root uses (H12)
fn project_options(plugins: &Arc<Registry>, schemas: &Arc<SchemaRegistry>, limits: Limits) -> Options {
    let core = Arc::new(Mutex::new(CompilerCore::new(CompilerConfig {
        spec: ProcessSpec { sample_rate: 48_000, max_block: 256 },
        limits,
        schemas: schemas.clone(),
        plugins: plugins.clone(),
    })));
    let usage: lm_doc::UsageFn = Arc::new(move |_registry: &SchemaRegistry, root: &Value| {
        let u = core.lock().ok().and_then(|mut c| c.usage(root).ok());
        u.map_or(lm_doc::Usage::default(), |u| lm_doc::Usage { nodes: u.nodes, params: u.params })
    });
    Options {
        limits: Some(lm_doc::Limits { max_nodes: limits.max_nodes, max_params: limits.max_params }),
        ports: Some(plugins.ports_fn()),
        usage,
        ..Options::default()
    }
}

/// The interface's document (I-3.8): a free tree, no history, no journal
fn interface_document(tree: Option<Value>) -> Result<Document> {
    let schemas = interface_schemas()?;
    let root = match tree {
        Some(t) if t.get("state").is_some() => t,
        _ => schemas.defaults(INTERFACE_TYPE)?,
    };
    Ok(Document::from_root(schemas.clone(), INTERFACE_TYPE, root, Options::interface())?.document)
}

/// The interface document's type: an object of anything (zoom, scroll, windows), kept as it is
pub const INTERFACE_TYPE: &str = "interface";

/// The interface document's own registry: never mixed with the project's, so the saved project's `versions` lists
/// only the project's types
fn interface_schemas() -> Result<Arc<SchemaRegistry>> {
    let mut registry = SchemaRegistry::new();
    registry.add(
        lm_state::NodeSchema::new(INTERFACE_TYPE, 1)
            .field(lm_state::FieldSpec::new("state", lm_state::FieldKind::Any).default(Value::empty_object())),
    )?;
    registry.check_complete()?;
    Ok(Arc::new(registry))
}

fn open_device(setup: &DeviceSetup, rate: u32, block: u32) -> Result<(Device, lm_io::DeviceInfo)> {
    let backend: Box<dyn lm_io::Backend> = match setup {
        DeviceSetup::Dummy(o) => Box::new(DummyBackend::new(*o)),
        #[cfg(target_os = "linux")]
        DeviceSetup::Alsa { .. } => Box::new(lm_io::AlsaBackend::new(None)),
        #[cfg(not(target_os = "linux"))]
        DeviceSetup::Alsa { .. } => return Err(err("session.device.alsa_unavailable")),
    };
    let name = match setup {
        DeviceSetup::Dummy(_) => "dummy".to_owned(),
        DeviceSetup::Alsa { device } => device.clone(),
    };
    let mut device = Device::new(backend, RenderThreads::default());
    let info = device.open(&DeviceConfig::duplex(&name, rate, block))?;
    Ok((device, info))
}

impl Session {
    /// A session with a new, untitled project. Opens the device (the Dummy when the chosen one fails, with a notice)
    pub fn new(config: SessionConfig) -> Result<Session> {
        let main = Main::acquire().ok_or_else(|| err("session.main.taken"))?;
        let plugins = config.registry.clone();
        let schemas = Arc::new(plugins.project_schemas()?);
        let mut notices = Notices::default();
        let options = project_options(&plugins, &schemas, config.limits);
        let document = Document::new_project(schemas.clone(), options)?;
        let ui_document = interface_document(None)?;
        let folder = ProjectFolder::new(config.untitled.join(format!("untitled-{}", lm_io::new_uuid())));
        folder.create(&config.faults)?;
        let rate = project_rate(document.root());
        let (device, info) = match open_device(&config.device, rate, config.block) {
            Ok(d) => d,
            Err(e) => {
                notices.error(&e, Some(Target::Device), None);
                open_device(&DeviceSetup::Dummy(DummyOptions::default()), rate, config.block)?
            }
        };
        let spec = info.spec();
        let (engine, renderer) = lm_render::engine_with(spec, config.limits, RenderThreads::default());
        let mut device = device;
        device.start(renderer).map_err(|(e, _)| e)?;
        let compiler = Compiler::new(CompilerConfig {
            spec,
            limits: config.limits,
            schemas: schemas.clone(),
            plugins: plugins.clone(),
        });
        let journal = Journal::start(&folder, config.faults.clone(), String::new(), 0, false);
        let resources = ResourceStore::new(folder.root().to_path_buf(), config.clock.clone());
        let disk = match DiskWriter::start() {
            Ok(d) => Some(d),
            Err(e) => {
                notices.error(&e, None, None);
                None
            }
        };
        let mut document = document;
        let feed = document.feed(Topic::All);
        let now = config.clock.now();
        let saved_root = document.root().clone();
        let mut s = Session {
            _ticker: config.waker.clone().map(Ticker::start),
            device: Some(device),
            disk,
            journal,
            untitled_guard: UntitledGuard(Some(folder.root().to_path_buf())),
            compiler,
            engine,
            open_ops: Ops::default(),
            build_ops: Ops::default(),
            load_ops: Ops::default(),
            import_ops: Ops::default(),
            take_ops: Ops::default(),
            generic_ops: Ops::default(),
            activity: Activity::default(),
            notices,
            ui_document,
            document,
            resources,
            plugins,
            schemas,
            preferences: config.preferences.clone(),
            workers: Workers::new("lm-worker", config.workers.max(1), Role::Worker),
            project: Project {
                folder,
                lock: ProjectLock::none(),
                read_only: false,
                untitled: true,
                migrated_from: None,
                saved_root,
                locked_by: None,
            },
            feed,
            table: Arc::new(InstalledTable::default()),
            spec,
            limits: config.limits,
            clock: config.clock.clone(),
            faults: config.faults.clone(),
            delay: config.delay,
            device_setup: config.device.clone(),
            untitled: config.untitled.clone(),
            pending_save_as: None,
            block: config.block,
            builds_waiting: Vec::new(),
            waiting_nodes: BTreeSet::new(),
            replacing_nodes: BTreeSet::new(),
            compiling_since: None,
            needs_compile: true,
            telemetry: empty_telemetry(rate),
            status: Value::NULL,
            save_serial: 0,
            export_serial: 0,
            saving: None,
            last_autosave: now,
            last_gesture_journal: now,
            autosave_warned: false,
            record_start: None,
            recording_tracks: Vec::new(),
            recordings: std::collections::BTreeMap::new(),
            recording_serial: 0,
            forget_after: Vec::new(),
            take_queue: std::collections::VecDeque::new(),
            next_take_after: false,
            xruns: 0,
            clipboard: None,
            imported: Vec::new(),
            finished: Vec::new(),
            main,
        };
        s.refresh_status();
        s.submit();
        Ok(s)
    }

    // ---- reading ----

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.document
    }

    pub fn ui_document(&self) -> &Document {
        &self.ui_document
    }

    pub fn ui_document_mut(&mut self) -> &mut Document {
        &mut self.ui_document
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.plugins
    }

    pub fn schemas(&self) -> &Arc<SchemaRegistry> {
        &self.schemas
    }

    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn spec(&self) -> ProcessSpec {
        self.spec
    }

    pub fn table(&self) -> &Arc<InstalledTable> {
        &self.table
    }

    pub fn telemetry(&self) -> &TelemetryView {
        &self.telemetry
    }

    pub fn status(&self) -> &Value {
        &self.status
    }

    pub fn notices(&mut self) -> &mut Notices {
        &mut self.notices
    }

    pub fn resources(&self) -> &ResourceStore {
        &self.resources
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    pub fn faults(&self) -> &Arc<DiskFaults> {
        &self.faults
    }

    pub fn workers(&self) -> &Workers {
        &self.workers
    }

    /// The main thread's proof this session holds
    pub fn main(&self) -> &Main {
        &self.main
    }

    /// Whether the project changed since it was saved
    pub fn modified(&self) -> bool {
        !self.document.root().same_node(&self.project.saved_root)
    }

    pub fn engine(&mut self) -> &mut EngineControl {
        &mut self.engine
    }

    /// What keeps the session from being settled (diagnostics for the tests and the headless tool)
    pub fn busy(&self) -> Vec<String> {
        let mut out = Vec::new();
        let counts = [
            ("builds", self.build_ops.len()),
            ("loads", self.load_ops.len()),
            ("builds_waiting", self.builds_waiting.len()),
            ("waiting_nodes", self.waiting_nodes.len()),
            ("replacing_nodes", self.replacing_nodes.len()),
            ("installs_in_flight", self.engine.installs_in_flight()),
            ("installs_queued", self.engine.installs_queued()),
            ("opening", self.open_ops.len()),
        ];
        for (k, n) in counts {
            if n > 0 {
                out.push(format!("{k}: {n}"));
            }
        }
        if self.compiling_since.is_some() {
            out.push("compiling".into());
        }
        if self.needs_compile {
            out.push("needs_compile".into());
        }
        if self.saving.is_some() {
            out.push("saving".into());
        }
        for (id, _) in self.document.index().ids() {
            if self.is_waiting(*id) {
                out.push(format!("waiting {id}"));
            }
        }
        out
    }

    /// Whether a document node's processor is still being built (new nodes are silent meanwhile, I-2.3)
    pub fn is_waiting(&self, id: NodeId) -> bool {
        // A plugin of the document the installed program does not have yet: the compiler has not answered
        if self.document.type_of(id) == Some(lm_doc::schemas::types::PLUGIN)
            && self
                .document
                .find(id)
                .and_then(|n| n.get("plugin").and_then(Value::as_text).map(str::to_owned))
                .is_some_and(|p| self.plugins.get(&p).is_some())
            && self.table.node(NodeKey::Instrument(id)).is_none()
            && self.table.node(NodeKey::Insert(id)).is_none()
        {
            return true;
        }
        self.waiting_nodes.iter().any(|k| k.owner() == Some(id))
            || self.builds_waiting.iter().any(|b| b.node.owner() == Some(id))
            || self.build_ops.list.iter().any(|op| op.data.node.owner() == Some(id))
    }

    /// Nothing waits: no build, no load, no compilation, no install in flight (the tests' and the export's "ready")
    pub fn settled(&self) -> bool {
        self.build_ops.is_empty()
            && self.load_ops.is_empty()
            && self.builds_waiting.is_empty()
            && self.waiting_nodes.is_empty()
            && self.replacing_nodes.is_empty()
            && self.compiling_since.is_none()
            && !self.needs_compile
            && self.engine.installs_in_flight() == 0
            && self.engine.installs_queued() == 0
            && self.open_ops.is_empty()
            && self.saving.is_none()
            && !self.document.index().ids().any(|(id, _)| self.is_waiting(*id))
    }

    // ---- operations (I-2) ----

    /// Every operation running (the global indicator)
    pub fn activity(&self) -> Vec<OperationInfo> {
        let now = self.clock.now();
        let mut out = Vec::new();
        let lists: [&dyn OpList; 6] =
            [&self.open_ops, &self.build_ops, &self.load_ops, &self.import_ops, &self.take_ops, &self.generic_ops];
        for l in lists {
            l.infos(now, &mut out);
        }
        if let Some(since) = self.compiling_since {
            out.push(OperationInfo {
                id: lm_base::OperationId(0),
                label: Label::new("session.compiling"),
                target: Target::Project,
                progress: None,
                stage: None,
                cancellable: false,
                since: Duration::from_nanos(now.since(since)),
            });
        }
        if let Some((_, _, since)) = &self.saving {
            out.push(OperationInfo {
                id: lm_base::OperationId(0),
                label: Label::new("session.saving"),
                target: Target::Project,
                progress: None,
                stage: None,
                cancellable: false,
                since: Duration::from_nanos(now.since(*since)),
            });
        }
        out
    }

    pub fn cancel(&mut self, id: lm_base::OperationId) {
        let lists: [&dyn OpList; 6] =
            [&self.open_ops, &self.build_ops, &self.load_ops, &self.import_ops, &self.take_ops, &self.generic_ops];
        for l in lists {
            if l.cancel(id) {
                return;
            }
        }
    }

    /// What the interface draws on a target (I-2.2): the operations on it, and for a node everything it depends on
    /// (its builds, the resources its state cites, T10)
    pub fn loading(&self, target: &Target) -> Loading {
        let all = self.activity();
        let mut mine: Vec<&OperationInfo> = all.iter().filter(|i| i.target == *target).collect();
        if let Target::Node(id) = target {
            if let Some(node) = self.document.find(*id) {
                for (_, r) in self.cited_resources(*id, &node) {
                    match self.resources.state(r.key()) {
                        lm_io::ResourceState::Missing => return Loading::Missing { resource: r },
                        lm_io::ResourceState::Failed(e) => return Loading::Failed(e),
                        _ => {}
                    }
                    let rt = Target::Resource(r.clone());
                    mine.extend(all.iter().filter(|i| i.target == rt));
                }
            }
            // A node silent until built counts as loading from when the compiler asked for it
            let waiting = self.is_waiting(*id);
            if mine.is_empty() && waiting {
                return Loading::Pending;
            }
        }
        if let Some(e) = self.activity.failure(target)
            && mine.is_empty()
        {
            return Loading::Failed(e.clone());
        }
        loading_of(&mine)
    }

    fn cited_resources(&self, id: NodeId, node: &Value) -> Vec<(Vec<PathStep>, Resource)> {
        let Some(t) = self.document.type_of(id) else { return Vec::new() };
        let mut out = self.schemas.resources(t, node).unwrap_or_default();
        // A plugin's state is `Any` in its node: its own schema names its resources
        if t == lm_doc::schemas::types::PLUGIN
            && let (Some(p), Some(state)) = (node.get("plugin").and_then(Value::as_text), node.get("state"))
        {
            out.extend(self.schemas.resources(&lm_plugin::plugin_type(p), state).unwrap_or_default());
        }
        out
    }

    /// Starts a long operation on a worker (I-2.2): its result comes back through the pump. For the session's own
    /// kinds of operation see `open`, `save`, `import`...; this one is for actions that compute a value
    pub fn start(
        &mut self,
        label: Label,
        target: Target,
        cancellable: bool,
        kind: &'static str,
        work: impl FnOnce(&Cancel, &Progress) -> Result<Value> + Send + 'static,
    ) -> lm_base::OperationId {
        self.activity.clear(&target);
        let handle = self.workers.start(work);
        let id = handle.id();
        let started = self.clock.now();
        self.generic_ops.push(Op { handle, label, target, cancellable, started, data: kind });
        id
    }

    // ---- the pump (I-16, T17) ----

    /// One turn of the main thread's work, in the order of T17. True when something shown may have changed
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        // 1. what the audio gave back; the compiler frees indices and slots
        let compiler = &mut self.compiler;
        let returned = self.engine.collect(|r| compiler.returned(&r));
        changed |= returned > 0;
        // 2. telemetry: the frame and the event rings
        changed |= self.read_telemetry();
        self.poll_device();
        // 3. plugin requests (in the telemetry), and the takes
        changed |= self.poll_disk();
        // 4. the operations
        changed |= self.poll_operations();
        // 5. gestures dropped without commit, feeds dropped
        let delivered = self.document.deliver();
        for (_, r) in delivered.committed {
            if let Err(e) = r {
                self.notices.error(&e, None, None);
            }
        }
        let _ = self.ui_document.deliver();
        // 6 and 7. the changes since the last pump: parameter values to their slots (undo, redo, transactions), the
        // compile when the shape changed
        if let Some(diff) = self.feed.drain(&self.document) {
            changed = true;
            self.apply_params(&diff);
            if diff.structural {
                self.needs_compile = true;
            }
            for n in &diff.notices {
                self.notices.label(n, Severity::Warning);
            }
        }
        // 8. compile if the root changed shape
        if self.needs_compile {
            self.submit();
        }
        // 9. the ready installs
        changed |= self.poll_compiler();
        // 10. the journal and the autosave
        self.journal_turn();
        self.forget_records();
        let finished_recordings: Vec<u64> = self.recordings.keys().copied().collect();
        for r in finished_recordings {
            self.commit_recording_if_whole(r);
        }
        self.refresh_status();
        changed
    }

    /// Pumps until nothing waits (builds, loads, compilations, installs), at most `timeout` (tests, the export of a
    /// session, closing)
    pub fn settle(&mut self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        loop {
            self.pump();
            if self.settled() {
                return true;
            }
            if start.elapsed() > timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn apply_params(&mut self, diff: &Diff) {
        for (node, key, value) in &diff.params {
            for slot in self.table.slots_of(*node, key) {
                self.engine.param_write(*slot, *value as f32);
            }
        }
    }

    /// Writes a parameter's slots at once (a knob being dragged, without an edit)
    pub fn param_write(&mut self, node: NodeId, key: &str, value: f32) {
        for slot in self.table.slots_of(node, key) {
            self.engine.param_write(*slot, value);
        }
    }

    fn environment(&self) -> Environment {
        Environment {
            playhead: self.telemetry.position.samples,
            low_latency_monitoring: self.preferences.bool(&["lowLatencyMonitoring"]).unwrap_or(false),
            ..Environment::default()
        }
    }

    fn snapshot(&self) -> ResourceSnapshot {
        let mut missing = BTreeSet::new();
        let resources = &self.resources;
        self.document.root().walk(&mut |_, v| {
            if let Some(r) = v.as_resource()
                && resources.state(r.key()) == lm_io::ResourceState::Missing
            {
                missing.insert(r.key().to_owned());
            }
        });
        ResourceSnapshot { view: self.resources.view(), missing }
    }

    fn submit(&mut self) {
        self.needs_compile = false;
        let snapshot = self.snapshot();
        let env = self.environment();
        self.compiler.submit(self.document.snapshot(), Arc::new(snapshot), env);
        if self.compiling_since.is_none() {
            self.compiling_since = Some(self.clock.now());
        }
    }

    fn poll_compiler(&mut self) -> bool {
        let mut changed = false;
        while let Some(result) = self.compiler.poll() {
            changed = true;
            match result {
                Ok(c) => self.installed(c),
                Err(e) => match e.key() {
                    "compile.limits.nodes_pending" | "compile.limits.params_pending" => {}
                    _ => self.notices.error(&e, Some(Target::Project), None),
                },
            }
        }
        if self.compiler.activity().is_none() {
            self.compiling_since = None;
        }
        changed
    }

    fn installed(&mut self, c: Compiled) {
        let Compiled { install, builds, cancelled, table, waiting, replacing, waiting_items, .. } = c;
        if let Err(e) = self.engine.install(install) {
            self.notices.error(&e, Some(Target::Project), None);
            return;
        }
        self.table = table;
        self.waiting_nodes = waiting.into_iter().collect();
        self.replacing_nodes = replacing.into_iter().collect();
        for t in cancelled {
            self.builds_waiting.retain(|b| b.ticket != t);
            for op in &self.build_ops.list {
                if op.data.ticket == t {
                    op.handle.cancel();
                }
            }
        }
        for b in builds {
            self.builds_waiting.retain(|w| w.node != b.node);
            self.builds_waiting.push(b);
        }
        for w in waiting_items {
            if !w.missing {
                self.request(&w.resource);
            }
        }
        self.start_ready_builds();
        // Write the tree's parameter values to the slots: a slot bound by this install starts from the tree (S20)
    }

    /// Loads a resource in an operation on its target (I-12), once
    pub fn request(&mut self, r: &Resource) {
        if self.load_ops.list.iter().any(|op| op.data == r.key()) {
            return;
        }
        match self.resources.begin_load(r.key()) {
            Ok(job) => {
                let delay = self.delay;
                let target = Target::Resource(r.clone());
                self.activity.clear(&target);
                let handle = self.workers.start(move |cancel, progress| {
                    sleep_cancellable(delay, cancel)?;
                    job.run(cancel, progress)
                });
                let started = self.clock.now();
                self.load_ops.push(Op {
                    handle,
                    label: Label::new("session.loading_resource"),
                    target,
                    cancellable: true,
                    started,
                    data: r.key().to_owned(),
                });
            }
            Err(lm_io::ResourceState::Missing) => {
                self.needs_compile = true;
            }
            Err(lm_io::ResourceState::Failed(e)) => {
                self.activity.fail(Target::Resource(r.clone()), e);
            }
            Err(_) => {}
        }
    }

    /// Starts the builds whose resources are all loaded (C11); asks for the others' resources
    fn start_ready_builds(&mut self) {
        let waiting = std::mem::take(&mut self.builds_waiting);
        for b in waiting {
            let cited = self.build_resources(&b);
            let mut ready = true;
            for r in &cited {
                match self.resources.state(r.key()) {
                    lm_io::ResourceState::Ready | lm_io::ResourceState::OnDisk => {}
                    lm_io::ResourceState::Missing | lm_io::ResourceState::Failed(_) => {
                        // A missing resource: the plugin decides (a wave not played is no error); built without it
                    }
                    _ => {
                        ready = false;
                        self.request(r);
                    }
                }
            }
            if ready {
                self.start_build(b, &cited);
            } else {
                self.builds_waiting.push(b);
            }
        }
    }

    fn build_resources(&self, b: &BuildRequest) -> Vec<Resource> {
        match &b.build {
            lm_compile::Build::Plugin { plugin, state, .. } => self
                .plugins
                .complete(plugin, state)
                .ok()
                .and_then(|s| self.schemas.resources(&lm_plugin::plugin_type(plugin), &s).ok())
                .unwrap_or_default()
                .into_iter()
                .map(|(_, r)| r)
                .collect(),
            lm_compile::Build::Rack { .. } => Vec::new(),
        }
    }

    fn start_build(&mut self, b: BuildRequest, cited: &[Resource]) {
        let view = self.resources.view();
        let mut resources = lm_plugin::ResourceView::new();
        for r in cited {
            if let Some(d) = view.get(r.key()) {
                resources.insert(r.clone(), lm_plugin::Loaded::Audio(d.clone()));
            }
        }
        let plugins = self.plugins.clone();
        let schemas = self.schemas.clone();
        let delay = self.delay;
        let BuildRequest { ticket, node, build, seed, spec, .. } = b;
        let target = match node.owner() {
            Some(id) => Target::Node(id),
            None => Target::Project,
        };
        let handle = self.workers.start(move |cancel, _| {
            sleep_cancellable(delay, cancel)?;
            build_processor(&plugins, &schemas, &build, spec, &resources, seed)
        });
        let started = self.clock.now();
        self.build_ops.push(Op {
            handle,
            label: Label::new("session.building"),
            target,
            cancellable: false,
            started,
            data: BuildData { ticket, node, seed },
        });
    }

    fn poll_operations(&mut self) -> bool {
        let now = self.clock.now();
        let mut done = Vec::new();
        let mut changed = false;
        for (op, result) in self.build_ops.finished(now, &mut done) {
            changed = true;
            match result {
                Ok(p) => self.compiler.ready(op.data.ticket, NodeCore::new(p, op.data.seed, None)),
                Err(e) if e.is_cancelled() => {}
                Err(e) => {
                    self.notices.error(&e, Some(op.target.clone()), None);
                    self.activity.fail(op.target, e);
                    let _ = op.data.node;
                }
            }
        }
        for (op, result) in self.load_ops.finished(now, &mut done) {
            changed = true;
            match result {
                Ok(loaded) => {
                    self.resources.finish(&op.data, Ok(loaded));
                    self.needs_compile = true;
                }
                Err(e) if e.is_cancelled() => self.resources.finish(&op.data, Err(e)),
                Err(e) => {
                    self.activity.fail(op.target.clone(), e.clone());
                    self.resources.finish(&op.data, Err(e));
                }
            }
        }
        if changed {
            self.start_ready_builds();
        }
        for (op, result) in self.open_ops.finished(now, &mut done) {
            changed = true;
            match result {
                Ok(read) => {
                    if let Err(e) = self.finish_open(read) {
                        self.notices.error(&e, Some(Target::Project), None);
                        self.activity.fail(Target::Project, e);
                    }
                }
                Err(e) if e.is_cancelled() => {}
                Err(e) => {
                    self.notices.error(&e, Some(Target::Project), None);
                    self.activity.fail(op.target, e);
                }
            }
        }
        for (op, result) in self.import_ops.finished(now, &mut done) {
            changed = true;
            let id = op.handle.id();
            self.imported.push((
                id,
                match &result {
                    Ok(file) => Resource::new("audio", &file.key),
                    Err(e) => Err(e.clone()),
                },
            ));
            match result {
                Ok(file) => {
                    let peaks = file.peaks.clone();
                    let path = file.path.clone();
                    match lm_io::load_file(&path, &Cancel::new(), &Progress::new()) {
                        Ok(data) => self.resources.loaded(file.key.clone(), data, peaks),
                        Err(e) => self.notices.error(&e, None, None),
                    }
                    self.status_imported(file.key);
                }
                Err(e) if e.is_cancelled() => {}
                Err(e) => {
                    self.notices.error(&e, Some(op.target.clone()), None);
                    self.activity.fail(op.target, e);
                }
            }
        }
        let takes_done = self.take_ops.finished(now, &mut done);
        if !takes_done.is_empty() {
            self.next_take_after = true;
        }
        for (op, result) in takes_done {
            changed = true;
            match result {
                Ok(take) => self.take_finalized(op.data, take),
                Err(e) => {
                    self.notices.error(&e, Some(op.target.clone()), None);
                    self.activity.fail(op.target, e);
                    if let Some(r) = self.recordings.get_mut(&op.data) {
                        r.failed += 1;
                    }
                    self.commit_recording_if_whole(op.data);
                }
            }
        }
        if std::mem::take(&mut self.next_take_after) {
            self.next_take();
        }
        for (op, result) in self.generic_ops.finished(now, &mut done) {
            changed = true;
            self.generic_finished(op.data, op.target, result);
        }
        self.finished.extend(done.iter().cloned());
        self.activity.record(done);
        changed
    }

    fn generic_finished(&mut self, kind: &'static str, target: Target, result: Result<Value>) {
        if kind == "save_as" && result.is_err() {
            self.pending_save_as = None;
        }
        match (kind, result) {
            ("export", Ok(report)) => {
                self.status = self.status.with("lastExport", report);
            }
            ("save_as", Ok(_)) => {
                if let Some((_, to, lock)) = self.pending_save_as.take()
                    && let Err(e) = self.finish_save_as(to, lock)
                {
                    self.notices.error(&e, Some(Target::Project), None);
                }
            }
            ("paste", Ok(list)) => {
                for r in list.items().filter_map(Value::as_resource).cloned().collect::<Vec<_>>() {
                    self.request(&r);
                }
                self.needs_compile = true;
            }
            (_, Ok(_)) => {}
            (_, Err(e)) if e.is_cancelled() => {}
            (kind, Err(e)) => {
                if kind == "export" {
                    let report = Value::object([("ok", Value::bool(false)), ("error", Value::text(e.key()))]);
                    self.status = self.status.with("lastExport", report);
                }
                self.notices.error(&e, Some(target.clone()), None);
                self.activity.fail(target, e);
            }
        }
    }

    fn status_imported(&mut self, _key: String) {
        self.needs_compile = true;
    }

    // ---- telemetry (I-8) ----

    fn read_telemetry(&mut self) -> bool {
        let table = self.table.clone();
        let frame = self.engine.frame().clone();
        let mut meters = std::collections::HashMap::new();
        if frame.generation == table.generation {
            for (key, index) in &table.nodes {
                let Some(id) = key.owner() else { continue };
                if table.meter_of(id) == Some(*index)
                    && let Some(m) = frame.meters.get(index.index as usize)
                    && u32::from(m.generation as u16) == m.generation
                {
                    meters.insert(id, *m);
                }
            }
        }
        let changed =
            frame.position != self.telemetry.position.samples || frame.playing != self.telemetry.position.playing;
        self.telemetry.position = Position {
            samples: frame.position,
            host_time: frame.host_time,
            sample_rate: self.spec.sample_rate,
            playing: frame.playing,
            recording: frame.recording,
        };
        if !meters.is_empty() || frame.generation == table.generation {
            self.telemetry.meters = meters;
        }
        self.telemetry.xruns = u32::try_from(frame.xruns).unwrap_or(u32::MAX).max(self.xruns);
        let mut events = Vec::new();
        self.engine.events(|t| events.push(t));
        for t in events {
            self.telemetry_event(t, &table);
        }
        let mut transport = Vec::new();
        self.engine.transport_events(|t| transport.push(t));
        for t in transport {
            if let TransportEvent::RecordStopped { .. } = t {
                self.recording_tracks.clear();
            }
        }
        let mut panics = Vec::new();
        self.engine.panics(|p| panics.push(p));
        for p in panics {
            self.notices.push(
                "session.plugin.panicked",
                vec![("message", Value::text(p.message()))],
                Severity::Error,
                None,
                Vec::new(),
            );
        }
        changed
    }

    fn telemetry_event(&mut self, t: Telemetry, table: &InstalledTable) {
        match t {
            Telemetry::TailEnded { node, .. } => self.compiler.tail_ended(node),
            Telemetry::PluginRequest { node, request: lm_render::PluginRequest::LatencyChanged, .. } => {
                if let Some(key) = table.key_of(node) {
                    // The new latency is read from the processor when it is built again (U23)
                    self.compiler.latency_changed(key, 0);
                    self.needs_compile = true;
                }
            }
            Telemetry::Xrun { .. } => {
                self.xruns = self.xruns.saturating_add(1);
                let at = self.telemetry.position.samples;
                self.telemetry.xrun_marks.push(at);
            }
            Telemetry::NodePanicked { node, .. } | Telemetry::NodeNonFinite { node, .. } => {
                let id = table.key_of(node).and_then(NodeKey::owner);
                self.notices.push(
                    "session.node.silenced",
                    vec![("node", Value::text(id.map(|i| i.to_string()).unwrap_or_default()))],
                    Severity::Error,
                    id.map(Target::Node),
                    Vec::new(),
                );
            }
            Telemetry::MasterProtection { peak, .. } => {
                self.notices.push(
                    "session.master.protection",
                    vec![("peak", Value::real(f64::from(peak)).unwrap_or(Value::NULL))],
                    Severity::Error,
                    None,
                    Vec::new(),
                );
            }
            Telemetry::LocateRefused => self.notices.warn("session.transport.locate_refused", Vec::new()),
            _ => {}
        }
    }

    fn poll_device(&mut self) {
        let Some(device) = self.device.as_mut() else { return };
        let mut events = Vec::new();
        let returned = device.poll(|e| events.push(e));
        drop(returned);
        for e in events {
            match e {
                DeviceEvent::Xrun { .. } => self.xruns = self.xruns.saturating_add(1),
                DeviceEvent::Disconnected | DeviceEvent::Failed { .. } | DeviceEvent::Panicked => {
                    self.notices.push(
                        "session.device.lost",
                        Vec::new(),
                        Severity::Error,
                        Some(Target::Device),
                        Vec::new(),
                    );
                    self.fall_back_to_dummy();
                }
                DeviceEvent::Started { realtime: Some(e) } => self.notices.error(&e, Some(Target::Device), None),
                _ => {}
            }
        }
        // Recording waits for the device to arm (lm-io: the first blocks would be lost otherwise)
        if let Some(rs) = &self.record_start
            && let Some(device) = self.device.as_ref()
            && device.is_armed()
        {
            let _ = rs.since;
            self.record_start = None;
            let _ = self.engine.commands().send(Command::Record { on: true });
            let _ = self.engine.commands().send(Command::Play);
        }
    }

    /// U11: the card went away; the renderer goes on on the Dummy, the session stays alive
    fn fall_back_to_dummy(&mut self) {
        let Some(mut device) = self.device.take() else { return };
        match device.stop() {
            Ok(renderer) => {
                match open_device(&DeviceSetup::Dummy(DummyOptions::default()), self.spec.sample_rate, self.block) {
                    Ok((mut dummy, _)) => match dummy.start(renderer) {
                        Ok(()) => self.device = Some(dummy),
                        Err((e, _)) => self.notices.error(&e, Some(Target::Device), None),
                    },
                    Err(e) => self.notices.error(&e, Some(Target::Device), None),
                }
            }
            Err(e) => self.notices.error(&e.to_error(), Some(Target::Device), None),
        }
    }

    // ---- the journal and the autosave (I-4, T11) ----

    fn journal_turn(&mut self) {
        let now = self.clock.now();
        let mut events = Vec::new();
        self.journal.poll(|e| events.push(e));
        for e in events {
            match e {
                JournalEvent::LineFailed { error, .. } => {
                    self.notices.error(&error, Some(Target::Project), None);
                    // The autosave that follows makes up for the lost line
                    self.autosave_now();
                }
                JournalEvent::AutosaveFailed(e) => {
                    if !self.autosave_warned {
                        self.autosave_warned = true;
                        self.notices.error(&e, Some(Target::Project), None);
                    }
                }
                JournalEvent::AutosaveDone { .. } => {
                    let at = crate::lock::now_seconds().to_string();
                    let file = self.project.folder.autosave().display().to_string();
                    self.status = self
                        .status
                        .with("autosave", Value::object([("at", Value::text(at)), ("file", Value::text(file))]));
                }
                JournalEvent::Saved { id, .. } => {
                    if let Some((sid, root, _)) = self.saving.take() {
                        if sid == id {
                            self.project.saved_root = root;
                            self.project.migrated_from = None;
                            self.project.untitled = false;
                        } else {
                            self.saving = Some((sid, root, now));
                        }
                    }
                }
                JournalEvent::SaveFailed { error, .. } => {
                    self.saving = None;
                    self.notices.error(&error, Some(Target::Project), Some(("project.save".into(), Value::NULL)));
                    self.activity.fail(Target::Project, error);
                }
            }
        }
        if self.project.read_only || !self.journal.is_enabled() {
            return;
        }
        // Gestures reach the journal at their commit, or every second while open (T11)
        let gesture_open = self.document.open_gestures() > 0;
        let due = !gesture_open || now.since(self.last_gesture_journal) >= GESTURE_JOURNAL.as_nanos() as u64;
        if due {
            self.last_gesture_journal = now;
            if let Some(patch) = self.document.take_journal() {
                self.write_journal(&patch);
            }
        }
        let every = self.preferences.int(&["autosaveSeconds"]).unwrap_or(60).max(5) as u64;
        if (self.modified() && now.since(self.last_autosave) >= every * 1_000_000_000) || self.journal.is_incomplete() {
            self.autosave_now();
        }
    }

    fn write_journal(&mut self, patch: &JournalPatch) {
        if let Err(e) = self.journal.write(patch) {
            self.notices.error(&e, Some(Target::Project), None);
        }
    }

    fn autosave_now(&mut self) {
        self.last_autosave = self.clock.now();
        if !self.project.read_only && self.journal.is_enabled() {
            self.journal.autosave(self.document.saved());
        }
    }

    /// The imports that ended since the last call, by their operation: the resource each made
    pub fn take_imported(&mut self) -> Vec<(lm_base::OperationId, Result<Resource>)> {
        std::mem::take(&mut self.imported)
    }

    /// The operations that finished since the last call (the headless printer; the duration profile, I-2.3)
    pub fn take_finished(&mut self) -> Vec<Finished> {
        std::mem::take(&mut self.finished)
    }

    /// Waits until the journal is on the disk (closing; the crash tests)
    pub fn sync_journal(&mut self, timeout: Duration) -> bool {
        if let Some(patch) = self.document.take_journal() {
            self.write_journal(&patch);
        }
        self.journal.sync(timeout)
    }

    // ---- the project: new, open, save (H3, H10, H11, I-4) ----

    /// Saves the project where it is (an untitled one needs `save_as`). On the disk thread, in order with the
    /// journal; the loading shows on the project while it runs. Keys `session.project.read_only`,
    /// `session.project.untitled`
    pub fn save(&mut self) -> Result<()> {
        if self.project.read_only {
            return Err(err("session.project.read_only"));
        }
        if self.project.untitled {
            return Err(err("session.project.untitled"));
        }
        self.save_here()
    }

    fn save_here(&mut self) -> Result<()> {
        // Everything before the save goes into the file, not the journal
        if let Some(patch) = self.document.take_journal() {
            self.write_journal(&patch);
        }
        self.save_serial += 1;
        let id = self.save_serial;
        let root = self.document.root().clone();
        let tree = self.document.saved();
        let ui = self.ui_document.root().clone();
        if !self.journal.save(tree, Some(ui), self.project.migrated_from, id) {
            return Err(err("session.disk.busy"));
        }
        self.saving = Some((id, root, self.clock.now()));
        Ok(())
    }

    /// Saves into another folder ("save as"): the folder is made, the audio the project cites copied (an
    /// operation), and the project goes on there (with its own lock, journal and resources)
    pub fn save_as(&mut self, folder: PathBuf) -> Result<()> {
        let to = ProjectFolder::new(folder);
        to.create(&self.faults)?;
        let (lock, locking) = ProjectLock::take(&to, &self.faults)?;
        if let Locking::Held(owner) = locking {
            return Err(err("session.project.locked").arg("host", owner.host).arg("pid", i64::from(owner.pid)));
        }
        // The audio files cited (content-addressed: those already there are kept), copied in an operation; the
        // project moves there when they are all in
        let from_audio = self.project.folder.audio();
        let mut keys = BTreeSet::new();
        self.document.root().walk(&mut |_, v| {
            if let Some(r) = v.as_resource() {
                keys.insert(r.key().to_owned());
            }
        });
        let mut copies = Vec::new();
        for entry in std::fs::read_dir(&from_audio).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name.split('.').next().unwrap_or("");
            let dest = to.audio().join(&name);
            if keys.contains(stem) && !dest.exists() {
                copies.push((entry.path(), dest));
            }
        }
        if copies.is_empty() {
            return self.finish_save_as(to, lock);
        }
        let faults = self.faults.clone();
        let id = self.start(
            Label::new("session.save_as.copying"),
            Target::Project,
            true,
            "save_as",
            move |cancel, progress| {
                for (i, (from, dest)) in copies.iter().enumerate() {
                    cancel.check()?;
                    faults.check(std::fs::metadata(from).map(|m| m.len()).unwrap_or(0))?;
                    let tmp = dest.with_extension("save_as.tmp");
                    std::fs::copy(from, &tmp).map_err(|e| crate::folder::io_error("session.save_as.copy", &tmp, &e))?;
                    std::fs::rename(&tmp, dest)
                        .map_err(|e| crate::folder::io_error("session.save_as.copy", dest, &e))?;
                    progress.set((i + 1) as f32 / copies.len() as f32);
                }
                Ok(Value::NULL)
            },
        );
        self.pending_save_as = Some((id, to, lock));
        Ok(())
    }

    /// The project moves to its new folder: its journal, its resources, its lock; then it is saved there
    fn finish_save_as(&mut self, to: ProjectFolder, lock: ProjectLock) -> Result<()> {
        self.journal = Journal::start(&to, self.faults.clone(), String::new(), self.journal.seq(), false);
        self.resources = ResourceStore::new(to.root().to_path_buf(), self.clock.clone());
        // The untitled folder is no longer ours to remove (its audio was copied; it goes now if it holds none)
        drop(std::mem::replace(&mut self.untitled_guard, UntitledGuard(None)));
        self.project.folder = to;
        self.project.lock = lock;
        self.project.untitled = false;
        self.project.read_only = false;
        self.project.locked_by = None;
        self.needs_compile = true;
        self.save_here()?;
        self.preferences.add_recent(&self.project.folder.name(), &self.project.folder.root().display().to_string());
        self.request_all_resources();
        Ok(())
    }

    /// Closes the project: a new untitled one takes its place (its lock released, its journal ended). The caller
    /// asks about saving before
    pub fn close_project(&mut self) -> Result<()> {
        let _ = self.engine.commands().send(Command::Stop);
        let folder = ProjectFolder::new(self.untitled.join(format!("untitled-{}", lm_io::new_uuid())));
        folder.create(&self.faults)?;
        if let Some(patch) = self.document.take_journal() {
            self.write_journal(&patch);
        }
        self.journal = Journal::start(&folder, self.faults.clone(), String::new(), 0, false);
        self.resources = ResourceStore::new(folder.root().to_path_buf(), self.clock.clone());
        let options = project_options(&self.plugins, &self.schemas, self.limits);
        let document = Document::new_project(self.schemas.clone(), options)?;
        self.replace_document(document, None)?;
        self.untitled_guard = UntitledGuard(Some(folder.root().to_path_buf()));
        self.project = Project {
            folder,
            lock: ProjectLock::none(),
            read_only: false,
            untitled: true,
            migrated_from: None,
            saved_root: self.document.root().clone(),
            locked_by: None,
        };
        self.refresh_status();
        Ok(())
    }

    /// Opens a project folder in an operation on the project (reading, the lock, the journal); the pump applies it
    pub fn open(&mut self, folder: PathBuf) -> lm_base::OperationId {
        let folder = ProjectFolder::new(folder);
        let faults = self.faults.clone();
        let delay = self.delay;
        self.activity.clear(&Target::Project);
        let handle = self.workers.start(move |cancel, progress| {
            sleep_cancellable(delay, cancel)?;
            read_project(folder, &faults, cancel, progress)
        });
        let id = handle.id();
        let started = self.clock.now();
        self.open_ops.push(Op {
            handle,
            label: Label::new("session.opening"),
            target: Target::Project,
            cancellable: true,
            started,
            data: (),
        });
        id
    }

    /// Makes a new project in `folder` and saves it there at once
    pub fn create(&mut self, folder: PathBuf) -> Result<()> {
        let options = project_options(&self.plugins, &self.schemas, self.limits);
        let document = Document::new_project(self.schemas.clone(), options)?;
        self.replace_document(document, None)?;
        self.save_as(folder)
    }

    fn replace_document(&mut self, mut document: Document, ui: Option<Value>) -> Result<()> {
        let _ = self.engine.commands().send(Command::Stop);
        self.feed = document.feed(Topic::All);
        self.document = document;
        self.ui_document = interface_document(ui)?;
        self.project.saved_root = self.document.root().clone();
        let rate = project_rate(self.document.root());
        if rate != self.spec.sample_rate {
            // A new compiler: the old one's requests are stale
            self.builds_waiting.clear();
            self.build_ops.cancel_all();
            self.restart_audio(rate)?;
        }
        // Otherwise the compiler says which requests are stale (`Compiled::cancelled`); the others still count
        self.needs_compile = true;
        Ok(())
    }

    /// A project at another rate: the device is opened again at it, with a new engine and compiler
    fn restart_audio(&mut self, rate: u32) -> Result<()> {
        if let Some(mut d) = self.device.take() {
            let _ = d.stop();
            d.close();
        }
        let (mut device, info) = open_device(&self.device_setup, rate, self.block)
            .or_else(|_| open_device(&DeviceSetup::Dummy(DummyOptions::default()), rate, self.block))?;
        let spec = info.spec();
        let (engine, renderer) = lm_render::engine_with(spec, self.limits, RenderThreads::default());
        device.start(renderer).map_err(|(e, _)| e)?;
        self.engine = engine;
        self.compiler = Compiler::new(CompilerConfig {
            spec,
            limits: self.limits,
            schemas: self.schemas.clone(),
            plugins: self.plugins.clone(),
        });
        self.table = Arc::new(InstalledTable::default());
        self.spec = spec;
        self.device = Some(device);
        self.telemetry = empty_telemetry(rate);
        Ok(())
    }

    fn finish_open(&mut self, read: ReadProject) -> Result<()> {
        let ReadProject { folder, bytes_hash, saved, journal, autosave, interface, lock, locking, interrupted } = read;
        let options = || project_options(&self.plugins, &self.schemas, self.limits);
        let opened = Document::open(self.schemas.clone(), saved.clone(), options())?;
        for n in &opened.notices {
            self.notices.label(n, Severity::Warning);
        }
        let migrated_from = if opened.upgraded {
            saved
                .get("versions")
                .and_then(|v| v.get("project"))
                .and_then(Value::as_int)
                .and_then(|v| u32::try_from(v).ok())
        } else {
            None
        };
        let mut document = opened.document;
        let saved_root = document.root().clone();
        let mut replayed = false;
        let mut seq = 0;
        if journal.base.as_deref() == Some(bytes_hash.as_str()) {
            // The autosave holds what an incomplete journal lost
            let (base_doc, from_seq) = match &autosave {
                // The autosave has everything up to its number: it wins when the journal lacks some of it (a line
                // the disk refused, or marked incomplete)
                Some(a)
                    if a.base == bytes_hash
                        && (journal.incomplete || a.seq > journal.patches.last().map_or(0, |(s, _)| *s)) =>
                {
                    (Document::open(self.schemas.clone(), a.saved.clone(), options())?.document, a.seq)
                }
                _ => (document, 0),
            };
            let patches: Vec<JournalPatch> =
                journal.patches.iter().filter(|(s, _)| *s > from_seq).map(|(_, p)| p.clone()).collect();
            seq = journal.patches.last().map_or(from_seq, |(s, _)| *s).max(from_seq);
            if patches.is_empty() && from_seq == 0 {
                document = base_doc;
            } else {
                let root = lm_doc::replay(base_doc.root(), &patches)?;
                document = Document::from_root(self.schemas.clone(), lm_doc::PROJECT, root, options())?.document;
                replayed = true;
                self.notices.info("session.journal.recovered", vec![("steps", Value::int(patches.len() as i64))]);
            }
            if journal.broken_tail > 0 {
                self.notices
                    .warn("session.journal.broken_tail", vec![("lines", Value::int(journal.broken_tail as i64))]);
            }
        } else if journal.base.is_some() || !journal.patches.is_empty() {
            // A journal of another save: kept apart, never applied
            if let Ok(aside) = set_aside(&folder.journal()) {
                self.notices
                    .warn("session.journal.set_aside", vec![("file", Value::text(aside.display().to_string()))]);
            }
        }
        let read_only = document.read_only() || matches!(locking, Locking::Held(_));
        match &locking {
            Locking::Held(owner) => self.notices.warn(
                "session.project.locked",
                vec![("host", Value::text(owner.host.as_str())), ("pid", Value::int(i64::from(owner.pid)))],
            ),
            Locking::TookStale(owner) => self.notices.info(
                "session.project.stale_lock",
                vec![("host", Value::text(owner.host.as_str())), ("pid", Value::int(i64::from(owner.pid)))],
            ),
            Locking::Taken => {}
        }
        if document.read_only() {
            self.notices.warn("session.project.newer", Vec::new());
        }
        // The project is in: the old one's journal ends (its lines are all written), the new one's starts
        let _ = self.journal.sync(Duration::from_secs(2));
        self.journal = if read_only {
            Journal::disabled(&folder)
        } else {
            Journal::start(&folder, self.faults.clone(), bytes_hash, seq, !replayed)
        };
        self.resources = ResourceStore::new(folder.root().to_path_buf(), self.clock.clone());
        self.replace_document(document, interface)?;
        drop(std::mem::replace(&mut self.untitled_guard, UntitledGuard(None)));
        self.project = Project {
            locked_by: match &locking {
                Locking::Held(o) => Some(o.clone()),
                _ => None,
            },
            folder,
            lock,
            read_only,
            untitled: false,
            migrated_from,
            saved_root,
        };
        if !interrupted.is_empty() {
            self.notices.warn("session.takes.interrupted", vec![("count", Value::int(interrupted.len() as i64))]);
        }
        self.preferences.add_recent(&self.project.folder.name(), &self.project.folder.root().display().to_string());
        self.request_all_resources();
        self.refresh_status();
        Ok(())
    }

    /// Forgets every resource and loads them again (files replaced on the disk)
    pub fn reload_resources(&mut self) {
        let mut all = Vec::new();
        self.document.root().walk(&mut |_, v| {
            if let Some(r) = v.as_resource() {
                all.push(r.key().to_owned());
            }
        });
        for key in all {
            self.resources.forget(&key);
        }
        self.request_all_resources();
    }

    /// Asks for every resource the project cites (opening: C11, the builds wait for them)
    fn request_all_resources(&mut self) {
        let mut all = Vec::new();
        self.document.root().walk(&mut |_, v| {
            if let Some(r) = v.as_resource() {
                all.push(r.clone());
            }
        });
        all.sort();
        all.dedup();
        for r in all {
            if r.kind() == "audio" {
                self.request(&r);
            }
        }
    }

    // ---- the status tree (lm-port `trees::status`) ----

    fn refresh_status(&mut self) {
        let p = &self.project;
        let interrupted: Vec<Value> = lm_io::find_interrupted(&p.folder.audio())
            .unwrap_or_default()
            .into_iter()
            .map(|path| {
                let track =
                    lm_io::read_record(&path).ok().and_then(|(h, _)| h.tracks.first().cloned()).unwrap_or_default();
                Value::object([
                    ("track", Value::text(track)),
                    ("file", Value::text(path.display().to_string())),
                    ("seconds", Value::real(0.0).unwrap_or(Value::NULL)),
                ])
            })
            .collect();
        let project = Value::object([
            ("name", Value::text(p.folder.name())),
            ("folder", Value::text(p.folder.root().display().to_string())),
            ("readOnly", Value::bool(p.read_only)),
            ("lockedBy", p.locked_by.as_ref().map_or(Value::NULL, |o| o.status())),
        ]);
        let base = if self.status.is_object() { self.status.clone() } else { Value::empty_object() };
        self.status = base
            .with("project", project)
            .with("recent", self.preferences.get(&["recent"]).cloned().unwrap_or_else(Value::empty_list))
            .with("templates", Value::empty_list())
            .with("interruptedTakes", Value::list(interrupted))
            .with("safeMode", Value::bool(false))
            .with("safeModePlugins", Value::empty_list());
        if self.status.get("lastExport").is_none() {
            self.status = self.status.with("lastExport", Value::NULL);
        }
        if self.status.get("autosave").is_none() {
            self.status = self.status.with("autosave", Value::NULL);
        }
    }

    // ---- preferences ----

    pub fn set_preference(&mut self, path: &[PathStep], value: Value) -> Result<()> {
        self.preferences.set(path, value)?;
        let file = self.preferences.file().cloned();
        if let Some(file) = file {
            let tree = self.preferences.tree().clone();
            let faults = self.faults.clone();
            self.start(Label::new("session.saving_preferences"), Target::Project, false, "preferences", move |_, _| {
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| crate::folder::io_error("session.preferences.write", dir, &e))?;
                }
                let text = lm_state::to_json(&tree)?;
                crate::folder::write_atomic(&file, text.as_bytes(), &faults)?;
                Ok(Value::NULL)
            });
        }
        self.needs_compile = true;
        Ok(())
    }

    // ---- transport (I-7.2) ----

    pub fn play(&mut self) {
        let _ = self.engine.commands().send(Command::Play);
    }

    pub fn stop(&mut self) {
        let _ = self.engine.commands().send(Command::Stop);
        if self.record_start.take().is_some() || self.recording_tracks.is_empty() {
            // nothing recording
        }
        let _ = self.engine.commands().send(Command::Record { on: false });
        if let Some(d) = self.device.as_mut() {
            d.disarm();
        }
    }

    pub fn locate(&mut self, sample: i64) {
        let _ = self.engine.commands().send(Command::Locate { sample: sample.max(0) });
    }

    pub fn panic(&mut self) {
        let _ = self.engine.commands().send(Command::Panic);
    }

    // ---- recording (I-11) ----

    /// Records the armed tracks (or `fallback`, armed for this take, when none is): the inputs are planned from the
    /// tracks' `input`, the disk writer and the device armed, and the transport starts once the device is armed.
    /// Keys `session.record.no_track`, `session.record.no_space` (H3: less than 30 s)
    pub fn record(&mut self, fallback: Option<NodeId>) -> Result<()> {
        if self.project.read_only {
            return Err(err("session.project.read_only"));
        }
        let mut tracks: Vec<(NodeId, Vec<u16>)> = Vec::new();
        for t in self.document.root().get("tracks").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default()
        {
            let Some(id) = t.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else { continue };
            let armed = t.get("armed").and_then(Value::as_bool) == Some(true) || fallback == Some(id);
            if !armed {
                continue;
            }
            let channels: Vec<u16> = t
                .get("input")
                .and_then(|i| i.get("channels"))
                .map(|l| {
                    l.items().filter_map(Value::as_int).map(|c| u16::try_from((c - 1).max(0)).unwrap_or(0)).collect()
                })
                .unwrap_or_else(|| vec![0]);
            tracks.push((id, channels));
        }
        if tracks.is_empty() {
            return Err(err("session.record.no_track"));
        }
        let audio = self.project.folder.audio();
        let total: usize = tracks.iter().map(|(_, c)| c.len()).sum();
        match lm_io::check_record_space(&audio, total, self.spec.sample_rate) {
            Ok(lm_io::SpaceCheck::Refused { .. }) => return Err(err("session.record.no_space")),
            Ok(lm_io::SpaceCheck::Low { .. }) => self.notices.warn("session.record.low_space", Vec::new()),
            _ => {}
        }
        self.faults.check(0)?;
        let inputs = tracks
            .iter()
            .enumerate()
            .map(|(i, (id, channels))| RecordInput {
                input: lm_io::InputIndex(u32::try_from(i).unwrap_or(0)),
                channels: channels.clone(),
                tracks: vec![id.to_string()],
            })
            .collect();
        let plan = RecordPlan::new(audio, inputs, self.spec.sample_rate);
        let (sender, receiver) = lm_io::recording_channel(plan)?;
        let disk = self.disk.as_mut().ok_or_else(|| err("session.record.no_disk"))?;
        disk.record(receiver)?;
        let device = self.device.as_mut().ok_or_else(|| err("session.record.no_device"))?;
        if device.arm(sender).is_err() {
            return Err(err("session.record.busy"));
        }
        self.recording_serial += 1;
        let inputs = tracks.len();
        self.recordings.insert(self.recording_serial, PendingRecording { inputs, ..PendingRecording::default() });
        self.recording_tracks = tracks.into_iter().map(|(id, _)| id).collect();
        self.record_start = Some(RecordStart { since: self.clock.now() });
        Ok(())
    }

    fn poll_disk(&mut self) -> bool {
        let Some(disk) = self.disk.as_mut() else { return false };
        let mut events = Vec::new();
        disk.poll(|e| events.push(e));
        let mut changed = false;
        for e in events {
            changed = true;
            match e {
                DiskEvent::TakeClosed(take) => {
                    // Takes close in the order their recordings started: the oldest recording still gathering
                    let recording = self.recordings.keys().next().copied().unwrap_or(0);
                    self.finalize(take, recording);
                }
                DiskEvent::InputStopped { reason, seconds, error, .. } => {
                    let key = match reason {
                        lm_io::StopReason::DiskFull => "session.record.disk_full",
                        _ => "session.record.overflow",
                    };
                    self.notices.push(
                        key,
                        vec![("seconds", Value::real(seconds).unwrap_or(Value::NULL))],
                        Severity::Error,
                        None,
                        Vec::new(),
                    );
                    if let Some(e) = error {
                        self.notices.error(&e, None, None);
                    }
                }
                DiskEvent::Failed { error, .. } => self.notices.error(&error, None, None),
                _ => {}
            }
        }
        changed
    }

    /// Closes a take in an operation on the project (I-11), after the ones before it
    fn finalize(&mut self, take: Take, recording: u64) {
        self.take_queue.push_back((TakeJob::Recorded(take), recording));
        self.next_take();
    }

    fn next_take(&mut self) {
        if !self.take_ops.is_empty() {
            return;
        }
        let Some((job, recording)) = self.take_queue.pop_front() else { return };
        let format = match self.preferences.text(&["recording", "format"]) {
            Some("rf64-32f") => TakeFormat::Float32,
            _ => TakeFormat::Int24,
        };
        let (label, cancellable) = match &job {
            TakeJob::Recorded(_) => ("session.closing_take", false),
            TakeJob::Interrupted(_) => ("session.recovering_take", true),
        };
        let handle = self.workers.start(move |cancel, progress| {
            let take = match job {
                TakeJob::Recorded(t) => t,
                TakeJob::Interrupted(record) => lm_io::recover_take(&record, cancel, progress)?,
            };
            lm_io::finalize_take(&take, format, cancel, progress)
        });
        let started = self.clock.now();
        self.take_ops.push(Op {
            handle,
            label: Label::new(label),
            target: Target::Project,
            cancellable,
            started,
            data: recording,
        });
    }

    /// Recovers the interrupted takes of the project (U5): operations; each take goes back to its track, or to a
    /// new "Take recuperado" track when its track is gone (H8)
    pub fn recover_takes(&mut self) -> Result<()> {
        for record in lm_io::find_interrupted(&self.project.folder.audio())? {
            self.take_queue.push_back((TakeJob::Interrupted(record), 0));
        }
        self.next_take();
        Ok(())
    }

    /// A take on the disk: it waits for the other takes of its recording (a recovered one goes alone)
    fn take_finalized(&mut self, recording: u64, f: FinalizedTake) {
        if let Ok(data) = lm_io::load_file(&f.take.path, &Cancel::new(), &Progress::new()) {
            self.resources.loaded(f.key.clone(), data, f.peaks.clone());
        }
        if recording == 0 {
            self.commit_recording(vec![f]);
            return;
        }
        self.recordings.entry(recording).or_default().takes.push(f);
        self.commit_recording_if_whole(recording);
    }

    fn commit_recording_if_whole(&mut self, recording: u64) {
        let whole = self.recordings.get(&recording).is_some_and(|r| r.takes.len() + r.failed >= r.inputs);
        if whole && let Some(r) = self.recordings.remove(&recording) {
            self.commit_recording(r.takes);
        }
    }

    /// The takes of one recording enter the tree in ONE transaction (`Origin::Recording`: one undo removes the whole
    /// recording), each on its track with the latency and the PDC taken off (H5), or on a new "Take recuperado"
    /// track when its track is gone (H8: a take is never thrown away). Their records are forgotten once the journal
    /// line is on the disk (U5)
    fn commit_recording(&mut self, takes: Vec<FinalizedTake>) {
        if takes.is_empty() {
            return;
        }
        let map = tempo_map(self.document.root());
        let mut placed = Vec::new();
        for f in &takes {
            let resource = match Resource::new("audio", &f.key) {
                Ok(r) => r,
                Err(e) => {
                    self.notices.error(&e, None, None);
                    continue;
                }
            };
            let take = &f.take;
            let start = (take.start_sample - take.latency - take.pdc).max(0);
            let (start_tick, length) = match &map {
                Ok(m) => {
                    let a = m.tick_at(start);
                    (a, (m.tick_at(start + take.frames) - a).max(1))
                }
                Err(_) => (0, 1),
            };
            let track = take.tracks.first().and_then(|t| NodeId::parse(t).ok()).filter(|t| self.document.exists(*t));
            let item = Value::object([
                ("kind", Value::text("audio")),
                ("name", Value::text("")),
                ("start", Value::int(start_tick)),
                ("length", Value::int(length)),
                (
                    "lanes",
                    Value::list([Value::object([
                        ("source", Value::resource(resource)),
                        ("sourceOffset", Value::int(0)),
                    ])]),
                ),
            ]);
            placed.push((track, item));
        }
        let mut recovered_track = false;
        let result = self.document.transact_as(Label::new("session.take"), Origin::Recording, |e| {
            let mut recovered: Option<NodeId> = None;
            for (track, item) in &placed {
                let t = match track {
                    Some(t) => *t,
                    None => match recovered {
                        Some(r) => r,
                        None => {
                            let tracks = e.root().get("tracks").map_or(0, Value::len);
                            let new = Value::object([("name", Value::text("Take recuperado"))]);
                            let r = e.insert(lm_state::ROOT, "tracks", tracks, new)?;
                            recovered = Some(r);
                            r
                        }
                    },
                };
                let items = e.find(t).and_then(|n| n.get("items").map(Value::len)).unwrap_or(0);
                e.insert(t, "items", items, item.clone())?;
            }
            recovered_track = recovered.is_some();
            // I-21: the MIDI take joins here
            Ok(())
        });
        match result {
            Ok(_) => {
                if recovered_track {
                    self.notices.warn("session.take.recovered_track", Vec::new());
                }
                if let Some(patch) = self.document.take_journal() {
                    self.write_journal(&patch);
                }
                if let Some(marker) = self.journal.marker() {
                    for f in takes {
                        self.forget_after.push((marker.clone(), f.take.record.clone()));
                    }
                }
            }
            Err(e) => self.notices.error(&e, None, None),
        }
    }

    /// The records of takes whose journal line is on the disk are forgotten (never before: a crash in between still
    /// finds the take, U5)
    fn forget_records(&mut self) {
        let mut i = 0;
        while i < self.forget_after.len() {
            if self.forget_after[i].0.load(Ordering::Acquire) {
                let (_, record) = self.forget_after.swap_remove(i);
                if let Err(e) = lm_io::forget_record(&record) {
                    self.notices.error(&e, None, None);
                }
            } else {
                i += 1;
            }
        }
    }

    // ---- import (I-12, H3) ----

    /// Imports an audio file into the project's `audio/` (copied or converted, hashed, peaks): an operation on the
    /// resource list; the result is loaded. A full disk fails before copying (H3)
    pub fn import(&mut self, source: PathBuf) -> lm_base::OperationId {
        let project = self.project.folder.root().to_path_buf();
        let rate = self.spec.sample_rate;
        let faults = self.faults.clone();
        let size = std::fs::metadata(&source).map(|m| m.len()).unwrap_or(0);
        let handle = self.workers.start(move |cancel, progress| {
            faults.check(size).map_err(|e| e.arg("file", source.display().to_string()))?;
            lm_io::import_file(&source, &project, rate, cancel, progress)
        });
        let id = handle.id();
        let started = self.clock.now();
        self.import_ops.push(Op {
            handle,
            label: Label::new("session.importing"),
            target: Target::Scan(lm_port::ScanKind::Library),
            cancellable: true,
            started,
            data: (),
        });
        id
    }

    // ---- copy and paste (H13) ----

    /// Copies a node (with everything under it) to a clipboard that can be pasted into any project
    pub fn copy(&self, id: NodeId) -> Result<Clipboard> {
        Ok(Clipboard { clip: self.document.copy(id)?, project: self.project.folder.root().to_path_buf() })
    }

    /// Pastes a clipboard at `index` of `list` of `parent`: new ids everywhere, the references inside fixed, those
    /// to the outside kept only when it came from this project (lm-doc). The audio it cites that this project lacks
    /// is copied into `audio/` in an operation, and the pasted items show loading until then
    pub fn paste(&mut self, clipboard: &Clipboard, parent: NodeId, list: &str, index: usize) -> Result<Option<NodeId>> {
        let mut made = None;
        self.document.transact(Label::new("session.paste"), |e| {
            made = e.paste(parent, list, index, &clipboard.clip)?;
            Ok(())
        })?;
        let mut cited = Vec::new();
        clipboard.clip.node.walk(&mut |_, v| {
            if let Some(r) = v.as_resource() {
                cited.push(r.clone());
            }
        });
        cited.sort();
        cited.dedup();
        let here = self.project.folder.root().to_path_buf();
        let from = clipboard.project.clone();
        let missing: Vec<Resource> =
            cited.into_iter().filter(|r| matches!(lm_io::resource_path(&here, r.key()), Ok(None))).collect();
        if !missing.is_empty() {
            let faults = self.faults.clone();
            self.start(Label::new("session.paste.copying"), Target::Project, true, "paste", move |cancel, progress| {
                let audio = here.join(crate::folder::AUDIO_DIR);
                for (i, r) in missing.iter().enumerate() {
                    cancel.check()?;
                    if let Some(source) = lm_io::resource_path(&from, r.key())? {
                        let name = source.file_name().map(|n| n.to_owned()).unwrap_or_default();
                        let dest = audio.join(name);
                        let tmp = dest.with_extension("paste.tmp");
                        faults.check(std::fs::metadata(&source).map(|m| m.len()).unwrap_or(0))?;
                        std::fs::copy(&source, &tmp)
                            .map_err(|e| crate::folder::io_error("session.paste.copy", &tmp, &e))?;
                        std::fs::rename(&tmp, &dest)
                            .map_err(|e| crate::folder::io_error("session.paste.copy", &dest, &e))?;
                        // The peaks go along (they are by hash too)
                        let peaks = source.with_extension("peaks");
                        if peaks.exists() {
                            let _ = std::fs::copy(&peaks, dest.with_extension("peaks"));
                        }
                    }
                    progress.set((i + 1) as f32 / missing.len() as f32);
                }
                Ok(Value::list(missing.into_iter().map(Value::resource)))
            });
        }
        Ok(made)
    }

    /// Exports the project as it is now into `target` (I-13): an operation on `Target::Export`; the report goes to
    /// the status tree's `lastExport`. It does not touch the transport: exporting while recording is safe
    pub fn export(&mut self, target: crate::export::ExportTarget) -> lm_base::OperationId {
        let job = self.export_job();
        self.export_serial += 1;
        let target_id = Target::Export(self.export_serial);
        self.start(Label::new("session.exporting"), target_id, true, "export", move |cancel, progress| {
            crate::export::export(job, &target, cancel, progress)
        })
    }

    /// The project as it is now, for an export
    pub fn export_job(&self) -> crate::export::ExportJob {
        crate::export::ExportJob {
            root: self.document.snapshot(),
            project: self.project.folder.root().to_path_buf(),
            plugins: self.plugins.clone(),
            schemas: self.schemas.clone(),
            limits: self.limits,
        }
    }

    /// The imported resources so far (by key), and whether each is loaded
    pub fn resource_state(&self, key: &str) -> lm_io::ResourceState {
        self.resources.state(key)
    }

    /// Ends the session the way U13 asks: the recording stops and closes, cancellable operations are cancelled,
    /// the others finish, the journal reaches the disk. The caller asks about saving before
    pub fn close(mut self) {
        self.stop();
        let lists: [&dyn OpList; 3] = [&self.open_ops, &self.load_ops, &self.import_ops];
        for l in lists {
            l.cancel_all();
        }
        let start = std::time::Instant::now();
        while (!self.take_ops.is_empty() || !self.build_ops.is_empty()) && start.elapsed() < Duration::from_secs(10) {
            self.pump();
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = self.sync_journal(Duration::from_secs(5));
    }
}

fn empty_telemetry(rate: u32) -> TelemetryView {
    TelemetryView {
        position: Position { samples: 0, host_time: HostTime(0), sample_rate: rate, playing: false, recording: false },
        meters: std::collections::HashMap::new(),
        inputs: Vec::new(),
        recording: Vec::new(),
        counting_in: false,
        xruns: 0,
        xrun_marks: Vec::new(),
    }
}

/// Sleeps `d` in small steps, stopping when cancelled (the test mode's injected delay, I-2.4)
pub(crate) fn sleep_cancellable(d: Duration, cancel: &Cancel) -> Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < d {
        cancel.check()?;
        std::thread::sleep(Duration::from_millis(1).min(d));
    }
    cancel.check()
}

/// Builds the processor of a build request (in an operation): a plugin through the registry, or a rack segment
pub fn build_processor(
    plugins: &Registry,
    schemas: &SchemaRegistry,
    build: &lm_compile::Build,
    spec: ProcessSpec,
    resources: &lm_plugin::ResourceView,
    seed: u64,
) -> Result<Box<dyn Processor>> {
    match build {
        lm_compile::Build::Plugin { plugin, state, channels, .. } => {
            let cx = lm_plugin::BuildContext { spec, resources, channels: *channels };
            plugins.build(plugin, state, &cx)
        }
        lm_compile::Build::Rack { master, stages, rack, channels, channel } => {
            let params = lm_compile::rack_params(schemas, *master);
            let _ = channel;
            Ok(Box::new(RackProcessor::new(*master, stages, rack, params, *channels, spec.sample_rate, seed)?))
        }
    }
}

/// Reads a project folder (the work of the open operation): the file, the lock, the journal, the autosave, the
/// interface document, the interrupted takes
pub fn read_project(
    folder: ProjectFolder,
    faults: &DiskFaults,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<ReadProject> {
    progress.stage(Label::new("session.open.reading"));
    let path = folder.project();
    let bytes = std::fs::read(&path).map_err(|e| crate::folder::io_error("session.open.read", &path, &e))?;
    cancel.check()?;
    let text = std::str::from_utf8(&bytes).map_err(|_| err("session.open.not_utf8"))?;
    let saved = from_json(text)?;
    progress.set(0.5);
    let bytes_hash = base_hash(&bytes);
    let journal = read_journal(&folder.journal())?;
    let autosave = read_autosave(&folder.autosave());
    let interface = std::fs::read_to_string(folder.interface()).ok().and_then(|t| from_json(&t).ok());
    for dir in [folder.audio(), folder.backup()] {
        let _ = std::fs::create_dir_all(&dir);
    }
    let (lock, locking) = ProjectLock::take(&folder, faults)?;
    let interrupted = lm_io::find_interrupted(&folder.audio()).unwrap_or_default();
    let _ = lm_io::clean_import_leftovers(folder.root());
    progress.set(1.0);
    Ok(ReadProject { folder, bytes_hash, saved, journal, autosave, interface, lock, locking, interrupted })
}

/// Where a project's file lives, for the tools
pub fn project_file(folder: &Path) -> PathBuf {
    ProjectFolder::new(folder).project()
}
