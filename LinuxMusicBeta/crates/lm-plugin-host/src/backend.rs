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
        ])
    }
}

/// A plugin instance in the helper
pub trait Plugin: Send {
    fn params(&self) -> &[Param];
    fn latency(&self) -> u32;
    fn set_param(&mut self, index: u32, value: f32);
    /// One block: `events` with their offsets, in order
    fn process(&mut self, frames: usize, inputs: &[&[f32]], outputs: &mut [&mut [f32]], events: &[(u32, EventKind)]);
    fn save(&mut self) -> Result<Option<Vec<u8>>>;
    fn restore(&mut self, chunk: &[u8]) -> Result<()>;
    /// Opens or closes the plugin's own window; `Ok(false)` when it has none
    fn window(&mut self, on: bool) -> Result<bool> {
        let _ = on;
        Ok(false)
    }
    /// The plugin's own parameter changes since the last call (its window, a preset): (index, value)
    fn changes(&mut self, out: &mut Vec<(u32, f32)>) {
        let _ = out;
    }
}

pub fn unknown_format(format: &str) -> Error {
    Error::new("plugin_host.format").arg("format", format)
}
