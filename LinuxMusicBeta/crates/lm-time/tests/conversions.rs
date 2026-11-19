//! Tick ↔ sample conversions on random maps: the rounding rules, the exact round trips, the reference, ramps
mod common;

use common::{MapSpec, Reference, Rng, bpm, random_map, sig};
use lm_time::{Ppq, SegmentShape, TempoMap, TempoPoint, TempoShape};

const MAPS: u64 = 600;

/// Ticks worth testing on a map: around every point, inside every segment, before 0 and past the end
fn interesting_ticks(rng: &mut Rng, map: &TempoMap) -> Vec<i64> {
    let last = map.points().last().unwrap().tick;
    let mut out = vec![0, 1, -1, last, last + 1, last.saturating_mul(2) + 7];
    for p in map.points() {
        for d in -3..=3 {
            out.push(p.tick + d);
        }
    }
    for _ in 0..200 {
        out.push(rng.range(-1_000_000, last.saturating_mul(2).max(1000)));
    }
    out
}

fn interesting_samples(rng: &mut Rng, map: &TempoMap) -> Vec<i64> {
    let mut out = vec![0, 1, -1];
    for seg in map.segments() {
        for edge in [seg.start_sample(), seg.end_sample()] {
            if edge.unsigned_abs() < (1 << 62) {
                for d in -3..=3 {
                    out.push(edge + d);
                }
            }
        }
    }
    let end = map.sample_at(map.points().last().unwrap().tick.saturating_mul(2)).max(1000);
    for _ in 0..200 {
        out.push(rng.range(-1_000_000, end));
    }
    out
}

/// Guarantees 1 to 4 of the crate docs on one map
fn check_guarantees(seed: u64, rng: &mut Rng, map: &TempoMap) {
    for t in interesting_ticks(rng, map) {
        let s = map.sample_at(t);
        assert!(map.sample_at(t + 1) >= s, "seed {seed}: sample_at not monotone at tick {t}");
        // Galois: t <= tick_at(sample_at(t)), with equality where ticks are no finer than samples
        let back = map.tick_at(s);
        assert!(back >= t, "seed {seed}: tick_at(sample_at({t})) = {back}");
        if map.ticks_coarser_at_tick(t) {
            assert_eq!(back, t, "seed {seed}: tick → sample → tick not exact at {t} (sample {s})");
        }
    }
    for s in interesting_samples(rng, map) {
        let t = map.tick_at(s);
        assert!(map.tick_at(s + 1) >= t, "seed {seed}: tick_at not monotone at sample {s}");
        let start = map.sample_at(t);
        let next = map.sample_at(t + 1);
        assert!(start <= s && s < next, "seed {seed}: sample {s} → tick {t} starting at {start}, next at {next}");
        if map.ticks_finer_at_sample(s) {
            assert_eq!(start, s, "seed {seed}: sample → tick → sample not exact at {s}");
        }
    }
}

#[test]
fn guarantees_hold_on_random_maps_with_jumps() {
    for seed in 0..MAPS {
        let mut rng = Rng::new(seed);
        let map = random_map(&mut rng, false, false).build();
        check_guarantees(seed, &mut rng, &map);
    }
}

#[test]
fn guarantees_hold_on_random_maps_with_ramps() {
    for seed in 0..MAPS {
        let mut rng = Rng::new(10_000 + seed);
        let map = random_map(&mut rng, true, false).build();
        check_guarantees(seed, &mut rng, &map);
    }
}

#[test]
fn tick_sample_tick_is_exact_on_every_realistic_map() {
    // 960 PPQ, 44.1 kHz and up, any tempo up to 1000 BPM: ticks are always coarser than samples
    for seed in 0..MAPS {
        let mut rng = Rng::new(20_000 + seed);
        let map = random_map(&mut rng, true, true).build();
        for t in interesting_ticks(&mut rng, &map) {
            assert!(map.ticks_coarser_at_tick(t));
            assert_eq!(map.tick_at(map.sample_at(t)), t, "seed {seed} tick {t}");
        }
    }
}

#[test]
fn constant_maps_match_the_independent_reference() {
    for seed in 0..MAPS {
        let mut rng = Rng::new(30_000 + seed);
        let spec = random_map(&mut rng, false, seed % 2 == 0);
        let map = spec.build();
        let reference = Reference::new(&spec);
        for t in interesting_ticks(&mut rng, &map) {
            assert_eq!(i128::from(map.sample_at(t)), reference.sample_at(t), "seed {seed} tick {t}");
        }
        for s in interesting_samples(&mut rng, &map) {
            assert_eq!(map.tick_at(s), reference.tick_at(s), "seed {seed} sample {s}");
        }
    }
}

/// The real sample offset of `dt` ticks into a ramp, in f64 (for comparison only)
fn ramp_offset_f64(map: &TempoMap, b0: f64, b1: f64, length: i64, dt: i64) -> f64 {
    let sr = f64::from(map.sample_rate());
    let ppq = f64::from(map.ppq().get());
    let duration = 120.0 * sr * length as f64 / (ppq * (b0 + b1));
    // ppq / (60 sr) · (b0 y + (b1 − b0) y² / (2Y)) = dt
    let a = (b1 - b0) / (2.0 * duration);
    let c = dt as f64 * 60.0 * sr / ppq;
    2.0 * c / (b0 + (b0 * b0 + 4.0 * a * c).sqrt())
}

#[test]
fn ramps_agree_with_the_real_valued_solution() {
    let mut checked = 0;
    for seed in 0..MAPS {
        let mut rng = Rng::new(40_000 + seed);
        let map = random_map(&mut rng, true, seed % 2 == 0).build();
        for seg in map.segments() {
            let SegmentShape::Linear { from, to } = seg.shape() else { continue };
            let length = seg.end_tick() - seg.start_tick();
            let mut dts = vec![0, 1, length - 1, length];
            for _ in 0..100 {
                dts.push(rng.range(0, length));
            }
            for dt in dts {
                let exact = map.sample_at(seg.start_tick() + dt) - seg.start_sample();
                let real = ramp_offset_f64(&map, from.to_f64(), to.to_f64(), length, dt);
                let tolerance = 0.5 + 1e-9 * real.max(1.0);
                assert!(
                    (exact as f64 - real).abs() <= tolerance,
                    "seed {seed}: ramp {from}→{to} over {length} ticks, dt {dt}: exact {exact}, real {real}"
                );
                checked += 1;
            }
            // The ramp ends exactly on the next segment's anchor
            assert_eq!(map.sample_at(seg.end_tick()), seg.end_sample());
        }
    }
    assert!(checked > 10_000, "only {checked} ramp ticks checked");
}

#[test]
fn every_sample_of_small_maps_against_the_staircase() {
    // Walk every tick, build the staircase of start samples, and check tick_at on every sample under it
    for seed in 0..200 {
        let mut rng = Rng::new(50_000 + seed);
        let sample_rate = rng.pick(&[1_000, 8_000, 44_100, 48_000]);
        let ppq = Ppq::new(rng.pick(&[1, 24, 96, 960])).unwrap();
        let mut points = vec![TempoPoint::new(0, common::random_bpm(&mut rng), sig(4, 4))];
        let mut tick = 0;
        for _ in 0..rng.range(0, 5) {
            tick += rng.range(1, 3000);
            let shape = if rng.chance(50) { TempoShape::Linear } else { TempoShape::Constant };
            points.last_mut().unwrap().shape = shape;
            points.push(TempoPoint::new(tick, common::random_bpm(&mut rng), sig(4, 4)));
        }
        let spec = MapSpec { sample_rate, ppq, points };
        let map = spec.build();
        let first_tick = -50;
        let last_tick = tick + 3000;
        let mut current = first_tick;
        let mut s = map.sample_at(first_tick);
        // At most 300 000 samples per map (slow tempos at low PPQ have millions of samples per tick)
        let end = map.sample_at(last_tick).min(s + 300_000);
        while s < end {
            while map.sample_at(current + 1) <= s {
                current += 1;
            }
            assert_eq!(map.tick_at(s), current, "seed {seed} sample {s}");
            s += 1;
        }
    }
}

#[test]
fn known_values() {
    let map = TempoMap::new(
        48_000,
        Ppq::DEFAULT,
        &[
            TempoPoint::new(0, bpm("120"), sig(4, 4)),
            TempoPoint::new(3840, bpm("60"), sig(4, 4)),
            TempoPoint::new(7680, bpm("119.88"), sig(4, 4)).linear(),
            TempoPoint::new(11_520, bpm("60"), sig(4, 4)),
        ],
    )
    .unwrap();
    // One bar at 120 = 2 s, one bar at 60 = 4 s
    assert_eq!(map.sample_at(3840), 96_000);
    assert_eq!(map.sample_at(7680), 288_000);
    assert_eq!(map.tick_at(95_999), 3839);
    assert_eq!(map.tick_at(96_000), 3840);
    // 25 samples per tick at 120, 50 at 60
    assert_eq!(map.sample_at(1), 25);
    assert_eq!(map.sample_at(3841), 96_050);
    // Before 0 the first tempo continues
    assert_eq!(map.sample_at(-1), -25);
    assert_eq!(map.tick_at(-1), -1);
    assert_eq!(map.tick_at(-25), -1);
    assert_eq!(map.tick_at(-26), -2);
    // The ramp 119.88 → 60 over a bar lasts 120 · 48000 · 3840 / (960 · 179.88) s·samples
    let ramp_samples = (120.0 * 48_000.0 * 3840.0 / (960.0 * 179.88_f64)).round_ties_even() as i64;
    assert_eq!(map.sample_at(11_520), 288_000 + ramp_samples);
    // 119.88 at 48 kHz, 960 PPQ: 48000·60/(119.88·960) = 25.025... samples per tick
    let tempo = TempoMap::constant(48_000, Ppq::DEFAULT, bpm("119.88"), sig(4, 4)).unwrap();
    assert_eq!(tempo.sample_at(1000), 25_025);
    assert_eq!(tempo.sample_at(1), 25);
    assert_eq!(tempo.sample_at(40), 1001);
}

#[test]
fn ties_round_to_even() {
    // 1 BPM, 1 PPQ at 1 Hz: 60 samples per tick; at 2 PPQ... use 7 BPM at 1 Hz and 120 PPQ: 60/(7·120) = 1/14
    // A cleaner tie: 30 samples per 4 ticks → 7.5 samples per tick (sample rate 1, 8 BPM, 1 PPQ)
    let map = TempoMap::constant(1, Ppq::new(1).unwrap(), bpm("8"), sig(4, 4)).unwrap();
    assert_eq!(map.sample_at(1), 8); // 7.5 → 8 (even)
    assert_eq!(map.sample_at(3), 22); // 22.5 → 22 (even)
    assert_eq!(map.sample_at(-1), -8); // -7.5 → -8
    assert_eq!(map.sample_at(-3), -22); // -22.5 → -22
    // tick_at is the inverse staircase: tick 1 starts at 8, tick 2 at 15, tick 3 at 22
    assert_eq!(map.tick_at(7), 0);
    assert_eq!(map.tick_at(8), 1);
    assert_eq!(map.tick_at(21), 2);
    assert_eq!(map.tick_at(22), 3);
}

#[test]
fn extreme_positions_saturate_instead_of_overflowing() {
    let slow = TempoMap::constant(4_000_000, Ppq::new(1).unwrap(), bpm("1"), sig(4, 4)).unwrap();
    assert_eq!(slow.sample_at(i64::MAX), i64::MAX);
    assert_eq!(slow.sample_at(i64::MIN), i64::MIN);
    let fast = TempoMap::constant(1, Ppq::new(100_000).unwrap(), bpm("1000"), sig(4, 4)).unwrap();
    assert_eq!(fast.tick_at(i64::MAX), i64::MAX);
    assert_eq!(fast.tick_at(i64::MIN), i64::MIN);
    for map in [&slow, &fast] {
        assert!(map.tick_at(i64::MAX) >= map.tick_at(i64::MAX - 1));
        let _ = map.sample_at(map.tick_at(0));
    }
}

#[test]
fn construction_errors() {
    let p = |tick| TempoPoint::new(tick, bpm("120"), sig(4, 4));
    let key = |r: lm_base::Result<TempoMap>| r.unwrap_err().key();
    assert_eq!(key(TempoMap::new(48_000, Ppq::DEFAULT, &[])), "time.map_empty");
    assert_eq!(key(TempoMap::new(0, Ppq::DEFAULT, &[p(0)])), "time.sample_rate_invalid");
    assert_eq!(key(TempoMap::new(4_000_001, Ppq::DEFAULT, &[p(0)])), "time.sample_rate_invalid");
    assert_eq!(key(TempoMap::new(48_000, Ppq::DEFAULT, &[p(10)])), "time.map_first_point_not_zero");
    assert_eq!(key(TempoMap::new(48_000, Ppq::DEFAULT, &[p(0), p(5), p(5)])), "time.map_points_not_increasing");
    assert_eq!(key(TempoMap::new(48_000, Ppq::DEFAULT, &[p(0), p(5), p(3)])), "time.map_points_not_increasing");
    assert_eq!(key(TempoMap::new(48_000, Ppq::DEFAULT, &[p(0), p((1 << 62) + 1)])), "time.map_too_long");
    // In samples: a point at 2^61 ticks at 1 BPM, 1 PPQ is far past 2^62 samples
    let slow = |tick| TempoPoint::new(tick, bpm("1"), sig(4, 4));
    assert_eq!(key(TempoMap::new(48_000, Ppq::new(1).unwrap(), &[slow(0), slow(1 << 61)])), "time.map_too_long");
    assert_eq!(key(TempoMap::new(48_000, Ppq::DEFAULT, &[p(0), p(5).linear()])), "time.ramp_without_end");
    let odd = TempoPoint::new(0, bpm("120"), sig(4, 16));
    assert_eq!(key(TempoMap::new(48_000, Ppq::new(3).unwrap(), &[odd])), "time.signature_not_on_tick_grid");
}

#[test]
fn a_ramp_between_equal_tempos_is_constant() {
    let map = TempoMap::new(
        48_000,
        Ppq::DEFAULT,
        &[TempoPoint::new(0, bpm("100"), sig(4, 4)).linear(), TempoPoint::new(960, bpm("100"), sig(4, 4))],
    )
    .unwrap();
    assert!(!map.segments().iter().any(|s| s.is_ramp()));
    assert_eq!(map.segments()[0].shape(), SegmentShape::Constant(bpm("100")));
}

#[test]
fn a_first_ramp_gets_a_constant_lead_in() {
    let map = TempoMap::new(
        48_000,
        Ppq::DEFAULT,
        &[TempoPoint::new(0, bpm("120"), sig(4, 4)).linear(), TempoPoint::new(3840, bpm("60"), sig(4, 4))],
    )
    .unwrap();
    let segs = map.segments();
    assert_eq!(segs.len(), 3);
    assert_eq!((segs[0].start_sample(), segs[0].end_sample()), (i64::MIN, 0));
    assert_eq!(segs[0].shape(), SegmentShape::Constant(bpm("120")));
    assert!(segs[1].is_ramp());
    assert_eq!(map.sample_at(-4), -100);
    assert_eq!(map.sample_at(3840), 128_000);
    assert_eq!(map.bpm_at_sample(-1000), 120.0);
    assert_eq!(map.bpm_at_sample(0), 120.0);
    assert!((map.bpm_at_sample(64_000) - 90.0).abs() < 1e-9);
    assert_eq!(map.bpm_at_sample(200_000), 60.0);
}

#[test]
fn maps_are_cheap_to_clone_and_compare_by_content() {
    let mut rng = Rng::new(7);
    let spec = random_map(&mut rng, true, true);
    let a = spec.build();
    let b = a.clone();
    let c = spec.build();
    assert_eq!(a, b);
    assert_eq!(a, c);
    let other = TempoMap::new(spec.sample_rate + 1, spec.ppq, &spec.points).unwrap();
    assert_ne!(a, other);
    let moved = std::thread::spawn(move || b.sample_at(12_345)).join().unwrap();
    assert_eq!(moved, a.sample_at(12_345));
}
