//! Reading a plugin's state in its build (arquitetura-nova.md, 7a: the state is the tree and nothing else).
//!
//! The registry hands the build a **complete** state (every field and every parameter present: what the tree left
//! out takes the schema's frozen default, U29), so a build reads values without defaults of its own. A value that is
//! missing or of the wrong kind is a bug of the plugin's schema, reported as `plugin.state.*`.

use lm_base::{Error, Result};
use lm_state::{PARAMS_KEY, Resource, Value};

/// A complete state, read by key
#[derive(Clone, Copy, Debug)]
pub struct StateView<'a> {
    state: &'a Value,
}

fn missing(key: &str) -> Error {
    Error::new("plugin.state.missing").arg("key", key)
}

fn wrong(key: &str, expected: &'static str) -> Error {
    Error::new("plugin.state.wrong_kind").arg("key", key).arg("expected", expected)
}

impl<'a> StateView<'a> {
    pub fn new(state: &'a Value) -> Self {
        Self { state }
    }

    pub fn value(&self) -> &'a Value {
        self.state
    }

    fn field(&self, key: &str) -> Result<&'a Value> {
        self.state.get(key).ok_or_else(|| missing(key))
    }

    /// A parameter's value (in its own unit)
    pub fn param(&self, key: &str) -> Result<f64> {
        let v = self.state.get(PARAMS_KEY).and_then(|p| p.get(key)).ok_or_else(|| missing(key))?;
        v.as_real().ok_or_else(|| wrong(key, "real"))
    }

    /// A parameter as `f32`, the engine's precision for parameter values
    pub fn param_f32(&self, key: &str) -> Result<f32> {
        self.param(key).map(|v| v as f32)
    }

    pub fn boolean(&self, key: &str) -> Result<bool> {
        self.field(key)?.as_bool().ok_or_else(|| wrong(key, "bool"))
    }

    pub fn int(&self, key: &str) -> Result<i64> {
        self.field(key)?.as_int().ok_or_else(|| wrong(key, "int"))
    }

    pub fn real(&self, key: &str) -> Result<f64> {
        self.field(key)?.as_real().ok_or_else(|| wrong(key, "real"))
    }

    pub fn text(&self, key: &str) -> Result<&'a str> {
        self.field(key)?.as_text().ok_or_else(|| wrong(key, "text"))
    }

    /// A choice, mapped to the plugin's own type by its key (U30). An unknown key is `plugin.state.choice`
    pub fn choice<T>(&self, key: &str, from_key: impl Fn(&str) -> Option<T>) -> Result<T> {
        let text = self.text(key)?;
        from_key(text).ok_or_else(|| Error::new("plugin.state.choice").arg("key", key).arg("value", text))
    }

    /// A nullable resource field
    pub fn resource(&self, key: &str) -> Result<Option<&'a Resource>> {
        let v = self.field(key)?;
        if v.is_null() {
            return Ok(None);
        }
        v.as_resource().map(Some).ok_or_else(|| wrong(key, "resource"))
    }
}
