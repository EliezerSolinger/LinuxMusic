//! Scale and transposition: the notes snap to the nearest degree of a scale (the lower one on a tie), then move by
//! `transpose` semitones. A note that would leave 0 to 127 is dropped, with its note-off. The note-off carries the key
//! its note-on was sent on, whatever the settings became meanwhile.

use std::sync::LazyLock;

use lm_base::Result;
use lm_plugin::note_effect::{BlockCx, NoteEffect, NoteFx};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity};
use lm_render::{EventKind, NoteId, NoteOut, Processor};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::table::{CHORD_INTERVALS, CHORD_KEYS, chord_index, is_scale};

pub const ID: &str = "scale";
/// The notes followed at once (more pass unchanged)
pub const MAX_NOTES: usize = 512;

/// `off` and the scales of the table
pub static SCALE_KEYS: LazyLock<Box<[&'static str]>> =
    LazyLock::new(|| std::iter::once("off").chain(CHORD_KEYS.iter().copied().filter(|k| is_scale(k))).collect());

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.scale.name",
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
        NodeSchema::new("plugin:scale", 1)
            .field(FieldSpec::choice("scale", &SCALE_KEYS, "off"))
            .param(ParamSpec::new("root", 0.0, 11.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("transpose", -48.0, 48.0, 0.0).scale(Scale::Stepped).unit("st")),
    ]
}

fn build(state: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let v = StateView::new(state);
    let key = v.text("scale")?;
    let mut degrees = [false; 12];
    let on = key != "off";
    if on && let Some(i) = chord_index(key) {
        for &d in CHORD_INTERVALS[i] {
            degrees[usize::from(d % 12)] = true;
        }
    }
    let fx = ScaleFx { degrees, on, root: 0, transpose: 0, sent: Vec::with_capacity(MAX_NOTES) };
    Ok(Box::new(NoteFx::new(fx, &v, &DESCRIPTOR.schema().params)?))
}

/// Scale and transposition
#[derive(Debug)]
pub struct ScaleFx {
    degrees: [bool; 12],
    on: bool,
    root: i32,
    transpose: i32,
    /// The key each note was sent on (`None`: dropped)
    sent: Vec<(NoteId, Option<u8>)>,
}

impl ScaleFx {
    /// The key a played key becomes now
    pub fn map(&self, key: u8) -> Option<u8> {
        let mut k = i32::from(key);
        if self.on {
            let pc = (k - self.root).rem_euclid(12);
            let has = |d: i32| self.degrees[(d.rem_euclid(12)) as usize];
            for d in 0..=6 {
                if has(pc - d) {
                    k -= d;
                    break;
                }
                if has(pc + d) {
                    k += d;
                    break;
                }
            }
        }
        k += self.transpose;
        (0..=127).contains(&k).then_some(k as u8)
    }
}

impl NoteEffect for ScaleFx {
    const PARAMS: usize = 2;

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.root = (value as i32).clamp(0, 11),
            _ => self.transpose = (value as i32).clamp(-48, 48),
        }
    }

    fn event(&mut self, at: u32, kind: EventKind, _: &mut BlockCx<'_>, out: &mut NoteOut) {
        match kind {
            EventKind::NoteOn { note, channel, key, velocity } => {
                let mapped = self.map(key);
                if self.sent.len() < self.sent.capacity() {
                    self.sent.push((note, mapped));
                } else {
                    // Not recorded, its note-off could not find the mapped key: the note is dropped instead
                    return;
                }
                if let Some(key) = mapped {
                    out.push(at, EventKind::NoteOn { note, channel, key, velocity });
                }
            }
            EventKind::NoteOff { note, channel, key, velocity } => {
                match self.sent.iter().position(|(n, _)| *n == note) {
                    Some(i) => {
                        if let (_, Some(key)) = self.sent.swap_remove(i) {
                            out.push(at, EventKind::NoteOff { note, channel, key, velocity });
                        }
                    }
                    // (a note-on this effect never saw: its key as it comes)
                    None => {
                        out.push(at, EventKind::NoteOff { note, channel, key, velocity });
                    }
                }
            }
            EventKind::AllNotesRelease | EventKind::AllSoundOff => {
                self.sent.clear();
                out.push(at, kind);
            }
            kind => {
                out.push(at, kind);
            }
        }
    }

    fn reset(&mut self) {
        self.sent.clear();
    }

    fn take_state(&mut self, old: &mut Self) {
        std::mem::swap(&mut self.sent, &mut old.sent);
    }
}
