//! Modulators: they add to the parameter's value on the automation grid, the same events for any block size, under
//! the allocation trap (`make` runs the render threads under it)

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use lm_render::modulation::{ModKind, ModTarget, ModulatorSpec, P_SPEED, P_VALUE};
use lm_render::{
    Command, EventKind, Install, NodeDesc, ParamInfo, ParamRange, ParamSlot, ProcessContext, Processor, ProgramBuilder,
    ProgramGen, Rate,
};

/// Records each parameter event it gets: (engine frame, value)
struct ParamLog(Arc<Mutex<Vec<(u64, f32)>>>, Vec<(u64, f32)>);

impl Processor for ParamLog {
    fn process(&mut self, cx: &mut ProcessContext<'_>) {
        for e in cx.events {
            if let EventKind::Param { value, .. } = e.kind
                && self.1.len() < self.1.capacity()
            {
                self.1.push((cx.transport.frame + u64::from(e.offset), value));
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl Drop for ParamLog {
    fn drop(&mut self) {
        if let Ok(mut v) = self.0.lock() {
            v.extend_from_slice(&self.1);
        }
    }
}

/// Renders `total` frames of a program with one modulator on a parameter (slot 1, 0 to 10, at 5) of node 1
fn run(spec: ModulatorSpec, size: usize, total: usize) -> Vec<(u64, f32)> {
    let log = Arc::new(Mutex::new(Vec::new()));
    {
        let (mut control, mut dummy) = make(0);
        let mut b = ProgramBuilder::new(ProgramGen(1), common::spec(), &limits(0), tempo(120));
        master(&mut b);
        b.node(NodeDesc::processor(n(1), 0, 1));
        b.param(ParamSlot(1), n(1), 0, ParamInfo { rate: Rate::Block, discrete: false, initial: 5.0 }, None);
        let mut m = spec;
        m.targets.push(ModTarget {
            slot: ParamSlot(1),
            range: ParamRange::linear(0.0, 10.0),
            amount: 0.2,
            min: 0.0,
            max: 1.0,
        });
        b.modulator(m);
        let mut i = Install::new(b.build().expect("program"));
        i.add(n(1), core(ParamLog(log.clone(), Vec::with_capacity(4096))));
        install(&mut control, i);
        control.commands().send(Command::Play).expect("play");
        render(&mut dummy, total, size);
    }
    let got = log.lock().expect("log").clone();
    got
}

#[test]
fn a_macro_adds_to_the_value() {
    let mut m = ModulatorSpec::new(ModKind::Macro, 1);
    m.initial[P_VALUE] = 0.5;
    let got = run(m, 256, 1024);
    // 5 + 0.2 * 0.5 of the range (10) = 6, once: the value does not change
    assert_eq!(got.first(), Some(&(0, 6.0)));
    assert_eq!(got.len(), 1, "{got:?}");
}

#[test]
fn an_lfo_is_the_same_for_any_block_size() {
    let mut m = ModulatorSpec::new(ModKind::Lfo, 7);
    m.initial[P_SPEED] = 0.05;
    let a = run(m.clone(), 64, 8192);
    let b = run(m, 1000, 8192);
    assert!(a.len() > 20, "it moves");
    assert!(a.iter().all(|(f, v)| (2.9..=7.1).contains(v) && f % 32 == 0), "{a:?}");
    assert_eq!(a, b);
}
