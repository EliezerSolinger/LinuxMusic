//! The arrangement's and the desk's domain edits as actions (F-ARR-005, F-ARR-007, F-MIX-012): each is one
//! transaction, one undo step, and keeps the item's notes, control lanes, takes and comp coherent.
//!
//! ```text
//! arrange.split      { item: text | items: [text], tick: int }  one undo step; `items: []` = every item there
//! arrange.duplicate  { item: text | items: [text] }             the group after itself, whole bars later
//! arrange.copy       { items: [text] }                  the session's clipboard (F-ARR-006)
//! arrange.cut        { items: [text] }                  copy, then remove them (one step)
//! arrange.paste      { at: int (ticks), track?: text }  the copies at `at`, the first on `track` (one step)
//! arrange.insert_bar { tick: int }                      a bar of the meter at `tick` (F-ARR-002)
//! arrange.remove_bar { tick: int }
//! mixer.create_channels_for_outputs { track: text (NodeId) }
//! mixer.group        { channels: [text], name?: text }   a group channel before the first of them (P-18, one step)
//! ```

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, PathStep, ROOT, Value};

use crate::actions::ActionSpec;
use crate::session::{Session, tempo_map};

fn node_param(p: &Value, key: &str) -> Result<NodeId> {
    p.get(key).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(|| Error::new("action.params"))
}

fn node_list(p: &Value, key: &str) -> Result<Vec<NodeId>> {
    let list = p.get(key).ok_or_else(|| Error::new("action.params"))?;
    list.items()
        .map(|v| v.as_text().and_then(|t| NodeId::parse(t).ok()).ok_or_else(|| Error::new("action.params")))
        .collect()
}

fn int_param(p: &Value, key: &str) -> Result<i64> {
    p.get(key).and_then(Value::as_int).ok_or_else(|| Error::new("action.params"))
}

/// An item on the arrangement's clipboard: the copy, the track it came from, its start in ticks
#[derive(Clone)]
pub(crate) struct Copied {
    clip: lm_doc::Clip,
    track: NodeId,
    start: i64,
}

/// `item` (one) or `items` (a list; for the split an empty list means every item)
fn items_param(p: &Value) -> Result<Vec<NodeId>> {
    if p.get("items").is_some() { node_list(p, "items") } else { Ok(vec![node_param(p, "item")?]) }
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
            s.split_items(&items_param(p)?, tick).map(|_| ())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "arrange.duplicate",
        label: "action.arrange.duplicate",
        run: |s, p| s.duplicate_items(&items_param(p)?).map(|_| ()),
        enabled: writable,
    },
    ActionSpec {
        id: "arrange.copy",
        label: "action.arrange.copy",
        run: |s, p| s.copy_items(&node_list(p, "items")?),
        enabled: |_| true,
    },
    ActionSpec {
        id: "arrange.cut",
        label: "action.arrange.cut",
        run: |s, p| s.cut_items(&node_list(p, "items")?),
        enabled: writable,
    },
    ActionSpec {
        id: "arrange.paste",
        label: "action.arrange.paste",
        run: |s, p| {
            let at = p.get("at").and_then(Value::as_int).ok_or_else(|| Error::new("action.params"))?;
            let track = p.get("track").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
            s.paste_items(at, track).map(|_| ())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "arrange.insert_bar",
        label: "action.arrange.insert_bar",
        run: |s, p| s.shift_bar(int_param(p, "tick")?, true),
        enabled: writable,
    },
    ActionSpec {
        id: "arrange.remove_bar",
        label: "action.arrange.remove_bar",
        run: |s, p| s.shift_bar(int_param(p, "tick")?, false),
        enabled: writable,
    },
    ActionSpec {
        id: "mixer.create_channels_for_outputs",
        label: "action.mixer.create_channels_for_outputs",
        run: |s, p| s.create_channels_for_outputs(node_param(p, "track")?).map(|_| ()),
        enabled: writable,
    },
    ActionSpec {
        id: "mixer.group",
        label: "action.mixer.group",
        run: |s, p| {
            let name = p.get("name").and_then(Value::as_text).map(str::to_owned);
            s.group_channels(&node_list(p, "channels")?, name).map(|_| ())
        },
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
        self.split_items(&[item], tick)?.first().copied().ok_or_else(|| Error::new("arrange.split.refused"))
    }

    /// [`Session::split_item`] on several items in one undo step; those the tick is not inside are left alone (an
    /// empty list: every item the tick is inside, F-ARR-005). The right parts' ids. Error: `arrange.split.outside`
    /// when the tick is inside none of them
    pub fn split_items(&mut self, items: &[NodeId], tick: i64) -> Result<Vec<NodeId>> {
        let root = self.document().root().clone();
        let map = tempo_map(&root)?;
        let all: Vec<NodeId>;
        let items = if items.is_empty() {
            all = all_items(&root);
            &all[..]
        } else {
            items
        };
        let mut plans = Vec::new();
        for item in items {
            let v = self.document().find(*item).ok_or_else(|| Error::new("action.no_such_node"))?;
            let audio_time = v.get("timebase").and_then(Value::as_text) == Some("audio");
            let (start, length) = (int(&v, "start", 0), int(&v, "length", 1));
            // A pattern that repeats inside the item (its `loop` shorter than the item)
            let repeat = v.get("loop").and_then(Value::as_int).filter(|l| *l >= 1 && *l < length);
            // The cut in the item's own unit (ticks, or samples for an item in audio time)
            let at = if audio_time { map.sample_at(tick) } else { tick };
            let cut = at - start;
            if cut <= 0 || cut >= length {
                continue;
            }
            // How far into its takes the right part starts, in samples
            let skip = if audio_time { cut } else { map.sample_at(start + cut) - map.sample_at(start) };
            let loc = self.document().loc(*item).cloned().ok_or_else(|| Error::new("action.no_such_node"))?;
            let clip = self.document().copy(*item)?;
            plans.push(SplitPlan { item: *item, at, cut, length, skip, repeat, loc, clip });
        }
        if plans.is_empty() {
            return Err(Error::new("arrange.split.outside").arg("tick", tick));
        }
        let mut made = Vec::new();
        self.document_mut().transact(Label::new("action.arrange.split"), |e| {
            for p in &plans {
                made.push(split_in(e, p)?);
            }
            Ok(())
        })?;
        Ok(made)
    }
}

/// One item's split, computed before the transaction
struct SplitPlan {
    item: NodeId,
    at: i64,
    cut: i64,
    length: i64,
    skip: i64,
    repeat: Option<i64>,
    loc: lm_doc::Loc,
    clip: lm_doc::Clip,
}

/// Every item of the tracks and the desk's channels
fn all_items(root: &Value) -> Vec<NodeId> {
    let mut owners: Vec<&Value> = root.get("tracks").map(|l| l.items().collect()).unwrap_or_default();
    if let Some(m) = root.get("mixer") {
        owners.extend(m.get("channels").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default());
        owners.extend(m.get("master"));
    }
    owners
        .iter()
        .flat_map(|o| o.get("items").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default())
        .filter_map(|it| it.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()))
        .collect()
}

fn split_in(e: &mut lm_doc::Edit<'_>, p: &SplitPlan) -> Result<NodeId> {
    let SplitPlan { item, at, cut, length, skip, repeat, ref loc, ref clip } = *p;
    let k = PathStep::key;
    let list = loc.container.join(".");
    let index = e
        .find(loc.parent)
        .and_then(|t| t.get("items"))
        .and_then(|l| l.items().position(|x| x.get("id").and_then(Value::as_text) == Some(&item.to_string())))
        .map_or(0, |i| i + 1);
    let right = e.paste(loc.parent, &list, index, clip)?.ok_or_else(|| Error::new("arrange.split.refused"))?;
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
    Ok(right)
}

impl Session {
    /// A copy of the item right after it, whole bars later (Ctrl+D, F-ARR-006)
    pub fn duplicate_item(&mut self, item: NodeId) -> Result<NodeId> {
        self.duplicate_items(&[item])?.first().copied().ok_or_else(|| Error::new("arrange.duplicate.refused"))
    }

    /// Copies of the items as a group (the legacy's Ctrl+D on a selection, F-ARR-006): each on its own track, the
    /// group moved by whole bars so that it starts at or after its end. One undo step; the copies' ids
    pub fn duplicate_items(&mut self, items: &[NodeId]) -> Result<Vec<NodeId>> {
        let map = tempo_map(self.document().root())?;
        // Each item: (start tick, in audio time, end in its unit, its location, its copy)
        let mut group = Vec::new();
        let (mut first, mut end_tick) = (i64::MAX, i64::MIN);
        for item in items {
            let v = self.document().find(*item).ok_or_else(|| Error::new("action.no_such_node"))?;
            let audio_time = v.get("timebase").and_then(Value::as_text) == Some("audio");
            let (start, end) = (int(&v, "start", 0), int(&v, "start", 0) + int(&v, "length", 1));
            let (s, e) = if audio_time { (map.tick_at(start), map.tick_at(end)) } else { (start, end) };
            first = first.min(s);
            end_tick = end_tick.max(e);
            let loc = self.document().loc(*item).cloned().ok_or_else(|| Error::new("action.no_such_node"))?;
            group.push((s, audio_time, end, loc, self.document().copy(*item)?));
        }
        if group.is_empty() {
            return Err(Error::new("arrange.copy.empty"));
        }
        // As the legacy (SongEditor::copyPaste, Key_D): whole bars of the map from the group's start, at least one,
        // until the copy starts at or after the group's end; the place in the bar is kept
        let bbt = map.bar_beat_tick(first);
        let bar0 = map.bar_start(bbt.bar)?;
        let mut n = 1;
        let mut to = map.bar_start(bbt.bar + n)? + (first - bar0);
        while to < end_tick {
            n += 1;
            to = map.bar_start(bbt.bar + n)? + (first - bar0);
        }
        let offset = to - first;
        let mut made = Vec::new();
        self.document_mut().transact(Label::new("action.arrange.duplicate"), |e| {
            for (s, audio_time, end, loc, clip) in &group {
                let list = loc.container.join(".");
                let n = e.find(loc.parent).and_then(|t| t.get("items")).map_or(0, Value::len);
                let copy =
                    e.paste(loc.parent, &list, n, clip)?.ok_or_else(|| Error::new("arrange.duplicate.refused"))?;
                let to = if *audio_time { map.sample_at(s + offset).max(*end) } else { s + offset };
                e.set(copy, &[PathStep::key("start")], Value::int(to))?;
                made.push(copy);
            }
            Ok(())
        })?;
        Ok(made)
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

impl Session {
    /// Groups channels (P-18): a new channel before the first of them, their outputs into it, in one undo step. A
    /// group is a channel that others output to; its own output stays the default (the master). Ids that are not
    /// channels of the desk are left out. Errors: `action.params` (none left), `mixer.group.empty`
    pub fn group_channels(&mut self, channels: &[NodeId], name: Option<String>) -> Result<NodeId> {
        let list: Vec<NodeId> = self
            .document()
            .root()
            .get("mixer")
            .and_then(|m| m.get("channels"))
            .map(|l| l.items().filter_map(|v| NodeId::parse(v.get("id")?.as_text()?).ok()).collect())
            .unwrap_or_default();
        let members: Vec<NodeId> = channels.iter().copied().filter(|c| list.contains(c)).collect();
        let Some(index) = members.iter().filter_map(|c| list.iter().position(|l| l == c)).min() else {
            return Err(Error::new("mixer.group.empty"));
        };
        let name = name.unwrap_or_else(|| format!("Group {}", list.len() + 1));
        let mut made = None;
        self.document_mut().transact(Label::new("action.mixer.group"), |e| {
            let group =
                e.insert(ROOT, "mixer.channels", index, Value::object([("name", Value::text(name.as_str()))]))?;
            for id in &members {
                e.set(*id, &[PathStep::key("output")], Value::text(group.to_string()))?;
            }
            made = Some(group);
            Ok(())
        })?;
        made.ok_or_else(|| Error::new("mixer.group.empty"))
    }

    /// The items' copies on the clipboard, by their start (F-ARR-006). Error: `arrange.copy.empty`
    pub fn copy_items(&mut self, items: &[NodeId]) -> Result<()> {
        let map = tempo_map(self.document().root())?;
        let mut out = Vec::new();
        for id in items {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            let track = self.document().loc(*id).map(|l| l.parent).ok_or_else(|| Error::new("action.no_such_node"))?;
            let start = int(&v, "start", 0);
            let start =
                if v.get("timebase").and_then(Value::as_text) == Some("audio") { map.tick_at(start) } else { start };
            out.push(Copied { clip: self.document().copy(*id)?, track, start });
        }
        if out.is_empty() {
            return Err(Error::new("arrange.copy.empty"));
        }
        out.sort_by_key(|c| c.start);
        self.arrange_clipboard = out;
        Ok(())
    }

    /// Copy, then remove the items: one undo step
    pub fn cut_items(&mut self, items: &[NodeId]) -> Result<()> {
        self.copy_items(items)?;
        self.document_mut()
            .transact(Label::new("action.arrange.cut"), |e| {
                for id in items {
                    e.remove(*id)?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// The clipboard's items at `at` (ticks), keeping their distances; the first on `track` (else its own track)
    /// and the others on the tracks as far from it as they were. One undo step. The new items' ids
    pub fn paste_items(&mut self, at: i64, track: Option<NodeId>) -> Result<Vec<NodeId>> {
        let copies = self.arrange_clipboard.clone();
        let first = copies.first().ok_or_else(|| Error::new("arrange.paste.empty"))?;
        let root = self.document().root().clone();
        let map = tempo_map(&root)?;
        let tracks: Vec<NodeId> = root
            .get("tracks")
            .map(|l| l.items().filter_map(|t| t.get("id")?.as_text().and_then(|x| NodeId::parse(x).ok())).collect())
            .unwrap_or_default();
        let index = |id: NodeId| tracks.iter().position(|t| *t == id);
        let shift = match (track.and_then(index), index(first.track)) {
            (Some(to), Some(from)) => to as i64 - from as i64,
            _ => 0,
        };
        let (base, mut made) = (first.start, Vec::new());
        self.document_mut().transact(Label::new("action.arrange.paste"), |e| {
            for c in &copies {
                let to = index(c.track).map(|i| i as i64 + shift).filter(|i| *i >= 0 && (*i as usize) < tracks.len());
                let parent = to.map_or(c.track, |i| tracks[i as usize]);
                let n = e.find(parent).and_then(|t| t.get("items")).map_or(0, Value::len);
                let copy = e.paste(parent, "items", n, &c.clip)?.ok_or_else(|| Error::new("arrange.paste.refused"))?;
                let tick = (at + c.start - base).max(0);
                let audio = c.clip.node.get("timebase").and_then(Value::as_text) == Some("audio");
                e.set(copy, &[PathStep::key("start")], Value::int(if audio { map.sample_at(tick) } else { tick }))?;
                made.push(copy);
            }
            Ok(())
        })?;
        Ok(made)
    }

    /// Inserts (or removes) a bar of the meter at `tick` (F-ARR-002, the legacy's Track::insertBar/removeBar): the
    /// items that start at or after it move a bar later (earlier); the automation points after it move with them
    /// (removing: the points inside the removed bar go); the markers and regions too. One undo step
    pub fn shift_bar(&mut self, tick: i64, insert: bool) -> Result<()> {
        let root = self.document().root().clone();
        let map = tempo_map(&root)?;
        let seg = map.segment_at_tick(tick.max(0));
        let bar = seg.time_signature().bar_ticks(map.ppq()).unwrap_or(3840).max(1);
        let d = if insert { bar } else { -bar };
        let id_of = |v: &Value| v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
        let mut items: Vec<(NodeId, i64)> = Vec::new();
        let mut lanes: Vec<(NodeId, Value)> = Vec::new();
        let mut owners: Vec<&Value> = root.get("tracks").map(|l| l.items().collect()).unwrap_or_default();
        if let Some(m) = root.get("mixer") {
            owners.extend(m.get("channels").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default());
            owners.extend(m.get("master"));
        }
        for o in &owners {
            for it in o.get("items").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default() {
                let (Some(id), start) = (id_of(it), int(it, "start", 0)) else { continue };
                let audio = it.get("timebase").and_then(Value::as_text) == Some("audio");
                let t = if audio { map.tick_at(start) } else { start };
                if t >= tick {
                    let nt = (t + d).max(tick.min(t));
                    items.push((id, if audio { map.sample_at(nt) } else { nt }));
                }
            }
            for a in o.get("automation").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default() {
                let Some(id) = id_of(a) else { continue };
                let pts: Vec<Value> = a.get("points").map(|l| l.items().cloned().collect()).unwrap_or_default();
                let moved: Vec<Value> = pts
                    .into_iter()
                    .filter(|p| insert || !(tick..tick + bar).contains(&int(p, "tick", 0)))
                    .map(|p| {
                        let t = int(&p, "tick", 0);
                        if t >= tick { p.with("tick", Value::int((t + d).max(0))) } else { p }
                    })
                    .collect();
                lanes.push((id, Value::list(moved)));
            }
        }
        let marks: Vec<(NodeId, &'static str, i64)> = ["markers", "regions"]
            .iter()
            .flat_map(|list| root.get(list).map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default())
            .flat_map(|m| {
                let id = id_of(&m);
                ["tick", "from", "to"]
                    .into_iter()
                    .filter_map(move |k| Some((id?, k, m.get(k)?.as_int()?)))
                    .filter(|(_, _, t)| *t >= tick)
                    .map(|(id, k, t)| (id, k, (t + d).max(tick)))
                    .collect::<Vec<_>>()
            })
            .collect();
        let label = if insert { "action.arrange.insert_bar" } else { "action.arrange.remove_bar" };
        self.document_mut()
            .transact(Label::new(label), |e| {
                for (id, start) in &items {
                    e.set(*id, &[PathStep::key("start")], Value::int(*start))?;
                }
                for (id, pts) in &lanes {
                    e.set(*id, &[PathStep::key("points")], pts.clone())?;
                }
                for (id, k, t) in &marks {
                    e.set(*id, &[PathStep::key(k)], Value::int(*t))?;
                }
                Ok(())
            })
            .map(|_| ())
    }
}
