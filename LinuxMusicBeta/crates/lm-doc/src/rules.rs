//! The domain rules (I-3.5, I-3.9, H12): what one node's schema cannot say. They run at the commit of **every**
//! transaction (and every gesture update, every heavy edit, every paste), whatever made it, and never in the journal
//! replay (the patches carry their results).
//!
//! Two kinds: [`RuleKind::Reconcile`] rules change the transaction (the instrument's ports and the track's outputs,
//! M1); [`RuleKind::Check`] rules refuse it (routing cycles, a folder inside itself, the engine's limits...). The
//! reconciling rules run first. Opening a project runs only the checks.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, PathStep, ROOT, SchemaRegistry, Value};

use crate::edit::{Edit, param_path};
use crate::schemas::{KIND_CHANNEL, KIND_PLUGIN, MASTER, PROJECT, plugin_type, types};

/// An audio port of a plugin: a permanent key (like a parameter's, C5) and its number of channels
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub key: Arc<str>,
    pub channels: u32,
}

impl Port {
    pub fn new(key: &str, channels: u32) -> Self {
        Self { key: Arc::from(key), channels }
    }
}

/// The ports a plugin declares for a state (I-9.1): its outputs, the first being the main one, and the sidechain
/// inputs a send can feed
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ports {
    pub outputs: Vec<Port>,
    pub sidechains: Vec<Port>,
}

/// The plugin layer's `ports(plugin_id, &state)`; `None` for a plugin it does not know (its outputs are then left
/// as they are)
pub type PortsFn = Arc<dyn Fn(&str, &Value) -> Option<Ports> + Send + Sync>;

/// The engine's capacities the document must stay within (H12), from `lm-render`'s `Limits`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_nodes: usize,
    pub max_params: usize,
}

/// What a project needs of the engine
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub nodes: usize,
    pub params: usize,
}

/// How a project's usage is counted; [`default_usage`] unless the session gives the compiler's own count
pub type UsageFn = Arc<dyn Fn(&SchemaRegistry, &Value) -> Usage + Send + Sync>;

/// What a transaction did, for the rules to decide whether they apply
#[derive(Clone, Debug, Default)]
pub struct Touched {
    pub created: Vec<(NodeId, &'static str)>,
    /// With the changed paths (an item whose notes changed has the path `["notes"]`)
    pub changed: Vec<(NodeId, &'static str, Vec<Vec<PathStep>>)>,
    pub removed: Vec<(NodeId, &'static str)>,
}

impl Touched {
    /// Everything created, changed or removed, of these types (all types if empty)
    pub fn any_of(&self, types: &[&str]) -> bool {
        let hit = |t: &str| types.is_empty() || types.contains(&t);
        self.created.iter().any(|(_, t)| hit(t))
            || self.changed.iter().any(|(_, t, _)| hit(t))
            || self.removed.iter().any(|(_, t)| hit(t))
    }

    /// Created or changed nodes of a type
    pub fn alive_of<'a>(&'a self, type_name: &'a str) -> impl Iterator<Item = NodeId> + 'a {
        self.created
            .iter()
            .filter(move |(_, t)| *t == type_name)
            .map(|(id, _)| *id)
            .chain(self.changed.iter().filter(move |(_, t, _)| *t == type_name).map(|(id, _, _)| *id))
    }

    pub fn changed_paths(&self, id: NodeId) -> Option<&[Vec<PathStep>]> {
        self.changed.iter().find(|(n, _, _)| *n == id).map(|(_, _, p)| p.as_slice())
    }

    pub fn is_created(&self, id: NodeId) -> bool {
        self.created.iter().any(|(n, _)| *n == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// Changes the transaction to keep the tree coherent
    Reconcile,
    /// Refuses an invalid transaction
    Check,
}

pub type RuleFn = Arc<dyn Fn(&mut Edit<'_>, &Touched) -> Result<()> + Send + Sync>;

/// A domain rule, registered for the types whose changes make it run (empty: every transaction)
#[derive(Clone)]
pub struct Rule {
    pub name: &'static str,
    pub kind: RuleKind,
    pub types: &'static [&'static str],
    pub run: RuleFn,
}

impl fmt::Debug for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rule").field("name", &self.name).field("kind", &self.kind).finish_non_exhaustive()
    }
}

impl Rule {
    pub fn new(
        name: &'static str,
        kind: RuleKind,
        types: &'static [&'static str],
        run: impl Fn(&mut Edit<'_>, &Touched) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self { name, kind, types, run: Arc::new(run) }
    }
}

/// The project's own rules
pub fn project_rules() -> Vec<Rule> {
    vec![
        Rule::new("ports", RuleKind::Reconcile, &[types::PLUGIN, types::TRACK], reconcile_ports),
        Rule::new("automation_targets", RuleKind::Reconcile, &[types::PLUGIN], drop_orphan_automation),
        Rule::new("master", RuleKind::Check, &[], check_master),
        Rule::new("folders", RuleKind::Check, &[types::TRACK], check_folders),
        Rule::new(
            "routing",
            RuleKind::Check,
            &[types::CHANNEL, types::MASTER, types::SEND, types::PLUGIN, types::MIXER],
            check_routing,
        ),
        Rule::new("items", RuleKind::Check, &[types::ITEM], check_items),
        Rule::new("tempo", RuleKind::Check, &[PROJECT], check_tempo),
        Rule::new("marks", RuleKind::Check, &[PROJECT, types::MARKER, types::REGION], check_marks),
        Rule::new("automation", RuleKind::Check, &[types::AUTOMATION], check_automation),
        Rule::new("limits", RuleKind::Check, &[], check_limits),
    ]
}

fn id_of(v: &Value) -> Option<NodeId> {
    v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())
}

fn ref_of(v: &Value, key: &str) -> Option<NodeId> {
    v.get(key).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())
}

// ---- M1: the instrument's ports and the track's outputs ----

/// For every instrument created or changed (its plugin or its state), the track's outputs follow its ports
/// (I-3.9): a port that stays keeps its output; a new port gets an output to where the main output goes; an output
/// whose port is gone leaves, with a notice. A new track without instrument and without outputs gets its main
/// output, to the master
fn reconcile_ports(e: &mut Edit<'_>, touched: &Touched) -> Result<()> {
    let mut tracks = BTreeSet::new();
    for p in touched.alive_of(types::PLUGIN) {
        let changed_state = touched.is_created(p)
            || touched.changed_paths(p).is_some_and(|paths| {
                paths.iter().any(|path| {
                    path.is_empty()
                        || matches!(path.first(), Some(PathStep::Key(k)) if matches!(&**k, "plugin" | "state"))
                })
            });
        if let Some(loc) = e.loc(p)
            && changed_state
            && loc.container.len() == 1
            && &*loc.container[0] == "instrument"
        {
            tracks.insert(loc.parent);
        }
    }
    for t in touched.alive_of(types::TRACK) {
        if touched.is_created(t)
            || touched
                .changed_paths(t)
                .is_some_and(|ps| ps.iter().any(|p| p.first() == Some(&PathStep::key("instrument"))))
        {
            tracks.insert(t);
        }
    }
    for track in tracks {
        reconcile_track(e, track)?;
    }
    Ok(())
}

fn reconcile_track(e: &mut Edit<'_>, track: NodeId) -> Result<()> {
    let Some(node) = e.find(track).cloned() else { return Ok(()) };
    let outputs: Vec<Value> = node.get("outputs").map(|o| o.items().cloned().collect()).unwrap_or_default();
    let main_to = outputs.first().and_then(|o| ref_of(o, "to")).unwrap_or(MASTER);
    let instrument = node.get("instrument").filter(|i| !i.is_null());
    let Some(instrument) = instrument else {
        let is_folder = node.get("folder").and_then(Value::as_bool).unwrap_or(false);
        if outputs.is_empty() && !is_folder {
            let channels = node.get("channels").cloned().unwrap_or(Value::int(2));
            e.insert(
                track,
                "outputs",
                0,
                Value::object([
                    ("port", Value::text("main")),
                    ("channels", channels),
                    ("to", Value::text(MASTER.to_string())),
                ]),
            )?;
        }
        return Ok(());
    };
    let plugin = instrument.get("plugin").and_then(Value::as_text).unwrap_or("");
    let state = instrument.get("state").cloned().unwrap_or_default();
    let Some(ports) = e.ports(plugin, &state) else { return Ok(()) };
    let mut keys = BTreeSet::new();
    for port in &ports.outputs {
        if !keys.insert(Arc::clone(&port.key)) {
            return Err(Error::new("doc.ports.duplicate").arg("port", &*port.key));
        }
        if !(1..=64).contains(&port.channels) {
            return Err(Error::new("doc.ports.channels").arg("port", &*port.key));
        }
    }
    // Gone ports: their outputs leave
    let mut gone = Vec::new();
    for o in &outputs {
        let port = o.get("port").and_then(Value::as_text).unwrap_or("");
        if !keys.contains(port)
            && let Some(id) = id_of(o)
        {
            gone.push(port.to_owned());
            e.remove(id)?;
        }
    }
    // Staying and new ports, in the plugin's order
    for (i, port) in ports.outputs.iter().enumerate() {
        let current: Vec<Value> =
            e.find(track).and_then(|t| t.get("outputs")).map(|o| o.items().cloned().collect()).unwrap_or_default();
        let found = current.iter().find(|o| o.get("port").and_then(Value::as_text) == Some(&*port.key));
        let channels = Value::int(i64::from(port.channels));
        match found.and_then(id_of) {
            Some(id) => {
                if found.and_then(|o| o.get("channels")) != Some(&channels) {
                    e.set(id, &[PathStep::key("channels")], channels)?;
                }
                e.move_to(id, track, "outputs", i)?;
            }
            None => {
                let node = Value::object([
                    ("port", Value::text(Arc::clone(&port.key))),
                    ("channels", channels),
                    ("to", Value::text(main_to.to_string())),
                ]);
                e.insert(track, "outputs", i, node)?;
            }
        }
    }
    if !gone.is_empty() {
        e.notice(Label::new("doc.outputs.removed").arg("track", track.to_string()).arg("ports", gone.join(",")));
    }
    Ok(())
}

// ---- automation ----

/// Whether `param` is a parameter of the node `target` (through aliases, C5). `None` when it cannot be known (a
/// plugin not available)
fn param_exists(e: &Edit<'_>, target: NodeId, param: &str) -> Option<bool> {
    let t = e.type_of(target).ok()?;
    let node = e.find(target)?;
    if t == types::PLUGIN {
        let plugin = node.get("plugin").and_then(Value::as_text)?;
        let schema = e.registry().find(&plugin_type(plugin))?;
        return Some(schema.find_param(param).is_some());
    }
    let path = param_path(t, param);
    let is_param = crate::edit::param_key(t, &path).is_some();
    Some(is_param && node.at_path(&path).and_then(Value::as_real).is_some())
}

/// A plugin replaced by another: the automation lanes of parameters it no longer has leave, with a notice
fn drop_orphan_automation(e: &mut Edit<'_>, touched: &Touched) -> Result<()> {
    for p in touched.alive_of(types::PLUGIN).collect::<Vec<_>>() {
        let swapped = touched
            .changed_paths(p)
            .is_some_and(|ps| ps.iter().any(|path| path.first() == Some(&PathStep::key("plugin"))));
        if !swapped {
            continue;
        }
        for r in e.referrers(p) {
            if e.type_of(r.owner).ok() != Some(types::AUTOMATION) {
                continue;
            }
            let param = e
                .find(r.owner)
                .and_then(|a| a.get("target"))
                .and_then(|t| t.get("param"))
                .and_then(Value::as_text)
                .map(str::to_owned);
            if let Some(param) = param
                && param_exists(e, p, &param) == Some(false)
            {
                e.remove(r.owner)?;
                e.notice(Label::new("doc.automation.removed").arg("lane", r.owner.to_string()).arg("param", param));
            }
        }
    }
    Ok(())
}

fn check_automation(e: &mut Edit<'_>, touched: &Touched) -> Result<()> {
    for a in touched.alive_of(types::AUTOMATION) {
        let Some(lane) = e.find(a) else { continue };
        let target = lane.get("target").and_then(|t| ref_of(t, "node"));
        let param = lane.get("target").and_then(|t| t.get("param")).and_then(Value::as_text).unwrap_or("");
        if let Some(target) = target
            && param_exists(e, target, param) == Some(false)
        {
            return Err(Error::new("doc.automation.no_such_param").arg("lane", a.to_string()).arg("param", param));
        }
        let ticks: Vec<i64> = lane
            .get("points")
            .map(|p| p.items().filter_map(|p| p.get("tick").and_then(Value::as_int)).collect())
            .unwrap_or_default();
        if ticks.windows(2).any(|w| w[0] > w[1]) {
            return Err(Error::new("doc.automation.unsorted").arg("lane", a.to_string()));
        }
    }
    Ok(())
}

// ---- checks ----

fn check_master(e: &mut Edit<'_>, _: &Touched) -> Result<()> {
    if e.registry().find(PROJECT).is_none() || e.type_of(ROOT)? != PROJECT {
        return Ok(());
    }
    let master = e.root().get("mixer").and_then(|m| m.get("master")).and_then(id_of);
    if master == Some(MASTER) { Ok(()) } else { Err(Error::new("doc.master.id")) }
}

/// A track's parent is a folder, and no folder is inside itself
fn check_folders(e: &mut Edit<'_>, touched: &Touched) -> Result<()> {
    let relevant = touched.created.iter().any(|(_, t)| *t == types::TRACK)
        || touched.changed.iter().any(|(_, t, paths)| {
            *t == types::TRACK
                && paths.iter().any(|p| {
                    p.is_empty()
                        || p.first() == Some(&PathStep::key("parent"))
                        || p.first() == Some(&PathStep::key("folder"))
                })
        });
    if !relevant {
        return Ok(());
    }
    let Some(tracks) = e.root().get("tracks") else { return Ok(()) };
    let mut parent = BTreeMap::new();
    let mut folder = BTreeMap::new();
    for t in tracks.items() {
        let Some(id) = id_of(t) else { continue };
        folder.insert(id, t.get("folder").and_then(Value::as_bool).unwrap_or(false));
        if let Some(p) = ref_of(t, "parent") {
            parent.insert(id, p);
        }
    }
    for (child, p) in &parent {
        if folder.get(p) != Some(&true) {
            return Err(Error::new("doc.track.parent_not_folder")
                .arg("track", child.to_string())
                .arg("parent", p.to_string()));
        }
        let mut at = *p;
        let mut steps = 0;
        while let Some(next) = parent.get(&at) {
            at = *next;
            steps += 1;
            if at == *child || steps > parent.len() {
                return Err(Error::new("doc.track.folder_cycle").arg("track", child.to_string()));
            }
        }
        if at == *child {
            return Err(Error::new("doc.track.folder_cycle").arg("track", child.to_string()));
        }
    }
    Ok(())
}

/// The desk is a graph without cycles (base spec, item 3): a channel's output, its sends to channels, and its sends
/// to an insert's sidechain (which feed that insert's channel). A sidechain send names a port the insert declares
fn check_routing(e: &mut Edit<'_>, _: &Touched) -> Result<()> {
    let Some(mixer) = e.root().get("mixer").cloned() else { return Ok(()) };
    let mut edges: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let channels: Vec<Value> = mixer.get("channels").map(|c| c.items().cloned().collect()).unwrap_or_default();
    let mut all = channels.clone();
    if let Some(m) = mixer.get("master") {
        all.push(m.clone());
    }
    for ch in &all {
        let Some(id) = id_of(ch) else { continue };
        let out = edges.entry(id).or_default();
        if let Some(o) = ref_of(ch, "output") {
            out.push(o);
        }
        for send in ch.get("sends").map(|s| s.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
            let Some(to) = ref_of(&send, "to") else { continue };
            let port = send.get("port").and_then(Value::as_text);
            if to.kind == KIND_CHANNEL {
                if port.is_some() {
                    return Err(Error::new("doc.send.port_on_channel")
                        .arg("send", id_of(&send).map(|s| s.to_string()).unwrap_or_default()));
                }
                edges.entry(id).or_default().push(to);
            } else if to.kind == KIND_PLUGIN {
                let Some(loc) = e.loc(to) else { continue };
                let is_insert =
                    loc.parent.kind == KIND_CHANNEL && loc.container.len() == 1 && &*loc.container[0] == "inserts";
                if !is_insert {
                    return Err(Error::new("doc.send.not_an_insert").arg("to", to.to_string()));
                }
                let Some(port) = port else {
                    return Err(Error::new("doc.send.no_port").arg("to", to.to_string()));
                };
                if let Some(insert) = e.find(to) {
                    let plugin = insert.get("plugin").and_then(Value::as_text).unwrap_or("");
                    let state = insert.get("state").cloned().unwrap_or_default();
                    if let Some(ports) = e.ports(plugin, &state)
                        && !ports.sidechains.iter().any(|p| &*p.key == port)
                    {
                        return Err(Error::new("doc.send.no_such_port").arg("to", to.to_string()).arg("port", port));
                    }
                }
                edges.entry(id).or_default().push(loc.parent);
            }
        }
    }
    // Depth-first search with colours
    let mut colour: BTreeMap<NodeId, u8> = BTreeMap::new();
    for start in edges.keys().copied().collect::<Vec<_>>() {
        if colour.contains_key(&start) {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        colour.insert(start, 1);
        while let Some((node, i)) = stack.pop() {
            let next = edges.get(&node).and_then(|n| n.get(i)).copied();
            match next {
                Some(n) => {
                    stack.push((node, i + 1));
                    match colour.get(&n) {
                        Some(1) => return Err(Error::new("doc.routing.cycle").arg("channel", n.to_string())),
                        Some(_) => {}
                        None => {
                            colour.insert(n, 1);
                            stack.push((n, 0));
                        }
                    }
                }
                None => {
                    colour.insert(node, 2);
                }
            }
        }
    }
    Ok(())
}

/// A MIDI item has notes and no lanes; an audio item the opposite. A comp segment points to a lane of its own item,
/// within the item
fn check_items(e: &mut Edit<'_>, touched: &Touched) -> Result<()> {
    for c in touched.alive_of(types::ITEM) {
        let Some(item) = e.find(c) else { continue };
        let kind = item.get("kind").and_then(Value::as_text).unwrap_or("");
        let has = |k: &str| item.get(k).is_some_and(|v| !v.is_empty());
        let bad = |what: &'static str| Error::new("doc.item.invalid").arg("item", c.to_string()).arg("problem", what);
        match kind {
            "midi" if has("lanes") || has("comp") => return Err(bad("midi_with_lanes")),
            "audio" if has("notes") => return Err(bad("audio_with_notes")),
            _ => {}
        }
        let length = item.get("length").and_then(Value::as_int).unwrap_or(0);
        for seg in item.get("comp").map(|c| c.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
            let from = seg.get("from").and_then(Value::as_int).unwrap_or(0);
            let to = seg.get("to").and_then(Value::as_int).unwrap_or(0);
            if from >= to || to > length {
                return Err(bad("comp_range"));
            }
            if let Some(lane) = ref_of(&seg, "lane")
                && e.loc(lane).map(|l| l.parent) != Some(c)
            {
                return Err(bad("comp_lane_elsewhere"));
            }
        }
    }
    Ok(())
}

/// Markers and regions (F-REG-011, F-REG-012): one marker per tick; a region at least a sixteenth of a 4/4 bar
fn check_marks(e: &mut Edit<'_>, _: &Touched) -> Result<()> {
    if e.type_of(ROOT)? != PROJECT {
        return Ok(());
    }
    let root = e.root();
    let ppq = root.get("project").and_then(|p| p.get("ppq")).and_then(Value::as_int).unwrap_or(960);
    let bad = |what: &'static str| Error::new("doc.marks.invalid").arg("problem", what);
    let mut ticks: Vec<i64> = root
        .get("markers")
        .map(|l| l.items().filter_map(|m| m.get("tick").and_then(Value::as_int)).collect())
        .unwrap_or_default();
    let n = ticks.len();
    ticks.sort_unstable();
    ticks.dedup();
    if ticks.len() != n {
        return Err(bad("two_markers_on_one_tick"));
    }
    for r in root.get("regions").into_iter().flat_map(Value::items) {
        let from = r.get("from").and_then(Value::as_int).unwrap_or(0);
        let to = r.get("to").and_then(Value::as_int).unwrap_or(0);
        if to - from < (ppq / 4).max(1) {
            return Err(bad("region_too_short"));
        }
    }
    Ok(())
}

/// The tempo map: a first point at tick 0, ticks increasing, exact BPMs, time signatures with a power-of-two
/// denominator, and the last point not a ramp
fn check_tempo(e: &mut Edit<'_>, touched: &Touched) -> Result<()> {
    let relevant = touched.changed.iter().any(|(id, _, paths)| {
        *id == ROOT && paths.iter().any(|p| p.is_empty() || p.first() == Some(&PathStep::key("tempoMap")))
    });
    if !relevant || e.type_of(ROOT)? != PROJECT {
        return Ok(());
    }
    check_tempo_map(e.root().get("tempoMap").unwrap_or(&Value::NULL))
}

pub(crate) fn check_tempo_map(map: &Value) -> Result<()> {
    let bad = |what: &'static str| Error::new("doc.tempo.invalid").arg("problem", what);
    let points: Vec<&Value> = map.items().collect();
    let Some(first) = points.first() else { return Err(bad("empty")) };
    if first.get("tick").and_then(Value::as_int) != Some(0) {
        return Err(bad("first_not_at_zero"));
    }
    let mut last = -1;
    for p in &points {
        let tick = p.get("tick").and_then(Value::as_int).unwrap_or(-1);
        if tick <= last {
            return Err(bad("ticks_not_increasing"));
        }
        last = tick;
        if !valid_bpm(p.get("bpm").and_then(Value::as_text).unwrap_or("")) {
            return Err(bad("bpm"));
        }
        let sig: Vec<i64> = p.get("timeSig").map(|s| s.items().filter_map(Value::as_int).collect()).unwrap_or_default();
        if sig.len() != 2 || !(sig[1] > 0 && (sig[1] & (sig[1] - 1)) == 0) {
            return Err(bad("time_signature"));
        }
    }
    if points.last().and_then(|p| p.get("ramp")).and_then(Value::as_bool) == Some(true) {
        return Err(bad("last_is_ramp"));
    }
    Ok(())
}

/// The BPM text `lm-time` reads: a plain decimal, 1 to 1000, up to 6 decimals
pub fn valid_bpm(text: &str) -> bool {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    if int.is_empty()
        || int.len() > 4
        || !int.bytes().all(|b| b.is_ascii_digit())
        || (int.len() > 1 && int.starts_with('0'))
    {
        return false;
    }
    if text.contains('.') && (frac.is_empty() || frac.len() > 6 || !frac.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    let Ok(whole) = int.parse::<u32>() else { return false };
    let any_frac = frac.bytes().any(|b| b != b'0');
    (1..1000).contains(&whole) || (whole == 1000 && !any_frac)
}

fn check_limits(e: &mut Edit<'_>, touched: &Touched) -> Result<()> {
    let relevant = !touched.created.is_empty() || touched.changed.iter().any(|(_, t, _)| *t == types::PLUGIN);
    if !relevant {
        return Ok(());
    }
    e.check_limits()
}

/// The engine's usage of a project, as counted by default (the compiler's own count can replace it, see
/// [`crate::Options::usage`]):
/// - nodes: per track, its source (items and input) and its instrument; per track output, its path; per desk
///   channel and the master, the channel, its rack (M5) and each insert;
/// - params: per plugin, its schema's parameters; per track, its volume; per channel, its volume, pan and the
///   real fields of its rack; for the master, its volume and its rack's
pub fn default_usage(registry: &SchemaRegistry, root: &Value) -> Usage {
    let mut usage = Usage::default();
    let plugin_params = |p: &Value| {
        let name = p.get("plugin").and_then(Value::as_text).unwrap_or("");
        registry.find(&plugin_type(name)).map_or(0, |s| s.params.len())
    };
    let rack_params = |rack: Option<&Value>| {
        rack.map_or(0, |r| {
            r.entries().map(|(_, section)| section.entries().filter(|(_, v)| v.as_real().is_some()).count()).sum()
        })
    };
    for t in root.get("tracks").map(|t| t.items().collect::<Vec<_>>()).unwrap_or_default() {
        usage.nodes += 1;
        usage.params += 1;
        if let Some(i) = t.get("instrument").filter(|i| !i.is_null()) {
            usage.nodes += 1;
            usage.params += plugin_params(i);
        }
        usage.nodes += t.get("outputs").map_or(0, Value::len);
    }
    let mixer = root.get("mixer");
    let mut channels: Vec<(&Value, usize)> =
        mixer.and_then(|m| m.get("channels")).map(|c| c.items().map(|c| (c, 2)).collect()).unwrap_or_default();
    if let Some(m) = mixer.and_then(|m| m.get("master")) {
        channels.push((m, 1));
    }
    for (ch, own) in channels {
        usage.nodes += 2;
        usage.params += own + rack_params(ch.get("rack"));
        for p in ch.get("inserts").map(|i| i.items().collect::<Vec<_>>()).unwrap_or_default() {
            usage.nodes += 1;
            usage.params += plugin_params(p);
        }
    }
    usage
}

impl Edit<'_> {
    /// Refuses the transaction when the project needs more of the engine than its limits (H12)
    pub fn check_limits(&self) -> Result<()> {
        let Some(limits) = self.w.env.limits else { return Ok(()) };
        let usage = (self.w.env.usage)(self.registry(), self.root());
        limits_error(&limits, &usage).map_or(Ok(()), Err)
    }
}

pub(crate) fn limits_error(limits: &Limits, usage: &Usage) -> Option<Error> {
    if usage.nodes > limits.max_nodes {
        return Some(
            Error::new("doc.limits.nodes").arg("used", usage.nodes as i64).arg("max", limits.max_nodes as i64),
        );
    }
    if usage.params > limits.max_params {
        return Some(
            Error::new("doc.limits.params").arg("used", usage.params as i64).arg("max", limits.max_params as i64),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpm_texts() {
        for ok in ["120", "1", "1000", "119.88", "60.000001", "999.999999", "1000.0"] {
            assert!(valid_bpm(ok), "{ok}");
        }
        for bad in ["", "0", "1000.1", "01", "1.", ".5", "1.0000001", "-1", "1e3", "abc", "10000"] {
            assert!(!valid_bpm(bad), "{bad}");
        }
    }
}
