//! The arrangement's edits of agent B in the fake (the session's are in lm-session/src/arrange.rs, the same
//! rules): the clipboard (copy, cut, paste) and inserting or removing a bar (F-ARR-002, F-ARR-006). The fake's
//! items are all in ticks (no audio timebase conversions).

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, PathStep, Value};

use crate::FakePort;
use crate::demo::BAR;

#[derive(Clone)]
pub(crate) struct Copied {
    clip: lm_doc::Clip,
    track: NodeId,
    start: i64,
}

fn err(key: &'static str) -> Error {
    Error::new(key)
}

fn int(v: &Value, k: &str, d: i64) -> i64 {
    v.get(k).and_then(Value::as_int).unwrap_or(d)
}

fn id_of(v: &Value) -> Option<NodeId> {
    v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())
}

impl FakePort {
    pub(crate) fn arrange_b(&mut self, action: &str, p: &Value) -> Result<()> {
        let items = || -> Result<Vec<NodeId>> {
            p.get("items")
                .ok_or_else(|| err("port.params"))?
                .items()
                .map(|v| v.as_text().and_then(|t| NodeId::parse(t).ok()).ok_or_else(|| err("port.params")))
                .collect()
        };
        let tick = || p.get("tick").and_then(Value::as_int).ok_or_else(|| err("port.params"));
        match action {
            "arrange.copy" => self.copy_items(&items()?),
            "arrange.cut" => {
                let ids = items()?;
                self.copy_items(&ids)?;
                self.doc.transact(Label::new("Cut"), |e| ids.iter().try_for_each(|id| e.remove(*id))).map(|_| ())
            }
            "arrange.paste" => {
                let at = p.get("at").and_then(Value::as_int).ok_or_else(|| err("port.params"))?;
                let track = p.get("track").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
                self.paste_items(at, track)
            }
            "arrange.insert_bar" => self.shift_bar(tick()?, true),
            _ => self.shift_bar(tick()?, false),
        }
    }

    fn copy_items(&mut self, items: &[NodeId]) -> Result<()> {
        let mut out = Vec::new();
        for id in items {
            let v = self.doc.find(*id).ok_or_else(|| err("port.no_such_node"))?;
            let track = self.doc.loc(*id).map(|l| l.parent).ok_or_else(|| err("port.no_such_node"))?;
            out.push(Copied { clip: self.doc.copy(*id)?, track, start: int(&v, "start", 0) });
        }
        if out.is_empty() {
            return Err(err("arrange.copy.empty"));
        }
        out.sort_by_key(|c| c.start);
        self.clipboard = out;
        Ok(())
    }

    fn paste_items(&mut self, at: i64, track: Option<NodeId>) -> Result<()> {
        let copies = self.clipboard.clone();
        let first = copies.first().ok_or_else(|| err("arrange.paste.empty"))?;
        let tracks: Vec<NodeId> =
            self.doc.root().get("tracks").map(|l| l.items().filter_map(id_of).collect()).unwrap_or_default();
        let index = |id: NodeId| tracks.iter().position(|t| *t == id);
        let shift = match (track.and_then(index), index(first.track)) {
            (Some(to), Some(from)) => to as i64 - from as i64,
            _ => 0,
        };
        let base = first.start;
        self.doc
            .transact(Label::new("Paste"), |e| {
                for c in &copies {
                    let to =
                        index(c.track).map(|i| i as i64 + shift).filter(|i| *i >= 0 && (*i as usize) < tracks.len());
                    let parent = to.map_or(c.track, |i| tracks[i as usize]);
                    let n = e.find(parent).and_then(|t| t.get("items")).map_or(0, Value::len);
                    let copy = e.paste(parent, "items", n, &c.clip)?.ok_or_else(|| err("arrange.paste.refused"))?;
                    e.set(copy, &[PathStep::key("start")], Value::int((at + c.start - base).max(0)))?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    fn shift_bar(&mut self, tick: i64, insert: bool) -> Result<()> {
        let root = self.doc.root().clone();
        let d = if insert { BAR } else { -BAR };
        let mut sets: Vec<(NodeId, &'static str, Value)> = Vec::new();
        let mut owners: Vec<Value> = root.get("tracks").map(|l| l.items().cloned().collect()).unwrap_or_default();
        if let Some(m) = root.get("mixer") {
            owners.extend(m.get("channels").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default());
            owners.extend(m.get("master").cloned());
        }
        for o in &owners {
            for it in o.get("items").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                let (Some(id), t) = (id_of(&it), int(&it, "start", 0)) else { continue };
                if t >= tick {
                    sets.push((id, "start", Value::int((t + d).max(tick.min(t)))));
                }
            }
            for a in o.get("automation").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                let Some(id) = id_of(&a) else { continue };
                let pts: Vec<Value> = a
                    .get("points")
                    .map(|l| l.items().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| insert || !(tick..tick + BAR).contains(&int(p, "tick", 0)))
                    .map(|p| {
                        let t = int(&p, "tick", 0);
                        if t >= tick { p.with("tick", Value::int((t + d).max(0))) } else { p }
                    })
                    .collect();
                sets.push((id, "points", Value::list(pts)));
            }
        }
        for list in ["markers", "regions"] {
            for m in root.get(list).map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                let Some(id) = id_of(&m) else { continue };
                for k in ["tick", "from", "to"] {
                    if let Some(t) = m.get(k).and_then(Value::as_int).filter(|t| *t >= tick) {
                        sets.push((id, k, Value::int((t + d).max(tick))));
                    }
                }
            }
        }
        self.doc
            .transact(Label::new(if insert { "Insert bar" } else { "Remove bar" }), |e| {
                sets.iter().try_for_each(|(id, k, v)| e.set(*id, &[PathStep::key(k)], v.clone()))
            })
            .map(|_| ())
    }
}
