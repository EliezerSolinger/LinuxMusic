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
