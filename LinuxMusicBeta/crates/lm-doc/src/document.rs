//! [`Document`]: the owner of the tree (I-3, arquitetura-nova.md 2a). The current root with its indices, the history
//! (the states that existed, with the diff of each step and its inverse, U18), the open gestures (I-3.7), the feeds
//! (I-3.6) and the journal (I-4).

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lm_base::{Error, Label, Result};
use lm_state::{FieldKind, NodeId, PathStep, ROOT, SchemaRegistry, Value};

use crate::diff::{Diff, DiffAcc, Origin};
use crate::edit::{Clip, Edit, Env, GestureRecord, Phase, State, Work, check_plugin, param_path};
use crate::index::{Ctx, Index, Loc, RefFrom};
use crate::ops::{JournalOp, JournalPatch, Patch};
use crate::rules::{Limits, PortsFn, Rule, RuleKind, Touched, UsageFn, default_usage, limits_error, project_rules};
use crate::schemas::{MASTER, PROJECT, plugin_type, types};

/// How a document is set up
#[derive(Clone)]
pub struct Options {
    /// The number of undo steps kept; 0: no history (the interface's document, I-3.8)
    pub history_limit: usize,
    /// Whether the changes are kept for the journal ([`Document::take_journal`])
    pub journal: bool,
    /// The engine's limits, as document rules (H12)
    pub limits: Option<Limits>,
    /// The plugin layer's ports of a plugin's state (M1)
    pub ports: Option<PortsFn>,
    /// How the engine usage is counted for the limits
    pub usage: UsageFn,
    /// The domain rules, in order (reconciling ones run first anyway)
    pub rules: Vec<Rule>,
}

impl Default for Options {
    /// A project document: 1000 undo steps, the journal, the project's rules
    fn default() -> Self {
        Self {
            history_limit: 1000,
            journal: true,
            limits: None,
            ports: None,
            usage: Arc::new(default_usage),
            rules: project_rules(),
        }
    }
}

impl Options {
    /// The interface's document (I-3.8): no history, no journal, no project rules
    pub fn interface() -> Self {
        Self { history_limit: 0, journal: false, rules: Vec::new(), ..Self::default() }
    }
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("history_limit", &self.history_limit)
            .field("journal", &self.journal)
            .field("limits", &self.limits)
            .field("rules", &self.rules)
            .finish_non_exhaustive()
    }
}

/// A gesture's handle (I-3.7). The interface keeps it between frames. Dropped without `commit` or `cancel`, it is
/// committed at the next [`Document::deliver`]
#[must_use = "a gesture must be committed or cancelled"]
pub struct GestureId {
    raw: u64,
    dropped: Option<Rc<RefCell<Vec<u64>>>>,
}

impl GestureId {
    pub fn raw(&self) -> u64 {
        self.raw
    }

    /// A handle for a gesture opened elsewhere, by its number: the RemotePort's id of a gesture the server opened
    /// (I-15). Not armed: dropping it commits nothing. A number this document did not issue, or a gesture already
    /// closed, is refused with `doc.gesture.closed` by `gesture_update`, `gesture_commit` and `gesture_cancel`
    pub fn from_raw(raw: u64) -> Self {
        Self { raw, dropped: None }
    }

    fn disarm(mut self) -> u64 {
        self.dropped = None;
        self.raw
    }
}

impl fmt::Debug for GestureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GestureId({})", self.raw)
    }
}

impl Drop for GestureId {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            dropped.borrow_mut().push(self.raw);
        }
    }
}

/// What a feed follows
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Node(NodeId),
    /// Every node of a type (`"track"`)
    Type(&'static str),
    All,
}

/// A receiver of diffs for one topic (I-3.6): nobody is called back; whoever wants to know drains it
#[must_use = "a feed only gathers diffs while it is kept"]
pub struct Feed {
    doc: u64,
    key: u64,
    dropped: Rc<RefCell<Vec<u64>>>,
}

impl fmt::Debug for Feed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Feed({})", self.key)
    }
}

impl Feed {
    /// The diffs since the last drain, merged by the final state (T13); `None` if nothing happened
    pub fn drain(&mut self, doc: &Document) -> Option<Diff> {
        if doc.serial != self.doc {
            return None;
        }
        let mut feeds = doc.feeds.borrow_mut();
        let slot = feeds.get_mut(&self.key)?;
        let acc = std::mem::take(&mut slot.acc);
        let diff = acc.finish(Origin::User);
        if diff.is_empty() && diff.notices.is_empty() { None } else { Some(diff) }
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.dropped.borrow_mut().push(self.key);
    }
}

#[derive(Debug)]
struct FeedSlot {
    topic: Topic,
    acc: DiffAcc,
}

#[derive(Debug)]
struct Gesture {
    label: Label,
    record: GestureRecord,
    /// The number of history steps pushed when it began: a different number at cancel means a foreign step came
    steps_at_begin: u64,
}

/// One step of the history: what it changed, its inverse, and the journal ops of both (U18)
#[derive(Debug)]
struct Step {
    label: Label,
    acc: DiffAcc,
    inverse_acc: DiffAcc,
    ops: Vec<JournalOp>,
    /// Ready to apply, in order
    inverse: Vec<JournalOp>,
}

/// The changes since the last history step (an open gesture's, or a cancelled one's)
#[derive(Debug, Default)]
struct Pending {
    ops: Vec<JournalOp>,
    inverse: Vec<JournalOp>,
    acc: DiffAcc,
}

/// What [`Document::deliver`] did
#[derive(Debug, Default)]
pub struct Delivered {
    /// The gestures dropped without commit, now committed (with their diff, or the error of the commit)
    pub committed: Vec<(u64, Result<Diff>)>,
    pub forgotten_feeds: usize,
}

/// A document opened from a saved tree
#[derive(Debug)]
pub struct Opened {
    pub document: Document,
    pub notices: Vec<Label>,
    /// Some node was upgraded from an older version (the session keeps `projeto.json.v<N>.bak`, H11)
    pub upgraded: bool,
}

static SERIAL: AtomicU64 = AtomicU64::new(1);

/// The document: the tree, its history, its gestures and its feeds. Lives on the main thread (`!Send`); what other
/// threads get is a [`Document::snapshot`]
pub struct Document {
    env: Arc<Env>,
    serial: u64,
    current: State,
    issued: u64,
    read_only: bool,
    history_limit: usize,
    journal_on: bool,
    /// The states at each step boundary: `checkpoints[cursor]` is the last one
    checkpoints: VecDeque<State>,
    steps: VecDeque<Step>,
    cursor: usize,
    steps_pushed: u64,
    pending: Pending,
    journal: Vec<JournalOp>,
    gestures: BTreeMap<u64, Gesture>,
    next_gesture: u64,
    gestures_dropped: Rc<RefCell<Vec<u64>>>,
    feeds: RefCell<BTreeMap<u64, FeedSlot>>,
    next_feed: u64,
    feeds_dropped: Rc<RefCell<Vec<u64>>>,
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("root_type", &self.env.ctx.root_type)
            .field("nodes", &self.current.index.len())
            .field("steps", &self.steps.len())
            .field("cursor", &self.cursor)
            .field("gestures", &self.gestures.len())
            .finish_non_exhaustive()
    }
}

/// A new uuid (version 4 form) for a project's `meta.uuid`
pub fn new_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = blake3::Hasher::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    h.update(&now.to_le_bytes());
    h.update(&std::process::id().to_le_bytes());
    h.update(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let mut random = std::collections::hash_map::RandomState::new().build_hasher();
    random.write_u64(0);
    h.update(&random.finish().to_le_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h.finalize().as_bytes()[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

fn touched_of(acc: &DiffAcc) -> Touched {
    let mut t = Touched::default();
    for (id, e) in &acc.nodes {
        if e.is_created() {
            t.created.push((*id, e.type_name));
        } else if e.is_removed() {
            t.removed.push((*id, e.type_name));
        } else {
            let mut paths: Vec<Vec<PathStep>> = e.paths().map(<[PathStep]>::to_vec).collect();
            if e.has_note_changes() {
                paths.push(vec![PathStep::key("notes")]);
            }
            if !paths.is_empty() {
                t.changed.push((*id, e.type_name, paths));
            }
        }
    }
    t
}

fn run_rules(env: &Env, w: &mut Work, kinds: &[RuleKind], skip: &[&str]) -> Result<()> {
    w.phase = Phase::Rules;
    for kind in kinds {
        let touched = touched_of(&w.acc);
        for rule in env.rules.iter().filter(|r| r.kind == *kind && !skip.contains(&r.name)) {
            if touched.any_of(rule.types) {
                (rule.run)(&mut Edit { w }, &touched).map_err(|e| e.arg("rule", rule.name))?;
            }
        }
    }
    Ok(())
}

impl Document {
    // ---- making and opening ----

    /// A new, empty project: the master `m:0`, the default tempo map, a new uuid and seed
    pub fn new_project(registry: Arc<SchemaRegistry>, options: Options) -> Result<Document> {
        Self::new_project_with(registry, options, false)
    }

    /// A new project the way the program starts one: [`Document::new_project`] and the two effect returns, "FX 1" and
    /// "FX 2" (P-19), that every channel's FX knobs send to
    pub fn new_desk_project(registry: Arc<SchemaRegistry>, options: Options) -> Result<Document> {
        Self::new_project_with(registry, options, true)
    }

    fn new_project_with(registry: Arc<SchemaRegistry>, options: Options, returns: bool) -> Result<Document> {
        let mut root = registry.defaults(PROJECT)?;
        let mut issued = 1u64;
        if returns {
            let mut channels = Vec::new();
            for n in 1..=2u64 {
                channels.push(
                    registry
                        .defaults(types::CHANNEL)?
                        .with("id", Value::text(NodeId::new(crate::schemas::KIND_CHANNEL, n)?.to_string()))
                        .with("name", Value::text(format!("FX {n}")))
                        .with("return", Value::bool(true)),
                );
                issued = n + 1;
            }
            root = root.with_path(&[PathStep::key("mixer"), PathStep::key("channels")], Value::list(channels))?;
        }
        let uuid = new_uuid();
        let seed = i64::from_str_radix(&uuid.replace('-', "")[..12], 16).unwrap_or(0);
        let root = root
            .with_path(&[PathStep::key("meta"), PathStep::key("issued")], Value::int(issued as i64))?
            .with_path(&[PathStep::key("meta"), PathStep::key("uuid")], Value::text(uuid))?
            .with_path(&[PathStep::key("project"), PathStep::key("seed")], Value::int(seed))?
            .with_path(
                &[PathStep::key("mixer"), PathStep::key("master"), PathStep::key("id")],
                Value::text(MASTER.to_string()),
            )?;
        Ok(Self::from_root(registry, PROJECT, root, options)?.document)
    }

    /// Opens a saved project tree (the JSON of `projeto.json`, read by `lm_state::from_json`): reads the versions
    /// it was saved with, upgrades every older node (parents first), validates everything (every node, every
    /// reference across nodes, every plugin's state, the domain rules), and opens read-only when a node is of a
    /// newer version than this program's (H11)
    pub fn open(registry: Arc<SchemaRegistry>, saved: Value, options: Options) -> Result<Opened> {
        let versions = saved
            .get("versions")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(|| Error::new("doc.load.no_versions"))?;
        let root = saved.without("versions");
        let mut notices = Vec::new();
        let mut read_only = false;
        let mut from: BTreeMap<String, u32> = BTreeMap::new();
        for (t, v) in versions.entries() {
            let v = v
                .as_int()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| Error::new("doc.load.bad_version").arg("type", t))?;
            match registry.find(t) {
                Some(schema) if v <= schema.version => {
                    from.insert(t.to_owned(), v);
                }
                _ => {
                    read_only = true;
                    notices.push(Label::new("doc.load.newer").arg("type", t).arg("version", i64::from(v)));
                }
            }
        }
        let issued_before = root
            .get("meta")
            .and_then(|m| m.get("issued"))
            .and_then(Value::as_int)
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or(0);
        let mut up = Upgrader {
            registry: &registry,
            from: &from,
            outdated: false,
            upgraded: false,
            read_only,
            notices: Vec::new(),
            next_id: issued_before.max(first_free(crate::migrate::max_id(&root))?),
        };
        let next_before = up.next_id;
        up.outdated = from.iter().any(|(t, v)| registry.find(t).is_some_and(|s| s.version > *v));
        let ctx = Ctx::new(Arc::clone(&registry), PROJECT)?;
        let root = up.value(&ctx, &FieldKind::Object(PROJECT), root, 0)?;
        let upgraded = up.upgraded;
        // Nodes made by a migration took numbers from the counter
        let root = if up.next_id > next_before && root.get("meta").is_some() {
            root.with_path(&[PathStep::key("meta"), PathStep::key("issued")], Value::int(up.next_id as i64))?
        } else {
            root
        };
        notices.extend(up.notices);
        read_only |= up.read_only;
        let mut opened = Self::open_root(registry, PROJECT, root, options, read_only)?;
        notices.append(&mut opened.notices);
        opened.notices = notices;
        opened.upgraded = upgraded;
        Ok(opened)
    }

    /// A document over a tree already at the current versions (a test, the interface's document): validated like
    /// [`Document::open`], without upgrades
    pub fn from_root(registry: Arc<SchemaRegistry>, root_type: &str, root: Value, options: Options) -> Result<Opened> {
        Self::open_root(registry, root_type, root, options, false)
    }

    fn open_root(
        registry: Arc<SchemaRegistry>,
        root_type: &str,
        root: Value,
        options: Options,
        read_only: bool,
    ) -> Result<Opened> {
        let ctx = Ctx::new(registry, root_type)?;
        let mut notices = Vec::new();
        let tolerate = |e: Error, notices: &mut Vec<Label>| -> Result<()> {
            if read_only {
                notices.push(Label::new("doc.load.unvalidated").arg("error", e.to_string()));
                Ok(())
            } else {
                Err(e)
            }
        };
        if let Err(e) = ctx.registry.validate(ctx.root_type, &root) {
            tolerate(e, &mut notices)?;
        }
        let index = ctx.rebuild(&root)?;
        for (target, from) in index.refs.iter() {
            if !index.contains(*target) {
                let by = from.iter().next().map(|r| r.owner.to_string()).unwrap_or_default();
                tolerate(
                    Error::new("doc.load.dangling_ref").arg("target", target.to_string()).arg("by", by),
                    &mut notices,
                )?;
            }
        }
        for (id, loc) in index.ids.iter() {
            if loc.type_name == types::PLUGIN {
                let node =
                    ctx.path_of(&index, &root, *id).ok().and_then(|p| root.at_path(&p).cloned()).unwrap_or_default();
                match check_plugin(&ctx, *id, &node) {
                    Ok(Some(label)) => notices.push(label),
                    Ok(None) => {}
                    Err(e) => tolerate(e, &mut notices)?,
                }
            }
        }
        // The counter is above every id and every note number
        let mut root = root;
        let max_id = index.ids.keys().map(|id| id.number).max().unwrap_or(0);
        let max_note = ctx.max_note(&FieldKind::Object(ctx.root_type), &root).unwrap_or(0);
        let needed = first_free(max_id.max(max_note))?;
        let mut issued = needed;
        if ctx.has_meta {
            let saved = root
                .get("meta")
                .and_then(|m| m.get("issued"))
                .and_then(Value::as_int)
                .and_then(|v| u64::try_from(v).ok())
                .unwrap_or(0);
            if saved < needed {
                notices.push(Label::new("doc.load.issued_repaired").arg("from", saved as i64).arg("to", needed as i64));
                root = root.with_path(&[PathStep::key("meta"), PathStep::key("issued")], Value::int(needed as i64))?;
            } else {
                issued = saved;
            }
        }
        let env = Arc::new(Env {
            ctx,
            ports: options.ports,
            limits: options.limits,
            usage: options.usage,
            rules: options.rules,
        });
        // The checking rules, over everything
        let state = State { root, index };
        let mut w = Work::new(Arc::clone(&env), state.clone(), issued);
        for (id, loc) in state.index.ids.iter() {
            w.acc.created(*id, loc.type_name);
        }
        w.acc.changed(ROOT, env.ctx.root_type, &[]);
        if let Err(e) = run_rules(&env, &mut w, &[RuleKind::Check], &["limits"]) {
            tolerate(e, &mut notices)?;
        }
        if let Some(limits) = env.limits {
            let usage = (env.usage)(&env.ctx.registry, &state.root);
            if let Some(e) = limits_error(&limits, &usage) {
                notices.push(Label::new("doc.load.limits").arg("error", e.to_string()));
            }
        }
        let gestures_dropped = Rc::new(RefCell::new(Vec::new()));
        let document = Document {
            env,
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
            current: state.clone(),
            issued,
            read_only,
            history_limit: options.history_limit,
            journal_on: options.journal,
            checkpoints: VecDeque::from([state]),
            steps: VecDeque::new(),
            cursor: 0,
            steps_pushed: 0,
            pending: Pending::default(),
            journal: Vec::new(),
            gestures: BTreeMap::new(),
            next_gesture: 1,
            gestures_dropped,
            feeds: RefCell::new(BTreeMap::new()),
            next_feed: 1,
            feeds_dropped: Rc::new(RefCell::new(Vec::new())),
        };
        Ok(Opened { document, notices, upgraded: false })
    }

    /// The tree to save: the root plus `"versions"`, the schema version of every type at save time (plugins carry
    /// their own in each node's `"version"`). Saving is the session's: `lm_state::to_json` of this, then the journal
    /// base is the BLAKE3 of those bytes
    pub fn saved(&self) -> Value {
        let versions = self
            .env
            .ctx
            .registry
            .schemas()
            .filter(|s| !s.type_name.starts_with("plugin:"))
            .map(|s| (s.type_name, Value::int(i64::from(s.version))));
        self.current.root.with("versions", Value::object(versions))
    }

    // ---- reading ----

    pub fn root(&self) -> &Value {
        &self.current.root
    }

    /// The current root, to keep or send to another thread (an O(1) clone; a heavy edit computes on it, H7)
    pub fn snapshot(&self) -> Value {
        self.current.root.clone()
    }

    pub fn registry(&self) -> &Arc<SchemaRegistry> {
        &self.env.ctx.registry
    }

    /// Goes on with a registry that has every type of the current one, at the same version, and more (the plugins a
    /// scan found, I-9.6): the tree, the history and the gestures stay as they are. `doc.registry.narrower` when a
    /// type is missing or at another version
    pub fn widen(&mut self, registry: Arc<SchemaRegistry>, ports: Option<PortsFn>) -> Result<()> {
        for old in self.env.ctx.registry.schemas() {
            match registry.find(old.type_name) {
                Some(new) if new.version == old.version => {}
                _ => return Err(Error::new("doc.registry.narrower").arg("type", old.type_name)),
            }
        }
        let ctx = Ctx::new(registry, self.env.ctx.root_type)?;
        let env = Env {
            ctx,
            ports: ports.or_else(|| self.env.ports.clone()),
            limits: self.env.limits,
            usage: Arc::clone(&self.env.usage),
            rules: self.env.rules.clone(),
        };
        self.env = Arc::new(env);
        Ok(())
    }

    /// A node, as a clone (never a pointer into a root, R8)
    pub fn find(&self, id: NodeId) -> Option<Value> {
        self.path_of(id).ok().and_then(|p| self.current.root.at_path(&p).cloned())
    }

    pub fn exists(&self, id: NodeId) -> bool {
        self.current.index.contains(id)
    }

    pub fn type_of(&self, id: NodeId) -> Option<&'static str> {
        self.env.ctx.type_of(&self.current.index, id).ok()
    }

    pub fn loc(&self, id: NodeId) -> Option<&Loc> {
        self.current.index.loc(id)
    }

    pub fn path_of(&self, id: NodeId) -> Result<Vec<PathStep>> {
        self.env.ctx.path_of(&self.current.index, &self.current.root, id)
    }

    /// Who points to `id`
    pub fn referrers(&self, id: NodeId) -> Vec<RefFrom> {
        self.current.index.referrers(id)
    }

    pub fn index(&self) -> &Index {
        &self.current.index
    }

    /// Rebuilds the indices from scratch and compares them with the ones kept (a test of the incremental upkeep)
    pub fn verify_index(&self) -> Result<()> {
        let fresh = self.env.ctx.rebuild(&self.current.root)?;
        if fresh == self.current.index { Ok(()) } else { Err(Error::new("doc.index.mismatch")) }
    }

    /// A parameter's value (`"rack.tube.drive"`, `"vol1"`)
    pub fn param(&self, node: NodeId, key: &str) -> Option<f64> {
        let t = self.type_of(node)?;
        self.find(node)?.at_path(&param_path(t, key)).and_then(Value::as_real)
    }

    /// The project opened read-only (a node newer than this program, H11)
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn uuid(&self) -> Option<&str> {
        self.current.root.get("meta").and_then(|m| m.get("uuid")).and_then(Value::as_text)
    }

    /// The next number the global counter gives
    pub fn issued(&self) -> u64 {
        self.issued
    }

    /// A node copied for pasting (H13)
    pub fn copy(&self, id: NodeId) -> Result<Clip> {
        let type_name = self.env.ctx.type_of(&self.current.index, id)?;
        let node = self.find(id).ok_or_else(|| crate::index::not_found(id))?;
        Ok(Clip { type_name, node, source_uuid: Arc::from(self.uuid().unwrap_or("")) })
    }

    // ---- transactions ----

    /// One transaction, all or nothing: the closure's changes, then the domain rules; any error leaves the document
    /// as it was. Enters the history as one step
    pub fn transact(&mut self, label: Label, edit: impl FnOnce(&mut Edit<'_>) -> Result<()>) -> Result<Diff> {
        self.transact_as(label, Origin::User, edit)
    }

    /// [`Document::transact`] with another origin (`Plugin`, `Recording`)
    pub fn transact_as(
        &mut self,
        label: Label,
        origin: Origin,
        edit: impl FnOnce(&mut Edit<'_>) -> Result<()>,
    ) -> Result<Diff> {
        let (diff, _) = self.run(origin, None, false, edit)?;
        self.push_step(label);
        Ok(diff)
    }

    /// Applies a heavy edit computed on a snapshot (H7), as one transaction; `doc.patch.conflict` if a node it was
    /// computed from changed since
    pub fn apply_patch(&mut self, label: Label, patch: &Patch) -> Result<Diff> {
        for (id, v) in &patch.based_on {
            let now = if *id == ROOT { Some(self.current.root.clone()) } else { self.find(*id) };
            if !now.is_some_and(|n| n.same_node(v)) {
                return Err(Error::new("doc.patch.conflict").arg("node", id.to_string()));
            }
        }
        self.transact(label, |e| patch.ops.iter().try_for_each(|op| e.apply(op)))
    }

    fn run(
        &mut self,
        origin: Origin,
        gesture: Option<GestureRecord>,
        gesture_mode: bool,
        edit: impl FnOnce(&mut Edit<'_>) -> Result<()>,
    ) -> Result<(Diff, Option<GestureRecord>)> {
        let env = Arc::clone(&self.env);
        let mut w = Work::new(Arc::clone(&env), self.current.clone(), self.issued);
        w.gesture = gesture;
        w.gesture_mode = gesture_mode;
        edit(&mut Edit { w: &mut w })?;
        run_rules(&env, &mut w, &[RuleKind::Reconcile, RuleKind::Check], &[])?;
        w.check()?;
        let mut state = std::mem::replace(&mut w.state, State { root: Value::NULL, index: Index::default() });
        let mut meta_op = None;
        if w.issued != self.issued && env.ctx.has_meta {
            let path = vec![PathStep::key("meta"), PathStep::key("issued")];
            let value = Value::int(i64::try_from(w.issued).map_err(|_| Error::new("doc.id.exhausted"))?);
            state.root = state.root.with_path(&path, value.clone())?;
            meta_op = Some(JournalOp::Set { path, value });
        }
        self.issued = w.issued;
        self.current = state;
        w.acc.set_origin(origin);
        if self.journal_on {
            self.journal.extend(w.ops.iter().cloned());
            self.journal.extend(meta_op);
        }
        self.pending.ops.append(&mut w.ops);
        self.pending.inverse.append(&mut w.inverse);
        self.pending.acc.absorb(&w.acc);
        let diff = w.acc.finish(origin);
        self.publish(&w.acc);
        Ok((diff, w.gesture.take()))
    }

    fn publish(&self, acc: &DiffAcc) {
        let mut feeds = self.feeds.borrow_mut();
        for key in self.feeds_dropped.borrow_mut().drain(..) {
            feeds.remove(&key);
        }
        for slot in feeds.values_mut() {
            match slot.topic {
                Topic::All => slot.acc.absorb(acc),
                Topic::Node(id) => slot.acc.absorb(&acc.filtered(&|n, _| n == id)),
                Topic::Type(t) => slot.acc.absorb(&acc.filtered(&|_, ty| ty == t)),
            }
        }
    }

    fn param_in(&self, state: &State, id: NodeId, key: &str) -> Option<f64> {
        let t = self.env.ctx.type_of(&state.index, id).ok()?;
        let path = self.env.ctx.path_of(&state.index, &state.root, id).ok()?;
        state.root.at_path(&path)?.at_path(&param_path(t, key)).and_then(Value::as_real)
    }

    /// Ends a history step with what is pending: the current state becomes a state that existed (I-3.7)
    fn push_step(&mut self, label: Label) -> Option<Diff> {
        let pending = std::mem::take(&mut self.pending);
        if pending.ops.is_empty() {
            return None;
        }
        self.steps_pushed += 1;
        let diff = pending.acc.finish(Origin::User);
        if self.history_limit == 0 {
            self.checkpoints = VecDeque::from([self.current.clone()]);
            self.cursor = 0;
            return Some(diff);
        }
        let before = &self.checkpoints[self.cursor];
        let inverse_acc = pending.acc.inverted(&|id, key| self.param_in(before, id, key));
        let mut inverse = pending.inverse;
        inverse.reverse();
        let step = Step { label, acc: pending.acc, inverse_acc, ops: pending.ops, inverse };
        self.steps.truncate(self.cursor);
        self.checkpoints.truncate(self.cursor + 1);
        self.steps.push_back(step);
        self.checkpoints.push_back(self.current.clone());
        self.cursor += 1;
        while self.steps.len() > self.history_limit {
            self.steps.pop_front();
            self.checkpoints.pop_front();
            self.cursor -= 1;
        }
        Some(diff)
    }

    /// The state of a step boundary, with the current `meta` (undo never restores it, R7)
    fn switch_to(&mut self, state: State) {
        let mut root = state.root;
        if self.env.ctx.has_meta
            && let Some(meta) = self.current.root.get("meta")
        {
            root = root.with("meta", meta.clone());
        }
        self.current = State { root, index: state.index };
    }

    // ---- history ----

    /// Undoes the last step: first every open gesture is committed (G2: the first undo in the middle of a drag
    /// undoes the drag), then the state goes back to the one before the step, with the step's stored inverse diff
    /// (U18: no roots compared)
    pub fn undo(&mut self) -> Result<Diff> {
        self.commit_all_gestures();
        if self.cursor == 0 {
            return Err(Error::new("doc.history.nothing_to_undo"));
        }
        let target = self.checkpoints[self.cursor - 1].clone();
        self.switch_to(target);
        let step = &self.steps[self.cursor - 1];
        if self.journal_on {
            self.journal.extend(step.inverse.iter().cloned());
        }
        let mut acc = step.inverse_acc.clone();
        acc.set_origin(Origin::Undo);
        self.cursor -= 1;
        self.publish(&acc);
        Ok(acc.finish(Origin::Undo))
    }

    pub fn redo(&mut self) -> Result<Diff> {
        self.commit_all_gestures();
        if self.cursor >= self.steps.len() {
            return Err(Error::new("doc.history.nothing_to_redo"));
        }
        let target = self.checkpoints[self.cursor + 1].clone();
        self.switch_to(target);
        let step = &self.steps[self.cursor];
        if self.journal_on {
            self.journal.extend(step.ops.iter().cloned());
        }
        let mut acc = step.acc.clone();
        acc.set_origin(Origin::Redo);
        self.cursor += 1;
        self.publish(&acc);
        Ok(acc.finish(Origin::Redo))
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0 || !self.gestures.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.steps.len()
    }

    pub fn undo_label(&self) -> Option<&Label> {
        self.cursor.checked_sub(1).map(|i| &self.steps[i].label)
    }

    pub fn redo_label(&self) -> Option<&Label> {
        self.steps.get(self.cursor).map(|s| &s.label)
    }

    /// Forgets every step, undoable and redoable: the state stays as it is (the files the steps could bring back
    /// are gone, H4). Open gestures are committed first
    pub fn clear_history(&mut self) {
        self.commit_all_gestures();
        self.checkpoints = VecDeque::from([self.current.clone()]);
        self.steps.clear();
        self.cursor = 0;
    }

    /// The number of steps kept (undoable and redoable)
    pub fn history_len(&self) -> usize {
        self.steps.len()
    }

    // ---- gestures ----

    /// Opens a gesture (I-3.7). Several may be open at once (T4)
    pub fn begin_gesture(&mut self, label: Label) -> GestureId {
        let raw = self.next_gesture;
        self.next_gesture += 1;
        self.gestures
            .insert(raw, Gesture { label, record: GestureRecord::default(), steps_at_begin: self.steps_pushed });
        GestureId { raw, dropped: Some(Rc::clone(&self.gestures_dropped)) }
    }

    /// A gesture's change: applied to the current root at once, without a history step. Only `set`, `set_param`,
    /// `set_note` and `move_to` (`doc.gesture.structural` otherwise). `doc.gesture.closed` after a commit, a cancel,
    /// or an undo that committed it (G2); `doc.gesture.target_gone` (and the gesture is closed without a step) when
    /// a node it changes left the tree (G3)
    pub fn gesture_update(&mut self, g: &GestureId, edit: impl FnOnce(&mut Edit<'_>) -> Result<()>) -> Result<Diff> {
        let gesture = self.gestures.get(&g.raw).ok_or_else(|| Error::new("doc.gesture.closed"))?;
        if gesture.record.targets.iter().any(|t| !self.current.index.contains(*t)) {
            self.gestures.remove(&g.raw);
            return Err(Error::new("doc.gesture.target_gone"));
        }
        let record = gesture.record.clone();
        let (diff, record) = self.run(Origin::Gesture, Some(record), true, edit)?;
        if let (Some(gesture), Some(record)) = (self.gestures.get_mut(&g.raw), record) {
            gesture.record = record;
        }
        Ok(diff)
    }

    /// Ends a gesture: its step enters the history now, with the current root. Gives the step's diff (what changed
    /// since the last step; empty if a foreign step already took everything)
    pub fn gesture_commit(&mut self, g: GestureId) -> Result<Diff> {
        let raw = g.disarm();
        self.commit_raw(raw)
    }

    fn commit_raw(&mut self, raw: u64) -> Result<Diff> {
        let gesture = self.gestures.remove(&raw).ok_or_else(|| Error::new("doc.gesture.closed"))?;
        let mut diff = self.push_step(gesture.label).unwrap_or_else(|| Diff::empty(Origin::Gesture));
        diff.origin = Origin::Gesture;
        Ok(diff)
    }

    /// G2: every open gesture is committed (its id becomes invalid); anything else still pending becomes a step,
    /// so that undo always starts from a state that existed
    fn commit_all_gestures(&mut self) {
        let open: Vec<u64> = self.gestures.keys().copied().collect();
        for raw in open {
            // A commit cannot fail once the gesture is open
            let _ = self.commit_raw(raw);
        }
        self.push_step(Label::new("doc.history.pending"));
    }

    /// Cancels a gesture (G1): only the paths it touched go back to their values before it, over the current root;
    /// foreign transactions and other gestures stay. If a foreign step entered the history meanwhile, the cancel is
    /// a step of its own ("doc.gesture.cancel"); otherwise there is no step
    pub fn gesture_cancel(&mut self, g: GestureId) -> Result<Diff> {
        let raw = g.disarm();
        let gesture = self.gestures.remove(&raw).ok_or_else(|| Error::new("doc.gesture.closed"))?;
        let foreign = self.steps_pushed != gesture.steps_at_begin;
        if !foreign && self.gestures.is_empty() {
            // Nothing else happened since the last step: back to it exactly
            let pending = std::mem::take(&mut self.pending);
            let target = self.checkpoints[self.cursor].clone();
            let mut acc = pending.acc.inverted(&|id, key| self.param_in(&target, id, key));
            self.switch_to(target);
            if self.journal_on {
                self.journal.extend(pending.inverse.into_iter().rev());
            }
            acc.set_origin(Origin::Gesture);
            self.publish(&acc);
            return Ok(acc.finish(Origin::Gesture));
        }
        let record = gesture.record;
        let result = self.run(Origin::Gesture, None, false, |e| e.restore(&record));
        let cancel_label = Label::new("doc.gesture.cancel").arg("label", gesture.label.key);
        match result {
            Ok((diff, _)) => {
                if foreign {
                    self.push_step(cancel_label);
                }
                Ok(diff)
            }
            Err(e) => {
                if foreign {
                    self.push_step(gesture.label);
                }
                Err(Error::new("doc.gesture.cancel_failed").arg("error", e.to_string()))
            }
        }
    }

    /// The open gestures
    pub fn open_gestures(&self) -> usize {
        self.gestures.len()
    }

    /// What the session's `pump` calls: commits the gestures whose ids were dropped (T4) and forgets the feeds that
    /// were dropped (T13)
    pub fn deliver(&mut self) -> Delivered {
        let mut out = Delivered::default();
        let dropped: Vec<u64> = self.gestures_dropped.borrow_mut().drain(..).collect();
        for raw in dropped {
            if self.gestures.contains_key(&raw) {
                out.committed.push((raw, self.commit_raw(raw)));
            }
        }
        let mut feeds = self.feeds.borrow_mut();
        for key in self.feeds_dropped.borrow_mut().drain(..) {
            if feeds.remove(&key).is_some() {
                out.forgotten_feeds += 1;
            }
        }
        out
    }

    // ---- feeds ----

    /// A feed of the diffs of a topic, from now on
    pub fn feed(&mut self, topic: Topic) -> Feed {
        let key = self.next_feed;
        self.next_feed += 1;
        self.feeds.borrow_mut().insert(key, FeedSlot { topic, acc: DiffAcc::default() });
        Feed { doc: self.serial, key, dropped: Rc::clone(&self.feeds_dropped) }
    }

    /// The feeds alive (dropped ones are forgotten at the next change or [`Document::deliver`])
    pub fn feed_count(&self) -> usize {
        self.feeds.borrow().len()
    }

    // ---- mirror (I-15) ----

    /// Makes this document a mirror's next state: the root becomes `root`, and `diff` — the diff the original
    /// published for that change — reaches this document's feeds as it is (the RemotePort's feeds give the same
    /// diffs as the core's). With `None` (a resync after a reconnection) the diff is worked out by id: the nodes that
    /// appeared, the ones that went, and the ones whose value differs, as wholly changed. No rule runs; nothing enters
    /// the history nor the journal. Keys: those of building the index (`doc.id.duplicate`...)
    pub fn mirror(&mut self, root: Value, diff: Option<&Diff>) -> Result<Diff> {
        let index = self.env.ctx.rebuild(&root)?;
        let old = std::mem::replace(&mut self.current, State { root, index });
        let new = &self.current;
        let type_in = |state: &State, id: NodeId| self.env.ctx.type_of(&state.index, id).unwrap_or("");
        let worked_out;
        let diff = match diff {
            Some(d) => d,
            None => {
                let mut d = Diff::empty(Origin::Journal);
                for (id, _) in new.index.ids() {
                    if !old.index.contains(*id) {
                        d.created.push(*id);
                    }
                }
                for (id, _) in old.index.ids() {
                    if !new.index.contains(*id) {
                        d.removed.push(*id);
                    } else {
                        let before = self
                            .env
                            .ctx
                            .path_of(&old.index, &old.root, *id)
                            .ok()
                            .and_then(|p| old.root.at_path(&p).cloned());
                        let now = self
                            .env
                            .ctx
                            .path_of(&new.index, &new.root, *id)
                            .ok()
                            .and_then(|p| new.root.at_path(&p).cloned());
                        if !matches!((&before, &now), (Some(a), Some(b)) if a.same_node(b) || a == b) {
                            d.changed.push(crate::Changed {
                                node: *id,
                                paths: vec![Vec::new()],
                                notes: Default::default(),
                            });
                        }
                    }
                }
                d.structural = true;
                worked_out = d;
                &worked_out
            }
        };
        let acc = DiffAcc::from_diff(diff, &|id, before| type_in(if before { &old } else { new }, id));
        if let Some(issued) = new.root.get("meta").and_then(|m| m.get("issued")).and_then(Value::as_int) {
            self.issued = u64::try_from(issued).unwrap_or(self.issued);
        }
        self.publish(&acc);
        Ok(acc.finish(diff.origin))
    }

    // ---- journal ----

    /// The changes to the root since the last call, in order (transactions, gesture updates, undo and redo, the
    /// counter in `meta`), for one journal line; `None` if nothing changed. The session numbers them (`seq`) and
    /// keeps the base hash; it calls this after each step, and every second while a gesture is open (T11)
    pub fn take_journal(&mut self) -> Option<JournalPatch> {
        if self.journal.is_empty() { None } else { Some(JournalPatch { ops: std::mem::take(&mut self.journal) }) }
    }
}

/// The upgrade of a saved tree, top-down: a node is upgraded first, then its fields are walked by its current schema
struct Upgrader<'a> {
    registry: &'a SchemaRegistry,
    from: &'a BTreeMap<String, u32>,
    /// Some type is older than the program's: every node is visited
    outdated: bool,
    upgraded: bool,
    read_only: bool,
    notices: Vec<Label>,
    /// The next id number to issue to a node a migration makes (`crate::migrate`)
    next_id: u64,
}

/// Deeper than the reader accepts (`lm_state::MAX_DEPTH`): a tree this deep is not a project
const MAX_UPGRADE_DEPTH: usize = 256;

impl Upgrader<'_> {
    fn value(&mut self, ctx: &Ctx, kind: &FieldKind, v: Value, depth: usize) -> Result<Value> {
        if depth > MAX_UPGRADE_DEPTH {
            return Err(Error::new("state.json.too_deep"));
        }
        if v.is_null() || !(self.outdated || ctx.is_deep(kind)) {
            return Ok(v);
        }
        match kind {
            FieldKind::List(inner) => {
                let mut items = Vec::with_capacity(v.len());
                for item in v.items() {
                    items.push(self.value(ctx, inner, item.clone(), depth + 1)?);
                }
                Ok(if items.iter().zip(v.items()).all(|(a, b)| a.same_node(b)) { v } else { Value::list(items) })
            }
            FieldKind::NoteMap(t) => {
                let inner = FieldKind::Object(t);
                let mut out = v.clone();
                for (k, note) in v.shared_entries() {
                    let new = self.value(ctx, &inner, note.clone(), depth + 1)?;
                    out = out.with_key(Arc::clone(k), new);
                }
                Ok(out)
            }
            FieldKind::Object(t) => {
                let Some(schema) = self.registry.find(t) else { return Ok(v) };
                let mut node = v;
                if let Some(from) = self.from.get(*t).copied()
                    && from < schema.version
                {
                    node = self.registry.upgrade(t, from, node)?;
                    self.upgraded = true;
                }
                if *t == types::TRACK {
                    // (before its instrument is upgraded: the old state still has the chord and the arpeggio)
                    node = crate::migrate::note_effects_from_instrument(self.registry, node, &mut self.next_id)?;
                }
                if *t == types::PLUGIN {
                    node = self.plugin(node)?;
                }
                for field in &schema.fields {
                    if let Some(child) = node.get(field.key).cloned() {
                        let new = self.value(ctx, &field.kind, child, depth + 1)?;
                        node = node.with(field.key, new);
                    }
                }
                Ok(node)
            }
            _ => Ok(v),
        }
    }

    /// A plugin's state, at its plugin's version
    fn plugin(&mut self, node: Value) -> Result<Value> {
        let name = node.get("plugin").and_then(Value::as_text).unwrap_or("").to_owned();
        let Some(schema) = self.registry.find(&plugin_type(&name)) else { return Ok(node) };
        let version = node.get("version").and_then(Value::as_int).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
        if version > schema.version {
            self.read_only = true;
            self.notices
                .push(Label::new("doc.load.newer").arg("type", schema.type_name).arg("version", i64::from(version)));
            return Ok(node);
        }
        if version == schema.version {
            return Ok(node);
        }
        let state = node.get("state").cloned().unwrap_or_default();
        let state = self.registry.upgrade(schema.type_name, version, state)?;
        self.upgraded = true;
        Ok(node.with("state", state).with("version", Value::int(i64::from(schema.version))))
    }
}

/// The number after the highest one a file uses. The file's numbers are not ours: one at the end of the range would
/// wrap the counter (new ids colliding with old ones) or not fit in the saved counter, so such a file is refused
fn first_free(highest: u64) -> Result<u64> {
    highest
        .checked_add(1)
        .filter(|n| i64::try_from(*n).is_ok_and(|n| n <= lm_state::MAX_SAFE_INT))
        .ok_or_else(|| Error::new("doc.id.exhausted"))
}

#[cfg(test)]
mod id_range_tests {
    use super::first_free;

    #[test]
    fn ids_at_the_end_of_the_range_are_refused() {
        assert_eq!(first_free(0).unwrap(), 1);
        assert_eq!(first_free(41).unwrap(), 42);
        assert!(first_free(u64::MAX).is_err());
        assert!(first_free(lm_state::MAX_SAFE_INT as u64).is_err());
    }
}
