//! The plugin in the real engine (what lm-compile will do): built by the registry, added in an `Install`, played by
//! a MIDI item, then replaced by a processor built from a changed tree, which adopts it on the audio thread. Every
//! render runs under the allocation trap (this binary's allocator)

use std::sync::Arc;

use lm_plugin::{BuildContext, Registry, ResourceView};
use lm_plugins_testsine::DESCRIPTOR;
use lm_render::*;
use lm_state::Value;
use lm_time::{Bpm, Ppq, TempoMap, TimeSignature};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: lm_plugin::testing::AllocDisabler = lm_plugin::testing::AllocDisabler;

const RATE: u32 = 48_000;

fn trapped_threads() -> RenderThreads {
    RenderThreads::new(Arc::new(|_, body: &mut dyn FnMut()| lm_plugin::testing::assert_no_alloc(body)))
}

fn program(generation: u64, spec: ProcessSpec, limits: &Limits) -> Box<Program> {
    let tempo = TempoMap::constant(RATE, Ppq::DEFAULT, Bpm::from_integer(120).expect("bpm"), TimeSignature::COMMON)
        .expect("tempo");
    let mut b = ProgramBuilder::new(ProgramGen(generation), spec, limits, tempo);
    let master = NodeIndex::new(0, 1);
    let inst = NodeIndex::new(1, 1);
    b.node(NodeDesc::sum(master, 2));
    b.master(master);
    b.output(OutputRoute { node: master, channel: 0, device_channel: 0 });
    b.output(OutputRoute { node: master, channel: 1, device_channel: 1 });
    b.node(NodeDesc::processor(inst, 0, 2));
    b.feed(Feed::new(inst, master, 2));
    let item = MidiItem::new(
        7,
        vec![
            ItemEvent { tick: 0, kind: ItemEventKind::NoteOn { n: 1, channel: 0, key: 69, velocity: 1.0 } },
            ItemEvent { tick: 960 * 16, kind: ItemEventKind::NoteOff { n: 1 } },
        ],
    )
    .expect("item");
    b.midi_items(inst, vec![Arc::new(item)]);
    b.build().expect("program")
}

/// The peak and the largest step between two samples heard
#[derive(Default)]
struct Ear {
    peak: f32,
    last: f32,
    step: f32,
}

impl Ear {
    fn listen(&mut self, dummy: &mut Dummy, blocks: usize) {
        for _ in 0..blocks {
            dummy.block(256);
            for &x in &dummy.outputs()[0][..256] {
                self.step = self.step.max((x - self.last).abs());
                self.last = x;
                self.peak = self.peak.max(x.abs());
            }
        }
    }
}

#[test]
fn built_installed_played_and_replaced_while_playing() {
    install_panic_hook();
    let spec = ProcessSpec { sample_rate: RATE, max_block: 256 };
    let limits = Limits { max_nodes: 16, max_params: 64, ..Limits::default() };
    let (mut control, renderer) = engine_with(spec, limits, trapped_threads());
    let mut dummy = Dummy::new(renderer, 0, 2);
    let registry = Registry::new(&[&DESCRIPTOR]).expect("registry");
    let resources = ResourceView::new();
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let inst = NodeIndex::new(1, 1);

    let first = registry.build("testsine", &Value::empty_object(), &cx).expect("build");
    let mut install = Install::new(program(1, spec, &limits));
    install.add(inst, NodeCore::new(first, Rng::seed_for(1, 1), None));
    control.install(install).expect("install");
    control.commands().send(Command::Play).expect("play");

    let mut ear = Ear::default();
    ear.listen(&mut dummy, 40);
    assert!(ear.peak > 0.1, "the note sounds: {}", ear.peak);
    let before = ear.step;

    // A field that is not a parameter changes: a new processor from the new tree, adopted on the audio thread
    let changed = Value::empty_object().with("aux", Value::bool(true));
    let second = registry.build("testsine", &changed, &cx).expect("build");
    let mut install = Install::new(program(2, spec, &limits));
    install.replace(inst, NodeCore::new(second, Rng::seed_for(1, 1), None), None);
    control.install(install).expect("install");
    ear.peak = 0.0;
    ear.listen(&mut dummy, 40);
    assert!(ear.peak > 0.1, "the note goes on in the new processor: {}", ear.peak);
    assert!(ear.step <= before * 1.5 + 1.0e-4, "no click at the replacement: {} against {before}", ear.step);
    let mut returned = 0;
    control.collect(|r| returned += r.processors.len());
    assert_eq!(returned, 1, "the old processor came back to the main thread");
}
