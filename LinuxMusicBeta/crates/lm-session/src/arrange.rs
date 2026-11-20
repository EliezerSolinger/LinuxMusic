//! The arrangement's and the desk's domain edits as actions (F-ARR-005, F-ARR-007, F-MIX-012): each is one
//! transaction, one undo step, and keeps the item's notes, control lanes, takes and comp coherent.
//!
//! ```text
//! arrange.split      { item: text (NodeId), tick: int (timeline ticks) }
//! arrange.duplicate  { item: text (NodeId) }
//! mixer.create_channels_for_outputs { track: text (NodeId) }
//! ```

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, PathStep, ROOT, Value};

use crate::actions::ActionSpec;
use crate::session::{Session, tempo_map};

fn node_param(p: &Value, key: &str) -> Result<NodeId> {
    p.get(key).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(|| Error::new("action.params"))
}

fn int(v: &Value, k: &str, d: i64) -> i64 {
    v.get(k).and_then(Value::as_int).unwrap_or(d)
}

fn writable(s: &Session) -> bool {
    !s.project().read_only
}

pub static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "arrange.split",
        label: "action.arrange.split",
        run: |s, p| {
            let tick = p.get("tick").and_then(Value::as_int).ok_or_else(|| Error::new("action.params"))?;
            s.split_item(node_param(p, "item")?, tick).map(|_| ())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "arrange.duplicate",
        label: "action.arrange.duplicate",
        run: |s, p| s.duplicate_item(node_param(p, "item")?).map(|_| ()),
        enabled: writable,
    },
    ActionSpec {
        id: "mixer.create_channels_for_outputs",
        label: "action.mixer.create_channels_for_outputs",
        run: |s, p| s.create_channels_for_outputs(node_param(p, "track")?).map(|_| ()),
        enabled: writable,
    },
];

/// The ids of the notes of an item with their start
fn note_starts(item: &Value) -> Vec<(u64, i64)> {
    item.get("notes")
        .map(|m| m.entries().filter_map(|(n, v)| Some((n.parse().ok()?, v.get("start")?.as_int()?))).collect())
        .unwrap_or_default()
}

impl Session {
    /// Splits an item at a timeline tick (S, F-ARR-005). The right part is a copy of the item: each side keeps the
    /// notes that start in it, the control points and comp segments inside it (moved to its own start), and an
    /// audio item's right side reads its takes further on (`sourceOffset`). The fades at the cut are dropped.
    /// A repeating pattern's right side repeats the same pattern from the cut on. Error: `arrange.split.outside`
    /// (the tick is not strictly inside the item)
    pub fn split_item(&mut self, item: NodeId, tick: i64) -> Result<NodeId> {
        let v = self.document().find(item).ok_or_else(|| Error::new("action.no_such_node"))?;
        let map = tempo_map(self.document().root())?;
        let audio_time = v.get("timebase").and_then(Value::as_text) == Some("audio");
        let (start, length) = (int(&v, "start", 0), int(&v, "length", 1));
        // A pattern that repeats inside the item (its `loop` shorter than the item)
        let repeat = v.get("loop").and_then(Value::as_int).filter(|l| *l >= 1 && *l < length);
        // The cut in the item's own unit (ticks, or samples for an item in audio time)
        let at = if audio_time { map.sample_at(tick) } else { tick };
        let cut = at - start;
        if cut <= 0 || cut >= length {
            return Err(Error::new("arrange.split.outside").arg("tick", tick));
        }
        // How far into its takes the right part starts, in samples
        let skip = if audio_time { cut } else { map.sample_at(start + cut) - map.sample_at(start) };
        let loc = self.document().loc(item).cloned().ok_or_else(|| Error::new("action.no_such_node"))?;
        let clip = self.document().copy(item)?;
        let mut made = None;
        self.document_mut().transact(Label::new("action.arrange.split"), |e| {
            let k = PathStep::key;
            let list = loc.container.join(".");
            let index = e
                .find(loc.parent)
                .and_then(|t| t.get("items"))
                .and_then(|l| l.items().position(|x| x.get("id").and_then(Value::as_text) == Some(&item.to_string())))
                .map_or(0, |i| i + 1);
            let right = e.paste(loc.parent, &list, index, &clip)?.ok_or_else(|| Error::new("arrange.split.refused"))?;
            e.set(item, &[k("length")], Value::int(cut))?;
            e.set(item, &[k("fadeOut")], Value::int(0))?;
            e.set(right, &[k("start")], Value::int(at))?;
            e.set(right, &[k("length")], Value::int(length - cut))?;
            e.set(right, &[k("fadeIn")], Value::int(0))?;
            // The notes: each side keeps the ones that start in it. The right side of a repeating pattern keeps the
            // whole pattern, turned so that it begins where the cut fell in it
            for (side, left) in [(item, true), (right, false)] {
                let notes = e.find(side).map(note_starts).unwrap_or_default();
                for (n, s0) in notes {
                    match (left, repeat) {
                        (true, _) if s0 >= cut => e.remove_note(side, n)?,
                        (true, _) => {}
                        (false, Some(l)) if s0 >= l => e.remove_note(side, n)?,
                        (false, Some(l)) => {
                            e.set_note(side, n, &[k("start")], Value::int((s0 - cut).rem_euclid(l)))?;
                        }
                        (false, None) if s0 < cut => e.remove_note(side, n)?,
                        (false, None) => e.set_note(side, n, &[k("start")], Value::int(s0 - cut))?,
                    }
                }
            }
            for (side, from, to) in [(item, 0, cut), (right, cut, length)] {
                let Some(now) = e.find(side).cloned() else { continue };
                // The comp, clipped and moved
                let comp: Vec<Value> = now
                    .get("comp")
                    .map(|l| {
                        l.items()
                            .filter_map(|seg| {
                                let (a, b) = (int(seg, "from", 0).max(from), int(seg, "to", 0).min(to));
                                (b > a).then(|| seg.with("from", Value::int(a - from)).with("to", Value::int(b - from)))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if now.get("comp").is_some_and(|c| !c.is_empty()) {
                    e.set(side, &[k("comp")], Value::list(comp))?;
                }
                // The control lanes' points (ticks from the item's start), clipped and moved
                if let Some(controls) = now.get("controls") {
                    let kept: Vec<Value> = controls
                        .items()
                        .map(|c| {
                            let points: Vec<Value> = c
                                .get("points")
                                .map(|l| {
                                    l.items()
                                        .filter(|p| (from..to).contains(&int(p, "tick", 0)))
                                        .map(|p| p.with("tick", Value::int(int(p, "tick", 0) - from)))
                                        .collect()
                                })
                                .unwrap_or_default();
                            c.with("points", Value::list(points))
                        })
                        .collect();
                    e.set(side, &[k("controls")], Value::list(kept))?;
                }
            }
            // The right part's takes start further on
            if skip > 0 {
                let lanes: Vec<(NodeId, i64)> = e
                    .find(right)
                    .and_then(|v| v.get("lanes").cloned())
                    .map(|l| {
                        l.items()
                            .filter_map(|lane| {
                                let id = NodeId::parse(lane.get("id")?.as_text()?).ok()?;
                                Some((id, int(lane, "sourceOffset", 0)))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                for (lane, offset) in lanes {
                    e.set(lane, &[k("sourceOffset")], Value::int(offset + skip))?;
                }
            }
            made = Some(right);
            Ok(())
        })?;
        made.ok_or_else(|| Error::new("arrange.split.refused"))
    }

    /// A copy of the item right after it, on the next bar line (Ctrl+D, F-ARR-007)
    pub fn duplicate_item(&mut self, item: NodeId) -> Result<NodeId> {
        let v = self.document().find(item).ok_or_else(|| Error::new("action.no_such_node"))?;
        let map = tempo_map(self.document().root())?;
        let audio_time = v.get("timebase").and_then(Value::as_text) == Some("audio");
        let end = int(&v, "start", 0) + int(&v, "length", 1);
        let end_tick = if audio_time { map.tick_at(end) } else { end };
        let seg = map.segment_at_tick(end_tick);
        let bar = seg.time_signature().bar_ticks(map.ppq()).unwrap_or(3840).max(1);
        // The bars of the meter count from the segment's start (the first one from tick 0)
        let from = seg.start_tick().max(0);
        let next_bar = from + (end_tick - from + bar - 1).div_euclid(bar) * bar;
        let start = if audio_time { map.sample_at(next_bar).max(end) } else { next_bar };
        let loc = self.document().loc(item).cloned().ok_or_else(|| Error::new("action.no_such_node"))?;
        let clip = self.document().copy(item)?;
        let mut made = None;
        self.document_mut().transact(Label::new("action.arrange.duplicate"), |e| {
            let list = loc.container.join(".");
            let n = e.find(loc.parent).and_then(|t| t.get("items")).map_or(0, Value::len);
            let copy = e.paste(loc.parent, &list, n, &clip)?.ok_or_else(|| Error::new("arrange.duplicate.refused"))?;
            e.set(copy, &[PathStep::key("start")], Value::int(start))?;
            made = Some(copy);
            Ok(())
        })?;
        made.ok_or_else(|| Error::new("arrange.duplicate.refused"))
    }

    /// "Create channels for the outputs" (I-3.9, F-MIX-012): a group channel, and one channel per output of the
    /// track sending to it, each output routed to its channel; one transaction. The new channels' ids, the group's
    /// first
    pub fn create_channels_for_outputs(&mut self, track: NodeId) -> Result<Vec<NodeId>> {
        let t = self.document().find(track).ok_or_else(|| Error::new("action.no_such_node"))?;
        let name = t.get("name").and_then(Value::as_text).unwrap_or("").to_owned();
        let outputs: Vec<(NodeId, String, i64)> = t
            .get("outputs")
            .map(|l| {
                l.items()
                    .filter_map(|o| {
                        let id = NodeId::parse(o.get("id")?.as_text()?).ok()?;
                        Some((id, o.get("port")?.as_text()?.to_owned(), int(o, "channels", 2)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if outputs.is_empty() {
            return Err(Error::new("mixer.no_outputs"));
        }
        let mut made = Vec::new();
        self.document_mut().transact(Label::new("action.mixer.create_channels_for_outputs"), |e| {
            let k = PathStep::key;
            let n = e.root().get("mixer").and_then(|m| m.get("channels")).map_or(0, Value::len);
            let group = e.insert(ROOT, "mixer.channels", n, Value::object([("name", Value::text(name.as_str()))]))?;
            made.push(group);
            for (i, (out, port, channels)) in outputs.iter().enumerate() {
                let ch = e.insert(
                    ROOT,
                    "mixer.channels",
                    n + 1 + i,
                    Value::object([
                        ("name", Value::text(format!("{name} {port}"))),
                        ("channels", Value::int(*channels)),
                    ]),
                )?;
                e.set(ch, &[k("output")], Value::text(group.to_string()))?;
                e.set(*out, &[k("to")], Value::text(ch.to_string()))?;
                made.push(ch);
            }
            Ok(())
        })?;
        Ok(made)
    }
}
