//! The immutable value tree (interfaces.md, I-3.1): everything that matters in the program is one of these.
//!
//! A [`Value`] is immutable and its clone is O(1). Lists are persistent vectors and objects are persistent maps with
//! their keys in sorted order (S14), both from `imbl`: a change makes a new value that shares every part that did
//! not change, so keeping every previous root (the undo history) costs only the paths that changed.
//!
//! There are no implicit conversions (R15): a value is made by [`Value::int`], [`Value::real`], [`Value::text`]...

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use imbl::{OrdMap, Vector};
use lm_base::{Error, Result};

use crate::id::{parse_canonical_u64, truncated};
use crate::resource::Resource;

/// The kind of a [`Value`]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Null,
    Bool,
    Int,
    Real,
    Text,
    Resource,
    List,
    Object,
}

impl Kind {
    /// A stable name, for error arguments
    pub fn name(self) -> &'static str {
        match self {
            Kind::Null => "null",
            Kind::Bool => "bool",
            Kind::Int => "int",
            Kind::Real => "real",
            Kind::Text => "text",
            Kind::Resource => "resource",
            Kind::List => "list",
            Kind::Object => "object",
        }
    }
}

/// The persistent map of an object: keys in sorted (byte) order
pub(crate) type Map = OrdMap<Arc<str>, Value>;
/// The persistent vector of a list
pub(crate) type List = Vector<Value>;

/// An immutable JSON-like tree node. Clone is O(1); every container is shared, never copied.
///
/// Reals are never NaN or infinite, and `-0.0` is kept. Equality (`==`) is exact: deep, and bit for bit for reals,
/// so `0.0 != -0.0`. [`Value::same_node`] is the O(1) identity the compiler and the diff use to skip what did not
/// change.
#[derive(Clone)]
pub struct Value(Repr);

#[derive(Clone)]
enum Repr {
    Null,
    Bool(bool),
    Int(i64),
    /// Never NaN nor ±Inf (checked by [`Value::real`])
    Real(f64),
    Text(Arc<str>),
    Resource(Arc<Resource>),
    List(Arc<List>),
    Object(Arc<Map>),
}

/// One step of a path into the tree
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum PathStep {
    /// A key of an object
    Key(Arc<str>),
    /// A position in a list
    Index(usize),
    /// A note of an item's note map, by its number `n` (C1, T12): the key is the canonical decimal text of `n`
    Note(u64),
}

impl PathStep {
    pub fn key(key: &str) -> Self {
        PathStep::Key(Arc::from(key))
    }
}

impl fmt::Debug for PathStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathStep::Key(k) => write!(f, "Key({k:?})"),
            PathStep::Index(i) => write!(f, "Index({i})"),
            PathStep::Note(n) => write!(f, "Note({n})"),
        }
    }
}

/// A path in its text form, for errors and logs: a JSON Pointer (RFC 6901), `/tracks/0/notes/812/key`. The empty
/// path is `""`
pub fn path_text(path: &[PathStep]) -> String {
    let mut out = String::new();
    for step in path {
        out.push('/');
        match step {
            PathStep::Key(k) => {
                for c in k.chars() {
                    match c {
                        '~' => out.push_str("~0"),
                        '/' => out.push_str("~1"),
                        c => out.push(c),
                    }
                }
            }
            PathStep::Index(i) => out.push_str(&i.to_string()),
            PathStep::Note(n) => out.push_str(&n.to_string()),
        }
    }
    out
}

/// The key of note `n` in a note map: its canonical decimal text
pub fn note_key(n: u64) -> Arc<str> {
    Arc::from(n.to_string().as_str())
}

/// The note number of a note-map key, if it is the canonical decimal text of a `u64`
pub fn parse_note_key(key: &str) -> Option<u64> {
    parse_canonical_u64(key)
}

/// The decimal text of `n` without allocating, for lookups
struct NoteKeyBuf {
    buf: [u8; 20],
    start: usize,
}

impl NoteKeyBuf {
    fn new(mut n: u64) -> Self {
        let mut buf = [0u8; 20];
        let mut start = buf.len();
        loop {
            start -= 1;
            // The remainder is below 10, so the narrowing is exact
            buf[start] = b'0' + u8::try_from(n % 10).unwrap_or(0);
            n /= 10;
            if n == 0 {
                break;
            }
        }
        Self { buf, start }
    }

    fn as_str(&self) -> &str {
        // Only ASCII digits were written
        std::str::from_utf8(&self.buf[self.start..]).unwrap_or("")
    }
}

impl Value {
    // ---- making values ----

    pub const NULL: Value = Value(Repr::Null);

    pub fn null() -> Value {
        Value::NULL
    }

    pub fn bool(v: bool) -> Value {
        Value(Repr::Bool(v))
    }

    /// Any `i64`. Note that the JSON writer refuses integers beyond ±(2^53 − 1), which JSON readers cannot keep
    /// exact (ids and note numbers are text for this reason)
    pub fn int(v: i64) -> Value {
        Value(Repr::Int(v))
    }

    /// A real: NaN and ±Inf are refused; `-0.0` is kept (R15)
    pub fn real(v: f64) -> Result<Value> {
        if v.is_finite() { Ok(Value(Repr::Real(v))) } else { Err(Error::new("state.real.not_finite")) }
    }

    pub fn text(v: impl Into<Arc<str>>) -> Value {
        Value(Repr::Text(v.into()))
    }

    pub fn resource(r: Resource) -> Value {
        Value(Repr::Resource(Arc::new(r)))
    }

    pub fn empty_list() -> Value {
        Value(Repr::List(Arc::new(List::new())))
    }

    pub fn empty_object() -> Value {
        Value(Repr::Object(Arc::new(Map::new())))
    }

    pub fn list(items: impl IntoIterator<Item = Value>) -> Value {
        Value(Repr::List(Arc::new(items.into_iter().collect())))
    }

    /// An object from key-value pairs. The keys end up sorted whatever the order given; a repeated key keeps the
    /// last value
    pub fn object<K: Into<Arc<str>>>(entries: impl IntoIterator<Item = (K, Value)>) -> Value {
        Value(Repr::Object(Arc::new(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())))
    }

    pub(crate) fn from_map(map: Map) -> Value {
        Value(Repr::Object(Arc::new(map)))
    }

    pub(crate) fn from_vector(list: List) -> Value {
        Value(Repr::List(Arc::new(list)))
    }

    // ---- reading ----

    pub fn kind(&self) -> Kind {
        match &self.0 {
            Repr::Null => Kind::Null,
            Repr::Bool(_) => Kind::Bool,
            Repr::Int(_) => Kind::Int,
            Repr::Real(_) => Kind::Real,
            Repr::Text(_) => Kind::Text,
            Repr::Resource(_) => Kind::Resource,
            Repr::List(_) => Kind::List,
            Repr::Object(_) => Kind::Object,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.0, Repr::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self.0 {
            Repr::Bool(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self.0 {
            Repr::Int(v) => Some(v),
            _ => None,
        }
    }

    /// Only a real: an int is not converted (no implicit conversions, R15)
    pub fn as_real(&self) -> Option<f64> {
        match self.0 {
            Repr::Real(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match &self.0 {
            Repr::Text(v) => Some(v),
            _ => None,
        }
    }

    /// The shared text, to keep it without copying
    pub fn as_shared_text(&self) -> Option<&Arc<str>> {
        match &self.0 {
            Repr::Text(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_resource(&self) -> Option<&Resource> {
        match &self.0 {
            Repr::Resource(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_list(&self) -> bool {
        matches!(self.0, Repr::List(_))
    }

    pub fn is_object(&self) -> bool {
        matches!(self.0, Repr::Object(_))
    }

    /// The number of items of a list or entries of an object; 0 for anything else
    pub fn len(&self) -> usize {
        match &self.0 {
            Repr::List(l) => l.len(),
            Repr::Object(m) => m.len(),
            _ => 0,
        }
    }

    /// True for an empty list or object, and for anything that is neither
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // ---- objects ----

    /// The value at `key`; `None` if there is no such key or this is not an object
    pub fn get(&self, key: &str) -> Option<&Value> {
        match &self.0 {
            Repr::Object(m) => m.get(key),
            _ => None,
        }
    }

    /// The note `n` of a note map (C1): O(log n), without allocating
    pub fn note(&self, n: u64) -> Option<&Value> {
        self.get(NoteKeyBuf::new(n).as_str())
    }

    /// A new object with `key` set to `v`, sharing everything else. A non-object is treated as an empty object.
    /// Setting a key to the very node it already holds returns this same node (nothing changed)
    pub fn with(&self, key: &str, v: Value) -> Value {
        match &self.0 {
            Repr::Object(m) => match m.get_key_value(key) {
                Some((_, old)) if old.same_node(&v) => self.clone(),
                Some((k, _)) => {
                    let k = Arc::clone(k);
                    Value::from_map(m.update(k, v))
                }
                None => Value::from_map(m.update(Arc::from(key), v)),
            },
            _ => Value::from_map(Map::unit(Arc::from(key), v)),
        }
    }

    /// Like [`Value::with`], with a key that is already shared
    pub fn with_key(&self, key: Arc<str>, v: Value) -> Value {
        match &self.0 {
            Repr::Object(m) => match m.get(&key) {
                Some(old) if old.same_node(&v) => self.clone(),
                _ => Value::from_map(m.update(key, v)),
            },
            _ => Value::from_map(Map::unit(key, v)),
        }
    }

    /// A new object without `key`. If the key is absent, or this is not an object, the same node comes back
    pub fn without(&self, key: &str) -> Value {
        match &self.0 {
            Repr::Object(m) if m.contains_key(key) => Value::from_map(m.without(key)),
            _ => self.clone(),
        }
    }

    /// The keys of an object, in sorted order (empty for anything else)
    pub fn keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.entries().map(|(k, _)| k)
    }

    /// The entries of an object, in key order (empty for anything else)
    pub fn entries(&self) -> impl Iterator<Item = (&str, &Value)> + '_ {
        let map = match &self.0 {
            Repr::Object(m) => Some(m.iter()),
            _ => None,
        };
        map.into_iter().flatten().map(|(k, v)| (&**k, v))
    }

    /// The entries with their shared keys
    pub fn shared_entries(&self) -> impl Iterator<Item = (&Arc<str>, &Value)> + '_ {
        let map = match &self.0 {
            Repr::Object(m) => Some(m.iter()),
            _ => None,
        };
        map.into_iter().flatten()
    }

    // ---- lists ----

    /// The item at `index`; `None` if out of range or this is not a list
    pub fn at(&self, index: usize) -> Option<&Value> {
        match &self.0 {
            Repr::List(l) => l.get(index),
            _ => None,
        }
    }

    /// The items of a list (empty for anything else)
    pub fn items(&self) -> impl Iterator<Item = &Value> + '_ {
        let list = match &self.0 {
            Repr::List(l) => Some(l.iter()),
            _ => None,
        };
        list.into_iter().flatten()
    }

    /// A new list with the item at `index` replaced
    pub fn with_at(&self, index: usize, v: Value) -> Result<Value> {
        let l = self.list_ref()?;
        match l.get(index) {
            Some(old) if old.same_node(&v) => Ok(self.clone()),
            Some(_) => Ok(Value::from_vector(l.update(index, v))),
            None => Err(index_error(index, l.len())),
        }
    }

    /// A new list with `v` inserted at `index` (`index == len` appends)
    pub fn with_inserted(&self, index: usize, v: Value) -> Result<Value> {
        let l = self.list_ref()?;
        if index > l.len() {
            return Err(index_error(index, l.len()));
        }
        let mut l = List::clone(l);
        l.insert(index, v);
        Ok(Value::from_vector(l))
    }

    /// A new list with `v` appended
    pub fn with_pushed(&self, v: Value) -> Result<Value> {
        let l = self.list_ref()?;
        let mut l = List::clone(l);
        l.push_back(v);
        Ok(Value::from_vector(l))
    }

    /// A new list without the item at `index`
    pub fn without_at(&self, index: usize) -> Result<Value> {
        let l = self.list_ref()?;
        if index >= l.len() {
            return Err(index_error(index, l.len()));
        }
        let mut l = List::clone(l);
        l.remove(index);
        Ok(Value::from_vector(l))
    }

    fn list_ref(&self) -> Result<&List> {
        match &self.0 {
            Repr::List(l) => Ok(l),
            _ => Err(Error::new("state.path.not_list").arg("found", self.kind().name())),
        }
    }

    // ---- paths ----

    /// One step down: a key of an object, an index of a list, or a note of a note map
    pub fn step(&self, step: &PathStep) -> Option<&Value> {
        match step {
            PathStep::Key(k) => self.get(k),
            PathStep::Index(i) => self.at(*i),
            PathStep::Note(n) => self.note(*n),
        }
    }

    /// The value at `path`; the empty path is this value
    pub fn at_path(&self, path: &[PathStep]) -> Option<&Value> {
        path.iter().try_fold(self, |v, step| v.step(step))
    }

    /// A new tree with the value at `path` set to `v`, sharing everything off the path. Every step but the last
    /// must exist; the last may add a key (or a note) to an object, but an index must already exist in its list
    /// (inserting is [`Value::with_inserted`]). The empty path gives `v` itself
    pub fn with_path(&self, path: &[PathStep], v: Value) -> Result<Value> {
        self.with_path_from(path, 0, v)
    }

    fn with_path_from(&self, path: &[PathStep], depth: usize, v: Value) -> Result<Value> {
        let Some(step) = path.get(depth) else {
            return Ok(v);
        };
        let last = depth + 1 == path.len();
        match step {
            PathStep::Index(i) => {
                let l = self.list_ref().map_err(|e| e.arg("path", path_text(&path[..depth])))?;
                let child =
                    l.get(*i).ok_or_else(|| index_error(*i, l.len()).arg("path", path_text(&path[..=depth])))?;
                let new = if last { v } else { child.with_path_from(path, depth + 1, v)? };
                if new.same_node(child) {
                    return Ok(self.clone());
                }
                Ok(Value::from_vector(l.update(*i, new)))
            }
            PathStep::Key(_) | PathStep::Note(_) => {
                let Repr::Object(m) = &self.0 else {
                    return Err(Error::new("state.path.not_object")
                        .arg("found", self.kind().name())
                        .arg("path", path_text(&path[..depth])));
                };
                let found = match step {
                    PathStep::Key(k) => m.get_key_value(&**k),
                    PathStep::Note(n) => m.get_key_value(NoteKeyBuf::new(*n).as_str()),
                    PathStep::Index(_) => None,
                };
                let new = match (found, last) {
                    (Some((_, child)), false) => child.with_path_from(path, depth + 1, v)?,
                    (None, false) => {
                        return Err(Error::new("state.path.not_found").arg("path", path_text(&path[..=depth])));
                    }
                    (_, true) => v,
                };
                let key = match (found, step) {
                    (Some((_, child)), _) if child.same_node(&new) => return Ok(self.clone()),
                    (Some((k, _)), _) => Arc::clone(k),
                    (None, PathStep::Key(k)) => Arc::clone(k),
                    (None, _) => match step {
                        PathStep::Note(n) => note_key(*n),
                        _ => return Err(Error::new("state.path.not_found")),
                    },
                };
                Ok(Value::from_map(m.update(key, new)))
            }
        }
    }

    /// A new tree without the value at `path`: the last step removes a key (or a note) from an object, or an item
    /// from a list. Every step must exist
    pub fn without_path(&self, path: &[PathStep]) -> Result<Value> {
        let Some((last, parent_path)) = path.split_last() else {
            return Err(Error::new("state.path.empty"));
        };
        let parent = self
            .at_path(parent_path)
            .ok_or_else(|| Error::new("state.path.not_found").arg("path", path_text(parent_path)))?;
        let new_parent = match last {
            PathStep::Index(i) => parent.without_at(*i).map_err(|e| e.arg("path", path_text(path)))?,
            PathStep::Key(_) | PathStep::Note(_) => {
                if !parent.is_object() {
                    return Err(Error::new("state.path.not_object")
                        .arg("found", parent.kind().name())
                        .arg("path", path_text(parent_path)));
                }
                if parent.step(last).is_none() {
                    return Err(Error::new("state.path.not_found").arg("path", path_text(path)));
                }
                match last {
                    PathStep::Key(k) => parent.without(k),
                    PathStep::Note(n) => parent.without(NoteKeyBuf::new(*n).as_str()),
                    PathStep::Index(_) => parent.clone(),
                }
            }
        };
        self.with_path(parent_path, new_parent)
    }

    /// Calls `visit` for this value and every value below it, depth first, parents before children, with the path
    /// from this value. Note maps are plain objects here, so their steps are [`PathStep::Key`]
    pub fn walk(&self, visit: &mut dyn FnMut(&[PathStep], &Value)) {
        let mut path = Vec::new();
        self.walk_inner(&mut path, visit);
    }

    fn walk_inner(&self, path: &mut Vec<PathStep>, visit: &mut dyn FnMut(&[PathStep], &Value)) {
        visit(path, self);
        match &self.0 {
            Repr::List(l) => {
                for (i, item) in l.iter().enumerate() {
                    path.push(PathStep::Index(i));
                    item.walk_inner(path, visit);
                    path.pop();
                }
            }
            Repr::Object(m) => {
                for (k, item) in m.iter() {
                    path.push(PathStep::Key(Arc::clone(k)));
                    item.walk_inner(path, visit);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    // ---- identity and equality ----

    /// O(1) identity: true when both are the very same node (a clone of it, or the result of an edit that changed
    /// nothing). Containers, texts and resources compare by pointer; null, bools, ints and reals by value (bit for
    /// bit). False never means "different", only "not known to be the same": use `==` for that
    pub fn same_node(&self, other: &Value) -> bool {
        match (&self.0, &other.0) {
            (Repr::Null, Repr::Null) => true,
            (Repr::Bool(a), Repr::Bool(b)) => a == b,
            (Repr::Int(a), Repr::Int(b)) => a == b,
            (Repr::Real(a), Repr::Real(b)) => a.to_bits() == b.to_bits(),
            (Repr::Text(a), Repr::Text(b)) => Arc::ptr_eq(a, b),
            (Repr::Resource(a), Repr::Resource(b)) => Arc::ptr_eq(a, b),
            (Repr::List(a), Repr::List(b)) => Arc::ptr_eq(a, b),
            (Repr::Object(a), Repr::Object(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// An order over values, for sorting and deterministic output: by kind (in the order of [`Kind`]), then by
    /// content. Reals order by `f64::total_cmp` (so `-0.0 < 0.0`); consistent with `==`
    pub fn total_cmp(&self, other: &Value) -> Ordering {
        if self.same_node(other) {
            return Ordering::Equal;
        }
        let rank = |v: &Value| v.kind() as u8;
        match (&self.0, &other.0) {
            (Repr::Bool(a), Repr::Bool(b)) => a.cmp(b),
            (Repr::Int(a), Repr::Int(b)) => a.cmp(b),
            (Repr::Real(a), Repr::Real(b)) => a.total_cmp(b),
            (Repr::Text(a), Repr::Text(b)) => a.cmp(b),
            (Repr::Resource(a), Repr::Resource(b)) => a.cmp(b),
            (Repr::List(a), Repr::List(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    match x.total_cmp(y) {
                        Ordering::Equal => {}
                        o => return o,
                    }
                }
                a.len().cmp(&b.len())
            }
            (Repr::Object(a), Repr::Object(b)) => {
                for ((ka, va), (kb, vb)) in a.iter().zip(b.iter()) {
                    match ka.cmp(kb).then_with(|| va.total_cmp(vb)) {
                        Ordering::Equal => {}
                        o => return o,
                    }
                }
                a.len().cmp(&b.len())
            }
            _ => rank(self).cmp(&rank(other)),
        }
    }
}

fn index_error(index: usize, len: usize) -> Error {
    Error::new("state.path.index_out_of_range")
        .arg("index", i64::try_from(index).unwrap_or(i64::MAX))
        .arg("len", i64::try_from(len).unwrap_or(i64::MAX))
}

impl PartialEq for Value {
    /// Exact equality: deep, and bit for bit for reals (`0.0 != -0.0`). Shared subtrees are skipped in O(1)
    fn eq(&self, other: &Value) -> bool {
        if self.same_node(other) {
            return true;
        }
        match (&self.0, &other.0) {
            (Repr::Text(a), Repr::Text(b)) => a == b,
            (Repr::Resource(a), Repr::Resource(b)) => a == b && a.name() == b.name(),
            (Repr::List(a), Repr::List(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y),
            (Repr::Object(a), Repr::Object(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|((ka, va), (kb, vb))| ka == kb && va == vb)
            }
            _ => false,
        }
    }
}

impl Eq for Value {}

impl Default for Value {
    fn default() -> Self {
        Value::NULL
    }
}

impl fmt::Debug for Value {
    /// A compact JSON-like form, for tests and logs (never refuses, unlike the JSON writer)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Null => f.write_str("null"),
            Repr::Bool(v) => write!(f, "{v}"),
            Repr::Int(v) => write!(f, "{v}"),
            Repr::Real(v) => f.write_str(ryu::Buffer::new().format_finite(*v)),
            Repr::Text(v) => write!(f, "{:?}", truncated(v)),
            Repr::Resource(r) => write!(f, "{r:?}"),
            Repr::List(l) => f.debug_list().entries(l.iter()).finish(),
            Repr::Object(m) => f.debug_map().entries(m.iter().map(|(k, v)| (&**k, v))).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> PathStep {
        PathStep::key(k)
    }

    fn sample() -> Value {
        let note = Value::object([("key", Value::int(57)), ("start", Value::int(0))]);
        let notes = Value::object([(note_key(812), note)]);
        let item = Value::object([("id", Value::text("c:52")), ("notes", notes)]);
        let track = Value::object([("id", Value::text("t:17")), ("items", Value::list([item]))]);
        Value::object([("tracks", Value::list([track]))])
    }

    #[test]
    fn value_is_send_sync_and_small() {
        fn check<T: Send + Sync>() {}
        check::<Value>();
        check::<PathStep>();
        assert!(size_of::<Value>() <= 24);
    }

    #[test]
    fn reals_refuse_nan_and_inf_and_keep_negative_zero() {
        assert_eq!(Value::real(f64::NAN).unwrap_err().key(), "state.real.not_finite");
        assert_eq!(Value::real(f64::INFINITY).unwrap_err().key(), "state.real.not_finite");
        assert_eq!(Value::real(f64::NEG_INFINITY).unwrap_err().key(), "state.real.not_finite");
        let nz = Value::real(-0.0).unwrap();
        assert!(nz.as_real().unwrap().is_sign_negative());
        assert_ne!(nz, Value::real(0.0).unwrap());
        assert_eq!(nz, Value::real(-0.0).unwrap());
    }

    #[test]
    fn no_implicit_conversions() {
        assert_eq!(Value::int(1).as_real(), None);
        assert_eq!(Value::real(1.0).unwrap().as_int(), None);
        assert_ne!(Value::int(1), Value::real(1.0).unwrap());
        assert_eq!(Value::text("1").as_int(), None);
    }

    #[test]
    fn objects_keep_keys_sorted() {
        let v = Value::object([("b", Value::int(2)), ("a", Value::int(1)), ("c", Value::int(3))]);
        assert_eq!(v.keys().collect::<Vec<_>>(), ["a", "b", "c"]);
        let v = v.with("aa", Value::NULL).without("b");
        assert_eq!(v.keys().collect::<Vec<_>>(), ["a", "aa", "c"]);
        // A repeated key keeps the last value
        let v = Value::object([("a", Value::int(1)), ("a", Value::int(2))]);
        assert_eq!(v.get("a"), Some(&Value::int(2)));
    }

    #[test]
    fn edits_share_what_did_not_change() {
        let a = Value::object([("x", Value::list([Value::int(1)])), ("y", Value::list([Value::int(2)]))]);
        let b = a.with("x", Value::int(5));
        assert!(a.get("y").unwrap().same_node(b.get("y").unwrap()));
        assert!(!a.same_node(&b));
        // Setting a node to itself changes nothing, and gives the same node back
        let c = a.with("y", a.get("y").unwrap().clone());
        assert!(c.same_node(&a));
        assert!(a.without("missing").same_node(&a));
        // Equal content made separately is equal but not the same node
        let d = Value::object([("x", Value::list([Value::int(1)])), ("y", Value::list([Value::int(2)]))]);
        assert_eq!(a, d);
        assert!(!a.same_node(&d));
        assert!(a.clone().same_node(&a));
    }

    #[test]
    fn list_operations() {
        let l = Value::list([Value::int(0), Value::int(1), Value::int(2)]);
        assert_eq!(l.at(1), Some(&Value::int(1)));
        assert_eq!(l.at(3), None);
        let l2 = l.with_at(1, Value::int(10)).unwrap();
        assert_eq!(l2, Value::list([Value::int(0), Value::int(10), Value::int(2)]));
        let l3 = l.with_inserted(3, Value::int(3)).unwrap().with_inserted(0, Value::int(-1)).unwrap();
        assert_eq!(l3.items().filter_map(Value::as_int).collect::<Vec<_>>(), [-1, 0, 1, 2, 3]);
        assert_eq!(l.without_at(0).unwrap().items().filter_map(Value::as_int).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(l.with_pushed(Value::NULL).unwrap().len(), 4);
        assert_eq!(l.with_at(3, Value::NULL).unwrap_err().key(), "state.path.index_out_of_range");
        assert_eq!(l.with_inserted(4, Value::NULL).unwrap_err().key(), "state.path.index_out_of_range");
        assert_eq!(l.without_at(3).unwrap_err().key(), "state.path.index_out_of_range");
        assert_eq!(Value::int(1).with_at(0, Value::NULL).unwrap_err().key(), "state.path.not_list");
        assert!(l.with_at(0, Value::int(0)).unwrap().same_node(&l));
    }

    #[test]
    fn paths_reach_notes_by_number() {
        let root = sample();
        let path = [
            key("tracks"),
            PathStep::Index(0),
            key("items"),
            PathStep::Index(0),
            key("notes"),
            PathStep::Note(812),
            key("key"),
        ];
        assert_eq!(root.at_path(&path), Some(&Value::int(57)));
        assert_eq!(root.at_path(&path[..0]), Some(&root));
        let edited = root.with_path(&path, Value::int(60)).unwrap();
        assert_eq!(edited.at_path(&path), Some(&Value::int(60)));
        assert_eq!(root.at_path(&path), Some(&Value::int(57)), "the old root is untouched");
        // Off-path parts are shared
        let id = [key("tracks"), PathStep::Index(0), key("id")];
        assert!(root.at_path(&id).unwrap().same_node(edited.at_path(&id).unwrap()));
        // Adding a note by number
        let mut new_note = path[..6].to_vec();
        new_note[5] = PathStep::Note(900);
        let added = root.with_path(&new_note, Value::object([("key", Value::int(1))])).unwrap();
        assert_eq!(added.at_path(&new_note[..5]).unwrap().keys().collect::<Vec<_>>(), ["812", "900"]);
        // And removing it
        let removed = added.without_path(&new_note).unwrap();
        assert_eq!(removed, root);
        assert!(root.with_path(&path, Value::int(57)).unwrap().same_node(&root));
    }

    #[test]
    fn path_errors_name_the_path() {
        let root = sample();
        let e = root.with_path(&[key("tracks"), PathStep::Index(5), key("x")], Value::NULL).unwrap_err();
        assert_eq!(e.key(), "state.path.index_out_of_range");
        assert!(e.to_string().contains("/tracks/5"), "{e}");
        let e = root.with_path(&[key("nope"), key("x")], Value::NULL).unwrap_err();
        assert_eq!(e.key(), "state.path.not_found");
        let e = root.with_path(&[key("tracks"), key("x")], Value::NULL).unwrap_err();
        assert_eq!(e.key(), "state.path.not_object");
        let e = root
            .with_path(&[key("tracks"), PathStep::Index(0), key("id"), PathStep::Index(0)], Value::NULL)
            .unwrap_err();
        assert_eq!(e.key(), "state.path.not_list");
        assert_eq!(root.without_path(&[]).unwrap_err().key(), "state.path.empty");
        assert_eq!(root.without_path(&[key("nope")]).unwrap_err().key(), "state.path.not_found");
    }

    #[test]
    fn path_text_is_a_json_pointer() {
        let p = [key("a/b"), key("c~d"), PathStep::Index(3), PathStep::Note(812)];
        assert_eq!(path_text(&p), "/a~1b/c~0d/3/812");
        assert_eq!(path_text(&[]), "");
    }

    #[test]
    fn note_keys_are_canonical() {
        assert_eq!(&*note_key(0), "0");
        assert_eq!(&*note_key(u64::MAX), "18446744073709551615");
        assert_eq!(NoteKeyBuf::new(u64::MAX).as_str(), "18446744073709551615");
        assert_eq!(NoteKeyBuf::new(0).as_str(), "0");
        assert_eq!(parse_note_key("812"), Some(812));
        assert_eq!(parse_note_key("0812"), None);
        assert_eq!(parse_note_key("-1"), None);
    }

    #[test]
    fn walk_visits_parents_first() {
        let mut seen = Vec::new();
        sample().walk(&mut |path, v| seen.push((path_text(path), v.kind())));
        assert_eq!(seen[0], (String::new(), Kind::Object));
        assert!(seen.contains(&("/tracks/0/items/0/notes/812/key".to_owned(), Kind::Int)));
        assert_eq!(seen.len(), 11);
    }

    #[test]
    fn total_order_is_consistent_with_equality() {
        let vals = [
            Value::NULL,
            Value::bool(false),
            Value::int(-1),
            Value::real(-0.0).unwrap(),
            Value::real(0.0).unwrap(),
            Value::text("a"),
            Value::empty_list(),
            Value::empty_object(),
        ];
        for (i, a) in vals.iter().enumerate() {
            for (j, b) in vals.iter().enumerate() {
                assert_eq!(a.total_cmp(b), i.cmp(&j), "{a:?} {b:?}");
                assert_eq!(a == b, i == j);
            }
        }
    }
}
