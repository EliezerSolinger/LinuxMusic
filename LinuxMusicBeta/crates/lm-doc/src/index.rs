//! The schema-guided walk of the tree and the two persistent indices that travel with every root (I-3.4, S13, R8):
//!
//! - **id → location**: the parent node, the keys from the parent to the list (or to the object field) holding the
//!   node, and whether it is a list. The position in a list is found when needed (lists with ids are short), so
//!   inserting and moving never rewrite the siblings' entries;
//! - **reverse references**: for each id, who points to it (the owner, the nearest enclosing node with an id, and the
//!   path from the owner to the reference).
//!
//! Both are `imbl` maps: a transaction changes only the entries of what it touched, and undo brings them back with
//! the root, in O(1).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use imbl::{OrdMap, OrdSet};
use lm_base::{Error, Result};
use lm_state::{
    FieldKind, INLINE_MAX_BYTES, NodeId, OnRemove, PARAMS_KEY, PathStep, ROOT, SchemaRegistry, Value, parse_note_key,
    path_text,
};

/// A path inside a node, with an order (keys, then indexes, then notes; each by value) so it can key a persistent
/// set
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RelPath(pub Arc<[PathStep]>);

impl RelPath {
    pub fn new(steps: &[PathStep]) -> Self {
        Self(Arc::from(steps))
    }

    pub fn steps(&self) -> &[PathStep] {
        &self.0
    }
}

impl fmt::Debug for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", path_text(&self.0))
    }
}

fn step_cmp(a: &PathStep, b: &PathStep) -> Ordering {
    let rank = |s: &PathStep| match s {
        PathStep::Key(_) => 0,
        PathStep::Index(_) => 1,
        PathStep::Note(_) => 2,
    };
    match (a, b) {
        (PathStep::Key(x), PathStep::Key(y)) => x.cmp(y),
        (PathStep::Index(x), PathStep::Index(y)) => x.cmp(y),
        (PathStep::Note(x), PathStep::Note(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Lexicographic order of paths: a prefix comes before everything that extends it, and every extension of a path
/// comes right after it
pub(crate) fn path_cmp(a: &[PathStep], b: &[PathStep]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        match step_cmp(x, y) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    a.len().cmp(&b.len())
}

impl Ord for RelPath {
    fn cmp(&self, other: &Self) -> Ordering {
        path_cmp(&self.0, &other.0)
    }
}

impl PartialOrd for RelPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Where a node with an id is
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loc {
    /// The nearest enclosing node with an id ([`ROOT`] for the top level)
    pub parent: NodeId,
    /// The keys from the parent to the list (or the object field) that holds the node: `["tracks"]`,
    /// `["mixer", "channels"]`, `["instrument"]`
    pub container: Arc<[Arc<str>]>,
    /// The node is an item of a list (else the value of an object field)
    pub in_list: bool,
    pub type_name: &'static str,
}

impl Loc {
    /// The container as a path from the parent
    pub fn container_path(&self) -> Vec<PathStep> {
        self.container.iter().map(|k| PathStep::Key(Arc::clone(k))).collect()
    }
}

/// A reference, seen from its target: the node that holds it and the path from that node
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RefFrom {
    pub owner: NodeId,
    pub path: RelPath,
}

/// The two indices of a root
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub(crate) ids: OrdMap<NodeId, Loc>,
    pub(crate) refs: OrdMap<NodeId, OrdSet<RefFrom>>,
}

impl Index {
    pub fn loc(&self, id: NodeId) -> Option<&Loc> {
        self.ids.get(&id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        id == ROOT || self.ids.contains_key(&id)
    }

    /// Who points to `id`, in order
    pub fn referrers(&self, id: NodeId) -> Vec<RefFrom> {
        self.refs.get(&id).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn first_referrer(&self, id: NodeId) -> Option<RefFrom> {
        self.refs.get(&id).and_then(|s| s.iter().next().cloned())
    }

    /// Every id, in order
    pub fn ids(&self) -> impl Iterator<Item = (&NodeId, &Loc)> + '_ {
        self.ids.iter()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub(crate) fn remove_found(&mut self, found: &Found) {
        for (id, _) in &found.ids {
            self.ids.remove(id);
        }
        for (target, from) in &found.refs {
            if let Some(set) = self.refs.get(target) {
                let set = set.without(from);
                if set.is_empty() {
                    self.refs.remove(target);
                } else {
                    self.refs.insert(*target, set);
                }
            }
        }
    }

    pub(crate) fn add_found(&mut self, found: &Found) -> Result<()> {
        for (id, loc) in &found.ids {
            if *id == ROOT || self.ids.insert(*id, loc.clone()).is_some() {
                return Err(Error::new("doc.id.duplicate").arg("id", id.to_string()));
            }
        }
        for (target, from) in &found.refs {
            let set = self.refs.get(target).cloned().unwrap_or_default().update(from.clone());
            self.refs.insert(*target, set);
        }
        Ok(())
    }
}

/// What a walk found: ids with their locations, references with their targets
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub ids: Vec<(NodeId, Loc)>,
    pub refs: Vec<(NodeId, RefFrom)>,
}

/// What a walk sees
pub(crate) enum Event<'a> {
    /// A node with an id. `parent` is the enclosing node with an id; `rel` the path from it
    Node { id: NodeId, type_name: &'static str, parent: NodeId, rel: &'a [PathStep], abs: &'a [PathStep] },
    /// A non-null reference. `owner` is the nearest enclosing node with an id; `rel` the path from it; `abs` the path
    /// from where the walk started, `owner_abs` the length of the owner's part of it
    Ref { target: NodeId, owner: NodeId, rel: &'a [PathStep], abs: &'a [PathStep], owner_abs: usize, on_remove: OnRemove, in_list: bool },
}

/// What a path in a node leads to
#[derive(Clone, Debug)]
pub(crate) enum Slot {
    Kind(FieldKind),
    /// Inside an `Any` (a plugin's state) or a `params` object
    Any,
}

/// A path resolved against the schemas: rebased on the deepest node with an id it goes through
#[derive(Debug)]
pub(crate) struct Resolved {
    pub owner: NodeId,
    pub owner_type: &'static str,
    pub rel: Vec<PathStep>,
    pub slot: Slot,
    pub nullable: bool,
}

/// The registry and what is worked out from it once
pub(crate) struct Ctx {
    pub registry: Arc<SchemaRegistry>,
    pub root_type: &'static str,
    pub has_meta: bool,
    /// Types holding an id field, with its kind letter
    id_kind: BTreeMap<&'static str, u8>,
    /// Types that hold (at any depth) an id or a reference
    deep: BTreeMap<&'static str, bool>,
    /// Types that hold (at any depth) a note map
    notes: BTreeMap<&'static str, bool>,
}

impl fmt::Debug for Ctx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ctx").field("root_type", &self.root_type).finish_non_exhaustive()
    }
}

fn fixpoint(registry: &SchemaRegistry, base: impl Fn(&FieldKind) -> bool) -> BTreeMap<&'static str, bool> {
    let mut flags: BTreeMap<&'static str, bool> = registry.schemas().map(|s| (s.type_name, false)).collect();
    fn has(kind: &FieldKind, flags: &BTreeMap<&'static str, bool>, base: &dyn Fn(&FieldKind) -> bool) -> bool {
        if base(kind) {
            return true;
        }
        match kind {
            FieldKind::List(inner) => has(inner, flags, base),
            FieldKind::Object(t) | FieldKind::NoteMap(t) => flags.get(t).copied().unwrap_or(false),
            _ => false,
        }
    }
    loop {
        let mut changed = false;
        for schema in registry.schemas() {
            if flags.get(schema.type_name) == Some(&true) {
                continue;
            }
            if schema.fields.iter().any(|f| has(&f.kind, &flags, &base)) {
                flags.insert(schema.type_name, true);
                changed = true;
            }
        }
        if !changed {
            return flags;
        }
    }
}

pub(crate) fn not_found(id: NodeId) -> Error {
    Error::new("doc.node.not_found").arg("id", id.to_string())
}

impl Ctx {
    pub fn new(registry: Arc<SchemaRegistry>, root_type: &str) -> Result<Self> {
        let schema = registry
            .find(root_type)
            .ok_or_else(|| Error::new("doc.root.unknown_type").arg("type", root_type.to_owned()))?;
        let root_type = schema.type_name;
        let has_meta = schema.find_field("meta").is_some();
        let id_kind = registry
            .schemas()
            .filter_map(|s| {
                s.fields.iter().find_map(|f| match f.kind {
                    FieldKind::Id(k) => Some((s.type_name, k)),
                    _ => None,
                })
            })
            .collect();
        let deep = fixpoint(&registry, |k: &FieldKind| matches!(k, FieldKind::Ref { .. } | FieldKind::Id(_)));
        let notes = fixpoint(&registry, |k: &FieldKind| matches!(k, FieldKind::NoteMap(_)));
        Ok(Self { registry, root_type, has_meta, id_kind, deep, notes })
    }

    pub fn id_kind(&self, type_name: &str) -> Option<u8> {
        self.id_kind.get(type_name).copied()
    }

    pub fn is_deep(&self, kind: &FieldKind) -> bool {
        match kind {
            FieldKind::Ref { .. } | FieldKind::Id(_) => true,
            FieldKind::List(inner) => self.is_deep(inner),
            FieldKind::Object(t) | FieldKind::NoteMap(t) => self.deep.get(t).copied().unwrap_or(false),
            _ => false,
        }
    }

    pub fn has_notes(&self, kind: &FieldKind) -> bool {
        match kind {
            FieldKind::NoteMap(_) => true,
            FieldKind::List(inner) => self.has_notes(inner),
            FieldKind::Object(t) => self.notes.get(t).copied().unwrap_or(false),
            _ => false,
        }
    }

    // ---- the walk ----

    /// Walks the parts of `v` (of `kind`) that can hold ids or references. `path` is where `v` is, from `owner`
    pub fn walk(
        &self,
        kind: &FieldKind,
        v: &Value,
        owner: NodeId,
        path: &mut Vec<PathStep>,
        visit: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut owners = vec![(owner, 0usize)];
        self.walk_inner(kind, v, path, &mut owners, false, visit)
    }

    fn walk_inner(
        &self,
        kind: &FieldKind,
        v: &Value,
        path: &mut Vec<PathStep>,
        owners: &mut Vec<(NodeId, usize)>,
        in_list: bool,
        visit: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        if !self.is_deep(kind) || v.is_null() {
            return Ok(());
        }
        match kind {
            FieldKind::Ref { on_remove, .. } => {
                let Some(text) = v.as_text() else {
                    return Err(Error::new("doc.ref.invalid").arg("path", path_text(path)));
                };
                let target = NodeId::parse(text)?;
                let (owner, depth) = owners.last().copied().unwrap_or((ROOT, 0));
                visit(Event::Ref {
                    target,
                    owner,
                    rel: &path[depth..],
                    abs: path,
                    owner_abs: depth,
                    on_remove: *on_remove,
                    in_list,
                })
            }
            FieldKind::List(inner) => {
                for (i, item) in v.items().enumerate() {
                    path.push(PathStep::Index(i));
                    self.walk_inner(inner, item, path, owners, true, visit)?;
                    path.pop();
                }
                Ok(())
            }
            FieldKind::NoteMap(t) => {
                let inner = FieldKind::Object(t);
                for (key, note) in v.entries() {
                    let n = parse_note_key(key).ok_or_else(|| Error::new("doc.note.key").arg("path", path_text(path)))?;
                    path.push(PathStep::Note(n));
                    self.walk_inner(&inner, note, path, owners, false, visit)?;
                    path.pop();
                }
                Ok(())
            }
            FieldKind::Object(t) => {
                let schema = self
                    .registry
                    .find(t)
                    .ok_or_else(|| Error::new("doc.schema.unknown_type").arg("type", (*t).to_owned()))?;
                let pushed = if self.id_kind(t).is_some() {
                    let id = v
                        .get("id")
                        .and_then(Value::as_text)
                        .ok_or_else(|| Error::new("doc.id.missing").arg("path", path_text(path)))?;
                    let id = NodeId::parse(id)?;
                    let (parent, depth) = owners.last().copied().unwrap_or((ROOT, 0));
                    visit(Event::Node { id, type_name: schema.type_name, parent, rel: &path[depth..], abs: path })?;
                    owners.push((id, path.len()));
                    true
                } else {
                    false
                };
                for field in &schema.fields {
                    if let (Some(child), true) = (v.get(field.key), self.is_deep(&field.kind)) {
                        path.push(PathStep::key(field.key));
                        self.walk_inner(&field.kind, child, path, owners, false, visit)?;
                        path.pop();
                    }
                }
                if pushed {
                    owners.pop();
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The ids and references in `v`, at `rel` from `owner`, for the index
    pub fn collect(&self, kind: &FieldKind, v: &Value, owner: NodeId, rel: &[PathStep]) -> Result<Found> {
        let mut found = Found::default();
        let mut path = rel.to_vec();
        self.walk(kind, v, owner, &mut path, &mut |event| {
            match event {
                Event::Node { id, type_name, parent, rel, .. } => {
                    let (keys, in_list) = match rel.split_last() {
                        Some((PathStep::Index(_), keys)) => (keys, true),
                        _ => (rel, false),
                    };
                    let mut container = Vec::with_capacity(keys.len());
                    for step in keys {
                        match step {
                            PathStep::Key(k) => container.push(Arc::clone(k)),
                            _ => {
                                return Err(Error::new("doc.schema.unsupported_nesting")
                                    .arg("type", type_name)
                                    .arg("path", path_text(rel)));
                            }
                        }
                    }
                    found.ids.push((id, Loc { parent, container: Arc::from(container), in_list, type_name }));
                }
                Event::Ref { target, owner, rel, .. } => {
                    found.refs.push((target, RefFrom { owner, path: RelPath::new(rel) }));
                }
            }
            Ok(())
        })?;
        Ok(found)
    }

    /// The indices of a whole root, from scratch
    pub fn rebuild(&self, root: &Value) -> Result<Index> {
        let found = self.collect(&FieldKind::Object(self.root_type), root, ROOT, &[])?;
        let mut index = Index::default();
        index.add_found(&found)?;
        Ok(index)
    }

    // ---- locating ----

    pub fn type_of(&self, index: &Index, id: NodeId) -> Result<&'static str> {
        if id == ROOT {
            return Ok(self.root_type);
        }
        index.loc(id).map(|l| l.type_name).ok_or_else(|| not_found(id))
    }

    /// The position of the node `id` in a list
    pub fn position(list: &Value, id: NodeId) -> Option<usize> {
        let text = id.to_string();
        list.items().position(|item| item.get("id").and_then(Value::as_text) == Some(text.as_str()))
    }

    /// The path of a node from the root
    pub fn path_of(&self, index: &Index, root: &Value, id: NodeId) -> Result<Vec<PathStep>> {
        let mut chain = Vec::new();
        let mut at = id;
        while at != ROOT {
            let loc = index.loc(at).ok_or_else(|| not_found(at))?;
            chain.push((at, loc));
            if chain.len() > index.len() {
                return Err(Error::new("doc.index.cycle"));
            }
            at = loc.parent;
        }
        let mut path = Vec::new();
        let mut node = root;
        for (id, loc) in chain.into_iter().rev() {
            for key in loc.container.iter() {
                path.push(PathStep::Key(Arc::clone(key)));
                node = node.get(key).ok_or_else(|| not_found(id))?;
            }
            if loc.in_list {
                let i = Self::position(node, id).ok_or_else(|| not_found(id))?;
                path.push(PathStep::Index(i));
                node = node.at(i).ok_or_else(|| not_found(id))?;
            }
        }
        Ok(path)
    }

    /// Resolves `path` from the node `owner` against the schemas, rebasing it on the deepest node with an id it goes
    /// through (so `set(t:1, ["outputs", 0, "to"])` is a change of `o:N`). Note-map keys become [`PathStep::Note`].
    /// Every step but the last must exist in `root`
    pub fn resolve(&self, index: &Index, root: &Value, owner: NodeId, path: &[PathStep]) -> Result<Resolved> {
        let mut owner = owner;
        let mut owner_type = self.type_of(index, owner)?;
        let base = self.path_of(index, root, owner)?;
        let mut value = root.at_path(&base);
        let mut slot = Slot::Kind(FieldKind::Object(owner_type));
        let mut nullable = false;
        let mut rel: Vec<PathStep> = Vec::with_capacity(path.len());
        let bad = |owner: NodeId, rel: &[PathStep], step: &PathStep| {
            let mut p = rel.to_vec();
            p.push(step.clone());
            Error::new("doc.path.invalid").arg("node", owner.to_string()).arg("path", path_text(&p))
        };
        for (i, step) in path.iter().enumerate() {
            // Entering a nested node with an id: it is the owner of what follows
            if let (Slot::Kind(FieldKind::Object(t)), false) = (&slot, rel.is_empty())
                && self.id_kind(t).is_some()
            {
                let id = value
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_text)
                    .ok_or_else(|| Error::new("doc.path.not_found").arg("path", path_text(&path[..i])))?;
                owner = NodeId::parse(id)?;
                owner_type = t;
                rel.clear();
            }
            let (next, step) = match (&slot, step) {
                (Slot::Kind(FieldKind::Object(t)), PathStep::Key(k)) => {
                    let schema = self.registry.find(t).ok_or_else(|| bad(owner, &rel, step))?;
                    match schema.find_field(k) {
                        Some(field) => {
                            nullable = field.nullable;
                            (Slot::Kind(field.kind.clone()), step.clone())
                        }
                        None if &**k == PARAMS_KEY && !schema.params.is_empty() => {
                            nullable = false;
                            (Slot::Any, step.clone())
                        }
                        None => {
                            return Err(Error::new("doc.path.unknown_field")
                                .arg("type", schema.type_name)
                                .arg("field", k.to_string()));
                        }
                    }
                }
                (Slot::Kind(FieldKind::List(inner)), PathStep::Index(_)) => {
                    nullable = false;
                    (Slot::Kind((**inner).clone()), step.clone())
                }
                (Slot::Kind(FieldKind::NoteMap(t)), PathStep::Note(_)) => {
                    nullable = false;
                    (Slot::Kind(FieldKind::Object(t)), step.clone())
                }
                (Slot::Kind(FieldKind::NoteMap(t)), PathStep::Key(k)) => {
                    let n = parse_note_key(k).ok_or_else(|| bad(owner, &rel, step))?;
                    nullable = false;
                    (Slot::Kind(FieldKind::Object(t)), PathStep::Note(n))
                }
                (Slot::Any | Slot::Kind(FieldKind::Any), _) => {
                    nullable = true;
                    (Slot::Any, step.clone())
                }
                _ => return Err(bad(owner, &rel, step)),
            };
            value = value.and_then(|v| v.step(&step));
            if value.is_none() && i + 1 < path.len() {
                let mut p = rel.clone();
                p.push(step);
                return Err(Error::new("doc.path.not_found").arg("node", owner.to_string()).arg("path", path_text(&p)));
            }
            rel.push(step);
            slot = next;
        }
        Ok(Resolved { owner, owner_type, rel, slot, nullable })
    }

    /// The rule of the reference at `path` in a node of type `owner_type`, and whether it is an item of a list of
    /// references
    pub fn ref_rule(&self, owner_type: &str, path: &[PathStep]) -> Result<(OnRemove, bool)> {
        let mut kind = FieldKind::Object(
            self.registry.find(owner_type).map(|s| s.type_name).ok_or_else(|| Error::new("doc.schema.unknown_type"))?,
        );
        let mut in_list = false;
        for step in path {
            kind = match (&kind, step) {
                (FieldKind::Object(t), PathStep::Key(k)) => self
                    .registry
                    .find(t)
                    .and_then(|s| s.find_field(k))
                    .map(|f| f.kind.clone())
                    .ok_or_else(|| Error::new("doc.path.unknown_field").arg("field", k.to_string()))?,
                (FieldKind::List(inner), PathStep::Index(_)) => (**inner).clone(),
                (FieldKind::NoteMap(t), PathStep::Note(_) | PathStep::Key(_)) => FieldKind::Object(t),
                _ => return Err(Error::new("doc.path.invalid").arg("path", path_text(path))),
            };
            in_list = matches!(step, PathStep::Index(_));
        }
        match kind {
            FieldKind::Ref { on_remove, .. } => Ok((on_remove, in_list)),
            _ => Err(Error::new("doc.ref.not_a_ref").arg("path", path_text(path))),
        }
    }

    // ---- values ----

    /// Checks a value for a slot: the kind and range of a scalar, the whole schema of an object (with what it
    /// nests), the size rule of `Any` (U24)
    pub fn check_slot(&self, slot: &Slot, nullable: bool, v: &Value, path: &[PathStep]) -> Result<()> {
        if v.is_null() && nullable {
            return Ok(());
        }
        match slot {
            Slot::Any => check_any(v, path),
            Slot::Kind(kind) => self.check_kind(kind, v, path),
        }
    }

    fn check_kind(&self, kind: &FieldKind, v: &Value, path: &[PathStep]) -> Result<()> {
        let wrong = |expected: &'static str| {
            Error::new("doc.value.wrong_kind")
                .arg("expected", expected)
                .arg("found", v.kind().name())
                .arg("path", path_text(path))
        };
        let range = || Error::new("doc.value.out_of_range").arg("path", path_text(path));
        match kind {
            FieldKind::Bool => v.as_bool().map(|_| ()).ok_or_else(|| wrong("bool")),
            FieldKind::Int { min, max } => {
                let x = v.as_int().ok_or_else(|| wrong("int"))?;
                if x < *min || x > *max { Err(range().arg("value", x)) } else { Ok(()) }
            }
            FieldKind::Real { min, max } => {
                let x = v.as_real().ok_or_else(|| wrong("real"))?;
                if x < *min || x > *max { Err(range().arg("value", x)) } else { Ok(()) }
            }
            FieldKind::Text => {
                let t = v.as_text().ok_or_else(|| wrong("text"))?;
                if t.len() > INLINE_MAX_BYTES { Err(Error::new("doc.value.text_too_large")) } else { Ok(()) }
            }
            FieldKind::Choice(choices) => {
                let t = v.as_text().ok_or_else(|| wrong("text"))?;
                if choices.contains(&t) {
                    Ok(())
                } else {
                    Err(Error::new("doc.value.not_a_choice").arg("path", path_text(path)).arg("value", t))
                }
            }
            FieldKind::Resource(kind) => {
                let r = v.as_resource().ok_or_else(|| wrong("resource"))?;
                if r.kind() == *kind { Ok(()) } else { Err(wrong("resource")) }
            }
            FieldKind::Ref { to, .. } => {
                let id = NodeId::parse(v.as_text().ok_or_else(|| wrong("ref"))?)?;
                if to.contains(&id.kind) {
                    Ok(())
                } else {
                    Err(Error::new("doc.value.wrong_ref_kind").arg("path", path_text(path)).arg("value", id.to_string()))
                }
            }
            FieldKind::Id(_) => Err(Error::new("doc.set.id").arg("path", path_text(path))),
            FieldKind::List(inner) => {
                if !v.is_list() {
                    return Err(wrong("list"));
                }
                let mut p = path.to_vec();
                for (i, item) in v.items().enumerate() {
                    p.push(PathStep::Index(i));
                    self.check_kind(inner, item, &p)?;
                    p.pop();
                }
                Ok(())
            }
            FieldKind::Object(t) => self.registry.validate_at(t, v, path),
            FieldKind::NoteMap(t) => {
                if !v.is_object() {
                    return Err(wrong("object"));
                }
                let mut p = path.to_vec();
                for (key, note) in v.entries() {
                    p.push(PathStep::key(key));
                    if parse_note_key(key).is_none() {
                        return Err(Error::new("doc.note.key").arg("path", path_text(&p)));
                    }
                    self.registry.validate_at(t, note, &p)?;
                    p.pop();
                }
                Ok(())
            }
            FieldKind::Any => check_any(v, path),
        }
    }

    /// Gives an id to every node of `v` that has none, parents first. A node that already has one is refused unless
    /// `keep` says it may stay
    pub fn assign_ids(
        &self,
        kind: &FieldKind,
        v: Value,
        issue: &mut dyn FnMut() -> Result<u64>,
        keep: &dyn Fn(NodeId) -> bool,
    ) -> Result<Value> {
        if !self.is_deep(kind) || v.is_null() {
            return Ok(v);
        }
        match kind {
            FieldKind::List(inner) => {
                let mut items = Vec::with_capacity(v.len());
                for item in v.items() {
                    items.push(self.assign_ids(inner, item.clone(), issue, keep)?);
                }
                Ok(Value::list(items))
            }
            FieldKind::Object(t) => {
                let Some(schema) = self.registry.find(t) else { return Ok(v) };
                let mut node = v;
                if let Some(letter) = self.id_kind(t) {
                    match node.get("id").and_then(Value::as_text) {
                        Some(text) => {
                            let id = NodeId::parse(text)?;
                            if !keep(id) {
                                return Err(Error::new("doc.insert.has_id").arg("id", id.to_string()));
                            }
                        }
                        None => {
                            let id = NodeId { kind: letter, number: issue()? };
                            node = node.with("id", Value::text(id.to_string()));
                        }
                    }
                }
                for field in &schema.fields {
                    if !matches!(field.kind, FieldKind::Id(_))
                        && self.is_deep(&field.kind)
                        && let Some(child) = node.get(field.key)
                    {
                        let new = self.assign_ids(&field.kind, child.clone(), issue, keep)?;
                        node = node.with(field.key, new);
                    }
                }
                Ok(node)
            }
            _ => Ok(v),
        }
    }

    /// Gives new numbers to every note of `v`, in the order of the old ones (S12: a note number is never given
    /// twice)
    pub fn renumber_notes(&self, kind: &FieldKind, v: Value, issue: &mut dyn FnMut() -> Result<u64>) -> Result<Value> {
        if !self.has_notes(kind) || v.is_null() {
            return Ok(v);
        }
        match kind {
            FieldKind::NoteMap(_) => {
                let mut old: Vec<(u64, Value)> = Vec::with_capacity(v.len());
                for (key, note) in v.entries() {
                    let n = parse_note_key(key).ok_or_else(|| Error::new("doc.note.key"))?;
                    old.push((n, note.clone()));
                }
                old.sort_by_key(|(n, _)| *n);
                let mut map = Vec::with_capacity(old.len());
                for (_, note) in old {
                    map.push((lm_state::note_key(issue()?), note));
                }
                Ok(Value::object(map))
            }
            FieldKind::List(inner) => {
                let mut items = Vec::with_capacity(v.len());
                for item in v.items() {
                    items.push(self.renumber_notes(inner, item.clone(), issue)?);
                }
                Ok(Value::list(items))
            }
            FieldKind::Object(t) => {
                let Some(schema) = self.registry.find(t) else { return Ok(v) };
                let mut node = v;
                for field in &schema.fields {
                    if self.has_notes(&field.kind)
                        && let Some(child) = node.get(field.key)
                    {
                        let new = self.renumber_notes(&field.kind, child.clone(), issue)?;
                        node = node.with(field.key, new);
                    }
                }
                Ok(node)
            }
            _ => Ok(v),
        }
    }

    /// The largest note number in `v`
    pub fn max_note(&self, kind: &FieldKind, v: &Value) -> Option<u64> {
        if !self.has_notes(kind) || v.is_null() {
            return None;
        }
        match kind {
            FieldKind::NoteMap(_) => v.keys().filter_map(parse_note_key).max(),
            FieldKind::List(inner) => v.items().filter_map(|i| self.max_note(inner, i)).max(),
            FieldKind::Object(t) => {
                let schema = self.registry.find(t)?;
                schema.fields.iter().filter_map(|f| v.get(f.key).and_then(|c| self.max_note(&f.kind, c))).max()
            }
            _ => None,
        }
    }
}

fn check_any(v: &Value, path: &[PathStep]) -> Result<()> {
    let mut too_large = false;
    v.walk(&mut |_, x| {
        if x.as_text().is_some_and(|t| t.len() > INLINE_MAX_BYTES) {
            too_large = true;
        }
    });
    if too_large { Err(Error::new("doc.value.text_too_large").arg("path", path_text(path))) } else { Ok(()) }
}
