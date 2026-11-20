//! Step recording (F-PAS-001) and recording into the item open in the Piano Roll (F-NOT-018, F-REC-029), in the
//! core (interfaces.md, I-21.4).
//!
//! Step: a cursor in one MIDI item. Keys played on the item's track input (or sent by the editor for the computer
//! keyboard) become a chord at the cursor when the last key is released; the cursor then advances one step. Each
//! chord is one undo step. The cursor lives in the interface document (`stepRecord: {item, at, step}`), where the
//! editor draws it. Keys still play the instrument as always.
//!
//! Record into: `recordInto: "c:N"` in the interface document makes the next recording write its notes and control
//! lanes into that item (overdub or replace) instead of a new item.

use std::collections::BTreeMap;

use lm_base::{Error, Label, Result};
use lm_render::PortIndex;
use lm_state::{NodeId, PathStep, ROOT, Value};

use crate::session::Session;

/// The step cursor
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Step {
    pub item: NodeId,
    pub track: NodeId,
    /// Ticks from the item's start
    pub at: i64,
    pub step: i64,
    /// `None`: the played velocity
    pub velocity: Option<f64>,
    /// Keys down now, and the chord gathered since the first went down
    held: BTreeMap<u8, f64>,
    chord: Vec<(u8, f64)>,
    /// The last chord's notes (for step_back and step_length) and where it started
    last: Option<(i64, Vec<u64>)>,
}

impl Session {
    fn item_track(&self, item: NodeId) -> Option<NodeId> {
        let root = self.document.root();
        for t in root.get("tracks").map(|l| l.items().collect::<Vec<_>>()).unwrap_or_default() {
            if t.get("items")
                .is_some_and(|l| l.items().any(|i| i.get("id").and_then(Value::as_text) == Some(&item.to_string())))
            {
                return t.get("id").and_then(Value::as_text).and_then(|x| NodeId::parse(x).ok());
            }
        }
        None
    }

    /// The cursor into the interface document (the editor draws it), or null
    fn step_publish(&mut self) {
        let v = match &self.midi.step {
            Some(s) => Value::object([
                ("item", Value::text(s.item.to_string())),
                ("at", Value::int(s.at)),
                ("step", Value::int(s.step)),
            ]),
            None => Value::NULL,
        };
        let _ = self.ui_document.transact(Label::new("ui.step"), |e| e.set(ROOT, &[PathStep::key("stepRecord")], v));
    }

    /// `notes.step_record {item, at?, step?, velocity?}`: starts step mode, or (the same item, no `at`) changes the step
    pub(crate) fn step_start(&mut self, p: &Value) -> Result<()> {
        let item = p
            .get("item")
            .and_then(Value::as_text)
            .and_then(|t| NodeId::parse(t).ok())
            .ok_or_else(|| Error::new("action.params"))?;
        let node = self.document.find(item).ok_or_else(|| Error::new("notes.no_item"))?;
        if node.get("kind").and_then(Value::as_text) != Some("midi") {
            return Err(Error::new("notes.not_midi"));
        }
        let track = self.item_track(item).ok_or_else(|| Error::new("notes.no_item"))?;
        let default_step = node.get("step").and_then(Value::as_int).unwrap_or(240);
        let same = self.midi.step.as_ref().filter(|s| s.item == item).cloned();
        let at = p.get("at").and_then(Value::as_int).or(same.as_ref().map(|s| s.at)).unwrap_or(0).max(0);
        let step =
            p.get("step").and_then(Value::as_int).or(same.as_ref().map(|s| s.step)).unwrap_or(default_step).max(1);
        let velocity = p.get("velocity").and_then(Value::as_real).map(|v| v.clamp(0.0, 1.0));
        self.midi.step = Some(Step {
            item,
            track,
            at,
            step,
            velocity: velocity.or(same.and_then(|s| s.velocity)),
            held: BTreeMap::new(),
            chord: Vec::new(),
            last: None,
        });
        self.step_publish();
        Ok(())
    }

    pub(crate) fn step_stop(&mut self) {
        self.midi.step = None;
        self.step_publish();
    }

    /// A key from a device while step mode is on: the chord goes in when the last key is released
    pub(crate) fn step_key(&mut self, port: PortIndex, bytes: &[u8]) {
        let Some(st) = self.midi.step.as_ref() else { return };
        let Some(raw) = lm_midimap::Raw::parse(bytes) else { return };
        if !self.track_listens(st.track, port, raw.channel()) {
            return;
        }
        let Some(st) = self.midi.step.as_mut() else { return };
        match raw {
            lm_midimap::Raw::NoteOn { key, velocity, .. } => {
                let v = f64::from(velocity) / 127.0;
                st.held.insert(key, v);
                if !st.chord.iter().any(|(k, _)| *k == key) {
                    st.chord.push((key, v));
                }
            }
            lm_midimap::Raw::NoteOff { key, .. } => {
                st.held.remove(&key);
                if st.held.is_empty() && !st.chord.is_empty() {
                    let chord = std::mem::take(&mut st.chord);
                    if let Err(e) = self.step_chord(&chord) {
                        self.notices.error(&e, None, None);
                    }
                }
            }
            _ => {}
        }
    }

    /// Whether a track's MIDI input takes this port and channel (`midiInput`; absent: every port, every channel)
    fn track_listens(&self, track: NodeId, port: PortIndex, channel: u8) -> bool {
        let Some(t) = self.document.find(track) else { return false };
        let input = t.get("midiInput");
        let ports: Vec<u16> = input
            .and_then(|i| i.get("ports"))
            .map(|l| l.items().filter_map(Value::as_text).filter_map(|a| self.midi_port_of(a)).map(|p| p.0).collect())
            .unwrap_or_default();
        let ch = input.and_then(|i| i.get("channel")).and_then(Value::as_int).unwrap_or(-1);
        (ports.is_empty() || ports.contains(&port.0)) && (ch < 0 || ch == i64::from(channel))
    }

    /// One chord at the cursor (one undo step); the cursor advances one step
    pub(crate) fn step_chord(&mut self, chord: &[(u8, f64)]) -> Result<()> {
        let st = self.midi.step.clone().ok_or_else(|| Error::new("notes.step_off"))?;
        if chord.is_empty() {
            return Ok(());
        }
        let mut added = Vec::new();
        self.document.transact(Label::new("notes.step"), |e| {
            let len = e.find(st.item).and_then(|i| i.get("length").and_then(Value::as_int)).unwrap_or(0);
            if st.at + st.step > len {
                e.set(st.item, &[PathStep::key("length")], Value::int(st.at + st.step))?;
            }
            for (key, v) in chord {
                let note = Value::object([
                    ("key", Value::int(i64::from(*key))),
                    ("start", Value::int(st.at)),
                    ("length", Value::int(st.step)),
                    ("velocity", Value::real(st.velocity.unwrap_or(*v))?),
                ]);
                added.push(e.add_note(st.item, note)?);
            }
            Ok(())
        })?;
        if let Some(s) = self.midi.step.as_mut() {
            s.last = Some((s.at, added));
            s.at += s.step;
        }
        self.step_publish();
        Ok(())
    }

    /// `notes.step_rest`: the cursor advances one step without a note
    pub(crate) fn step_rest(&mut self) -> Result<()> {
        let s = self.midi.step.as_mut().ok_or_else(|| Error::new("notes.step_off"))?;
        s.at += s.step;
        s.last = None;
        self.step_publish();
        Ok(())
    }

    /// `notes.step_back`: the last chord goes, and the cursor goes back to where it was (or one step back)
    pub(crate) fn step_back(&mut self) -> Result<()> {
        let s = self.midi.step.clone().ok_or_else(|| Error::new("notes.step_off"))?;
        match s.last {
            Some((at, notes)) => {
                self.document.transact(Label::new("notes.step_back"), |e| {
                    for n in &notes {
                        e.remove_note(s.item, *n)?;
                    }
                    Ok(())
                })?;
                if let Some(st) = self.midi.step.as_mut() {
                    st.at = at;
                    st.last = None;
                }
            }
            None => {
                if let Some(st) = self.midi.step.as_mut() {
                    st.at = (st.at - st.step).max(0);
                }
            }
        }
        self.step_publish();
        Ok(())
    }

    /// `notes.step_length {delta}`: the last chord gets longer (→) or shorter (←), and the cursor moves with it
    pub(crate) fn step_length(&mut self, delta: i64) -> Result<()> {
        let s = self.midi.step.clone().ok_or_else(|| Error::new("notes.step_off"))?;
        if let Some((at, notes)) = &s.last {
            let new_len = (s.at - at + delta).max(1);
            self.document.transact(Label::new("notes.step_length"), |e| {
                let len = e.find(s.item).and_then(|i| i.get("length").and_then(Value::as_int)).unwrap_or(0);
                if at + new_len > len {
                    e.set(s.item, &[PathStep::key("length")], Value::int(at + new_len))?;
                }
                for n in notes {
                    e.set_note(s.item, *n, &[PathStep::key("length")], Value::int(new_len))?;
                }
                Ok(())
            })?;
            if let Some(st) = self.midi.step.as_mut() {
                st.at = at + new_len;
            }
        } else if let Some(st) = self.midi.step.as_mut() {
            st.at = (st.at + delta).max(0);
        }
        self.step_publish();
        Ok(())
    }

    /// The item the next recording writes into (`recordInto` in the interface document), if it is still a MIDI item
    pub(crate) fn record_into(&self) -> Option<NodeId> {
        let id =
            self.ui_document.root().get("recordInto").and_then(Value::as_text).and_then(|t| NodeId::parse(t).ok())?;
        (self.document.find(id)?.get("kind").and_then(Value::as_text) == Some("midi")).then_some(id)
    }

    /// `notes.record_into {item}` / `{}`
    pub(crate) fn set_record_into(&mut self, p: &Value) -> Result<()> {
        let v = match p.get("item").and_then(Value::as_text) {
            Some(t) => {
                let id = NodeId::parse(t)?;
                if self
                    .document
                    .find(id)
                    .and_then(|i| i.get("kind").and_then(Value::as_text).map(str::to_owned))
                    .as_deref()
                    != Some("midi")
                {
                    return Err(Error::new("notes.not_midi"));
                }
                Value::text(t)
            }
            None => Value::NULL,
        };
        self.ui_document.transact(Label::new("ui.record_into"), |e| e.set(ROOT, &[PathStep::key("recordInto")], v))?;
        Ok(())
    }
}
