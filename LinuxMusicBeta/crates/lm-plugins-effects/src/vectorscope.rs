//! Vectorscope (F-FX-029: legacy `plugins/Vectorscope`, Martin Pavelek's): the incoming audio is passed through
//! unchanged and the last frames are published for the window, which draws the stereo field (side across, mid
//! up) from them.
//!
//! State, version 1: the legacy's two switches, `logarithmic` (the amplitude on a logarithmic scale, to see small
//! values; off) and `linesMode` (lines between the frames, else points; on). The window applies both (the legacy
//! applies them when it paints, to the frames that arrived since the last paint); the plugin only keeps the frames.
//! The legacy has no other option (no high-quality or persistence knob: the trace fades along the frames of a
//! paint).
//!
//! The window's live frame, little-endian `f32`: the number of frames kept (up to [`POINTS`]) and then the
//! last [`POINTS`] frames, oldest first, as left and right (the unused ones zero).

use lm_base::Result;
use lm_plugin::{BuildContext, Descriptor, DisplaySpec, PluginKind, TestHints, Velocity};
use lm_render::Processor;
use lm_state::{NodeSchema, ParamSpec, Scale, Value};

use crate::effect::{Effect, EffectProcessor, Knobs, no_ports, put_f32s};

pub const ID: &str = "vectorscope";

/// The frames of a live frame: the legacy paints what came in since its last paint, about a sixtieth of a second
pub const POINTS: usize = 1024;
/// The numbers of a frame
pub const FRAME_FLOATS: usize = 1 + POINTS * 2;

const KEYS: [&str; 2] = ["logarithmic", "linesMode"];

pub static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    name: "plugin.vectorscope.name",
    kind: PluginKind::Effect,
    schemas,
    ports: no_ports,
    velocity: Velocity::Ignored,
    display: Some(DisplaySpec { bytes: FRAME_FLOATS * 4 }),
    build,
    tests: TestHints::new(),
};

fn schemas() -> Vec<NodeSchema> {
    vec![
        NodeSchema::new("plugin:vectorscope", 1)
            .param(ParamSpec::new("logarithmic", 0.0, 1.0, 0.0).scale(Scale::Stepped))
            .param(ParamSpec::new("linesMode", 0.0, 1.0, 1.0).scale(Scale::Stepped)),
    ]
}

struct Vector {
    /// The pair that records (the first)
    active: bool,
    ring: Vec<[f32; 2]>,
    /// Where the next frame goes
    write: usize,
    /// How many frames the ring holds, up to its length
    filled: usize,
    /// The live frame, worked out each time (allocated when built)
    scratch: Vec<f32>,
}

impl Vector {
    fn new(active: bool) -> Self {
        Self {
            active,
            ring: vec![[0.0; 2]; if active { POINTS } else { 1 }],
            write: 0,
            filled: 0,
            scratch: vec![0.0; if active { FRAME_FLOATS } else { 0 }],
        }
    }
}

impl Effect for Vector {
    fn set_param(&mut self, _index: usize, _value: f32) {}

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        if !self.active {
            return;
        }
        // A block longer than the ring keeps its end
        let frames = &frames[frames.len().saturating_sub(POINTS)..];
        for f in frames {
            self.ring[self.write] = *f;
            self.write = (self.write + 1) % POINTS;
        }
        self.filled = (self.filled + frames.len()).min(POINTS);
    }

    fn reset(&mut self) {
        self.ring.fill([0.0; 2]);
        self.filled = 0;
    }

    fn adopt(&mut self, old: &mut Self) -> bool {
        if self.ring.len() != old.ring.len() {
            return false;
        }
        std::mem::swap(&mut self.ring, &mut old.ring);
        self.write = old.write;
        self.filled = old.filled;
        true
    }

    fn display(&mut self, frame: &mut [u8]) {
        // Oldest first: the ring from the write position on
        self.scratch[0] = self.filled as f32;
        for i in 0..POINTS {
            let f = self.ring[(self.write + i) % POINTS];
            self.scratch[1 + 2 * i] = f[0];
            self.scratch[2 + 2 * i] = f[1];
        }
        put_f32s(frame, &self.scratch);
    }
}

fn build(state: &Value, cx: &BuildContext<'_>) -> Result<Box<dyn Processor>> {
    // Both switches are the window's: they are read to prove the state is whole
    Knobs::<2>::from_state(state, &KEYS)?;
    let mut first = true;
    EffectProcessor::build(cx, || Ok(Vector::new(std::mem::take(&mut first))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn published(v: &mut Vector) -> Vec<f32> {
        let mut bytes = vec![0u8; FRAME_FLOATS * 4];
        v.display(&mut bytes);
        bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    #[test]
    fn the_last_frames_come_oldest_first() {
        let mut v = Vector::new(true);
        let mut frames: Vec<[f32; 2]> = (0..1500).map(|i| [i as f32, -(i as f32)]).collect();
        v.process(&mut frames[..700]);
        v.process(&mut frames[700..]);
        let out = published(&mut v);
        assert_eq!(out[0], POINTS as f32);
        assert_eq!(out[1], (1500 - POINTS) as f32, "the oldest kept");
        assert_eq!(out[2], -((1500 - POINTS) as f32));
        assert_eq!(out[FRAME_FLOATS - 2], 1499.0, "the newest is last");
    }

    #[test]
    fn a_young_ring_says_how_much_it_holds() {
        let mut v = Vector::new(true);
        v.process(&mut [[0.5, 0.25]; 10]);
        assert_eq!(published(&mut v)[0], 10.0);
        v.reset();
        assert_eq!(published(&mut v)[0], 0.0);
    }

    #[test]
    fn only_the_first_pair_records() {
        let mut v = Vector::new(false);
        v.process(&mut [[1.0, 1.0]; 10]);
        assert_eq!(v.filled, 0);
    }
}
