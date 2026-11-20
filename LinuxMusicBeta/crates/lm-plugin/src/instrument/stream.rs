//! The instrument functions of a single-stream instrument (the legacy `IsSingleStreamed`: one synthesizer plays every
//! note, as FluidSynth and libgig do): chords, the arpeggio, mono, legato and glide, the sustain pedal, the pitch knob
//! and the pitch bend — the same notes as a per-note [`super::Instrument`] (both run `notes`), made to sound by the
//! plugin's [`Stream`].
//!
//! The legacy program applied neither the envelopes, the LFOs nor the filter of the track to a single-stream
//! instrument (`InstrumentTrack::processAudioBuffer` skips them), so they are not part of this: a stream instrument's
//! schema takes the note functions only ([`add_note_functions`]); a plugin that hands the volume envelope to its
//! synthesizer, as the Sf2 Player does, declares those parameters as its own.
//!
//! # Pitch
//!
//! A sounding note starts at its key; everything else that moves its pitch is a *detune* in semitones the stream
//! applies to the note's voices while they play: the pitch knob (cents), a glide, and in legato the keys it moved
//! from the key it was struck at. The pitch bend is the synthesizer's own (a channel message, with its range).
//!
//! # A change that is not a parameter
//!
//! A new file (another SoundFont, another `.gig`) is a new synthesizer, built apart. In `adopt` the old one becomes
//! a *ghost*: the notes that sound go on sounding on it until their note-off and release, mixed with the new one,
//! whose notes are the new ones; nothing is cut and nothing clicks (U32). The same state rebuilt takes the old
//! synthesizer over whole, so the sound goes on bit for bit.

use std::any::Any;
use std::fmt;

use lm_base::{Error, Result};
use lm_render::{Adopted, Event, EventKind, NoteId, ProcessContext, ProcessSpec, Processor, VoiceNote};
use lm_state::{FieldSpec, NodeSchema, ParamSpec};

use super::notes::{ArpDirection, ArpMode, NoteCx, NoteSettings, Run, Sounder, VoiceMode};
use super::schema::{ARP_DIRECTIONS, ARP_MODES, SYNC_KEYS, VOICE_MODES, instrument_params, sync_beats};
use super::{CHORD_KEYS, GRID, MAX_SOUNDING, chord, fingerprint};
use crate::state::StateView;

/// The note functions' parameters, in index order after the instrument's own (the same keys, ranges and defaults as
/// the instrument functions')
pub mod q {
    pub const PITCH: usize = 0;
    pub const BEND_RANGE: usize = 1;
    pub const CHORD_ON: usize = 2;
    pub const CHORD_RANGE: usize = 3;
    pub const ARP_ON: usize = 4;
    pub const ARP_RANGE: usize = 5;
    pub const ARP_REPEATS: usize = 6;
    pub const ARP_CYCLE: usize = 7;
    pub const ARP_SKIP: usize = 8;
    pub const ARP_MISS: usize = 9;
    pub const ARP_TIME: usize = 10;
    pub const ARP_GATE: usize = 11;
    pub const GLIDE: usize = 12;
    /// How many
    pub const COUNT: usize = 13;
    /// Their keys, in index order
    pub const KEYS: [&str; COUNT] = [
        "pitch",
        "bendRange",
        "chordOn",
        "chordRange",
        "arpOn",
        "arpRange",
        "arpRepeats",
        "arpCycle",
        "arpSkip",
        "arpMiss",
        "arpTime",
        "arpGate",
        "glide",
    ];
}

/// The note functions' parameters
pub fn note_params() -> Vec<ParamSpec> {
    q::KEYS.iter().filter_map(|k| instrument_params().iter().find(|p| p.key == *k).cloned()).collect()
}

/// The note functions' fields (choices: rebuilding the processor, which adopts the notes)
pub fn note_fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::choice("chord", &CHORD_KEYS, "octave"),
        FieldSpec::choice("arpChord", &CHORD_KEYS, "octave"),
        FieldSpec::choice("arpDirection", &ARP_DIRECTIONS, "up"),
        FieldSpec::choice("arpMode", &ARP_MODES, "free"),
        FieldSpec::choice("arpSync", &SYNC_KEYS, "off"),
        FieldSpec::choice("voiceMode", &VOICE_MODES, "poly"),
    ]
}

/// A single-stream instrument's schema with the note functions added after its own fields and parameters
pub fn add_note_functions(mut schema: NodeSchema) -> NodeSchema {
    for f in note_fields() {
        schema = schema.field(f);
    }
    for p in note_params() {
        schema = schema.param(p);
    }
    schema
}

/// A single-stream instrument's synthesizer. Every method but the build's runs on the audio thread: none may
/// allocate, lock or do I/O
pub trait Stream: Send + 'static {
    /// The plugin's own parameters: indices `0..PARAMS`; the note functions' follow
    const PARAMS: usize;

    /// A parameter changed
    fn set_param(&mut self, index: usize, value: f32);
    /// A sounding note starts: `key`, velocity 0 to 1 (linear, U28), and `detune` semitones from the start
    fn note_on(&mut self, note: NoteId, key: u8, velocity: f32, detune: f32);
    /// A sounding note's detune changes (semitones from the key it was struck at)
    fn detune(&mut self, note: NoteId, semitones: f32);
    /// A sounding note ends (its release)
    fn note_off(&mut self, note: NoteId);
    /// A sounding note's pan (a note expression, −1 left to 1 right; 0 at the start): a stream that cannot pan per
    /// note ignores it
    fn pan(&mut self, note: NoteId, pan: f32) {
        let _ = (note, pan);
    }
    /// The pitch bend (−1 to 1) and its range (semitones)
    fn bend(&mut self, bend: f32, range: f32);
    /// A channel message that is not a note, a pitch bend nor the sustain pedal (the synthesizer's controllers)
    fn midi(&mut self, bytes: [u8; 3]) {
        let _ = bytes;
    }
    /// Every note to its release
    fn release_all(&mut self);
    /// Silence now
    fn sound_off(&mut self);
    /// Writes the next `left.len()` frames (both slices have that length), replacing what they held
    fn render(&mut self, left: &mut [f32], right: &mut [f32]);
    /// Back to rest: nothing sounds and nothing of what played is kept
    fn reset(&mut self) {
        self.sound_off();
    }
}

/// The choices of the note functions, read in the build
#[derive(Clone, Copy, Debug)]
struct Choices {
    chord: &'static [u8],
    arp_chord: &'static [u8],
    arp_direction: ArpDirection,
    arp_mode: ArpMode,
    arp_sync: Option<f64>,
    mode: VoiceMode,
}

impl Choices {
    fn read(state: &StateView<'_>) -> Result<Self> {
        Ok(Self {
            chord: state.choice("chord", chord)?,
            arp_chord: state.choice("arpChord", chord)?,
            arp_direction: state.choice("arpDirection", ArpDirection::from_key)?,
            arp_mode: state.choice("arpMode", ArpMode::from_key)?,
            arp_sync: state.choice("arpSync", |s| SYNC_KEYS.contains(&s).then(|| sync_beats(s)))?,
            mode: state.choice("voiceMode", VoiceMode::from_key)?,
        })
    }
}

/// A sounding note, as the stream plays it
#[derive(Clone, Copy, Debug)]
struct Note {
    id: NoteId,
    /// The key the synthesizer started it at
    struck: u8,
    /// The key it sounds at (legato moves it)
    key: u8,
    glide_from: f32,
    glide_frames: i32,
    glide_left: i32,
    glide_offset: f32,
    /// The detune last sent
    sent: f32,
    /// Its base's note expressions now (kept once the base lets go): semitones, and the pan
    tuning: f32,
    pan: f32,
    /// It plays on the ghost (the synthesizer before a change)
    ghost: bool,
}

impl Note {
    fn pitch(&self) -> f32 {
        f32::from(self.key) + self.glide_offset
    }

    fn glide_from_pitch(&mut self, from_pitch: f32, frames: i32) {
        if frames <= 0 {
            self.glide_left = 0;
            self.glide_offset = 0.0;
            return;
        }
        self.glide_from = from_pitch - f32::from(self.key);
        self.glide_frames = frames;
        self.glide_left = frames;
        self.glide_offset = self.glide_from;
    }

    /// The detune now, with the pitch knob (semitones)
    fn detune(&self, pitch: f32) -> f32 {
        f32::from(self.key) - f32::from(self.struck) + self.glide_offset + pitch + self.tuning
    }
}

/// The sounding notes and the synthesizers, as the notes' [`Sounder`]
struct Voices<'a, S: Stream> {
    notes: &'a mut Vec<Note>,
    stream: &'a mut S,
    ghost: Option<&'a mut S>,
    /// The pitch knob, semitones
    pitch: f32,
}

impl<S: Stream> Voices<'_, S> {
    fn find(&mut self, id: NoteId) -> Option<usize> {
        self.notes.iter().position(|n| n.id == id)
    }
}

impl<S: Stream> Sounder for Voices<'_, S> {
    fn on(&mut self, note: VoiceNote) {
        if self.notes.len() == self.notes.capacity() {
            return;
        }
        let n = Note {
            id: note.note,
            struck: note.key,
            key: note.key,
            glide_from: 0.0,
            glide_frames: 0,
            glide_left: 0,
            glide_offset: 0.0,
            sent: self.pitch,
            tuning: 0.0,
            pan: 0.0,
            ghost: false,
        };
        self.notes.push(n);
        self.stream.note_on(note.note, note.key, note.velocity, self.pitch);
    }

    fn glide(&mut self, id: NoteId, from: f32, frames: i32) {
        let pitch = self.pitch;
        let Some(i) = self.find(id) else { return };
        let n = &mut self.notes[i];
        n.glide_from_pitch(from, frames);
        n.sent = n.detune(pitch);
        let (ghost, d) = (n.ghost, n.sent);
        match (ghost, self.ghost.as_deref_mut()) {
            (true, Some(g)) => g.detune(id, d),
            _ => self.stream.detune(id, d),
        }
    }

    fn off(&mut self, id: NoteId) {
        let Some(i) = self.find(id) else { return };
        let n = self.notes.swap_remove(i);
        match (n.ghost, self.ghost.as_deref_mut()) {
            (true, Some(g)) => g.note_off(id),
            (true, None) => {}
            (false, _) => self.stream.note_off(id),
        }
    }

    fn pitch(&mut self, id: NoteId) -> Option<f32> {
        let i = self.find(id)?;
        Some(self.notes[i].pitch())
    }

    fn retarget(&mut self, id: NoteId, key: u8, frames: i32) {
        let pitch = self.pitch;
        let Some(i) = self.find(id) else { return };
        let n = &mut self.notes[i];
        let from = n.pitch();
        n.key = key;
        n.glide_from_pitch(from, frames);
        n.sent = n.detune(pitch);
        let (ghost, d) = (n.ghost, n.sent);
        match (ghost, self.ghost.as_deref_mut()) {
            (true, Some(g)) => g.detune(id, d),
            _ => self.stream.detune(id, d),
        }
    }

    fn release_all(&mut self) {
        self.stream.release_all();
        if let Some(g) = self.ghost.as_deref_mut() {
            g.release_all();
        }
        self.notes.clear();
    }

    fn kill_all(&mut self) {
        self.stream.sound_off();
        if let Some(g) = self.ghost.as_deref_mut() {
            g.sound_off();
        }
        self.notes.clear();
    }
}

/// How long the ghost keeps rendering once it has no note and is silent, frames at 48 kHz scaled to the rate
const GHOST_QUIET_MS: f32 = 100.0;

/// A single-stream instrument: the plugin's [`Stream`] with the note functions around it
pub struct StreamInstrument<S: Stream> {
    stream: Option<S>,
    ghost: Option<S>,
    /// Frames the ghost has been silent with no note
    ghost_quiet: u32,
    ghost_quiet_max: u32,
    choices: Choices,
    /// Every parameter's value now (the stream's, then the note functions'), and its range
    values: Box<[f32]>,
    ranges: Box<[(f32, f32)]>,
    rate: f32,
    sample_rate: u32,
    bpm: f64,
    run: Run,
    notes: Vec<Note>,
    scratch: [[f32; GRID]; 2],
    fingerprint: u64,
}

impl<S: Stream> fmt::Debug for StreamInstrument<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamInstrument")
            .field("rate", &self.sample_rate)
            .field("notes", &self.notes.len())
            .field("ghost", &self.ghost.is_some())
            .finish_non_exhaustive()
    }
}

impl<S: Stream> StreamInstrument<S> {
    /// The instrument for a complete `state` whose parameters are `params` (all of the plugin's: the stream's
    /// `S::PARAMS` first, then the note functions'). Allocates everything (U31)
    pub fn new(stream: S, state: &StateView<'_>, params: &[ParamSpec], spec: &ProcessSpec) -> Result<Self> {
        if params.len() != S::PARAMS + q::COUNT {
            return Err(Error::new("plugin.instrument.params").arg("found", params.len() as i64));
        }
        let choices = Choices::read(state)?;
        let mut values = Vec::with_capacity(params.len());
        for p in params {
            values.push(state.param_f32(p.key)?);
        }
        let ranges: Box<[(f32, f32)]> = params.iter().map(|p| (p.min as f32, p.max as f32)).collect();
        let rate = spec.sample_rate as f32;
        let mut stream = stream;
        for (i, v) in values.iter().take(S::PARAMS).enumerate() {
            stream.set_param(i, *v);
        }
        let mut me = Self {
            stream: Some(stream),
            ghost: None,
            ghost_quiet: 0,
            ghost_quiet_max: (GHOST_QUIET_MS / 1000.0 * rate) as u32,
            choices,
            values: values.into_boxed_slice(),
            ranges,
            rate,
            sample_rate: spec.sample_rate,
            bpm: 0.0,
            run: Run::new(),
            notes: Vec::with_capacity(MAX_SOUNDING),
            scratch: [[0.0; GRID]; 2],
            fingerprint: fingerprint(state.value()),
        };
        me.send_bend();
        Ok(me)
    }

    /// The stream (the synthesizer the new notes play on)
    pub fn stream(&self) -> Option<&S> {
        self.stream.as_ref()
    }

    /// Sounding notes followed
    pub fn sounding(&self) -> usize {
        self.notes.len()
    }

    fn note(&self, k: usize) -> f32 {
        self.values[S::PARAMS + k]
    }

    fn settings(&self) -> NoteSettings {
        let c = &self.choices;
        NoteSettings {
            chord_on: self.note(q::CHORD_ON),
            chord_range: self.note(q::CHORD_RANGE),
            chord: c.chord,
            arp_on: self.note(q::ARP_ON),
            arp_chord: c.arp_chord,
            arp_range: self.note(q::ARP_RANGE),
            arp_repeats: self.note(q::ARP_REPEATS),
            arp_cycle: self.note(q::ARP_CYCLE),
            arp_skip: self.note(q::ARP_SKIP),
            arp_miss: self.note(q::ARP_MISS),
            arp_time: self.note(q::ARP_TIME),
            arp_gate: self.note(q::ARP_GATE),
            arp_direction: c.arp_direction,
            arp_mode: c.arp_mode,
            arp_sync: c.arp_sync,
            glide: self.note(q::GLIDE),
            mode: c.mode,
        }
    }

    fn note_cx(&self) -> NoteCx {
        NoteCx { ns: self.settings(), rate: self.rate, bpm: self.bpm }
    }

    /// The pitch knob, semitones
    fn pitch(&self) -> f32 {
        self.note(q::PITCH) / 100.0
    }

    fn send_bend(&mut self) {
        let (bend, range) = (self.run.bend, self.note(q::BEND_RANGE));
        if let Some(s) = self.stream.as_mut() {
            s.bend(bend, range);
        }
        if let Some(g) = self.ghost.as_mut() {
            g.bend(bend, range);
        }
    }

    /// Runs `f` with the notes' sounder
    fn with_voices<R>(&mut self, f: impl FnOnce(&mut Run, &NoteCx, &mut Voices<'_, S>) -> R) -> Option<R> {
        let cx = self.note_cx();
        let pitch = self.pitch();
        let stream = self.stream.as_mut()?;
        let mut v = Voices { notes: &mut self.notes, stream, ghost: self.ghost.as_mut(), pitch };
        Some(f(&mut self.run, &cx, &mut v))
    }

    fn set_param(&mut self, index: u32, value: f32) {
        let i = index as usize;
        let Some(&(lo, hi)) = self.ranges.get(i) else { return };
        if !value.is_finite() {
            return;
        }
        let v = value.clamp(lo, hi);
        self.values[i] = v;
        if i < S::PARAMS {
            if let Some(s) = self.stream.as_mut() {
                s.set_param(i, v);
            }
            return;
        }
        match i - S::PARAMS {
            q::BEND_RANGE => self.send_bend(),
            q::PITCH => self.send_detunes(true),
            _ => {}
        }
    }

    /// Sends each note's detune that changed (every one when `all`)
    fn send_detunes(&mut self, all: bool) {
        let pitch = self.pitch();
        for n in self.notes.iter_mut() {
            let d = n.detune(pitch);
            if all || d != n.sent {
                n.sent = d;
                let target = if n.ghost { self.ghost.as_mut() } else { self.stream.as_mut() };
                if let Some(s) = target {
                    s.detune(n.id, d);
                }
            }
        }
    }

    fn event(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn { note, channel, key, velocity } => {
                self.with_voices(|run, cx, v| {
                    run.note_on(cx, v, note, channel, key.min(127), velocity.clamp(0.0, 1.0))
                });
            }
            EventKind::NoteOff { note, .. } => {
                self.with_voices(|run, cx, v| run.note_off(cx, v, note));
            }
            EventKind::Param { index, value } => self.set_param(index, value),
            EventKind::Midi { bytes } => match bytes[0] & 0xF0 {
                0xE0 => {
                    self.run.set_bend(bytes);
                    self.send_bend();
                }
                0xB0 => match bytes[1] {
                    64 => {
                        self.with_voices(|run, cx, v| run.pedal(cx, v, bytes[2] >= 64));
                    }
                    120 => {
                        self.with_voices(|run, _, v| run.sound_off(v));
                    }
                    123 => {
                        self.with_voices(|run, _, v| run.release_all(v));
                    }
                    _ => {
                        if let Some(s) = self.stream.as_mut() {
                            s.midi(bytes);
                        }
                    }
                },
                _ => {
                    if let Some(s) = self.stream.as_mut() {
                        s.midi(bytes);
                    }
                }
            },
            EventKind::AllNotesRelease => {
                self.with_voices(|run, _, v| run.release_all(v));
            }
            EventKind::AllSoundOff => {
                self.with_voices(|run, _, v| run.sound_off(v));
            }
            EventKind::NoteExpression { note, expression, value, ramp } => {
                self.run.express(note, expression, value, ramp);
            }
        }
    }

    /// Plays `n` frames into `left`/`right` from `at` (`n` at most [`GRID`])
    fn piece(&mut self, at: usize, n: usize, left: &mut [f32], right: &mut [f32]) {
        // The note expressions of each note's base, while it has one (by piece, as the glide)
        for note in self.notes.iter_mut() {
            if let Some(base) = self.run.sounding.iter().find(|s| s.id == note.id).map(|s| s.base)
                && let Some(b) = self.run.bases.iter().find(|b| b.id == base)
            {
                note.tuning = b.tuning.value;
                let pan = b.pan.value;
                if pan != note.pan {
                    note.pan = pan;
                    let target = if note.ghost { self.ghost.as_mut() } else { self.stream.as_mut() };
                    if let Some(s) = target {
                        s.pan(note.id, pan);
                    }
                }
            }
        }
        for b in &mut self.run.bases {
            b.tuning.advance(n as u32);
            b.pan.advance(n as u32);
        }
        // Glides move on by piece, as a per-note instrument's
        for note in self.notes.iter_mut() {
            if note.glide_left > 0 {
                note.glide_offset = note.glide_from * note.glide_left as f32 / note.glide_frames as f32;
                note.glide_left = note.glide_left.saturating_sub(n as i32).max(0);
            } else {
                note.glide_offset = 0.0;
            }
        }
        self.send_detunes(false);
        let (l, r) = (&mut left[at..at + n], &mut right[at..at + n]);
        match self.stream.as_mut() {
            Some(s) => s.render(l, r),
            None => {
                l.fill(0.0);
                r.fill(0.0);
            }
        }
        if let Some(g) = self.ghost.as_mut()
            && self.ghost_quiet < self.ghost_quiet_max
        {
            let [gl, gr] = &mut self.scratch;
            g.render(&mut gl[..n], &mut gr[..n]);
            let mut peak = 0.0f32;
            for i in 0..n {
                l[i] += gl[i];
                r[i] += gr[i];
                peak = peak.max(gl[i].abs()).max(gr[i].abs());
            }
            let playing = self.notes.iter().any(|x| x.ghost);
            if playing || peak > 1.0e-7 {
                self.ghost_quiet = 0;
            } else {
                self.ghost_quiet += n as u32;
            }
        }
        self.run.frame += n as u64;
    }
}

impl<S: Stream> Processor for StreamInstrument<S> {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let events: &[Event] = cx.events;
        let transport = cx.transport;
        let rng = &mut *cx.rng;
        if transport.bpm > 0.0 {
            self.bpm = transport.bpm;
        }
        let outputs = cx.buffers.outputs();
        if outputs == 0 {
            for e in events {
                self.event(e.kind);
            }
            return;
        }
        let (_, mut outs, _) = cx.buffers.split();
        let (left, right): (&mut [f32], &mut [f32]) =
            if outputs >= 2 { outs.pair_mut(0, 1) } else { (outs.get_mut(0), &mut []) };
        let mono = outputs < 2;
        let mut e = 0;
        let mut at = 0;
        let grid_origin = transport.frame;
        while at < frames {
            while e < events.len() && (events[e].offset as usize) <= at {
                self.event(events[e].kind);
                e += 1;
            }
            let now = self.run.frame;
            self.with_voices(|run, ncx, v| run.timed(ncx, v, now, rng));
            let mut end = frames;
            if e < events.len() {
                end = end.min(events[e].offset as usize);
            }
            let grid = GRID - ((grid_origin + at as u64) % GRID as u64) as usize;
            end = end.min(at + grid);
            let next = self.run.next_timed(&self.note_cx(), now);
            if next != u64::MAX {
                end = end.min(at + (next - now).min(frames as u64) as usize);
            }
            let n = end.saturating_sub(at).max(1).min(frames - at);
            if mono {
                let mut tmp_l = [0.0f32; GRID];
                let mut tmp_r = [0.0f32; GRID];
                self.piece(0, n, &mut tmp_l, &mut tmp_r);
                for i in 0..n {
                    left[at + i] = 0.5 * (tmp_l[i] + tmp_r[i]);
                }
            } else {
                self.piece(at, n, left, right);
            }
            at += n;
        }
        for ev in &events[e..] {
            self.event(ev.kind);
        }
    }

    fn reset(&mut self) {
        self.with_voices(|run, _, v| run.sound_off(v));
        if let Some(s) = self.stream.as_mut() {
            s.reset();
        }
        if let Some(g) = self.ghost.as_mut() {
            g.reset();
        }
        self.ghost_quiet = self.ghost_quiet_max;
    }

    /// Takes over the notes and the parameter values of the processor it replaces, when that is the same instrument
    /// at the same rate (R18): the same state takes its synthesizer over too; another state keeps the old
    /// synthesizer as the ghost of the notes that sound
    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if old.sample_rate != self.sample_rate || old.values.len() != self.values.len() {
            return Adopted::No;
        }
        std::mem::swap(&mut self.run, &mut old.run);
        std::mem::swap(&mut self.notes, &mut old.notes);
        self.bpm = old.bpm;
        if old.fingerprint == self.fingerprint {
            std::mem::swap(&mut self.stream, &mut old.stream);
            std::mem::swap(&mut self.ghost, &mut old.ghost);
            self.ghost_quiet = old.ghost_quiet;
        } else {
            // The old synthesizer plays on what it was playing; an older ghost goes back with the old processor
            self.ghost = old.stream.take();
            self.ghost_quiet = 0;
            for n in self.notes.iter_mut() {
                n.ghost = true;
            }
        }
        // The values the old one heard last (automation and gestures since the tree was read)
        for i in 0..self.values.len() {
            let v = old.values[i];
            self.set_param(i as u32, v);
        }
        self.send_bend();
        if old.choices.mode != self.choices.mode {
            self.run.mode_changed(self.choices.mode);
        }
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
