//! The modulator actions in the fake (the session's are in lm-session/src/modulators.rs, the same rules):
//! `modulator.attach` and `modulator.remove`, which the modulator editor and the controls' menus call.

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, Value};

use crate::FakePort;

fn node(p: &Value, key: &str) -> Result<NodeId> {
    p.get(key).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(|| Error::new("action.params"))
}

/// The modulator that has `(node, param)` as a target
fn taken(root: &Value, owner: NodeId, param: &str) -> bool {
    let mut found = false;
    root.walk(&mut |_, v| {
        let is_modulator =
            v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).is_some_and(|i| i.kind == b'n');
        if is_modulator
            && v.get("targets").is_some_and(|ts| {
                ts.items().any(|t| {
                    t.get("node").and_then(Value::as_text) == Some(owner.to_string().as_str())
                        && t.get("param").and_then(Value::as_text) == Some(param)
                })
            })
        {
            found = true;
        }
    });
    found
}

impl FakePort {
    /// The parameter's modulator: the one it has, or a new LFO in the node's list with the parameter as its target
    pub(crate) fn modulator_attach(&mut self, params: &Value) -> Result<()> {
        let owner = node(params, "node")?;
        let param = params
            .get("param")
            .and_then(Value::as_text)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::new("action.params"))?;
        if taken(self.doc.root(), owner, param) {
            return Ok(());
        }
        let kind = params.get("kind").and_then(Value::as_text).unwrap_or("lfo").to_owned();
        self.doc
            .transact(Label::new("modulator.attach"), |e| {
                let n = e.find(owner).and_then(|o| o.get("modulators")).map_or(0, Value::len);
                let target = Value::object([("node", Value::text(owner.to_string())), ("param", Value::text(param))]);
                let m = Value::object([("kind", Value::text(kind)), ("targets", Value::list([target]))]);
                e.insert(owner, "modulators", n, m).map(|_| ())
            })
            .map(|_| ())
    }

    pub(crate) fn modulator_remove(&mut self, params: &Value) -> Result<()> {
        let m = node(params, "modulator")?;
        self.doc.transact(Label::new("modulator.remove"), |e| e.remove(m)).map(|_| ())
    }
}
