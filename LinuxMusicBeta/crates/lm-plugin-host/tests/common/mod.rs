//! What the helper's tests share: a runner that drives a processor as the renderer would
#![allow(dead_code)]

use lm_render::{BlockShape, Buffers, Event, HostOutbox, PluginRequest, ProcessContext, Processor, Rng, TransportInfo};
use lm_time::TimeSignature;

pub const RATE: u32 = 48_000;
pub const BLOCK: usize = 256;

/// Runs blocks through a processor as the renderer would: an in-place effect (2 channels) or an instrument
pub struct Runner {
    data: Vec<f32>,
    shape: BlockShape,
    rng: Rng,
    host: HostOutbox,
    pub frame: u64,
    pub requests: Vec<PluginRequest>,
}

impl Runner {
    pub fn effect() -> Runner {
        Runner::new(BlockShape { inputs: 2, outputs: 2, sidechain: 0, in_place: true })
    }

    pub fn instrument() -> Runner {
        Runner::new(BlockShape { inputs: 0, outputs: 2, sidechain: 0, in_place: false })
    }

    pub fn new(shape: BlockShape) -> Runner {
        let len = (if shape.in_place { 0 } else { shape.inputs } + shape.outputs) * BLOCK;
        Runner {
            data: vec![0.0; len],
            shape,
            rng: Rng::new(1),
            host: HostOutbox::default(),
            frame: 0,
            requests: Vec::with_capacity(1024),
        }
    }

    /// One block: `input(frame, channel)` into the in-place channels, `events`; the output channels back
    pub fn block(
        &mut self,
        p: &mut dyn Processor,
        input: impl Fn(u64, usize) -> f32,
        events: &[Event],
        trap: bool,
    ) -> [Vec<f32>; 2] {
        let start = self.frame;
        for ch in 0..2 {
            for k in 0..BLOCK {
                self.data[ch * BLOCK + k] = if self.shape.in_place { input(start + k as u64, ch) } else { 0.0 };
            }
        }
        let transport = TransportInfo {
            sample_rate: RATE,
            playing: true,
            recording: false,
            position: start as i64,
            tick: 0,
            ppq: 960,
            bpm: 120.0,
            time_signature: TimeSignature::COMMON,
            frame: start,
        };
        let mut run = || {
            let buffers = Buffers::from_block(&mut self.data, BLOCK, BLOCK, self.shape);
            let mut cx = ProcessContext {
                frames: BLOCK,
                buffers,
                events,
                transport: &transport,
                rng: &mut self.rng,
                host: &mut self.host,
                display: None,
            };
            p.process(&mut cx);
        };
        if trap {
            assert_no_alloc::assert_no_alloc(run);
        } else {
            run();
        }
        self.requests.extend(self.host.drain());
        self.frame += BLOCK as u64;
        [self.data[..BLOCK].to_vec(), self.data[BLOCK..2 * BLOCK].to_vec()]
    }
}

pub fn ramp(frame: u64, ch: usize) -> f32 {
    ((frame % 997) as f32 / 997.0 - 0.5) * if ch == 0 { 1.0 } else { -1.0 }
}
