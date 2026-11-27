//! The instrument functions every per-note instrument gets (the legacy `InstrumentTrack`, `NotePlayHandle`,
//! `InstrumentSoundShaping`, `InstrumentFunctions` and `MonoMode`, rewritten as one processor around the plugin's
//! voices): pitch and pitch bend (F-IJN-002), mono, legato and glide (F-IJN-014), the note's fade-in and the
//! instrument's release, and the velocity as level (U28). The envelopes and LFOs of the volume, the cutoff and the
//! resonance, with the filter (F-IJN-010, F-IJN-011), are still here but apart, in [`shaping`], until they move to an
//! audio effect. Chords and the arpeggio are not the instrument's: they are note effects that run before it.
//!
//! A plugin implements [`Synth`] (its parameters and how one voice sounds) and builds an [`Instrument`] around it.
//! The instrument functions' fields and parameters are added to its schema by [`add_instrument_functions`].
//!
//! # How a block is played
//!
//! The block is cut into pieces at every event and on a grid of [`GRID`] samples of the engine's frame count. Each
//! piece starts with its events; the LFOs move on by piece. The grid makes the pieces — and so the audio — the same
//! for any block size that is a multiple of it (V3: 32, 64, 256, 1024...).
//!
//! # Notes
//!
//! A note from the host is a *base* note. It sounds as one *sounding* note, a voice of the pool (lm-render's
//! [`VoicePool`], in `Poly`, with its stealing); in mono and legato only the last base note held sounds. Sounding
//! notes get numbers of their own, above `0x8000_0000`.
//!
//! # Note expressions
//!
//! A `NoteExpression` names a base note and moves the note it sounds (and the mono or legato voice): `Tuning` adds
//! semitones to their pitch before the bend range and glide, `Pan` is a balance (the legacy instrument track's law:
//! the left at 1 − p for p > 0, the right at 1 + p for p < 0), after the velocity's gain. Each goes linearly from
//! where it is to the new value over the event's `ramp` frames (a new event replaces a ramp under way); the pan moves
//! sample by sample, the tuning by piece, as the glide (the synth renders a piece at one frequency). A base starts
//! at 0 and 0; a note released keeps the values it had.

mod notes;
mod schema;
pub mod shaping;
pub mod stream;
#[cfg(test)]
mod stream_tests;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use lm_base::{Error, Result};
use lm_dsp::basic_filters::BasicFilter;
use lm_dsp::envelope::LfoShape;
use lm_dsp::sound_shaping::{NotePosition, SoundShaping, SoundShapingSettings};
use lm_render::{
    Adopted, Event, EventKind, NoteId, Policy, ProcessContext, ProcessSpec, Processor, Rng, SampleData, Voice,
    VoiceNote, VoicePool,
};
use lm_state::ParamSpec;

pub use schema::{
    SYNC_KEYS, VOICE_MODES, add_instrument_functions, drop_chords_and_arpeggio, instrument_fields, instrument_params,
    p, sync_beats,
};
pub use shaping::{FILTER_KEYS, LFO_SHAPE_FIELDS, LFO_SHAPE_KEYS, LFO_SYNC_FIELDS, LFO_WAVE_FIELDS, TARGETS};

use crate::resources::ResourceView;
use crate::state::StateView;
use notes::{ExpressionRamp, NoteCx, NoteSettings, Run, Sounder, VoiceMode};
use shaping::ShapingKit;
pub use stream::{Stream, StreamInstrument, add_note_functions, note_fields, note_params};

/// The pieces of a block fall on this grid of the engine's frame count (the same as lm-render's automation grid)
pub const GRID: usize = 32;
/// Base notes held at once (more are ignored, as the held stack of U27)
pub const MAX_BASES: usize = 128;
/// Sounding notes followed at once (chords, arpeggio steps)
pub const MAX_SOUNDING: usize = 1024;
/// The fade of a stolen voice, ms
const STEAL_FADE_MS: f32 = 5.0;
/// The first number of a sounding note
const SOUNDING_IDS: u32 = 0x8000_0000;
/// The crossfade from the old synth to the new one when a field changes while notes sound, ms
pub const CROSSFADE_MS: f32 = 10.0;

/// A fingerprint of a state (its stable JSON hashed)
fn fingerprint(state: &lm_state::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    lm_state::to_json(state).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// What a voice is started with
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteStart {
    pub key: u8,
    /// 0 to 1, linear (U28)
    pub velocity: f32,
    /// Hz, the pitch at the start (with pitch, bend and glide)
    pub freq: f32,
}

/// A per-note instrument's own sound, around which [`Instrument`] does the rest
pub trait Synth: Clone + Send + 'static {
    /// One note's state (allocated in the build, for every voice of the pool). `clone_from` runs on the audio thread
    /// (the crossfade of a change, see [`Instrument`]'s `adopt`) and must not allocate
    type Voice: Clone + Send + 'static;
    /// The synth's own parameters: indices `0..PARAMS` of the plugin's parameters; the instrument functions' follow
    const PARAMS: usize;
    /// Whether the velocity is the note's level (the instrument multiplies by it; [`crate::Velocity::Level`]).
    /// `false`: the synth uses it itself ([`crate::Velocity::Timbre`] or [`crate::Velocity::Ignored`])
    const VELOCITY_IS_LEVEL: bool = true;

    /// A voice, in the build
    fn voice(&self) -> Self::Voice;
    /// A parameter changed (audio thread: no allocation)
    fn set_param(&mut self, index: usize, value: f32);
    /// A note starts on `voice`; `rng` is the node's (seed the voice's noise from it)
    fn start(&self, voice: &mut Self::Voice, note: &NoteStart, rng: &mut Rng);
    /// Writes the voice's next `left.len()` samples (both slices have that length), at `freq` Hz
    fn render(&self, voice: &mut Self::Voice, freq: f32, left: &mut [f32], right: &mut [f32]);
    /// Whether the voice has ended on its own (a one-shot sample played to its end, a drum's decay past): checked
    /// after each piece, the voice is then freed without waiting for its note-off. Most synths sound until released
    fn finished(&self, voice: &Self::Voice) -> bool {
        let _ = voice;
        false
    }
    /// The note is let go: its release starts at the start of the piece about to be rendered (after
    /// [`Synth::start`] when the note starts in the same piece). A synth whose voice has a release of its own (an
    /// emulated chip's gate) starts it here; the others need nothing (the instrument's release shapes their sound)
    fn release(&self, voice: &mut Self::Voice) {
        let _ = voice;
    }
    /// The release the synth wants even without a volume envelope, ms (legacy `desiredReleaseTimeMs`)
    fn release_ms(&self) -> f32 {
        0.0
    }
    /// Whether the instrument fades a released voice out over [`Synth::release_ms`] (legacy `applyRelease`).
    /// `false` for a synth whose voice fades itself (its own envelopes): the voice is only kept that long
    const RELEASE_FADE: bool = true;
    /// The song's tempo, BPM: before the first piece and whenever it changes (a synth with tempo-synced knobs, legacy
    /// `TempoSyncKnob`, works out their values from it). Most synths ignore it
    fn set_tempo(&mut self, bpm: f64) {
        let _ = bpm;
    }
    /// Takes over what the old synth keeps across notes (most keep nothing: their voices carry the sound)
    fn adopt(&mut self, old: &mut Self) {
        let _ = old;
    }
    /// Whether this synth can play on the old one's voices (an adopting instrument keeps the old pool): `false` when
    /// they lack what its notes need (room for longer strings than they were made for); the old instrument then plays
    /// its tail apart (U32). Most synths can
    fn can_adopt(&self, old: &Self) -> bool {
        let _ = old;
        true
    }
    /// Whether this synth's sounding voices can be copied on the audio thread (`clone_from` into a voice made in the
    /// build), for the crossfade of a change: `false` when a voice holds state that cannot be copied without
    /// allocating (a C library's converter). A changed instrument whose old synth says `false` is not adopted: the old
    /// one plays its tail apart (U32). The same state is still adopted (no crossfade, nothing copied)
    fn copies_voices(&self) -> bool {
        true
    }
    /// Fills the plugin window's live frame (its [`crate::DisplaySpec`]'s bytes, all of them), once per block, when the
    /// window is open. What the synth shows must not depend on the block size
    fn display(&self, frame: &mut [u8]) {
        frame.fill(0);
    }
}

#[derive(Clone, Debug)]
struct Kit {
    params: [f32; p::COUNT],
    shaping: ShapingKit,
    mode: VoiceMode,
}

impl Kit {
    fn read(state: &StateView<'_>) -> Result<Self> {
        let mut params = [0.0f32; p::COUNT];
        for (i, spec) in instrument_params().iter().enumerate() {
            params[i] = state.param_f32(spec.key)?;
        }
        Ok(Self { params, shaping: ShapingKit::read(state)?, mode: state.choice("voiceMode", VoiceMode::from_key)? })
    }

    /// The sound shaping's settings now (see [`shaping`])
    fn shaping_settings(&self, bpm: f64) -> SoundShapingSettings {
        self.shaping.settings(&self.params[p::SHAPING..p::SHAPING + shaping::s::COUNT], bpm)
    }

    /// What the notes read now
    fn notes(&self) -> NoteSettings {
        NoteSettings { glide: self.params[p::GLIDE], mode: self.mode }
    }
}

/// A voice of the pool: the synth's voice and what the instrument functions keep per note
pub struct KitVoice<V> {
    inner: V,
    /// The voice as the old synth plays it, during a crossfade
    ghost: V,
    ghost_on: bool,
    note: NoteId,
    key: u8,
    velocity: f32,
    /// Frames played since the note started
    frame: i32,
    started: bool,
    release_pending: bool,
    released: bool,
    release_begin: i32,
    release_end: i32,
    finished: bool,
    /// The fade-in's length, frames (legacy `m_fadeInLength`); 0 before the first piece
    fade_in: f32,
    /// Glide: semitones from the key at the start, the frames it takes, the frames left, the offset now
    glide_from: f32,
    glide_frames: i32,
    glide_left: i32,
    glide_offset: f32,
    filter: BasicFilter,
    level: f32,
    /// The note expressions it sounds with, as its base had them at the last piece (kept once the base lets go)
    tuning: f32,
    pan: f32,
}

impl<V> fmt::Debug for KitVoice<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KitVoice").field("key", &self.key).field("frame", &self.frame).finish_non_exhaustive()
    }
}

impl<V> KitVoice<V> {
    fn new(inner: V, ghost: V, rate: f32) -> Self {
        Self {
            inner,
            ghost,
            ghost_on: false,
            note: NoteId(0),
            key: 0,
            velocity: 0.0,
            frame: 0,
            started: false,
            release_pending: false,
            released: false,
            release_begin: 0,
            release_end: 0,
            finished: true,
            fade_in: 0.0,
            glide_from: 0.0,
            glide_frames: 0,
            glide_left: 0,
            glide_offset: 0.0,
            filter: BasicFilter::new(rate),
            level: 0.0,
            tuning: 0.0,
            pan: 0.0,
        }
    }

    /// The pitch it sounds now, in keys
    fn pitch(&self) -> f32 {
        f32::from(self.key) + self.glide_offset
    }

    /// Glides from `from_pitch` (keys) to its key over `frames`
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
}

impl<V: Send> Voice for KitVoice<V> {
    fn start(&mut self, note: VoiceNote) {
        self.note = note.note;
        self.key = note.key;
        self.velocity = note.velocity;
        self.frame = 0;
        self.started = false;
        self.release_pending = false;
        self.released = false;
        self.release_begin = 0;
        self.release_end = 0;
        self.finished = false;
        self.fade_in = 0.0;
        self.glide_left = 0;
        self.glide_offset = 0.0;
        self.filter.clear_history();
        self.level = note.velocity;
        self.ghost_on = false;
        self.tuning = 0.0;
        self.pan = 0.0;
    }

    fn glide(&mut self, note: VoiceNote) {
        // (the pool is always Poly here: legato is the instrument's, by base note)
        let from = self.pitch();
        self.note = note.note;
        self.key = note.key;
        self.glide_from_pitch(from, 0);
    }

    fn release(&mut self) {
        if !self.released {
            self.release_pending = true;
        }
    }

    fn level(&self) -> f32 {
        self.level
    }

    fn is_finished(&self) -> bool {
        self.finished
    }

    fn kill(&mut self) {
        self.finished = true;
        self.started = false;
        self.release_pending = false;
        self.released = false;
        self.level = 0.0;
    }
}

/// The voice pool as the notes' [`Sounder`]
struct Pool<'a, V: Send>(&'a mut VoicePool<KitVoice<V>>);

impl<V: Send> Sounder for Pool<'_, V> {
    fn on(&mut self, note: VoiceNote) {
        self.0.note_on(note);
    }

    fn glide(&mut self, id: NoteId, from: f32, frames: i32) {
        for (v, _) in self.0.voices_mut() {
            if v.note == id {
                v.glide_from_pitch(from, frames);
            }
        }
    }

    fn off(&mut self, id: NoteId) {
        self.0.note_off(id);
    }

    fn pitch(&mut self, id: NoteId) -> Option<f32> {
        self.0.voices_mut().find(|(v, _)| v.note == id).map(|(v, _)| v.pitch())
    }

    fn retarget(&mut self, id: NoteId, key: u8, frames: i32) {
        for (v, _) in self.0.voices_mut() {
            if v.note == id {
                let pitch = v.pitch();
                v.key = key;
                v.glide_from_pitch(pitch, frames);
            }
        }
    }

    fn release_all(&mut self) {
        self.0.release_all();
    }

    fn kill_all(&mut self) {
        self.0.kill_all();
    }
}

/// The scratch of one piece
struct Scratch {
    left: Box<[f32]>,
    right: Box<[f32]>,
    ghost_left: Box<[f32]>,
    ghost_right: Box<[f32]>,
    frames: Box<[[f32; 2]]>,
}

/// A per-note instrument: the plugin's [`Synth`] with the instrument functions around it
pub struct Instrument<S: Synth> {
    synth: S,
    kit: Kit,
    /// Every parameter's value now (the synth's, then the instrument functions'), and its range
    values: Box<[f32]>,
    ranges: Box<[(f32, f32)]>,
    rate: f32,
    sample_rate: u32,
    lfo_waves: [Option<Arc<SampleData>>; 3],
    shaping: SoundShaping,
    settings: SoundShapingSettings,
    dirty: bool,
    bpm: f64,
    /// The tempo the synths were last told ([`Synth::set_tempo`]); 0 before the first piece
    synth_bpm: f64,
    pool: VoicePool<KitVoice<S::Voice>>,
    run: Run,
    scratch: Scratch,
    previous_piece: usize,
    /// A change that is not a parameter's crossfades from the old synth to the new: the old one plays the copies of
    /// the voices (`ghost`) for `fade_len` frames. `fading` is a spare synth swapped with the old processor's
    fading: S,
    fade_left: u32,
    fade_len: u32,
    /// A fingerprint of the state it was built from: the same state adopted needs no crossfade
    fingerprint: u64,
}

impl<S: Synth> fmt::Debug for Instrument<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Instrument").field("rate", &self.sample_rate).field("voices", &self.pool.active()).finish()
    }
}

impl<S: Synth> Instrument<S> {
    /// The instrument for a complete `state` whose schema is `params` (all of the plugin's parameters: the synth's
    /// `S::PARAMS` first, then the instrument functions'), with `voices` voices. Allocates everything (U31)
    pub fn new(
        synth: S,
        state: &StateView<'_>,
        params: &[ParamSpec],
        spec: &ProcessSpec,
        resources: &ResourceView,
        voices: usize,
    ) -> Result<Self> {
        if params.len() != S::PARAMS + p::COUNT {
            return Err(Error::new("plugin.instrument.params").arg("found", params.len() as i64));
        }
        let kit = Kit::read(state)?;
        let mut values = Vec::with_capacity(params.len());
        for spec in params {
            values.push(state.param_f32(spec.key)?);
        }
        let ranges: Box<[(f32, f32)]> = params.iter().map(|p| (p.min as f32, p.max as f32)).collect();
        let mut lfo_waves: [Option<Arc<SampleData>>; 3] = [None, None, None];
        for (t, key) in LFO_WAVE_FIELDS.iter().enumerate() {
            if let Some(r) = state.resource(key)? {
                // (a missing wave matters only if the LFO plays it, as the legacy program)
                match resources.audio(r) {
                    Ok(a) if a.frames() > 0 => lfo_waves[t] = Some(a),
                    Ok(_) => {}
                    Err(e) if kit.shaping.lfo_shape[t] == LfoShape::UserDefined => return Err(e),
                    Err(_) => {}
                }
            }
        }
        let rate = spec.sample_rate as f32;
        let mut synth = synth;
        for (i, v) in values.iter().take(S::PARAMS).enumerate() {
            synth.set_param(i, *v);
        }
        let fade = ((STEAL_FADE_MS / 1000.0) * rate) as u32;
        let pool =
            VoicePool::new(Policy::Poly, voices.max(1), fade, || KitVoice::new(synth.voice(), synth.voice(), rate));
        let fading = synth.clone();
        let settings = kit.shaping_settings(120.0);
        let mut shaping = SoundShaping::new(rate, GRID, 0x5eed_1f05);
        shaping.set(&settings, GRID);
        Ok(Self {
            synth,
            kit,
            values: values.into_boxed_slice(),
            ranges,
            rate,
            sample_rate: spec.sample_rate,
            lfo_waves,
            shaping,
            settings,
            dirty: true,
            bpm: 0.0,
            synth_bpm: 0.0,
            pool,
            run: Run::new(),
            scratch: Scratch {
                left: vec![0.0; GRID].into_boxed_slice(),
                right: vec![0.0; GRID].into_boxed_slice(),
                ghost_left: vec![0.0; GRID].into_boxed_slice(),
                ghost_right: vec![0.0; GRID].into_boxed_slice(),
                frames: vec![[0.0; 2]; GRID].into_boxed_slice(),
            },
            previous_piece: 0,
            fading,
            fade_left: 0,
            fade_len: ((CROSSFADE_MS / 1000.0) * rate) as u32,
            fingerprint: fingerprint(state.value()),
        })
    }

    /// Voices sounding (fading ones included)
    pub fn active_voices(&self) -> usize {
        self.pool.active()
    }

    /// What the notes are played with now
    fn note_cx(&self) -> NoteCx {
        NoteCx { ns: self.kit.notes(), rate: self.rate }
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
            self.synth.set_param(i, v);
            return;
        }
        let k = i - S::PARAMS;
        self.kit.params[k] = v;
        if (p::SHAPING..p::GLIDE).contains(&k) {
            self.dirty = true;
        }
    }

    fn sound_off(&mut self) {
        self.run.sound_off(&mut Pool(&mut self.pool));
    }

    fn event(&mut self, kind: EventKind) {
        let cx = self.note_cx();
        let out = &mut Pool(&mut self.pool);
        match kind {
            EventKind::NoteOn { note, channel, key, velocity } => {
                self.run.note_on(&cx, out, note, channel, key.min(127), clean_velocity(velocity));
            }
            EventKind::NoteOff { note, .. } => self.run.note_off(&cx, out, note),
            EventKind::Param { index, value } => self.set_param(index, value),
            EventKind::Midi { bytes } => match bytes[0] & 0xF0 {
                0xE0 => self.run.set_bend(bytes),
                0xB0 => match bytes[1] {
                    64 => self.run.pedal(&cx, out, bytes[2] >= 64),
                    120 => self.run.sound_off(out),
                    123 => self.run.release_all(out),
                    _ => {}
                },
                _ => {}
            },
            EventKind::AllNotesRelease => self.run.release_all(out),
            EventKind::AllSoundOff => self.run.sound_off(out),
            EventKind::NoteExpression { note, expression, value, ramp } => {
                self.run.express(note, expression, value, ramp)
            }
        }
    }

    // ---- playing ----

    /// Plays every voice for `n` frames into `left`/`right` from `at`
    fn piece(&mut self, at: usize, n: usize, left: &mut [f32], right: &mut [f32], rng: &mut Rng) {
        let rate = self.rate;
        let pitch = self.kit.params[p::PITCH] / 100.0 + self.run.bend * self.kit.params[p::BEND_RANGE];
        let desired = (self.synth.release_ms() * rate / 1000.0).ceil() as i32;
        let (fade_left, fade_len) = (self.fade_left, self.fade_len.max(1));
        let Self { synth, pool, shaping, settings, scratch, fading, run, .. } = self;
        for (v, gain) in pool.voices_mut() {
            if v.finished {
                continue;
            }
            // The note expressions of its base, while it has one (a released note keeps the last ones)
            let mut pan_ramp = ExpressionRamp { value: v.pan, target: v.pan, left: 0 };
            if let Some(base) = run.sounding.iter().find(|s| s.id == v.note).map(|s| s.base)
                && let Some(b) = run.bases.iter().find(|b| b.id == base)
            {
                v.tuning = b.tuning.value;
                pan_ramp = b.pan;
            }
            let mut releasing = false;
            if v.release_pending && !v.released {
                v.released = true;
                v.release_pending = false;
                v.release_begin = v.frame;
                let frames = shaping.release_frames(desired, false).max(desired).max(1);
                v.release_end = v.release_begin.saturating_add(frames);
                releasing = true;
            }
            let m = if v.released { n.min((v.release_end - v.frame).max(0) as usize) } else { n };
            if m == 0 {
                v.finished = true;
                continue;
            }
            if v.glide_left > 0 {
                v.glide_offset = v.glide_from * v.glide_left as f32 / v.glide_frames as f32;
                v.glide_left = v.glide_left.saturating_sub(n as i32).max(0);
            } else {
                v.glide_offset = 0.0;
            }
            let keys = f32::from(v.key) - 69.0 + v.glide_offset + pitch + v.tuning;
            let freq = 440.0 * libm::exp2f(keys / 12.0);
            if !v.started {
                v.started = true;
                synth.start(&mut v.inner, &NoteStart { key: v.key, velocity: v.velocity, freq }, rng);
            }
            if releasing {
                synth.release(&mut v.inner);
                if v.ghost_on {
                    fading.release(&mut v.ghost);
                }
            }
            let (l, r) = (&mut scratch.left[..m], &mut scratch.right[..m]);
            l.fill(0.0);
            r.fill(0.0);
            synth.render(&mut v.inner, freq, l, r);
            if v.ghost_on && fade_left > 0 {
                let (gl, gr) = (&mut scratch.ghost_left[..m], &mut scratch.ghost_right[..m]);
                gl.fill(0.0);
                gr.fill(0.0);
                fading.render(&mut v.ghost, freq, gl, gr);
                for i in 0..m {
                    let x = (1.0 - fade_left.saturating_sub(i as u32) as f32 / fade_len as f32).clamp(0.0, 1.0);
                    l[i] = gl[i] + (l[i] - gl[i]) * x;
                    r[i] = gr[i] + (r[i] - gr[i]) * x;
                }
            }
            let buf = &mut scratch.frames[..m];
            for (i, f) in buf.iter_mut().enumerate() {
                *f = [l[i], r[i]];
            }
            fade_in(v, buf, rate);
            if S::RELEASE_FADE && v.released && desired > 0 {
                for (i, f) in buf.iter_mut().enumerate() {
                    let left_frames = v.release_end - (v.frame + i as i32);
                    if left_frames <= desired {
                        let fac = left_frames as f32 / desired as f32;
                        f[0] *= fac;
                        f[1] *= fac;
                    }
                }
            }
            let release_begin = if v.released { v.release_begin } else { i32::MAX };
            shaping.process(buf, &mut v.filter, NotePosition { frame: v.frame, release_begin }, settings);
            let velocity = if S::VELOCITY_IS_LEVEL { v.velocity } else { 1.0 };
            let mut level = 0.0f32;
            let panned = pan_ramp.value != 0.0 || pan_ramp.target != 0.0;
            for (i, f) in buf.iter().enumerate() {
                let g = velocity * gain.at(i);
                let (mut a, mut b) = (f[0] * g, f[1] * g);
                if panned {
                    let p = pan_ramp.at(i as u32);
                    a *= if p > 0.0 { 1.0 - p } else { 1.0 };
                    b *= if p < 0.0 { 1.0 + p } else { 1.0 };
                }
                left[at + i] += a;
                right[at + i] += b;
                level = level.max(a.abs()).max(b.abs());
            }
            v.level = level;
            v.pan = pan_ramp.at(m as u32);
            v.frame = v.frame.saturating_add(m as i32);
            if (v.released && v.frame >= v.release_end) || synth.finished(&v.inner) {
                v.finished = true;
            }
        }
        for b in &mut self.run.bases {
            b.tuning.advance(n as u32);
            b.pan.advance(n as u32);
        }
        self.pool.advance(n);
        self.run.frame += n as u64;
        self.fade_left = self.fade_left.saturating_sub(n as u32);
    }
}

/// The legacy `Instrument::applyFadeIn`: at most 85 frames at 44.1 kHz (about 2 ms), shorter the more often the note
/// crosses zero (counted per 128 frames at 44.1 kHz)
fn fade_in<V>(v: &mut KitVoice<V>, buf: &mut [[f32; 2]], rate: f32) {
    let rate_factor = rate / 44100.0;
    let max_length = 85.0 * rate_factor;
    let block = 128.0 * rate_factor;
    let frames = buf.len();
    let crossings = |buf: &[[f32; 2]]| -> i32 {
        let mut counts = [0i32; 2];
        let mut most = 0;
        for f in 1..buf.len() {
            for ch in 0..2 {
                let (a, b) = (buf[f - 1][ch], buf[f][ch]);
                if (a <= 0.0 && b > 0.0) || (a >= 0.0 && b < 0.0) {
                    counts[ch] += 1;
                    most = most.max(counts[ch]);
                }
            }
        }
        most
    };
    let length_for = |zc: i32| -> f32 { (max_length / (zc as f32 / (frames as f32 / block) + 1.0)).trunc() };
    let pi = std::f32::consts::PI;
    let total = v.frame;
    if total == 0 {
        let length = length_for(crossings(buf));
        v.fade_in = length;
        let n = (length as usize).min(frames);
        for (f, frame) in buf.iter_mut().enumerate().take(n) {
            let g = 0.5 - 0.5 * libm::cosf(pi * f as f32 / length);
            frame[0] *= g;
            frame[1] *= g;
        }
    } else if (total as f32) < v.fade_in {
        let new_length = length_for(crossings(buf));
        for (f, frame) in buf.iter_mut().enumerate() {
            let x = f as f32 / frames as f32;
            let current = v.fade_in * (1.0 - x) + new_length * x;
            if (total + f as i32) as f32 >= current {
                v.fade_in = current.trunc();
                return;
            }
            let g = 0.5 - 0.5 * libm::cosf(pi * (total + f as i32) as f32 / current);
            frame[0] *= g;
            frame[1] *= g;
        }
        v.fade_in = new_length;
    }
}

impl<S: Synth> Processor for Instrument<S> {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let events: &[Event] = cx.events;
        let transport = cx.transport;
        let rng = &mut *cx.rng;
        let outputs = cx.buffers.outputs();
        if outputs == 0 {
            for e in events {
                self.event(e.kind);
            }
            return;
        }
        let (_, mut outs, _) = cx.buffers.split();
        let (left, right): (&mut [f32], &mut [f32]) = if outputs >= 2 {
            outs.pair_mut(0, 1)
        } else {
            // (a mono output: both sides summed into it below)
            let l = outs.get_mut(0);
            (l, &mut [])
        };
        let mono = outputs < 2;
        let mut e = 0;
        let mut at = 0;
        let grid_origin = transport.frame;
        while at < frames {
            while e < events.len() && (events[e].offset as usize) <= at {
                self.event(events[e].kind);
                e += 1;
            }
            let mut end = frames;
            if e < events.len() {
                end = end.min(events[e].offset as usize);
            }
            let grid = GRID - ((grid_origin + at as u64) % GRID as u64) as usize;
            end = end.min(at + grid);
            let n = end.saturating_sub(at).max(1).min(frames - at);
            self.lfos(n, at, transport);
            if mono {
                // Render into the scratch of a second side: the left output gets the average
                let mut tmp_l = [0.0f32; GRID];
                let mut tmp_r = [0.0f32; GRID];
                self.piece(0, n, &mut tmp_l, &mut tmp_r, rng);
                for i in 0..n {
                    left[at + i] += 0.5 * (tmp_l[i] + tmp_r[i]);
                }
            } else {
                self.piece(at, n, left, right, rng);
            }
            at += n;
        }
        for ev in &events[e..] {
            self.event(ev.kind);
        }
        if let Some(writer) = cx.display.as_mut() {
            writer.write(|frame| self.synth.display(frame));
        }
    }

    fn reset(&mut self) {
        self.sound_off();
        for env in [&mut self.shaping.volume, &mut self.shaping.cutoff, &mut self.shaping.resonance] {
            env.reset_lfo(None, GRID);
        }
        self.dirty = true;
        self.previous_piece = 0;
    }

    /// Takes over the voices, the notes, the LFOs and the current parameter values of the processor it replaces, when
    /// that is the same instrument at the same rate (R18)
    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        if old.sample_rate != self.sample_rate
            || old.values.len() != self.values.len()
            || !self.synth.can_adopt(&old.synth)
            || (old.fingerprint != self.fingerprint && !old.synth.copies_voices())
        {
            return Adopted::No;
        }
        std::mem::swap(&mut self.pool, &mut old.pool);
        std::mem::swap(&mut self.run, &mut old.run);
        std::mem::swap(&mut self.shaping, &mut old.shaping);
        self.previous_piece = old.previous_piece;
        self.bpm = old.bpm;
        // The values the old one heard last (automation and gestures since the tree was read)
        for i in 0..self.values.len() {
            let v = old.values[i];
            self.set_param(i as u32, v);
        }
        self.synth.adopt(&mut old.synth);
        if old.fingerprint != self.fingerprint {
            // The old synth plays copies of the voices while the new one fades in
            std::mem::swap(&mut self.fading, &mut old.synth);
            for (v, _) in self.pool.voices_mut() {
                v.ghost_on = v.started && !v.finished;
                if v.ghost_on {
                    v.ghost.clone_from(&v.inner);
                }
            }
            self.fade_left = self.fade_len;
        }
        // A change between poly, mono and legato: the notes go on sounding; in mono and legato the last base held is
        // the one that sounds from now on (the others end at their note-off)
        if old.kit.mode != self.kit.mode {
            self.run.mode_changed(self.kit.mode);
        }
        self.dirty = true;
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A velocity inside 0 to 1 (`f32::clamp` hands NaN on: a NaN velocity is silence)
pub(crate) fn clean_velocity(velocity: f32) -> f32 {
    if velocity.is_finite() { velocity.clamp(0.0, 1.0) } else { 0.0 }
}

#[cfg(test)]
mod velocity_tests {
    use super::clean_velocity;

    #[test]
    fn a_velocity_is_always_a_number_in_range() {
        assert_eq!(clean_velocity(f32::NAN), 0.0);
        assert_eq!(clean_velocity(f32::INFINITY), 0.0);
        assert_eq!(clean_velocity(7.0), 1.0);
        assert_eq!(clean_velocity(-1.0), 0.0);
        assert_eq!(clean_velocity(0.5), 0.5);
    }
}
