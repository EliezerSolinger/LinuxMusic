//! The notes of the instrument functions, apart from how they sound: base notes from the host become sounding notes
//! (mono, legato and glide, F-IJN-014; the sustain pedal and the pitch bend). A [`Sounder`] makes the sounding notes
//! sound: the voice pool of a per-note [`super::Instrument`], or the one synthesizer of a single-stream instrument
//! ([`super::stream`]: FluidSynth, libgig). Both run the same code here, so mono, legato and glide are the same on
//! every instrument.
//!
//! Chords and the arpeggio are not here: they are note effects (`lm-plugins-noteeffects`) that run before the
//! instrument, so a base note is exactly one sounding note.

use lm_render::{Expression, NoteId, VoiceNote};

use super::{MAX_BASES, MAX_SOUNDING, SOUNDING_IDS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VoiceMode {
    Poly,
    Mono,
    Legato,
}

impl VoiceMode {
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "poly" => VoiceMode::Poly,
            "mono" => VoiceMode::Mono,
            "legato" => VoiceMode::Legato,
            _ => return None,
        })
    }
}

/// The settings the notes read: the parameters' current values (raw, in their units) and the choices of the build
#[derive(Clone, Copy, Debug)]
pub(crate) struct NoteSettings {
    /// ms
    pub glide: f32,
    pub mode: VoiceMode,
}

/// What the notes are played with now
#[derive(Clone, Copy, Debug)]
pub(crate) struct NoteCx {
    pub ns: NoteSettings,
    pub rate: f32,
}

impl NoteCx {
    pub(crate) fn glide_frames(&self) -> i32 {
        (self.ns.glide / 1000.0 * self.rate) as i32
    }
}

/// What makes the sounding notes sound
pub(crate) trait Sounder {
    /// A sounding note starts
    fn on(&mut self, note: VoiceNote);
    /// The note `id` just started glides from `from` (keys) to its key over `frames`
    fn glide(&mut self, id: NoteId, from: f32, frames: i32);
    /// A sounding note ends (its release)
    fn off(&mut self, id: NoteId);
    /// The pitch a sounding note sounds at now, keys
    fn pitch(&mut self, id: NoteId) -> Option<f32>;
    /// Legato: a sounding note moves to `key`, gliding from where it is over `frames`
    fn retarget(&mut self, id: NoteId, key: u8, frames: i32);
    /// Every note to its release
    fn release_all(&mut self);
    /// Every note silent at once
    fn kill_all(&mut self);
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Base {
    pub id: NoteId,
    pub channel: u8,
    pub key: u8,
    pub velocity: f32,
    /// The instrument frame it started at
    pub start: u64,
    /// Let go while the sustain pedal was down: released when the pedal goes up
    pub sustained: bool,
    /// Its note expressions
    pub tuning: ExpressionRamp,
    pub pan: ExpressionRamp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ExpressionRamp {
    pub value: f32,
    pub target: f32,
    /// Frames left to the target
    pub left: u32,
}

impl ExpressionRamp {
    /// To `target` over `frames` from where it is now (at once with 0)
    pub(crate) fn go(&mut self, target: f32, frames: u32) {
        self.target = target;
        self.left = frames;
        if frames == 0 {
            self.value = target;
        }
    }

    /// Its value `f` frames from now
    pub(crate) fn at(&self, f: u32) -> f32 {
        if f >= self.left { self.target } else { self.value + (self.target - self.value) * f as f32 / self.left as f32 }
    }

    pub(crate) fn advance(&mut self, frames: u32) {
        self.value = self.at(frames);
        self.left = self.left.saturating_sub(frames);
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sounding {
    pub id: NoteId,
    pub base: NoteId,
    pub key: u8,
}

/// What the instrument keeps of the notes across blocks; taken over whole in `adopt`
#[derive(Debug)]
pub(crate) struct Run {
    pub frame: u64,
    pub bases: Vec<Base>,
    pub sounding: Vec<Sounding>,
    /// Mono and legato: the bases held, in order; the last one sounds
    pub held: Vec<NoteId>,
    pub current: Option<NoteId>,
    pub next_id: u32,
    /// Pitch bend, −1 to 1
    pub bend: f32,
    pub sustain: bool,
    /// Note-offs that came before their note-on in a block (a note of no length): the note-on releases at once
    pub early_offs: Vec<NoteId>,
    /// Sounding notes refused for lack of room
    pub dropped: u64,
}

impl Run {
    pub(crate) fn new() -> Self {
        Self {
            frame: 0,
            bases: Vec::with_capacity(MAX_BASES),
            sounding: Vec::with_capacity(MAX_SOUNDING),
            held: Vec::with_capacity(MAX_BASES),
            current: None,
            next_id: SOUNDING_IDS,
            bend: 0.0,
            sustain: false,
            early_offs: Vec::with_capacity(32),
            dropped: 0,
        }
    }

    pub(crate) fn clear_notes(&mut self) {
        self.bases.clear();
        self.sounding.clear();
        self.held.clear();
        self.current = None;
        self.sustain = false;
        self.early_offs.clear();
    }

    pub(crate) fn base(&self, id: NoteId) -> Option<usize> {
        self.bases.iter().position(|b| b.id == id)
    }

    fn new_id(&mut self) -> NoteId {
        let id = NoteId(self.next_id);
        self.next_id = self.next_id.checked_add(1).unwrap_or(SOUNDING_IDS);
        id
    }

    /// Starts the sounding note of a base, at `key`
    #[allow(clippy::too_many_arguments)]
    fn sound(
        &mut self,
        cx: &NoteCx,
        out: &mut impl Sounder,
        base: NoteId,
        channel: u8,
        key: i32,
        velocity: f32,
        glide_from: Option<f32>,
    ) {
        if !(0..=127).contains(&key) {
            return;
        }
        if self.sounding.len() == self.sounding.capacity() {
            self.dropped += 1;
            return;
        }
        let glide = cx.glide_frames();
        let id = self.new_id();
        let k = key as u8;
        self.sounding.push(Sounding { id, base, key: k });
        out.on(VoiceNote { note: id, channel, key: k, velocity });
        if let Some(from) = glide_from
            && glide > 0
        {
            out.glide(id, from, glide);
        }
    }

    /// Releases every sounding note of a base
    fn release_sounding(&mut self, out: &mut impl Sounder, base: NoteId) {
        let mut i = 0;
        while i < self.sounding.len() {
            if self.sounding[i].base == base {
                let s = self.sounding.swap_remove(i);
                out.off(s.id);
            } else {
                i += 1;
            }
        }
    }

    /// The pitch the base's root sounds at now (keys), if it sounds
    fn root_pitch(&mut self, out: &mut impl Sounder, base: NoteId, key: u8) -> Option<f32> {
        let root = self.sounding.iter().find(|s| s.base == base && s.key == key).map(|s| s.id)?;
        out.pitch(root)
    }

    /// Legato: the sounding notes of `from` move by `delta` keys and now belong to `to`, gliding
    fn retarget(&mut self, cx: &NoteCx, out: &mut impl Sounder, from: NoteId, to: NoteId, delta: i32) {
        let glide = cx.glide_frames();
        for s in self.sounding.iter_mut().filter(|s| s.base == from) {
            let k = (i32::from(s.key) + delta).clamp(0, 127) as u8;
            s.base = to;
            s.key = k;
            out.retarget(s.id, k, glide);
        }
    }

    pub(crate) fn note_on(
        &mut self,
        cx: &NoteCx,
        out: &mut impl Sounder,
        id: NoteId,
        channel: u8,
        key: u8,
        velocity: f32,
    ) {
        if self.base(id).is_some() {
            return;
        }
        if self.bases.len() == self.bases.capacity() {
            self.dropped += 1;
            return;
        }
        let base = Base {
            id,
            channel,
            key,
            velocity,
            start: self.frame,
            sustained: false,
            tuning: ExpressionRamp::default(),
            pan: ExpressionRamp::default(),
        };
        self.bases.push(base);
        match cx.ns.mode {
            VoiceMode::Poly => self.sound(cx, out, id, channel, i32::from(key), velocity, None),
            VoiceMode::Mono | VoiceMode::Legato => {
                if self.held.len() == self.held.capacity() {
                    self.held.remove(0);
                }
                self.held.push(id);
                self.take_over(cx, out, id);
            }
        }
        if let Some(pos) = self.early_offs.iter().position(|n| *n == id) {
            self.early_offs.swap_remove(pos);
            self.note_off(cx, out, id);
        }
    }

    /// Mono and legato: the base `to` becomes the one that sounds
    fn take_over(&mut self, cx: &NoteCx, out: &mut impl Sounder, to: NoteId) {
        let Some(b) = self.base(to).map(|i| self.bases[i]) else { return };
        match self.current.and_then(|c| self.base(c).map(|i| self.bases[i])) {
            Some(cur) if cur.id != to => {
                if cx.ns.mode == VoiceMode::Legato {
                    self.retarget(cx, out, cur.id, to, i32::from(b.key) - i32::from(cur.key));
                } else {
                    let from = self.root_pitch(out, cur.id, cur.key);
                    self.release_sounding(out, cur.id);
                    self.sound(cx, out, to, b.channel, i32::from(b.key), b.velocity, from);
                }
            }
            Some(_) => {}
            None => self.sound(cx, out, to, b.channel, i32::from(b.key), b.velocity, None),
        }
        self.current = Some(to);
    }

    pub(crate) fn note_off(&mut self, cx: &NoteCx, out: &mut impl Sounder, id: NoteId) {
        let Some(i) = self.base(id) else {
            if self.early_offs.len() < self.early_offs.capacity() {
                self.early_offs.push(id);
            }
            return;
        };
        if self.sustain {
            self.bases[i].sustained = true;
            return;
        }
        self.end_base(cx, out, i);
    }

    fn end_base(&mut self, cx: &NoteCx, out: &mut impl Sounder, i: usize) {
        let base = self.bases.swap_remove(i);
        match cx.ns.mode {
            VoiceMode::Poly => self.release_sounding(out, base.id),
            VoiceMode::Mono | VoiceMode::Legato => {
                if let Some(pos) = self.held.iter().position(|n| *n == base.id) {
                    self.held.remove(pos);
                }
                if self.current != Some(base.id) {
                    self.release_sounding(out, base.id);
                    return;
                }
                match self.held.last().copied() {
                    Some(prev) => {
                        // Back to the one held before: its key, as a new note (mono) or moving the voice (legato)
                        if cx.ns.mode == VoiceMode::Legato {
                            if let Some(p) = self.base(prev).map(|j| self.bases[j]) {
                                self.retarget(cx, out, base.id, prev, i32::from(p.key) - i32::from(base.key));
                                self.current = Some(prev);
                            }
                        } else {
                            let from = self.root_pitch(out, base.id, base.key);
                            self.release_sounding(out, base.id);
                            self.current = None;
                            if let Some(p) = self.base(prev).map(|j| self.bases[j]) {
                                self.sound(cx, out, prev, p.channel, i32::from(p.key), p.velocity, from);
                                self.current = Some(prev);
                            }
                        }
                    }
                    None => {
                        self.release_sounding(out, base.id);
                        self.current = None;
                    }
                }
            }
        }
    }

    pub(crate) fn release_all(&mut self, out: &mut impl Sounder) {
        out.release_all();
        self.clear_notes();
    }

    pub(crate) fn sound_off(&mut self, out: &mut impl Sounder) {
        out.kill_all();
        self.clear_notes();
    }

    pub(crate) fn pedal(&mut self, cx: &NoteCx, out: &mut impl Sounder, down: bool) {
        self.sustain = down;
        if down {
            return;
        }
        while let Some(i) = self.bases.iter().position(|b| b.sustained) {
            self.end_base(cx, out, i);
        }
    }

    /// A note expression of a base note (`NoteExpression`): it ramps from where it is
    pub(crate) fn express(&mut self, note: NoteId, expression: Expression, value: f32, ramp: u32) {
        if let Some(i) = self.base(note) {
            let b = &mut self.bases[i];
            match expression {
                Expression::Tuning => b.tuning.go(value, ramp),
                Expression::Pan => b.pan.go(value.clamp(-1.0, 1.0), ramp),
                // The built-in instruments do not use the pressure and the timbre of a note
                Expression::Pressure | Expression::Timbre => {}
            }
        }
    }

    /// The pitch bend of a channel message (`0xE0`), −1 to 1
    pub(crate) fn set_bend(&mut self, bytes: [u8; 3]) {
        let v = (i32::from(bytes[2] & 0x7F) << 7) | i32::from(bytes[1] & 0x7F);
        self.bend = ((v - 8192) as f32 / 8192.0).clamp(-1.0, 1.0);
    }

    /// A mode change while notes play (poly, mono, legato): the notes go on sounding; in mono and legato the last base
    /// held is the one that sounds from now on (the others end at their note-off)
    pub(crate) fn mode_changed(&mut self, mode: VoiceMode) {
        self.held.clear();
        self.current = None;
        if mode != VoiceMode::Poly {
            // (in place, without allocating: the held stack goes oldest first)
            self.bases.sort_unstable_by_key(|b| (b.start, b.id));
            for b in &self.bases {
                self.held.push(b.id);
            }
            self.current = self.bases.iter().max_by_key(|b| b.start).map(|b| b.id);
        }
    }
}
