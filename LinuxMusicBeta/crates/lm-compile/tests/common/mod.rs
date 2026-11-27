//! What the tests share: test plugins in an `lm-plugin` registry, a document, an engine driven by `lm-render`'s
//! `Dummy`, and the loop that installs, builds and returns like the session will
#![allow(dead_code)]

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use lm_base::{Label, Result};
use lm_compile::{Build, Compiled, CompilerConfig, CompilerCore, Environment, InstalledTable, NoResources, Resources};
use lm_doc::{Document, Options, Port, Ports};
use lm_plugin::{BuildContext, Descriptor, PluginKind, Registry, ResourceView, StateView, TestHints, Velocity};
use lm_render::{
    Adopted, Dummy, EngineControl, EventKind, Limits, NodeCore, ProcessContext, ProcessSpec, Processor, Returned,
    engine,
};
use lm_state::{FieldSpec, NodeId, NodeSchema, ParamSpec, PathStep, ROOT, SchemaRegistry, Value};

pub const RATE: u32 = 48_000;
pub const BLOCK: usize = 256;

pub fn spec() -> ProcessSpec {
    ProcessSpec { sample_rate: RATE, max_block: BLOCK }
}

pub fn limits() -> Limits {
    Limits { max_nodes: 4096, max_params: 32_768, ..Limits::default() }
}

// ---------------------------------------------------------------------------------------------------------------
// Test plugins

/// `dc`: while a note is held, every output channel is `level × velocity` (the last note's); silence otherwise
struct Dc {
    level: f32,
    velocity: f32,
    held: u32,
}

impl Processor for Dc {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let outs = cx.buffers.outputs();
        let mut at = 0usize;
        let mut value = if self.held > 0 { self.level * self.velocity } else { 0.0 };
        let mut events = cx.events.iter().peekable();
        while at < frames {
            while let Some(e) = events.peek()
                && e.offset as usize <= at
            {
                match e.kind {
                    EventKind::NoteOn { velocity, .. } => {
                        self.held += 1;
                        self.velocity = velocity;
                    }
                    EventKind::NoteOff { .. } => self.held = self.held.saturating_sub(1),
                    EventKind::Param { index: 0, value } => self.level = value,
                    EventKind::AllNotesRelease | EventKind::AllSoundOff => self.held = 0,
                    _ => {}
                }
                value = if self.held > 0 { self.level * self.velocity } else { 0.0 };
                events.next();
            }
            let end = events.peek().map_or(frames, |e| (e.offset as usize).min(frames)).max(at + 1).min(frames);
            for ch in 0..outs {
                cx.buffers.output(ch)[at..end].fill(value);
            }
            at = end;
        }
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        match old.as_any_mut().downcast_mut::<Dc>() {
            Some(o) => {
                self.held = o.held;
                self.velocity = o.velocity;
                Adopted::Yes
            }
            None => Adopted::No,
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn dc_schemas() -> Vec<NodeSchema> {
    vec![NodeSchema::new("plugin:dc", 1).param(ParamSpec::new("level", 0.0, 1.0, 1.0))]
}

fn dc_ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![] }
}

fn dc_build(state: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let level = StateView::new(state).param_f32("level")?;
    Ok(Box::new(Dc { level, velocity: 0.0, held: 0 }))
}

pub static DC: Descriptor = Descriptor {
    id: "dc",
    name: "test.dc",
    kind: PluginKind::Instrument,
    schemas: dc_schemas,
    ports: dc_ports,
    velocity: Velocity::Level,
    display: None,
    build: dc_build,
    tests: TestHints::new(),
};

/// `kit`: a drum kit whose ports depend on the kit (`small`: kick mono, oh stereo; `big`: plus room stereo)
fn kit_schemas() -> Vec<NodeSchema> {
    vec![NodeSchema::new("plugin:kit", 1).field(FieldSpec::choice("kit", &["small", "big"], "small"))]
}

fn kit_ports(state: &Value) -> Ports {
    let mut outputs = vec![Port::new("kick", 1), Port::new("oh", 2)];
    if state.get("kit").and_then(Value::as_text) == Some("big") {
        outputs.push(Port::new("room", 2));
    }
    Ports { outputs, sidechains: vec![] }
}

fn kit_build(_: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    Ok(Box::new(Dc { level: 1.0, velocity: 0.0, held: 0 }))
}

pub static KIT: Descriptor = Descriptor {
    id: "kit",
    name: "test.kit",
    kind: PluginKind::Instrument,
    schemas: kit_schemas,
    ports: kit_ports,
    velocity: Velocity::Level,
    display: None,
    build: kit_build,
    tests: TestHints::new(),
};

/// `delay`: an effect that delays its input by `latency` samples and reports that latency (a look-ahead plugin)
pub struct Delay {
    line: Vec<Vec<f32>>,
    pos: usize,
    latency: usize,
}

impl Processor for Delay {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        if self.latency == 0 {
            return;
        }
        let frames = cx.frames;
        for (ch, line) in self.line.iter_mut().enumerate().take(cx.buffers.outputs()) {
            let io = cx.buffers.in_out(ch);
            let mut p = self.pos;
            for x in io.iter_mut().take(frames) {
                std::mem::swap(&mut line[p], x);
                p = (p + 1) % self.latency;
            }
        }
        self.pos = (self.pos + frames) % self.latency;
    }

    fn latency(&self) -> u32 {
        self.latency as u32
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn delay_schemas() -> Vec<NodeSchema> {
    vec![NodeSchema::new("plugin:delay", 1).field(FieldSpec::int("latency", 0, 100_000, 0))]
}

fn effect_ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![] }
}

fn delay_build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let latency = StateView::new(state).int("latency")? as usize;
    Ok(Box::new(Delay { line: vec![vec![0.0; latency.max(1)]; usize::from(cx.channels.max(1))], pos: 0, latency }))
}

pub static DELAY: Descriptor = Descriptor {
    id: "delay",
    name: "test.delay",
    kind: PluginKind::Effect,
    schemas: delay_schemas,
    ports: effect_ports,
    velocity: Velocity::Ignored,
    display: None,
    build: delay_build,
    tests: TestHints::new(),
};

/// `duck`: an effect with a stereo sidechain port (`sidechain`); it passes its input
struct Pass;

impl Processor for Pass {
    fn process(&mut self, _: &mut ProcessContext<'_>) {}

    fn adopt(&mut self, _: &mut dyn Processor) -> Adopted {
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn duck_schemas() -> Vec<NodeSchema> {
    vec![NodeSchema::new("plugin:duck", 1).param(ParamSpec::new("depth", 0.0, 1.0, 0.5))]
}

fn duck_ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: vec![Port::new("sidechain", 2)] }
}

fn pass_build(_: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    Ok(Box::new(Pass))
}

pub static DUCK: Descriptor = Descriptor {
    id: "duck",
    name: "test.duck",
    kind: PluginKind::Effect,
    schemas: duck_schemas,
    ports: duck_ports,
    velocity: Velocity::Ignored,
    display: None,
    build: pass_build,
    tests: TestHints::new(),
};

/// What `probe` heard: (timeline sample, event), notes and their expressions
pub static PROBE: std::sync::Mutex<Vec<(i64, EventKind)>> = std::sync::Mutex::new(Vec::new());

/// `probe`: an instrument that writes down the notes and expressions it gets, at their timeline sample
struct Probe;

impl Processor for Probe {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let at = cx.transport.position;
        let mut seen = PROBE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for e in cx.events {
            if matches!(e.kind, EventKind::NoteOn { .. } | EventKind::NoteOff { .. } | EventKind::NoteExpression { .. })
            {
                seen.push((at + i64::from(e.offset), e.kind));
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn probe_schemas() -> Vec<NodeSchema> {
    vec![NodeSchema::new("plugin:probe", 1)]
}

fn probe_build(_: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    Ok(Box::new(Probe))
}

pub static PROBE_PLUGIN: Descriptor = Descriptor {
    id: "probe",
    name: "test.probe",
    kind: PluginKind::Instrument,
    schemas: probe_schemas,
    ports: dc_ports,
    velocity: Velocity::Ignored,
    display: None,
    build: probe_build,
    tests: TestHints::new(),
};

fn nfx_schemas() -> Vec<NodeSchema> {
    vec![NodeSchema::new("plugin:nfx", 1).field(FieldSpec::int("mode", 0, 100, 0))]
}

/// `nfx`: a note effect that passes everything (its `mode` only makes a state to change)
pub static NFX: Descriptor = Descriptor {
    id: "nfx",
    name: "test.nfx",
    kind: PluginKind::NoteEffect,
    schemas: nfx_schemas,
    ports: effect_ports,
    velocity: Velocity::Ignored,
    display: None,
    build: pass_build,
    tests: TestHints::new(),
};

pub fn plugins() -> Arc<Registry> {
    Registry::new(&[&DC, &KIT, &DELAY, &DUCK, &PROBE_PLUGIN, &NFX]).expect("registry")
}

pub fn schemas(plugins: &Registry) -> Arc<SchemaRegistry> {
    Arc::new(plugins.project_schemas().expect("schemas"))
}

pub fn config(plugins: &Arc<Registry>) -> CompilerConfig {
    CompilerConfig { spec: spec(), limits: limits(), schemas: schemas(plugins), plugins: plugins.clone() }
}

// ---------------------------------------------------------------------------------------------------------------
// The document

pub fn k(key: &str) -> PathStep {
    PathStep::key(key)
}

pub fn real(v: f64) -> Value {
    Value::real(v).expect("finite")
}

pub fn text(t: &str) -> Value {
    Value::text(t)
}

pub fn new_doc(plugins: &Arc<Registry>) -> Document {
    let options = Options { ports: Some(plugins.ports_fn()), ..Options::default() };
    Document::new_project(schemas(plugins), options).expect("document")
}

pub fn edit(doc: &mut Document, f: impl FnOnce(&mut lm_doc::Edit<'_>) -> Result<()>) {
    doc.transact(Label::new("test.edit"), f).expect("transaction");
}

pub fn plugin(name: &str, state: Value) -> Value {
    Value::object([("plugin", text(name)), ("state", state)])
}

pub fn dc() -> Value {
    plugin("dc", Value::object([("params", Value::object([("level", real(1.0))]))]))
}

pub fn delay(latency: i64) -> Value {
    plugin("delay", Value::object([("latency", Value::int(latency))]))
}

pub fn add_track(doc: &mut Document, instrument: Option<Value>) -> NodeId {
    let mut out = None;
    edit(doc, |e| {
        let n = e.root().get("tracks").map_or(0, Value::len);
        let mut node = Value::object([("name", text("t"))]);
        if let Some(i) = instrument {
            node = node.with("instrument", i);
        }
        out = Some(e.insert(ROOT, "tracks", n, node)?);
        Ok(())
    });
    out.expect("track")
}

pub fn add_channel(doc: &mut Document) -> NodeId {
    let mut out = None;
    edit(doc, |e| {
        let n = e.root().get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len);
        out = Some(e.insert(ROOT, "mixer.channels", n, Value::object([("name", text("c"))]))?);
        Ok(())
    });
    out.expect("channel")
}

pub fn add_item(doc: &mut Document, track: NodeId, kind: &str, start: i64, length: i64) -> NodeId {
    let mut out = None;
    edit(doc, |e| {
        let n = e.find(track).and_then(|t| t.get("items")).map_or(0, Value::len);
        out = Some(e.insert(
            track,
            "items",
            n,
            Value::object([("kind", text(kind)), ("start", Value::int(start)), ("length", Value::int(length))]),
        )?);
        Ok(())
    });
    out.expect("item")
}

pub fn note(key: i64, start: i64, length: i64, velocity: f64) -> Value {
    Value::object([
        ("key", Value::int(key)),
        ("start", Value::int(start)),
        ("length", Value::int(length)),
        ("velocity", real(velocity)),
    ])
}

pub fn outputs(doc: &Document, track: NodeId) -> Vec<NodeId> {
    doc.find(track)
        .and_then(|t| t.get("outputs").cloned())
        .map(|o| o.items().filter_map(|o| NodeId::parse(o.get("id")?.as_text()?).ok()).collect())
        .unwrap_or_default()
}

pub fn route_output(doc: &mut Document, output: NodeId, channel: NodeId) {
    edit(doc, |e| e.set(output, &[k("to")], text(&channel.to_string())));
}

pub fn insert(doc: &mut Document, channel: NodeId, p: Value) -> NodeId {
    let mut out = None;
    edit(doc, |e| {
        let n = e.find(channel).and_then(|c| c.get("inserts")).map_or(0, Value::len);
        out = Some(e.insert(channel, "inserts", n, p)?);
        Ok(())
    });
    out.expect("insert")
}

pub fn digital(doc: &mut Document) {
    edit(doc, |e| {
        let mixer = e.root().get("mixer").cloned().unwrap_or_default();
        let _ = mixer;
        e.set(ROOT, &[k("mixer"), k("mode")], text("digital"))
    });
}

// ---------------------------------------------------------------------------------------------------------------
// Builds, as the session will do them

/// The rack in the tests: passes the audio (the real one is `lm-dsp`'s strip, built by the session)
pub fn build(plugins: &Registry, b: &lm_compile::BuildRequest) -> Box<NodeCore> {
    let processor: Box<dyn Processor> = match &b.build {
        Build::Plugin { plugin, state, channels, .. } => {
            let view = ResourceView::new();
            let cx = BuildContext { spec: b.spec, resources: &view, channels: *channels };
            plugins.build(plugin, state, &cx).expect("build")
        }
        Build::Rack { .. } => Box::new(Pass),
    };
    NodeCore::new(processor, b.seed, None)
}

/// A compiler and an engine, with the session's loop
pub struct Harness {
    pub plugins: Arc<Registry>,
    pub core: CompilerCore,
    pub control: EngineControl,
    pub dummy: Dummy,
    pub table: Option<Arc<InstalledTable>>,
    pub env: Environment,
    pub resources: Arc<dyn Resources>,
    /// Every `Returned` checked: nothing skipped
    pub returns: usize,
    /// The builds asked and not made yet (the session's build operations)
    pub outstanding: Vec<lm_compile::BuildRequest>,
}

impl Harness {
    pub fn new(plugins: &Arc<Registry>) -> Self {
        lm_render::install_panic_hook();
        let (control, renderer) = engine(spec(), limits());
        Self {
            plugins: plugins.clone(),
            core: CompilerCore::new(config(plugins)),
            control,
            dummy: Dummy::new(renderer, 2, 2),
            table: None,
            env: Environment::default(),
            resources: Arc::new(NoResources),
            returns: 0,
            outstanding: Vec::new(),
        }
    }

    /// Compiles and installs (builds are not made)
    pub fn compile(&mut self, root: &Value) -> Compiled {
        let mut c = self.core.compile(root, self.resources.as_ref(), &self.env).expect("compile");
        let install =
            std::mem::replace(&mut c.install, lm_render::Install::new(lm_render::Program::empty(spec(), &limits())));
        self.control.install(install).expect("install");
        self.table = Some(c.table.clone());
        self.outstanding.retain(|b| !c.cancelled.contains(&b.ticket));
        self.outstanding.extend(c.builds.iter().cloned());
        c
    }

    /// Renders a block and gives back what came back
    pub fn pump(&mut self) {
        self.dummy.block(BLOCK);
        let mut returned: Vec<Returned> = Vec::new();
        self.control.collect(|r| returned.push(r));
        for r in returned {
            assert_eq!(r.skipped, 0, "the renderer skipped parts of install {:?}", r.generation);
            self.core.returned(&r);
            self.returns += 1;
        }
    }

    /// Compiles, builds everything asked and compiles again until nothing waits
    pub fn settle(&mut self, root: &Value) -> Compiled {
        for _ in 0..8 {
            let c = self.compile(root);
            self.pump();
            if c.complete() {
                return c;
            }
            self.build_all();
        }
        let c = self.compile(root);
        panic!(
            "never settled: waiting {:?} replacing {:?} builds {:?} items {:?}",
            c.waiting,
            c.replacing,
            c.builds.iter().map(|b| b.node).collect::<Vec<_>>(),
            c.waiting_items
        );
    }
}

pub fn wait() -> Duration {
    Duration::from_secs(10)
}

impl Harness {
    /// Makes every outstanding build (as the session's operations finishing)
    pub fn build_all(&mut self) {
        for b in std::mem::take(&mut self.outstanding) {
            let core = build(&self.plugins, &b);
            assert!(self.core.ready(b.ticket, core), "an outstanding ticket is accepted");
        }
    }

    /// Makes the outstanding builds `keep` says yes to
    pub fn build_some(&mut self, mut keep: impl FnMut() -> bool) {
        let all = std::mem::take(&mut self.outstanding);
        for b in all {
            if keep() {
                let core = build(&self.plugins, &b);
                assert!(self.core.ready(b.ticket, core), "an outstanding ticket is accepted");
            } else {
                self.outstanding.push(b);
            }
        }
    }

    pub fn play(&mut self) {
        self.control.commands().send(lm_render::Command::Play).expect("play");
    }

    /// Renders `blocks` blocks; for every sample rendered while playing, its timeline position and the two device
    /// outputs
    pub fn render(&mut self, blocks: usize) -> Vec<(i64, f32, f32)> {
        let mut out = Vec::new();
        for _ in 0..blocks {
            let playing = self.dummy.renderer().is_playing();
            let pos = self.dummy.renderer().position();
            self.pump();
            if playing && self.dummy.renderer().is_playing() {
                let o = self.dummy.outputs();
                for (i, (l, r)) in o[0][..BLOCK].iter().zip(&o[1][..BLOCK]).enumerate() {
                    out.push((pos + i as i64, *l, *r));
                }
            }
        }
        out
    }
}
