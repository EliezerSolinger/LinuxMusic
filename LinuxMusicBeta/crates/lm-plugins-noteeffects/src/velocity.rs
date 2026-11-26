//! Velocity: every note-on takes a fixed velocity, is scaled, goes through a curve, or gets a random one. The random
//! one comes from a hash of the seed, the note's key and its time (the timeline's while playing, the engine's while
//! stopped), so the same take plays the same velocities every time.

use lm_base::Result;
use lm_plugin::note_effect::{BlockCx, NoteEffect, NoteFx};
use lm_plugin::{BuildContext, Descriptor, PluginKind, StateView, TestHints, Velocity as Vel};
use lm_render::{EventKind, NoteOut, Processor};
use lm_state::{FieldSpec, NodeSchema, ParamSpec, Scale, Value};

pub const ID: &str = "velocity";

pub static MODES: [&str; 4] = ["fixed", "scale", "curve", "random"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.velocity.name",
    kind: PluginKind::NoteEffect,
    schemas,
    ports: crate::no_ports,
    velocity: Vel::Ignored,
    display: None,
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:velocity", 1)
            .field(FieldSpec::choice("mode", &MODES, "scale"))
            .param(ParamSpec::new("fixed", 1.0, 127.0, 100.0).scale(Scale::Stepped))
            .param(ParamSpec::new("scale", 0.0, 200.0, 100.0).unit("%"))
            .param(ParamSpec::new("curve", -1.0, 1.0, 0.0))
            .param(ParamSpec::new("random", 0.0, 100.0, 0.0).unit("%"))
            .param(ParamSpec::new("seed", 0.0, 9999.0, 0.0).scale(Scale::Stepped)),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Fixed,
    Scale,
    Curve,
    Random,
}

fn build(state: &Value, _: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    let v = StateView::new(state);
    let mode = v.choice("mode", |k| {
        Some(match k {
            "fixed" => Mode::Fixed,
            "scale" => Mode::Scale,
            "curve" => Mode::Curve,
            "random" => Mode::Random,
            _ => return None,
        })
    })?;
    let fx = VelocityFx { mode, fixed: 100.0, scale: 100.0, curve: 0.0, random: 0.0, seed: 0 };
    Ok(Box::new(NoteFx::new(fx, &v, &DESCRIPTOR.schema().params)?))
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The velocity
#[derive(Debug)]
pub struct VelocityFx {
    mode: Mode,
    fixed: f32,
    scale: f32,
    curve: f32,
    random: f32,
    seed: u64,
}

impl VelocityFx {
    /// The velocity (0 to 1) a note of velocity `v` gets; `time` is the note's moment in samples
    pub fn apply(&self, v: f32, key: u8, time: i64) -> f32 {
        let out = match self.mode {
            Mode::Fixed => self.fixed / 127.0,
            Mode::Scale => v * self.scale / 100.0,
            // 0 leaves it; above 0 lifts the soft notes, below 0 lets them fall
            Mode::Curve => libm::powf(v.clamp(0.0, 1.0), libm::powf(4.0, -self.curve)),
            Mode::Random => {
                let z = mix(self.seed ^ mix(time as u64) ^ (u64::from(key) << 56));
                let u = (z >> 40) as f32 / (1u64 << 24) as f32;
                v + (u * 2.0 - 1.0) * self.random / 100.0
            }
        };
        out.clamp(1.0 / 127.0, 1.0)
    }
}

impl NoteEffect for VelocityFx {
    const PARAMS: usize = 5;

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.fixed = value,
            1 => self.scale = value,
            2 => self.curve = value,
            3 => self.random = value,
            _ => self.seed = value as u64,
        }
    }

    fn event(&mut self, at: u32, kind: EventKind, cx: &mut BlockCx<'_>, out: &mut NoteOut) {
        match kind {
            EventKind::NoteOn { note, channel, key, velocity } => {
                let time = if cx.transport.playing {
                    cx.transport.position + i64::from(at)
                } else {
                    cx.transport.frame as i64 + i64::from(at)
                };
                let velocity = self.apply(velocity, key, time);
                out.push(at, EventKind::NoteOn { note, channel, key, velocity });
            }
            kind => {
                out.push(at, kind);
            }
        }
    }

    fn take_state(&mut self, _: &mut Self) {}
}
