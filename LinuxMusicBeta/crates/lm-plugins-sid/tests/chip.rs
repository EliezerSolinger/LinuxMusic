//! The chip's registers reach it at every rate: at 192 kHz a piece of 32 samples is 164 cycles and the write
//! sequence takes 237; voice 3's registers come last in it

use lm_plugin::testing::{Bench, note_on, rms};
use lm_plugin::{BuildContext, Registry, ResourceView};
use lm_plugins_sid::DESCRIPTOR;
use lm_render::ProcessSpec;
use lm_state::{PARAMS_KEY, Value};

fn level(rate: u32, block: usize) -> f64 {
    let registry = Registry::new(&[&DESCRIPTOR]).unwrap();
    // Voices 1 and 2 held silent by their test bits: only voice 3 sounds
    let params = Value::empty_object()
        .with("test1", Value::real(1.0).unwrap())
        .with("test2", Value::real(1.0).unwrap())
        .with("attack3", Value::real(0.0).unwrap());
    let state = Value::empty_object().with(PARAMS_KEY, params);
    let spec = ProcessSpec { sample_rate: rate, max_block: 1024 };
    let resources = ResourceView::new();
    let cx = BuildContext { spec, resources: &resources, channels: 0 };
    let processor = registry.build(DESCRIPTOR.id, &state, &cx).unwrap();
    let frames = rate as usize / 2;
    let out = Bench::instrument(processor, spec, 2, 0).render(frames, block, &[note_on(0, 1, 69, 1.0)], false);
    rms(&out[0][frames / 2..])
}

#[test]
fn voice_three_sounds_at_every_rate() {
    let reference = level(48_000, 256);
    assert!(reference > 0.01, "voice 3 alone: {reference}");
    for rate in [44_100, 96_000, 192_000] {
        for block in [32, 1024] {
            let l = level(rate, block);
            let db = 20.0 * (l / reference).log10();
            assert!(db.abs() < 0.5, "{rate} Hz, blocks of {block}: voice 3 is {db:.2} dB from 48 kHz");
        }
    }
}
