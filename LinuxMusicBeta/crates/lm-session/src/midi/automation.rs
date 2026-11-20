//! Automation: recording and the heavy edits (interfaces.md, I-21.5).
//!
//! Recording: the renderer sends every touched value with its sample on a lossless ring (I-6.4, C3), from the hand
//! in the interface, from mapped MIDI and from a remote client alike. Each slot's pass is gathered here and
//! written when the hand lets go (Touch), when the transport stops (Latch, Write) and at each loop wrap: one
//! `Origin::Recording` transaction per pass (one undo step), simplified within the parameter's tolerance (0.1 dB
//! for gains, 1 cent for logarithmic parameters, 0.5 % of the range otherwise, exact steps for discrete ones). The
//! pass replaces what it covered; a lane that does not exist yet is created in the same transaction.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use lm_base::{Error, Label, Result};
use lm_doc::Origin;
use lm_doc::schemas::{KIND_CHANNEL, KIND_PLUGIN, KIND_TRACK};
use lm_render::{ParamRange, RangeScale, Touched};
use lm_state::{NodeId, PathStep, Value};

use crate::session::{Session, tempo_map};

/// How a lane records now (the global button over the lane's own mode)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecMode {
    Read,
    Touch,
    Latch,
    Write,
}

#[derive(Debug)]
struct Pass {
    node: NodeId,
    key: Arc<str>,
    mode: RecMode,
    points: Vec<(i64, f32)>,
}

#[derive(Debug, Default)]
pub(crate) struct Recorder {
    passes: BTreeMap<u32, Pass>,
    /// The last position each pass saw (a jump back is a loop wrap)
    last: HashMap<u32, i64>,
    playing: bool,
}

/// One point of a lane, as the document has it
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point {
    pub tick: i64,
    pub value: f64,
    pub curve: f64,
    pub step: bool,
}

pub(crate) fn read_points(lane: &Value) -> Vec<Point> {
    lane.get("points")
        .map(|l| {
            l.items()
                .map(|p| Point {
                    tick: p.get("tick").and_then(Value::as_int).unwrap_or(0),
                    value: p.get("value").and_then(Value::as_real).unwrap_or(0.0),
                    curve: p.get("curve").and_then(Value::as_real).unwrap_or(0.0),
                    step: p.get("shape").and_then(Value::as_text) == Some("step"),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn write_points(points: &[Point]) -> Result<Value> {
    let mut out = Vec::with_capacity(points.len());
    for p in points {
        let mut v = Value::object([("tick", Value::int(p.tick)), ("value", Value::real(p.value)?)]);
        if p.curve != 0.0 {
            v = v.with("curve", Value::real(p.curve)?);
        }
        if p.step {
            v = v.with("shape", Value::text("step"));
        }
        out.push(v);
    }
    Ok(Value::list(out))
}

/// The value of a lane's points at a tick (before the first point: the first value)
pub(crate) fn value_at(points: &[Point], tick: i64) -> Option<f64> {
    let first = points.first()?;
    let i = points.partition_point(|p| p.tick <= tick);
    if i == 0 {
        return Some(first.value);
    }
    let a = points[i - 1];
    let Some(b) = points.get(i) else { return Some(a.value) };
    if b.step || b.tick == a.tick {
        return Some(a.value);
    }
    let t = (tick - a.tick) as f64 / (b.tick - a.tick) as f64;
    Some(a.value + (b.value - a.value) * lm_render::shape(t, b.curve))
}

/// The distance between two values that the ear cannot tell apart, in the parameter's unit (I-21.5)
fn tolerance_space(range: &ParamRange, v: f64) -> f64 {
    match range.scale {
        // dB, 0.1 dB tolerance: scaled ×10 so the tolerance is 1
        RangeScale::Gain => 10.0 * 20.0 * v.max(1e-5).log10(),
        // cents, 1 cent tolerance
        RangeScale::Log => 1200.0 * v.max(1e-9).log2(),
        // 0.5 % of the range is 1
        RangeScale::Linear => v / (0.005 * f64::from(range.max - range.min).abs().max(1e-9)),
        RangeScale::Stepped => v,
    }
}

/// Ramer–Douglas–Peucker on (tick, value) with the vertical distance measured in the tolerance space (1 = the
/// tolerance). Discrete parameters keep exactly the points where the value changes, as steps
pub(crate) fn simplify(points: &[(i64, f64)], range: &ParamRange, scale: f64) -> Vec<Point> {
    if range.scale == RangeScale::Stepped {
        let mut out: Vec<Point> = Vec::new();
        for &(tick, value) in points {
            if out.last().is_none_or(|l| l.value != value) {
                out.push(Point { tick, value, curve: 0.0, step: true });
            }
        }
        return out;
    }
    if points.len() <= 2 {
        return points.iter().map(|&(tick, value)| Point { tick, value, curve: 0.0, step: false }).collect();
    }
    let space: Vec<f64> = points.iter().map(|p| tolerance_space(range, p.1)).collect();
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0usize, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (ta, tb) = (points[a].0 as f64, points[b].0 as f64);
        let mut worst = (0usize, 0.0f64);
        for k in a + 1..b {
            // The curve plays straight in the parameter's unit; the error is measured in the tolerance space
            let t = if tb > ta { (points[k].0 as f64 - ta) / (tb - ta) } else { 0.0 };
            let line = points[a].1 + (points[b].1 - points[a].1) * t;
            let d = (space[k] - tolerance_space(range, line)).abs();
            if d > worst.1 {
                worst = (k, d);
            }
        }
        if worst.1 > scale {
            keep[worst.0] = true;
            stack.push((a, worst.0));
            stack.push((worst.0, b));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(&(tick, value), _)| Point { tick, value, curve: 0.0, step: false })
        .collect()
}

/// Replaces `from..=to` of `old` with `new` (sorted), with jumps at both ends so the curve outside is unchanged
pub(crate) fn splice(old: &[Point], new: &[Point], from: i64, to: i64) -> Vec<Point> {
    let mut out: Vec<Point> = old.iter().copied().filter(|p| p.tick < from).collect();
    if let Some(v) = value_at(old, from)
        && !old.is_empty()
    {
        out.push(Point { tick: from, value: v, curve: 0.0, step: false });
    }
    out.extend_from_slice(new);
    if let Some(v) = value_at(old, to)
        && old.iter().any(|p| p.tick > to)
    {
        if let Some(last) = new.last() {
            out.push(Point { tick: to, value: last.value, curve: 0.0, step: false });
        }
        out.push(Point { tick: to, value: v, curve: 0.0, step: false });
    }
    out.extend(old.iter().copied().filter(|p| p.tick > to));
    out
}

impl Session {
    /// The node that keeps the automation of a parameter of `node`: a track or a channel, itself; a plugin, the
    /// track or channel it is in
    pub(crate) fn automation_owner(&self, node: NodeId) -> Option<NodeId> {
        match node.kind {
            KIND_TRACK | KIND_CHANNEL => Some(node),
            KIND_PLUGIN => {
                let root = self.document.root();
                let id_of = |v: &Value| v.get("id").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
                for t in root.get("tracks").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default() {
                    if t.get("instrument").and_then(id_of) == Some(node) {
                        return id_of(t);
                    }
                }
                let mixer = root.get("mixer")?;
                let channels = mixer.get("channels").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default();
                for c in channels.into_iter().chain(mixer.get("master")) {
                    if c.get("inserts").is_some_and(|l| l.items().any(|p| id_of(p) == Some(node))) {
                        return id_of(c);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// The lane of a parameter: (owner, the lane's id, the lane)
    pub(crate) fn automation_lane(&self, node: NodeId, key: &str) -> Option<(NodeId, Option<NodeId>, Option<Value>)> {
        let owner = self.automation_owner(node)?;
        let o = self.document.find(owner)?;
        let lane = o.get("automation").and_then(|l| {
            l.items()
                .find(|a| {
                    let t = a.get("target");
                    t.and_then(|t| t.get("node")).and_then(Value::as_text) == Some(&node.to_string())
                        && t.and_then(|t| t.get("param")).and_then(Value::as_text) == Some(key)
                })
                .cloned()
        });
        let id = lane.as_ref().and_then(|l| l.get("id")).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
        Some((owner, id, lane))
    }

    /// The mode a parameter records in now: the global button, or the lane's mode (no lane: only the global button)
    fn automation_mode(&self, node: NodeId, key: &str) -> Option<RecMode> {
        let global =
            self.document.root().get("transport").and_then(|t| t.get("automationMode")).and_then(Value::as_text);
        let lane = self.automation_lane(node, key).and_then(|(_, _, l)| l);
        let mode = match global.filter(|g| *g != "lanes") {
            Some(g) => g,
            None => lane.as_ref()?.get("mode").and_then(Value::as_text).unwrap_or("read"),
        };
        Some(match mode {
            "touch" => RecMode::Touch,
            "latch" => RecMode::Latch,
            "write" => RecMode::Write,
            "read" => RecMode::Read,
            _ => return None,
        })
    }

    /// A hand (a mapped control) moved a parameter: in Read or Touch, over a lane, "the hand won" until re-armed
    pub(crate) fn automation_hand(&mut self, node: NodeId, key: &Arc<str>) {
        let has_lane = self.automation_lane(node, key).is_some_and(|(_, id, _)| id.is_some());
        if has_lane && matches!(self.automation_mode(node, key), Some(RecMode::Read | RecMode::Touch)) {
            self.midi.hand_won.insert((node, key.clone()));
            self.telemetry.hand_won = self.midi.hand_won.iter().cloned().collect();
        }
    }

    /// Re-arm: every lane (or one) back to its curve
    pub(crate) fn automation_rearm(&mut self, lane: Option<NodeId>) {
        match lane {
            None => self.midi.hand_won.clear(),
            Some(l) => {
                let target = self.document.find(l).and_then(|v| v.get("target").cloned());
                let node = target
                    .as_ref()
                    .and_then(|t| t.get("node"))
                    .and_then(Value::as_text)
                    .and_then(|t| NodeId::parse(t).ok());
                let key = target.as_ref().and_then(|t| t.get("param")).and_then(Value::as_text).map(Arc::<str>::from);
                if let (Some(n), Some(k)) = (node, key) {
                    self.midi.hand_won.remove(&(n, k));
                }
            }
        }
        self.telemetry.hand_won = self.midi.hand_won.iter().cloned().collect();
    }

    /// The touched values of the renderer (I-6.4): the passes of Touch, Latch and Write
    pub(crate) fn automation_touched(&mut self, touched: &[Touched]) {
        if touched.is_empty() {
            return;
        }
        let by_slot: HashMap<u32, (NodeId, Arc<str>)> = self
            .table
            .slots
            .iter()
            .flat_map(|((n, k), slots)| slots.iter().map(move |s| (s.0, (*n, k.clone()))))
            .collect();
        for t in touched {
            let Some((node, key)) = by_slot.get(&t.slot.0).cloned() else { continue };
            let Some(mode) = self.automation_mode(node, &key) else { continue };
            if mode == RecMode::Read || (mode == RecMode::Touch && t.touching) {
                self.automation_hand(node, &key);
            }
            if mode == RecMode::Read {
                continue;
            }
            // A jump back in position: the loop wrapped, the pass so far is written and a new one starts
            if self.midi.automation.last.get(&t.slot.0).is_some_and(|l| t.sample < *l) {
                self.automation_finish(t.slot.0);
            }
            self.midi.automation.last.insert(t.slot.0, t.sample);
            let pass = self.midi.automation.passes.entry(t.slot.0).or_insert_with(|| Pass {
                node,
                key: key.clone(),
                mode,
                points: Vec::new(),
            });
            pass.points.push((t.sample, t.value));
            if !t.touching && mode == RecMode::Touch {
                self.automation_finish(t.slot.0);
            }
        }
        self.midi.automation.playing = self.telemetry.position.playing;
    }

    /// Each pump: a stop ends every pass (Latch and Write hold the last value up to the stop)
    pub(crate) fn automation_pump(&mut self) {
        let playing = self.telemetry.position.playing;
        if !playing && !self.midi.automation.passes.is_empty() {
            let stop = self.telemetry.position.samples;
            let slots: Vec<u32> = self.midi.automation.passes.keys().copied().collect();
            for s in slots {
                if let Some(p) = self.midi.automation.passes.get_mut(&s)
                    && matches!(p.mode, RecMode::Latch | RecMode::Write)
                    && let Some(&(at, v)) = p.points.last()
                    && stop > at
                {
                    p.points.push((stop, v));
                }
                self.automation_finish(s);
            }
            self.midi.automation.last.clear();
        }
        self.midi.automation.playing = playing;
    }

    /// Writes one pass: one transaction, one undo step (I-21.5)
    fn automation_finish(&mut self, slot: u32) {
        let Some(pass) = self.midi.automation.passes.remove(&slot) else { return };
        self.midi.automation.last.remove(&slot);
        if pass.points.is_empty() || !self.document.exists(pass.node) {
            return;
        }
        if let Err(e) = self.automation_write(&pass) {
            self.notices.error(&e, None, None);
        }
    }

    fn automation_write(&mut self, pass: &Pass) -> Result<()> {
        let map = tempo_map(self.document.root())?;
        let range = self.param_range(pass.node, &pass.key);
        let mut ticks: Vec<(i64, f64)> = pass.points.iter().map(|&(s, v)| (map.tick_at(s), f64::from(v))).collect();
        ticks.sort_by_key(|p| p.0);
        let new = simplify(&ticks, &range, 1.0);
        let (from, to) = (ticks[0].0, ticks[ticks.len() - 1].0);
        let (owner, lane_id, lane) =
            self.automation_lane(pass.node, &pass.key).ok_or_else(|| Error::new("automation.no_owner"))?;
        let old = lane.as_ref().map(read_points).unwrap_or_default();
        let points = write_points(&splice(&old, &new, from, to))?;
        let label = Label::new("automation.record").arg("target", format!("{} {}", pass.node, pass.key));
        let (node, key) = (pass.node, pass.key.clone());
        self.document
            .transact_as(label, Origin::Recording, |e| match lane_id {
                Some(l) => e.set(l, &[PathStep::key("points")], points),
                None => {
                    let n = e.find(owner).and_then(|o| o.get("automation").map(Value::len)).unwrap_or(0);
                    let lane = Value::object([
                        (
                            "target",
                            Value::object([("node", Value::text(node.to_string())), ("param", Value::text(&*key))]),
                        ),
                        ("points", points),
                    ]);
                    e.insert(owner, "automation", n, lane).map(|_| ())
                }
            })
            .map(|_| ())
    }

    // ---- the heavy edits (I-19 actions) ----

    fn lane_param(&self, lane: NodeId) -> Result<(NodeId, Arc<str>, Value)> {
        let l = self.document.find(lane).ok_or_else(|| Error::new("automation.no_lane"))?;
        let t = l.get("target").ok_or_else(|| Error::new("automation.no_lane"))?;
        let node = t.get("node").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok());
        let key = t.get("param").and_then(Value::as_text).map(Arc::<str>::from);
        match (node, key) {
            (Some(n), Some(k)) => Ok((n, k, l)),
            _ => Err(Error::new("automation.no_lane")),
        }
    }

    /// Thins a lane: the points that the tolerance says are redundant go (`from`..`to` in ticks, all by default;
    /// `tolerance` scales the parameter's tolerance)
    pub(crate) fn automation_thin(&mut self, lane: NodeId, from: i64, to: i64, tolerance: f64) -> Result<usize> {
        let (node, key, l) = self.lane_param(lane)?;
        let range = self.param_range(node, &key);
        let old = read_points(&l);
        let inside: Vec<(i64, f64)> = old
            .iter()
            .filter(|p| p.tick >= from && p.tick <= to && !p.step && p.curve == 0.0)
            .map(|p| (p.tick, p.value))
            .collect();
        if inside.len() < 3 {
            return Ok(0);
        }
        let kept = simplify(&inside, &range, tolerance.max(0.0));
        let removed = inside.len() - kept.len();
        let kept_ticks: Vec<(i64, u64)> = kept.iter().map(|p| (p.tick, p.value.to_bits())).collect();
        let points: Vec<Point> = old
            .into_iter()
            .filter(|p| {
                p.tick < from
                    || p.tick > to
                    || p.step
                    || p.curve != 0.0
                    || kept_ticks.contains(&(p.tick, p.value.to_bits()))
            })
            .collect();
        let v = write_points(&points)?;
        self.document.transact(Label::new("automation.thin"), |e| e.set(lane, &[PathStep::key("points")], v))?;
        Ok(removed)
    }

    /// Scales and offsets the values of a lane's points in `from..=to`, in the parameter's normalized space (its
    /// range; the fader law for gains): `norm · scale + offset`, kept in the range
    pub(crate) fn automation_scale(&mut self, lane: NodeId, from: i64, to: i64, scale: f64, offset: f64) -> Result<()> {
        let (node, key, l) = self.lane_param(lane)?;
        let range = self.param_range(node, &key);
        let points: Vec<Point> = read_points(&l)
            .into_iter()
            .map(|mut p| {
                if p.tick >= from && p.tick <= to {
                    let n = f64::from(range.normalize(p.value as f32)) * scale + offset;
                    p.value = f64::from(range.to_units(n.clamp(0.0, 1.0) as f32));
                }
                p
            })
            .collect();
        let v = write_points(&points)?;
        self.document
            .transact(Label::new("automation.scale"), |e| e.set(lane, &[PathStep::key("points")], v))
            .map(|_| ())
    }

    /// A lane becomes a control lane of a MIDI item (`cc` number, values 0..127 over the parameter's range), from
    /// the item's start to its end
    pub(crate) fn automation_to_item(&mut self, lane: NodeId, item: NodeId, cc: u8) -> Result<()> {
        let (node, key, l) = self.lane_param(lane)?;
        let range = self.param_range(node, &key);
        let it = self.document.find(item).ok_or_else(|| Error::new("automation.no_item"))?;
        let start = it.get("start").and_then(Value::as_int).unwrap_or(0);
        let length = it.get("length").and_then(Value::as_int).unwrap_or(0);
        let pts: Vec<Value> = read_points(&l)
            .into_iter()
            .filter(|p| p.tick >= start && p.tick < start + length)
            .map(|p| {
                let v = (range.normalize(p.value as f32) * 127.0).round() as i64;
                Value::object([("tick", Value::int(p.tick - start)), ("value", Value::int(v))])
            })
            .collect();
        let control = Value::object([
            ("kind", Value::text("cc")),
            ("number", Value::int(i64::from(cc))),
            ("channel", Value::int(0)),
            ("points", Value::list(pts)),
        ]);
        let controls = it.get("controls").cloned().unwrap_or_else(Value::empty_list);
        let kept = Value::list(
            controls
                .items()
                .filter(|c| {
                    !(c.get("kind").and_then(Value::as_text) == Some("cc")
                        && c.get("number").and_then(Value::as_int) == Some(i64::from(cc)))
                })
                .cloned()
                .chain([control]),
        );
        self.document
            .transact(Label::new("automation.to_item"), |e| e.set(item, &[PathStep::key("controls")], kept))
            .map(|_| ())
    }

    /// An item's control lane (`cc`) becomes the automation of `(owner node, param)`, replacing the range it covers
    pub(crate) fn automation_from_item(&mut self, item: NodeId, cc: u8, node: NodeId, key: &str) -> Result<()> {
        let range = self.param_range(node, key);
        let it = self.document.find(item).ok_or_else(|| Error::new("automation.no_item"))?;
        let start = it.get("start").and_then(Value::as_int).unwrap_or(0);
        let length = it.get("length").and_then(Value::as_int).unwrap_or(0);
        let lane = it.get("controls").and_then(|l| {
            l.items()
                .find(|c| {
                    c.get("kind").and_then(Value::as_text) == Some("cc")
                        && c.get("number").and_then(Value::as_int) == Some(i64::from(cc))
                })
                .cloned()
        });
        let lane = lane.ok_or_else(|| Error::new("automation.no_control"))?;
        let new: Vec<Point> = lane
            .get("points")
            .map(|l| {
                l.items()
                    .map(|p| Point {
                        tick: start + p.get("tick").and_then(Value::as_int).unwrap_or(0),
                        value: f64::from(
                            range.to_units(p.get("value").and_then(Value::as_int).unwrap_or(0) as f32 / 127.0),
                        ),
                        curve: 0.0,
                        step: false,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if new.is_empty() {
            return Ok(());
        }
        let (owner, lane_id, existing) =
            self.automation_lane(node, key).ok_or_else(|| Error::new("automation.no_owner"))?;
        let old = existing.as_ref().map(read_points).unwrap_or_default();
        let points = write_points(&splice(&old, &new, start, start + length))?;
        self.document
            .transact(Label::new("automation.from_item"), |e| match lane_id {
                Some(l) => e.set(l, &[PathStep::key("points")], points),
                None => {
                    let n = e.find(owner).and_then(|o| o.get("automation").map(Value::len)).unwrap_or(0);
                    let lane = Value::object([
                        (
                            "target",
                            Value::object([("node", Value::text(node.to_string())), ("param", Value::text(key))]),
                        ),
                        ("points", points),
                    ]);
                    e.insert(owner, "automation", n, lane).map(|_| ())
                }
            })
            .map(|_| ())
    }

    /// Moves items by `delta` ticks, and with them (by default) the automation points under them on their tracks'
    /// lanes (I-21.5): one transaction
    pub(crate) fn items_move(&mut self, items: &[NodeId], delta: i64, automation: bool) -> Result<()> {
        // The ranges each track's items cover before the move
        let mut ranges: BTreeMap<NodeId, Vec<(i64, i64)>> = BTreeMap::new();
        let mut starts = Vec::new();
        for t in self.document.root().get("tracks").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default()
        {
            let Some(tid) = t.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else { continue };
            for it in t.get("items").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                let Some(id) = it.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else {
                    continue;
                };
                if items.contains(&id) {
                    let s = it.get("start").and_then(Value::as_int).unwrap_or(0);
                    let l = it.get("length").and_then(Value::as_int).unwrap_or(0);
                    ranges.entry(tid).or_default().push((s, s + l));
                    starts.push((id, (s + delta).max(0)));
                }
            }
        }
        let mut lanes: Vec<(NodeId, Value)> = Vec::new();
        if automation {
            for (track, spans) in &ranges {
                let Some(t) = self.document.find(*track) else { continue };
                for lane in t.get("automation").map(|l| l.items().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                    let Some(lid) = lane.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok()) else {
                        continue;
                    };
                    let old = read_points(&lane);
                    let inside = |p: &Point| spans.iter().any(|(a, b)| p.tick >= *a && p.tick < *b);
                    let moved: Vec<Point> = old
                        .iter()
                        .filter(|p| inside(p))
                        .map(|p| Point { tick: (p.tick + delta).max(0), ..*p })
                        .collect();
                    if moved.is_empty() {
                        continue;
                    }
                    let mut rest: Vec<Point> = old.iter().copied().filter(|p| !inside(p)).collect();
                    // The moved points replace what was where they land
                    let (lo, hi) = (moved[0].tick, moved[moved.len() - 1].tick);
                    rest.retain(|p| p.tick < lo || p.tick > hi);
                    rest.extend(moved);
                    rest.sort_by_key(|p| p.tick);
                    lanes.push((lid, write_points(&rest)?));
                }
            }
        }
        self.document
            .transact(Label::new("items.move"), |e| {
                for (id, start) in &starts {
                    e.set(*id, &[PathStep::key("start")], Value::int(*start))?;
                }
                for (lane, points) in &lanes {
                    e.set(*lane, &[PathStep::key("points")], points.clone())?;
                }
                Ok(())
            })
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_keeps_the_shape_within_the_tolerance() {
        let range = ParamRange::linear(0.0, 1.0);
        // A slow sine sampled every 32 samples: far fewer points, and none off by more than 0.5 % of the range
        let pts: Vec<(i64, f64)> = (0..2000).map(|i| (i * 32, 0.5 + 0.4 * (i as f64 * 0.01).sin())).collect();
        let s = simplify(&pts, &range, 1.0);
        assert!(s.len() < pts.len() / 10, "{} points", s.len());
        for &(t, v) in &pts {
            let got = value_at(&s, t).unwrap();
            assert!((got - v).abs() <= 0.005 + 1e-9, "at {t}: {got} vs {v}");
        }
        // Discrete: exact steps
        let steps = vec![(0, 1.0), (10, 1.0), (20, 2.0), (30, 2.0), (40, 0.0)];
        let s = simplify(&steps, &ParamRange { min: 0.0, max: 4.0, scale: RangeScale::Stepped }, 1.0);
        assert_eq!(s.iter().map(|p| (p.tick, p.value)).collect::<Vec<_>>(), vec![(0, 1.0), (20, 2.0), (40, 0.0)]);
        assert!(s.iter().all(|p| p.step));
        // Gains: 0.1 dB
        let gain = ParamRange { min: 0.0, max: 4.0, scale: RangeScale::Gain };
        let pts: Vec<(i64, f64)> = (0..500).map(|i| (i * 10, 10f64.powf((-30.0 + 0.06 * i as f64) / 20.0))).collect();
        let s = simplify(&pts, &gain, 1.0);
        for &(t, v) in &pts {
            let got = value_at(&s, t).unwrap();
            assert!((20.0 * (got / v).log10()).abs() <= 0.11, "at {t}");
        }
    }

    #[test]
    fn splice_keeps_the_outside() {
        let old: Vec<Point> = [(0, 0.0), (100, 1.0), (200, 0.0)]
            .iter()
            .map(|&(tick, value)| Point { tick, value, curve: 0.0, step: false })
            .collect();
        let new = vec![
            Point { tick: 50, value: 0.3, curve: 0.0, step: false },
            Point { tick: 150, value: 0.4, curve: 0.0, step: false },
        ];
        let out = splice(&old, &new, 50, 150);
        assert_eq!(value_at(&out, 0), Some(0.0));
        assert_eq!(value_at(&out, 200), Some(0.0));
        assert!((value_at(&out, 100).unwrap() - 0.35).abs() < 1e-9);
        assert!((value_at(&out, 175).unwrap() - 0.25).abs() < 1e-9, "outside the pass the old curve: {out:?}");
    }
}
