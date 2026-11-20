//! The desk's rack processor (I-3.9, M5; P-17, P-20, P-21): what the session builds for lm-compile's
//! `Build::Rack`. One processor per segment of stages: the channel's rack runs `lm-dsp`'s `ChannelStrip` (the gain
//! stage, the filters, the dynamics, the EQ) and the output transformer; the master's runs the drive, the stereo
//! section with the bus compressor, or the transformer after the fader.
//!
//! Its parameters are the rack's real fields, indexed as `lm_compile::rack_params` lists them; the switches and
//! choices of the sections (`on`, models, speeds) are read from the tree at the build (a change rebuilds, and the
//! new processor adopts the old one's filters and dynamics state). The tolerances are drawn from the build's seed,
//! the channel's number (P-21).

use std::any::Any;

use lm_base::{Error, Result};
use lm_compile::{RackParam, Stage};
use lm_dsp::channel_strip::{ChannelStrip, HIGH_PASS_OFF, LOW_PASS_OFF, StripSettings, UNITY_POSITION, pot_gain};
use lm_dsp::dynamics::{CompressorModel, CompressorSettings, CompressorSpeed, GateMode, GateSettings};
use lm_dsp::gain_stage::GainModel;
use lm_dsp::master::{BusCompressor, BusCompressorSettings, BusRatio, MasterAnalog, StereoSection, StereoSettings};
use lm_dsp::tolerance::{ChannelTolerance, OutputTransformer, TransformerCore};
use lm_render::{Adopted, EventKind, ProcessContext, Processor};
use lm_state::{PathStep, Value};

/// The largest piece processed at once (the scratch of interleaved frames)
const PIECE: usize = 256;

fn section<'a>(rack: &'a Value, name: &str) -> Option<&'a Value> {
    rack.get(name)
}

fn flag(rack: &Value, section_name: &str, key: &str, default: bool) -> bool {
    section(rack, section_name).and_then(|s| s.get(key)).and_then(Value::as_bool).unwrap_or(default)
}

fn text<'a>(rack: &'a Value, section_name: &str, key: &str) -> Option<&'a str> {
    section(rack, section_name).and_then(|s| s.get(key)).and_then(Value::as_text)
}

fn int(rack: &Value, section_name: &str, key: &str) -> Option<i64> {
    section(rack, section_name).and_then(|s| s.get(key)).and_then(Value::as_int)
}

/// The switches and choices of a rack (what is not a parameter)
#[derive(Clone, Copy, Debug, PartialEq)]
struct Choices {
    tube_on: bool,
    model: GainModel,
    realistic: bool,
    low_cut: bool,
    eq_on: bool,
    dynamics_on: bool,
    comp_model: CompressorModel,
    comp_speed: CompressorSpeed,
    gate_mode: GateMode,
    transformer_on: bool,
    tolerance_on: bool,
    // Master
    bus_on: bool,
    bus_ratio: BusRatio,
    bus_attack: usize,
    bus_release: usize,
}

impl Choices {
    fn read(rack: &Value) -> Result<Self> {
        let bad = |what: &'static str| Error::new("session.rack.choice").arg("field", what);
        let model = GainModel::from_key(text(rack, "tube", "model").unwrap_or("4558")).ok_or_else(|| bad("model"))?;
        let comp_model = CompressorModel::from_key(text(rack, "dynamics", "compModel").unwrap_or("ssl"))
            .ok_or_else(|| bad("compModel"))?;
        let comp_speed = match text(rack, "dynamics", "compSpeed").unwrap_or("norm") {
            "fast" => CompressorSpeed::Fast,
            "slow" => CompressorSpeed::Slow,
            _ => CompressorSpeed::Norm,
        };
        let gate_mode =
            GateMode::from_key(text(rack, "dynamics", "gateMode").unwrap_or("std")).ok_or_else(|| bad("gateMode"))?;
        let bus_ratio = match text(rack, "dynamics", "ratio").unwrap_or("4") {
            "2" => BusRatio::Two,
            "10" => BusRatio::Ten,
            _ => BusRatio::Four,
        };
        Ok(Self {
            tube_on: flag(rack, "tube", "on", true),
            model,
            realistic: flag(rack, "tube", "realistic", false),
            low_cut: flag(rack, "tube", "lowCut", false),
            eq_on: flag(rack, "eq", "on", true),
            dynamics_on: flag(rack, "dynamics", "on", true),
            comp_model,
            comp_speed,
            gate_mode,
            transformer_on: flag(rack, "transformer", "on", false),
            tolerance_on: flag(rack, "tolerance", "on", true),
            bus_on: flag(rack, "dynamics", "on", false),
            bus_ratio,
            bus_attack: int(rack, "dynamics", "attack").unwrap_or(4).clamp(0, 5) as usize,
            bus_release: int(rack, "dynamics", "release").unwrap_or(4).clamp(0, 4) as usize,
        })
    }
}

/// One channel pair's state
struct Pair {
    strip: ChannelStrip,
    transformer: OutputTransformer,
    master: MasterAnalog,
    stereo: StereoSection,
    bus: BusCompressor,
}

/// A segment of a rack
pub struct RackProcessor {
    master: bool,
    stages: Vec<Stage>,
    params: Vec<RackParam>,
    values: Vec<f32>,
    choices: Choices,
    tolerance: ChannelTolerance,
    pairs: Vec<Pair>,
    scratch: Box<[[f32; 2]]>,
    rate: u32,
    /// The channel strip's stages this segment runs (lm-dsp's numbering: 0 gain, 1 filters, 2 dynamics, 3 EQ)
    strip_stages: std::ops::Range<usize>,
}

impl std::fmt::Debug for RackProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RackProcessor").field("master", &self.master).field("stages", &self.stages).finish()
    }
}

impl RackProcessor {
    /// A segment of `stages` of a rack (`rack`, the channel's tree), for `channels` channels at `rate`; `params` is
    /// `lm_compile::rack_params(schemas, master)`; `seed` the channel's number (P-21)
    pub fn new(
        master: bool,
        stages: &[Stage],
        rack: &Value,
        params: Vec<RackParam>,
        channels: u16,
        rate: u32,
        seed: u64,
    ) -> Result<Self> {
        if stages.is_empty() {
            return Err(Error::new("session.rack.no_stages"));
        }
        let strip_stages = if master {
            0..0
        } else {
            // The channel's stages, as lm-dsp numbers them: a contiguous run
            let indices: Vec<usize> =
                stages.iter().filter_map(|s| Stage::CHANNEL.iter().position(|c| c == s)).collect();
            let (Some(first), Some(last)) = (indices.first(), indices.last()) else {
                return Err(Error::new("session.rack.no_stages"));
            };
            if indices.len() != stages.len() || indices.windows(2).any(|w| w[1] != w[0] + 1) {
                return Err(Error::new("session.rack.stages")
                    .arg("stages", stages.iter().map(|s| s.key()).collect::<Vec<_>>().join(",")));
            }
            *first..*last + 1
        };
        let choices = Choices::read(rack)?;
        let mut values = Vec::with_capacity(params.len());
        for p in &params {
            let path = [PathStep::key(p.section), PathStep::key(p.field)];
            values.push(rack.at_path(&path).and_then(Value::as_real).unwrap_or(p.default) as f32);
        }
        let channel = i32::try_from(seed % 1_000_000).unwrap_or(0);
        let tolerance =
            if choices.tolerance_on { ChannelTolerance::for_channel(channel) } else { ChannelTolerance::default() };
        let rate_f = rate as f32;
        let pairs = (0..usize::from(channels).div_ceil(2).max(1))
            .map(|_| Pair {
                strip: ChannelStrip::new(rate_f),
                transformer: OutputTransformer::new(TransformerCore::default(), f64::from(rate)),
                master: MasterAnalog::new(rate_f),
                stereo: StereoSection::new(rate_f),
                bus: BusCompressor::new(rate_f),
            })
            .collect();
        Ok(Self {
            master,
            stages: stages.to_vec(),
            params,
            values,
            choices,
            tolerance,
            pairs,
            scratch: vec![[0.0; 2]; PIECE].into_boxed_slice(),
            rate,
            strip_stages,
        })
    }

    fn value(&self, section_name: &str, field: &str) -> f32 {
        self.params.iter().position(|p| p.section == section_name && p.field == field).map_or(0.0, |i| self.values[i])
    }

    fn strip_settings(&self) -> StripSettings {
        let c = &self.choices;
        let mut s = StripSettings::default();
        if c.tube_on {
            s.gain = self.value("tube", "drive");
            s.drive = c.model;
            s.realistic = c.realistic;
            s.low_cut = c.low_cut;
        } else {
            s.gain = UNITY_POSITION;
            s.drive = GainModel::Flat;
        }
        if c.eq_on {
            s.high_pass = self.value("eq", "highPass");
            s.low_pass = self.value("eq", "lowPass");
            s.mono_bass = self.value("eq", "monoBass");
            s.mono_bass_frequency = self.value("eq", "monoBassFrequency");
            s.eq = [
                self.value("eq", "low"),
                self.value("eq", "lowMid"),
                self.value("eq", "highMid"),
                self.value("eq", "high"),
            ];
        } else {
            s.high_pass = HIGH_PASS_OFF;
            s.low_pass = LOW_PASS_OFF;
        }
        if c.dynamics_on {
            s.compressor = CompressorSettings {
                model: c.comp_model,
                amount: self.value("dynamics", "compAmount"),
                ratio: self.value("dynamics", "compRatio"),
                speed: c.comp_speed,
                release: self.value("dynamics", "compRelease"),
            };
            s.gate = GateSettings {
                threshold: self.value("dynamics", "gateThreshold"),
                range: self.value("dynamics", "gateRange"),
                mode: c.gate_mode,
            };
        }
        s
    }

    fn stereo_settings(&self) -> StereoSettings {
        StereoSettings {
            width: self.value("stereo", "width"),
            mono_bass: self.value("stereo", "monoBass"),
            bass_frequency: self.value("stereo", "bassFrequency"),
            air: self.value("stereo", "air"),
        }
    }

    fn bus_settings(&self) -> BusCompressorSettings {
        let c = &self.choices;
        BusCompressorSettings {
            on: c.bus_on,
            threshold: self.value("dynamics", "threshold"),
            ratio: c.bus_ratio,
            attack: c.bus_attack,
            release: if c.bus_release >= 4 { BusCompressor::AUTO_RELEASE } else { c.bus_release },
            makeup: self.value("dynamics", "makeup"),
        }
    }

    /// One piece of interleaved frames through the segment's stages
    fn run(&mut self, pair: usize, n: usize, had_input: bool) {
        let strip = self.strip_settings();
        let stereo = self.stereo_settings();
        let bus = self.bus_settings();
        let drive = pot_gain(self.value("tube", "drive"));
        let c = self.choices;
        let tolerance = self.tolerance;
        let stages = &self.stages;
        let frames = &mut self.scratch[..n];
        let p = &mut self.pairs[pair];
        if !self.strip_stages.is_empty() {
            // The channel's segment: its stages of the strip, then the transformer after the EQ (the end of the rack)
            p.strip.process_stages(frames, &strip, had_input, &tolerance, self.strip_stages.clone());
            if self.strip_stages.end == lm_dsp::channel_strip::STRIP_STAGES && c.transformer_on {
                for f in frames.iter_mut() {
                    f[0] = p.transformer.process(f[0], 0);
                    f[1] = p.transformer.process(f[1], 1);
                }
            }
        }
        for stage in stages {
            match stage {
                Stage::Gain | Stage::Filter | Stage::Dynamics | Stage::Eq => {}
                Stage::MasterDrive => {
                    if c.tube_on && c.model != GainModel::Flat {
                        if drive != 1.0 {
                            for f in frames.iter_mut() {
                                f[0] *= drive;
                                f[1] *= drive;
                            }
                        }
                        p.master.process_drive(frames, c.model, c.realistic);
                    }
                }
                Stage::MasterBus => {
                    p.stereo.process(frames, &stereo);
                    p.bus.process(frames, &bus);
                }
                Stage::MasterTransformer => {
                    if c.transformer_on {
                        p.master.process_transformer(frames);
                    } else {
                        p.master.bypass_transformer();
                    }
                }
            }
        }
    }
}

impl Processor for RackProcessor {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        let frames = cx.frames;
        for e in cx.events {
            if let EventKind::Param { index, value } = e.kind
                && let Some(p) = self.params.get(index as usize)
                && value.is_finite()
            {
                self.values[index as usize] = value.clamp(p.min as f32, p.max as f32);
            }
        }
        let channels = cx.buffers.outputs();
        let (_, mut io, _) = cx.buffers.split();
        let pairs = channels.div_ceil(2);
        for pair in 0..pairs.min(self.pairs.len()) {
            let (a, b) = (2 * pair, 2 * pair + 1);
            let mut done = 0;
            while done < frames {
                let n = PIECE.min(frames - done);
                let mut had = false;
                for k in 0..n {
                    let l = io.get_mut(a)[done + k];
                    let r = if b < channels { io.get_mut(b)[done + k] } else { l };
                    had |= l != 0.0 || r != 0.0;
                    self.scratch[k] = [l, r];
                }
                self.run(pair, n, had);
                for k in 0..n {
                    io.get_mut(a)[done + k] = self.scratch[k][0];
                    if b < channels {
                        io.get_mut(b)[done + k] = self.scratch[k][1];
                    }
                }
                done += n;
            }
        }
    }

    fn reset(&mut self) {
        for p in &mut self.pairs {
            p.strip.reset();
            p.transformer.reset();
            p.master.reset();
            p.stereo.reset();
            p.bus.reset();
        }
    }

    /// The same segment at the same rate takes over the filters', the dynamics' and the transformers' state
    fn adopt(&mut self, old: &mut dyn Processor) -> Adopted {
        match old.as_any_mut().downcast_mut::<RackProcessor>() {
            Some(o)
                if o.rate == self.rate
                    && o.master == self.master
                    && o.stages == self.stages
                    && o.pairs.len() == self.pairs.len() =>
            {
                std::mem::swap(&mut self.pairs, &mut o.pairs);
                Adopted::Yes
            }
            _ => Adopted::No,
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
