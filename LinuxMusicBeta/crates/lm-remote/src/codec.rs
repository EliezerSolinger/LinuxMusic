//! The port's types as `Value`s (the JSON of the protocol), both ways. Every decoder checks what it reads and
//! answers `remote.decode` with the field at fault: nothing a peer sends can panic it (I-20.6).
//!
//! Translation keys arrive as text but the core's labels hold `&'static str`: they are interned in a bounded table
//! (keys are a finite set; a peer that sends too many different ones gets `remote.unknown_key` for the rest).

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use lm_base::{Arg, Error, Label, OperationId, Result};
use lm_doc::{Changed, Diff, NoteChanges, Origin};
use lm_port::{
    Applied, BrowseEntry, BrowsePage, DeviceInfo, EditOp, EntryId, EntryKind, Loading, NoteRef, Notice, OperationInfo,
    Preview, Ref, ScanKind, Severity, Target,
};
use lm_state::{NodeId, PathStep, Resource, Value};

/// Keys longer than this, or past this many different ones, are not interned
const MAX_KEY: usize = 160;
const MAX_KEYS: usize = 16_384;

/// A `&'static str` for a key received from the network (bounded, see the module's docs)
pub fn intern(key: &str) -> &'static str {
    static TABLE: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    if key.len() > MAX_KEY {
        return "remote.unknown_key";
    }
    let mut t = TABLE.get_or_init(|| Mutex::new(HashSet::new())).lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(k) = t.get(key) {
        return k;
    }
    if t.len() >= MAX_KEYS {
        return "remote.unknown_key";
    }
    let k: &'static str = Box::leak(key.to_owned().into_boxed_str());
    t.insert(k);
    k
}

pub(crate) fn bad(what: &str) -> Error {
    Error::new("remote.decode").arg("field", what)
}

// ---------------------------------------------------------------------------------------------------------------
// Reading helpers

pub(crate) fn get<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key).ok_or_else(|| bad(key))
}

pub(crate) fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    get(v, key)?.as_text().ok_or_else(|| bad(key))
}

pub(crate) fn int(v: &Value, key: &str) -> Result<i64> {
    get(v, key)?.as_int().ok_or_else(|| bad(key))
}

pub(crate) fn uint(v: &Value, key: &str) -> Result<u64> {
    u64::try_from(int(v, key)?).map_err(|_| bad(key))
}

pub(crate) fn real(v: &Value, key: &str) -> Result<f64> {
    let x = get(v, key)?;
    x.as_real().or_else(|| x.as_int().map(|i| i as f64)).ok_or_else(|| bad(key))
}

pub(crate) fn flag(v: &Value, key: &str) -> Result<bool> {
    get(v, key)?.as_bool().ok_or_else(|| bad(key))
}

pub(crate) fn list<'a>(v: &'a Value, key: &str) -> Result<impl Iterator<Item = &'a Value> + 'a> {
    let l = get(v, key)?;
    if l.is_list() { Ok(l.items()) } else { Err(bad(key)) }
}

pub(crate) fn opt<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).filter(|x| !x.is_null())
}

pub(crate) fn int_value(i: u64) -> Result<Value> {
    i64::try_from(i).map(Value::int).map_err(|_| Error::new("remote.encode.int"))
}

fn usize_value(i: usize) -> Result<Value> {
    int_value(i as u64)
}

pub(crate) fn real_value(x: f64) -> Value {
    Value::real(x).unwrap_or(Value::NULL)
}

// ---------------------------------------------------------------------------------------------------------------
// Ids, paths, labels, errors

pub fn id_to(id: NodeId) -> Value {
    Value::text(id.to_string())
}

pub fn id_from(v: &Value) -> Result<NodeId> {
    NodeId::parse(v.as_text().ok_or_else(|| bad("id"))?).map_err(|_| bad("id"))
}

pub fn path_to(path: &[PathStep]) -> Result<Value> {
    let mut out = Vec::with_capacity(path.len());
    for s in path {
        out.push(match s {
            PathStep::Key(k) => Value::text(Arc::clone(k)),
            PathStep::Index(i) => usize_value(*i)?,
            PathStep::Note(n) => Value::object([("note", int_value(*n)?)]),
        });
    }
    Ok(Value::list(out))
}

pub fn path_from(v: &Value) -> Result<Vec<PathStep>> {
    if !v.is_list() {
        return Err(bad("path"));
    }
    v.items()
        .map(|s| {
            if let Some(k) = s.as_shared_text() {
                Ok(PathStep::Key(Arc::clone(k)))
            } else if let Some(i) = s.as_int() {
                usize::try_from(i).map(PathStep::Index).map_err(|_| bad("path"))
            } else {
                Ok(PathStep::Note(uint(s, "note")?))
            }
        })
        .collect()
}

pub fn label_to(l: &Label) -> Value {
    let args = l.args.iter().map(|(name, a)| {
        let v = match a {
            Arg::Int(i) => Value::int(*i),
            Arg::Real(x) => real_value(*x),
            Arg::Text(t) => Value::text(t.as_str()),
        };
        Value::list([Value::text(*name), v])
    });
    Value::object([("key", Value::text(l.key)), ("args", Value::list(args))])
}

pub fn label_from(v: &Value) -> Result<Label> {
    let mut l = Label::new(intern(text(v, "key")?));
    for a in list(v, "args")? {
        let (Some(name), Some(x)) = (a.at(0).and_then(Value::as_text), a.at(1)) else { return Err(bad("args")) };
        let arg = if let Some(i) = x.as_int() {
            Arg::Int(i)
        } else if let Some(r) = x.as_real() {
            Arg::Real(r)
        } else if let Some(t) = x.as_text() {
            Arg::Text(t.to_owned())
        } else {
            return Err(bad("args"));
        };
        l.args.push((intern(name), arg));
    }
    Ok(l)
}

pub fn error_to(e: &Error) -> Value {
    label_to(&e.label)
}

pub fn error_from(v: &Value) -> Result<Error> {
    Ok(Error { label: label_from(v)? })
}

pub fn result_to(r: &Result<Value>) -> Value {
    match r {
        Ok(v) => Value::object([("ok", v.clone())]),
        Err(e) => Value::object([("err", error_to(e))]),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Diffs

fn origin_key(o: Origin) -> &'static str {
    match o {
        Origin::User => "user",
        Origin::Undo => "undo",
        Origin::Redo => "redo",
        Origin::Gesture => "gesture",
        Origin::Plugin => "plugin",
        Origin::Recording => "recording",
        Origin::Journal => "journal",
    }
}

fn origin_from(t: &str) -> Result<Origin> {
    Ok(match t {
        "user" => Origin::User,
        "undo" => Origin::Undo,
        "redo" => Origin::Redo,
        "gesture" => Origin::Gesture,
        "plugin" => Origin::Plugin,
        "recording" => Origin::Recording,
        "journal" => Origin::Journal,
        _ => return Err(bad("origin")),
    })
}

fn ids_to(ids: &[NodeId]) -> Value {
    Value::list(ids.iter().map(|i| id_to(*i)))
}

fn ids_from(v: &Value, key: &str) -> Result<Vec<NodeId>> {
    list(v, key)?.map(id_from).collect()
}

fn ns_to(ns: &[u64]) -> Result<Value> {
    Ok(Value::list(ns.iter().map(|n| int_value(*n)).collect::<Result<Vec<_>>>()?))
}

fn ns_from(v: &Value, key: &str) -> Result<Vec<u64>> {
    list(v, key)?.map(|n| n.as_int().and_then(|i| u64::try_from(i).ok()).ok_or_else(|| bad(key))).collect()
}

pub fn diff_to(d: &Diff) -> Result<Value> {
    let changed = d
        .changed
        .iter()
        .map(|c| {
            Ok(Value::object([
                ("node", id_to(c.node)),
                ("paths", Value::list(c.paths.iter().map(|p| path_to(p)).collect::<Result<Vec<_>>>()?)),
                (
                    "notes",
                    Value::object([
                        ("created", ns_to(&c.notes.created)?),
                        ("changed", ns_to(&c.notes.changed)?),
                        ("removed", ns_to(&c.notes.removed)?),
                    ]),
                ),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    let params =
        d.params.iter().map(|(id, k, v)| Value::list([id_to(*id), Value::text(Arc::clone(k)), real_value(*v)]));
    Ok(Value::object([
        ("created", ids_to(&d.created)),
        ("removed", ids_to(&d.removed)),
        ("changed", Value::list(changed)),
        ("structural", Value::bool(d.structural)),
        ("params", Value::list(params)),
        ("origin", Value::text(origin_key(d.origin))),
        ("notices", Value::list(d.notices.iter().map(label_to))),
    ]))
}

pub fn diff_from(v: &Value) -> Result<Diff> {
    let mut d = Diff::empty(origin_from(text(v, "origin")?)?);
    d.created = ids_from(v, "created")?;
    d.removed = ids_from(v, "removed")?;
    for c in list(v, "changed")? {
        let notes = get(c, "notes")?;
        d.changed.push(Changed {
            node: id_from(get(c, "node")?)?,
            paths: list(c, "paths")?.map(path_from).collect::<Result<_>>()?,
            notes: NoteChanges {
                created: ns_from(notes, "created")?,
                changed: ns_from(notes, "changed")?,
                removed: ns_from(notes, "removed")?,
            },
        });
    }
    d.structural = flag(v, "structural")?;
    for p in list(v, "params")? {
        let (Some(id), Some(k), Some(x)) = (p.at(0), p.at(1).and_then(Value::as_shared_text), p.at(2)) else {
            return Err(bad("params"));
        };
        d.params.push((
            id_from(id)?,
            Arc::clone(k),
            x.as_real().or_else(|| x.as_int().map(|i| i as f64)).ok_or_else(|| bad("params"))?,
        ));
    }
    d.notices = list(v, "notices")?.map(label_from).collect::<Result<_>>()?;
    Ok(d)
}

// ---------------------------------------------------------------------------------------------------------------
// Edits

fn ref_to(r: Ref) -> Value {
    match r {
        Ref::Id(id) => Value::object([("id", id_to(id))]),
        Ref::Temp(t) => Value::object([("temp", Value::int(i64::from(t)))]),
    }
}

fn temp(v: &Value, key: &str) -> Result<u32> {
    u32::try_from(int(v, key)?).map_err(|_| bad(key))
}

fn ref_from(v: &Value) -> Result<Ref> {
    match v.get("id") {
        Some(id) => Ok(Ref::Id(id_from(id)?)),
        None => Ok(Ref::Temp(temp(v, "temp")?)),
    }
}

fn note_ref_to(n: NoteRef) -> Result<Value> {
    Ok(match n {
        NoteRef::N(n) => Value::object([("n", int_value(n)?)]),
        NoteRef::Temp(t) => Value::object([("temp", Value::int(i64::from(t)))]),
    })
}

fn note_ref_from(v: &Value) -> Result<NoteRef> {
    match v.get("n") {
        Some(_) => Ok(NoteRef::N(uint(v, "n")?)),
        None => Ok(NoteRef::Temp(temp(v, "temp")?)),
    }
}

pub fn op_to(op: &EditOp) -> Result<Value> {
    Ok(match op {
        EditOp::Set { id, path, value } => Value::object([
            ("op", Value::text("set")),
            ("id", ref_to(*id)),
            ("path", path_to(path)?),
            ("value", value.clone()),
        ]),
        EditOp::Insert { temp, parent, list, index, node } => Value::object([
            ("op", Value::text("insert")),
            ("temp", Value::int(i64::from(*temp))),
            ("parent", ref_to(*parent)),
            ("list", Value::text(list.as_str())),
            ("index", usize_value(*index)?),
            ("node", node.clone()),
        ]),
        EditOp::Remove { id } => Value::object([("op", Value::text("remove")), ("id", ref_to(*id))]),
        EditOp::Move { id, parent, list, index } => Value::object([
            ("op", Value::text("move")),
            ("id", ref_to(*id)),
            ("parent", ref_to(*parent)),
            ("list", Value::text(list.as_str())),
            ("index", usize_value(*index)?),
        ]),
        EditOp::AddNote { temp, item, note } => Value::object([
            ("op", Value::text("add_note")),
            ("temp", Value::int(i64::from(*temp))),
            ("item", ref_to(*item)),
            ("note", note.clone()),
        ]),
        EditOp::SetNote { item, n, path, value } => Value::object([
            ("op", Value::text("set_note")),
            ("item", ref_to(*item)),
            ("n", note_ref_to(*n)?),
            ("path", path_to(path)?),
            ("value", value.clone()),
        ]),
        EditOp::RemoveNote { item, n } => {
            Value::object([("op", Value::text("remove_note")), ("item", ref_to(*item)), ("n", note_ref_to(*n)?)])
        }
    })
}

fn index(v: &Value, key: &str) -> Result<usize> {
    usize::try_from(int(v, key)?).map_err(|_| bad(key))
}

pub fn op_from(v: &Value) -> Result<EditOp> {
    Ok(match text(v, "op")? {
        "set" => EditOp::Set {
            id: ref_from(get(v, "id")?)?,
            path: path_from(get(v, "path")?)?,
            value: get(v, "value")?.clone(),
        },
        "insert" => EditOp::Insert {
            temp: temp(v, "temp")?,
            parent: ref_from(get(v, "parent")?)?,
            list: text(v, "list")?.to_owned(),
            index: index(v, "index")?,
            node: get(v, "node")?.clone(),
        },
        "remove" => EditOp::Remove { id: ref_from(get(v, "id")?)? },
        "move" => EditOp::Move {
            id: ref_from(get(v, "id")?)?,
            parent: ref_from(get(v, "parent")?)?,
            list: text(v, "list")?.to_owned(),
            index: index(v, "index")?,
        },
        "add_note" => {
            EditOp::AddNote { temp: temp(v, "temp")?, item: ref_from(get(v, "item")?)?, note: get(v, "note")?.clone() }
        }
        "set_note" => EditOp::SetNote {
            item: ref_from(get(v, "item")?)?,
            n: note_ref_from(get(v, "n")?)?,
            path: path_from(get(v, "path")?)?,
            value: get(v, "value")?.clone(),
        },
        "remove_note" => EditOp::RemoveNote { item: ref_from(get(v, "item")?)?, n: note_ref_from(get(v, "n")?)? },
        _ => return Err(bad("op")),
    })
}

pub fn ops_to(ops: &[EditOp]) -> Result<Value> {
    Ok(Value::list(ops.iter().map(op_to).collect::<Result<Vec<_>>>()?))
}

pub fn ops_from(v: &Value) -> Result<Vec<EditOp>> {
    if !v.is_list() {
        return Err(bad("ops"));
    }
    v.items().map(op_from).collect()
}

pub fn applied_to(a: &Applied) -> Result<Value> {
    let pairs =
        |p: &[(u32, Value)]| Value::list(p.iter().map(|(t, v)| Value::list([Value::int(i64::from(*t)), v.clone()])));
    let created: Vec<(u32, Value)> = a.created.iter().map(|(t, id)| (*t, id_to(*id))).collect();
    let notes: Vec<(u32, Value)> = a.notes.iter().map(|(t, n)| Ok((*t, int_value(*n)?))).collect::<Result<_>>()?;
    Ok(Value::object([
        (
            "diff",
            match &a.diff {
                Some(d) => diff_to(d)?,
                None => Value::NULL,
            },
        ),
        ("created", pairs(&created)),
        ("notes", pairs(&notes)),
    ]))
}

pub fn applied_from(v: &Value) -> Result<Applied> {
    fn pair(p: &Value) -> Result<(u32, &Value)> {
        let t = p.at(0).and_then(Value::as_int).and_then(|t| u32::try_from(t).ok()).ok_or_else(|| bad("created"))?;
        Ok((t, p.at(1).ok_or_else(|| bad("created"))?))
    }
    Ok(Applied {
        diff: opt(v, "diff").map(diff_from).transpose()?,
        created: list(v, "created")?.map(|p| pair(p).and_then(|(t, x)| Ok((t, id_from(x)?)))).collect::<Result<_>>()?,
        notes: list(v, "notes")?
            .map(|p| {
                pair(p).and_then(|(t, x)| {
                    Ok((t, x.as_int().and_then(|n| u64::try_from(n).ok()).ok_or_else(|| bad("notes"))?))
                })
            })
            .collect::<Result<_>>()?,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Targets, loadings, operations, notices

pub fn resource_to(r: &Resource) -> Value {
    Value::resource(r.clone())
}

pub fn resource_from(v: &Value) -> Result<Resource> {
    v.as_resource().cloned().ok_or_else(|| bad("resource"))
}

fn scan_key(s: ScanKind) -> &'static str {
    match s {
        ScanKind::MidiPorts => "midi_ports",
        ScanKind::Plugins => "plugins",
        ScanKind::Library => "library",
    }
}

pub fn target_to(t: &Target) -> Result<Value> {
    Ok(match t {
        Target::Project => Value::object([("kind", Value::text("project"))]),
        Target::Node(id) => Value::object([("kind", Value::text("node")), ("id", id_to(*id))]),
        Target::Resource(r) => Value::object([("kind", Value::text("resource")), ("resource", resource_to(r))]),
        Target::Device => Value::object([("kind", Value::text("device"))]),
        Target::Export(n) => Value::object([("kind", Value::text("export")), ("n", int_value(*n)?)]),
        Target::Scan(s) => Value::object([("kind", Value::text("scan")), ("scan", Value::text(scan_key(*s)))]),
    })
}

pub fn target_from(v: &Value) -> Result<Target> {
    Ok(match text(v, "kind")? {
        "project" => Target::Project,
        "node" => Target::Node(id_from(get(v, "id")?)?),
        "resource" => Target::Resource(resource_from(get(v, "resource")?)?),
        "device" => Target::Device,
        "export" => Target::Export(uint(v, "n")?),
        "scan" => Target::Scan(match text(v, "scan")? {
            "midi_ports" => ScanKind::MidiPorts,
            "plugins" => ScanKind::Plugins,
            "library" => ScanKind::Library,
            _ => return Err(bad("scan")),
        }),
        _ => return Err(bad("kind")),
    })
}

fn millis(d: Duration) -> Value {
    Value::int(i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn duration(v: &Value, key: &str) -> Result<Duration> {
    Ok(Duration::from_millis(uint(v, key)?))
}

fn opt_real(x: Option<f32>) -> Value {
    x.map_or(Value::NULL, |x| real_value(f64::from(x)))
}

fn opt_label(l: &Option<Label>) -> Value {
    l.as_ref().map_or(Value::NULL, label_to)
}

pub fn loading_to(l: &Loading) -> Value {
    match l {
        Loading::Ready => Value::object([("state", Value::text("ready"))]),
        Loading::Pending => Value::object([("state", Value::text("pending"))]),
        Loading::Visible { progress, stage, cancellable, since } => Value::object([
            ("state", Value::text("visible")),
            ("progress", opt_real(*progress)),
            ("stage", opt_label(stage)),
            ("cancellable", Value::bool(*cancellable)),
            ("since", millis(*since)),
        ]),
        Loading::Failed(e) => Value::object([("state", Value::text("failed")), ("error", error_to(e))]),
        Loading::Missing { resource } => {
            Value::object([("state", Value::text("missing")), ("resource", resource_to(resource))])
        }
    }
}

fn opt_f32(v: &Value, key: &str) -> Result<Option<f32>> {
    match opt(v, key) {
        None => Ok(None),
        Some(x) => Ok(Some(x.as_real().or_else(|| x.as_int().map(|i| i as f64)).ok_or_else(|| bad(key))? as f32)),
    }
}

pub fn loading_from(v: &Value) -> Result<Loading> {
    Ok(match text(v, "state")? {
        "ready" => Loading::Ready,
        "pending" => Loading::Pending,
        "visible" => Loading::Visible {
            progress: opt_f32(v, "progress")?,
            stage: opt(v, "stage").map(label_from).transpose()?,
            cancellable: flag(v, "cancellable")?,
            since: duration(v, "since")?,
        },
        "failed" => Loading::Failed(error_from(get(v, "error")?)?),
        "missing" => Loading::Missing { resource: resource_from(get(v, "resource")?)? },
        _ => return Err(bad("state")),
    })
}

pub fn operation_to(o: &OperationInfo) -> Result<Value> {
    Ok(Value::object([
        ("id", int_value(o.id.0)?),
        ("label", label_to(&o.label)),
        ("target", target_to(&o.target)?),
        ("progress", opt_real(o.progress)),
        ("stage", opt_label(&o.stage)),
        ("cancellable", Value::bool(o.cancellable)),
        ("since", millis(o.since)),
    ]))
}

pub fn operation_from(v: &Value) -> Result<OperationInfo> {
    Ok(OperationInfo {
        id: OperationId(uint(v, "id")?),
        label: label_from(get(v, "label")?)?,
        target: target_from(get(v, "target")?)?,
        progress: opt_f32(v, "progress")?,
        stage: opt(v, "stage").map(label_from).transpose()?,
        cancellable: flag(v, "cancellable")?,
        since: duration(v, "since")?,
    })
}

fn severity_key(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

pub fn notice_to(n: &Notice) -> Result<Value> {
    Ok(Value::object([
        ("id", int_value(n.id)?),
        ("key", Value::text(n.key)),
        ("args", Value::list(n.args.iter().map(|(k, v)| Value::list([Value::text(*k), v.clone()])))),
        ("severity", Value::text(severity_key(n.severity))),
        (
            "target",
            match &n.target {
                Some(t) => target_to(t)?,
                None => Value::NULL,
            },
        ),
        ("actions", Value::list(n.actions.iter().map(|(a, p)| Value::list([Value::text(a.as_str()), p.clone()])))),
        ("repeats", Value::int(i64::from(n.repeats))),
        ("read", Value::bool(n.read)),
    ]))
}

pub fn notice_from(v: &Value) -> Result<Notice> {
    let mut args = Vec::new();
    for a in list(v, "args")? {
        let (Some(k), Some(x)) = (a.at(0).and_then(Value::as_text), a.at(1)) else { return Err(bad("args")) };
        args.push((intern(k), x.clone()));
    }
    let mut actions = Vec::new();
    for a in list(v, "actions")? {
        let (Some(k), Some(x)) = (a.at(0).and_then(Value::as_text), a.at(1)) else { return Err(bad("actions")) };
        actions.push((lm_port::ActionId::from(k), x.clone()));
    }
    Ok(Notice {
        id: uint(v, "id")?,
        key: intern(text(v, "key")?),
        args,
        severity: match text(v, "severity")? {
            "info" => Severity::Info,
            "warning" => Severity::Warning,
            "error" => Severity::Error,
            _ => return Err(bad("severity")),
        },
        target: opt(v, "target").map(target_from).transpose()?,
        actions,
        repeats: u32::try_from(int(v, "repeats")?).map_err(|_| bad("repeats"))?,
        read: flag(v, "read")?,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Devices and the browser

pub fn device_to(d: &DeviceInfo) -> Value {
    Value::object([
        ("id", Value::text(Arc::clone(&d.id))),
        ("name", Value::text(Arc::clone(&d.name))),
        ("inputs", Value::int(i64::from(d.inputs))),
        ("outputs", Value::int(i64::from(d.outputs))),
        ("rates", Value::list(d.rates.iter().map(|r| Value::int(i64::from(*r))))),
        ("details", d.details.clone()),
    ])
}

fn u32_of(v: &Value, key: &str) -> Result<u32> {
    u32::try_from(int(v, key)?).map_err(|_| bad(key))
}

pub fn device_from(v: &Value) -> Result<DeviceInfo> {
    Ok(DeviceInfo {
        id: Arc::from(text(v, "id")?),
        name: Arc::from(text(v, "name")?),
        inputs: u32_of(v, "inputs")?,
        outputs: u32_of(v, "outputs")?,
        rates: list(v, "rates")?
            .map(|r| r.as_int().and_then(|r| u32::try_from(r).ok()).ok_or_else(|| bad("rates")))
            .collect::<Result<_>>()?,
        details: get(v, "details")?.clone(),
    })
}

pub fn page_key(p: BrowsePage) -> &'static str {
    match p {
        BrowsePage::Instruments => "instruments",
        BrowsePage::Effects => "effects",
        BrowsePage::Samples => "samples",
        BrowsePage::Presets => "presets",
        BrowsePage::Projects => "projects",
        BrowsePage::ProjectFiles => "project_files",
        BrowsePage::Favorites => "favorites",
    }
}

pub fn page_from(t: &str) -> Result<BrowsePage> {
    Ok(match t {
        "instruments" => BrowsePage::Instruments,
        "effects" => BrowsePage::Effects,
        "samples" => BrowsePage::Samples,
        "presets" => BrowsePage::Presets,
        "projects" => BrowsePage::Projects,
        "project_files" => BrowsePage::ProjectFiles,
        "favorites" => BrowsePage::Favorites,
        _ => return Err(bad("page")),
    })
}

fn kind_key(k: EntryKind) -> &'static str {
    match k {
        EntryKind::Folder => "folder",
        EntryKind::Audio => "audio",
        EntryKind::Midi => "midi",
        EntryKind::Preset => "preset",
        EntryKind::Project => "project",
        EntryKind::Plugin => "plugin",
        EntryKind::Other => "other",
    }
}

pub fn entry_to(e: &BrowseEntry) -> Result<Value> {
    Ok(Value::object([
        ("id", int_value(e.id.0)?),
        (
            "parent",
            match e.parent {
                Some(p) => int_value(p.0)?,
                None => Value::NULL,
            },
        ),
        ("name", Value::text(Arc::clone(&e.name))),
        ("kind", Value::text(kind_key(e.kind))),
        ("factory", Value::bool(e.factory)),
        ("hidden", Value::bool(e.hidden)),
        ("favorite", Value::bool(e.favorite)),
        ("info", e.info.clone()),
    ]))
}

pub fn entry_from(v: &Value) -> Result<BrowseEntry> {
    Ok(BrowseEntry {
        id: EntryId(uint(v, "id")?),
        parent: match opt(v, "parent") {
            Some(_) => Some(EntryId(uint(v, "parent")?)),
            None => None,
        },
        name: Arc::from(text(v, "name")?),
        kind: match text(v, "kind")? {
            "folder" => EntryKind::Folder,
            "audio" => EntryKind::Audio,
            "midi" => EntryKind::Midi,
            "preset" => EntryKind::Preset,
            "project" => EntryKind::Project,
            "plugin" => EntryKind::Plugin,
            "other" => EntryKind::Other,
            _ => return Err(bad("kind")),
        },
        factory: flag(v, "factory")?,
        hidden: flag(v, "hidden")?,
        favorite: flag(v, "favorite")?,
        info: get(v, "info")?.clone(),
    })
}

pub fn entries_to(e: &[BrowseEntry]) -> Result<Value> {
    Ok(Value::list(e.iter().map(entry_to).collect::<Result<Vec<_>>>()?))
}

pub fn entries_from(v: &Value) -> Result<Vec<BrowseEntry>> {
    if !v.is_list() {
        return Err(bad("entries"));
    }
    v.items().map(entry_from).collect()
}

pub fn preview_to(p: &Preview) -> Result<Value> {
    Ok(Value::object([
        ("entry", int_value(p.entry.0)?),
        ("resource", resource_to(&p.resource)),
        ("length", Value::int(p.length)),
        ("rate", Value::int(i64::from(p.sample_rate))),
    ]))
}

pub fn preview_from(v: &Value) -> Result<Preview> {
    Ok(Preview {
        entry: EntryId(uint(v, "entry")?),
        resource: resource_from(get(v, "resource")?)?,
        length: int(v, "length")?,
        sample_rate: u32_of(v, "rate")?,
    })
}

// --- MIDI mapping and the interface document (I-21.2, I-3.8; additions of 20/11) -----------------------------------

/// A mapping's target, in the document's form (`midi.target`, the same as in a mapping node)
pub fn map_target_to(t: &lm_port::MapTarget) -> Value {
    let m = lm_port::Mapping {
        id: None,
        source: lm_port::DocSource {
            port: Arc::from(""),
            channel: None,
            message: lm_port::MidiMessage::Cc { number: 0 },
        },
        touch: None,
        target: t.clone(),
        transform: Default::default(),
        feedback: true,
        consume: true,
        feedback_to: None,
    };
    m.to_value().get("target").cloned().unwrap_or(Value::NULL)
}

pub fn map_target_from(v: &Value) -> Result<lm_port::MapTarget> {
    let source =
        lm_port::DocSource { port: Arc::from(""), channel: None, message: lm_port::MidiMessage::Cc { number: 0 } };
    let m = Value::object([("source", source.to_value()), ("target", v.clone())]);
    lm_port::Mapping::from_value(&m).map(|m| m.target)
}

pub fn scope_to(s: lm_port::MapScope) -> Value {
    Value::text(match s {
        lm_port::MapScope::Project => "project",
        lm_port::MapScope::Machine => "machine",
    })
}

pub fn scope_from(v: &Value) -> Result<lm_port::MapScope> {
    match v.as_text() {
        Some("project") => Ok(lm_port::MapScope::Project),
        Some("machine") => Ok(lm_port::MapScope::Machine),
        _ => Err(Error::new("remote.bad_message").arg("field", "scope")),
    }
}

pub fn ui_action_to(a: &lm_port::MidiUiAction) -> Value {
    Value::object([
        ("action", Value::text(a.action.as_str())),
        ("params", a.params.clone()),
        ("value", Value::real(f64::from(a.value)).unwrap_or(Value::int(0))),
    ])
}

pub fn ui_action_from(v: &Value) -> Result<lm_port::MidiUiAction> {
    let bad = |f: &'static str| Error::new("remote.bad_message").arg("field", f);
    Ok(lm_port::MidiUiAction {
        action: lm_port::ActionId::from(v.get("action").and_then(Value::as_text).ok_or_else(|| bad("action"))?),
        params: v.get("params").cloned().unwrap_or(Value::NULL),
        value: v.get("value").and_then(Value::as_real).unwrap_or(0.0) as f32,
    })
}

pub fn activity_to(a: &lm_port::MidiActivity) -> Value {
    let s = lm_port::DocSource { port: a.port.clone(), channel: Some(a.channel), message: a.message };
    Value::object([
        ("source", s.to_value()),
        ("value", Value::int(i64::from(a.value))),
        ("mapped", Value::bool(a.mapped)),
        ("time", Value::int(a.time.0 as i64)),
    ])
}

pub fn activity_from(v: &Value) -> Result<lm_port::MidiActivity> {
    let bad = |f: &'static str| Error::new("remote.bad_message").arg("field", f);
    let s = lm_port::DocSource::from_value(v.get("source").ok_or_else(|| bad("source"))?)?;
    Ok(lm_port::MidiActivity {
        port: s.port,
        channel: s.channel.unwrap_or(0),
        message: s.message,
        value: u16::try_from(v.get("value").and_then(Value::as_int).unwrap_or(0)).map_err(|_| bad("value"))?,
        mapped: v.get("mapped").and_then(Value::as_bool).unwrap_or(false),
        time: lm_base::HostTime(v.get("time").and_then(Value::as_int).unwrap_or(0) as u64),
    })
}

pub fn hand_won_to(h: &[(NodeId, Arc<str>)]) -> Value {
    Value::list(h.iter().map(|(n, k)| Value::list([Value::text(n.to_string()), Value::text(k.clone())])))
}

pub fn hand_won_from(v: &Value) -> Result<Vec<(NodeId, Arc<str>)>> {
    let bad = || Error::new("remote.bad_message").arg("field", "hand_won");
    v.items()
        .map(|p| {
            let n = p.at(0).and_then(Value::as_text).ok_or_else(bad)?;
            let k = p.at(1).and_then(Value::as_shared_text).ok_or_else(bad)?;
            Ok((NodeId::parse(n)?, k.clone()))
        })
        .collect()
}
