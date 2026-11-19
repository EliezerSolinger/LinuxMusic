//! The gain stage of an analog desk, of a choice of op-amps and tubes (legacy `GainStage`).

use std::sync::OnceLock;

use crate::math::{amp_to_dbfs, f32c::PI, tanhf};
use crate::oversampling::{Downsampler, Upsampler};
use crate::rbj::{CrossfadedBiquad, Shape};
use crate::tube::{Triode, TubeCircuit, TubeModel, TubeStage};

/// 4 times
const OVERSAMPLING_STAGES: usize = 2;
const OVERSAMPLING: usize = 1 << OVERSAMPLING_STAGES;
/// Volts per unit of the signal: an op-amp's rail of +24 dBFS is a swing of 13 V on a +-15 V supply
const VOLTS_PER_UNIT: f32 = 13.0 / 16.0;
const DC_BLOCKER_FREQUENCY: f32 = 5.0;
/// The CLIP LED lights this close to the limit (-3 dB)
const CLIP_WARNING: f32 = 0.708;
/// The SAT LED of a tube spans this range, around where it starts to clip (dB)
const TUBE_COLOR_RANGE: f32 = 12.0;

/// The models. Their keys are saved with projects and never change (interfaces.md, U30).
/// - Flat: nothing, the gain is clean.
/// - Op-amps, saturating softly from their knee towards their rails: the headroom of a desk above its nominal level,
///   so they are clean at usual levels and saturate when the gain is pushed (datasheet slew rates; knee and rails
///   approximate each chip's output swing):
///   - 4558, of many Soundcraft and Mackie desks: knee at +18 dBFS, the negative rail 5% lower (even harmonics),
///     slow (1.7 V/us), its swing giving way a little into 600 ohms, and the crossover distortion of its class AB
///     output stage, which its feedback reduces less the higher the frequency (always there, very low);
///   - TL072, JFET input, of Mackie desks and much studio gear: knee at +19 dBFS, rails a little lower and slightly
///     asymmetric, 13 V/us, weak into 600 ohms, and its output flipping to the positive rail when its input goes
///     past its common mode range (+22.6 dBFS negative peaks): its ugly overload;
///   - NE5532, the dual NE5534, the op-amp of most clean desks (SSL, Neve, Amek, Mackie Onyx): knee at +21 dBFS,
///     symmetric rails, 9 V/us, made to drive 600 ohms;
///   - NE5534, the single one: as the NE5532, 13 V/us.
/// - Tubes, preamp triodes in a common cathode stage (Norman Koren's model, see [`TubeStage`]), driven in volts:
///   with the GAIN pot in the middle, the 12AX7 starts to clip at 0 dBFS, so it colors from -6 dBFS; the others clip
///   at their own levels:
///   - 12AX7, the usual stage;
///   - 12AX7 V (vintage): biased cold, clipping first towards cutoff, rich in even harmonics;
///   - 12AX7 H (hot): biased hot on a higher supply, clipping alike on both sides, stronger in odd harmonics;
///   - 12AT7 and 12AU7: less gain, so more headroom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GainModel {
    Flat,
    OpAmp4558,
    OpAmp5532,
    Tube12AX7,
    Tube12AX7Vintage,
    Tube12AX7Hot,
    OpAmp5534,
    OpAmpTL072,
    Tube12AT7,
    Tube12AU7,
}

impl GainModel {
    /// The order the models are offered in: flat, the op-amps, the tubes
    pub const DISPLAY_ORDER: [GainModel; 10] = [
        GainModel::Flat,
        GainModel::OpAmp4558,
        GainModel::OpAmpTL072,
        GainModel::OpAmp5532,
        GainModel::OpAmp5534,
        GainModel::Tube12AX7,
        GainModel::Tube12AX7Vintage,
        GainModel::Tube12AX7Hot,
        GainModel::Tube12AT7,
        GainModel::Tube12AU7,
    ];

    /// The permanent key of the model, as saved in projects
    pub fn key(self) -> &'static str {
        match self {
            GainModel::Flat => "flat",
            GainModel::OpAmp4558 => "4558",
            GainModel::OpAmp5532 => "5532",
            GainModel::Tube12AX7 => "12ax7",
            GainModel::Tube12AX7Vintage => "12ax7-vintage",
            GainModel::Tube12AX7Hot => "12ax7-hot",
            GainModel::OpAmp5534 => "5534",
            GainModel::OpAmpTL072 => "tl072",
            GainModel::Tube12AT7 => "12at7",
            GainModel::Tube12AU7 => "12au7",
        }
    }

    pub fn from_key(key: &str) -> Option<GainModel> {
        Self::DISPLAY_ORDER.into_iter().find(|m| m.key() == key)
    }

    pub fn is_tube(self) -> bool {
        matches!(
            self,
            GainModel::Tube12AX7
                | GainModel::Tube12AX7Vintage
                | GainModel::Tube12AX7Hot
                | GainModel::Tube12AT7
                | GainModel::Tube12AU7
        )
    }

    pub fn is_op_amp(self) -> bool {
        matches!(self, GainModel::OpAmp4558 | GainModel::OpAmp5532 | GainModel::OpAmp5534 | GainModel::OpAmpTL072)
    }
}

/// What an op-amp model is like
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpAmpSpec {
    /// Where it starts to saturate, linear
    pub knee: f32,
    /// Where it saturates towards with no load, linear
    pub rail: f32,
    /// The negative side's knee and rail relative to the positive one's
    pub negative_rail_ratio: f32,
    /// V/us
    pub slew_rate: f32,
    /// A it can drive before its swing gives way
    pub output_current: f32,
    /// Hz
    pub gain_bandwidth: f32,
    /// V of its output stage's dead band (class AB), 0 for none worth modelling
    pub crossover: f32,
    /// Below this input (linear) its output flips to the positive rail; 0 for none
    pub common_mode_limit: f32,
}

/// The load the op-amps drive, as a desk's line driver does: 600 ohms
pub const LOAD: f32 = 600.0;
/// Where FLAT would clip on a desk, for its CLIP LED (+24 dBFS, an op-amp's rail)
pub const FLAT_CLIP_LEVEL: f32 = 16.0;

pub fn op_amp_spec(model: GainModel) -> OpAmpSpec {
    // Knee, rail, negative side, slew rate, output current into 600 ohms, gain bandwidth, crossover dead band, common
    // mode limit (V- + 4 V on +-15 V, in units)
    let spec =
        |knee, rail, negative_rail_ratio, slew_rate, output_current, gain_bandwidth, crossover, limit| OpAmpSpec {
            knee,
            rail,
            negative_rail_ratio,
            slew_rate,
            output_current,
            gain_bandwidth,
            crossover,
            common_mode_limit: limit,
        };
    match model {
        GainModel::OpAmp5532 => spec(11.22, 16.0, 1.0, 9.0, 0.026, 10e6, 0.0, 0.0), // knee at +21 dBFS
        GainModel::OpAmp5534 => spec(11.22, 16.0, 1.0, 13.0, 0.026, 10e6, 0.0, 0.0),
        // Knee at +19 dBFS, rail at +23.4 dBFS
        GainModel::OpAmpTL072 => spec(8.91, 14.8, 0.97, 13.0, 0.015, 3e6, 0.0, -11.0 / VOLTS_PER_UNIT),
        _ => spec(8.0, 16.0, 0.95, 1.7, 0.02, 3e6, 0.04, 0.0), // 4558: knee at +18 dBFS
    }
}

/// Where an op-amp's output saturates towards, linear: its rail, or lower if it cannot drive the load that far
pub fn op_amp_rail(model: GainModel) -> f32 {
    let spec = op_amp_spec(model);
    spec.rail.min(spec.output_current * LOAD / VOLTS_PER_UNIT)
}

/// The op-amp's transfer function: linear up to the knee, then softly towards the rail (see [`op_amp_rail`]); for
/// the TL072, the output flips to the positive rail when the input goes past its common mode range
pub fn op_amp(model: GainModel, sample: f32) -> f32 {
    let spec = op_amp_spec(model);
    let rail = op_amp_rail(model);
    if spec.common_mode_limit != 0.0 && sample < spec.common_mode_limit {
        return rail; // phase reversal
    }
    let scale = if sample < 0.0 { spec.negative_rail_ratio } else { 1.0 };
    let knee = spec.knee * scale;
    let magnitude = sample.abs();
    if magnitude <= knee {
        return sample;
    }
    let span = (rail - spec.knee) * scale;
    (knee + span * tanhf((magnitude - knee) / span)).copysign(sample)
}

/// The tubes' stages, built once for the whole program (when the first gain stage is prepared)
struct TubeModels {
    ax7: TubeModel,
    ax7_cold: TubeModel,
    ax7_hot: TubeModel,
    at7: TubeModel,
    au7: TubeModel,
    /// Volts at the grid per unit of the signal with the GAIN pot in the middle: the 12AX7 starts to clip at
    /// 0 dBFS, so it colors from -6 dBFS; the other tubes, at their own levels
    volts_per_unit: f32,
}

/// The tubes, in the common cathode stage of [`TubeCircuit`]. Koren's parameters (mu, ex, kg1, kp, kvb) and the
/// datasheets' capacitances (grid to plate, grid to cathode); usual plate and cathode resistors for each tube. The
/// 12AX7 V and H are the 12AX7 biased cold (a larger cathode resistor: it clips first towards cutoff, rich in even
/// harmonics) and hot on a higher supply (a smaller cathode resistor: both sides clip alike, stronger in odd
/// harmonics).
fn tube_models() -> &'static TubeModels {
    static MODELS: OnceLock<TubeModels> = OnceLock::new();
    MODELS.get_or_init(|| {
        let triode = |mu, ex, kg1, kp, kvb, grid_to_plate, grid_to_cathode| Triode {
            mu,
            ex,
            kg1,
            kp,
            kvb,
            grid_to_plate,
            grid_to_cathode,
        };
        let ax7 = triode(100.0, 1.4, 1060.0, 600.0, 300.0, 1.7e-12, 1.6e-12);
        let at7 = triode(60.0, 1.35, 460.0, 300.0, 300.0, 1.5e-12, 2.2e-12);
        let au7 = triode(21.5, 1.3, 1180.0, 84.0, 300.0, 1.5e-12, 1.6e-12);
        let ax7_model = TubeModel::new(TubeCircuit::new(ax7, 250.0, 100e3, 1.5e3, 22e-6));
        let volts_per_unit = ax7_model.headroom as f32;
        TubeModels {
            ax7: ax7_model,
            ax7_cold: TubeModel::new(TubeCircuit::new(ax7, 250.0, 100e3, 2.7e3, 22e-6)),
            ax7_hot: TubeModel::new(TubeCircuit::new(ax7, 330.0, 100e3, 1.0e3, 22e-6)),
            at7: TubeModel::new(TubeCircuit::new(at7, 250.0, 47e3, 820.0, 22e-6)),
            au7: TubeModel::new(TubeCircuit::new(au7, 250.0, 47e3, 1.5e3, 22e-6)),
            volts_per_unit,
        }
    })
}

fn tube_model(model: GainModel) -> &'static TubeModel {
    let m = tube_models();
    match model {
        GainModel::Tube12AX7Vintage => &m.ax7_cold,
        GainModel::Tube12AX7Hot => &m.ax7_hot,
        GainModel::Tube12AT7 => &m.at7,
        GainModel::Tube12AU7 => &m.au7,
        _ => &m.ax7,
    }
}

/// The tube's static curve, with unity gain and the same polarity for small signals, and no offset
pub fn tube(model: GainModel, sample: f32) -> f32 {
    let stage = tube_model(model);
    let volts = tube_models().volts_per_unit;
    stage.at(sample * volts).plate / (stage.gain * f64::from(volts)) as f32
}

/// Where a tube starts to clip, linear, with the GAIN pot in the middle (0 dBFS for the 12AX7)
pub fn tube_clip_level(model: GainModel) -> f32 {
    tube_model(model).headroom as f32 / tube_models().volts_per_unit
}

/// The gain stage of an analog desk, used by the channels' strips and the master. It works at 4 times the sample
/// rate, so that its harmonics do not alias:
/// - op-amps: only the part the op-amp changes (its output minus its input, which is 0 below the knee) goes through
///   the oversampling, then is added to the signal, so below the knee the signal is untouched, bit for bit; blocks
///   far below the knee skip the oversampling. The op-amp's slew rate limits very loud, very high frequencies.
/// - tubes: the whole signal goes through a tube stage, in volts at its grid: the GAIN pot sets how hard it is
///   driven, and its output is brought back to unity gain. Either light (its curve, with the Miller capacitance,
///   blocking and sag approximated around it, then a DC blocker) or realistic (its circuit solved at each sample).
///
/// It measures how far it saturates and when it nears its limit, for the SAT and CLIP LEDs.
#[derive(Clone, Debug)]
pub struct GainStage {
    sample_rate: f32,
    upsamplers: [Upsampler<OVERSAMPLING_STAGES>; 2],
    downsamplers: [Downsampler<OVERSAMPLING_STAGES>; 2],
    /// The op-amp's output, which the slew rate limits
    slewed: [f32; 2],
    /// Of the last frame
    crossover_error: [f32; 2],
    dc_blocker: CrossfadedBiquad,
    /// Whether the oversamplers carry signal
    oversampling: bool,
    /// What they last carried
    model: GainModel,
    realistic: bool,
    tubes: [TubeStage; 2],
    saturation: f32,
    clip: bool,
}

impl GainStage {
    /// Prepared for @p sample_rate. The first one made also works out the tubes' curves, which takes a while.
    pub fn new(sample_rate: f32) -> Self {
        let tube = TubeStage::new(tube_model(GainModel::Tube12AX7), f64::from(sample_rate) * OVERSAMPLING as f64);
        let mut dc_blocker = CrossfadedBiquad::new(Shape::HighPass);
        dc_blocker.set_parameters(sample_rate, DC_BLOCKER_FREQUENCY, core::f32::consts::FRAC_1_SQRT_2, 1.0);
        Self {
            sample_rate,
            upsamplers: [Upsampler::new(sample_rate), Upsampler::new(sample_rate)],
            downsamplers: [Downsampler::new(sample_rate), Downsampler::new(sample_rate)],
            slewed: [0.0; 2],
            crossover_error: [0.0; 2],
            dc_blocker,
            oversampling: false,
            model: GainModel::Flat,
            realistic: false,
            tubes: [tube.clone(), tube],
            saturation: 0.0,
            clip: false,
        }
    }

    /// Forgets the oversamplers', the DC blocker's and the tubes' state
    pub fn reset(&mut self) {
        self.clear();
        self.slewed = [0.0; 2];
        self.crossover_error = [0.0; 2];
    }

    fn clear(&mut self) {
        for u in &mut self.upsamplers {
            u.reset();
        }
        for d in &mut self.downsamplers {
            d.reset();
        }
        self.dc_blocker.clear();
        if self.model.is_tube() {
            let model = tube_model(self.model);
            for t in &mut self.tubes {
                t.reset(model);
            }
        }
        self.oversampling = false;
    }

    fn use_model(&mut self, next: GainModel, realistic: bool) {
        if next == self.model && realistic == self.realistic {
            return;
        }
        self.model = next;
        self.realistic = realistic;
        self.crossover_error = [0.0; 2];
        if next.is_tube() {
            // No allocation: a tube stage is a few small matrices
            let stage = TubeStage::new(tube_model(next), f64::from(self.sample_rate) * OVERSAMPLING as f64);
            self.tubes = [stage.clone(), stage];
        }
        self.clear();
    }

    /// Processes both channels of @p frames with @p model, and measures them for the LEDs.
    /// @p realistic: for a tube, its circuit solved at each sample rather than approximated around its curve;
    /// @p drive: a factor on its input, undone at its output: where it saturates (a channel's tolerance)
    pub fn process(&mut self, frames: &mut [[f32; 2]], model: GainModel, realistic: bool, drive: f32) {
        if drive != 1.0 && model != GainModel::Flat {
            for frame in frames.iter_mut() {
                frame[0] *= drive;
                frame[1] *= drive;
            }
            self.process(frames, model, realistic, 1.0);
            let undo = 1.0 / drive;
            for frame in frames.iter_mut() {
                frame[0] *= undo;
                frame[1] *= undo;
            }
            return;
        }

        self.use_model(model, realistic && model.is_tube());

        let peak = frames.iter().fold(0.0f32, |p, f| p.max(f[0].abs()).max(f[1].abs()));
        self.measure(peak, model);
        if model == GainModel::Flat {
            return;
        }

        let mut oversampled = [0.0f32; 8];
        if model.is_tube() {
            // In volts at the grid, and back to unity gain and the same polarity
            let stage = tube_model(model);
            let volts = tube_models().volts_per_unit;
            let back = 1.0 / (stage.gain * f64::from(volts)) as f32;
            for frame in frames.iter_mut() {
                for ch in 0..2 {
                    self.upsamplers[ch].process(&mut oversampled, frame[ch]);
                    let tube = &mut self.tubes[ch];
                    if self.realistic {
                        // The circuit's coupling capacitors take the offset away
                        for sample in &mut oversampled[..OVERSAMPLING] {
                            *sample = tube.process_circuit(stage, *sample * volts) * back;
                        }
                        frame[ch] = self.downsamplers[ch].process(&mut oversampled);
                    } else {
                        for sample in &mut oversampled[..OVERSAMPLING] {
                            *sample = tube.process_light(stage, *sample * volts) * back;
                        }
                        let down = self.downsamplers[ch].process(&mut oversampled);
                        frame[ch] = self.dc_blocker.process(down, ch, 1.0);
                    }
                }
            }
            self.oversampling = true;
            return;
        }

        let spec = op_amp_spec(model);
        if spec.crossover > 0.0 {
            self.process_crossover(frames, &spec);
        }

        // Op-amp: far below the knee, nothing more to do
        if peak < spec.knee * spec.negative_rail_ratio / 4.0 {
            if self.oversampling {
                self.clear();
            }
            if let Some(last) = frames.last() {
                self.slewed = *last;
            }
            return;
        }

        // The slew rate in units per oversampled frame
        let max_step = spec.slew_rate * 1e6 / VOLTS_PER_UNIT / (self.sample_rate * OVERSAMPLING as f32);
        for frame in frames.iter_mut() {
            for ch in 0..2 {
                // What the op-amp changes, at the oversampled rate
                self.upsamplers[ch].process(&mut oversampled, frame[ch]);
                for sample in &mut oversampled[..OVERSAMPLING] {
                    let target = op_amp(model, *sample);
                    self.slewed[ch] += (target - self.slewed[ch]).clamp(-max_step, max_step);
                    *sample = self.slewed[ch] - *sample;
                }
                frame[ch] += self.downsamplers[ch].process(&mut oversampled);
            }
        }
        self.oversampling = true;
    }

    fn measure(&mut self, peak: f32, model: GainModel) {
        let mut saturation = 0.0f32;
        let mut limit = FLAT_CLIP_LEVEL;
        if model.is_op_amp() {
            let spec = op_amp_spec(model);
            let knee = spec.knee * spec.negative_rail_ratio;
            let rail = op_amp_rail(model) * spec.negative_rail_ratio;
            saturation = ((peak - knee) / (rail - knee)).clamp(0.0, 1.0);
            limit = rail;
        } else if model.is_tube() {
            // From 6 dB below where it starts to clip to 6 dB above, where it clips hard
            let clip = tube_clip_level(model);
            let level = if peak > 0.0 { amp_to_dbfs(peak / clip) } else { -TUBE_COLOR_RANGE };
            saturation = ((level + TUBE_COLOR_RANGE / 2.0) / TUBE_COLOR_RANGE).clamp(0.0, 1.0);
            limit = clip * 2.0;
        }
        // The LEDs show the most since they were last read
        self.saturation = self.saturation.max(saturation);
        if peak >= limit * CLIP_WARNING {
            self.clip = true;
        }
    }

    /// How far the stage saturated since the last call, 0 (clean) to 1 (fully), for the SAT LED: for an op-amp,
    /// from its knee to its rail; for a tube, from 6 dB below where it starts to clip to 6 dB above
    pub fn take_saturation(&mut self) -> f32 {
        core::mem::take(&mut self.saturation)
    }

    /// Whether the stage came within 3 dB of clipping since the last call, for the CLIP LED
    pub fn take_clip(&mut self) -> bool {
        core::mem::take(&mut self.clip)
    }

    /// The 4558's crossover distortion, at every level: the output stage's dead band, -d tanh(x / d), as the
    /// op-amp's feedback reduces it: divided by its loop gain, its open loop gain at low frequencies and its gain
    /// bandwidth over the frequency above (so its derivative)
    fn process_crossover(&mut self, frames: &mut [[f32; 2]], spec: &OpAmpSpec) {
        const OPEN_LOOP_GAIN: f32 = 2e5;
        let band = spec.crossover / VOLTS_PER_UNIT;
        let derivative = self.sample_rate / (2.0 * PI * spec.gain_bandwidth);
        for frame in frames.iter_mut() {
            for ch in 0..2 {
                let error = -band * tanhf(frame[ch] / band);
                frame[ch] += (error - self.crossover_error[ch]) * derivative + error / OPEN_LOOP_GAIN;
                self.crossover_error[ch] = error;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_amps_are_clean_below_their_knee() {
        for model in GainModel::DISPLAY_ORDER.into_iter().filter(|m| m.is_op_amp()) {
            assert_eq!(op_amp(model, 1.0), 1.0);
            assert!(op_amp(model, 30.0) <= op_amp_rail(model) + 1e-4);
        }
    }

    #[test]
    fn tubes_are_unity_for_small_signals() {
        for model in GainModel::DISPLAY_ORDER.into_iter().filter(|m| m.is_tube()) {
            let y = tube(model, 0.01);
            assert!((y / 0.01 - 1.0).abs() < 0.02, "{model:?}: {y}");
        }
        assert!((tube_clip_level(GainModel::Tube12AX7) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn keys_round_trip() {
        for model in GainModel::DISPLAY_ORDER {
            assert_eq!(GainModel::from_key(model.key()), Some(model));
        }
    }
}
