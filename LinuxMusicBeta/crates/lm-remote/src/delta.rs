//! How the server sends a new root to a mirror: the differences from the last root it sent, found by identity
//! (`same_node`, so an untouched subtree costs nothing) — a field set, a key removed, a list rebuilt from the items
//! it kept (by position in the old list, matched by `id`) and the new ones. Moving one note sends one note.

use std::collections::HashMap;

use lm_base::Result;
use lm_state::{PathStep, Value};

use crate::codec::{bad, get, int_value, list, path_from, path_to, text};

/// One difference
#[derive(Clone, Debug, PartialEq)]
pub enum DeltaOp {
    Set {
        path: Vec<PathStep>,
        value: Value,
    },
    Remove {
        path: Vec<PathStep>,
    },
    /// The list at `path` becomes these items: `Ok(i)` the old list's item `i`, `Err(v)` a new value
    List {
        path: Vec<PathStep>,
        items: Vec<std::result::Result<usize, Value>>,
    },
}

/// The differences that make `old` into `new`
pub fn delta(old: &Value, new: &Value) -> Vec<DeltaOp> {
    let mut out = Vec::new();
    walk(old, new, &mut Vec::new(), &mut out);
    out
}

fn id_of(v: &Value) -> Option<&str> {
    v.get("id").and_then(Value::as_text)
}

fn walk(old: &Value, new: &Value, path: &mut Vec<PathStep>, out: &mut Vec<DeltaOp>) {
    if old.same_node(new) {
        return;
    }
    if old.is_object() && new.is_object() {
        for (k, v) in new.shared_entries() {
            path.push(PathStep::Key(k.clone()));
            match old.get(k) {
                Some(o) => walk(o, v, path, out),
                None => out.push(DeltaOp::Set { path: path.clone(), value: v.clone() }),
            }
            path.pop();
        }
        for (k, _) in old.shared_entries() {
            if new.get(k).is_none() {
                path.push(PathStep::Key(k.clone()));
                out.push(DeltaOp::Remove { path: path.clone() });
                path.pop();
            }
        }
        return;
    }
    if old.is_list() && new.is_list() {
        if old.len() == new.len() {
            let ids_match = old.items().zip(new.items()).all(|(a, b)| id_of(a) == id_of(b));
            if ids_match {
                for (i, (a, b)) in old.items().zip(new.items()).enumerate() {
                    path.push(PathStep::Index(i));
                    walk(a, b, path, out);
                    path.pop();
                }
                return;
            }
        }
        // Rebuilt: the old items kept (by id, or the very same node), the rest sent
        let by_id: HashMap<&str, usize> = old.items().enumerate().filter_map(|(i, v)| Some((id_of(v)?, i))).collect();
        let mut items = Vec::with_capacity(new.len());
        let mut changed_kept = Vec::new();
        for (i, v) in new.items().enumerate() {
            let kept = match id_of(v) {
                Some(id) => by_id.get(id).copied(),
                None => old.at(i).filter(|o| o.same_node(v)).map(|_| i),
            };
            match kept {
                Some(j) => {
                    items.push(Ok(j));
                    if let Some(o) = old.at(j)
                        && !o.same_node(v)
                    {
                        changed_kept.push((i, j));
                    }
                }
                None => items.push(Err(v.clone())),
            }
        }
        out.push(DeltaOp::List { path: path.clone(), items });
        for (i, j) in changed_kept {
            if let (Some(o), Some(v)) = (old.at(j), new.at(i)) {
                path.push(PathStep::Index(i));
                walk(o, v, path, out);
                path.pop();
            }
        }
        return;
    }
    out.push(DeltaOp::Set { path: path.clone(), value: new.clone() });
}

/// Applies differences to a root
pub fn apply(root: &Value, ops: &[DeltaOp]) -> Result<Value> {
    let mut root = root.clone();
    for op in ops {
        root = match op {
            DeltaOp::Set { path, value } if path.is_empty() => value.clone(),
            DeltaOp::Set { path, value } => root.with_path(path, value.clone())?,
            DeltaOp::Remove { path } => root.without_path(path)?,
            DeltaOp::List { path, items } => {
                let old = if path.is_empty() { Some(&root) } else { root.at_path(path) };
                let old = old.filter(|l| l.is_list()).ok_or_else(|| bad("delta.list"))?;
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(match it {
                        Ok(j) => old.at(*j).cloned().ok_or_else(|| bad("delta.keep"))?,
                        Err(v) => v.clone(),
                    });
                }
                let list = Value::list(out);
                if path.is_empty() { list } else { root.with_path(path, list)? }
            }
        };
    }
    Ok(root)
}

pub fn to_value(ops: &[DeltaOp]) -> Result<Value> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        out.push(match op {
            DeltaOp::Set { path, value } => {
                Value::object([("op", Value::text("set")), ("path", path_to(path)?), ("value", value.clone())])
            }
            DeltaOp::Remove { path } => Value::object([("op", Value::text("remove")), ("path", path_to(path)?)]),
            DeltaOp::List { path, items } => {
                let items = items
                    .iter()
                    .map(|i| {
                        Ok(match i {
                            Ok(j) => Value::object([("keep", int_value(*j as u64)?)]),
                            Err(v) => Value::object([("value", v.clone())]),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Value::object([("op", Value::text("list")), ("path", path_to(path)?), ("items", Value::list(items))])
            }
        });
    }
    Ok(Value::list(out))
}

pub fn from_value(v: &Value) -> Result<Vec<DeltaOp>> {
    if !v.is_list() {
        return Err(bad("delta"));
    }
    v.items()
        .map(|op| {
            let path = path_from(get(op, "path")?)?;
            Ok(match text(op, "op")? {
                "set" => DeltaOp::Set { path, value: get(op, "value")?.clone() },
                "remove" => DeltaOp::Remove { path },
                "list" => DeltaOp::List {
                    path,
                    items: list(op, "items")?
                        .map(|i| match i.get("keep") {
                            Some(k) => {
                                k.as_int().and_then(|k| usize::try_from(k).ok()).map(Ok).ok_or_else(|| bad("keep"))
                            }
                            None => Ok(Err(get(i, "value")?.clone())),
                        })
                        .collect::<Result<_>>()?,
                },
                _ => return Err(bad("delta.op")),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(entries: &[(&str, Value)]) -> Value {
        Value::object(entries.iter().map(|(k, v)| (*k, v.clone())))
    }

    #[test]
    fn round_trips_and_sends_little() {
        let a = obj(&[("id", Value::text("t:1")), ("v", Value::int(1))]);
        let b = obj(&[("id", Value::text("t:2")), ("v", Value::int(2))]);
        let c = obj(&[("id", Value::text("t:3")), ("v", Value::int(3))]);
        let old =
            obj(&[("tracks", Value::list([a.clone(), b.clone()])), ("x", Value::int(1)), ("gone", Value::bool(true))]);
        // Insert c at the front, change b, change x, drop gone, add y
        let b2 = b.with("v", Value::int(20));
        let new = obj(&[
            ("tracks", Value::list([c.clone(), a.clone(), b2])),
            ("x", Value::int(2)),
            ("y", Value::text("new")),
        ]);
        let ops = delta(&old, &new);
        let back = from_value(&to_value(&ops).unwrap()).unwrap();
        assert_eq!(back, ops);
        assert_eq!(apply(&old, &back).unwrap(), new);
        // a is kept by reference, not sent
        let DeltaOp::List { items, .. } = ops.iter().find(|o| matches!(o, DeltaOp::List { .. })).unwrap() else {
            panic!()
        };
        assert_eq!(items[1], Ok(0));
        assert!(delta(&new, &new).is_empty());
        // Whole root replaced
        assert_eq!(apply(&old, &delta(&old, &Value::int(5))).unwrap(), Value::int(5));
    }
}
