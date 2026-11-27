//! The engine: a pre-allocated pool of voices that plays an [`Instrument`]. It has no thread of its own and
//! allocates nothing after [`Engine::new`]; the same instrument, the same seed and the same calls give the same
//! samples (its random numbers come from its own generator).
//!
//! # Notes
//!
//! A note-on picks the zones that match the key and the velocity, minus the ones the round-robin turn, the random
//! range and the trigger rule leave out, and starts a voice for each. Before the voices start, the exclusive groups
//! end what they choke. A note-off releases the note's voices (unless the zone is a one-shot) and starts the
//! release-trigger zones. The sustain pedal holds the note-offs. When the pool is full the voice to go is the
//! quietest releasing one, or else the oldest, and it falls in a few milliseconds while the new voice takes a spare
//! slot: stealing never clicks.
//!
//! # Rendering
//!
//! [`Engine::render`] mixes `frames` frames into the output buses, one stereo pair per output group, replacing what
//! they held. A voice's pitch, filter, pan and gain are worked out on a grid of [`SUB`] frames of the engine's own
//! absolute frame count (and once when the voice begins or a legato note moves it), and its envelopes step every
//! frame: the grid never depends on where a call starts, so the host's block size cannot change the samples.

use std::sync::Arc;

use crate::env::Env;
use crate::filter::Biquad;
use crate::interp::{Quality, SincTable, Wrap, read};
use crate::model::{Ahdsr, Instrument, LoopMode, Trigger, VelCurve, Window, Zone};

/// The most voices that sound at once (the polyphony's ceiling)
pub const MAX_POLYPHONY: usize = 256;
/// Voices beyond the polyphony that hold the ones stolen while they fade out
const SPARE: usize = 16;
/// The frames of a sub-block
pub const SUB: usize = 32;
/// Notes remembered for the release triggers and the pedal
const HELD: usize = 512;
/// Declick fade, frames
const FADE: u32 = 32;
/// The time a stolen or choked voice takes to fall, seconds
const FAST_RELEASE: f32 = 0.004;

/// Replaces the zones' amplitude envelopes (the plugin's envelope knobs)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvOverride {
    pub envelope: Ahdsr,
    /// 0 to 1: how much of the sustain level the envelope keeps, as the legacy ENV tab's amount
    pub amount: f32,
}

/// What the plugin's parameters set; every change is applied at the next grid boundary
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub gain_db: f32,
    /// −1 to 1, added to every zone's pan
    pub pan: f32,
    /// Cents
    pub tune: f32,
    pub polyphony: usize,
    /// Semitones for a full bend
    pub bend_range: f32,
    /// `Some` while the amount is above 0
    pub env: Option<EnvOverride>,
    /// The window knobs of the zones that have one (`Zone::windowed`)
    pub window: Window,
    pub reverse: bool,
    pub stutter: bool,
    /// Mono: a new note ends the sounding ones; releasing it brings back the note held before
    pub mono: bool,
    /// With `mono`: a note played while another is held moves the sounding voices to its pitch instead of starting
    /// again (when the voices' zones cover its key)
    pub legato: bool,
    /// Seconds a new note takes to glide from the last note's pitch (0: none)
    pub glide: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            pan: 0.0,
            tune: 0.0,
            polyphony: 64,
            bend_range: 2.0,
            env: None,
            window: Window::default(),
            reverse: false,
            stutter: false,
            mono: false,
            legato: false,
            glide: 0.0,
        }
    }
}

/// How an engine is made
#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    /// The output rate
    pub rate: u32,
    /// The most frames one [`Engine::render`] call is asked
    pub max_block: usize,
    pub quality: Quality,
    /// Seeds the random groups
    pub seed: u64,
    /// One output pair per group of the instrument (otherwise every group mixes to the first)
    pub multi: bool,
}

impl EngineConfig {
    pub fn new(rate: u32, max_block: usize) -> Self {
        Self { rate, max_block, quality: Quality::Sinc, seed: 1, multi: false }
    }
}

/// xorshift64*: small, fast and the same everywhere
#[derive(Clone, Copy, Debug)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (x >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[derive(Clone, Copy, Debug)]
struct Held {
    id: u32,
    key: u8,
    velocity: u8,
    at: u64,
    /// The note-off came while the pedal was down
    pedal: bool,
}

#[derive(Clone, Debug)]
struct Voice {
    active: bool,
    /// Falling in a few milliseconds: does not count against the polyphony
    dying: bool,
    zone: u32,
    id: u32,
    key: u8,
    order: u64,
    released: bool,
    /// A note-off does not end it (a one-shot, a release trigger)
    ignores_off: bool,
    pos: f64,
    dir: f64,
    start: f64,
    end: f64,
    loop_start: f64,
    loop_end: f64,
    looping: LoopMode,
    /// Frames of the sample per output frame
    ratio: f64,
    /// The pitch from the key and the zone's tuning, cents
    cents: f32,
    /// The note expression's tuning, cents
    detune: f32,
    /// The glide's offset from the key's pitch, cents, and how fast it closes, cents per second
    glide: f32,
    glide_rate: f32,
    /// MPE pressure (0 to 1) and timbre (0 to 1, 0.5 the middle)
    pressure: f32,
    timbre: f32,
    pan: f32,
    gain: f32,
    vel: f32,
    amp: Env,
    fenv: Env,
    filter: Biquad,
    has_filter: bool,
    fade: u32,
    declick: bool,
    output: u8,
    stutter: bool,
    release_time: f32,
    /// What the last update (at a grid boundary, or when the voice started) worked out and the frames until the
    /// next one use: the pitch step, the pan gains and the total gain
    step: f64,
    gl: f32,
    gr: f32,
    k_gain: f32,
    /// Work the values out at the next frame even if it is not a grid boundary (a voice that just began or moved)
    fresh: bool,
}

impl Voice {
    fn idle() -> Self {
        Self {
            active: false,
            dying: false,
            zone: 0,
            id: 0,
            key: 0,
            order: 0,
            released: false,
            ignores_off: false,
            pos: 0.0,
            dir: 1.0,
            start: 0.0,
            end: 0.0,
            loop_start: 0.0,
            loop_end: 0.0,
            looping: LoopMode::Off,
            ratio: 1.0,
            cents: 0.0,
            detune: 0.0,
            glide: 0.0,
            glide_rate: 0.0,
            pressure: 0.0,
            timbre: 0.5,
            pan: 0.0,
            gain: 1.0,
            vel: 1.0,
            amp: Env::idle(),
            fenv: Env::idle(),
            filter: Biquad::default(),
            has_filter: false,
            fade: 0,
            declick: false,
            output: 0,
            stutter: false,
            release_time: 0.0,
            step: 0.0,
            gl: 1.0,
            gr: 1.0,
            k_gain: 1.0,
            fresh: true,
        }
    }
}

/// The bounds of a window on a sample of `size` frames: the start before the end and a frame apart, the loop between
/// them (the AudioFileProcessor's rule)
pub fn window_bounds(size: usize, w: &Window) -> (u32, u32, u32) {
    let size = u32::try_from(size).unwrap_or(u32::MAX);
    let frame = |p: f32| ((p * size as f32) as i64).clamp(0, i64::from(size)) as u32;
    let (mut start, mut end) = (frame(w.start), frame(w.end));
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    if end - start < 1 && size > 0 {
        if end < size {
            end += 1;
        } else {
            start = end - 1;
        }
    }
    let loop_start = frame(w.loop_start).clamp(start, if end > start { end - 1 } else { start });
    (start, end, loop_start)
}

fn db_to_lin(db: f32) -> f32 {
    libm::powf(10.0, db / 20.0)
}

/// The engine
pub struct Engine {
    instrument: Arc<Instrument>,
    table: Arc<SincTable>,
    cfg: EngineConfig,
    params: Params,
    voices: Vec<Voice>,
    held: Vec<Held>,
    /// The round-robin turn of each group, and whether the note in hand touched it
    turns: Vec<u32>,
    touched: Vec<bool>,
    /// Where the last note of a stutter zone stopped (−1: nowhere yet)
    stutter_at: Vec<f64>,
    rng: Rng,
    /// Notes started, for the order of voices
    order: u64,
    /// Frames rendered since the start or the reset
    frame: u64,
    sustain: bool,
    /// The key of the last note started (the glide's origin)
    last_key: Option<u8>,
    bend: f32,
    volume: f32,
    expression: f32,
    groups: usize,
    bus: Vec<f32>,
    bus_frames: usize,
    rate: f32,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("instrument", &self.instrument.name)
            .field("zones", &self.instrument.zones.len())
            .field("active", &self.active_voices())
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// An engine for `instrument` (build time: this allocates everything the engine will ever use)
    pub fn new(instrument: Arc<Instrument>, cfg: EngineConfig) -> Self {
        let groups = if cfg.multi { instrument.outputs.len().max(1) } else { 1 };
        let seqs = instrument.sequence_groups();
        let zones = instrument.zones.len();
        let held_cap = HELD;
        Self {
            table: SincTable::new(),
            params: Params::default(),
            voices: vec![Voice::idle(); MAX_POLYPHONY + SPARE],
            held: Vec::with_capacity(held_cap),
            turns: vec![0; seqs],
            touched: vec![false; seqs],
            stutter_at: vec![-1.0; zones],
            rng: Rng::new(cfg.seed),
            order: 0,
            frame: 0,
            sustain: false,
            last_key: None,
            bend: 0.0,
            volume: 1.0,
            expression: 1.0,
            groups,
            bus: vec![0.0; groups * 2 * cfg.max_block.max(1)],
            bus_frames: 0,
            rate: cfg.rate as f32,
            instrument,
            cfg,
        }
    }

    pub fn instrument(&self) -> &Arc<Instrument> {
        &self.instrument
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn set_params(&mut self, p: Params) {
        self.params = Params { polyphony: p.polyphony.clamp(1, MAX_POLYPHONY), ..p };
    }

    /// Output groups the buses hold
    pub fn groups(&self) -> usize {
        self.groups
    }

    /// The pitch in cents of the first sounding voice from the key, without bend and glide (for the tests)
    pub fn voice_cents(&self, n: usize) -> Option<f32> {
        self.voices.iter().filter(|v| v.active).nth(n).map(|v| v.cents)
    }

    /// The glide's offset of a sounding voice, cents
    pub fn voice_glide(&self, n: usize) -> Option<f32> {
        self.voices.iter().filter(|v| v.active).nth(n).map(|v| v.glide)
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// The pair of buses of output group `g` as the last [`Engine::render`] left them
    pub fn bus(&self, g: usize) -> (&[f32], &[f32]) {
        let n = self.cfg.max_block.max(1);
        let g = g.min(self.groups - 1);
        let l = &self.bus[2 * g * n..2 * g * n + self.bus_frames];
        let r = &self.bus[(2 * g + 1) * n..(2 * g + 1) * n + self.bus_frames];
        (l, r)
    }

    /// Whether a new engine on the same instrument could take this one's voices over bit for bit
    pub fn same_sound(&self, other: &Engine) -> bool {
        Arc::ptr_eq(&self.instrument, &other.instrument)
            && self.cfg.rate == other.cfg.rate
            && self.cfg.multi == other.cfg.multi
            && self.cfg.max_block == other.cfg.max_block
            && self.cfg.quality == other.cfg.quality
    }

    /// Takes the live state (voices, turns, pedal) of `old`, which must be [`Engine::same_sound`]
    pub fn take_over(&mut self, old: &mut Engine) {
        std::mem::swap(&mut self.voices, &mut old.voices);
        std::mem::swap(&mut self.held, &mut old.held);
        std::mem::swap(&mut self.turns, &mut old.turns);
        std::mem::swap(&mut self.stutter_at, &mut old.stutter_at);
        std::mem::swap(&mut self.rng, &mut old.rng);
        self.order = old.order;
        self.frame = old.frame;
        self.sustain = old.sustain;
        self.last_key = old.last_key;
        self.bend = old.bend;
        self.volume = old.volume;
        self.expression = old.expression;
    }

    // --- events -----------------------------------------------------------------------------------------------

    /// A note starts. `velocity` is 0 to 1
    pub fn note_on(&mut self, id: u32, key: u8, velocity: f32) {
        let key = key.min(127);
        let vel = (velocity.clamp(0.0, 1.0) * 127.0 + 0.5).clamp(1.0, 127.0) as u8;
        let others_held = !self.held.is_empty();
        self.trigger(id, key, vel, others_held);
        if self.held.len() < self.held.capacity() {
            self.held.push(Held { id, key, velocity: vel, at: self.frame, pedal: false });
        }
    }

    /// Starts the voices of a note (`vel` is 1 to 127), in the mono modes ending or moving the sounding ones first
    fn trigger(&mut self, id: u32, key: u8, vel: u8, others_held: bool) {
        if self.params.mono {
            if self.params.legato && others_held && self.legato_move(id, key, vel) {
                self.last_key = Some(key);
                return;
            }
            // Mono: the sounding notes end (fast: no click) as this one starts
            for v in self.voices.iter_mut().filter(|v| v.active && !v.dying && !v.released && !v.ignores_off) {
                if v.id != 0 {
                    v.amp.release(FAST_RELEASE, self.rate);
                    v.released = true;
                    v.dying = true;
                }
            }
        }
        let r = self.rng.next_f32();
        let inst = Arc::clone(&self.instrument);
        self.touched.iter_mut().for_each(|t| *t = false);
        // First, the groups choke (before any voice of this note exists)
        for z in inst.zones.iter() {
            if z.group != 0 && self.selected(z, key, vel, r, others_held) {
                self.choke(z.group);
            }
        }
        for (i, z) in inst.zones.iter().enumerate() {
            if self.selected(z, key, vel, r, others_held) {
                if let Some(s) = z.sequence {
                    self.touched[usize::from(s.group)] = true;
                }
                self.start_voice(i, z, id, key, f32::from(vel) / 127.0, 0.0);
            } else if let Some(s) = z.sequence
                && z.trigger != Trigger::Release
                && z.matches_key(key, vel)
            {
                // (the turn moves on even where this zone is not the one whose turn it is)
                self.touched[usize::from(s.group)] = true;
            }
        }
        for (t, turn) in self.touched.iter().zip(self.turns.iter_mut()) {
            if *t {
                *turn = turn.wrapping_add(1);
            }
        }
        self.last_key = Some(key);
    }

    /// Legato: the sounding voices of the held note take the pitch of `key` (and its id), gliding when the glide is on.
    /// False (nothing done) when no voice sounds or a voice's zone does not cover the key
    fn legato_move(&mut self, id: u32, key: u8, vel: u8) -> bool {
        let inst = Arc::clone(&self.instrument);
        let live = |v: &Voice| v.active && !v.dying && !v.released && !v.ignores_off && v.id != 0;
        let mut any = false;
        for v in self.voices.iter().filter(|v| live(v)) {
            if !inst.zones[v.zone as usize].matches_key(key, vel) {
                return false;
            }
            any = true;
        }
        if !any {
            return false;
        }
        let time = self.params.glide;
        for v in self.voices.iter_mut().filter(|v| live(v)) {
            let z = &inst.zones[v.zone as usize];
            let from = (f32::from(v.key) - f32::from(z.root_key)) * z.key_track;
            let to = (f32::from(key) - f32::from(z.root_key)) * z.key_track;
            v.key = key;
            v.id = id;
            v.fresh = true;
            v.cents = to + z.tune;
            if time > 0.0 {
                v.glide += from - to;
                v.glide_rate = v.glide.abs() / time;
            } else {
                v.glide = 0.0;
            }
        }
        true
    }

    fn selected(&self, z: &Zone, key: u8, vel: u8, r: f32, others_held: bool) -> bool {
        if !z.matches_key(key, vel) {
            return false;
        }
        match z.trigger {
            Trigger::Attack => {}
            Trigger::First if others_held => return false,
            Trigger::Legato if !others_held => return false,
            Trigger::First | Trigger::Legato => {}
            Trigger::Release => return false,
        }
        if let Some((lo, hi)) = z.random
            && !(r >= lo && r < hi)
        {
            return false;
        }
        if let Some(p) = z.pedal
            && p != self.sustain
        {
            return false;
        }
        if let Some(s) = z.sequence {
            let len = u32::from(s.length.max(1));
            let turn = self.turns.get(usize::from(s.group)).copied().unwrap_or(0);
            if turn % len + 1 != u32::from(s.position) {
                return false;
            }
        }
        true
    }

    /// Ends the sounding voices whose zone is switched off by `group`
    fn choke(&mut self, group: u32) {
        let inst = &self.instrument;
        for v in self.voices.iter_mut() {
            if v.active && !v.dying && inst.zones[v.zone as usize].off_by == group {
                let z = &inst.zones[v.zone as usize];
                let t = if z.off_fast { FAST_RELEASE } else { z.amp.release.max(FAST_RELEASE) };
                v.amp.release(t, self.rate);
                v.released = true;
                v.dying = z.off_fast;
            }
        }
    }

    /// A slot for a new voice: a free one, or the one of the voice that goes
    fn allocate(&mut self) -> usize {
        let live = self.voices.iter().filter(|v| v.active && !v.dying).count();
        if live >= self.params.polyphony {
            // The quietest of the releasing ones; else the oldest
            let victim = self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.active && !v.dying)
                .min_by(|(_, a), (_, b)| {
                    let key = |v: &Voice| (!v.released, if v.released { v.amp.level() } else { 0.0 }, v.order);
                    let (ka, kb) = (key(a), key(b));
                    ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1)).then(ka.2.cmp(&kb.2))
                })
                .map(|(i, _)| i);
            if let Some(i) = victim {
                let v = &mut self.voices[i];
                v.amp.release(FAST_RELEASE, self.rate);
                v.released = true;
                v.dying = true;
            }
        }
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        // No spare slot: the oldest voice is cut
        self.voices.iter().enumerate().min_by_key(|(_, v)| (!v.dying, v.order)).map_or(0, |(i, _)| i)
    }

    fn start_voice(&mut self, index: usize, z: &Zone, id: u32, key: u8, velocity: f32, held_seconds: f32) {
        let slot = self.allocate();
        let frames = z.sample.frames();
        let (start, end, loop_start, loop_end);
        let window = z.windowed;
        if window {
            let (s, e, l) = window_bounds(frames, &self.params.window);
            (start, end, loop_start, loop_end) = (s, e, l, e);
        } else {
            let e = z.end_frame();
            let l_end = if z.loop_end == 0 { e } else { z.loop_end.min(e) };
            (start, end, loop_start, loop_end) = (z.start.min(e), e, z.loop_start, l_end);
        }
        let mut looping = z.loop_mode;
        if looping.is_looping() && loop_end <= loop_start {
            looping = LoopMode::Off;
        }
        let reverse = z.reverse || (window && self.params.reverse);
        let stutter = z.stutter || (window && self.params.stutter);
        let mut at = if reverse { f64::from(end) - 1.0 } else { f64::from(start) };
        if stutter {
            let last = self.stutter_at[index];
            if last >= f64::from(start) && last < f64::from(end) {
                at = last;
            }
        }
        let vel_gain = if let Some(t) = &z.vel_table {
            t[((velocity * 127.0 + 0.5) as usize).min(127)]
        } else {
            let c = match z.vel_curve {
                VelCurve::Linear => velocity,
                VelCurve::Concave => velocity * velocity,
            };
            (1.0 - z.vel_track) + z.vel_track * c
        };
        let release_fade = if held_seconds > 0.0 { db_to_lin(-z.release_decay * held_seconds) } else { 1.0 };
        let amp = match self.params.env {
            Some(o) if o.amount > 0.0 => {
                let mut e = o.envelope;
                e.sustain = (1.0 - o.amount + o.amount * e.sustain).clamp(0.0, 1.0);
                e
            }
            _ => z.amp,
        };
        let glide = match self.last_key {
            Some(prev) if self.params.glide > 0.0 && z.trigger != Trigger::Release => {
                (f32::from(prev) - f32::from(key)) * z.key_track
            }
            _ => 0.0,
        };
        self.order += 1;
        let release_trigger = z.trigger == Trigger::Release;
        let mut v = Voice {
            active: true,
            dying: false,
            zone: index as u32,
            id: if release_trigger { 0 } else { id },
            key,
            order: self.order,
            released: false,
            ignores_off: release_trigger || z.loop_mode == LoopMode::OneShot,
            pos: at,
            dir: if reverse { -1.0 } else { 1.0 },
            start: f64::from(start),
            end: f64::from(end),
            loop_start: f64::from(loop_start),
            loop_end: f64::from(loop_end),
            looping,
            ratio: f64::from(z.sample.rate) / f64::from(self.cfg.rate),
            cents: (f32::from(key) - f32::from(z.root_key)) * z.key_track + z.tune,
            detune: 0.0,
            glide,
            glide_rate: if glide != 0.0 && self.params.glide > 0.0 { glide.abs() / self.params.glide } else { 0.0 },
            pressure: 0.0,
            timbre: 0.5,
            pan: z.pan,
            gain: db_to_lin(z.gain) * vel_gain * release_fade,
            vel: velocity,
            amp: Env::idle(),
            fenv: Env::idle(),
            filter: Biquad::default(),
            has_filter: z.filter.is_some(),
            fade: if z.declick { FADE } else { 0 },
            declick: z.declick,
            output: z.output,
            stutter,
            release_time: amp.release,
            step: 0.0,
            gl: 1.0,
            gr: 1.0,
            k_gain: 1.0,
            fresh: true,
        };
        v.amp.start(&amp, self.rate);
        if let Some(f) = &z.filter {
            v.fenv.start(&f.env, self.rate);
        }
        self.voices[slot] = v;
    }

    /// A note ends
    pub fn note_off(&mut self, id: u32) {
        let Some(i) = self.held.iter().position(|h| h.id == id) else {
            self.release_voices(id);
            return;
        };
        if self.sustain {
            self.held[i].pedal = true;
            return;
        }
        let h = self.held.remove(i);
        let resume = self.mono_resume_target(h.id);
        match resume {
            Some(back) if self.params.legato => {
                // The voice moves back to the note held before, with no new attack
                self.legato_move(back.id, back.key, back.velocity);
                self.last_key = Some(back.key);
            }
            Some(back) => {
                self.finish_note(h);
                self.trigger(back.id, back.key, back.velocity, false);
            }
            None => self.finish_note(h),
        }
    }

    /// Mono: when the note `id` that sounds ends, the note held before it (not on the pedal) comes back
    fn mono_resume_target(&self, id: u32) -> Option<Held> {
        if !self.params.mono || id == 0 {
            return None;
        }
        if !self.voices.iter().any(|v| v.active && v.id == id && !v.released) {
            return None;
        }
        self.held.iter().rev().find(|h| !h.pedal && h.id != id).copied()
    }

    fn finish_note(&mut self, h: Held) {
        self.release_voices(h.id);
        let inst = Arc::clone(&self.instrument);
        let seconds = (self.frame.saturating_sub(h.at)) as f32 / self.rate;
        for (i, z) in inst.zones.iter().enumerate() {
            if z.trigger == Trigger::Release && z.matches_key(h.key, h.velocity) {
                if let Some((lo, hi)) = z.random {
                    // (the release's own draw: a trigger group picks one of its zones per note-off)
                    let r = self.rng.next_f32();
                    if !(r >= lo && r < hi) {
                        continue;
                    }
                }
                self.start_voice(i, z, 0, h.key, f32::from(h.velocity) / 127.0, seconds);
            }
        }
    }

    fn release_voices(&mut self, id: u32) {
        let inst = &self.instrument;
        for v in self.voices.iter_mut() {
            if v.active && v.id == id && id != 0 && !v.released && !v.ignores_off {
                v.amp.release(v.release_time, self.rate);
                v.fenv.release(inst.zones[v.zone as usize].filter.map_or(0.0, |f| f.env.release), self.rate);
                v.released = true;
            }
        }
    }

    /// A note's expression: pan −1 to 1 (added to the zone's), tuning in semitones
    pub fn note_expression(&mut self, id: u32, pan: Option<f32>, tuning: Option<f32>) {
        for v in self.voices.iter_mut().filter(|v| v.active && v.id == id && id != 0) {
            let zone_pan = self.instrument.zones[v.zone as usize].pan;
            if let Some(p) = pan {
                v.pan = (zone_pan + p).clamp(-1.0, 1.0);
            }
            if let Some(t) = tuning {
                v.detune = t * 100.0;
            }
        }
    }

    /// A note's MPE pressure (0 to 1: up to 6 dB louder) and timbre (0 to 1: moves the filter's cutoff by up to two
    /// octaves either way around 0.5; zones without a filter ignore it)
    pub fn note_mpe(&mut self, id: u32, pressure: Option<f32>, timbre: Option<f32>) {
        for v in self.voices.iter_mut().filter(|v| v.active && v.id == id && id != 0) {
            if let Some(p) = pressure {
                v.pressure = p.clamp(0.0, 1.0);
            }
            if let Some(t) = timbre {
                v.timbre = t.clamp(0.0, 1.0);
            }
        }
    }

    /// The pitch bend, −1 to 1
    pub fn pitch_bend(&mut self, bend: f32) {
        self.bend = bend.clamp(-1.0, 1.0);
    }

    /// A control change: the sustain pedal (64), the volume (7) and the expression (11); the all-notes-off (123) and
    /// all-sound-off (120) messages
    pub fn cc(&mut self, number: u8, value: u8) {
        let v = f32::from(value.min(127)) / 127.0;
        match number {
            64 => {
                let down = value >= 64;
                if self.sustain && !down {
                    self.sustain = false;
                    let mut i = 0;
                    while i < self.held.len() {
                        if self.held[i].pedal {
                            let h = self.held.remove(i);
                            self.finish_note(h);
                        } else {
                            i += 1;
                        }
                    }
                }
                self.sustain = down;
            }
            7 => self.volume = v * v,
            11 => self.expression = v * v,
            120 => self.all_sound_off(),
            123 => self.all_notes_release(),
            _ => {}
        }
    }

    /// Every note to its release
    pub fn all_notes_release(&mut self) {
        let inst = &self.instrument;
        for v in self.voices.iter_mut().filter(|v| v.active && !v.released && !v.ignores_off) {
            v.amp.release(v.release_time, self.rate);
            v.fenv.release(inst.zones[v.zone as usize].filter.map_or(0.0, |f| f.env.release), self.rate);
            v.released = true;
        }
        self.held.clear();
        self.sustain = false;
    }

    /// Silence now
    pub fn all_sound_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.active = false;
        }
        self.held.clear();
        self.sustain = false;
    }

    /// Back to rest: nothing sounds and nothing of what played is kept
    pub fn reset(&mut self) {
        self.all_sound_off();
        self.turns.iter_mut().for_each(|t| *t = 0);
        self.stutter_at.iter_mut().for_each(|s| *s = -1.0);
        self.rng = Rng::new(self.cfg.seed);
        self.last_key = None;
        self.bend = 0.0;
        self.volume = 1.0;
        self.expression = 1.0;
        self.order = 0;
        self.frame = 0;
        self.bus.iter_mut().for_each(|b| *b = 0.0);
    }

    // --- rendering --------------------------------------------------------------------------------------------

    /// Renders `frames` (at most the configuration's `max_block`) into the buses
    pub fn render(&mut self, frames: usize) {
        let n = frames.min(self.cfg.max_block.max(1));
        self.bus_frames = n;
        let stride = self.cfg.max_block.max(1);
        for g in 0..self.groups {
            self.bus[2 * g * stride..2 * g * stride + n].fill(0.0);
            self.bus[(2 * g + 1) * stride..(2 * g + 1) * stride + n].fill(0.0);
        }
        let master = db_to_lin(self.params.gain_db) * self.volume * self.expression;
        let inst = Arc::clone(&self.instrument);
        let first_frame = self.frame;
        let Self { voices, bus, table, cfg, params, bend, rate, groups, stutter_at, .. } = self;
        for v in voices.iter_mut().filter(|v| v.active) {
            let zone = &inst.zones[v.zone as usize];
            let group = if cfg.multi { usize::from(v.output).min(*groups - 1) } else { 0 };
            let mono_out = cfg.multi && !inst.outputs.get(group).is_none_or(|o| o.stereo);
            let (lo, hi) = bus.split_at_mut((2 * group + 1) * stride);
            let left = &mut lo[2 * group * stride..2 * group * stride + n];
            let right = &mut hi[..n];
            let mut at = 0;
            while at < n && v.active {
                // The grid is the engine's absolute frame, never the call's start: the host may cut the stream
                // anywhere and the same notes still give the same samples
                let abs = first_frame + at as u64;
                let to_boundary = SUB - (abs % SUB as u64) as usize;
                let m = (n - at).min(to_boundary);
                let update = if v.fresh || to_boundary == SUB { Some(to_boundary) } else { None };
                render_sub(
                    v,
                    zone,
                    cfg.quality,
                    table,
                    params,
                    *bend,
                    *rate,
                    master,
                    mono_out,
                    update,
                    &mut left[at..at + m],
                    &mut right[at..at + m],
                );
                at += m;
            }
            if v.stutter {
                stutter_at[v.zone as usize] = v.pos;
            }
        }
        self.frame += n as u64;
    }
}

/// The per-voice values that change on the grid (see `render_sub`): pitch, filter, pan and gain, and the glide's
/// advance over the `until_next` frames they will serve
fn update_voice(v: &mut Voice, zone: &Zone, params: &Params, bend: f32, rate: f32, master: f32, until_next: usize) {
    let cents = v.cents + v.glide + v.detune + params.tune + bend * params.bend_range * 100.0;
    if v.glide != 0.0 {
        let step = v.glide_rate * until_next as f32 / rate;
        v.glide = if v.glide > 0.0 { (v.glide - step).max(0.0) } else { (v.glide + step).min(0.0) };
    }
    // (a note expression or a host value may be anything: ten octaves either way is more than any pitch needs)
    let cents = if cents.is_finite() { cents.clamp(-12_000.0, 12_000.0) } else { 0.0 };
    v.step = v.ratio * f64::from(libm::exp2f(cents / 1200.0));
    if let Some(f) = &zone.filter {
        let mut c = 1200.0 * libm::log2f((f.cutoff / 440.0).max(1.0e-3)) + 6900.0;
        c += f.key_track * (f32::from(v.key) - 60.0);
        c += f.vel_cents * (1.0 - v.vel);
        c += f.env_depth * v.fenv.level();
        c += (v.timbre - 0.5) * 2.0 * 2400.0;
        let hz = 8.175_798 * libm::exp2f(c / 1200.0);
        v.filter.set(f.kind, hz, f.resonance, rate);
    }
    // The pan: constant power for a mono sample, a balance for a stereo one
    let pan = (v.pan + params.pan).clamp(-1.0, 1.0);
    (v.gl, v.gr) = if zone.sample.is_stereo() {
        ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
    } else {
        let a = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        (libm::cosf(a), libm::sinf(a))
    };
    v.k_gain = v.gain * master * (1.0 + v.pressure);
}

#[allow(clippy::too_many_arguments)]
fn render_sub(
    v: &mut Voice,
    zone: &Zone,
    quality: Quality,
    table: &SincTable,
    params: &Params,
    bend: f32,
    rate: f32,
    master: f32,
    mono_out: bool,
    update: Option<usize>,
    left: &mut [f32],
    right: &mut [f32],
) {
    // The pitch, the filter, the pan and the gain are worked out once per grid cell (and when a voice begins) for the
    // frames up to the next boundary: the same frames whatever the host's block
    if let Some(until_next) = update {
        v.fresh = false;
        update_voice(v, zone, params, bend, rate, master, until_next);
    }
    let (step, gl, gr, gain) = (v.step, v.gl, v.gr, v.k_gain);
    let loop_active = |v: &Voice| match v.looping {
        LoopMode::Forward | LoopMode::PingPong => true,
        LoopMode::UntilRelease => !v.released,
        _ => false,
    };
    for i in 0..left.len() {
        let a = v.amp.next();
        if v.has_filter {
            v.fenv.next();
        }
        if v.amp.is_idle() {
            v.active = false;
            break;
        }
        let active_loop = loop_active(v);
        let wrap = if active_loop {
            Wrap { loop_start: v.loop_start as i64, loop_end: v.loop_end as i64, active: v.dir > 0.0 }
        } else {
            Wrap::NONE
        };
        let mut s = read(&zone.sample, v.pos, quality, wrap, table);
        if v.has_filter {
            s = v.filter.process(s);
        }
        let mut fade = 1.0;
        if v.declick {
            if v.fade > 0 {
                fade = 1.0 - v.fade as f32 / FADE as f32;
                v.fade -= 1;
            }
            let left_frames = if v.dir > 0.0 { v.end - v.pos } else { v.pos - v.start };
            if !active_loop && left_frames < f64::from(FADE) {
                fade *= (left_frames / f64::from(FADE)).clamp(0.0, 1.0) as f32;
            }
        }
        let k = a * gain * fade;
        if mono_out {
            let m = (s[0] * gl + s[1] * gr) * k * std::f32::consts::FRAC_1_SQRT_2;
            left[i] += m;
            right[i] += m;
        } else {
            left[i] += s[0] * gl * k;
            right[i] += s[1] * gr * k;
        }
        // Forward or back
        v.pos += v.dir * step;
        if active_loop {
            let len = v.loop_end - v.loop_start;
            if v.dir > 0.0 && v.pos >= v.loop_end {
                if v.looping == LoopMode::PingPong {
                    v.pos = 2.0 * v.loop_end - v.pos;
                    v.dir = -1.0;
                } else {
                    v.pos -= len * ((v.pos - v.loop_start) / len).floor();
                }
            } else if v.dir < 0.0 && v.pos < v.loop_start {
                if v.looping == LoopMode::PingPong {
                    v.pos = 2.0 * v.loop_start - v.pos;
                    v.dir = 1.0;
                } else {
                    v.pos += len * ((v.loop_end - v.pos) / len).floor().max(1.0);
                }
            }
        }
        if (v.dir > 0.0 && v.pos >= v.end) || (v.dir < 0.0 && v.pos < v.start) {
            v.active = false;
            break;
        }
    }
}

#[cfg(test)]
mod tests;
