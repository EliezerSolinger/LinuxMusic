//! The fake's MIDI mapping (I-21.2): learning, unmapping, the interface's mapped actions and the raw activity, with
//! the mappings kept where the real session keeps them (the project's `midiMap`, the preferences' `midiMap`). There
//! is no MIDI device: the tests play messages in with `FakePort::midi_in`.

use std::collections::VecDeque;
use std::sync::Arc;

use lm_doc::Edit;
use lm_port::{
    ActionId, DocSource, Error, HostTime, Label, LearnId, MapScope, MapTarget, Mapping, MidiActivity, MidiMessage,
    MidiUiAction, NodeId, NodeRef, PathStep, ROOT, Result, Severity, Value,
};

use crate::FakePort;

/// The activity kept (I-21.2: the last 256)
const ACTIVITY: usize = 256;

#[derive(Debug, Default)]
pub struct Midi {
    learning: Option<(LearnId, MapTarget, MapScope)>,
    next: LearnId,
    ui: Vec<MidiUiAction>,
    activity: VecDeque<(u64, MidiActivity)>,
    counter: u64,
}

fn mappings(list: Option<&Value>) -> Vec<Mapping> {
    list.map(|l| l.items().filter_map(|m| Mapping::from_value(m).ok()).collect()).unwrap_or_default()
}

fn matches(source: &DocSource, port: &str, channel: u8, message: MidiMessage) -> bool {
    (source.port.is_empty() || &*source.port == port)
        && source.channel.is_none_or(|c| c == channel)
        && source.message == message
}

impl FakePort {
    pub(crate) fn midi_learn_impl(&mut self, target: MapTarget, scope: MapScope) -> Result<LearnId> {
        if let Some(NodeRef::Id(id)) = target.node()
            && self.doc.find(*id).is_none()
        {
            return Err(Error::new("port.no_such_node"));
        }
        self.midi.next += 1;
        let id = self.midi.next;
        self.midi.learning = Some((id, target, scope));
        Ok(id)
    }

    pub(crate) fn cancel_learn_impl(&mut self, id: LearnId) {
        if self.midi.learning.as_ref().is_some_and(|(l, ..)| *l == id) {
            self.midi.learning = None;
        }
    }

    pub(crate) fn learning_impl(&self) -> Option<LearnId> {
        self.midi.learning.as_ref().map(|(id, ..)| *id)
    }

    /// The mappings of a scope, as the interface reads them
    pub fn mappings(&self, scope: MapScope) -> Vec<Mapping> {
        match scope {
            MapScope::Project => mappings(self.doc.root().get("midiMap")),
            MapScope::Machine => mappings(self.preferences.get("midiMap")),
        }
    }

    pub(crate) fn unmap_impl(&mut self, target: &MapTarget, scope: MapScope) -> Result<()> {
        match scope {
            MapScope::Project => {
                let ids: Vec<NodeId> = self
                    .mappings(MapScope::Project)
                    .into_iter()
                    .filter(|m| &m.target == target)
                    .filter_map(|m| m.id)
                    .collect();
                if ids.is_empty() {
                    return Ok(());
                }
                self.doc
                    .transact(Label::new("MIDI: tirar o mapa"), |e| {
                        for id in &ids {
                            e.remove(*id)?;
                        }
                        Ok(())
                    })
                    .map(|_| ())
            }
            MapScope::Machine => {
                let kept: Vec<Value> = self
                    .preferences
                    .get("midiMap")
                    .map(|l| {
                        l.items()
                            .filter(|m| Mapping::from_value(m).is_ok_and(|m| &m.target != target))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                self.preferences = self.preferences.with("midiMap", Value::list(kept));
                Ok(())
            }
        }
    }

    pub(crate) fn drain_midi_ui_impl(&mut self, out: &mut Vec<MidiUiAction>) {
        out.append(&mut self.midi.ui);
    }

    pub(crate) fn midi_activity_impl(&self, since: u64) -> (u64, Vec<MidiActivity>) {
        let new = self.midi.activity.iter().filter(|(n, _)| *n > since).map(|(_, a)| a.clone()).collect();
        (self.midi.counter, new)
    }

    /// A MIDI message arrives (the tests' controller): it is shown in the activity; a learn under way takes it as
    /// the source of a new mapping; a mapping to an action sends it to the interface
    pub fn midi_in(&mut self, port: &str, channel: u8, message: MidiMessage, value: u16) {
        let mut mapped = false;
        if let Some((_, target, scope)) = self.midi.learning.take() {
            let mapping = Mapping {
                id: None,
                source: DocSource { port: Arc::from(port), channel: Some(channel), message },
                touch: None,
                target,
                transform: Default::default(),
                feedback: true,
                consume: true,
                feedback_to: None,
            };
            match self.add_mapping(mapping, scope) {
                Ok(()) => {
                    self.push_notice(
                        "midi.learned",
                        vec![("port", Value::text(port))],
                        Severity::Info,
                        None,
                        Vec::new(),
                    );
                    mapped = true;
                }
                Err(e) => self.push_notice(
                    "midi.learn_refused",
                    vec![("error", Value::text(e.key().to_string()))],
                    Severity::Warning,
                    None,
                    Vec::new(),
                ),
            }
        } else {
            let all = [self.mappings(MapScope::Project), self.mappings(MapScope::Machine)].concat();
            let full = if matches!(message, MidiMessage::Cc { .. }) { 127.0 } else { 16383.0 };
            for m in all.iter().filter(|m| matches(&m.source, port, channel, message)) {
                mapped = true;
                if let MapTarget::Action { id, params, .. } = &m.target {
                    self.midi.ui.push(MidiUiAction {
                        action: ActionId(id.clone()),
                        params: params.clone(),
                        value: f32::from(value) / full,
                    });
                }
            }
        }
        self.midi.counter += 1;
        let a = MidiActivity { port: Arc::from(port), channel, message, value, mapped, time: HostTime::now() };
        self.midi.activity.push_back((self.midi.counter, a));
        while self.midi.activity.len() > ACTIVITY {
            self.midi.activity.pop_front();
        }
    }

    fn add_mapping(&mut self, mapping: Mapping, scope: MapScope) -> Result<()> {
        let node = mapping.to_value();
        match scope {
            MapScope::Project => self
                .doc
                .transact(Label::new("MIDI: aprender"), |e: &mut Edit<'_>| {
                    if e.root().get("midiMap").is_none() {
                        e.set(ROOT, &[PathStep::key("midiMap")], Value::empty_list())?;
                    }
                    let n = e.root().get("midiMap").map_or(0, Value::len);
                    e.insert(ROOT, "midiMap", n, node)?;
                    Ok(())
                })
                .map(|_| ()),
            MapScope::Machine => {
                let mut list: Vec<Value> =
                    self.preferences.get("midiMap").map(|l| l.items().cloned().collect()).unwrap_or_default();
                list.push(node);
                self.preferences = self.preferences.with("midiMap", Value::list(list));
                Ok(())
            }
        }
    }

    /// The hand overrode a lane's parameter (the tests' stand-in for the engine's report, I-21.5)
    pub fn hand_wins(&mut self, owner: NodeId, param: &str) {
        let key: Arc<str> = Arc::from(param);
        if !self.telemetry.hand_won.iter().any(|(n, k)| *n == owner && *k == key) {
            self.telemetry.hand_won.push((owner, key));
        }
    }

    /// `automation.rearm`: every lane back to its curve, or one (`{"lane": "a:5"}`)
    pub(crate) fn rearm(&mut self, params: &Value) -> Result<()> {
        match params.get("lane").and_then(Value::as_text) {
            None => self.telemetry.hand_won.clear(),
            Some(lane) => {
                let lane = self.doc.find(NodeId::parse(lane)?).ok_or_else(|| Error::new("port.no_such_node"))?;
                let target = lane.get("target");
                let node =
                    target.and_then(|t| t.get("node")).and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok());
                let param = target.and_then(|t| t.get("param")).and_then(Value::as_text).unwrap_or("");
                self.telemetry.hand_won.retain(|(n, k)| Some(*n) != node || &**k != param);
            }
        }
        Ok(())
    }
}

fn id_in(params: &Value, key: &str) -> Result<NodeId> {
    params
        .get(key)
        .and_then(Value::as_text)
        .and_then(|t| NodeId::parse(t).ok())
        .ok_or_else(|| Error::new("port.params"))
}

fn tick_of(p: &Value) -> i64 {
    p.get("tick").and_then(Value::as_int).unwrap_or(0)
}

fn value_of(p: &Value) -> f64 {
    p.get("value").and_then(Value::as_real).unwrap_or(0.0)
}

impl FakePort {
    /// `items.move {items, delta, automation}` (I-21.5): items move by `delta` ticks, and with `automation` (the
    /// default) the points of their tracks' lanes under them move too; one transaction
    pub(crate) fn move_items(&mut self, params: &Value) -> Result<()> {
        let ids: Vec<NodeId> = params
            .get("items")
            .map(|l| l.items().filter_map(|v| v.as_text().and_then(|t| NodeId::parse(t).ok())).collect())
            .unwrap_or_default();
        let delta = params.get("delta").and_then(Value::as_int).ok_or_else(|| Error::new("port.params"))?;
        let with_automation = params.get("automation").and_then(Value::as_bool).unwrap_or(true);
        let mut moves = Vec::new();
        for id in &ids {
            let item = self.doc.find(*id).ok_or_else(|| Error::new("port.no_such_node"))?;
            let track = self.doc.loc(*id).map(|l| l.parent).ok_or_else(|| Error::new("port.no_such_node"))?;
            let start = item.get("start").and_then(Value::as_int).unwrap_or(0);
            let length = item.get("length").and_then(Value::as_int).unwrap_or(1);
            moves.push((*id, track, start, length));
        }
        let lanes: Vec<(NodeId, NodeId, Value)> = if with_automation {
            moves
                .iter()
                .flat_map(|(_, track, ..)| {
                    let t = self.doc.find(*track);
                    t.and_then(|t| t.get("automation").cloned())
                        .map(|l| {
                            l.items()
                                .filter_map(|a| crate::lm_id(a).map(|id| (*track, id, a.clone())))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .collect()
        } else {
            Vec::new()
        };
        self.doc
            .transact(Label::new("Move"), |e| {
                for (id, _, start, _) in &moves {
                    e.set(*id, &[PathStep::key("start")], Value::int((start + delta).max(0)))?;
                }
                for (track, lane, node) in &lanes {
                    let spans: Vec<(i64, i64)> =
                        moves.iter().filter(|m| m.1 == *track).map(|m| (m.2, m.2 + m.3)).collect();
                    let mut points: Vec<Value> = node
                        .get("points")
                        .map(|l| {
                            l.items()
                                .map(|p| {
                                    let t = tick_of(p);
                                    if spans.iter().any(|(a, b)| t >= *a && t < *b) {
                                        p.with("tick", Value::int((t + delta).max(0)))
                                    } else {
                                        p.clone()
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    points.sort_by_key(tick_of);
                    e.set(*lane, &[PathStep::key("points")], Value::list(points))?;
                }
                Ok(())
            })
            .map(|_| ())
    }

    /// `automation.thin {lane, from?, to?, tolerance?}` (I-21.5): the points a straight line through their neighbours
    /// already gives (within the tolerance) go
    pub(crate) fn thin(&mut self, params: &Value) -> Result<()> {
        let lane = id_in(params, "lane")?;
        let from = params.get("from").and_then(Value::as_int).unwrap_or(i64::MIN);
        let to = params.get("to").and_then(Value::as_int).unwrap_or(i64::MAX);
        let tolerance = params.get("tolerance").and_then(Value::as_real).unwrap_or(1e-3);
        let node = self.doc.find(lane).ok_or_else(|| Error::new("port.no_such_node"))?;
        let points: Vec<Value> = node.get("points").map(|l| l.items().cloned().collect()).unwrap_or_default();
        let mut kept: Vec<Value> = Vec::new();
        for (i, p) in points.iter().enumerate() {
            let (t, v) = (tick_of(p), value_of(p));
            let inner = i > 0 && i + 1 < points.len() && t >= from && t <= to;
            if inner {
                let prev = kept.last().expect("the first point is kept");
                let next = &points[i + 1];
                let (t0, v0, t1, v1) = (tick_of(prev), value_of(prev), tick_of(next), value_of(next));
                let line = if t1 == t0 { v0 } else { v0 + (v1 - v0) * (t - t0) as f64 / (t1 - t0) as f64 };
                if (line - v).abs() <= tolerance {
                    continue;
                }
            }
            kept.push(p.clone());
        }
        self.doc
            .transact(Label::new("Thin"), |e| e.set(lane, &[PathStep::key("points")], Value::list(kept.clone())))
            .map(|_| ())
    }
}

/// What reaches the fake from outside the interface, as a device and the engine would: MIDI messages and the hand
/// overriding a lane. A test keeps the handle (`FakePort::outside`) after giving the port away; `pump` applies it
#[derive(Clone, Debug, Default)]
pub struct Outside(std::sync::Arc<std::sync::Mutex<Vec<OutsideEvent>>>);

#[derive(Clone, Debug)]
enum OutsideEvent {
    Midi { port: String, channel: u8, message: MidiMessage, value: u16 },
    HandWins { owner: NodeId, param: String },
}

impl Outside {
    /// A MIDI message from a controller
    pub fn midi(&self, port: &str, channel: u8, message: MidiMessage, value: u16) {
        self.push(OutsideEvent::Midi { port: port.to_string(), channel, message, value });
    }

    /// The hand overrode a lane's parameter
    pub fn hand_wins(&self, owner: NodeId, param: &str) {
        self.push(OutsideEvent::HandWins { owner, param: param.to_string() });
    }

    fn push(&self, e: OutsideEvent) {
        if let Ok(mut q) = self.0.lock() {
            q.push(e);
        }
    }

    fn take(&self) -> Vec<OutsideEvent> {
        self.0.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }
}

impl FakePort {
    /// The handle to play MIDI and engine events into this port from a test
    pub fn outside(&self) -> Outside {
        self.outside.clone()
    }

    /// Applies what came from outside since the last pump; whether anything did
    pub(crate) fn pump_outside(&mut self) -> bool {
        let events = self.outside.take();
        let any = !events.is_empty();
        for e in events {
            match e {
                OutsideEvent::Midi { port, channel, message, value } => self.midi_in(&port, channel, message, value),
                OutsideEvent::HandWins { owner, param } => self.hand_wins(owner, &param),
            }
        }
        any
    }
}
