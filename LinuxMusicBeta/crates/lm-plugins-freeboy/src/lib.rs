//! FreeBoy (F-INS-005; legacy `plugins/FreeBoy`): the Game Boy's sound unit, one per note, its four channels (two
//! squares, the wave, the noise) playing the note, on the instrument functions. The unit is game-music-emu's
//! `Gb_Apu` with its `Blip_Buffer`, ported to Rust here ([`apu`], [`blip`]).
//!
//! Its state, version 1 (the legacy saved names in brackets):
//! - channel 1: `sweepTime` (0–7, 4) [st], `sweepDirection` (switch) [sd], `sweepRateShift` (0–7, 4) [srs],
//!   `ch1Duty` (0–3, 2: 12.5/25/50/75 %) [ch1wpd], `ch1Volume` (0–15, 15) [ch1vol], `ch1VolumeSweepDirection`
//!   (switch) [ch1vsd], `ch1SweepStepLength` (0–7, 0) [ch1ssl];
//! - channel 2: `ch2Duty`, `ch2Volume`, `ch2VolumeSweepDirection`, `ch2SweepStepLength` (as channel 1's);
//! - channel 3: `ch3Volume` (0–3, 3) [ch3vol]; field `wave` (its 32 steps of 0–15, drawn; all 0) [sampleShape];
//! - channel 4: `ch4Volume` (0–15, 15) [ch4vol], `ch4VolumeSweepDirection` [ch4vsd], `ch4SweepStepLength` [ch4ssl],
//!   `ch4ShiftRegisterWidth` (switch: 7 bits instead of 15) [srw];
//! - the outputs: `so1Volume` (the right, 0–7, 7) [so1vol], `so2Volume` (the left) [so2vol], and the switches
//!   `ch<n>ToSo1` (channel n to the right) and `ch<n>ToSo2` (to the left), on but for channel 4 (the legacy saved
//!   each SO1 switch under the SO2 name and the other way round: here each is named for what it does);
//! - `treble` (−100–200 dB, −20), `bass` (−1–600 Hz, 461);
//! - the instrument functions (`lm_plugin::instrument`).
//!
//! # How a note plays
//!
//! As the legacy `FreeBoyInstrument::playNoteImpl`: a fresh unit per note, powered on, the envelopes of channels 1,
//! 2 and 4 set (only then: a change of those knobs is heard from the next note), then the registers the legacy wrote
//! at every period of its engine, with a fake CPU clock of 4 cycles a write; the note triggers channels 1 to 3 at its
//! pitch (only between 65 and 4000 Hz, the integer Hz the legacy took) and the noise at the divider nearest it. The
//! samples come from frames of 70224 cycles (a Game Boy video frame), made when the last is used up: a register
//! written now is heard from the next frame, as in the legacy plugin.
//!
//! The periodic writes come at every [`TICK`] frames of the note (the legacy's default period, counted from the
//! note's start: the note sounds the same at any block size) and at once when one of them changes (a knob, the
//! pitch: the legacy waited for its next period). The left is SO2 at its level, the right SO1 (`so<n>Volume / 7`).

#![forbid(unsafe_code)]

pub mod apu;
pub mod blip;

use lm_base::Result;
use lm_plugin::instrument::{Instrument, NoteStart, Synth, add_instrument_functions};
use lm_plugin::{BuildContext, Descriptor, PluginKind, Port, Ports, StateView, TestHints, TuningProbe, Velocity};
use lm_render::{Processor, Rng};
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::apu::GbApu;
use crate::blip::BlipEq;

pub const ID: &str = "freeboy";
/// Notes sounding at once (a unit each; more steal, U27). The legacy kept a pool of 32 and made more on the audio
/// thread
pub const VOICES: usize = 32;
/// The Game Boy's clock
pub const CLOCK_RATE: i64 = 4_194_304;
/// Cycles of a frame of samples
pub const FRAME_LENGTH: i32 = 70_224;
/// The periodic writes, every this many frames of the note (the legacy's default period)
pub const TICK: usize = 256;
/// Steps of the wave channel's table
pub const WAVE_STEPS: usize = 32;

/// The parameters, in the schema's order
pub const KEYS: [&str; PARAMS] = [
    "sweepTime",
    "sweepDirection",
    "sweepRateShift",
    "ch1Duty",
    "ch1Volume",
    "ch1VolumeSweepDirection",
    "ch1SweepStepLength",
    "ch2Duty",
    "ch2Volume",
    "ch2VolumeSweepDirection",
    "ch2SweepStepLength",
    "ch3Volume",
    "ch4Volume",
    "ch4VolumeSweepDirection",
    "ch4SweepStepLength",
    "ch4ShiftRegisterWidth",
    "so1Volume",
    "so2Volume",
    "ch1ToSo1",
    "ch2ToSo1",
    "ch3ToSo1",
    "ch4ToSo1",
    "ch1ToSo2",
    "ch2ToSo2",
    "ch3ToSo2",
    "ch4ToSo2",
    "treble",
    "bass",
];
const PARAMS: usize = 28;
const SWEEP_TIME: usize = 0;
const SWEEP_DIR: usize = 1;
const SWEEP_SHIFT: usize = 2;
const CH1_DUTY: usize = 3;
const CH1_VOL: usize = 4;
const CH1_VSD: usize = 5;
const CH1_SSL: usize = 6;
const CH2_DUTY: usize = 7;
const CH2_VOL: usize = 8;
const CH2_VSD: usize = 9;
const CH2_SSL: usize = 10;
const CH3_VOL: usize = 11;
const CH4_VOL: usize = 12;
const CH4_VSD: usize = 13;
const CH4_SSL: usize = 14;
const CH4_SRW: usize = 15;
const SO1_VOL: usize = 16;
const SO2_VOL: usize = 17;
const TO_SO1: usize = 18;
const TO_SO2: usize = 22;
const TREBLE: usize = 26;
const BASS: usize = 27;

/// The descriptor
pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.freeboy.name",
    kind: PluginKind::Instrument,
    schemas,
    ports,
    velocity: Velocity::Level,
    display: None,
    build,
    tests: TestHints {
        tuning: Some(TuningProbe { state: probe_state, key: 69, hz: PROBE_HZ }),
        no_adopt: &[],
        voices: VOICES,
        max_level: 1.0e3,
    },
};

/// What A4 sounds at: the legacy takes 440 Hz, and the tone channels' period is a whole number of 32 cycles
/// (`2048 − ((4194304 / 440) >> 5)` = 1751: 4194304 / (297 × 32) Hz, 5 cents sharp)
pub const PROBE_HZ: f64 = 131_072.0 / 297.0;

/// The defaults without channel 1's sweep (which glides it up): two squares at the key, the wave channel's table
/// silent, the noise off
fn probe_state() -> Value {
    Value::empty_object()
        .with(lm_state::PARAMS_KEY, Value::empty_object().with("sweepTime", Value::real(0.0).unwrap_or(Value::NULL)))
}

fn schemas() -> Vec<NodeSchema> {
    let stepped = |key: &'static str, max: f64, def: f64| ParamSpec::new(key, 0.0, max, def).scale(Scale::Stepped);
    let switch = |key: &'static str, def: f64| ParamSpec::new(key, 0.0, 1.0, def).scale(Scale::Stepped);
    let s = NodeSchema::new("plugin:freeboy", 1)
        .param(stepped(KEYS[SWEEP_TIME], 7.0, 4.0))
        .param(switch(KEYS[SWEEP_DIR], 0.0))
        .param(stepped(KEYS[SWEEP_SHIFT], 7.0, 4.0))
        .param(stepped(KEYS[CH1_DUTY], 3.0, 2.0))
        .param(stepped(KEYS[CH1_VOL], 15.0, 15.0))
        .param(switch(KEYS[CH1_VSD], 0.0))
        .param(stepped(KEYS[CH1_SSL], 7.0, 0.0))
        .param(stepped(KEYS[CH2_DUTY], 3.0, 2.0))
        .param(stepped(KEYS[CH2_VOL], 15.0, 15.0))
        .param(switch(KEYS[CH2_VSD], 0.0))
        .param(stepped(KEYS[CH2_SSL], 7.0, 0.0))
        .param(stepped(KEYS[CH3_VOL], 3.0, 3.0))
        .param(stepped(KEYS[CH4_VOL], 15.0, 15.0))
        .param(switch(KEYS[CH4_VSD], 0.0))
        .param(stepped(KEYS[CH4_SSL], 7.0, 0.0))
        .param(switch(KEYS[CH4_SRW], 0.0))
        .param(stepped(KEYS[SO1_VOL], 7.0, 7.0))
        .param(stepped(KEYS[SO2_VOL], 7.0, 7.0))
        .param(switch(KEYS[TO_SO1], 1.0))
        .param(switch(KEYS[TO_SO1 + 1], 1.0))
        .param(switch(KEYS[TO_SO1 + 2], 1.0))
        .param(switch(KEYS[TO_SO1 + 3], 0.0))
        .param(switch(KEYS[TO_SO2], 1.0))
        .param(switch(KEYS[TO_SO2 + 1], 1.0))
        .param(switch(KEYS[TO_SO2 + 2], 1.0))
        .param(switch(KEYS[TO_SO2 + 3], 0.0))
        .param(ParamSpec::new(KEYS[TREBLE], -100.0, 200.0, -20.0).scale(Scale::Stepped).unit("dB"))
        .param(ParamSpec::new(KEYS[BASS], -1.0, 600.0, 461.0).scale(Scale::Stepped).unit("Hz"))
        .field(
            FieldSpec::new("wave", FieldKind::List(Box::new(FieldKind::Real { min: 0.0, max: 15.0 })))
                .default(Value::list((0..WAVE_STEPS).map(|_| Value::real(0.0).unwrap_or(Value::NULL)))),
        );
    vec![add_instrument_functions(s)]
}

fn ports(_: &Value) -> Ports {
    Ports { outputs: vec![Port::new("main", 2)], sidechains: Vec::new() }
}

/// The synth: the knobs, the wave channel's table, and a fresh unit every note starts from
#[derive(Clone, Debug)]
pub struct FreeBoySynth {
    knobs: [f32; PARAMS],
    wave: [f32; WAVE_STEPS],
    fresh: GbApu,
}

/// The registers the legacy wrote at every period, in its order (address, value), and the equalization
#[derive(Clone, Copy, Debug, PartialEq)]
struct Periodic {
    treble: f32,
    bass: i32,
    writes: [(u16, u8); 7 + 16],
    /// The tone channels' frequency registers (0xff18/19, 0xff1d/1e), when the pitch is within 65–4000 Hz
    tone: Option<u16>,
}

/// One note: its unit and what was done to it
#[derive(Debug)]
pub struct Voice {
    apu: GbApu,
    /// The fake CPU clock: cycles since the frame began (4 a write)
    time: i32,
    /// Frames since the note started (none: not started)
    frames: Option<u64>,
    written: Option<Periodic>,
    until_tick: usize,
    samples: [[i16; 2]; 64],
}

impl Clone for Voice {
    /// A new unit (allocates: build time only)
    fn clone(&self) -> Self {
        Self {
            apu: self.apu.clone(),
            time: self.time,
            frames: self.frames,
            written: self.written,
            until_tick: self.until_tick,
            samples: self.samples,
        }
    }

    /// In place, without allocating (the instrument's crossfade, on the audio thread)
    fn clone_from(&mut self, source: &Self) {
        self.apu.copy_from(&source.apu);
        self.time = source.time;
        self.frames = source.frames;
        self.written = source.written;
        self.until_tick = source.until_tick;
    }
}

impl Voice {
    fn write(&mut self, addr: u16, data: u8) {
        self.time += 4;
        self.apu.write_register(self.time, u32::from(addr), data);
    }
}

fn on(v: f32) -> bool {
    v >= 0.5
}

/// `(data << 1) + switch << 3 + steps`, the envelope register's layout
fn envelope(volume: f32, direction: f32, steps: f32) -> u8 {
    let mut data = volume as i32;
    data <<= 1;
    data += i32::from(on(direction));
    data <<= 3;
    data += steps as i32;
    data as u8
}

impl FreeBoySynth {
    /// The synth of a complete state at `rate` Hz (it makes the fresh unit: build time only)
    pub fn from_state(state: &Value, rate: u32) -> Result<Self> {
        let s = StateView::new(state);
        let mut knobs = [0.0f32; PARAMS];
        for (k, key) in knobs.iter_mut().zip(&KEYS[..PARAMS]) {
            *k = s.param_f32(key)?;
        }
        let mut wave = [0.0f32; WAVE_STEPS];
        if let Some(list) = state.get("wave") {
            for (w, v) in wave.iter_mut().zip(list.items()) {
                *w = v.as_real().unwrap_or(0.0) as f32;
            }
        }
        Ok(Self { knobs, wave, fresh: GbApu::new(i64::from(rate), CLOCK_RATE) })
    }

    fn periodic(&self, freq: i32) -> Periodic {
        let k = &self.knobs;
        let mut writes = [(0u16, 0u8); 23];
        // channel 1 - square
        writes[0] = (0xff10, envelope(k[SWEEP_TIME], k[SWEEP_DIR], k[SWEEP_SHIFT]));
        writes[1] = (0xff11, ((k[CH1_DUTY] as i32) << 6) as u8);
        // channel 2 - square
        writes[2] = (0xff16, ((k[CH2_DUTY] as i32) << 6) as u8);
        // channel 3 - wave
        writes[3] = (0xff1a, 128);
        const CH3_VOLUME: [i32; 4] = [0, 3, 2, 1];
        writes[4] = (0xff1c, (CH3_VOLUME[(k[CH3_VOL] as i32).clamp(0, 3) as usize] << 5) as u8);
        // both outputs at the top, each side scaled after (the emulator applies one volume to both)
        writes[5] = (0xff24, 0x77);
        let mut routing = 0i32;
        for ch in 0..4 {
            if on(k[TO_SO2 + ch]) {
                routing += 16 << ch;
            }
            if on(k[TO_SO1 + ch]) {
                routing += 1 << ch;
            }
        }
        writes[6] = (0xff25, routing as u8);
        for i in 0..16 {
            let hi = libm::floorf(self.wave[i * 2]) as i32;
            let lo = libm::floorf(self.wave[i * 2 + 1]) as i32;
            writes[7 + i] = (0xff30 + i as u16, ((hi << 4) + lo) as u8);
        }
        let tone = (65..=4000).contains(&freq).then(|| (2048 - ((4_194_304 / freq) >> 5)) as u16);
        Periodic { treble: k[TREBLE], bass: k[BASS] as i32, writes, tone }
    }

    /// The legacy's writes of a period: `first` the note's first, with its triggers and the noise
    fn play_period(&self, voice: &mut Voice, p: &Periodic, freq: i32, first: bool) {
        let k = &self.knobs;
        voice.apu.treble_eq(&BlipEq::treble(f64::from(p.treble)));
        voice.apu.buf.set_bass_freq(p.bass);
        for &(addr, data) in &p.writes[..7] {
            voice.write(addr, data);
        }
        for &(addr, data) in &p.writes[7..] {
            voice.write(addr, data);
        }
        if let Some(data) = p.tone {
            let init: u16 = if first { 128 } else { 0 };
            if first {
                voice.write(0xff13, (data & 0xff) as u8);
                voice.write(0xff14, ((data >> 8) | init) as u8);
            }
            voice.write(0xff18, (data & 0xff) as u8);
            voice.write(0xff19, ((data >> 8) | init) as u8);
            voice.write(0xff1d, (data & 0xff) as u8);
            voice.write(0xff1e, ((data >> 8) | init) as u8);
        }
        if first {
            // The noise: the divider whose frequency is nearest the note's (`524288 / (r · 2^(s + 1))`, r = 0 as 0.5)
            let mut clock_freq = 0i32;
            let mut div_ratio = 0i32;
            let mut closest = (524_288.0 / (0.5 * libm::exp2(f64::from(clock_freq) + 1.0))) as f32;
            for s in 0..16 {
                for r in 1..8 {
                    let f = (524_288.0 / (f64::from(r) * libm::exp2(f64::from(s) + 1.0))) as f32;
                    if (freq as f32 - closest).abs() > (freq as f32 - f).abs() {
                        closest = f;
                        div_ratio = r;
                        clock_freq = s;
                    }
                }
            }
            let mut data = clock_freq;
            data <<= 1;
            data += i32::from(on(k[CH4_SRW]));
            data <<= 3;
            data += div_ratio;
            voice.write(0xff22, data as u8);
            voice.write(0xff23, 128);
        }
        voice.written = Some(*p);
    }

    /// Reads `out.len()` frames, making frames of samples as they are used up
    fn read(&self, voice: &mut Voice, left: &mut [f32], right: &mut [f32]) {
        let gain = [self.knobs[SO2_VOL] / 7.0 / 32768.0, self.knobs[SO1_VOL] / 7.0 / 32768.0];
        let mut done = 0;
        while done < left.len() {
            if voice.apu.buf.frames_avail() <= 0 {
                voice.time = 0;
                voice.apu.end_frame(FRAME_LENGTH);
            }
            let want = (left.len() - done).min(voice.samples.len());
            let got = voice.apu.buf.read(&mut voice.samples[..want]);
            for (i, s) in voice.samples[..got].iter().enumerate() {
                left[done + i] = f32::from(s[0]) * gain[0];
                right[done + i] = f32::from(s[1]) * gain[1];
            }
            done += got;
        }
    }
}

impl Synth for FreeBoySynth {
    type Voice = Voice;
    const PARAMS: usize = PARAMS;

    fn voice(&self) -> Voice {
        Voice { apu: self.fresh.clone(), time: 0, frames: None, written: None, until_tick: 0, samples: [[0; 2]; 64] }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(k) = self.knobs.get_mut(index) {
            *k = value;
        }
    }

    fn start(&self, voice: &mut Voice, _: &NoteStart, _: &mut Rng) {
        voice.apu.copy_from(&self.fresh);
        voice.time = 0;
        voice.frames = None;
        voice.written = None;
        voice.until_tick = 0;
    }

    fn render(&self, voice: &mut Voice, freq: f32, left: &mut [f32], right: &mut [f32]) {
        // (the legacy took the note's frequency in whole Hz)
        let hz = freq as i32;
        let k = &self.knobs;
        let mut done = 0;
        while done < left.len() {
            let first = voice.frames.is_none();
            if first {
                // Master sound circuitry power control, then the envelopes (only at the note's start)
                voice.write(0xff26, 0x80);
                voice.write(0xff12, envelope(k[CH1_VOL], k[CH1_VSD], k[CH1_SSL]));
                voice.write(0xff17, envelope(k[CH2_VOL], k[CH2_VSD], k[CH2_SSL]));
                voice.write(0xff21, envelope(k[CH4_VOL], k[CH4_VSD], k[CH4_SSL]));
                voice.frames = Some(0);
            }
            let tick = voice.until_tick == 0;
            if tick {
                voice.until_tick = TICK;
            }
            let p = self.periodic(hz);
            if first || tick || voice.written != Some(p) {
                self.play_period(voice, &p, hz, first);
            }
            let n = (left.len() - done).min(voice.until_tick);
            self.read(voice, &mut left[done..done + n], &mut right[done..done + n]);
            done += n;
            voice.until_tick -= n;
            voice.frames = voice.frames.map(|f| f + n as u64);
        }
    }

    fn release_ms(&self) -> f32 {
        // (the legacy's: 1000 samples at 44.1 kHz were 23 ms)
        23.0
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let s = StateView::new(state);
    let synth = FreeBoySynth::from_state(state, cx.spec.sample_rate)?;
    let params = &DESCRIPTOR.schema().params;
    let instrument = Instrument::new(synth, &s, params, &cx.spec, cx.resources, VOICES)?;
    Ok(Box::new(instrument))
}
