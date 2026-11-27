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
use lm_render::{Command, EngineControl, Limits, NodeCore, ProcessSpec, Processor, Telemetry, TransportEvent};
use lm_state::{NodeId, PathStep, Resource, SchemaRegistry, Value, from_json};

use crate::activity::{Activity, Finished, Op, OpList, Ops, loading_of};
use crate::folder::{DiskFaults, ProjectFolder};
use crate::journal::{Journal, JournalEvent, base_hash, read_autosave, read_journal, set_aside};
use crate::lock::{Locking, ProjectLock};
use crate::notices::Notices;
use crate::prefs::Preferences;
use crate::rack::RackProcessor;

/// MIDI and automation (I-21): a child module, so it works on the session's own fields
#[path = "midi/mod.rs"]
pub(crate) mod midi;

/// The OSC server (`decisoes-autonomas.md`, 26/11, item 5): a child module, so it works on the session's own fields
#[path = "osc/mod.rs"]
pub(crate) mod osc;

/// Third-party plugins (I-9.6): the scan, the catalog growing, the helpers, their state and notices
#[path = "external.rs"]
pub(crate) mod external;

/// The actions the interface sends that the session answers (project templates, the zip, cleaning, takes, the safe
/// mode, the devices, locating files, presets): a child module, so it works on the session's own fields
#[path = "core_actions.rs"]
pub(crate) mod core_actions;

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
    /// A CoreAudio device on macOS: `output` is a UID, a name or `"default"`; `input` is opened only when named
    /// (macOS asks for the microphone permission the first time an input is read). An input on a different
    /// device than the output is an explicit private aggregate the user chose (U7)
    CoreAudio { output: String, input: Option<String> },
    /// A JACK server (feature `jack`): one client with audio and MIDI ports. `device` is `jack` (the default server)
    /// or `jack:<server>`; `inputs` and `outputs` are the audio ports registered; `midi` is the state the session's
    /// MIDI link shares with the backend (I-21.1)
    #[cfg(feature = "jack")]
    Jack { device: String, client: String, inputs: u32, outputs: u32, midi: lm_io::JackMidiShared },
}

/// The audio systems this build can open (`audio.system` is one of them): the screen offers only these
pub fn available_backends() -> Vec<&'static str> {
    let mut v = Vec::new();
    if cfg!(target_os = "linux") {
        v.push("alsa");
    }
    if cfg!(target_os = "macos") {
        v.push("coreaudio");
    }
    if cfg!(feature = "jack") {
        v.push("jack");
    }
    v
}

impl DeviceSetup {
    /// The JACK server `device` names (`jack` or `jack:<server>`), with the default client name and ports
    #[cfg(feature = "jack")]
    pub fn jack(device: &str) -> Self {
        let defaults = lm_io::JackOptions::default();
        DeviceSetup::Jack {
            device: device.to_owned(),
            client: defaults.client_name,
            inputs: defaults.inputs,
            outputs: defaults.outputs,
            midi: lm_io::JackMidiShared::new(),
        }
    }
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
    /// The recording's disk, made to fill up on purpose (the tests of H3)
    pub record_faults: lm_io::DiskFaults,
    /// What the Dummy device's inputs hear (silence when `None`)
    pub dummy_source: Option<fn() -> Box<dyn lm_io::DummySource>>,
}

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfig").field("device", &self.device).field("block", &self.block).finish()
    }
}

/// Every internal plugin of this build
pub fn internal_plugins() -> Result<Arc<Registry>> {
    Registry::new(&internal_descriptors())
}

/// Every plugin crate of the program (a test fails when a `crates/lm-plugins-*` is missing here)
pub fn internal_descriptors() -> Vec<&'static lm_plugin::Descriptor> {
    let mut all: Vec<&'static lm_plugin::Descriptor> = vec![
        &lm_plugins_testsine::DESCRIPTOR,
        &lm_plugins_tripleoscillator::DESCRIPTOR,
        &lm_plugins_ampandir::DESCRIPTOR,
        &lm_plugins_sid::DESCRIPTOR,
        &lm_plugins_freeboy::DESCRIPTOR,
        &lm_plugins_opulenz::DESCRIPTOR,
        &lm_plugins_stk::DESCRIPTOR,
        &lm_plugins_sampler::DESCRIPTOR,
    ];
    all.extend(lm_plugins_guitarix::ALL.iter().map(|(_, d)| *d));
    all.extend(lm_plugins_effects::ALL.iter().copied());
    all.extend(lm_plugins_instruments::ALL.iter().copied());
    all.extend(lm_plugins_noteeffects::ALL.iter().copied());
    all
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
            record_faults: lm_io::DiskFaults::default(),
            dummy_source: None,
        })
    }
}

/// A steady noise for the Dummy's inputs, like the fan the ThinkPad's microphone hears: about -20 dBFS,
/// deterministic
#[derive(Debug, Default)]
pub struct FanNoise(u64);

impl lm_io::DummySource for FanNoise {
    fn fill(&mut self, _: u64, _: HostTime, inputs: &mut [Vec<f32>], frames: usize) {
        for c in inputs.iter_mut() {
            for x in &mut c[..frames] {
                self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                *x = ((z >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * 0.17;
            }
        }
    }
}

/// The clock thread (H2): wakes the main thread every 10 ms, whether anything is drawn or not
struct Ticker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Ticker {
    /// Without the clock the pump never runs and the interface would freeze, so a failed spawn fails the session (M4)
    fn start(waker: Arc<dyn Fn() + Send + Sync>) -> Result<Ticker> {
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
            .map_err(|e| Error::new("session.thread.failed").arg("name", "lm-clock").arg("cause", e.to_string()))?;
        Ok(Ticker { stop, thread: Some(thread) })
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

    fn bytes(&self, resource: &Resource) -> Option<Arc<[u8]>> {
        match self.view.content(resource.kind(), resource.key()) {
            Some(lm_io::Content::Bytes(b)) => Some(Arc::clone(b)),
            _ => None,
        }
    }
}

/// A plugin or rack build waiting for its resources, or running
pub(crate) struct BuildData {
    ticket: Ticket,
    node: NodeKey,
    seed: u64,
    /// The live display of a rack node (its reader is kept by the session, I-9.5)
    display: Option<(lm_render::DisplayWriter, lm_render::DisplayReader)>,
}

impl std::fmt::Debug for BuildData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildData").field("ticket", &self.ticket).field("node", &self.node).finish()
    }
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
    /// The watchdog of the callback (U21)
    watchdog: crate::watchdog::Watchdog,
    /// The native-crash note (U21), armed while a project is open
    crash: crate::crashnote::CrashNote,
    /// The arrangement's clipboard (F-ARR-006): the items copied, each with its track and start (`arrange.rs`)
    pub(crate) arrange_clipboard: Vec<crate::arrange::Copied>,
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
    /// Loads of the other kinds (U24, C9): bytes or a checked file
    kind_ops: Ops<lm_io::KindLoaded, Resource>,
    import_ops: Ops<lm_io::ImportedFile, String>,
    /// Takes being closed, by the recording they belong to (0: a recovered take, alone)
    take_ops: Ops<FinalizedTake, u64>,
    generic_ops: Ops<Value, &'static str>,
    pub(crate) activity: Activity,
    pub(crate) notices: Notices,
    /// The browser entries given out, for the actions that name one
    pub(crate) browser: crate::browser::BrowserEntries,
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
    dummy_source: Option<fn() -> Box<dyn lm_io::DummySource>>,
    /// The device a calibration took to its worker comes back through here (`core_actions`)
    calibrating: Option<Arc<Mutex<Option<Device>>>>,
    /// When the slow parts of the status (the unused files, the templates) were last read (host ns)
    slow_status_at: u64,
    untitled: PathBuf,
    /// A "save as" copying the audio: where the project goes, with the lock already taken
    pending_save_as: Option<(lm_base::OperationId, ProjectFolder, ProjectLock)>,
    block: u32,
    /// Builds waiting for the resources their state cites (C11)
    builds_waiting: Vec<BuildRequest>,
    /// New nodes silent until built; nodes whose old processor plays while the new one builds (I-2.3, T7)
    waiting_nodes: BTreeSet<NodeKey>,
    replacing_nodes: BTreeSet<NodeKey>,
    /// Nodes whose last build failed (a plugin refusing a missing resource): nothing will come for them until the
    /// tree asks for another build, so they do not keep the session from being settled
    failed_builds: BTreeSet<NodeKey>,
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
    /// Where the cursor was when playing began: where Stop goes back to (F-REG-004)
    play_origin: Option<i64>,
    recording_tracks: Vec<NodeId>,
    /// The waveforms of the takes being written (U3)
    live: crate::recording::LiveTakes,
    /// The recordings whose takes are being closed: all of one recording enter the tree in one transaction
    recordings: std::collections::BTreeMap<u64, PendingRecording>,
    recording_serial: u64,
    /// Takes waiting to be closed (started at the next pump)
    take_queue: std::collections::VecDeque<(TakeJob, u64)>,
    next_take_after: bool,
    /// Records forgotten once the journal line that holds their take is on the disk (U5)
    forget_after: Vec<(Arc<AtomicBool>, PathBuf)>,
    xruns: u32,
    /// The imports that ended, by their operation, for the port to answer (`take_imported`)
    imported: Vec<(lm_base::OperationId, Result<Resource>)>,
    /// The live displays of the rack nodes, by node (read through `&self`: the reader's read needs `&mut`)
    displays: std::cell::RefCell<std::collections::BTreeMap<NodeKey, lm_render::DisplayReader>>,
    /// The project opened in safe mode (U21)
    safe_mode_on: bool,
    /// MIDI mapping, feedback, control surfaces, MIDI and automation recording (I-21)
    pub(crate) midi: midi::MidiCore,
    /// The OSC server
    pub(crate) osc: osc::OscCore,
    pub(crate) external: external::ExternalCore,
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
    lm_port::tempo_map(root)
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
            .field(lm_state::FieldSpec::new("state", lm_state::FieldKind::Any).default(Value::empty_object()))
            // What the MIDI targets follow (I-21.2): `{track, channel, plugin}`, ids as text
            .field(lm_state::FieldSpec::new("selection", lm_state::FieldKind::Any).default(Value::empty_object()))
            // Step recording's cursor `{item, at, step}` and the item a recording goes into (I-21.4); null when off
            .field(lm_state::FieldSpec::new("stepRecord", lm_state::FieldKind::Any).default(Value::NULL))
            .field(lm_state::FieldSpec::new("recordInto", lm_state::FieldKind::Any).default(Value::NULL)),
    )?;
    registry.check_complete()?;
    Ok(Arc::new(registry))
}

fn open_device(
    setup: &DeviceSetup,
    rate: u32,
    block: u32,
    source: Option<fn() -> Box<dyn lm_io::DummySource>>,
) -> Result<(Device, lm_io::DeviceInfo)> {
    let backend: Box<dyn lm_io::Backend> = match setup {
        DeviceSetup::Dummy(o) => match source {
            Some(make) => Box::new(DummyBackend::new(*o).with_source(make())),
            None => Box::new(DummyBackend::new(*o)),
        },
        #[cfg(target_os = "linux")]
        DeviceSetup::Alsa { .. } => Box::new(lm_io::AlsaBackend::new(None)),
        #[cfg(not(target_os = "linux"))]
        DeviceSetup::Alsa { .. } => return Err(err("session.device.alsa_unavailable")),
        #[cfg(target_os = "macos")]
        DeviceSetup::CoreAudio { .. } => Box::new(lm_io::CoreAudioBackend::new()),
        #[cfg(not(target_os = "macos"))]
        DeviceSetup::CoreAudio { .. } => return Err(err("session.device.coreaudio_unavailable")),
        #[cfg(feature = "jack")]
        DeviceSetup::Jack { client, inputs, outputs, midi, .. } => {
            Box::new(lm_io::JackBackend::new(lm_io::JackOptions {
                client_name: client.clone(),
                inputs: *inputs,
                outputs: *outputs,
                auto_connect: true,
                midi: Some(midi.clone()),
            }))
        }
    };
    let name = match setup {
        DeviceSetup::Dummy(_) => "dummy".to_owned(),
        DeviceSetup::Alsa { device } => device.clone(),
        DeviceSetup::CoreAudio { output, .. } => output.clone(),
        #[cfg(feature = "jack")]
        DeviceSetup::Jack { device, .. } => device.clone(),
    };
    let config = match setup {
        // The input only when one is named: opening it triggers the microphone permission
        DeviceSetup::CoreAudio { output, input } => {
            let mut c = DeviceConfig::duplex(output, rate, block);
            c.capture = input.clone().filter(|i| !i.is_empty());
            // Two named devices that differ are the user's explicit combination (U7)
            c.drift_correction = c.capture.as_ref().is_some_and(|i| i != output);
            c
        }
        _ => DeviceConfig::duplex(&name, rate, block),
    };
    let mut device = Device::new(backend, crate::crashnote::render_threads());
    let info = device.open(&config)?;
    Ok((device, info))
}

impl Session {
    /// A session with a new, untitled project. Opens the device (the Dummy when the chosen one fails, with a notice)
    pub fn new(config: SessionConfig) -> Result<Session> {
        let main = Main::acquire().ok_or_else(|| err("session.main.taken"))?;
        let plugins = config.registry.clone();
        let schemas = Arc::new(plugins.project_schemas()?);
        let mut notices = Notices::default();
        // a session with real preferences (the program, the headless server) keeps its errors in the data folder
        if config.preferences.file().is_some() {
            let logs = crate::prefs::data_dir().join("logs");
            notices.set_log(logs.join("errors.log"));
            if let Some(report) = crate::crashreport::take_previous(&logs) {
                notices.warn("session.crash_report", vec![("path", Value::text(report.display().to_string()))]);
            }
        }
        let options = project_options(&plugins, &schemas, config.limits);
        let document = Document::new_desk_project(schemas.clone(), options)?;
        let ui_document = interface_document(None)?;
        let folder = ProjectFolder::new(config.untitled.join(format!("untitled-{}", lm_io::new_uuid())));
        folder.create(&config.faults)?;
        let rate = project_rate(document.root());
        let (device, info) = match open_device(&config.device, rate, config.block, config.dummy_source) {
            Ok(d) => d,
            Err(e) => {
                notices.error(&e, Some(Target::Device), None);
                open_device(&DeviceSetup::Dummy(DummyOptions::default()), rate, config.block, config.dummy_source)?
            }
        };
        let spec = info.spec();
        let (engine, renderer) = lm_render::engine_with(spec, config.limits, crate::crashnote::render_threads());
        let mut device = device;
        device.start(renderer).map_err(lm_io::StartError::into_error)?;
        let compiler = Compiler::new(CompilerConfig {
            spec,
            limits: config.limits,
            schemas: schemas.clone(),
            plugins: plugins.clone(),
        });
        let journal = Journal::start(&folder, config.faults.clone(), String::new(), 0, false)?;
        let mut resources = ResourceStore::new(folder.root().to_path_buf(), config.clock.clone());
        resources.set_allowed(allowed_folders(&config.preferences));
        let disk = match DiskWriter::start_with(config.record_faults.clone()) {
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
        let watchdog = crate::watchdog::Watchdog::start(Arc::new(crate::watchdog::Shared {
            table: Mutex::new(Arc::new(InstalledTable::default())),
            folder: Mutex::new(folder.root().to_path_buf()),
            running: AtomicBool::new(true),
            watch: Mutex::new(engine.watch()),
        }));
        // The watchdog is a safety net, not a need: without it the session runs degraded, and says so (M4)
        if !watchdog.is_alive() {
            notices.error(
                &Error::new("session.thread.failed").arg("name", "lm-watchdog").arg("cause", "spawn failed"),
                None,
                None,
            );
        }
        let mut s = Session {
            watchdog,
            _ticker: config.waker.clone().map(Ticker::start).transpose()?,
            crash: crate::crashnote::CrashNote::default(),
            arrange_clipboard: Vec::new(),
            device: Some(device),
            disk,
            journal,
            untitled_guard: UntitledGuard(Some(folder.root().to_path_buf())),
            compiler,
            engine,
            open_ops: Ops::default(),
            build_ops: Ops::default(),
            load_ops: Ops::default(),
            kind_ops: Ops::default(),
            import_ops: Ops::default(),
            take_ops: Ops::default(),
            generic_ops: Ops::default(),
            activity: Activity::default(),
            notices,
            browser: Default::default(),
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
            dummy_source: config.dummy_source,
            calibrating: None,
            slow_status_at: 0,
            untitled: config.untitled.clone(),
            pending_save_as: None,
            block: config.block,
            builds_waiting: Vec::new(),
            waiting_nodes: BTreeSet::new(),
            replacing_nodes: BTreeSet::new(),
            failed_builds: BTreeSet::new(),
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
            play_origin: None,
            recording_tracks: Vec::new(),
            live: crate::recording::LiveTakes::default(),
            recordings: std::collections::BTreeMap::new(),
            recording_serial: 0,
            forget_after: Vec::new(),
            take_queue: std::collections::VecDeque::new(),
            next_take_after: false,
            xruns: 0,
            clipboard: None,
            safe_mode_on: false,
            displays: std::cell::RefCell::new(std::collections::BTreeMap::new()),
            midi: midi::MidiCore::new(),
            osc: osc::OscCore::default(),
            external: external::ExternalCore::default(),
            imported: Vec::new(),
            finished: Vec::new(),
            main,
        };
        s.external_start();
        s.refresh_status();
        s.submit();
        // A real device: the machine's MIDI devices open too, in an operation (I-21.1); the Dummy (tests) has none
        #[cfg(target_os = "linux")]
        if matches!(s.device_setup, DeviceSetup::Alsa { .. }) {
            s.midi_open_devices();
        }
        #[cfg(feature = "jack")]
        if matches!(s.device_setup, DeviceSetup::Jack { .. }) {
            s.midi_open_jack();
        }
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

    /// Sets one key of the status tree (what the screens read; nothing of the project)
    pub(crate) fn set_status_key(&mut self, key: &str, value: Value) {
        self.status = self.status.with(key, value);
    }

    /// The keys of the notices kept (tests, the headless tool)
    pub fn notices_keys(&self) -> Vec<&'static str> {
        self.notices.keys()
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

    /// The device open now, as it was negotiated (latencies, period, rate)
    pub fn device_info(&self) -> Option<&lm_io::DeviceInfo> {
        self.device.as_ref().and_then(Device::info)
    }

    pub fn engine(&mut self) -> &mut EngineControl {
        &mut self.engine
    }

    /// What keeps the session from being settled (diagnostics for the tests and the headless tool)
    pub fn busy(&self) -> Vec<String> {
        let mut out = Vec::new();
        let counts = [
            ("builds", self.build_ops.len()),
            ("loads", self.load_ops.len() + self.kind_ops.len()),
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

    /// What the desk shows of a channel or the master (I-9.5): its rack segments' display frames merged (each
    /// segment fills what it measures; `rack::display` gives the layout); `None` for a node with no rack display
    pub fn rack_display(&self, id: NodeId) -> Option<Vec<f32>> {
        self.drop_stale_displays();
        let mut displays = self.displays.borrow_mut();
        let mut out: Option<Vec<f32>> = None;
        for (_, reader) in displays.iter_mut().filter(|(k, _)| matches!(k, NodeKey::Rack(c, _) if *c == id)) {
            let bytes = reader.read();
            let merged = out.get_or_insert_with(|| vec![f32::NAN; crate::rack::display::FLOATS]);
            for (i, chunk) in bytes.as_chunks::<4>().0.iter().enumerate().take(merged.len()) {
                let v = f32::from_le_bytes(*chunk);
                if !v.is_nan() {
                    merged[i] = v;
                }
            }
        }
        out
    }

    /// The live frame of a plugin node, instrument or insert (I-9.5): the newest frame its processor wrote through
    /// its display writer (a lock-free triple buffer: nothing is allocated or waited for on the audio thread), as
    /// `f32` little-endian words (the layout is the plugin's, a trailing partial word is dropped), and the plugin's
    /// id. `None` for a node whose plugin declares no display (or that is not built yet)
    pub fn plugin_display(&self, id: NodeId) -> Option<(String, Vec<f32>)> {
        let plugin = self.document.find(id)?.get("plugin").and_then(Value::as_text)?.to_owned();
        self.plugins.display(&plugin)?;
        self.drop_stale_displays();
        let mut displays = self.displays.borrow_mut();
        let key = [NodeKey::Instrument(id), NodeKey::Insert(id)].into_iter().find(|k| displays.contains_key(k))?;
        let reader = displays.get_mut(&key)?;
        let frame = reader.read().as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        Some((plugin, frame))
    }

    /// The readers of nodes the installed program no longer has (and no build is making) are dropped
    fn drop_stale_displays(&self) {
        let table = self.table.clone();
        let building = |k: &NodeKey| {
            self.build_ops.list.iter().any(|o| o.data.node == *k)
                || self.builds_waiting.iter().any(|b| b.node == *k)
                || self.waiting_nodes.contains(k)
        };
        self.displays.borrow_mut().retain(|k, _| table.node(*k).is_some() || building(k));
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
            && self.document.find(id).and_then(|n| n.get("bypass").and_then(Value::as_bool)) != Some(true)
            && self.table.node(NodeKey::Instrument(id)).is_none()
            && self.table.node(NodeKey::Insert(id)).is_none()
            && self.table.node(NodeKey::NoteEffect(id)).is_none()
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
            && self.kind_ops.is_empty()
            && self.builds_waiting.is_empty()
            && self.waiting_nodes.is_empty()
            && self.replacing_nodes.is_empty()
            && self.compiling_since.is_none()
            && !self.compiler.pending()
            && !self.needs_compile
            && self.engine.installs_in_flight() == 0
            && self.engine.installs_queued() == 0
            && self.open_ops.is_empty()
            && self.saving.is_none()
            && !self.external.save_waiting
            && !self.document.index().ids().any(|(id, _)| self.is_waiting(*id))
    }

    // ---- operations (I-2) ----

    /// Every operation running (the global indicator)
    pub fn activity(&self) -> Vec<OperationInfo> {
        let now = self.clock.now();
        let mut out = Vec::new();
        let lists: [&dyn OpList; 7] = [
            &self.open_ops,
            &self.build_ops,
            &self.load_ops,
            &self.kind_ops,
            &self.import_ops,
            &self.take_ops,
            &self.generic_ops,
        ];
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
        let lists: [&dyn OpList; 7] = [
            &self.open_ops,
            &self.build_ops,
            &self.load_ops,
            &self.kind_ops,
            &self.import_ops,
            &self.take_ops,
            &self.generic_ops,
        ];
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
                    match self.resources.state_kind(r.kind(), r.key()) {
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
        // 2b. MIDI (I-21): mapped fields and actions, gesture ends, learning, the map, feedback, the recordings
        changed |= self.midi_pump();
        changed |= self.osc_pump();
        changed |= self.external_pump();
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
            self.midi_diff(&diff);
            self.osc_diff(&diff);
            self.slicert_auto(&diff);
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
                // A slot a MIDI control holds has a newer value than the tree (I-21.2)
                if !self.midi_holds(*slot) {
                    self.engine.param_write(*slot, *value as f32);
                }
            }
        }
    }

    /// Writes a parameter's slots at once (a knob being dragged, without an edit)
    pub fn param_write(&mut self, node: NodeId, key: &str, value: f32) {
        for slot in self.table.slots_of(node, key) {
            self.engine.param_write(*slot, value);
        }
    }

    /// A live note from the interface to the instrument of `track` (I-15 `note_on`/`note_off`/`all_notes_off`): a MIDI
    /// command on the track's own port, channel 0. Never waits: a full ring drops the note. `on`: `Some(velocity)` for
    /// a note-on, `None` for a note-off; `key` 255 with `None`: all notes off
    pub fn live_note(&mut self, track: NodeId, key: u8, on: Option<u8>) {
        let want = track.to_string();
        let Some(ordinal) = self
            .document
            .root()
            .get("tracks")
            .and_then(|l| l.items().position(|t| t.get("id").and_then(Value::as_text) == Some(want.as_str())))
        else {
            return;
        };
        let bytes = match (key, on) {
            (255, None) => [0xB0, 123, 0],
            (k, Some(v)) => [0x90, k.min(127), v.clamp(1, 127)],
            (k, None) => [0x80, k.min(127), 0],
        };
        let cmd = Command::Midi { port: lm_render::gui_port(ordinal), at: HostTime::now(), bytes };
        let _ = self.engine.commands().send(cmd);
    }

    fn environment(&self) -> Environment {
        Environment {
            playhead: self.telemetry.position.samples,
            low_latency_monitoring: self.preferences.bool(&["lowLatencyMonitoring"]).unwrap_or(false),
            monitor: self.preferences.get(&["monitor"]).cloned(),
            // The tracks' MIDI inputs and outputs (I-21.1, P-38)
            midi_ports: self.midi_instrument_ports(),
            midi_aliases: self.midi_aliases(),
            mpe_ports: self.midi_mpe_ports(),
            selected_track: self.midi.selection.track,
            // The metronome (I-5.5): its own sounds, when it sounds and where (F-REC-024, F-REC-025)
            metronome_sounds: crate::recording::metronome_sounds(self.spec.sample_rate),
            metronome_when: lm_compile::MetronomeWhen::parse(
                self.preferences.text(&["metronome", "when"]).unwrap_or(""),
            ),
            metronome_output: self.metronome_output(),
            ..Environment::default()
        }
    }

    /// The device channels of the metronome (the preference `metronome.output`, 1-based); none: the master's output
    fn metronome_output(&self) -> Option<Vec<u16>> {
        let list = self.preferences.get(&["metronome", "output"])?;
        let channels: Vec<u16> =
            list.items().filter_map(Value::as_int).filter_map(|c| u16::try_from(c - 1).ok()).collect();
        (!channels.is_empty()).then_some(channels)
    }

    fn snapshot(&self) -> ResourceSnapshot {
        let mut missing = BTreeSet::new();
        let resources = &self.resources;
        self.document.root().walk(&mut |_, v| {
            if let Some(r) = v.as_resource()
                && r.kind() == "audio"
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
        if self.compiler.activity().is_none() && !self.compiler.pending() {
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
        // The map's parameter slots are this install's
        self.midi.mark_dirty();
        if let Ok(mut t) = self.watchdog.shared.table.lock() {
            *t = self.table.clone();
        }
        self.crash.update(&self.table, &self.engine.watch());
        for b in &builds {
            self.failed_builds.remove(&b.node);
        }
        self.waiting_nodes = waiting.into_iter().filter(|k| !self.failed_builds.contains(k)).collect();
        self.replacing_nodes = replacing.into_iter().filter(|k| !self.failed_builds.contains(k)).collect();
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
        self.request_one(r);
        self.post_refused(r);
    }

    /// C9: a link out of the project that no allowed folder covers left the resource `Missing`; the notice offers
    /// "allow this folder"
    fn post_refused(&mut self, r: &Resource) {
        for e in self.resources.take_refused() {
            let folder = e
                .label
                .args
                .iter()
                .find_map(|(k, a)| match (k, a) {
                    (&"folder", lm_base::Arg::Text(t)) => Some(t.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let action =
                (lm_port::ActionId::from("folder.allow"), Value::object([("folder", Value::text(folder.as_str()))]));
            self.notices.push(
                "session.resource.not_allowed",
                vec![("folder", Value::text(folder))],
                Severity::Warning,
                Some(Target::Resource(r.clone())),
                vec![action],
            );
        }
    }

    fn request_one(&mut self, r: &Resource) {
        if r.kind() != "audio" {
            return self.request_kind(r);
        }
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

    /// A resource of another kind (a NAM model as bytes, a SoundFont as a file): the same, through the store's kinds
    fn request_kind(&mut self, r: &Resource) {
        if self.kind_ops.list.iter().any(|op| op.data == *r) {
            return;
        }
        match self.resources.begin_load_kind(r.kind(), r.key()) {
            Ok(job) => {
                let delay = self.delay;
                let target = Target::Resource(r.clone());
                self.activity.clear(&target);
                let handle = self.workers.start(move |cancel, progress| {
                    sleep_cancellable(delay, cancel)?;
                    job.run(cancel, progress)
                });
                let started = self.clock.now();
                self.kind_ops.push(Op {
                    handle,
                    label: Label::new("session.loading_resource"),
                    target,
                    cancellable: true,
                    started,
                    data: r.clone(),
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
                match self.resources.state_kind(r.kind(), r.key()) {
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
            if r.kind() == "audio" {
                if let Some(d) = view.get(r.key()) {
                    resources.insert(r.clone(), lm_plugin::Loaded::Audio(d.clone()));
                }
            } else if let Some(c) = view.content(r.kind(), r.key()) {
                resources.insert(r.clone(), plugin_content(c));
            }
        }
        let plugins = self.plugins.clone();
        let schemas = self.schemas.clone();
        let delay = self.delay;
        let BuildRequest { ticket, node, build, seed, spec, .. } = b;
        // The live frame of the node (I-9.5): a rack segment's fixed one, or what the plugin declares
        let display = match &build {
            lm_compile::Build::Rack { .. } => Some(lm_render::display_channel(crate::rack::display::BYTES)),
            lm_compile::Build::Plugin { plugin, .. } => {
                self.plugins.display(plugin).map(|d| lm_render::display_channel(d.bytes))
            }
        };
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
            data: BuildData { ticket, node, seed, display },
        });
    }

    fn poll_operations(&mut self) -> bool {
        let now = self.clock.now();
        let mut done = Vec::new();
        let mut changed = false;
        for (op, result) in self.build_ops.finished(now, &mut done) {
            changed = true;
            match result {
                Ok(p) => {
                    let BuildData { ticket, node, seed, display } = op.data;
                    let writer = display.map(|(w, r)| {
                        self.displays.borrow_mut().insert(node, r);
                        w
                    });
                    let mut p = p;
                    self.external_built(node, &mut p);
                    self.compiler.ready(ticket, NodeCore::new(p, seed, writer));
                }
                Err(e) if e.is_cancelled() => {}
                Err(e) => {
                    self.notices.error(&e, Some(op.target.clone()), None);
                    self.activity.fail(op.target, e);
                    // Unless a newer build of the node is on its way
                    let node = op.data.node;
                    let newer = self.build_ops.list.iter().any(|o| o.data.node == node)
                        || self.builds_waiting.iter().any(|w| w.node == node);
                    if !newer {
                        self.failed_builds.insert(node);
                        self.waiting_nodes.remove(&node);
                        self.replacing_nodes.remove(&node);
                    }
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
        for (op, result) in self.kind_ops.finished(now, &mut done) {
            changed = true;
            let r = &op.data;
            match result {
                Ok(loaded) => {
                    self.resources.finish_kind(r.kind(), r.key(), Ok(loaded));
                    self.needs_compile = true;
                }
                Err(e) if e.is_cancelled() => self.resources.finish_kind(r.kind(), r.key(), Err(e)),
                Err(e) => {
                    self.activity.fail(op.target.clone(), e.clone());
                    self.resources.finish_kind(r.kind(), r.key(), Err(e));
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
                    Ok(file) => Resource::new("audio", &file.key).map(|r| r.with_name(&op.data)),
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
            ("plugins.scan", Ok(report)) => {
                if let Err(e) = self.scan_finished(&report) {
                    self.notices.error(&e, None, None);
                }
            }
            ("plugins.state" | "plugins.state.save", r) => self.states_finished(kind, r),
            ("plugins.window", r) => self.window_finished(r),
            (
                "locate" | "export_zip" | "save_template" | "clean_unused" | "new_from_template" | "device.calibrate"
                | "devices.scan",
                r,
            ) => self.core_op_finished(kind, target, r),
            ("midi_import", Ok(result)) => self.midi_import_finished(result),
            ("slicert.analyze", Ok(result)) => {
                if let Err(e) = self.slicert_finished(&result) {
                    self.notices.error(&e, Some(target), None);
                }
            }
            ("sampler.list_presets", Ok(result)) => self.sampler_presets_finished(&result),
            ("plugin.load_into_field", Ok(result)) => {
                if let Err(e) = self.field_load_finished(&result) {
                    self.notices.error(&e, Some(target), None);
                }
            }
            ("clip.reverse", Ok(made)) => {
                if let Err(e) = self.reverse_finished(&made) {
                    self.notices.error(&e, Some(target), None);
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
        self.compiler.observe_frames(frame.frames);
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
        self.telemetry.load = frame.load;
        self.telemetry.counting_in = frame.counting_in;
        // The input meters (F-REC-009): one peak per device input channel, falling smoothly
        let inputs = self.device.as_ref().and_then(|d| d.info()).map_or(0, |i| i.inputs).min(frame.inputs.len());
        self.telemetry.inputs.resize(inputs, 0.0);
        for (level, peak) in self.telemetry.inputs.iter_mut().zip(frame.inputs.iter()) {
            *level = peak.max(*level * 0.85);
        }
        let mut events = Vec::new();
        self.engine.events(|t| events.push(t));
        for t in events {
            self.telemetry_event(t, &table);
        }
        let mut transport = Vec::new();
        self.engine.transport_events(|t| transport.push(t));
        for t in transport {
            self.midi_transport(&t);
            if let TransportEvent::RecordStopped { .. } = t {
                self.recording_tracks.clear();
            }
            if let TransportEvent::RecordFailed { sample, .. } = t {
                self.notices.push(
                    "session.record.failed",
                    vec![("sample", Value::int(sample))],
                    Severity::Error,
                    None,
                    Vec::new(),
                );
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
        self.report_render_panics();
        changed
    }

    /// Panics that unwound out of the render call without a catch site of the engine (R-4): the hook logged their
    /// messages in real-time-safe memory, and here they reach the user and the log
    fn report_render_panics(&mut self) {
        let mut messages: Vec<String> = Vec::new();
        let lost = lm_render::take_render_panics(|m| messages.push(m.to_owned()));
        if lost > 0 {
            messages.push(format!("({lost} more audio panics not kept)"));
        }
        for m in messages {
            self.notices.push(
                "session.audio.panicked",
                vec![("message", Value::text(m))],
                Severity::Error,
                None,
                Vec::new(),
            );
        }
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
            Telemetry::NodeStuck { node, .. } => {
                let id = table.key_of(node).and_then(NodeKey::owner);
                self.notices.push(
                    "session.node.stuck",
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
            Telemetry::PluginRequest { node, request, .. } => {
                if let Some(key) = table.key_of(node) {
                    self.external_request(key, request);
                }
            }
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
        self.report_render_panics();
        // Recording waits for the device to arm (lm-io: the first blocks would be lost otherwise)
        if let Some(rs) = &self.record_start
            && let Some(device) = self.device.as_ref()
            && device.is_armed()
        {
            let _ = rs.since;
            self.record_start = None;
            self.play_origin = Some(self.telemetry.position.samples);
            let _ = self.engine.commands().send(Command::Record { on: true });
            let _ = self.engine.commands().send(Command::Play);
        }
    }

    /// U11: the card went away; the renderer goes on on the Dummy, the session stays alive
    fn fall_back_to_dummy(&mut self) {
        let Some(mut device) = self.device.take() else { return };
        match device.stop() {
            Ok(renderer) => {
                match open_device(
                    &DeviceSetup::Dummy(DummyOptions::default()),
                    self.spec.sample_rate,
                    self.block,
                    self.dummy_source,
                ) {
                    Ok((mut dummy, _)) => match dummy.start(renderer) {
                        Ok(()) => self.device = Some(dummy),
                        Err(failed) => self.notices.error(failed.error(), Some(Target::Device), None),
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
        // `backup.autosaveMinutes` (0: off; an incomplete journal is still written)
        let minutes = self.preferences.int(&["backup", "autosaveMinutes"]).unwrap_or(1).clamp(0, 30) as u64;
        let due_by_time = minutes > 0 && self.modified() && now.since(self.last_autosave) >= minutes * 60_000_000_000;
        if due_by_time || self.journal.is_incomplete() {
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
        // Third-party plugins' own states first (an operation); the save goes on when they arrive
        if self.external_before_save() {
            return Ok(());
        }
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
                    crate::folder::copy_via_tmp(from, &tmp, dest, "session.save_as.copy")?;
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
        self.journal = Journal::start(&to, self.faults.clone(), String::new(), self.journal.seq(), false)?;
        self.resources = ResourceStore::new(to.root().to_path_buf(), self.clock.clone());
        self.resources.set_allowed(allowed_folders(&self.preferences));
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

    /// U21: the project was left with a plugin hanging the callback (the watchdog's file): it opens with that
    /// plugin bypassed, in one undoable step, with a notice and `status().safeMode`
    fn safe_mode(&mut self) {
        let folder = self.project.folder.root().to_path_buf();
        let Some(suspect) = crate::watchdog::read_suspect(&folder) else { return };
        let plugin =
            self.document.find(suspect.node).and_then(|n| n.get("plugin").and_then(Value::as_text).map(str::to_owned));
        if let Some(plugin) = plugin
            && !self.project.read_only
        {
            let node = suspect.node;
            let r = self.document.transact(Label::new("session.safe_mode"), |e| {
                e.set(node, &[PathStep::key("bypass")], Value::bool(true))
            });
            if let Err(e) = r {
                self.notices.error(&e, None, None);
            }
            let entry = Value::object([
                ("node", Value::text(node.to_string())),
                ("plugin", Value::text(plugin.as_str())),
                ("reason", Value::text(suspect.reason.as_str())),
            ]);
            self.status = self.status.with("safeMode", Value::bool(true)).with("safeModePlugins", Value::list([entry]));
            self.safe_mode_on = true;
            self.notices.push(
                "session.safe_mode",
                vec![("node", Value::text(node.to_string())), ("plugin", Value::text(plugin))],
                Severity::Warning,
                Some(Target::Node(node)),
                Vec::new(),
            );
        }
        let _ = std::fs::remove_file(crate::watchdog::suspect_path(&folder));
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
        self.journal = Journal::start(&folder, self.faults.clone(), String::new(), 0, false)?;
        self.resources = ResourceStore::new(folder.root().to_path_buf(), self.clock.clone());
        self.resources.set_allowed(allowed_folders(&self.preferences));
        let options = project_options(&self.plugins, &self.schemas, self.limits);
        let document = Document::new_desk_project(self.schemas.clone(), options)?;
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
        self.create_with(folder, None, None)
    }

    fn replace_document(&mut self, mut document: Document, ui: Option<Value>) -> Result<()> {
        self.safe_mode_on = false;
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
        self.watchdog.shared.running.store(false, Ordering::Release);
        if let Some(mut d) = self.device.take() {
            let _ = d.stop();
            d.close();
        }
        let (mut device, info) =
            open_device(&self.device_setup, rate, self.block, self.dummy_source).or_else(|_| {
                open_device(&DeviceSetup::Dummy(DummyOptions::default()), rate, self.block, self.dummy_source)
            })?;
        let spec = info.spec();
        let (engine, renderer) = lm_render::engine_with(spec, self.limits, crate::crashnote::render_threads());
        device.start(renderer).map_err(lm_io::StartError::into_error)?;
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
        if let Ok(mut w) = self.watchdog.shared.watch.lock() {
            *w = self.engine.watch();
        }
        self.crash.update(&self.table, &self.engine.watch());
        self.watchdog.shared.running.store(true, Ordering::Release);
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
            Journal::disabled(&folder)?
        } else {
            Journal::start(&folder, self.faults.clone(), bytes_hash, seq, !replayed)?
        };
        self.resources = ResourceStore::new(folder.root().to_path_buf(), self.clock.clone());
        self.resources.set_allowed(allowed_folders(&self.preferences));
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
        self.safe_mode();
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
            // Audio, and the tuning files the compiler reads (the plugins' resources are asked by their builds)
            if r.kind() == "audio" || r.kind() == lm_doc::schemas::TUNING_KIND {
                self.request(&r);
            }
        }
    }

    // ---- the status tree (lm-port `trees::status`) ----

    fn refresh_status(&mut self) {
        if let Ok(mut f) = self.watchdog.shared.folder.lock()
            && *f != self.project.folder.root()
        {
            *f = self.project.folder.root().to_path_buf();
        }
        // A read-only project is not ours to write into
        if self.project.read_only {
            self.crash.disarm();
        } else if self.crash.arm(self.project.folder.root()) {
            self.crash.update(&self.table, &self.engine.watch());
        }
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
        let (unused, templates) = self.slow_status();
        let project = Value::object([
            ("name", Value::text(self.project.folder.name())),
            ("folder", Value::text(self.project.folder.root().display().to_string())),
            ("readOnly", Value::bool(self.project.read_only)),
            ("lockedBy", self.project.locked_by.as_ref().map_or(Value::NULL, |o| o.status())),
            ("unused", unused),
        ]);
        let device = self.device_status();
        let base = if self.status.is_object() { self.status.clone() } else { Value::empty_object() };
        self.status = base
            .with("project", project)
            .with("recent", self.preferences.get(&["recent"]).cloned().unwrap_or_else(Value::empty_list))
            .with("templates", templates)
            .with("device", device)
            .with("backends", Value::list(available_backends().into_iter().map(Value::text)))
            .with("interruptedTakes", Value::list(interrupted))
            .with("safeMode", Value::bool(self.safe_mode_on));
        if !self.safe_mode_on {
            self.status = self.status.with("safeModePlugins", Value::empty_list());
        }
        if self.status.get("lastExport").is_none() {
            self.status = self.status.with("lastExport", Value::NULL);
        }
        if self.status.get("autosave").is_none() {
            self.status = self.status.with("autosave", Value::NULL);
        }
    }

    // ---- preferences ----

    pub fn set_preference(&mut self, path: &[PathStep], value: Value) -> Result<()> {
        self.preferences.set(path, value.clone())?;
        // The monitor's LEVEL moves live through its slot; the other monitor switches recompile
        if path == [PathStep::key("monitor"), PathStep::key("level")]
            && let Some(v) = value.as_real()
        {
            self.param_write(lm_doc::MASTER, "monitor.level", v as f32);
        }
        if path.first() == Some(&PathStep::key("plugins")) {
            self.refresh_plugin_folders();
        }
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
        if !self.telemetry.position.playing {
            self.play_origin = Some(self.telemetry.position.samples);
        }
        let _ = self.engine.commands().send(Command::Play);
    }

    /// Stops; the cursor then goes back to where playing began ("origin", the default), to the start of the song
    /// ("start") or stays ("stay"), by the preference `interface.stopBehavior` (F-REG-004)
    pub fn stop(&mut self) {
        let was_playing = self.telemetry.position.playing || self.record_start.is_some();
        let _ = self.engine.commands().send(Command::Stop);
        let origin = self.play_origin.take();
        if was_playing {
            let back = match self.preferences.get(&["interface", "stopBehavior"]).and_then(Value::as_text) {
                Some("stay") => None,
                Some("start") => Some(0),
                _ => origin,
            };
            if let Some(sample) = back {
                self.locate(sample);
            }
        }
        self.record_start = None;
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
        // Recording is already running and the music plays: the button punches out, and in again (F-REC-021); the
        // take goes on until Stop
        if self.telemetry.position.playing && self.device.as_ref().is_some_and(|d| d.is_armed()) {
            let on = !self.telemetry.position.recording;
            let _ = self.engine.commands().send(Command::Record { on });
            return Ok(());
        }
        let device_inputs = self.device.as_ref().and_then(|d| d.info()).map(|i| i.inputs);
        let mut tracks: Vec<(NodeId, Vec<u16>)> = Vec::new();
        let mut refused = 0usize;
        for t in self.document.root().get("tracks").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default()
        {
            let Some(id) = t.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else { continue };
            let armed = t.get("armed").and_then(Value::as_bool) == Some(true) || fallback == Some(id);
            if !armed {
                continue;
            }
            let instrument = t.get("instrument").is_some_and(|i| !i.is_null());
            let name = Value::text(t.get("name").and_then(Value::as_text).unwrap_or(""));
            let channels: Vec<u16> = t
                .get("input")
                .and_then(|i| i.get("channels"))
                .map(|l| {
                    l.items().filter_map(Value::as_int).map(|c| u16::try_from((c - 1).max(0)).unwrap_or(0)).collect()
                })
                .unwrap_or_default();
            if instrument {
                // The notes are recorded by the MIDI recorder; the audio file only carries the take (as before)
                tracks.push((id, if channels.is_empty() { vec![0] } else { channels }));
                continue;
            }
            // F-REC-004: an audio track armed without an input, or with one the device does not have, says so
            // and records nothing
            if channels.is_empty() {
                self.notices.warn("session.record.no_input", vec![("track", name)]);
                refused += 1;
                continue;
            }
            if let (Some(have), Some(worst)) = (device_inputs, channels.iter().max())
                && usize::from(*worst) >= have
            {
                let input = Value::int(i64::from(*worst) + 1);
                self.notices.warn("session.record.no_such_input", vec![("track", name), ("input", input)]);
                refused += 1;
                continue;
            }
            tracks.push((id, channels));
        }
        if tracks.is_empty() {
            return Err(err(if refused > 0 { "session.record.no_input" } else { "session.record.no_track" }));
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
        let mut plan = RecordPlan::new(audio, inputs, self.spec.sample_rate);
        plan.latency = self.recording_latency();
        let plan_latency = plan.latency;
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
        let shift = plan_latency;
        self.live.plan(
            tracks.iter().enumerate().map(|(i, (id, channels))| (u32::try_from(i).unwrap_or(0), *id, channels.len())),
            shift,
        );
        self.recording_tracks = tracks.into_iter().map(|(id, _)| id).collect();
        self.record_start = Some(RecordStart { since: self.clock.now() });
        Ok(())
    }

    /// The peaks of a take being recorded, when `key` is its live resource
    pub(crate) fn live_peaks(&mut self, key: &str) -> Option<Arc<lm_io::Peaks>> {
        self.live.peaks(key)
    }

    /// The latency a take is placed back by (F-REC-080..082, H5, H19): what the open device reports (input plus
    /// output), the residual of its calibration (the entry of this device pair, rate and edge converter), and the
    /// manual offset of the preferences on top
    pub(crate) fn recording_latency(&self) -> i64 {
        let reported = self
            .device
            .as_ref()
            .and_then(|d| d.info())
            .map_or((0, false), |i| (i64::from(i.reported_round_trip()), i.edge_resampler));
        let pair = |k: &str| self.preferences.text(&["audio", k]).unwrap_or("");
        let calibrations = self.preferences.get(&["audio", "calibrations"]);
        let residual = calibration_residual(
            calibrations,
            &format!("{}|{}", pair("input"), pair("output")),
            i64::from(self.spec.sample_rate),
            reported.1,
        );
        let manual = self.preferences.int(&["audio", "latencyFrames"]).unwrap_or(0).clamp(-48_000, 48_000);
        reported.0 + residual + manual
    }

    fn poll_disk(&mut self) -> bool {
        let Some(disk) = self.disk.as_mut() else { return false };
        let mut events = Vec::new();
        disk.poll(|e| events.push(e));
        // The growing waveforms (U3)
        let mut pairs = Vec::new();
        disk.live_peaks(|p| pairs.push(p));
        let mut changed = !pairs.is_empty();
        for p in &pairs {
            self.live.pair(p);
        }
        if changed || !self.live.is_empty() {
            self.telemetry.recording = self.live.views();
        }
        for e in events {
            changed = true;
            match e {
                DiskEvent::TakeStarted { input, start_sample, .. } => self.live.started(input.0, start_sample),
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
        while let Some((job, recording)) = self.take_queue.pop_front() {
            self.start_take(job, recording);
        }
    }

    fn start_take(&mut self, job: TakeJob, recording: u64) {
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
        self.recover_midi()?;
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
        let layered = self.preferences.bool(&["recording", "lanes"]).unwrap_or(true);
        let mut placed = Vec::new();
        let mut gaps = 0usize;
        for f in &takes {
            let resource = match Resource::new("audio", &f.key) {
                Ok(r) => r,
                Err(e) => {
                    self.notices.error(&e, None, None);
                    continue;
                }
            };
            let take = &f.take;
            gaps += take.gaps.len();
            let track = take.tracks.first().and_then(|t| NodeId::parse(t).ok()).filter(|t| self.document.exists(*t));
            for planned in crate::recording::plan_items(take, layered) {
                let item =
                    crate::recording::audio_item(map.as_ref().ok(), &planned, &resource, take.latency + take.pdc);
                placed.push((track, item));
            }
        }
        if gaps > 0 {
            // The silence a capture xrun left is in the take (C2, F-REC-084)
            self.notices.warn("session.take.gaps", vec![("count", Value::int(gaps as i64))]);
        }
        if self.recordings.is_empty() {
            self.live.clear();
            self.telemetry.recording.clear();
        }
        let mut recovered_track = false;
        let midi_take = self.midi_take_for_commit();
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
            // I-21: the MIDI take joins here (one undo removes the whole recording)
            if let Some((take, map)) = &midi_take {
                midi::record::apply(take, map, e)?;
            }
            Ok(())
        });
        match result {
            Ok(_) => {
                if recovered_track {
                    self.notices.warn("session.take.recovered_track", Vec::new());
                }
                if let Some((take, _)) = &midi_take {
                    let _ = std::fs::remove_file(&take.path);
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
        // The file's name stays with the resource, for the panels to show (a hash tells nothing)
        let name = source.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
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
            data: name,
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
                        crate::folder::copy_via_tmp(&source, &tmp, &dest, "session.paste.copy")?;
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

    /// Exports as the export screen asks (stems, formats, rate, normalisation…), in an operation; the report goes
    /// to `status().lastExport`
    pub fn export_request(&mut self, req: crate::export::ExportRequest) -> lm_base::OperationId {
        let job = self.export_job();
        self.export_serial += 1;
        let target_id = Target::Export(self.export_serial);
        self.start(Label::new("session.exporting"), target_id, true, "export", move |cancel, progress| {
            crate::export::export_request(job, &req, cancel, progress)
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

    /// The state of a resource of any kind
    pub fn resource_state_of(&self, r: &Resource) -> lm_io::ResourceState {
        self.resources.state_kind(r.kind(), r.key())
    }

    /// Ends the session the way U13 asks: the recording stops and closes, cancellable operations are cancelled,
    /// the others finish, the journal reaches the disk. The caller asks about saving before
    pub fn close(mut self) {
        self.watchdog.shared.running.store(false, Ordering::Release);
        self.stop();
        let lists: [&dyn OpList; 4] = [&self.open_ops, &self.load_ops, &self.kind_ops, &self.import_ops];
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

pub(crate) fn empty_telemetry(rate: u32) -> TelemetryView {
    TelemetryView {
        position: Position { samples: 0, host_time: HostTime(0), sample_rate: rate, playing: false, recording: false },
        meters: std::collections::HashMap::new(),
        inputs: Vec::new(),
        recording: Vec::new(),
        counting_in: false,
        xruns: 0,
        xrun_marks: Vec::new(),
        load: 0.0,
        hand_won: Vec::new(),
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

/// A loaded resource of another kind as a build sees it
pub(crate) fn plugin_content(c: &lm_io::Content) -> lm_plugin::Loaded {
    match c {
        lm_io::Content::Bytes(b) => lm_plugin::Loaded::Bytes(b.clone()),
        lm_io::Content::File(p) => lm_plugin::Loaded::File(Arc::from(p.as_path())),
    }
}

/// The residual stored by `device.calibrate` for this device pair, rate and edge converter (0 when not calibrated)
pub(crate) fn calibration_residual(calibrations: Option<&Value>, pair: &str, rate: i64, edge: bool) -> i64 {
    calibrations
        .into_iter()
        .flat_map(Value::items)
        .find(|c| {
            c.get("devices").and_then(Value::as_text) == Some(pair)
                && c.get("rate").and_then(Value::as_int) == Some(rate)
                && c.get("edgeConverter").and_then(Value::as_bool).unwrap_or(false) == edge
        })
        .and_then(|c| c.get("residual").and_then(Value::as_int))
        .unwrap_or(0)
}

#[cfg(test)]
mod calibration_tests {
    use super::*;

    #[test]
    fn the_stored_residual_is_found_by_pair_rate_and_converter() {
        let entry = |pair: &str, rate: i64, edge: bool, residual: i64| {
            Value::object([
                ("devices", Value::text(pair)),
                ("rate", Value::int(rate)),
                ("edgeConverter", Value::bool(edge)),
                ("residual", Value::int(residual)),
            ])
        };
        let list = Value::list([
            entry("a|b", 48000, false, 120),
            entry("a|b", 44100, false, 7),
            entry("a|b", 48000, true, 33),
        ]);
        assert_eq!(calibration_residual(Some(&list), "a|b", 48000, false), 120);
        assert_eq!(calibration_residual(Some(&list), "a|b", 48000, true), 33);
        assert_eq!(calibration_residual(Some(&list), "a|b", 96000, false), 0);
        assert_eq!(calibration_residual(Some(&list), "x|y", 48000, false), 0);
        assert_eq!(calibration_residual(None, "a|b", 48000, false), 0);
    }
}

/// The folders of the `folders.allowed` preference (C9)
pub(crate) fn allowed_folders(preferences: &Preferences) -> Vec<std::path::PathBuf> {
    preferences
        .get(&["folders", "allowed"])
        .map(|l| l.items().filter_map(Value::as_text).map(std::path::PathBuf::from).collect())
        .unwrap_or_default()
}
