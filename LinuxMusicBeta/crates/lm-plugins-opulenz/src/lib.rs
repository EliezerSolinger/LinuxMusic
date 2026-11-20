//! OpulenZ (F-INS-011; legacy `plugins/OpulenZ`): a 2-operator FM synth on an emulated OPL2 (the YM3812 of an AdLib
//! card: adplug's `fmopl.c`, ported in [`opl`]), nine voices on one chip.
//!
//! Its state, version 1 (the legacy saved names in brackets):
//! - per operator `n` (1, the modulator, and 2, the carrier): `op<n>Attack`, `op<n>Decay`, `op<n>Sustain`,
//!   `op<n>Release` (0–15; 14/14/3/10 and 1/3/14/12) [op<n>_a, _d, _s, _r], `op<n>Level` (0–63; 62 and 63) [_lvl],
//!   `op<n>LevelScaling` (0–3, 0) [_scale], `op<n>Multiplier` (0–15; 0 and 1) [_mul], and the switches
//!   `op<n>KeyScalingRate` [_ksr], `op<n>Percussive` [_perc], `op<n>Tremolo` (on for 1) [_trem], `op<n>Vibrato` (on
//!   for 2) [_vib]; field `op<n>Waveform` (`sine`, `half-sine`, `abs-sine`, `quarter-sine`; `sine`) [_waveform];
//! - `feedback` (operator 1's, 0–7, 0) [feedback], the switches `fm` (on: FM, off: additive) [fm], `vibratoDepth`
//!   [vib_depth], `tremoloDepth` [trem_depth], and `volume` (−24–24 dB, +16) [volume].
//!
//! One stream, as the legacy instrument (single-streamed, MIDI-based): a note takes the voice let go the longest ago
//! (a tenth note at once is dropped, as on the chip), and its note-off lets it go; the pitch bend moves every voice,
//! within the range the RPN 0 sets (100 cents until then); key pressure sets a playing note's level; a tuning
//! expression moves a note's pitch. The chip's output, mono, is the sound on both sides, at the volume. Like the
//! legacy synth it has no envelopes, LFOs, filter, chords or arpeggio of the instrument functions (the chip's
//! envelopes are its own).
//!
//! The velocity sets the operators' attenuation in the chip's steps of 0.75 dB, as the legacy did with the track's
//! default base velocity (63): at velocity 1 an operator is at `63 − level · 63 / 127` steps, and a lower velocity
//! adds `20 · log10(1 / velocity) / 0.75` steps (in FM only the carrier's, so the timbre stays). Being in steps, the
//! level follows the velocity within 0.75 dB: [`Velocity::Timbre`].
//!
//! Differences from the legacy plugin, on purpose (decisoes-autonomas.md, "Chips e amplificadores"): a knob change
//! reaches the chip at its sample (the legacy waited for the next period); a note-off lets go of its own note only
//! (the legacy let go of every voice playing the same key); a change of the tremolo or vibrato depth is heard (see
//! [`opl`]); every project's velocity is proportional (the legacy kept the older law for projects saved before it).

#![forbid(unsafe_code)]

pub mod opl;

use std::any::Any;

use lm_base::Result;
use lm_dsp::math::dbfs_to_amp;
use lm_plugin::instrument::GRID;
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Adopted, EventKind, Expression, NoteId, ProcessContext, Processor};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::opl::Opl;

pub const ID: &str = "opulenz";
/// The chip's voices
pub const VOICES: usize = 9;
/// The YM3812's clock on an AdLib card
pub const CLOCK: i32 = 3_579_545;
/// The base velocity the legacy velocity law took (the track's default, `MidiMaxVelocity / 2`)
pub const BASE_VELOCITY: f64 = 63.0;

/// The parameters, in the schema's order
pub const KEYS: [&str; PARAMS] = [
    "op1Attack",
    "op1Decay",
    "op1Sustain",
    "op1Release",
    "op1Level",
    "op1LevelScaling",
    "op1Multiplier",
    "feedback",
    "op1KeyScalingRate",
    "op1Percussive",
    "op1Tremolo",
    "op1Vibrato",
    "op2Attack",
    "op2Decay",
    "op2Sustain",
    "op2Release",
    "op2Level",
    "op2LevelScaling",
    "op2Multiplier",
    "op2KeyScalingRate",
    "op2Percussive",
    "op2Tremolo",
    "op2Vibrato",
    "fm",
    "vibratoDepth",
    "tremoloDepth",
    "volume",
];
const PARAMS: usize = 27;
const FEEDBACK: usize = 7;
const OP1_KSR: usize = 8;
const OP1_PERC: usize = 9;
const OP1_TREM: usize = 10;
const OP1_VIB: usize = 11;
const OP2: usize = 12;
const FM: usize = 23;
const VIB_DEPTH: usize = 24;
const TREM_DEPTH: usize = 25;
const VOLUME: usize = 26;

/// The waveforms of the wave selector, in its order
pub const WAVEFORM_KEYS: [&str; 4] = ["sine", "half-sine", "abs-sine", "quarter-sine"];
const WAVEFORM_FIELDS: [&str; 2] = ["op1Waveform", "op2Waveform"];

/// The register offsets of each voice's operators ("weird ordering of voice parameters")
const ADLIB_OPADD: [i32; VOICES] = [0x00, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x10, 0x11, 0x12];
const VOICE_FREE: u8 = 128;
const NO_VOICE: u8 = 255;

/// The descriptor
pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.opulenz.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    velocity: Velocity::Timbre,
    display: None,
    build,
    tests: TestHints {
        tuning: Some(TuningProbe { state: probe_state, key: 69, hz: PROBE_HZ }),
        no_adopt: &[],
        voices: VOICES,
        max_level: 1.0e3,
        // The chip's release at the defaults (the carrier's rate 3): 9.8 s from the sustain to silence
        tail: 10.0,
    },
};

/// What A4 sounds at: its F-number at block 4, `440 · 2^16 / 49716` truncated = 580, played at `580 · 49716 / 2^16`
/// Hz (the chip's frequency unit of 49716 Hz / 2^20 per step)
pub const PROBE_HZ: f64 = 580.0 * 49_716.0 / 65_536.0;

/// A plain sine: the carrier alone, multiplier 1, no tremolo or vibrato, sustained
fn probe_state() -> Value {
    let real = |v: f64| Value::real(v).unwrap_or(Value::NULL);
    let params = Value::empty_object()
        .with("fm", real(0.0))
        .with("op1Level", real(0.0))
        .with("op1Tremolo", real(0.0))
        .with("op2Vibrato", real(0.0))
        .with("op2Sustain", real(15.0));
    Value::empty_object().with(lm_state::PARAMS_KEY, params)
}

fn schemas() -> Vec<NodeSchema> {
    let stepped = |key: &'static str, max: f64, def: f64| ParamSpec::new(key, 0.0, max, def).scale(Scale::Stepped);
    let mut s = NodeSchema::new("plugin:opulenz", 1);
    let defaults = [[14.0, 14.0, 3.0, 10.0, 62.0, 0.0, 0.0], [1.0, 3.0, 14.0, 12.0, 63.0, 0.0, 1.0]];
    for (op, d) in defaults.iter().enumerate() {
        let base = if op == 0 { 0 } else { OP2 };
        s = s
            .param(stepped(KEYS[base], 15.0, d[0]))
            .param(stepped(KEYS[base + 1], 15.0, d[1]))
            .param(stepped(KEYS[base + 2], 15.0, d[2]))
            .param(stepped(KEYS[base + 3], 15.0, d[3]))
            .param(stepped(KEYS[base + 4], 63.0, d[4]))
            .param(stepped(KEYS[base + 5], 3.0, d[5]))
            .param(stepped(KEYS[base + 6], 15.0, d[6]));
        if op == 0 {
            s = s.param(stepped(KEYS[FEEDBACK], 7.0, 0.0));
        }
        let switches = if op == 0 { [0.0, 0.0, 1.0, 0.0] } else { [0.0, 0.0, 0.0, 1.0] };
        let first = if op == 0 { OP1_KSR } else { OP2 + 7 };
        for (i, def) in switches.iter().enumerate() {
            s = s.param(stepped(KEYS[first + i], 1.0, *def));
        }
        s = s.field(FieldSpec::choice(WAVEFORM_FIELDS[op], &WAVEFORM_KEYS, "sine"));
    }
    s = s
        .param(stepped(KEYS[FM], 1.0, 1.0))
        .param(stepped(KEYS[VIB_DEPTH], 1.0, 0.0))
        .param(stepped(KEYS[TREM_DEPTH], 1.0, 0.0))
        .param(ParamSpec::new(KEYS[VOLUME], -24.0, 24.0, 16.0).unit("dB"));
    vec![s]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() }
}

/// The parameter index of operator 2's knob `k`: attack, decay, sustain, release, level, scaling, multiplier (0–6),
/// then the switches key scaling rate, percussive, tremolo, vibrato (7–10)
fn op2(k: usize) -> usize {
    OP2 + k
}

/// The voices' notes and the chip's MIDI state (`OpulenzInstrument`'s)
#[derive(Clone, Copy, Debug)]
struct Voices {
    /// Each voice's key, with [`VOICE_FREE`] once let go
    note: [u8; VOICES],
    /// Each voice's note
    id: [Option<NoteId>; VOICES],
    /// Each voice's tuning expression, semitones
    tuning: [f32; VOICES],
    /// Free voices, the one let go the longest ago first; [`NO_VOICE`] after the last
    lru: [u8; VOICES],
    /// Each playing voice's velocity (0 to 1)
    velocity: [f32; VOICES],
    /// The bend, cents
    pitchbend: i32,
    /// Its range, cents (RPN 0)
    bend_range: i32,
    rpn_coarse: u8,
    rpn_fine: u8,
    fnums: [i32; 128],
}

impl Voices {
    fn new() -> Self {
        let mut v = Self {
            note: [VOICE_FREE; VOICES],
            id: [None; VOICES],
            tuning: [0.0; VOICES],
            lru: std::array::from_fn(|i| i as u8),
            velocity: [0.0; VOICES],
            pitchbend: 0,
            bend_range: 100,
            rpn_coarse: 255,
            rpn_fine: 255,
            fnums: [0; 128],
        };
        v.tune_equal(69, 440.0);
        v
    }

    /// `tuneEqual`
    fn tune_equal(&mut self, center: i32, hz: f32) {
        for n in 0..128i32 {
            let tmp = hz * libm::exp2f((n - center) as f32 / 12.0 + self.pitchbend as f32 / 1200.0);
            self.fnums[n as usize] = hz2fnum(tmp);
        }
    }

    /// The F-number and block of voice `v`'s note
    fn fnum(&self, v: usize) -> i32 {
        let key = usize::from(self.note[v] & !VOICE_FREE);
        if self.tuning[v] == 0.0 {
            return self.fnums[key];
        }
        let hz =
            440.0 * libm::exp2f((key as f32 - 69.0) / 12.0 + self.pitchbend as f32 / 1200.0 + self.tuning[v] / 12.0);
        hz2fnum(hz)
    }

    /// `popVoice`
    fn pop(&mut self) -> u8 {
        let first = self.lru[0];
        self.lru.copy_within(1.., 0);
        self.lru[VOICES - 1] = NO_VOICE;
        first
    }

    /// `pushVoice`
    fn push(&mut self, v: u8) {
        let mut i = VOICES - 1;
        while i > 0 {
            if self.lru[i - 1] != NO_VOICE {
                break;
            }
            i -= 1;
        }
        self.lru[i] = v;
    }
}

/// `Hz2fnum`: the F-number in the lowest block that holds it
fn hz2fnum(hz: f32) -> i32 {
    for block in 0..8 {
        let fnum = (hz * libm::exp2f(20.0 - block as f32) / 49716.0) as u32;
        if fnum < 1023 {
            return fnum as i32 + (block << 10);
        }
    }
    0
}

/// The instrument
#[derive(Debug)]
pub struct OpulenZ {
    chip: Opl,
    voices: Voices,
    knobs: [f32; PARAMS],
    waveforms: [usize; 2],
    patch_dirty: bool,
    buf: [i16; GRID],
    /// The chip as it was before a change of waveform, played on for [`CROSSFADE_MS`] while the new one fades in
    ghost: Opl,
    ghost_buf: [i16; GRID],
    fade_left: u32,
    fade_len: u32,
    /// Note-offs of this block whose note-on has not come yet (a note of no length: its note-off sorts first at the
    /// same offset); the note-on then lets go at once
    early_offs: [Option<NoteId>; EARLY_OFFS],
}

const EARLY_OFFS: usize = 32;

/// The crossfade of a rebuild that changes the patch (as the instrument functions' crossfade)
pub const CROSSFADE_MS: f32 = 10.0;

fn on(v: f32) -> bool {
    v >= 0.5
}

impl OpulenZ {
    fn knob(&self, k: usize) -> f32 {
        self.knobs[k]
    }

    /// The knob `k` of operator `op` (0 or 1)
    fn op(&self, op: usize, k: usize) -> f32 {
        if op == 0 { self.knobs[k] } else { self.knobs[op2(k)] }
    }

    /// `setVoiceVelocity`, the legacy's proportional law at its base velocity
    fn set_voice_velocity(&mut self, voice: usize, velocity: f32) {
        let attenuation = |level: f32| -> i32 {
            if velocity <= 0.0 {
                return 63;
            }
            let steps =
                63.0 - f64::from(level) * BASE_VELOCITY / 127.0 + 20.0 * libm::log10(1.0 / f64::from(velocity)) / 0.75;
            libm::round(steps.clamp(0.0, 63.0)) as i32
        };
        let op1 = if !on(self.knob(FM)) { attenuation(self.op(0, 4)) } else { 63 - self.op(0, 4) as i32 };
        let scale1 = (self.op(0, 5) as i32) & 0x03;
        self.chip.write(0x40 + ADLIB_OPADD[voice], (scale1 << 6) + (op1 & 0x3f));
        let op2 = attenuation(self.op(1, 4));
        let scale2 = (self.op(1, 5) as i32) & 0x03;
        self.chip.write(0x43 + ADLIB_OPADD[voice], (scale2 << 6) + (op2 & 0x3f));
    }

    /// `updatePatch`: the knobs to every voice
    fn update_patch(&mut self) {
        let reg20 = |o: &Self, op: usize| -> i32 {
            let (ksr, perc, trem, vib) = if op == 0 {
                (o.knob(OP1_KSR), o.knob(OP1_PERC), o.knob(OP1_TREM), o.knob(OP1_VIB))
            } else {
                (o.knob(op2(7)), o.knob(op2(8)), o.knob(op2(9)), o.knob(op2(10)))
            };
            (if on(trem) { 128 } else { 0 })
                + (if on(vib) { 64 } else { 0 })
                + (if on(perc) { 0 } else { 32 })
                + (if on(ksr) { 16 } else { 0 })
                + ((o.op(op, 6) as i32) & 0x0f)
        };
        let ad =
            |o: &Self, op: usize| ((15 - ((o.op(op, 0) as i32) & 0x0f)) << 4) + (15 - ((o.op(op, 1) as i32) & 0x0f));
        let sr =
            |o: &Self, op: usize| ((15 - ((o.op(op, 2) as i32) & 0x0f)) << 4) + (15 - ((o.op(op, 3) as i32) & 0x0f));
        let inst = [
            reg20(self, 0),
            reg20(self, 1),
            ad(self, 0),
            ad(self, 1),
            sr(self, 0),
            sr(self, 1),
            (self.waveforms[0] & 0x03) as i32,
            (self.waveforms[1] & 0x03) as i32,
            (if on(self.knob(FM)) { 0 } else { 1 }) + (((self.knob(FEEDBACK) as i32) & 0x07) << 1),
        ];
        // Not part of the per-voice patch
        let depth = (if on(self.knob(TREM_DEPTH)) { 128 } else { 0 }) + (if on(self.knob(VIB_DEPTH)) { 64 } else { 0 });
        self.chip.write(0xBD, depth);
        // the level knobs might have changed
        for voice in 0..VOICES {
            if self.voices.note[voice] & VOICE_FREE == 0 {
                self.set_voice_velocity(voice, self.voices.velocity[voice]);
            }
        }
        // loadPatch
        for (v, &op) in ADLIB_OPADD.iter().enumerate() {
            self.chip.write(0x20 + op, inst[0]);
            self.chip.write(0x23 + op, inst[1]);
            self.chip.write(0x60 + op, inst[2]);
            self.chip.write(0x63 + op, inst[3]);
            self.chip.write(0x80 + op, inst[4]);
            self.chip.write(0x83 + op, inst[5]);
            self.chip.write(0xe0 + op, inst[6]);
            self.chip.write(0xe3 + op, inst[7]);
            self.chip.write(0xc0 + v as i32, inst[8]);
        }
        self.patch_dirty = false;
    }

    /// The chip from rest, every voice free, then the patch (`init`, the wave selector on, `updatePatch`)
    fn restart(&mut self) {
        self.chip.reset();
        self.chip.write(0x01, 0x20);
        let (pitchbend, range, coarse, fine) =
            (self.voices.pitchbend, self.voices.bend_range, self.voices.rpn_coarse, self.voices.rpn_fine);
        self.voices = Voices::new();
        self.voices.pitchbend = pitchbend;
        self.voices.bend_range = range;
        self.voices.rpn_coarse = coarse;
        self.voices.rpn_fine = fine;
        self.voices.tune_equal(69, 440.0);
        self.update_patch();
    }

    fn write_frequency(&mut self, v: usize, key_on: bool) {
        let fnum = self.voices.fnum(v);
        self.chip.write(0xA0 + v as i32, fnum & 0xff);
        self.chip.write(0xB0 + v as i32, if key_on { 32 } else { 0 } + ((fnum & 0x1f00) >> 8));
    }

    fn note_on(&mut self, id: NoteId, key: u8, velocity: f32) {
        let voice = self.voices.pop();
        if voice == NO_VOICE {
            return;
        }
        let v = usize::from(voice);
        self.voices.note[v] = key.min(127);
        self.voices.id[v] = Some(id);
        self.voices.tuning[v] = 0.0;
        // NB: the frequencies are by voice number, not by the operator offsets
        self.write_frequency(v, true);
        self.set_voice_velocity(v, velocity);
        self.voices.velocity[v] = velocity;
    }

    fn note_off(&mut self, v: usize) {
        self.write_frequency(v, false);
        self.voices.note[v] |= VOICE_FREE;
        self.voices.id[v] = None;
        self.voices.velocity[v] = 0.0;
        self.voices.push(v as u8);
    }

    fn voice_of(&self, id: NoteId) -> Option<usize> {
        (0..VOICES).find(|&v| self.voices.id[v] == Some(id) && self.voices.note[v] & VOICE_FREE == 0)
    }

    fn midi(&mut self, bytes: [u8; 3]) {
        match bytes[0] & 0xf0 {
            0xA0 => {
                // key pressure: a playing note's level
                let velocity = f32::from(bytes[2]) / BASE_VELOCITY as f32;
                for v in 0..VOICES {
                    if self.voices.note[v] == bytes[1] {
                        self.voices.velocity[v] = velocity;
                        self.set_voice_velocity(v, velocity);
                    }
                }
            }
            0xE0 => {
                // neutral 8192, full down 0, full up 16383
                let value = i32::from(bytes[1] & 0x7f) | (i32::from(bytes[2] & 0x7f) << 7);
                let bend = (value - 8192) * self.voices.bend_range / 8192;
                if bend != self.voices.pitchbend {
                    self.voices.pitchbend = bend;
                    self.voices.tune_equal(69, 440.0);
                }
                // every voice, the released ones too
                for v in 0..VOICES {
                    let playing = self.voices.note[v] & VOICE_FREE == 0;
                    self.write_frequency(v, playing);
                }
            }
            0xB0 => match bytes[1] {
                0x64 => self.voices.rpn_fine = bytes[2],
                0x65 => self.voices.rpn_coarse = bytes[2],
                0x06 if (u32::from(self.voices.rpn_coarse) << 8) + u32::from(self.voices.rpn_fine) == 0 => {
                    self.voices.bend_range = i32::from(bytes[2]) * 100;
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn event(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn { note, key, velocity, .. } => {
                if self.patch_dirty {
                    self.update_patch();
                }
                self.note_on(note, key, velocity);
                if let Some(slot) = self.early_offs.iter_mut().find(|s| **s == Some(note)) {
                    *slot = None;
                    if let Some(v) = self.voice_of(note) {
                        self.note_off(v);
                    }
                }
            }
            EventKind::NoteOff { note, .. } => {
                if let Some(v) = self.voice_of(note) {
                    self.note_off(v);
                } else if let Some(slot) = self.early_offs.iter_mut().find(|s| s.is_none()) {
                    *slot = Some(note);
                }
            }
            EventKind::Param { index, value } => {
                if let Some(k) = self.knobs.get_mut(index as usize)
                    && *k != value
                {
                    *k = value;
                    // (the volume is read when rendering; the patch is written again only when it changed, as the
                    // legacy did: a rewrite is not nothing on the chip, a sustained envelope starts its decay again)
                    if index as usize != VOLUME {
                        self.patch_dirty = true;
                    }
                }
            }
            EventKind::NoteExpression { note, expression: Expression::Tuning, value, .. } => {
                if let Some(v) = self.voice_of(note) {
                    self.voices.tuning[v] = value;
                    self.write_frequency(v, true);
                }
            }
            EventKind::NoteExpression { .. } => {}
            EventKind::Midi { bytes } => self.midi(bytes),
            EventKind::AllNotesRelease => {
                for v in 0..VOICES {
                    if self.voices.note[v] & VOICE_FREE == 0 {
                        self.note_off(v);
                    }
                }
            }
            EventKind::AllSoundOff => self.restart(),
        }
    }
}

impl Processor for OpulenZ {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        let events = cx.events;
        let origin = cx.transport.frame;
        let mut e = 0;
        let mut at = 0;
        while at < frames {
            while e < events.len() && (events[e].offset as usize) <= at {
                self.event(events[e].kind);
                e += 1;
            }
            if self.patch_dirty {
                self.update_patch();
            }
            let grid = GRID - ((origin + at as u64) % GRID as u64) as usize;
            let mut end = (at + grid).min(frames);
            if e < events.len() {
                end = end.min(events[e].offset as usize).max(at + 1);
            }
            let n = end - at;
            let mut buf = self.buf;
            self.chip.update(&mut buf[..n]);
            let mut ghost = self.ghost_buf;
            let fading = self.fade_left > 0;
            if fading {
                self.ghost.update(&mut ghost[..n]);
            }
            // the chip's 16-bit output, at the volume
            let gain = dbfs_to_amp(self.knob(VOLUME)) / 8192.0;
            let (_, mut outs, _) = cx.buffers.split();
            let (l, r) = outs.pair_mut(0, 1);
            for (i, s) in buf[..n].iter().enumerate() {
                let mut x = f32::from(*s) * gain;
                if fading {
                    let g = f32::from(ghost[i]) * gain;
                    let w =
                        (1.0 - self.fade_left.saturating_sub(i as u32) as f32 / self.fade_len as f32).clamp(0.0, 1.0);
                    x = g + (x - g) * w;
                }
                l[at + i] = x;
                r[at + i] = x;
            }
            self.fade_left = self.fade_left.saturating_sub(n as u32);
            at = end;
        }
        for ev in &events[e..] {
            self.event(ev.kind);
        }
        self.early_offs = [None; EARLY_OFFS];
    }

    fn reset(&mut self) {
        self.restart();
    }

    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        let Some(old) = old.as_any_mut().downcast_mut::<Self>() else { return Adopted::No };
        // The chip and its voices go on, with the new knobs and waveforms
        std::mem::swap(&mut self.chip, &mut old.chip);
        self.voices = old.voices;
        if self.knobs[..VOLUME] != old.knobs[..VOLUME] || self.waveforms != old.waveforms || old.patch_dirty {
            // The old patch plays on while the new one fades in (no click: a waveform jumps)
            self.ghost.clone_from(&self.chip);
            self.fade_left = self.fade_len;
            self.update_patch();
        }
        Adopted::Yes
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let mut knobs = [0.0f32; PARAMS];
    for (k, key) in knobs.iter_mut().zip(&KEYS[..PARAMS]) {
        *k = s.param_f32(key)?;
    }
    let mut waveforms = [0usize; 2];
    for (w, key) in waveforms.iter_mut().zip(WAVEFORM_FIELDS) {
        *w = s.choice(key, |k| WAVEFORM_KEYS.iter().position(|w| *w == k))?;
    }
    let rate = i32::try_from(cx.spec.sample_rate).unwrap_or(i32::MAX);
    let mut synth = OpulenZ {
        chip: Opl::new(CLOCK, rate),
        voices: Voices::new(),
        knobs,
        waveforms,
        patch_dirty: false,
        buf: [0; GRID],
        ghost: Opl::new(CLOCK, rate),
        ghost_buf: [0; GRID],
        fade_left: 0,
        early_offs: [None; EARLY_OFFS],
        fade_len: (CROSSFADE_MS / 1000.0 * cx.spec.sample_rate as f32).ceil() as u32,
    };
    synth.restart();
    Ok(Box::new(synth))
}
