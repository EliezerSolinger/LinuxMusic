//! The clips' and tracks' edits of the arrangement as actions (F-CLP-003, F-CLP-022, F-CLP-023, F-TRK-031): each is
//! one transaction, one undo step.
//!
//! ```text
//! clip.trim_start  { item: text, tick: int }       the left edge to `tick`; the content stays where it was
//! clip.glue        { items: [text] }                MIDI items of one track become the first one (F-CLP-022)
//! clip.transpose   { items: [text], semitones: int }  every note, kept inside 0..=127 (F-CLP-023)
//! clip.clear_notes { items: [text] }                every note out
//! clip.trim_notes { items: [text] }                 the notes past the item's end out, the last ones cut to it (F-CLP-022)
//! clip.fit         { items: [text] }                 the MIDI items end on the whole bar after their last note (F-CLP-025)
//! clip.relocate    { items: [text], delta: int, tracks?: [text], copy?: bool }  items by `delta` ticks, each to its
//!                  track of `tracks` (same index; absent: where it is), or copies (F-CLP-004, F-CLP-005)
//! clip.normalize  { items: [text], peak?: real }   audio items: the gain that brings the loudest sample they play to
//!                  `peak` dB (default 0; at most 0), 4 at most (F-CLP-065)
//! clip.reverse    { items: [text] }                audio items play backwards: each lane is written reversed as a new
//!                  resource in an operation, and the item (comp, fades) is mirrored when it is done (F-CLP-042)
//! track.move       { track: text, to: int, parent?: text|null, copy?: bool }  the track (with the tracks inside it)
//!                  before the `to`th track of the list, in the folder `parent` (F-TRK-029)
//! track.clone      { track: text }                  a copy of the track right below it (F-TRK-031)
//! track.clear      { track: text }                  every item out (F-TRK-031)
//! ```

use lm_base::{Error, Label, Result};
use lm_state::{NodeId, PathStep, ROOT, Value};

use crate::actions::ActionSpec;
use crate::session::{Session, tempo_map};

fn params() -> Error {
    Error::new("action.params")
}

fn node(p: &Value, key: &str) -> Result<NodeId> {
    p.get(key).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()).ok_or_else(params)
}

fn nodes(p: &Value, key: &str) -> Result<Vec<NodeId>> {
    p.get(key)
        .ok_or_else(params)?
        .items()
        .map(|v| v.as_text().and_then(|t| NodeId::parse(t).ok()).ok_or_else(params))
        .collect()
}

fn int(v: &Value, k: &str, d: i64) -> i64 {
    v.get(k).and_then(Value::as_int).unwrap_or(d)
}

fn writable(s: &Session) -> bool {
    !s.project().read_only
}

pub static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "clip.trim_start",
        label: "action.clip.trim_start",
        run: |s, p| {
            let tick = p.get("tick").and_then(Value::as_int).ok_or_else(params)?;
            s.trim_start(node(p, "item")?, tick)
        },
        enabled: writable,
    },
    ActionSpec {
        id: "clip.glue",
        label: "action.clip.glue",
        run: |s, p| s.glue_items(&nodes(p, "items")?).map(|_| ()),
        enabled: writable,
    },
    ActionSpec {
        id: "clip.transpose",
        label: "action.clip.transpose",
        run: |s, p| {
            let by = p.get("semitones").and_then(Value::as_int).ok_or_else(params)?;
            s.transpose_items(&nodes(p, "items")?, by)
        },
        enabled: writable,
    },
    ActionSpec {
        id: "clip.clear_notes",
        label: "action.clip.clear_notes",
        run: |s, p| s.clear_notes(&nodes(p, "items")?),
        enabled: writable,
    },
    ActionSpec {
        id: "clip.trim_notes",
        label: "action.clip.trim_notes",
        run: |s, p| s.trim_notes(&nodes(p, "items")?),
        enabled: writable,
    },
    ActionSpec {
        id: "clip.fit",
        label: "action.clip.fit",
        run: |s, p| s.fit_items(&nodes(p, "items")?),
        enabled: writable,
    },
    ActionSpec {
        id: "clip.relocate",
        label: "action.clip.relocate",
        run: |s, p| {
            let tracks = if p.get("tracks").is_some() { nodes(p, "tracks")? } else { Vec::new() };
            let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);
            s.relocate_items(&nodes(p, "items")?, int(p, "delta", 0), &tracks, copy).map(|_| ())
        },
        enabled: writable,
    },
    ActionSpec {
        id: "clip.normalize",
        label: "action.clip.normalize",
        run: |s, p| {
            let db = p.get("peak").and_then(Value::as_real).unwrap_or(0.0);
            s.normalize_items(&nodes(p, "items")?, db)
        },
        enabled: writable,
    },
    ActionSpec {
        id: "clip.reverse",
        label: "action.clip.reverse",
        run: |s, p| s.reverse_items(&nodes(p, "items")?),
        enabled: writable,
    },
    ActionSpec {
        id: "track.move",
        label: "action.track.move",
        run: |s, p| {
            let to = usize::try_from(int(p, "to", 0)).map_err(|_| params())?;
            let parent = match p.get("parent") {
                Some(v) if !v.is_null() => Some(node(p, "parent")?),
                _ => None,
            };
            let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);
            s.move_track(node(p, "track")?, to, parent, copy)
        },
        enabled: writable,
    },
    ActionSpec {
        id: "track.clone",
        label: "action.track.clone",
        run: |s, p| s.clone_track(node(p, "track")?).map(|_| ()),
        enabled: writable,
    },
    ActionSpec {
        id: "track.clear",
        label: "action.track.clear",
        run: |s, p| s.clear_track(node(p, "track")?),
        enabled: writable,
    },
];

/// A lane an audio item plays: the lane, its resource, the source frame at the item's start, and the stretch of
/// the item (in samples from its start) that plays it
struct LaneUse {
    lane: NodeId,
    resource: lm_state::Resource,
    offset: i64,
    from: i64,
    to: i64,
}

/// The lanes an audio item plays and its length in samples: the comp's segments (their stretches joined per lane), or
/// the last lane over the whole item
fn lane_uses(item: &Value, map: &lm_time::TempoMap) -> (Vec<LaneUse>, i64) {
    let (start, length) = (int(item, "start", 0), int(item, "length", 1));
    let audio_time = item.get("timebase").and_then(Value::as_text) == Some("audio");
    // An item's own unit (ticks, or samples on an audio timebase) from its start, in samples
    let rel = |x: i64| if audio_time { x } else { map.sample_at(start.saturating_add(x)) - map.sample_at(start) };
    let len = rel(length).max(1);
    let lanes: Vec<&Value> = item.get("lanes").map(|l| l.items().collect()).unwrap_or_default();
    let id_of = |l: &Value| l.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
    let make = |l: &Value, from: i64, to: i64| -> Option<LaneUse> {
        Some(LaneUse {
            lane: id_of(l)?,
            resource: l.get("source")?.as_resource()?.clone(),
            offset: int(l, "sourceOffset", 0),
            from,
            to,
        })
    };
    let comp: Vec<&Value> = item.get("comp").map(|c| c.items().collect()).unwrap_or_default();
    let mut out: Vec<LaneUse> = Vec::new();
    if comp.is_empty() {
        out.extend(lanes.last().and_then(|l| make(l, 0, len)));
    } else {
        for seg in comp {
            let Some(wanted) = seg.get("lane").and_then(Value::as_text) else { continue };
            let Some(l) = lanes.iter().find(|l| l.get("id").and_then(Value::as_text) == Some(wanted)) else { continue };
            let (from, to) = (rel(int(seg, "from", 0).clamp(0, length)), rel(int(seg, "to", 0).clamp(0, length)));
            if to <= from {
                continue;
            }
            match out.iter_mut().find(|u| Some(u.lane) == id_of(l)) {
                Some(u) => {
                    u.from = u.from.min(from);
                    u.to = u.to.max(to);
                }
                None => out.extend(make(l, from, to)),
            }
        }
    }
    (out, len)
}

fn notes_of(item: &Value) -> Vec<(u64, Value)> {
    item.get("notes")
        .map(|m| m.entries().filter_map(|(n, v)| Some((n.parse().ok()?, v.clone()))).collect())
        .unwrap_or_default()
}

impl Session {
    /// The left edge of an item to `tick`, the content staying in place (F-CLP-003): the notes and the control
    /// points move back by what was cut, the audio takes start further on, the comp is clipped. Error:
    /// `clip.trim.outside` when the tick is not inside the item
    pub fn trim_start(&mut self, item: NodeId, tick: i64) -> Result<()> {
        let root = self.document().root().clone();
        let map = tempo_map(&root)?;
        let v = self.document().find(item).ok_or_else(|| Error::new("action.no_such_node"))?;
        let audio_time = v.get("timebase").and_then(Value::as_text) == Some("audio");
        let (start, length) = (int(&v, "start", 0), int(&v, "length", 1));
        let at = if audio_time { map.sample_at(tick) } else { tick };
        let d = at - start;
        if d == 0 {
            return Ok(());
        }
        if d >= length {
            return Err(Error::new("clip.trim.outside").arg("tick", tick));
        }
        // Extending to the left is allowed only where there is content to show: audio with a source offset
        let skip = if audio_time { d } else { map.sample_at(start + d) - map.sample_at(start) };
        let lanes: Vec<(NodeId, i64)> = v
            .get("lanes")
            .map(|l| {
                l.items()
                    .filter_map(|lane| {
                        Some((NodeId::parse(lane.get("id")?.as_text()?).ok()?, int(lane, "sourceOffset", 0)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let is_audio = v.get("kind").and_then(Value::as_text) == Some("audio");
        if d < 0 && (!is_audio || lanes.iter().any(|(_, o)| *o + skip < 0)) {
            return Err(Error::new("clip.trim.outside").arg("tick", tick));
        }
        let k = PathStep::key;
        self.document_mut()
            .transact(Label::new("action.clip.trim_start"), |e| {
                e.set(item, &[k("start")], Value::int(at))?;
                e.set(item, &[k("length")], Value::int(length - d))?;
                if d > 0 {
                    e.set(item, &[k("fadeIn")], Value::int(0))?;
                }
                for (n, note) in notes_of(&v) {
                    let (s, l) = (int(&note, "start", 0), int(&note, "length", 1));
                    if s - d + l <= 0 {
                        e.remove_note(item, n)?;
                    } else if s - d < 0 {
                        e.set_note(item, n, &[k("start")], Value::int(0))?;
                        e.set_note(item, n, &[k("length")], Value::int(l + (s - d)))?;
                    } else {
                        e.set_note(item, n, &[k("start")], Value::int(s - d))?;
                    }
                }
                if let Some(comp) = v.get("comp").filter(|c| !c.is_empty()) {
                    let moved: Vec<Value> = comp
                        .items()
                        .filter_map(|seg| {
                            let (a, b) = ((int(seg, "from", 0) - d).max(0), int(seg, "to", 0) - d);
                            (b > a).then(|| seg.with("from", Value::int(a)).with("to", Value::int(b)))
                        })
                        .collect();
                    e.set(item, &[k("comp")], Value::list(moved))?;
                }
                if let Some(controls) = v.get("controls") {
                    let kept: Vec<Value> = controls
                        .items()
                        .map(|c| {
                            let pts: Vec<Value> = c
                                .get("points")
                                .map(|l| {
                                    l.items()
                                        .filter(|p| int(p, "tick", 0) >= d)
                                        .map(|p| p.with("tick", Value::int(int(p, "tick", 0) - d)))
                                        .collect()
                                })
                                .unwrap_or_default();
                            c.with("points", Value::list(pts))
                        })
                        .collect();
                    e.set(item, &[k("controls")], Value::list(kept))?;
                }
                for (lane, offset) in &lanes {
                    e.set(*lane, &[k("sourceOffset")], Value::int(offset + skip))?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// "Join the selection" (F-CLP-022): the MIDI items of one track become the earliest one, with all the notes
    /// (the control lanes of the others are dropped); the first one grows to the last end. One undo step. Errors:
    /// `clip.glue.count` (fewer than two), `clip.glue.kind` (audio, or items of different tracks)
    pub fn glue_items(&mut self, items: &[NodeId]) -> Result<NodeId> {
        if items.len() < 2 {
            return Err(Error::new("clip.glue.count"));
        }
        let mut all = Vec::new();
        let mut track = None;
        for id in items {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            let parent = self.document().loc(*id).map(|l| l.parent).ok_or_else(|| Error::new("action.no_such_node"))?;
            if v.get("kind").and_then(Value::as_text) == Some("audio") || track.is_some_and(|t| t != parent) {
                return Err(Error::new("clip.glue.kind"));
            }
            track = Some(parent);
            all.push((int(&v, "start", 0), *id, v));
        }
        all.sort_by_key(|(s, id, _)| (*s, id.number));
        let (first_start, first, first_v) = (all[0].0, all[0].1, all[0].2.clone());
        let end = all.iter().map(|(s, _, v)| s + int(v, "length", 1)).max().unwrap_or(first_start + 1);
        let k = PathStep::key;
        self.document_mut()
            .transact(Label::new("action.clip.glue"), |e| {
                for (s, _, v) in &all[1..] {
                    for (_, mut note) in notes_of(v) {
                        note = note.with("start", Value::int(int(&note, "start", 0) + s - first_start));
                        e.add_note(first, note)?;
                    }
                }
                e.set(first, &[k("length")], Value::int(end - first_start))?;
                e.set(first, &[k("fadeOut")], Value::int(int(&first_v, "fadeOut", 0).min(0)))?;
                for (_, id, _) in &all[1..] {
                    e.remove(*id)?;
                }
                Ok(())
            })
            .map(|_| first)
    }

    /// Transposes the notes of the items by `by` semitones, limited so that every note stays in 0..=127 (F-CLP-023)
    pub fn transpose_items(&mut self, items: &[NodeId], by: i64) -> Result<()> {
        let mut plan = Vec::new();
        let (mut lo, mut hi) = (i64::MIN, i64::MAX);
        for id in items {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            for (n, note) in notes_of(&v) {
                let key = int(&note, "key", 60);
                lo = lo.max(-key);
                hi = hi.min(127 - key);
                plan.push((*id, n, key));
            }
        }
        if plan.is_empty() {
            return Ok(());
        }
        let by = by.clamp(lo.min(hi), hi.max(lo));
        self.document_mut()
            .transact(Label::new("action.clip.transpose"), |e| {
                for (id, n, key) in &plan {
                    e.set_note(*id, *n, &[PathStep::key("key")], Value::int((key + by).clamp(0, 127)))?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// Every note of the items out (F-CLP-022)
    pub fn clear_notes(&mut self, items: &[NodeId]) -> Result<()> {
        let mut plan = Vec::new();
        for id in items {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            plan.extend(notes_of(&v).into_iter().map(|(n, _)| (*id, n)));
        }
        self.document_mut()
            .transact(Label::new("action.clip.clear_notes"), |e| {
                for (id, n) in &plan {
                    e.remove_note(*id, *n)?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// A copy of the track right below it, with its items, its output routing and its instrument (F-TRK-031)
    pub fn clone_track(&mut self, track: NodeId) -> Result<NodeId> {
        let loc = self.document().loc(track).cloned().ok_or_else(|| Error::new("action.no_such_node"))?;
        let clip = self.document().copy(track)?;
        let list = loc.container.join(".");
        let index = self
            .document()
            .find(loc.parent)
            .and_then(|t| t.get(&list).cloned())
            .and_then(|l| l.items().position(|x| x.get("id").and_then(Value::as_text) == Some(&track.to_string())))
            .map_or(usize::MAX, |i| i + 1);
        let mut made = None;
        self.document_mut().transact(Label::new("action.track.clone"), |e| {
            let len = e.find(loc.parent).and_then(|t| t.get(&list)).map_or(0, Value::len);
            let parent = if loc.parent == ROOT { ROOT } else { loc.parent };
            made = e.paste(parent, &list, index.min(len), &clip)?;
            Ok(())
        })?;
        made.ok_or_else(|| Error::new("track.clone.refused"))
    }

    /// Every item of the track out, in one step (F-TRK-031 "Clear")
    pub fn clear_track(&mut self, track: NodeId) -> Result<()> {
        let v = self.document().find(track).ok_or_else(|| Error::new("action.no_such_node"))?;
        let ids: Vec<NodeId> = v
            .get("items")
            .map(|l| {
                l.items()
                    .filter_map(|i| i.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok()))
                    .collect()
            })
            .unwrap_or_default();
        self.document_mut()
            .transact(Label::new("action.track.clear"), |e| {
                for id in &ids {
                    e.remove(*id)?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// "Cut what is left over" (F-CLP-022): the notes that start at or after the item's end out, those that cross it
    /// cut to it. One undo step
    pub fn trim_notes(&mut self, items: &[NodeId]) -> Result<()> {
        let mut plan = Vec::new();
        for id in items {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            let length = int(&v, "length", 1);
            for (n, note) in notes_of(&v) {
                let (start, len) = (int(&note, "start", 0), int(&note, "length", 1));
                if start >= length {
                    plan.push((*id, n, None));
                } else if start + len > length {
                    plan.push((*id, n, Some(length - start)));
                }
            }
        }
        self.document_mut()
            .transact(Label::new("action.clip.trim_notes"), |e| {
                for (id, n, cut) in &plan {
                    match cut {
                        None => e.remove_note(*id, *n)?,
                        Some(len) => e.set_note(*id, *n, &[PathStep::key("length")], Value::int(*len))?,
                    }
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// Items moved by `delta` ticks, each to the track `tracks[i]` (absent: its own), or copies of them there
    /// (F-CLP-004, F-CLP-005). All or nothing: a MIDI item goes only to an instrument track, an audio item only to an
    /// audio one, none to a folder (error `clip.relocate.kind`). One undo step; the ids of the items or the copies
    pub fn relocate_items(
        &mut self,
        items: &[NodeId],
        delta: i64,
        tracks: &[NodeId],
        copy: bool,
    ) -> Result<Vec<NodeId>> {
        if items.is_empty() {
            return Err(Error::new("clip.relocate.empty"));
        }
        let map = tempo_map(self.document().root())?;
        struct Plan {
            id: NodeId,
            from: NodeId,
            target: NodeId,
            start: i64,
            clip: lm_doc::Clip,
        }
        let mut plans = Vec::new();
        for (i, id) in items.iter().enumerate() {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            let from = self.document().loc(*id).map(|l| l.parent).ok_or_else(|| Error::new("action.no_such_node"))?;
            let target = tracks.get(i).copied().unwrap_or(from);
            if target != from {
                let t = self.document().find(target).ok_or_else(|| Error::new("action.no_such_node"))?;
                let folder = t.get("folder").and_then(Value::as_bool).unwrap_or(false);
                let instrument = t.get("instrument").is_some_and(|x| !x.is_null());
                let audio = v.get("kind").and_then(Value::as_text) == Some("audio");
                if folder || audio == instrument {
                    return Err(Error::new("clip.relocate.kind").arg("track", target.to_string()));
                }
            }
            let start = int(&v, "start", 0);
            let start = if v.get("timebase").and_then(Value::as_text) == Some("audio") {
                map.sample_at((map.tick_at(start) + delta).max(0))
            } else {
                (start + delta).max(0)
            };
            plans.push(Plan { id: *id, from, target, start, clip: self.document().copy(*id)? });
        }
        let mut made = Vec::new();
        let label = if copy { "action.clip.copy_to" } else { "action.clip.relocate" };
        self.document_mut().transact(Label::new(label), |e| {
            for p in &plans {
                let n = e.find(p.target).and_then(|t| t.get("items")).map_or(0, Value::len);
                let id = if copy {
                    e.paste(p.target, "items", n, &p.clip)?.ok_or_else(|| Error::new("clip.relocate.refused"))?
                } else {
                    if p.target != p.from {
                        e.move_to(p.id, p.target, "items", n)?;
                    }
                    p.id
                };
                e.set(id, &[PathStep::key("start")], Value::int(p.start))?;
                made.push(id);
            }
            Ok(())
        })?;
        Ok(made)
    }

    /// Moves a track, with the tracks inside it when it is a folder, before the `to`th track of the list (a position
    /// of the list as it is now), into the folder `parent` (none: out of every folder); `copy` makes a copy there
    /// instead (the folder's tracks stay). Errors: `track.move.parent` (not a folder, or the track or one of its own),
    /// `track.move.into_itself` (dropped inside its own block), `track.move.index`
    pub fn move_track(&mut self, track: NodeId, to: usize, parent: Option<NodeId>, copy: bool) -> Result<()> {
        let root = self.document().root().clone();
        let list: Vec<(NodeId, Option<NodeId>)> = root
            .get("tracks")
            .map(|l| {
                l.items()
                    .filter_map(|t| {
                        let id = NodeId::parse(t.get("id")?.as_text()?).ok()?;
                        let parent = t.get("parent").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok());
                        Some((id, parent))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let pos = list.iter().position(|(id, _)| *id == track).ok_or_else(|| Error::new("action.no_such_node"))?;
        if to > list.len() {
            return Err(Error::new("track.move.index").arg("index", to as i64));
        }
        // the track and everything inside it
        let inside = |mut at: Option<NodeId>| {
            while let Some(a) = at {
                if a == track {
                    return true;
                }
                at = list.iter().find(|(id, _)| *id == a).and_then(|(_, p)| *p);
            }
            false
        };
        let block: Vec<NodeId> =
            list.iter().filter(|(id, p)| *id == track || (!copy && inside(*p))).map(|(id, _)| *id).collect();
        if let Some(p) = parent {
            let folder = self.document().find(p).and_then(|t| t.get("folder").and_then(Value::as_bool)) == Some(true);
            if !folder || p == track || inside(Some(p)) {
                return Err(Error::new("track.move.parent").arg("track", p.to_string()));
            }
        }
        let mut order: Vec<NodeId> = list.iter().map(|(id, _)| *id).collect();
        let current_parent = list[pos].1;
        if copy {
            let clip = self.document().copy(track)?;
            let at = to.min(order.len());
            return self
                .document_mut()
                .transact(Label::new("action.track.move"), |e| {
                    let made = e.paste(ROOT, "tracks", at, &clip)?.ok_or_else(|| Error::new("track.move.refused"))?;
                    e.set(
                        made,
                        &[PathStep::key("parent")],
                        parent.map_or(Value::NULL, |p| Value::text(p.to_string())),
                    )?;
                    Ok(())
                })
                .map(|_| ());
        }
        let first = order.iter().position(|id| block.contains(id)).unwrap_or(pos);
        let last = order.iter().rposition(|id| block.contains(id)).unwrap_or(first);
        if to > first && to <= last {
            return Err(Error::new("track.move.into_itself"));
        }
        let before = order[..to.min(order.len())].iter().filter(|id| !block.contains(id)).count();
        order.retain(|id| !block.contains(id));
        for (k, id) in block.iter().enumerate() {
            order.insert(before + k, *id);
        }
        let moved = order.iter().zip(list.iter()).any(|(a, (b, _))| a != b);
        if !moved && parent == current_parent {
            return Ok(());
        }
        self.document_mut()
            .transact(Label::new("action.track.move"), |e| {
                for (i, id) in order.iter().enumerate() {
                    let here = e.find(ROOT).and_then(|r| r.get("tracks")).and_then(|l| {
                        l.items().position(|t| t.get("id").and_then(Value::as_text) == Some(&id.to_string()))
                    });
                    if here != Some(i) {
                        e.move_to(*id, ROOT, "tracks", i)?;
                    }
                }
                if parent != current_parent {
                    e.set(
                        track,
                        &[PathStep::key("parent")],
                        parent.map_or(Value::NULL, |p| Value::text(p.to_string())),
                    )?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// "Auto size" (F-CLP-025): each MIDI item with notes ends on the whole bar after its last note (the tempo map's
    /// bars); the notes stay. One undo step
    pub fn fit_items(&mut self, items: &[NodeId]) -> Result<()> {
        let map = tempo_map(self.document().root())?;
        let mut plan = Vec::new();
        for id in items {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            if v.get("kind").and_then(Value::as_text) == Some("audio") {
                continue;
            }
            let Some(last) = notes_of(&v).iter().map(|(_, n)| int(n, "start", 0) + int(n, "length", 1)).max() else {
                continue;
            };
            let start = int(&v, "start", 0);
            let bar = map.bar_beat_tick((start + last - 1).max(0)).bar;
            let end = map.bar_start(bar + 1)?;
            plan.push((*id, (end - start).max(1)));
        }
        self.document_mut()
            .transact(Label::new("action.clip.fit"), |e| {
                for (id, length) in &plan {
                    e.set(*id, &[PathStep::key("length")], Value::int(*length))?;
                }
                Ok(())
            })
            .map(|_| ())
    }
}

impl Session {
    /// "Normalize" (F-CLP-065): each audio item gets the gain that brings the loudest sample it plays to `peak_db`
    /// (0 at the most), from the source's peaks (a block of the pyramid counts whole, so the result is never above
    /// the target); a silent item keeps its gain, and the gain stops at the field's maximum. One undo step. Error:
    /// `clip.normalize.loading` while a source's peaks are not there yet
    pub fn normalize_items(&mut self, items: &[NodeId], peak_db: f64) -> Result<()> {
        if !peak_db.is_finite() {
            return Err(params());
        }
        let target = 10f64.powf(peak_db.min(0.0) / 20.0);
        let map = tempo_map(self.document().root())?;
        let mut plan = Vec::new();
        for id in items {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            if v.get("kind").and_then(Value::as_text) != Some("audio") {
                continue;
            }
            let (uses, _) = lane_uses(&v, &map);
            let mut peak = 0.0f64;
            for u in &uses {
                let peaks = self
                    .resources()
                    .peaks(u.resource.key())
                    .ok_or_else(|| Error::new("clip.normalize.loading").arg("resource", u.resource.key()))?;
                let block = lm_io::Peaks::frames_per_pair(0);
                let (a, b) = ((u.offset + u.from).max(0) as u64, (u.offset + u.to).max(0) as u64);
                if b <= a {
                    continue;
                }
                let (first, last) = ((a / block) as usize, b.div_ceil(block) as usize);
                for ch in 0..peaks.channels() {
                    for pair in peaks.range(0, ch, first, last).chunks(2) {
                        let m = pair.iter().map(|v| f64::from(*v).abs()).fold(0.0, f64::max) / 32767.0;
                        peak = peak.max(m);
                    }
                }
            }
            if peak > 1e-6 {
                plan.push((*id, (target / peak).clamp(0.0, 4.0)));
            }
        }
        self.document_mut()
            .transact(Label::new("action.clip.normalize"), |e| {
                for (id, gain) in &plan {
                    e.set(*id, &[PathStep::key("gain")], Value::real(*gain)?)?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// "Reverse" (F-CLP-042, P-31): each lane an audio item plays is written backwards over the item's length as a
    /// new audio resource (an operation on the item, cancellable); when it is done the item's lanes point to those,
    /// the comp's segments and the fades are mirrored, and the resources load. The item shows loading meanwhile
    pub fn reverse_items(&mut self, items: &[NodeId]) -> Result<()> {
        let map = tempo_map(self.document().root())?;
        let project = self.project().folder.root().to_path_buf();
        let rate = self.spec().sample_rate;
        let mut jobs = Vec::new();
        for id in items {
            let v = self.document().find(*id).ok_or_else(|| Error::new("action.no_such_node"))?;
            if v.get("kind").and_then(Value::as_text) != Some("audio") {
                continue;
            }
            let (uses, len) = lane_uses(&v, &map);
            for u in uses {
                let path = lm_io::resource_path(&project, u.resource.key())?
                    .ok_or_else(|| Error::new("clip.reverse.missing").arg("resource", u.resource.key()))?;
                jobs.push((*id, u, path, len));
            }
        }
        let Some(first) = jobs.first().map(|j| j.0) else { return Ok(()) };
        self.start(
            Label::new("session.reversing"),
            lm_port::Target::Node(first),
            true,
            "clip.reverse",
            move |cancel, progress| {
                let mut made = Vec::new();
                for (item, u, path, len) in &jobs {
                    cancel.check()?;
                    let done = lm_io::reverse::reverse_range(
                        path,
                        &project,
                        u.offset.max(0) as u64,
                        *len as u64,
                        rate,
                        cancel,
                        progress,
                    )?;
                    made.push(Value::object([
                        ("item", Value::text(item.to_string())),
                        ("lane", Value::text(u.lane.to_string())),
                        ("old", Value::resource(u.resource.clone())),
                        ("offset", Value::int(u.offset)),
                        ("key", Value::text(done.key)),
                    ]));
                }
                Ok(Value::list(made))
            },
        );
        Ok(())
    }

    /// The reversal is written: the lanes that still play what was reversed point to the new resources, from frame 0;
    /// each item whose lanes changed is mirrored, once: the comp's segments (and the crossfade at each boundary), the
    /// fades and their tensions. A lane or item edited meanwhile is left alone
    pub(crate) fn reverse_finished(&mut self, result: &Value) -> Result<()> {
        let mut lanes: Vec<(NodeId, NodeId, lm_state::Resource)> = Vec::new();
        let mut items: Vec<NodeId> = Vec::new();
        let mut keep = Vec::new();
        for r in result.items() {
            let text = |k: &str| r.get(k).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
            let (Some(item), Some(lane)) = (text("item"), text("lane")) else { continue };
            let Some(key) = r.get("key").and_then(Value::as_text) else { continue };
            let new = lm_state::Resource::new("audio", key)?;
            let Some(l) = self.document().find(lane) else { continue };
            // Still the lane that was reversed: the same source at the same frame
            if l.get("source").and_then(Value::as_resource) != r.get("old").and_then(Value::as_resource)
                || int(&l, "sourceOffset", 0) != int(r, "offset", 0)
            {
                continue;
            }
            keep.push(new.clone());
            lanes.push((item, lane, new));
            if !items.contains(&item) {
                items.push(item);
            }
        }
        let k = PathStep::key;
        self.document_mut().transact(Label::new("action.clip.reverse"), |e| {
            for (_, lane, new) in &lanes {
                e.set(*lane, &[k("source")], Value::resource(new.clone()))?;
                e.set(*lane, &[k("sourceOffset")], Value::int(0))?;
            }
            for item in &items {
                let v = match e.find(*item) {
                    Some(v) => v.clone(),
                    None => continue,
                };
                let length = int(&v, "length", 1);
                let (fi, fo) = (int(&v, "fadeIn", 0), int(&v, "fadeOut", 0));
                e.set(*item, &[k("fadeIn")], Value::int(fo))?;
                e.set(*item, &[k("fadeOut")], Value::int(fi))?;
                let (ci, co) = (real(&v, "fadeInCurve"), real(&v, "fadeOutCurve"));
                e.set(*item, &[k("fadeInCurve")], Value::real(co)?)?;
                e.set(*item, &[k("fadeOutCurve")], Value::real(ci)?)?;
                if let Some(comp) = v.get("comp").filter(|c| !c.is_empty()) {
                    let mut segs: Vec<&Value> = comp.items().collect();
                    segs.sort_by_key(|s| int(s, "from", 0));
                    let n = segs.len();
                    // Mirrored in time; the crossfade into a segment is the one the boundary had on its other side
                    let mirrored: Vec<Value> = (0..n)
                        .map(|j| {
                            let s = segs[n - 1 - j];
                            let b = &segs[if j == 0 { 0 } else { n - j }];
                            s.with("from", Value::int((length - int(s, "to", 0)).max(0)))
                                .with("to", Value::int((length - int(s, "from", 0)).max(0)))
                                .with("fade", Value::int(int(b, "fade", 240)))
                        })
                        .collect();
                    e.set(*item, &[k("comp")], Value::list(mirrored))?;
                }
            }
            Ok(())
        })?;
        for r in keep {
            self.request(&r);
        }
        Ok(())
    }
}

fn real(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_real).unwrap_or(0.0)
}

