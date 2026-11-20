//! Wave Shaper (F-EFX: legacy `plugins/WaveShaper`): an input gain, a drawn transfer curve (mirrored for the
//! negative side), an output gain.
//!
//! State, version 1: parameters `inputGain` and `outputGain` (0–5, 1; ramped over the piece in which they change, as
//! the legacy value buffers over a period), `clipInput` (a switch, off); field `shape`: the curve, 200 points from 0
//! to 1 for the input levels 1/200 to 1 (a straight line by default). A curve of another length is read as a curve
//! over the same range (resampled linearly).

use lm_base::Result;
use lm_dsp::distortion_effects::{WaveShape, default_wave_shape, wave_shaper};
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{FieldKind, FieldSpec, NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s, reals};

pub const ID: &str = "waveshaper";
const KEYS: [&str; 3] = ["inputGain", "outputGain", "clipInput"];
pub const POINTS: usize = 200;

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.waveshaper.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    // The curve's window shows where the input is on the curve: the peak of each side after the input gain since the
    // last frame, two little-endian f32
    display: Some(DisplaySpec { bytes: 8 }),
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    let line = Value::list(default_wave_shape().iter().map(|v| Value::real(f64::from(*v)).unwrap_or(Value::NULL)));
    vec![
        NodeSchema::new("plugin:waveshaper", 1)
            .param(ParamSpec::new("inputGain", 0.0, 5.0, 1.0))
            .param(ParamSpec::new("outputGain", 0.0, 5.0, 1.0))
            .param(ParamSpec::new("clipInput", 0.0, 1.0, 0.0).scale(Scale::Stepped))
            .field(
                FieldSpec::new("shape", FieldKind::List(Box::new(FieldKind::Real { min: 0.0, max: 1.0 })))
                    .default(line),
            ),
    ]
}

/// The state's curve, resampled to [`POINTS`] points if it has another length (an empty one is the straight line)
pub fn curve(points: &[f64]) -> WaveShape {
    match points.len() {
        0 => default_wave_shape(),
        1 => [points[0] as f32; POINTS],
        n if n == POINTS => core::array::from_fn(|i| points[i] as f32),
        n => core::array::from_fn(|i| {
            let x = i as f64 * (n - 1) as f64 / (POINTS - 1) as f64;
            let j = (x.floor() as usize).min(n - 2);
            let t = x - j as f64;
            (points[j] + (points[j + 1] - points[j]) * t) as f32
        }),
    }
}

struct Shaper {
    knobs: Knobs<3>,
    shape: WaveShape,
    /// The input's peaks after the input gain, for the window
    peaks: [f32; 2],
}

impl Effect for Shaper {
    fn set_param(&mut self, index: usize, value: f32) {
        self.knobs.set(index, value);
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        let k = &self.knobs;
        let (gain, n) = (k.ramp(0), frames.len());
        for (f, x) in frames.iter().enumerate() {
            let g = gain.at(f, n);
            self.peaks = [self.peaks[0].max((x[0] * g).abs()), self.peaks[1].max((x[1] * g).abs())];
        }
        wave_shaper(frames, &self.shape, k.ramp(0), k.ramp(1), k.on(2));
        self.knobs.settle();
    }

    fn display(&mut self, frame: &mut [u8]) {
        put_f32s(frame, &self.peaks);
        self.peaks = [0.0; 2];
    }

    fn reset(&mut self) {}

    fn adopt(&mut self, old: &mut Self) -> bool {
        self.knobs.adopt(&old.knobs);
        true
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let knobs = Knobs::from_state(state, &KEYS)?;
    let shape = curve(&reals(state, "shape")?);
    EffectProcessor::build(cx, || Ok(Shaper { knobs, shape, peaks: [0.0; 2] }))
}
