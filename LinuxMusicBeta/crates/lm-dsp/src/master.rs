//! The master section of a recording studio's desk (legacy `MasterSection`), its DSP: the PRE faders, the summing
//! amplifier, the stereo section, the bus compressor, the output transformer, the monitoring and the correlation.

use crate::gain_stage::{GainModel, GainStage};
use crate::math::{amp_to_dbfs, dbfs_to_amp, one_pole};
use crate::mono_bass::MonoBass;
use crate::rbj::{CrossfadedBiquad, Shape};
use crate::tolerance::{OutputTransformer, TransformerCore};

/// The PRE faders, one per side, linear like the channels' faders: the mix's level into the master's processing,
/// ramped over each call
#[derive(Clone, Debug)]
pub struct PreFaders {
    /// The gains of the last frame
    gain: [f32; 2],
}

impl Default for PreFaders {
    fn default() -> Self {
        Self { gain: [1.0; 2] }
    }
}

impl PreFaders {
    pub fn process(&mut self, frames: &mut [[f32; 2]], target: [f32; 2]) {
        if target == self.gain && target == [1.0, 1.0] {
            return;
        }
        let count = frames.len().max(1) as f32;
        for ch in 0..2 {
            let step = (target[ch] - self.gain[ch]) / count;
            let mut gain = self.gain[ch];
            for frame in frames.iter_mut() {
                gain += step;
                frame[ch] *= gain;
            }
        }
        self.gain = target;
    }
}

/// The stereo section's controls
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StereoSettings {
    /// The side's level, % (100 is as it is, 0 mono)
    pub width: f32,
    /// % mono below the mono bass frequency: 0 is off, 100 all mono
    pub mono_bass: f32,
    /// Hz
    pub bass_frequency: f32,
    /// dB of the side's high shelf
    pub air: f32,
}

impl Default for StereoSettings {
    fn default() -> Self {
        Self { width: 100.0, mono_bass: 0.0, bass_frequency: 120.0, air: 0.0 }
    }
}

/// The stereo section, in mid/side (M = (L+R)/2, S = (L-R)/2), before the bus compressor:
/// - WIDTH: the side's level, ramped over each call;
/// - MONO BASS: below its frequency, the side is narrowed to its width (0 = mono), see [`MonoBass`];
/// - AIR: a high shelf on the side only, opening the mix without brightening its center.
///
/// Each part at its neutral setting is skipped.
#[derive(Clone, Debug)]
pub struct StereoSection {
    sample_rate: f32,
    mono_bass: MonoBass,
    /// Of the last frame, ramped
    width: f32,
    air: CrossfadedBiquad,
    air_was_on: bool,
}

impl StereoSection {
    pub const AIR_FREQUENCY: f32 = 12000.0;

    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            mono_bass: MonoBass::default(),
            width: 1.0,
            air: CrossfadedBiquad::new(Shape::HighShelf),
            air_was_on: false,
        }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], settings: &StereoSettings) {
        let width = settings.width / 100.0;
        let bass_active =
            self.mono_bass.begin(settings.bass_frequency, (100.0 - settings.mono_bass) / 100.0, self.sample_rate);
        let air = settings.air;
        // One more call after the air is back at 0 dB, so that the shelf fades out
        let air_active = air != 0.0 || self.air_was_on;
        self.air_was_on = air != 0.0;

        if width == 1.0 && self.width == 1.0 && !bass_active && !air_active {
            self.air.clear();
            return;
        }
        if air_active {
            self.air.set_parameters(self.sample_rate, Self::AIR_FREQUENCY, core::f32::consts::FRAC_1_SQRT_2, air);
        }

        let count = frames.len();
        for (f, frame) in frames.iter_mut().enumerate() {
            let progress = (f + 1) as f32 / count as f32;
            let mut mid = (frame[0] + frame[1]) * 0.5;
            let mut side = (frame[0] - frame[1]) * 0.5;
            if bass_active {
                self.mono_bass.process(&mut mid, &mut side, progress);
            }
            if air_active {
                side = self.air.process(side, 0, progress);
            }
            side *= self.width + (width - self.width) * progress;
            frame[0] = mid + side;
            frame[1] = mid - side;
        }

        if bass_active {
            self.mono_bass.end();
        }
        self.width = width;
    }

    pub fn reset(&mut self) {
        self.mono_bass.reset();
        self.air.clear();
        self.air_was_on = false;
    }
}

/// The bus compressor's ratio switch
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BusRatio {
    Two,
    Four,
    Ten,
}

impl BusRatio {
    pub fn value(self) -> f32 {
        match self {
            BusRatio::Two => 2.0,
            BusRatio::Four => 4.0,
            BusRatio::Ten => 10.0,
        }
    }

    /// The knee's width: the curve is rounder at the lower ratios, almost hard at 10:1 (as measured)
    pub fn knee_width(self) -> f32 {
        match self {
            BusRatio::Two => 10.0,
            BusRatio::Four => 6.0,
            BusRatio::Ten => 2.0,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            BusRatio::Two => "2",
            BusRatio::Four => "4",
            BusRatio::Ten => "10",
        }
    }
}

/// The bus compressor's controls, with the SSL G-series panel (Thompson and Heilemann, 2026, measuring a G384)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BusCompressorSettings {
    /// In or out
    pub on: bool,
    /// dB on the panel's scale, -15 to +15: 0 is [`BusCompressor::THRESHOLD_REFERENCE`] dBFS
    pub threshold: f32,
    pub ratio: BusRatio,
    /// An index of [`BusCompressor::ATTACK_TIMES`]
    pub attack: usize,
    /// An index of [`BusCompressor::RELEASE_TIMES`], or [`BusCompressor::AUTO_RELEASE`]
    pub release: usize,
    /// dB, -5 to +15
    pub makeup: f32,
}

impl Default for BusCompressorSettings {
    /// The panel's defaults
    fn default() -> Self {
        Self {
            on: false,
            threshold: 5.0,
            ratio: BusRatio::Four,
            attack: 4,
            release: BusCompressor::AUTO_RELEASE,
            makeup: 0.0,
        }
    }
}

/// A bus compressor modelled on the SSL G-series one. Its sidechain VCA gets the same control as the audio VCA (less
/// the make-up, an offset on the audio VCA only), so the detector hears the compressed signal: a feedback loop,
/// which with strength r - 1 holds the static ratio at r
#[derive(Clone, Debug)]
pub struct BusCompressor {
    sample_rate: f32,
    /// dB, the control voltage (the fast pole with Auto)
    reduction: f32,
    /// dB, Auto's slow pole, which follows the fast one
    slow_reduction: f32,
    /// The deepest reduction since it was last read, dB (0 or less)
    deepest: f32,
}

impl BusCompressor {
    /// dBFS at the threshold's 0
    pub const THRESHOLD_REFERENCE: f32 = -15.0;
    /// ms
    pub const ATTACK_TIMES: [f32; 6] = [0.1, 0.3, 1.0, 3.0, 10.0, 30.0];
    /// s; then Auto
    pub const RELEASE_TIMES: [f32; 4] = [0.1, 0.3, 0.6, 1.2];
    /// The release's last step
    pub const AUTO_RELEASE: usize = 4;
    /// s, Auto's fast pole
    const AUTO_FAST_RELEASE: f32 = 0.1;
    /// s, Auto's slow pole
    const AUTO_SLOW_TIME: f32 = 1.2;

    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate, reduction: 0.0, slow_reduction: 0.0, deepest: 0.0 }
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]], settings: &BusCompressorSettings) {
        if !settings.on {
            self.reduction = 0.0;
            self.slow_reduction = 0.0;
            return;
        }
        let rate = self.sample_rate;
        let strength = settings.ratio.value() - 1.0;
        let knee = settings.ratio.knee_width();
        let threshold = Self::THRESHOLD_REFERENCE + settings.threshold;
        let attack_step = settings.attack.min(Self::ATTACK_TIMES.len() - 1);
        let release_step = settings.release.min(Self::AUTO_RELEASE);
        let auto_release = release_step == Self::AUTO_RELEASE;
        let attack = one_pole(Self::ATTACK_TIMES[attack_step] / 1000.0, rate);
        let release =
            one_pole(if auto_release { Self::AUTO_FAST_RELEASE } else { Self::RELEASE_TIMES[release_step] }, rate);
        let slow = one_pole(Self::AUTO_SLOW_TIME, rate);
        let makeup = settings.makeup;

        for frame in frames.iter_mut() {
            // What the detector hears: the louder side, through the sidechain VCA (the reduction so far)
            let level = frame[0].abs().max(frame[1].abs());
            let heard = amp_to_dbfs(level.max(1e-9)) + self.reduction.min(self.slow_reduction);
            let over = heard - threshold;
            let mut target = 0.0;
            if over >= knee / 2.0 {
                target = -strength * over;
            } else if over > -knee / 2.0 {
                let into = over + knee / 2.0;
                target = -strength * into * into / (2.0 * knee);
            }

            // The control voltage: the attack's time constant down, the release's up; with Auto, a second, slow pole
            // follows it, and the deeper of the two holds: short peaks recover fast, sustained compression slowly
            self.reduction += (target - self.reduction) * if target < self.reduction { attack } else { release };
            if auto_release {
                self.slow_reduction += (self.reduction - self.slow_reduction) * slow;
            } else {
                self.slow_reduction = self.reduction;
            }
            let reduction = self.reduction.min(self.slow_reduction);

            let gain = dbfs_to_amp(reduction + makeup);
            frame[0] *= gain;
            frame[1] *= gain;
            self.deepest = self.deepest.min(reduction);
        }
    }

    /// The deepest gain reduction since the last call, in dB (0 or less), for its meter
    pub fn take_gain_reduction(&mut self) -> f32 {
        core::mem::take(&mut self.deepest)
    }

    pub fn reset(&mut self) {
        self.reduction = 0.0;
        self.slow_reduction = 0.0;
        self.deepest = 0.0;
    }
}

/// The master's gain stage (the summing amplifier, in analog mode) and its output transformer, Neve style
#[derive(Clone, Debug)]
pub struct MasterAnalog {
    gain_stage: GainStage,
    transformer: OutputTransformer,
    transformer_was_on: bool,
}

impl MasterAnalog {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            gain_stage: GainStage::new(sample_rate),
            transformer: OutputTransformer::new(TransformerCore::default(), f64::from(sample_rate)),
            transformer_was_on: false,
        }
    }

    /// The gain stage, after the PRE faders, which drive it
    pub fn process_drive(&mut self, frames: &mut [[f32; 2]], model: GainModel, realistic: bool) {
        self.gain_stage.process(frames, model, realistic, 1.0);
    }

    /// The gain stage, which also measures its saturation for its LEDs
    pub fn gain_stage(&mut self) -> &mut GainStage {
        &mut self.gain_stage
    }

    /// The transformer, after the master's fader
    pub fn process_transformer(&mut self, frames: &mut [[f32; 2]]) {
        if !self.transformer_was_on {
            self.transformer.reset();
        }
        self.transformer_was_on = true;
        for frame in frames.iter_mut() {
            frame[0] = self.transformer.process(frame[0], 0);
            frame[1] = self.transformer.process(frame[1], 1);
        }
    }

    /// While it is out: it starts afresh when it is in again
    pub fn bypass_transformer(&mut self) {
        self.transformer_was_on = false;
    }

    pub fn reset(&mut self) {
        self.gain_stage.reset();
        self.transformer.reset();
        self.transformer_was_on = false;
    }
}

/// The monitoring: what is heard from the speakers, without changing the mix or the export. Listening states, so
/// neither saved with the project nor undoable
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MonitorSettings {
    /// dB
    pub level: f32,
    /// -20 dB
    pub dim: bool,
    pub cut: bool,
    /// Listen to the mid (L+R)/2
    pub mono: bool,
    /// Listen to the side (L-R)/2; MONO and SIDE exclude each other
    pub side: bool,
}

/// The monitoring's gain, ramped so that pressing DIM or CUT does not click
#[derive(Clone, Debug)]
pub struct Monitoring {
    gain: f32,
}

impl Default for Monitoring {
    fn default() -> Self {
        Self { gain: 1.0 }
    }
}

impl Monitoring {
    pub const DIM_LEVEL: f32 = -20.0;

    pub fn process(&mut self, frames: &mut [[f32; 2]], settings: &MonitorSettings) {
        let target = if settings.cut {
            0.0
        } else {
            dbfs_to_amp(settings.level + if settings.dim { Self::DIM_LEVEL } else { 0.0 })
        };
        if target == 1.0 && self.gain == 1.0 && !settings.mono && !settings.side {
            return;
        }
        let step = (target - self.gain) / frames.len().max(1) as f32;
        for frame in frames.iter_mut() {
            self.gain += step;
            if settings.mono || settings.side {
                // The mid or the side on both speakers, in phase
                let sum = (frame[0] + if settings.side { -frame[1] } else { frame[1] }) * 0.5;
                frame[0] = sum;
                frame[1] = sum;
            }
            frame[0] *= self.gain;
            frame[1] *= self.gain;
        }
        self.gain = target;
    }
}

/// The correlation of the left and right channels over the last ~300 ms: 1 mono, 0 unrelated, -1 cancelling each
/// other
#[derive(Clone, Debug, Default)]
pub struct Correlation {
    sum_lr: f32,
    sum_ll: f32,
    sum_rr: f32,
}

impl Correlation {
    /// s
    const TIME: f32 = 0.3;

    /// Measures @p frames and gives the correlation after them
    pub fn measure(&mut self, frames: &[[f32; 2]], sample_rate: f32) -> f32 {
        let coefficient = one_pole(Self::TIME, sample_rate);
        for frame in frames {
            self.sum_lr += (frame[0] * frame[1] - self.sum_lr) * coefficient;
            self.sum_ll += (frame[0] * frame[0] - self.sum_ll) * coefficient;
            self.sum_rr += (frame[1] * frame[1] - self.sum_rr) * coefficient;
        }
        let power = (self.sum_ll * self.sum_rr).sqrt();
        if power > 1e-12 { (self.sum_lr / power).clamp(-1.0, 1.0) } else { 0.0 }
    }

    /// The master is silent
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_width_makes_mono() {
        let mut stereo = StereoSection::new(48000.0);
        let settings = StereoSettings { width: 0.0, ..StereoSettings::default() };
        let mut frames = vec![[0.5f32, -0.25]; 64];
        stereo.process(&mut frames, &settings);
        let mut frames = vec![[0.5f32, -0.25]; 64];
        stereo.process(&mut frames, &settings);
        for f in &frames {
            assert!((f[0] - f[1]).abs() < 1e-6);
        }
    }

    #[test]
    fn bus_compressor_squeezes_the_mix() {
        let mut c = BusCompressor::new(48000.0);
        let settings = BusCompressorSettings { on: true, threshold: -10.0, ..BusCompressorSettings::default() };
        let mut frames = vec![[0.9f32, 0.9]; 48000];
        c.process(&mut frames, &settings);
        assert!(frames[47999][0] < 0.5);
        assert!(c.take_gain_reduction() < -3.0);
    }

    #[test]
    fn correlation_follows_the_phase() {
        let mut c = Correlation::default();
        let frames: Vec<[f32; 2]> = (0..48000)
            .map(|n| {
                let x = crate::math::sinf(n as f32 * 0.05);
                [x, -x]
            })
            .collect();
        assert!(c.measure(&frames, 48000.0) < -0.99);
    }
}
