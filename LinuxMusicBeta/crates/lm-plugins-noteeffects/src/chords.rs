//! Chords (legacy `InstrumentFunctionNoteStacking`, F-IJN-013): every note becomes the notes of a chord or scale from
//! the table, repeated over `range` octaves. The notes it makes are its own (numbered from
//! [`lm_plugin::note_effect::GENERATED_IDS`]); the note-off of a note ends them all.

use lm_base::Result;
use lm_plugin::note_effect::{BlockCx, IdCounter, NoteEffect, NoteFx};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{EventKind, NoteId, NoteOut, Processor};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::table::{CHORD_KEYS, chord};

pub const ID: &str = "chords";
/// The notes it makes at once (more are not made)
pub const MAX_MADE: usize = 1024;

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.chords.name",
    kind: PluginKind::NoteEffect,
    schemas,
    ports: crate::no_ports,
    velocity: Velocity::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:chords", 1)
            .field(FieldSpec::choice("chord", &CHORD_KEYS, "major"))
            .param(ParamSpec::new("range", 1.0, 9.0, 1.0).scale(Scale::Stepped)),
    ]
}

fn build(state: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let v = StateView::new(state);
    let fx = Chords {
        intervals: v.choice("chord", chord)?,
        range: 1,
        made: Vec::with_capacity(MAX_MADE),
        ids: IdCounter::default(),
    };
    Ok(Box::new(NoteFx::new(fx, &v, &DESCRIPTOR.schema().params)?))
}

#[derive(Clone, Copy, Debug)]
struct Made {
    base: NoteId,
    id: NoteId,
    channel: u8,
    key: u8,
}

/// The chords
#[derive(Debug)]
pub struct Chords {
    intervals: &'static [u8],
    range: i32,
    made: Vec<Made>,
    ids: IdCounter,
}

impl NoteEffect for Chords {
    const PARAMS: usize = 1;

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.range = (value as i32).clamp(1, 9);
        }
    }

    fn event(&mut self, at: u32, kind: EventKind, _: &mut BlockCx<'_>, out: &mut NoteOut) {
        match kind {
            EventKind::NoteOn { note, channel, key, velocity } => {
                if self.made.iter().any(|m| m.base == note) {
                    return;
                }
                for octave in 0..self.range {
                    for &i in self.intervals {
                        let k = i32::from(key) + octave * 12 + i32::from(i);
                        if k > 127 {
                            break;
                        }
                        if self.made.len() == self.made.capacity() {
                            return;
                        }
                        let id = self.ids.next();
                        let key = k as u8;
                        self.made.push(Made { base: note, id, channel, key });
                        out.push(at, EventKind::NoteOn { note: id, channel, key, velocity });
                    }
                }
            }
            EventKind::NoteOff { note, velocity, .. } => {
                let mut i = 0;
                while i < self.made.len() {
                    if self.made[i].base == note {
                        let m = self.made.swap_remove(i);
                        out.push(at, EventKind::NoteOff { note: m.id, channel: m.channel, key: m.key, velocity });
                    } else {
                        i += 1;
                    }
                }
            }
            EventKind::NoteExpression { note, expression, value, ramp } => {
                for m in self.made.iter().filter(|m| m.base == note) {
                    out.push(at, EventKind::NoteExpression { note: m.id, expression, value, ramp });
                }
            }
            EventKind::AllNotesRelease | EventKind::AllSoundOff => {
                self.made.clear();
                out.push(at, kind);
            }
            kind => {
                out.push(at, kind);
            }
        }
    }

    fn reset(&mut self) {
        self.made.clear();
    }

    fn take_state(&mut self, old: &mut Self) {
        std::mem::swap(&mut self.made, &mut old.made);
        self.ids = old.ids;
    }
}
