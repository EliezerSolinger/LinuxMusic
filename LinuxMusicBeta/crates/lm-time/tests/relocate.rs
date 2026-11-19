//! H9: editing the tempo map while playing keeps the musical position, without replaying or skipping a tick
mod common;

use common::{Rng, bpm, random_map, sig};
use lm_time::{Ppq, TempoMap, TempoPoint, relocate};

#[test]
fn halving_the_tempo_keeps_the_tick() {
    let old = TempoMap::constant(48_000, Ppq::DEFAULT, bpm("120"), sig(4, 4)).unwrap();
    let new = TempoMap::constant(48_000, Ppq::DEFAULT, bpm("60"), sig(4, 4)).unwrap();
    // On a tick start (tick 1000 at 25 000 samples, not yet played): the new start of tick 1000
    assert_eq!(relocate(&old, &new, 25_000), 50_000);
    // 10 samples into tick 1000 (of 25): 20 samples into it (of 50)
    assert_eq!(relocate(&old, &new, 25_010), 50_020);
    // The last sample of the tick: still inside it
    assert_eq!(relocate(&old, &new, 25_024), 50_048);
    assert_eq!(relocate(&old, &new, 0), 0);
    assert_eq!(relocate(&old, &new, -25), -50);
    // Back again
    assert_eq!(relocate(&new, &old, 50_020), 25_010);
}

#[test]
fn equal_maps_do_not_move() {
    let mut rng = Rng::new(3);
    let spec = random_map(&mut rng, true, true);
    let (a, b) = (spec.build(), spec.build());
    for _ in 0..1000 {
        let s = rng.range(-1_000_000, 1 << 40);
        assert_eq!(relocate(&a, &b, s), s);
        assert_eq!(relocate(&a, &a.clone(), s), s);
    }
}

#[test]
fn no_tick_is_replayed_or_skipped_on_random_edits() {
    for seed in 0..400 {
        let mut rng = Rng::new(70_000 + seed);
        let realistic = seed % 2 == 0;
        let old = random_map(&mut rng, true, realistic).build();
        // The edit: same rate and PPQ, other points
        let mut edited = random_map(&mut rng, true, realistic);
        edited.sample_rate = old.sample_rate();
        edited.ppq = old.ppq();
        if edited.points.iter().any(|p| p.time_signature.beat_ticks(edited.ppq).is_none()) {
            continue;
        }
        let new = edited.build();
        let end = old.sample_at(old.points().last().unwrap().tick.saturating_mul(2)).max(10_000);
        for _ in 0..200 {
            let s = rng.range(-100_000, end);
            let r = relocate(&old, &new, s);
            // The ticks started before the playhead: t <= tau, in both maps
            let tau = old.tick_at(s - 1);
            let (a, b) = (new.sample_at(tau), new.sample_at(tau + 1));
            if b > a {
                assert_eq!(new.tick_at(r - 1), tau, "seed {seed}: sample {s} → {r}");
                assert!(a < r && r <= b);
            } else {
                assert_eq!(r, a + 1, "seed {seed}");
            }
            // The fraction of the tick already played is kept (to a sample of rounding)
            let (oa, ob) = (old.sample_at(tau), old.sample_at(tau + 1));
            let fraction = (s - oa) as f64 / (ob - oa) as f64;
            if b > a + 1 {
                let kept = (r - a) as f64 / (b - a) as f64;
                assert!((kept - fraction).abs() <= 1.0 / (b - a) as f64 + 1e-12, "seed {seed}");
            }
        }
    }
}

#[test]
fn relocation_across_a_new_ramp() {
    let old = TempoMap::constant(48_000, Ppq::DEFAULT, bpm("120"), sig(4, 4)).unwrap();
    let new = TempoMap::new(
        48_000,
        Ppq::DEFAULT,
        &[TempoPoint::new(0, bpm("120"), sig(4, 4)).linear(), TempoPoint::new(3840, bpm("60"), sig(4, 4))],
    )
    .unwrap();
    // Bar 2 starts at 96 000 in the old map and at 128 000 in the new one (the ramp lasts 8/3 s)
    assert_eq!(relocate(&old, &new, 96_000), 128_000);
    // Inside the ramp: the tick is kept
    let s = 50_000;
    let r = relocate(&old, &new, s);
    assert_eq!(new.tick_at(r - 1), old.tick_at(s - 1));
}
