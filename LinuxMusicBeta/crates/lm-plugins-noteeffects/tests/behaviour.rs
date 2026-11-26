//! What each note effect does to a stream, by hand (the generated tests check the contract, these the music)

use lm_plugin::testing::{NoteBench, Timed, note_off, note_on};
use lm_plugin::{BuildContext, Descriptor, Registry, ResourceView};
use lm_render::{EventKind, ProcessSpec};
use lm_state::{PARAMS_KEY, Value};

const RATE: u32 = 48_000;

fn bench(d: &'static Descriptor, state: Value) -> NoteBench {
    let registry = Registry::new(&[d]).expect("registry");
    let view = ResourceView::new();
    let spec = ProcessSpec { sample_rate: RATE, max_block: 1024 };
    let cx = BuildContext { spec, resources: &view, channels: 2 };
    NoteBench::new(registry.build(d.id, &state, &cx).expect("build"), spec, 7)
}

fn params(pairs: &[(&str, f64)]) -> Value {
    let mut p = Value::empty_object();
    for (k, v) in pairs {
        p = p.with(k, Value::real(*v).expect("finite"));
    }
    Value::empty_object().with(PARAMS_KEY, p)
}

fn ons(out: &[Timed]) -> Vec<(u64, u8, f32)> {
    out.iter()
        .filter_map(|t| match t.kind {
            EventKind::NoteOn { key, velocity, .. } => Some((t.at, key, velocity)),
            _ => None,
        })
        .collect()
}

#[test]
fn chords_make_the_notes_of_the_chord() {
    let state = params(&[]).with("chord", Value::text("major"));
    let out = bench(&lm_plugins_noteeffects::chords::DESCRIPTOR, state).run(
        4000,
        256,
        &[note_on(100, 1, 60, 0.5), note_off(2000, 1, 60)],
        true,
    );
    let keys: Vec<u8> = ons(&out).iter().map(|o| o.1).collect();
    assert_eq!(keys, vec![60, 64, 67]);
    assert!(ons(&out).iter().all(|o| o.0 == 100));
    let offs = out.iter().filter(|t| matches!(t.kind, EventKind::NoteOff { .. }) && t.at == 2000).count();
    assert_eq!(offs, 3);
}

#[test]
fn chords_repeat_over_the_range() {
    let state = params(&[("range", 2.0)]).with("chord", Value::text("major"));
    let out = bench(&lm_plugins_noteeffects::chords::DESCRIPTOR, state).run(1000, 256, &[note_on(0, 1, 60, 0.5)], true);
    let keys: Vec<u8> = ons(&out).iter().map(|o| o.1).collect();
    assert_eq!(keys, vec![60, 64, 67, 72, 76, 79]);
}

#[test]
fn the_arpeggio_steps_on_the_exact_sample_for_any_block() {
    let state = params(&[("time", 200.0)]).with("chord", Value::text("major"));
    for block in [32, 256, 1000] {
        let out = bench(&lm_plugins_noteeffects::arpeggiator::DESCRIPTOR, state.clone()).run(
            40_000,
            block,
            &[note_on(100, 1, 60, 0.5)],
            true,
        );
        let got: Vec<(u64, u8)> = ons(&out).iter().map(|o| (o.0, o.1)).collect();
        // 200 ms at 48 kHz is 9600 frames
        assert_eq!(got, vec![(100, 60), (9700, 64), (19_300, 67), (28_900, 60), (38_500, 64)], "block {block}");
    }
}

#[test]
fn the_held_note_is_silent_and_its_release_ends_the_step() {
    let state = params(&[("gate", 200.0)]).with("chord", Value::text("major"));
    let out = bench(&lm_plugins_noteeffects::arpeggiator::DESCRIPTOR, state).run(
        30_000,
        256,
        &[note_on(0, 1, 60, 0.5), note_off(10_000, 1, 60)],
        true,
    );
    // the first step (gate 200 %: 19200 frames) is ended by the release at 10000
    let off = out.iter().find(|t| matches!(t.kind, EventKind::NoteOff { .. })).expect("an off");
    assert_eq!(off.at, 10_000);
    assert!(!ons(&out).iter().any(|o| o.0 > 10_000));
}

#[test]
fn the_synced_arpeggio_follows_the_tempo() {
    let state = params(&[]).with("sync", Value::text("1/8")).with("chord", Value::text("major"));
    let out = bench(&lm_plugins_noteeffects::arpeggiator::DESCRIPTOR, state).run(
        30_000,
        256,
        &[note_on(0, 1, 60, 0.5)],
        true,
    );
    // an eighth at 120 BPM is 12000 frames
    let at: Vec<u64> = ons(&out).iter().map(|o| o.0).collect();
    assert_eq!(at, vec![0, 12_000, 24_000]);
}

#[test]
fn scale_snaps_and_transposes() {
    let state = params(&[("transpose", 12.0)]).with("scale", Value::text("scale-major"));
    let out = bench(&lm_plugins_noteeffects::scale::DESCRIPTOR, state).run(
        1000,
        256,
        &[note_on(0, 1, 61, 0.5), note_on(10, 2, 64, 0.5)],
        true,
    );
    // C# is not in C major: it falls to C (the lower on a tie); E stays; both an octave up
    let keys: Vec<u8> = ons(&out).iter().map(|o| o.1).collect();
    assert_eq!(keys, vec![72, 76]);
}

#[test]
fn the_note_off_carries_the_key_its_note_on_was_sent_on() {
    let fx = &lm_plugins_noteeffects::scale::DESCRIPTOR;
    let mut b = bench(fx, params(&[("transpose", 5.0)]));
    let mut out = b.run(500, 256, &[note_on(0, 1, 60, 0.5)], true);
    // the transposition changes while the note is held
    out.extend(b.run(
        1000,
        256,
        &[Timed::new(600, EventKind::Param { index: 1, value: 7.0 }), note_off(900, 1, 60)],
        true,
    ));
    let off = out.iter().find_map(|t| match t.kind {
        EventKind::NoteOff { key, .. } => Some(key),
        _ => None,
    });
    assert_eq!(off, Some(65));
}

#[test]
fn velocity_fixed_scaled_and_random() {
    let one = |mode: &str, p: &[(&str, f64)], v: f32| {
        let state = params(p).with("mode", Value::text(mode));
        let out =
            bench(&lm_plugins_noteeffects::velocity::DESCRIPTOR, state).run(100, 64, &[note_on(0, 1, 60, v)], true);
        ons(&out)[0].2
    };
    assert!((one("fixed", &[("fixed", 127.0)], 0.2) - 1.0).abs() < 1e-6);
    assert!((one("scale", &[("scale", 50.0)], 0.8) - 0.4).abs() < 1e-6);
    let curved = one("curve", &[("curve", 1.0)], 0.25);
    assert!(curved > 0.25 && curved < 1.0);
    let a = one("random", &[("random", 50.0), ("seed", 3.0)], 0.5);
    let b = one("random", &[("random", 50.0), ("seed", 3.0)], 0.5);
    assert_eq!(a, b, "the same seed, the same velocity");
    assert!((0.0..=1.0).contains(&a));
}
