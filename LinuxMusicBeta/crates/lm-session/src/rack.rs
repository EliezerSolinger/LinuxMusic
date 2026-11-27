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
use lm_dsp::channel_strip::{ChannelStrip, HIGH_PASS_OFF, LOW_PASS_OFF, StripSettings, UNITY_POSITION};
use lm_dsp::dynamics::{CompressorModel, CompressorSettings, CompressorSpeed, GateMode, GateSettings};
use lm_dsp::gain_stage::GainModel;
use lm_dsp::master::{
    BusCompressor, BusCompressorSettings, BusRatio, Correlation, MasterAnalog, MonitorSettings, Monitoring, PreFaders,
    StereoSection, StereoSettings,
};
use lm_dsp::tolerance::{ChannelTolerance, OutputTransformer, TransformerCore};
use lm_render::{Adopted, EventKind, ProcessContext, Processor};
use lm_state::{PathStep, Value};

/// The largest piece processed at once (the scratch of interleaved frames)
const PIECE: usize = 256;

/// The loudness meter's reset requests: `loudness.reset` bumps it, each master segment restarts its meter when it
/// sees a number it has not (a counter, so the audio thread reads it without a lock)
static LOUDNESS_RESET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Asks every master segment to start its loudness meter again (the action `loudness.reset`)
pub fn reset_loudness() {
    LOUDNESS_RESET.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// What the rack shows on the desk (I-9.5: the display frame of each rack node, `f32` little-endian). Each segment
/// writes what it measures and NaN elsewhere; the session merges a channel's segments (`LocalPort::display`, kind
/// `"rack"`)
pub mod display {
    /// The gain stage's saturation in the last block, 0 to 1 (the SAT LED)
    pub const SAT: usize = 0;
    /// How many blocks clipped so far (the CLIP LED lights when it changes)
    pub const CLIP: usize = 1;
    /// The channel's gate and compressor gain reductions, dB (≥ 0)
    pub const GATE_GR: usize = 2;
    pub const COMP_GR: usize = 3;
    /// The master's bus compressor gain reduction, dB (≥ 0)
    pub const BUS_GR: usize = 4;
    /// The master's correlation over ~300 ms, -1 to 1
    pub const CORRELATION: usize = 5;
    /// The goniometer: `POINTS` (left, right) pairs of the master's output, spread over the last piece
    pub const GONIO: usize = 6;
    pub const POINTS: usize = 64;
    /// The master's loudness by BS.1770-4 (`lm_dsp::loudness`), taken before the speakers' monitoring: momentary,
    /// short-term and integrated LUFS (−∞ before anything passed), the true peak since the reset (dBTP) and the
    /// loudness range (LU). `loudness.reset` starts the integrated value, the range and the peak again
    pub const LUFS_M: usize = GONIO + 2 * POINTS;
    pub const LUFS_S: usize = LUFS_M + 1;
    pub const LUFS_I: usize = LUFS_M + 2;
    pub const TRUE_PEAK: usize = LUFS_M + 3;
    pub const LRA: usize = LUFS_M + 4;
    pub const FLOATS: usize = LRA + 1;
    pub const BYTES: usize = FLOATS * 4;
}

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
    // The channel-level controls (the legacy strip's input and output) and the desk's mode
    mono: bool,
    phase_invert: bool,
    swap: bool,
    digital: bool,
    // The monitoring (the master's last segment, live only)
    monitor: Option<(bool, bool, bool, bool)>,
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
            mono: flag(rack, "channel", "mono", false),
            phase_invert: flag(rack, "channel", "phaseInvert", false),
            swap: flag(rack, "channel", "swap", false),
            digital: rack.get("digital").and_then(Value::as_bool) == Some(true),
            monitor: rack.get("monitor").map(|m| {
                let b = |k: &str| m.get(k).and_then(Value::as_bool) == Some(true);
                (b("dim"), b("cut"), b("mono"), b("side"))
            }),
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
    pre: PreFaders,
    monitoring: Monitoring,
    /// The MONO switch's crossfade: the mix of the last frame (legacy `m_monoMix`)
    mono_mix: f32,
    /// The WIDTH knob's ramp: the width of the last frame (1 is untouched)
    width_now: f32,
    correlation: Correlation,
    /// The master's loudness meter (pair 0 of a master segment that ends the chain), with its split buffers
    loudness: Option<Box<lm_dsp::loudness::Loudness>>,
    loud_l: Box<[f32]>,
    loud_r: Box<[f32]>,
    loud_seen: u64,
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
    /// What the desk shows (pair 0), written to the display frame after each call
    shown: Box<[f32]>,
    clips: f32,
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
        // The channel strip's stages, as lm-dsp numbers them: a contiguous run (none for the input, polarity and
        // master segments)
        let indices: Vec<usize> = stages.iter().filter_map(|s| Stage::CHANNEL.iter().position(|c| c == s)).collect();
        let strip_stages = match (indices.first(), indices.last()) {
            _ if master => 0..0,
            (Some(first), Some(last)) => {
                if indices.windows(2).any(|w| w[1] != w[0] + 1)
                    || stages.iter().any(|s| matches!(s, Stage::Input | Stage::Polarity))
                {
                    return Err(Error::new("session.rack.stages")
                        .arg("stages", stages.iter().map(|s| s.key()).collect::<Vec<_>>().join(",")));
                }
                *first..*last + 1
            }
            _ => 0..0,
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
        let initial = |key: &str| params.iter().position(|p| p.key == key).map_or(1.0, |i| values[i]);
        let pre_now = [initial("preLeft"), initial("preRight")];
        let pairs = (0..usize::from(channels).div_ceil(2).max(1))
            .enumerate()
            .map(|(i, _)| Pair {
                strip: ChannelStrip::new(rate_f),
                transformer: OutputTransformer::new(TransformerCore::default(), f64::from(rate)),
                master: MasterAnalog::new(rate_f),
                stereo: StereoSection::new(rate_f),
                bus: BusCompressor::new(rate_f),
                // Built at its value: a change while playing is adopted and ramps
                pre: {
                    let mut p = PreFaders::default();
                    p.process(&mut [], pre_now);
                    p
                },
                monitoring: Monitoring::default(),
                // Built with MONO on, it starts summed (a change while playing is adopted and crossfades)
                mono_mix: if choices.mono { 1.0 } else { 0.0 },
                width_now: 1.0,
                correlation: Correlation::default(),
                loudness: (master && i == 0 && stages.contains(&Stage::MasterTransformer))
                    .then(|| Box::new(lm_dsp::loudness::Loudness::new(rate))),
                loud_l: vec![0.0; if master { PIECE } else { 0 }].into_boxed_slice(),
                loud_r: vec![0.0; if master { PIECE } else { 0 }].into_boxed_slice(),
                loud_seen: LOUDNESS_RESET.load(std::sync::atomic::Ordering::Relaxed),
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
            shown: vec![f32::NAN; display::FLOATS].into_boxed_slice(),
            clips: 0.0,
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
        let c = self.choices;
        let trim = self.value("channel", "trim");
        let width = (self.value("channel", "width") / 100.0).clamp(0.0, 2.0);
        let pre = [self.value("channel", "preLeft"), self.value("channel", "preRight")];
        let level = self.value("monitor", "level");
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
                Stage::Input => {
                    // TRIM (all the way down is off), then MONO crossfaded over the call (legacy Mixer.cpp)
                    if trim != 0.0 {
                        let g = if trim <= -60.0 { 0.0 } else { lm_dsp::math::dbfs_to_amp(trim) };
                        for f in frames.iter_mut() {
                            f[0] *= g;
                            f[1] *= g;
                        }
                    }
                    let target = if c.mono { 1.0 } else { 0.0 };
                    if target > 0.0 || p.mono_mix > 0.0 {
                        let count = frames.len().max(1) as f32;
                        for (i, f) in frames.iter_mut().enumerate() {
                            let mix = p.mono_mix + (target - p.mono_mix) * (i + 1) as f32 / count;
                            let sum = (f[0] + f[1]) * 0.5;
                            f[0] += (sum - f[0]) * mix;
                            f[1] += (sum - f[1]) * mix;
                        }
                    }
                    p.mono_mix = target;
                    // WIDTH: the side signal scaled, ramped over the call so a turn does not click
                    if width != 1.0 || p.width_now != 1.0 {
                        let count = frames.len().max(1) as f32;
                        for (i, f) in frames.iter_mut().enumerate() {
                            let w = p.width_now + (width - p.width_now) * (i + 1) as f32 / count;
                            let (mid, side) = ((f[0] + f[1]) * 0.5, (f[0] - f[1]) * 0.5 * w);
                            *f = [mid + side, mid - side];
                        }
                    }
                    p.width_now = width;
                }
                Stage::Polarity => {
                    let phase = if c.phase_invert { -1.0 } else { 1.0 };
                    for f in frames.iter_mut() {
                        let (l, r) = (f[0] * phase, f[1] * phase);
                        *f = if c.swap { [r, l] } else { [l, r] };
                    }
                }
                Stage::MasterDrive => {
                    // The PRE faders, then (analog) the gain stage they drive: the legacy master has no drive pot
                    p.pre.process(frames, pre);
                    if !c.digital && c.tube_on && c.model != GainModel::Flat {
                        p.master.process_drive(frames, c.model, c.realistic);
                    }
                }
                Stage::MasterBus => {
                    p.stereo.process(frames, &stereo);
                    p.bus.process(frames, &bus);
                }
                Stage::MasterTransformer => {
                    if c.transformer_on && !c.digital {
                        p.master.process_transformer(frames);
                    } else {
                        p.master.bypass_transformer();
                    }
                    // The loudness of what the master delivers, before the speakers' monitoring
                    if let Some(m) = p.loudness.as_mut() {
                        let want = LOUDNESS_RESET.load(std::sync::atomic::Ordering::Relaxed);
                        if want != p.loud_seen {
                            p.loud_seen = want;
                            m.reset();
                        }
                        let n = frames.len().min(p.loud_l.len());
                        for (k, f) in frames[..n].iter().enumerate() {
                            p.loud_l[k] = f[0];
                            p.loud_r[k] = f[1];
                        }
                        m.process(&p.loud_l[..n], &p.loud_r[..n]);
                    }
                    // The speakers' monitoring, after everything (never in an export: its build has no "monitor")
                    if let Some((dim, cut, mono, side)) = c.monitor {
                        p.monitoring.process(frames, &MonitorSettings { level, dim, cut, mono, side });
                    }
                }
            }
        }
    }
}

impl RackProcessor {
    /// What this segment measures, after a piece of pair 0
    fn measure(&mut self, n: usize) {
        let p = &mut self.pairs[0];
        let gain_here =
            self.strip_stages.contains(&0) || (self.stages.contains(&Stage::MasterDrive) && !self.choices.digital);
        if gain_here {
            let stage = if self.strip_stages.contains(&0) { p.strip.gain_stage() } else { p.master.gain_stage() };
            self.shown[display::SAT] = stage.take_saturation();
            if stage.take_clip() {
                self.clips += 1.0;
            }
            self.shown[display::CLIP] = self.clips;
        }
        if self.strip_stages.contains(&2) {
            let (gate, comp) = p.strip.reductions();
            self.shown[display::GATE_GR] = gate;
            self.shown[display::COMP_GR] = comp;
        }
        if let Some(m) = p.loudness.as_ref() {
            self.shown[display::LUFS_M] = m.momentary() as f32;
            self.shown[display::LUFS_S] = m.short_term() as f32;
            self.shown[display::LUFS_I] = m.integrated() as f32;
            self.shown[display::TRUE_PEAK] = m.true_peak() as f32;
            self.shown[display::LRA] = m.range() as f32;
        }
        if self.stages.contains(&Stage::MasterBus) {
            self.shown[display::BUS_GR] = p.bus.take_gain_reduction();
            let frames = &self.scratch[..n];
            self.shown[display::CORRELATION] = p.correlation.measure(frames, self.rate as f32);
            for k in 0..display::POINTS {
                let f = frames[(k * n) / display::POINTS];
                self.shown[display::GONIO + 2 * k] = f[0];
                self.shown[display::GONIO + 2 * k + 1] = f[1];
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
        let display = cx.display.as_deref_mut();
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
                if pair == 0 {
                    self.measure(n);
                }
                for k in 0..n {
                    io.get_mut(a)[done + k] = self.scratch[k][0];
                    if b < channels {
                        io.get_mut(b)[done + k] = self.scratch[k][1];
                    }
                }
                done += n;
            }
        }
        if let Some(w) = display {
            let shown = &self.shown;
            w.write(|bytes| {
                for (chunk, v) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(shown.iter()) {
                    chunk.copy_from_slice(&v.to_le_bytes());
                }
            });
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
