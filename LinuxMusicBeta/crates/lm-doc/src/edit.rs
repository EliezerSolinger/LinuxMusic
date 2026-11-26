//! [`Edit`]: the changes of one transaction (I-3.5), on a working copy of the current root and its indices.
//!
//! Every change goes through three primitives (replace a value inside a node, insert into a list of nodes, remove
//! from one). Each keeps the indices exact, records the journal op and its inverse, the diff, and (in a gesture) the
//! value before the gesture.

use std::collections::BTreeSet;
use std::sync::Arc;

use imbl::{OrdMap, OrdSet, Vector};
use lm_base::{Error, Label, Result};
use lm_state::{FieldKind, MAX_SAFE_INT, NodeId, OnRemove, PathStep, ROOT, Value, path_text};

use crate::diff::{DiffAcc, NoteEvent};
use crate::index::{Ctx, Event, Index, Loc, RelPath, Slot, not_found};
use crate::ops::{JournalOp, PatchOp};
use crate::rules::{Limits, PortsFn, Rule, UsageFn};
use crate::schemas::{plugin_type, types};

/// A root and its indices
#[derive(Clone, Debug)]
pub(crate) struct State {
    pub root: Value,
    pub index: Index,
}

/// What a document is configured with, shared by every transaction
pub(crate) struct Env {
    pub ctx: Ctx,
    pub ports: Option<PortsFn>,
    pub limits: Option<Limits>,
    pub usage: UsageFn,
    pub rules: Vec<Rule>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env").field("ctx", &self.ctx).field("limits", &self.limits).finish_non_exhaustive()
    }
}

/// Where a moved node was: parent, container, index
pub(crate) type Place = (NodeId, Arc<[Arc<str>]>, usize);

/// What a gesture touched, and the values before it (I-3.7)
#[derive(Clone, Debug, Default)]
pub(crate) struct GestureRecord {
    /// The value before the gesture at each path it touched (`None`: there was nothing there)
    pub touched: OrdMap<(NodeId, RelPath), Option<Value>>,
    /// The order of first touch, for restoring in reverse
    pub order: Vector<(NodeId, RelPath)>,
    /// Where each moved node was before the gesture: parent, container, index
    pub moves: OrdMap<NodeId, Place>,
    pub move_order: Vector<NodeId>,
    /// The nodes the gesture changes (G3)
    pub targets: OrdSet<NodeId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The caller's closure
    User,
    /// The domain rules and the document's own changes
    Rules,
}

/// The working state of one transaction
#[derive(Debug)]
pub(crate) struct Work {
    pub env: Arc<Env>,
    pub state: State,
    pub issued: u64,
    pub ops: Vec<JournalOp>,
    /// The inverse of each op, in the order of the ops (apply reversed)
    pub inverse: Vec<JournalOp>,
    pub acc: DiffAcc,
    added_refs: BTreeSet<NodeId>,
    plugins: BTreeSet<NodeId>,
    pub gesture: Option<GestureRecord>,
    pub gesture_mode: bool,
    pub phase: Phase,
}

/// The changes of one transaction. Every method checks what it can at once (schemas, paths, ids); the rules across
/// nodes run when the transaction commits, and any error refuses the whole transaction
#[derive(Debug)]
pub struct Edit<'a> {
    pub(crate) w: &'a mut Work,
}

/// A copied part of a project (H13): the node, its type and the uuid of the project it came from
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub type_name: &'static str,
    pub node: Value,
    pub source_uuid: Arc<str>,
}

impl Work {
    pub fn new(env: Arc<Env>, state: State, issued: u64) -> Self {
        Self {
            env,
            state,
            issued,
            ops: Vec::new(),
            inverse: Vec::new(),
            acc: DiffAcc::default(),
            added_refs: BTreeSet::new(),
            plugins: BTreeSet::new(),
            gesture: None,
            gesture_mode: false,
            phase: Phase::User,
        }
    }

    fn ctx(&self) -> &Ctx {
        &self.env.ctx
    }

    pub fn issue(&mut self) -> Result<u64> {
        let n = self.issued;
        if n >= u64::try_from(MAX_SAFE_INT).unwrap_or(u64::MAX) {
            return Err(Error::new("doc.id.exhausted"));
        }
        self.issued += 1;
        Ok(n)
    }

    pub fn path_of(&self, id: NodeId) -> Result<Vec<PathStep>> {
        self.ctx().path_of(&self.state.index, &self.state.root, id)
    }

    pub fn type_of(&self, id: NodeId) -> Result<&'static str> {
        self.ctx().type_of(&self.state.index, id)
    }

    pub fn find(&self, id: NodeId) -> Option<&Value> {
        let path = self.path_of(id).ok()?;
        self.state.root.at_path(&path)
    }

    fn record_touch(&mut self, owner: NodeId, rel: &[PathStep], old: &Option<Value>) {
        if let Some(g) = &mut self.gesture {
            let key = (owner, RelPath::new(rel));
            if !g.touched.contains_key(&key) {
                g.touched.insert(key.clone(), old.clone());
                g.order.push_back(key);
            }
            g.targets.insert(owner);
        }
    }

    /// Replaces (or removes, with `None`) the value at `rel` inside the node `owner`. `rel` never ends at an item
    /// of a list of nodes (that is [`Work::list_insert`] and [`Work::list_remove`]), nor removes an item of any
    /// list. Gives back the ids that were in the old value and are not in the new one
    pub fn replace(
        &mut self,
        owner: NodeId,
        owner_type: &'static str,
        rel: &[PathStep],
        slot: &Slot,
        new: Option<Value>,
    ) -> Result<Vec<NodeId>> {
        let mut abs = self.path_of(owner)?;
        abs.extend_from_slice(rel);
        let old = self.state.root.at_path(&abs).cloned();
        match (&old, &new) {
            (None, None) => return Ok(Vec::new()),
            (Some(o), Some(n)) if o.same_node(n) => return Ok(Vec::new()),
            _ => {}
        }
        let mut gone = Vec::new();
        if let Slot::Kind(kind) = slot
            && self.ctx().is_deep(kind)
        {
            let env = Arc::clone(&self.env);
            let ctx = &env.ctx;
            let collect = |v: &Option<Value>| match v {
                Some(v) if !v.is_null() => ctx.collect(kind, v, owner, rel),
                _ => Ok(crate::index::Found::default()),
            };
            let before = collect(&old)?;
            let after = collect(&new)?;
            self.state.index.remove_found(&before);
            self.state.index.add_found(&after)?;
            let new_ids: BTreeSet<NodeId> = after.ids.iter().map(|(id, _)| *id).collect();
            let old_ids: BTreeSet<NodeId> = before.ids.iter().map(|(id, _)| *id).collect();
            for (id, loc) in &before.ids {
                if !new_ids.contains(id) {
                    self.acc.removed(*id, loc.type_name);
                    gone.push(*id);
                }
            }
            for (id, loc) in &after.ids {
                if !old_ids.contains(id) {
                    self.acc.created(*id, loc.type_name);
                    if loc.type_name == types::PLUGIN {
                        self.plugins.insert(*id);
                    }
                }
            }
            self.added_refs.extend(after.refs.iter().map(|(t, _)| *t));
        }
        let root = match &new {
            Some(v) => self.state.root.with_path(&abs, v.clone())?,
            None => self.state.root.without_path(&abs)?,
        };
        self.state.root = root;
        self.ops.push(match &new {
            Some(v) => JournalOp::Set { path: abs.clone(), value: v.clone() },
            None => JournalOp::Remove { path: abs.clone() },
        });
        self.inverse.push(match &old {
            Some(v) => JournalOp::Set { path: abs, value: v.clone() },
            None => JournalOp::Remove { path: abs },
        });
        self.record_touch(owner, rel, &old);
        if owner_type == types::PLUGIN
            && matches!(rel.first(), Some(PathStep::Key(k)) if matches!(&**k, "plugin" | "state" | "version"))
        {
            self.plugins.insert(owner);
        }
        match rel.get(1) {
            Some(PathStep::Note(n)) => {
                let event = match (rel.len(), &old, &new) {
                    (2, None, _) => NoteEvent::Created,
                    (2, _, None) => NoteEvent::Removed,
                    _ => NoteEvent::Changed,
                };
                self.acc.note(owner, owner_type, *n, event);
            }
            _ => self.acc.changed(owner, owner_type, rel),
        }
        match (param_key(owner_type, rel), new.as_ref().and_then(Value::as_real)) {
            (Some(key), Some(v)) => self.acc.param(owner, key, v),
            _ => self.acc.structural = true,
        }
        Ok(gone)
    }

    /// Inserts the node `node` (of the list's item kind `item`) at `index` of the list at `container` in `parent`
    pub fn list_insert(
        &mut self,
        parent: NodeId,
        container: &[Arc<str>],
        item: &FieldKind,
        index: usize,
        node: Value,
        moving: bool,
    ) -> Result<()> {
        let parent_type = self.type_of(parent)?;
        let mut abs = self.path_of(parent)?;
        abs.extend(container.iter().map(|k| PathStep::Key(Arc::clone(k))));
        // An optional list that is absent (`project.midiMap`, a note's `bend`) is an empty list: its owner must exist
        let empty = Value::empty_list();
        let list = match self.state.root.at_path(&abs) {
            Some(l) => l,
            None if abs.len() > 1 && self.state.root.at_path(&abs[..abs.len() - 1]).is_some_and(Value::is_object) => {
                &empty
            }
            None if abs.len() == 1 => &empty,
            None => return Err(Error::new("doc.path.not_found")),
        };
        let new_list = list.with_inserted(index, node.clone())?;
        let mut rel: Vec<PathStep> = container.iter().map(|k| PathStep::Key(Arc::clone(k))).collect();
        let list_rel = rel.clone();
        rel.push(PathStep::Index(index));
        let found = self.ctx().collect(item, &node, parent, &rel)?;
        self.state.index.add_found(&found)?;
        self.state.root = self.state.root.with_path(&abs, new_list)?;
        let mut item_path = abs.clone();
        item_path.push(PathStep::Index(index));
        self.ops.push(JournalOp::Insert { path: abs, index, value: node });
        self.inverse.push(JournalOp::Remove { path: item_path });
        if !moving {
            for (id, loc) in &found.ids {
                self.acc.created(*id, loc.type_name);
                if loc.type_name == types::PLUGIN {
                    self.plugins.insert(*id);
                }
            }
        }
        self.added_refs.extend(found.refs.iter().map(|(t, _)| *t));
        self.acc.changed(parent, parent_type, &list_rel);
        self.acc.structural = true;
        Ok(())
    }

    /// Removes the node at `index` of the list at `container` in `parent`, and gives it back
    pub fn list_remove(
        &mut self,
        parent: NodeId,
        container: &[Arc<str>],
        item: &FieldKind,
        index: usize,
        moving: bool,
    ) -> Result<Value> {
        let parent_type = self.type_of(parent)?;
        let mut abs = self.path_of(parent)?;
        abs.extend(container.iter().map(|k| PathStep::Key(Arc::clone(k))));
        let list = self.state.root.at_path(&abs).ok_or_else(|| Error::new("doc.path.not_found"))?;
        let node = list.at(index).cloned().ok_or_else(|| Error::new("doc.path.not_found"))?;
        let new_list = list.without_at(index)?;
        let mut rel: Vec<PathStep> = container.iter().map(|k| PathStep::Key(Arc::clone(k))).collect();
        let list_rel = rel.clone();
        rel.push(PathStep::Index(index));
        let found = self.ctx().collect(item, &node, parent, &rel)?;
        self.state.index.remove_found(&found);
        self.state.root = self.state.root.with_path(&abs, new_list)?;
        let mut item_path = abs.clone();
        item_path.push(PathStep::Index(index));
        self.ops.push(JournalOp::Remove { path: item_path });
        self.inverse.push(JournalOp::Insert { path: abs, index, value: node.clone() });
        if !moving {
            for (id, loc) in &found.ids {
                self.acc.removed(*id, loc.type_name);
            }
        }
        self.acc.changed(parent, parent_type, &list_rel);
        self.acc.structural = true;
        Ok(node)
    }

    /// The ids in the subtree of a node, itself first
    fn subtree_ids(&self, id: NodeId) -> Result<Vec<NodeId>> {
        let loc = self.state.index.loc(id).ok_or_else(|| not_found(id))?.clone();
        let value = self.find(id).cloned().ok_or_else(|| not_found(id))?;
        let found = self.ctx().collect(&FieldKind::Object(loc.type_name), &value, loc.parent, &[PathStep::key("x")])?;
        Ok(found.ids.into_iter().map(|(id, _)| id).collect())
    }

    /// Removes a node, then applies the rules of the references to everything that left with it
    pub fn remove_node(&mut self, id: NodeId) -> Result<()> {
        if id == ROOT {
            return Err(Error::new("doc.remove.root"));
        }
        let loc = self.state.index.loc(id).cloned().ok_or_else(|| not_found(id))?;
        let gone = self.subtree_ids(id)?;
        let inside: BTreeSet<NodeId> = gone.iter().copied().collect();
        for t in &gone {
            for r in self.state.index.referrers(*t) {
                if inside.contains(&r.owner) {
                    continue;
                }
                let owner_type = self.type_of(r.owner)?;
                if let (OnRemove::Refuse, _) = self.ctx().ref_rule(owner_type, r.path.steps())? {
                    return Err(Error::new("doc.remove.refused")
                        .arg("node", t.to_string())
                        .arg("by", r.owner.to_string()));
                }
            }
        }
        if loc.in_list {
            let list = self.find(loc.parent).and_then(|p| p.at_path(&loc.container_path())).cloned();
            let pos = list.as_ref().and_then(|l| Ctx::position(l, id)).ok_or_else(|| not_found(id))?;
            self.list_remove(loc.parent, &loc.container, &FieldKind::Object(loc.type_name), pos, false)?;
        } else {
            let container = loc.container_path();
            let r = self.ctx().resolve(&self.state.index, &self.state.root, loc.parent, &container)?;
            if !r.nullable {
                return Err(Error::new("doc.remove.required").arg("node", id.to_string()));
            }
            self.replace(r.owner, r.owner_type, &r.rel, &r.slot, Some(Value::NULL))?;
        }
        self.process_removed(&gone)
    }

    /// Applies `OnRemove` to every reference still pointing to one of `gone` (R10)
    pub fn process_removed(&mut self, gone: &[NodeId]) -> Result<()> {
        loop {
            let next = gone.iter().find_map(|t| self.state.index.first_referrer(*t).map(|r| (*t, r)));
            let Some((target, r)) = next else { return Ok(()) };
            let owner_type = self.type_of(r.owner)?;
            let (rule, in_list) = self.ctx().ref_rule(owner_type, r.path.steps())?;
            match rule {
                OnRemove::Refuse => {
                    return Err(Error::new("doc.remove.refused")
                        .arg("node", target.to_string())
                        .arg("by", r.owner.to_string()));
                }
                OnRemove::RemoveOwner => {
                    if r.owner == ROOT {
                        return Err(Error::new("doc.remove.root"));
                    }
                    self.remove_node(r.owner)?;
                }
                OnRemove::Clear if in_list => {
                    let steps = r.path.steps();
                    let (Some((PathStep::Index(i), list_rel)), Some(node)) = (steps.split_last(), self.find(r.owner))
                    else {
                        return Err(Error::new("doc.ref.invalid"));
                    };
                    let list = node.at_path(list_rel).ok_or_else(|| Error::new("doc.ref.invalid"))?;
                    let new = list.without_at(*i)?;
                    let res = self.ctx().resolve(&self.state.index, &self.state.root, r.owner, list_rel)?;
                    self.replace(res.owner, res.owner_type, &res.rel, &res.slot, Some(new))?;
                }
                OnRemove::Clear | OnRemove::Redirect(_) => {
                    let value = match rule {
                        OnRemove::Redirect(to) => {
                            if gone.contains(&to) || !self.state.index.contains(to) {
                                return Err(Error::new("doc.remove.redirect_gone").arg("to", to.to_string()));
                            }
                            Value::text(to.to_string())
                        }
                        _ => Value::NULL,
                    };
                    let res = self.ctx().resolve(&self.state.index, &self.state.root, r.owner, r.path.steps())?;
                    self.replace(res.owner, res.owner_type, &res.rel, &res.slot, Some(value))?;
                }
            }
        }
    }

    // ---- the checks at commit ----

    /// Every reference made in this transaction resolves; every plugin changed has a valid state
    pub fn check(&mut self) -> Result<()> {
        for target in std::mem::take(&mut self.added_refs) {
            if self.state.index.refs.contains_key(&target) && !self.state.index.contains(target) {
                return Err(Error::new("doc.ref.dangling").arg("target", target.to_string()));
            }
        }
        for p in std::mem::take(&mut self.plugins) {
            let Some(node) = self.find(p).cloned() else { continue };
            if let Some(label) = check_plugin(&self.env.ctx, p, &node)? {
                self.acc.notice(label);
            }
        }
        Ok(())
    }
}

/// Validates a plugin node's state with its plugin's schema (I-3.3: `state` is `Any` in the node's schema). An
/// unknown plugin is kept as it is, with a notice (arquitetura-nova.md, 7a)
pub(crate) fn check_plugin(ctx: &Ctx, id: NodeId, node: &Value) -> Result<Option<Label>> {
    let name = node.get("plugin").and_then(Value::as_text).unwrap_or("");
    let Some(schema) = ctx.registry.find(&plugin_type(name)) else {
        return Ok(Some(Label::new("doc.plugin.unavailable").arg("node", id.to_string()).arg("plugin", name)));
    };
    let version = node.get("version").and_then(Value::as_int).unwrap_or(0);
    if version != i64::from(schema.version) {
        return Err(Error::new("doc.plugin.version")
            .arg("node", id.to_string())
            .arg("found", version)
            .arg("version", i64::from(schema.version)));
    }
    let state = node.get("state").cloned().unwrap_or_default();
    ctx.registry.validate(schema.type_name, &state).map_err(|e| e.arg("node", id.to_string()))?;
    Ok(None)
}

/// The slot key of a parameter written at `rel` in a node of `owner_type` (M5): `"vol1"` for a plugin's
/// `state/params/vol1`, `"volume"`, `"pan"`, `"rack.tube.drive"` for the desk, a send's `"amount"`
pub fn param_key(owner_type: &str, rel: &[PathStep]) -> Option<Arc<str>> {
    let keys: Option<Vec<&str>> = rel
        .iter()
        .map(|s| match s {
            PathStep::Key(k) => Some(&**k),
            _ => None,
        })
        .collect();
    let keys = keys?;
    match (owner_type, keys.as_slice()) {
        (types::PLUGIN, ["state", "params", k]) => Some(Arc::from(*k)),
        // An insert's wet/dry mix (the legacy EffectView's W/D)
        (types::PLUGIN, ["wet"]) => Some(Arc::from("insert.wet")),
        // A modulator's own parameters: `params/speed`, `params/attack`...
        (types::MODULATOR, ["params", k]) => Some(Arc::from(*k)),
        (types::TRACK, ["volume"])
        | (types::CHANNEL, ["volume" | "pan"])
        | (types::MASTER, ["volume"])
        | (types::SEND, ["amount"]) => Some(Arc::from(keys[0])),
        // The CC rack (F-IJN-025): "cc.7"
        (types::TRACK, ["cc", c])
            if c.strip_prefix('c').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| n < 128) =>
        {
            Some(Arc::from(format!("cc.{}", &c[1..])))
        }
        (types::CHANNEL | types::MASTER, ["rack", section, field]) => {
            Some(Arc::from(format!("rack.{section}.{field}")))
        }
        _ => None,
    }
}

/// The path of a parameter in a node of `owner_type` (the inverse of [`param_key`])
pub fn param_path(owner_type: &str, key: &str) -> Vec<PathStep> {
    // The CC rack: "cc.7" lives at `cc.c7`
    if owner_type == types::TRACK
        && let Some(n) = key.strip_prefix("cc.")
    {
        return vec![PathStep::key("cc"), PathStep::key(&format!("c{n}"))];
    }
    if owner_type == types::MODULATOR {
        return vec![PathStep::key("params"), PathStep::key(key)];
    }
    if owner_type == types::PLUGIN && key == "insert.wet" {
        return vec![PathStep::key("wet")];
    }
    if owner_type == types::PLUGIN {
        return vec![PathStep::key("state"), PathStep::key("params"), PathStep::key(key)];
    }
    key.split('.').map(PathStep::key).collect()
}

fn container_keys(list: &str) -> Vec<Arc<str>> {
    list.split('.').map(Arc::from).collect()
}

fn not_a_node_list(parent: NodeId, list: &str) -> Error {
    Error::new("doc.insert.not_a_node_list").arg("node", parent.to_string()).arg("list", list)
}

impl Edit<'_> {
    fn structural_allowed(&self) -> Result<()> {
        if self.w.gesture_mode && self.w.phase == Phase::User {
            Err(Error::new("doc.gesture.structural"))
        } else {
            Ok(())
        }
    }

    fn ctx(&self) -> &Ctx {
        &self.w.env.ctx
    }

    // ---- reading the working copy ----

    /// The working root, with the changes made so far
    pub fn root(&self) -> &Value {
        &self.w.state.root
    }

    /// A node of the working root
    pub fn find(&self, id: NodeId) -> Option<&Value> {
        self.w.find(id)
    }

    pub fn exists(&self, id: NodeId) -> bool {
        self.w.state.index.contains(id)
    }

    pub fn type_of(&self, id: NodeId) -> Result<&'static str> {
        self.w.type_of(id)
    }

    pub fn loc(&self, id: NodeId) -> Option<&Loc> {
        self.w.state.index.loc(id)
    }

    pub fn path_of(&self, id: NodeId) -> Result<Vec<PathStep>> {
        self.w.path_of(id)
    }

    /// Who points to `id`
    pub fn referrers(&self, id: NodeId) -> Vec<crate::index::RefFrom> {
        self.w.state.index.referrers(id)
    }

    /// The registry of schemas
    pub fn registry(&self) -> &lm_state::SchemaRegistry {
        &self.ctx().registry
    }

    /// Adds a notice to the transaction's diff
    pub fn notice(&mut self, label: Label) {
        self.w.acc.notice(label);
    }

    /// The ports of a plugin, from the callback the plugin layer registered (`None` without one, or for a plugin it
    /// does not know)
    pub fn ports(&self, plugin: &str, state: &Value) -> Option<crate::rules::Ports> {
        self.w.env.ports.as_ref().and_then(|f| f(plugin, state))
    }

    // ---- changing ----

    /// Sets a field at any depth (D8). The path may go through nested nodes (the change is theirs) and into a
    /// plugin's state. Setting a value that holds nodes: those without an id get new ones; ids already there may
    /// stay; the nodes that left are removed with their references' rules. `meta` and ids cannot be set
    pub fn set(&mut self, id: NodeId, path: &[PathStep], v: Value) -> Result<()> {
        if path.is_empty() {
            return Err(Error::new("doc.set.empty_path"));
        }
        let env = Arc::clone(&self.w.env);
        let ctx = &env.ctx;
        let r = match ctx.resolve(&self.w.state.index, &self.w.state.root, id, path) {
            Ok(r) => r,
            // An optional object on the way that is absent (a track's `cc`, I-21): it starts with its defaults
            Err(e) if e.key() == "doc.path.not_found" && path.len() > 1 => {
                let Some(prefix) = self.absent_object_prefix(id, path) else { return Err(e) };
                self.set(id, &prefix, Value::empty_object()).map_err(|_| e)?;
                return self.set(id, path, v);
            }
            Err(e) => return Err(e),
        };
        if r.owner == ROOT && ctx.has_meta && matches!(r.rel.first(), Some(PathStep::Key(k)) if &**k == "meta") {
            return Err(Error::new("doc.meta.read_only"));
        }
        if matches!(r.slot, Slot::Kind(FieldKind::Id(_))) {
            return Err(Error::new("doc.set.id").arg("path", path_text(path)));
        }
        let mut v = v;
        if let Slot::Kind(kind) = &r.slot {
            v = ctx.with_defaults(kind, v)?;
            if ctx.is_deep(kind) && !v.is_null() {
                let mut abs = self.w.path_of(r.owner)?;
                abs.extend_from_slice(&r.rel);
                let old_ids: BTreeSet<NodeId> = match self.w.state.root.at_path(&abs) {
                    Some(old) if !old.is_null() => {
                        ctx.collect(kind, old, r.owner, &r.rel)?.ids.into_iter().map(|(id, _)| id).collect()
                    }
                    _ => BTreeSet::new(),
                };
                let w = &mut *self.w;
                v = ctx
                    .assign_ids(kind, v, &mut || w.issue(), &|id| old_ids.contains(&id))
                    .map_err(|e| if e.key() == "doc.insert.has_id" { Error::new("doc.set.foreign_id") } else { e })?;
            }
            if ctx.max_note(kind, &v).is_some_and(|n| n >= self.w.issued) {
                return Err(Error::new("doc.note.unissued"));
            }
        }
        ctx.check_slot(&r.slot, r.nullable, &v, path)?;
        let gone = self.w.replace(r.owner, r.owner_type, &r.rel, &r.slot, Some(v))?;
        self.w.process_removed(&gone)
    }

    /// The shortest prefix of `path` (not the whole path) that is absent under `id`, when it is a plain object field
    fn absent_object_prefix(&self, id: NodeId, path: &[PathStep]) -> Option<Vec<PathStep>> {
        let node = self.find(id)?;
        for n in 1..path.len() {
            if node.at_path(&path[..n]).is_none() {
                return matches!(path[n - 1], PathStep::Key(_)).then(|| path[..n].to_vec());
            }
        }
        None
    }

    /// Sets a parameter by its key (`"vol1"`, `"rack.tube.drive"`; an old key resolves through its alias, C5)
    pub fn set_param(&mut self, node: NodeId, key: &str, v: f64) -> Result<()> {
        let t = self.type_of(node)?;
        let mut key = key.to_owned();
        if t == types::PLUGIN {
            let plugin = self.find(node).and_then(|n| n.get("plugin")).and_then(Value::as_text).unwrap_or("");
            if let Some(spec) = self.registry().resolve_param(&plugin_type(plugin), &key) {
                key = spec.key.to_owned();
            }
        }
        self.set(node, &param_path(t, &key), Value::real(v)?)
    }

    /// Inserts a new node at `index` of the list `list` of `parent`. Missing fields take their defaults; the node
    /// and every node in it get new ids (a node given with an id is refused: pasting is [`Edit::paste`]); its notes
    /// get new numbers. Gives back its id
    pub fn insert(&mut self, parent: NodeId, list: &str, index: usize, node: Value) -> Result<NodeId> {
        self.structural_allowed()?;
        let env = Arc::clone(&self.w.env);
        let ctx = &env.ctx;
        let (item, type_name) = self.node_list(parent, list)?;
        if !node.is_object() {
            return Err(Error::new("doc.insert.not_object"));
        }
        if node.get("id").is_some() {
            return Err(Error::new("doc.insert.has_id"));
        }
        let full = ctx.with_defaults(&item, node)?;
        let w = &mut *self.w;
        let full = ctx.assign_ids(&item, full, &mut || w.issue(), &|_| false)?;
        let full = ctx.renumber_notes(&item, full, &mut || w.issue())?;
        ctx.registry.validate(type_name, &full)?;
        let id = NodeId::parse(full.get("id").and_then(Value::as_text).unwrap_or(""))?;
        self.insert_ready(parent, list, index, &item, full)?;
        Ok(id)
    }

    /// The item kind and type of a list of nodes. `list` is a field of `parent`, or a dotted path of fields through
    /// objects without ids (`"mixer.channels"` of the root)
    fn node_list(&self, parent: NodeId, list: &str) -> Result<(FieldKind, &'static str)> {
        let mut t = self.type_of(parent)?;
        let keys: Vec<&str> = list.split('.').collect();
        for (i, key) in keys.iter().enumerate() {
            let field =
                self.registry().find(t).and_then(|s| s.find_field(key)).ok_or_else(|| not_a_node_list(parent, list))?;
            match (&field.kind, i + 1 == keys.len()) {
                (FieldKind::Object(next), false) if self.ctx().id_kind(next).is_none() => t = next,
                (FieldKind::List(inner), true) => match &**inner {
                    FieldKind::Object(it) if self.ctx().id_kind(it).is_some() => return Ok(((**inner).clone(), it)),
                    _ => return Err(not_a_node_list(parent, list)),
                },
                _ => return Err(not_a_node_list(parent, list)),
            }
        }
        Err(not_a_node_list(parent, list))
    }

    fn list_len(&self, parent: NodeId, list: &str) -> usize {
        let path: Vec<PathStep> = list.split('.').map(PathStep::key).collect();
        self.find(parent).and_then(|p| p.at_path(&path)).map_or(0, Value::len)
    }

    fn insert_ready(&mut self, parent: NodeId, list: &str, index: usize, item: &FieldKind, node: Value) -> Result<()> {
        let len = self.list_len(parent, list);
        if index > len {
            return Err(Error::new("doc.insert.index").arg("index", index as i64).arg("len", len as i64));
        }
        self.w.list_insert(parent, &container_keys(list), item, index, node, false)
    }

    /// Removes a node and everything in it, applying the rules of every reference to what left (R10): an output
    /// whose channel is removed goes to the master; a send whose destination is removed goes too; a removal that a
    /// `Refuse` reference forbids fails the transaction
    pub fn remove(&mut self, id: NodeId) -> Result<()> {
        self.structural_allowed()?;
        self.w.remove_node(id)
    }

    /// Moves a node to `index` of the list `list` of `parent`: `index` is its position **after** the move (R11)
    pub fn move_to(&mut self, id: NodeId, parent: NodeId, list: &str, index: usize) -> Result<()> {
        let loc = self.loc(id).cloned().ok_or_else(|| not_found(id))?;
        if !loc.in_list {
            return Err(Error::new("doc.move.not_in_list").arg("node", id.to_string()));
        }
        let (item, type_name) = self.node_list(parent, list)?;
        if type_name != loc.type_name {
            return Err(Error::new("doc.move.wrong_type").arg("node", id.to_string()).arg("list", list));
        }
        let mut at = parent;
        while at != ROOT {
            if at == id {
                return Err(Error::new("doc.move.into_itself").arg("node", id.to_string()));
            }
            at = self.loc(at).map(|l| l.parent).ok_or_else(|| not_found(at))?;
        }
        let list_now = self.find(loc.parent).and_then(|p| p.at_path(&loc.container_path())).cloned();
        let pos = list_now.as_ref().and_then(|l| Ctx::position(l, id)).ok_or_else(|| not_found(id))?;
        let same_list = loc.parent == parent && *loc.container == *container_keys(list);
        if same_list && pos == index {
            return Ok(());
        }
        if let Some(g) = &mut self.w.gesture {
            if !g.moves.contains_key(&id) {
                g.moves.insert(id, (loc.parent, Arc::clone(&loc.container), pos));
                g.move_order.push_back(id);
            }
            g.targets.insert(id);
        }
        let node = self.w.list_remove(loc.parent, &loc.container, &item, pos, true)?;
        let len = self.list_len(parent, list);
        if index > len {
            return Err(Error::new("doc.move.index").arg("index", index as i64).arg("len", len as i64));
        }
        self.w.list_insert(parent, &container_keys(list), &item, index, node, true)
    }

    /// Adds a note to an item (C1); missing fields take their defaults. Gives back its number, from the global
    /// counter (S12)
    pub fn add_note(&mut self, item: NodeId, note: Value) -> Result<u64> {
        self.structural_allowed()?;
        let env = Arc::clone(&self.w.env);
        let ctx = &env.ctx;
        let r = ctx.resolve(&self.w.state.index, &self.w.state.root, item, &[PathStep::key("notes")])?;
        let Slot::Kind(FieldKind::NoteMap(t)) = r.slot else {
            return Err(Error::new("doc.note.not_an_item").arg("node", item.to_string()));
        };
        if !note.is_object() {
            return Err(Error::new("doc.note.not_object"));
        }
        let full = ctx.with_defaults(&FieldKind::Object(t), note)?;
        ctx.registry.validate(t, &full)?;
        let n = self.w.issue()?;
        let rel = [PathStep::key("notes"), PathStep::Note(n)];
        self.w.replace(item, r.owner_type, &rel, &Slot::Kind(FieldKind::Object(t)), Some(full))?;
        Ok(n)
    }

    fn note_exists(&self, item: NodeId, n: u64) -> Result<()> {
        match self.find(item).and_then(|i| i.get("notes")).and_then(|m| m.note(n)) {
            Some(_) => Ok(()),
            None => Err(Error::new("doc.note.not_found").arg("node", item.to_string()).arg("n", n as i64)),
        }
    }

    /// Sets a field of a note (T12)
    pub fn set_note(&mut self, item: NodeId, n: u64, path: &[PathStep], v: Value) -> Result<()> {
        self.note_exists(item, n)?;
        let mut full = vec![PathStep::key("notes"), PathStep::Note(n)];
        full.extend_from_slice(path);
        self.set(item, &full, v)
    }

    pub fn remove_note(&mut self, item: NodeId, n: u64) -> Result<()> {
        self.structural_allowed()?;
        self.note_exists(item, n)?;
        let t = self.type_of(item)?;
        let rel = [PathStep::key("notes"), PathStep::Note(n)];
        self.w.replace(item, t, &rel, &Slot::Any, None)?;
        Ok(())
    }

    /// Pastes a copied node at `index` of the list `list` of `parent` (H13): every node in it gets a new id and
    /// every note a new number; references inside it follow the new ids; a reference to outside it is kept when the
    /// clip came from this same project (`meta.uuid`) and its target still exists, and otherwise gets the rule of
    /// its field (`OnRemove`). `None` when that rule removed the pasted node itself
    pub fn paste(&mut self, parent: NodeId, list: &str, index: usize, clip: &Clip) -> Result<Option<NodeId>> {
        self.structural_allowed()?;
        let env = Arc::clone(&self.w.env);
        let ctx = &env.ctx;
        let (item, type_name) = self.node_list(parent, list)?;
        if clip.type_name != type_name {
            return Err(Error::new("doc.paste.wrong_type").arg("expected", type_name).arg("found", clip.type_name));
        }
        let uuid = self.root().get("meta").and_then(|m| m.get("uuid")).and_then(Value::as_text).unwrap_or("");
        let same_project = !uuid.is_empty() && *clip.source_uuid == *uuid;
        let mut node = clip.node.clone();
        // The references that leave the clip, until every one is kept or ruled
        loop {
            let mut inside = BTreeSet::new();
            let mut refs = Vec::new();
            ctx.walk(&item, &node, ROOT, &mut Vec::new(), &mut |e| {
                match e {
                    Event::Node { id, .. } => {
                        inside.insert(id);
                    }
                    Event::Ref { target, abs, owner_abs, on_remove, in_list, .. } => {
                        refs.push((target, abs.to_vec(), owner_abs, on_remove, in_list));
                    }
                }
                Ok(())
            })?;
            let outside = refs.into_iter().find(|(target, _, _, rule, _)| {
                let exists = self.w.state.index.contains(*target);
                let redirected = matches!(rule, OnRemove::Redirect(to) if to == target);
                !inside.contains(target) && !(exists && (same_project || redirected))
            });
            let Some((target, abs, owner_abs, rule, in_list)) = outside else { break };
            node = match rule {
                OnRemove::Refuse => {
                    return Err(Error::new("doc.paste.refused").arg("target", target.to_string()));
                }
                OnRemove::RemoveOwner if owner_abs == 0 => return Ok(None),
                OnRemove::RemoveOwner => {
                    let owner = &abs[..owner_abs];
                    match owner.last() {
                        Some(PathStep::Index(_)) => node.without_path(owner)?,
                        _ => node.with_path(owner, Value::NULL)?,
                    }
                }
                OnRemove::Clear if in_list => node.without_path(&abs)?,
                OnRemove::Clear => node.with_path(&abs, Value::NULL)?,
                OnRemove::Redirect(to) if self.w.state.index.contains(to) => {
                    node.with_path(&abs, Value::text(to.to_string()))?
                }
                OnRemove::Redirect(to) => {
                    return Err(Error::new("doc.remove.redirect_gone").arg("to", to.to_string()));
                }
            };
        }
        // New ids, and the references inside follow them
        let mut ids = Vec::new();
        let mut refs = Vec::new();
        ctx.walk(&item, &node, ROOT, &mut Vec::new(), &mut |e| {
            match e {
                Event::Node { id, abs, .. } => ids.push((id, abs.to_vec())),
                Event::Ref { target, abs, .. } => refs.push((target, abs.to_vec())),
            }
            Ok(())
        })?;
        let mut map = std::collections::BTreeMap::new();
        for (old, path) in &ids {
            let new = NodeId { kind: old.kind, number: self.w.issue()? };
            map.insert(*old, new);
            let mut p = path.clone();
            p.push(PathStep::key("id"));
            node = node.with_path(&p, Value::text(new.to_string()))?;
        }
        for (target, path) in &refs {
            if let Some(new) = map.get(target) {
                node = node.with_path(path, Value::text(new.to_string()))?;
            }
        }
        let w = &mut *self.w;
        let node = ctx.renumber_notes(&item, node, &mut || w.issue())?;
        ctx.registry.validate(type_name, &node)?;
        let id = NodeId::parse(node.get("id").and_then(Value::as_text).unwrap_or(""))?;
        self.insert_ready(parent, list, index, &item, node)?;
        Ok(Some(id))
    }

    /// Applies one op of a heavy edit
    pub fn apply(&mut self, op: &PatchOp) -> Result<()> {
        match op {
            PatchOp::Set { node, path, value } => self.set(*node, path, value.clone()),
            PatchOp::Insert { parent, list, index, node } => {
                self.insert(*parent, list, *index, node.clone()).map(|_| ())
            }
            PatchOp::Remove { node } => self.remove(*node),
            PatchOp::Move { node, parent, list, index } => self.move_to(*node, *parent, list, *index),
            PatchOp::AddNote { item, note } => self.add_note(*item, note.clone()).map(|_| ()),
            PatchOp::SetNote { item, n, path, value } => self.set_note(*item, *n, path, value.clone()),
            PatchOp::RemoveNote { item, n } => self.remove_note(*item, *n),
        }
    }

    // ---- for the gesture cancel ----

    /// Puts back the values a gesture found, on the paths it touched (G1), and the nodes it moved
    pub(crate) fn restore(&mut self, record: &GestureRecord) -> Result<()> {
        let env = Arc::clone(&self.w.env);
        let ctx = &env.ctx;
        for key in record.order.iter().rev() {
            let (owner, rel) = key;
            if !self.exists(*owner) {
                continue;
            }
            let before = record.touched.get(key).cloned().flatten();
            let Ok(r) = ctx.resolve(&self.w.state.index, &self.w.state.root, *owner, rel.steps()) else { continue };
            if r.owner != *owner {
                continue;
            }
            let gone = self.w.replace(r.owner, r.owner_type, &r.rel, &r.slot, before)?;
            self.w.process_removed(&gone)?;
        }
        for id in record.move_order.iter().rev() {
            let Some((parent, container, index)) = record.moves.get(id) else { continue };
            if !self.exists(*id) || !self.exists(*parent) {
                continue;
            }
            let list = container.iter().map(|k| &**k).collect::<Vec<_>>().join(".");
            let len = self.list_len(*parent, &list);
            let here = self.loc(*id).is_some_and(|l| l.parent == *parent && l.container == *container);
            let max = if here { len.saturating_sub(1) } else { len };
            self.move_to(*id, *parent, &list, (*index).min(max))?;
        }
        Ok(())
    }
}
