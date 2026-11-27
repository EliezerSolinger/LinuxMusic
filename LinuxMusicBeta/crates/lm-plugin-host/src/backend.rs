//! A plugin as the helper runs it, whatever its format
use lm_base::{Error, Result};
use lm_render::EventKind;
use lm_state::Value;

/// One parameter, in the order the core indexes them
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub key: String,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub integer: bool,
    pub toggled: bool,
    pub logarithmic: bool,
}

impl Param {
    pub fn to_value(&self) -> Value {
        let r = |x: f64| Value::real(x).unwrap_or(Value::NULL);
        Value::object([
            ("key", Value::text(self.key.as_str())),
            ("name", Value::text(self.name.as_str())),
            ("min", r(self.min)),
            ("max", r(self.max)),
            ("default", r(self.default)),
            ("integer", Value::bool(self.integer)),
            ("toggled", Value::bool(self.toggled)),
            ("logarithmic", Value::bool(self.logarithmic)),
        ])
    }
}

/// What the scan lists of a plugin (from its metadata: no plugin code runs)
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub format: String,
    pub uri: String,
    pub name: String,
    pub instrument: bool,
    pub inputs: usize,
    pub outputs: usize,
    pub midi_in: bool,
    pub params: Vec<Param>,
    pub latency: bool,
    pub has_window: bool,
    /// The file it was found in (CLAP, VST3, LADSPA), tried first when it is loaded
    pub file: Option<String>,
}

impl Listed {
    pub fn to_value(&self) -> Value {
        Value::object([
            ("t", Value::text("plugin")),
            ("format", Value::text(self.format.as_str())),
            ("uri", Value::text(self.uri.as_str())),
            ("name", Value::text(self.name.as_str())),
            ("instrument", Value::bool(self.instrument)),
            ("inputs", Value::int(self.inputs as i64)),
            ("outputs", Value::int(self.outputs as i64)),
            ("midi_in", Value::bool(self.midi_in)),
            ("params", Value::list(self.params.iter().map(Param::to_value))),
            ("latency", Value::bool(self.latency)),
            ("has_window", Value::bool(self.has_window)),
            ("file", self.file.as_deref().map_or(Value::NULL, Value::text)),
        ])
    }
}

/// The song's position for one block, from the shared block
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transport {
    pub rate: u32,
    pub playing: bool,
    pub recording: bool,
    /// The timeline sample of the block's first frame
    pub position: i64,
    pub tick: i64,
    pub ppq: u32,
    pub bpm: f64,
    /// (numerator, denominator)
    pub signature: (u8, u8),
}

impl Transport {
    /// Quarter notes from the start
    pub fn beats(&self) -> f64 {
        if self.ppq == 0 { 0.0 } else { self.tick as f64 / f64::from(self.ppq) }
    }

    pub fn seconds(&self) -> f64 {
        if self.rate == 0 { 0.0 } else { self.position as f64 / f64::from(self.rate) }
    }
}

/// What the core asks of a plugin's own window (I-22)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRequest {
    Show,
    Hide,
    /// The client area's new size, from the core's side (the user dragged, a preference)
    Resize(u32, u32),
}

/// What a plugin's window is after a request
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowState {
    pub open: bool,
    /// The window's client size when it is open and the helper knows it
    pub size: Option<(u32, u32)>,
    /// Why there is no window when one was asked for (an error key: the core says it and keeps its generic editor)
    pub reason: Option<&'static str>,
}

impl WindowState {
    pub fn open(size: Option<(u32, u32)>) -> WindowState {
        WindowState { open: true, size, reason: None }
    }

    pub fn closed() -> WindowState {
        WindowState { open: false, size: None, reason: None }
    }

    /// No window, and why
    pub fn none(reason: &'static str) -> WindowState {
        WindowState { open: false, size: None, reason: Some(reason) }
    }
}

/// Saves a plugin's state without holding the plugin (see [`Plugin::state_saver`])
pub type StateSaver = Box<dyn Fn() -> Result<Option<Vec<u8>>> + Send + Sync>;

/// A plugin instance in the helper
pub trait Plugin: Send {
    fn params(&self) -> &[Param];
    /// The audio channels the plugin has: (inputs, outputs); `None` when it does not say (it never runs as dual mono)
    fn io(&self) -> Option<(usize, usize)> {
        None
    }
    fn latency(&self) -> u32;
    fn set_param(&mut self, index: u32, value: f32);
    /// A parameter's value now (after a state was restored, the core reads them back)
    fn param(&self, index: u32) -> f32;
    /// One block: `events` with their offsets, in order
    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[(u32, EventKind)],
        transport: &Transport,
    );
    fn save(&mut self) -> Result<Option<Vec<u8>>>;
    /// For a format that lets the main thread save while the audio thread processes (CLAP): a function that saves
    /// without the plugin held, so a slow `save` does not silence it. Used from the main thread only, and dropped
    /// before the plugin. The other formats answer `None` and are saved with the plugin held (the audio thread
    /// answers silence meanwhile)
    fn state_saver(&self) -> Option<StateSaver> {
        None
    }
    fn restore(&mut self, chunk: &[u8]) -> Result<()>;
    /// Shows, hides or resizes the plugin's own window (main thread). A plugin without one answers
    /// [`WindowState::none`] with the reason
    fn window(&mut self, request: WindowRequest) -> Result<WindowState> {
        let _ = request;
        Ok(WindowState::none("plugin.window.none"))
    }
    /// The user closed the plugin's window since the last call (main thread; polled with `idle`)
    fn window_closed(&mut self) -> bool {
        false
    }
    /// The gesture edges the plugin's window made since the last call: (parameter index, `true` begin / `false`
    /// end), in order; each begin is followed by its changes ([`Plugin::changes`]) and its end
    fn gestures(&mut self, out: &mut Vec<(u32, bool)>) {
        let _ = out;
    }
    /// The main thread's turn, every ~10 ms (the plugin's callbacks, restarts)
    fn idle(&mut self) {}
    /// Whether the plugin said its own state changed since the last call (the core then asks for it)
    fn state_changed(&mut self) -> bool {
        false
    }
    /// The plugin's own parameter changes since the last call (its window, a preset): (index, value)
    fn changes(&mut self, out: &mut Vec<(u32, f32)>) {
        let _ = out;
    }
}

pub fn unknown_format(format: &str) -> Error {
    Error::new("plugin_host.format").arg("format", format)
}
