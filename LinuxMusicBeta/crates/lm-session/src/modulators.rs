//! The modulators as actions (decisoes-autonomas, 26/11, item 1): a modulator lives in the `modulators` list of the
//! owner of the parameters it moves (a track, a channel, a plugin); each is one transaction, one undo step.
//!
//! ```text
//! modulator.add           { owner: text, kind?: text }                a modulator in the owner's list (default "lfo")
//! modulator.attach        { node: text, param: text, kind?: text }    the parameter's modulator: the one it has, or a
//!                         new one in the node's list with the parameter as its target
//! modulator.remove        { modulator: text }
//! modulator.add_target    { modulator: text, node: text, param: text, amount?: real }
//! modulator.remove_target { target: text }
//! ```
//!
//! A parameter has at most one modulator (`modulator.param_taken`)

use lm_base::{Error, Label, Result};
use lm_doc::schemas::{KIND_MODULATOR, MODULATOR_KINDS};
use lm_state::{NodeId, Value};

use crate::actions::ActionSpec;
use crate::session::Session;

fn params() -> Error {
    Error::new("action.params")
}

fn node(p: &Value, key: &str) -> Result<NodeId> {
    p.get(key).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)
}

fn kind(p: &Value) -> Result<&'static str> {
    match p.get("kind").and_then(Value::as_text) {
        None => Ok("lfo"),
        Some(k) => MODULATOR_KINDS.iter().copied().find(|x| *x == k).ok_or_else(params),
    }
}

fn writable(s: &Session) -> bool {
    !s.project().read_only
}

/// The modulator that has `(node, param)` as a target
fn taken(root: &Value, node: NodeId, param: &str) -> Option<NodeId> {
    let mut found = None;
    root.walk(&mut |_, v| {
        if found.is_some() {
            return;
        }
        let Some(id) = v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()) else { return };
        if id.kind != KIND_MODULATOR {
            return;
        }
        let hit = v.get("targets").is_some_and(|ts| {
            ts.items().any(|t| {
                t.get("node").and_then(Value::as_text) == Some(node.to_string().as_str())
                    && t.get("param").and_then(Value::as_text) == Some(param)
            })
        });
        if hit {
            found = Some(id);
        }
    });
    found
}

fn target_value(node: NodeId, param: &str, amount: Option<f64>) -> Result<Value> {
    let mut t = Value::object([("node", Value::text(node.to_string())), ("param", Value::text(param))]);
    if let Some(a) = amount {
        t = t.with("amount", Value::real(a.clamp(-1.0, 1.0))?);
    }
    Ok(t)
}

pub(crate) static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "modulator.add",
        label: "action.modulator.add",
        run: |s, p| {
            let owner = node(p, "owner")?;
            let kind = kind(p)?;
            s.document_mut().transact(Label::new("modulator.add"), |e| {
                let n = e.find(owner).and_then(|o| o.get("modulators")).map_or(0, Value::len);
                e.insert(owner, "modulators", n, Value::object([("kind", Value::text(kind))])).map(|_| ())
            })?;
            Ok(())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "modulator.attach",
        label: "action.modulator.attach",
        run: |s, p| {
            let owner = node(p, "node")?;
            let param = p.get("param").and_then(Value::as_text).filter(|t| !t.is_empty()).ok_or_else(params)?;
            let kind = kind(p)?;
            if taken(s.document().root(), owner, param).is_some() {
                return Ok(());
            }
            s.document_mut().transact(Label::new("modulator.attach"), |e| {
                let n = e.find(owner).and_then(|o| o.get("modulators")).map_or(0, Value::len);
                let m = Value::object([
                    ("kind", Value::text(kind)),
                    ("targets", Value::list([target_value(owner, param, None)?])),
                ]);
                e.insert(owner, "modulators", n, m).map(|_| ())
            })?;
            Ok(())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "modulator.remove",
        label: "action.modulator.remove",
        run: |s, p| {
            let m = node(p, "modulator")?;
            s.document_mut().transact(Label::new("modulator.remove"), |e| e.remove(m)).map(|_| ())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "modulator.add_target",
        label: "action.modulator.add_target",
        run: |s, p| {
            let m = node(p, "modulator")?;
            let target = node(p, "node")?;
            let param = p.get("param").and_then(Value::as_text).filter(|t| !t.is_empty()).ok_or_else(params)?;
            if taken(s.document().root(), target, param).is_some() {
                return Err(Error::new("modulator.param_taken").arg("param", param));
            }
            let amount = p.get("amount").and_then(Value::as_real);
            s.document_mut().transact(Label::new("modulator.add_target"), |e| {
                let n = e.find(m).and_then(|o| o.get("targets")).map_or(0, Value::len);
                e.insert(m, "targets", n, target_value(target, param, amount)?).map(|_| ())
            })?;
            Ok(())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "modulator.remove_target",
        label: "action.modulator.remove_target",
        run: |s, p| {
            let t = node(p, "target")?;
            s.document_mut().transact(Label::new("modulator.remove_target"), |e| e.remove(t)).map(|_| ())
        },
        enabled: writable,
    },
];
