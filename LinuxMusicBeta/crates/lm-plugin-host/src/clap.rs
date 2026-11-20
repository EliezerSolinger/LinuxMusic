//! CLAP plugins in the helper, through `lm-sys-clap`: the files (found without running plugin code), the listing of
//! one file (it runs the plugin's code: the scan does it in a helper of its own per file), and an instance behind
//! [`Plugin`].
//!
//! - A parameter's key is `p<id>`: CLAP's parameter ids are stable across versions of a plugin. Hidden parameters
//!   are left out.
//! - The plugin is identified by its CLAP id (`uri`); the file it was found in is tried first, and when it moved the
//!   search paths are looked through.
//! - Notes go as CLAP note events with their note id; a note expression's ramp is sent as a value every
//!   [`RAMP_STEP`] frames until it ends; pan goes from −1..1 to CLAP's 0..1; "all notes release" is a wildcard
//!   note-off, "all sound off" a wildcard choke.

use std::path::{Path, PathBuf};

use lm_base::{Error, Result};
use lm_render::{EventKind, Expression};
use lm_sys_clap::{Event, Instance, Library, MAX_EVENTS};

use crate::backend::{Listed, Param, Plugin, Transport};

/// The frames between two values of a note expression's ramp
pub const RAMP_STEP: u32 = 32;
/// The most ramps running at once
const MAX_RAMPS: usize = 256;
/// The most sounding notes remembered (for their expressions' start values)
const MAX_NOTES: usize = 512;

/// The `.clap` files under the given folders and the system's (`CLAP_PATH`, the user's, the system's)
pub fn files(paths: &[PathBuf], system: bool) -> Vec<PathBuf> {
    let mut dirs = paths.to_vec();
    if system {
        dirs.extend(lm_sys_clap::search_paths());
    }
    lm_sys_clap::find_files(&dirs)
}

fn params_of(inst: &Instance) -> (Vec<u32>, Vec<Param>) {
    inst.params()
        .iter()
        .filter(|p| !p.hidden)
        .map(|p| {
            let toggled = p.stepped && p.min == 0.0 && p.max == 1.0;
            (
                p.id,
                Param {
                    key: format!("p{}", p.id),
                    name: p.name.clone(),
                    min: p.min,
                    max: p.max,
                    default: p.default,
                    integer: p.stepped,
                    toggled,
                    logarithmic: false,
                },
            )
        })
        .unzip()
}

fn channels(ports: &[lm_sys_clap::AudioPort]) -> usize {
    ports.iter().map(|p| p.channels as usize).sum()
}

/// The plugins of one file (or of every file found): each one is made once to read its parameters and ports
pub fn list(paths: &[PathBuf], file: Option<&Path>, system: bool) -> Result<Vec<Listed>> {
    let all = match file {
        Some(f) => vec![f.to_path_buf()],
        None => files(paths, system),
    };
    let mut out = Vec::new();
    for f in all {
        let lib = match Library::open(&f) {
            Ok(l) => l,
            Err(e) if file.is_some() => return Err(e),
            Err(_) => continue,
        };
        for d in lib.descriptors() {
            let inst = match Instance::new(&lib, &d.id, 48_000.0, 256) {
                Ok(i) => i,
                Err(e) if file.is_some() => return Err(e),
                Err(_) => continue,
            };
            let (_, params) = params_of(&inst);
            let takes_notes = inst.takes_notes();
            out.push(Listed {
                format: "clap".into(),
                uri: d.id.clone(),
                name: d.name.clone(),
                instrument: d.is_instrument() || (takes_notes && inst.audio_inputs().is_empty()),
                inputs: channels(inst.audio_inputs()),
                outputs: channels(inst.audio_outputs()),
                midi_in: takes_notes,
                params,
                latency: inst.latency() > 0,
                has_window: inst.has_gui(),
                file: Some(f.display().to_string()),
            });
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
struct Note {
    id: u32,
    channel: i16,
    key: i16,
    pan: f64,
    tuning: f64,
}

#[derive(Clone, Copy, Debug)]
struct Ramp {
    note: u32,
    expression: Expression,
    from: f64,
    to: f64,
    /// Frames into the ramp at the start of the coming block
    done: u32,
    length: u32,
    /// Where it starts in the block it was given in
    offset: u32,
}

/// One CLAP plugin
pub struct Clap {
    inst: Instance,
    ids: Vec<u32>,
    params: Vec<Param>,
    events: Vec<Event>,
    notes: Vec<Note>,
    ramps: Vec<Ramp>,
}

fn clap_id(note: lm_render::NoteId) -> i32 {
    (note.0 & 0x7fff_ffff) as i32
}

fn clap_value(expression: Expression, v: f64) -> (i32, f64) {
    match expression {
        Expression::Pan => (lm_sys_clap::CLAP_NOTE_EXPRESSION_PAN, ((v + 1.0) / 2.0).clamp(0.0, 1.0)),
        Expression::Tuning => (lm_sys_clap::CLAP_NOTE_EXPRESSION_TUNING, v.clamp(-120.0, 120.0)),
    }
}

impl Clap {
    pub fn open(paths: &[PathBuf], file: Option<&Path>, uri: &str, rate: u32, max_block: usize) -> Result<Clap> {
        let has = |lib: &Library| lib.descriptors().iter().any(|d| d.id == uri);
        let lib = file
            .filter(|f| f.exists())
            .and_then(|f| Library::open(f).ok())
            .filter(|l| has(l))
            .or_else(|| files(paths, true).iter().filter_map(|f| Library::open(f).ok()).find(|l| has(l)))
            .ok_or_else(|| Error::new("plugin_host.not_found").arg("uri", uri))?;
        let inst = Instance::new(&lib, uri, f64::from(rate), u32::try_from(max_block).unwrap_or(u32::MAX))?;
        let (ids, params) = params_of(&inst);
        Ok(Clap {
            inst,
            ids,
            params,
            events: Vec::with_capacity(MAX_EVENTS),
            notes: Vec::with_capacity(MAX_NOTES),
            ramps: Vec::with_capacity(MAX_RAMPS),
        })
    }

    fn note(&self, id: u32) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    fn push(&mut self, e: Event) {
        if self.events.len() < self.events.capacity() {
            self.events.push(e);
        }
    }

    fn expression(&mut self, time: u32, note: u32, expression: Expression, value: f64) {
        let (channel, key) = self.note(note).map_or((-1, -1), |n| (n.channel, n.key));
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == note) {
            match expression {
                Expression::Pan => n.pan = value,
                Expression::Tuning => n.tuning = value,
            }
        }
        let (id, v) = clap_value(expression, value);
        self.push(Event::Expression {
            time,
            expression: id,
            note_id: clap_id(lm_render::NoteId(note)),
            channel,
            key,
            value: v,
        });
    }

    /// The ramps' values in this block, every `RAMP_STEP` frames; the ones that end are dropped
    fn run_ramps(&mut self, frames: u32) {
        let mut k = 0;
        while k < self.ramps.len() {
            let r = self.ramps[k];
            let mut t = r.offset;
            let mut done = r.done;
            while t < frames && done < r.length {
                done = (done + RAMP_STEP).min(r.length);
                let x = r.from + (r.to - r.from) * f64::from(done) / f64::from(r.length);
                self.expression(t, r.note, r.expression, x);
                t += RAMP_STEP;
            }
            if done >= r.length {
                self.ramps.swap_remove(k);
            } else {
                self.ramps[k].done = done;
                self.ramps[k].offset = t.saturating_sub(frames);
                k += 1;
            }
        }
    }
}

impl Plugin for Clap {
    fn params(&self) -> &[Param] {
        &self.params
    }

    fn latency(&self) -> u32 {
        self.inst.latency()
    }

    fn param(&self, index: u32) -> f32 {
        self.ids.get(index as usize).and_then(|id| self.inst.param_value(*id)).unwrap_or(0.0) as f32
    }

    fn set_param(&mut self, index: u32, value: f32) {
        if let Some(id) = self.ids.get(index as usize) {
            self.inst.set_param(*id, f64::from(value));
        }
    }

    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[(u32, EventKind)],
        transport: &Transport,
    ) {
        self.events.clear();
        for (time, e) in events {
            let time = *time;
            match *e {
                EventKind::NoteOn { note, channel, key, velocity } => {
                    if self.notes.len() == self.notes.capacity() {
                        self.notes.remove(0);
                    }
                    self.notes.push(Note {
                        id: note.0,
                        channel: i16::from(channel),
                        key: i16::from(key),
                        pan: 0.0,
                        tuning: 0.0,
                    });
                    self.push(Event::NoteOn {
                        time,
                        note_id: clap_id(note),
                        channel: i16::from(channel),
                        key: i16::from(key),
                        velocity: f64::from(velocity),
                    });
                }
                EventKind::NoteOff { note, channel, key, velocity } => {
                    self.notes.retain(|n| n.id != note.0);
                    self.ramps.retain(|r| r.note != note.0);
                    self.push(Event::NoteOff {
                        time,
                        note_id: clap_id(note),
                        channel: i16::from(channel),
                        key: i16::from(key),
                        velocity: f64::from(velocity),
                    });
                }
                EventKind::Param { index, value } => {
                    if let Some(id) = self.ids.get(index as usize) {
                        let id = *id;
                        self.push(Event::Param { time, id, value: f64::from(value) });
                    }
                }
                EventKind::Midi { bytes } => self.push(Event::Midi { time, data: bytes }),
                EventKind::NoteExpression { note, expression, value, ramp } => {
                    self.ramps.retain(|r| !(r.note == note.0 && r.expression == expression));
                    if ramp == 0 {
                        self.expression(time, note.0, expression, f64::from(value));
                    } else if self.ramps.len() < self.ramps.capacity() {
                        let from = self.note(note.0).map_or(0.0, |n| match expression {
                            Expression::Pan => n.pan,
                            Expression::Tuning => n.tuning,
                        });
                        self.ramps.push(Ramp {
                            note: note.0,
                            expression,
                            from,
                            to: f64::from(value),
                            done: 0,
                            length: ramp,
                            offset: time,
                        });
                    }
                }
                EventKind::AllNotesRelease => {
                    self.notes.clear();
                    self.ramps.clear();
                    self.push(Event::NoteOff { time, note_id: -1, channel: -1, key: -1, velocity: 0.0 });
                }
                EventKind::AllSoundOff => {
                    self.notes.clear();
                    self.ramps.clear();
                    self.push(Event::Choke { time, note_id: -1, channel: -1, key: -1 });
                }
            }
        }
        self.run_ramps(frames as u32);
        // In time order, stable (the ramps' values were added after the block's own events); no allocation
        let ev = &mut self.events;
        for i in 1..ev.len() {
            let mut j = i;
            while j > 0 && time_of(&ev[j - 1]) > time_of(&ev[j]) {
                ev.swap(j - 1, j);
                j -= 1;
            }
        }
        let t = lm_sys_clap::Transport {
            playing: transport.playing,
            recording: transport.recording,
            beats: transport.beats(),
            seconds: transport.seconds(),
            tempo: transport.bpm,
            signature: (transport.signature.0 > 0 && transport.signature.1 > 0)
                .then(|| (u16::from(transport.signature.0), u16::from(transport.signature.1))),
        };
        self.inst.process(frames, inputs, outputs, &self.events, Some(&t));
    }

    fn save(&mut self) -> Result<Option<Vec<u8>>> {
        self.inst.save_state()
    }

    fn restore(&mut self, chunk: &[u8]) -> Result<()> {
        self.inst.load_state(chunk)
    }

    fn idle(&mut self) {
        self.inst.idle();
    }

    fn state_changed(&mut self) -> bool {
        self.inst.take_state_dirty()
    }

    fn changes(&mut self, out: &mut Vec<(u32, f32)>) {
        for (id, v) in self.inst.changes() {
            if let Some(i) = self.ids.iter().position(|x| x == id)
                && out.len() < out.capacity()
            {
                out.push((i as u32, *v as f32));
            }
        }
    }
}

fn time_of(e: &Event) -> u32 {
    match *e {
        Event::NoteOn { time, .. }
        | Event::NoteOff { time, .. }
        | Event::Choke { time, .. }
        | Event::Expression { time, .. }
        | Event::Param { time, .. }
        | Event::Midi { time, .. } => time,
    }
}
