//! Dual mono: a mono plugin (one audio in, one out) on a stereo channel runs as two instances, the left channel
//! through one and the right through the other, with every parameter and event going to both. Any format: it wraps
//! two [`Plugin`]s.
//!
//! - The parameters are the first instance's (it is the one the core reads back and the one that saves its state);
//!   a value set goes to both. A parameter change the plugin makes by itself ([`Plugin::changes`]) is the first's
//!   and is copied to the second at once, so the two never drift apart.
//! - The state of the first is saved; a restore goes to both.
//! - The latency is the larger of the two (they are the same plugin: equal unless one is mid-restart).
use lm_base::Result;
use lm_render::EventKind;

use crate::backend::{Param, Plugin, Transport, WindowRequest, WindowState};

/// Whether a plugin with `io` channels on a block of `inputs` × `outputs` runs as dual mono
pub fn applies(io: Option<(usize, usize)>, inputs: usize, outputs: usize) -> bool {
    io == Some((1, 1)) && inputs == 2 && outputs == 2
}

/// Whether a plugin with `io` channels on a block of `outputs` runs spread: a mono instrument (no audio in, one out)
/// on two outputs plays its one channel on both
pub fn spreads(io: Option<(usize, usize)>, inputs: usize, outputs: usize) -> bool {
    io == Some((0, 1)) && inputs == 0 && outputs == 2
}

/// A mono instrument on two outputs: it runs once, and its output is copied to the second channel
pub struct Spread {
    inner: Box<dyn Plugin>,
}

impl Spread {
    pub fn new(inner: Box<dyn Plugin>) -> Spread {
        Spread { inner }
    }
}

impl Plugin for Spread {
    fn params(&self) -> &[Param] {
        self.inner.params()
    }

    fn io(&self) -> Option<(usize, usize)> {
        Some((0, 2))
    }

    fn latency(&self) -> u32 {
        self.inner.latency()
    }

    fn set_param(&mut self, index: u32, value: f32) {
        self.inner.set_param(index, value);
    }

    fn param(&self, index: u32) -> f32 {
        self.inner.param(index)
    }

    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[(u32, EventKind)],
        transport: &Transport,
    ) {
        let Some((first, rest)) = outputs.split_first_mut() else { return };
        self.inner.process(frames, inputs, std::slice::from_mut(first), events, transport);
        for out in rest {
            let n = frames.min(out.len()).min(first.len());
            out[..n].copy_from_slice(&first[..n]);
        }
    }

    fn save(&mut self) -> Result<Option<Vec<u8>>> {
        self.inner.save()
    }

    fn restore(&mut self, chunk: &[u8]) -> Result<()> {
        self.inner.restore(chunk)
    }

    fn window(&mut self, request: WindowRequest) -> Result<WindowState> {
        self.inner.window(request)
    }

    fn window_closed(&mut self) -> bool {
        self.inner.window_closed()
    }

    fn gestures(&mut self, out: &mut Vec<(u32, bool)>) {
        self.inner.gestures(out);
    }

    fn idle(&mut self) {
        self.inner.idle();
    }

    fn state_changed(&mut self) -> bool {
        self.inner.state_changed()
    }

    fn changes(&mut self, out: &mut Vec<(u32, f32)>) {
        self.inner.changes(out);
    }
}

pub struct DualMono {
    left: Box<dyn Plugin>,
    right: Box<dyn Plugin>,
    /// Parameter changes the first made on its own, to give the second
    seen: Vec<(u32, f32)>,
    /// The second's own changes, drained and dropped (it is the first that speaks for the pair)
    scratch: Vec<(u32, f32)>,
}

impl DualMono {
    pub fn new(left: Box<dyn Plugin>, right: Box<dyn Plugin>) -> DualMono {
        DualMono { left, right, seen: Vec::with_capacity(256), scratch: Vec::with_capacity(256) }
    }
}

impl Plugin for DualMono {
    fn params(&self) -> &[Param] {
        self.left.params()
    }

    fn latency(&self) -> u32 {
        self.left.latency().max(self.right.latency())
    }

    fn set_param(&mut self, index: u32, value: f32) {
        self.left.set_param(index, value);
        self.right.set_param(index, value);
    }

    fn param(&self, index: u32) -> f32 {
        self.left.param(index)
    }

    fn io(&self) -> Option<(usize, usize)> {
        Some((2, 2))
    }

    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[(u32, EventKind)],
        transport: &Transport,
    ) {
        let (Some(l_in), Some(r_in)) = (inputs.first(), inputs.get(1)) else { return };
        let (o_l, o_r) = outputs.split_at_mut(1);
        let (Some(l_out), Some(r_out)) = (o_l.first_mut(), o_r.first_mut()) else { return };
        self.left.process(frames, std::slice::from_ref(l_in), std::slice::from_mut(l_out), events, transport);
        self.right.process(frames, std::slice::from_ref(r_in), std::slice::from_mut(r_out), events, transport);
        self.seen.clear();
        self.left.changes(&mut self.seen);
        for (i, v) in &self.seen {
            self.right.set_param(*i, *v);
        }
        self.scratch.clear();
        self.right.changes(&mut self.scratch);
    }

    fn save(&mut self) -> Result<Option<Vec<u8>>> {
        self.left.save()
    }

    fn restore(&mut self, chunk: &[u8]) -> Result<()> {
        self.left.restore(chunk)?;
        self.right.restore(chunk)
    }

    fn window(&mut self, request: WindowRequest) -> Result<WindowState> {
        self.left.window(request)
    }

    fn window_closed(&mut self) -> bool {
        self.left.window_closed()
    }

    fn gestures(&mut self, out: &mut Vec<(u32, bool)>) {
        self.left.gestures(out);
    }

    fn idle(&mut self) {
        self.left.idle();
        self.right.idle();
    }

    fn state_changed(&mut self) -> bool {
        let a = self.left.state_changed();
        let b = self.right.state_changed();
        a || b
    }

    /// The first's own changes (the second was given them in `process`, so it is not asked twice)
    fn changes(&mut self, out: &mut Vec<(u32, f32)>) {
        out.extend(self.seen.iter().copied().take(out.capacity().saturating_sub(out.len())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mono instrument: a constant on its one output
    struct One;

    impl Plugin for One {
        fn params(&self) -> &[Param] {
            &[]
        }
        fn io(&self) -> Option<(usize, usize)> {
            Some((0, 1))
        }
        fn latency(&self) -> u32 {
            0
        }
        fn set_param(&mut self, _: u32, _: f32) {}
        fn param(&self, _: u32) -> f32 {
            0.0
        }
        fn process(
            &mut self,
            frames: usize,
            _: &[&[f32]],
            outputs: &mut [&mut [f32]],
            _: &[(u32, EventKind)],
            _: &Transport,
        ) {
            assert_eq!(outputs.len(), 1, "the plugin sees its own one output");
            outputs[0][..frames].fill(0.25);
        }
        fn save(&mut self) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }
        fn restore(&mut self, _: &[u8]) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_mono_instrument_is_spread_to_both_outputs() {
        assert!(spreads(Some((0, 1)), 0, 2) && !spreads(Some((0, 2)), 0, 2) && !spreads(None, 0, 2));
        let mut p = Spread::new(Box::new(One));
        assert_eq!(p.io(), Some((0, 2)));
        let (mut l, mut r) = (vec![0.0f32; 8], vec![0.0f32; 8]);
        {
            let mut outs: Vec<&mut [f32]> = vec![&mut l, &mut r];
            p.process(8, &[], &mut outs, &[], &Transport::default());
        }
        assert!(l.iter().chain(&r).all(|x| *x == 0.25));
    }
}
