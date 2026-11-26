//! The sound shaping of the legacy instrument track (`InstrumentSoundShaping`): the volume, cutoff and resonance
//! envelopes with their LFOs (F-IJN-010) and the filter (F-IJN-011).
//!
//! **This is everything of that kind, in one place, on purpose.** It is not the instrument's: decisoes-autonomas.md
//! ("O que deixa de seguir o legado", item 2) takes it out of the instruments, to become a per-voice effect when the
//! instrument exposes one or a normal audio effect ("Filtro com envelope") in the inserts. That move is the next
//! step; until then the instruments keep these fields and parameters (so their A/B tests against the legacy program
//! hold), and nothing outside this module and its two call sites in [`super::Instrument`] (`Kit::read` and the
//! `lfos`/`piece` methods, which call into here) knows what they are. Taking them out is deleting this module, the
//! `shaping` member of `Kit` and those calls, plus a schema upgrade that drops the fields.
//!
//! What may change while playing without allocating is a parameter (automatable; a switch is a stepped 0–1
//! parameter); every choice is a `Choice` field with permanent text keys (U30), and changing one rebuilds the
//! processor, which takes the sound over in `adopt` (U32).

use std::sync::LazyLock;

use lm_base::Result;
use lm_dsp::basic_filters::FilterType;
use lm_dsp::envelope::{EnvelopeSettings, LfoShape};
use lm_dsp::sound_shaping::SoundShapingSettings;
use lm_render::TransportInfo;
use lm_state::{FieldKind, FieldSpec, ParamSpec, Scale, Value};

use super::schema::{SYNC_KEYS, sync_beats};
use super::{GRID, Instrument, Synth};
use crate::state::StateView;

/// The three targets of the envelopes and LFOs, as key prefixes
pub const TARGETS: [&str; 3] = ["vol", "cut", "res"];

pub static FILTER_KEYS: LazyLock<Box<[&'static str]>> =
    LazyLock::new(|| FilterType::ALL.iter().map(|t| t.key()).collect());
pub static LFO_SHAPE_KEYS: LazyLock<Box<[&'static str]>> =
    LazyLock::new(|| LfoShape::ALL.iter().map(|s| s.key()).collect());

/// The key of the LFO user wave field of target `t`
pub const LFO_WAVE_FIELDS: [&str; 3] = ["volLfoWave", "cutLfoWave", "resLfoWave"];
pub const LFO_SHAPE_FIELDS: [&str; 3] = ["volLfoShape", "cutLfoShape", "resLfoShape"];
pub const LFO_SYNC_FIELDS: [&str; 3] = ["volLfoSync", "cutLfoSync", "resLfoSync"];

/// The parameters of the shaping, as offsets inside its own block (it starts at [`super::p::SHAPING`])
pub mod s {
    /// The first parameter of target `t` (0 volume, 1 cutoff, 2 resonance); the offsets below follow it
    pub const fn env(t: usize) -> usize {
        t * ENV_PARAMS
    }
    pub const ENV_PARAMS: usize = 13;
    pub const PREDELAY: usize = 0;
    pub const ATTACK: usize = 1;
    pub const HOLD: usize = 2;
    pub const DECAY: usize = 3;
    pub const SUSTAIN: usize = 4;
    pub const RELEASE: usize = 5;
    pub const AMOUNT: usize = 6;
    pub const LFO_PREDELAY: usize = 7;
    pub const LFO_ATTACK: usize = 8;
    pub const LFO_SPEED: usize = 9;
    pub const LFO_AMOUNT: usize = 10;
    pub const LFO_X100: usize = 11;
    pub const LFO_ENV_AMOUNT: usize = 12;
    pub const FILTER_ON: usize = 3 * ENV_PARAMS;
    pub const CUTOFF: usize = FILTER_ON + 1;
    pub const RESONANCE: usize = FILTER_ON + 2;
    /// How many there are
    pub const COUNT: usize = RESONANCE + 1;
}

fn switch(key: &'static str) -> ParamSpec {
    ParamSpec::new(key, 0.0, 1.0, 0.0).scale(Scale::Stepped)
}

fn env_params(t: &str) -> Vec<ParamSpec> {
    // The keys are permanent (C5): interned symbols, made once for the life of the program (lm_base::intern)
    let key = |name: &str| -> &'static str { lm_base::intern(&format!("{t}{name}")) };
    vec![
        ParamSpec::new(key("Predelay"), 0.0, 2.0, 0.0),
        ParamSpec::new(key("Attack"), 0.0, 2.0, 0.0),
        ParamSpec::new(key("Hold"), 0.0, 2.0, 0.5),
        ParamSpec::new(key("Decay"), 0.0, 2.0, 0.5),
        ParamSpec::new(key("Sustain"), 0.0, 1.0, 0.5),
        ParamSpec::new(key("Release"), 0.0, 2.0, 0.1),
        ParamSpec::new(key("Amount"), -1.0, 1.0, 0.0),
        ParamSpec::new(key("LfoPredelay"), 0.0, 1.0, 0.0),
        ParamSpec::new(key("LfoAttack"), 0.0, 1.0, 0.0),
        ParamSpec::new(key("LfoSpeed"), 0.001, 1.0, 0.1).scale(Scale::Log),
        ParamSpec::new(key("LfoAmount"), -1.0, 1.0, 0.0),
        switch(key("LfoX100")),
        switch(key("LfoEnvAmount")),
    ]
}

/// The shaping's parameters, in index order
pub fn params() -> Vec<ParamSpec> {
    let mut v = Vec::new();
    for t in TARGETS {
        v.extend(env_params(t));
    }
    v.extend([
        switch("filterOn"),
        ParamSpec::new("cutoff", 1.0, 14000.0, 14000.0).scale(Scale::Log).unit("Hz"),
        ParamSpec::new("resonance", 0.01, 10.0, 0.5),
    ]);
    debug_assert_eq!(v.len(), s::COUNT);
    v
}

/// The shaping's fields
pub fn fields() -> Vec<FieldSpec> {
    let mut v = Vec::new();
    for (shape, wave, sync) in [
        ("volLfoShape", "volLfoWave", "volLfoSync"),
        ("cutLfoShape", "cutLfoWave", "cutLfoSync"),
        ("resLfoShape", "resLfoWave", "resLfoSync"),
    ] {
        v.push(FieldSpec::choice(shape, &LFO_SHAPE_KEYS, "sine"));
        v.push(FieldSpec::new(wave, FieldKind::Resource("audio")).nullable().default(Value::NULL));
        v.push(FieldSpec::choice(sync, &SYNC_KEYS, "off"));
    }
    v.push(FieldSpec::choice("filterType", &FILTER_KEYS, "low-pass"));
    v
}

/// The choices of the shaping, read in the build
#[derive(Clone, Debug)]
pub(crate) struct ShapingKit {
    pub lfo_shape: [LfoShape; 3],
    pub lfo_sync: [Option<f64>; 3],
    pub filter: FilterType,
}

impl ShapingKit {
    pub(crate) fn read(state: &StateView<'_>) -> Result<Self> {
        let shape = |k: &str| state.choice(k, LfoShape::from_key);
        let sync = |k: &str| state.choice(k, |s| SYNC_KEYS.contains(&s).then(|| sync_beats(s)));
        Ok(Self {
            lfo_shape: [shape(LFO_SHAPE_FIELDS[0])?, shape(LFO_SHAPE_FIELDS[1])?, shape(LFO_SHAPE_FIELDS[2])?],
            lfo_sync: [sync(LFO_SYNC_FIELDS[0])?, sync(LFO_SYNC_FIELDS[1])?, sync(LFO_SYNC_FIELDS[2])?],
            filter: state.choice("filterType", FilterType::from_key)?,
        })
    }

    /// `block` is the shaping's parameter block (`s::COUNT` values)
    fn envelope(&self, block: &[f32], t: usize, bpm: f64) -> EnvelopeSettings {
        let e = |k: usize| block[s::env(t) + k];
        let on = |v: f32| v >= 0.5;
        let lfo_speed = match self.lfo_sync[t] {
            // The period of the synced length at this tempo, over the 20 s of the knob's end
            Some(beats) if bpm > 0.0 => (beats * 60.0 / bpm / 20.0) as f32,
            _ => e(s::LFO_SPEED),
        };
        EnvelopeSettings {
            predelay: e(s::PREDELAY),
            attack: e(s::ATTACK),
            hold: e(s::HOLD),
            decay: e(s::DECAY),
            sustain: e(s::SUSTAIN),
            release: e(s::RELEASE),
            amount: e(s::AMOUNT),
            lfo_predelay: e(s::LFO_PREDELAY),
            lfo_attack: e(s::LFO_ATTACK),
            lfo_speed,
            lfo_amount: e(s::LFO_AMOUNT),
            lfo_shape: self.lfo_shape[t],
            x100: on(e(s::LFO_X100)),
            control_env_amount: on(e(s::LFO_ENV_AMOUNT)),
        }
    }

    pub(crate) fn settings(&self, block: &[f32], bpm: f64) -> SoundShapingSettings {
        SoundShapingSettings {
            filter_enabled: block[s::FILTER_ON] >= 0.5,
            filter: self.filter,
            cutoff: block[s::CUTOFF],
            resonance: block[s::RESONANCE],
            volume: self.envelope(block, 0, bpm),
            cutoff_envelope: self.envelope(block, 1, bpm),
            resonance_envelope: self.envelope(block, 2, bpm),
        }
    }

    /// Whether any LFO follows the tempo
    pub(crate) fn synced(&self) -> bool {
        self.lfo_sync.iter().any(Option::is_some)
    }
}

impl<S: Synth> Instrument<S> {
    /// The LFOs move on to the piece of `n` frames at offset `at` (by the tempo while playing, when synced: U37)
    pub(super) fn lfos(&mut self, n: usize, at: usize, transport: &TransportInfo) {
        let bpm = transport.bpm;
        if bpm != self.synth_bpm {
            self.synth_bpm = bpm;
            self.synth.set_tempo(bpm);
            self.fading.set_tempo(bpm);
        }
        if self.kit.shaping.synced() && bpm != self.bpm {
            self.bpm = bpm;
            self.dirty = true;
        }
        if self.dirty {
            self.settings = self.kit.shaping_settings(self.bpm);
        }
        let previous = self.previous_piece;
        for t in 0..3 {
            let wave = self.lfo_waves[t].as_deref().map(|w| w.channel(0));
            let settings = match t {
                0 => self.settings.volume,
                1 => self.settings.cutoff_envelope,
                _ => self.settings.resonance_envelope,
            };
            let env = match t {
                0 => &mut self.shaping.volume,
                1 => &mut self.shaping.cutoff,
                _ => &mut self.shaping.resonance,
            };
            match self.kit.shaping.lfo_sync[t] {
                Some(beats) if transport.playing && bpm > 0.0 => {
                    // The phase from the position on the time line, not from how long the LFO has run: the same at a
                    // locate or a loop (U37), and the same for any block size (V3: an integer sample, not a sum of
                    // fractions). At a constant tempo it is the phase of the bar
                    let cycle = beats * 60.0 / bpm * f64::from(self.rate);
                    let sample = transport.position + at as i64;
                    let phase = (sample as f64).rem_euclid(cycle) / cycle;
                    let mut frames =
                        (lm_dsp::envelope::SECS_PER_LFO_OSCILLATION * self.rate * settings.lfo_speed) as i32;
                    if settings.x100 {
                        frames /= 100;
                    }
                    let target = (phase * f64::from(frames.max(1))) as usize;
                    if self.dirty {
                        env.set(&settings, wave, n);
                    }
                    env.reset_lfo(wave, n);
                    env.next_period(target, wave, n);
                }
                _ => {
                    env.next_period(previous, wave, n);
                    if self.dirty {
                        env.set(&settings, wave, n);
                    }
                }
            }
        }
        self.dirty = false;
        self.previous_piece = n;
    }
}

#[allow(dead_code)]
const _: usize = GRID;
