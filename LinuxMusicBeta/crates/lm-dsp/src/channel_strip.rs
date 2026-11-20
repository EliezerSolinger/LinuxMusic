//! The channel strip of an analog mixing desk (legacy `AnalogChannelStrip`): the rack of each mixer channel.

use crate::dynamics::{ChannelCompressor, ChannelGate, CompressorSettings, GateSettings};
use crate::filters::{Biquad, DeskEq, RampedBiquad};
use crate::gain_stage::{GainModel, GainStage};
use crate::math::dbfs_to_amp;
use crate::mono_bass::MonoBass;
use crate::tolerance::ChannelTolerance;
use core::ops::Range;

const LOW_CUT_FREQUENCY: f64 = 100.0;
const BUTTERWORTH_Q: f64 = core::f64::consts::FRAC_1_SQRT_2;
/// s, how long the filters may ring after the input
const FILTER_TAIL: f32 = 0.05;

/// The GAIN pot, as on a desk's microphone preamp: muted at the far left, 0 dB at a quarter of its travel, +60 dB at
/// the far right
pub const MAX_GAIN: f32 = 60.0;
pub const UNITY_POSITION: f32 = 0.25;
/// The high-pass is off at its lowest setting
pub const HIGH_PASS_OFF: f32 = 20.0;
/// The low-pass is off at its highest setting
pub const LOW_PASS_OFF: f32 = 20000.0;

/// The gain, linear, at pot position @p position (0 to 1): up to unity, (position / UNITY_POSITION)^3 (-18 dB
/// halfway there), then linear in dB up to MAX_GAIN
pub fn pot_gain(position: f32) -> f32 {
    if position <= 0.0 {
        return 0.0;
    }
    if position <= UNITY_POSITION {
        let x = position / UNITY_POSITION;
        return x * x * x;
    }
    dbfs_to_amp((position - UNITY_POSITION) / (1.0 - UNITY_POSITION) * MAX_GAIN)
}

/// Where the channel's effects go in its signal path (the user's notes of 13/11)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InsertPoint {
    /// Before the GAIN pot
    PreGain,
    /// After the gain stage (and the low cut)
    PostGain,
    /// After the HPF, the LPF and the mono bass
    PostFilter,
    /// After the gate and the compressor
    PostDynamics,
    /// After the EQ: the end of the strip
    #[default]
    PostEq,
    /// After the fader, before the panning (post-fader): not in the strip
    PrePan,
}

impl InsertPoint {
    pub fn key(self) -> &'static str {
        match self {
            InsertPoint::PreGain => "pre-gain",
            InsertPoint::PostGain => "post-gain",
            InsertPoint::PostFilter => "post-filter",
            InsertPoint::PostDynamics => "post-dynamics",
            InsertPoint::PostEq => "post-eq",
            InsertPoint::PrePan => "pre-pan",
        }
    }
}

/// The strip's stages, for [`ChannelStrip::process_stages`]: 0 the gain (pot, low cut, gain stage), 1 the filters
/// (high-pass, low-pass, mono bass), 2 the dynamics (gate, compressor), 3 the EQ
pub const STRIP_STAGES: usize = 4;

/// The insert point at the end of each stage
const STAGE_ENDS: [InsertPoint; STRIP_STAGES] =
    [InsertPoint::PostGain, InsertPoint::PostFilter, InsertPoint::PostDynamics, InsertPoint::PostEq];

/// The stages to run before an insert at @p point, as `0..k` (the rest is `k..STRIP_STAGES`): `PreGain` 0, `PostGain`
/// 1, `PostFilter` 2, `PostDynamics` 3, `PostEq` and `PrePan` 4
pub fn stage_after(point: InsertPoint) -> usize {
    match point {
        InsertPoint::PreGain => 0,
        InsertPoint::PostGain => 1,
        InsertPoint::PostFilter => 2,
        InsertPoint::PostDynamics => 3,
        InsertPoint::PostEq | InsertPoint::PrePan => STRIP_STAGES,
    }
}

/// What the strip calls at its insert point: the channel's effects, on the frames
pub type InsertHook<'a> = &'a mut dyn FnMut(&mut [[f32; 2]]);

/// The strip's controls
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StripSettings {
    /// The GAIN pot's position, 0 to 1 (see [`pot_gain`])
    pub gain: f32,
    pub drive: GainModel,
    /// For a tube: its circuit solved at each sample
    pub realistic: bool,
    pub low_cut: bool,
    /// Hz; off at [`HIGH_PASS_OFF`]
    pub high_pass: f32,
    /// Hz; off at [`LOW_PASS_OFF`]
    pub low_pass: f32,
    /// % mono below the mono bass frequency: 0 is off, 100 all mono
    pub mono_bass: f32,
    /// Hz
    pub mono_bass_frequency: f32,
    pub compressor: CompressorSettings,
    pub gate: GateSettings,
    /// dB: low, low mid, high mid, high
    pub eq: [f32; 4],
}

impl Default for StripSettings {
    /// The neutral strip, which leaves the audio untouched but for the 4558's very low crossover distortion
    fn default() -> Self {
        Self {
            gain: UNITY_POSITION,
            drive: GainModel::OpAmp4558,
            realistic: false,
            low_cut: false,
            high_pass: HIGH_PASS_OFF,
            low_pass: LOW_PASS_OFF,
            mono_bass: 0.0,
            mono_bass_frequency: 120.0,
            compressor: CompressorSettings::default(),
            gate: GateSettings::default(),
            eq: [0.0; 4],
        }
    }
}

/// The channel strip of an analog mixing desk. In signal order:
/// - the GAIN pot, muted to +60 dB, 0 dB at a quarter of its travel;
/// - the low cut button (100 Hz, 18 dB/oct, as on Soundcraft desks), before the gain stage so that it gets no rumble
///   to saturate on;
/// - the gain stage: flat, an op-amp or a tube, of a choice of models (see [`GainStage`]);
/// - sweepable high-pass and low-pass filters (12 dB/oct), and mono bass (see [`MonoBass`]);
/// - the dynamics of an SSL E series channel: a gate/expander and a compressor;
/// - the 4-band EQ of a Soundcraft or Mackie desk (see [`DeskEq`]).
///
/// Each section left at its neutral setting is skipped. The filters match their analog responses up to Nyquist, and
/// ramp their changes over each call.
#[derive(Clone, Debug)]
pub struct ChannelStrip {
    sample_rate: f32,
    gain_stage: GainStage,
    /// Of the last frame, ramped
    gain: f32,
    /// Third order Butterworth: 18 dB/oct
    low_cut2: RampedBiquad,
    low_cut1: RampedBiquad,
    high_pass: RampedBiquad,
    low_pass: RampedBiquad,
    eq: DeskEq,
    mono_bass: MonoBass,
    /// Frames the filters may still ring for after the input fell silent
    tail_frames: u64,
    /// Whether a section run so far in this block rings after its input (settled by the EQ stage, the last)
    block_rings: bool,
    compressor: ChannelCompressor,
    gate: ChannelGate,
}

impl ChannelStrip {
    /// Prepared for @p sample_rate
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            gain_stage: GainStage::new(sample_rate),
            gain: 1.0,
            low_cut2: RampedBiquad::default(),
            low_cut1: RampedBiquad::default(),
            high_pass: RampedBiquad::default(),
            low_pass: RampedBiquad::default(),
            eq: DeskEq::default(),
            mono_bass: MonoBass::default(),
            tail_frames: 0,
            block_rings: false,
            compressor: ChannelCompressor::new(sample_rate),
            gate: ChannelGate::new(sample_rate),
        }
    }

    /// The gain stage, which also measures its saturation for the strip's LEDs
    pub fn gain_stage(&mut self) -> &mut GainStage {
        &mut self.gain_stage
    }

    /// The gate's and the compressor's gain reductions now, dB
    pub fn reductions(&self) -> (f32, f32) {
        (self.gate.reduction(), self.compressor.reduction())
    }

    /// Processes the channel's main pair in place; @p had_input tells whether it had signal. @p hook is called once,
    /// where @p point is in the strip (not for [`InsertPoint::PrePan`], which is after it), with the frames
    pub fn process(
        &mut self,
        frames: &mut [[f32; 2]],
        settings: &StripSettings,
        had_input: bool,
        tolerance: &ChannelTolerance,
        point: InsertPoint,
        mut hook: Option<InsertHook<'_>>,
    ) {
        let mut insert_at = |here: InsertPoint, frames: &mut [[f32; 2]]| {
            if point == here
                && let Some(h) = hook.as_mut()
            {
                h(frames);
            }
        };
        insert_at(InsertPoint::PreGain, frames);
        for stage in 0..STRIP_STAGES {
            self.run_stage(stage, frames, settings, had_input, tolerance);
            insert_at(STAGE_ENDS[stage], frames);
        }
    }

    /// Runs only the strip's stages in @p stages (of [`STRIP_STAGES`]: 0 gain, 1 filters, 2 dynamics, 3 EQ; see
    /// [`stage_after`]), for a rack whose inserts sit between sections and that runs the strip in segments.
    ///
    /// Each block must go through every stage once, in order: running `0..k` and then `k..4` on the same frames
    /// (whatever is done to them between) is bit for bit [`process`](Self::process) with its hook at that point,
    /// for every `k`, the meters and [`holds_signal`](Self::holds_signal) included. Each section's state lives in
    /// its own stage; the only thing a block shares between stages, whether a filter is on (for the tail), is
    /// gathered stage by stage and settled by stage 3, so @p had_input must be the same for every segment of a
    /// block. A range past the strip's end is cut at it. Neither allocates nor locks
    pub fn process_stages(
        &mut self,
        frames: &mut [[f32; 2]],
        settings: &StripSettings,
        had_input: bool,
        tolerance: &ChannelTolerance,
        stages: Range<usize>,
    ) {
        for stage in stages.start.min(STRIP_STAGES)..stages.end.min(STRIP_STAGES) {
            self.run_stage(stage, frames, settings, had_input, tolerance);
        }
    }

    fn run_stage(
        &mut self,
        stage: usize,
        frames: &mut [[f32; 2]],
        settings: &StripSettings,
        had_input: bool,
        tolerance: &ChannelTolerance,
    ) {
        match stage {
            0 => self.gain_section(frames, settings, tolerance),
            1 => self.filter_section(frames, settings, tolerance),
            2 => self.dynamics_section(frames, settings),
            _ => self.eq_section(frames, settings, had_input, tolerance),
        }
    }

    /// How far through a stretch of @p count frames frame @p f is (1 at the last), for the ramps
    #[inline]
    fn progress(f: usize, count: usize) -> f32 {
        if count > 1 { f as f32 / (count - 1) as f32 } else { 1.0 }
    }

    /// Stage 0: the GAIN pot, the low cut and the gain stage
    fn gain_section(&mut self, frames: &mut [[f32; 2]], settings: &StripSettings, tolerance: &ChannelTolerance) {
        let rate64 = f64::from(self.sample_rate);
        let low_cut = settings.low_cut;
        // A section switched off starts afresh when switched on again
        if !low_cut {
            self.low_cut2.reset();
            self.low_cut1.reset();
        }
        let low_cut_frequency = LOW_CUT_FREQUENCY * f64::from(tolerance.low_cut);
        self.low_cut2.set_target(Biquad::high_pass(low_cut_frequency, 1.0, rate64));
        self.low_cut1.set_target(Biquad::high_pass1(low_cut_frequency, rate64));

        let count = frames.len();
        // The GAIN pot, ramped so that turning it does not click
        let gain = pot_gain(settings.gain) * if tolerance.gain != 0.0 { dbfs_to_amp(tolerance.gain) } else { 1.0 };
        if gain != 1.0 || self.gain != 1.0 {
            for (f, frame) in frames.iter_mut().enumerate() {
                let g = self.gain + (gain - self.gain) * Self::progress(f, count);
                frame[0] *= g;
                frame[1] *= g;
            }
        }
        self.gain = gain;

        // The low cut comes before the gain stage, so that the op-amp or the tube gets no rumble to saturate on
        if low_cut {
            for (f, frame) in frames.iter_mut().enumerate() {
                let progress = f64::from(Self::progress(f, count));
                for ch in 0..2 {
                    let x = self.low_cut2.process(f64::from(frame[ch]), ch, progress);
                    frame[ch] = self.low_cut1.process(x, ch, progress) as f32;
                }
            }
        }

        let drive = settings.drive;
        self.gain_stage.process(frames, drive, settings.realistic, tolerance.drive);
        self.low_cut2.finish();
        self.low_cut1.finish();
        // (the tube's coupling rings too)
        self.block_rings |= low_cut || drive.is_tube();
    }

    /// Stage 1: the high-pass, the low-pass and the mono bass
    fn filter_section(&mut self, frames: &mut [[f32; 2]], settings: &StripSettings, tolerance: &ChannelTolerance) {
        let rate = self.sample_rate;
        let rate64 = f64::from(rate);
        let high_pass_on = settings.high_pass > HIGH_PASS_OFF;
        let low_pass_on = settings.low_pass < LOW_PASS_OFF;
        let mono_bass_on =
            self.mono_bass.begin(settings.mono_bass_frequency, (100.0 - settings.mono_bass) / 100.0, rate);
        if !high_pass_on {
            self.high_pass.reset();
        }
        if !low_pass_on {
            self.low_pass.reset();
        }
        self.high_pass.set_target(Biquad::high_pass(
            f64::from(settings.high_pass * tolerance.high_pass),
            BUTTERWORTH_Q,
            rate64,
        ));
        self.low_pass.set_target(Biquad::low_pass(
            f64::from(settings.low_pass * tolerance.low_pass),
            BUTTERWORTH_Q,
            rate64,
        ));

        let count = frames.len();
        if high_pass_on || low_pass_on || mono_bass_on {
            for (f, frame) in frames.iter_mut().enumerate() {
                let progress = Self::progress(f, count);
                for ch in 0..2 {
                    let mut x = frame[ch];
                    if high_pass_on {
                        x = self.high_pass.process(f64::from(x), ch, f64::from(progress)) as f32;
                    }
                    if low_pass_on {
                        x = self.low_pass.process(f64::from(x), ch, f64::from(progress)) as f32;
                    }
                    frame[ch] = x;
                }
                if mono_bass_on {
                    let mut mid = (frame[0] + frame[1]) * 0.5;
                    let mut side = (frame[0] - frame[1]) * 0.5;
                    self.mono_bass.process(&mut mid, &mut side, progress);
                    frame[0] = mid + side;
                    frame[1] = mid - side;
                }
            }
        }
        if mono_bass_on {
            self.mono_bass.end();
        }
        self.high_pass.finish();
        self.low_pass.finish();
        self.block_rings |= high_pass_on || low_pass_on || mono_bass_on;
    }

    /// Stage 2: stereo-linked dynamics, the gate first
    fn dynamics_section(&mut self, frames: &mut [[f32; 2]], settings: &StripSettings) {
        self.gate.begin(settings.gate);
        self.compressor.begin(settings.compressor);
        let gate_on = self.gate.is_active();
        let compressor_on = self.compressor.is_active();
        if !gate_on {
            self.gate.reset();
        }
        if !compressor_on {
            self.compressor.reset();
        }
        if gate_on || compressor_on {
            for frame in frames.iter_mut() {
                let gain = if gate_on { self.gate.gain(frame[0], frame[1]) } else { 1.0 };
                if compressor_on {
                    let (mut left, mut right) = (frame[0] * gain, frame[1] * gain);
                    self.compressor.process(&mut left, &mut right);
                    frame[0] = left;
                    frame[1] = right;
                } else {
                    frame[0] *= gain;
                    frame[1] *= gain;
                }
            }
        }
    }

    /// Stage 3: the EQ; and the end of the block: how long the filters may ring on
    fn eq_section(
        &mut self,
        frames: &mut [[f32; 2]],
        settings: &StripSettings,
        had_input: bool,
        tolerance: &ChannelTolerance,
    ) {
        self.eq.begin(settings.eq, f64::from(self.sample_rate), tolerance.eq_frequency, tolerance.eq_gain);
        let eq_on = self.eq.is_active();
        let count = frames.len();
        if eq_on {
            for (f, frame) in frames.iter_mut().enumerate() {
                let progress = f64::from(Self::progress(f, count));
                for ch in 0..2 {
                    frame[ch] = self.eq.process(f64::from(frame[ch]), ch, progress) as f32;
                }
            }
        }
        self.eq.finish();

        // After the input falls silent, the filters ring out for a while
        let any_filter = std::mem::take(&mut self.block_rings) || eq_on;
        if !any_filter {
            self.tail_frames = 0;
        } else if had_input {
            self.tail_frames = (FILTER_TAIL * self.sample_rate) as u64;
        } else {
            self.tail_frames = self.tail_frames.saturating_sub(count as u64);
        }
    }

    /// Whether the filters may still ring after the input fell silent, so the channel must be processed
    pub fn holds_signal(&self) -> bool {
        self.tail_frames > 0
    }

    /// Forgets the filters' and the dynamics' state, e.g. while the channel is muted
    pub fn reset(&mut self) {
        for filter in [&mut self.low_cut2, &mut self.low_cut1, &mut self.high_pass, &mut self.low_pass] {
            filter.reset();
        }
        self.eq.reset();
        self.tail_frames = 0;
        self.block_rings = false;
        self.mono_bass.reset();
        self.gain_stage.reset();
        self.gate.reset();
        self.compressor.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pot_is_muted_to_the_left_and_unity_at_a_quarter() {
        assert_eq!(pot_gain(0.0), 0.0);
        assert_eq!(pot_gain(UNITY_POSITION), 1.0);
        assert!((crate::math::amp_to_dbfs(pot_gain(1.0)) - MAX_GAIN).abs() < 1e-3);
        assert!((crate::math::amp_to_dbfs(pot_gain(UNITY_POSITION / 2.0)) + 18.06).abs() < 0.01);
    }

    #[test]
    fn a_flat_neutral_strip_leaves_the_audio_untouched() {
        let mut strip = ChannelStrip::new(48000.0);
        let settings = StripSettings { drive: GainModel::Flat, ..StripSettings::default() };
        let mut frames: Vec<[f32; 2]> = (0..256).map(|n| [n as f32 / 256.0 - 0.5, 0.25]).collect();
        let input = frames.clone();
        strip.process(&mut frames, &settings, true, &ChannelTolerance::NONE, InsertPoint::PostEq, None);
        assert_eq!(frames, input);
    }

    #[test]
    fn the_hook_is_called_where_the_effects_go() {
        let mut strip = ChannelStrip::new(48000.0);
        let mut frames = vec![[0.1f32, 0.1]; 64];
        let mut called = 0;
        let mut hook = |f: &mut [[f32; 2]]| {
            called += 1;
            assert_eq!(f.len(), 64);
        };
        strip.process(
            &mut frames,
            &StripSettings::default(),
            true,
            &ChannelTolerance::NONE,
            InsertPoint::PostFilter,
            Some(&mut hook),
        );
        assert_eq!(called, 1);
    }
}
