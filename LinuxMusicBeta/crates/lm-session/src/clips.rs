//! The clips' and tracks' edits of the arrangement as actions (F-CLP-003, F-CLP-022, F-CLP-023, F-TRK-031): each is
//! one transaction, one undo step.
//!
//! ```text
//! clip.trim_start  { item: text, tick: int }       the left edge to `tick`; the content stays where it was
//! clip.glue        { items: [text] }                MIDI items of one track become the first one (F-CLP-022)
//! clip.transpose   { items: [text], semitones: int }  every note, kept inside 0..=127 (F-CLP-023)
//! clip.clear_notes { items: [text] }                every note out
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
}
