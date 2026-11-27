//! The project's tempo map as the core reads it from the tree, and its cache by revision: the one implementation the
//! session, the fake and the client share (the interface asks `CorePort::tempo_map` and never builds one itself).

use std::cell::RefCell;
use std::sync::Arc;

use lm_base::Result;
use lm_state::Value;
use lm_time::{Bpm, Ppq, TempoMap, TempoPoint, TimeSignature};

/// The project's tempo map (as lm-compile reads it)
pub fn tempo_map(root: &Value) -> Result<TempoMap> {
    let p = root.get("project");
    let int = |k: &str, d: i64| p.and_then(|p| p.get(k)).and_then(Value::as_int).unwrap_or(d);
    let rate = u32::try_from(int("sampleRate", 48_000)).unwrap_or(48_000);
    let ppq = Ppq::new(u32::try_from(int("ppq", 960)).unwrap_or(960))?;
    let mut points = Vec::new();
    for t in root.get("tempoMap").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        let bpm = Bpm::parse(t.get("bpm").and_then(Value::as_text).unwrap_or("120"))?;
        let sig: Vec<i64> = t.get("timeSig").map(|l| l.items().filter_map(Value::as_int).collect()).unwrap_or_default();
        let (n, d) = match sig.as_slice() {
            [n, d] => (*n, *d),
            _ => (4, 4),
        };
        let ts = TimeSignature::new(u8::try_from(n).unwrap_or(4), u8::try_from(d).unwrap_or(4))?;
        let mut point = TempoPoint::new(t.get("tick").and_then(Value::as_int).unwrap_or(0), bpm, ts);
        if t.get("ramp").and_then(Value::as_bool) == Some(true) {
            point = point.linear();
        }
        points.push(point);
    }
    if points.is_empty() {
        points.push(TempoPoint::new(0, Bpm::from_integer(120)?, TimeSignature::COMMON));
    }
    TempoMap::new(rate, ppq, &points)
}

/// The map of the tree, or a constant 120 bpm 4/4 one when the tree's points are not a valid map (what is shown
/// while the document is being fixed)
pub fn tempo_map_or_default(root: &Value) -> TempoMap {
    tempo_map(root).unwrap_or_else(|_| {
        let rate = root
            .get("project")
            .and_then(|p| p.get("sampleRate"))
            .and_then(Value::as_int)
            .and_then(|r| u32::try_from(r).ok())
            .unwrap_or(48_000);
        let ppq = Ppq::new(960).expect("960 is a valid ppq");
        TempoMap::constant(rate, ppq, Bpm::from_integer(120).expect("120 bpm"), TimeSignature::COMMON)
            .expect("a constant map")
    })
}

/// The tempo map kept for the tree's revision: built again only when the tempo points, the rate or the ppq changed
/// (the tree is persistent, so the unchanged node is the same node)
#[derive(Default)]
pub struct TempoCache {
    cell: RefCell<Option<Cached>>,
}

impl std::fmt::Debug for TempoCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TempoCache").finish_non_exhaustive()
    }
}

struct Cached {
    points: Option<Value>,
    rate: Option<i64>,
    ppq: Option<i64>,
    map: Arc<TempoMap>,
}

impl TempoCache {
    pub fn get(&self, root: &Value) -> Arc<TempoMap> {
        let points = root.get("tempoMap");
        let project = root.get("project");
        let rate = project.and_then(|p| p.get("sampleRate")).and_then(Value::as_int);
        let ppq = project.and_then(|p| p.get("ppq")).and_then(Value::as_int);
        let mut cell = self.cell.borrow_mut();
        if let Some(c) = cell.as_ref() {
            let same_points = match (&c.points, points) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_node(b),
                _ => false,
            };
            if same_points && c.rate == rate && c.ppq == ppq {
                return c.map.clone();
            }
        }
        let map = Arc::new(tempo_map_or_default(root));
        *cell = Some(Cached { points: points.cloned(), rate, ppq, map: map.clone() });
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_tree_is_a_constant_map_and_the_cache_reuses_it() {
        let root = Value::object([("project", Value::object([("sampleRate", Value::int(44_100))]))]);
        let cache = TempoCache::default();
        let a = cache.get(&root);
        let b = cache.get(&root);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.tick_at(0), 0);
    }

    #[test]
    fn a_new_tempo_list_builds_a_new_map() {
        let one = Value::object([("tempoMap", Value::list([Value::object([("tick", Value::int(0)), ("bpm", Value::text("120"))])]))]);
        let two = Value::object([("tempoMap", Value::list([Value::object([("tick", Value::int(0)), ("bpm", Value::text("60"))])]))]);
        let cache = TempoCache::default();
        let a = cache.get(&one);
        let b = cache.get(&two);
        assert!(!Arc::ptr_eq(&a, &b));
    }
}
