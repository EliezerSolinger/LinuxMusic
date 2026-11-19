//! The data forms of changes:
//!
//! - [`JournalOp`] / [`JournalPatch`]: what the journal writes (I-4, R16): changes to the root by absolute path, in
//!   the order they were applied, results of the domain rules included. Replaying them onto the saved root gives the
//!   live root bit for bit, without running any rule (I-3.9: the patches already carry the result);
//! - [`PatchOp`] / [`Patch`]: a heavy edit computed off the main thread on a snapshot (H7), by node id, applied as
//!   one transaction (with the domain rules) if the nodes it was computed from did not change.

use std::sync::Arc;

use lm_base::{Error, Result};
use lm_state::{NodeId, PathStep, Value, path_text};

/// One change to the root, by absolute path
#[derive(Clone, Debug, PartialEq)]
pub enum JournalOp {
    /// Sets the value at `path` (every step but the last must exist; the last may add a key)
    Set { path: Vec<PathStep>, value: Value },
    /// Removes the key (or note) at `path`, or the item of a list at a final index
    Remove { path: Vec<PathStep> },
    /// Inserts `value` at `index` in the list at `path`
    Insert { path: Vec<PathStep>, index: usize, value: Value },
}

impl JournalOp {
    /// Applies the change to a root
    pub fn apply(&self, root: &Value) -> Result<Value> {
        match self {
            JournalOp::Set { path, value } => root.with_path(path, value.clone()),
            JournalOp::Remove { path } => root.without_path(path),
            JournalOp::Insert { path, index, value } => {
                let list = root
                    .at_path(path)
                    .ok_or_else(|| Error::new("doc.journal.not_found").arg("path", path_text(path)))?;
                root.with_path(path, list.with_inserted(*index, value.clone())?)
            }
        }
    }

    /// `{"op": "set", "path": [...], "value": ...}`; a path step is a text (a key, or a note's number) or an int (a
    /// list index)
    pub fn to_value(&self) -> Result<Value> {
        let path = |p: &[PathStep]| -> Result<Value> {
            let mut steps = Vec::with_capacity(p.len());
            for step in p {
                steps.push(match step {
                    PathStep::Key(k) => Value::text(Arc::clone(k)),
                    PathStep::Note(n) => Value::text(lm_state::note_key(*n)),
                    PathStep::Index(i) => Value::int(i64::try_from(*i).map_err(|_| Error::new("doc.journal.index"))?),
                });
            }
            Ok(Value::list(steps))
        };
        Ok(match self {
            JournalOp::Set { path: p, value } => {
                Value::object([("op", Value::text("set")), ("path", path(p)?), ("value", value.clone())])
            }
            JournalOp::Remove { path: p } => Value::object([("op", Value::text("remove")), ("path", path(p)?)]),
            JournalOp::Insert { path: p, index, value } => Value::object([
                ("op", Value::text("insert")),
                ("path", path(p)?),
                ("index", Value::int(i64::try_from(*index).map_err(|_| Error::new("doc.journal.index"))?)),
                ("value", value.clone()),
            ]),
        })
    }

    pub fn from_value(v: &Value) -> Result<Self> {
        let bad = || Error::new("doc.journal.invalid");
        let path = |p: Option<&Value>| -> Result<Vec<PathStep>> {
            let p = p.filter(|p| p.is_list()).ok_or_else(bad)?;
            p.items()
                .map(|s| match (s.as_shared_text(), s.as_int()) {
                    (Some(k), _) => Ok(PathStep::Key(Arc::clone(k))),
                    (_, Some(i)) => usize::try_from(i).map(PathStep::Index).map_err(|_| bad()),
                    _ => Err(bad()),
                })
                .collect()
        };
        let value = || v.get("value").cloned().ok_or_else(bad);
        match v.get("op").and_then(Value::as_text) {
            Some("set") => Ok(JournalOp::Set { path: path(v.get("path"))?, value: value()? }),
            Some("remove") => Ok(JournalOp::Remove { path: path(v.get("path"))? }),
            Some("insert") => {
                let index =
                    v.get("index").and_then(Value::as_int).and_then(|i| usize::try_from(i).ok()).ok_or_else(bad)?;
                Ok(JournalOp::Insert { path: path(v.get("path"))?, index, value: value()? })
            }
            _ => Err(bad()),
        }
    }
}

/// The changes since the last time the session took them (one journal line)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JournalPatch {
    pub ops: Vec<JournalOp>,
}

impl JournalPatch {
    pub fn apply(&self, root: &Value) -> Result<Value> {
        self.ops.iter().try_fold(root.clone(), |r, op| op.apply(&r))
    }

    /// The patch as a list of ops (for the journal line `{"seq": N, "patch": [...]}`)
    pub fn to_value(&self) -> Result<Value> {
        Ok(Value::list(self.ops.iter().map(JournalOp::to_value).collect::<Result<Vec<_>>>()?))
    }

    pub fn from_value(v: &Value) -> Result<Self> {
        if !v.is_list() {
            return Err(Error::new("doc.journal.invalid"));
        }
        Ok(Self { ops: v.items().map(JournalOp::from_value).collect::<Result<_>>()? })
    }
}

/// Replays journal patches, in order, onto a root (the saved one, as loaded). No rule runs: the patches carry their
/// results
pub fn replay(root: &Value, patches: &[JournalPatch]) -> Result<Value> {
    patches.iter().try_fold(root.clone(), |r, p| p.apply(&r))
}

/// One edit of a heavy edit, by node id (applied through [`crate::Edit`])
#[derive(Clone, Debug, PartialEq)]
pub enum PatchOp {
    Set { node: NodeId, path: Vec<PathStep>, value: Value },
    Insert { parent: NodeId, list: Arc<str>, index: usize, node: Value },
    Remove { node: NodeId },
    Move { node: NodeId, parent: NodeId, list: Arc<str>, index: usize },
    AddNote { item: NodeId, note: Value },
    SetNote { item: NodeId, n: u64, path: Vec<PathStep>, value: Value },
    RemoveNote { item: NodeId, n: u64 },
}

/// A heavy edit, ready, and the nodes it was computed from (H7). If any of them is no longer the same node in the
/// current root, applying it is a conflict (`doc.patch.conflict`): the session computes it again once, or refuses
/// it with a notice
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch {
    pub ops: Vec<PatchOp>,
    pub based_on: Vec<(NodeId, Value)>,
}
