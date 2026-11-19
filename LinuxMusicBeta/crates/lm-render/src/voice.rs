//! Voices for instruments (I-9.2, U27, U32; arquitetura-nova.md piece 7): allocated in the build, never grown.
//!
//! - **Policies:** `Poly`, `Mono` (each note retriggers), `Legato` (a note played while another is held glides
//!   the sounding voice, without restarting it). Mono and legato keep the held keys in a stack of 128: the last note
//!   wins, and releasing it goes back to the previous held one.
//! - **Stealing** when every voice sounds: the oldest voice in release, then the quietest by envelope, then the
//!   oldest. The stolen voice is not cut: it fades out over a short preallocated fade in a spare slot while the new
//!   note starts.
//!
//! A synth sends its note events to the pool, renders [`VoicePool::voices_mut`] multiplying each voice by its
//! [`VoiceGain`], then calls [`VoicePool::advance`]

use crate::ids::NoteId;

/// A note given to a voice
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceNote {
    pub note: NoteId,
    pub channel: u8,
    pub key: u8,
    pub velocity: f32,
}

/// What the pool needs from a synth's voice
pub trait Voice {
    /// Starts a note from the beginning of its envelope
    fn start(&mut self, note: VoiceNote);
    /// Legato: takes the new key and velocity without restarting the envelope
    fn glide(&mut self, note: VoiceNote);
    /// Enters the release
    fn release(&mut self);
    /// The envelope's current level (for stealing the quietest)
    fn level(&self) -> f32;
    /// The voice has nothing more to play (its release ended, or a one-shot finished)
    fn is_finished(&self) -> bool;
    /// Stops at once, back to the initial state
    fn kill(&mut self);
}

/// How notes take voices (U27)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Poly,
    Mono,
    Legato,
}

/// The gain ramp of one voice for one block: `start + step · i`, never below 0
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceGain {
    pub start: f32,
    pub step: f32,
}

impl VoiceGain {
    pub const UNITY: VoiceGain = VoiceGain { start: 1.0, step: 0.0 };

    pub fn at(&self, i: usize) -> f32 {
        (self.start + self.step * i as f32).max(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    Playing,
    Released,
    /// Stolen: fading out, `left` samples to go
    Fading { left: u32 },
}

#[derive(Debug)]
struct Slot<V> {
    voice: V,
    state: State,
    note: Option<NoteId>,
    age: u64,
}

const HELD: usize = 128;

/// A fixed set of voices with a policy (U27)
#[derive(Debug)]
pub struct VoicePool<V> {
    slots: Box<[Slot<V>]>,
    policy: Policy,
    polyphony: usize,
    fade: u32,
    held: [Option<VoiceNote>; HELD],
    held_len: usize,
    clock: u64,
}

impl<V: Voice> VoicePool<V> {
    /// `polyphony` sounding voices (1 for mono and legato) plus spare slots for the fades of stolen voices;
    /// `fade` is the stealing fade in samples (a few milliseconds). Allocates: call it in the build
    pub fn new(policy: Policy, polyphony: usize, fade: u32, mut make: impl FnMut() -> V) -> Self {
        let polyphony = if policy == Policy::Poly { polyphony.max(1) } else { 1 };
        let spare = (polyphony / 4).max(2);
        let slots = (0..polyphony + spare)
            .map(|_| Slot { voice: make(), state: State::Free, note: None, age: 0 })
            .collect();
        Self { slots, policy, polyphony, fade: fade.max(1), held: [None; HELD], held_len: 0, clock: 0 }
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Voices sounding, fading ones included
    pub fn active(&self) -> usize {
        self.slots.iter().filter(|s| s.state != State::Free).count()
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn free_slot(&mut self) -> usize {
        if let Some(i) = self.slots.iter().position(|s| s.state == State::Free) {
            return i;
        }
        // Every spare slot is still fading: cut the one closest to silence
        let i = (0..self.slots.len())
            .min_by_key(|&i| match self.slots[i].state {
                State::Fading { left } => (0, left, self.slots[i].age),
                _ => (1, 0, self.slots[i].age),
            })
            .unwrap_or(0);
        self.slots[i].voice.kill();
        self.slots[i].state = State::Free;
        self.slots[i].note = None;
        i
    }

    /// The victim when every voice sounds: oldest in release, then quietest, then oldest
    fn victim(&self) -> Option<usize> {
        let sounding = |s: &Slot<V>| matches!(s.state, State::Playing | State::Released);
        let released = (0..self.slots.len())
            .filter(|&i| self.slots[i].state == State::Released)
            .min_by_key(|&i| self.slots[i].age);
        if released.is_some() {
            return released;
        }
        (0..self.slots.len()).filter(|&i| sounding(&self.slots[i])).min_by(|&a, &b| {
            let (la, lb) = (self.slots[a].voice.level(), self.slots[b].voice.level());
            la.total_cmp(&lb).then(self.slots[a].age.cmp(&self.slots[b].age))
        })
    }

    fn steal(&mut self, i: usize) {
        self.slots[i].state = State::Fading { left: self.fade };
        self.slots[i].note = None;
    }

    fn start_new(&mut self, note: VoiceNote) {
        let sounding =
            self.slots.iter().filter(|s| matches!(s.state, State::Playing | State::Released)).count();
        if sounding >= self.polyphony
            && let Some(v) = self.victim()
        {
            self.steal(v);
        }
        let i = self.free_slot();
        let age = self.tick();
        let s = &mut self.slots[i];
        s.voice.start(note);
        s.state = State::Playing;
        s.note = Some(note.note);
        s.age = age;
    }

    fn playing_mono(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.state == State::Playing)
    }

    fn held_push(&mut self, note: VoiceNote) {
        if self.held_len == HELD {
            self.held.copy_within(1.., 0);
            self.held_len -= 1;
        }
        self.held[self.held_len] = Some(note);
        self.held_len += 1;
    }

    /// Removes a note from the held stack; whether it was the top
    fn held_remove(&mut self, note: NoteId) -> Option<bool> {
        let pos = self.held[..self.held_len].iter().position(|h| h.is_some_and(|h| h.note == note))?;
        let top = pos + 1 == self.held_len;
        self.held.copy_within(pos + 1..self.held_len, pos);
        self.held_len -= 1;
        self.held[self.held_len] = None;
        Some(top)
    }

    fn held_top(&self) -> Option<VoiceNote> {
        self.held_len.checked_sub(1).and_then(|i| self.held[i])
    }

    /// A note-on
    pub fn note_on(&mut self, note: VoiceNote) {
        match self.policy {
            Policy::Poly => self.start_new(note),
            Policy::Mono => {
                self.held_push(note);
                if let Some(i) = self.playing_mono() {
                    self.steal(i);
                }
                self.start_new(note);
            }
            Policy::Legato => {
                self.held_push(note);
                match self.playing_mono() {
                    Some(i) => {
                        self.slots[i].voice.glide(note);
                        self.slots[i].note = Some(note.note);
                    }
                    None => self.start_new(note),
                }
            }
        }
    }

    /// A note-off (by the note's number, U25)
    pub fn note_off(&mut self, note: NoteId) {
        match self.policy {
            Policy::Poly => {
                for s in self.slots.iter_mut() {
                    if s.state == State::Playing && s.note == Some(note) {
                        s.voice.release();
                        s.state = State::Released;
                    }
                }
            }
            Policy::Mono | Policy::Legato => {
                let Some(was_top) = self.held_remove(note) else { return };
                if !was_top {
                    return;
                }
                let Some(i) = self.playing_mono() else { return };
                match (self.held_top(), self.policy) {
                    (Some(prev), Policy::Mono) => {
                        self.steal(i);
                        self.start_new(prev);
                    }
                    (Some(prev), _) => {
                        self.slots[i].voice.glide(prev);
                        self.slots[i].note = Some(prev.note);
                    }
                    (None, _) => {
                        self.slots[i].voice.release();
                        self.slots[i].state = State::Released;
                    }
                }
            }
        }
    }

    /// Every note to its release (stop, locate, U32)
    pub fn release_all(&mut self) {
        self.held = [None; HELD];
        self.held_len = 0;
        for s in self.slots.iter_mut() {
            if s.state == State::Playing {
                s.voice.release();
                s.state = State::Released;
            }
        }
    }

    /// Silence now (panic, reset)
    pub fn kill_all(&mut self) {
        self.held = [None; HELD];
        self.held_len = 0;
        for s in self.slots.iter_mut() {
            s.voice.kill();
            s.state = State::Free;
            s.note = None;
        }
    }

    /// The sounding voices with their gain for this block, oldest first is not guaranteed (slot order: stable)
    pub fn voices_mut(&mut self) -> impl Iterator<Item = (&mut V, VoiceGain)> {
        let fade = self.fade as f32;
        self.slots.iter_mut().filter(|s| s.state != State::Free).map(move |s| {
            let gain = match s.state {
                State::Fading { left } => VoiceGain { start: left as f32 / fade, step: -1.0 / fade },
                _ => VoiceGain::UNITY,
            };
            (&mut s.voice, gain)
        })
    }

    /// After rendering `frames` samples: fades advance, finished voices free their slots
    pub fn advance(&mut self, frames: usize) {
        let frames = u32::try_from(frames).unwrap_or(u32::MAX);
        for s in self.slots.iter_mut() {
            match s.state {
                State::Fading { left } => {
                    if left <= frames {
                        s.voice.kill();
                        s.state = State::Free;
                    } else {
                        s.state = State::Fading { left: left - frames };
                    }
                }
                State::Playing | State::Released if s.voice.is_finished() => {
                    s.state = State::Free;
                    s.note = None;
                }
                _ => {}
            }
        }
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct V {
        key: u8,
        level: f32,
        released: bool,
        starts: u32,
        glides: u32,
    }

    impl Voice for V {
        fn start(&mut self, n: VoiceNote) {
            self.key = n.key;
            self.level = n.velocity;
            self.released = false;
            self.starts += 1;
        }
        fn glide(&mut self, n: VoiceNote) {
            self.key = n.key;
            self.glides += 1;
        }
        fn release(&mut self) {
            self.released = true;
        }
        fn level(&self) -> f32 {
            self.level
        }
        fn is_finished(&self) -> bool {
            self.released && self.level == 0.0
        }
        fn kill(&mut self) {
            *self = V::default();
        }
    }

    fn n(id: u32, key: u8, vel: f32) -> VoiceNote {
        VoiceNote { note: NoteId(id), channel: 0, key, velocity: vel }
    }

    fn keys(p: &mut VoicePool<V>) -> Vec<(u8, bool)> {
        let mut k: Vec<(u8, bool)> = p.voices_mut().map(|(v, g)| (v.key, g.step < 0.0)).collect();
        k.sort();
        k
    }

    #[test]
    fn steals_released_then_quietest_then_oldest() {
        let mut p = VoicePool::new(Policy::Poly, 2, 64, V::default);
        p.note_on(n(1, 60, 0.9));
        p.note_on(n(2, 62, 0.3));
        p.note_on(n(3, 64, 0.8)); // steals the quietest (62)
        assert_eq!(keys(&mut p), vec![(60, false), (62, true), (64, false)]);
        p.note_off(NoteId(1)); // 60 in release: the next victim
        p.note_on(n(4, 65, 0.1));
        assert_eq!(keys(&mut p), vec![(60, true), (62, true), (64, false), (65, false)]);
        p.advance(64);
        assert_eq!(keys(&mut p), vec![(64, false), (65, false)]);
    }

    #[test]
    fn mono_goes_back_to_the_held_note() {
        let mut p = VoicePool::new(Policy::Mono, 8, 32, V::default);
        p.note_on(n(1, 60, 1.0));
        p.note_on(n(2, 67, 1.0));
        p.advance(32);
        assert_eq!(keys(&mut p), vec![(67, false)]);
        p.note_off(NoteId(2));
        p.advance(32);
        assert_eq!(keys(&mut p), vec![(60, false)]);
        p.note_off(NoteId(1));
        assert!(p.voices_mut().all(|(v, _)| v.released));
    }

    #[test]
    fn legato_glides_without_restarting() {
        let mut p = VoicePool::new(Policy::Legato, 1, 32, V::default);
        p.note_on(n(1, 60, 1.0));
        p.note_on(n(2, 64, 1.0));
        p.note_off(NoteId(2));
        let v: Vec<(u8, u32, u32)> = p.voices_mut().map(|(v, _)| (v.key, v.starts, v.glides)).collect();
        assert_eq!(v, vec![(60, 1, 2)]);
    }

    #[test]
    fn one_sample_note_starts_and_releases() {
        let mut p = VoicePool::new(Policy::Poly, 4, 32, V::default);
        p.note_on(n(1, 60, 1.0));
        p.note_off(NoteId(1));
        let v: Vec<bool> = p.voices_mut().map(|(v, _)| v.released).collect();
        assert_eq!(v, vec![true]);
    }

    #[test]
    fn more_notes_than_voices_never_grows() {
        let mut p = VoicePool::new(Policy::Poly, 4, 16, V::default);
        for i in 0..1000u32 {
            p.note_on(n(i, (i % 128) as u8, (i % 7) as f32 / 7.0));
            if i % 3 == 0 {
                p.note_off(NoteId(i));
            }
            p.advance(5);
            assert!(p.active() <= 4 + 2);
        }
        p.kill_all();
        assert_eq!(p.active(), 0);
    }
}
