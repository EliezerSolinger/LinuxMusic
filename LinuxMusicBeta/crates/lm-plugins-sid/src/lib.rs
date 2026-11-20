//! Sid (F-INS-016; legacy `plugins/Sid`): the Commodore 64's SID chip, a MOS6581 or a MOS8580 emulated by reSID
//! (`lm-sys-resid`), one chip per note, its three voices playing the note, on the instrument functions.
//!
//! Its state, version 1:
//! - per voice `n` (1 to 3), parameters `pulseWidth<n>` (0–4095, 2048), `attack<n>` (0–15, 8), `decay<n>` (0–15,
//!   8), `sustain<n>` (0–15, 15), `release<n>` (0–15, 8), `coarse<n>` (−24–24 semitones, 0), and the switches
//!   `sync<n>`, `ringMod<n>`, `filtered<n>`, `test<n>` (stepped, 0–1, off); field `wave<n>` (`square`, `triangle`,
//!   `saw`, `noise`; `triangle`);
//! - parameters `filterCutoff` (0–2047, 1024), `filterResonance` (0–15, 8), `voice3Off` (switch, off), `volume`
//!   (0–15, 15); fields `filterMode` (`highpass`, `bandpass`, `lowpass`; `lowpass`) and `chipModel` (`mos6581`,
//!   `mos8580`; `mos8580`);
//! - the instrument functions (`lm_plugin::instrument`).
//!
//! # How a note plays
//!
//! As the legacy `SidInstrument::playNoteImpl`: a fresh chip per note (reSID's `SAMPLE_FAST` at the project's rate
//! from the PAL clock, the filter on), its 25 registers worked out from the knobs and the note's pitch, and written in
//! the legacy order and timing (`sid_fillbuffer`: 9 cycles after each write, 4 more before each control register),
//! the chip clocked between the writes, then until the samples are made. Its 16-bit output is the sound, the same on
//! both sides.
//!
//! The legacy plugin wrote the 25 registers at the start of every period of its engine: here at every [`TICK`] of
//! the note (the legacy's default period, counted from the note's start, so a note sounds the same at any block
//! size), and at once when one of them changes (a knob, the pitch, the gate: the legacy waited for the next
//! period). Two legacy behaviours are not kept, on purpose (decisoes-autonomas.md, "Chips e amplificadores"):
//! - the legacy set the chip model at every period, and reSID clears the filter's integrators when it does: the
//!   filtered sound restarted from zero every period (a buzz at the period's rate). Here the model is set when it
//!   changes;
//! - the legacy asked for `relTime[release]` **frames** of release (the table is in milliseconds: at 48 kHz the
//!   SID's release of 300 ms was cut after 6 ms). Here the release lasts its milliseconds.
//!
//! A write sequence that does not fit in a piece (at 192 kHz a piece of 32 samples is 164 cycles; the sequence takes
//! 237) goes on in the next piece where it stopped, cycle for cycle: on the MOS8580 a write lands on the next cycle
//! clocked, and the legacy dropped the writes past the period's last sample.

#![forbid(unsafe_code)]

use lm_base::{Error, Result};
use lm_plugin::instrument::{Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};
use lm_sys_resid::{ChipModel, PAL_CLOCK, REGISTERS, Sid};

pub const ID: &str = "sid";
/// Notes sounding at once (one chip each; more steal, U27)
pub const VOICES: usize = 32;
const SID_VOICES: usize = 3;
/// Parameters per voice
const PER_VOICE: usize = 10;
const PULSE_WIDTH: usize = 0;
const ATTACK: usize = 1;
const DECAY: usize = 2;
const SUSTAIN: usize = 3;
const RELEASE: usize = 4;
const COARSE: usize = 5;
const SYNC: usize = 6;
const RING_MOD: usize = 7;
const FILTERED: usize = 8;
const TEST: usize = 9;
/// The chip's parameters, after the voices'
const CUTOFF: usize = SID_VOICES * PER_VOICE;
const RESONANCE: usize = CUTOFF + 1;
const VOICE3_OFF: usize = CUTOFF + 2;
const VOLUME: usize = CUTOFF + 3;
const PARAMS: usize = CUTOFF + 4;

const VOICE_KEYS: [[&str; PER_VOICE]; SID_VOICES] = [
    ["pulseWidth1", "attack1", "decay1", "sustain1", "release1", "coarse1", "sync1", "ringMod1", "filtered1", "test1"],
    ["pulseWidth2", "attack2", "decay2", "sustain2", "release2", "coarse2", "sync2", "ringMod2", "filtered2", "test2"],
    ["pulseWidth3", "attack3", "decay3", "sustain3", "release3", "coarse3", "sync3", "ringMod3", "filtered3", "test3"],
];
const CHIP_KEYS: [&str; 4] = ["filterCutoff", "filterResonance", "voice3Off", "volume"];
const WAVE_FIELDS: [&str; SID_VOICES] = ["wave1", "wave2", "wave3"];

/// A voice's waveform (the legacy `VoiceObject::WaveForm`, in its order)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Square,
    Triangle,
    Saw,
    Noise,
}

impl Wave {
    pub const ALL: [Wave; 4] = [Wave::Square, Wave::Triangle, Wave::Saw, Wave::Noise];
    pub const KEYS: [&'static str; 4] = ["square", "triangle", "saw", "noise"];

    pub fn key(self) -> &'static str {
        Self::KEYS[self as usize]
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::KEYS.iter().position(|k| *k == key).map(|i| Self::ALL[i])
    }

    /// Its bits in the control register
    fn bits(self) -> u8 {
        match self {
            Wave::Noise => 128,
            Wave::Square => 64,
            Wave::Saw => 32,
            Wave::Triangle => 16,
        }
    }
}

/// The filter's mode (the legacy `SidInstrument::FilterType`, in its order)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    HighPass,
    BandPass,
    LowPass,
}

impl FilterMode {
    pub const ALL: [FilterMode; 3] = [FilterMode::HighPass, FilterMode::BandPass, FilterMode::LowPass];
    pub const KEYS: [&'static str; 3] = ["highpass", "bandpass", "lowpass"];

    pub fn key(self) -> &'static str {
        Self::KEYS[self as usize]
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::KEYS.iter().position(|k| *k == key).map(|i| Self::ALL[i])
    }

    fn bits(self) -> u8 {
        match self {
            FilterMode::LowPass => 16,
            FilterMode::BandPass => 32,
            FilterMode::HighPass => 64,
        }
    }
}

const CHIP_MODEL_KEYS: [&str; 2] = ["mos6581", "mos8580"];

fn chip_model(key: &str) -> Option<ChipModel> {
    match key {
        "mos6581" => Some(ChipModel::Mos6581),
        "mos8580" => Some(ChipModel::Mos8580),
        _ => None,
    }
}

/// The order the legacy plugin wrote the registers in (`sidorder`): the filter, then each voice's
pub const WRITE_ORDER: [u8; REGISTERS] = [
    0x15, 0x16, 0x18, 0x17, 0x05, 0x06, 0x02, 0x03, 0x00, 0x01, 0x04, 0x0c, 0x0d, 0x09, 0x0a, 0x07, 0x08, 0x0b, 0x13,
    0x14, 0x10, 0x11, 0x0e, 0x0f, 0x12,
];
/// Cycles after each write (`lda $xxxx,x` 4 cycles, `sta $d400,x` 5)
pub const WRITE_DELAY: i32 = 9;
/// Cycles before writing a control register (`and $xxxx,x`)
pub const WAVE_DELAY: i32 = 4;

/// The player's tick, frames: the 25 registers are written again every tick, as the legacy plugin wrote them at the
/// start of every period of its engine (256 frames by default), counted from the note's start. On the chip a write
/// is not always nothing when the value is the same: a write to a control register recomputes the waveform's output
/// (and moves the MOS8580's half-cycle pipeline of the triangle and the saw), as a C64 player writing every frame
/// does
pub const TICK: usize = 256;

/// The release of each value of the release knob, ms (the legacy `relTime`)
pub const RELEASE_MS: [f32; 16] =
    [6.0, 24.0, 48.0, 72.0, 114.0, 168.0, 204.0, 240.0, 300.0, 750.0, 1500.0, 2400.0, 3000.0, 9000.0, 15000.0, 24000.0];

/// The descriptor
pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.sid.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    velocity: Velocity::Level,
    display: None,
    build,
    tests: TestHints {
        // The defaults: three triangles at the key (the chip's frequency register truncates: 0.1 cent flat at A4)
        tuning: Some(TuningProbe { state: Value::empty_object, key: 69, hz: 440.0 }),
        no_adopt: &[],
        voices: VOICES,
        max_level: 1.0e3,
        tail: lm_plugin::DEFAULT_TAIL,
    },
};

fn schemas() -> Vec<NodeSchema> {
    let mut s = NodeSchema::new("plugin:sid", 1);
    let switch = |key: &'static str| ParamSpec::new(key, 0.0, 1.0, 0.0).scale(Scale::Stepped);
    for (i, keys) in VOICE_KEYS.iter().enumerate() {
        s = s
            .param(ParamSpec::new(keys[PULSE_WIDTH], 0.0, 4095.0, 2048.0).scale(Scale::Stepped))
            .param(ParamSpec::new(keys[ATTACK], 0.0, 15.0, 8.0).scale(Scale::Stepped))
            .param(ParamSpec::new(keys[DECAY], 0.0, 15.0, 8.0).scale(Scale::Stepped))
            .param(ParamSpec::new(keys[SUSTAIN], 0.0, 15.0, 15.0).scale(Scale::Stepped))
            .param(ParamSpec::new(keys[RELEASE], 0.0, 15.0, 8.0).scale(Scale::Stepped))
            .param(ParamSpec::new(keys[COARSE], -24.0, 24.0, 0.0).scale(Scale::Stepped).unit("st"))
            .param(switch(keys[SYNC]))
            .param(switch(keys[RING_MOD]))
            .param(switch(keys[FILTERED]))
            .param(switch(keys[TEST]));
        s = s.field(FieldSpec::choice(WAVE_FIELDS[i], &Wave::KEYS, "triangle"));
    }
    s = s
        .param(ParamSpec::new(CHIP_KEYS[0], 0.0, 2047.0, 1024.0).scale(Scale::Stepped))
        .param(ParamSpec::new(CHIP_KEYS[1], 0.0, 15.0, 8.0).scale(Scale::Stepped))
        .param(switch(CHIP_KEYS[2]))
        .param(ParamSpec::new(CHIP_KEYS[3], 0.0, 15.0, 15.0).scale(Scale::Stepped))
        .field(FieldSpec::choice("filterMode", &FilterMode::KEYS, "lowpass"))
        .field(FieldSpec::choice("chipModel", &CHIP_MODEL_KEYS, "mos8580"));
    vec![add_instrument_functions(s)]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() }
}

/// The synth: the knobs and the choices, and a fresh chip every note starts from
#[derive(Clone, Debug)]
pub struct SidSynth {
    knobs: [f32; PARAMS],
    waves: [Wave; SID_VOICES],
    filter_mode: FilterMode,
    model: ChipModel,
    /// A chip as the legacy plugin made one for each note (made in the build: a note copies it, never allocates)
    fresh: Sid,
}

/// Where a register write sequence is (`sid_fillbuffer`'s loop): the register about to be written and the cycles
/// still to clock before or after it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sequence {
    /// Index in [`WRITE_ORDER`]; `REGISTERS` when every register is written
    next: usize,
    /// Cycles to clock before the next step
    pending: i32,
    /// Whether the wave delay of `next` is clocked already
    waited: bool,
}

/// One note: its chip and what was written to it
#[derive(Debug)]
pub struct Voice {
    sid: Sid,
    /// The registers as last written (none before the first sequence)
    written: Option<[u8; REGISTERS]>,
    sequence: Option<Sequence>,
    model: Option<ChipModel>,
    released: bool,
    /// Frames to the player's next tick (0: a tick now)
    until_tick: usize,
    samples: [i16; SCRATCH],
}

/// Samples made per call to the chip
const SCRATCH: usize = 64;

impl Clone for Voice {
    /// A new chip (allocates: build time only)
    fn clone(&self) -> Self {
        Self {
            sid: self.sid.clone(),
            written: self.written,
            sequence: self.sequence,
            model: self.model,
            released: self.released,
            until_tick: self.until_tick,
            samples: self.samples,
        }
    }

    /// In place, without allocating (the instrument's crossfade, on the audio thread)
    fn clone_from(&mut self, source: &Self) {
        self.sid.clone_from(&source.sid);
        self.written = source.written;
        self.sequence = source.sequence;
        self.model = source.model;
        self.released = source.released;
        self.until_tick = source.until_tick;
    }
}

impl SidSynth {
    fn knob(&self, voice: usize, k: usize) -> f32 {
        self.knobs[voice * PER_VOICE + k]
    }

    fn on(&self, voice: usize, k: usize) -> bool {
        self.knob(voice, k) >= 0.5
    }

    /// The registers for a note at `freq` Hz, gate on or off (`playNoteImpl`, computed as it computes them)
    pub fn registers(&self, freq: f32, gate: bool) -> [u8; REGISTERS] {
        let mut reg = [0u8; REGISTERS];
        for v in 0..SID_VOICES {
            let base = v * 7;
            // freq (Fn = Fout / Fclk * 16777216) + coarse detuning, in the legacy's mix of float and double
            let mut note = (69.0 + 12.0 * libm::log2(f64::from(freq) / 440.0)) as f32;
            note += self.knob(v, COARSE);
            let f = (440.0 * libm::exp2((f64::from(note) - 69.0) / 12.0)) as f32;
            let data16 = (f64::from(f / PAL_CLOCK as f32) * 16_777_216.0) as i32 as u32;
            reg[base] = (data16 & 0xff) as u8;
            reg[base + 1] = ((data16 >> 8) & 0xff) as u8;
            let pw = self.knob(v, PULSE_WIDTH) as i32 as u32;
            reg[base + 2] = (pw & 0xff) as u8;
            reg[base + 3] = ((pw >> 8) & 0x0f) as u8;
            let mut control = u8::from(gate);
            control += if self.on(v, SYNC) { 2 } else { 0 };
            control += if self.on(v, RING_MOD) { 4 } else { 0 };
            control += if self.on(v, TEST) { 8 } else { 0 };
            control += self.waves[v].bits();
            reg[base + 4] = control;
            let nibble = |k: usize| (self.knob(v, k) as i32 as u32 & 0x0f) as u8;
            reg[base + 5] = (nibble(ATTACK) << 4) + nibble(DECAY);
            reg[base + 6] = (nibble(SUSTAIN) << 4) + nibble(RELEASE);
        }
        let fc = self.knobs[CUTOFF] as i32 as u32;
        reg[21] = (fc & 0x07) as u8;
        reg[22] = ((fc >> 3) & 0xff) as u8;
        let res = self.knobs[RESONANCE] as i32 as u32;
        let mut filt = ((res & 0x0f) << 4) as u8;
        filt += if self.on(2, FILTERED) { 4 } else { 0 };
        filt += if self.on(1, FILTERED) { 2 } else { 0 };
        filt += if self.on(0, FILTERED) { 1 } else { 0 };
        reg[23] = filt;
        let vol = self.knobs[VOLUME] as i32 as u32;
        let mut mode = (vol & 0x0f) as u8;
        mode += if self.knobs[VOICE3_OFF] >= 0.5 { 128 } else { 0 };
        mode += self.filter_mode.bits();
        reg[24] = mode;
        reg
    }
}

/// Clocks `voice`'s chip for at most `*cycles` cycles or until `out` is full, from `*made`; converts what it makes
fn clock(sid: &mut Sid, cycles: &mut i32, samples: &mut [i16; SCRATCH], out: &mut [f32], made: &mut usize) {
    while *cycles > 0 && *made < out.len() {
        let room = (out.len() - *made).min(SCRATCH);
        let n = sid.clock(cycles, &mut samples[..room]);
        for (o, s) in out[*made..*made + n].iter_mut().zip(&samples[..n]) {
            *o = f32::from(*s) / 32768.0;
        }
        *made += n;
        if n < room {
            // The cycles ran out before the room did
            break;
        }
    }
}

impl Voice {
    /// Makes `out.len()` samples: a write sequence starts at every tick of the player and whenever a register
    /// changes, and goes on, cycle for cycle, until it is done
    fn render(&mut self, desired: &[u8; REGISTERS], model: ChipModel, out: &mut [f32]) {
        if self.model != Some(model) {
            // (the legacy set it at the start of every period, before the writes, which also cleared the filter: here
            // only when it changes)
            self.sid.set_chip_model(model);
            self.model = Some(model);
        }
        let mut done = 0;
        while done < out.len() {
            let tick = self.until_tick == 0;
            if tick {
                self.until_tick = TICK;
            }
            if self.sequence.is_none() && (tick || self.written.as_ref() != Some(desired)) {
                self.sequence = Some(Sequence { next: 0, pending: 0, waited: false });
            }
            let n = (out.len() - done).min(self.until_tick);
            self.play(desired, &mut out[done..done + n]);
            done += n;
            self.until_tick -= n;
        }
    }

    /// Writes what is due of the sequence, then clocks the chip until `out` is full
    fn play(&mut self, desired: &[u8; REGISTERS], out: &mut [f32]) {
        let mut made = 0;
        while let Some(mut seq) = self.sequence {
            if made == out.len() {
                break;
            }
            if seq.pending > 0 {
                clock(&mut self.sid, &mut seq.pending, &mut self.samples, out, &mut made);
                self.sequence = Some(seq);
                if seq.pending > 0 {
                    // The piece is full: the sequence goes on in the next one
                    break;
                }
                continue;
            }
            if seq.next == REGISTERS {
                self.sequence = None;
                break;
            }
            let o = WRITE_ORDER[seq.next];
            if matches!(o, 4 | 11 | 18) && !seq.waited {
                // Extra delay for loading the waveform
                seq.waited = true;
                seq.pending = WAVE_DELAY;
                self.sequence = Some(seq);
                continue;
            }
            self.sid.write(o, desired[usize::from(o)]);
            let written = self.written.get_or_insert([0; REGISTERS]);
            written[usize::from(o)] = desired[usize::from(o)];
            self.sequence = Some(Sequence { next: seq.next + 1, pending: WRITE_DELAY, waited: false });
        }
        // Then until the samples are made
        let mut cycles = i32::MAX;
        clock(&mut self.sid, &mut cycles, &mut self.samples, out, &mut made);
    }
}

impl Synth for SidSynth {
    type Voice = Voice;
    const PARAMS: usize = PARAMS;

    fn voice(&self) -> Voice {
        Voice {
            sid: self.fresh.clone(),
            written: None,
            sequence: None,
            model: None,
            released: false,
            until_tick: 0,
            samples: [0; SCRATCH],
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(k) = self.knobs.get_mut(index) {
            *k = value;
        }
    }

    fn start(&self, voice: &mut Voice, _: &NoteStart, _: &mut Rng) {
        // A new chip, as the legacy made one per note
        voice.sid.copy_from(&self.fresh);
        voice.written = None;
        voice.sequence = None;
        voice.model = None;
        voice.released = false;
        voice.until_tick = 0;
    }

    fn release(&self, voice: &mut Voice) {
        voice.released = true;
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        let desired = self.registers(freq, !voice.released);
        voice.render(&desired, self.model, left);
        right.copy_from_slice(left);
    }

    fn release_ms(&self) -> f32 {
        let longest = (0..SID_VOICES).map(|v| self.knob(v, RELEASE) as i32).max().unwrap_or(0);
        RELEASE_MS[longest.clamp(0, 15) as usize]
    }
}

impl SidSynth {
    /// The synth of a complete state at `rate` Hz (it makes the fresh chip: build time only)
    pub fn from_state(state: &Value, rate: f64) -> Result<Self> {
        let s = StateView::new(state);
        let mut knobs = [0.0f32; PARAMS];
        for (v, keys) in VOICE_KEYS.iter().enumerate() {
            for (k, key) in keys.iter().enumerate() {
                knobs[v * PER_VOICE + k] = s.param_f32(key)?;
            }
        }
        for (k, key) in CHIP_KEYS.iter().enumerate() {
            knobs[CUTOFF + k] = s.param_f32(key)?;
        }
        let mut waves = [Wave::Triangle; SID_VOICES];
        for (w, key) in waves.iter_mut().zip(WAVE_FIELDS) {
            *w = s.choice(key, Wave::from_key)?;
        }
        let filter_mode = s.choice("filterMode", FilterMode::from_key)?;
        let model = s.choice("chipModel", chip_model)?;
        let fresh = Sid::new(f64::from(PAL_CLOCK), rate)
            .map_err(|e| Error::new("plugin.sid.chip").arg("why", e.to_string()))?;
        Ok(Self { knobs, waves, filter_mode, model, fresh })
    }

    /// The chip model chosen
    pub fn chip_model(&self) -> ChipModel {
        self.model
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let synth = SidSynth::from_state(state, f64::from(cx.spec.sample_rate))?;
    let params = &DESCRIPTOR.schema().params;
    let instrument = Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?;
    Ok(Box::new(instrument))
}
