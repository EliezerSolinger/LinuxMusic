//! The MIDI map compiled into the fixed table of an `Install` (interfaces.md, I-21.2): sources resolved from port
//! aliases to port indices, targets that follow the selection or a bank resolved to nodes, parameters to their
//! slots and ranges. Cheap (no program is built): following the selection and changing banks send a map-only
//! install.

use std::sync::Arc;

use lm_doc::schemas::MASTER;
use lm_midimap::{ANY_PORT, Entry, MapTarget as DocTarget, Mapping, Message, Mode, NodeRef, SelRole, Source, Takeover};
use lm_render::{MapTarget, MidiMap, ParamSlot, PortIndex, ProcessSpec};
use lm_state::{NodeId, Value};

use crate::read::id_of;
use crate::types::InstalledTable;

/// A port the machine knows by an alias (I-21.1)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortAlias {
    pub alias: Arc<str>,
    pub index: PortIndex,
    /// The port's role includes "controller" (an alias-less source listens to these)
    pub controller: bool,
}

/// What the selection-following targets follow (from the interface document, I-3.8), and the banks
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub track: Option<NodeId>,
    pub channel: Option<NodeId>,
    pub plugin: Option<NodeId>,
    /// The first track and channel of a control surface's bank (an index in the project's order)
    pub track_bank: u32,
    pub channel_bank: u32,
}

#[derive(Clone, Debug, Default)]
pub struct MapInput {
    /// The project's mappings first, then the machine's (from `project_count` on); a machine mapping on a source a
    /// project mapping uses is left out (the project wins, I-21.2). One source may drive several targets
    pub mappings: Vec<Mapping>,
    pub project_count: usize,
    pub ports: Vec<PortAlias>,
    /// How many port indices exist (14-bit and NRPN state per port)
    pub port_count: u16,
    pub selection: Selection,
}

/// A mapping as compiled: where it listens and what it reaches now
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// The index in `MapInput::mappings`
    pub mapping: usize,
    /// The mapping's id (its node number; the renderer's events carry it)
    pub id: u64,
    pub port: Option<PortIndex>,
    /// The node the target resolved to (none: an action, or nothing selected)
    pub node: Option<NodeId>,
    /// The parameter's slots (a parameter target with slots: driven in the renderer)
    pub slots: Vec<ParamSlot>,
}

#[derive(Debug)]
pub struct CompiledMap {
    pub map: Box<MidiMap>,
    pub resolved: Vec<Resolved>,
    /// Mappings that do nothing now (their port alias has no port, their target is not there), by index
    pub inactive: Vec<usize>,
}

/// The id the renderer uses for a mapping: its node number, or `1 << 63 | index` for one without an id
pub fn mapping_id(m: &Mapping, index: usize) -> u64 {
    m.id.map_or((1 << 63) | index as u64, |id| id.number)
}

/// Resolves a target's node: a fixed id, or what the selection or the bank points at now
pub fn resolve_node(root: &Value, node: &NodeRef, sel: &Selection) -> Option<NodeId> {
    let list = |path: &[&str]| -> Vec<NodeId> {
        let mut v = root;
        for k in path {
            match v.get(k) {
                Some(x) => v = x,
                None => return Vec::new(),
            }
        }
        v.items().filter_map(id_of).collect()
    };
    let nth = |ids: Vec<NodeId>, base: Option<usize>, offset: u32| -> Option<NodeId> {
        ids.get(base? + offset as usize).copied()
    };
    match node {
        NodeRef::Id(id) => Some(*id),
        NodeRef::Selected { role, offset } => match role {
            SelRole::Track => {
                let ids = list(&["tracks"]);
                let base = sel.track.and_then(|t| ids.iter().position(|i| *i == t));
                nth(ids, base, *offset)
            }
            SelRole::TrackBank => nth(list(&["tracks"]), Some(sel.track_bank as usize), *offset),
            SelRole::Channel => {
                let ids = list(&["mixer", "channels"]);
                if sel.channel == Some(MASTER) && *offset == 0 {
                    return Some(MASTER);
                }
                let base = sel.channel.and_then(|c| ids.iter().position(|i| *i == c));
                nth(ids, base, *offset)
            }
            SelRole::ChannelBank => nth(list(&["mixer", "channels"]), Some(sel.channel_bank as usize), *offset),
            SelRole::Plugin => sel.plugin.filter(|_| *offset == 0),
        },
    }
}

/// A node of the tree by id, through the document's index (O(log n); the session passes `Document::find`)
pub type NodeLookup<'a> = &'a dyn Fn(NodeId) -> Option<Value>;

/// The renderer's MIDI map for `input` against the tree and the slots of the last install. `lookup` finds a node by
/// id through the document's index: the cost is linear in the mappings, never in the tree (a thousand mappings
/// compile in milliseconds, I-21.2: the map follows the selection on the main thread)
pub fn compile_map(
    root: &Value,
    lookup: NodeLookup<'_>,
    table: &InstalledTable,
    input: &MapInput,
    spec: ProcessSpec,
) -> CompiledMap {
    let mut entries: Vec<Entry<MapTarget>> = Vec::new();
    let mut resolved = Vec::new();
    let mut inactive = Vec::new();
    let project = &input.mappings[..input.project_count.min(input.mappings.len())];
    for (i, m) in input.mappings.iter().enumerate() {
        let same_source = |p: &Mapping| {
            p.source.port == m.source.port
                && p.source.channel == m.source.channel
                && p.source.message == m.source.message
        };
        if i >= input.project_count && project.iter().any(same_source) {
            inactive.push(i);
            continue;
        }
        let port = match port_of(&m.source.port, input) {
            Some(p) => p,
            None => {
                inactive.push(i);
                continue;
            }
        };
        let touch_port = m.touch.as_ref().and_then(|t| port_of(&t.port, input).map(|p| (p, t)));
        let id = mapping_id(m, i);
        let node = m.target.node().and_then(|n| resolve_node(root, n, &input.selection));
        if m.target.node().is_some() && node.is_none_or(|n| n != lm_state::ROOT && lookup(n).is_none()) {
            inactive.push(i);
            continue;
        }
        let source = Source { port, channel: m.source.channel, message: m.source.message };
        let touch = touch_port.map(|(p, t)| Source { port: p, channel: t.channel, message: t.message });
        let mut slots = Vec::new();
        let targets: Vec<MapTarget> = match (&m.target, node) {
            (DocTarget::Param { key, .. }, Some(n)) => {
                slots = table.slots_of(n, key).to_vec();
                match table.ranges.get(&(n, Arc::from(key.as_str()))) {
                    Some(range) if !slots.is_empty() => {
                        slots.iter().map(|s| MapTarget::Param { slot: *s, range: *range }).collect()
                    }
                    // A parameter without a slot now (the rack in digital mode): the session writes the document
                    _ => {
                        slots.clear();
                        vec![MapTarget::Session]
                    }
                }
            }
            _ => vec![MapTarget::Session],
        };
        let session = matches!(targets[0], MapTarget::Session);
        let mut transform = m.transform;
        if session {
            // The session applies toggles, steps and values against the document's value
            transform.takeover = Takeover::Jump;
        }
        let switch = match m.source.message {
            Message::Note { .. } | Message::ProgramChange => true,
            Message::Cc { .. } => {
                transform.mode == Mode::Absolute
                    && match &m.target {
                        DocTarget::Action { .. } => true,
                        DocTarget::Field { path, .. } => node
                            .and_then(|n| if n == lm_state::ROOT { Some(root.clone()) } else { lookup(n) })
                            .and_then(|v| v.at_path(path).cloned())
                            .is_some_and(|v| v.as_bool().is_some()),
                        DocTarget::Param { .. } => false,
                    }
            }
            _ => false,
        };
        for target in targets {
            entries.push(Entry { id, source, transform, switch, consume: m.consume, feedback: None, touch, target });
        }
        resolved.push(Resolved { mapping: i, id, port: (port != ANY_PORT).then_some(PortIndex(port)), node, slots });
    }
    CompiledMap { map: MidiMap::new(entries, input.port_count, spec), resolved, inactive }
}

fn port_of(alias: &str, input: &MapInput) -> Option<u16> {
    if alias.is_empty() {
        return Some(ANY_PORT);
    }
    input.ports.iter().find(|p| &*p.alias == alias).map(|p| p.index.0)
}

/// A node of the tree by id, searching the tree (for callers without a document: tests, tools). Linear in the tree:
/// the session uses the document's index instead. Notes and plugin states are not searched: they hold no ids
pub fn find(root: &Value, id: NodeId) -> Option<&Value> {
    if id == lm_state::ROOT {
        return Some(root);
    }
    search(root, id)
}

fn search(v: &Value, id: NodeId) -> Option<&Value> {
    if v.is_object() {
        if id_of(v) == Some(id) {
            return Some(v);
        }
        for (k, child) in v.entries() {
            if k == "notes" || k == "state" {
                continue;
            }
            if let Some(f) = search(child, id) {
                return Some(f);
            }
        }
    } else if v.is_list() {
        for child in v.items() {
            if let Some(f) = search(child, id) {
                return Some(f);
            }
        }
    }
    None
}
