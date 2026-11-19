//! A device without hardware (tests and the first `lm-io` backend's model): it calls [`Renderer::render`] in a loop
//! like a device would, with its own buffers and a host clock that advances by the block's duration

use lm_base::HostTime;

use crate::renderer::{BlockReport, DeviceBlock, Renderer};

/// Drives a renderer block by block
#[derive(Debug)]
pub struct Dummy {
    renderer: Renderer,
    inputs: Vec<Vec<f32>>,
    outputs: Vec<Vec<f32>>,
    time: HostTime,
    rate: u32,
}

impl Dummy {
    /// `inputs` and `outputs` device channels; buffers of `max_block` frames (allocated here, not per block)
    pub fn new(renderer: Renderer, inputs: usize, outputs: usize) -> Self {
        let spec = renderer.spec();
        Self {
            renderer,
            inputs: vec![vec![0.0; spec.max_block]; inputs],
            outputs: vec![vec![0.0; spec.max_block]; outputs],
            time: HostTime(1_000_000_000),
            rate: spec.sample_rate,
        }
    }

    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    pub fn into_renderer(self) -> Renderer {
        self.renderer
    }

    /// The host time of the next block's first sample
    pub fn host_time(&self) -> HostTime {
        self.time
    }

    /// The capture buffers for the next block (fill them before [`Dummy::block`])
    pub fn inputs_mut(&mut self) -> &mut [Vec<f32>] {
        &mut self.inputs
    }

    /// The last block's output
    pub fn outputs(&self) -> &[Vec<f32>] {
        &self.outputs
    }

    /// Renders one block of `frames` (`<= max_block`); `capture_lost` simulates a capture xrun before it
    pub fn block_with(&mut self, frames: usize, capture_lost: u32) -> &BlockReport {
        let frames = frames.min(self.outputs.first().map_or(frames, Vec::len));
        let n_in = self.inputs.len().min(16);
        let n_out = self.outputs.len().min(16);
        let mut in_refs: [&[f32]; 16] = [&[]; 16];
        for (r, c) in in_refs.iter_mut().zip(&self.inputs) {
            *r = &c[..];
        }
        let mut out_refs: [&mut [f32]; 16] = Default::default();
        for (r, c) in out_refs.iter_mut().zip(self.outputs.iter_mut()) {
            *r = &mut c[..];
        }
        let time = self.time;
        self.time = HostTime(self.time.0 + (frames as u64 * 1_000_000_000) / u64::from(self.rate));
        self.renderer.render(DeviceBlock {
            frames,
            host_time: time,
            inputs: &in_refs[..n_in],
            outputs: &mut out_refs[..n_out],
            capture_lost,
            late_by: 0,
        })
    }

    pub fn block(&mut self, frames: usize) -> &BlockReport {
        self.block_with(frames, 0)
    }

    /// Renders `total` frames in blocks of `block` and appends each output channel to `out` (the caller sizes `out`
    /// beforehand if it wants no allocation)
    pub fn run_into(&mut self, total: usize, block: usize, out: &mut [Vec<f32>]) {
        let mut done = 0;
        while done < total {
            let n = block.min(total - done);
            self.block(n);
            for (o, c) in out.iter_mut().zip(&self.outputs) {
                o.extend_from_slice(&c[..n]);
            }
            done += n;
        }
    }
}
