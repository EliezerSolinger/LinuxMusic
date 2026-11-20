//! What changed (I-3.5, I-3.6, T12, T13, S20): the [`Diff`] of a transaction, of a history step, or of everything a
//! [`crate::Feed`] gathered between two drains, always **merged by the final state**: created and removed in between
//! does not appear; removed wins over changed; changed many times appears once, with the union of the paths.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lm_base::Label;
use lm_state::{NodeId, PathStep};

use crate::index::RelPath;

/// Where a change came from
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    User,
    Undo,
    Redo,
    Gesture,
    Plugin,
    Recording,
    Journal,
}

/// The notes of an item that changed, by their number `n` (C1, T12)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteChanges {
    pub created: Vec<u64>,
    pub changed: Vec<u64>,
    pub removed: Vec<u64>,
}

impl NoteChanges {
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
}

/// A node that exists before and after, and what changed in it. The paths are from the node, minimal (no path is
/// listed under another one); the empty path means the whole node. The notes are not in the paths
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    pub node: NodeId,
    pub paths: Vec<Vec<PathStep>>,
    pub notes: NoteChanges,
}

/// What a transaction (or an undo, a redo, a gesture's update, a feed's drain) changed
#[derive(Clone, Debug, PartialEq)]
pub struct Diff {
    /// Nodes with an id that did not exist before and exist now (nested ones included), in id order
    pub created: Vec<NodeId>,
    /// Nodes with an id that existed before and do not now, in id order
    pub removed: Vec<NodeId>,
    /// In id order
    pub changed: Vec<Changed>,
    /// Anything else than parameter values changed: the program must be compiled again
    pub structural: bool,
    /// The parameter values written, by node and key (`"rack.tube.drive"`, `"vol1"`), for the slots of the nodes
    /// already installed (S20). Never for a node created or removed in the same diff
    pub params: Vec<(NodeId, Arc<str>, f64)>,
    pub origin: Origin,
    /// What the user must be told (an output removed by a port reconciliation, I-3.9)
    pub notices: Vec<Label>,
}

impl Diff {
    pub fn empty(origin: Origin) -> Self {
        Self {
            created: Vec::new(),
            removed: Vec::new(),
            changed: Vec::new(),
            structural: false,
            params: Vec::new(),
            origin,
            notices: Vec::new(),
        }
    }

    /// Nothing changed (notices aside)
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.removed.is_empty() && self.changed.is_empty() && self.params.is_empty()
    }

    pub fn changed_node(&self, id: NodeId) -> Option<&Changed> {
        self.changed.binary_search_by_key(&id, |c| c.node).ok().map(|i| &self.changed[i])
    }

    /// The node was created, removed or changed
    pub fn touches(&self, id: NodeId) -> bool {
        self.created.binary_search(&id).is_ok()
            || self.removed.binary_search(&id).is_ok()
            || self.changed_node(id).is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoteEvent {
    Created,
    Changed,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Presence {
    before: bool,
    now: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeAcc {
    pub type_name: &'static str,
    before: bool,
    now: bool,
    /// Removed and created again in between: the whole node changed
    whole: bool,
    paths: BTreeSet<RelPath>,
    notes: BTreeMap<u64, Presence>,
}

impl NodeAcc {
    fn new(type_name: &'static str, before: bool) -> Self {
        Self { type_name, before, now: before, whole: false, paths: BTreeSet::new(), notes: BTreeMap::new() }
    }

    pub fn is_created(&self) -> bool {
        !self.before && self.now
    }

    pub fn is_removed(&self) -> bool {
        self.before && !self.now
    }

    pub fn paths(&self) -> impl Iterator<Item = &[PathStep]> + '_ {
        self.paths.iter().map(RelPath::steps)
    }

    pub fn has_note_changes(&self) -> bool {
        !self.notes.is_empty()
    }
}

/// A diff being gathered, event by event, merged by the final state
#[derive(Clone, Debug, Default)]
pub(crate) struct DiffAcc {
    pub nodes: BTreeMap<NodeId, NodeAcc>,
    params: BTreeMap<(NodeId, Arc<str>), f64>,
    pub structural: bool,
    pub notices: Vec<Label>,
    origin: Option<Origin>,
}

impl DiffAcc {
    fn entry(&mut self, id: NodeId, type_name: &'static str, before: bool) -> &mut NodeAcc {
        self.nodes.entry(id).or_insert_with(|| NodeAcc::new(type_name, before))
    }

    pub fn created(&mut self, id: NodeId, type_name: &'static str) {
        let e = self.entry(id, type_name, false);
        if e.before && !e.now {
            e.whole = true;
        }
        e.now = true;
        self.structural = true;
    }

    pub fn removed(&mut self, id: NodeId, type_name: &'static str) {
        self.entry(id, type_name, true).now = false;
        self.structural = true;
    }

    pub fn changed(&mut self, id: NodeId, type_name: &'static str, path: &[PathStep]) {
        let e = self.entry(id, type_name, true);
        if e.now {
            e.paths.insert(RelPath::new(path));
        }
    }

    pub fn note(&mut self, item: NodeId, type_name: &'static str, n: u64, event: NoteEvent) {
        let e = self.entry(item, type_name, true);
        let p = e.notes.entry(n).or_insert(Presence { before: event != NoteEvent::Created, now: true });
        p.now = event != NoteEvent::Removed;
        self.structural = true;
    }

    pub fn param(&mut self, id: NodeId, key: Arc<str>, v: f64) {
        self.params.insert((id, key), v);
    }

    pub fn notice(&mut self, label: Label) {
        self.notices.push(label);
    }

    /// Adds what happened after everything already here
    pub fn absorb(&mut self, later: &DiffAcc) {
        for (id, o) in &later.nodes {
            match self.nodes.get_mut(id) {
                None => {
                    self.nodes.insert(*id, o.clone());
                }
                Some(e) => {
                    e.whole |= o.whole || (e.before && !o.before);
                    e.now = o.now;
                    e.paths.extend(o.paths.iter().cloned());
                    for (n, p) in &o.notes {
                        match e.notes.get_mut(n) {
                            None => {
                                e.notes.insert(*n, *p);
                            }
                            Some(q) => q.now = p.now,
                        }
                    }
                }
            }
        }
        for (k, v) in &later.params {
            self.params.insert(k.clone(), *v);
        }
        self.structural |= later.structural;
        self.notices.extend(later.notices.iter().cloned());
        if later.origin.is_some() {
            self.origin = later.origin;
        }
    }

    /// A finished diff made an accumulator again (a mirror delivers another document's diff to its own feeds,
    /// I-15): `type_of(id, before)` names each node's type in the state before or after the change
    pub fn from_diff(diff: &Diff, type_of: &dyn Fn(NodeId, bool) -> &'static str) -> DiffAcc {
        let mut acc = DiffAcc { structural: diff.structural, origin: Some(diff.origin), ..DiffAcc::default() };
        for id in &diff.created {
            acc.nodes.insert(*id, NodeAcc { now: true, ..NodeAcc::new(type_of(*id, false), false) });
        }
        for id in &diff.removed {
            acc.nodes.insert(*id, NodeAcc { now: false, ..NodeAcc::new(type_of(*id, true), true) });
        }
        for c in &diff.changed {
            let mut e = NodeAcc::new(type_of(c.node, false), true);
            for p in &c.paths {
                if p.is_empty() {
                    e.whole = true;
                } else {
                    e.paths.insert(RelPath::new(p));
                }
            }
            let n = &c.notes;
            for (list, before, now) in [(&n.created, false, true), (&n.removed, true, false), (&n.changed, true, true)] {
                for k in list {
                    e.notes.insert(*k, Presence { before, now });
                }
            }
            acc.nodes.insert(c.node, e);
        }
        for (id, key, v) in &diff.params {
            acc.params.insert((*id, key.clone()), *v);
        }
        acc.notices.clone_from(&diff.notices);
        acc
    }

    pub fn set_origin(&mut self, origin: Origin) {
        self.origin = Some(origin);
    }

    /// Only what passes `keep` (node and its type)
    pub fn filtered(&self, keep: &dyn Fn(NodeId, &'static str) -> bool) -> DiffAcc {
        let nodes: BTreeMap<NodeId, NodeAcc> =
            self.nodes.iter().filter(|(id, e)| keep(**id, e.type_name)).map(|(k, v)| (*k, v.clone())).collect();
        let params = self
            .params
            .iter()
            .filter(|((id, _), _)| nodes.contains_key(id) || keep(*id, ""))
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        DiffAcc { nodes, params, structural: self.structural, notices: self.notices.clone(), origin: self.origin }
    }

    /// The inverse: what undoing this does. Parameter values come from `before_param`
    pub fn inverted(&self, before_param: &dyn Fn(NodeId, &str) -> Option<f64>) -> DiffAcc {
        let mut out = DiffAcc { structural: self.structural, origin: self.origin, ..DiffAcc::default() };
        for (id, e) in &self.nodes {
            let mut inv = e.clone();
            inv.before = e.now;
            inv.now = e.before;
            for p in inv.notes.values_mut() {
                *p = Presence { before: p.now, now: p.before };
            }
            out.nodes.insert(*id, inv);
        }
        for (id, key) in self.params.keys() {
            if let Some(v) = before_param(*id, key) {
                out.params.insert((*id, key.clone()), v);
            }
        }
        out
    }

    pub fn finish(&self, origin: Origin) -> Diff {
        let mut diff = Diff::empty(self.origin.unwrap_or(origin));
        for (id, e) in &self.nodes {
            match (e.before, e.now) {
                (false, true) => diff.created.push(*id),
                (true, false) => diff.removed.push(*id),
                (true, true) => {
                    let mut notes = NoteChanges::default();
                    for (n, p) in &e.notes {
                        match (p.before, p.now) {
                            (false, true) => notes.created.push(*n),
                            (true, false) => notes.removed.push(*n),
                            (true, true) => notes.changed.push(*n),
                            (false, false) => {}
                        }
                    }
                    let paths = if e.whole { vec![Vec::new()] } else { minimal(&e.paths) };
                    if !paths.is_empty() || !notes.is_empty() {
                        diff.changed.push(Changed { node: *id, paths, notes });
                    }
                }
                (false, false) => {}
            }
        }
        for ((id, key), v) in &self.params {
            let alive = self.nodes.get(id).is_none_or(|e| e.before && e.now);
            if alive {
                diff.params.push((*id, key.clone(), *v));
            }
        }
        diff.structural = self.structural;
        diff.notices.clone_from(&self.notices);
        diff
    }
}

fn is_prefix(prefix: &[PathStep], path: &[PathStep]) -> bool {
    prefix.len() <= path.len() && prefix.iter().zip(path).all(|(a, b)| a == b)
}

/// The paths with none under another (the set is sorted so that a path's extensions follow it)
fn minimal(paths: &BTreeSet<RelPath>) -> Vec<Vec<PathStep>> {
    let mut out: Vec<Vec<PathStep>> = Vec::new();
    for p in paths {
        if out.last().is_some_and(|last| is_prefix(last, p.steps())) {
            continue;
        }
        out.push(p.steps().to_vec());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> NodeId {
        NodeId { kind: b't', number: n }
    }

    #[test]
    fn merge_is_by_final_state() {
        let mut a = DiffAcc::default();
        a.created(id(1), "track");
        a.changed(id(2), "track", &[PathStep::key("name")]);
        a.changed(id(3), "track", &[PathStep::key("volume")]);
        let mut b = DiffAcc::default();
        b.removed(id(1), "track");
        b.removed(id(2), "track");
        b.changed(id(3), "track", &[PathStep::key("mute")]);
        b.changed(id(3), "track", &[PathStep::key("volume")]);
        a.absorb(&b);
        let d = a.finish(Origin::User);
        assert_eq!(d.created, []);
        assert_eq!(d.removed, [id(2)]);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].paths, vec![vec![PathStep::key("mute")], vec![PathStep::key("volume")]]);
    }

    #[test]
    fn removed_then_created_is_a_whole_change_and_paths_are_minimal() {
        let mut a = DiffAcc::default();
        a.removed(id(1), "track");
        let mut b = DiffAcc::default();
        b.created(id(1), "track");
        a.absorb(&b);
        let d = a.finish(Origin::Redo);
        assert_eq!(d.changed[0].paths, vec![Vec::<PathStep>::new()]);

        let mut c = DiffAcc::default();
        c.changed(id(2), "track", &[PathStep::key("a"), PathStep::key("b")]);
        c.changed(id(2), "track", &[PathStep::key("a")]);
        c.changed(id(2), "track", &[PathStep::key("b")]);
        let d = c.finish(Origin::User);
        assert_eq!(d.changed[0].paths, vec![vec![PathStep::key("a")], vec![PathStep::key("b")]]);
    }

    #[test]
    fn notes_merge_like_nodes() {
        let mut a = DiffAcc::default();
        a.note(id(1), "item", 10, NoteEvent::Created);
        a.note(id(1), "item", 11, NoteEvent::Changed);
        a.note(id(1), "item", 12, NoteEvent::Changed);
        let mut b = DiffAcc::default();
        b.note(id(1), "item", 10, NoteEvent::Removed);
        b.note(id(1), "item", 11, NoteEvent::Removed);
        b.note(id(1), "item", 12, NoteEvent::Changed);
        b.note(id(1), "item", 13, NoteEvent::Created);
        a.absorb(&b);
        let d = a.finish(Origin::User);
        let notes = &d.changed[0].notes;
        assert_eq!(notes.created, [13]);
        assert_eq!(notes.changed, [12]);
        assert_eq!(notes.removed, [11]);
    }
}
